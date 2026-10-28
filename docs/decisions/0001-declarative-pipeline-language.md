# ADR 0001: PSL is a declarative pipeline language

- Status: accepted
- Date: 2026-10-28

## Context

The project name says "language". The options were a general-purpose
language with its own interpreter or compiler, a pipeline language built on
top of existing languages, or no language at all (YAML/TOML configuration
like Timeflux and ROS launch files).

## Decision

PSL is a declarative language for describing pipelines, manifests and
workspaces. It compiles to a canonical JSON model. Node logic is written in
existing languages through SDKs. PSL has no expressions, variables or control
flow in version one; room is reserved for `let`, `group`, `use` and
`include`.

## Consequences

- Users write signal processing in Python with numpy and scipy, which is what
  they already know, and never learn a new language for that part.
- A general-purpose language would have consumed years before any pipeline
  ran; this grammar is a few hundred lines to parse.
- The language can still grow (groups, references) without breaking files.
- The name is slightly aspirational, as ROS's is. Accepted.
