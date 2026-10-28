import unittest

import numpy as np

import psl
from psl.testing import Harness

from main import Synthetic, ecg_shape

PARAMS = {
    "waveform": "ecg", "srate": 250, "channels": ["ECG"], "chunk_ms": 20,
    "amplitude": 1.0, "frequency": 1.0, "bpm": 60, "seed": 0,
}


def make(**overrides):
    params = dict(PARAMS)
    params.update(overrides)
    return Harness(Synthetic, params=params)


class SyntheticTests(unittest.TestCase):
    def test_header_for_ecg(self):
        h = make()
        h.start()
        headers = h.headers("out")
        self.assertEqual(len(headers), 1)
        hdr = headers[0]
        self.assertIsInstance(hdr, psl.SignalHeader)
        self.assertEqual(hdr.srate, 250.0)
        self.assertEqual(hdr.dtype, "f32")
        self.assertEqual([c.label for c in hdr.channels], ["ECG"])
        self.assertEqual(hdr.channels[0].unit, "mV")
        self.assertEqual(hdr.channels[0].modality, "ecg")
        self.assertEqual(hdr.source["device"], "psl/synthetic")
        self.assertEqual(hdr.source["waveform"], "ecg")

    def test_header_for_sine_and_noise(self):
        for waveform in ("sine", "noise"):
            h = make(waveform=waveform, channels=["A", "B"])
            h.start()
            hdr = h.headers("out")[0]
            self.assertEqual([c.unit for c in hdr.channels], ["au", "au"])
            self.assertEqual([c.modality for c in hdr.channels], ["other", "other"])

    def test_chunks_have_right_shape_timing_and_seq(self):
        h = make(channels=["A", "B", "C"])
        h.start()
        for i in range(5):
            h.advance(0.020)
            h.tick()
        out = h.outputs("out")
        self.assertEqual(len(out), 5)
        for i, chunk in enumerate(out):
            self.assertIsInstance(chunk, psl.Chunk)
            self.assertEqual(chunk.data.shape, (5, 3))
            self.assertEqual(chunk.data.dtype, np.float32)
            self.assertEqual(chunk.seq, i + 1)   # seq 0 was the header
            self.assertEqual(chunk.t0, (i + 1) * 20_000_000 - 20_000_000)
            self.assertTrue(np.array_equal(chunk.data[:, 0], chunk.data[:, 1]))

    def test_fractional_samples_per_tick_are_carried_over(self):
        h = make(srate=100, chunk_ms=15)
        h.start()
        counts = []
        for _ in range(4):
            h.advance(0.015)
            h.tick()
            counts.append(h.outputs("out")[-1].n_samples)
        self.assertEqual(counts, [1, 2, 1, 2])   # 1.5 samples per tick on average
        self.assertEqual(sum(c.n_samples for c in h.outputs("out")), 6)

    def test_chunk_size_does_not_depend_on_timer_jitter(self):
        h = make()
        h.start()
        h.advance(0.031)   # a late first tick
        h.tick()
        h.advance(0.009)   # and an early second one
        h.tick()
        self.assertEqual([c.n_samples for c in h.outputs("out")], [5, 5])
        self.assertEqual([c.t0 for c in h.outputs("out")], [31_000_000 - 20_000_000, 40_000_000 - 20_000_000])

    def test_ecg_is_continuous_and_periodic(self):
        h = make(bpm=60, srate=250, chunk_ms=20)
        h.start()
        for _ in range(100):   # 2 s = 2 beats
            h.advance(0.020)
            h.tick()
        y = np.concatenate([c.data[:, 0] for c in h.outputs("out")])
        self.assertEqual(len(y), 500)
        phase = np.arange(500) / 250.0   # beats at 60 bpm
        expected = ecg_shape(phase).astype(np.float32)
        np.testing.assert_allclose(y, expected, rtol=1e-5, atol=1e-6)
        self.assertAlmostEqual(float(y.max()), 1.0, places=2)         # R peak
        self.assertTrue(np.allclose(y[:250], y[250:], atol=1e-5))     # beat 1 == beat 2
        self.assertEqual(int(np.argmax(y[:250])), 100)               # R at 0.40 of the beat

    def test_sine_is_continuous_across_chunks(self):
        h = make(waveform="sine", frequency=5.0, srate=200, chunk_ms=10, amplitude=2.0)
        h.start()
        for _ in range(40):
            h.advance(0.010)
            h.tick()
        y = np.concatenate([c.data[:, 0] for c in h.outputs("out")])
        t = np.arange(len(y)) / 200.0
        np.testing.assert_allclose(y, 2.0 * np.sin(2 * np.pi * 5.0 * t), atol=1e-5)

    def test_noise_is_seeded(self):
        a = make(waveform="noise", seed=7, channels=["X", "Y"])
        b = make(waveform="noise", seed=7, channels=["X", "Y"])
        c = make(waveform="noise", seed=8, channels=["X", "Y"])
        for h in (a, b, c):
            h.start()
            h.advance(0.020)
            h.tick()
        self.assertTrue(np.array_equal(a.outputs("out")[0].data, b.outputs("out")[0].data))
        self.assertFalse(np.array_equal(a.outputs("out")[0].data, c.outputs("out")[0].data))
        self.assertFalse(np.array_equal(a.outputs("out")[0].data[:, 0], a.outputs("out")[0].data[:, 1]))

    def test_mutable_params(self):
        h = make(waveform="sine", frequency=1.0, srate=100, chunk_ms=10)
        h.start()
        self.assertTrue(h.set_param("amplitude", 2.0))
        self.assertEqual(h.node.params["amplitude"], 2.0)
        self.assertTrue(h.set_param("bpm", 90))
        self.assertFalse(h.set_param("bpm", 0))
        self.assertFalse(h.set_param("bpm", "fast"))
        self.assertFalse(h.set_param("amplitude", "loud"))
        self.assertFalse(h.set_param("frequency", 2.0))   # not mutable
        self.assertFalse(h.set_param("nonsense", 1))
        h.advance(0.25)
        h.tick()
        y = h.outputs("out")[0].data[:, 0]
        t = np.arange(len(y)) / 100.0
        np.testing.assert_allclose(y, 2.0 * np.sin(2 * np.pi * t), atol=1e-5)

    def test_rejects_bad_params(self):
        with self.assertRaises(ValueError):
            make(waveform="square")
        with self.assertRaises(ValueError):
            make(channels=[])

    def test_emit_before_start_raises(self):
        h = Harness(Synthetic, params=PARAMS, outputs=[("out", "signal")])
        with self.assertRaises(RuntimeError):
            h.node.emit("out", psl.Chunk(t0=0, data=np.zeros((1, 1), np.float32)))


if __name__ == "__main__":
    unittest.main()
