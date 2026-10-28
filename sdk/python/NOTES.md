# Python SDK — implementation notes

Decisions taken where docs/02, 03 and 05 are silent, loose or open to more
than one reading. Each one should be reconciled with the Rust core; when the
docs are amended these notes can be deleted.

## Wire protocol and control plane

1. **Sequence numbers.** One counter per output stream, starting at 0,
   incremented by every frame of any type on that stream: `STREAM_HEADER`,
   `SIGNAL`, `EVENT` and `END_STREAM` all take the next number (docs/02 and
   docs/03 now say this explicitly). The counter is not reset by `end()` +
   re-`declare_output()`; it runs for the whole session. A consumer must not
   expect the first `SIGNAL` to carry seq 0 (it carries 1 when a header
   preceded it).
2. **`stats` counts every frame.** `sent[<output>]` and `received[<input>]`
   count all frames on that port including headers and end-of-stream (the
   same frames the sequence numbers count). `latency_ns[<input>]` is
   `now() - t0` of the last chunk (or `now() - t` of the last event) received
   on that port, across all its sources, or 0 if nothing was received yet.
   `dropped_local` is always 0: the SDK's worker queue is unbounded; the only
   bounded queues are in the supervisor.
3. **Clock monotonicity by clamping.** `now()` returns
   `monotonic_ns() + offset` clamped to never be lower than the previous
   value returned. If a re-sync (every 10 s, 8 exchanges, smallest rtt wins)
   lowers the offset by X ns, `now()` stands still for X ns of real time
   instead of slewing gradually. On one machine X is microseconds.
4. **`clock_res` matching.** A response is accepted when its `t1` equals the
   `t1` just sent (or `t1` is absent). Stale `clock_res` messages are skipped;
   more than 16 in one round is a protocol error. During the handshake the
   exchange is run on the main thread: `ping` arriving then is answered
   immediately, and `start`/`stop`/`set_param` arriving before `ready` are
   queued and processed in order once the control reader starts.
5. **Nothing before `start`, including headers.** `declare_output` before
   `start` stores the header and sends it when `start` arrives (before
   `on_start` runs). `emit` or `end` before `start` raise `RuntimeError`. This
   matters for modules that receive a cached input header (docs/03 handshake
   diagram) before their `start`.
6. **Header required before data.** `emit` on an output with no current header
   (never declared, or ended and not re-declared) raises `RuntimeError`.
7. **`emit` converts.** Chunk data is converted to the declared header's dtype
   (little-endian, C-contiguous) rather than rejected; a channel count that
   differs from the header raises `ValueError`. Timestamps become int64.
8. **`param_ack`.** `ok: false` always carries `reason` (`"rejected by node"`
   when `on_param` returned a falsy value; `"name must be a string"` for a
   malformed request). An exception raised by `on_param` is treated like any
   other callback exception: log with traceback, `state{failed}`, exit 1.
9. **`pong` echoes `n` as received** (JSON null if the ping had none).
10. **`log` always carries `fields`** (an object, possibly empty). Values that
    are not JSON-serialisable (numpy scalars, arrays, anything else) are
    converted (`.item()`, `.tolist()`, `str()`). Nothing is mirrored to
    stderr: the supervisor writes every forwarded `log` message to
    `logs/<node-id>.log` and echoes `info` and above to its terminal, and the
    node's own stderr goes to the same file, so a mirror would duplicate lines.
    Resolved 2026-10-28 by removing the mirror from the SDK.
11. **`hello.outputs`** is sent only when the `Node` subclass has a class
    attribute `dynamic_outputs` that is a list of `{"name", "type"}`.
    `manifest_hash` is the SHA-256 of the raw bytes of `$PSL_NODE_DIR/node.psl` (or `./node.psl` without that variable), or `""`
    if the file is absent. The `sdk` string is `psl-python/0.1.0`.
