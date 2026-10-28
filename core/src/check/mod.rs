//! `psl check`: load, resolve and validate a pipeline (docs/04 §Validation).

pub mod resolve;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::diag::{Diagnostic, Diagnostics, SourceMap};
use crate::json::Json;
use crate::model::manifest::{
    check_param_value, Channels, Manifest, Only, ParamError, ParamType, PortType,
};
use crate::model::value::{describe_json, to_json};
use crate::model::{
    parse_manifest, parse_pipeline, parse_workspace, Edge, GInput, GNode, GOutput, Graph, Pipeline,
    Workspace,
};
use crate::syntax::{parse_document, Endpoint, Kind, Span};
use crate::util::display_path;

pub struct CheckResult {
    pub sm: SourceMap,
    pub diags: Diagnostics,
    pub graph: Option<Graph>,
    pub workspace: Option<Workspace>,
}

/// Load a `.psl` file into the source map and parse it. Errors are pushed to `diags`.
pub fn load_document(
    sm: &mut SourceMap,
    diags: &mut Diagnostics,
    root: &Path,
    path: &Path,
) -> Option<(crate::syntax::FileId, crate::syntax::Document)> {
    let display = display_path(root, path);
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            diags.push(
                Diagnostic::error("E0101", format!("cannot read `{}`: {}", display, e))
                    .at_path(display),
            );
            return None;
        }
    };
    let file = sm.add(display, text);
    let src = sm.get(file).text.clone();
    match parse_document(file, &src) {
        Ok(doc) => Some((file, doc)),
        Err(e) => {
            diags.push(e);
            None
        }
    }
}

/// Load `workspace.psl` from `root` if present.
pub fn load_workspace(
    sm: &mut SourceMap,
    diags: &mut Diagnostics,
    root: &Path,
) -> Option<Workspace> {
    let path = root.join("workspace.psl");
    if !path.is_file() {
        return None;
    }
    let (file, doc) = load_document(sm, diags, root, &path)?;
    let (ws, d) = parse_workspace(&doc, file);
    diags.extend(d);
    ws
}

/// Check the pipeline at `pipeline_path` inside the workspace at `root`.
pub fn check_pipeline(root: &Path, pipeline_path: &Path) -> CheckResult {
    let mut sm = SourceMap::new();
    let mut diags = Diagnostics::new();
    let workspace = load_workspace(&mut sm, &mut diags, root);

    let pipeline =
        load_document(&mut sm, &mut diags, root, pipeline_path).and_then(|(file, doc)| {
            let (p, d) = parse_pipeline(&doc, file);
            diags.extend(d);
            p
        });
    let pipeline = match pipeline {
        Some(p) => p,
        None => {
            diags.sort();
            return CheckResult {
                sm,
                diags,
                graph: None,
                workspace,
            };
        }
    };
    let graph = build_graph(
        &mut sm,
        &mut diags,
        root,
        pipeline_path,
        &pipeline,
        workspace.as_ref(),
    );
    diags.sort();
    CheckResult {
        sm,
        diags,
        graph,
        workspace,
    }
}

struct ResolvedNode {
    manifest: Manifest,
    version: String,
    path: PathBuf,
}

