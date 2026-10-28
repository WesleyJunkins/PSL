//! First-party node packages and the Python SDK, embedded at build time (see `build.rs`).

include!(concat!(env!("OUT_DIR"), "/embedded.rs"));

/// Names of the embedded first-party packages, in order.
pub fn node_names() -> Vec<&'static str> {
    let mut names: Vec<&str> = Vec::new();
    for (name, _, _) in NODE_FILES {
        if !names.contains(name) {
            names.push(name);
        }
    }
    names
}

/// The manifest text of an embedded package.
pub fn node_manifest(name: &str) -> Option<&'static str> {
    NODE_FILES
        .iter()
        .find(|(n, rel, _)| *n == name && *rel == "node.psl")
        .and_then(|(_, _, b)| std::str::from_utf8(b).ok())
}
