//! The pipeline document, docs/04 §Pipeline schema.

use crate::diag::{Diagnostic, Diagnostics};
use crate::json::Json;
use crate::model::reader::Body;
use crate::syntax::{Attribute, Block, Document, FileId, Kind, Label, MapStmt, Span, Statement};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restart {
    Never,
    OnFailure,
    Always,
}

impl Restart {
    pub fn as_str(&self) -> &'static str {
        match self {
            Restart::Never => "never",
            Restart::OnFailure => "on-failure",
            Restart::Always => "always",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnFull {
    DropOldest,
    Block,
}

impl OnFull {
    pub fn as_str(&self) -> &'static str {
        match self {
            OnFull::DropOldest => "drop_oldest",
            OnFull::Block => "block",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeRuntime {
    pub restart: Restart,
    pub queue: i64,
    pub on_full: OnFull,
    pub env: Vec<(String, String)>,
}

impl Default for NodeRuntime {
    fn default() -> NodeRuntime {
        NodeRuntime {
            restart: Restart::Never,
            queue: 64,
            on_full: OnFull::DropOldest,
            env: Vec::new(),
        }
    }
}

impl NodeRuntime {
    pub fn to_json(&self) -> Json {
        Json::object()
            .with("restart", self.restart.as_str())
            .with("queue", self.queue)
            .with("on_full", self.on_full.as_str())
            .with(
                "env",
                Json::Object(
                    self.env
                        .iter()
                        .map(|(k, v)| (k.clone(), Json::Str(v.clone())))
                        .collect(),
                ),
            )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PipelineRuntime {
    pub ready_timeout_s: f64,
    pub stop_grace_s: f64,
    pub stop_when_inputs_end: bool,
    pub record: Option<String>,
}

impl Default for PipelineRuntime {
    fn default() -> PipelineRuntime {
        PipelineRuntime {
            ready_timeout_s: 10.0,
            stop_grace_s: 5.0,
            stop_when_inputs_end: false,
            record: None,
        }
    }
}

impl PipelineRuntime {
    pub fn to_json(&self) -> Json {
        Json::object()
            .with("ready_timeout_s", self.ready_timeout_s)
            .with("stop_grace_s", self.stop_grace_s)
            .with("stop_when_inputs_end", self.stop_when_inputs_end)
            .with("record", self.record.clone())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PNode {
    pub id: String,
    pub id_span: Span,
    pub kind: Kind,
    pub kind_span: Span,
    pub package: String,
    pub package_span: Span,
    /// Raw parameter attributes; typed against the manifest by the checker.
    pub params: Vec<Attribute>,
    pub runtime: NodeRuntime,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pipeline {
    pub name: String,
    pub name_span: Span,
    pub description: String,
    pub runtime: PipelineRuntime,
    pub nodes: Vec<PNode>,
    pub maps: Vec<MapStmt>,
    pub file: FileId,
}

pub fn parse_pipeline(doc: &Document, file: FileId) -> (Option<Pipeline>, Diagnostics) {
    let mut diags = Diagnostics::new();
    let mut header: Option<&Block> = None;
    let mut nodes = Vec::new();
    let mut maps = Vec::new();
    for (i, s) in doc.statements.iter().enumerate() {
        match s {
            Statement::Block(b) if b.keyword.name == "pipeline" => {
                if header.is_some() {
                    diags.push(
                        Diagnostic::error(
                            "E0004",
                            "a pipeline file has exactly one `pipeline` block",
                        )
                        .primary(b.keyword.span, "second `pipeline` block"),
                    );
                } else {
                    if i != 0 {
                        diags.push(
                            Diagnostic::error(
                                "E0002",
                                "the `pipeline` block must be the first statement in the file",
                            )
                            .primary(b.keyword.span, "move this to the top"),
                        );
                    }
                    header = Some(b);
                }
            }
            Statement::Node(n) => nodes.push(n),
            Statement::Map(m) => maps.push(m.clone()),
            Statement::Block(b) => diags.push(
                Diagnostic::error(
                    "E0002",
                    format!("unknown block `{}` in a pipeline file", b.keyword.name),
                )
                .primary(b.keyword.span, "expected `pipeline`, `node` or `map`"),
            ),
            Statement::Attribute(a) => diags.push(
                Diagnostic::error(
                    "E0002",
                    format!(
                        "unexpected attribute `{}` at the top of a pipeline file",
                        a.name.name
                    ),
                )
                .primary(a.name.span, "attributes belong inside a block"),
            ),
        }
    }
    let header = match header {
        Some(h) => h,
        None => {
            diags.push(
                Diagnostic::error(
                    "E0006",
                    "pipeline file has no `pipeline \"name\" { ... }` block",
                )
                .primary(
                    doc.statements
                        .first()
                        .map(|s| s.span())
                        .unwrap_or(Span::new(file, 0, 0)),
                    "add `pipeline \"name\" {}` at the top",
                ),
            );
            return (None, diags);
        }
    };
    let (name, name_span) = match header.labels.as_slice() {
        [Label::Str(s)] => (s.value.clone(), s.span),
        _ => {
            diags.push(
                Diagnostic::error(
                    "E0006",
                    "`pipeline` needs a name in quotes: `pipeline \"name\" { ... }`",
                )
                .primary(header.keyword.span, ""),
            );
            return (None, diags);
        }
    };
    let mut body = Body::new(&header.body, "the `pipeline` block", header.span);
    let description = body.str("description").map(|v| v.0).unwrap_or_default();
    let mut runtime = PipelineRuntime::default();
    let rblocks = body.blocks("runtime");
    if rblocks.len() > 1 {
        body.diags.push(
            Diagnostic::error(
                "E0004",
                "more than one `runtime` block in the `pipeline` block",
            )
            .primary(rblocks[1].keyword.span, ""),
        );
    }
    if let Some(r) = rblocks.first() {
        let mut rb = Body::new(&r.body, "the pipeline `runtime` block", r.span);
        if let Some((v, span)) = rb.num("ready_timeout_s") {
            if v <= 0.0 {
                rb.diags.push(
                    Diagnostic::error("E0003", "`ready_timeout_s` must be positive")
                        .primary(span, ""),
                );
            } else {
                runtime.ready_timeout_s = v;
            }
        }
        if let Some((v, span)) = rb.num("stop_grace_s") {
            if v < 0.0 {
                rb.diags.push(
                    Diagnostic::error("E0003", "`stop_grace_s` cannot be negative")
                        .primary(span, ""),
                );
            } else {
                runtime.stop_grace_s = v;
            }
        }
        if let Some((v, _)) = rb.bool("stop_when_inputs_end") {
            runtime.stop_when_inputs_end = v;
        }
        runtime.record = rb.str("record").map(|v| v.0);
        rb.finish();
        body.diags.extend(rb.diags);
    }
    body.finish();
    diags.extend(body.diags);

    let mut pnodes: Vec<PNode> = Vec::new();
    for n in nodes {
        if let Some(prev) = pnodes.iter().find(|p| p.id == n.id.name) {
            diags.push(
                Diagnostic::error("E0103", format!("duplicate node id `{}`", n.id.name))
                    .primary(n.id.span, "declared again here")
                    .secondary(prev.id_span, "first declared here"),
            );
            continue;
        }
        let mut body = Body::new(&n.body, format!("node `{}`", n.id.name), n.span);
        let params: Vec<Attribute> = body.all_attrs().into_iter().cloned().collect();
        let mut nr = NodeRuntime::default();
        let rblocks = body.blocks("runtime");
        if rblocks.len() > 1 {
            body.diags.push(
                Diagnostic::error(
                    "E0004",
                    format!("more than one `runtime` block in node `{}`", n.id.name),
                )
                .primary(rblocks[1].keyword.span, ""),
            );
        }
        if let Some(r) = rblocks.first() {
            let mut rb = Body::new(
                &r.body,
                format!("the `runtime` block of node `{}`", n.id.name),
                r.span,
            );
            if let Some((v, _)) = rb.symbol("restart", &["never", "on-failure", "always"]) {
                nr.restart = match v.as_str() {
                    "always" => Restart::Always,
                    "on-failure" => Restart::OnFailure,
                    _ => Restart::Never,
                };
            }
            if let Some((v, span)) = rb.int("queue") {
                if v < 1 {
                    rb.diags.push(
                        Diagnostic::error("E0003", "`queue` must be at least 1").primary(span, ""),
                    );
                } else {
                    nr.queue = v;
                }
            }
            if let Some((v, _)) = rb.symbol("on_full", &["drop_oldest", "block"]) {
                nr.on_full = if v == "block" {
                    OnFull::Block
                } else {
                    OnFull::DropOldest
                };
            }
            if let Some((env, _)) = rb.str_object("env") {
                nr.env = env;
            }
            rb.finish();
            body.diags.extend(rb.diags);
        }
        body.finish();
        diags.extend(body.diags);
        pnodes.push(PNode {
            id: n.id.name.clone(),
            id_span: n.id.span,
            kind: n.kind,
            kind_span: n.kind_span,
            package: n.package.value.clone(),
            package_span: n.package.span,
            params,
            runtime: nr,
            span: n.span,
        });
    }
    if diags.has_errors() {
        return (None, diags);
    }
    (
        Some(Pipeline {
            name,
            name_span,
            description,
            runtime,
            nodes: pnodes,
            maps,
            file,
        }),
        diags,
    )
}