fn build_graph(
    sm: &mut SourceMap,
    diags: &mut Diagnostics,
    root: &Path,
    pipeline_path: &Path,
    p: &Pipeline,
    ws: Option<&Workspace>,
) -> Option<Graph> {
    // 1. Resolve every node's package and manifest.
    let mut manifest_cache: HashMap<PathBuf, Option<Manifest>> = HashMap::new();
    let mut resolved: Vec<Option<ResolvedNode>> = Vec::new();
    for n in &p.nodes {
        let r = match resolve::resolve_package(root, &n.package, ws) {
            Ok(r) => r,
            Err(mut e) => {
                e.labels.insert(
                    0,
                    crate::diag::Label {
                        span: n.package_span,
                        text: "referenced here".into(),
                    },
                );
                diags.push(e);
                resolved.push(None);
                continue;
            }
        };
        let manifest_path = r.dir.join("node.psl");
        let manifest = match manifest_cache.get(&manifest_path) {
            Some(m) => m.clone(),
            None => {
                let m = load_document(sm, diags, root, &manifest_path).and_then(|(file, doc)| {
                    let (m, d) = parse_manifest(&doc, file);
                    diags.extend(d);
                    m
                });
                manifest_cache.insert(manifest_path.clone(), m.clone());
                m
            }
        };
        let m = match manifest {
            Some(m) => m,
            None => {
                diags.push(
                    Diagnostic::error(
                        "E0104",
                        format!("the manifest of `{}` has errors (see above)", n.package),
                    )
                    .primary(
                        n.package_span,
                        format!("manifest: {}", display_path(root, &manifest_path)),
                    ),
                );
                resolved.push(None);
                continue;
            }
        };
        if m.kind != n.kind {
            diags.push(
                Diagnostic::error(
                    "E0102",
                    format!(
                        "node `{}` is declared as `{}` but `{}` is a {}",
                        n.id,
                        n.kind.as_str(),
                        n.package,
                        describe_kind(m.kind)
                    ),
                )
                .primary(n.kind_span, format!("declared `{}` here", n.kind.as_str()))
                .secondary(m.kind_span, format!("manifest says `{}`", m.kind.as_str()))
                .help(format!(
                    "write `node {} : {} \"{}\"`",
                    n.id,
                    m.kind.as_str(),
                    n.package
                )),
            );
        }
        if n.package.contains('/') && !is_local_version_ok(&r, ws, &n.package) {
            // Reported inside resolve when a requirement exists; nothing more here.
        }
        resolved.push(Some(ResolvedNode {
            manifest: m,
            version: r.version,
            path: r.dir,
        }));
    }

    // 2. Parameters.
    let mut params_json: Vec<Json> = Vec::new();
    for (n, r) in p.nodes.iter().zip(resolved.iter()) {
        let r = match r {
            Some(r) => r,
            None => {
                params_json.push(Json::object());
                continue;
            }
        };
        let mut obj = Json::object();
        for a in &n.params {
            let decl = match r.manifest.param(&a.name.name) {
                Some(d) => d,
                None => {
                    let known: Vec<&str> =
                        r.manifest.params.iter().map(|p| p.name.as_str()).collect();
                    let mut d = Diagnostic::error(
                        "E0201",
                        format!(
                            "node `{}` (`{}`) has no parameter `{}`",
                            n.id, n.package, a.name.name
                        ),
                    )
                    .primary(a.name.span, "unknown parameter");
                    if let Some(s) = closest(&a.name.name, &known) {
                        d = d.help(format!("did you mean `{}`?", s));
                    } else if known.is_empty() {
                        d = d.help(format!("`{}` declares no parameters", n.package));
                    } else {
                        d = d.help(format!(
                            "parameters of `{}`: {}",
                            n.package,
                            known.join(", ")
                        ));
                    }
                    diags.push(d);
                    continue;
                }
            };
            let mut d = Diagnostics::new();
            let value = to_json(&a.value, decl.ty == ParamType::Enum, &mut d);
            if !d.is_empty() {
                diags.extend(d);
                continue;
            }
            match check_param_value(decl, &value) {
                Ok(()) => obj.set(&a.name.name, value),
                Err(err) => {
                    let code = match err {
                        ParamError::Type(_) => "E0202",
                        ParamError::Range(_) => "E0204",
                    };
                    let mut diag = Diagnostic::error(
                        code,
                        format!(
                            "parameter `{}` of node `{}` {}",
                            a.name.name,
                            n.id,
                            err.message()
                        ),
                    )
                    .primary(a.value.span(), "");
                    diag = diag.secondary(
                        decl.span,
                        format!("declared as {} in `{}`", decl.ty.as_str(), n.package),
                    );
                    if let Some(u) = &decl.unit {
                        diag = diag.help(format!("unit: {}", u));
                    }
                    diags.push(diag);
                }
            }
        }
        for decl in &r.manifest.params {
            if obj.get(&decl.name).is_some() {
                continue;
            }
            match &decl.default {
                Some(d) => obj.set(&decl.name, d.clone()),
                None => diags.push(
                    Diagnostic::error(
                        "E0203",
                        format!(
                            "node `{}` is missing required parameter `{}`",
                            n.id, decl.name
                        ),
                    )
                    .primary(n.span, format!("add `{} = ...`", decl.name))
                    .secondary(decl.span, "declared here without a default")
                    .help(if decl.description.is_empty() {
                        format!("`{}` is {}", decl.name, decl.ty.as_str())
                    } else {
                        decl.description.clone()
                    }),
                ),
            }
        }
        params_json.push(obj);
    }

    // 3. Ports and maps.
    let node_index: HashMap<&str, usize> = p
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut inputs: Vec<Vec<GInput>> = Vec::new();
    let mut outputs: Vec<Vec<GOutput>> = Vec::new();
    for (n, r) in p.nodes.iter().zip(resolved.iter()) {
        match r {
            Some(r) => {
                inputs.push(
                    r.manifest
                        .inputs
                        .iter()
                        .enumerate()
                        .map(|(i, d)| GInput {
                            name: d.name.clone(),
                            index: i as u16,
                            ty: d.ty,
                            sources: Vec::new(),
                        })
                        .collect(),
                );
                outputs.push(
                    r.manifest
                        .outputs
                        .iter()
                        .enumerate()
                        .map(|(i, d)| GOutput {
                            name: d.name.clone(),
                            index: i as u16,
                            ty: d.ty,
                            stream: format!("{}.{}", n.id, d.name),
                        })
                        .collect(),
                );
            }
            None => {
                inputs.push(Vec::new());
                outputs.push(Vec::new());
            }
        }
    }

    let mut edges: Vec<Edge> = Vec::new();
    let mut output_used: Vec<Vec<bool>> = outputs.iter().map(|o| vec![false; o.len()]).collect();
    for m in &p.maps {
        for from in &m.from {
            for to in &m.to {
                // Resolve endpoints.
                let (fi, fo) = match endpoint_output(from, &node_index, &resolved, diags, root, sm)
                {
                    Some(v) => v,
                    None => continue,
                };
                let (ti, tin) = match endpoint_input(to, &node_index, &resolved, diags) {
                    Some(v) => v,
                    None => continue,
                };
                let fdecl = &resolved[fi].as_ref().unwrap().manifest.outputs[fo];
                let tdecl = &resolved[ti].as_ref().unwrap().manifest.inputs[tin];
                if fdecl.ty != tdecl.ty {
                    diags.push(
                        Diagnostic::error("E0303", "port types differ")
                            .primary(to.span, format!("input port `{}` of `{}` ({}) is `{}`", tdecl.name, to.node.name, p.nodes[ti].package, tdecl.ty.as_str()))
                            .secondary(from.span, format!("output port `{}` of `{}` ({}) is `{}`", fdecl.name, from.node.name, p.nodes[fi].package, fdecl.ty.as_str()))
                            .help(match tdecl.ty {
                                PortType::Signal => "route events to an event-capable input, or convert with psl/to-signal",
                                PortType::Event => "route signals to a signal input, or detect events with a module such as psl/threshold",
                            }),
                    );
                    continue;
                }
                // Static constraints.
                if let (Channels::Exactly(a), Channels::Exactly(b)) =
                    (&fdecl.channels, &tdecl.channels)
                {
                    if a != b {
                        diags.push(
                            Diagnostic::error("E0304", "channel counts differ")
                                .primary(
                                    to.span,
                                    format!(
                                        "`{}.{}` expects {} channel(s)",
                                        to.node.name, tdecl.name, b
                                    ),
                                )
                                .secondary(
                                    from.span,
                                    format!(
                                        "`{}.{}` produces {} channel(s)",
                                        from.node.name, fdecl.name, a
                                    ),
                                )
                                .help("select channels with psl/select before this input"),
                        );
                        continue;
                    }
                }
                if let (Only::Of(a), Only::Of(b)) = (&fdecl.modality, &tdecl.modality) {
                    if !a.iter().any(|m| b.contains(m)) {
                        diags.push(
                            Diagnostic::error("E0304", "modalities are incompatible")
                                .primary(
                                    to.span,
                                    format!(
                                        "`{}.{}` accepts {}",
                                        to.node.name,
                                        tdecl.name,
                                        b.join(", ")
                                    ),
                                )
                                .secondary(
                                    from.span,
                                    format!(
                                        "`{}.{}` produces {}",
                                        from.node.name,
                                        fdecl.name,
                                        a.join(", ")
                                    ),
                                ),
                        );
                        continue;
                    }
                }
                if let (Only::Of(a), Only::Of(b)) = (&fdecl.labels, &tdecl.labels) {
                    if !a.iter().any(|m| b.contains(m)) {
                        diags.push(
                            Diagnostic::error("E0304", "event labels are incompatible")
                                .primary(
                                    to.span,
                                    format!(
                                        "`{}.{}` expects labels {}",
                                        to.node.name,
                                        tdecl.name,
                                        b.join(", ")
                                    ),
                                )
                                .secondary(
                                    from.span,
                                    format!(
                                        "`{}.{}` emits labels {}",
                                        from.node.name,
                                        fdecl.name,
                                        a.join(", ")
                                    ),
                                ),
                        );
                        continue;
                    }
                }
                let stream = outputs[fi][fo].stream.clone();
                let target = &mut inputs[ti][tin];
                if target.sources.contains(&stream) {
                    continue; // duplicate map, harmless
                }
                if !tdecl.multiple && !target.sources.is_empty() {
                    diags.push(
                        Diagnostic::error(
                            "E0306",
                            format!(
                                "input port `{}.{}` accepts a single source",
                                to.node.name, tdecl.name
                            ),
                        )
                        .primary(to.span, format!("already fed by `{}`", target.sources[0]))
                        .secondary(tdecl.span, "declared `multiple = false`")
                        .help("merge the sources with psl/merge first"),
                    );
                    continue;
                }
                target.sources.push(stream);
                output_used[fi][fo] = true;
                edges.push(Edge {
                    from_node: from.node.name.clone(),
                    from_port: fdecl.name.clone(),
                    to_node: to.node.name.clone(),
                    to_port: tdecl.name.clone(),
                    span: m.span,
                });
            }
        }
    }

    // 4. Unconnected required inputs, unused outputs, dynamic outputs, cycles.
    for (i, n) in p.nodes.iter().enumerate() {
        let r = match &resolved[i] {
            Some(r) => r,
            None => continue,
        };
        for (pi, decl) in r.manifest.inputs.iter().enumerate() {
            if decl.required && inputs[i][pi].sources.is_empty() {
                diags.push(
                    Diagnostic::error(
                        "E0305",
                        format!("required input `{}.{}` has no source", n.id, decl.name),
                    )
                    .primary(
                        n.id_span,
                        format!(
                            "`{}` needs `map <node>.<port> -> {}.{}`",
                            n.id, n.id, decl.name
                        ),
                    )
                    .secondary(decl.span, "declared required here"),
                );
            }
        }
        for (po, decl) in r.manifest.outputs.iter().enumerate() {
            if !output_used[i][po] {
                diags.push(
                    Diagnostic::warning("W0001", "output port is mapped nowhere").primary(
                        n.id_span,
                        format!("output `{}` of `{}` has no consumer", decl.name, n.id),
                    ),
                );
            }
        }
        if r.manifest.dynamic_outputs {
            diags.push(
                Diagnostic::warning(
                    "W0002",
                    format!(
                        "`{}` declares dynamic outputs; maps from it are checked at run time only",
                        n.id
                    ),
                )
                .primary(n.package_span, ""),
            );
        }
    }
    report_cycles(
        &p.nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>(),
        &edges,
        diags,
    );

    if diags.has_errors() {
        return None;
    }
    let nodes = p
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let r = resolved[i].take().unwrap();
            GNode {
                id: n.id.clone(),
                kind: n.kind,
                package: n.package.clone(),
                version: r.version,
                display_path: display_path(root, &r.path),
                path: r.path,
                params: std::mem::replace(&mut params_json[i], Json::Null),
                runtime: n.runtime.clone(),
                manifest: r.manifest,
                inputs: std::mem::take(&mut inputs[i]),
                outputs: std::mem::take(&mut outputs[i]),
            }
        })
        .collect();
    Some(Graph {
        name: p.name.clone(),
        file: display_path(root, pipeline_path),
        description: p.description.clone(),
        runtime: p.runtime.clone(),
        nodes,
        maps: edges,
    })
}

