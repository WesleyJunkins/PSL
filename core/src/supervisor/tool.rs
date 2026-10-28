//! A client for tools (`psl ps`, `psl echo`, `psl stop`, later PhysioLab).

use std::io;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::json::Json;
use crate::protocol::control::{msg, read_message, write_message};
use crate::protocol::frame::{read_frame, write_frame};
use crate::protocol::{Conn, Frame};
use crate::supervisor::session::{self, SessionInfo};

pub struct ToolClient {
    pub info: SessionInfo,
    writer: Arc<Mutex<Conn>>,
    inbox: Receiver<Json>,
    data: Option<Conn>,
    pub node_id: String,
}

impl ToolClient {
    /// Connect to the workspace's current session. `Ok(None)` when none is running.
    pub fn connect(root: &Path) -> Result<Option<ToolClient>, String> {
        let info = match session::current(root)? {
            Some(i) => i,
            None => return Ok(None),
        };
        let conn = match info.control.connect() {
            Ok(c) => c,
            Err(_) => {
                session::clear_current(root);
                return Ok(None);
            }
        };
        let mut reader = conn.try_clone().map_err(|e| e.to_string())?;
        let writer = Arc::new(Mutex::new(conn));
        let (tx, rx) = channel::<Json>();
        let w2 = writer.clone();
        thread::spawn(move || loop {
            match read_message(&mut reader) {
                Ok(Some(m)) => {
                    if m.str_field("type") == Some("ping") {
                        let pong = msg("pong").with("n", m.get("n").cloned().unwrap_or(Json::Null));
                        if let Ok(mut w) = w2.lock() {
                            let _ = write_message(&mut *w, &pong);
                        }
                        continue;
                    }
                    if tx.send(m).is_err() {
                        return;
                    }
                }
                _ => return,
            }
        });
        let mut client = ToolClient {
            info,
            writer,
            inbox: rx,
            data: None,
            node_id: String::new(),
        };
        client.send(
            &msg("hello")
                .with("role", "tool")
                .with("protocol", crate::PROTOCOL_VERSION as i64)
                .with("pid", std::process::id() as i64)
                .with("sdk", format!("psl-cli/{}", crate::VERSION)),
        );
        let cfg = client.wait_for(&["config", "reject"], Duration::from_secs(5))?;
        if cfg.str_field("type") == Some("reject") {
            return Err(format!(
                "supervisor rejected the connection: {}",
                cfg.str_field("reason").unwrap_or("")
            ));
        }
        client.node_id = cfg.str_field("node_id").unwrap_or("").to_string();
        Ok(Some(client))
    }

    pub fn send(&self, m: &Json) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = write_message(&mut *w, m);
        }
    }

    /// Wait for the next control message whose type is one of `types`.
    pub fn wait_for(&mut self, types: &[&str], timeout: Duration) -> Result<Json, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.inbox.recv_timeout(left) {
                Ok(m) => {
                    let ty = m.str_field("type").unwrap_or("");
                    if types.contains(&ty) || ty == "error" {
                        if ty == "error" && !types.contains(&"error") {
                            return Err(m
                                .str_field("msg")
                                .unwrap_or("supervisor error")
                                .to_string());
                        }
                        return Ok(m);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err("timed out waiting for the supervisor".into())
                }
                Err(RecvTimeoutError::Disconnected) => return Err("the session ended".into()),
            }
        }
    }

    pub fn query(&mut self, what: &str) -> Result<Json, String> {
        self.send(&msg("query").with("what", what));
        let r = self.wait_for(&["result"], Duration::from_secs(5))?;
        Ok(r.get("data").cloned().unwrap_or(Json::Null))
    }

    pub fn set_param(&mut self, node: &str, name: &str, value: Json) -> Result<Json, String> {
        self.send(
            &msg("set_param")
                .with("node_id", node)
                .with("name", name)
                .with("value", value),
        );
        self.wait_for(&["param_ack"], Duration::from_secs(5))
    }

    pub fn stop_pipeline(&mut self) -> Result<(), String> {
        self.send(&msg("stop_pipeline"));
        self.wait_for(&["result"], Duration::from_secs(5))
            .map(|_| ())
    }

    pub fn record_start(&mut self, path: Option<&str>) -> Result<Json, String> {
        let mut m = msg("record_start");
        if let Some(p) = path {
            m.set("path", p);
        }
        self.send(&m);
        self.wait_for(&["result"], Duration::from_secs(5))
    }

    pub fn record_stop(&mut self) -> Result<Json, String> {
        self.send(&msg("record_stop"));
        self.wait_for(&["result"], Duration::from_secs(5))
    }

    fn ensure_data(&mut self) -> Result<(), String> {
        if self.data.is_some() {
            return Ok(());
        }
        let mut conn = self
            .info
            .data
            .connect()
            .map_err(|e| format!("cannot open the data socket: {}", e))?;
        write_frame(&mut conn, &Frame::identify(&self.node_id, &self.info.id))
            .map_err(|e| e.to_string())?;
        self.data = Some(conn);
        Ok(())
    }

    /// Subscribe to a stream; returns the port index its frames arrive with.
    pub fn subscribe(&mut self, stream: &str) -> Result<u16, String> {
        self.ensure_data()?;
        self.send(&msg("subscribe").with("stream", stream));
        let r = self.wait_for(&["subscribed"], Duration::from_secs(5))?;
        Ok(r.i64_field("index").unwrap_or(0) as u16)
    }

    pub fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        match self.data.as_mut() {
            Some(c) => read_frame(c),
            None => Ok(None),
        }
    }
}
