# ADR 0011: One syntax for pipelines, manifests and workspaces

- Status: accepted
- Date: 2026-10-28

## Context

With a hand-written parser already required for pipeline files (ADR 0004),
using TOML or JSON for manifests and workspaces would mean a second parser
and two syntaxes for users to learn.

## Decision

All three document types use the PSL grammar with different schemas. The
lock file uses it too. Every document compiles to canonical JSON.

## Consequences

- One parser, one formatter, one set of diagnostics, one highlighting
  grammar for editors.
- Manifests gain comments and a consistent look next to pipelines.
- The grammar must stay general enough for configuration, which it is:
  blocks, attributes, lists, objects, enumeration symbols.
