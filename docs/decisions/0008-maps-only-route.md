# ADR 0008: Maps only route

- Status: accepted
- Date: 2026-10-28

## Context

It is tempting to let a map select channels or merge streams. That would
make maps a second place where data is manipulated and would complicate the
GUI's model of an edge.

## Decision

A map is a set of edges from output ports to input ports. Fan-out copies.
Fan-in delivers interleaved frames tagged with a source index. Nothing else.
Selection, merging, synchronisation and resampling are first-party utility
modules. Self-loops are allowed.

## Consequences

- An edge in PhysioLab is just a line; it has no settings.
- Pipelines may contain a few more small nodes. That is the Unix trade.
- Fan-in semantics are simple to implement in every SDK.
