//! Package resolution: from a package reference in a pipeline to a directory on disk (docs/07).

use std::path::{Path, PathBuf};

use crate::diag::Diagnostic;
use crate::model::workspace::{is_package_name, Workspace};
use crate::util::{Version, VersionReq};

pub struct Resolved {
    pub dir: PathBuf,
    pub version: String,
}

/// Where installed packages live inside a workspace.
pub fn packages_dir(root: &Path) -> PathBuf {
    root.join(".psl").join("packages")
}

pub fn resolve_package(
    root: &Path,
    reference: &str,
    ws: Option<&Workspace>,
) -> Result<Resolved, Diagnostic> {
    if !reference.contains('/') {
        // Workspace-local node.
        let dir = root.join("nodes").join(reference);
        if !dir.join("node.psl").is_file() {
            return Err(Diagnostic::error("E0101", format!("local node `{}` not found", reference)).help(format!(
                "expected `nodes/{}/node.psl`; create it with `psl new node {} --kind <input|module|output>`, or write `\"owner/{}\"` for an installed package",
                reference, reference, reference
            )));
        }
        return Ok(Resolved {
            dir,
            version: "local".into(),
        });
    }
    if !is_package_name(reference) {
        return Err(Diagnostic::error("E0101", format!("`{}` is not a valid package reference", reference))
            .help("use `\"name\"` for a local node under nodes/, or `\"owner/name\"` (lowercase, hyphens) for an installed package"));
    }
    let (owner, name) = reference.split_once('/').unwrap();
    let base = packages_dir(root).join(owner).join(name);
    let req = ws
        .and_then(|w| w.require(reference))
        .and_then(|r| r.version.as_deref())
        .and_then(VersionReq::parse);
    let mut candidates: Vec<(Version, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.join("node.psl").is_file() {
                continue;
            }
            if let Some(v) = p
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(Version::parse)
            {
                candidates.push((v, p));
            }
        }
    }
    if candidates.is_empty() {
        return Err(Diagnostic::error("E0101", format!("package `{}` is not installed", reference))
            .help(format!("install it with `psl install {}` (first-party `psl/*` packages are installed by `psl init`)", reference)));
    }
    let matching: Vec<&(Version, PathBuf)> = candidates
        .iter()
        .filter(|(v, _)| req.as_ref().map(|r| r.matches(v)).unwrap_or(true))
        .collect();
    let best = match matching.iter().max_by(|a, b| a.0.cmp(&b.0)) {
        Some(b) => b,
        None => {
            let have: Vec<String> = candidates.iter().map(|(v, _)| v.to_string()).collect();
            return Err(Diagnostic::error(
                "E0501",
                format!(
                    "no installed version of `{}` satisfies the workspace requirement",
                    reference
                ),
            )
            .help(format!(
                "installed: {}; run `psl install {}` to update",
                have.join(", "),
                reference
            )));
        }
    };
    Ok(Resolved {
        dir: best.1.clone(),
        version: best.0.to_string(),
    })
}
