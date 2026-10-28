use super::span::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Ident(String),
    Str(String),
    Int(i64),
    Float(f64),
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Eq,
    Colon,
    Comma,
    Dot,
    Arrow,
    Newline,
    Eof,
}

impl TokenKind {
    /// Human-readable name used in syntax errors.
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Ident(s) => format!("identifier `{}`", s),
            TokenKind::Str(_) => "string".to_string(),
            TokenKind::Int(_) | TokenKind::Float(_) => "number".to_string(),
            TokenKind::LBrace => "`{`".to_string(),
            TokenKind::RBrace => "`}`".to_string(),
            TokenKind::LBracket => "`[`".to_string(),
            TokenKind::RBracket => "`]`".to_string(),
            TokenKind::Eq => "`=`".to_string(),
            TokenKind::Colon => "`:`".to_string(),
            TokenKind::Comma => "`,`".to_string(),
            TokenKind::Dot => "`.`".to_string(),
            TokenKind::Arrow => "`->`".to_string(),
            TokenKind::Newline => "end of line".to_string(),
            TokenKind::Eof => "end of file".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}
