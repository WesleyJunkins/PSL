# ADR 0010: LSL interoperability through bridge packages

- Status: accepted
- Date: 2026-10-28

## Context

Lab Streaming Layer is the de-facto standard for streaming biosignals, with
many devices and recorders already speaking it. Options were to adopt LSL as
PSL's transport, to adopt its data model, or to treat it as one more device.

## Decision

PSL keeps its own transport and model. The stream header is designed to map
losslessly to and from LSL's stream info (name, type, channel count, nominal
rate, channel format, source id, per-channel labels and units). LSL inlet
and outlet bridges are ordinary packages built on `pylsl`, outside the core.
XDF conversion is a package.

## Consequences

- The core has no native dependency (ADR 0004).
- Any LSL-capable device works through one bridge package.
- PSL pipelines can feed existing LSL recorders and vice versa.
- Clock semantics differ (LSL uses a local clock per host); the bridge stamps
  in session time and preserves LSL timestamps in `t_device`.
