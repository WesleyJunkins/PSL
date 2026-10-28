//! Session directories, socket paths and `session.json` (docs/01 §Session).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::json::Json;
use crate::model::Graph;
use crate::protocol::{Address, Listener};
use crate::util::{format_utc_compact, random_suffix, wall_now_ns};

/// The monotonic reference for session time.
pub struct Clock {
    pub start: Instant,
}

impl Clock {
    pub fn now_ns(&self) -> i64 {
        self.start.elapsed().as_nanos() as i64
    }
}

pub struct Session {
    pub id: String,
    /// `<workspace>/.psl/sessions/<id>`
    pub dir: PathBuf,
    /// Short path holding the socket files (Unix socket paths are length-limited).
    pub sock_dir: PathBuf,
    pub control: Option<Listener>,
    pub data: Option<Listener>,
    pub control_addr: Address,
    pub data_addr: Address,
    pub wall_start_ns: i64,
    pub clock: std::sync::Arc<Clock>,
    root: PathBuf,
}

pub fn sessions_dir(root: &Path) -> PathBuf {
    root.join(".psl").join("sessions")
}

pub fn current_file(root: &Path) -> PathBuf {
    root.join(".psl").join("current")
}

fn socket_base() -> PathBuf {
    if cfg!(unix) {
        PathBuf::from("/tmp/psl")
    } else {
        std::env::temp_dir().join("psl")
    }
}

impl Session {
    pub fn create(root: &Path, graph: &Graph, pipeline_file: &str) -> io::Result<Session> {
        let wall_start_ns = wall_now_ns();
        let id = format!("{}-{}", format_utc_compact(wall_start_ns), random_suffix());
        let dir = sessions_dir(root).join(&id);
        fs::create_dir_all(dir.join("logs"))?;
        remove_stale_socket_dirs();
        let sock_dir = socket_base().join(&id);
        fs::create_dir_all(&sock_dir)?;
        let control = Listener::bind_local(&sock_dir.join("control.sock"))?;
        let data = Listener::bind_local(&sock_dir.join("data.sock"))?;
        let control_addr = control.address();
        let data_addr = data.address();
        let session = Session {
            id,
            dir,
            sock_dir,
            control_addr,
            data_addr,
            control: Some(control),
            data: Some(data),
            wall_start_ns,
            clock: std::sync::Arc::new(Clock {
                start: Instant::now(),
            }),
            root: root.to_path_buf(),
        };
        let j = Json::object()
            .with("id", session.id.as_str())
            .with("pipeline", pipeline_file)
            .with("wall_start_ns", wall_start_ns)
            .with("pid", std::process::id() as i64)
            .with("control", session.control_addr.to_string())
            .with("data", session.data_addr.to_string())
            .with("graph", graph.to_json());
        fs::write(session.dir.join("session.json"), j.to_pretty())?;
        fs::write(current_file(root), &session.id)?;
        Ok(session)
    }

    pub fn log_path(&self, node_id: &str) -> PathBuf {
        self.dir.join("logs").join(format!("{}.log", node_id))
    }

    /// Remove the `current` marker and the socket directory.
    pub fn cleanup(&mut self) {
        self.control = None;
        self.data = None;
        let _ = fs::remove_dir_all(&self.sock_dir);
        if let Ok(cur) = fs::read_to_string(current_file(&self.root)) {
            if cur.trim() == self.id {
                let _ = fs::remove_file(current_file(&self.root));
            }
        }
    }
}

/// What a tool needs to know about a running session.
#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub id: String,
    pub dir: PathBuf,
    pub control: Address,
    pub data: Address,
    pub json: Json,
}

/// Find the session recorded in `.psl/current`. Returns `Ok(None)` when there is none.
pub fn current(root: &Path) -> Result<Option<SessionInfo>, String> {
    let cur = match fs::read_to_string(current_file(root)) {
        Ok(c) => c.trim().to_string(),
        Err(_) => return Ok(None),
    };
    if cur.is_empty() {
        return Ok(None);
    }
    let dir = sessions_dir(root).join(&cur);
    let text = fs::read_to_string(dir.join("session.json"))
        .map_err(|e| format!("cannot read {}: {}", dir.join("session.json").display(), e))?;
    let json = Json::parse(&text).map_err(|e| format!("session.json is corrupt: {}", e))?;
    let control = Address::parse(json.str_field("control").unwrap_or(""));
    let data = Address::parse(json.str_field("data").unwrap_or(""));
    Ok(Some(SessionInfo {
        id: cur,
        dir,
        control,
        data,
        json,
    }))
}

/// Forget a session that is no longer running.
pub fn clear_current(root: &Path) {
    let _ = fs::remove_file(current_file(root));
}

/// Remove socket directories left behind by sessions that are no longer running
/// (a crashed supervisor, or a test that panicked). A directory is stale when its
/// control socket refuses connections.
fn remove_stale_socket_dirs() {
    let base = socket_base();
    let entries = match fs::read_dir(&base) {
        Ok(e) => e,
        Err(_) => return,
    };
    for e in entries.flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let sock = dir.join("control.sock");
        let alive = sock.exists() && Address::Unix(sock.clone()).connect().is_ok();
        if !alive {
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
