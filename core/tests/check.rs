//! Every diagnostic code in docs/04 §Validation is triggered by a fixture here.

use std::fs;
use std::path::{Path, PathBuf};

use psl::check::check_pipeline;

struct Ws {
    root: PathBuf,
}

impl Ws {
    fn new(name: &str) -> Ws {
        let root = std::env::temp_dir().join(format!("psl-check-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("pipelines")).unwrap();
        fs::write(root.join("workspace.psl"), "workspace \"t\" {}\n").unwrap();
        Ws { root }
    }

    fn workspace(&self, src: &str) -> &Ws {
        fs::write(self.root.join("workspace.psl"), src).unwrap();
        self
    }

    fn local(&self, name: &str, manifest: &str) -> &Ws {
        let d = self.root.join("nodes").join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("node.psl"), manifest).unwrap();
        fs::write(d.join("main.py"), "").unwrap();
        self
    }

    fn package(&self, owner: &str, name: &str, version: &str, manifest: &str) -> &Ws {
        let d = self
            .root
            .join(".psl/packages")
            .join(owner)
            .join(name)
            .join(version);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("node.psl"), manifest).unwrap();
        self
    }

    fn codes(&self, pipeline: &str) -> Vec<&'static str> {
        let p = self.root.join("pipelines/main.psl");
        fs::write(&p, pipeline).unwrap();
        let r = check_pipeline(&self.root, &p);
        let rendered = r.diags.render(&r.sm);
        eprintln!("{}", rendered);
        r.diags.items.iter().map(|d| d.code).collect()
    }

    fn graph_ok(&self, pipeline: &str) -> psl::model::Graph {
        let p = self.root.join("pipelines/main.psl");
        fs::write(&p, pipeline).unwrap();
        let r = check_pipeline(&self.root, &p);
        assert!(!r.diags.has_errors(), "{}", r.diags.render(&r.sm));
        r.graph.unwrap()
    }
}

impl Drop for Ws {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const SRC: &str = r#"node "src" {
  kind = input
  version = "0.1.0"
  runtime python { entry = "main.py" }
  output "out" { type = signal, channels = 2, modality = ecg }
  output "ev" { type = event, labels = ["beat"] }
  param "rate" { type = number, default = 250, min = 1, max = 1000 }
  param "name" { type = string, default = "x" }
  param "mode" { type = enum, values = ["a", "b"], default = "a" }
}"#;

const MOD: &str = r#"node "mod" {
  kind = module
  version = "0.1.0"
  runtime python { entry = "main.py" }
  input "in" { type = signal }
  input "single" { type = signal, required = false, multiple = false }
  input "mono" { type = signal, required = false, channels = 1 }
  input "eeg" { type = signal, required = false, modality = [eeg] }
  output "out" { type = signal }
  param "window" { type = integer, min = 1 }
}"#;

const SINK: &str = r#"node "sink" {
  kind = output
  version = "0.1.0"
  runtime python { entry = "main.py" }
  input "in" { type = signal }
  input "events" { type = event, required = false, labels = ["marker"] }
}"#;

fn full(ws: &Ws) -> &Ws {
    ws.local("src", SRC).local("mod", MOD).local("sink", SINK)
}

const GOOD: &str = r#"pipeline "p" { description = "d" }
node s : input "src" { rate = 100 }
node m : module "mod" { window = 3, runtime { restart = "on-failure", queue = 8 } }
node k : output "sink" {}
map s.out -> m.in
map m.out -> k.in
"#;

#[test]
fn good_pipeline_builds_graph() {
    let ws = Ws::new("good");
    let g = full(&ws).graph_ok(GOOD);
    assert_eq!(g.nodes.len(), 3);
    assert_eq!(g.maps.len(), 2);
    assert_eq!(
        g.node("s").unwrap().params.get("rate"),
        Some(&psl::json::Json::Int(100))
    );
    assert_eq!(
        g.node("s").unwrap().params.get("name"),
        Some(&psl::json::Json::Str("x".into()))
    );
    assert_eq!(
        g.node("m").unwrap().inputs[0].sources,
        vec!["s.out".to_string()]
    );
    assert_eq!(g.node("m").unwrap().runtime.queue, 8);
    assert_eq!(g.start_order(), vec!["k", "m", "s"]);
    let j = g.to_json();
    assert_eq!(j.get("maps").unwrap().as_array().unwrap().len(), 2);
    // W0001: s.ev is unused.
    let codes = full(&ws).codes(GOOD);
    assert_eq!(codes, vec!["W0001"]);
}

