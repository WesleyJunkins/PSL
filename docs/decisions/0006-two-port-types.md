# ADR 0006: Two port types, signal and event

- Status: accepted
- Date: 2026-10-28

## Context

A rich type system invites endless debate and makes every SDK heavier. Too
little typing and `psl check` cannot catch wiring mistakes.

## Decision

Exactly two port types: `signal` (regularly sampled, multichannel) and
`event` (irregular, labelled, JSON payload). Static checks cover the type and
declared constraints; dynamic checks happen when a stream header arrives.
A third primitive may be added with a protocol bump.

## Consequences

- Every wiring mistake between signals and events is caught before launch.
- Derived features are events or low-rate signals; conversions are modules.
- Custom binary payloads are not supported in version one; `event.data` as
  JSON covers small structured values.
