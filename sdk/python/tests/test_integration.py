"""End-to-end: a fake supervisor written from docs/03 alone drives real node processes.

The supervisor side deliberately uses only struct, json and sockets (no SDK
helpers), so these tests check the node processes against the contract rather
than against the SDK's own codec.
"""

import hashlib
import json
import os
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

import numpy as np

SDK_DIR = Path(__file__).resolve().parents[1]
REPO = SDK_DIR.parents[1]
NODES = REPO / "nodes" / "psl"
PYTHON = sys.executable

IDENTIFY, STREAM_HEADER, SIGNAL, EVENT, END_STREAM = 0, 1, 2, 3, 4
FRAME = struct.Struct("<IBBHHHI")
SIGNAL_HEAD = struct.Struct("<qqIHBB")
NP_DTYPES = {0: "<f4", 1: "<f8", 2: "<i2", 3: "<i4"}


def read_exact(sock, n):
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise EOFError(f"socket closed after {len(buf)} of {n} bytes")
        buf += chunk
    return bytes(buf)


def node_env(session, node_id, control, data, protocol="1"):
    env = dict(os.environ)
    env["PSL_SESSION"] = session
    env["PSL_NODE_ID"] = node_id
    env["PSL_CONTROL"] = control
    env["PSL_DATA"] = data
    env["PSL_PROTOCOL"] = protocol
    env["PYTHONPATH"] = str(SDK_DIR) + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    env["PYTHONUNBUFFERED"] = "1"
    return env


