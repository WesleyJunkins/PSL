//! `psl init` and `psl new`.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{workspace_root, write_file, Args, CliError, CliResult};
use crate::embedded;
use crate::model::parse_manifest;
use crate::syntax::{parse_document, FileId};

pub fn init(args: &Args) -> CliResult {
    let dir = PathBuf::from(args.pos(0).unwrap_or("."));
    std::fs::create_dir_all(&dir)
        .map_err(|e| CliError::failed(format!("cannot create {}: {}", dir.display(), e)))?;
    let dir = dir
        .canonicalize()
        .map_err(|e| CliError::failed(e.to_string()))?;
    if dir.join("workspace.psl").is_file() {
        return refresh(&dir, args);
    }
    let name = args.value("--name").map(String::from).unwrap_or_else(|| {
        dir.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "workspace".into())
    });
    println!("creating workspace `{}` in {}", name, dir.display());

    // workspace.psl with a require per first-party package.
    let mut ws = format!("workspace \"{}\" {{\n  psl = \"0.1\"\n}}\n", name);
    let mut installed = Vec::new();
    for pkg in embedded::node_names() {
        let version = install_embedded(&dir, pkg)?;
        ws.push_str(&format!(
            "\nrequire \"psl/{}\" {{ version = \"^{}\" }}\n",
            pkg, version
        ));
        installed.push(format!("psl/{} {}", pkg, version));
    }
    write_file(&dir.join("workspace.psl"), ws.as_bytes())?;
    let mut kept = Vec::new();
    for (rel, contents) in [
        ("pipelines/main.psl", MAIN_PIPELINE.as_bytes()),
        (".gitignore", b".psl/\n__pycache__/\n*.pyc\nout/\n" as &[u8]),
    ] {
        let path = dir.join(rel);
        if path.exists() {
            kept.push(rel);
        } else {
            write_file(&path, contents)?;
        }
    }
    write_file(&dir.join("nodes").join(".gitkeep"), b"")?;
    install_sdk(&dir)?;
    write_file(&dir.join(".psl").join("version"), crate::VERSION.as_bytes())?;
    if !kept.is_empty() {
        println!("  kept existing {}", kept.join(", "));
    }
    println!("  workspace.psl, pipelines/main.psl, nodes/");
    println!("  packages: {}", installed.join(", "));
    println!(
        "  Python SDK: .psl/sdk/psl ({} files)",
        embedded::SDK_FILES.len()
    );

    if !args.flag("--no-venv") {
        match create_venv(&dir) {
            Ok(python) => println!("  Python env: {}", python),
            Err(e) => println!("  warning: {}\n  (nodes need numpy; install it for the interpreter `psl doctor` reports, or rerun without --no-venv)", e),
        }
    }
    let rel = display_rel(&dir);
    if rel == "." {
        println!("\nnext:\n  psl check\n  psl run");
    } else {
        println!("\nnext:\n  cd {}\n  psl check\n  psl run", rel);
    }
    Ok(0)
}

fn display_rel(dir: &Path) -> String {
    match std::env::current_dir() {
        Ok(cwd) if dir == cwd => ".".into(),
        Ok(cwd) => dir
            .strip_prefix(&cwd)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| dir.display().to_string()),
        Err(_) => dir.display().to_string(),
    }
}

/// Copy an embedded first-party package into `.psl/packages/psl/<name>/<version>/`. Returns the version.
pub fn install_embedded(root: &Path, pkg: &str) -> Result<String, CliError> {
    let manifest = embedded::node_manifest(pkg)
        .ok_or_else(|| CliError::failed(format!("embedded package `{}` has no manifest", pkg)))?;
    let doc = parse_document(FileId(0), manifest).map_err(|e| {
        CliError::failed(format!(
            "embedded manifest of `{}` does not parse: {}",
            pkg, e.message
        ))
    })?;
    let (m, d) = parse_manifest(&doc, FileId(0));
    let m = m.ok_or_else(|| {
        CliError::failed(format!(
            "embedded manifest of `{}` is invalid: {}",
            pkg,
            d.items
                .iter()
                .map(|x| x.message.clone())
                .collect::<Vec<_>>()
                .join("; ")
        ))
    })?;
    let dest = root
        .join(".psl")
        .join("packages")
        .join("psl")
        .join(pkg)
        .join(&m.version);
    for (name, rel, bytes) in embedded::NODE_FILES {
        if *name == pkg {
            write_file(&dest.join(rel), bytes)?;
        }
    }
    Ok(m.version)
}

/// Write the embedded Python SDK into `.psl/sdk/`.
pub fn install_sdk(root: &Path) -> Result<(), CliError> {
    let dest = root.join(".psl").join("sdk");
    for (rel, bytes) in embedded::SDK_FILES {
        write_file(&dest.join(rel), bytes)?;
    }
    Ok(())
}

