"""The node contract: ``Node`` (what authors subclass) and ``run`` (the runtime).

Dispatch is separated from transport. ``Node`` owns ports, sequence counters,
header caches and the callback entry points; it talks to the outside world
through a small ``Runtime`` interface. ``_SocketRuntime`` implements it with
the two sockets, three threads (control reader, data reader, worker) and a
scheduler thread; ``psl.testing.Harness`` implements it in-process.

Threading (docs/05 §Threading model): every callback and every timer runs on
the single worker thread, in order. ``emit``, ``declare_output``, ``end``,
``now`` and ``log`` may be called from any thread.
"""

from __future__ import annotations

import functools
import hashlib
import heapq
import itertools
import os
import queue
import signal
import sys
import threading
import time
import traceback
from dataclasses import dataclass, field
from typing import Any, Callable, NoReturn, Protocol

import numpy as np

from ._protocol import (
    END_STREAM,
    EVENT,
    IDENTIFY,
    PROTOCOL_VERSION,
    SEQ_MASK,
    SIGNAL,
    STREAM_HEADER,
    FrameHeader,
    ProtocolError,
    decode_event,
    decode_signal,
    decode_stream_header,
    end_stream_frame,
    event_frame,
    identify_frame,
    signal_frame,
    stream_header_frame,
)
from ._transport import Connection, ConnectionClosed, connect
from ._values import NUMPY_DTYPES, Chunk, Event, EventHeader, Header, Port, SignalHeader

SDK_VERSION: str = "0.1.0"
SDK_NAME: str = f"psl-python/{SDK_VERSION}"

#: Grace period used when the node is stopped by SIGTERM/SIGINT rather than a
#: ``stop`` message (the pipeline default ``stop_grace_s`` is 5).
DEFAULT_GRACE_MS: int = 5000
CLOCK_SYNC_ROUNDS: int = 8
CLOCK_RESYNC_S: float = 10.0
STATS_INTERVAL_S: float = 1.0
LOG_LEVELS: tuple[str, ...] = ("debug", "info", "warn", "error")

ENV_VARS: tuple[str, ...] = ("PSL_SESSION", "PSL_NODE_ID", "PSL_CONTROL", "PSL_DATA", "PSL_PROTOCOL")

Callback = Callable[[], None]


# --------------------------------------------------------------------------- #
# Small public helpers
# --------------------------------------------------------------------------- #


class Timer:
    """Handle returned by ``Node.every`` and ``Node.after``.

    ``cancel()`` prevents any further run. A repeating timer whose previous
    run is still waiting on the worker queue is not queued again (ticks
    coalesce rather than pile up when the worker is slow).
    """

    __slots__ = ("fn", "seconds", "repeat", "_cancelled", "_pending", "_direct")

    def __init__(self, fn: Callback, seconds: float, repeat: bool, *, direct: bool = False) -> None:
        self.fn = fn
        self.seconds = float(seconds)
        self.repeat = repeat
        self._cancelled = False
        self._pending = False
        self._direct = direct   # run on the scheduler thread itself (SDK-internal timers only)

    def cancel(self) -> None:
        self._cancelled = True

    @property
    def cancelled(self) -> bool:
        return self._cancelled

    def _run(self) -> None:
        self._pending = False
        if not self._cancelled:
            self.fn()


class Logger:
    """``self.log`` on a node: structured log lines forwarded to the supervisor."""

    def __init__(self, sink: Callable[[str, str, dict[str, Any]], None]) -> None:
        self._sink = sink

    def debug(self, msg: str, **fields: Any) -> None:
        self._sink("debug", str(msg), fields)

    def info(self, msg: str, **fields: Any) -> None:
        self._sink("info", str(msg), fields)

    def warn(self, msg: str, **fields: Any) -> None:
        self._sink("warn", str(msg), fields)

    warning = warn

    def error(self, msg: str, **fields: Any) -> None:
        self._sink("error", str(msg), fields)


class NodeFailed(RuntimeError):
    """Raised by ``Node.fail`` when there is no process to exit (the test harness)."""

    def __init__(self, reason: str) -> None:
        super().__init__(reason)
        self.reason = reason


# --------------------------------------------------------------------------- #
# Internal state
# --------------------------------------------------------------------------- #


@dataclass
class _InputState:
    name: str
    index: int
    type: str
    sources: dict[int, str] = field(default_factory=dict)
    ports: dict[int, Port] = field(default_factory=dict)
    headers: dict[int, Header] = field(default_factory=dict)
    received: int = 0
    last_t: int | None = None

    def port_for(self, source_index: int) -> Port:
        port = self.ports.get(source_index)
        if port is None:
            # A source index the config did not list. Deliver it anyway with an
            # empty stream id rather than drop data on the floor (NOTES.md).
            port = Port(self.name, self.index, self.type, source="", source_index=source_index)
            self.ports[source_index] = port
        return port