#[test]
fn syntax_and_schema_codes() {
    let ws = Ws::new("schema");
    full(&ws);
    assert!(ws.codes("pipeline \"p\" {\nnode").contains(&"E0001"));
    assert!(ws.codes("pipeline \"p\" { bogus = 1 }").contains(&"E0002"));
    assert!(ws
        .codes("pipeline \"p\" { runtime { ready_timeout_s = \"x\" } }")
        .contains(&"E0003"));
    assert!(ws
        .codes("pipeline \"p\" { description = \"a\", description = \"b\" }")
        .contains(&"E0004"));
    assert!(ws.codes("pipeline \"p\" {}\nnode s : input \"src\" { name = hello }\nnode k : output \"sink\" {}\nmap s.out -> k.in").contains(&"E0005"));
    assert!(ws.codes("node s : input \"src\" {}").contains(&"E0006"));
    assert!(ws
        .codes("node s : input \"src\" {}\npipeline \"p\" {}")
        .contains(&"E0002"));
}

#[test]
fn resolution_codes() {
    let ws = Ws::new("resolve");
    full(&ws);
    assert!(ws
        .codes("pipeline \"p\" {}\nnode x : input \"nope\" {}")
        .contains(&"E0101"));
    assert!(ws
        .codes("pipeline \"p\" {}\nnode x : input \"nope/nope\" {}")
        .contains(&"E0101"));
    assert!(ws
        .codes("pipeline \"p\" {}\nnode x : module \"src\" {}")
        .contains(&"E0102"));
    assert!(ws
        .codes("pipeline \"p\" {}\nnode x : input \"src\" {}\nnode x : input \"src\" {}")
        .contains(&"E0103"));
    ws.local("bad", "node \"bad\" { kind = input, version = \"1.0.0\", runtime python { entry = \"m.py\" }, input \"a\" { type = signal } }");
    let codes = ws.codes("pipeline \"p\" {}\nnode x : input \"bad\" {}");
    assert!(
        codes.contains(&"E0104") && codes.contains(&"E0401"),
        "{:?}",
        codes
    );
    ws.local(
        "bad2",
        "node \"bad2\" { kind = input, version = \"1.0.0\" }",
    );
    let codes = ws.codes("pipeline \"p\" {}\nnode x : input \"bad2\" {}");
    assert!(codes.contains(&"E0402"), "{:?}", codes);
}

#[test]
fn param_codes() {
    let ws = Ws::new("params");
    full(&ws);
    let base = "pipeline \"p\" {}\nnode k : output \"sink\" {}\nnode m : module \"mod\" { window = 1 }\nmap s.out -> m.in\nmap m.out -> k.in\n";
    assert!(ws
        .codes(&format!("{}node s : input \"src\" {{ rat = 1 }}", base))
        .contains(&"E0201"));
    assert!(ws
        .codes(&format!(
            "{}node s : input \"src\" {{ rate = \"fast\" }}",
            base
        ))
        .contains(&"E0202"));
    assert!(ws
        .codes(&format!("{}node s : input \"src\" {{ rate = 5000 }}", base))
        .contains(&"E0204"));
    assert!(ws
        .codes(&format!(
            "{}node s : input \"src\" {{ mode = \"c\" }}",
            base
        ))
        .contains(&"E0204"));
    assert!(ws.codes("pipeline \"p\" {}\nnode s : input \"src\" {}\nnode m : module \"mod\" {}\nnode k : output \"sink\" {}\nmap s.out -> m.in\nmap m.out -> k.in").contains(&"E0203"));
    assert!(ws.codes("pipeline \"p\" {}\nnode s : input \"src\" {}\nnode m : module \"mod\" { window = 2.5 }\nnode k : output \"sink\" {}\nmap s.out -> m.in\nmap m.out -> k.in").contains(&"E0202"));
}

