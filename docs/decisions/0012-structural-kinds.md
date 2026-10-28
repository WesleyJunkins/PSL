# ADR 0012: Node kinds are enforced structurally

- Status: accepted
- Date: 2026-10-28

## Context

The project's central rule is that drivers never transform data and modules
do one thing. A runtime cannot inspect code to verify this. At the same
time, some devices emit packed, compressed or unscaled bytes that no
application could use as delivered.

## Decision

- An `input` manifest may declare no input ports; an `output` manifest may
  declare no output ports. The checker rejects violations (E0401).
- The pipeline repeats the kind and the checker verifies it against the
  manifest (E0102), so a pipeline reads honestly.
- A driver may do to the data only what **every** application using that
  device would have to do anyway to make it readable: unpack the device's
  packet format, decode packed or compressed samples, apply the
  manufacturer's documented scale factor to reach physical units, and label
  channels. The test is: could any reasonable application choose to skip this
  step or do it differently? If yes, it is processing and belongs in a
  module. Filtering, resampling, interpolation, artifact handling,
  re-referencing, averaging and feature extraction in a driver are grounds
  for refusal from the official index. Whatever decoding a driver does must
  be listed in the stream header's `source.decoding`.
- The same test applies to output drivers in reverse: they may do only the
  formatting that the target requires.

## Consequences

- The graph's shape cannot lie about where data is transformed.
- Consumers never need per-device decoding tables; that burden sits with
  the one party who knows the device best.
- "One thing per module" remains a review guideline, not a runtime rule.
