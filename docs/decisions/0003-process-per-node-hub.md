# ADR 0003: One process per node with a hub supervisor

- Status: accepted
- Date: 2026-10-28

## Context

Nodes must be language-agnostic and isolated from each other's crashes,
which implies separate processes. Data can then flow peer-to-peer (each node
connects to its sources) or through a central hub.

## Decision

Every node is its own process. All data frames pass through the supervisor,
which routes by a table compiled from the maps. Nodes never address each
other; they publish on output indices and receive on input indices. Tools
and the future GUI are clients of the same hub.

## Consequences

- One extra local hop per frame, on the order of microseconds. Acceptable at
  chunk granularity.
- Recording, `echo`, statistics, late-subscriber header replay and the GUI
  feed fall out of the hub for free.
- Peer-to-peer or shared-memory paths can be added later behind the SDK
  because nodes have no knowledge of the topology.
- The supervisor is a single point of failure for a session; that is also
  true of ROS 1's master and is accepted for a single-machine system.