class FakeSupervisor:
    """Listens on two Unix sockets, spawns one node and speaks docs/03 to it."""

    def __init__(self, node_dir, node_id, session="20261028-000000-test"):
        self.node_dir = Path(node_dir)
        self.node_id = node_id
        self.session = session
        self.tmp = tempfile.mkdtemp(prefix="psl-it-")
        self.control_path = os.path.join(self.tmp, "control.sock")
        self.data_path = os.path.join(self.tmp, "data.sock")
        self.log_path = os.path.join(self.tmp, f"{node_id}.log")
        self._listeners = []
        for path in (self.control_path, self.data_path):
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            s.bind(path)
            s.listen(1)
            s.settimeout(10.0)
            self._listeners.append(s)
        self.t_start = time.monotonic_ns()
        self.proc = None
        self.control = None
        self.data = None
        self.log_file = None
        self.messages = []        # every control message except clock_req, in order
        self.clock_reqs = 0

    # -- process -------------------------------------------------------------

    def session_ns(self):
        return time.monotonic_ns() - self.t_start

    def spawn(self, env=None):
        self.log_file = open(self.log_path, "wb")
        self.proc = subprocess.Popen(
            [PYTHON, "main.py"],
            cwd=str(self.node_dir),
            env=env or node_env(self.session, self.node_id, self.control_path, self.data_path),
            stdin=subprocess.DEVNULL,
            stdout=self.log_file,
            stderr=subprocess.STDOUT,
        )
        return self.proc

    def wait_exit(self, timeout):
        return self.proc.wait(timeout=timeout)

    def log_text(self):
        if self.log_file:
            self.log_file.flush()
        with open(self.log_path, "rb") as f:
            return f.read().decode("utf-8", "replace")

    def close(self):
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait(5)
        for s in (self.control, self.data, *self._listeners):
            if s is not None:
                try:
                    s.close()
                except OSError:
                    pass
        if self.log_file:
            self.log_file.close()
        shutil.rmtree(self.tmp, ignore_errors=True)

    # -- control plane -------------------------------------------------------

    def accept_control(self):
        self.control, _ = self._listeners[0].accept()
        self.control.settimeout(10.0)

    def accept_data(self):
        self.data, _ = self._listeners[1].accept()
        self.data.settimeout(10.0)

    def send(self, msg):
        body = json.dumps(msg).encode("utf-8")
        self.control.sendall(struct.pack("<I", len(body)) + body)

    def _recv_raw(self):
        (n,) = struct.unpack("<I", read_exact(self.control, 4))
        msg = json.loads(read_exact(self.control, n).decode("utf-8"))
        assert isinstance(msg, dict) and isinstance(msg.get("type"), str), msg
        return msg

    def recv(self, timeout=5.0):
        """Next control message, answering clock_req on the way."""
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"no control message within {timeout} s; log:\n{self.log_text()}")
            self.control.settimeout(remaining)
            try:
                msg = self._recv_raw()
            except socket.timeout:
                raise TimeoutError(f"no control message within {timeout} s; log:\n{self.log_text()}")
            if msg["type"] == "clock_req":
                t2 = self.session_ns()
                self.send({"type": "clock_res", "t1": msg["t1"], "t2": t2, "t3": self.session_ns()})
                self.clock_reqs += 1
                continue
            self.messages.append(msg)
            return msg

    def wait_for(self, predicate, timeout=5.0, what="message"):
        deadline = time.monotonic() + timeout
        while True:
            msg = self.recv(max(0.01, deadline - time.monotonic()))
            if predicate(msg):
                return msg
            if time.monotonic() > deadline:
                raise TimeoutError(f"no {what} within {timeout} s; got {self.messages}\nlog:\n{self.log_text()}")

    def wait_state(self, state, timeout=5.0):
        return self.wait_for(lambda m: m["type"] == "state" and m["state"] == state, timeout, f"state {state}")

    def handshake(self, params, inputs, outputs, options=None):
        self.accept_control()
        hello = self._recv_raw()
        assert hello["type"] == "hello", hello
        self.send({
            "type": "config",
            "session": self.session,
            "node_id": self.node_id,
            "wall_start_ns": 1793196612000000000,
            "params": params,
            "inputs": inputs,
            "outputs": outputs,
            "options": options or {"queue": 64, "on_full": "drop_oldest"},
        })
        self.accept_data()
        hdr, payload = self.read_frame()
        assert hdr["type"] == IDENTIFY, hdr
        assert json.loads(payload) == {"node_id": self.node_id, "session": self.session}, payload
        assert (hdr["port"], hdr["source"], hdr["seq"]) == (0, 0, 0), hdr
        self.wait_for(lambda m: m["type"] == "ready", 10.0, "ready")
        return hello

    # -- data plane ----------------------------------------------------------

    def read_frame(self, timeout=5.0):
        self.data.settimeout(timeout)
        try:
            raw = read_exact(self.data, 16)
            length, version, ftype, flags, port, source, seq = FRAME.unpack(raw)
            assert 16 <= length <= 64 * 1024 * 1024, length
            payload = read_exact(self.data, length - 16)
        except socket.timeout:
            raise TimeoutError(f"no data frame within {timeout} s; log:\n{self.log_text()}")
        return {"length": length, "version": version, "type": ftype, "flags": flags,
                "port": port, "source": source, "seq": seq}, payload

    def send_frame(self, ftype, payload=b"", port=0, source=0, seq=0, flags=0):
        self.data.sendall(FRAME.pack(16 + len(payload), 1, ftype, flags, port, source, seq) + payload)

    @staticmethod
    def parse_signal(payload, flags):
        t0, t_device, n, c, code, reserved = SIGNAL_HEAD.unpack_from(payload, 0)
        assert reserved == 0
        dt = np.dtype(NP_DTYPES[code])
        end = 24 + n * c * dt.itemsize
        data = np.frombuffer(payload[24:end], dtype=dt).reshape(n, c)
        ts = None
        if flags & 1:
            ts = np.frombuffer(payload[end:end + 8 * n], dtype="<i8")
            end += 8 * n
        assert end == len(payload), (end, len(payload))
        return t0, t_device, data, ts

    @staticmethod
    def signal_payload(t0, data, dtype_code=0, t_device=0):
        arr = np.ascontiguousarray(data, dtype=NP_DTYPES[dtype_code])
        return SIGNAL_HEAD.pack(t0, t_device, arr.shape[0], arr.shape[1], dtype_code, 0) + arr.tobytes()


