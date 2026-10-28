# 04 — The PSL language (grammar version 1)

One syntax, three document types:

| Document | File | Declares |
|----------|------|----------|
| Pipeline | `pipelines/<name>.psl` | Nodes, their parameters, maps between ports |
| Manifest | `node.psl` at a package root | A node's kind, runtime, ports, parameters |
| Workspace | `workspace.psl` at a workspace root | Workspace name, required packages, settings |

The grammar is shared; each document type has its own schema and validation
rules. Every document compiles to plain JSON (the **canonical model**), which
is what the runtime, the tools and later PhysioLab consume. The syntax is for
humans; the JSON is for programs. Both are stable contracts.

## A complete pipeline

```psl
# Heart-rate demo: synthetic ECG → band-pass → CSV + live beats

pipeline "hr_demo" {
  description = "Synthetic ECG through a band-pass filter into a CSV file"
  runtime {
    ready_timeout_s = 10
  }
}

node ecg : input "psl/synthetic" {
  waveform = "ecg"
  srate    = 250
  channels = ["ECG"]
}

node bp : module "psl/bandpass" {
  low  = 0.5
  high = 40
  runtime { restart = "on-failure" }
}

node beats : module "psl/threshold" {
  level = 0.6
  label = "beat"
}

node csv : output "psl/csv-writer" {
  path = "out/ecg.csv"
}

node log : output "psl/print" {}

map ecg.out   -> bp.in
map bp.out    -> beats.in, csv.in
map beats.out -> log.in
```

## Lexical structure

- **Encoding:** UTF-8. Line endings `\n` or `\r\n`.
- **Comments:** `#` to end of line.
- **Identifiers:** `[A-Za-z_][A-Za-z0-9_]*`. Used for node ids, port names,
  parameter names, block keywords and enumeration symbols.
- **Strings:** double-quoted. Escapes: `\"`, `\\`, `\n`, `\t`, `\r`,
  `\uXXXX`. No multi-line strings in version one.
- **Numbers:** `-?[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?`. A number without a
  fraction or exponent is an integer; otherwise it is a float.
- **Booleans:** `true`, `false`. **Null:** `null`.
- **Punctuation:** `{ } [ ] = : , . ->`
- **Separators:** statements end at a newline. Inside `{ }` a comma may also
  separate statements, so `{ low = 0.5, high = 40 }` is valid.
- **Reserved words** (cannot be identifiers): `pipeline`, `node`, `map`,
  `input`, `module`, `output`, `param`, `runtime`, `workspace`, `require`,
  `locked`, `true`, `false`, `null`, and reserved for the future: `group`,
  `use`, `let`, `include`, `with`, `as`, `import`.

## Grammar

```ebnf
document    = { statement } ;
statement   = node_decl | block | attribute | map_stmt ;

block       = IDENT { label } "{" { statement } "}" ;
label       = STRING | IDENT ;

node_decl   = "node" IDENT ":" kind STRING [ "{" { statement } "}" ] ;
kind        = "input" | "module" | "output" ;

attribute   = IDENT "=" value ;

map_stmt    = "map" endpoints "->" endpoints ;
endpoints   = endpoint { "," endpoint } ;
endpoint    = IDENT "." IDENT ;

value       = STRING | NUMBER | "true" | "false" | "null"
            | IDENT                                 (* enumeration symbol *)
            | "[" [ value { "," value } [ "," ] ] "]"
            | "{" [ attribute { sep attribute } [ sep ] ] "}" ;
sep         = "," | NEWLINE ;
```

Notes:

- A bare identifier as a value is an **enumeration symbol**, allowed only
  where the schema defines an enumeration (`kind = module`, `type = signal`,
  `channels = any`). Anywhere else it is error E0005 with the hint "did you
  mean a string?". This keeps the door open for variables later.
- Inside a `node` block, **attributes are parameters** and **nested blocks
  are node options**. Only the `runtime` block is defined in version one.
- Object values (`{ a = 1 }`) and blocks look alike but cannot be confused:
  an object only appears after `=`.

## Pipeline schema

```psl
pipeline STRING {                 # exactly one per file, must be first
  description = STRING            # optional
  runtime {                       # optional, all fields optional
    ready_timeout_s      = NUMBER   # default 10
    stop_grace_s         = NUMBER   # default 5
    stop_when_inputs_end = BOOL     # default false
    record               = STRING   # default none; path to write a .pslrec
  }
}

node ID : KIND STRING {           # any number
  <param> = value                 # must match the manifest's params
  runtime {                       # optional
    restart = "never" | "on-failure" | "always"   # default "never"
    queue   = INTEGER                             # default 64 frames
    on_full = "drop_oldest" | "block"             # default "drop_oldest"
    env     = { KEY = STRING, ... }               # extra environment for the process
  }
}

map ENDPOINTS -> ENDPOINTS        # any number
```

Package reference strings:

| Form | Resolves to |
|------|-------------|
| `"name"` (no slash) | The workspace-local node at `nodes/name/node.psl` |
| `"owner/name"` | An installed package from `workspace.psl`'s `require` list |

The two forms cannot collide, so there is no precedence rule. A version is never
written in a pipeline; `workspace.psl` and `psl.lock` pin versions.

## Manifest schema

