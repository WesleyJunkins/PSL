"""Offline testing: drive a node with no supervisor and no sockets.

``Harness`` constructs the node, applies the parameters you pass (it does not
read ``node.psl``; pass complete params), calls ``on_init`` and then lets a
test push headers, chunks and events into input ports and read back what the
node emitted. Callbacks run synchronously on the calling thread, in order,
exactly as the worker thread would run them.

Everything that crosses an input or output port is round-tripped through the
wire codec, so a test also proves the node's data is encodable.
"""

from __future__ import annotations

import json
import struct
import traceback
from typing import Any, NoReturn

import numpy as np

from ._node import NodeFailed, Node, Timer, _OutputState
from ._protocol import (
    END_STREAM,
    EVENT,
    HEADER_SIZE,
    SIGNAL,
    STREAM_HEADER,
    FrameHeader,
    decode_event,
    decode_signal,
    decode_stream_header,
    encode_event,
    encode_signal,
    encode_stream_header,
)
from ._values import NS_PER_S, Chunk, Event, EventHeader, Header, Port, SignalHeader, _dtype_name_of

__all__ = ["Harness", "NodeFailed"]

_PSLREC_MAGIC = b"PSLREC\x00\x01"


class _HarnessRuntime:
    """In-process ``Runtime``: records everything the node sends."""

    def __init__(self, harness: Harness) -> None:
        self._h = harness

    def now(self) -> int:
        return self._h.now_ns

    def send_header(self, out: _OutputState, header: Header, seq: int) -> None:
        copy = decode_stream_header(encode_stream_header(header))
        self._h._headers.setdefault(out.name, []).append(copy)
        self._h.frames.append((out.name, STREAM_HEADER, seq))

    def send_chunk(self, out: _OutputState, chunk: Chunk) -> None:
        dtype = out.header.dtype if isinstance(out.header, SignalHeader) else "f32"
        payload, flags = encode_signal(chunk, dtype)
        copy = decode_signal(payload, flags, seq=chunk.seq, srate=chunk.srate)
        self._h._emitted.setdefault(out.name, []).append(copy)
        self._h.frames.append((out.name, SIGNAL, chunk.seq))

    def send_event(self, out: _OutputState, event: Event) -> None:
        copy = decode_event(encode_event(event), seq=event.seq)
        self._h._emitted.setdefault(out.name, []).append(copy)
        self._h.frames.append((out.name, EVENT, event.seq))

    def send_end(self, out: _OutputState, seq: int) -> None:
        self._h._ended[out.name] = self._h._ended.get(out.name, 0) + 1
        self._h.frames.append((out.name, END_STREAM, seq))

    def send_log(self, level: str, msg: str, fields: dict[str, Any]) -> None:
        self._h.logs.append((level, msg))
        self._h.log_records.append((level, msg, dict(fields)))

    def add_timer(self, timer: Timer) -> None:
        self._h.timers.append(timer)

    def fail(self, reason: str) -> NoReturn:
        self._h.failed = reason
        raise NodeFailed(reason)

    def unknown_output(self, node: Node, name: str, port_type: str) -> _OutputState | None:
        index = len(node._out_by_name)
        node.outputs.append({"name": name, "index": index, "type": port_type, "stream": f"{node.node_id}.{name}"})
        return node._add_output(name, index, port_type, f"{node.node_id}.{name}")

    def crash(self, exc: BaseException, where: str) -> None:
        self._h.failed = f"{type(exc).__name__}: {exc}"
        self._h.crashes.append((where, exc))
        traceback.print_exception(exc)


