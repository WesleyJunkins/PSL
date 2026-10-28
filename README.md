# PSL

PSL is a terminal-first system for building real-time physiological computing
pipelines. You create a workspace, write or install nodes (input drivers,
modules, output drivers), connect their ports with maps in a `.psl` pipeline
file, and run the pipeline from the terminal.

PSL is modelled loosely on ROS: a small core, a language-neutral wire
protocol, SDKs for the languages people already use, and a package ecosystem.
PSL itself is a pipeline language, not a general-purpose language. Node logic
is written in ordinary languages (Python first) through an SDK.

**Status:** Phase 1 vertical slice works on macOS. `psl init` creates a
workspace, `psl check` validates it with compiler-style diagnostics, and
`psl run` launches Python nodes under the Rust supervisor; `psl ps`,
`psl streams`, `psl echo`, `psl set` and `psl record` talk to the running
session. The specifications in `docs/` are the source of truth; the roadmap in
`docs/08-roadmap.md` says what is done and what comes next.

## Install

macOS (Apple Silicon or Intel) and Linux (x86_64 or arm64):

```sh
curl -fsSL https://raw.githubusercontent.com/wesleyjunkins/PSL/main/install.sh | sh
```

This downloads the latest release from the
[releases page](https://github.com/wesleyjunkins/PSL/releases), verifies its
checksum and installs it as `~/.local/bin/psl` (or `/usr/local/bin/psl`). The
same files can be fetched by hand from that page: unpack the one for your
machine and put `psl` on your PATH. On macOS a file saved by a browser is
quarantined; run `xattr -d com.apple.quarantine psl` once before using it.

From source, which needs Rust from https://rustup.rs:

```sh
git clone https://github.com/wesleyjunkins/PSL && cd PSL
./install.sh
```

Either way the `psl` binary is self-contained: it carries the Python SDK and
the first-party nodes inside it, and nothing else is installed. Rerun the
install command to update; `./install.sh --uninstall` (or deleting the file)
removes it. A Homebrew tap is planned (roadmap, Phase 4).

## Use

In any folder:

```sh
psl init          # workspace.psl, pipelines/main.psl, nodes/, and .psl/ with the packages,
                  # the SDK and a Python environment with numpy
psl check         # validate the pipeline; errors come with file, line and a suggestion
psl run           # Ctrl-C to stop
```

From another terminal while it runs: `psl ps`, `psl streams`,
`psl echo smooth.out`, `psl set ecg.bpm 90`, `psl record start`, `psl stop`.
Write your own node with `psl new node <name> --kind module` and refer to it in
a pipeline as `"<name>"`. `psl doctor` checks the environment.

Nodes run on Python 3.11 or newer. `psl init` finds one on your machine, or
downloads one when [uv](https://docs.astral.sh/uv/) is installed. No Python
knowledge is needed to run the first-party nodes; writing your own node is a
short Python file (see `docs/05-node-sdk.md`).

## Documentation

- [docs/00-overview.md](docs/00-overview.md) — what PSL is, principles, glossary
- [docs/01-architecture.md](docs/01-architecture.md) — processes, planes, routing, lifecycle
- [docs/02-data-model.md](docs/02-data-model.md) — port types, stream headers, chunks, time
- [docs/03-wire-protocol.md](docs/03-wire-protocol.md) — byte-level framing and messages
- [docs/04-psl-language.md](docs/04-psl-language.md) — the `.psl` syntax for pipelines, manifests, workspaces
- [docs/05-node-sdk.md](docs/05-node-sdk.md) — the node contract and the Python SDK
- [docs/06-cli.md](docs/06-cli.md) — the `psl` command
- [docs/07-packages.md](docs/07-packages.md) — package layout, install, future registry
- [docs/08-roadmap.md](docs/08-roadmap.md) — phases and milestones
- [docs/decisions/](docs/decisions/) — architecture decision records

Code: [core/](core/) (Rust: parser, checker, supervisor, CLI),
[sdk/python/](sdk/python/) (the `psl` Python package),
[nodes/psl/](nodes/psl/) (first-party nodes), [examples/hello/](examples/hello/).

macOS is the platform PSL is developed and used on; Linux binaries are built
and tested in CI but not yet used with real pipelines; Windows comes later. A visual editor,
PhysioLab, will be built later on top of the same core.