```psl
node STRING {                     # the package's bare name
  kind        = input | module | output
  version     = STRING            # semver
  description = STRING
  author      = STRING            # optional
  license     = STRING            # optional, SPDX id
  homepage    = STRING            # optional
  dynamic_outputs = BOOL          # optional, input only, default false

  runtime python {                # exactly one runtime block
    entry    = STRING             # script path relative to the package
    requires = [ STRING, ... ]    # pip requirement specifiers, optional
    python   = STRING             # minimum version, optional, e.g. ">=3.11"
  }
  # or
  runtime exec {
    command = [ STRING, ... ]     # argv, relative paths resolved against the package
  }

  input STRING {                  # zero or more; forbidden when kind = input
    type        = signal | event
    description = STRING
    required    = BOOL            # default true
    multiple    = BOOL            # default true (fan-in allowed)
    channels    = INTEGER | any   # signal only
    modality    = SYMBOL | [SYMBOL, ...] | any
    dtype       = SYMBOL | [SYMBOL, ...] | any
    srate       = NUMBER | [NUMBER, NUMBER] | any
    labels      = [STRING, ...] | any   # event only
  }

  output STRING {                 # zero or more; forbidden when kind = output
    type        = signal | event
    description = STRING
    channels    = INTEGER | any   # what the node promises, if it knows
    modality    = SYMBOL | any
  }

  param STRING {                  # zero or more
    type        = number | integer | string | boolean | enum | list
    description = STRING
    default     = value           # if absent, the param is required
    mutable     = BOOL            # default false; true means set_param works at run time
    min         = NUMBER          # number/integer
    max         = NUMBER
    values      = [value, ...]    # enum
    of          = number | integer | string | boolean   # list element type
    unit        = STRING          # documentation only
  }
}
```

Port and parameter names are identifiers. Declaration order defines port
indices on the wire.

## Workspace schema

```psl
workspace STRING {
  psl    = STRING                 # minimum psl version, e.g. "0.1"
  python = STRING                 # interpreter used to create .psl/venv, optional
}

require STRING {                  # "owner/name"
  version = STRING                # semver requirement, e.g. "^0.1" or "=0.1.3"
  source  = STRING                # optional: "index", "git+<url>", or "path:<dir>"; default "index"
}
```

`psl.lock` is generated, uses the same syntax, and is not edited by hand:

```psl
locked "psl/bandpass" {
  version = "0.1.3"
  source  = "git+https://github.com/psl-nodes/bandpass#v0.1.3"
  hash    = "sha256:…"
}
```

## Canonical model

`psl check --json` prints the compiled pipeline. This JSON is the contract
with tooling and PhysioLab.

```json
{
  "psl": 1,
  "pipeline": {
    "name": "hr_demo",
    "file": "pipelines/main.psl",
    "description": "Synthetic ECG through a band-pass filter into a CSV file",
    "runtime": {"ready_timeout_s": 10, "stop_grace_s": 5, "stop_when_inputs_end": false}
  },
  "nodes": [
    {
      "id": "ecg",
      "kind": "input",
      "package": "psl/synthetic",
      "version": "0.1.0",
      "path": ".psl/packages/psl/synthetic/0.1.0",
      "params": {"waveform": "ecg", "srate": 250, "channels": ["ECG"]},
      "runtime": {"restart": "never", "queue": 64, "on_full": "drop_oldest", "env": {}},
      "inputs": [],
      "outputs": [{"name": "out", "index": 0, "type": "signal"}]
    }
  ],
  "maps": [
    {"from": "ecg.out", "to": "bp.in"},
    {"from": "bp.out",  "to": "beats.in"},
    {"from": "bp.out",  "to": "csv.in"},
    {"from": "beats.out", "to": "log.in"}
  ]
}
```

Maps are expanded to single edges. Parameters include defaults filled in.
Order of nodes is declaration order.

## Validation

`psl check` runs every rule below and reports all findings, not just the
first. Errors stop `psl run`; warnings do not.

| Code | Rule |
|------|------|
| E0001 | Syntax error |
| E0002 | Unknown block or attribute for this document type |
| E0003 | Wrong value type for an attribute |
| E0004 | Duplicate attribute or block label |
| E0005 | Bare identifier used where a string is expected |
| E0006 | Missing required attribute or block (e.g. no `pipeline` block, a port without `type`) |
| E0101 | Package or local node not found |
| E0102 | Kind in pipeline does not match the manifest's kind |
| E0103 | Duplicate node id |
| E0104 | Manifest fails its own schema (E02xx/E04xx inside it are reported at the manifest's location) |
| E0201 | Unknown parameter for this node |
| E0202 | Parameter value has the wrong type |
| E0203 | Required parameter not given |
| E0204 | Parameter outside `min`/`max` or not in `values` |
| E0301 | Endpoint refers to an unknown node or port |
| E0302 | Map source is not an output port, or map target is not an input port |
| E0303 | Port types differ (`signal` vs `event`) |
| E0304 | Static constraint cannot be satisfied (channels, modality, dtype, srate, labels) |
| E0305 | Required input port has no source |
| E0306 | Fan-in into a port declared `multiple = false` |
| E0401 | Manifest declares inputs on an `input` node or outputs on an `output` node |
| E0402 | Manifest has no runtime block, or more than one |
| E0501 | Workspace requires a package that is not installed or whose lock hash mismatches |
| W0001 | Output port is mapped nowhere |
| W0002 | Node has `dynamic_outputs`; maps from it are checked at run time only |
| W0003 | Map creates a cycle (allowed, but worth knowing) |

Diagnostics are printed in the format defined in 06.

## Formatting

`psl fmt` rewrites a document in canonical style: two-space indentation, one
statement per line, a blank line between top-level statements, aligned `=`
within a block, maps last. Formatting never changes meaning.

## Reserved for later

- `group "name" { ... }` and `use "file.psl" as x` for reusable
  sub-pipelines with their own ports.
- `let name = value` for named values shared across nodes.
- `include "file.psl"` for splitting large pipelines.
- Expressions on the right of `=` (arithmetic, references). Today the right
  side is a literal only.
