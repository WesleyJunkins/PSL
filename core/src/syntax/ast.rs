//! Abstract syntax tree for PSL documents (docs/04 §Grammar).

use super::span::Span;

#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StringLit {
    pub value: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub statements: Vec<Statement>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    Node(NodeDecl),
    Block(Block),
    Attribute(Attribute),
    Map(MapStmt),
}

impl Statement {
    pub fn span(&self) -> Span {
        match self {
            Statement::Node(n) => n.span,
            Statement::Block(b) => b.span,
            Statement::Attribute(a) => a.span,
            Statement::Map(m) => m.span,
        }
    }
}

/// `keyword label* { statements }`
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub keyword: Ident,
    pub labels: Vec<Label>,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Label {
    Str(StringLit),
    Ident(Ident),
}

impl Label {
    pub fn text(&self) -> &str {
        match self {
            Label::Str(s) => &s.value,
            Label::Ident(i) => &i.name,
        }
    }
    pub fn span(&self) -> Span {
        match self {
            Label::Str(s) => s.span,
            Label::Ident(i) => i.span,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Input,
    Module,
    Output,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Input => "input",
            Kind::Module => "module",
            Kind::Output => "output",
        }
    }
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "input" => Some(Kind::Input),
            "module" => Some(Kind::Module),
            "output" => Some(Kind::Output),
            _ => None,
        }
    }
}

/// `node id : kind "package" { ... }`
#[derive(Clone, Debug, PartialEq)]
pub struct NodeDecl {
    pub id: Ident,
    pub kind: Kind,
    pub kind_span: Span,
    pub package: StringLit,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: Ident,
    pub value: Value,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapStmt {
    pub from: Vec<Endpoint>,
    pub to: Vec<Endpoint>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub node: Ident,
    pub port: Ident,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String, Span),
    Int(i64, Span),
    Float(f64, Span),
    Bool(bool, Span),
    Null(Span),
    /// A bare identifier: an enumeration symbol.
    Symbol(String, Span),
    List(Vec<Value>, Span),
    Object(Vec<Attribute>, Span),
}

impl Value {
    pub fn span(&self) -> Span {
        match self {
            Value::Str(_, s)
            | Value::Int(_, s)
            | Value::Float(_, s)
            | Value::Bool(_, s)
            | Value::Null(s)
            | Value::Symbol(_, s)
            | Value::List(_, s)
            | Value::Object(_, s) => *s,
        }
    }

    /// Name of the value's type, for diagnostics.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Str(..) => "string",
            Value::Int(..) => "integer",
            Value::Float(..) => "number",
            Value::Bool(..) => "boolean",
            Value::Null(..) => "null",
            Value::Symbol(..) => "identifier",
            Value::List(..) => "list",
            Value::Object(..) => "object",
        }
    }
}
