# psl/synthetic

Input driver that generates a signal without hardware. Every feature of PSL
can be exercised with it.

| Port | Type | Notes |
|------|------|-------|
| `out` | signal | float32, one chunk every `chunk_ms`, the same waveform on every channel |

| Param | Type | Default | Notes |
|-------|------|---------|-------|
| `waveform` | enum | `"ecg"` | `ecg` (P-QRS-T beat built from Gaussians, unit `mV`, modality `ecg`), `sine`, `noise` |
| `srate` | number | 250 | Hz |
| `channels` | list of string | `["ECG"]` | one channel per label |
| `chunk_ms` | integer | 20 | chunk period in ms |
| `amplitude` | number | 1.0 | mutable: R-peak height, sine peak, or noise standard deviation |
| `frequency` | number | 1.0 | sine frequency in Hz (channels are shifted a quarter cycle each) |
| `bpm` | number | 60 | mutable: heart rate for `ecg` |
| `seed` | integer | 0 | seed for `noise` |

Chunks are stamped `t0 = now() - chunk duration`. Each tick emits
`srate * chunk_ms / 1000` samples, with a fractional remainder carried over to
the next tick (1.5 samples per tick gives 1, 2, 1, 2, ...), so the average rate
is exactly `srate`; the waveform phase is carried across chunks.

```psl
node ecg : input "psl/synthetic" {
  waveform = "ecg"
  srate    = 250
  channels = ["ECG"]
}
```

Tests: `PYTHONPATH=../../../sdk/python python -m unittest discover -s tests -v`
