# PSL — notes for Claude Code

PSL is a terminal-first, ROS-like system for physiological computing
pipelines. Read `docs/00-overview.md` first. The documents in `docs/` are the
source of truth; when code and docs disagree, fix one of them in the same
change.

## Hard rules

- **Node kinds are strict.** Input drivers have no data inputs and may do
  only the decoding every consumer of that device would need anyway, nothing
  more (ADR 0012). Output drivers have no data outputs and must not transform
  data. Modules do one job. Maps only route; they never transform.
- **Minimal dependencies.** The Rust core uses the standard library only.
  The Python SDK uses the standard library plus numpy. Any new dependency
  needs an ADR in `docs/decisions/` before it is added. Node packages written
  by users may depend on whatever they like.
- **The wire protocol must be implementable from any language's standard
  library** (sockets, bytes, JSON). Never design a message that needs a
  specific library to parse.
- **Chunks, never single samples.** Data moves as chunks of samples.
- **Contracts are versioned.** Protocol, manifest schema, pipeline grammar and
  the recording format all carry a version. Breaking changes bump it and get
  an ADR.
- **Nothing requires hardware.** Every feature must be testable with the
  synthetic driver and the replay driver.

## Layout (planned)

- `docs/` — specifications and ADRs
- `core/` — Rust crate `psl`: parser, checker, supervisor/hub, CLI. `build.rs` embeds
  `nodes/psl/*` and `sdk/python/psl` into the binary for `psl init`.
- `sdk/python/` — Python package `psl`
- `nodes/` — first-party node packages (`psl/synthetic`, `psl/replay`, ...)
- `examples/` — example workspaces

## Conventions

- Rust: `cargo fmt`, `cargo clippy` clean, no `unsafe` without a comment
  explaining why.
- Python: standard library + numpy only in the SDK; type hints everywhere.
- Diagnostics from `psl check` follow the format in `docs/06-cli.md`.
- Commit messages: imperative mood, one logical change per commit.

## Development

- Build and test the core: `cd core && cargo test && cargo clippy --all-targets && cargo fmt`.
  `tests/check.rs` has a fixture for every diagnostic code; `tests/e2e.rs` runs a real
  pipeline and needs `.venv/bin/python` with numpy at the repo root
  (`uv venv .venv && uv pip install --python .venv/bin/python numpy`).
- Python SDK tests: `cd sdk/python && ../../.venv/bin/python -m unittest discover -s tests`.
  Node tests: `cd nodes/psl/<name> && PYTHONPATH=../../../sdk/python ../../../.venv/bin/python -m unittest discover -s tests`.
- To run a workspace against the repo's SDK instead of the embedded copy, set
  `PSL_SDK_PATH=<repo>/sdk/python`.
- `sdk/python/NOTES.md` records SDK decisions not yet folded into the docs; keep it in sync or fold them in.
- Delete `__pycache__` directories before committing; `build.rs` skips them but they clutter the tree.
- CI (`.github/workflows/ci.yml`) runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`,
  the SDK tests and the node tests on macOS and Linux for every push, so keep all of them green locally.
- Releasing: set `version` in `core/Cargo.toml`, commit, then `git tag vX.Y.Z && git push origin vX.Y.Z`.
  `.github/workflows/release.yml` checks that the tag equals the crate version, builds the four targets
  and publishes a GitHub release with `SHA256SUMS`. Asset names carry no version so that
  `releases/latest/download/<asset>` keeps working; `install.sh` relies on that. "Run workflow" on the
  Actions tab builds without publishing (a dry run).
