# 02 — Data model

## Port types

There are two primitive port types. Every port in every manifest has exactly
one of them.

| Type | What flows | Timing | Typical uses |
|------|------------|--------|--------------|
| `signal` | Chunks of regularly sampled, multichannel numeric data | Nominal sample rate; each chunk carries the time of its first sample | EEG, ECG, EMG, PPG, EDA, respiration, accelerometer, any filtered or derived continuous signal |
| `event` | Individual timestamped records with a label and a JSON payload | Irregular | Markers, button presses, detected beats, classifier decisions, derived features such as heart rate per beat |

Derived values that arrive at a regular rate (e.g. band power every 100 ms)
are a `signal` with a low sample rate. Derived values that arrive when
something happens are `event`s. If a module cannot decide, it should prefer
`event` and let a downstream `psl/to-signal` module resample.

Custom types are deliberately not supported in version one. A third
primitive (`table` or `blob`) can be added with a protocol version bump; the
frame type byte has room.

## Stream header

A stream is one output port during one session. Before publishing any data on
a stream, and again whenever its properties change, the producing node sends
a **stream header**. The supervisor caches the latest header per stream and
delivers it to late subscribers before any data.

Header for a `signal` stream (JSON, see 03 for framing):

```json
{
  "type": "signal",
  "srate": 250.0,
  "dtype": "f32",
  "channels": [
    {"label": "ECG",  "unit": "uV", "modality": "ecg"},
    {"label": "RESP", "unit": "au", "modality": "resp"}
  ],
  "source": {
    "device": "Polar H10",
    "serial": "A1B2C3",
    "firmware": "3.1.0"
  },
  "meta": {}
}
```

| Field | Required | Meaning |
|-------|----------|---------|
| `type` | yes | `"signal"` |
| `srate` | yes | Nominal sample rate in Hz, `> 0`. |
| `dtype` | yes | Sample type of every channel: `f32`, `f64`, `i16`, `i32`. One dtype per stream. |
| `channels` | yes | Ordered list, one entry per channel. `label` required; `unit` and `modality` default to `"au"` and `"other"`. |
| `source` | no | Free-form description of the originating device. Drivers should fill it. Modules should pass it through. |
| `meta` | no | Free-form object for anything else. Modules append rather than replace. |

Header for an `event` stream:

```json
{
  "type": "event",
  "labels": ["beat", "artifact"],
  "schema": {},
  "source": {},
  "meta": {}
}
```

| Field | Required | Meaning |
|-------|----------|---------|
| `type` | yes | `"event"` |
| `labels` | no | Known labels, for tooling and validation. Events may use labels not listed. |
| `schema` | no | Free-form description of the `data` payload. Not validated in version one. |

### Modality vocabulary

`eeg`, `ecg`, `emg`, `eog`, `ppg`, `eda`, `resp`, `temp`, `acc`, `gyro`,
`mag`, `bp`, `spo2`, `marker`, `other`. Units are free text; SI
abbreviations are preferred (`uV`, `mV`, `g`, `deg/s`, `C`, `bpm`). Modality
and unit exist so that modules can check what they are given and so that
tooling and the GUI can label plots; the runtime does not interpret them.

## Signal chunk

A chunk is a contiguous block of `n_samples × n_channels` samples from one
stream.

| Field | Meaning |
|-------|---------|
| `seq` | Sequence number per stream, starting at 0, incrementing by 1 per frame of any type (headers and end-of-stream included). A gap means frames were dropped. |
| `t0` | Session time of the first sample, in nanoseconds. |
| `t_device` | Device clock reading for the first sample if the device provides one, else 0. Units and epoch are device-defined and described in the header's `source`. |
| `n_samples`, `n_channels` | Dimensions. `n_channels` must equal the header's channel count. |
| `samples` | Sample-major (interleaved) array: sample 0 channel 0, sample 0 channel 1, ..., sample 1 channel 0, ... |
| `timestamps` | Optional per-sample session timestamps, for devices with measurable jitter. When absent, sample `i` is at `t0 + i / srate`. |

Chunk size is the producer's choice and may vary from chunk to chunk. Drivers
should publish whatever the device delivers; a typical BLE device produces
chunks of a few tens of milliseconds. Modules should emit one chunk per chunk
received unless their function requires buffering, in which case the manifest
should say so in the port description.

Rules:

- A chunk may not straddle a header change. Send a new header, then chunks.
- `t0` must be non-decreasing within a stream. Replay re-bases timestamps so
  this holds.
- Samples are little-endian. Nothing in PSL ever uses big-endian.

## Event

| Field | Meaning |
|-------|---------|
| `seq` | Sequence number per stream. |
| `t` | Session time of the event in nanoseconds. |
| `t_device` | As for chunks, or 0. |
| `label` | Short string, e.g. `"beat"`. |
| `data` | Any JSON value, often an object. Keep it small; events are not for bulk data. |

## End of stream

A producer may send **end-of-stream** on an output to say no more data will
follow in this session (replay reaching the end of a file, a driver whose
device disconnected permanently). Consumers receive `on_end`. The stream can
be reopened by sending a new header.

## Time

- **Session time** is the only clock in data frames: signed 64-bit
  nanoseconds since the session started, monotonic, shared by every node via
  offset estimation (03 §Clock sync).
- **Drivers stamp on receipt.** `t0` is the session time at which the driver
  received the chunk minus the chunk's duration, unless the driver has a
  better estimate from the device's own clock. The header's `source.timing`
  may describe how the driver stamps.
- **Device time is preserved, never trusted.** `t_device` lets a module align
  streams from devices that have their own clocks; the runtime does not use
  it.
- **Latency is observable.** Any node can compare `now()` to a chunk's `t0`.
  `psl ps` reports per-node receive latency.
- **Multi-device alignment is a module.** The first-party `psl/sync` module
  resamples several signal inputs onto one time base. The hub never
  resamples.

## Type checking

Checking happens in two stages.

**Static, at `psl check`.** From manifests alone:

- Every map connects an output port to an input port.
- Port types match (`signal` to `signal`, `event` to `event`).
- Declared constraints that do not depend on the device match: an input
  declaring `channels = 1` cannot be fed by an output declaring
  `channels = 3`. Constraints declared as `any` always match.
- Every input port with `required = true` (the default) has at least one
  source.
- Kinds are structurally valid: inputs have no input ports, outputs have no
  output ports.

**Dynamic, at stream header time.** (Specified; not implemented in version
one, see the roadmap.) When a node receives a header on an input port, the
SDK checks it against the port's manifest constraints (channel count,
modality, dtype). A mismatch is reported as a node error with the
stream name, the constraint and the actual value, and the node moves to
`failed` unless the node's `on_header` callback accepts it explicitly.

Manifest constraints available on a port (all optional):

| Constraint | Applies to | Values |
|------------|------------|--------|
| `channels` | signal | integer, or `any` (default) |
| `modality` | signal | one modality, a list, or `any` (default) |
| `dtype` | signal | one dtype, a list, or `any` (default) |
| `srate` | signal | a number, a range `[min, max]`, or `any` (default) |
| `labels` | event | list of labels the port expects, or `any` (default) |
| `required` | both | `true` (default) or `false` |
| `multiple` | both | whether fan-in is allowed, `true` (default) or `false` |
