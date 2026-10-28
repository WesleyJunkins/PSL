//! The workspace document (`workspace.psl`), docs/04 §Workspace schema.

use crate::diag::{Diagnostic, Diagnostics};
use crate::model::reader::Body;
use crate::syntax::{Document, FileId, Label, Span, Statement};
use crate::util::VersionReq;

#[derive(Clone, Debug, PartialEq)]
pub struct Require {
    pub name: String,
    pub span: Span,
    pub version: Option<String>,
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    pub name: String,
    pub name_span: Span,
    pub psl: Option<String>,
    pub python: Option<String>,
    pub requires: Vec<Require>,
    pub file: FileId,
}

impl Workspace {
    pub fn require(&self, name: &str) -> Option<&Require> {
        self.requires.iter().find(|r| r.name == name)
    }
}

pub fn parse_workspace(doc: &Document, file: FileId) -> (Option<Workspace>, Diagnostics) {
    let mut diags = Diagnostics::new();
    let mut header = None;
    let mut requires = Vec::new();
    for s in &doc.statements {
        match s {
            Statement::Block(b) if b.keyword.name == "workspace" => {
                if header.is_some() {
                    diags.push(
                        Diagnostic::error(
                            "E0004",
                            "a workspace file has exactly one `workspace` block",
                        )
                        .primary(b.keyword.span, ""),
                    );
                } else {
                    header = Some(b);
                }
            }
            Statement::Block(b) if b.keyword.name == "require" => {
                let (name, span) = match b.labels.as_slice() {
                    [Label::Str(s)] => (s.value.clone(), s.span),
                    _ => {
                        diags.push(Diagnostic::error("E0006", "`require` needs a package name in quotes: `require \"owner/name\" { ... }`").primary(b.keyword.span, ""));
                        continue;
                    }
                };
                if !is_package_name(&name) {
                    diags.push(Diagnostic::error("E0003", format!("`{}` is not a valid package name; expected `owner/name` in lowercase", name)).primary(span, ""));
                }
                let mut body = Body::new(&b.body, "a `require` block", b.span);
                let version = body.str("version").map(|v| v.0);
                if let Some(v) = &version {
                    if VersionReq::parse(v).is_none() {
                        body.diags.push(Diagnostic::error("E0003", format!("`version` requirement `{}` is malformed; examples: `^0.1`, `~1.2.3`, `=1.2.3`, `>=1.2`", v)).primary(span, ""));
                    }
                }
                let source = body.str("source").map(|v| v.0);
                body.finish();
                diags.extend(body.diags);
                requires.push(Require {
                    name,
                    span,
                    version,
                    source,
                });
            }
            other => diags.push(
                Diagnostic::error(
                    "E0002",
                    "only `workspace` and `require` blocks are allowed in a workspace file",
                )
                .primary(other.span(), ""),
            ),
        }
    }
    let header = match header {
        Some(h) => h,
        None => {
            diags.push(
                Diagnostic::error(
                    "E0006",
                    "workspace file has no `workspace \"name\" { ... }` block",
                )
                .at_path(""),
            );
            return (None, diags);
        }
    };
    let (name, name_span) = match header.labels.as_slice() {
        [Label::Str(s)] => (s.value.clone(), s.span),
        _ => {
            diags.push(
                Diagnostic::error("E0006", "`workspace` needs a name in quotes")
                    .primary(header.keyword.span, ""),
            );
            return (None, diags);
        }
    };
    let mut body = Body::new(&header.body, "the `workspace` block", header.span);
    let psl = body.str("psl").map(|v| v.0);
    let python = body.str("python").map(|v| v.0);
    body.finish();
    diags.extend(body.diags);
    if diags.has_errors() {
        return (None, diags);
    }
    (
        Some(Workspace {
            name,
            name_span,
            psl,
            python,
            requires,
            file,
        }),
        diags,
    )
}

/// `owner/name`, lowercase letters, digits and hyphens on both sides.
pub fn is_package_name(s: &str) -> bool {
    let mut parts = s.split('/');
    let ok = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !p.starts_with('-')
    };
    match (parts.next(), parts.next(), parts.next()) {
        (Some(a), Some(b), None) => ok(a) && ok(b),
        _ => false,
    }
}
