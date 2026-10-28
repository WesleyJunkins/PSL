//! Helper for extracting typed attributes and blocks from a block body with diagnostics.

use crate::diag::{Diagnostic, Diagnostics};
use crate::syntax::{Attribute, Block, Span, Statement, Value};

pub struct Body<'a> {
    stmts: &'a [Statement],
    used: Vec<bool>,
    what: String,
    span: Span,
    pub diags: Diagnostics,
}

impl<'a> Body<'a> {
    /// `what` describes the enclosing construct for messages, e.g. "a `param` block".
    pub fn new(stmts: &'a [Statement], what: impl Into<String>, span: Span) -> Body<'a> {
        Body {
            stmts,
            used: vec![false; stmts.len()],
            what: what.into(),
            span,
            diags: Diagnostics::new(),
        }
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn attr(&mut self, name: &str) -> Option<&'a Attribute> {
        let mut found: Option<&'a Attribute> = None;
        for (i, s) in self.stmts.iter().enumerate() {
            if let Statement::Attribute(a) = s {
                if a.name.name == name {
                    self.used[i] = true;
                    if let Some(first) = found {
                        self.diags.push(
                            Diagnostic::error("E0004", format!("duplicate attribute `{}`", name))
                                .primary(a.name.span, "given again here")
                                .secondary(first.name.span, "first given here"),
                        );
                    } else {
                        found = Some(a);
                    }
                }
            }
        }
        found
    }

    pub fn blocks(&mut self, keyword: &str) -> Vec<&'a Block> {
        let mut out = Vec::new();
        for (i, s) in self.stmts.iter().enumerate() {
            if let Statement::Block(b) = s {
                if b.keyword.name == keyword {
                    self.used[i] = true;
                    out.push(b);
                }
            }
        }
        out
    }

