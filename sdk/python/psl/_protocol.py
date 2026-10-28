"""Wire protocol version 1: byte-level codec (docs/03-wire-protocol.md).

Everything here is pure: bytes in, values out, no sockets. All integers are
little-endian, all text is UTF-8. The data frame header is 16 bytes::

    offset size field    offset size field
    0      4    length   8      2    port
    4      1    version  10     2    source
    5      1    type     12     4    seq
    6      2    flags
"""

from __future__ import annotations

import json
import struct
from dataclasses import dataclass
from typing import Any

import numpy as np

from ._values import NUMPY_DTYPES, Chunk, Event, EventHeader, Header, SignalHeader

PROTOCOL_VERSION: int = 1

HEADER_SIZE: int = 16
MAX_FRAME_LENGTH: int = 64 * 1024 * 1024
MAX_CONTROL_LENGTH: int = 16 * 1024 * 1024
SEQ_MASK: int = 0xFFFF_FFFF

# Frame types.
IDENTIFY: int = 0x00
STREAM_HEADER: int = 0x01
SIGNAL: int = 0x02
EVENT: int = 0x03
END_STREAM: int = 0x04

FRAME_TYPE_NAMES: dict[int, str] = {
    IDENTIFY: "IDENTIFY",
    STREAM_HEADER: "STREAM_HEADER",
    SIGNAL: "SIGNAL",
    EVENT: "EVENT",
    END_STREAM: "END_STREAM",
}

# SIGNAL flags.
FLAG_HAS_TIMESTAMPS: int = 0x0001

# SIGNAL dtype codes.
DTYPE_CODES: dict[str, int] = {"f32": 0, "f64": 1, "i16": 2, "i32": 3}
DTYPE_NAMES: dict[int, str] = {code: name for name, code in DTYPE_CODES.items()}

_FRAME_HEADER = struct.Struct("<IBBHHHI")   # length, version, type, flags, port, source, seq
_SIGNAL_HEAD = struct.Struct("<qqIHBB")     # t0, t_device, n_samples, n_channels, dtype, reserved
_EVENT_HEAD = struct.Struct("<qq")          # t, t_device
_CONTROL_LEN = struct.Struct("<I")
_TIMESTAMP_DTYPE = np.dtype("<i8")

SIGNAL_HEAD_SIZE: int = _SIGNAL_HEAD.size   # 24
EVENT_HEAD_SIZE: int = _EVENT_HEAD.size     # 16
CONTROL_LEN_SIZE: int = _CONTROL_LEN.size   # 4

assert _FRAME_HEADER.size == HEADER_SIZE


class ProtocolError(ValueError):
    """Malformed bytes or a message that violates docs/03."""


def _json_default(obj: Any) -> Any:
    """Make numpy scalars/arrays and other odd values JSON-serialisable."""
    if isinstance(obj, np.generic):
        return obj.item()
    if isinstance(obj, np.ndarray):
        return obj.tolist()
    if isinstance(obj, (set, frozenset, tuple)):
        return list(obj)
    if isinstance(obj, bytes):
        return obj.decode("utf-8", "replace")
    return str(obj)


def dumps(obj: Any) -> bytes:
    """Compact UTF-8 JSON as used everywhere on the wire."""
    return json.dumps(obj, separators=(",", ":"), ensure_ascii=False, default=_json_default).encode("utf-8")


