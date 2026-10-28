# psl — Python SDK for PSL nodes

`psl` is the Python package a node imports. It implements the node contract in
`docs/05-node-sdk.md` over the wire protocol in `docs/03-wire-protocol.md`:
handshake, clock sync, lifecycle, heartbeats, stats, timers, logging and the
data plane, so a node author writes only callbacks. Dependencies: the Python
standard library and numpy, nothing else (ADR 0004). Python 3.11 or newer.

## A node in one file

```python
import numpy as np
import psl

class Scale(psl.Node):
    def on_init(self, params):            # params: dict with defaults applied
        self.gain = params["gain"]

    def on_header(self, port, header):    # header: psl.SignalHeader / psl.EventHeader
        out = header.copy()
        out.meta["scale"] = {"gain": self.gain}
        self.declare_output("out", out)   # sends STREAM_HEADER

    def on_signal(self, port, chunk):     # chunk.data: ndarray [n_samples, n_channels]
        self.emit("out", chunk.replace(data=chunk.data * self.gain))

    def on_param(self, name, value):      # live change of a mutable param
        if name == "gain":
            self.gain = value
            return True
        return False

if __name__ == "__main__":
    psl.run(Scale)
```

The supervisor launches `python main.py` with the package directory as the
working directory and `PSL_SESSION`, `PSL_NODE_ID`, `PSL_CONTROL`, `PSL_DATA`,
`PSL_PROTOCOL` in the environment. `psl.run` never returns; exit codes are 0
(clean stop), 1 (failure), 2 (bad environment), 3 (rejected by the supervisor).

## API in brief

Callbacks on `psl.Node` (all optional): `on_init(params)`, `on_start()`,
`on_header(port, header)`, `on_signal(port, chunk)`, `on_event(port, event)`,
`on_end(port)`, `on_param(name, value) -> bool`, `on_stop()`. Every callback
and every timer runs on one worker thread, in order; user code needs no locks.

Methods: `declare_output(name, header)`, `emit(name, chunk_or_event)` (thread
safe), `end(name)`, `now()` (session ns), `every(seconds, fn)` and
`after(seconds, fn)` (return a `Timer` with `cancel()`), `spawn(fn)` (daemon
thread for blocking I/O), `log.debug/info/warn/error(msg, **fields)`,
`fail(reason)`. Read-only attributes: `params`, `inputs`, `outputs`,
`options`, `node_id`, `session`, `wall_start_ns`.

Values: `psl.Channel(label, unit="au", modality="other")`,
`psl.SignalHeader(srate, channels, dtype="f32", source={}, meta={})`,
`psl.EventHeader(labels=[], schema={}, source={}, meta={})` (both have `copy()`,
`to_json()`, `from_json()`), `psl.Chunk(t0, data, timestamps=None, t_device=0,
seq=0)` with `replace(**fields)`, `time_of(i)`, `times()`, `n_samples`,
`n_channels`, `psl.Event(t, label, data=None, t_device=0, seq=0)`, and
`psl.Port(name, index, type, source, source_index)` as passed to callbacks
(`port.source` is the stream id the frame came from, e.g. `"ecg.out"`).

Rules the SDK enforces: nothing is published before `start` (a header declared
earlier is sent on start; `emit` before start raises); a `STREAM_HEADER`
precedes any data on an output (`emit` on an undeclared output raises); every
frame on an output stream, header or data or end, takes the next sequence
number; data is converted to the header's dtype and checked against its
channel count.

## Offline testing

```python
import numpy as np, psl
from psl.testing import Harness

h = Harness(MovingAverage, params={"window": 5})     # no sockets, no supervisor
h.header("in", psl.SignalHeader(srate=100, channels=[psl.Channel("A")]))
h.signal("in", psl.Chunk(t0=0, data=np.ones((10, 1), np.float32)))
h.outputs("out")        # [Chunk]        h.headers("out")   # [SignalHeader]
h.logs                  # [(level, msg)] h.set_param("window", 3) -> bool
h.tick()                # run every() timers once; h.advance(0.02) moves the clock
h.replay("tests/fixtures/ecg.pslrec", map={"ecg.out": "in"})
```

The harness passes the params you give it unchanged (it does not read
`node.psl`) and round-trips everything through the wire codec.

## Layout

| File | Contents |
|------|----------|
| `psl/_values.py` | `Channel`, `SignalHeader`, `EventHeader`, `Chunk`, `Event`, `Port` |
| `psl/_protocol.py` | byte-level codec for frames, payloads and control messages |
| `psl/_transport.py` | Unix-socket / TCP connection with framed reads and locked writes |
| `psl/_node.py` | `Node`, the runtime threads, clock sync, `run()` |
| `psl/testing.py` | `Harness` |
| `NOTES.md` | decisions taken where the docs were open to interpretation |

## Running the tests

```sh
cd sdk/python
../../.venv/bin/python -m unittest discover -s tests -v
```

`tests/test_integration.py` starts the real `psl/synthetic` and `psl/scale`
node processes against a fake supervisor written from the protocol document.
Each first-party node has its own tests:

```sh
cd nodes/psl/<name>
PYTHONPATH=../../../sdk/python ../../../.venv/bin/python -m unittest discover -s tests -v
```
