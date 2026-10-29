# Sagat Octopus Orchestrator Constitution

## Core Principles

### I. State and Retry Semantics Are Explicit and Durable

PostgreSQL is the source of truth for execution state; no other component holds the only copy of
a transition. A state transition, the scheduling of the next task, a timer or subscription update
and the outbox record that announces them MUST commit in one transaction. The journal is
append-only with a monotonic per-run sequence. A worker's result is accepted only with a valid
lease and fencing token; a stale attempt is rejected, never merged. A terminal run never returns
to running: reprocessing opens a new run or a new compensation segment and keeps the history.
When a timeout and a signal race, the first commit wins and the other is a no-op that is audited.

*Rationale:* a transition lost after the commit, or applied twice, cannot be repaired afterwards.
Sources: ADR-0002, `docs/architecture/execution-semantics.md`, AGENTS.md rule 3.

### II. Modular Domain, Ports and Adapters

The platform is a Rust modular monolith (a crate per bounded context); a process is split out
only after measurement shows the need. The domain crate MUST NOT depend on a web framework, SQL
driver, broker client, Kubernetes client or `serde_json` as a representation of business rules;
serde on wire and adapter types is allowed. Layers depend inwards only (domain ← application ←
adapters ← apps) and the dependency graph MUST stay acyclic. Mappers and DTOs MUST NOT be created
without independent semantics: modules share types and map only at a boundary that needs it.

*Enforced by:* `scripts/check-architecture.py` (CI job `architecture`), which reads
`cargo metadata` and fails on a forbidden or non-allowlisted domain dependency, a forbidden
layer edge or a cycle. *Not enforced by tooling:* the rule against empty mappers (review).
Sources: ADR-0001, AGENTS.md rules 5 and 6.

### III. External Effects Are At-Least-Once; Sagas Compensate, They Do Not Roll Back

The platform MUST NOT promise exactly-once for an external effect. A critical effect requires an
idempotency key or a reconciliation lookup, and an attempt whose outcome is unknown MUST be looked
up before it is repeated. A compensation asks the provider to undo what was done; it is a step of
its own, planned in reverse order, and runs through the same machinery as forward work (lease,
fencing, retry, deadline, audit). An effect that cannot be undone ends the run `PARTIAL`; a
compensation that keeps failing waits for a person with an audited decision and is never assumed
done.

*Rationale:* a provider that does not deduplicate can still do an effect twice, and the product
must say so rather than hide it. Sources: ADR-0004, ADR-0005, ADR-0010, AGENTS.md rule 4.

### IV. Compatibility Is Versioned and Pinned

