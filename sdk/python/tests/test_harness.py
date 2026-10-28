"""Drive first-party nodes and the Node base class through psl.testing.Harness."""

import importlib.util
import json
import os
import struct
import tempfile
import unittest
from pathlib import Path

import numpy as np

import psl
from psl import _protocol as P
from psl.testing import Harness, NodeFailed

REPO = Path(__file__).resolve().parents[3]
NODES = REPO / "nodes" / "psl"


def load_node_module(name):
    path = NODES / name / "main.py"
    spec = importlib.util.spec_from_file_location("node_" + name.replace("-", "_"), path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class MovingAverageThroughHarness(unittest.TestCase):
    def setUp(self):
        self.MovingAverage = load_node_module("moving-average").MovingAverage

    def test_docs_example(self):
        h = Harness(self.MovingAverage, params={"window": 5})
        h.header("in", psl.SignalHeader(srate=100, channels=[psl.Channel("A")]))
        h.signal("in", psl.Chunk(t0=0, data=np.ones((10, 1), np.float32)))
        out = h.outputs("out")
        self.assertEqual(len(out), 1)
        np.testing.assert_allclose(out[0].data[:, 0], [0.2, 0.4, 0.6, 0.8, 1, 1, 1, 1, 1, 1], atol=1e-6)
        self.assertEqual(h.headers("out")[0].meta["moving_average"], {"window": 5})
        self.assertEqual(out[0].srate, 100.0)           # srate attached from the declared header
        self.assertEqual(out[0].time_of(1), 10_000_000)

    def test_state_is_continuous_and_ports_carry_source(self):
        seen = []
        MA = self.MovingAverage

        class Spy(MA):
            def on_signal(self, port, chunk):
                seen.append((port.name, port.index, port.type, port.source, port.source_index, chunk.srate))
                super().on_signal(port, chunk)

        h = Harness(Spy, params={"window": 3})
        h.header("in", psl.SignalHeader(srate=10, channels=["A", "B"]))
        x = np.arange(12, dtype=np.float32).reshape(6, 2)
        h.signal("in", psl.Chunk(t0=0, data=x[:4]))
        h.signal("in", psl.Chunk(t0=400_000_000, data=x[4:]))
        y = np.concatenate([c.data for c in h.outputs("out")])
        kernel = np.ones(3) / 3
        expected = np.stack([np.convolve(x[:, c], kernel)[:6] for c in range(2)], axis=1)
        np.testing.assert_allclose(y, expected, atol=1e-5)
        self.assertEqual(seen[0], ("in", 0, "signal", "test.in", 0, 10.0))
        self.assertEqual([c.seq for c in h.outputs("out")], [1, 2])
        self.assertEqual(h.frames, [("out", P.STREAM_HEADER, 0), ("out", P.SIGNAL, 1), ("out", P.SIGNAL, 2)])

    def test_replay_from_recording(self):
        hdr = psl.SignalHeader(srate=50, channels=["A"])
        x = np.arange(20, dtype=np.float32).reshape(20, 1)
        frames = [
            (0, P.stream_header_frame(0, 0, hdr)),
            (1, P.stream_header_frame(0, 0, psl.EventHeader(labels=["m"]))),
            (0, P.signal_frame(0, 1, psl.Chunk(t0=0, data=x[:12]), "f32")),
            (1, P.event_frame(0, 1, psl.Event(t=5, label="m"))),
            (0, P.signal_frame(0, 2, psl.Chunk(t0=240_000_000, data=x[12:]), "f32")),
            (0, P.end_stream_frame(0, 3)),
        ]
        header = {
            "version": 1, "session": "s", "wall_start_ns": 0, "pipeline": "p.psl", "graph": {},
            "streams": [{"index": 0, "stream": "ecg.out", "node": "ecg", "port": "out", "type": "signal"},
                        {"index": 1, "stream": "mk.out", "node": "mk", "port": "out", "type": "event"}],
        }
        body = json.dumps(header).encode()
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "rec.pslrec")
            with open(path, "wb") as f:
                f.write(b"PSLREC\x00\x01" + struct.pack("<I", len(body)) + body)
                for t, (stream, frame) in enumerate(frames):
                    f.write(struct.pack("<qHH", t * 1000, stream, 0) + frame)
            h = Harness(self.MovingAverage, params={"window": 4})
            delivered = h.replay(path, map={"ecg.out": "in"})
        self.assertEqual(delivered, 4)   # the event stream was not mapped
        y = np.concatenate([c.data for c in h.outputs("out")])
        np.testing.assert_allclose(y[:, 0], np.convolve(x[:, 0], np.ones(4) / 4)[:20], atol=1e-5)
        self.assertEqual(h.ended("out"), 1)
        self.assertEqual(h.node.inputs[0]["name"], "in")