fn is_local_version_ok(_r: &resolve::Resolved, _ws: Option<&Workspace>, _pkg: &str) -> bool {
    true
}

fn describe_kind(k: Kind) -> &'static str {
    match k {
        Kind::Input => "an input driver",
        Kind::Module => "a module",
        Kind::Output => "an output driver",
    }
}

fn endpoint_output(
    e: &Endpoint,
    index: &HashMap<&str, usize>,
    resolved: &[Option<ResolvedNode>],
    diags: &mut Diagnostics,
    _root: &Path,
    _sm: &SourceMap,
) -> Option<(usize, usize)> {
    let ni = match index.get(e.node.name.as_str()) {
        Some(i) => *i,
        None => {
            diags.push(unknown_node(e, index));
            return None;
        }
    };
    let r = resolved[ni].as_ref()?; // earlier error already reported
    if let Some((pi, _)) = r.manifest.output(&e.port.name) {
        return Some((ni, pi));
    }
    if r.manifest.input(&e.port.name).is_some() {
        diags.push(
            Diagnostic::error(
                "E0302",
                format!(
                    "`{}.{}` is an input port, but a map source must be an output port",
                    e.node.name, e.port.name
                ),
            )
            .primary(e.span, "input port used as a source")
            .help("maps read `map <output> -> <input>`"),
        );
        return None;
    }
    diags.push(unknown_port(e, r, true));
    None
}