class Harness:
    """Exercise a ``psl.Node`` subclass in-process.

    >>> h = Harness(MovingAverage, params={"window": 5})
    >>> h.header("in", psl.SignalHeader(srate=100, channels=[psl.Channel("A")]))
    >>> h.signal("in", psl.Chunk(t0=0, data=np.ones((10, 1), np.float32)))
    >>> h.outputs("out")[0].data[:, 0]

    ``start()`` is implied by the first input or ``tick()`` if it was not
    called explicitly, so a module test can push a header straight away.
    Time is virtual: ``now_ns`` starts at 0 and moves only with ``advance``.
    Input and output ports are created on first use (the harness does not read
    the manifest); pass ``inputs``/``outputs`` as ``[(name, type), ...]`` to
    fix their indices and types up front.
    """

    def __init__(
        self,
        node_class: type[Node],
        params: dict[str, Any] | None = None,
        *,
        inputs: list[tuple[str, str]] | None = None,
        outputs: list[tuple[str, str]] | None = None,
        node_id: str = "test",
        session: str = "test-session",
    ) -> None:
        self.node: Node = node_class()
        self.now_ns: int = 0
        self.logs: list[tuple[str, str]] = []
        self.log_records: list[tuple[str, str, dict[str, Any]]] = []
        self.frames: list[tuple[str, int, int]] = []   # (output name, frame type, seq) in emission order
        self.timers: list[Timer] = []
        self.crashes: list[tuple[str, BaseException]] = []
        self.failed: str | None = None
        self.started = False
        self.stopped = False
        self._emitted: dict[str, list[Chunk | Event]] = {}
        self._headers: dict[str, list[Header]] = {}
        self._ended: dict[str, int] = {}
        self._rt = _HarnessRuntime(self)
        self.node._configure({
            "session": session,
            "node_id": node_id,
            "wall_start_ns": 0,
            "params": dict(params or {}),
            "inputs": [
                {"name": n, "index": i, "type": t, "sources": [{"index": 0, "stream": f"test.{n}"}]}
                for i, (n, t) in enumerate(inputs or [])
            ],
            "outputs": [
                {"name": n, "index": i, "type": t, "stream": f"{node_id}.{n}"}
                for i, (n, t) in enumerate(outputs or [])
            ],
            "options": {},
        })
        self.node._attach(self._rt)
        self.node._dispatch_init()

    # -- time ---------------------------------------------------------------

    def advance(self, seconds: float) -> None:
        """Move the virtual session clock forward."""
        if seconds < 0:
            raise ValueError("cannot move time backwards")
        self.now_ns += round(seconds * NS_PER_S)

    # -- lifecycle ----------------------------------------------------------

    def start(self) -> None:
        if self.started:
            return
        self.started = True
        self.node._set_running()
        self.node._dispatch_start()

    def stop(self) -> None:
        if self.stopped:
            return
        self.stopped = True
        self.node._dispatch_stop()

    def set_param(self, name: str, value: Any) -> bool:
        ok, _reason = self.node._dispatch_param(name, value)
        return ok

    def tick(self) -> None:
        """Run every ``every()`` timer once, in registration order."""
        self.start()
        for timer in list(self.timers):
            if timer.repeat and not timer.cancelled:
                timer._run()

    def run_after(self) -> None:
        """Run every pending ``after()`` timer once and discard it."""
        self.start()
        for timer in list(self.timers):
            if not timer.repeat:
                self.timers.remove(timer)
                if not timer.cancelled:
                    timer._run()

    # -- inputs -------------------------------------------------------------

    def header(self, port_name: str, header: Header, *, source: int = 0) -> None:
        if not isinstance(header, (SignalHeader, EventHeader)):
            raise TypeError("header must be a SignalHeader or EventHeader")
        self.start()
        port = self._input(port_name, header.type, source)
        copy = decode_stream_header(encode_stream_header(header))
        self.node._note_header(port, copy)
        self.node._note_received(port, None)
        self.node._dispatch_header(port, copy)

    def signal(self, port_name: str, chunk: Chunk, *, source: int = 0) -> None:
        self.start()
        port = self._input(port_name, "signal", source)
        cached = self.node._input_header(port)
        if isinstance(cached, SignalHeader):
            dtype, srate = cached.dtype, cached.srate
        else:
            dtype, srate = _dtype_name_of(chunk.data) or "f32", chunk.srate
        payload, flags = encode_signal(chunk, dtype)
        copy = decode_signal(payload, flags, seq=chunk.seq, srate=srate)
        self.node._note_received(port, copy.t0)
        self.node._dispatch_signal(port, copy)

    def event(self, port_name: str, event: Event, *, source: int = 0) -> None:
        self.start()
        port = self._input(port_name, "event", source)
        copy = decode_event(encode_event(event), seq=event.seq)
        self.node._note_received(port, copy.t)
        self.node._dispatch_event(port, copy)

    def end(self, port_name: str, *, source: int = 0) -> None:
        self.start()
        port = self._input(port_name, None, source)
        self.node._note_received(port, None)
        self.node._dispatch_end(port)

    def replay(self, path: str, map: dict[str, str]) -> int:
        """Feed a ``.pslrec`` recording: ``map`` is ``{stream id: input port name}``.

        Frames of unmapped streams are skipped. Returns the number of frames
        delivered. Each mapped stream gets its own source index on the port and
        the ``Port.source`` seen by the node is the recorded stream id.
        """
        self.start()
        with open(path, "rb") as f:
            blob = f.read()
        if blob[:8] != _PSLREC_MAGIC:
            raise ValueError(f"{path} is not a PSLREC version 1 file")
        (header_len,) = struct.unpack_from("<I", blob, 8)
        header = json.loads(blob[12:12 + header_len].decode("utf-8"))
        streams = {int(s["index"]): str(s["stream"]) for s in header.get("streams", [])}
        sources: dict[tuple[str, str], int] = {}
        pos = 12 + header_len
        delivered = 0
        while pos < len(blob):
            _t_record, stream_index, _reserved = struct.unpack_from("<qHH", blob, pos)
            pos += 12
            fh = FrameHeader.unpack(blob[pos:pos + HEADER_SIZE])
            fh.validate()
            payload = blob[pos + HEADER_SIZE:pos + fh.length]
            pos += fh.length
            stream = streams.get(stream_index)
            if stream is None or stream not in map:
                continue
            port_name = map[stream]
            key = (port_name, stream)
            if key not in sources:
                sources[key] = len([k for k in sources if k[0] == port_name])
            source = sources[key]
            if fh.type == STREAM_HEADER:
                hdr = decode_stream_header(payload)
                port = self._input(port_name, hdr.type, source, stream=stream)
                self.node._note_header(port, hdr)
                self.node._note_received(port, None)
                self.node._dispatch_header(port, hdr)
            elif fh.type == SIGNAL:
                port = self._input(port_name, "signal", source, stream=stream)
                cached = self.node._input_header(port)
                srate = cached.srate if isinstance(cached, SignalHeader) else None
                chunk = decode_signal(payload, fh.flags, seq=fh.seq, srate=srate)
                self.node._note_received(port, chunk.t0)
                self.node._dispatch_signal(port, chunk)
            elif fh.type == EVENT:
                port = self._input(port_name, "event", source, stream=stream)
                event = decode_event(payload, seq=fh.seq)
                self.node._note_received(port, event.t)
                self.node._dispatch_event(port, event)
            elif fh.type == END_STREAM:
                port = self._input(port_name, None, source, stream=stream)
                self.node._note_received(port, None)
                self.node._dispatch_end(port)
            else:
                continue
            delivered += 1
        return delivered

    # -- outputs ------------------------------------------------------------

    def outputs(self, port_name: str) -> list[Chunk | Event]:
        """Everything emitted on ``port_name`` so far, in order."""
        return list(self._emitted.get(port_name, []))

    def headers(self, port_name: str) -> list[Header]:
        """Every header declared on ``port_name`` so far, in order."""
        return list(self._headers.get(port_name, []))

    def ended(self, port_name: str) -> int:
        """How many times ``end(port_name)`` was called."""
        return self._ended.get(port_name, 0)

    def clear(self) -> None:
        """Forget recorded outputs, headers, frames and logs (ports and state stay)."""
        self._emitted.clear()
        self._headers.clear()
        self._ended.clear()
        self.frames.clear()
        self.logs.clear()
        self.log_records.clear()

    # -- internals ------------------------------------------------------------

    def _input(self, port_name: str, port_type: str | None, source: int, *, stream: str | None = None) -> Port:
        state = self.node._in_by_name.get(port_name)
        if state is None:
            if port_type is None:
                raise ValueError(f"input port {port_name!r} has not received a header yet")
            index = len(self.node._in_by_index)
            state = self.node._add_input(port_name, index, port_type, [])
            self.node.inputs.append({"name": port_name, "index": index, "type": port_type, "sources": []})
        elif port_type is not None and state.type != port_type:
            raise ValueError(f"input port {port_name!r} is a {state.type} port; cannot feed a {port_type}")
        if source not in state.ports:
            if stream is None:
                stream = f"test.{port_name}" if source == 0 else f"test.{port_name}.{source}"
            state.sources[source] = stream
            state.ports[source] = Port(port_name, state.index, state.type, source=stream, source_index=source)
        return state.ports[source]
