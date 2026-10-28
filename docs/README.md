# PSL documentation

Read in order. Each document is a specification, not a tutorial; tutorials
come once there is something to run.

| # | Document | Covers |
|---|----------|--------|
| 00 | [Overview](00-overview.md) | Vision, principles, glossary, non-goals, prior art |
| 01 | [Architecture](01-architecture.md) | Processes, control and data planes, routing, lifecycle, failure |
| 02 | [Data model](02-data-model.md) | Port types, stream headers, chunks, events, time |
| 03 | [Wire protocol](03-wire-protocol.md) | Byte-level framing, handshake, control messages |
| 04 | [PSL language](04-psl-language.md) | Grammar and semantics of pipeline, manifest and workspace files |
| 05 | [Node SDK](05-node-sdk.md) | The node contract, the Python SDK, how to write each kind of node |
| 06 | [CLI](06-cli.md) | Every `psl` command and the diagnostic format |
| 07 | [Packages](07-packages.md) | Package layout, versioning, install sources, the future registry |
| 08 | [Roadmap](08-roadmap.md) | Phases and milestones |
| — | [Decisions](decisions/) | Architecture decision records |

Versions of the contracts defined here:

| Contract | Version | Defined in |
|----------|---------|-----------|
| Wire protocol | 1 | 03 |
| Manifest schema | 1 | 04 |
| Pipeline grammar | 1 | 04 |
| Recording format | 1 | 01, 03 |
