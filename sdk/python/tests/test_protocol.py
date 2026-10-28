"""Byte-level tests of the wire codec against docs/03-wire-protocol.md."""

import json
import struct
import unittest

import numpy as np

import psl
from psl import _protocol as P


def le_u32(v): return struct.pack("<I", v)
def le_i64(v): return struct.pack("<q", v)


class FrameHeaderTests(unittest.TestCase):
    def test_layout_is_16_bytes_little_endian(self):
        hdr = P.FrameHeader(length=0x11223344, version=1, type=P.SIGNAL, flags=0x0001, port=3, source=2, seq=0xDEADBEEF)
        packed = hdr.pack()
        self.assertEqual(len(packed), 16)
        self.assertEqual(packed, bytes([0x44, 0x33, 0x22, 0x11, 0x01, 0x02, 0x01, 0x00, 0x03, 0x00, 0x02, 0x00, 0xEF, 0xBE, 0xAD, 0xDE]))
        self.assertEqual(P.FrameHeader.unpack(packed), hdr)
        self.assertEqual(P.unpack_header(packed + b"extra"), hdr)

    def test_pack_frame_fills_length_and_version(self):
        frame = P.pack_frame(P.END_STREAM, b"", port=1, seq=9)
        self.assertEqual(frame, le_u32(16) + bytes([1, 4, 0, 0, 1, 0, 0, 0]) + le_u32(9))
        hdr, payload = P.unpack_frame(frame)
        self.assertEqual((hdr.type, hdr.port, hdr.source, hdr.seq, hdr.flags, payload), (P.END_STREAM, 1, 0, 9, 0, b""))

    def test_seq_wraps_at_2_32(self):
        hdr = P.FrameHeader.unpack(P.pack_header(P.SIGNAL, 0, seq=(1 << 32) + 5))
        self.assertEqual(hdr.seq, 5)

    def test_invalid_headers(self):
        with self.assertRaises(P.ProtocolError):
            P.FrameHeader.unpack(b"\x00" * 15)
        with self.assertRaises(P.ProtocolError):
            P.unpack_frame(le_u32(8) + bytes(12))              # length < 16
        with self.assertRaises(P.ProtocolError):
            P.unpack_frame(le_u32(32) + bytes(12))             # length != buffer
        too_big = P.FrameHeader(length=P.MAX_FRAME_LENGTH + 1, version=1, type=0, flags=0, port=0, source=0, seq=0)
        with self.assertRaises(P.ProtocolError):
            too_big.validate()


