//! End-to-end: the binary scaffolds and checks a workspace; the library runs a
//! real pipeline of Python nodes through the supervisor and a tool client talks to it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use psl::json::Json;
use psl::supervisor::tool::ToolClient;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn temp_ws(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("psl-e2e-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// The development interpreter with numpy, if the repo's `.venv` exists.
fn dev_python() -> Option<PathBuf> {
    let p = repo_root().join(".venv").join("bin").join("python");
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

#[test]
fn binary_init_check_graph() {
    let root = temp_ws("cli");
    let bin = env!("CARGO_BIN_EXE_psl");
    let out = Command::new(bin)
        .args(["init", "--no-venv", "--name", "t"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(root.join("workspace.psl").is_file());
    assert!(root
        .join(".psl/packages/psl/synthetic/0.1.0/node.psl")
        .is_file());
    assert!(root.join(".psl/sdk/psl/__init__.py").is_file());

    let out = Command::new(bin)
        .args(["check", "--workspace"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("check: ok"));

    let out = Command::new(bin)
        .args(["check", "--json", "--workspace"])
        .arg(&root)
        .output()
        .unwrap();
    let j = Json::parse(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(j.get("ok"), Some(&Json::Bool(true)));
    assert_eq!(
        j.get("graph")
            .unwrap()
            .get("nodes")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        3
    );

    let out = Command::new(bin)
        .args(["graph", "--dot", "--workspace"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("digraph"));

    // A broken pipeline exits 1 with a diagnostic.
    fs::write(
        root.join("pipelines/bad.psl"),
        "pipeline \"b\" {}\nnode a : input \"psl/synthetic\" { nope = 1 }\n",
    )
    .unwrap();
    let out = Command::new(bin)
        .args(["check", "pipelines/bad.psl", "--workspace"])
        .arg(&root)
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("E0201"));

    let out = Command::new(bin)
        .args(["new", "node", "gain", "--kind", "module", "--workspace"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(root.join("nodes/gain/node.psl").is_file());

    // No session: live commands exit 4.
    let out = Command::new(bin)
        .args(["ps", "--workspace"])
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn pipeline_runs_end_to_end() {
    let python = match dev_python() {
        Some(p) => p,
        None => {
            eprintln!("skipping: no .venv with numpy at the repository root");
            return;
        }
    };
    let root = temp_ws("run");
    fs::create_dir_all(root.join("pipelines")).unwrap();
    fs::write(
        root.join("workspace.psl"),
        format!("workspace \"e2e\" {{ python = \"{}\" }}\nrequire \"psl/synthetic\" {{ version = \"^0.1\" }}\n", python.display()),
    )
    .unwrap();
    for pkg in ["synthetic", "scale", "csv-writer", "print"] {
        psl::cli::init_support::install_embedded(&root, pkg)
            .map_err(|e| e.message)
            .unwrap();
    }
    fs::write(
        root.join("pipelines/main.psl"),
        r#"pipeline "e2e" {}
node ecg : input "psl/synthetic" { waveform = "sine", srate = 100, chunk_ms = 20, channels = ["A", "B"] }
node mv : module "psl/scale" { gain = 2.0 }
node csv : output "psl/csv-writer" { path = "out/x.csv" }
node log : output "psl/print" { every = 10 }
map ecg.out -> mv.in
map mv.out -> csv.in, log.in
"#,
    )
    .unwrap();
    let r = psl::check::check_pipeline(&root, &root.join("pipelines/main.psl"));
    assert!(!r.diags.has_errors(), "{}", r.diags.render(&r.sm));
    let graph = r.graph.unwrap();
    let opts = psl::supervisor::RunOptions {
        record: Some(root.join("out/rec.pslrec")),
        echo_level: psl::supervisor::LogLevel::Warn,
        python: python.display().to_string(),
        sdk_path: repo_root().join("sdk").join("python"),
    };
    let root2 = root.clone();
    let handle = thread::spawn(move || psl::supervisor::run(&root2, graph, opts));

    // Wait for the session and for every node to be running.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut client = loop {
        assert!(Instant::now() < deadline, "session never came up");
        if let Ok(Some(c)) = ToolClient::connect(&root) {
            break c;
        }
        thread::sleep(Duration::from_millis(100));
    };
    loop {
        assert!(Instant::now() < deadline, "nodes never reached running");
        let nodes = client.query("nodes").unwrap();
        let states: Vec<String> = nodes
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.str_field("state").unwrap().to_string())
            .collect();
        if states.iter().all(|s| s == "running") {
            break;
        }
        assert!(
            !states.iter().any(|s| s == "failed" || s == "dead"),
            "a node failed: {:?}",
            states
        );
        thread::sleep(Duration::from_millis(100));
    }

    // Subscribe to the scaled stream: header first, then signal chunks with increasing seq and gain applied.
    let port = client.subscribe("mv.out").unwrap();
    let first = client.next_frame().unwrap().unwrap();
    assert_eq!(
        first.header.frame_type(),
        Some(psl::protocol::FrameType::StreamHeader)
    );
    assert_eq!(first.header.port, port);
    let header = psl::protocol::frame::decode_stream_header(&first.payload).unwrap();
    assert_eq!(header.get("srate").and_then(Json::as_f64), Some(100.0));
    assert_eq!(header.get("channels").unwrap().as_array().unwrap().len(), 2);
    let mut last_seq = None;
    let mut max_abs: f64 = 0.0;
    for _ in 0..5 {
        let f = client.next_frame().unwrap().unwrap();
        assert_eq!(
            f.header.frame_type(),
            Some(psl::protocol::FrameType::Signal)
        );
        if let Some(s) = last_seq {
            assert_eq!(f.header.seq, s + 1, "frames must be consecutive");
        }
        last_seq = Some(f.header.seq);
        let info = psl::protocol::frame::SignalInfo::decode(&f.payload).unwrap();
        assert_eq!(info.n_channels, 2);
        assert_eq!(info.n_samples, 2);
        for v in info.first_values(&f.payload, 2) {
            max_abs = max_abs.max(v.abs());
        }
    }
    assert!(
        max_abs > 1.0 && max_abs <= 2.0001,
        "gain 2 on a unit sine should give values above 1, got {}",
        max_abs
    );

    // Live parameter change goes through to the node.
    let ack = client.set_param("mv", "gain", Json::Float(4.0)).unwrap();
    assert_eq!(ack.get("ok"), Some(&Json::Bool(true)), "{:?}", ack);
    let ack = client
        .set_param("csv", "path", Json::Str("x".into()))
        .unwrap();
    assert_eq!(ack.get("ok"), Some(&Json::Bool(false)));

    let streams = client.query("streams").unwrap();
    assert!(streams
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.str_field("stream") == Some("ecg.out") && !s.get("header").unwrap().is_null()));

    // Keep running until the scaled stream has carried 20 chunks. A loaded CI machine
    // produces chunks slower than real time, so count frames instead of sleeping.
    client
        .set_frame_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut signal_frames = 5;
    while signal_frames < 20 {
        let f = client
            .next_frame()
            .expect("frame within 30 s")
            .expect("stream ended early");
        if f.header.frame_type() == Some(psl::protocol::FrameType::Signal) {
            signal_frames += 1;
        }
    }
    client.stop_pipeline().unwrap();
    let code = handle.join().unwrap().unwrap();
    assert_eq!(code, 0);

    // The CSV has a header row and data rows with two channels; the recording exists.
    let csv = fs::read_to_string(root.join("out/x.csv")).unwrap();
    let mut lines = csv.lines();
    assert_eq!(lines.next().unwrap(), "t_ns,A,B");
    let rows = lines.count();
    assert!(rows > 20, "expected more than 20 rows, got {}", rows);
    let rec = fs::read(root.join("out/rec.pslrec")).unwrap();
    assert_eq!(&rec[..6], b"PSLREC");
    assert!(
        !root.join(".psl/current").exists(),
        "current marker must be removed after a run"
    );
    let _ = fs::remove_dir_all(&root);
}
