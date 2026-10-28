# 03 — Wire protocol (version 1)

The protocol is designed so that a node can be written in any language with
nothing beyond sockets, byte packing and a JSON codec. No library is required
or assumed. All integers are **little-endian**. All text is **UTF-8**.

A client opens two stream connections to the supervisor: one to `PSL_CONTROL`
and one to `PSL_DATA`. On macOS and Linux these are Unix domain socket paths;
on Windows they are `127.0.0.1:<port>`. The client's first message on each
connection must be `hello` (control) or `identify` (data) so the supervisor
can pair the two.

## Control plane

### Framing

```
u32  length      number of bytes that follow
u8[] body        a UTF-8 JSON object with a "type" field
```

Maximum body length is 16 MiB. Messages are processed in order per
connection.

### Messages from client to supervisor

| `type` | Fields | Meaning |
|--------|--------|---------|
| `hello` | `role` (`"node"` or `"tool"`), `node_id`, `protocol` (1), `pid`, `sdk` (free text), `manifest_hash` (hex, nodes only), `outputs` (nodes with `dynamic_outputs` only: list of `{name, type}`) | First message. Supervisor answers `config` or `reject`. |
| `ready` | — | Handshake and clock sync done; the node can accept `start`. |
| `state` | `state` (`configured`, `ready`, `running`, `stopping`, `stopped`, `failed`), `reason` (optional) | Node reports a lifecycle transition. |
| `clock_req` | `t1` (node local monotonic ns) | Clock sync request. |
| `param_ack` | `name`, `ok` (bool), `reason` (optional) | Reply to `set_param`. |
| `stats` | `sent`, `received`, `dropped_local`, `latency_ns` (per input port) | Sent every second by the SDK. |
| `log` | `level` (`debug`, `info`, `warn`, `error`), `msg`, `fields` (optional object) | Forwarded log line. |
| `subscribe` | `stream` | Tools only. Receive this stream's frames on the data connection. |
| `unsubscribe` | `stream` | Tools only. |
| `set_param` | `node_id`, `name`, `value` | Tools only. Supervisor forwards to the node if the param is `mutable`. |
| `record_start` / `record_stop` | `path` (optional) | Tools only. |
| `stop_pipeline` | — | Tools only. |
| `query` | `what` (`graph`, `nodes`, `streams`, `stats`) | Tools only. Supervisor answers `result`. |

### Messages from supervisor to client

| `type` | Fields | Meaning |
|--------|--------|---------|
| `config` | `session`, `node_id`, `params` (object), `inputs` (list), `outputs` (list), `options` (object), `wall_start_ns` | Reply to `hello`. Shape below. |
| `reject` | `reason` | Reply to `hello` when the protocol version, node id or manifest hash is wrong. The connection is then closed. |
| `clock_res` | `t1`, `t2` (supervisor session ns at receipt), `t3` (supervisor session ns at send) | Reply to `clock_req`. |
| `start` | — | Begin publishing / processing. |
| `stop` | `grace_ms` | Finish in-flight work and exit. |
| `set_param` | `name`, `value` | Change a mutable parameter. Node replies `param_ack`. |
| `ping` | `n` | Node replies `pong` with the same `n`. |
| `result` | `query`, `data` | Reply to `query`. |
| `subscribed` | `stream`, `index` | Reply to `subscribe`: the tool will see this stream's frames with `port = index`. |
| `error` | `code`, `msg` | Protocol violation or a failed tool request (e.g. `unknown_stream`). A violation closes the connection. |

`config` shape:

```json
{
  "type": "config",
  "session": "20261028-143012-7f3a",
  "node_id": "bp",
  "wall_start_ns": 1793196612000000000,
  "params": {"low": 0.5, "high": 40.0, "order": 4},
  "inputs": [
    {"name": "in", "index": 0, "type": "signal",
     "sources": [{"index": 0, "stream": "polar.ecg"}]}
  ],
  "outputs": [
    {"name": "out", "index": 0, "type": "signal", "stream": "bp.out"}
  ],
  "options": {"queue": 64, "on_full": "drop_oldest"}
}
```

Port indices are assigned by the supervisor from the manifest's declaration
order and are stable for the session. Source indices are per input port.

### Clock sync

Session time is nanoseconds since session start on the supervisor's
monotonic clock. A node computes its offset with the standard four-timestamp
exchange:

```
node:        t1 = local_monotonic_ns();  send clock_req{t1}
supervisor:  t2 = session_ns() on receipt; t3 = session_ns() on send; reply clock_res{t1,t2,t3}
node:        t4 = local_monotonic_ns() on receipt
             rtt    = (t4 - t1) - (t3 - t2)
             offset = ((t2 - t1) + (t3 - t4)) / 2
```

The SDK performs eight exchanges during handshake, keeps the offset from the
exchange with the smallest `rtt`, and defines `now() = local_monotonic_ns() +
offset`. It repeats the exchange every ten seconds and applies changes
gradually (never stepping backwards) so timestamps stay monotonic. On one
machine `rtt` is a few microseconds and the offset is effectively exact;
the same code works unchanged for a remote node.

