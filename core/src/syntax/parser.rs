//! Recursive-descent parser for PSL documents (docs/04 §Grammar).
//!
//! Statements end at a newline; inside `{ }` a comma may also separate them.
//! Newlines are insignificant inside `[ ]` and directly after `{` / before `}`.

use super::ast::*;
use super::lexer::Lexer;
use super::span::{FileId, Span};
use super::token::{Token, TokenKind};
use crate::diag::Diagnostic;

pub fn parse_document(file: FileId, src: &str) -> Result<Document, Diagnostic> {
    let tokens = Lexer::new(file, src).tokenize()?;
    let mut p = Parser { tokens, pos: 0 };
    p.document()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

type PResult<T> = Result<T, Diagnostic>;

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn peek_kind_at(&self, off: usize) -> &TokenKind {
        &self.tokens[(self.pos + off).min(self.tokens.len() - 1)].kind
    }

    fn bump(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at_ident(&self, name: &str) -> bool {
        matches!(self.peek_kind(), TokenKind::Ident(n) if n == name)
    }

    fn skip_newlines(&mut self) {
        while *self.peek_kind() == TokenKind::Newline {
            self.bump();
        }
    }

    fn skip_separators(&mut self) {
        while matches!(self.peek_kind(), TokenKind::Newline | TokenKind::Comma) {
            self.bump();
        }
    }

    fn unexpected(&self, expected: &str) -> Diagnostic {
        let t = self.peek();
        Diagnostic::error(
            "E0001",
            format!("expected {}, found {}", expected, t.kind.describe()),
        )
        .primary(t.span, format!("expected {}", expected))
    }

    fn expect_ident(&mut self, what: &str) -> PResult<Ident> {
        match self.peek_kind().clone() {
            TokenKind::Ident(name) => {
                let t = self.bump();
                Ok(Ident { name, span: t.span })
            }
            _ => Err(self.unexpected(what)),
        }
    }

    fn expect_string(&mut self, what: &str) -> PResult<StringLit> {
        match self.peek_kind().clone() {
            TokenKind::Str(value) => {
                let t = self.bump();
                Ok(StringLit {
                    value,
                    span: t.span,
                })
            }
            _ => Err(self.unexpected(what)),
        }
    }

    fn expect(&mut self, kind: TokenKind) -> PResult<Token> {
        if *self.peek_kind() == kind {
            Ok(self.bump())
        } else {
            Err(self.unexpected(&kind.describe()))
        }
    }

    fn document(&mut self) -> PResult<Document> {
        let mut statements = Vec::new();
        loop {
            self.skip_newlines();
            if *self.peek_kind() == TokenKind::Eof {
                break;
            }
            statements.push(self.statement()?);
            match self.peek_kind() {
                TokenKind::Newline | TokenKind::Eof => {}
                _ => return Err(self.unexpected("end of line")),
            }
        }
        Ok(Document { statements })
    }

    /// `{ statement* }` — the opening brace is the current token.
    fn body(&mut self) -> PResult<(Vec<Statement>, Span)> {
        let open = self.expect(TokenKind::LBrace)?;
        let mut body = Vec::new();
        loop {
            self.skip_separators();
            if *self.peek_kind() == TokenKind::RBrace {
                let close = self.bump();
                return Ok((body, open.span.to(close.span)));
            }
            if *self.peek_kind() == TokenKind::Eof {
                return Err(Diagnostic::error("E0001", "unclosed block")
                    .primary(open.span, "this `{` is never closed"));
            }
            body.push(self.statement()?);
            match self.peek_kind() {
                TokenKind::Newline | TokenKind::Comma | TokenKind::RBrace => {}
                _ => return Err(self.unexpected("end of line, `,` or `}`")),
            }
        }
    }

    fn statement(&mut self) -> PResult<Statement> {
        if self.at_ident("node") && matches!(self.peek_kind_at(1), TokenKind::Ident(_)) {
            return self.node_decl().map(Statement::Node);
        }
        if self.at_ident("map") {
            return self.map_stmt().map(Statement::Map);
        }
        match self.peek_kind() {
            TokenKind::Ident(_) => {}
            _ => return Err(self.unexpected("a statement")),
        }
        if *self.peek_kind_at(1) == TokenKind::Eq {
            return self.attribute().map(Statement::Attribute);
        }
        self.block().map(Statement::Block)
    }

    fn attribute(&mut self) -> PResult<Attribute> {
        let name = self.expect_ident("attribute name")?;
        self.expect(TokenKind::Eq)?;
        let value = self.value()?;
        let span = name.span.to(value.span());
        Ok(Attribute { name, value, span })
    }

    fn block(&mut self) -> PResult<Block> {
        let keyword = self.expect_ident("block keyword")?;
        let mut labels = Vec::new();
        loop {
            match self.peek_kind().clone() {
                TokenKind::Str(value) => {
                    let t = self.bump();
                    labels.push(Label::Str(StringLit {
                        value,
                        span: t.span,
                    }));
                }
                TokenKind::Ident(name) => {
                    let t = self.bump();
                    labels.push(Label::Ident(Ident { name, span: t.span }));
                }
                TokenKind::LBrace => break,
                _ => return Err(self.unexpected("`{`")),
            }
        }
        let (body, body_span) = self.body()?;
        let span = keyword.span.to(body_span);
        Ok(Block {
            keyword,
            labels,
            body,
            span,
        })
    }

    fn node_decl(&mut self) -> PResult<NodeDecl> {
        let kw = self.bump(); // `node`
        let id = self.expect_ident("node id")?;
        self.expect(TokenKind::Colon)?;
        let (kind, kind_span) = match self.peek_kind().clone() {
            TokenKind::Ident(k) => match Kind::parse(&k) {
                Some(kind) => {
                    let t = self.bump();
                    (kind, t.span)
                }
                None => {
                    let t = self.peek();
                    return Err(
                        Diagnostic::error("E0001", format!("unknown node kind `{}`", k))
                            .primary(t.span, "expected `input`, `module` or `output`"),
                    );
                }
            },
            _ => return Err(self.unexpected("node kind (`input`, `module` or `output`)")),
        };
        let package = self.expect_string("package name in quotes")?;
        let (body, end) = if *self.peek_kind() == TokenKind::LBrace {
            self.body()?
        } else {
            (Vec::new(), package.span)
        };
        let span = kw.span.to(end);
        Ok(NodeDecl {
            id,
            kind,
            kind_span,
            package,
            body,
            span,
        })
    }

    fn map_stmt(&mut self) -> PResult<MapStmt> {
        let kw = self.bump(); // `map`
        let from = self.endpoints()?;
        self.expect(TokenKind::Arrow)?;
        let to = self.endpoints()?;
        let end = to.last().map(|e| e.span).unwrap_or(kw.span);
        Ok(MapStmt {
            from,
            to,
            span: kw.span.to(end),
        })
    }

    fn endpoints(&mut self) -> PResult<Vec<Endpoint>> {
        let mut out = vec![self.endpoint()?];
        while *self.peek_kind() == TokenKind::Comma {
            self.bump();
            out.push(self.endpoint()?);
        }
        Ok(out)
    }

    fn endpoint(&mut self) -> PResult<Endpoint> {
        let node = self.expect_ident("endpoint (`node.port`)")?;
        if *self.peek_kind() != TokenKind::Dot {
            return Err(self.unexpected("`.` followed by a port name"));
        }
        self.bump();
        let port = self.expect_ident("port name")?;
        let span = node.span.to(port.span);
        Ok(Endpoint { node, port, span })
    }

    fn value(&mut self) -> PResult<Value> {
        let t = self.peek().clone();
        match t.kind {
            TokenKind::Str(s) => {
                self.bump();
                Ok(Value::Str(s, t.span))
            }
            TokenKind::Int(v) => {
                self.bump();
                Ok(Value::Int(v, t.span))
            }
            TokenKind::Float(v) => {
                self.bump();
                Ok(Value::Float(v, t.span))
            }
            TokenKind::Ident(name) => {
                self.bump();
                Ok(match name.as_str() {
                    "true" => Value::Bool(true, t.span),
                    "false" => Value::Bool(false, t.span),
                    "null" => Value::Null(t.span),
                    _ => Value::Symbol(name, t.span),
                })
            }
            TokenKind::LBracket => {
                let open = self.bump();
                let mut items = Vec::new();
                loop {
                    self.skip_newlines();
                    if *self.peek_kind() == TokenKind::RBracket {
                        let close = self.bump();
                        return Ok(Value::List(items, open.span.to(close.span)));
                    }
                    items.push(self.value()?);
                    self.skip_newlines();
                    match self.peek_kind() {
                        TokenKind::Comma => {
                            self.bump();
                        }
                        TokenKind::RBracket => {}
                        _ => return Err(self.unexpected("`,` or `]`")),
                    }
                }
            }
            TokenKind::LBrace => {
                let open = self.bump();
                let mut attrs = Vec::new();
                loop {
                    self.skip_separators();
                    if *self.peek_kind() == TokenKind::RBrace {
                        let close = self.bump();
                        return Ok(Value::Object(attrs, open.span.to(close.span)));
                    }
                    attrs.push(self.attribute()?);
                    match self.peek_kind() {
                        TokenKind::Newline | TokenKind::Comma | TokenKind::RBrace => {}
                        _ => return Err(self.unexpected("`,` or `}`")),
                    }
                }
            }
            _ => Err(self.unexpected("a value")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Document {
        match parse_document(FileId(0), src) {
            Ok(d) => d,
            Err(e) => panic!(
                "parse error: {} at {:?}",
                e.message,
                e.labels.first().map(|l| l.span)
            ),
        }
    }

    #[test]
    fn pipeline_example() {
        let doc = parse(
            r#"
# comment
pipeline "hr_demo" {
  description = "demo"
  runtime { ready_timeout_s = 10 }
}

node ecg : input "psl/synthetic" {
  waveform = "ecg"
  srate    = 250
  channels = ["ECG"]
}

node bp : module "psl/bandpass" { low = 0.5, high = 40 }
node log : output "psl/print" {}
node bare : output "psl/print"

map ecg.out -> bp.in
map bp.out  -> log.in, bare.in
"#,
        );
        assert_eq!(doc.statements.len(), 7);
        match &doc.statements[1] {
            Statement::Node(n) => {
                assert_eq!(n.id.name, "ecg");
                assert_eq!(n.kind, Kind::Input);
                assert_eq!(n.package.value, "psl/synthetic");
                assert_eq!(n.body.len(), 3);
            }
            other => panic!("unexpected {:?}", other),
        }
        match &doc.statements[2] {
            Statement::Node(n) => assert_eq!(n.body.len(), 2),
            other => panic!("unexpected {:?}", other),
        }
        match &doc.statements[6] {
            Statement::Map(m) => {
                assert_eq!(m.from.len(), 1);
                assert_eq!(m.to.len(), 2);
                assert_eq!(m.to[1].node.name, "bare");
            }
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn values() {
        let doc = parse(
            "a = [1, 2.5,\n \"x\", true, null, sym]\nb = { x = 1, y = \"z\" }\nc = {}\nd = []",
        );
        match &doc.statements[0] {
            Statement::Attribute(a) => match &a.value {
                Value::List(items, _) => assert_eq!(items.len(), 6),
                v => panic!("{:?}", v),
            },
            s => panic!("{:?}", s),
        }
        match &doc.statements[1] {
            Statement::Attribute(a) => match &a.value {
                Value::Object(attrs, _) => assert_eq!(attrs.len(), 2),
                v => panic!("{:?}", v),
            },
            s => panic!("{:?}", s),
        }
    }

    #[test]
    fn manifest_blocks() {
        let doc = parse(
            r#"node "bandpass" {
  kind = module
  runtime python { entry = "main.py" }
  input "in" { type = signal }
  param "low" { type = number, default = 0.5 }
}"#,
        );
        match &doc.statements[0] {
            Statement::Block(b) => {
                assert_eq!(b.keyword.name, "node");
                assert_eq!(b.labels[0].text(), "bandpass");
                assert_eq!(b.body.len(), 4);
                match &b.body[1] {
                    Statement::Block(r) => {
                        assert_eq!(r.keyword.name, "runtime");
                        assert_eq!(r.labels[0].text(), "python");
                    }
                    s => panic!("{:?}", s),
                }
            }
            s => panic!("{:?}", s),
        }
    }

    #[test]
    fn syntax_errors() {
        for src in [
            "node x input \"p\"",
            "node x : thing \"p\"",
            "map a -> b",
            "a = ",
            "a = [1 2]",
            "block {",
            "a = 1 b = 2",
            "node x : input \"p\" { a = 1 } junk",
        ] {
            assert!(
                parse_document(FileId(0), src).is_err(),
                "should fail: {}",
                src
            );
        }
    }
}
