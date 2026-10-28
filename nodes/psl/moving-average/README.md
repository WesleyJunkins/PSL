# psl/moving-average

Module: causal moving average over the last `window` samples of every channel.
State is carried across chunks, so the output is identical whether the input
arrives in one chunk or many. Samples before the start of the stream count as
zero.

| Port | Type | Notes |
|------|------|-------|
| `in` | signal | any channels, any rate; fan-in not allowed |
| `out` | signal | the input header plus `meta.moving_average = {window}`; integer dtypes become `f32` |

| Param | Type | Default | Notes |
|-------|------|---------|-------|
| `window` | integer | 5 | 1..100000, mutable; a change resets the state and re-announces the header |

```psl
node smooth : module "psl/moving-average" { window = 5 }
map ecg.out -> smooth.in
```

Tests: `PYTHONPATH=../../../sdk/python python -m unittest discover -s tests -v`