/// Why nodes need Python 3.11+, and how to get it. Shown whenever no suitable interpreter is found.
pub const PYTHON_HELP: &str = "PSL nodes run on Python 3.11 or newer. Install it with `brew install python` \
or from https://www.python.org/downloads/, or install uv (https://docs.astral.sh/uv/) and `psl init` \
will fetch Python by itself.";

pub fn have_uv() -> bool {
    Command::new("uv")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Find a Python 3.11+ interpreter: `PSL_PYTHON`, then versioned names, then `python3`.
/// Returns the command and its version, or a message listing what was found.
pub fn find_python() -> Result<(String, String), String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(p) = std::env::var("PSL_PYTHON") {
        if !p.is_empty() {
            candidates.push(p);
        }
    }
    for c in [
        "python3.14",
        "python3.13",
        "python3.12",
        "python3.11",
        "python3",
        "python",
    ] {
        candidates.push(c.to_string());
    }
    let mut too_old = Vec::new();
    for c in candidates {
        let out = match Command::new(&c)
            .args(["-c", "import sys; print('%d.%d.%d' % sys.version_info[:3])"])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => continue,
        };
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let ok = crate::util::Version::parse(&v)
            .map(|v| (v.major, v.minor) >= (3, 11))
            .unwrap_or(false);
        if ok {
            return Ok((c, v));
        }
        too_old.push(format!("{} is {}", c, v));
    }
    if too_old.is_empty() {
        Err("no Python interpreter found".to_string())
    } else {
        Err(format!(
            "no Python 3.11 or newer found ({})",
            too_old.join(", ")
        ))
    }
}

/// Create `.psl/venv` with numpy: with uv when available (it downloads a Python if none is
/// new enough), else with the newest Python 3.11+ on PATH and pip.
fn create_venv(root: &Path) -> Result<String, String> {
    let venv = root.join(".psl").join("venv");
    let python = venv.join("bin").join("python");
    if have_uv() {
        println!("  creating Python env with uv (downloads Python if none is new enough) ...");
        run_quiet(
            Command::new("uv")
                .args(["venv", "-q", "--python", ">=3.11"])
                .arg(&venv),
        )?;
        run_quiet(
            Command::new("uv")
                .args(["pip", "install", "-q", "--python"])
                .arg(&python)
                .arg("numpy"),
        )?;
    } else {
        let (base, ver) = find_python().map_err(|e| format!("{}. {}", e, PYTHON_HELP))?;
        println!(
            "  creating Python env with {} ({}) and installing numpy (can take a minute) ...",
            base, ver
        );
        run_quiet(Command::new(&base).args(["-m", "venv"]).arg(&venv))?;
        run_quiet(Command::new(&python).args(["-m", "pip", "install", "-q", "numpy"]))?;
    }
    Ok(python.display().to_string())
}

