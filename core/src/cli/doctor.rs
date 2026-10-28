//! `psl version`, `psl list`, `psl doctor`.

use std::process::Command;

use super::{workspace_root, Args, CliResult};
use crate::check::load_workspace;
use crate::diag::{Diagnostics, SourceMap};
use crate::supervisor::proc::{python_for, sdk_path};

pub fn version(args: &Args) -> CliResult {
    if args.flag("--json") {
        println!(
            "{}",
            crate::json::Json::object()
                .with("psl", crate::VERSION)
                .with("protocol", crate::PROTOCOL_VERSION as i64)
                .with("grammar", crate::GRAMMAR_VERSION as i64)
                .with("manifest", crate::MANIFEST_VERSION as i64)
                .with("recording", crate::RECORDING_VERSION as i64)
        );
    } else {
        println!(
            "psl {} (protocol {}, grammar {}, manifest {}, recording {})",
            crate::VERSION,
            crate::PROTOCOL_VERSION,
            crate::GRAMMAR_VERSION,
            crate::MANIFEST_VERSION,
            crate::RECORDING_VERSION
        );
    }
    Ok(0)
}

pub fn list(args: &Args) -> CliResult {
    let root = workspace_root(args)?;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut sm = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut describe = |reference: String, dir: &std::path::Path, version: String| {
        let path = dir.join("node.psl");
        let (kind, desc) = match crate::check::load_document(&mut sm, &mut diags, &root, &path) {
            Some((file, doc)) => match crate::model::parse_manifest(&doc, file).0 {
                Some(m) => (m.kind.as_str().to_string(), m.description),
                None => ("?".into(), "(manifest has errors)".into()),
            },
            None => ("?".into(), "(cannot read manifest)".into()),
        };
        rows.push(vec![reference, kind, version, desc]);
    };
    if let Ok(entries) = std::fs::read_dir(root.join("nodes")) {
        let mut dirs: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join("node.psl").is_file())
            .collect();
        dirs.sort();
        for d in dirs {
            describe(
                d.file_name().unwrap().to_string_lossy().to_string(),
                &d,
                "local".into(),
            );
        }
    }
    let pkgs = crate::check::resolve::packages_dir(&root);
    if let Ok(owners) = std::fs::read_dir(&pkgs) {
        let mut owners: Vec<_> = owners
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        owners.sort();
        for o in owners {
            let mut names: Vec<_> = std::fs::read_dir(&o)
                .map(|r| {
                    r.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_dir())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            for n in names {
                let mut versions: Vec<_> = std::fs::read_dir(&n)
                    .map(|r| {
                        r.flatten()
                            .map(|e| e.path())
                            .filter(|p| p.join("node.psl").is_file())
                            .collect()
                    })
                    .unwrap_or_default();
                versions.sort();
                for v in versions {
                    describe(
                        format!(
                            "{}/{}",
                            o.file_name().unwrap().to_string_lossy(),
                            n.file_name().unwrap().to_string_lossy()
                        ),
                        &v,
                        v.file_name().unwrap().to_string_lossy().to_string(),
                    );
                }
            }
        }
    }
    if rows.is_empty() {
        println!("no nodes or packages in this workspace");
    } else {
        super::live::print_table(&["NODE", "KIND", "VERSION", "DESCRIPTION"], &rows);
    }
    Ok(0)
}

