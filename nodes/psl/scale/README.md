# psl/scale

Module: `out = in * gain + offset` on every channel, one chunk out per chunk in.

| Port | Type | Notes |
|------|------|-------|
| `in` | signal | any channels, any rate; fan-in not allowed |
| `out` | signal | the input header plus `meta.scale = {gain, offset}`; integer dtypes become `f32` |

| Param | Type | Default | Notes |
|-------|------|---------|-------|
| `gain` | number | 1.0 | mutable |
| `offset` | number | 0.0 | mutable, in the signal's unit |

A live change to either parameter re-announces the stream header (its `meta`
carries the current values) before the next chunk.

```psl
node mv : module "psl/scale" { gain = 0.001 }   # uV -> mV
map ecg.out -> mv.in
```

Tests: `PYTHONPATH=../../../sdk/python python -m unittest discover -s tests -v`
