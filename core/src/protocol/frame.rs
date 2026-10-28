//! Data-plane frames (docs/03 §Data plane).

use std::io::{self, Read, Write};
use std::sync::Arc;

use crate::json::Json;

pub const HEADER_LEN: usize = 16;
pub const MAX_FRAME_LEN: usize = 64 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    Identify = 0x00,
    StreamHeader = 0x01,
    Signal = 0x02,
    Event = 0x03,
    EndStream = 0x04,
}

impl FrameType {
    pub fn from_u8(v: u8) -> Option<FrameType> {
        Some(match v {
            0x00 => FrameType::Identify,
            0x01 => FrameType::StreamHeader,
            0x02 => FrameType::Signal,
            0x03 => FrameType::Event,
            0x04 => FrameType::EndStream,
            _ => return None,
        })
    }
    pub fn name(&self) -> &'static str {
        match self {
            FrameType::Identify => "IDENTIFY",
            FrameType::StreamHeader => "STREAM_HEADER",
            FrameType::Signal => "SIGNAL",
            FrameType::Event => "EVENT",
            FrameType::EndStream => "END_STREAM",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub version: u8,
    pub ty: u8,
    pub flags: u16,
    pub port: u16,
    pub source: u16,
    pub seq: u32,
}

impl FrameHeader {
    pub fn encode(&self, payload_len: usize) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..4].copy_from_slice(&((HEADER_LEN + payload_len) as u32).to_le_bytes());
        b[4] = self.version;
        b[5] = self.ty;
        b[6..8].copy_from_slice(&self.flags.to_le_bytes());
        b[8..10].copy_from_slice(&self.port.to_le_bytes());
        b[10..12].copy_from_slice(&self.source.to_le_bytes());
        b[12..16].copy_from_slice(&self.seq.to_le_bytes());
        b
    }

    /// Decode a header; returns the header and the total frame length.
    pub fn decode(b: &[u8; HEADER_LEN]) -> (FrameHeader, usize) {
        let len = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
        (
            FrameHeader {
                version: b[4],
                ty: b[5],
                flags: u16::from_le_bytes([b[6], b[7]]),
                port: u16::from_le_bytes([b[8], b[9]]),
                source: u16::from_le_bytes([b[10], b[11]]),
                seq: u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
            },
            len,
        )
    }

    pub fn frame_type(&self) -> Option<FrameType> {
        FrameType::from_u8(self.ty)
    }
}

/// A frame with a shared payload so fan-out never copies sample data.
#[derive(Clone, Debug)]
pub struct Frame {
    pub header: FrameHeader,
    pub payload: Arc<Vec<u8>>,
}

impl Frame {
    pub fn new(ty: FrameType, flags: u16, port: u16, seq: u32, payload: Vec<u8>) -> Frame {
        Frame {
            header: FrameHeader {
                version: crate::PROTOCOL_VERSION,
                ty: ty as u8,
                flags,
                port,
                source: 0,
                seq,
            },
            payload: Arc::new(payload),
        }
    }

    pub fn identify(node_id: &str, session: &str) -> Frame {
        let body = Json::object()
            .with("node_id", node_id)
            .with("session", session)
            .to_string();
        Frame::new(FrameType::Identify, 0, 0, 0, body.into_bytes())
    }

    pub fn total_len(&self) -> usize {
        HEADER_LEN + self.payload.len()
    }

    /// Encode the whole frame into one buffer.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.total_len());
        out.extend_from_slice(&self.header.encode(self.payload.len()));
        out.extend_from_slice(&self.payload);
        out
    }
}

/// Read one frame. `Ok(None)` on a clean end of stream before any byte.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut hb = [0u8; HEADER_LEN];
    match r.read_exact(&mut hb) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let (header, len) = FrameHeader::decode(&hb);
    if !(HEADER_LEN..=MAX_FRAME_LEN).contains(&len) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("bad frame length {}", len),
        ));
    }
    if header.version != crate::PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported protocol version {}", header.version),
        ));
    }
    let mut payload = vec![0u8; len - HEADER_LEN];
    r.read_exact(&mut payload)?;
    Ok(Some(Frame {
        header,
        payload: Arc::new(payload),
    }))
}

pub fn write_frame(w: &mut impl Write, frame: &Frame) -> io::Result<()> {
    w.write_all(&frame.header.encode(frame.payload.len()))?;
    w.write_all(&frame.payload)?;
    w.flush()
}

// ----- payload views -----

pub const FLAG_HAS_TIMESTAMPS: u16 = 0x0001;
pub const SIGNAL_PREFIX_LEN: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dtype {
    F32 = 0,
    F64 = 1,
    I16 = 2,
    I32 = 3,
}

impl Dtype {
    pub fn from_u8(v: u8) -> Option<Dtype> {
        Some(match v {
            0 => Dtype::F32,
            1 => Dtype::F64,
            2 => Dtype::I16,
            3 => Dtype::I32,
            _ => return None,
        })
    }
    pub fn size(&self) -> usize {
        match self {
            Dtype::F32 | Dtype::I32 => 4,
            Dtype::F64 => 8,
            Dtype::I16 => 2,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Dtype::F32 => "f32",
            Dtype::F64 => "f64",
            Dtype::I16 => "i16",
            Dtype::I32 => "i32",
        }
    }
}