#[test]
fn map_codes() {
    let ws = Ws::new("maps");
    full(&ws);
    let nodes = "pipeline \"p\" {}\nnode s : input \"src\" {}\nnode m : module \"mod\" { window = 1 }\nnode k : output \"sink\" {}\n";
    let with = |maps: &str| format!("{}{}", nodes, maps);
    assert!(ws
        .codes(&with(
            "map s.out -> m.in\nmap m.out -> k.in\nmap zz.out -> k.in"
        ))
        .contains(&"E0301"));
    assert!(ws
        .codes(&with(
            "map s.out -> m.in\nmap m.out -> k.in\nmap s.nope -> k.in"
        ))
        .contains(&"E0301"));
    assert!(ws
        .codes(&with(
            "map s.out -> m.in\nmap m.out -> k.in\nmap s.out -> m.out"
        ))
        .contains(&"E0302"));
    assert!(ws
        .codes(&with(
            "map s.out -> m.in\nmap m.out -> k.in\nmap m.in -> k.in"
        ))
        .contains(&"E0302"));
    assert!(ws
        .codes(&with(
            "map s.out -> m.in\nmap m.out -> k.in\nmap s.ev -> k.in"
        ))
        .contains(&"E0303"));
    assert!(ws
        .codes(&with(
            "map s.out -> m.in\nmap m.out -> k.in\nmap s.out -> m.mono"
        ))
        .contains(&"E0304"));
    assert!(ws
        .codes(&with(
            "map s.out -> m.in\nmap m.out -> k.in\nmap s.out -> m.eeg"
        ))
        .contains(&"E0304"));
    assert!(ws
        .codes(&with(
            "map s.out -> m.in\nmap m.out -> k.in\nmap s.ev -> k.events"
        ))
        .contains(&"E0304"));
    assert!(ws.codes(&with("map m.out -> k.in")).contains(&"E0305"));
    assert!(ws
        .codes(&with(
            "map s.out -> m.in\nmap m.out -> k.in\nmap s.out, m.out -> m.single"
        ))
        .contains(&"E0306"));
    let codes = ws.codes(&with("map s.out -> m.in\nmap m.out -> k.in, m.in"));
    assert!(codes.contains(&"W0003"), "{:?}", codes);
    assert!(!codes.iter().any(|c| c.starts_with('E')), "{:?}", codes);
}

#[test]
fn package_resolution_and_versions() {
    let ws = Ws::new("packages");
    ws.local("sink", SINK);
    ws.package("acme", "src", "0.1.0", SRC)
        .package("acme", "src", "0.2.0", SRC);
    let g = ws.graph_ok("pipeline \"p\" {}\nnode s : input \"acme/src\" {}\nnode k : output \"sink\" {}\nmap s.out -> k.in");
    assert_eq!(g.node("s").unwrap().version, "0.2.0");
    ws.workspace("workspace \"t\" {}\nrequire \"acme/src\" { version = \"^0.1\" }\n");
    let g = ws.graph_ok("pipeline \"p\" {}\nnode s : input \"acme/src\" {}\nnode k : output \"sink\" {}\nmap s.out -> k.in");
    assert_eq!(g.node("s").unwrap().version, "0.1.0");
    ws.workspace("workspace \"t\" {}\nrequire \"acme/src\" { version = \"^1.0\" }\n");
    assert!(ws.codes("pipeline \"p\" {}\nnode s : input \"acme/src\" {}\nnode k : output \"sink\" {}\nmap s.out -> k.in").contains(&"E0501"));
}

#[test]
fn dynamic_outputs_warning() {
    let ws = Ws::new("dynamic");
    ws.local("sink", SINK).local(
        "rep",
        "node \"rep\" { kind = input, version = \"0.1.0\", dynamic_outputs = true, runtime python { entry = \"m.py\" }, output \"out\" { type = signal } }",
    );
    let codes = ws.codes("pipeline \"p\" {}\nnode r : input \"rep\" {}\nnode k : output \"sink\" {}\nmap r.out -> k.in");
    assert_eq!(codes, vec!["W0002"]);
}

#[test]
fn renders_like_a_compiler() {
    let ws = Ws::new("render");
    full(&ws);
    let p = ws.root.join("pipelines/main.psl");
    fs::write(&p, "pipeline \"p\" {}\nnode s : input \"src\" {}\nnode k : output \"sink\" {}\nmap s.ev -> k.in\n").unwrap();
    let r = check_pipeline(Path::new(&ws.root), &p);
    let out = r.diags.render(&r.sm);
    assert!(out.contains("error[E0303]: port types differ"), "{}", out);
    assert!(out.contains("--> pipelines/main.psl:4:13"), "{}", out);
    assert!(out.contains("4 | map s.ev -> k.in"), "{}", out);
    assert!(out.contains("= help:"), "{}", out);
}
