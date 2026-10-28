# 05 — The node contract and the Python SDK

## The contract (language-neutral)

A node is a process. To be a valid PSL node it must:

1. Read `PSL_SESSION`, `PSL_NODE_ID`, `PSL_CONTROL`, `PSL_DATA`,
   `PSL_PROTOCOL` from its environment (`PSL_WORKSPACE` and `PSL_NODE_DIR` say
   where it runs and where its package lives; the manifest hash in `hello` is
   computed from `$PSL_NODE_DIR/node.psl`).
2. Connect to the control and data addresses and perform the handshake in 03.
3. Follow the lifecycle below and report each transition with a `state`
   message.
4. Answer every `ping` with `pong` within three seconds.
5. Send a `STREAM_HEADER` on an output before any `SIGNAL` or `EVENT` on it,
   and never publish before receiving `start`.
6. Stamp every chunk and event in session time (03 §Clock sync).
7. On `stop`, `SIGTERM` or `SIGINT`: finish in-flight work, send
   `state{stopped}`, and exit within the grace period.
8. Never write to stdout for anything but logs; stdout and stderr go to the
   session log file.

Anything that does these eight things is a node, in any language, with no
PSL library at all. The SDKs exist so that nobody has to do them by hand.

### Lifecycle

```
created ──hello/config──▶ configured ──clock sync──▶ ready ──start──▶ running
                                                                       │
                         failed ◀──── any error ──────────────────────┤
                                                                       ▼
                                                          stopping ──▶ stopped
```

### Kind rules restated for authors

| Kind | Must | Must not |
|------|------|----------|
| Input driver | Open the device, decode what it delivers into readable samples, stamp on receipt, fill `source` in the header | Filter, resample, interpolate, reject artifacts, re-reference, average, reorder samples |
| Module | One well-defined transformation; pass `source` and `meta` through, appending to `meta` | Hide several unrelated steps behind one node |
| Output driver | Accept data and deliver it somewhere in a documented format | Alter values or timestamps |

The line for drivers (ADR 0012): a driver may do only what **every**
application using that device would have to do anyway to make the data
readable. Unpacking the packet format, decoding packed or compressed samples,
applying the manufacturer's documented scale factor to reach microvolts, and
labelling channels all pass that test. Anything an application could
reasonably skip or do differently does not, and belongs in a module. List
what the driver does in the header's `source.decoding`.

## Python SDK

Package name `psl`, installed into the workspace's `.psl/venv` by the core.
Dependencies: the Python standard library and numpy. Nothing else, ever
(ADR 0004).

### A module

```python
import numpy as np
import psl

class MovingAverage(psl.Node):
    def on_init(self, params):
        self.window = params["window"]          # already type-checked by psl check
        self.buf = None

    def on_header(self, port, header):
        # header.srate, header.channels, header.dtype, header.source, header.meta
        self.buf = np.zeros((self.window, len(header.channels)), dtype=np.float32)
        out = header.copy()
        out.meta["moving_average"] = {"window": self.window}
        self.declare_output("out", out)       # sends STREAM_HEADER

    def on_signal(self, port, chunk):
        # chunk.data is an ndarray shaped [n_samples, n_channels]
        y = self._filter(chunk.data)
        self.emit("out", chunk.replace(data=y))

    def on_param(self, name, value):
        if name == "window":
            self.window = value
            return True                        # accepted
        return False

if __name__ == "__main__":
    psl.run(MovingAverage)
```

### An input driver

```python
import time, numpy as np, psl

class Synthetic(psl.Node):
    def on_init(self, params):
        self.srate = params["srate"]
        self.n = 0
        self.header = psl.SignalHeader(
            srate=self.srate,
            channels=[psl.Channel(label=c, unit="uV", modality="ecg") for c in params["channels"]],
            source={"device": "psl/synthetic", "waveform": params["waveform"]},
        )

    def on_start(self):
        self.declare_output("out", self.header)
        self.every(0.02, self.tick)              # 20 ms chunks

    def tick(self):
        n = int(self.srate * 0.02)
        t0 = self.now() - int(0.02 * 1e9)
        data = self._wave(self.n, n).astype(np.float32)
        self.n += n
        self.emit("out", psl.Chunk(t0=t0, data=data))

if __name__ == "__main__":
    psl.run(Synthetic)
```

A driver that blocks on a vendor SDK runs it on a background thread with
`self.spawn(fn)` and calls `self.emit` from there; `emit` is thread-safe.

### An output driver

```python
import csv, psl

class CsvWriter(psl.Node):
    def on_init(self, params):
        self.f = open(params["path"], "w", newline="")
        self.w = csv.writer(self.f)

    def on_header(self, port, header):
        self.w.writerow(["t_ns"] + [c.label for c in header.channels])

    def on_signal(self, port, chunk):
        for i, row in enumerate(chunk.data):
            self.w.writerow([chunk.time_of(i)] + row.tolist())

    def on_stop(self):
        self.f.close()

if __name__ == "__main__":
    psl.run(CsvWriter)
```

### API reference (version one)

