# 07 — Packages

A package is a directory with a manifest and the code the manifest points
at. Packages are how input drivers, modules and output drivers are shared.
Version one supports installing from a local path, from git, and from an
index; a hosted registry comes later and the design leaves room for it.

## Layout

```
bandpass/
  node.psl            # manifest (04 §Manifest schema)
  main.py             # entry named in the manifest
  README.md           # what it does, ports, params, example map
  LICENSE
  CHANGELOG.md
  tests/
    test_bandpass.py  # uses psl.testing.Harness
    fixtures/
      ecg_10s.pslrec  # short recording for offline tests
```

A compiled driver uses `runtime exec { command = ["./bin/driver"] }` and
ships the binary or a build script; version one does not build packages, so
compiled packages ship prebuilt binaries per platform under `bin/<os>-<arch>/`
and the manifest's command may use `${os}` and `${arch}` placeholders.

## Naming and versions

- Package names are `owner/name`: lowercase letters, digits and hyphens;
  `owner` is a GitHub-style namespace. First-party packages use `psl/`.
- The manifest's `node "name"` is the bare name; the owner comes from where
  the package was installed from. A package whose manifest name disagrees
  with its install name is rejected.
- Versions are semantic. `require` uses `^`, `~`, `=`, `>=` requirements
  as in Cargo.
- A node's manifest hash (SHA-256 of `node.psl`) is sent in `hello` so the
  supervisor can refuse a process whose manifest changed after checking.

## Where things go in a workspace

```
my_lab/
  workspace.psl
  psl.lock
  pipelines/
  nodes/                       # local, unpublished nodes, referenced as "name"
  .psl/
    packages/<owner>/<name>/<version>/   # installed packages
    venv/                      # one Python environment for the whole workspace
    sessions/<id>/             # session directories (01)
    current                    # id of the running session
    cache/                     # fetched archives, index clones
```

`.psl/` is ignored by git. `workspace.psl` and `psl.lock` are committed.

## Install sources

| Source string | Meaning |
|---------------|---------|
| `index` (default) | Look the name up in the configured indexes. |
| `git+https://host/owner/repo` with optional `#tag` or `#rev` | Clone at that ref. The package is the repo root, or a subdirectory given by `path:` after the ref. |
| `path:../relative/or/absolute` | Use a directory on disk, not copied. For developing a package next to a workspace. |

`psl install` resolves every `require`, writes `psl.lock` with exact
versions, sources and content hashes, then installs Python requirements. A
later `psl install` with an unchanged lock file is reproducible.

## The index

An index is a git repository with one JSON file per package:

```
index/
  psl/
    bandpass.json
    csv-writer.json
  someone/
    polar-h10.json
```

```json
{
  "name": "psl/bandpass",
  "kind": "module",
  "description": "Butterworth band-pass filter",
  "versions": [
    {"version": "0.1.3", "source": "git+https://github.com/psl-nodes/bandpass#v0.1.3",
     "hash": "sha256:…", "psl": ">=0.1", "yanked": false}
  ]
}
```

The official index lives in its own repository and is the default. Users can
add indexes in `~/.config/psl/config.psl`. `psl search` and `psl info` read
indexes; `psl install` fetches from the source they name.

Submitting to the official index is a pull request in version one. Review
enforces the kind rules: a driver that filters or resamples is refused.

## Python environments

What version one does (Phase 1):

- The Python SDK is embedded in the `psl` binary and written to `.psl/sdk/psl/`
  by `psl init`. Python nodes are launched with that directory prepended to
  `PYTHONPATH`, so SDK and core versions always match. `PSL_SDK_PATH` overrides
  the location, which is how the repository's own `sdk/python` is used during
  development.
- The first-party packages are embedded the same way and installed into
  `.psl/packages/psl/<name>/<version>/` by `psl init`.
- `psl init` creates `.psl/venv` with numpy using `uv` when it is on `PATH`,
  else the standard `venv` module and pip. The interpreter used to run nodes is
  `workspace.psl`'s `python`, else `.psl/venv/bin/python` when it exists, else
  `python3`. `psl doctor` reports which one and whether numpy imports.

What Phase 3 adds:

- One `venv` per workspace, created with the standard library's `venv`
  module using the interpreter named in `workspace.psl` (default `python3`).
- Requirements from every installed package's `runtime python { requires }`
  are merged and installed with `pip`. If `uv` is on `PATH` it is used
  instead because it is much faster; it is never required.
- Conflicting requirements between packages are reported as E0501 with both
  packages named. Resolving them is the user's job in version one.
- The `psl` Python SDK itself is installed into the venv from a wheel
  bundled inside the `psl` binary, so SDK and core versions always match.

## Room left for the registry phase

- `psl publish` will package a directory, sign it with the maintainer's key
  and upload it; the index entry will carry the signature.
- The index JSON already carries `hash` and `yanked`, which is what a
  registry needs to be trusted.
- Install sources are an enum, so `registry+https://…` is one more variant.
- Nothing in a pipeline or manifest refers to where a package came from, so
  moving packages between sources never changes user files.