class SyntheticNodeProcess(unittest.TestCase):
    def test_full_lifecycle(self):
        sup = FakeSupervisor(NODES / "synthetic", "ecg")
        self.addCleanup(sup.close)
        proc = sup.spawn()
        hello = sup.handshake(
            params={"waveform": "ecg", "srate": 250, "channels": ["ECG"], "chunk_ms": 20,
                    "amplitude": 1.0, "frequency": 1.0, "bpm": 60, "seed": 0},
            inputs=[],
            outputs=[{"name": "out", "index": 0, "type": "signal", "stream": "ecg.out"}],
        )
        # hello
        self.assertEqual(hello["role"], "node")
        self.assertEqual(hello["node_id"], "ecg")
        self.assertEqual(hello["protocol"], 1)
        self.assertEqual(hello["pid"], proc.pid)
        self.assertTrue(hello["sdk"].startswith("psl-python/"))
        self.assertEqual(hello["manifest_hash"], hashlib.sha256((NODES / "synthetic" / "node.psl").read_bytes()).hexdigest())
        self.assertNotIn("outputs", hello)
        # clock sync and states before start: state{configured}, ready, state{ready}
        self.assertEqual(sup.clock_reqs, 8)
        sup.wait_state("ready")
        kinds = [(m["type"], m.get("state")) for m in sup.messages if m["type"] in ("state", "ready")]
        self.assertEqual(kinds, [("state", "configured"), ("ready", None), ("state", "ready")])

        # start -> header, then chunks
        t_sent = sup.session_ns()
        sup.send({"type": "start"})
        sup.wait_state("running")
        hdr, payload = sup.read_frame()
        self.assertEqual((hdr["version"], hdr["type"], hdr["port"], hdr["seq"], hdr["flags"]), (1, STREAM_HEADER, 0, 0, 0))
        header = json.loads(payload)
        self.assertEqual(header["type"], "signal")
        self.assertEqual(header["srate"], 250.0)
        self.assertEqual(header["dtype"], "f32")
        self.assertEqual(header["channels"], [{"label": "ECG", "unit": "mV", "modality": "ecg"}])
        self.assertEqual(header["source"]["device"], "psl/synthetic")

        last_t0 = None
        for i in range(1, 6):
            hdr, payload = sup.read_frame()
            self.assertEqual((hdr["version"], hdr["type"], hdr["port"], hdr["source"]), (1, SIGNAL, 0, 0))
            self.assertEqual(hdr["seq"], i)
            self.assertEqual(hdr["flags"], 0)
            self.assertEqual(hdr["length"], 16 + 24 + 5 * 4)
            t0, t_device, data, ts = sup.parse_signal(payload, hdr["flags"])
            self.assertEqual(data.shape, (5, 1))
            self.assertEqual(data.dtype, np.dtype("<f4"))
            self.assertEqual(t_device, 0)
            self.assertIsNone(ts)
            self.assertGreaterEqual(t0, t_sent - 100_000_000, "t0 before start")
            self.assertLessEqual(t0, sup.session_ns(), "t0 in the future")
            if last_t0 is not None:
                self.assertGreater(t0, last_t0)
            last_t0 = t0

        # ping / pong within 3 s while data is flowing
        t = time.monotonic()
        sup.send({"type": "ping", "n": 7})
        pong = sup.wait_for(lambda m: m["type"] == "pong", 3.0, "pong")
        self.assertEqual(pong["n"], 7)
        self.assertLess(time.monotonic() - t, 3.0)

        # set_param
        sup.send({"type": "set_param", "name": "amplitude", "value": 2.0})
        ack = sup.wait_for(lambda m: m["type"] == "param_ack", 3.0, "param_ack")
        self.assertEqual((ack["name"], ack["ok"]), ("amplitude", True))
        sup.send({"type": "set_param", "name": "frequency", "value": 3.0})
        ack = sup.wait_for(lambda m: m["type"] == "param_ack", 3.0, "param_ack")
        self.assertEqual((ack["name"], ack["ok"]), ("frequency", False))
        self.assertIn("reason", ack)

        # the new amplitude shows up in the data (an R peak of ~2 mV within 1.2 s)
        peak, deadline = 0.0, time.monotonic() + 3.0
        while peak < 1.9 and time.monotonic() < deadline:
            hdr, payload = sup.read_frame()
            if hdr["type"] == SIGNAL:
                peak = max(peak, float(sup.parse_signal(payload, hdr["flags"])[2].max()))
        self.assertGreaterEqual(peak, 1.9)

        # stats every second
        stats = sup.wait_for(lambda m: m["type"] == "stats", 3.0, "stats")
        self.assertGreater(stats["sent"]["out"], 0)
        self.assertEqual(stats["received"], {})
        self.assertEqual(stats["dropped_local"], 0)
        self.assertEqual(stats["latency_ns"], {})

        # stop
        t = time.monotonic()
        sup.send({"type": "stop", "grace_ms": 2000})
        sup.wait_state("stopping", 2.0)
        sup.wait_state("stopped", 2.0)
        self.assertEqual(sup.wait_exit(2.0), 0)
        self.assertLess(time.monotonic() - t, 2.0)
        self.assertNotIn("Traceback", sup.log_text())

    def test_sigterm_stops_cleanly(self):
        sup = FakeSupervisor(NODES / "synthetic", "ecg")
        self.addCleanup(sup.close)
        proc = sup.spawn()
        sup.handshake(params={"waveform": "sine", "srate": 100, "channels": ["A"], "chunk_ms": 50,
                              "amplitude": 1.0, "frequency": 1.0, "bpm": 60, "seed": 0},
                      inputs=[], outputs=[{"name": "out", "index": 0, "type": "signal", "stream": "ecg.out"}])
        sup.send({"type": "start"})
        sup.wait_state("running")
        sup.read_frame()
        proc.send_signal(signal.SIGTERM)
        sup.wait_state("stopping", 3.0)
        sup.wait_state("stopped", 3.0)
        self.assertEqual(sup.wait_exit(3.0), 0)

    def test_reject_exits_3(self):
        sup = FakeSupervisor(NODES / "synthetic", "ecg")
        self.addCleanup(sup.close)
        sup.spawn()
        sup.accept_control()
        hello = sup._recv_raw()
        self.assertEqual(hello["type"], "hello")
        sup.send({"type": "reject", "reason": "manifest hash mismatch"})
        self.assertEqual(sup.wait_exit(5.0), 3)
        self.assertIn("manifest hash mismatch", sup.log_text())

    def test_missing_env_exits_2(self):
        env = dict(os.environ)
        for name in ("PSL_SESSION", "PSL_NODE_ID", "PSL_CONTROL", "PSL_DATA", "PSL_PROTOCOL"):
            env.pop(name, None)
        env["PYTHONPATH"] = str(SDK_DIR)
        env["PYTHONDONTWRITEBYTECODE"] = "1"
        result = subprocess.run([PYTHON, "main.py"], cwd=str(NODES / "synthetic"), env=env,
                                capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 2)
        self.assertIn("PSL_SESSION", result.stderr)
        self.assertEqual(result.stdout, "")

    def test_failing_on_init_reports_failed_and_exits_1(self):
        sup = FakeSupervisor(NODES / "synthetic", "ecg")
        self.addCleanup(sup.close)
        sup.spawn()
        sup.accept_control()
        self.assertEqual(sup._recv_raw()["type"], "hello")
        sup.send({"type": "config", "session": sup.session, "node_id": "ecg", "wall_start_ns": 0,
                  "params": {"waveform": "square"}, "inputs": [],
                  "outputs": [{"name": "out", "index": 0, "type": "signal", "stream": "ecg.out"}], "options": {}})
        sup.accept_data()
        sup.read_frame()
        log = sup.wait_for(lambda m: m["type"] == "log" and m["level"] == "error", 10.0, "error log")
        self.assertIn("square", log["msg"])
        self.assertIn("traceback", log["fields"])
        failed = sup.wait_state("failed", 5.0)
        self.assertIn("square", failed["reason"])
        self.assertEqual(sup.wait_exit(5.0), 1)


