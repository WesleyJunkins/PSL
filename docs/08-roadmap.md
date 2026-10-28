# 08 — Roadmap

Phases are sequential. Each has a definition of done that can be checked
without hardware. Dates are deliberately absent; this is a years-long
project and the order matters more than the calendar.

## Phase 0 — Foundation (current)

- [x] Specifications 00–07 written
- [x] Architecture decision records 0001–0012
- [x] Specifications reviewed and ADRs marked accepted
- [x] Repository initialised with the planned layout

**Done when** every open question in the ADRs is resolved and no spec
contradicts another.

## Phase 1 — Vertical slice on macOS

Core (`core/`, Rust, std only):

- [x] Lexer and parser for the PSL grammar with precise spans
- [x] JSON encoder/decoder
- [x] Schemas for pipeline, manifest and workspace documents
- [x] Checker implementing every code in 04 §Validation
- [x] Canonical model output (`psl check --json`)
- [x] Supervisor: session directory, Unix domain sockets, handshake, clock sync, routing, bounded queues, heartbeats, logs, graceful stop
- [x] Commands: `init`, `new node`, `new pipeline`, `check`, `graph`, `run`, `stop`, `ps`, `streams`, `echo`, `logs`, `version`, `doctor`
- [~] Protocol conformance: `core/tests/e2e.rs` drives the Python SDK through the real supervisor, and `sdk/python/tests/test_integration.py` drives real nodes from a fake supervisor. A standalone conformance runner for other SDKs comes with the C SDK (Phase 6)

Python SDK (`sdk/python/`):

- [x] Transport, framing, handshake, clock sync
- [x] `Node` base class, worker thread, timers, `spawn`
- [x] `Chunk`, `Event`, headers backed by numpy
- [x] `psl.testing.Harness`

First-party nodes (`nodes/`):

- [x] `psl/synthetic` — input: ECG, EEG-like noise, sine, with `srate`, `channels`, `waveform`
- [x] `psl/moving-average` — module, numpy only
- [x] `psl/scale` — module, multiply/offset, numpy only
- [x] `psl/print` — output to the session log and `psl echo`
- [x] `psl/csv-writer` — output

- [x] Example workspace `examples/hello/` (synthetic → moving average → scale → print + CSV)
- [x] `psl list`, `psl set`, `psl record start/stop` and `psl run --record` landed early because the hub made them cheap

**Done when** `psl init && psl run` streams synthetic ECG through a module
into a CSV file, `psl echo` shows live chunks from another terminal, every
error code in 04 is triggered by a fixture under `core/tests/check.rs`, and
the SDK passes the conformance suite.

**Status (2026-10-28):** done on macOS except the standalone conformance
runner. Verified by `cargo test` (35 Rust tests) and 87 Python tests.

## Phase 2 — Record, replay, events, live control

- [ ] Reconcile `sdk/python/NOTES.md` into docs 02, 03 and 05 (decisions the SDK took where the docs were silent), then delete it
- [ ] Dynamic constraint checking at stream-header time (docs/02 §Type checking): the SDK does not do it yet; decide whether the hub or the SDK owns it

- [x] Recording in the supervisor (`--record`, `psl record start/stop`), `.pslrec` format
- [ ] `psl/replay` input driver with `dynamic_outputs`; `psl check` reads recording headers
- [ ] `psl rec info`, `psl rec cut`
- [ ] Event streams end to end: `psl/threshold` (signal → event), `psl/event-log` (output)
- [x] `set_param` live, `psl set`, mutable params in the SDK
- [ ] Utility modules: `psl/select` (channels), `psl/merge` (fan-in to one stream), `psl/sync` (resample several inputs to one time base), `psl/to-signal` (events → signal)
- [ ] Stats and latency in `psl ps --watch`
- [ ] Restart policies exercised by tests that kill nodes
- [ ] First external bridges as ordinary packages, outside the core: `community/lsl-in`, `community/lsl-out` (pylsl), `community/brainflow-in`, `community/xdf-export`

**Done when** a pipeline can be recorded, replayed through a different
pipeline with the same results, and a parameter can be changed while running
with the effect visible in `psl echo`.

## Phase 3 — Packages

- [ ] `psl install` from path, git and index; `psl.lock`; content hashes
- [ ] Workspace venv management, requirement merging, `uv` fast path
- [ ] `psl list`, `info`, `search`, `remove`, `update`, `test`
- [ ] The official index repository with the first-party packages
- [ ] Contribution guide with the driver review checklist

**Done when** a fresh machine can `psl init`, `psl install psl/bandpass`,
and run a pipeline using it with no manual Python setup.

## Phase 4 — Linux and Windows, distribution

- [ ] TCP loopback transport on Windows; path handling; process spawning
- [ ] Linux: CI runs the whole test suite on macOS and Linux (`.github/workflows/ci.yml`, 2026-10-28); Linux still to be tried by hand. Windows CI once Phase 4's transport work lands
- [x] `install.sh`: build from source and install a single binary (2026-10-28)
- [x] GitHub releases with prebuilt binaries for macOS (arm64, x86_64) and Linux (x86_64, arm64): `.github/workflows/release.yml` runs on a `v*` tag, `install.sh` downloads and verifies them (2026-10-28; first release run pending)
- [ ] Homebrew tap
- [ ] `psl doctor` covers platform specifics (Bluetooth permissions on macOS, firewall on Windows)

## Phase 5 — First real hardware

- [ ] Write the first device driver with the guide in 05, revise the guide from experience
- [ ] Commit a short recording as a shared fixture
- [ ] Multi-device alignment validated with `psl/sync`

## Phase 6 — Ecosystem and language growth

- [ ] C SDK for manufacturers; JavaScript SDK
- [ ] `group` / `use` sub-pipelines; `let`; `include`
- [ ] Registry with signing and `psl publish`
- [ ] Remote nodes over TCP using the existing clock sync

## Phase 7 — PhysioLab

A separate application that connects to the supervisor as a tool client,
reads the canonical model, renders the graph, subscribes to streams for live
plots, sets mutable parameters, and writes `.psl` files through `psl fmt`.
Nothing in the core changes for it; that is the test of the architecture.
