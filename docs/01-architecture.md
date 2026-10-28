# 01 — Architecture

## Processes

A running pipeline is a set of operating-system processes on one machine:

```
                 ┌──────────────────────────────────────────┐
  psl run ─────▶ │  supervisor (hub)                        │
                 │  • parses + checks the pipeline          │
                 │  • owns the session directory + sockets  │
                 │  • spawns node processes                 │
                 │  • routes data frames between nodes      │
                 │  • monitors heartbeats, restarts, stats  │
                 └───┬──────────┬──────────┬──────────┬─────┘
                     │          │          │          │
              control+data  control+data  control+data  control+data
                     │          │          │          │
                ┌────▼───┐ ┌────▼───┐ ┌────▼────┐ ┌───▼──────┐
                │ polar  │ │ bp     │ │ csv     │ │ psl echo │
                │ input  │ │ module │ │ output  │ │ (tool)   │
                └────────┘ └────────┘ └─────────┘ └──────────┘
```

- **Supervisor.** One per session. Started by `psl run`. Written in Rust, in
  the `psl` binary. It is also the hub: every data frame passes through it.
- **Node processes.** One per node declared in the pipeline. Started by the
  supervisor with the command from the node's manifest. A node knows nothing
  about the pipeline; it learns its parameters, its input sources and its
  output streams from the supervisor at handshake.
- **Tool clients.** `psl echo`, `psl ps`, `psl record` and later PhysioLab
  connect to a running session's sockets exactly like a node does, with role
  `tool`. They can subscribe to any stream and send control requests.

### Why a hub

A central hub costs one extra local hop per frame, measured in microseconds.
In return the supervisor sees every stream, which gives recording, `echo`,
statistics, late-subscriber header replay and the GUI's live feed for free.
The transport is hidden behind the SDK, so peer-to-peer or shared-memory
paths can be added later without touching node code (ADR 0003).

## Two planes, two sockets

Each client opens two connections to the supervisor:

| Plane | Carries | Framing | Notes |
|-------|---------|---------|-------|
| Control | hello, config, start, stop, params, ping/pong, clock sync, stats, logs | Length-prefixed JSON | Low volume. Must stay responsive under data load, hence its own socket. |
| Data | stream headers, signal chunks, events, end-of-stream | Length-prefixed binary frames | High volume. Never blocks on control. |

Transport per platform:

| Platform | Local transport | Remote transport (future) |
|----------|-----------------|---------------------------|
| macOS, Linux | Unix domain sockets in the session directory | TCP |
| Windows | TCP on loopback, ports chosen by the supervisor | TCP |

The protocol is byte-identical over both; only the address differs.

## Session

`psl run` creates a session:

```
<workspace>/.psl/sessions/<session-id>/
  session.json        # id, pipeline path, wall-clock start, socket addresses, pid, resolved graph
  logs/<node-id>.log  # stdout/stderr of each node process plus its forwarded log messages
  recording.pslrec    # default location when a tool starts a recording without a path

/tmp/psl/<session-id>/   # Unix only: the socket files live in a short path because
  control.sock           # Unix socket paths are limited to about 100 bytes
  data.sock
```

On Windows the two addresses are `127.0.0.1:<port>` and there are no socket
files. Tools never guess addresses; they read them from `session.json`.

`<workspace>/.psl/current` holds the id of the running session so tools can
find it without arguments. Session ids are `YYYYMMDD-HHMMSS-xxxx` with a
random suffix.

The supervisor passes these environment variables to every node process:

| Variable | Value |
|----------|-------|
| `PSL_SESSION` | session id |
| `PSL_NODE_ID` | the node's id from the pipeline (e.g. `bp`) |
| `PSL_CONTROL` | address of the control socket |
| `PSL_DATA` | address of the data socket |
| `PSL_PROTOCOL` | wire protocol version, `1` |
| `PSL_WORKSPACE` | the workspace root, which is also the process's working directory |
| `PSL_NODE_DIR` | the node's package directory (where `node.psl` and the entry script live) |

A node receives nothing else on the command line, so any language can
implement the contract by reading these variables and opening two sockets.
Nodes start in the workspace root so that relative paths in parameters (a CSV
to write, a recording to replay) mean what the pipeline author expects.

## Startup sequence

1. Parse `workspace.psl`, the pipeline, and the manifest of every referenced
   node. Resolve packages. Run every check in 04 §Validation. Any error stops
   here with diagnostics.
2. Create the session directory and sockets. Write `session.json`.
3. Spawn every node process with its working directory set to the workspace
   root, the environment above, and stdout/stderr redirected to its log file.
4. Each node connects both sockets and sends `hello`. The supervisor replies
   with `config` (parameters, input sources, output streams, options).
5. Each node runs the clock-sync exchange (03 §Clock sync) and replies
   `ready`. The supervisor waits for all nodes, with a timeout from the
   pipeline's `runtime` block (default 10 s).
6. The supervisor sends `start` to output drivers first, then modules in
   reverse topological order, then input drivers last, so no frame is
   published before its consumers are running. Cycles are started in
   declaration order.
7. The pipeline runs until `psl stop`, Ctrl-C, an input driver ends all of
   its streams and the pipeline's `runtime { stop_when_inputs_end = true }`
   is set, or a fatal failure.
8. Shutdown is the reverse: `stop` to input drivers first, then modules, then
   outputs, so every chunk already in flight is delivered. Nodes that do not
   exit within the grace period (default 5 s) are killed.