@dataclass
class _OutputState:
    name: str
    index: int
    type: str
    stream: str
    seq: int = 0
    header: Header | None = None     # header currently on the wire (stream open)
    pending: Header | None = None    # declared before start; sent on start
    sent: int = 0
    lock: threading.Lock = field(default_factory=threading.Lock)

    def next_seq(self) -> int:
        seq = self.seq
        self.seq = (seq + 1) & SEQ_MASK
        self.sent += 1
        return seq


class Runtime(Protocol):
    """What a ``Node`` needs from its environment (sockets or a harness)."""

    def now(self) -> int: ...
    def send_header(self, out: _OutputState, header: Header, seq: int) -> None: ...
    def send_chunk(self, out: _OutputState, chunk: Chunk) -> None: ...
    def send_event(self, out: _OutputState, event: Event) -> None: ...
    def send_end(self, out: _OutputState, seq: int) -> None: ...
    def send_log(self, level: str, msg: str, fields: dict[str, Any]) -> None: ...
    def add_timer(self, timer: Timer) -> None: ...
    def fail(self, reason: str) -> NoReturn: ...
    def crash(self, exc: BaseException, where: str) -> None: ...
    def unknown_output(self, node: "Node", name: str, port_type: str) -> _OutputState | None: ...


# --------------------------------------------------------------------------- #
# Node
# --------------------------------------------------------------------------- #