fn endpoint_input(
    e: &Endpoint,
    index: &HashMap<&str, usize>,
    resolved: &[Option<ResolvedNode>],
    diags: &mut Diagnostics,
) -> Option<(usize, usize)> {
    let ni = match index.get(e.node.name.as_str()) {
        Some(i) => *i,
        None => {
            diags.push(unknown_node(e, index));
            return None;
        }
    };
    let r = resolved[ni].as_ref()?;
    if let Some((pi, _)) = r.manifest.input(&e.port.name) {
        return Some((ni, pi));
    }
    if r.manifest.output(&e.port.name).is_some() {
        diags.push(
            Diagnostic::error(
                "E0302",
                format!(
                    "`{}.{}` is an output port, but a map target must be an input port",
                    e.node.name, e.port.name
                ),
            )
            .primary(e.span, "output port used as a target")
            .help("maps read `map <output> -> <input>`"),
        );
        return None;
    }
    diags.push(unknown_port(e, r, false));
    None
}

fn unknown_node(e: &Endpoint, index: &HashMap<&str, usize>) -> Diagnostic {
    let known: Vec<&str> = index.keys().copied().collect();
    let mut d = Diagnostic::error("E0301", format!("unknown node `{}`", e.node.name))
        .primary(e.node.span, "no node with this id");
    if let Some(s) = closest(&e.node.name, &known) {
        d = d.help(format!("did you mean `{}`?", s));
    }
    d
}

