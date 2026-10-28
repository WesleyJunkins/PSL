# ADR 0009: Record and replay are core; the recording format is the wire format

- Status: accepted
- Date: 2026-10-28

## Context

There is no hardware during early development, and even with hardware,
reproducible development needs recordings. A separate recording format would
need its own encoder and decoder in the core and in every tool.

## Decision

The supervisor records frames verbatim as they cross the hub, with an
envelope of record time and stream index. Replay is a first-party input
driver that re-bases timestamps and republishes. XDF import and export are
later packages, not core.

## Consequences

- Recording costs almost nothing and is lossless by construction.
- `psl check` can validate maps from a replay node by reading the file
  header.
- The recording format is versioned with the protocol.