## Data plane

### Frame header

Every data frame begins with a 16-byte header:

```
offset  size  field     meaning
0       4     length    total frame length in bytes, including this header
4       1     version   protocol version, 1
5       1     type      frame type (table below)
6       2     flags     type-specific flags
8       2     port      client→supervisor: output port index
                        supervisor→client: input port index
10      2     source    client→supervisor: ignored, write 0
                        supervisor→client: index into the input port's source list
12      4     seq       per-stream sequence number, starts at 0, wraps at 2^32
```

`seq` counts every frame published on a stream, of any type: headers,
chunks, events and end-of-stream share one counter, so a gap always means a
lost frame.

Maximum frame length is 64 MiB. The supervisor forwards frames without
inspecting the payload; it rewrites only `port` and `source`.

### Frame types

| Value | Name | Payload |
|-------|------|---------|
| `0x00` | `IDENTIFY` | UTF-8 JSON `{"node_id": "...", "session": "..."}`. First frame on the data connection, pairs it with the control connection. `port`, `source`, `seq` are 0. |
| `0x01` | `STREAM_HEADER` | UTF-8 JSON stream header (02 §Stream header). |
| `0x02` | `SIGNAL` | Signal chunk, below. |
| `0x03` | `EVENT` | Event, below. |
| `0x04` | `END_STREAM` | Empty. |

Values `0x05`–`0x7F` are reserved for future standard types. `0x80`–`0xFF`
are reserved for experiments and are never forwarded by a released
supervisor.

### `SIGNAL` payload

```
offset  size  field        meaning
0       8     t0           i64 session ns of first sample
8       8     t_device     i64 device clock of first sample, or 0
16      4     n_samples    u32
20      2     n_channels   u16
22      1     dtype        0 = f32, 1 = f64, 2 = i16, 3 = i32
23      1     reserved     0
24      —     samples      n_samples × n_channels × sizeof(dtype) bytes, sample-major
        —     timestamps   only if flags & 0x0001: n_samples × i64 session ns
```

Flags: bit 0 `HAS_TIMESTAMPS`. Other bits reserved, must be 0.

### `EVENT` payload

```
offset  size  field        meaning
0       8     t            i64 session ns
8       8     t_device     i64 or 0
16      —     body         UTF-8 JSON {"label": "...", "data": <any>}
```

## Handshake, end to end

```
client                                  supervisor
  │── control: hello ───────────────────▶│
  │◀─ control: config ───────────────────│
  │── data: IDENTIFY ───────────────────▶│
  │── control: clock_req ×8 ────────────▶│
  │◀─ control: clock_res ×8 ─────────────│
  │── control: ready ───────────────────▶│
  │◀─ data: STREAM_HEADER for each ──────│  (cached headers for inputs, if any producer is already up)
  │◀─ control: start ────────────────────│
  │── data: STREAM_HEADER (outputs) ────▶│
  │── data: SIGNAL / EVENT ... ─────────▶│
  │◀─ data: SIGNAL / EVENT ... ──────────│
  │◀─ control: ping ─────────────────────│  every 1 s
  │── control: pong ────────────────────▶│
  │── control: stats ───────────────────▶│  every 1 s
  │◀─ control: stop ─────────────────────│
  │── control: state{stopped} ──────────▶│
```

## Recording file (`.pslrec`, version 1)

```
u8[8]   magic        "PSLREC\0\x01"
u32     header_len
u8[]    header       UTF-8 JSON (below)
records, repeated until EOF:
  i64   t_record     session ns at which the hub forwarded the frame
  u16   stream       index into header.streams
  u16   reserved     0
  u8[]  frame        a complete data frame including its 16-byte header,
                     with port = the producer's output port index and source = 0
```

Header JSON:

```json
{
  "version": 1,
  "session": "20261028-143012-7f3a",
  "wall_start_ns": 1793196612000000000,
  "pipeline": "pipelines/main.psl",
  "graph": { "...canonical graph JSON from 04..." },
  "streams": [
    {"index": 0, "stream": "polar.ecg", "node": "polar", "port": "ecg", "type": "signal"},
    {"index": 1, "stream": "bp.out",    "node": "bp",    "port": "out", "type": "signal"}
  ]
}
```

Because the frames are stored verbatim, replay is a loop that reads a record,
waits until its `t_record` relative to replay start, rewrites `t0`/`t`
by the replay offset, and publishes. `psl check` reads only the header to
learn which streams a replay node will offer.

## Versioning

- `PSL_PROTOCOL` and the header `version` byte identify the protocol. A
  supervisor rejects a client whose version it does not support, with a
  `reject` naming both versions.
- Adding a control message type, an optional JSON field, a new frame type or
  a new flag bit is backwards compatible and does not bump the version.
- Changing the meaning or layout of an existing field bumps the version and
  requires an ADR.
