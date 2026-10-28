//! Tokeniser for the PSL grammar (docs/04 §Lexical structure).

use super::span::{FileId, Span};
use super::token::{Token, TokenKind};
use crate::diag::Diagnostic;

pub struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    file: FileId,
}

impl<'a> Lexer<'a> {
    pub fn new(file: FileId, src: &'a str) -> Lexer<'a> {
        Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            file,
        }
    }

    /// Tokenise the whole file. Stops at the first lexical error.
    pub fn tokenize(mut self) -> Result<Vec<Token>, Diagnostic> {
        let mut out = Vec::new();
        loop {
            let tok = self.next_token()?;
            let eof = tok.kind == TokenKind::Eof;
            out.push(tok);
            if eof {
                break;
            }
        }
        Ok(out)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.bytes.get(self.pos + off).copied()
    }

    fn span(&self, start: usize) -> Span {
        Span::new(self.file, start, self.pos)
    }

    fn error(&self, start: usize, msg: impl Into<String>) -> Diagnostic {
        Diagnostic::error("E0001", msg).primary(self.span(start), "")
    }

    fn next_token(&mut self) -> Result<Token, Diagnostic> {
        // Skip horizontal whitespace and comments; newlines are tokens.
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\r') => self.pos += 1,
                Some(b'#') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
        let start = self.pos;
        let c = match self.peek() {
            None => {
                return Ok(Token {
                    kind: TokenKind::Eof,
                    span: self.span(start),
                })
            }
            Some(c) => c,
        };
        let kind = match c {
            b'\n' => {
                self.pos += 1;
                TokenKind::Newline
            }
            b'{' => {
                self.pos += 1;
                TokenKind::LBrace
            }
            b'}' => {
                self.pos += 1;
                TokenKind::RBrace
            }
            b'[' => {
                self.pos += 1;
                TokenKind::LBracket
            }
            b']' => {
                self.pos += 1;
                TokenKind::RBracket
            }
            b'=' => {
                self.pos += 1;
                TokenKind::Eq
            }
            b':' => {
                self.pos += 1;
                TokenKind::Colon
            }
            b',' => {
                self.pos += 1;
                TokenKind::Comma
            }
            b'.' => {
                self.pos += 1;
                TokenKind::Dot
            }
            b'-' if self.peek_at(1) == Some(b'>') => {
                self.pos += 2;
                TokenKind::Arrow
            }
            b'"' => self.string(start)?,
            b'-' | b'0'..=b'9' => self.number(start)?,
            c if c == b'_' || c.is_ascii_alphabetic() => {
                while let Some(c) = self.peek() {
                    if c == b'_' || c.is_ascii_alphanumeric() {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                TokenKind::Ident(self.src[start..self.pos].to_string())
            }
            _ => {
                // Consume one UTF-8 character for the error span.
                let ch = self.src[start..].chars().next().unwrap();
                self.pos += ch.len_utf8();
                return Err(self.error(start, format!("unexpected character `{}`", ch)));
            }
        };
        Ok(Token {
            kind,
            span: self.span(start),
        })
    }

    fn string(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let c = match self.peek() {
                None | Some(b'\n') => {
                    return Err(self.error(start, "unterminated string"));
                }
                Some(c) => c,
            };
            match c {
                b'"' => {
                    self.pos += 1;
                    break;
                }
                b'\\' => {
                    let esc_start = self.pos;
                    self.pos += 1;
                    match self.peek() {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'n') => out.push('\n'),
                        Some(b't') => out.push('\t'),
                        Some(b'r') => out.push('\r'),
                        Some(b'u') => {
                            let hex = self.src.get(self.pos + 1..self.pos + 5).unwrap_or("");
                            let code = u32::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 4);
                            match code.and_then(char::from_u32) {
                                Some(ch) => {
                                    out.push(ch);
                                    self.pos += 4;
                                }
                                None => {
                                    self.pos += 1;
                                    return Err(self.error(
                                        esc_start,
                                        "invalid `\\u` escape: expected four hex digits",
                                    ));
                                }
                            }
                        }
                        _ => {
                            self.pos += 1;
                            return Err(self.error(esc_start, "invalid escape sequence"));
                        }
                    }
                    self.pos += 1;
                }
                _ => {
                    let ch = self.src[self.pos..].chars().next().unwrap();
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        Ok(TokenKind::Str(out))
    }

    fn number(&mut self, start: usize) -> Result<TokenKind, Diagnostic> {
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let digits_start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.pos == digits_start {
            return Err(self.error(start, "expected digits after `-`"));
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') && matches!(self.peek_at(1), Some(b'0'..=b'9')) {
            is_float = true;
            self.pos += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == exp_start {
                self.pos = save;
                return Err(self.error(start, "malformed exponent in number"));
            }
            is_float = true;
        }
        // A number immediately followed by an identifier character is malformed (e.g. `12abc`).
        if let Some(c) = self.peek() {
            if c == b'_' || c.is_ascii_alphabetic() {
                return Err(self.error(start, "malformed number"));
            }
        }
        let text = &self.src[start..self.pos];
        if is_float {
            text.parse::<f64>()
                .map(TokenKind::Float)
                .map_err(|_| self.error(start, "malformed number"))
        } else {
            match text.parse::<i64>() {
                Ok(v) => Ok(TokenKind::Int(v)),
                Err(_) => text
                    .parse::<f64>()
                    .map(TokenKind::Float)
                    .map_err(|_| self.error(start, "malformed number")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        Lexer::new(FileId(0), src)
            .tokenize()
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn punctuation_and_idents() {
        let k = kinds("map a.b -> c.d { x = 1 }");
        assert_eq!(
            k,
            vec![
                TokenKind::Ident("map".into()),
                TokenKind::Ident("a".into()),
                TokenKind::Dot,
                TokenKind::Ident("b".into()),
                TokenKind::Arrow,
                TokenKind::Ident("c".into()),
                TokenKind::Dot,
                TokenKind::Ident("d".into()),
                TokenKind::LBrace,
                TokenKind::Ident("x".into()),
                TokenKind::Eq,
                TokenKind::Int(1),
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds("1 -2 0.5 -1.25e3 40 1e2")[..6],
            [
                TokenKind::Int(1),
                TokenKind::Int(-2),
                TokenKind::Float(0.5),
                TokenKind::Float(-1250.0),
                TokenKind::Int(40),
                TokenKind::Float(100.0),
            ]
        );
    }

    #[test]
    fn strings_and_escapes() {
        assert_eq!(
            kinds(r#""a\"b\\c\n\u0041""#)[0],
            TokenKind::Str("a\"b\\c\nA".into())
        );
    }

    #[test]
    fn comments_and_newlines() {
        let k = kinds("a = 1 # comment\n\nb = 2");
        assert_eq!(k.iter().filter(|k| **k == TokenKind::Newline).count(), 2);
    }

    #[test]
    fn errors() {
        assert!(Lexer::new(FileId(0), "\"open").tokenize().is_err());
        assert!(Lexer::new(FileId(0), "x = @").tokenize().is_err());
        assert!(Lexer::new(FileId(0), "12abc").tokenize().is_err());
        assert!(Lexer::new(FileId(0), "\"\\q\"").tokenize().is_err());
    }
}
