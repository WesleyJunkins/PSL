# psl/print

Output driver that logs what flows, for looking at a pipeline from the
terminal (`psl logs print -f`). It never writes files or touches the data.

| Port | Type | Notes |
|------|------|-------|
| `in` | signal | optional; logs the header and one line every `every` chunks: `in seq=... t0=... shape=... first=[...]` |
| `events` | event | optional; logs the header and one line per event |

| Param | Type | Default | Notes |
|-------|------|---------|-------|
| `every` | integer | 25 | chunks per log line, counted per source; the first chunk is always logged |

```psl
node log : output "psl/print" { every = 50 }
map ecg.out   -> log.in
map beats.out -> log.events
```

Tests: `PYTHONPATH=../../../sdk/python python -m unittest discover -s tests -v`
