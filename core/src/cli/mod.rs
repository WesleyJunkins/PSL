//! Command-line entry point (docs/06).

mod check;
mod doctor;
mod init;
mod live;
mod run;

use std::path::{Path, PathBuf};

use crate::util::find_workspace_root;

/// Flags that take a value; everything else starting with `-` is a boolean flag.
const VALUE_FLAGS: &[&str] = &[
    "--workspace",
    "--kind",
    "--lang",
    "--record",
    "--param",
    "--version",
    "--from",
    "--to",
    "-o",
    "--level",
    "--name",
];

pub struct Args {
    pub command: Option<String>,
    pub positional: Vec<String>,
    pub flags: Vec<(String, Option<String>)>,
}

impl Args {
    pub fn parse(raw: Vec<String>) -> Args {
        let mut command = None;
        let mut positional = Vec::new();
        let mut flags = Vec::new();
        let mut i = 0;
        while i < raw.len() {
            let a = &raw[i];
            if a == "--" {
                positional.extend(raw[i + 1..].iter().cloned());
                break;
            }
            if a.starts_with('-') && a.len() > 1 {
                if let Some((k, v)) = a.split_once('=') {
                    flags.push((k.to_string(), Some(v.to_string())));
                } else if VALUE_FLAGS.contains(&a.as_str()) {
                    let v = raw.get(i + 1).cloned();
                    flags.push((a.clone(), v));
                    i += 1;
                } else if a.len() > 2 && !a.starts_with("--") {
                    // Grouped short flags: -qv
                    for c in a[1..].chars() {
                        flags.push((format!("-{}", c), None));
                    }
                } else {
                    flags.push((a.clone(), None));
                }
            } else if command.is_none() {
                command = Some(a.clone());
            } else {
                positional.push(a.clone());
            }
            i += 1;
        }
        Args {
            command,
            positional,
            flags,
        }
    }

    pub fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == name)
    }

    pub fn any_flag(&self, names: &[&str]) -> bool {
        names.iter().any(|n| self.flag(n))
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .and_then(|(_, v)| v.as_deref())
    }

    pub fn values(&self, name: &str) -> Vec<&str> {
        self.flags
            .iter()
            .filter(|(k, _)| k == name)
            .filter_map(|(_, v)| v.as_deref())
            .collect()
    }

    pub fn pos(&self, i: usize) -> Option<&str> {
        self.positional.get(i).map(String::as_str)
    }
}

pub const USAGE: &str = "\
psl — physiological computing pipelines from the terminal

USAGE
  psl <command> [args] [--workspace <dir>] [--json] [-q] [-v]

WORKSPACE
  init [dir] [--no-venv]        Create a workspace with the first-party packages
  new node <name> --kind <k>    Scaffold a node (input | module | output) under nodes/
  new pipeline <name>           Scaffold pipelines/<name>.psl
  list                          Local nodes and installed packages
  doctor                        Check the environment
  version                       Versions of the binary and its contracts

PIPELINES
  check [pipeline] [--json]     Validate; print diagnostics (or the canonical model)
  graph [pipeline] [--dot]      Print the graph
  run [pipeline] [--record f]   Run in the foreground   (--param node.name=value)
  stop                          Stop the current session

LIVE (connect to the current session)
  ps [--watch]                  Node states, frames, drops, latency
  streams                       Streams with their current headers
  echo <node.port> [--raw]      Print frames as they arrive
  set <node.param> <value>      Change a mutable parameter
  record start [file] | stop    Record every stream to a .pslrec file
  logs [node] [-f]              Print or follow session logs

The default pipeline is pipelines/main.psl.
";

pub fn main() -> i32 {
    crate::supervisor::signal::default_sigpipe();
    let args = Args::parse(std::env::args().skip(1).collect());
    if args.flag("--help") || args.flag("-h") {
        print!("{}", USAGE);
        return 0;
    }
    let r = match args.command.as_deref() {
        None | Some("help") => {
            print!("{}", USAGE);
            Ok(0)
        }
        Some("version") => doctor::version(&args),
        Some("init") => init::init(&args),
        Some("new") => init::new(&args),
        Some("list") => doctor::list(&args),
        Some("doctor") => doctor::doctor(&args),
        Some("check") => check::check(&args),
        Some("graph") => check::graph(&args),
        Some("run") => run::run(&args),
        Some("stop") => live::stop(&args),
        Some("ps") => live::ps(&args),
        Some("streams") => live::streams(&args),
        Some("echo") => live::echo(&args),
        Some("set") => live::set(&args),
        Some("record") => live::record(&args),
        Some("logs") => live::logs(&args),
        Some(other) => Err(CliError::usage(format!(
            "unknown command `{}`\n\n{}",
            other, USAGE
        ))),
    };
    match r {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{}", e.message);
            e.code
        }
    }
}

pub struct CliError {
    pub code: i32,
    pub message: String,
}

impl CliError {
    pub fn usage(m: impl Into<String>) -> CliError {
        CliError {
            code: 2,
            message: format!("usage: {}", m.into()),
        }
    }
    pub fn failed(m: impl Into<String>) -> CliError {
        CliError {
            code: 3,
            message: format!("error: {}", m.into()),
        }
    }
    pub fn no_session() -> CliError {
        CliError {
            code: 4,
            message: "error: no running session in this workspace (start one with `psl run`)"
                .into(),
        }
    }
}

impl From<String> for CliError {
    fn from(s: String) -> CliError {
        CliError::failed(s)
    }
}

pub type CliResult = Result<i32, CliError>;

/// The workspace root: `--workspace`, else the nearest ancestor holding `workspace.psl`.
pub fn workspace_root(args: &Args) -> Result<PathBuf, CliError> {
    if let Some(w) = args.value("--workspace") {
        let p = PathBuf::from(w);
        if p.join("workspace.psl").is_file() {
            return Ok(p);
        }
        return Err(CliError::usage(format!("`{}` has no workspace.psl", w)));
    }
    let cwd = std::env::current_dir().map_err(|e| CliError::failed(e.to_string()))?;
    find_workspace_root(&cwd).ok_or_else(|| CliError::usage("not inside a PSL workspace (no workspace.psl found here or above); create one with `psl init`"))
}

/// The pipeline file: the first positional argument, else `pipelines/main.psl`.
pub fn pipeline_path(root: &Path, args: &Args) -> PathBuf {
    match args.pos(0) {
        Some(p) => {
            let p = PathBuf::from(p);
            if p.is_absolute() {
                p
            } else {
                std::env::current_dir().map(|c| c.join(&p)).unwrap_or(p)
            }
        }
        None => root.join("pipelines").join("main.psl"),
    }
}

pub fn write_file(path: &Path, contents: &[u8]) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CliError::failed(format!("cannot create {}: {}", parent.display(), e)))?;
    }
    std::fs::write(path, contents)
        .map_err(|e| CliError::failed(format!("cannot write {}: {}", path.display(), e)))
}

/// Pieces of the CLI that tests and other crates may reuse.
pub mod init_support {
    pub use super::init::{install_embedded, install_sdk};
}
