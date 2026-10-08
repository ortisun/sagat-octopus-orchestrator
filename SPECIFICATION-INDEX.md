# Executive index — Sagat Octopus Orchestrator Spec Kit

## What will be built

Rust engine for durable workflows, scheduler and journal, Saga orchestration, events/webhooks/Kafka/NATS/SQS, sandboxed inline scripts in TS/Python/C#/Go/Java, SDKs in the same five languages, React Flow/Monaco UI with execution graph, multi-tenant security, OTel observability, GitOps and local experience.

## How to navigate

- **Vision:** `README.md`, `docs/architecture/overview.md`.
- **Principles:** `.specify/memory/constitution.md`.
- **Critical semantics:** `docs/architecture/execution-semantics.md`, `worker-runtime-protocol.md`, `sagas.md`, `storage-and-events.md`.
- **Visual and SDK experience:** `docs/architecture/studio-ux.md`, `workflow-ir-and-sdk-modes.md`, `sdk-contract.md`.
- **Security and operations:** `docs/architecture/security-and-sandbox.md`, `observability-slo.md`, `capacity-and-topology.md`.
- **Per-feature plan:** `specs/{001..012}-{slug}/spec.md`, `plan.md`, `tasks.md`, `research.md`, `data-model.md`, `quickstart.md`, `checklists/requirements.md`.
- **Contracts:** `specs/001-platform-foundation/contracts/`.
- **Tasks:** `specs/*/tasks.md` have IDs, paths and tests to direct agents.
- **Example:** `examples/payment-saga.workflow.json`, `examples/sdk-parity.md`.

## Recommended implementation sequence

1. 001 Foundation: tenant/auth/API/proto/migrations.
2. 002 Durable Engine: 2 steps, transactions, leases, outbox, recovery.
3. 009 Security baseline: isolation, tokens, RBAC, sandbox constraints (gate for inline code).
4. 003 Sagas and 004 Events: develop in parallel after stabilizing the engine.
5. 005 Inline Scripts and 006 SDKs: Rust/SDK protocol + sandbox builds and runtimes.
6. 007 Studio: draft/publish, Monaco, execution DAG and timeline.
7. 008 Observability, 011 GitOps and 012 DX.
8. 010 Scale/HA: use of benchmarks to guide refactors.

## Pending before development starts

- Confirm ownership and delivery model of the SDKs in the MVP (contract parity is mandatory; rollout can be staggered).
- Choose the reference provider for identity, secrets, storage and CI/CD.
- Pin exact Rust/Node/Go/.NET/Python/Java versions and libs after compatibility spikes.
- Define the hardware/testbed to prove the performance objectives.
- Approve financial and compensation policies per application domain.

## Commitment terminology

`MUST` = design requirement. `V1` = first full implementation of the specified scope. `P2/later` = explicit roadmap. No functional code/product is included; only specifications, contracts and illustrative examples.
