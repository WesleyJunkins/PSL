//! Control-plane framing (docs/03 §Control plane): `u32 length` + UTF-8 JSON object.

use std::io::{self, Read, Write};

use crate::json::Json;

pub const MAX_MESSAGE_LEN: usize = 16 << 20;

/// Read one message. `Ok(None)` on a clean end of stream before any byte.
pub fn read_message(r: &mut impl Read) -> io::Result<Option<Json>> {
    let mut lb = [0u8; 4];
    match r.read_exact(&mut lb) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(lb) as usize;
    if len > MAX_MESSAGE_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("control message too long: {}", len),
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let text =
        std::str::from_utf8(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let j =
        Json::parse(text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    if j.get("type").and_then(Json::as_str).is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control message has no \"type\"",
        ));
    }
    Ok(Some(j))
}

pub fn encode_message(j: &Json) -> Vec<u8> {
    let body = j.to_string().into_bytes();
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

pub fn write_message(w: &mut impl Write, j: &Json) -> io::Result<()> {
    w.write_all(&encode_message(j))?;
    w.flush()
}

/// Build a message with a `type` field.
pub fn msg(ty: &str) -> Json {
    Json::object().with("type", ty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let m = msg("hello").with("node_id", "x").with("protocol", 1);
        let bytes = encode_message(&m);
        let back = read_message(&mut &bytes[..]).unwrap().unwrap();
        assert_eq!(back, m);
        assert!(read_message(&mut &encode_message(&Json::object())[..]).is_err());
    }
}
