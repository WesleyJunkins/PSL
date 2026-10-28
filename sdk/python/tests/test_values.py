import unittest

import numpy as np

import psl


class ChunkTests(unittest.TestCase):
    def test_time_of_with_srate(self):
        c = psl.Chunk(t0=1_000, data=np.zeros((4, 2), np.float32), srate=250)
        self.assertEqual([c.time_of(i) for i in range(4)], [1_000, 4_001_000, 8_001_000, 12_001_000])
        self.assertEqual(c.time_of(-1), 12_001_000)
        np.testing.assert_array_equal(c.times(), [1_000, 4_001_000, 8_001_000, 12_001_000])
        self.assertEqual(c.duration_ns, 16_000_000)

    def test_time_of_with_timestamps(self):
        ts = np.array([5, 9, 20])
        c = psl.Chunk(t0=5, data=np.zeros((3, 1)), timestamps=ts, srate=1.0)
        self.assertEqual([c.time_of(i) for i in range(3)], [5, 9, 20])
        self.assertEqual(c.timestamps.dtype, np.int64)
        np.testing.assert_array_equal(c.times(), ts)

    def test_time_of_needs_rate_or_timestamps(self):
        c = psl.Chunk(t0=0, data=np.zeros((2, 1)))
        with self.assertRaises(ValueError):
            c.time_of(0)
        with self.assertRaises(IndexError):
            psl.Chunk(t0=0, data=np.zeros((2, 1)), srate=1).time_of(2)

    def test_shape_normalisation_and_validation(self):
        self.assertEqual(psl.Chunk(t0=0, data=np.arange(3)).data.shape, (3, 1))
        self.assertEqual(psl.Chunk(t0=0, data=[[1, 2], [3, 4]]).data.shape, (2, 2))
        self.assertEqual(psl.Chunk(t0=0, data=5.0).data.shape, (1, 1))
        with self.assertRaises(ValueError):
            psl.Chunk(t0=0, data=np.zeros((2, 2, 2)))
        with self.assertRaises(ValueError):
            psl.Chunk(t0=0, data=np.zeros((2, 1)), timestamps=[1, 2, 3])
        c = psl.Chunk(t0=np.int64(7), data=np.zeros((1, 1)), seq=np.uint32(3))
        self.assertIsInstance(c.t0, int)
        self.assertIsInstance(c.seq, int)
        self.assertEqual((c.n_samples, c.n_channels), (1, 1))

    def test_replace(self):
        c = psl.Chunk(t0=10, data=np.ones((2, 1), np.float32), t_device=4, seq=1, srate=100)
        d = c.replace(data=np.zeros((2, 1)), seq=2)
        self.assertEqual((d.t0, d.t_device, d.seq, d.srate), (10, 4, 2, 100.0))
        self.assertEqual(float(d.data.sum()), 0.0)
        self.assertEqual(float(c.data.sum()), 2.0)   # original untouched
        self.assertIsNot(c, d)


class EventTests(unittest.TestCase):
    def test_fields_and_replace(self):
        e = psl.Event(t=5, label="beat", data={"x": 1})
        self.assertEqual((e.t_device, e.seq), (0, 0))
        f = e.replace(seq=3, data=None)
        self.assertEqual(f, psl.Event(t=5, label="beat", data=None, seq=3))
        self.assertEqual(e.data, {"x": 1})


class HeaderTests(unittest.TestCase):
    def test_signal_header_json_round_trip(self):
        h = psl.SignalHeader(srate=250, channels=[psl.Channel("ECG", "uV", "ecg"), "RESP"],
                             source={"device": "x"}, meta={"a": 1})
        j = h.to_json()
        self.assertEqual(j, {
            "type": "signal", "srate": 250.0, "dtype": "f32",
            "channels": [{"label": "ECG", "unit": "uV", "modality": "ecg"},
                         {"label": "RESP", "unit": "au", "modality": "other"}],
            "source": {"device": "x"}, "meta": {"a": 1},
        })
        back = psl.SignalHeader.from_json(j)
        self.assertEqual(back, h)
        self.assertEqual(psl.SignalHeader.type, "signal")
        self.assertEqual(h.labels, ["ECG", "RESP"])
        self.assertEqual(h.n_channels, 2)

    def test_signal_header_from_minimal_json(self):
        h = psl.SignalHeader.from_json({"srate": 10, "channels": [{"label": "A"}]})
        self.assertEqual((h.dtype, h.source, h.meta, h.channels[0].unit), ("f32", {}, {}, "au"))
        with self.assertRaises(ValueError):
            psl.SignalHeader.from_json({"type": "event"})
        with self.assertRaises(ValueError):
            psl.SignalHeader.from_json({"srate": 10})
        with self.assertRaises(ValueError):
            psl.SignalHeader(srate=0, channels=["A"])
        with self.assertRaises(ValueError):
            psl.SignalHeader(srate=1, channels=["A"], dtype="u8")
        with self.assertRaises(ValueError):
            psl.SignalHeader(srate=1, channels=[{"unit": "uV"}])

    def test_copy_is_deep(self):
        h = psl.SignalHeader(srate=1, channels=["A"], source={"nested": {"k": 1}}, meta={"list": [1]})
        c = h.copy()
        c.meta["list"].append(2)
        c.source["nested"]["k"] = 2
        c.channels[0].label = "B"
        self.assertEqual(h.meta, {"list": [1]})
        self.assertEqual(h.source, {"nested": {"k": 1}})
        self.assertEqual(h.channels[0].label, "A")

    def test_event_header_json_round_trip(self):
        h = psl.EventHeader(labels=["beat", "artifact"], schema={"rr": "ms"}, source={"s": 1}, meta={"m": 2})
        j = h.to_json()
        self.assertEqual(j, {"type": "event", "labels": ["beat", "artifact"], "schema": {"rr": "ms"}, "source": {"s": 1}, "meta": {"m": 2}})
        self.assertEqual(psl.EventHeader.from_json(j), h)
        self.assertEqual(psl.EventHeader.from_json({}), psl.EventHeader())
        self.assertEqual(psl.EventHeader.type, "event")
        c = h.copy()
        c.labels.append("x")
        self.assertEqual(h.labels, ["beat", "artifact"])
        with self.assertRaises(ValueError):
            psl.EventHeader.from_json({"type": "signal"})

    def test_channel(self):
        self.assertEqual(psl.Channel("A").to_json(), {"label": "A", "unit": "au", "modality": "other"})
        self.assertEqual(psl.Channel.from_json("A"), psl.Channel("A"))
        self.assertEqual(psl.Channel.from_json({"label": "A", "unit": "mV"}), psl.Channel("A", "mV"))


class PortTests(unittest.TestCase):
    def test_port_is_frozen(self):
        p = psl.Port("in", 0, "signal", source="ecg.out", source_index=1)
        self.assertEqual((p.name, p.index, p.type, p.source, p.source_index), ("in", 0, "signal", "ecg.out", 1))
        with self.assertRaises(Exception):
            p.name = "x"


class PackageTests(unittest.TestCase):
    def test_exports(self):
        self.assertEqual(psl.__version__, "0.1.0")
        for name in ("Node", "run", "Channel", "SignalHeader", "EventHeader", "Chunk", "Event", "Port"):
            self.assertTrue(hasattr(psl, name), name)


if __name__ == "__main__":
    unittest.main()