class Node:
    """Base class for every Python node. Override the ``on_*`` callbacks.

    Do not override ``__init__``; use ``on_init(params)``. All callbacks run on
    one worker thread, so no locks are needed in user code.
    """

    #: Input drivers with ``dynamic_outputs = true`` in their manifest set this
    #: to a list of ``{"name": ..., "type": "signal"|"event"}`` before ``run``
    #: is called; it is announced in ``hello``. Leave ``None`` otherwise.
    dynamic_outputs: list[dict[str, str]] | None = None

    def __init__(self) -> None:
        self.params: dict[str, Any] = {}
        self.inputs: list[dict[str, Any]] = []
        self.outputs: list[dict[str, Any]] = []
        self.options: dict[str, Any] = {}
        self.node_id: str = ""
        self.session: str = ""
        self.wall_start_ns: int = 0
        self.log: Logger = Logger(self._emit_log)
        self._rt: Runtime | None = None
        self._in_by_index: dict[int, _InputState] = {}
        self._in_by_name: dict[str, _InputState] = {}
        self._out_by_name: dict[str, _OutputState] = {}
        self._running: bool = False
        self._run_lock = threading.Lock()
        self._stats_lock = threading.Lock()

    # -- callbacks (all optional) -------------------------------------------

    def on_init(self, params: dict[str, Any]) -> None:
        """After ``config``. ``params`` has defaults applied."""

    def on_start(self) -> None:
        """After ``start``. Drivers declare outputs and begin here."""

    def on_header(self, port: Port, header: Header) -> None:
        """A stream header arrived on an input port."""

    def on_signal(self, port: Port, chunk: Chunk) -> None:
        """A signal chunk arrived."""

    def on_event(self, port: Port, event: Event) -> None:
        """An event arrived."""

    def on_end(self, port: Port) -> None:
        """The source sent end-of-stream."""

    def on_param(self, name: str, value: Any) -> bool:
        """A mutable parameter was set from outside. Return True to accept."""
        return False

    def on_stop(self) -> None:
        """Before exit. Flush and close here."""

    # -- methods ------------------------------------------------------------

    def declare_output(self, name: str, header: Header) -> None:
        """Send a stream header for output ``name`` (reopens the stream after ``end``).

        Before ``start`` the header is kept and sent as soon as ``start``
        arrives, because nothing may be published before start (docs/05 rule 5).
        """
        if not isinstance(header, (SignalHeader, EventHeader)):
            raise TypeError(f"header must be a SignalHeader or EventHeader, got {type(header).__name__}")
        out = self._out_by_name.get(name)
        if out is None:
            # Only a harness answers this; the socket runtime knows every port from config.
            out = self._runtime().unknown_output(self, name, header.type)
        if out is None:
            out = self._output(name)   # raises with the list of known outputs
        if header.type != out.type:
            raise ValueError(f"output {name!r} is a {out.type} port; cannot declare a {header.type} header on it")
        hdr = header.copy()
        with self._run_lock:
            if not self._running:
                out.pending = hdr
                return
        self._send_header(out, hdr)

    def emit(self, name: str, item: Chunk | Event) -> None:
        """Publish a chunk or event on output ``name``. Thread-safe."""
        out = self._output(name)
        if isinstance(item, Chunk):
            kind = "signal"
        elif isinstance(item, Event):
            kind = "event"
        else:
            raise TypeError(f"emit() takes a Chunk or an Event, got {type(item).__name__}")
        if out.type != kind:
            raise ValueError(f"output {name!r} is a {out.type} port; cannot emit a {kind} on it")
        with self._run_lock:
            running = self._running
        if not running:
            raise RuntimeError(f"emit({name!r}) before start: nothing may be published before the supervisor sends start")
        rt = self._runtime()
        with out.lock:
            header = out.header
            if header is None:
                raise RuntimeError(f"output {name!r} has no stream header; call declare_output() first")
            if isinstance(item, Chunk):
                assert isinstance(header, SignalHeader)
                chunk = self._prepare_chunk(item, header, name)
                chunk = chunk.replace(seq=out.next_seq())
                rt.send_chunk(out, chunk)
            else:
                event = item.replace(seq=out.next_seq())
                rt.send_event(out, event)

    def end(self, name: str) -> None:
        """Send end-of-stream on output ``name``. ``declare_output`` reopens it."""
        out = self._output(name)
        with self._run_lock:
            if not self._running:
                raise RuntimeError(f"end({name!r}) before start")
        rt = self._runtime()
        with out.lock:
            if out.header is None:
                raise RuntimeError(f"output {name!r} has no open stream to end")
            out.header = None
            rt.send_end(out, out.next_seq())

    def now(self) -> int:
        """Session time in nanoseconds (docs/03 §Clock sync)."""
        return self._runtime().now()

    def every(self, seconds: float, fn: Callback) -> Timer:
        """Call ``fn`` on the worker thread every ``seconds``."""
        timer = Timer(fn, self._check_seconds(seconds), repeat=True)
        self._runtime().add_timer(timer)
        return timer

    def after(self, seconds: float, fn: Callback) -> Timer:
        """Call ``fn`` once on the worker thread after ``seconds``."""
        timer = Timer(fn, self._check_seconds(seconds), repeat=False)
        self._runtime().add_timer(timer)
        return timer

    def spawn(self, fn: Callback) -> threading.Thread:
        """Run ``fn`` on a daemon thread (for blocking I/O). An uncaught exception fails the node."""
        rt = self._runtime()

        def runner() -> None:
            try:
                fn()
            except SystemExit:
                pass
            except Exception as exc:  # noqa: BLE001 - reported, then the node fails
                rt.crash(exc, f"spawned thread {getattr(fn, '__name__', 'fn')}")

        thread = threading.Thread(target=runner, name=f"psl-spawn-{getattr(fn, '__name__', 'fn')}", daemon=True)
        thread.start()
        return thread

    def fail(self, reason: str) -> NoReturn:
        """Report ``state{failed}`` and exit with code 1. Never returns."""
        self._runtime().fail(str(reason))

    # -- internals shared by the socket runtime and the harness --------------

    def _attach(self, runtime: Runtime) -> None:
        self._rt = runtime

    def _runtime(self) -> Runtime:
        if self._rt is None:
            raise RuntimeError("node is not attached to a runtime (use psl.run or psl.testing.Harness)")
        return self._rt

    def _configure(self, config: dict[str, Any]) -> None:
        self.session = str(config.get("session", ""))
        self.node_id = str(config.get("node_id", ""))
        self.wall_start_ns = int(config.get("wall_start_ns", 0) or 0)
        self.params = dict(config.get("params") or {})
        self.options = dict(config.get("options") or {})
        self.inputs = [dict(spec) for spec in (config.get("inputs") or [])]
        self.outputs = [dict(spec) for spec in (config.get("outputs") or [])]
        self._in_by_index.clear()
        self._in_by_name.clear()
        self._out_by_name.clear()
        for spec in self.inputs:
            sources = []
            for src in spec.get("sources") or []:
                if isinstance(src, dict):
                    sources.append((int(src.get("index", len(sources))), str(src.get("stream", ""))))
                else:
                    sources.append((len(sources), str(src)))
            self._add_input(str(spec["name"]), int(spec["index"]), str(spec.get("type", "signal")), sources)
        for spec in self.outputs:
            self._add_output(
                str(spec["name"]), int(spec["index"]), str(spec.get("type", "signal")), str(spec.get("stream", ""))
            )

    def _add_input(self, name: str, index: int, port_type: str, sources: list[tuple[int, str]]) -> _InputState:
        state = _InputState(name=name, index=index, type=port_type)
        for source_index, stream in sources:
            state.sources[source_index] = stream
            state.ports[source_index] = Port(name, index, port_type, source=stream, source_index=source_index)
        self._in_by_index[index] = state
        self._in_by_name[name] = state
        return state

    def _add_output(self, name: str, index: int, port_type: str, stream: str) -> _OutputState:
        state = _OutputState(name=name, index=index, type=port_type, stream=stream)
        self._out_by_name[name] = state
        return state

    def _output(self, name: str) -> _OutputState:
        try:
            return self._out_by_name[name]
        except KeyError:
            known = ", ".join(sorted(self._out_by_name)) or "none"
            raise ValueError(f"unknown output port {name!r} (declared outputs: {known})") from None

    def _input_state(self, port_index: int) -> _InputState | None:
        return self._in_by_index.get(port_index)

    def _input_port(self, port_index: int, source_index: int) -> Port | None:
        state = self._in_by_index.get(port_index)
        return None if state is None else state.port_for(source_index)

    def _note_header(self, port: Port, header: Header) -> None:
        state = self._in_by_index.get(port.index)
        if state is not None:
            state.headers[port.source_index] = header

    def _input_header(self, port: Port) -> Header | None:
        state = self._in_by_index.get(port.index)
        return None if state is None else state.headers.get(port.source_index)

    def _note_received(self, port: Port, t: int | None) -> None:
        state = self._in_by_index.get(port.index)
        if state is None:
            return
        with self._stats_lock:
            state.received += 1
            if t is not None:
                state.last_t = t

    def _stats(self, now: int) -> dict[str, Any]:
        with self._stats_lock:
            sent = {o.name: o.sent for o in self._out_by_name.values()}
            received = {i.name: i.received for i in self._in_by_index.values()}
            latency = {i.name: (now - i.last_t if i.last_t is not None else 0) for i in self._in_by_index.values()}
        return {"type": "stats", "sent": sent, "received": received, "dropped_local": 0, "latency_ns": latency}

    def _set_running(self) -> None:
        """Mark the node running and send every header declared before start."""
        with self._run_lock:
            self._running = True
            pending = [(out, out.pending) for out in self._out_by_name.values() if out.pending is not None]
            for out, _ in pending:
                out.pending = None
        for out, header in pending:
            self._send_header(out, header)

    def _is_running(self) -> bool:
        with self._run_lock:
            return self._running

    def _send_header(self, out: _OutputState, header: Header) -> None:
        rt = self._runtime()
        with out.lock:
            out.header = header
            rt.send_header(out, header, out.next_seq())

    @staticmethod
    def _prepare_chunk(chunk: Chunk, header: SignalHeader, name: str) -> Chunk:
        """Data in the stream dtype, C-contiguous, with the channel count checked."""
        data = np.ascontiguousarray(chunk.data, dtype=NUMPY_DTYPES[header.dtype])
        if data.shape[1] != header.n_channels:
            raise ValueError(
                f"chunk for output {name!r} has {data.shape[1]} channels but its header declares {header.n_channels}"
            )
        timestamps = chunk.timestamps
        if timestamps is not None:
            timestamps = np.ascontiguousarray(timestamps, dtype=np.int64)
        return chunk.replace(data=data, timestamps=timestamps, srate=header.srate)

    @staticmethod
    def _check_seconds(seconds: float) -> float:
        seconds = float(seconds)
        if not seconds >= 0:
            raise ValueError(f"timer period must be >= 0 seconds, got {seconds}")
        return seconds

    def _emit_log(self, level: str, msg: str, fields: dict[str, Any]) -> None:
        if self._rt is None:
            print(f"[{self.node_id or 'node'}] {level}: {msg}" + (f" {fields}" if fields else ""), file=sys.stderr, flush=True)
            return
        self._rt.send_log(level, msg, fields)

    # -- dispatch: called on the worker thread by the runtime, directly by the harness

    def _dispatch_init(self) -> None:
        self.on_init(dict(self.params))

    def _dispatch_start(self) -> None:
        self.on_start()

    def _dispatch_header(self, port: Port, header: Header) -> None:
        self.on_header(port, header)

    def _dispatch_signal(self, port: Port, chunk: Chunk) -> None:
        self.on_signal(port, chunk)

    def _dispatch_event(self, port: Port, event: Event) -> None:
        self.on_event(port, event)

    def _dispatch_end(self, port: Port) -> None:
        self.on_end(port)

    def _dispatch_param(self, name: str, value: Any) -> tuple[bool, str | None]:
        ok = bool(self.on_param(name, value))
        if ok:
            self.params[name] = value
            return True, None
        return False, "rejected by node"

    def _dispatch_stop(self) -> None:
        self.on_stop()


