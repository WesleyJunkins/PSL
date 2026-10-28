import os
import tempfile
import unittest

import numpy as np

import psl
from psl.testing import Harness

from main import CsvWriter


class CsvWriterTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.tmp.name, "nested", "dir", "out.csv")

    def tearDown(self):
        self.tmp.cleanup()

    def read(self):
        with open(self.path, encoding="utf-8") as f:
            return f.read().splitlines()

    def test_writes_header_and_rows(self):
        h = Harness(CsvWriter, params={"path": self.path, "append": False})
        h.header("in", psl.SignalHeader(srate=1000, channels=["ECG", "RESP"]))
        h.signal("in", psl.Chunk(t0=5_000_000, data=np.array([[0.1, 1.0], [0.2, 2.0]], np.float32)))
        h.signal("in", psl.Chunk(t0=7_000_000, data=np.array([[0.3, 3.0]], np.float32)))
        h.stop()
        self.assertEqual(self.read(), [
            "t_ns,ECG,RESP",
            "5000000,0.1,1.0",
            "6000000,0.2,2.0",
            "7000000,0.3,3.0",
        ])
        self.assertEqual(h.node.rows, 3)

    def test_per_sample_timestamps_are_used(self):
        h = Harness(CsvWriter, params={"path": self.path})
        h.header("in", psl.SignalHeader(srate=10, channels=["A"], dtype="i16"))
        h.signal("in", psl.Chunk(t0=0, data=np.array([[1], [2]], np.int16), timestamps=np.array([11, 29])))
        h.stop()
        self.assertEqual(self.read(), ["t_ns,A", "11,1", "29,2"])

    def test_append_skips_header_on_existing_file(self):
        os.makedirs(os.path.dirname(self.path))
        with open(self.path, "w", encoding="utf-8") as f:
            f.write("t_ns,A\n1,0.5\n")
        h = Harness(CsvWriter, params={"path": self.path, "append": True})
        h.header("in", psl.SignalHeader(srate=1000, channels=["A"]))
        h.signal("in", psl.Chunk(t0=2_000_000, data=np.array([[0.25]], np.float32)))
        h.stop()
        self.assertEqual(self.read(), ["t_ns,A", "1,0.5", "2000000,0.25"])

    def test_append_to_missing_file_writes_header(self):
        h = Harness(CsvWriter, params={"path": self.path, "append": True})
        h.header("in", psl.SignalHeader(srate=1000, channels=["A"]))
        h.stop()
        self.assertEqual(self.read(), ["t_ns,A"])

    def test_overwrite_without_append(self):
        os.makedirs(os.path.dirname(self.path))
        with open(self.path, "w", encoding="utf-8") as f:
            f.write("old\n")
        h = Harness(CsvWriter, params={"path": self.path})
        h.header("in", psl.SignalHeader(srate=1000, channels=["A"]))
        h.stop()
        self.assertEqual(self.read(), ["t_ns,A"])

    def test_header_row_repeats_only_when_labels_change(self):
        h = Harness(CsvWriter, params={"path": self.path})
        hdr = psl.SignalHeader(srate=1000, channels=["A"])
        h.header("in", hdr)
        h.header("in", hdr)
        h.header("in", psl.SignalHeader(srate=1000, channels=["A", "B"]))
        h.stop()
        self.assertEqual(self.read(), ["t_ns,A", "t_ns,A,B"])

    def test_path_is_required(self):
        with self.assertRaises(ValueError):
            Harness(CsvWriter, params={})


if __name__ == "__main__":
    unittest.main()