fn run_quiet(cmd: &mut Command) -> Result<(), String> {
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run {:?}: {}", cmd.get_program(), e))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

const MAIN_PIPELINE: &str = r#"# Example pipeline: synthetic ECG, smoothed, printed to the log.
# Run it with `psl run`, inspect it with `psl ps` and `psl echo smooth.out`.

pipeline "main" {
  description = "Synthetic ECG through a moving average into the log"
}

node ecg : input "psl/synthetic" {
  waveform = "ecg"
  srate    = 250
}

node smooth : module "psl/moving-average" {
  window = 5
}

node log : output "psl/print" {
  every = 50
}

map ecg.out    -> smooth.in
map smooth.out -> log.in
"#;

pub fn new(args: &Args) -> CliResult {
    match args.pos(0) {
        Some("node") => new_node(args),
        Some("pipeline") => new_pipeline(args),
        _ => Err(CliError::usage("psl new node <name> --kind <input|module|output> [--lang python]\n       psl new pipeline <name>")),
    }
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !s.starts_with('-')
}

fn new_pipeline(args: &Args) -> CliResult {
    let root = workspace_root(args)?;
    let name = args
        .pos(1)
        .ok_or_else(|| CliError::usage("psl new pipeline <name>"))?;
    if !valid_name(name) {
        return Err(CliError::usage(
            "pipeline names use lowercase letters, digits, `-` and `_`",
        ));
    }
    let path = root.join("pipelines").join(format!("{}.psl", name));
    if path.exists() {
        return Err(CliError::failed(format!(
            "{} already exists",
            path.display()
        )));
    }
    let text = format!(
        "pipeline \"{}\" {{\n  description = \"\"\n}}\n\nnode ecg : input \"psl/synthetic\" {{\n  waveform = \"ecg\"\n  srate    = 250\n}}\n\nnode log : output \"psl/print\" {{}}\n\nmap ecg.out -> log.in\n",
        name
    );
    write_file(&path, text.as_bytes())?;
    println!("created {}", crate::util::display_path(&root, &path));
    Ok(0)
}

fn new_node(args: &Args) -> CliResult {
    let root = workspace_root(args)?;
    let name = args
        .pos(1)
        .ok_or_else(|| CliError::usage("psl new node <name> --kind <input|module|output>"))?;
    if !valid_name(name) {
        return Err(CliError::usage(
            "node names use lowercase letters, digits, `-` and `_`",
        ));
    }
    let kind = args
        .value("--kind")
        .ok_or_else(|| CliError::usage("psl new node <name> --kind <input|module|output>"))?;
    if !["input", "module", "output"].contains(&kind) {
        return Err(CliError::usage(format!(
            "--kind must be input, module or output, not `{}`",
            kind
        )));
    }
    let lang = args.value("--lang").unwrap_or("python");
    if lang != "python" {
        return Err(CliError::usage(format!(
            "only `--lang python` is supported in this version, not `{}`",
            lang
        )));
    }
    let dir = root.join("nodes").join(name);
    if dir.exists() {
        return Err(CliError::failed(format!(
            "{} already exists",
            dir.display()
        )));
    }
    let class = to_class_name(name);
    let (manifest, main, test) = match kind {
        "input" => (input_manifest(name), input_main(&class), input_test(&class)),
        "module" => (
            module_manifest(name),
            module_main(&class),
            module_test(&class),
        ),
        _ => (
            output_manifest(name),
            output_main(&class),
            output_test(&class),
        ),
    };
    write_file(&dir.join("node.psl"), manifest.as_bytes())?;
    write_file(&dir.join("main.py"), main.as_bytes())?;
    write_file(
        &dir.join("README.md"),
        format!(
            "# {}\n\nA PSL {} node. Describe what it does, its ports and its parameters here.\n",
            name, kind
        )
        .as_bytes(),
    )?;
    write_file(
        &dir.join("tests")
            .join(format!("test_{}.py", name.replace('-', "_"))),
        test.as_bytes(),
    )?;
    println!(
        "created nodes/{}/ (node.psl, main.py, README.md, tests/)",
        name
    );
    println!(
        "use it in a pipeline with:  node my_{} : {} \"{}\" {{ }}",
        kind, kind, name
    );
    Ok(0)
}

fn to_class_name(name: &str) -> String {
    name.split(['-', '_'])
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn input_manifest(name: &str) -> String {
    format!(
        r#"node "{name}" {{
  kind        = input
  version     = "0.1.0"
  description = "Pulls raw samples from a device"

  runtime python {{
    entry = "main.py"
  }}

  output "out" {{
    type        = signal
    description = "Raw samples as the device delivers them"
  }}

  param "srate" {{
    type        = number
    default     = 250
    min         = 1
    unit        = "Hz"
    description = "Sample rate to request from the device"
  }}
}}
"#
    )
}

fn input_main(class: &str) -> String {
    format!(
        r#"import numpy as np
import psl


class {class}(psl.Node):
    """An input driver: open the device, publish what it delivers, nothing more."""

    def on_init(self, params):
        self.srate = params["srate"]
        self.n = 0

    def on_start(self):
        # Announce what this stream looks like, then start producing.
        header = psl.SignalHeader(
            srate=self.srate,
            channels=[psl.Channel(label="CH1", unit="au", modality="other")],
            source={{"device": "replace-me"}},
        )
        self.declare_output("out", header)
        # Replace the timer with your device's own callback or a self.spawn(...) reader thread.
        self.every(0.02, self.tick)

    def tick(self):
        n = max(1, int(self.srate * 0.02))
        t0 = self.now() - int(n / self.srate * 1e9)
        data = np.zeros((n, 1), dtype=np.float32)
        self.n += n
        self.emit("out", psl.Chunk(t0=t0, data=data))

    def on_stop(self):
        pass  # close the device here


if __name__ == "__main__":
    psl.run({class})
"#
    )
}

fn input_test(class: &str) -> String {
    format!(
        r#"import unittest

from psl.testing import Harness

from main import {class}


class Test{class}(unittest.TestCase):
    def test_emits_chunks(self):
        h = Harness({class}, params={{"srate": 100}})
        h.start()
        h.tick()
        self.assertEqual(len(h.headers("out")), 1)
        self.assertGreaterEqual(len(h.outputs("out")), 1)


if __name__ == "__main__":
    unittest.main()
"#
    )
}