# --------------------------------------------------------------------------- #
# Clock
# --------------------------------------------------------------------------- #


class _Clock:
    """Session clock: local monotonic ns plus an offset estimated against the supervisor.

    ``now()`` never goes backwards: if a re-sync lowers the offset, returned
    values are clamped to the last value until the local clock catches up.
    """

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._offset = 0
        self._last = -(1 << 63)
        self.rtt: int | None = None

    @property
    def offset(self) -> int:
        with self._lock:
            return self._offset

    def now(self) -> int:
        with self._lock:
            value = time.monotonic_ns() + self._offset
            if value < self._last:
                value = self._last
            else:
                self._last = value
            return value

    def apply(self, offset: int, rtt: int) -> None:
        with self._lock:
            self._offset = offset
            self.rtt = rtt

    @staticmethod
    def exchange(
        send: Callable[[dict[str, Any]], None],
        recv: Callable[[], dict[str, Any]],
        rounds: int = CLOCK_SYNC_ROUNDS,
    ) -> tuple[int, int]:
        """``rounds`` four-timestamp exchanges; returns ``(offset, rtt)`` of the smallest rtt."""
        best: tuple[int, int] | None = None
        for _ in range(rounds):
            t1 = time.monotonic_ns()
            send({"type": "clock_req", "t1": t1})
            stale = 0
            while True:
                res = recv()
                if res.get("type") == "clock_res" and int(res.get("t1", t1)) == t1:
                    break
                stale += 1
                if stale > 2 * rounds:
                    raise ProtocolError("clock_res never echoed the t1 that was sent")
            t4 = time.monotonic_ns()
            t2 = int(res["t2"])
            t3 = int(res["t3"])
            rtt = (t4 - t1) - (t3 - t2)
            offset = ((t2 - t1) + (t3 - t4)) // 2
            if best is None or rtt < best[1]:
                best = (offset, rtt)
        assert best is not None
        return best


