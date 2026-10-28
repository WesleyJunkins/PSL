//! The supervisor state machine (docs/01 §Startup sequence, §Health and failure).

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::json::Json;
use crate::model::{Graph, Restart};
use crate::protocol::control::{msg, read_message, write_message};
use crate::protocol::frame::{read_frame, write_frame, FrameType};
use crate::protocol::{Conn, Frame, Listener};
use crate::supervisor::proc::{self, SpawnEnv};
use crate::supervisor::queue::DropQueue;
use crate::supervisor::router::Router;
use crate::supervisor::session::{Clock, Session};
use crate::supervisor::signal;
use crate::syntax::Kind;
use crate::util::sha256_hex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn parse(s: &str) -> LogLevel {
        match s {
            "debug" => LogLevel::Debug,
            "warn" | "warning" => LogLevel::Warn,
            "error" => LogLevel::Error,
            _ => LogLevel::Info,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

pub struct RunOptions {
    pub record: Option<PathBuf>,
    /// Node log lines at this level or above are echoed to the terminal.
    pub echo_level: LogLevel,
    pub python: String,
    pub sdk_path: PathBuf,
}

enum Ev {
    NewControl(Conn),
    NewData(Conn),
    DataIdentified {
        conn: Conn,
        node_id: String,
        session: String,
    },
    Control {
        cid: u64,
        msg: Json,
    },
    ControlClosed {
        cid: u64,
    },
    DataClosed {
        key: String,
    },
    StreamEnded {
        stream: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NState {
    Spawning,
    Connected,
    Ready,
    Running,
    Stopping,
    Stopped,
    Failed,
    Dead,
}

impl NState {
    fn as_str(&self) -> &'static str {
        match self {
            NState::Spawning => "spawning",
            NState::Connected => "configured",
            NState::Ready => "ready",
            NState::Running => "running",
            NState::Stopping => "stopping",
            NState::Stopped => "stopped",
            NState::Failed => "failed",
            NState::Dead => "dead",
        }
    }
    fn is_finished(&self) -> bool {
        matches!(self, NState::Stopped | NState::Failed | NState::Dead)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Pending,
    Node(usize),
    Tool,
}

struct Client {
    key: Option<String>,
    role: Role,
    control: Arc<Mutex<Conn>>,
    frames_in: Arc<AtomicU64>,
    subs: Vec<String>,
}

struct NodeRec {
    child: Option<Child>,
    pid: u32,
    cid: Option<u64>,
    state: NState,
    restarts: u32,
    spawned_at: Instant,
    missed_pings: u32,
    stats: Json,
    log: File,
    restart_at: Option<Instant>,
    failed_reason: Option<String>,
}

enum Phase {
    Starting {
        deadline: Instant,
    },
    Running,
    Stopping {
        stages: Vec<Vec<usize>>,
        stage: usize,
        deadline: Instant,
    },
    Done,
}

struct Supervisor {
    root: PathBuf,
    graph: Arc<Graph>,
    opts: RunOptions,
    session: Session,
    router: Arc<Router>,
    clients: HashMap<u64, Client>,
    key_to_cid: HashMap<String, u64>,
    nodes: Vec<NodeRec>,
    next_cid: u64,
    tx: Sender<Ev>,
    phase: Phase,
    ping_n: u64,
    exit_code: i32,
    pending_acks: HashMap<(String, String), u64>,
    ended_streams: std::collections::HashSet<String>,
    announced_running: bool,
}

/// Run a checked pipeline to completion. Returns the process exit code.
pub fn run(root: &Path, graph: Graph, opts: RunOptions) -> Result<i32, String> {
    signal::install();
    let (tx, rx) = channel::<Ev>();
    let mut session = Session::create(root, &graph, &graph.file)
        .map_err(|e| format!("cannot create session: {}", e))?;
    println!("[psl] session {}  ({})", session.id, session.dir.display());
    if let Some(l) = session.control.take() {
        spawn_acceptor(l, tx.clone(), true);
    }
    if let Some(l) = session.data.take() {
        spawn_acceptor(l, tx.clone(), false);
    }
    let graph = Arc::new(graph);
    let router = Arc::new(Router::from_graph(&graph));
    let ready_timeout = Duration::from_secs_f64(graph.runtime.ready_timeout_s);
    let mut sv = Supervisor {
        root: root.to_path_buf(),
        graph: graph.clone(),
        opts,
        session,
        router,
        clients: HashMap::new(),
        key_to_cid: HashMap::new(),
        nodes: Vec::new(),
        next_cid: 1,
        tx,
        phase: Phase::Starting {
            deadline: Instant::now() + ready_timeout,
        },
        ping_n: 0,
        exit_code: 0,
        pending_acks: HashMap::new(),
        ended_streams: Default::default(),
        announced_running: false,
    };
    if let Some(path) = sv.opts.record.clone() {
        let path = if path.is_relative() {
            root.join(path)
        } else {
            path
        };
        sv.router
            .start_recording(
                &path,
                &sv.graph,
                &sv.session.id,
                sv.session.wall_start_ns,
                sv.session.clock.clone(),
            )
            .map_err(|e| format!("cannot record to {}: {}", path.display(), e))?;
        println!("[psl] recording to {}", path.display());
    }
    for i in 0..graph.nodes.len() {
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(sv.session.log_path(&graph.nodes[i].id))
            .map_err(|e| e.to_string())?;
        sv.nodes.push(NodeRec {
            child: None,
            pid: 0,
            cid: None,
            state: NState::Spawning,
            restarts: 0,
            spawned_at: Instant::now(),
            missed_pings: 0,
            stats: Json::Null,
            log,
            restart_at: None,
            failed_reason: None,
        });
    }
    let names: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    println!(
        "[psl] starting {} node{}: {}",
        names.len(),
        if names.len() == 1 { "" } else { "s" },
        names.join(", ")
    );
    for i in 0..graph.nodes.len() {
        sv.spawn_node(i);
    }

    let mut last_tick = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(ev) => sv.handle(ev),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if signal::take() {
            match sv.phase {
                Phase::Stopping { .. } => {
                    println!("[psl] second interrupt: killing remaining nodes");
                    sv.kill_all();
                    sv.phase = Phase::Done;
                }
                Phase::Done => {}
                _ => {
                    println!("[psl] interrupt: stopping");
                    sv.begin_shutdown(0);
                }
            }
        }
        sv.poll_children();
        if last_tick.elapsed() >= Duration::from_secs(1) {
            last_tick = Instant::now();
            sv.tick();
        }
        sv.check_phase();
        if matches!(sv.phase, Phase::Done) {
            break;
        }
    }
    sv.finish()
}

fn spawn_acceptor(listener: Listener, tx: Sender<Ev>, control: bool) {
    thread::spawn(move || {
        while let Ok(conn) = listener.accept() {
            let ev = if control {
                Ev::NewControl(conn)
            } else {
                Ev::NewData(conn)
            };
            if tx.send(ev).is_err() {
                break;
            }
        }
    });
}

fn control_reader(
    mut conn: Conn,
    writer: Arc<Mutex<Conn>>,
    clock: Arc<Clock>,
    tx: Sender<Ev>,
    cid: u64,
) {
    loop {
        match read_message(&mut conn) {
            Ok(Some(m)) => {
                if m.str_field("type") == Some("clock_req") {
                    let t2 = clock.now_ns();
                    let reply = msg("clock_res")
                        .with("t1", m.get("t1").cloned().unwrap_or(Json::Int(0)))
                        .with("t2", t2)
                        .with("t3", clock.now_ns());
                    if let Ok(mut w) = writer.lock() {
                        let _ = write_message(&mut *w, &reply);
                    }
                    continue;
                }
                if tx.send(Ev::Control { cid, msg: m }).is_err() {
                    return;
                }
            }
            _ => {
                let _ = tx.send(Ev::ControlClosed { cid });
                return;
            }
        }
    }
}

fn data_reader(
    mut conn: Conn,
    key: String,
    outputs: Vec<String>,
    router: Arc<Router>,
    frames_in: Arc<AtomicU64>,
    tx: Sender<Ev>,
) {
    loop {
        match read_frame(&mut conn) {
            Ok(Some(frame)) => {
                frames_in.fetch_add(1, Ordering::Relaxed);
                if outputs.is_empty() {
                    continue; // tools do not publish
                }
                let port = frame.header.port as usize;
                if port >= outputs.len() {
                    continue;
                }
                if frame.header.frame_type() == Some(FrameType::EndStream) {
                    let _ = tx.send(Ev::StreamEnded {
                        stream: outputs[port].clone(),
                    });
                }
                router.route(&outputs[port], frame);
            }
            _ => {
                let _ = tx.send(Ev::DataClosed { key });
                return;
            }
        }
    }
}

fn data_writer(mut conn: Conn, queue: Arc<DropQueue>, key: String, tx: Sender<Ev>) {
    while let Some(f) = queue.pop() {
        if write_frame(&mut conn, &f).is_err() {
            let _ = tx.send(Ev::DataClosed { key });
            return;
        }
    }
}

impl Supervisor {
    fn node_index(&self, id: &str) -> Option<usize> {
        self.graph.nodes.iter().position(|n| n.id == id)
    }

    fn spawn_node(&mut self, idx: usize) {
        let node = &self.graph.nodes[idx];
        let log_path = self.session.log_path(&node.id);
        let env = SpawnEnv {
            workspace: &self.root,
            session: &self.session.id,
            control: &self.session.control_addr,
            data: &self.session.data_addr,
            python: &self.opts.python,
            sdk_path: &self.opts.sdk_path,
            log_path: &log_path,
        };
        let rec = &mut self.nodes[idx];
        rec.restart_at = None;
        rec.cid = None;
        rec.missed_pings = 0;
        rec.failed_reason = None;
        match proc::spawn(node, &env) {
            Ok(child) => {
                rec.pid = child.id();
                rec.child = Some(child);
                rec.state = NState::Spawning;
                rec.spawned_at = Instant::now();
            }
            Err(e) => {
                let id = node.id.clone();
                self.node_failed(idx, format!("cannot start `{}`: {}", id, e));
            }
        }
    }

    fn handle(&mut self, ev: Ev) {
        match ev {
            Ev::NewControl(conn) => {
                let cid = self.next_cid;
                self.next_cid += 1;
                let reader = match conn.try_clone() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let writer = Arc::new(Mutex::new(conn));
                self.clients.insert(
                    cid,
                    Client {
                        key: None,
                        role: Role::Pending,
                        control: writer.clone(),
                        frames_in: Arc::new(AtomicU64::new(0)),
                        subs: Vec::new(),
                    },
                );
                let clock = self.session.clock.clone();
                let tx = self.tx.clone();
                thread::spawn(move || control_reader(reader, writer, clock, tx, cid));
            }
            Ev::NewData(mut conn) => {
                let tx = self.tx.clone();
                thread::spawn(move || {
                    let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
                    if let Ok(Some(f)) = read_frame(&mut conn) {
                        if f.header.frame_type() == Some(FrameType::Identify) {
                            if let Some(j) =
                                crate::protocol::frame::decode_stream_header(&f.payload)
                            {
                                let node_id = j.str_field("node_id").unwrap_or("").to_string();
                                let session = j.str_field("session").unwrap_or("").to_string();
                                let _ = conn.set_read_timeout(None);
                                let _ = tx.send(Ev::DataIdentified {
                                    conn,
                                    node_id,
                                    session,
                                });
                            }
                        }
                    }
                });
            }
            Ev::DataIdentified {
                conn,
                node_id,
                session,
            } => self.on_data_identified(conn, node_id, session),
            Ev::Control { cid, msg } => self.on_control(cid, msg),
            Ev::ControlClosed { cid } => self.on_control_closed(cid),
            Ev::DataClosed { key } => {
                self.router.remove_client(&key);
            }
            Ev::StreamEnded { stream } => {
                self.ended_streams.insert(stream);
            }
        }
    }

    fn send_to(&self, cid: u64, m: &Json) {
        if let Some(c) = self.clients.get(&cid) {
            if let Ok(mut w) = c.control.lock() {
                let _ = write_message(&mut *w, m);
            }
        }
    }

    fn send_to_node(&self, idx: usize, m: &Json) -> bool {
        match self.nodes[idx].cid {
            Some(cid) => {
                self.send_to(cid, m);
                true
            }
            None => false,
        }
    }

    fn reject(&mut self, cid: u64, reason: &str) {
        self.send_to(cid, &msg("reject").with("reason", reason));
        if let Some(c) = self.clients.remove(&cid) {
            if let Ok(w) = c.control.lock() {
                w.shutdown();
            }
        }
    }

    fn on_control(&mut self, cid: u64, m: Json) {
        let ty = m.str_field("type").unwrap_or("").to_string();
        let role = match self.clients.get(&cid) {
            Some(c) => c.role,
            None => return,
        };
        if ty == "hello" {
            self.on_hello(cid, &m);
            return;
        }
        match role {
            Role::Pending => {
                self.reject(cid, "first message must be hello");
            }
            Role::Node(idx) => self.on_node_message(cid, idx, &ty, &m),
            Role::Tool => self.on_tool_message(cid, &ty, &m),
        }
    }

    fn on_hello(&mut self, cid: u64, m: &Json) {
        if m.i64_field("protocol") != Some(crate::PROTOCOL_VERSION as i64) {
            self.reject(
                cid,
                &format!(
                    "unsupported protocol version {:?}; this supervisor speaks {}",
                    m.get("protocol"),
                    crate::PROTOCOL_VERSION
                ),
            );
            return;
        }
        match m.str_field("role") {
            Some("node") => {
                let node_id = m.str_field("node_id").unwrap_or("").to_string();
                let idx = match self.node_index(&node_id) {
                    Some(i) => i,
                    None => {
                        self.reject(cid, &format!("unknown node id `{}`", node_id));
                        return;
                    }
                };
                if self.nodes[idx].cid.is_some() {
                    self.reject(cid, &format!("node `{}` is already connected", node_id));
                    return;
                }
                if let Some(h) = m.str_field("manifest_hash") {
                    if !h.is_empty() {
                        let expected = std::fs::read(self.graph.nodes[idx].path.join("node.psl"))
                            .map(|b| sha256_hex(&b))
                            .unwrap_or_default();
                        if !expected.is_empty() && expected != h {
                            self.reject(
                                cid,
                                "manifest changed since the pipeline was checked; run again",
                            );
                            return;
                        }
                    }
                }
                let c = self.clients.get_mut(&cid).unwrap();
                c.key = Some(node_id.clone());
                c.role = Role::Node(idx);
                self.key_to_cid.insert(node_id.clone(), cid);
                self.nodes[idx].cid = Some(cid);
                self.nodes[idx].state = NState::Connected;
                self.nodes[idx].missed_pings = 0;
                let cfg = self.config_for(idx);
                self.send_to(cid, &cfg);
            }
            Some("tool") => {
                let key = format!("tool-{}", cid);
                let c = self.clients.get_mut(&cid).unwrap();
                c.key = Some(key.clone());
                c.role = Role::Tool;
                self.key_to_cid.insert(key.clone(), cid);
                let cfg = msg("config")
                    .with("session", self.session.id.as_str())
                    .with("node_id", key.as_str())
                    .with("wall_start_ns", self.session.wall_start_ns)
                    .with("params", Json::object())
                    .with("inputs", Vec::<Json>::new())
                    .with("outputs", Vec::<Json>::new())
                    .with("options", Json::object());
                self.send_to(cid, &cfg);
            }
            _ => self.reject(cid, "role must be \"node\" or \"tool\""),
        }
    }

    fn config_for(&self, idx: usize) -> Json {
        let n = &self.graph.nodes[idx];
        let inputs: Vec<Json> = n
            .inputs
            .iter()
            .map(|p| {
                Json::object()
                    .with("name", p.name.as_str())
                    .with("index", p.index as i64)
                    .with("type", p.ty.as_str())
                    .with(
                        "sources",
                        p.sources
                            .iter()
                            .enumerate()
                            .map(|(i, s)| {
                                Json::object()
                                    .with("index", i as i64)
                                    .with("stream", s.as_str())
                            })
                            .collect::<Vec<_>>(),
                    )
            })
            .collect();
        let outputs: Vec<Json> = n
            .outputs
            .iter()
            .map(|p| {
                Json::object()
                    .with("name", p.name.as_str())
                    .with("index", p.index as i64)
                    .with("type", p.ty.as_str())
                    .with("stream", p.stream.as_str())
            })
            .collect();
        msg("config")
            .with("session", self.session.id.as_str())
            .with("node_id", n.id.as_str())
            .with("wall_start_ns", self.session.wall_start_ns)
            .with("params", n.params.clone())
            .with("inputs", inputs)
            .with("outputs", outputs)
            .with(
                "options",
                Json::object()
                    .with("queue", n.runtime.queue)
                    .with("on_full", n.runtime.on_full.as_str()),
            )
    }

    fn on_data_identified(&mut self, conn: Conn, node_id: String, session: String) {
        if session != self.session.id {
            return;
        }
        let cid = match self.key_to_cid.get(&node_id) {
            Some(c) => *c,
            None => return,
        };
        let (role, frames_in) = match self.clients.get(&cid) {
            Some(c) => (c.role, c.frames_in.clone()),
            None => return,
        };
        let (cap, on_full, outputs) = match role {
            Role::Node(idx) => {
                let n = &self.graph.nodes[idx];
                (
                    n.runtime.queue as usize,
                    n.runtime.on_full,
                    n.outputs
                        .iter()
                        .map(|o| o.stream.clone())
                        .collect::<Vec<_>>(),
                )
            }
            _ => (256, crate::model::OnFull::DropOldest, Vec::new()),
        };
        let queue = self.router.ensure_queue(&node_id, cap, on_full);
        if let Ok(wconn) = conn.try_clone() {
            let (k, tx) = (node_id.clone(), self.tx.clone());
            thread::spawn(move || data_writer(wconn, queue, k, tx));
        }
        let (router, tx, key) = (self.router.clone(), self.tx.clone(), node_id.clone());
        thread::spawn(move || data_reader(conn, key, outputs, router, frames_in, tx));
    }

    fn on_node_message(&mut self, cid: u64, idx: usize, ty: &str, m: &Json) {
        match ty {
            "ready" => {
                self.nodes[idx].state = NState::Ready;
                self.router
                    .replay_headers_for(&self.graph, &self.graph.nodes[idx].id);
                if let Phase::Starting { .. } = self.phase {
                    if self.nodes.iter().all(|n| n.state == NState::Ready) {
                        self.start_all();
                    }
                } else if matches!(self.phase, Phase::Running) {
                    // A restarted node: start it right away.
                    self.send_to_node(idx, &msg("start"));
                }
            }
            "state" => {
                let s = m.str_field("state").unwrap_or("");
                match s {
                    "running" => self.nodes[idx].state = NState::Running,
                    "stopping" => self.nodes[idx].state = NState::Stopping,
                    "stopped" => self.nodes[idx].state = NState::Stopped,
                    "failed" => {
                        let reason = m
                            .str_field("reason")
                            .unwrap_or("node reported failure")
                            .to_string();
                        self.node_failed(idx, reason);
                    }
                    _ => {}
                }
            }
            "pong" => self.nodes[idx].missed_pings = 0,
            "stats" => self.nodes[idx].stats = m.clone(),
            "log" => {
                let level = LogLevel::parse(m.str_field("level").unwrap_or("info"));
                let text = m.str_field("msg").unwrap_or("").to_string();
                let fields = m
                    .get("fields")
                    .filter(|f| !matches!(f, Json::Object(o) if o.is_empty()))
                    .map(|f| format!(" {}", f))
                    .unwrap_or_default();
                let t = self.session.clock.now_ns() as f64 / 1e9;
                let _ = writeln!(
                    self.nodes[idx].log,
                    "{:12.6} {:5} {}{}",
                    t,
                    level.as_str(),
                    text,
                    fields
                );
                if level >= self.opts.echo_level {
                    println!(
                        "[{}] {}: {}{}",
                        self.graph.nodes[idx].id,
                        level.as_str(),
                        text,
                        fields
                    );
                }
            }
            "param_ack" => {
                let name = m.str_field("name").unwrap_or("").to_string();
                let key = (self.graph.nodes[idx].id.clone(), name);
                if let Some(tool) = self.pending_acks.remove(&key) {
                    self.send_to(tool, &m.clone().with("node_id", key.0.as_str()));
                }
            }
            _ => {
                let _ = cid;
            }
        }
    }

    fn on_tool_message(&mut self, cid: u64, ty: &str, m: &Json) {
        let key = self
            .clients
            .get(&cid)
            .and_then(|c| c.key.clone())
            .unwrap_or_default();
        match ty {
            "subscribe" => {
                let stream = m.str_field("stream").unwrap_or("").to_string();
                let exists = self
                    .graph
                    .nodes
                    .iter()
                    .any(|n| n.outputs.iter().any(|o| o.stream == stream));
                if !exists {
                    self.send_to(
                        cid,
                        &msg("error")
                            .with("code", "unknown_stream")
                            .with("msg", format!("no stream `{}`", stream)),
                    );
                    return;
                }
                let c = self.clients.get_mut(&cid).unwrap();
                let port = c.subs.len() as u16;
                c.subs.push(stream.clone());
                self.router
                    .ensure_queue(&key, 256, crate::model::OnFull::DropOldest);
                self.router.subscribe(&stream, &key, port);
                self.send_to(
                    cid,
                    &msg("subscribed")
                        .with("stream", stream.as_str())
                        .with("index", port as i64),
                );
            }
            "unsubscribe" => {
                let stream = m.str_field("stream").unwrap_or("").to_string();
                self.router.remove_dest(&stream, &key);
            }
            "set_param" => {
                let node_id = m.str_field("node_id").unwrap_or("").to_string();
                let name = m.str_field("name").unwrap_or("").to_string();
                let value = m.get("value").cloned().unwrap_or(Json::Null);
                let idx = match self.node_index(&node_id) {
                    Some(i) => i,
                    None => {
                        self.send_to(
                            cid,
                            &msg("param_ack")
                                .with("node_id", node_id.as_str())
                                .with("name", name.as_str())
                                .with("ok", false)
                                .with("reason", "unknown node"),
                        );
                        return;
                    }
                };
                let decl = self.graph.nodes[idx].manifest.param(&name).cloned();
                let reason = match &decl {
                    None => Some(format!("`{}` has no parameter `{}`", node_id, name)),
                    Some(d) if !d.mutable => Some(format!(
                        "parameter `{}` of `{}` is not mutable",
                        name, node_id
                    )),
                    Some(d) => crate::model::manifest::check_param_value(d, &value)
                        .err()
                        .map(|e| format!("parameter `{}` {}", name, e.message())),
                };
                if let Some(r) = reason {
                    self.send_to(
                        cid,
                        &msg("param_ack")
                            .with("node_id", node_id.as_str())
                            .with("name", name.as_str())
                            .with("ok", false)
                            .with("reason", r.as_str()),
                    );
                    return;
                }
                if !self.send_to_node(
                    idx,
                    &msg("set_param")
                        .with("name", name.as_str())
                        .with("value", value),
                ) {
                    self.send_to(
                        cid,
                        &msg("param_ack")
                            .with("node_id", node_id.as_str())
                            .with("name", name.as_str())
                            .with("ok", false)
                            .with("reason", "node is not connected"),
                    );
                    return;
                }
                self.pending_acks.insert((node_id, name), cid);
            }
            "record_start" => {
                let path = m
                    .str_field("path")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.session.dir.join("recording.pslrec"));
                let path = if path.is_relative() {
                    self.root.join(path)
                } else {
                    path
                };
                let r = self.router.start_recording(
                    &path,
                    &self.graph,
                    &self.session.id,
                    self.session.wall_start_ns,
                    self.session.clock.clone(),
                );
                match r {
                    Ok(()) => {
                        println!("[psl] recording to {}", path.display());
                        self.send_to(
                            cid,
                            &msg("result").with("query", "record_start").with(
                                "data",
                                Json::object().with("path", path.display().to_string()),
                            ),
                        );
                    }
                    Err(e) => self.send_to(
                        cid,
                        &msg("error")
                            .with("code", "record")
                            .with("msg", e.to_string()),
                    ),
                }
            }
            "record_stop" => {
                let data = match self.router.stop_recording() {
                    Some((p, n)) => {
                        println!("[psl] recording stopped: {} ({} frames)", p.display(), n);
                        Json::object()
                            .with("path", p.display().to_string())
                            .with("frames", n as i64)
                    }
                    None => Json::Null,
                };
                self.send_to(
                    cid,
                    &msg("result")
                        .with("query", "record_stop")
                        .with("data", data),
                );
            }
            "stop_pipeline" => {
                println!("[psl] stop requested by a tool");
                self.send_to(
                    cid,
                    &msg("result")
                        .with("query", "stop_pipeline")
                        .with("data", Json::Bool(true)),
                );
                self.begin_shutdown(0);
            }
            "query" => {
                let what = m.str_field("what").unwrap_or("").to_string();
                let data = self.query(&what);
                self.send_to(
                    cid,
                    &msg("result")
                        .with("query", what.as_str())
                        .with("data", data),
                );
            }
            _ => {}
        }
    }

    fn query(&self, what: &str) -> Json {
        match what {
            "graph" => self.graph.to_json(),
            "nodes" => Json::Array(
                self.graph
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| {
                        let r = &self.nodes[i];
                        let frames_in = r
                            .cid
                            .and_then(|c| self.clients.get(&c))
                            .map(|c| c.frames_in.load(Ordering::Relaxed))
                            .unwrap_or(0);
                        let (out, dropped, queued) = self
                            .router
                            .queue(&n.id)
                            .map(|q| {
                                (
                                    q.pushed.load(Ordering::Relaxed),
                                    q.dropped.load(Ordering::Relaxed),
                                    q.len() as u64,
                                )
                            })
                            .unwrap_or((0, 0, 0));
                        Json::object()
                            .with("id", n.id.as_str())
                            .with("kind", n.kind.as_str())
                            .with("package", n.package.as_str())
                            .with("state", r.state.as_str())
                            .with("pid", r.pid as i64)
                            .with("uptime_s", r.spawned_at.elapsed().as_secs_f64())
                            .with("restarts", r.restarts as i64)
                            .with("frames_from_node", frames_in as i64)
                            .with("frames_to_node", out as i64)
                            .with("dropped", dropped as i64)
                            .with("queued", queued as i64)
                            .with("stats", r.stats.clone())
                            .with("reason", r.failed_reason.clone())
                    })
                    .collect(),
            ),
            "streams" => {
                let headers: HashMap<String, Json> = self.router.headers().into_iter().collect();
                let headers = &headers;
                Json::Array(
                    self.graph
                        .nodes
                        .iter()
                        .flat_map(|n| {
                            n.outputs.iter().map(move |o| {
                                Json::object()
                                    .with("stream", o.stream.as_str())
                                    .with("node", n.id.as_str())
                                    .with("port", o.name.as_str())
                                    .with("type", o.ty.as_str())
                                    .with("header", headers.get(&o.stream).cloned())
                                    .with("ended", self.ended_streams.contains(&o.stream))
                            })
                        })
                        .collect(),
                )
            }
            "stats" => Json::object()
                .with(
                    "forwarded",
                    self.router.forwarded.load(Ordering::Relaxed) as i64,
                )
                .with("session_ns", self.session.clock.now_ns())
                .with("recording", self.router.is_recording()),
            _ => Json::Null,
        }
    }

    fn on_control_closed(&mut self, cid: u64) {
        let c = match self.clients.remove(&cid) {
            Some(c) => c,
            None => return,
        };
        if let Some(key) = &c.key {
            self.key_to_cid.remove(key);
            self.router.remove_client(key);
        }
        if let Role::Node(idx) = c.role {
            self.nodes[idx].cid = None;
            let st = self.nodes[idx].state;
            if !st.is_finished() && !matches!(self.phase, Phase::Stopping { .. } | Phase::Done) {
                self.nodes[idx].state = NState::Dead;
            } else if st == NState::Stopping {
                self.nodes[idx].state = NState::Stopped;
            }
        }
    }

    fn start_all(&mut self) {
        let order = self.graph.start_order();
        for id in order {
            if let Some(idx) = self.node_index(&id) {
                self.send_to_node(idx, &msg("start"));
            }
        }
        self.phase = Phase::Running;
        if !self.announced_running {
            self.announced_running = true;
            println!("[psl] all nodes ready; running (Ctrl-C to stop)");
        }
    }

    fn node_failed(&mut self, idx: usize, reason: String) {
        let id = self.graph.nodes[idx].id.clone();
        self.nodes[idx].state = NState::Failed;
        self.nodes[idx].failed_reason = Some(reason.clone());
        println!("[psl] node `{}` failed: {}", id, reason);
        let _ = writeln!(
            self.nodes[idx].log,
            "{:12.6} error node failed: {}",
            self.session.clock.now_ns() as f64 / 1e9,
            reason
        );
        if matches!(self.phase, Phase::Stopping { .. } | Phase::Done) {
            return;
        }
        match self.graph.nodes[idx].runtime.restart {
            Restart::Never => {
                self.print_last_log_lines(idx);
                self.begin_shutdown(3);
            }
            Restart::OnFailure | Restart::Always => self.schedule_restart(idx),
        }
    }

    fn schedule_restart(&mut self, idx: usize) {
        let delay = Duration::from_millis(1000);
        self.nodes[idx].restart_at = Some(Instant::now() + delay);
        println!(
            "[psl] restarting `{}` in {} ms (restart {})",
            self.graph.nodes[idx].id,
            delay.as_millis(),
            self.nodes[idx].restarts + 1
        );
    }

    fn print_last_log_lines(&self, idx: usize) {
        let path = self.session.log_path(&self.graph.nodes[idx].id);
        if let Ok(text) = std::fs::read_to_string(&path) {
            let lines: Vec<&str> = text.lines().collect();
            let tail = &lines[lines.len().saturating_sub(15)..];
            if !tail.is_empty() {
                println!("[psl] last lines of {}:", path.display());
                for l in tail {
                    println!("    {}", l);
                }
            }
        }
    }

    /// Reap exited children and apply restart policies.
    fn poll_children(&mut self) {
        for idx in 0..self.nodes.len() {
            let status = match self.nodes[idx]
                .child
                .as_mut()
                .and_then(|c| c.try_wait().ok().flatten())
            {
                Some(s) => s,
                None => continue,
            };
            self.nodes[idx].child = None;
            let code = status.code();
            let id = self.graph.nodes[idx].id.clone();
            if let Some(cid) = self.nodes[idx].cid.take() {
                self.clients.remove(&cid);
                self.key_to_cid.remove(&id);
            }
            self.router.remove_client(&id);
            let stopping = matches!(self.phase, Phase::Stopping { .. } | Phase::Done);
            let prev = self.nodes[idx].state;
            if stopping || prev == NState::Stopped {
                self.nodes[idx].state = if prev == NState::Failed {
                    NState::Failed
                } else {
                    NState::Stopped
                };
                continue;
            }
            if prev == NState::Failed {
                // Failure already handled (restart scheduled or shutdown begun).
                continue;
            }
            match (code, self.graph.nodes[idx].runtime.restart) {
                (Some(0), Restart::Never) | (Some(0), Restart::OnFailure) => {
                    self.nodes[idx].state = NState::Stopped;
                    println!("[psl] node `{}` exited", id);
                }
                (Some(0), Restart::Always) => {
                    self.nodes[idx].state = NState::Stopped;
                    self.schedule_restart(idx);
                }
                (c, Restart::Never) => {
                    self.print_last_log_lines(idx);
                    self.node_failed(idx, format!("exited with {}", describe_exit(c)));
                }
                (c, _) => {
                    self.nodes[idx].state = NState::Dead;
                    println!("[psl] node `{}` exited with {}", id, describe_exit(c));
                    self.schedule_restart(idx);
                }
            }
        }
    }

    fn tick(&mut self) {
        self.ping_n += 1;
        let ping = msg("ping").with("n", self.ping_n as i64);
        let now = Instant::now();
        for idx in 0..self.nodes.len() {
            // Due restarts.
            if let Some(at) = self.nodes[idx].restart_at {
                if now >= at && !matches!(self.phase, Phase::Stopping { .. } | Phase::Done) {
                    self.nodes[idx].restarts += 1;
                    self.spawn_node(idx);
                    continue;
                }
            }
            if self.nodes[idx].cid.is_none() {
                continue;
            }
            if matches!(
                self.nodes[idx].state,
                NState::Connected | NState::Ready | NState::Running | NState::Stopping
            ) {
                self.nodes[idx].missed_pings += 1;
                if self.nodes[idx].missed_pings > 3 {
                    let id = self.graph.nodes[idx].id.clone();
                    println!("[psl] node `{}` missed 3 heartbeats; killing it", id);
                    if let Some(c) = self.nodes[idx].child.as_mut() {
                        let _ = c.kill();
                    }
                    self.nodes[idx].missed_pings = 0;
                    continue;
                }
                self.send_to_node(idx, &ping);
            }
        }
        // Stop when every input driver has ended all of its streams.
        if self.graph.runtime.stop_when_inputs_end && matches!(self.phase, Phase::Running) {
            let all_ended = self
                .graph
                .nodes
                .iter()
                .filter(|n| n.kind == Kind::Input)
                .all(|n| {
                    n.outputs
                        .iter()
                        .all(|o| self.ended_streams.contains(&o.stream))
                });
            if all_ended {
                println!("[psl] all input streams ended");
                self.begin_shutdown(0);
            }
        }
    }

    fn check_phase(&mut self) {
        match &self.phase {
            Phase::Starting { deadline } => {
                if Instant::now() >= *deadline {
                    let waiting: Vec<String> = self
                        .graph
                        .nodes
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| self.nodes[*i].state != NState::Ready)
                        .map(|(i, n)| format!("{} ({})", n.id, self.nodes[i].state.as_str()))
                        .collect();
                    println!(
                        "[psl] timed out after {} s waiting for: {}",
                        self.graph.runtime.ready_timeout_s,
                        waiting.join(", ")
                    );
                    for (i, _) in self.graph.nodes.iter().enumerate() {
                        if self.nodes[i].state != NState::Ready {
                            self.print_last_log_lines(i);
                        }
                    }
                    self.begin_shutdown(3);
                }
            }
            Phase::Stopping { .. } => self.advance_shutdown(),
            _ => {}
        }
    }

    fn begin_shutdown(&mut self, code: i32) {
        if matches!(self.phase, Phase::Stopping { .. } | Phase::Done) {
            return;
        }
        self.exit_code = code;
        let order = self.graph.stop_order();
        let by_kind = |k: Kind| -> Vec<usize> {
            order
                .iter()
                .filter_map(|id| self.node_index(id))
                .filter(|&i| self.graph.nodes[i].kind == k)
                .collect()
        };
        let stages = vec![
            by_kind(Kind::Input),
            by_kind(Kind::Module),
            by_kind(Kind::Output),
        ];
        println!("[psl] stopping (inputs, then modules, then outputs)");
        self.phase = Phase::Stopping {
            stages,
            stage: 0,
            deadline: Instant::now(),
        };
        self.send_stop_stage();
    }

    fn send_stop_stage(&mut self) {
        let (stage_nodes, grace) = match &self.phase {
            Phase::Stopping { stages, stage, .. } => (
                stages.get(*stage).cloned().unwrap_or_default(),
                self.graph.runtime.stop_grace_s,
            ),
            _ => return,
        };
        let stop = msg("stop").with("grace_ms", (grace * 1000.0) as i64);
        for idx in stage_nodes {
            self.nodes[idx].restart_at = None;
            if self.nodes[idx].state.is_finished() {
                continue;
            }
            if self.send_to_node(idx, &stop) {
                self.nodes[idx].state = NState::Stopping;
            } else if let Some(c) = self.nodes[idx].child.as_mut() {
                let _ = c.kill();
            }
        }
        if let Phase::Stopping { deadline, .. } = &mut self.phase {
            *deadline = Instant::now() + Duration::from_secs_f64(grace.max(0.05));
        }
    }

    fn advance_shutdown(&mut self) {
        let (stage_nodes, stage, deadline, n_stages) = match &self.phase {
            Phase::Stopping {
                stages,
                stage,
                deadline,
            } => (
                stages.get(*stage).cloned().unwrap_or_default(),
                *stage,
                *deadline,
                stages.len(),
            ),
            _ => return,
        };
        let done = stage_nodes
            .iter()
            .all(|&i| self.nodes[i].child.is_none() || self.nodes[i].state.is_finished());
        if !done && Instant::now() < deadline {
            return;
        }
        if !done {
            for &i in &stage_nodes {
                if self.nodes[i].child.is_some() && !self.nodes[i].state.is_finished() {
                    println!(
                        "[psl] node `{}` did not stop within the grace period; killing it",
                        self.graph.nodes[i].id
                    );
                    if let Some(c) = self.nodes[i].child.as_mut() {
                        let _ = c.kill();
                    }
                    self.nodes[i].state = NState::Dead;
                }
            }
        }
        if stage + 1 >= n_stages {
            self.phase = Phase::Done;
            return;
        }
        if let Phase::Stopping { stage, .. } = &mut self.phase {
            *stage += 1;
        }
        self.send_stop_stage();
    }

    fn kill_all(&mut self) {
        for n in &mut self.nodes {
            if let Some(c) = n.child.as_mut() {
                let _ = c.kill();
            }
            if !n.state.is_finished() {
                n.state = NState::Dead;
            }
        }
    }

    fn finish(mut self) -> Result<i32, String> {
        // Reap whatever is left.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && self.nodes.iter().any(|n| n.child.is_some()) {
            self.poll_children();
            thread::sleep(Duration::from_millis(20));
        }
        for n in &mut self.nodes {
            if let Some(c) = n.child.as_mut() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        if let Some((p, n)) = self.router.stop_recording() {
            println!("[psl] recording saved: {} ({} frames)", p.display(), n);
        }
        let ok = self
            .nodes
            .iter()
            .filter(|n| n.state == NState::Stopped)
            .count();
        let failed = self.nodes.len() - ok;
        println!(
            "[psl] stopped: {} ok, {} failed; {} frames routed",
            ok,
            failed,
            self.router.forwarded.load(Ordering::Relaxed)
        );
        self.session.cleanup();
        Ok(self.exit_code)
    }
}

fn describe_exit(code: Option<i32>) -> String {
    match code {
        Some(c) => format!("exit code {}", c),
        None => "a signal".to_string(),
    }
}

/// Load the session's recorded frames count etc. Not used yet; kept for `psl rec` in Phase 2.
#[allow(dead_code)]
fn _unused(_f: &Frame) {}