pub fn doctor(args: &Args) -> CliResult {
    let mut problems = 0;
    let mut report = |ok: bool, what: &str, detail: String| {
        println!(
            "{} {:<22} {}",
            if ok { "ok  " } else { "WARN" },
            what,
            detail
        );
        if !ok {
            problems += 1;
        }
    };
    report(
        true,
        "psl",
        format!("{} (protocol {})", crate::VERSION, crate::PROTOCOL_VERSION),
    );
    let root = match workspace_root(args) {
        Ok(r) => r,
        Err(e) => {
            report(
                false,
                "workspace",
                e.message.trim_start_matches("usage: ").to_string(),
            );
            return Ok(1);
        }
    };
    report(true, "workspace", root.display().to_string());
    let mut sm = SourceMap::new();
    let mut diags = Diagnostics::new();
    let ws = load_workspace(&mut sm, &mut diags, &root);
    if diags.has_errors() {
        report(
            false,
            "workspace.psl",
            format!("{} error(s); run `psl check`", diags.error_count()),
        );
    } else {
        report(
            true,
            "workspace.psl",
            format!(
                "workspace \"{}\"",
                ws.as_ref().map(|w| w.name.as_str()).unwrap_or("?")
            ),
        );
    }
    let python = python_for(&root, ws.as_ref().and_then(|w| w.python.as_deref()));
    match Command::new(&python)
        .args(["-c", "import sys; print('%d.%d.%d' % sys.version_info[:3])"])
        .output()
    {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let new_enough = crate::util::Version::parse(&v)
                .map(|v| (v.major, v.minor) >= (3, 11))
                .unwrap_or(false);
            if new_enough {
                report(true, "python", format!("{} ({})", v, python));
            } else {
                report(
                    false,
                    "python",
                    format!("{} is {}; {}", python, v, super::init::PYTHON_HELP),
                );
            }
            match Command::new(&python)
                .args(["-c", "import numpy; print(numpy.__version__)"])
                .output()
            {
                Ok(o) if o.status.success() => report(
                    true,
                    "numpy",
                    String::from_utf8_lossy(&o.stdout).trim().to_string(),
                ),
                _ => report(
                    false,
                    "numpy",
                    format!(
                        "not importable by {}; run `psl init` here to create .psl/venv with numpy",
                        python
                    ),
                ),
            }
        }
        _ => match super::init::find_python() {
            Ok((cmd, v)) => report(
                false,
                "python",
                format!(
                    "cannot run `{}`, but {} ({}) exists; run `psl init` to create .psl/venv",
                    python, cmd, v
                ),
            ),
            Err(e) => report(
                false,
                "python",
                format!("{}. {}", e, super::init::PYTHON_HELP),
            ),
        },
    }
    report(
        true,
        "uv",
        if super::init::have_uv() {
            "found (psl init can fetch Python with it)".into()
        } else {
            "not found (optional; https://docs.astral.sh/uv/)".into()
        },
    );
    let sdk = sdk_path(&root);
    if sdk.join("psl").join("__init__.py").is_file() {
        report(true, "python sdk", sdk.display().to_string());
    } else {
        report(
            false,
            "python sdk",
            format!(
                "{} is missing; run `psl init` here or set PSL_SDK_PATH",
                sdk.display()
            ),
        );
    }
    let names = crate::embedded::node_names();
    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| {
            !crate::check::resolve::packages_dir(&root)
                .join("psl")
                .join(n)
                .is_dir()
        })
        .collect();
    if missing.is_empty() {
        report(
            true,
            "first-party packages",
            format!("{} installed", names.len()),
        );
    } else {
        report(
            false,
            "first-party packages",
            format!("missing: {}", missing.join(", ")),
        );
    }
    let sock_base = if cfg!(unix) {
        std::path::PathBuf::from("/tmp/psl")
    } else {
        std::env::temp_dir().join("psl")
    };
    match std::fs::create_dir_all(&sock_base) {
        Ok(()) => report(true, "socket directory", sock_base.display().to_string()),
        Err(e) => report(
            false,
            "socket directory",
            format!("{}: {}", sock_base.display(), e),
        ),
    }
    match crate::supervisor::session::current(&root) {
        Ok(Some(s)) => match s.control.connect() {
            Ok(_) => report(true, "session", format!("{} is running", s.id)),
            Err(_) => {
                crate::supervisor::session::clear_current(&root);
                report(
                    true,
                    "session",
                    format!("stale marker for {} removed", s.id),
                );
            }
        },
        Ok(None) => report(true, "session", "none running".into()),
        Err(e) => report(false, "session", e),
    }
    if problems == 0 {
        println!("\nall good");
        Ok(0)
    } else {
        println!("\n{} problem(s)", problems);
        Ok(1)
    }
}
