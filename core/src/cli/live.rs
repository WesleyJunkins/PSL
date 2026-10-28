//! Commands that talk to a running session: stop, ps, streams, echo, set, record, logs.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;

use super::{workspace_root, Args, CliError, CliResult};
use crate::json::Json;
use crate::protocol::frame::{decode_stream_header, EventInfo, FrameType, SignalInfo};
use crate::supervisor::tool::ToolClient;

fn connect(args: &Args) -> Result<(PathBuf, ToolClient), CliError> {
    let root = workspace_root(args)?;
    match ToolClient::connect(&root)? {
        Some(c) => Ok((root, c)),
        None => Err(CliError::no_session()),
    }
}

pub fn stop(args: &Args) -> CliResult {
    let (_, mut c) = connect(args)?;
    c.stop_pipeline()?;
    println!("stop requested for session {}", c.info.id);
    Ok(0)
}

pub fn ps(args: &Args) -> CliResult {
    let (_, mut c) = connect(args)?;
    let watch = args.flag("--watch") || args.flag("-w");
    loop {
        let nodes = c.query("nodes")?;
        if args.flag("--json") {
            println!("{}", nodes.to_pretty());
        } else {
            if watch {
                print!("\x1b[2J\x1b[H");
            }
            println!("session {}", c.info.id);
            print_nodes_table(&nodes);
        }
        if !watch {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(0)
}

fn print_nodes_table(nodes: &Json) {
    let rows: Vec<Vec<String>> = nodes
        .as_array()
        .map(|a| {
            a.iter()
                .map(|n| {
                    let stats = n.get("stats").cloned().unwrap_or(Json::Null);
                    let lat = stats
                        .get("latency_ns")
                        .and_then(|l| l.as_object())
                        .and_then(|o| {
                            o.iter()
                                .map(|(_, v)| v.as_f64().unwrap_or(0.0))
                                .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))))
                        })
                        .map(|ns| format!("{:.1}", ns / 1e6))
                        .unwrap_or_else(|| "-".into());
                    let up = n.get("uptime_s").and_then(Json::as_f64).unwrap_or(0.0) as u64;
                    vec![
                        n.str_field("id").unwrap_or("").to_string(),
                        n.str_field("kind").unwrap_or("").to_string(),
                        n.str_field("state").unwrap_or("").to_string(),
                        n.i64_field("pid").unwrap_or(0).to_string(),
                        format!("{:02}:{:02}", up / 60, up % 60),
                        n.i64_field("frames_to_node").unwrap_or(0).to_string(),
                        n.i64_field("frames_from_node").unwrap_or(0).to_string(),
                        n.i64_field("dropped").unwrap_or(0).to_string(),
                        lat,
                        n.i64_field("restarts").unwrap_or(0).to_string(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    let header = [
        "NODE", "KIND", "STATE", "PID", "UP", "IN", "OUT", "DROPS", "LAT(ms)", "RESTARTS",
    ];
    print_table(&header, &rows);
}

pub fn print_table(header: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = header.iter().map(|h| h.len()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(c.chars().count());
            }
        }
    }
    let line = |cells: Vec<&str>| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{:<w$}", c, w = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    println!("{}", line(header.to_vec()));
    for r in rows {
        println!("{}", line(r.iter().map(String::as_str).collect()));
    }
}

pub fn streams(args: &Args) -> CliResult {
    let (_, mut c) = connect(args)?;
    let streams = c.query("streams")?;
    if args.flag("--json") {
        println!("{}", streams.to_pretty());
        return Ok(0);
    }
    let rows: Vec<Vec<String>> = streams
        .as_array()
        .map(|a| {
            a.iter()
                .map(|s| {
                    let h = s.get("header").filter(|h| !h.is_null());
                    let (srate, ch, labels) = match h {
                        Some(h) if h.str_field("type") == Some("signal") => {
                            let chans = h
                                .get("channels")
                                .and_then(Json::as_array)
                                .cloned()
                                .unwrap_or_default();
                            let labels: Vec<String> = chans
                                .iter()
                                .filter_map(|c| c.str_field("label").map(String::from))
                                .collect();
                            (
                                h.get("srate")
                                    .and_then(Json::as_f64)
                                    .map(|v| format!("{}", v))
                                    .unwrap_or("-".into()),
                                chans.len().to_string(),
                                truncate(&labels.join(","), 40),
                            )
                        }
                        Some(h) => (
                            "-".into(),
                            "-".into(),
                            truncate(
                                &h.get("labels").map(|l| l.to_string()).unwrap_or_default(),
                                40,
                            ),
                        ),
                        None => ("-".into(), "-".into(), "(no header yet)".into()),
                    };
                    vec![
                        s.str_field("stream").unwrap_or("").to_string(),
                        s.str_field("type").unwrap_or("").to_string(),
                        srate,
                        ch,
                        labels,
                        if s.get("ended").and_then(Json::as_bool).unwrap_or(false) {
                            "ended".into()
                        } else {
                            String::new()
                        },
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    print_table(&["STREAM", "TYPE", "SRATE", "CH", "LABELS", ""], &rows);
    Ok(0)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

pub fn echo(args: &Args) -> CliResult {
    let stream = args
        .pos(0)
        .ok_or_else(|| CliError::usage("psl echo <node.port> [--raw]"))?
        .to_string();
    if !stream.contains('.') {
        return Err(CliError::usage(
            "psl echo <node.port>: name the stream as node.port",
        ));
    }
    let (_, mut c) = connect(args)?;
    c.subscribe(&stream)?;
    let raw = args.flag("--raw");
    let json = args.flag("--json");
    eprintln!("echo {} (Ctrl-C to stop)", stream);
    while let Some(f) = c
        .next_frame()
        .map_err(|e| CliError::failed(e.to_string()))?
    {
        match f.header.frame_type() {
            Some(FrameType::StreamHeader) => {
                let h = decode_stream_header(&f.payload).unwrap_or(Json::Null);
                if json {
                    println!(
                        "{}",
                        Json::object()
                            .with("frame", "header")
                            .with("seq", f.header.seq as i64)
                            .with("header", h)
                    );
                } else {
                    println!("header seq={} {}", f.header.seq, h);
                }
            }
            Some(FrameType::Signal) => {
                let info = match SignalInfo::decode(&f.payload) {
                    Some(i) => i,
                    None => continue,
                };
                if json {
                    println!(
                        "{}",
                        Json::object()
                            .with("frame", "signal")
                            .with("seq", f.header.seq as i64)
                            .with("t0_ns", info.t0)
                            .with("n_samples", info.n_samples as i64)
                            .with("n_channels", info.n_channels as i64)
                            .with("dtype", info.dtype.name())
                            .with(
                                "first",
                                info.first_values(&f.payload, 4)
                                    .into_iter()
                                    .map(Json::Float)
                                    .collect::<Vec<_>>()
                            )
                    );
                } else if raw {
                    let stride = info.n_channels as usize * info.dtype.size();
                    let data = &f.payload[crate::protocol::frame::SIGNAL_PREFIX_LEN..];
                    for i in 0..info.n_samples as usize {
                        let row: Vec<String> = (0..info.n_channels as usize)
                            .map(|ch| {
                                let o = i * stride + ch * info.dtype.size();
                                let v = match info.dtype {
                                    crate::protocol::frame::Dtype::F32 => {
                                        f32::from_le_bytes(data[o..o + 4].try_into().unwrap())
                                            as f64
                                    }
                                    crate::protocol::frame::Dtype::F64 => {
                                        f64::from_le_bytes(data[o..o + 8].try_into().unwrap())
                                    }
                                    crate::protocol::frame::Dtype::I16 => {
                                        i16::from_le_bytes(data[o..o + 2].try_into().unwrap())
                                            as f64
                                    }
                                    crate::protocol::frame::Dtype::I32 => {
                                        i32::from_le_bytes(data[o..o + 4].try_into().unwrap())
                                            as f64
                                    }
                                };
                                format!("{:.6}", v)
                            })
                            .collect();
                        println!("{}\t{}", info.t0, row.join("\t"));
                    }
                } else {
                    let first: Vec<String> = info
                        .first_values(&f.payload, 4)
                        .iter()
                        .map(|v| format!("{:.4}", v))
                        .collect();
                    println!(
                        "signal seq={} t0={:.4}s shape={}x{} {} first=[{}]",
                        f.header.seq,
                        info.t0 as f64 / 1e9,
                        info.n_samples,
                        info.n_channels,
                        info.dtype.name(),
                        first.join(", ")
                    );
                }
            }
            Some(FrameType::Event) => {
                let e = match EventInfo::decode(&f.payload) {
                    Some(e) => e,
                    None => continue,
                };
                if json {
                    println!(
                        "{}",
                        Json::object()
                            .with("frame", "event")
                            .with("seq", f.header.seq as i64)
                            .with("t_ns", e.t)
                            .with("body", e.body)
                    );
                } else {
                    println!(
                        "event seq={} t={:.4}s {}",
                        f.header.seq,
                        e.t as f64 / 1e9,
                        e.body
                    );
                }
            }
            Some(FrameType::EndStream) => println!("end seq={}", f.header.seq),
            _ => {}
        }
    }
    eprintln!("stream closed");
    Ok(0)
}

pub fn set(args: &Args) -> CliResult {
    let target = args
        .pos(0)
        .ok_or_else(|| CliError::usage("psl set <node.param> <value>"))?;
    let raw = args
        .pos(1)
        .ok_or_else(|| CliError::usage("psl set <node.param> <value>"))?;
    let (node, name) = target
        .split_once('.')
        .ok_or_else(|| CliError::usage("psl set <node.param> <value>"))?;
    let (_, mut c) = connect(args)?;
    let ack = c.set_param(node, name, super::run::parse_value(raw))?;
    if ack.get("ok").and_then(Json::as_bool).unwrap_or(false) {
        println!("{}.{} = {}", node, name, raw);
        Ok(0)
    } else {
        Err(CliError::failed(
            ack.str_field("reason").unwrap_or("rejected").to_string(),
        ))
    }
}

pub fn record(args: &Args) -> CliResult {
    let (_, mut c) = connect(args)?;
    match args.pos(0) {
        Some("start") => {
            let r = c.record_start(args.pos(1))?;
            println!(
                "recording to {}",
                r.get("data")
                    .and_then(|d| d.str_field("path"))
                    .unwrap_or("?")
            );
            Ok(0)
        }
        Some("stop") => {
            let r = c.record_stop()?;
            match r.get("data") {
                Some(d) if !d.is_null() => println!(
                    "recording stopped: {} ({} frames)",
                    d.str_field("path").unwrap_or("?"),
                    d.i64_field("frames").unwrap_or(0)
                ),
                _ => println!("no recording was running"),
            }
            Ok(0)
        }
        _ => Err(CliError::usage("psl record start [file] | psl record stop")),
    }
}

pub fn logs(args: &Args) -> CliResult {
    let root = workspace_root(args)?;
    let follow = args.flag("-f") || args.flag("--follow");
    // The current session, else the newest session directory.
    let dir = match crate::supervisor::session::current(&root)? {
        Some(s) => s.dir,
        None => {
            let sessions = crate::supervisor::session::sessions_dir(&root);
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(&sessions)
                .map(|r| {
                    r.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_dir())
                        .collect()
                })
                .unwrap_or_default();
            dirs.sort();
            dirs.pop()
                .ok_or_else(|| CliError::failed("no sessions have run in this workspace yet"))?
        }
    };
    let logs = dir.join("logs");
    let mut files: Vec<PathBuf> = match args.pos(0) {
        Some(node) => vec![logs.join(format!("{}.log", node))],
        None => std::fs::read_dir(&logs)
            .map(|r| r.flatten().map(|e| e.path()).collect())
            .unwrap_or_default(),
    };
    files.sort();
    if files.is_empty() {
        return Err(CliError::failed(format!("no logs in {}", logs.display())));
    }
    let single = files.len() == 1;
    let mut offsets: Vec<u64> = vec![0; files.len()];
    loop {
        for (i, f) in files.iter().enumerate() {
            let name = f
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if let Ok(mut fh) = std::fs::File::open(f) {
                let _ = fh.seek(SeekFrom::Start(offsets[i]));
                let mut buf = String::new();
                if fh.read_to_string(&mut buf).is_ok() {
                    offsets[i] += buf.len() as u64;
                    for line in buf.lines() {
                        if single {
                            println!("{}", line);
                        } else {
                            println!("[{}] {}", name, line);
                        }
                    }
                }
            }
        }
        if !follow {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(0)
}