/// The fixed prefix of a `SIGNAL` payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalInfo {
    pub t0: i64,
    pub t_device: i64,
    pub n_samples: u32,
    pub n_channels: u16,
    pub dtype: Dtype,
}

impl SignalInfo {
    pub fn decode(payload: &[u8]) -> Option<SignalInfo> {
        if payload.len() < SIGNAL_PREFIX_LEN {
            return None;
        }
        let i64_at = |o: usize| i64::from_le_bytes(payload[o..o + 8].try_into().unwrap());
        Some(SignalInfo {
            t0: i64_at(0),
            t_device: i64_at(8),
            n_samples: u32::from_le_bytes(payload[16..20].try_into().unwrap()),
            n_channels: u16::from_le_bytes(payload[20..22].try_into().unwrap()),
            dtype: Dtype::from_u8(payload[22])?,
        })
    }

    pub fn encode(&self) -> [u8; SIGNAL_PREFIX_LEN] {
        let mut b = [0u8; SIGNAL_PREFIX_LEN];
        b[0..8].copy_from_slice(&self.t0.to_le_bytes());
        b[8..16].copy_from_slice(&self.t_device.to_le_bytes());
        b[16..20].copy_from_slice(&self.n_samples.to_le_bytes());
        b[20..22].copy_from_slice(&self.n_channels.to_le_bytes());
        b[22] = self.dtype as u8;
        b
    }

    pub fn samples_len(&self) -> usize {
        self.n_samples as usize * self.n_channels as usize * self.dtype.size()
    }

    /// Decode up to `max` samples of the first channel as f64, for display.
    pub fn first_values(&self, payload: &[u8], max: usize) -> Vec<f64> {
        let data = &payload[SIGNAL_PREFIX_LEN..];
        let stride = self.n_channels as usize * self.dtype.size();
        let mut out = Vec::new();
        for i in 0..(self.n_samples as usize).min(max) {
            let o = i * stride;
            if o + self.dtype.size() > data.len() {
                break;
            }
            let v = match self.dtype {
                Dtype::F32 => f32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as f64,
                Dtype::F64 => f64::from_le_bytes(data[o..o + 8].try_into().unwrap()),
                Dtype::I16 => i16::from_le_bytes(data[o..o + 2].try_into().unwrap()) as f64,
                Dtype::I32 => i32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as f64,
            };
            out.push(v);
        }
        out
    }
}

/// The fixed prefix of an `EVENT` payload plus its JSON body.
#[derive(Clone, Debug, PartialEq)]
pub struct EventInfo {
    pub t: i64,
    pub t_device: i64,
    pub body: Json,
}

impl EventInfo {
    pub fn decode(payload: &[u8]) -> Option<EventInfo> {
        if payload.len() < 16 {
            return None;
        }
        let t = i64::from_le_bytes(payload[0..8].try_into().unwrap());
        let t_device = i64::from_le_bytes(payload[8..16].try_into().unwrap());
        let body = Json::parse(std::str::from_utf8(&payload[16..]).ok()?).ok()?;
        Some(EventInfo { t, t_device, body })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.t.to_le_bytes());
        out.extend_from_slice(&self.t_device.to_le_bytes());
        out.extend_from_slice(self.body.to_string().as_bytes());
        out
    }
}

/// Parse a `STREAM_HEADER` payload.
pub fn decode_stream_header(payload: &[u8]) -> Option<Json> {
    Json::parse(std::str::from_utf8(payload).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trip() {
        let h = FrameHeader {
            version: 1,
            ty: 2,
            flags: 1,
            port: 3,
            source: 4,
            seq: 99,
        };
        let b = h.encode(10);
        assert_eq!(&b[0..4], &26u32.to_le_bytes());
        let (d, len) = FrameHeader::decode(&b);
        assert_eq!(d, h);
        assert_eq!(len, 26);
    }

    #[test]
    fn signal_round_trip() {
        let info = SignalInfo {
            t0: -5,
            t_device: 7,
            n_samples: 2,
            n_channels: 2,
            dtype: Dtype::F32,
        };
        let mut payload = info.encode().to_vec();
        for v in [1.0f32, 2.0, 3.0, 4.0] {
            payload.extend_from_slice(&v.to_le_bytes());
        }
        let frame = Frame::new(FrameType::Signal, 0, 1, 5, payload);
        let bytes = frame.to_bytes();
        let back = read_frame(&mut &bytes[..]).unwrap().unwrap();
        assert_eq!(back.header, frame.header);
        let d = SignalInfo::decode(&back.payload).unwrap();
        assert_eq!(d, info);
        assert_eq!(d.first_values(&back.payload, 10), vec![1.0, 3.0]);
        assert_eq!(read_frame(&mut &b""[..]).unwrap().map(|_| ()), None);
    }

    #[test]
    fn event_round_trip() {
        let e = EventInfo {
            t: 1,
            t_device: 0,
            body: Json::object()
                .with("label", "beat")
                .with("data", Json::Null),
        };
        let d = EventInfo::decode(&e.encode()).unwrap();
        assert_eq!(d, e);
    }
}
