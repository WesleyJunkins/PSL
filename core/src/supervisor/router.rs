//! The routing table, stream header cache and recorder (docs/01 §Routing, docs/03 §Recording file).

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::json::Json;
use crate::model::{Graph, OnFull};
use crate::protocol::frame::FrameType;
use crate::protocol::Frame;
use crate::supervisor::queue::DropQueue;

#[derive(Clone, Debug)]
pub struct Dest {
    /// Destination client key: a node id, or a tool's generated id.
    pub client: String,
    pub port: u16,
    pub source: u16,
}

#[derive(Default)]
struct Inner {
    dests: HashMap<String, Vec<Dest>>,
    queues: HashMap<String, Arc<DropQueue>>,
    headers: HashMap<String, Frame>,
}

pub struct Router {
    inner: RwLock<Inner>,
    recorder: Mutex<Option<Recorder>>,
    pub forwarded: AtomicU64,
}

impl Router {
    pub fn from_graph(graph: &Graph) -> Router {
        let mut dests: HashMap<String, Vec<Dest>> = HashMap::new();
        for n in &graph.nodes {
            for inp in &n.inputs {
                for (si, stream) in inp.sources.iter().enumerate() {
                    dests.entry(stream.clone()).or_default().push(Dest {
                        client: n.id.clone(),
                        port: inp.index,
                        source: si as u16,
                    });
                }
            }
        }
        Router {
            inner: RwLock::new(Inner {
                dests,
                queues: HashMap::new(),
                headers: HashMap::new(),
            }),
            recorder: Mutex::new(None),
            forwarded: AtomicU64::new(0),
        }
    }

    pub fn set_queue(&self, client: &str, q: Arc<DropQueue>) {
        self.inner
            .write()
            .unwrap()
            .queues
            .insert(client.to_string(), q);
    }

    /// The queue for a client, created on first use so frames can be queued before its
    /// data connection is up. A closed (removed) queue is replaced by a fresh one.
    pub fn ensure_queue(&self, client: &str, cap: usize, on_full: OnFull) -> Arc<DropQueue> {
        let mut g = self.inner.write().unwrap();
        g.queues
            .entry(client.to_string())
            .or_insert_with(|| Arc::new(DropQueue::new(cap, on_full)))
            .clone()
    }

    /// Subscribe a client to a stream atomically: the cached header (if any) is queued
    /// first, then live frames follow, so a subscriber never sees data before its header.
    pub fn subscribe(&self, stream: &str, client: &str, port: u16) {
        let mut g = self.inner.write().unwrap();
        if let (Some(h), Some(q)) = (g.headers.get(stream), g.queues.get(client)) {
            let mut f = h.clone();
            f.header.port = port;
            f.header.source = 0;
            q.push(f);
        }
        g.dests.entry(stream.to_string()).or_default().push(Dest {
            client: client.to_string(),
            port,
            source: 0,
        });
    }

    pub fn remove_client(&self, client: &str) {
        let mut g = self.inner.write().unwrap();
        if let Some(q) = g.queues.remove(client) {
            q.close();
        }
        for v in g.dests.values_mut() {
            v.retain(|d| d.client != client);
        }
    }

    pub fn queue(&self, client: &str) -> Option<Arc<DropQueue>> {
        self.inner.read().unwrap().queues.get(client).cloned()
    }

    /// Subscribe a tool; returns the port index it will see frames on.
    pub fn add_dest(&self, stream: &str, client: &str, port: u16) {
        let mut g = self.inner.write().unwrap();
        g.dests.entry(stream.to_string()).or_default().push(Dest {
            client: client.to_string(),
            port,
            source: 0,
        });
    }

    pub fn remove_dest(&self, stream: &str, client: &str) {
        let mut g = self.inner.write().unwrap();
        if let Some(v) = g.dests.get_mut(stream) {
            v.retain(|d| d.client != client);
        }
    }

    pub fn cached_header(&self, stream: &str) -> Option<Frame> {
        self.inner.read().unwrap().headers.get(stream).cloned()
    }