A published workflow version and its IR are immutable and every run references exactly one
version; there is no automatic migration of a running workflow to a new version. Wire contracts
(OpenAPI, Protobuf, AsyncAPI, the workflow IR schema) are versioned, and a change MUST NOT break
runs in progress. The SDK protocol has one semantic contract: the six official SDKs
(TypeScript, Go, Python, C#, Java, Rust) pass the same conformance suite and MUST NOT fork the server
semantics. The Studio draws the declarative IR; imperative code that cannot be converted is not
promised a round trip. Toolchains and dependencies are pinned (`rust-toolchain.toml`,
`Cargo.lock`, locked CI builds).

*Rationale:* a durable run outlives the release that started it. Sources: ADR-0003, ADR-0007,
`docs/architecture/overview.md` (invariant 1), AGENTS.md rule 8.

### V. Code, Tenant Boundary and Secrets Are Isolated

Untrusted code MUST NOT run inline in the control plane or scheduler process: it runs isolated,
under quotas, with no scheduler credentials, no database access and only the secrets and egress
its lease allows. No language or compiler counts as a sandbox by itself. The tenant is a
mandatory dimension of credentials, SQL, messages, metrics and audit; another tenant's resource
is indistinguishable from a missing one and the attempt is audited. Secrets are references,
resolved at execution time under a capability, and never reach the journal, a log, an audit
record or an API response. Authentication and authorization fail closed.

*Release-blocking if violated:* cross-tenant read or write, sandbox escape, unauthorized script
execution, secret exposure, quota bypass, incorrect webhook signature, replay of a financial
request. Sources: ADR-0006, `docs/architecture/security-and-sandbox.md`, AGENTS.md rule 7.

### VI. Observability and Audit Are Part of the Feature

Every feature emits OpenTelemetry traces and metrics on its critical path, structured logs without
sensitive data, and typed errors (`code`, `retryable`, `traceId`, no payload or secret). Metrics
MUST NOT carry high-cardinality identifiers; those belong in logs and spans. Publish, trigger,
cancel, rerun, manual override, secret access, quota and policy changes are audited. The runtime
graph shown to users is a projection of the persisted journal, never of ephemeral telemetry, and
its lag is displayed.

*Rationale:* an operator must be able to explain a run after the fact from evidence that was
written in the same transaction as the decision. Sources: ADR-0008,
`docs/architecture/observability-slo.md`, AGENTS.md rule 10.

### VII. Events Are Redeliverable; Inbox and Outbox Are the Contract

Every message can arrive twice, late or out of order. Outgoing messages are written to the outbox
in the transaction that caused them; incoming messages are deduplicated through the inbox before
they act. NATS JetStream carries a reference (`task_ref`), never the only record of a task, and a
reconciliation loop re-emits the outbox and finds ready tasks that were never delivered. A
consumer MUST tolerate duplicates, and backpressure and a dead-letter path are part of the design
of every integration (Kafka, NATS, SQS, webhooks).

*Rationale:* broker delivery is a hint; the database is the record. Sources: ADR-0002,
`docs/architecture/overview.md` (invariants 4 and 5), `docs/architecture/storage-and-events.md`.

### VIII. Failure Is Tested and Contracts Are Verified, Honestly

Each feature plans and runs tests for crashes at the critical points, duplicate delivery, the
timeout/signal race, a lost broker delivery, a failed compensation and a restart, besides unit,
integration (real PostgreSQL), contract, security and end-to-end tests. Contracts are linted or
compiled in CI. Test results are real and reported as they are: a failing or skipped test is
stated, a gap is explained, and a number is never invented. A guarantee that is not proven MUST
NOT be faked: a placeholder is marked explicitly as a pending decision or test.

*Rationale:* durability claims are only as good as the failure that was injected.
Sources: `docs/test-strategy.md`, AGENTS.md rules 9, 11 and 12.

## Delivery Constraints

- **Technology baseline (V1):** Rust (Tokio, Axum, tonic, SQLx) with a pinned stable toolchain;
  PostgreSQL as authority; NATS JetStream as transport and notification; graph-native declarative
  IR; single region with high availability. Changing any of these requires a new ADR (Governance).
- **Out of scope for V1:** multi-region active-active, exactly-once physical execution of external
  effects, a connector marketplace, WASM as a mandatory runtime, reimplementing Temporal or
  Restate, and automatic migration of arbitrary code.
- **Targets are not promises:** throughput, latency and SLO figures are benchmark criteria to
  verify on a stated environment (hardware, configuration, workload), not commitments. A benchmark
  report states the environment and the failures, not only the throughput.
- **Security-by-default:** seed data, development credentials and any development token issuer are
  for the development environment only and MUST NOT ship in the production image (FR-001-004;
  gaps G10 and G12 of feature 001).

## Development Workflow and Quality Gates

- **Specification first:** work is traced `requirement → automated test → contract → metrics →
  logs without sensitive data → reproducible run → documentation and a demo of the happy and the
  failure scenario` (the Definition of Done in AGENTS.md).
- **Spec Kit cycle:** each feature lives in `specs/NNN-slug/` and moves through
  `/speckit-specify`, `/speckit-clarify`, `/speckit-plan`, `/speckit-checklist`,
  `/speckit-tasks`, `/speckit-analyze`, `/speckit-implement` and `/speckit-converge`. Features
  001–012 already exist; a new feature continues the sequence from 013.
- **Identifiers:** requirements are `FR-NNN-xxx`, success criteria `SC-NNN-xxx`, non-functional
  requirements `NFR-NNN-xxx`, user stories `US0n` and tasks `TNNN-xxx`, where `NNN` is the feature
  number. This deliberately replaces the template's unscoped `FR-001` and `T001` so that an ID is
  unique across features; commit footers cite them (`Refs: T001-004, FR-001-001`).
- **Read before you change:** this constitution and the `spec.md`, `plan.md` and `tasks.md` of the
  active feature come first. A feature MUST NOT implement what belongs to another feature without
  an approved dependency.
- **Same-delivery rule:** spec, contracts, tests and documentation change in the same delivery as
  the code. A checked task in `tasks.md` names the evidence that closes it.
- **CI gates:** formatting, `clippy -D warnings`, the workspace tests, the architecture rules, the
  contract checks (OpenAPI, Protobuf, AsyncAPI, IR schema), the compose smoke test and the
  dependency advisory audit MUST pass before merge.
- **Branches and commits:** Gitflow and Conventional Commits as defined in AGENTS.md, which is the
  single source for those rules and for any runtime guidance to implementation agents.

## Governance

This constitution supersedes other practices and guidance in the repository. Where AGENTS.md, a
spec, a plan or a runbook disagrees with it, the constitution wins and the other document is
corrected.

**Architecture Decision Records** under `docs/adr/` record how a principle is met. A change that
diverges from an accepted ADR MUST open a new ADR, with benchmark data, failure tests, operational
cost and compatibility with runs in flight, and MUST update the documents that depend on it. The
semantics are never changed silently.

**Amendments** are made by pull request that changes this file, states the version bump and a Sync
Impact Report, and updates the affected plans, runbooks and AGENTS.md in the same change.

**Versioning** follows semantic versioning. MAJOR: a principle is removed or redefined
incompatibly. MINOR: a principle or section is added or its guidance materially expanded. PATCH:
wording and clarifications that change no obligation.

**Compliance review:** every `plan.md` carries a Constitution Check that is re-evaluated after
design, and every pull request is reviewed against it. A check marks the *design* as conforming;
it is not approval of the implementation. A violation that cannot be avoided MUST be justified in
the plan's Complexity Tracking table with the simpler alternative that was rejected.

**Version**: 1.1.0 | **Ratified**: 2026-10-11 | **Last Amended**: 2026-10-27
