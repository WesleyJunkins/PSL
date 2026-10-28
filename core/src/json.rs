//! A small JSON value type with parser and serialiser.
//!
//! Objects keep insertion order so canonical output is stable. Integers and
//! floats are kept apart so 64-bit timestamps survive a round trip.

use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonError {
    pub offset: usize,
    pub message: String,
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid JSON at byte {}: {}", self.offset, self.message)
    }
}

impl Json {
    pub fn object() -> Json {
        Json::Object(Vec::new())
    }

    /// Builder-style insert for objects. Replaces an existing key in place.
    pub fn with(mut self, key: &str, value: impl Into<Json>) -> Json {
        self.set(key, value);
        self
    }

    pub fn set(&mut self, key: &str, value: impl Into<Json>) {
        if let Json::Object(items) = self {
            let value = value.into();
            if let Some(slot) = items.iter_mut().find(|(k, _)| k == key) {
                slot.1 = value;
            } else {
                items.push((key.to_string(), value));
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Int(v) => Some(*v),
            Json::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Some(*f as i64),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Int(v) => Some(*v as f64),
            Json::Float(f) => Some(*f),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_object(&self) -> Option<&Vec<(String, Json)>> {
        match self {
            Json::Object(o) => Some(o),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    /// Convenience: `get(key)` then `as_str`.
    pub fn str_field(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Json::as_str)
    }
    pub fn i64_field(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Json::as_i64)
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Json::Null => "null",
            Json::Bool(_) => "boolean",
            Json::Int(_) => "integer",
            Json::Float(_) => "number",
            Json::Str(_) => "string",
            Json::Array(_) => "list",
            Json::Object(_) => "object",
        }
    }

    // ----- parsing -----

    pub fn parse(src: &str) -> Result<Json, JsonError> {
        let mut p = JsonParser {
            s: src.as_bytes(),
            src,
            pos: 0,
        };
        p.ws();
        let v = p.value()?;
        p.ws();
        if p.pos != p.s.len() {
            return Err(p.err("trailing characters"));
        }
        Ok(v)
    }

    // ----- serialising -----

    pub fn to_pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, Some(2), 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, indent: Option<usize>, depth: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Int(v) => write!(out, "{}", v).unwrap(),
            Json::Float(f) => {
                if f.is_finite() {
                    if f.fract() == 0.0 && f.abs() < 1e16 {
                        write!(out, "{:.1}", f).unwrap();
                    } else {
                        write!(out, "{}", f).unwrap();
                    }
                } else {
                    out.push_str("null");
                }
            }
            Json::Str(s) => write_string(out, s),
            Json::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    newline(out, indent, depth + 1);
                    item.write(out, indent, depth + 1);
                }
                newline(out, indent, depth);
                out.push(']');
            }
            Json::Object(items) => {
                if items.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (i, (k, v)) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    newline(out, indent, depth + 1);
                    write_string(out, k);
                    out.push(':');
                    if indent.is_some() {
                        out.push(' ');
                    }
                    v.write(out, indent, depth + 1);
                }
                newline(out, indent, depth);
                out.push('}');
            }
        }
    }
}

impl std::fmt::Display for Json {
    /// Compact JSON.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = String::new();
        self.write(&mut out, None, 0);
        f.write_str(&out)
    }
}

