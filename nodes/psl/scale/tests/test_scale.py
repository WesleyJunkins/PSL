import unittest

import numpy as np

import psl
from psl.testing import Harness

from main import Scale


class ScaleTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness(Scale, params={"gain": 2.0, "offset": 1.0})
        self.header = psl.SignalHeader(srate=250, channels=["A", "B"], dtype="f32", meta={"k": 1})

    def test_header_and_values(self):
        self.h.header("in", self.header)
        out = self.h.headers("out")[0]
        self.assertEqual(out.meta, {"k": 1, "scale": {"gain": 2.0, "offset": 1.0}})
        self.assertEqual(out.dtype, "f32")
        x = np.array([[1.0, -1.0], [0.5, 0.0], [3.0, 2.0]], np.float32)
        self.h.signal("in", psl.Chunk(t0=42, data=x, t_device=7))
        chunk = self.h.outputs("out")[0]
        np.testing.assert_allclose(chunk.data, x * 2 + 1)
        self.assertEqual(chunk.data.dtype, np.float32)
        self.assertEqual(chunk.t0, 42)
        self.assertEqual(chunk.t_device, 7)
        self.assertEqual(chunk.seq, 1)

    def test_timestamps_pass_through(self):
        self.h.header("in", self.header)
        ts = np.array([10, 20, 30], np.int64)
        self.h.signal("in", psl.Chunk(t0=10, data=np.zeros((3, 2), np.float32), timestamps=ts))
        np.testing.assert_array_equal(self.h.outputs("out")[0].timestamps, ts)

    def test_f64_stays_f64(self):
        hdr = psl.SignalHeader(srate=1, channels=["A"], dtype="f64")
        self.h.header("in", hdr)
        self.h.signal("in", psl.Chunk(t0=0, data=np.array([[0.1]], np.float64)))
        chunk = self.h.outputs("out")[0]
        self.assertEqual(chunk.data.dtype, np.float64)
        self.assertEqual(float(chunk.data[0, 0]), 0.1 * 2 + 1)

    def test_integer_input_becomes_f32(self):
        hdr = psl.SignalHeader(srate=1, channels=["A"], dtype="i32")
        self.h.header("in", hdr)
        self.assertEqual(self.h.headers("out")[0].dtype, "f32")
        self.h.signal("in", psl.Chunk(t0=0, data=np.array([[5]], np.int32)))
        self.assertEqual(float(self.h.outputs("out")[0].data[0, 0]), 11.0)

    def test_mutable_params(self):
        self.h.header("in", self.header)
        self.assertTrue(self.h.set_param("gain", 0.5))
        self.assertTrue(self.h.set_param("offset", -1))
        self.assertFalse(self.h.set_param("gain", "x"))
        self.assertFalse(self.h.set_param("gain", True))
        self.assertFalse(self.h.set_param("window", 3))
        self.assertEqual(len(self.h.headers("out")), 3)
        self.assertEqual(self.h.headers("out")[-1].meta["scale"], {"gain": 0.5, "offset": -1.0})
        self.h.signal("in", psl.Chunk(t0=0, data=np.array([[4.0, 8.0]], np.float32)))
        np.testing.assert_allclose(self.h.outputs("out")[0].data, [[1.0, 3.0]])

    def test_chunk_before_header_is_dropped_with_warning(self):
        self.h.signal("in", psl.Chunk(t0=0, data=np.zeros((2, 2), np.float32)))
        self.assertEqual(self.h.outputs("out"), [])
        self.assertEqual(self.h.logs[0][0], "warn")

    def test_end_is_propagated(self):
        self.h.header("in", self.header)
        self.h.end("in")
        self.assertEqual(self.h.ended("out"), 1)
        self.assertEqual([f[1] for f in self.h.frames], [0x01, 0x04])
        self.assertEqual([f[2] for f in self.h.frames], [0, 1])


if __name__ == "__main__":
    unittest.main()
