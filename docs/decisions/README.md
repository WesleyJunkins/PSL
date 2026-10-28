# Architecture decision records

One file per decision. A decision is `proposed` until the project owner
marks it `accepted`; it is `superseded` by a later ADR rather than edited.
New dependencies, contract version bumps and reversals of any decision below
require a new ADR.

| # | Decision |
|---|----------|
| [0001](0001-declarative-pipeline-language.md) | PSL is a declarative pipeline language, not a general-purpose one |
| [0002](0002-rust-core-python-sdk.md) | Rust core, Python first SDK, language-neutral protocol |
| [0003](0003-process-per-node-hub.md) | One process per node with a hub supervisor |
| [0004](0004-minimal-dependencies.md) | Minimal dependencies: std-only core, stdlib+numpy SDK, custom transport |
| [0005](0005-chunked-transport.md) | Data moves as chunks, never single samples |
| [0006](0006-two-port-types.md) | Two port types: signal and event |
| [0007](0007-session-time.md) | Session time with NTP-style offset estimation |
| [0008](0008-maps-only-route.md) | Maps only route; fan-in interleaves; utilities are modules |
| [0009](0009-record-replay-core.md) | Record and replay are core; the recording format is the wire format |
| [0010](0010-lsl-interop.md) | LSL interoperability through bridge packages with a lossless mapping |
| [0011](0011-one-syntax.md) | One syntax for pipelines, manifests and workspaces |
| [0012](0012-structural-kinds.md) | Node kinds are enforced structurally; drivers may apply calibration only |
