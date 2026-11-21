# Architecture

How the platform is built. These pages describe the system as a whole; a single feature's design is in
[`../designs/`](../designs/README.md) or its specification, and a single hard-to-reverse choice is in an
[architecture decision record](../adr/README.md). Start with the overview.

| Page | What it explains |
|---|---|
| [overview.md](overview.md) | The planes and their boundaries, the bounded contexts, the key invariants and the map of the repository |
| [execution-semantics.md](execution-semantics.md) | Durable execution: run and step states, what one transaction holds, step identity, loops, children, timers |
| [worker-runtime-protocol.md](worker-runtime-protocol.md) | How a worker claims, heartbeats and completes work: leases, generations and fencing |
| [workflow-ir-and-sdk-modes.md](workflow-ir-and-sdk-modes.md) | The authoring model: the workflow document (IR), the SDKs, inline scripts and the execution graph |
| [sagas.md](sagas.md) | The formal model of sagas, compensation and reconciliation |
| [storage-and-events.md](storage-and-events.md) | Persistence in PostgreSQL, messaging, the inbox and outbox, and the connectors |
| [security-and-sandbox.md](security-and-sandbox.md) | Security, tenant isolation and the script sandbox, with the threat model of each trust boundary |
| [sdk-contract.md](sdk-contract.md) | What every SDK must do the same way |
| [observability-slo.md](observability-slo.md) | Telemetry, service-level objectives and day-to-day operations |
| [capacity-and-topology.md](capacity-and-topology.md) | Deployment topologies, capacity, disaster recovery and the benchmark |
| [reliability-operational-playbooks.md](reliability-operational-playbooks.md) | A catalog of operational scenarios and the human action each one needs |
| [studio-ux.md](studio-ux.md) | The Workflow Studio: its user journeys and screens |
