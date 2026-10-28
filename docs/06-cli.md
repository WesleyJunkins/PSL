# 06 — The `psl` command

One static binary, `psl`, built from `core/`. Every command works from any
directory inside a workspace; `--workspace <dir>` overrides. `--json` makes
any command print machine-readable output; `-q` and `-v` adjust verbosity.

## Workspace

| Command | Does |
|---------|------|
| `psl init [dir] [--name n] [--no-venv]` | Create a workspace: `workspace.psl` requiring every first-party package, `pipelines/main.psl` with a synthetic→moving-average→print example, `nodes/`, `.gitignore`; install the first-party packages and the Python SDK (both embedded in the binary) under `.psl/`; create `.psl/venv` with numpy using `uv` if present (it downloads a Python 3.11+ when none is installed), else the newest Python 3.11+ on `PATH` (`PSL_PYTHON` overrides) and pip. Existing `pipelines/main.psl` and `.gitignore` are kept. Inside an existing workspace it refreshes the packages, SDK and venv and leaves user files alone. Records the binary's version in `.psl/version`; `psl run` warns when it differs. |
| `psl new node <name> --kind input\|module\|output [--lang python]` | Scaffold `nodes/<name>/` with a manifest, an entry script with the callbacks for that kind, a README and a test. |
| `psl new pipeline <name>` | Scaffold `pipelines/<name>.psl`. |
| `psl fmt [files…]` | Format `.psl` files in place. `--check` exits non-zero if anything would change. |
| `psl doctor` | Check the environment: Python available, venv healthy, sockets writable, stale sessions. |
| `psl version` | Print the binary version and the contract versions it speaks. |

## Pipelines

| Command | Does |
|---------|------|
| `psl check [pipeline]` | Parse, resolve and validate. Prints diagnostics. `--json` prints the canonical model on success. Default pipeline is `pipelines/main.psl`. |
| `psl graph [pipeline]` | Print the graph as text; `--dot` prints Graphviz. |
| `psl run [pipeline]` | Check, then start a session in the foreground. `--record <file>` records every stream. `--param node.name=value` overrides a parameter (validated against the manifest). Node log lines at `info` and above are echoed; `-q` shows `warn` and above, `-v` adds `debug`. Ctrl-C stops gracefully; a second Ctrl-C kills. Exit code is 0 on clean stop, non-zero on failure. |
| `psl stop` | Stop the current session from another terminal. |

## Live inspection (connect to the current session)

| Command | Does |
|---------|------|
| `psl ps` | One line per node: kind, state, pid, uptime, frames delivered to it (IN), frames it published (OUT), frames dropped from its queue, the worst receive latency it reported, restarts. `--watch` refreshes every second; `--json` prints the raw query. |
| `psl streams` | One line per stream with its latest header summary (type, srate, channels). |
| `psl echo <node.port>` | Print frames as they arrive. Signal chunks print `seq`, `t0`, shape and the first few samples; events print in full. `--raw` prints one tab-separated line per sample; `--json` prints one JSON object per frame. |
| `psl set <node.param> <value>` | Change a mutable parameter live. |
| `psl record start [file]` / `psl record stop` | Start or stop recording from a running session. |
| `psl logs [node] [-f]` | Print or follow session logs. |

## Recordings

| Command | Does |
|---------|------|
| `psl rec info <file>` | (Phase 2) Print a recording's header: pipeline, streams, duration, frame counts. |
| `psl rec cut <file> --from <s> --to <s> -o <out>` | (Phase 2) Extract a time range. |

Replay is not a command; it is the first-party input driver `psl/replay`
used in a pipeline with `file = "…"`, so a replayed pipeline is checked and
run exactly like a live one.

## Packages

| Command | Does |
|---------|------|
| `psl install [owner/name\|path\|git+url] [--version req]` | Add to `workspace.psl`, resolve, fetch into `.psl/packages/`, update `psl.lock`, install Python requirements into `.psl/venv`. With no argument, install everything in `workspace.psl`. |
| `psl remove <owner/name>` | Remove from the workspace. |
| `psl update [owner/name]` | Re-resolve within the version requirement. |
| `psl list` | Local nodes and installed packages with kind, version and description. |
| `psl search <text>` | Search configured indexes. |
| `psl info <owner/name>` | Show a package's manifest: ports, params, description. |
| `psl test [node]` | Run a node package's tests with the workspace venv. |
| `psl publish` | Reserved for the registry phase (07). |

## Diagnostic format

Every diagnostic from `psl check` and `psl run` has the same shape. Codes
are defined in 04.

```
error[E0303]: port types differ
  --> pipelines/main.psl:31:5
   |
31 | map beats.out -> csv.in
   |     ^^^^^^^^^    ^^^^^^ input port `in` of `csv` (psl/csv-writer) is `signal`
   |     output port `out` of `beats` (psl/threshold) is `event`
   |
   = help: route events to an event-capable output such as psl/event-log,
           or convert with psl/to-signal

warning[W0001]: output port is mapped nowhere
  --> pipelines/main.psl:14:1
   |
14 | node bp : module "psl/bandpass" {
   | ^^^^^^^ output `out` of `bp` has no consumer

check: 1 error, 1 warning in pipelines/main.psl
```

With `--json`, diagnostics are a list of
`{code, level, message, file, line, column, end_line, end_column, labels, help}`.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success, or a clean stop |
| 1 | Diagnostics with at least one error |
| 2 | Usage error |
| 3 | Runtime failure (a node failed and was not restarted, or the supervisor failed) |
| 4 | No running session when one was required |