# --------------------------------------------------------------------------- #
# Scheduler: timers, with callbacks queued to the worker
# --------------------------------------------------------------------------- #


class _Scheduler(threading.Thread):
    def __init__(self, work: "queue.SimpleQueue[Callback | None]") -> None:
        super().__init__(name="psl-scheduler", daemon=True)
        self._work = work
        self._cond = threading.Condition()
        self._heap: list[tuple[float, int, Timer]] = []
        self._counter = itertools.count()
        self._stopped = False

    def add(self, timer: Timer) -> None:
        with self._cond:
            heapq.heappush(self._heap, (time.monotonic() + timer.seconds, next(self._counter), timer))
            self._cond.notify()

    def stop(self) -> None:
        with self._cond:
            self._stopped = True
            self._cond.notify()

    def run(self) -> None:
        while True:
            with self._cond:
                while not self._stopped and (not self._heap or self._heap[0][0] > time.monotonic()):
                    timeout = None if not self._heap else max(0.0, self._heap[0][0] - time.monotonic())
                    self._cond.wait(timeout)
                if self._stopped:
                    return
                due, _, timer = heapq.heappop(self._heap)
            if timer.cancelled:
                continue
            self._fire(timer)
            if timer.repeat and not timer.cancelled:
                nxt = due + timer.seconds
                now = time.monotonic()
                if nxt < now:
                    nxt = now   # fell behind: skip the missed ticks rather than burst
                with self._cond:
                    heapq.heappush(self._heap, (nxt, next(self._counter), timer))

    def _fire(self, timer: Timer) -> None:
        if timer._direct:
            try:
                timer.fn()
            except Exception:  # noqa: BLE001 - SDK-internal timers must not kill the scheduler
                traceback.print_exc(file=sys.stderr)
            return
        if timer._pending:
            return
        timer._pending = True
        self._work.put(timer._run)


# --------------------------------------------------------------------------- #
# Socket runtime
# --------------------------------------------------------------------------- #


