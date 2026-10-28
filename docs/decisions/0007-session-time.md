# ADR 0007: Session time with offset estimation

- Status: accepted
- Date: 2026-10-28

## Context

Rust's `Instant`, Python's `monotonic_ns` and a C driver's `clock_gettime`
may read different clock sources. Wall-clock time can jump. Remote nodes are
a future goal. LSL solved this with NTP-style offset estimation.

## Decision

All timestamps are session time: signed 64-bit nanoseconds since session
start on the supervisor's monotonic clock. Each node estimates its offset
with a four-timestamp exchange at handshake and every ten seconds, keeping
the sample with the smallest round trip. Device clocks are carried as a
separate field and never used by the runtime.

## Consequences

- Nodes in any language and on any machine share one clock without a shared
  system clock.
- About fifty lines of code per SDK.
- Recordings map to calendar time through `wall_start_ns` in the session.
