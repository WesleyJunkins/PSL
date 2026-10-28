# 00 — Overview

## What PSL is

PSL (Physiological Scripting Language) is a terminal-first system for building
real-time physiological computing pipelines. A user creates a workspace,
writes or installs nodes, connects the nodes' ports with maps in a pipeline
file, and runs the pipeline from the terminal. The core launches one process
per node, routes data between them, and provides tooling to inspect, record
and replay what flows.

PSL is a **pipeline language with a runtime**, in the same way ROS is a
middleware with tooling. It is not a general-purpose programming language.
Node logic is written in ordinary languages through an SDK; Python comes
first. The `.psl` syntax is used for three kinds of document: pipelines, node
manifests and workspaces.

## The four building blocks

| Block | Role | Data inputs | Data outputs | May transform data |
|-------|------|-------------|--------------|--------------------|
| Input driver | Pull raw samples from a device (BLE, Wi-Fi, serial, socket, file) and publish them | none | one or more | **never** |
| Module | Do one specific thing to its inputs and publish the result | one or more | one or more | yes, that is its job |
| Output driver | Take inputs and deliver them to an external target (file, network, screen, device) | one or more | none | **never** |
| Map | Connect specific output ports to specific input ports | — | — | **never** |

Input drivers, modules and output drivers are collectively **nodes**. The
"no data inputs" and "no data outputs" rules are enforced by the checker from
each node's manifest. The "never transform" rule for drivers is a policy
enforced by review of the package index, not by the runtime; a driver still
takes parameters, because selecting a device address or a sample rate is
configuration, not transformation. A driver may also do whatever **every**
application using that device would have to do anyway to read the data, such
as unpacking the packet format or applying the manufacturer's scale factor,
and nothing more (ADR 0012).

## Principles

1. **Contracts over implementations.** The node manifest, the data model, the
   wire protocol, the pipeline grammar and the package format are the things
   that must stay stable for years. Transport, language and process model can
   change behind them.
2. **Drivers never touch data; maps never touch data.** All signal
   manipulation lives in modules, and each module does one thing.
3. **Chunks, not samples.** Data moves as chunks of samples with a timestamp
   and a sequence number.
4. **Nothing requires hardware.** The synthetic driver and the replay driver
   are part of the core so every feature can be developed and tested without
   a device.
5. **Minimal dependencies.** The Rust core uses only the standard library.
   The Python SDK uses the standard library plus numpy. The wire protocol can
   be implemented from any language's standard library.
6. **Everything is a client.** The `psl echo` tool, the recorder and the
   future PhysioLab GUI all speak the same protocol as a node. Nothing is
   privileged.
7. **Errors are diagnostics.** `psl check` reports problems with file, line,
   column and a suggestion, in the style of a modern compiler, before any
   process is launched.
8. **Leave room, do not build ahead.** Remote nodes, a package registry,
   shared-memory transport and sub-pipeline groups are designed for but not
   built in version one.

## Non-goals

- Hard real-time. Sub-millisecond closed loops (e.g. stimulation triggered on
  a single sample) are out of scope. Python nodes and garbage collection make
  that promise impossible to keep honestly.
- A general-purpose language or an interpreter for one.
- Re-implementing signal processing libraries. Modules use numpy, scipy,
  MNE, NeuroKit2 or whatever they like.
- A GUI in the core. PhysioLab is a separate client built later.

## Glossary

| Term | Meaning |
|------|---------|
| Node | A runnable unit: an input driver, module or output driver. One process at run time. |
| Kind | `input`, `module` or `output`. Declared in the manifest and repeated in the pipeline. |
| Port | A named, typed connection point on a node. Input ports receive; output ports publish. |
| Port type | `signal` or `event`. See 02. |
| Stream | The data flowing out of one output port during one session. Identified as `node_id.port_name`. |
| Stream header | Metadata announced on a stream before data: sample rate, channels, units. |
| Chunk | A contiguous block of samples from a signal stream. |
| Event | One timestamped record on an event stream. |
| Map | A routing rule from one or more output ports to one or more input ports. |
| Pipeline | A `.psl` file declaring nodes, their parameters and maps. |
| Manifest | A `node.psl` file declaring a node's kind, ports, parameters and runtime. |
| Workspace | A directory with a `workspace.psl`, pipelines, local nodes and installed packages. |
| Package | A distributable folder containing a manifest and node code. Named `owner/name`. |
| Supervisor | The process started by `psl run`. Launches nodes, routes data, monitors health. Also called the hub. |
| Session | One execution of a pipeline. Has an id, a directory, a clock and sockets. |
| Session time | Nanoseconds since the session started, on a monotonic clock shared by all nodes. |
| Control plane | The socket carrying lifecycle, parameters, heartbeats, stats and logs. |
| Data plane | The socket carrying stream headers, chunks and events. |
| Client | Anything connected to the supervisor: a node, a tool like `psl echo`, or PhysioLab. |

## Prior art

| System | What it is | What PSL takes from it | How PSL differs |
|--------|------------|------------------------|-----------------|
| ROS 2 | Robotics middleware | Workspaces, packages, one process per node, introspection tooling, launch files | Maps instead of topics; strict node kinds; a purpose-built pipeline language; no DDS |
| Lab Streaming Layer (LSL) | De-facto standard for streaming biosignals between programs | Stream metadata model, clock offset estimation, chunked transport | PSL has its own model designed to map losslessly to LSL; LSL is a bridge package, not a core dependency |
| Timeflux | Python real-time biosignal framework with YAML graphs | Node/port graph model, Python as the user language | Language-neutral protocol, compiled core, static checking, strict kinds, package ecosystem |
| BCI2000 | Source / signal processing / application modules over TCP | The three-way split is essentially input / module / output | Many nodes, any language, declarative maps |
| OpenViBE, NeuroPype | Visual box-and-link designers | Preview of what PhysioLab must support | PSL is terminal-first; the GUI is a client of the same core |
| GStreamer | Media pipeline framework | Format negotiation when elements link (stream headers) | Far smaller scope |
| BrainFlow | One API for many biosensor boards | A single PSL driver can wrap it to cover many devices | Bridge package, not core |

## What makes PSL different

- A **pipeline language** with one syntax for pipelines, manifests and
  workspaces, checked statically with compiler-quality diagnostics.
- **Structural node kinds.** Drivers cannot have the ports that would let
  them transform data. The graph's shape is honest by construction.
- **Maps, not topics.** The user thinks in connections between ports, never
  in names of channels in a global namespace.
- **Language-neutral by protocol, not by binding.** A manufacturer can write
  a driver in C with nothing but sockets. The SDKs are conveniences.
- **The recording format is the wire format.** Record and replay are trivial,
  lossless and cheap, and exist from day one.
- **The GUI is just a client.** PhysioLab will connect to the same control
  and data planes as `psl echo`.
