# ADR 0002: Rust core, Python first SDK, language-neutral protocol

- Status: accepted
- Date: 2026-10-28

## Context

The core (parser, checker, supervisor, CLI, package manager) does no numerical
work. Node authors do, and most of them use Python. Manufacturers writing
drivers are most likely to use C or C++. The tool must ship as a single
installable binary on macOS, Linux and Windows.

## Decision

- The core is a single Rust binary, `psl`, with a library crate the future
  GUI can reuse (Tauri pairs naturally with it).
- The first SDK is Python. The protocol is specified at the byte level so a
  node can be written in any language with sockets, bytes and JSON only.
- A C SDK and a JavaScript SDK follow once the Python SDK and the conformance
  suite are stable.

## Consequences

- Fast startup, no runtime to install, easy Homebrew distribution.
- Two codebases from day one (Rust core, Python SDK), kept honest by a
  conformance test suite the core ships.
- Go was the fallback and is not pursued. C++ was rejected for build and
  distribution pain; JavaScript for its thin numerical ecosystem.
