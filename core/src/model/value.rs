//! Conversion from syntax values to JSON, with the enumeration-symbol rule (docs/04).

use crate::diag::{Diagnostic, Diagnostics};
use crate::json::Json;
use crate::syntax::Value;

/// Convert a literal value to JSON. Bare identifiers are an error (E0005) unless `allow_symbols`.
pub fn to_json(v: &Value, allow_symbols: bool, diags: &mut Diagnostics) -> Json {
    match v {
        Value::Str(s, _) => Json::Str(s.clone()),
        Value::Int(i, _) => Json::Int(*i),
        Value::Float(f, _) => Json::Float(*f),
        Value::Bool(b, _) => Json::Bool(*b),
        Value::Null(_) => Json::Null,
        Value::Symbol(s, span) => {
            if !allow_symbols {
                diags.push(
                    Diagnostic::error(
                        "E0005",
                        format!("bare identifier `{}` used where a string is expected", s),
                    )
                    .primary(*span, "did you mean a string?")
                    .help(format!("write `\"{}\"`", s)),
                );
            }
            Json::Str(s.clone())
        }
        Value::List(items, _) => Json::Array(
            items
                .iter()
                .map(|i| to_json(i, allow_symbols, diags))
                .collect(),
        ),
        Value::Object(attrs, _) => Json::Object(
            attrs
                .iter()
                .map(|a| (a.name.name.clone(), to_json(&a.value, allow_symbols, diags)))
                .collect(),
        ),
    }
}

/// Render a JSON value the way it would be written in PSL, for messages.
pub fn describe_json(j: &Json) -> String {
    match j {
        Json::Str(s) => format!("\"{}\"", s),
        other => other.to_string(),
    }
}
