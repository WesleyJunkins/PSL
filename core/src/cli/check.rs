//! `psl check` and `psl graph`.

use super::{pipeline_path, workspace_root, Args, CliResult};
use crate::check::check_pipeline;
use crate::model::Graph;
use crate::syntax::Kind;

pub fn check(args: &Args) -> CliResult {
    let root = workspace_root(args)?;
    let path = pipeline_path(&root, args);
    let r = check_pipeline(&root, &path);
    let json = args.flag("--json");
    if json {
        let diags: Vec<_> = r.diags.items.iter().map(|d| d.to_json(&r.sm)).collect();
        let out = crate::json::Json::object()
            .with("ok", !r.diags.has_errors())
            .with("diagnostics", diags)
            .with("graph", r.graph.as_ref().map(|g| g.to_json()));
        println!("{}", out.to_pretty());
        return Ok(if r.diags.has_errors() { 1 } else { 0 });
    }
    eprint!("{}", r.diags.render(&r.sm));
    let file = crate::util::display_path(&root, &path);
    if r.diags.has_errors() {
        eprintln!(
            "check: {} error{}, {} warning{} in {}",
            r.diags.error_count(),
            plural(r.diags.error_count()),
            r.diags.warning_count(),
            plural(r.diags.warning_count()),
            file
        );
        Ok(1)
    } else {
        let g = r.graph.as_ref().unwrap();
        let w = r.diags.warning_count();
        let warn = if w > 0 {
            format!(", {} warning{}", w, plural(w))
        } else {
            String::new()
        };
        eprintln!(
            "check: ok — {} node{}, {} map{}{} in {}",
            g.nodes.len(),
            plural(g.nodes.len()),
            g.maps.len(),
            plural(g.maps.len()),
            warn,
            file
        );
        Ok(0)
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

pub fn graph(args: &Args) -> CliResult {
    let root = workspace_root(args)?;
    let path = pipeline_path(&root, args);
    let r = check_pipeline(&root, &path);
    if r.diags.has_errors() {
        eprint!("{}", r.diags.render(&r.sm));
        return Ok(1);
    }
    let g = r.graph.unwrap();
    if args.flag("--dot") {
        print!("{}", dot(&g));
    } else {
        print!("{}", text(&g));
    }
    Ok(0)
}

pub fn text(g: &Graph) -> String {
    let mut out = format!("pipeline {} ({})\n", g.name, g.file);
    for n in &g.nodes {
        let ver = if n.version == "local" {
            String::new()
        } else {
            format!(" {}", n.version)
        };
        out.push_str(&format!(
            "  {} [{} {}{}]\n",
            n.id,
            n.kind.as_str(),
            n.package,
            ver
        ));
        for o in &n.outputs {
            let targets: Vec<String> = g
                .maps
                .iter()
                .filter(|e| e.from_node == n.id && e.from_port == o.name)
                .map(|e| e.target())
                .collect();
            if targets.is_empty() {
                out.push_str(&format!("    {} -> (nowhere)\n", o.name));
            } else {
                out.push_str(&format!("    {} -> {}\n", o.name, targets.join(", ")));
            }
        }
        for i in &n.inputs {
            if i.sources.is_empty() {
                out.push_str(&format!("    {} <- (nothing)\n", i.name));
            }
        }
    }
    out
}

pub fn dot(g: &Graph) -> String {
    let mut out = format!(
        "digraph \"{}\" {{\n  rankdir=LR;\n  node [shape=record, fontname=\"Helvetica\"];\n",
        g.name
    );
    for n in &g.nodes {
        let ins: Vec<String> = n
            .inputs
            .iter()
            .map(|p| format!("<in_{}> {}", p.name, p.name))
            .collect();
        let outs: Vec<String> = n
            .outputs
            .iter()
            .map(|p| format!("<out_{}> {}", p.name, p.name))
            .collect();
        let color = match n.kind {
            Kind::Input => "#d9ead3",
            Kind::Module => "#fff2cc",
            Kind::Output => "#d0e0f0",
        };
        out.push_str(&format!(
            "  \"{}\" [label=\"{{{{{}}}|{}\\n{}|{{{}}}}}\", style=filled, fillcolor=\"{}\"];\n",
            n.id,
            ins.join("|"),
            n.id,
            n.package,
            outs.join("|"),
            color
        ));
    }
    for e in &g.maps {
        out.push_str(&format!(
            "  \"{}\":out_{} -> \"{}\":in_{};\n",
            e.from_node, e.from_port, e.to_node, e.to_port
        ));
    }
    out.push_str("}\n");
    out
}
