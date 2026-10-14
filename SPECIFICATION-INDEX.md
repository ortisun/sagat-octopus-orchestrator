# Executive index — Sagat Octopus Orchestrator Spec Kit

## What is built and what remains

Features 001–012 have implementations and automated evidence: a Rust durable-workflow engine and journal, Saga orchestration, events/webhooks plus inbound Kafka/NATS/SQS, sandboxed TypeScript/Python/C# scripts, SDKs in TypeScript/Go/Python/C#/Java, React Flow/Monaco Studio, multi-tenant governance, OTLP/HTTP JSON observability, GitOps/DX, scheduler sharding/fairness, Helm, fault/load suites and a DR drill.

The requirement-level authority is `docs/traceability.md`. Material gaps include `foreach`/`humanApproval`, blob-backed payloads, Go/Java script runtimes and arbitrary dependency catalogs, outbound customer broker producers, storage/log retention quotas, KMS-backed secret keys, registry/package releases, journal partitioning, queue quotas, 5 000 transitions/s, human studies and a full product/KEDA cluster rollout. MIT was selected on 2026-10-11; package metadata and publication verification remain current work.

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

## Delivered dependency sequence

1. 001 Foundation: tenant/auth/API/proto/migrations.
2. 002 Durable Engine: transactions, leases, outbox and recovery.
3. 009 Security baseline: isolation, tokens, RBAC, secrets, quotas, bindings and egress.
4. 003 Sagas and 004 Events.
5. 005 Inline Scripts and 006 SDKs.
6. 007 Studio.
7. 008 Observability, 011 GitOps and 012 DX.
8. 010 Scale/HA, followed by completion work that integrated C# sandboxing, Studio/DX and Helm network isolation.

## Pending product and validation decisions

- Choose registry ownership, signing and release policy for the five SDK packages and the `sagat` binary.
- Choose production identity/KMS/object-storage providers and the retention/classification tiers.
- Define a distributed performance testbed that separates workers, API and PostgreSQL; the checked-in reports are single-machine evidence and do not prove 5 000 transitions/s.
- Run a full product/KEDA deployment, node-drain exercise and real dependency-protocol checks; current cluster evidence covers NetworkPolicy canaries and restricted Pod admission only.
- Conduct the newcomer timing and human accessibility studies that automation cannot replace.
- Approve financial and compensation policies per application domain.

## Commitment terminology

`MUST` = design requirement. `V1` = first full implementation of the specified scope. `P2/later` = explicit roadmap. A checked task names implementation evidence, but it does not erase an explicit partial/open requirement in `docs/traceability.md` or a gap in the feature's `research.md`.
