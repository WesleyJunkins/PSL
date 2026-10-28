# hello

The smallest useful PSL workspace: a synthetic ECG, smoothed, rescaled, printed
to the log and written to `out/ecg.csv`.

```sh
cd examples/hello
psl init          # installs the first-party packages, the Python SDK and a venv with numpy under .psl/
psl check         # validates pipelines/main.psl
psl run           # Ctrl-C to stop
```

From another terminal, while it runs:

```sh
psl ps
psl streams
psl echo mv.out
psl set mv.gain 2000
```

`.psl/` and `out/` are generated and ignored by git.