class SpyNode(psl.Node):
    """A node that records what it is told, for testing the base class."""

    def on_init(self, params):
        self.calls = [("init", dict(params))]
        self.accept = params.get("accept", True)

    def on_start(self):
        self.calls.append(("start",))

    def on_header(self, port, header):
        self.calls.append(("header", port, header))

    def on_signal(self, port, chunk):
        self.calls.append(("signal", port, chunk))

    def on_event(self, port, event):
        self.calls.append(("event", port, event))

    def on_end(self, port):
        self.calls.append(("end", port))

    def on_param(self, name, value):
        self.calls.append(("param", name, value))
        return bool(self.accept)

    def on_stop(self):
        self.calls.append(("stop",))


class NodeBaseClassTests(unittest.TestCase):
    def test_params_and_lifecycle_callbacks(self):
        h = Harness(SpyNode, params={"a": 1, "accept": True})
        self.assertEqual(h.node.calls[0], ("init", {"a": 1, "accept": True}))
        self.assertEqual(h.node.params, {"a": 1, "accept": True})
        h.start()
        h.start()   # idempotent
        self.assertEqual(h.node.calls[1], ("start",))
        self.assertTrue(h.set_param("a", 2))
        self.assertEqual(h.node.params["a"], 2)
        h.node.accept = False
        self.assertFalse(h.set_param("a", 3))
        self.assertEqual(h.node.params["a"], 2)
        h.stop()
        self.assertEqual(h.node.calls[-1], ("stop",))

    def test_input_callbacks_and_port_objects(self):
        h = Harness(SpyNode, params={})
        hdr = psl.SignalHeader(srate=100, channels=["A"])
        h.header("in", hdr)
        h.signal("in", psl.Chunk(t0=7, data=np.ones((2, 1), np.float32), seq=3))
        h.header("ev", psl.EventHeader(labels=["x"]))
        h.event("ev", psl.Event(t=9, label="x", data=[1]))
        h.end("in")
        kinds = [c[0] for c in h.node.calls]
        self.assertEqual(kinds, ["init", "start", "header", "signal", "header", "event", "end"])
        _, port, header = h.node.calls[2]
        self.assertEqual(port, psl.Port("in", 0, "signal", "test.in", 0))
        self.assertEqual(header, hdr)
        _, port, chunk = h.node.calls[3]
        self.assertEqual((chunk.t0, chunk.seq, chunk.srate), (7, 3, 100.0))
        self.assertTrue(chunk.data.flags.writeable)
        _, port, event = h.node.calls[5]
        self.assertEqual(port, psl.Port("ev", 1, "event", "test.ev", 0))
        self.assertEqual(event, psl.Event(t=9, label="x", data=[1]))
        with self.assertRaises(ValueError):
            h.signal("ev", psl.Chunk(t0=0, data=np.zeros((1, 1))))   # wrong port type
        stats = h.node._stats(now=1_000)
        self.assertEqual(stats["received"], {"in": 3, "ev": 2})
        self.assertEqual(stats["latency_ns"], {"in": 993, "ev": 991})
        self.assertEqual(stats["dropped_local"], 0)

    def test_output_rules(self):
        h = Harness(SpyNode, params={}, outputs=[("out", "signal"), ("ev", "event")])
        node = h.node
        chunk = psl.Chunk(t0=0, data=np.ones((2, 1), np.float32))
        with self.assertRaisesRegex(RuntimeError, "before start"):
            node.emit("out", chunk)
        node.declare_output("out", psl.SignalHeader(srate=10, channels=["A"]))   # kept until start
        self.assertEqual(h.headers("out"), [])
        h.start()
        self.assertEqual(len(h.headers("out")), 1)                                 # flushed on start
        node.emit("out", chunk)
        with self.assertRaisesRegex(RuntimeError, "no stream header"):
            node.emit("ev", psl.Event(t=0, label="x"))
        with self.assertRaisesRegex(ValueError, "unknown output"):
            node.emit("nope", chunk)
        with self.assertRaisesRegex(ValueError, "signal port"):
            node.emit("out", psl.Event(t=0, label="x"))
        with self.assertRaisesRegex(ValueError, "channels"):
            node.emit("out", psl.Chunk(t0=0, data=np.ones((2, 3))))
        with self.assertRaisesRegex(ValueError, "event port"):
            node.declare_output("ev", psl.SignalHeader(srate=1, channels=["A"]))
        with self.assertRaises(TypeError):
            node.emit("out", "not a chunk")
        node.emit("out", psl.Chunk(t0=0, data=np.ones((2, 1), np.float64) * 0.1))   # converted to f32
        node.end("out")
        with self.assertRaisesRegex(RuntimeError, "no open stream"):
            node.end("out")
        with self.assertRaisesRegex(RuntimeError, "no stream header"):
            node.emit("out", chunk)
        node.declare_output("out", psl.SignalHeader(srate=10, channels=["A"]))     # reopen
        node.emit("out", chunk)
        self.assertEqual([f[1:] for f in h.frames], [
            (P.STREAM_HEADER, 0), (P.SIGNAL, 1), (P.SIGNAL, 2), (P.END_STREAM, 3), (P.STREAM_HEADER, 4), (P.SIGNAL, 5),
        ])
        out = h.outputs("out")
        self.assertEqual(out[1].data.dtype, np.float32)
        self.assertEqual([c.seq for c in out], [1, 2, 5])
        self.assertEqual(h.node._stats(0)["sent"], {"out": 6, "ev": 0})

    def test_timers_logs_and_fail(self):
        h = Harness(SpyNode, params={})
        ticks = []
        t1 = h.node.every(0.5, lambda: ticks.append("every"))
        t2 = h.node.after(1.0, lambda: ticks.append("after"))
        h.tick()
        h.tick()
        self.assertEqual(ticks, ["every", "every"])
        h.run_after()
        h.run_after()
        self.assertEqual(ticks, ["every", "every", "after"])
        t1.cancel()
        h.tick()
        self.assertEqual(ticks, ["every", "every", "after"])
        self.assertTrue(t1.cancelled and not t2.cancelled)
        with self.assertRaises(ValueError):
            h.node.every(-1, lambda: None)
        h.advance(1.5)
        self.assertEqual(h.node.now(), 1_500_000_000)
        h.node.log.info("hello", a=1)
        h.node.log.warn("careful")
        self.assertEqual(h.logs, [("info", "hello"), ("warn", "careful")])
        self.assertEqual(h.log_records[0][2], {"a": 1})
        with self.assertRaises(NodeFailed) as ctx:
            h.node.fail("boom")
        self.assertEqual(ctx.exception.reason, "boom")
        self.assertEqual(h.failed, "boom")

    def test_spawn_runs_on_a_thread(self):
        import threading
        h = Harness(SpyNode, params={})
        done = threading.Event()
        t = h.node.spawn(done.set)
        self.assertTrue(done.wait(2.0))
        t.join(2.0)
        self.assertTrue(t.daemon)


if __name__ == "__main__":
    unittest.main()