class _SocketRuntime:
    """The real runtime: two sockets, a worker, a scheduler and the lifecycle."""

    def __init__(
        self,
        node: Node,
        control: Connection,
        data: Connection,
        clock: _Clock,
        deferred: list[dict[str, Any]],
    ) -> None:
        self.node = node
        self.control = control
        self.data = data
        self.clock = clock
        self._deferred = deferred
        self._work: "queue.SimpleQueue[Callback | None]" = queue.SimpleQueue()
        self._scheduler = _Scheduler(self._work)
        self._clock_q: "queue.Queue[dict[str, Any]]" = queue.Queue()
        self._state_lock = threading.Lock()
        self._exited = threading.Event()
        self._exit_code = 0
        self._started = False
        self._stopping = False
        self._warned: set[tuple[Any, ...]] = set()
        self._threads: list[threading.Thread] = []

    # -- Runtime interface --------------------------------------------------

    def now(self) -> int:
        return self.clock.now()

    def send_header(self, out: _OutputState, header: Header, seq: int) -> None:
        self.data.write_frame(stream_header_frame(out.index, seq, header))

    def send_chunk(self, out: _OutputState, chunk: Chunk) -> None:
        header = out.header
        dtype = header.dtype if isinstance(header, SignalHeader) else "f32"
        self.data.write_frame(signal_frame(out.index, chunk.seq, chunk, dtype))

    def send_event(self, out: _OutputState, event: Event) -> None:
        self.data.write_frame(event_frame(out.index, event.seq, event))

    def send_end(self, out: _OutputState, seq: int) -> None:
        self.data.write_frame(end_stream_frame(out.index, seq))

    def send_log(self, level: str, msg: str, fields: dict[str, Any]) -> None:
        if level not in LOG_LEVELS:
            level = "info"
        # The supervisor writes forwarded log messages to the node's log file and echoes
        # them to its terminal, and stderr goes to the same file, so no mirror here.
        self._send_control_quiet({"type": "log", "level": level, "msg": msg, "fields": dict(fields)})

    def add_timer(self, timer: Timer) -> None:
        self._scheduler.add(timer)

    def fail(self, reason: str) -> NoReturn:
        self._fail(reason)
        raise SystemExit(1)

    def unknown_output(self, node: Node, name: str, port_type: str) -> _OutputState | None:
        return None

    def crash(self, exc: BaseException, where: str) -> None:
        tb = "".join(traceback.format_exception(exc))
        summary = f"{type(exc).__name__}: {exc}"
        self.send_log("error", f"uncaught exception in {where}: {summary}", {"traceback": tb})
        self._fail(summary)

    # -- lifecycle ----------------------------------------------------------

    def start(self) -> None:
        self._threads = [
            threading.Thread(target=self._control_loop, name="psl-control", daemon=True),
            threading.Thread(target=self._data_loop, name="psl-data", daemon=True),
            threading.Thread(target=self._worker_loop, name="psl-worker", daemon=True),
        ]
        self._scheduler.add(Timer(self._send_stats, STATS_INTERVAL_S, repeat=True, direct=True))
        self._scheduler.add(Timer(self._resync_clock, CLOCK_RESYNC_S, repeat=True, direct=True))
        self._scheduler.start()
        for thread in self._threads:
            thread.start()
        self._work.put(self._init_task)

    def wait(self) -> int:
        """Block the main thread until the node has exited; returns the exit code."""
        while not self._exited.wait(0.25):
            pass
        return self._exit_code

    def request_stop(self, grace_ms: int, *, code: int = 0, reason: str | None = None) -> None:
        with self._state_lock:
            if self._stopping or self._exited.is_set():
                return
            self._stopping = True
            if code:
                self._exit_code = code
        if reason:
            print(f"[{self.node.node_id}] stopping: {reason}", file=sys.stderr, flush=True)
        self._send_state("stopping")
        self._scheduler.stop()
        self._work.put(None)
        grace_s = max(0.05, int(grace_ms) / 1000.0)
        watchdog = threading.Timer(grace_s, self._grace_expired, args=(grace_ms,))
        watchdog.daemon = True
        watchdog.start()

    def _grace_expired(self, grace_ms: int) -> None:
        if self._exited.is_set():
            return
        print(f"[{self.node.node_id}] stop grace period of {grace_ms} ms exceeded; exiting", file=sys.stderr, flush=True)
        self._fail(f"stop grace period of {grace_ms} ms exceeded")

    def _fail(self, reason: str) -> None:
        with self._state_lock:
            if self._exited.is_set():
                return
            self._exit_code = 1
            self._send_state("failed", reason)
            self._exited.set()
        self._scheduler.stop()

    def _finish_stop(self) -> None:
        try:
            self.node._dispatch_stop()
        except SystemExit:
            return
        except Exception as exc:  # noqa: BLE001
            self.crash(exc, "on_stop")
            return
        with self._state_lock:
            if self._exited.is_set():
                return
            self._send_state("stopped")
            self._exited.set()

    def _connection_lost(self, which: str, exc: BaseException) -> None:
        with self._state_lock:
            if self._stopping or self._exited.is_set():
                return
        detail = "closed by peer" if isinstance(exc, ConnectionClosed) else f"{type(exc).__name__}: {exc}"
        self.request_stop(DEFAULT_GRACE_MS, code=1, reason=f"{which} connection lost ({detail})")

    # -- control messages ----------------------------------------------------

    def _send_state(self, state: str, reason: str | None = None) -> None:
        msg: dict[str, Any] = {"type": "state", "state": state}
        if reason is not None:
            msg["reason"] = reason
        self._send_control_quiet(msg)

    def _send_control_quiet(self, msg: dict[str, Any]) -> None:
        try:
            self.control.write_control(msg)
        except (OSError, ValueError):
            pass

    def _control_loop(self) -> None:
        try:
            for msg in self._deferred:
                self._handle_control(msg)
            self._deferred.clear()
            while True:
                self._handle_control(self.control.read_control())
        except (ConnectionClosed, OSError) as exc:
            self._connection_lost("control", exc)
        except ProtocolError as exc:
            self._fail(f"control protocol error: {exc}")

    def _handle_control(self, msg: dict[str, Any]) -> None:
        kind = msg.get("type")
        if kind == "ping":
            self._send_control_quiet({"type": "pong", "n": msg.get("n")})
        elif kind == "clock_res":
            self._clock_q.put(msg)
        elif kind == "start":
            self._on_start_message()
        elif kind == "stop":
            grace = msg.get("grace_ms", DEFAULT_GRACE_MS)
            self.request_stop(int(grace) if isinstance(grace, (int, float)) else DEFAULT_GRACE_MS)
        elif kind == "set_param":
            name = msg.get("name")
            if not isinstance(name, str):
                self._send_control_quiet({"type": "param_ack", "name": str(name), "ok": False, "reason": "name must be a string"})
                return
            self._work.put(functools.partial(self._param_task, name, msg.get("value")))
        elif kind == "error":
            self._fail(f"protocol error from supervisor: {msg.get('code')}: {msg.get('msg')}")
        elif kind in ("config", "reject"):
            self._warn_once(("control", kind), f"unexpected {kind} message after handshake; ignored")
        # Anything else is a newer message type; ignoring it is the compatible choice (docs/03 §Versioning).

    def _on_start_message(self) -> None:
        with self._state_lock:
            if self._started:
                return
            self._started = True
        self._send_state("running")
        try:
            self.node._set_running()   # sends headers declared before start
        except (OSError, ValueError) as exc:
            self._fail(f"could not send stream header: {exc}")
            return
        self._work.put(self.node._dispatch_start)

    def _param_task(self, name: str, value: Any) -> None:
        ok, reason = self.node._dispatch_param(name, value)
        ack: dict[str, Any] = {"type": "param_ack", "name": name, "ok": ok}
        if reason is not None:
            ack["reason"] = reason
        self._send_control_quiet(ack)

    def _send_stats(self) -> None:
        self._send_control_quiet(self.node._stats(self.clock.now()))

    def _resync_clock(self) -> None:
        while True:   # drop stale responses from an earlier, abandoned round
            try:
                self._clock_q.get_nowait()
            except queue.Empty:
                break
        try:
            offset, rtt = _Clock.exchange(self.control.write_control, lambda: self._clock_q.get(timeout=1.0))
        except (queue.Empty, OSError, ProtocolError, ValueError):
            return   # keep the previous estimate
        self.clock.apply(offset, rtt)

    # -- data frames ---------------------------------------------------------

    def _data_loop(self) -> None:
        try:
            while True:
                header, payload = self.data.read_frame()
                self._handle_frame(header, payload)
        except (ConnectionClosed, OSError) as exc:
            self._connection_lost("data", exc)
        except ProtocolError as exc:
            self._fail(f"data protocol error: {exc}")

    def _handle_frame(self, fh: FrameHeader, payload: bytes) -> None:
        node = self.node
        if fh.version != PROTOCOL_VERSION:
            self._warn_once(("version", fh.version), f"dropping frame with protocol version {fh.version}")
            return
        if fh.type == IDENTIFY:
            return
        port = node._input_port(fh.port, fh.source)
        if port is None:
            self._warn_once(("port", fh.port), f"dropping frame for unknown input port index {fh.port}")
            return
        if port.source == "":
            self._warn_once(("source", fh.port, fh.source), f"frame on input {port.name!r} from source index {fh.source} not listed in config")
        if fh.type == STREAM_HEADER:
            header = decode_stream_header(payload)
            node._note_header(port, header)
            node._note_received(port, None)
            self._work.put(functools.partial(node._dispatch_header, port, header))
        elif fh.type == SIGNAL:
            cached = node._input_header(port)
            srate = cached.srate if isinstance(cached, SignalHeader) else None
            if srate is None:
                self._warn_once(("noheader", fh.port, fh.source), f"signal on input {port.name!r} before its stream header")
            chunk = decode_signal(payload, fh.flags, seq=fh.seq, srate=srate)
            node._note_received(port, chunk.t0)
            self._work.put(functools.partial(node._dispatch_signal, port, chunk))
        elif fh.type == EVENT:
            event = decode_event(payload, seq=fh.seq)
            node._note_received(port, event.t)
            self._work.put(functools.partial(node._dispatch_event, port, event))
        elif fh.type == END_STREAM:
            node._note_received(port, None)
            self._work.put(functools.partial(node._dispatch_end, port))
        # Reserved frame types are ignored.

    def _warn_once(self, key: tuple[Any, ...], msg: str) -> None:
        if key in self._warned:
            return
        self._warned.add(key)
        self.send_log("warn", msg, {})

    # -- worker ---------------------------------------------------------------

    def _init_task(self) -> None:
        self.node._dispatch_init()
        self._send_state("configured")
        self.control.write_control({"type": "ready"})
        self._send_state("ready")

    def _worker_loop(self) -> None:
        while True:
            task = self._work.get()
            if task is None:
                self._finish_stop()
                return
            try:
                task()
            except SystemExit:
                return   # fail() already reported the state
            except Exception as exc:  # noqa: BLE001
                self.crash(exc, "callback")
                return


