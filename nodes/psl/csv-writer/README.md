# psl/csv-writer

Output driver that writes a signal to a CSV file: a header row
`t_ns,<label1>,<label2>,...` and then one row per sample with its session
time in nanoseconds. Values are written as the stream's dtype prints them
(shortest round-trip representation). The file is flushed on end-of-stream
and closed on stop.

| Port | Type | Notes |
|------|------|-------|
| `in` | signal | any channels, any rate; fan-in not allowed |

| Param | Type | Default | Notes |
|-------|------|---------|-------|
| `path` | string | required | parent directories are created |
| `append` | boolean | false | append to an existing non-empty file without repeating the header row |

```psl
node csv : output "psl/csv-writer" { path = "out/ecg.csv" }
map ecg.out -> csv.in
```

Tests: `PYTHONPATH=../../../sdk/python python -m unittest discover -s tests -v`
