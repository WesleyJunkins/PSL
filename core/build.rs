//! Embeds the first-party node packages (`../nodes/psl/*`) and the Python SDK
//! (`../sdk/python/psl`) into the binary so `psl init` works offline.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest_dir.parent().unwrap().to_path_buf();
    let nodes_dir = repo.join("nodes").join("psl");
    let sdk_dir = repo.join("sdk").join("python").join("psl");
    println!("cargo:rerun-if-changed={}", nodes_dir.display());
    println!("cargo:rerun-if-changed={}", sdk_dir.display());

    let mut out = String::new();
    out.push_str("/// (package name, relative path, contents) for every first-party node file.\n");
    out.push_str("pub static NODE_FILES: &[(&str, &str, &[u8])] = &[\n");
    if let Ok(entries) = fs::read_dir(&nodes_dir) {
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for d in dirs {
            if !d.join("node.psl").is_file() {
                continue;
            }
            let name = d.file_name().unwrap().to_string_lossy().to_string();
            for (rel, abs) in walk(&d) {
                println!("cargo:rerun-if-changed={}", abs.display());
                out.push_str(&format!(
                    "    ({:?}, {:?}, include_bytes!({:?})),\n",
                    name,
                    rel,
                    abs.display().to_string()
                ));
            }
        }
    }
    out.push_str("];\n\n");
    out.push_str("/// (relative path under the SDK root, contents) for the Python SDK.\n");
    out.push_str("pub static SDK_FILES: &[(&str, &[u8])] = &[\n");
    if sdk_dir.is_dir() {
        for (rel, abs) in walk(&sdk_dir) {
            println!("cargo:rerun-if-changed={}", abs.display());
            out.push_str(&format!(
                "    ({:?}, include_bytes!({:?})),\n",
                format!("psl/{}", rel),
                abs.display().to_string()
            ));
        }
    }
    out.push_str("];\n");
    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("embedded.rs");
    fs::write(dest, out).unwrap();
}

/// Regular files under `dir` (recursively), skipping caches, as (relative, absolute).
fn walk(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    fn rec(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        let mut entries: Vec<PathBuf> = match fs::read_dir(dir) {
            Ok(e) => e.flatten().map(|e| e.path()).collect(),
            Err(_) => return,
        };
        entries.sort();
        for p in entries {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if name == "__pycache__" || name.starts_with('.') || name.ends_with(".pyc") {
                continue;
            }
            if p.is_dir() {
                rec(base, &p, out);
            } else if p.is_file() {
                let rel = p
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, p));
            }
        }
    }
    rec(dir, dir, &mut out);
    out
}