    pub fn headers(&self) -> Vec<(String, Json)> {
        let g = self.inner.read().unwrap();
        let mut out: Vec<(String, Json)> = g
            .headers
            .iter()
            .filter_map(|(k, f)| {
                crate::protocol::frame::decode_stream_header(&f.payload).map(|j| (k.clone(), j))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Deliver a frame published on `stream` to every destination, rewriting `port` and `source`.
    pub fn route(&self, stream: &str, frame: Frame) {
        if frame.header.frame_type() == Some(FrameType::StreamHeader) {
            self.inner
                .write()
                .unwrap()
                .headers
                .insert(stream.to_string(), frame.clone());
        }
        if let Some(rec) = self.recorder.lock().unwrap().as_mut() {
            rec.write(stream, &frame);
        }
        let g = self.inner.read().unwrap();
        if let Some(dests) = g.dests.get(stream) {
            for d in dests {
                if let Some(q) = g.queues.get(&d.client) {
                    let mut f = frame.clone();
                    f.header.port = d.port;
                    f.header.source = d.source;
                    q.push(f);
                    self.forwarded.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Deliver the cached header of `stream` (if any) to one client directly.
    pub fn send_cached_header(&self, stream: &str, client: &str, port: u16, source: u16) {
        let g = self.inner.read().unwrap();
        if let (Some(h), Some(q)) = (g.headers.get(stream), g.queues.get(client)) {
            let mut f = h.clone();
            f.header.port = port;
            f.header.source = source;
            q.push(f);
        }
    }

    /// Deliver cached headers for every input source of a node that just (re)connected.
    pub fn replay_headers_for(&self, graph: &Graph, node_id: &str) {
        if let Some(n) = graph.node(node_id) {
            for inp in &n.inputs {
                for (si, stream) in inp.sources.iter().enumerate() {
                    self.send_cached_header(stream, node_id, inp.index, si as u16);
                }
            }
        }
    }

    pub fn start_recording(
        &self,
        path: &Path,
        graph: &Graph,
        session: &str,
        wall_start_ns: i64,
        clock: Arc<crate::supervisor::session::Clock>,
    ) -> io::Result<()> {
        let rec = Recorder::create(path, graph, session, wall_start_ns, clock)?;
        // Write cached headers first so a late recording still starts with stream metadata.
        let mut rec = rec;
        for (stream, f) in self.inner.read().unwrap().headers.iter() {
            rec.write(stream, f);
        }
        *self.recorder.lock().unwrap() = Some(rec);
        Ok(())
    }

    pub fn stop_recording(&self) -> Option<(std::path::PathBuf, u64)> {
        let rec = self.recorder.lock().unwrap().take()?;
        Some(rec.finish())
    }

    pub fn is_recording(&self) -> bool {
        self.recorder.lock().unwrap().is_some()
    }
}

/// Writes a `.pslrec` file (docs/03 §Recording file).
pub struct Recorder {
    w: BufWriter<File>,
    path: std::path::PathBuf,
    streams: HashMap<String, u16>,
    clock: Arc<crate::supervisor::session::Clock>,
    frames: u64,
}

impl Recorder {
    pub fn create(
        path: &Path,
        graph: &Graph,
        session: &str,
        wall_start_ns: i64,
        clock: Arc<crate::supervisor::session::Clock>,
    ) -> io::Result<Recorder> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut streams = HashMap::new();
        let mut list = Vec::new();
        for n in &graph.nodes {
            for o in &n.outputs {
                let idx = streams.len() as u16;
                streams.insert(o.stream.clone(), idx);
                list.push(
                    Json::object()
                        .with("index", idx as i64)
                        .with("stream", o.stream.as_str())
                        .with("node", n.id.as_str())
                        .with("port", o.name.as_str())
                        .with("type", o.ty.as_str()),
                );
            }
        }
        let header = Json::object()
            .with("version", crate::RECORDING_VERSION)
            .with("session", session)
            .with("wall_start_ns", wall_start_ns)
            .with("pipeline", graph.file.as_str())
            .with("graph", graph.to_json())
            .with("streams", list)
            .to_string();
        let mut w = BufWriter::new(File::create(path)?);
        w.write_all(b"PSLREC\0\x01")?;
        w.write_all(&(header.len() as u32).to_le_bytes())?;
        w.write_all(header.as_bytes())?;
        Ok(Recorder {
            w,
            path: path.to_path_buf(),
            streams,
            clock,
            frames: 0,
        })
    }

    pub fn write(&mut self, stream: &str, frame: &Frame) {
        let idx = match self.streams.get(stream) {
            Some(i) => *i,
            None => return,
        };
        let mut f = frame.clone();
        f.header.source = 0;
        let _ = self.w.write_all(&self.clock.now_ns().to_le_bytes());
        let _ = self.w.write_all(&idx.to_le_bytes());
        let _ = self.w.write_all(&0u16.to_le_bytes());
        let _ = self.w.write_all(&f.to_bytes());
        self.frames += 1;
    }

    pub fn finish(mut self) -> (std::path::PathBuf, u64) {
        let _ = self.w.flush();
        (self.path, self.frames)
    }
}