class SignalTests(unittest.TestCase):
    def test_hand_built_signal_frame(self):
        chunk = psl.Chunk(t0=1000, data=np.array([[1.0], [2.0]], np.float32))
        frame = P.signal_frame(3, 7, chunk, "f32")
        expected = (
            le_u32(48)                      # 16 header + 24 signal head + 2 * 4 bytes
            + bytes([1])                    # version
            + bytes([0x02])                 # SIGNAL
            + b"\x00\x00"                   # flags
            + b"\x03\x00"                   # port 3
            + b"\x00\x00"                   # source 0
            + le_u32(7)                     # seq
            + le_i64(1000)                  # t0
            + le_i64(0)                     # t_device
            + le_u32(2)                     # n_samples
            + b"\x01\x00"                   # n_channels
            + bytes([0])                    # dtype f32
            + bytes([0])                    # reserved
            + b"\x00\x00\x80\x3f"           # 1.0f
            + b"\x00\x00\x00\x40"           # 2.0f
        )
        self.assertEqual(frame, expected)

    def test_hand_built_signal_frame_with_timestamps_and_two_channels(self):
        chunk = psl.Chunk(
            t0=-5, t_device=99,
            data=np.array([[1, 2], [3, 4]], np.int16),
            timestamps=np.array([-5, 10], np.int64),
        )
        frame = P.signal_frame(0, 1, chunk, "i16")
        expected = (
            le_u32(16 + 24 + 8 + 16)
            + bytes([1, 0x02]) + b"\x01\x00"                 # flags HAS_TIMESTAMPS
            + b"\x00\x00" + b"\x00\x00" + le_u32(1)
            + le_i64(-5) + le_i64(99) + le_u32(2) + b"\x02\x00" + bytes([2, 0])
            + b"\x01\x00\x02\x00\x03\x00\x04\x00"           # sample-major i16
            + le_i64(-5) + le_i64(10)
        )
        self.assertEqual(frame, expected)
        hdr, payload = P.unpack_frame(frame)
        self.assertEqual(hdr.flags & P.FLAG_HAS_TIMESTAMPS, 1)
        back = P.decode_signal(payload, hdr.flags, seq=hdr.seq)
        np.testing.assert_array_equal(back.data, [[1, 2], [3, 4]])
        self.assertEqual(back.data.dtype, np.dtype("<i2"))
        np.testing.assert_array_equal(back.timestamps, [-5, 10])
        self.assertEqual((back.t0, back.t_device, back.seq), (-5, 99, 1))

    def test_round_trip_every_dtype(self):
        rng = np.random.default_rng(0)
        for name, npdt in psl.NUMPY_DTYPES.items():
            with self.subTest(dtype=name):
                if npdt.kind == "f":
                    data = rng.standard_normal((7, 3)).astype(npdt)
                else:
                    data = rng.integers(-1000, 1000, size=(7, 3)).astype(npdt)
                for with_ts in (False, True):
                    ts = np.arange(7, dtype=np.int64) * 4_000_000 + 123 if with_ts else None
                    chunk = psl.Chunk(t0=123, data=data, timestamps=ts, t_device=-1)
                    payload, flags = P.encode_signal(chunk, name)
                    self.assertEqual(flags, P.FLAG_HAS_TIMESTAMPS if with_ts else 0)
                    self.assertEqual(len(payload), 24 + 21 * npdt.itemsize + (56 if with_ts else 0))
                    self.assertEqual(payload[22], P.DTYPE_CODES[name])
                    back = P.decode_signal(payload, flags, seq=5, srate=250.0)
                    np.testing.assert_array_equal(back.data, data)
                    self.assertEqual(back.data.dtype, npdt)
                    self.assertTrue(back.data.flags.c_contiguous and back.data.flags.writeable)
                    self.assertEqual((back.t0, back.t_device, back.seq, back.srate), (123, -1, 5, 250.0))
                    if with_ts:
                        np.testing.assert_array_equal(back.timestamps, ts)
                        self.assertEqual(back.time_of(3), 3 * 4_000_000 + 123)
                    else:
                        self.assertIsNone(back.timestamps)
                        self.assertEqual(back.time_of(1), 123 + 4_000_000)

    def test_encode_converts_dtype_and_layout(self):
        # float64, Fortran-ordered input encoded on an f32 stream must come out sample-major f32.
        data = np.asfortranarray(np.array([[1.5, 2.5], [3.5, 4.5]], np.float64))
        payload, _ = P.encode_signal(psl.Chunk(t0=0, data=data), "f32")
        self.assertEqual(payload[24:], np.array([1.5, 2.5, 3.5, 4.5], "<f4").tobytes())

    def test_empty_chunk(self):
        payload, flags = P.encode_signal(psl.Chunk(t0=1, data=np.zeros((0, 2), np.float32)), "f32")
        self.assertEqual(len(payload), 24)
        back = P.decode_signal(payload, flags)
        self.assertEqual(back.data.shape, (0, 2))

    def test_decode_rejects_bad_payloads(self):
        good, _ = P.encode_signal(psl.Chunk(t0=0, data=np.ones((2, 2), np.float32)), "f32")
        with self.assertRaises(P.ProtocolError):
            P.decode_signal(good[:-1], 0)                      # truncated
        with self.assertRaises(P.ProtocolError):
            P.decode_signal(good, P.FLAG_HAS_TIMESTAMPS)       # claims timestamps it does not carry
        bad_dtype = bytearray(good); bad_dtype[22] = 9
        with self.assertRaises(P.ProtocolError):
            P.decode_signal(bytes(bad_dtype), 0)
        with self.assertRaises(P.ProtocolError):
            P.decode_signal(b"\x00" * 10, 0)
        with self.assertRaises(ValueError):
            P.encode_signal(psl.Chunk(t0=0, data=np.ones((1, 1))), "u8")


class EventTests(unittest.TestCase):
    def test_hand_built_event_frame(self):
        ev = psl.Event(t=77, label="beat", data={"rr_ms": 812}, t_device=3)
        frame = P.event_frame(2, 4, ev)
        body = b'{"label":"beat","data":{"rr_ms":812}}'
        self.assertEqual(frame, le_u32(16 + 16 + len(body)) + bytes([1, 0x03, 0, 0, 2, 0, 0, 0]) + le_u32(4) + le_i64(77) + le_i64(3) + body)

    def test_round_trip(self):
        for data in (None, 1, 1.5, "x", [1, 2], {"a": {"b": [True, None]}}, "unicode ✓"):
            ev = psl.Event(t=-10, label="lbl", data=data, t_device=0)
            back = P.decode_event(P.encode_event(ev), seq=9)
            self.assertEqual(back, psl.Event(t=-10, label="lbl", data=data, t_device=0, seq=9))

    def test_numpy_values_in_data_are_serialised(self):
        ev = psl.Event(t=0, label="x", data={"v": np.float32(0.5), "arr": np.arange(3)})
        back = P.decode_event(P.encode_event(ev))
        self.assertEqual(back.data, {"v": 0.5, "arr": [0, 1, 2]})

    def test_rejects_bad_body(self):
        with self.assertRaises(P.ProtocolError):
            P.decode_event(le_i64(0) + le_i64(0) + b"[1,2]")
        with self.assertRaises(P.ProtocolError):
            P.decode_event(le_i64(0) + le_i64(0) + b'{"data":1}')
        with self.assertRaises(P.ProtocolError):
            P.decode_event(b"\x00" * 15)