    /// All attributes, marking them used (for bodies where attributes are free-form, e.g. params).
    pub fn all_attrs(&mut self) -> Vec<&'a Attribute> {
        let mut out = Vec::new();
        for (i, s) in self.stmts.iter().enumerate() {
            if let Statement::Attribute(a) = s {
                self.used[i] = true;
                out.push(a);
            }
        }
        out
    }

    /// Report every statement that was not consumed.
    pub fn finish(&mut self) {
        for (i, s) in self.stmts.iter().enumerate() {
            if self.used[i] {
                continue;
            }
            let d = match s {
                Statement::Attribute(a) => Diagnostic::error(
                    "E0002",
                    format!("unknown attribute `{}` in {}", a.name.name, self.what),
                )
                .primary(a.name.span, "not a known attribute here"),
                Statement::Block(b) => Diagnostic::error(
                    "E0002",
                    format!("unknown block `{}` in {}", b.keyword.name, self.what),
                )
                .primary(b.keyword.span, "not a known block here"),
                Statement::Map(m) => {
                    Diagnostic::error("E0002", format!("`map` is not allowed in {}", self.what))
                        .primary(m.span, "")
                }
                Statement::Node(n) => Diagnostic::error(
                    "E0002",
                    format!("`node` declaration is not allowed in {}", self.what),
                )
                .primary(n.span, ""),
            };
            self.diags.push(d);
        }
    }

    fn wrong_type(&mut self, a: &Attribute, expected: &str) {
        if let Value::Symbol(sym, span) = &a.value {
            if expected == "string" || expected == "list of strings" {
                self.diags.push(
                    Diagnostic::error(
                        "E0005",
                        format!("bare identifier `{}` used where a string is expected", sym),
                    )
                    .primary(*span, "did you mean a string?")
                    .help(format!("write `{} = \"{}\"`", a.name.name, sym)),
                );
                return;
            }
        }
        self.diags.push(
            Diagnostic::error(
                "E0003",
                format!(
                    "attribute `{}` must be a {}, found {}",
                    a.name.name,
                    expected,
                    a.value.type_name()
                ),
            )
            .primary(a.value.span(), format!("expected a {}", expected)),
        );
    }

    pub fn missing(&mut self, name: &str) {
        self.diags.push(
            Diagnostic::error(
                "E0006",
                format!("missing required attribute `{}` in {}", name, self.what),
            )
            .primary(self.span, format!("`{}` must be given here", name)),
        );
    }

    pub fn str(&mut self, name: &str) -> Option<(String, Span)> {
        let a = self.attr(name)?;
        match &a.value {
            Value::Str(s, span) => Some((s.clone(), *span)),
            _ => {
                self.wrong_type(a, "string");
                None
            }
        }
    }

    pub fn req_str(&mut self, name: &str) -> Option<(String, Span)> {
        if self.attr(name).is_none() {
            self.missing(name);
            return None;
        }
        self.str(name)
    }

    pub fn num(&mut self, name: &str) -> Option<(f64, Span)> {
        let a = self.attr(name)?;
        match &a.value {
            Value::Int(v, span) => Some((*v as f64, *span)),
            Value::Float(v, span) => Some((*v, *span)),
            _ => {
                self.wrong_type(a, "number");
                None
            }
        }
    }

    pub fn int(&mut self, name: &str) -> Option<(i64, Span)> {
        let a = self.attr(name)?;
        match &a.value {
            Value::Int(v, span) => Some((*v, *span)),
            _ => {
                self.wrong_type(a, "integer");
                None
            }
        }
    }

    pub fn bool(&mut self, name: &str) -> Option<(bool, Span)> {
        let a = self.attr(name)?;
        match &a.value {
            Value::Bool(v, span) => Some((*v, *span)),
            _ => {
                self.wrong_type(a, "boolean");
                None
            }
        }
    }

    /// An enumeration symbol (bare identifier) or a string, restricted to `allowed`.
    pub fn symbol(&mut self, name: &str, allowed: &[&str]) -> Option<(String, Span)> {
        let a = self.attr(name)?;
        let (text, span) = match &a.value {
            Value::Symbol(s, span) | Value::Str(s, span) => (s.clone(), *span),
            _ => {
                self.wrong_type(a, &format!("one of {}", list_quoted(allowed)));
                return None;
            }
        };
        if allowed.contains(&text.as_str()) {
            Some((text, span))
        } else {
            self.diags.push(
                Diagnostic::error(
                    "E0003",
                    format!(
                        "attribute `{}` must be one of {}, found `{}`",
                        name,
                        list_quoted(allowed),
                        text
                    ),
                )
                .primary(span, "not an allowed value"),
            );
            None
        }
    }

    pub fn str_list(&mut self, name: &str) -> Option<(Vec<String>, Span)> {
        let a = self.attr(name)?;
        match &a.value {
            Value::List(items, span) => {
                let mut out = Vec::new();
                for item in items {
                    match item {
                        Value::Str(s, _) => out.push(s.clone()),
                        Value::Symbol(s, sp) => {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0005",
                                    format!(
                                        "bare identifier `{}` used where a string is expected",
                                        s
                                    ),
                                )
                                .primary(*sp, "did you mean a string?")
                                .help(format!("write `\"{}\"`", s)),
                            );
                        }
                        other => {
                            self.diags.push(
                                Diagnostic::error(
                                    "E0003",
                                    format!(
                                        "attribute `{}` must be a list of strings, found {} inside",
                                        name,
                                        other.type_name()
                                    ),
                                )
                                .primary(other.span(), "expected a string"),
                            );
                        }
                    }
                }
                Some((out, *span))
            }
            _ => {
                self.wrong_type(a, "list of strings");
                None
            }
        }
    }

    /// An object value `{ k = "v" }` whose values are all strings.
    pub fn str_object(&mut self, name: &str) -> Option<(Vec<(String, String)>, Span)> {
        let a = self.attr(name)?;
        match &a.value {
            Value::Object(attrs, span) => {
                let mut out = Vec::new();
                for at in attrs {
                    match &at.value {
                        Value::Str(s, _) => out.push((at.name.name.clone(), s.clone())),
                        other => self.diags.push(
                            Diagnostic::error(
                                "E0003",
                                format!(
                                    "value of `{}` must be a string, found {}",
                                    at.name.name,
                                    other.type_name()
                                ),
                            )
                            .primary(other.span(), "expected a string"),
                        ),
                    }
                }
                Some((out, *span))
            }
            _ => {
                self.wrong_type(a, "object");
                None
            }
        }
    }
}

pub fn list_quoted(items: &[&str]) -> String {
    items
        .iter()
        .map(|s| format!("`{}`", s))
        .collect::<Vec<_>>()
        .join(", ")
}