The implementation in `sdk/python/` adds a few conveniences beyond this
table (`Chunk.srate`, `Chunk.times()`, `Chunk.duration_ns`, `Timer.cancel()`,
`psl.NodeFailed`, Harness `advance()` and `replay()`); `sdk/python/README.md`
and `NOTES.md` describe them until they are folded in here.

**Callbacks** on `psl.Node`, all optional:

| Callback | When |
|----------|------|
| `on_init(params)` | After `config`. Parameters are a plain dict with defaults applied. |
| `on_start()` | After `start`. Drivers declare outputs and begin here. |
| `on_header(port, header)` | A stream header arrived on an input port. Called again if the producer re-announces. |
| `on_signal(port, chunk)` | A signal chunk arrived. |
| `on_event(port, event)` | An event arrived. |
| `on_end(port)` | The source sent end-of-stream. |
| `on_param(name, value) -> bool` | A mutable parameter was set from outside. Return `True` to accept. |
| `on_stop()` | Before exit. Flush and close here. |

**Methods** on `psl.Node`:

| Method | Does |
|--------|------|
| `declare_output(name, header)` | Send a stream header for an output port. |
| `emit(name, chunk_or_event)` | Publish on an output port. Thread-safe. |
| `end(name)` | Send end-of-stream on an output port. |
| `now() -> int` | Session time in nanoseconds. |
| `every(seconds, fn) -> Timer` | Repeating timer on the worker thread. |
| `after(seconds, fn) -> Timer` | One-shot timer. |
| `spawn(fn) -> Thread` | Background thread for blocking I/O. |
| `log.debug/info/warn/error(msg, **fields)` | Structured log, forwarded to the supervisor. |
| `fail(reason)` | Move to `failed` and exit. |
| `params`, `inputs`, `outputs`, `node_id`, `session` | Read-only attributes from `config`. |

**Values:**

| Class | Fields |
|-------|--------|
| `psl.Port` | `name`, `index`, `type`, `source` (stream id of the frame's origin, e.g. `"ecg.out"`), `source_index` |
| `psl.Channel` | `label`, `unit="au"`, `modality="other"` |
| `psl.SignalHeader` | `srate`, `channels`, `dtype="f32"`, `source={}`, `meta={}`; `copy()` |
| `psl.EventHeader` | `labels=[]`, `schema={}`, `source={}`, `meta={}`; `copy()` |
| `psl.Chunk` | `t0`, `data` (ndarray `[n_samples, n_channels]`), `timestamps=None`, `t_device=0`, `seq`; `replace(**fields)`, `time_of(i)`, `n_samples`, `n_channels` |
| `psl.Event` | `t`, `label`, `data=None`, `t_device=0`, `seq` |

**Threading model.** The SDK runs one receive thread per socket and one
worker thread. Every callback and every timer runs on the worker thread, in
order, so user code needs no locks. `emit` and `log` may be called from any
thread.

**Running.** `psl.run(NodeClass)` does everything in the contract and never
returns. The core launches the entry script named in the manifest's
`runtime python { entry = ... }` with the workspace's interpreter, from the
workspace root, with the SDK on `PYTHONPATH`. Python puts the script's own
directory on `sys.path`, so a package may import its sibling files.

### Offline testing

The SDK separates dispatch from transport so a node can be exercised with no
supervisor and no sockets:

```python
from psl.testing import Harness

h = Harness(MovingAverage, params={"window": 5})
h.header("in", psl.SignalHeader(srate=100, channels=[psl.Channel("A")]))
h.signal("in", psl.Chunk(t0=0, data=np.ones((10, 1), np.float32)))
out = h.outputs("out")               # list of Chunk / Event emitted
```

A recording can also be fed: `h.replay("tests/fixtures/ecg.pslrec",
map={"ecg.out": "in"})`. `psl test` runs a package's `tests/` directory
with the workspace venv.

## Writing a driver for real hardware (when hardware arrives)

1. Scaffold with `psl new node polar-h10 --kind input`.
2. In `on_start`, connect to the device on a background thread (`spawn`).
   For BLE on macOS, the `bleak` package works from a terminal process; the
   first run triggers the system Bluetooth permission prompt.
3. As soon as the device reports its configuration, build a
   `SignalHeader` with real channel labels, units, modality and a `source`
   describing device, serial and firmware. Call `declare_output`.
4. For each packet the device delivers, build one `Chunk`. Set `t0` to
   `now()` minus the packet duration unless the device gives a usable clock;
   put the device clock in `t_device` either way.
5. Expose device choices (address, sample rate, channel mask) as `param`s in
   the manifest. Expose nothing that would alter the data.
6. On disconnect, log at `warn`, try to reconnect, and if that fails call
   `end("out")` or `fail(...)` depending on the manifest's documented policy.
7. Record a session with `psl run --record` and commit a short recording as a
   test fixture so the driver's downstream users can test without the device.

## Other SDKs

The C SDK (for manufacturers) and a JavaScript SDK are planned after the
Python SDK is stable. Each must offer the same callbacks and values with
local naming conventions, and must pass the same protocol conformance tests
in `core/tests/conformance/`.