12. **Unknown control message types are ignored** (docs/03 §Versioning says
    adding one is compatible). `config`/`reject` after the handshake are
    logged as a warning and ignored. `error{code,msg}` from the supervisor
    makes the node send `state{failed}` and exit 1.
13. **Incoming frames.** A frame whose `version` byte is not 1 is dropped
    (warned once). `IDENTIFY` and reserved types (0x05-0xFF) are ignored.
    A frame for an input port index not in `config` is dropped (warned once).
    A frame whose `source` index is not in the port's source list is
    delivered with `Port.source == ""` (warned once) rather than dropped.
    A `SIGNAL` before any `STREAM_HEADER` on that (port, source) is delivered
    with `chunk.srate = None` (warned once); `time_of` then raises unless the
    chunk has per-sample timestamps. A malformed payload is a protocol error:
    the node fails.
14. **Stop and exit codes.** On `stop{grace_ms}` or SIGTERM/SIGINT (grace
    5000 ms): `state{stopping}` immediately, timers stop, the worker drains
    everything already queued, `on_stop` runs on the worker, `state{stopped}`,
    sockets close, exit 0. If that takes longer than the grace period the
    node sends `state{failed, "stop grace period ... exceeded"}` and exits 1.
    Unexpected EOF on either socket (supervisor gone) runs the same graceful
    path but exits 1. `reject` exits 3; a missing or empty `PSL_*` variable,
    or `PSL_PROTOCOL` other than `1`, exits 2 with a message on stderr; a
    failed connect or handshake exits 1. Exceptions in `on_stop` fail the
    node (exit 1) instead of reporting `stopped`.
15. **Ordering on start.** `state{running}` is sent, then pending headers,
    then `on_start` is queued. `state{configured}` is sent after `on_init`
    returns, then `ready`, then `state{ready}`. The core (as of this writing)
    acts on `ready` and ignores the `configured`/`ready` states, which is
    fine; both are sent because docs/03 lists them as lifecycle states.
16. **`STREAM_HEADER` JSON** written by the SDK always includes `source` and
    `meta` (possibly `{}`) and every channel's `unit` and `modality`. When
    reading, missing `dtype`/`source`/`meta`/`unit`/`modality` take their
    documented defaults; `type`, `srate` and `channels` are required for a
    signal header. Channel entries given as bare strings are accepted as
    labels.
17. **Control `set_param` from the supervisor** carries `name` and `value`
    only (the tool form with `node_id` is not expected on a node connection;
    an extra `node_id` field is ignored).

## Values

18. **`Chunk.srate`.** docs/05 lists `time_of(i)` on `Chunk`, and docs/02 says
    sample `i` is at `t0 + i / srate` when there are no per-sample
    timestamps, but the chunk on the wire carries no rate. `Chunk` therefore
    has an extra field `srate: float | None = None` that is not on the wire.
    The SDK fills it from the cached stream header of the (port, source) the
    chunk arrived on, and on emitted chunks from the declared output header;
    `replace()` preserves it. `Chunk.times()` is a vectorised `time_of` and
    `duration_ns` is `n_samples / srate`; both are additions.
19. `Chunk` accepts 1-D data as one channel and 0-D as one sample; anything
    other than 2-D after that raises. `Chunk` is not hashable/comparable
    (numpy arrays), `Event` is a plain comparable dataclass.
20. `SignalHeader.type` / `EventHeader.type` are class attributes
    (`"signal"` / `"event"`) mirroring the JSON field.

## Runtime behaviour not specified in the docs

21. **Timers.** `every` is fixed-rate (next deadline = previous deadline +
    period). If the scheduler falls more than one period behind, missed
    ticks are skipped rather than fired in a burst. If a timer's previous
    callback is still waiting on the worker queue, the new tick is coalesced
    (not queued again). `after` is one-shot. `cancel()` is immediate; a
    callback already queued does not run after cancellation.
22. **Clock re-sync runs on the scheduler thread** and waits up to 1 s per
    exchange for `clock_res`; an unanswered exchange keeps the old offset.
    While it waits, user timers on the scheduler are delayed (only when the
    supervisor is unresponsive).