# --------------------------------------------------------------------------- #
# run()
# --------------------------------------------------------------------------- #


@dataclass(frozen=True)
class _Env:
    session: str
    node_id: str
    control: str
    data: str
    protocol: str

    @classmethod
    def read(cls) -> _Env:
        values: dict[str, str] = {}
        for name in ENV_VARS:
            value = os.environ.get(name)
            if value is None or value == "":
                _die(2, f"missing environment variable {name}. A PSL node must be launched by the psl supervisor, "
                        "which sets PSL_SESSION, PSL_NODE_ID, PSL_CONTROL, PSL_DATA and PSL_PROTOCOL.")
            values[name] = value
        if values["PSL_PROTOCOL"].strip() != str(PROTOCOL_VERSION):
            _die(2, f"unsupported PSL_PROTOCOL {values['PSL_PROTOCOL']!r}; this SDK speaks version {PROTOCOL_VERSION}")
        return cls(
            session=values["PSL_SESSION"],
            node_id=values["PSL_NODE_ID"],
            control=values["PSL_CONTROL"],
            data=values["PSL_DATA"],
            protocol=values["PSL_PROTOCOL"],
        )


def _die(code: int, message: str) -> NoReturn:
    print(f"psl: {message}", file=sys.stderr, flush=True)
    sys.exit(code)