fn unknown_port(e: &Endpoint, r: &ResolvedNode, output: bool) -> Diagnostic {
    let ports: Vec<&str> = if output {
        r.manifest.outputs.iter().map(|p| p.name.as_str()).collect()
    } else {
        r.manifest.inputs.iter().map(|p| p.name.as_str()).collect()
    };
    let what = if output { "output" } else { "input" };
    let mut d = Diagnostic::error(
        "E0301",
        format!("`{}` has no {} port `{}`", e.node.name, what, e.port.name),
    )
    .primary(e.port.span, format!("unknown {} port", what));
    if ports.is_empty() {
        d = d.help(format!("`{}` has no {} ports", r.manifest.name, what));
    } else if let Some(s) = closest(&e.port.name, &ports) {
        d = d.help(format!("did you mean `{}.{}`?", e.node.name, s));
    } else {
        d = d.help(format!(
            "{} ports of `{}`: {}",
            what,
            r.manifest.name,
            ports.join(", ")
        ));
    }
    d
}

fn report_cycles(ids: &[String], edges: &[Edge], diags: &mut Diagnostics) {
    let index: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); ids.len()]; // (to, edge index)
    for (ei, e) in edges.iter().enumerate() {
        adj[index[e.from_node.as_str()]].push((index[e.to_node.as_str()], ei));
    }
    #[derive(Clone, Copy, PartialEq)]
    enum C {
        White,
        Grey,
        Black,
    }
    let mut color = vec![C::White; ids.len()];
    let mut stack: Vec<usize> = Vec::new();
    let mut reported = Vec::new();
    fn dfs(
        u: usize,
        adj: &[Vec<(usize, usize)>],
        color: &mut [C],
        stack: &mut Vec<usize>,
        reported: &mut Vec<(usize, Vec<usize>)>,
    ) {
        color[u] = C::Grey;
        stack.push(u);
        for &(v, ei) in &adj[u] {
            match color[v] {
                C::White => dfs(v, adj, color, stack, reported),
                C::Grey => {
                    let start = stack.iter().position(|&x| x == v).unwrap_or(0);
                    reported.push((ei, stack[start..].to_vec()));
                }
                C::Black => {}
            }
        }
        stack.pop();
        color[u] = C::Black;
    }
    for i in 0..ids.len() {
        if color[i] == C::White {
            dfs(i, &adj, &mut color, &mut stack, &mut reported);
        }
    }
    for (ei, cycle) in reported {
        let mut path: Vec<&str> = cycle.iter().map(|&i| ids[i].as_str()).collect();
        path.push(path[0]);
        diags.push(
            Diagnostic::warning(
                "W0003",
                format!("map creates a cycle: {}", path.join(" -> ")),
            )
            .primary(edges[ei].span, "this map closes the loop")
            .help("cycles are allowed; make sure the loop cannot amplify data without bound"),
        );
    }
}

/// The closest of `candidates` to `name` by edit distance, if reasonably close.
pub fn closest<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for c in candidates {
        let d = edit_distance(name, c);
        if d <= (name.len().max(c.len()) / 3).max(1) && best.map(|b| d < b.1).unwrap_or(true) {
            best = Some((c, d));
        }
    }
    best.map(|b| b.0)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Convenience used by tests and the CLI: a human-readable one-liner for a value.
pub fn describe(j: &Json) -> String {
    describe_json(j)
}

/// Spans of all nodes for tooling that needs them.
pub fn node_spans(p: &Pipeline) -> Vec<(String, Span)> {
    p.nodes.iter().map(|n| (n.id.clone(), n.span)).collect()
}