fn module_manifest(name: &str) -> String {
    format!(
        r#"node "{name}" {{
  kind        = module
  version     = "0.1.0"
  description = "Does one thing to a signal"

  runtime python {{
    entry = "main.py"
  }}

  input "in" {{
    type        = signal
    description = "Signal to process"
  }}

  output "out" {{
    type        = signal
    description = "Processed signal"
  }}

  param "gain" {{
    type        = number
    default     = 1.0
    mutable     = true
    description = "Multiplier applied to every sample"
  }}
}}
"#
    )
}

fn module_main(class: &str) -> String {
    format!(
        r#"import psl


class {class}(psl.Node):
    """A module: one well-defined transformation."""

    def on_init(self, params):
        self.gain = params["gain"]

    def on_header(self, port, header):
        out = header.copy()
        out.meta["{class}"] = {{"gain": self.gain}}
        self.declare_output("out", out)

    def on_signal(self, port, chunk):
        self.emit("out", chunk.replace(data=chunk.data * self.gain))

    def on_param(self, name, value):
        if name == "gain":
            self.gain = value
            return True
        return False


if __name__ == "__main__":
    psl.run({class})
"#
    )
}

fn module_test(class: &str) -> String {
    format!(
        r#"import unittest

import numpy as np
import psl
from psl.testing import Harness

from main import {class}


class Test{class}(unittest.TestCase):
    def test_scales(self):
        h = Harness({class}, params={{"gain": 2.0}})
        h.header("in", psl.SignalHeader(srate=100, channels=[psl.Channel("A")]))
        h.signal("in", psl.Chunk(t0=0, data=np.ones((4, 1), np.float32)))
        out = h.outputs("out")
        self.assertEqual(len(out), 1)
        self.assertTrue(np.allclose(out[0].data, 2.0))


if __name__ == "__main__":
    unittest.main()
"#
    )
}

fn output_manifest(name: &str) -> String {
    format!(
        r#"node "{name}" {{
  kind        = output
  version     = "0.1.0"
  description = "Delivers a signal somewhere outside the pipeline"

  runtime python {{
    entry = "main.py"
  }}

  input "in" {{
    type        = signal
    description = "Signal to deliver"
  }}

  param "target" {{
    type        = string
    default     = "stdout"
    description = "Where to send the data"
  }}
}}
"#
    )
}

fn output_main(class: &str) -> String {
    format!(
        r#"import psl


class {class}(psl.Node):
    """An output driver: take data and deliver it, without altering it."""

    def on_init(self, params):
        self.target = params["target"]

    def on_header(self, port, header):
        self.log.info("stream header", srate=header.srate, channels=len(header.channels))

    def on_signal(self, port, chunk):
        # Replace with the real delivery (file, socket, device ...).
        self.log.debug("chunk", seq=chunk.seq, samples=chunk.n_samples)

    def on_stop(self):
        pass  # flush and close here


if __name__ == "__main__":
    psl.run({class})
"#
    )
}

fn output_test(class: &str) -> String {
    format!(
        r#"import unittest

import numpy as np
import psl
from psl.testing import Harness

from main import {class}


class Test{class}(unittest.TestCase):
    def test_accepts_signal(self):
        h = Harness({class}, params={{"target": "stdout"}})
        h.header("in", psl.SignalHeader(srate=100, channels=[psl.Channel("A")]))
        h.signal("in", psl.Chunk(t0=0, data=np.ones((4, 1), np.float32)))
        self.assertTrue(any(level == "info" for level, _ in h.logs))


if __name__ == "__main__":
    unittest.main()
"#
    )
}

/// `psl init` inside an existing workspace: (re)install the embedded packages and SDK, create the
/// venv if missing, and leave user files alone.
fn refresh(dir: &Path, args: &Args) -> CliResult {
    println!("refreshing workspace in {}", dir.display());
    let mut installed = Vec::new();
    for pkg in embedded::node_names() {
        let version = install_embedded(dir, pkg)?;
        installed.push(format!("psl/{} {}", pkg, version));
    }
    install_sdk(dir)?;
    write_file(&dir.join(".psl").join("version"), crate::VERSION.as_bytes())?;
    println!(
        "  packages: {}",
        if installed.is_empty() {
            "(none embedded)".to_string()
        } else {
            installed.join(", ")
        }
    );
    println!(
        "  Python SDK: .psl/sdk/psl ({} files)",
        embedded::SDK_FILES.len()
    );
    if !args.flag("--no-venv")
        && !dir
            .join(".psl")
            .join("venv")
            .join("bin")
            .join("python")
            .is_file()
    {
        match create_venv(dir) {
            Ok(python) => println!("  Python env: {}", python),
            Err(e) => println!("  warning: {}", e),
        }
    }
    Ok(0)
}