23. **`spawn`** threads are daemon threads; an uncaught exception in one is
    logged with its traceback and fails the node, like a callback exception.
24. **`fail()`** sends `state{failed, reason}`, marks exit code 1 and raises
    `SystemExit(1)` in the calling thread (on the worker this ends dispatch;
    on a spawned thread it ends that thread). The main thread then closes the
    sockets and exits.
25. **`on_init` receives a copy** of `params`; `self.params` is updated in
    place by accepted `set_param` calls.
26. **Dynamic constraint checking is not implemented.** docs/02 §Type checking
    says the SDK checks an incoming header against the port's manifest
    constraints. That needs a PSL manifest parser in Python, which this SDK
    does not have (it only hashes `node.psl`). Either the supervisor performs
    this check (it has the parsed manifest) or a later SDK version adds it.
27. **Manifest defaults are not applied by the SDK**; `config.params` from the
    supervisor is trusted to be complete (docs/04 canonical model says
    defaults are filled in). The first-party nodes still use `params.get(...)`
    with the manifest default as a fallback.

## Test harness (`psl.testing.Harness`)

28. `Harness(NodeClass, params)` does not read `node.psl`; the caller passes
    complete params. Input ports are created on first use with the type of
    the header/chunk/event pushed; output ports are created by
    `declare_output` (type from the header). Optional `inputs=`/`outputs=`
    lists of `(name, type)` fix indices and types up front. `Port.source` is
    `test.<port>` for source index 0 and `test.<port>.<i>` otherwise.
29. `start()` is implied by the first input push or `tick()` so the docs'
    example (header then signal, no explicit start) works. `tick()` runs every
    `every()` timer once; `run_after()` runs pending `after()` timers once.
    The clock is virtual: `now_ns` starts at 0 and moves with `advance(s)`.
30. Everything pushed in or emitted out is round-tripped through the wire
    codec, so the node sees exactly what it would see from a socket (writable
    arrays, stream dtype, `srate` attached) and emitted data is proven
    encodable. `outputs(name)` returns the decoded copies with `seq` assigned.
31. `fail()` raises `psl.NodeFailed(reason)`; an exception from `spawn`ed
    threads is recorded in `h.crashes`; callback exceptions propagate to the
    test. `h.frames` lists `(output, frame type, seq)` in emission order;
    `h.ended(name)` counts end-of-stream frames.
32. `replay(path, map)` reads a `.pslrec` as specified in docs/03 and feeds
    mapped streams; each mapped stream gets its own source index on the
    target port and `Port.source` is the recorded stream id.

## First-party nodes

33. `psl/moving-average` and `psl/scale` promote integer input dtypes
    (`i16`, `i32`) to `f32` on their output header; float headers are copied
    unchanged. Both declare `multiple = false` on `in` and propagate
    end-of-stream. A live parameter change re-announces the output header
    (its `meta` carries the parameter values) before the next chunk.
34. `psl/synthetic` emits `srate * chunk_ms / 1000` samples per tick with the
    fractional remainder carried over, `t0 = now() - count / srate`. The ECG
    is a sum of Gaussians (P, Q, R, S, T) with the beat phase accumulated per
    sample so a live `bpm` change is continuous. Sine channels are shifted a
    quarter cycle each; ECG channels are identical; noise channels are
    independent. `source.timing` documents the stamping rule.
35. `psl/csv-writer` opens the file in `on_init`, writes the header row on
    `on_header` only when the labels differ from the last row written (and
    never on the first header when appending to a non-empty file), formats
    values with numpy's shortest round-trip `str()` of the stream dtype, and
    flushes on `on_end` and `on_stop`.
36. `psl/print` counts chunks per (port, source); the first chunk and then
    every `every`-th is logged; the structured `fields` carry `source`, `seq`,
    `t0` and the shape in addition to the text line.
