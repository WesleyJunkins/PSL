# ADR 0005: Data moves as chunks, never single samples

- Status: accepted
- Date: 2026-10-28

## Context

Physiological devices deliver packets of samples. Per-sample messaging would
mean thousands of frames per second per stream and would make Python nodes
unusable.

## Decision

The unit of transfer on a signal stream is a chunk: a contiguous block of
samples with one timestamp for the first sample, an optional per-sample
timestamp array, and a sequence number. Chunk size is the producer's choice.

## Consequences

- Latency is bounded by chunk duration, typically a few tens of milliseconds.
- Even dense EEG at high rates is only a few megabytes per second.
- Modules that need sample-level state keep it across chunks themselves.
- Sub-millisecond closed loops are explicitly out of scope (00 §Non-goals).
