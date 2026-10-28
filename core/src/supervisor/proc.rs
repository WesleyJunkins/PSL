//! Spawning node processes (docs/01 §Session, docs/05).

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::model::{GNode, Runtime};
use crate::protocol::Address;

pub struct SpawnEnv<'a> {
    pub workspace: &'a Path,
    pub session: &'a str,
    pub control: &'a Address,
    pub data: &'a Address,
    pub python: &'a str,
    pub sdk_path: &'a Path,
    pub log_path: &'a Path,
}

/// Where the Python SDK is read from: `PSL_SDK_PATH` if set, else `<workspace>/.psl/sdk`.
pub fn sdk_path(root: &Path) -> PathBuf {
    match std::env::var("PSL_SDK_PATH") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => root.join(".psl").join("sdk"),
    }
}

/// The interpreter to run Python nodes with: the workspace setting, else the workspace venv, else `python3`.
pub fn python_for(root: &Path, configured: Option<&str>) -> String {
    if let Some(p) = configured {
        return p.to_string();
    }
    let venv = root.join(".psl").join("venv");
    for candidate in [
        venv.join("bin").join("python"),
        venv.join("Scripts").join("python.exe"),
    ] {
        if candidate.is_file() {
            return candidate.display().to_string();
        }
    }
    "python3".to_string()
}

pub fn spawn(node: &GNode, env: &SpawnEnv) -> io::Result<Child> {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(env.log_path)?;
    let log_err = log.try_clone()?;
    let mut cmd = match &node.manifest.runtime {
        Runtime::Python { entry, .. } => {
            let mut c = Command::new(env.python);
            c.arg(node.path.join(entry));
            let existing = std::env::var("PYTHONPATH").unwrap_or_default();
            let sep = if cfg!(windows) { ";" } else { ":" };
            let pp = if existing.is_empty() {
                env.sdk_path.display().to_string()
            } else {
                format!("{}{}{}", env.sdk_path.display(), sep, existing)
            };
            c.env("PYTHONPATH", pp);
            c.env("PYTHONUNBUFFERED", "1");
            c
        }
        Runtime::Exec { command } => {
            let os = std::env::consts::OS;
            let arch = std::env::consts::ARCH;
            let subst = |s: &str| s.replace("${os}", os).replace("${arch}", arch);
            let program = subst(&command[0]);
            let program = if Path::new(&program).is_relative() && program.contains('/') {
                node.path.join(&program)
            } else {
                PathBuf::from(program)
            };
            let mut c = Command::new(program);
            for a in &command[1..] {
                c.arg(subst(a));
            }
            c
        }
    };
    // Nodes run from the workspace root so relative paths in parameters mean what the
    // pipeline author expects; the package directory travels in PSL_NODE_DIR.
    cmd.current_dir(env.workspace)
        .env("PSL_WORKSPACE", env.workspace)
        .env("PSL_NODE_DIR", &node.path)
        .env("PSL_SESSION", env.session)
        .env("PSL_NODE_ID", &node.id)
        .env("PSL_CONTROL", env.control.to_string())
        .env("PSL_DATA", env.data.to_string())
        .env("PSL_PROTOCOL", crate::PROTOCOL_VERSION.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    for (k, v) in &node.runtime.env {
        cmd.env(k, v);
    }
    cmd.spawn()
}