def loads_object(data: bytes | bytearray | memoryview, what: str) -> dict[str, Any]:
    try:
        obj = json.loads(bytes(data).decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ProtocolError(f"{what}: invalid JSON: {exc}") from exc
    if not isinstance(obj, dict):
        raise ProtocolError(f"{what}: expected a JSON object, got {type(obj).__name__}")
    return obj


# --------------------------------------------------------------------------- #
# Data frame header
# --------------------------------------------------------------------------- #


@dataclass(frozen=True)
class FrameHeader:
    """The 16-byte header that starts every data frame."""

    length: int
    version: int
    type: int
    flags: int
    port: int
    source: int
    seq: int

    def pack(self) -> bytes:
        return _FRAME_HEADER.pack(
            self.length, self.version, self.type, self.flags, self.port, self.source, self.seq
        )

    @classmethod
    def unpack(cls, buf: bytes | bytearray | memoryview) -> FrameHeader:
        if len(buf) < HEADER_SIZE:
            raise ProtocolError(f"frame header needs {HEADER_SIZE} bytes, got {len(buf)}")
        return cls(*_FRAME_HEADER.unpack_from(buf, 0))

    @property
    def payload_length(self) -> int:
        return self.length - HEADER_SIZE

    def validate(self) -> None:
        if self.length < HEADER_SIZE:
            raise ProtocolError(f"frame length {self.length} is smaller than the header")
        if self.length > MAX_FRAME_LENGTH:
            raise ProtocolError(f"frame length {self.length} exceeds the 64 MiB maximum")


def pack_header(
    frame_type: int,
    payload_length: int,
    *,
    port: int = 0,
    source: int = 0,
    seq: int = 0,
    flags: int = 0,
    version: int = PROTOCOL_VERSION,
) -> bytes:
    return _FRAME_HEADER.pack(
        HEADER_SIZE + payload_length, version, frame_type, flags, port, source, seq & SEQ_MASK
    )


def unpack_header(buf: bytes | bytearray | memoryview) -> FrameHeader:
    return FrameHeader.unpack(buf)


def pack_frame(
    frame_type: int,
    payload: bytes = b"",
    *,
    port: int = 0,
    source: int = 0,
    seq: int = 0,
    flags: int = 0,
    version: int = PROTOCOL_VERSION,
) -> bytes:
    """A complete frame: header followed by payload."""
    if len(payload) + HEADER_SIZE > MAX_FRAME_LENGTH:
        raise ProtocolError(f"frame payload of {len(payload)} bytes exceeds the 64 MiB maximum")
    return pack_header(frame_type, len(payload), port=port, source=source, seq=seq, flags=flags, version=version) + payload


def unpack_frame(buf: bytes | bytearray | memoryview) -> tuple[FrameHeader, bytes]:
    """Split one complete frame into its header and payload, checking the length."""
    header = FrameHeader.unpack(buf)
    header.validate()
    if len(buf) != header.length:
        raise ProtocolError(f"frame header says {header.length} bytes but buffer holds {len(buf)}")
    return header, bytes(buf[HEADER_SIZE:])


# --------------------------------------------------------------------------- #
# Payload codecs
# --------------------------------------------------------------------------- #


def encode_signal(chunk: Chunk, dtype: str = "f32") -> tuple[bytes, int]:
    """SIGNAL payload for ``chunk`` with samples stored as ``dtype``.

    Returns ``(payload, flags)``. The data is converted to the stream dtype,
    C-contiguous (sample-major) and explicitly little-endian.
    """
    if dtype not in DTYPE_CODES:
        raise ValueError(f"unknown dtype {dtype!r}")
    data = np.ascontiguousarray(chunk.data, dtype=NUMPY_DTYPES[dtype])
    if data.ndim != 2:
        raise ValueError(f"chunk data must be 2-D, got shape {data.shape}")
    n_samples, n_channels = data.shape
    if n_channels > 0xFFFF:
        raise ValueError(f"too many channels for the wire ({n_channels})")
    parts = [
        _SIGNAL_HEAD.pack(chunk.t0, chunk.t_device, n_samples, n_channels, DTYPE_CODES[dtype], 0),
        data.tobytes(order="C"),
    ]
    flags = 0
    if chunk.timestamps is not None:
        ts = np.ascontiguousarray(chunk.timestamps, dtype=_TIMESTAMP_DTYPE).reshape(-1)
        if ts.shape[0] != n_samples:
            raise ValueError("timestamps length does not match n_samples")
        parts.append(ts.tobytes(order="C"))
        flags |= FLAG_HAS_TIMESTAMPS
    return b"".join(parts), flags


def decode_signal(
    payload: bytes | bytearray | memoryview,
    flags: int = 0,
    *,
    seq: int = 0,
    srate: float | None = None,
) -> Chunk:
    """Parse a SIGNAL payload. The returned chunk owns writable memory."""
    if len(payload) < SIGNAL_HEAD_SIZE:
        raise ProtocolError(f"SIGNAL payload needs at least {SIGNAL_HEAD_SIZE} bytes, got {len(payload)}")
    t0, t_device, n_samples, n_channels, code, _reserved = _SIGNAL_HEAD.unpack_from(payload, 0)
    if code not in DTYPE_NAMES:
        raise ProtocolError(f"unknown SIGNAL dtype code {code}")
    npdt = NUMPY_DTYPES[DTYPE_NAMES[code]]
    count = n_samples * n_channels
    sample_bytes = count * npdt.itemsize
    has_ts = bool(flags & FLAG_HAS_TIMESTAMPS)
    expected = SIGNAL_HEAD_SIZE + sample_bytes + (n_samples * 8 if has_ts else 0)
    if len(payload) != expected:
        raise ProtocolError(
            f"SIGNAL payload length {len(payload)} does not match {n_samples}x{n_channels} "
            f"{DTYPE_NAMES[code]} samples{' with timestamps' if has_ts else ''} ({expected} bytes)"
        )
    data = np.frombuffer(payload, dtype=npdt, count=count, offset=SIGNAL_HEAD_SIZE)
    data = data.reshape(n_samples, n_channels).copy()
    timestamps: np.ndarray | None = None
    if has_ts:
        timestamps = np.frombuffer(
            payload, dtype=_TIMESTAMP_DTYPE, count=n_samples, offset=SIGNAL_HEAD_SIZE + sample_bytes
        ).copy()
    return Chunk(t0=t0, data=data, timestamps=timestamps, t_device=t_device, seq=seq, srate=srate)


def encode_event(event: Event) -> bytes:
    body = dumps({"label": event.label, "data": event.data})
    return _EVENT_HEAD.pack(event.t, event.t_device) + body


def decode_event(payload: bytes | bytearray | memoryview, *, seq: int = 0) -> Event:
    if len(payload) < EVENT_HEAD_SIZE:
        raise ProtocolError(f"EVENT payload needs at least {EVENT_HEAD_SIZE} bytes, got {len(payload)}")
    t, t_device = _EVENT_HEAD.unpack_from(payload, 0)
    body = loads_object(payload[EVENT_HEAD_SIZE:], "EVENT body")
    if "label" not in body:
        raise ProtocolError("EVENT body has no label")
    return Event(t=t, label=str(body["label"]), data=body.get("data"), t_device=t_device, seq=seq)


def encode_stream_header(header: Header) -> bytes:
    return dumps(header.to_json())


def decode_stream_header(payload: bytes | bytearray | memoryview) -> Header:
    obj = loads_object(payload, "STREAM_HEADER")
    kind = obj.get("type")
    try:
        if kind == "signal":
            return SignalHeader.from_json(obj)
        if kind == "event":
            return EventHeader.from_json(obj)
    except ValueError as exc:
        raise ProtocolError(f"STREAM_HEADER: {exc}") from exc
    raise ProtocolError(f"STREAM_HEADER has unknown type {kind!r}")


def encode_identify(node_id: str, session: str) -> bytes:
    return dumps({"node_id": node_id, "session": session})


def decode_identify(payload: bytes | bytearray | memoryview) -> tuple[str, str]:
    obj = loads_object(payload, "IDENTIFY")
    try:
        return str(obj["node_id"]), str(obj["session"])
    except KeyError as exc:
        raise ProtocolError(f"IDENTIFY is missing {exc.args[0]!r}") from exc


# --------------------------------------------------------------------------- #
# Whole frames
# --------------------------------------------------------------------------- #


def identify_frame(node_id: str, session: str) -> bytes:
    return pack_frame(IDENTIFY, encode_identify(node_id, session))


def stream_header_frame(port: int, seq: int, header: Header) -> bytes:
    return pack_frame(STREAM_HEADER, encode_stream_header(header), port=port, seq=seq)


def signal_frame(port: int, seq: int, chunk: Chunk, dtype: str = "f32") -> bytes:
    payload, flags = encode_signal(chunk, dtype)
    return pack_frame(SIGNAL, payload, port=port, seq=seq, flags=flags)


def event_frame(port: int, seq: int, event: Event) -> bytes:
    return pack_frame(EVENT, encode_event(event), port=port, seq=seq)


def end_stream_frame(port: int, seq: int) -> bytes:
    return pack_frame(END_STREAM, b"", port=port, seq=seq)


# --------------------------------------------------------------------------- #
# Control plane framing: u32 length + UTF-8 JSON object with a "type" field
# --------------------------------------------------------------------------- #


def encode_control(msg: dict[str, Any]) -> bytes:
    if not isinstance(msg, dict) or not isinstance(msg.get("type"), str):
        raise ValueError("control message must be an object with a string 'type'")
    body = dumps(msg)
    if len(body) > MAX_CONTROL_LENGTH:
        raise ProtocolError(f"control message of {len(body)} bytes exceeds the 16 MiB maximum")
    return _CONTROL_LEN.pack(len(body)) + body


def decode_control(body: bytes | bytearray | memoryview) -> dict[str, Any]:
    """Parse a control message body (the bytes after the u32 length)."""
    if len(body) > MAX_CONTROL_LENGTH:
        raise ProtocolError(f"control message of {len(body)} bytes exceeds the 16 MiB maximum")
    msg = loads_object(body, "control message")
    if not isinstance(msg.get("type"), str):
        raise ProtocolError("control message has no string 'type' field")
    return msg


def unpack_control_length(buf: bytes | bytearray | memoryview) -> int:
    if len(buf) < CONTROL_LEN_SIZE:
        raise ProtocolError("control length prefix needs 4 bytes")
    (n,) = _CONTROL_LEN.unpack_from(buf, 0)
    if n > MAX_CONTROL_LENGTH:
        raise ProtocolError(f"control message of {n} bytes exceeds the 16 MiB maximum")
    return n


def split_control(buf: bytes | bytearray | memoryview) -> tuple[dict[str, Any], int]:
    """Decode the first control message in ``buf``; returns ``(message, bytes_consumed)``."""
    n = unpack_control_length(buf)
    end = CONTROL_LEN_SIZE + n
    if len(buf) < end:
        raise ProtocolError("control message is truncated")
    return decode_control(buf[CONTROL_LEN_SIZE:end]), end
