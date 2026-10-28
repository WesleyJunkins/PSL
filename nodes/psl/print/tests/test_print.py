import unittest

import numpy as np

import psl
from psl.testing import Harness

from main import PrintNode


class PrintTests(unittest.TestCase):
    def test_header_and_every_nth_chunk(self):
        h = Harness(PrintNode, params={"every": 3})
        h.header("in", psl.SignalHeader(srate=100, channels=[psl.Channel("ECG", "mV", "ecg")]))
        self.assertEqual(len(h.logs), 1)
        self.assertEqual(h.logs[0][0], "info")
        self.assertIn("in header: signal srate=100 dtype=f32 channels=[ECG (mV, ecg)]", h.logs[0][1])
        for i in range(7):
            h.signal("in", psl.Chunk(t0=i * 1000, data=np.full((4, 1), i, np.float32), seq=i))
        lines = [msg for level, msg in h.logs[1:]]
        self.assertEqual(len(lines), 3)   # chunks 0, 3, 6
        self.assertEqual(lines[0], "in seq=0 t0=0 shape=(4, 1) first=[0]")
        self.assertEqual(lines[1], "in seq=3 t0=3000 shape=(4, 1) first=[3]")
        self.assertEqual(lines[2], "in seq=6 t0=6000 shape=(4, 1) first=[6]")
        self.assertEqual(h.log_records[1][2]["source"], "test.in")

    def test_events_are_all_logged(self):
        h = Harness(PrintNode, params={"every": 25})
        h.header("events", psl.EventHeader(labels=["beat"]))
        self.assertIn("events header: event labels=['beat']", h.logs[0][1])
        h.event("events", psl.Event(t=5, label="beat", data={"rr_ms": 812}, seq=0))
        h.event("events", psl.Event(t=9, label="beat", data=None, seq=1))
        self.assertEqual(h.logs[1][1], 'events seq=0 t=5 label=beat data={"rr_ms": 812}')
        self.assertEqual(h.logs[2][1], "events seq=1 t=9 label=beat data=null")

    def test_counts_are_per_source(self):
        h = Harness(PrintNode, params={"every": 2})
        hdr = psl.SignalHeader(srate=10, channels=["A"])
        h.header("in", hdr, source=0)
        h.header("in", hdr, source=1)
        for i in range(2):
            h.signal("in", psl.Chunk(t0=0, data=np.zeros((1, 1), np.float32)), source=0)
            h.signal("in", psl.Chunk(t0=0, data=np.zeros((1, 1), np.float32)), source=1)
        chunk_lines = [r for r in h.log_records if r[1].startswith("in seq=")]
        self.assertEqual([r[2]["source"] for r in chunk_lines], ["test.in", "test.in.1"])

    def test_first_values_are_truncated(self):
        h = Harness(PrintNode, params={"every": 1})
        h.header("in", psl.SignalHeader(srate=10, channels=[str(i) for i in range(10)]))
        h.signal("in", psl.Chunk(t0=0, data=np.arange(10, dtype=np.float32).reshape(1, 10)))
        self.assertTrue(h.logs[-1][1].endswith("first=[0, 1, 2, 3, 4, 5, 6, 7, ...]"))

    def test_end_and_bad_param(self):
        h = Harness(PrintNode, params={"every": 1})
        h.header("in", psl.SignalHeader(srate=10, channels=["A"]))
        h.end("in")
        self.assertEqual(h.logs[-1], ("info", "in end of stream"))
        with self.assertRaises(ValueError):
            Harness(PrintNode, params={"every": 0})


if __name__ == "__main__":
    unittest.main()
