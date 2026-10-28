//! The node manifest (`node.psl`), docs/04 §Manifest schema.

use crate::diag::{Diagnostic, Diagnostics};
use crate::json::Json;
use crate::model::reader::{list_quoted, Body};
use crate::model::value::{describe_json, to_json};
use crate::syntax::{Block, Document, FileId, Kind, Label, Span, Statement, Value};
use crate::util::Version;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortType {
    Signal,
    Event,
}

impl PortType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PortType::Signal => "signal",
            PortType::Event => "event",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Channels {
    Any,
    Exactly(i64),
}

/// A constraint that is either open or a set of allowed names (modality, dtype, labels).
#[derive(Clone, Debug, PartialEq)]
pub enum Only {
    Any,
    Of(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Srate {
    Any,
    Exactly(f64),
    Range(f64, f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PortDecl {
    pub name: String,
    pub span: Span,
    pub ty: PortType,
    pub description: String,
    pub required: bool,
    pub multiple: bool,
    pub channels: Channels,
    pub modality: Only,
    pub dtype: Only,
    pub srate: Srate,
    pub labels: Only,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamType {
    Number,
    Integer,
    String,
    Boolean,
    Enum,
    List,
}

impl ParamType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ParamType::Number => "number",
            ParamType::Integer => "integer",
            ParamType::String => "string",
            ParamType::Boolean => "boolean",
            ParamType::Enum => "enum",
            ParamType::List => "list",
        }
    }
    fn parse(s: &str) -> Option<ParamType> {
        Some(match s {
            "number" => ParamType::Number,
            "integer" => ParamType::Integer,
            "string" => ParamType::String,
            "boolean" => ParamType::Boolean,
            "enum" => ParamType::Enum,
            "list" => ParamType::List,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamDecl {
    pub name: String,
    pub span: Span,
    pub ty: ParamType,
    pub description: String,
    pub default: Option<Json>,
    pub mutable: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub values: Vec<Json>,
    pub of: Option<ParamType>,
    pub unit: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Runtime {
    Python {
        entry: String,
        requires: Vec<String>,
        python: Option<String>,
    },
    Exec {
        command: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub name_span: Span,
    pub kind: Kind,
    pub kind_span: Span,
    pub version: String,
    pub description: String,
    pub author: Option<String>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub dynamic_outputs: bool,
    pub runtime: Runtime,
    pub inputs: Vec<PortDecl>,
    pub outputs: Vec<PortDecl>,
    pub params: Vec<ParamDecl>,
    pub file: FileId,
    pub span: Span,
}

impl Manifest {
    pub fn input(&self, name: &str) -> Option<(usize, &PortDecl)> {
        self.inputs.iter().enumerate().find(|(_, p)| p.name == name)
    }
    pub fn output(&self, name: &str) -> Option<(usize, &PortDecl)> {
        self.outputs
            .iter()
            .enumerate()
            .find(|(_, p)| p.name == name)
    }
    pub fn param(&self, name: &str) -> Option<&ParamDecl> {
        self.params.iter().find(|p| p.name == name)
    }

    /// The manifest as JSON, for `psl info` and the canonical model.
    pub fn to_json(&self) -> Json {
        let port = |p: &PortDecl| {
            let mut j = Json::object()
                .with("name", p.name.as_str())
                .with("type", p.ty.as_str())
                .with("description", p.description.as_str())
                .with("required", p.required)
                .with("multiple", p.multiple);
            match &p.channels {
                Channels::Any => j.set("channels", "any"),
                Channels::Exactly(n) => j.set("channels", *n),
            }
            for (k, v) in [
                ("modality", &p.modality),
                ("dtype", &p.dtype),
                ("labels", &p.labels),
            ] {
                match v {
                    Only::Any => j.set(k, "any"),
                    Only::Of(items) => j.set(
                        k,
                        items
                            .iter()
                            .map(|s| Json::Str(s.clone()))
                            .collect::<Vec<_>>(),
                    ),
                }
            }
            match &p.srate {
                Srate::Any => j.set("srate", "any"),
                Srate::Exactly(v) => j.set("srate", *v),
                Srate::Range(a, b) => j.set("srate", vec![Json::Float(*a), Json::Float(*b)]),
            }
            j
        };
        let runtime = match &self.runtime {
            Runtime::Python {
                entry,
                requires,
                python,
            } => Json::object()
                .with("kind", "python")
                .with("entry", entry.as_str())
                .with(
                    "requires",
                    requires
                        .iter()
                        .map(|s| Json::Str(s.clone()))
                        .collect::<Vec<_>>(),
                )
                .with("python", python.clone()),
            Runtime::Exec { command } => Json::object().with("kind", "exec").with(
                "command",
                command
                    .iter()
                    .map(|s| Json::Str(s.clone()))
                    .collect::<Vec<_>>(),
            ),
        };
        Json::object()
            .with("name", self.name.as_str())
            .with("kind", self.kind.as_str())
            .with("version", self.version.as_str())
            .with("description", self.description.as_str())
            .with("author", self.author.clone())
            .with("license", self.license.clone())
            .with("homepage", self.homepage.clone())
            .with("dynamic_outputs", self.dynamic_outputs)
            .with("runtime", runtime)
            .with("inputs", self.inputs.iter().map(port).collect::<Vec<_>>())
            .with("outputs", self.outputs.iter().map(port).collect::<Vec<_>>())
            .with(
                "params",
                self.params
                    .iter()
                    .map(|p| {
                        Json::object()
                            .with("name", p.name.as_str())
                            .with("type", p.ty.as_str())
                            .with("description", p.description.as_str())
                            .with("default", p.default.clone())
                            .with("mutable", p.mutable)
                            .with("min", p.min)
                            .with("max", p.max)
                            .with("values", Json::Array(p.values.clone()))
                            .with("of", p.of.map(|t| t.as_str()))
                            .with("unit", p.unit.clone())
                    })
                    .collect::<Vec<_>>(),
            )
    }
}

const DTYPES: &[&str] = &["f32", "f64", "i16", "i32"];

/// Parse a manifest document. Returns the manifest when it is usable and all diagnostics found.
pub fn parse_manifest(doc: &Document, file: FileId) -> (Option<Manifest>, Diagnostics) {
    let mut diags = Diagnostics::new();
    let mut node_block: Option<&Block> = None;
    for s in &doc.statements {
        match s {
            Statement::Block(b) if b.keyword.name == "node" => {
                if node_block.is_some() {
                    diags.push(
                        Diagnostic::error("E0004", "a manifest has exactly one `node` block")
                            .primary(b.keyword.span, "second `node` block"),
                    );
                } else {
                    node_block = Some(b);
                }
            }
            other => diags.push(
                Diagnostic::error(
                    "E0002",
                    "only a `node \"name\" { ... }` block is allowed at the top of a manifest",
                )
                .primary(other.span(), ""),
            ),
        }
    }
    let b = match node_block {
        Some(b) => b,
        None => {
            diags.push(
                Diagnostic::error("E0006", "manifest has no `node \"name\" { ... }` block")
                    .at_path(""),
            );
            return (None, diags);
        }
    };
    let (name, name_span) = match b.labels.as_slice() {
        [Label::Str(s)] => (s.value.clone(), s.span),
        _ => {
            diags.push(
                Diagnostic::error(
                    "E0006",
                    "the `node` block needs its name in quotes: `node \"name\" { ... }`",
                )
                .primary(b.keyword.span, ""),
            );
            return (None, diags);
        }
    };
    let mut body = Body::new(&b.body, "a manifest `node` block", b.span);

    let kind = body
        .symbol("kind", &["input", "module", "output"])
        .and_then(|(k, s)| Kind::parse(&k).map(|k| (k, s)));
    if body.attr("kind").is_none() {
        body.missing("kind");
    }
    let version = body.req_str("version");
    if let Some((v, span)) = &version {
        if Version::parse(v).is_none() {
            body.diags.push(
                Diagnostic::error(
                    "E0003",
                    format!(
                        "`version` must be a semantic version like `0.1.0`, found `{}`",
                        v
                    ),
                )
                .primary(*span, ""),
            );
        }
    }
    let description = body.str("description").map(|v| v.0).unwrap_or_default();
    let author = body.str("author").map(|v| v.0);
    let license = body.str("license").map(|v| v.0);
    let homepage = body.str("homepage").map(|v| v.0);
    let dynamic_outputs = body.bool("dynamic_outputs");

    // runtime
    let runtimes = body.blocks("runtime");
    let runtime = match runtimes.as_slice() {
        [r] => parse_runtime(r, &mut body.diags),
        [] => {
            body.diags.push(
                Diagnostic::error("E0402", "manifest has no `runtime` block")
                    .primary(b.span, "add `runtime python { entry = \"main.py\" }`"),
            );
            None
        }
        [first, rest @ ..] => {
            for r in rest {
                body.diags.push(
                    Diagnostic::error("E0402", "manifest has more than one `runtime` block")
                        .primary(r.keyword.span, "second runtime")
                        .secondary(first.keyword.span, "first runtime"),
                );
            }
            None
        }
    };

    let mut inputs = Vec::new();
    for ib in body.blocks("input") {
        if let Some(p) = parse_port(ib, true, &mut body.diags) {
            inputs.push(p);
        }
    }
    let mut outputs = Vec::new();
    for ob in body.blocks("output") {
        if let Some(p) = parse_port(ob, false, &mut body.diags) {
            outputs.push(p);
        }
    }
    let mut params = Vec::new();
    for pb in body.blocks("param") {
        if let Some(p) = parse_param(pb, &mut body.diags) {
            params.push(p);
        }
    }
    body.finish();
    diags.extend(body.diags);

    // Duplicate names.
    for (i, p) in inputs.iter().chain(outputs.iter()).enumerate() {
        if let Some(prev) = inputs
            .iter()
            .chain(outputs.iter())
            .take(i)
            .find(|q| q.name == p.name)
        {
            diags.push(
                Diagnostic::error("E0004", format!("duplicate port name `{}`", p.name))
                    .primary(p.span, "given again here")
                    .secondary(prev.span, "first given here"),
            );
        }
    }
    for (i, p) in params.iter().enumerate() {
        if let Some(prev) = params.iter().take(i).find(|q| q.name == p.name) {
            diags.push(
                Diagnostic::error("E0004", format!("duplicate param name `{}`", p.name))
                    .primary(p.span, "given again here")
                    .secondary(prev.span, "first given here"),
            );
        }
    }

    let (kind, kind_span) = match kind {
        Some(k) => k,
        None => return (None, diags),
    };
    // Structural kind rules (E0401).
    match kind {
        Kind::Input => {
            for p in &inputs {
                diags.push(
                    Diagnostic::error("E0401", "an input driver cannot declare input ports")
                        .primary(p.span, "input port declared here")
                        .secondary(kind_span, "kind is `input`")
                        .help("input drivers only pull data from a device; if this node needs data from the pipeline it is a module"),
                );
            }
        }
        Kind::Output => {
            for p in &outputs {
                diags.push(
                    Diagnostic::error("E0401", "an output driver cannot declare output ports")
                        .primary(p.span, "output port declared here")
                        .secondary(kind_span, "kind is `output`")
                        .help("output drivers only deliver data outside the pipeline; if this node produces data it is a module"),
                );
            }
        }
        Kind::Module => {}
    }
    if let Some((true, span)) = dynamic_outputs {
        if kind != Kind::Input {
            diags.push(
                Diagnostic::error(
                    "E0003",
                    "`dynamic_outputs` is only allowed on input drivers",
                )
                .primary(span, ""),
            );
        }
    }
    let (version, _) = match version {
        Some(v) => v,
        None => return (None, diags),
    };
    let runtime = match runtime {
        Some(r) => r,
        None => return (None, diags),
    };
    if diags.has_errors() {
        return (None, diags);
    }
    (
        Some(Manifest {
            name,
            name_span,
            kind,
            kind_span,
            version,
            description,
            author,
            license,
            homepage,
            dynamic_outputs: dynamic_outputs.map(|v| v.0).unwrap_or(false),
            runtime,
            inputs,
            outputs,
            params,
            file,
            span: b.span,
        }),
        diags,
    )
}

fn parse_runtime(r: &Block, diags: &mut Diagnostics) -> Option<Runtime> {
    let label = match r.labels.as_slice() {
        [l] => l.text().to_string(),
        _ => {
            diags.push(
                Diagnostic::error(
                    "E0006",
                    "`runtime` needs a kind: `runtime python { ... }` or `runtime exec { ... }`",
                )
                .primary(r.keyword.span, ""),
            );
            return None;
        }
    };
    match label.as_str() {
        "python" => {
            let mut body = Body::new(&r.body, "a `runtime python` block", r.span);
            let entry = body.req_str("entry");
            let requires = body.str_list("requires").map(|v| v.0).unwrap_or_default();
            let python = body.str("python").map(|v| v.0);
            body.finish();
            let had_errors = body.diags.has_errors();
            diags.extend(body.diags);
            if had_errors {
                return None;
            }
            Some(Runtime::Python {
                entry: entry?.0,
                requires,
                python,
            })
        }
        "exec" => {
            let mut body = Body::new(&r.body, "a `runtime exec` block", r.span);
            let command = body.str_list("command");
            if command.is_none() && body.attr("command").is_none() {
                body.missing("command");
            }
            body.finish();
            let had_errors = body.diags.has_errors();
            diags.extend(body.diags);
            if had_errors {
                return None;
            }
            let (command, span) = command?;
            if command.is_empty() {
                diags.push(
                    Diagnostic::error("E0003", "`command` must have at least one element")
                        .primary(span, ""),
                );
                return None;
            }
            Some(Runtime::Exec { command })
        }
        other => {
            diags.push(
                Diagnostic::error(
                    "E0003",
                    format!("unknown runtime `{}`; expected `python` or `exec`", other),
                )
                .primary(r.labels[0].span(), ""),
            );
            None
        }
    }
}

fn block_name(b: &Block, what: &str, diags: &mut Diagnostics) -> Option<(String, Span)> {
    match b.labels.as_slice() {
        [Label::Str(s)] => Some((s.value.clone(), s.span)),
        _ => {
            diags.push(
                Diagnostic::error(
                    "E0006",
                    format!(
                        "`{}` needs a name in quotes: `{} \"name\" {{ ... }}`",
                        what, what
                    ),
                )
                .primary(b.keyword.span, ""),
            );
            None
        }
    }
}

fn parse_port(b: &Block, is_input: bool, diags: &mut Diagnostics) -> Option<PortDecl> {
    let what = if is_input { "input" } else { "output" };
    let (name, span) = block_name(b, what, diags)?;
    if !is_ident(&name) {
        diags.push(
            Diagnostic::error(
                "E0003",
                format!(
                    "port name `{}` must be an identifier (letters, digits, underscores)",
                    name
                ),
            )
            .primary(span, ""),
        );
    }
    let mut body = Body::new(&b.body, format!("an `{}` block", what), b.span);
    let ty = body.symbol("type", &["signal", "event"]);
    if body.attr("type").is_none() {
        body.missing("type");
    }
    let description = body.str("description").map(|v| v.0).unwrap_or_default();
    let required = body.bool("required").map(|v| v.0).unwrap_or(true);
    let multiple = body.bool("multiple").map(|v| v.0).unwrap_or(true);
    let channels = match body.attr("channels") {
        None => Channels::Any,
        Some(a) => match &a.value {
            Value::Int(n, span) if *n >= 1 => Channels::Exactly(*n),
            Value::Int(_, span) => {
                body.diags.push(
                    Diagnostic::error("E0003", "`channels` must be a positive integer or `any`")
                        .primary(*span, ""),
                );
                Channels::Any
            }
            Value::Symbol(s, _) if s == "any" => Channels::Any,
            other => {
                body.diags.push(
                    Diagnostic::error("E0003", "`channels` must be a positive integer or `any`")
                        .primary(other.span(), ""),
                );
                Channels::Any
            }
        },
    };
    let modality = parse_only(&mut body, "modality", None);
    let dtype = parse_only(&mut body, "dtype", Some(DTYPES));
    let labels = match body.attr("labels") {
        None => Only::Any,
        Some(a) => match &a.value {
            Value::Symbol(s, _) if s == "any" => Only::Any,
            Value::List(..) => body
                .str_list("labels")
                .map(|v| Only::Of(v.0))
                .unwrap_or(Only::Any),
            other => {
                body.diags.push(
                    Diagnostic::error("E0003", "`labels` must be a list of strings or `any`")
                        .primary(other.span(), ""),
                );
                Only::Any
            }
        },
    };
    let srate = match body.attr("srate") {
        None => Srate::Any,
        Some(a) => match &a.value {
            Value::Int(n, _) => Srate::Exactly(*n as f64),
            Value::Float(f, _) => Srate::Exactly(*f),
            Value::Symbol(s, _) if s == "any" => Srate::Any,
            Value::List(items, span) => {
                let nums: Vec<f64> = items
                    .iter()
                    .filter_map(|v| match v {
                        Value::Int(n, _) => Some(*n as f64),
                        Value::Float(f, _) => Some(*f),
                        _ => None,
                    })
                    .collect();
                if nums.len() == 2 && items.len() == 2 && nums[0] <= nums[1] {
                    Srate::Range(nums[0], nums[1])
                } else {
                    body.diags.push(
                        Diagnostic::error(
                            "E0003",
                            "`srate` range must be `[min, max]` with two numbers",
                        )
                        .primary(*span, ""),
                    );
                    Srate::Any
                }
            }
            other => {
                body.diags.push(
                    Diagnostic::error("E0003", "`srate` must be a number, `[min, max]` or `any`")
                        .primary(other.span(), ""),
                );
                Srate::Any
            }
        },
    };
    body.finish();
    let had_errors = body.diags.has_errors();
    diags.extend(body.diags);
    if had_errors {
        return None;
    }
    let ty = match ty?.0.as_str() {
        "signal" => PortType::Signal,
        _ => PortType::Event,
    };
    Some(PortDecl {
        name,
        span,
        ty,
        description,
        required,
        multiple,
        channels,
        modality,
        dtype,
        srate,
        labels,
    })
}

/// `name = any`, `name = foo`, `name = "foo"` or `name = [foo, "bar"]`.
fn parse_only(body: &mut Body, name: &str, allowed: Option<&[&str]>) -> Only {
    let a = match body.attr(name) {
        None => return Only::Any,
        Some(a) => a,
    };
    let mut items = Vec::new();
    let mut push = |s: &str, span: Span, diags: &mut Diagnostics| {
        if let Some(allowed) = allowed {
            if !allowed.contains(&s) {
                diags.push(
                    Diagnostic::error(
                        "E0003",
                        format!(
                            "`{}` must be one of {} or `any`",
                            name,
                            list_quoted(allowed)
                        ),
                    )
                    .primary(span, "not an allowed value"),
                );
                return;
            }
        }
        items.push(s.to_string());
    };
    match &a.value {
        Value::Symbol(s, _) if s == "any" => return Only::Any,
        Value::Symbol(s, span) | Value::Str(s, span) => push(s, *span, &mut body.diags),
        Value::List(list, _) => {
            for v in list {
                match v {
                    Value::Symbol(s, span) | Value::Str(s, span) => push(s, *span, &mut body.diags),
                    other => body.diags.push(
                        Diagnostic::error("E0003", format!("`{}` list must contain names", name))
                            .primary(other.span(), ""),
                    ),
                }
            }
        }
        other => body.diags.push(
            Diagnostic::error(
                "E0003",
                format!("`{}` must be a name, a list of names or `any`", name),
            )
            .primary(other.span(), ""),
        ),
    }
    Only::Of(items)
}

fn parse_param(b: &Block, diags: &mut Diagnostics) -> Option<ParamDecl> {
    let (name, span) = block_name(b, "param", diags)?;
    if !is_ident(&name) {
        diags.push(
            Diagnostic::error(
                "E0003",
                format!(
                    "param name `{}` must be an identifier (letters, digits, underscores)",
                    name
                ),
            )
            .primary(span, ""),
        );
    }
    let mut body = Body::new(&b.body, "a `param` block", b.span);
    let ty = body
        .symbol(
            "type",
            &["number", "integer", "string", "boolean", "enum", "list"],
        )
        .and_then(|(t, _)| ParamType::parse(&t));
    if body.attr("type").is_none() {
        body.missing("type");
    }
    let description = body.str("description").map(|v| v.0).unwrap_or_default();
    let mutable = body.bool("mutable").map(|v| v.0).unwrap_or(false);
    let min = body.num("min").map(|v| v.0);
    let max = body.num("max").map(|v| v.0);
    let unit = body.str("unit").map(|v| v.0);
    let of = body
        .symbol("of", &["number", "integer", "string", "boolean"])
        .and_then(|(t, _)| ParamType::parse(&t));
    let values: Vec<Json> = match body.attr("values") {
        None => Vec::new(),
        Some(a) => match &a.value {
            Value::List(items, _) => items
                .iter()
                .map(|v| to_json(v, true, &mut body.diags))
                .collect(),
            other => {
                body.diags.push(
                    Diagnostic::error("E0003", "`values` must be a list").primary(other.span(), ""),
                );
                Vec::new()
            }
        },
    };
    let default_attr = body.attr("default").cloned();
    body.finish();
    let had_errors = body.diags.has_errors();
    diags.extend(body.diags);
    if had_errors {
        return None;
    }
    let ty = ty?;
    if ty == ParamType::Enum && values.is_empty() {
        diags.push(
            Diagnostic::error(
                "E0006",
                format!("enum param `{}` needs `values = [...]`", name),
            )
            .primary(b.span, ""),
        );
        return None;
    }
    let mut decl = ParamDecl {
        name,
        span,
        ty,
        description,
        default: None,
        mutable,
        min,
        max,
        values,
        of,
        unit,
    };
    if let Some(a) = default_attr {
        let mut d = Diagnostics::new();
        let j = to_json(&a.value, ty == ParamType::Enum, &mut d);
        let ok = d.is_empty();
        diags.extend(d);
        if ok {
            if let Err(e) = check_param_value(&decl, &j) {
                diags.push(
                    Diagnostic::error(
                        "E0003",
                        format!("default of param `{}` {}", decl.name, e.message()),
                    )
                    .primary(a.value.span(), ""),
                );
                return None;
            }
            decl.default = Some(j);
        } else {
            return None;
        }
    }
    Some(decl)
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Check a JSON value against a parameter declaration. The error text completes
/// the sentence "param `x` ...".
/// Why a parameter value was rejected.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamError {
    /// Wrong type (E0202).
    Type(String),
    /// Outside `min`/`max` or not in `values` (E0204).
    Range(String),
}

impl ParamError {
    pub fn message(&self) -> &str {
        match self {
            ParamError::Type(s) | ParamError::Range(s) => s,
        }
    }
}

pub fn check_param_value(decl: &ParamDecl, value: &Json) -> Result<(), ParamError> {
    fn scalar(ty: ParamType, value: &Json) -> Result<(), ParamError> {
        let ok = match ty {
            ParamType::Number => matches!(value, Json::Int(_) | Json::Float(_)),
            ParamType::Integer => matches!(value, Json::Int(_)),
            ParamType::String => matches!(value, Json::Str(_)),
            ParamType::Boolean => matches!(value, Json::Bool(_)),
            ParamType::Enum => matches!(value, Json::Str(_)),
            ParamType::List => matches!(value, Json::Array(_)),
        };
        if ok {
            Ok(())
        } else {
            Err(ParamError::Type(format!(
                "must be {} {}, found {}",
                article(ty.as_str()),
                ty.as_str(),
                value.type_name()
            )))
        }
    }
    scalar(decl.ty, value)?;
    match decl.ty {
        ParamType::Number | ParamType::Integer => {
            let v = value.as_f64().unwrap();
            if let Some(min) = decl.min {
                if v < min {
                    return Err(ParamError::Range(format!(
                        "must be at least {}, found {}",
                        min,
                        describe_json(value)
                    )));
                }
            }
            if let Some(max) = decl.max {
                if v > max {
                    return Err(ParamError::Range(format!(
                        "must be at most {}, found {}",
                        max,
                        describe_json(value)
                    )));
                }
            }
        }
        ParamType::Enum => {
            if !decl.values.contains(value) {
                let allowed: Vec<String> = decl.values.iter().map(describe_json).collect();
                return Err(ParamError::Range(format!(
                    "must be one of {}, found {}",
                    allowed.join(", "),
                    describe_json(value)
                )));
            }
        }
        ParamType::List => {
            if let Some(of) = decl.of {
                for (i, item) in value.as_array().unwrap().iter().enumerate() {
                    if let Err(e) = scalar(of, item) {
                        return Err(ParamError::Type(format!("element {} {}", i, e.message())));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn article(word: &str) -> &'static str {
    if word.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parse_document;

    fn parse(src: &str) -> (Option<Manifest>, Diagnostics) {
        let doc = parse_document(FileId(0), src).unwrap();
        parse_manifest(&doc, FileId(0))
    }

    const GOOD: &str = r#"
node "bandpass" {
  kind = module
  version = "0.1.0"
  description = "x"
  runtime python { entry = "main.py", requires = ["numpy>=1.26"] }
  input "in" { type = signal, channels = any, modality = [ecg, "eeg"] }
  output "out" { type = signal }
  param "low" { type = number, default = 0.5, min = 0, unit = "Hz", mutable = true }
  param "mode" { type = enum, values = ["a", "b"], default = a }
  param "names" { type = list, of = string, default = ["x"] }
}"#;

    #[test]
    fn good_manifest() {
        let (m, d) = parse(GOOD);
        assert!(d.is_empty(), "{:?}", d.items);
        let m = m.unwrap();
        assert_eq!(m.name, "bandpass");
        assert_eq!(m.kind, Kind::Module);
        assert_eq!(
            m.inputs[0].modality,
            Only::Of(vec!["ecg".into(), "eeg".into()])
        );
        assert_eq!(m.params[1].default, Some(Json::Str("a".into())));
        assert!(m.params[0].mutable);
    }

    #[test]
    fn structural_and_schema_errors() {
        let cases: &[(&str, &str)] = &[
            ("node \"x\" { kind = input, version = \"1.0.0\", runtime python { entry = \"m.py\" }, input \"a\" { type = signal } }", "E0401"),
            ("node \"x\" { kind = output, version = \"1.0.0\", runtime python { entry = \"m.py\" }, output \"a\" { type = signal } }", "E0401"),
            ("node \"x\" { kind = module, version = \"1.0.0\" }", "E0402"),
            ("node \"x\" { kind = module, version = \"1.0.0\", runtime python { entry = \"a\" }, runtime exec { command = [\"b\"] } }", "E0402"),
            ("node \"x\" { kind = module, version = \"1.0.0\", runtime python { entry = \"a\" }, bogus = 1 }", "E0002"),
            ("node \"x\" { kind = module, version = 1, runtime python { entry = \"a\" } }", "E0003"),
            ("node \"x\" { kind = module, version = \"1.0.0\", runtime python { entry = \"a\" }, description = hello }", "E0005"),
            ("node \"x\" { kind = module, version = \"1.0.0\", version = \"1.0.1\", runtime python { entry = \"a\" } }", "E0004"),
            ("node \"x\" { version = \"1.0.0\", runtime python { entry = \"a\" } }", "E0006"),
            ("node \"x\" { kind = module, version = \"1.0.0\", runtime python { entry = \"a\" }, param \"p\" { type = integer, default = 1.5 } }", "E0003"),
            ("node \"x\" { kind = module, version = \"1.0.0\", runtime python { entry = \"a\" }, input \"i\" { type = signal, dtype = f16 } }", "E0003"),
        ];
        for (src, code) in cases {
            let (_, d) = parse(src);
            assert!(
                d.items.iter().any(|x| x.code == *code),
                "expected {} for {}; got {:?}",
                code,
                src,
                d.items
                    .iter()
                    .map(|x| (x.code, x.message.clone()))
                    .collect::<Vec<_>>()
            );
        }
    }
}