## Routing

Every output port of every node is a **stream**, identified by
`node_id.port_name`. The supervisor compiles the pipeline's maps into a
routing table:

```
stream            →  [(destination node, input port index, source index), ...]
polar.ecg         →  [(bp, 0, 0)]
bp.out            →  [(csv, 0, 0), (echo-tool, 0, 0)]
```

- A node publishes a frame with its **output port index**. It never names the
  destination.
- The supervisor looks up the stream and forwards a copy to each destination,
  rewriting the frame header's `port` to the destination's **input port
  index** and `source` to the index of that stream in the destination input's
  source list (from `config`). The payload is never touched or copied more
  than once per destination.
- **Fan-out** is the table having several destinations. **Fan-in** is one
  input port appearing under several streams; the node sees interleaved frames
  and distinguishes them by `source`. Merging or aligning them is a module's
  job (the first-party `psl/merge` and `psl/sync` modules).
- **Self-loops** (a node's output mapped to its own input) are allowed. The
  protocol is asynchronous, so they cannot deadlock; the queue bound below
  prevents unbounded growth.
- The supervisor **caches the latest stream header** per stream and delivers
  it to any subscriber that joins late (a tool, or a restarted node) before any
  data frames.

## Backpressure

Each destination (node, input port) has a bounded outbound queue in the
supervisor, default 64 frames, set per node with `runtime { queue = N }`.
When a queue is full the oldest frame is dropped and a counter is incremented.
Counters are reported in `stats` and shown by `psl ps`. The policy is
drop-oldest because for live physiological data a stale chunk is worth less
than a fresh one, and because blocking a producer would stall every other
consumer behind the hub. A node may opt into `runtime { on_full = "block" }`
for offline processing of a replayed recording, where losing frames is worse
than slowing down.

## Health and failure

- Heartbeat: the supervisor sends `ping` on the control plane every second;
  a node that misses three consecutive pings is declared dead and killed.
  Tools receive no pings and are simply dropped when their connection closes.
- A node process that exits or is declared dead is handled according to its
  `runtime { restart = ... }` option: `never` (default for `psl run`),
  `on-failure`, or `always`. A restarted node goes through the full handshake
  and receives cached stream headers for its inputs.
- Downstream nodes keep running during a restart; they simply receive no
  frames from that source. A node can observe gaps through sequence numbers.
- A fatal failure (a node with `restart = never` dies, or the supervisor's
  own error) stops the pipeline with a non-zero exit code and a diagnostic
  naming the node and its last log lines.
- Node logs go to files, and `log` control messages at level `info` or above
  are echoed to the supervisor's terminal prefixed with the node id. `psl run -q`
  raises the threshold to `warn`; `-v` lowers it to `debug`.
- A node that exits with code 0 on its own outside a shutdown is `exited`, not
  failed: an input driver may legitimately finish. Only a non-zero exit or a
  reported `failed` state triggers the restart policy. Restarts wait one second.
- Shutdown is staged by kind (inputs, then modules, then outputs). Each stage
  gets one grace period; nodes still alive at its end are killed.

## Supervisor threading model

Standard library threads only (ADR 0004):

- One **control reader** thread per client.
- One **data reader** thread per client: parses frames, consults the routing
  table behind a read-write lock, and pushes to destination queues.
- One **data writer** thread per client draining that client's bounded queue
  to its socket.
- One **state** thread owning the session state machine, heartbeats, restarts
  and stats aggregation, fed by a channel from the reader threads.

Typical pipelines have tens of nodes, so a few hundred threads at most. If a
pipeline with hundreds of nodes ever matters, the reader/writer threads can
move to a poll loop without changing the protocol.

## Recording and replay

Recording captures frames exactly as they cross the hub, so the recording
format is the wire format with a thin envelope (03 §Recording file). The
supervisor records when `psl run --record` is given or when a tool sends
`record_start`. Replay is the first-party input driver `psl/replay`, which
reads a recording, re-bases timestamps onto the current session clock, and
publishes the frames at their original pace (or faster). Conversion to and
from LSL's XDF format is a later first-party package, not part of the core.

## Clock

All timestamps are **session time**: nanoseconds since the session started,
as a signed 64-bit integer, on a monotonic clock. The supervisor owns the
reference. Each node estimates its offset to the supervisor with a small
NTP-style exchange at handshake and every ten seconds after (03 §Clock sync),
and the SDK's `now()` returns session time. Because the mechanism is an
offset estimate rather than a shared system clock, nodes on another machine
will work with the same code when remote transport is added. The session's
wall-clock start is stored in `session.json` so recordings can be mapped to
calendar time.

## Extension points deliberately left open

| Future feature | Where the room is |
|----------------|-------------------|
| Remote nodes | Addresses are opaque strings; the protocol is transport-agnostic; clock sync already estimates offsets. |
| Peer-to-peer or shared-memory data path | Nodes never address each other; the SDK owns the transport. |
| Package registry | `workspace.psl` already declares `require` blocks and a lock file; install sources are pluggable (07). |
| Sub-pipeline groups | `group` and `use` are reserved words in the grammar (04). |
| PhysioLab GUI | Tool clients can subscribe to any stream, set mutable params, and read the canonical graph JSON. |
| Dynamic outputs | Input drivers may declare `dynamic_outputs = true` and announce streams in `hello` (replay needs this). |