class HeaderAndControlFrameTests(unittest.TestCase):
    def test_stream_header_round_trip_signal(self):
        hdr = psl.SignalHeader(srate=250, channels=[psl.Channel("ECG", "uV", "ecg"), "RESP"], dtype="i16",
                               source={"device": "Polar H10"}, meta={"k": [1, 2]})
        frame = P.stream_header_frame(1, 0, hdr)
        fh, payload = P.unpack_frame(frame)
        self.assertEqual((fh.type, fh.port, fh.seq), (P.STREAM_HEADER, 1, 0))
        obj = json.loads(payload)
        self.assertEqual(obj["type"], "signal")
        self.assertEqual(obj["channels"][1], {"label": "RESP", "unit": "au", "modality": "other"})
        back = P.decode_stream_header(payload)
        self.assertIsInstance(back, psl.SignalHeader)
        self.assertEqual(back.to_json(), hdr.to_json())

    def test_stream_header_round_trip_event(self):
        hdr = psl.EventHeader(labels=["beat"], schema={"rr_ms": "number"}, source={"x": 1})
        back = P.decode_stream_header(P.encode_stream_header(hdr))
        self.assertIsInstance(back, psl.EventHeader)
        self.assertEqual(back.to_json(), hdr.to_json())

    def test_stream_header_rejects_unknown_type(self):
        with self.assertRaises(P.ProtocolError):
            P.decode_stream_header(b'{"type":"table"}')
        with self.assertRaises(P.ProtocolError):
            P.decode_stream_header(b'{"type":"signal","srate":0,"channels":[]}')
        with self.assertRaises(P.ProtocolError):
            P.decode_stream_header(b'not json')

    def test_identify_frame(self):
        frame = P.identify_frame("bp", "20261028-143012-7f3a")
        fh, payload = P.unpack_frame(frame)
        self.assertEqual((fh.type, fh.port, fh.source, fh.seq, fh.flags), (P.IDENTIFY, 0, 0, 0, 0))
        self.assertEqual(json.loads(payload), {"node_id": "bp", "session": "20261028-143012-7f3a"})
        self.assertEqual(P.decode_identify(payload), ("bp", "20261028-143012-7f3a"))
        with self.assertRaises(P.ProtocolError):
            P.decode_identify(b'{"node_id":"bp"}')

    def test_end_stream_frame(self):
        fh, payload = P.unpack_frame(P.end_stream_frame(2, 11))
        self.assertEqual((fh.type, fh.port, fh.seq, fh.length, payload), (P.END_STREAM, 2, 11, 16, b""))

    def test_control_framing(self):
        msg = {"type": "hello", "role": "node", "node_id": "bp", "protocol": 1, "pid": 42, "sdk": "psl-python/0.1.0", "manifest_hash": ""}
        packed = P.encode_control(msg)
        body = packed[4:]
        self.assertEqual(packed[:4], le_u32(len(body)))
        self.assertEqual(json.loads(body), msg)
        self.assertEqual(P.decode_control(body), msg)
        decoded, used = P.split_control(packed + b"tail")
        self.assertEqual((decoded, used), (msg, len(packed)))
        self.assertEqual(P.encode_control({"type": "pong", "n": 7}), le_u32(21) + b'{"type":"pong","n":7}')

    def test_control_rejects_bad_messages(self):
        with self.assertRaises(ValueError):
            P.encode_control({"role": "node"})
        with self.assertRaises(P.ProtocolError):
            P.decode_control(b'{"role":"node"}')
        with self.assertRaises(P.ProtocolError):
            P.decode_control(b"[]")
        with self.assertRaises(P.ProtocolError):
            P.unpack_control_length(le_u32(P.MAX_CONTROL_LENGTH + 1))
        with self.assertRaises(P.ProtocolError):
            P.split_control(le_u32(10) + b"short")

    def test_constants_match_the_spec(self):
        self.assertEqual((P.IDENTIFY, P.STREAM_HEADER, P.SIGNAL, P.EVENT, P.END_STREAM), (0, 1, 2, 3, 4))
        self.assertEqual(P.DTYPE_CODES, {"f32": 0, "f64": 1, "i16": 2, "i32": 3})
        self.assertEqual(P.PROTOCOL_VERSION, 1)
        self.assertEqual(P.HEADER_SIZE, 16)
        self.assertEqual(P.MAX_FRAME_LENGTH, 64 * 1024 * 1024)
        self.assertEqual(P.MAX_CONTROL_LENGTH, 16 * 1024 * 1024)


if __name__ == "__main__":
    unittest.main()