def manifest_hash(path: str | None = None) -> str:
    """SHA-256 hex of the node's manifest, or "" if absent.

    The supervisor starts nodes in the workspace root and names the package
    directory in PSL_NODE_DIR; without it the current directory is used.
    """
    if path is None:
        path = os.path.join(os.environ.get("PSL_NODE_DIR", "."), "node.psl")
    try:
        with open(path, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()
    except OSError:
        return ""


def _hello(node: Node, env: _Env) -> dict[str, Any]:
    msg: dict[str, Any] = {
        "type": "hello",
        "role": "node",
        "node_id": env.node_id,
        "protocol": PROTOCOL_VERSION,
        "pid": os.getpid(),
        "sdk": SDK_NAME,
        "manifest_hash": manifest_hash(),
    }
    dynamic = getattr(type(node), "dynamic_outputs", None)
    if isinstance(dynamic, list):
        msg["outputs"] = [{"name": str(o["name"]), "type": str(o["type"])} for o in dynamic]
    return msg


def run(node_class: type[Node]) -> NoReturn:
    """Run ``node_class`` as a PSL node process. Never returns.

    Exit codes: 0 clean stop, 1 failure, 2 bad environment, 3 rejected by the
    supervisor.
    """
    if not (isinstance(node_class, type) and issubclass(node_class, Node)):
        _die(2, "psl.run() needs a subclass of psl.Node")
    env = _Env.read()
    node = node_class()
    node.node_id = env.node_id
    node.session = env.session

    try:
        control = Connection(connect(env.control))
    except OSError as exc:
        _die(1, f"cannot connect to control socket {env.control!r}: {exc}")

    deferred: list[dict[str, Any]] = []
    try:
        control.write_control(_hello(node, env))
        reply = control.read_control()
        if reply.get("type") == "reject":
            _die(3, f"rejected by supervisor: {reply.get('reason', 'no reason given')}")
        if reply.get("type") != "config":
            _die(1, f"expected config after hello, got {reply.get('type')!r}")
        node._configure(reply)

        try:
            data = Connection(connect(env.data))
        except OSError as exc:
            _die(1, f"cannot connect to data socket {env.data!r}: {exc}")
        data.write_frame(identify_frame(env.node_id, env.session))

        def handshake_recv() -> dict[str, Any]:
            while True:
                msg = control.read_control()
                kind = msg.get("type")
                if kind == "clock_res":
                    return msg
                if kind == "ping":
                    control.write_control({"type": "pong", "n": msg.get("n")})
                else:
                    deferred.append(msg)   # start/stop/set_param that arrived early

        clock = _Clock()
        offset, rtt = _Clock.exchange(control.write_control, handshake_recv)
        clock.apply(offset, rtt)
    except (ConnectionClosed, OSError, ProtocolError, ValueError, KeyError) as exc:
        _die(1, f"handshake failed: {type(exc).__name__}: {exc}")

    runtime = _SocketRuntime(node, control, data, clock, deferred)
    node._attach(runtime)

    def on_signal(signum: int, _frame: Any) -> None:
        try:
            name = signal.Signals(signum).name
        except ValueError:
            name = str(signum)
        runtime.request_stop(DEFAULT_GRACE_MS, reason=f"received {name}")

    for sig in (signal.SIGTERM, signal.SIGINT):
        try:
            signal.signal(sig, on_signal)
        except (ValueError, OSError):
            pass   # not the main thread, or unsupported platform

    runtime.start()
    code = runtime.wait()
    control.close()
    data.close()
    sys.stderr.flush()
    sys.stdout.flush()
    sys.exit(code)
