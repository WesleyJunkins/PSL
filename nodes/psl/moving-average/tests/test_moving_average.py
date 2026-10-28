import unittest

import numpy as np

import psl
from psl.testing import Harness

from main import MovingAverage


def reference(x: np.ndarray, window: int) -> np.ndarray:
    """Causal zero-padded moving average, column by column."""
    kernel = np.ones(window) / window
    return np.stack([np.convolve(x[:, c], kernel)[: x.shape[0]] for c in range(x.shape[1])], axis=1)


class MovingAverageTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness(MovingAverage, params={"window": 5})
        self.header = psl.SignalHeader(srate=100, channels=[psl.Channel("A", "uV", "eeg")], source={"device": "x"})

    def test_header_is_copied_with_meta(self):
        self.h.header("in", self.header)
        headers = self.h.headers("out")
        self.assertEqual(len(headers), 1)
        out = headers[0]
        self.assertEqual(out.srate, 100.0)
        self.assertEqual(out.channels[0].label, "A")
        self.assertEqual(out.channels[0].unit, "uV")
        self.assertEqual(out.source, {"device": "x"})
        self.assertEqual(out.meta, {"moving_average": {"window": 5}})

    def test_numbers_and_continuity(self):
        self.h.header("in", self.header)
        self.h.signal("in", psl.Chunk(t0=0, data=np.ones((10, 1), np.float32)))
        self.h.signal("in", psl.Chunk(t0=100_000_000, data=np.ones((3, 1), np.float32)))
        out = self.h.outputs("out")
        self.assertEqual(len(out), 2)
        np.testing.assert_allclose(out[0].data[:, 0], [0.2, 0.4, 0.6, 0.8, 1, 1, 1, 1, 1, 1], atol=1e-6)
        np.testing.assert_allclose(out[1].data[:, 0], [1, 1, 1], atol=1e-6)
        self.assertEqual(out[0].t0, 0)
        self.assertEqual(out[1].t0, 100_000_000)
        self.assertEqual([c.seq for c in out], [1, 2])
        self.assertEqual(out[0].data.dtype, np.float32)

    def test_matches_reference_across_arbitrary_chunking(self):
        rng = np.random.default_rng(1)
        x = rng.standard_normal((200, 3)).astype(np.float32)
        hdr = psl.SignalHeader(srate=50, channels=["a", "b", "c"])
        h = Harness(MovingAverage, params={"window": 7})
        h.header("in", hdr)
        pos = 0
        for size in (1, 5, 3, 50, 7, 100, 34):
            h.signal("in", psl.Chunk(t0=pos * 20_000_000, data=x[pos:pos + size]))
            pos += size
        y = np.concatenate([c.data for c in h.outputs("out")])
        np.testing.assert_allclose(y, reference(x.astype(np.float64), 7), atol=1e-5)

    def test_window_one_is_identity(self):
        h = Harness(MovingAverage, params={"window": 1})
        h.header("in", self.header)
        data = np.arange(6, dtype=np.float32).reshape(6, 1)
        h.signal("in", psl.Chunk(t0=0, data=data))
        np.testing.assert_array_equal(h.outputs("out")[0].data, data)

    def test_set_window_resets_state_and_reannounces(self):
        self.h.header("in", self.header)
        self.h.signal("in", psl.Chunk(t0=0, data=np.ones((10, 1), np.float32)))
        self.assertTrue(self.h.set_param("window", 2))
        self.assertEqual(self.h.node.params["window"], 2)
        self.assertEqual(len(self.h.headers("out")), 2)
        self.assertEqual(self.h.headers("out")[1].meta["moving_average"]["window"], 2)
        self.h.signal("in", psl.Chunk(t0=0, data=np.ones((3, 1), np.float32)))
        np.testing.assert_allclose(self.h.outputs("out")[1].data[:, 0], [0.5, 1, 1], atol=1e-6)

    def test_rejects_bad_window(self):
        self.assertFalse(self.h.set_param("window", 0))
        self.assertFalse(self.h.set_param("window", 2.5))
        self.assertFalse(self.h.set_param("window", "5"))
        self.assertFalse(self.h.set_param("other", 5))
        self.assertEqual(self.h.node.params["window"], 5)

    def test_integer_input_becomes_f32(self):
        hdr = psl.SignalHeader(srate=10, channels=["A"], dtype="i16")
        self.h.header("in", hdr)
        self.assertEqual(self.h.headers("out")[0].dtype, "f32")
        self.h.signal("in", psl.Chunk(t0=0, data=np.array([[10], [20], [30]], np.int16)))
        np.testing.assert_allclose(self.h.outputs("out")[0].data[:, 0], [2.0, 6.0, 12.0])

    def test_end_is_propagated(self):
        self.h.header("in", self.header)
        self.h.end("in")
        self.assertEqual(self.h.ended("out"), 1)
        self.h.header("in", self.header)   # reopen
        self.assertEqual(len(self.h.headers("out")), 2)


if __name__ == "__main__":
    unittest.main()
