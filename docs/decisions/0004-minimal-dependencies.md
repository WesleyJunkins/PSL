# ADR 0004: Minimal dependencies

- Status: accepted
- Date: 2026-10-28

## Context

The project owner wants as little third-party code as possible, both for
control and so that the protocol never depends on a library that a node
author in another language might not have. The original plan used ZeroMQ.

## Decision

- The Rust core depends on the standard library only. The PSL parser, the
  JSON codec, command-line parsing, the wire framing, the socket handling and
  the threading are written in-house.
- The Python SDK depends on the standard library and numpy.
- The wire protocol must be implementable from any language's standard
  library. ZeroMQ is dropped; transport is Unix domain sockets (TCP loopback
  on Windows) with a custom 16-byte frame header.
- Node packages may depend on anything; that is their authors' concern.
- Any exception requires its own ADR. Foreseen candidates: TLS and signature
  verification for the registry phase, where hand-rolling cryptography would
  be irresponsible.

## Consequences

- A few weeks of extra work in Phase 1 for the parser and the JSON codec.
- Zero supply-chain exposure in the core, fast builds, full control over
  error messages.
- A manufacturer can write a driver in C with no PSL library at all.
- Async runtimes (tokio) are also excluded; the supervisor uses threads,
  which is adequate for tens of nodes.
- The standard library has no signal API, so Ctrl-C handling declares the C
  library's `signal` function through a four-line `extern "C"` block. That is
  an FFI declaration against a library Rust already links, not a dependency.
