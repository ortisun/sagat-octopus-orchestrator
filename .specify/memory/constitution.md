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
idempotency key or a reconciliation lookup. Unknown external outcomes default to consultation or
human review, including compensation outcomes and stop/drain paths; absent or false risk metadata
MUST NOT authorize repeat or defensive compensation. An unknown original effect MUST be resolved
by consultation or an authorized, audited human decision before it is repeated.

A compensation asks the provider to undo what was done; it is a step of its own, planned in reverse
dependency order, using the same lease, fencing, retry, deadline and audit machinery. Defensive
compensation of an unknown original effect, or safe repetition of an unknown compensation outcome,
requires an explicit version-pinned integration qualification: authoritative provider guarantees,
adapter tests, stable effect identity and current authorization. It MUST cover applied, absent and
late original effects and pending, unknown or late compensation outcomes. Acceptance is not finality:
only an authoritative final outcome can establish automatic compensation success. Existing
authorized, audited human resolution may also conclude a workflow as compensated, retaining human
provenance; it MUST NOT be presented as provider-confirmed settlement or qualify an integration.
A missing or invalid qualification refuses the exception and preserves consultation/review; the platform validates the
qualification reference and authority, not the truth of a third party's guarantee.

An effect that cannot be undone ends the run `PARTIAL`. Compensation failure defaults to an
authorized, audited human decision. Only an explicit saga's failure policy in its immutable
published version may select `fail` to end the run
`COMPENSATION_FAILED` for known permanent failure after its retry policy or known non-retryable
failure. Unknown, accepted or pending compensation outcomes MUST instead follow consultation/review
or the qualified safe-repeat rule; exhausting attempts MUST NOT discard that uncertainty. Terminal
failure retains state and step/history evidence, without promising an active post-terminal review queue or declaring the effects resolved. Cancellation and deadlines
MUST NOT replace a pending compensation review or discard unresolved-effect evidence. Independent
compensations remain eligible and dependencies retain their order. Terminal failure MUST wait for
all eligible independent planned compensations to settle and blocked dependents to be terminally
suppressed; a terminal run MUST have no active, runnable or review-waiting compensation.

*Rationale:* a provider that does not deduplicate can still do an effect twice, and the product
must say so rather than hide it. Sources: ADR-0004, ADR-0005, ADR-0010, ADR-0076, feature 037, AGENTS.md rule 4.

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
its lease allows. No language or compiler counts as a sandbox by itself. Credentials, SQL,
messages and audit retain tenant scope. Tenant and namespace identifiers MUST NOT be metric
labels; authorized logs, spans and durable queries provide tenant-specific diagnosis. Another
tenant's resource is indistinguishable from a missing one and the attempt is audited.

Secrets in workflow definitions are logical references, resolved at execution time under current
authorization and a current task-scoped capability. Secret values MUST NOT reach workflow state,
the journal, logs, audit records, telemetry or ordinary management/read API responses. The
executor-read exception is limited to two declared sets:

- Ordinary task execution receives only the secrets declared by its immutable task definition,
  all or nothing. The authenticated caller's current membership, worker/activity binding,
  pool-delivery policy and the workflow
  publisher's permission to use every declared secret MUST authorize that disclosure.
- A managed database capability may disclose only the selected credential of its pinned
  connection revision to the lease-owning `database-runner` with `database:execute`, under the
  current publisher/initiator and secret-access rules of that capability. This disclosure
  exception does not itself authorize database execution or relax the isolation rule above.

Both paths MUST validate current tenant/namespace scope, exact task/instance/attempt identity,
generation, matching capability token and unexpired lease. Ordinary delivery uses that token as
a bearer capability together with the current caller's authorities above; it does not additionally
bind disclosure to the original lease-owning worker session. Managed database delivery MUST also
verify the recorded lease-owning executor subject and live session. Revalidate those
authorities and the selected input/secret revision after provider I/O; stale authority or a
selection that no longer matches its capability refuses disclosure. A value-free disclosure audit
MUST commit before returning values. A failed required check or audit refuses disclosure.
This exception grants neither general secret reads nor permission to reveal classified-secret
payload fields. Authentication and authorization fail closed.

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
they act. The selected internal notification transport carries hints/references, never the only
record of a task. NATS JetStream and the Redis Streams alternative in ADR-0069 remain subordinate
to PostgreSQL; transport acceptance does not grant a worker lease or prove an external effect.
A reconciliation loop re-emits the outbox and finds ready tasks that were never delivered. A
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
  PostgreSQL as authority; NATS JetStream notifications with the configurable Redis Streams
  alternative governed by ADR-0069; graph-native declarative IR; single region with high
  availability. NATS remains supported. This does not approve arbitrary transports, changes to
  delivery authority or removal of the broker from a production profile. Other baseline changes
  require a new ADR (Governance).
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

**Version**: 3.0.0 | **Ratified**: 2026-10-11 | **Last Amended**: 2026-11-01

Amendment scope and preserved obligations: [3.0.0 Sync Impact Report](../../docs/governance/constitution-3.0.0-sync-impact.md). Previous amendment (2.0.0, Principle III): [compensation safety Sync Impact Report](../../docs/governance/compensation-safety-sync-impact.md). Proposed: adopted only when the owner approves this text and its pull request merges.