fn newline(out: &mut String, indent: Option<usize>, depth: usize) {
    if let Some(n) = indent {
        out.push('\n');
        for _ in 0..n * depth {
            out.push(' ');
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl From<bool> for Json {
    fn from(v: bool) -> Json {
        Json::Bool(v)
    }
}
impl From<i64> for Json {
    fn from(v: i64) -> Json {
        Json::Int(v)
    }
}
impl From<i32> for Json {
    fn from(v: i32) -> Json {
        Json::Int(v as i64)
    }
}
impl From<u32> for Json {
    fn from(v: u32) -> Json {
        Json::Int(v as i64)
    }
}
impl From<usize> for Json {
    fn from(v: usize) -> Json {
        Json::Int(v as i64)
    }
}
impl From<f64> for Json {
    fn from(v: f64) -> Json {
        Json::Float(v)
    }
}
impl From<&str> for Json {
    fn from(v: &str) -> Json {
        Json::Str(v.to_string())
    }
}
impl From<String> for Json {
    fn from(v: String) -> Json {
        Json::Str(v)
    }
}
impl From<Vec<Json>> for Json {
    fn from(v: Vec<Json>) -> Json {
        Json::Array(v)
    }
}
impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(v: Option<T>) -> Json {
        match v {
            Some(v) => v.into(),
            None => Json::Null,
        }
    }
}

struct JsonParser<'a> {
    s: &'a [u8],
    src: &'a str,
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn err(&self, msg: &str) -> JsonError {
        JsonError {
            offset: self.pos,
            message: msg.to_string(),
        }
    }

    fn ws(&mut self) {
        while let Some(c) = self.s.get(self.pos) {
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn value(&mut self) -> Result<Json, JsonError> {
        match self.s.get(self.pos) {
            None => Err(self.err("unexpected end of input")),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn literal(&mut self, word: &str, v: Json) -> Result<Json, JsonError> {
        if self.src[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(v)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.pos;
        if self.s.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        let mut is_float = false;
        while let Some(c) = self.s.get(self.pos) {
            match c {
                b'0'..=b'9' => self.pos += 1,
                b'.' | b'e' | b'E' | b'+' | b'-' => {
                    is_float = true;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        let text = &self.src[start..self.pos];
        if !is_float {
            if let Ok(v) = text.parse::<i64>() {
                return Ok(Json::Int(v));
            }
        }
        text.parse::<f64>().map(Json::Float).map_err(|_| JsonError {
            offset: start,
            message: "malformed number".into(),
        })
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let c = *self
                .s
                .get(self.pos)
                .ok_or_else(|| self.err("unterminated string"))?;
            match c {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let e = *self
                        .s
                        .get(self.pos)
                        .ok_or_else(|| self.err("unterminated escape"))?;
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..0xDC00).contains(&code) {
                                // surrogate pair
                                if self.src[self.pos..].starts_with("\\u") {
                                    self.pos += 2;
                                    let low = self.hex4()?;
                                    if (0xDC00..0xE000).contains(&low) {
                                        code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                    } else {
                                        return Err(self.err("invalid surrogate pair"));
                                    }
                                } else {
                                    return Err(self.err("lone surrogate"));
                                }
                            }
                            out.push(
                                char::from_u32(code)
                                    .ok_or_else(|| self.err("invalid code point"))?,
                            );
                        }
                        _ => return Err(self.err("invalid escape")),
                    }
                }
                _ => {
                    let ch = self.src[self.pos..].chars().next().unwrap();
                    if (ch as u32) < 0x20 {
                        return Err(self.err("control character in string"));
                    }
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let hex = self
            .src
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.err("short \\u escape"))?;
        let v = u32::from_str_radix(hex, 16).map_err(|_| self.err("invalid \\u escape"))?;
        self.pos += 4;
        Ok(v)
    }

    fn array(&mut self) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.ws();
        if self.s.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value()?);
            self.ws();
            match self.s.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.err("expected `,` or `]`")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.ws();
        if self.s.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(Json::Object(items));
        }
        loop {
            self.ws();
            if self.s.get(self.pos) != Some(&b'"') {
                return Err(self.err("expected string key"));
            }
            let key = self.string()?;
            self.ws();
            if self.s.get(self.pos) != Some(&b':') {
                return Err(self.err("expected `:`"));
            }
            self.pos += 1;
            self.ws();
            let value = self.value()?;
            items.push((key, value));
            self.ws();
            match self.s.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(items));
                }
                _ => return Err(self.err("expected `,` or `}`")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let src = r#"{"a":1,"b":-2.5,"c":"x\"y\n","d":[true,false,null],"e":{},"f":[],"g":1793196612000000000,"h":1e3}"#;
        let v = Json::parse(src).unwrap();
        assert_eq!(v.i64_field("g"), Some(1793196612000000000));
        assert_eq!(v.get("h"), Some(&Json::Float(1000.0)));
        let again = Json::parse(&v.to_string()).unwrap();
        assert_eq!(v, again);
        assert_eq!(
            v.to_string(),
            r#"{"a":1,"b":-2.5,"c":"x\"y\n","d":[true,false,null],"e":{},"f":[],"g":1793196612000000000,"h":1000.0}"#
        );
    }

    #[test]
    fn unicode() {
        let v = Json::parse(r#""é😀 ü""#).unwrap();
        assert_eq!(v, Json::Str("é😀 ü".into()));
    }

    #[test]
    fn errors() {
        for s in [
            "",
            "{",
            "[1,]",
            "{\"a\" 1}",
            "tru",
            "\"abc",
            "01x",
            "{\"a\":1} x",
        ] {
            assert!(Json::parse(s).is_err(), "should fail: {}", s);
        }
    }

    #[test]
    fn pretty() {
        let v = Json::object()
            .with("a", 1)
            .with("b", vec![Json::Int(1), Json::Int(2)]);
        assert_eq!(
            v.to_pretty(),
            "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2\n  ]\n}\n"
        );
    }
}