class ScaleModuleProcess(unittest.TestCase):
    def test_module_scales_frames(self):
        sup = FakeSupervisor(NODES / "scale", "sc")
        self.addCleanup(sup.close)
        sup.spawn()
        sup.handshake(
            params={"gain": 2.0, "offset": 1.0},
            inputs=[{"name": "in", "index": 0, "type": "signal", "sources": [{"index": 0, "stream": "ecg.out"}]}],
            outputs=[{"name": "out", "index": 0, "type": "signal", "stream": "sc.out"}],
        )
        sup.send({"type": "start"})
        sup.wait_state("running")

        header = {"type": "signal", "srate": 100.0, "dtype": "f32",
                  "channels": [{"label": "A", "unit": "uV", "modality": "eeg"}, {"label": "B"}],
                  "source": {"device": "fake"}, "meta": {}}
        sup.send_frame(STREAM_HEADER, json.dumps(header).encode(), port=0, source=0, seq=0)
        hdr, payload = sup.read_frame()
        self.assertEqual((hdr["type"], hdr["port"], hdr["seq"]), (STREAM_HEADER, 0, 0))
        out_header = json.loads(payload)
        self.assertEqual(out_header["srate"], 100.0)
        self.assertEqual(out_header["dtype"], "f32")
        self.assertEqual([c["label"] for c in out_header["channels"]], ["A", "B"])
        self.assertEqual(out_header["channels"][1], {"label": "B", "unit": "au", "modality": "other"})
        self.assertEqual(out_header["source"], {"device": "fake"})
        self.assertEqual(out_header["meta"], {"scale": {"gain": 2.0, "offset": 1.0}})

        x = np.array([[1.0, -1.0], [0.5, 0.25], [3.0, 2.0]], np.float32)
        for i in range(1, 4):
            sup.send_frame(SIGNAL, sup.signal_payload(1000 * i, x * i, t_device=i), port=0, source=0, seq=i)
        for i in range(1, 4):
            hdr, payload = sup.read_frame()
            self.assertEqual((hdr["type"], hdr["port"], hdr["source"], hdr["seq"], hdr["flags"]), (SIGNAL, 0, 0, i, 0))
            t0, t_device, data, ts = sup.parse_signal(payload, hdr["flags"])
            self.assertEqual((t0, t_device), (1000 * i, i))
            np.testing.assert_allclose(data, x * i * 2.0 + 1.0, rtol=1e-6)

        # per-sample timestamps pass through
        ts = np.array([10, 20, 30], "<i8")
        sup.send_frame(SIGNAL, sup.signal_payload(10, x) + ts.tobytes(), port=0, source=0, seq=4, flags=1)
        hdr, payload = sup.read_frame()
        self.assertEqual((hdr["seq"], hdr["flags"]), (4, 1))
        t0, _, data, out_ts = sup.parse_signal(payload, hdr["flags"])
        np.testing.assert_array_equal(out_ts, ts)

        # live param change re-announces the header, then scales with the new gain
        sup.send({"type": "set_param", "name": "gain", "value": -1})
        ack = sup.wait_for(lambda m: m["type"] == "param_ack", 3.0, "param_ack")
        self.assertEqual((ack["name"], ack["ok"]), ("gain", True))
        hdr, payload = sup.read_frame()
        self.assertEqual((hdr["type"], hdr["seq"]), (STREAM_HEADER, 5))
        self.assertEqual(json.loads(payload)["meta"]["scale"], {"gain": -1.0, "offset": 1.0})
        sup.send_frame(SIGNAL, sup.signal_payload(50, x), port=0, source=0, seq=5)
        hdr, payload = sup.read_frame()
        self.assertEqual((hdr["type"], hdr["seq"]), (SIGNAL, 6))
        np.testing.assert_allclose(sup.parse_signal(payload, 0)[2], -x + 1.0, rtol=1e-6)

        # end of stream is propagated
        sup.send_frame(END_STREAM, b"", port=0, source=0, seq=6)
        hdr, payload = sup.read_frame()
        self.assertEqual((hdr["type"], hdr["seq"], payload), (END_STREAM, 7, b""))

        stats = sup.wait_for(lambda m: m["type"] == "stats" and m["received"].get("in", 0) >= 7, 4.0, "stats")
        self.assertEqual(stats["received"], {"in": 7})
        self.assertEqual(stats["sent"], {"out": 8})
        self.assertIsInstance(stats["latency_ns"]["in"], int)

        sup.send({"type": "stop", "grace_ms": 2000})
        sup.wait_state("stopping", 2.0)
        sup.wait_state("stopped", 2.0)
        self.assertEqual(sup.wait_exit(2.0), 0)
        self.assertNotIn("Traceback", sup.log_text())


if __name__ == "__main__":
    unittest.main()
