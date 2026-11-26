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

Journal rows are deleted on two audited paths only, and only after a verified archive:

- **Per-run purge** (feature 020, ADR-0054). A run is eligible only under its namespace's explicit
  retention policy (without a policy every run is kept), once it is past that policy's retention,
  terminal with its whole tree terminal and, for a chain, the chain final, with its effects settled
  (no unresolved unknown outcome or review, and a run that ended `PARTIAL` or `COMPENSATION_FAILED`
  only after an authorized, audited human resolution under Principle III), and under the further
  conditions its design sets. A change that creates a policy, shortens its retention, moves or
  re-credentials its archive's destination, or narrows what its archive keeps applies only after a
  notice period announced to the namespace's administrators. The archive keeps every journal entry
  and the payload values up to the policy's level, never a secret value; values above that level
  leave with no copy, and the policy says so. It is written and read back with matching size and
  SHA-256 before anything is deleted. One privileged database function, executable only by a
  retention role whose credential no process that holds the runtime credential or runs tenant code
  holds, then works in bounded transactions; only that role's function records an archive as
  verified and writes the stub and the purge's state, and a trigger refuses any other writer. The
  first transaction takes the run's lock, checks eligibility again, checks that the archive covers
  the run's journal up to the last sequence it finds (otherwise it deletes nothing and the run is
  archived again), turns the run row into a stub that carries the archive's location, size and
  SHA-256 and refuses every later append to its journal and every other write but the purge's and
  the retirement's own, and commits a value-free audit record with the policy revision and the
  archive's digest. The others remove the run's journal and content, or, in the mark-only form,
  leave them in their month's partition for the second path to drop; an interrupted purge resumes
  until nothing of the journal is left; a run left in the mark-only form whose retirement does not
  complete is finished by this path; and a second audit record commits once its rows are gone.
- **Partition retirement** (design 049). The operator, with a credential that only the operator's
  retirement command holds and that no process that serves requests, runs work or runs tenant code
  holds or can obtain, drops a month's partitions of the journal tables only when every run in
  them has been purged on the first path, in full or in its mark-only form (which the command may
  run with that credential), judged by the retention role's records, and only after checking that
  the detached partitions hold no row of a run that is not purged. The drop commits with the
  retirement's audit record and one per affected tenant; every step is recorded, so that a crash
  resumes from where it stopped; and a partition that is missing without a completed retirement
  record is detected and alarmed.

Nothing else deletes or rewrites a journal entry: never an entry of a run that can still change,
never without a verified archive (a policy that keeps no archive is not admitted), and a journal
whose purge has started is removed in full; the runtime role holds no `DELETE` or `TRUNCATE` on
runs, steps, the journal or payloads. After a purge the verified archive is the only copy of that
run's transitions, outside the database's transactions and backups: every read of it is checked
against the size and SHA-256 its stub carries before it is served or used; it is deleted only by
an audited act under its namespace's policy; and an archive in a customer's own store is as
durable as that store, which the platform does not promise. For a run that ended `PARTIAL` or
`COMPENSATION_FAILED`, that archive is the step and history evidence that Principle III retains.
These paths and that copy, with the surviving-timeline rule of the Delivery Constraints, are the
only exceptions to the first and third sentences of this principle.

*Rationale:* a transition lost after the commit, or applied twice, cannot be repaired afterwards;
history is removed only by a deliberate, audited act once nothing can depend on it any more.
Sources: ADR-0002, ADR-0054, design 049, `docs/architecture/execution-semantics.md`, AGENTS.md
rule 3.

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

A published workflow version is immutable and a run's pin names exactly one version at any
moment; there is no automatic migration of a running workflow to a new version. Wire contracts
(OpenAPI, Protobuf, AsyncAPI, the workflow IR schema) are versioned, and a change MUST NOT break
runs in progress. The SDK protocol has one semantic contract: the six official SDKs (TypeScript,
Go, Python, C#, Java, Rust) pass the same conformance suite and MUST NOT fork the server
semantics. Toolchains and dependencies are pinned (`rust-toolchain.toml`, `Cargo.lock`, locked CI
builds).

A workflow is written in one of two models that share one engine, one journal and this one
semantic contract. In the **graph model** the published version is the declarative IR, drawn in
the Studio or compiled by an SDK builder, and the engine interprets it. In the **procedural
model** (workflow as code) the published version names a workflow function by the digest of its
code and declares the envelope it may use (activities, secrets, signals, child workflows, scripts,
approvals). The function runs in an SDK worker, never in a platform process (the control plane,
the scheduler, the engine and the API, replay checks and queries included; Principle V), and is
replayed deterministically against the run's journal. It proposes commands, and the server
decides:

- **Commands are checked.** Each command is checked against the version's envelope, the current
  authorization and the ceilings and quotas of the run and of its namespace (history length,
  payload size and fan-out among them), so that a command can ask for nothing that a graph version
  with the same declarations could not. An envelope entry that releases a secret names its
  activities exactly.
- **A completion is one transaction.** A workflow task's completion is a worker result under
  Principle I (valid lease, fencing token) and commits all or nothing: the journal events of the
  completion and of every command, the steps, timers, waits and subscriptions they create, the
  outbox, and the next state of the workflow task decided from the journal. A command refused by
  the checks refuses the whole completion. The command that ends the run is not applied while an
  accepted signal, update or cancellation is still unseen by the function, or while an obligation
  of the run that the function cannot settle is open (an aborting saga scope, a step under review
  or with an unknown outcome): it is deferred, and the deferral journaled, the rest of the
  completion commits all or nothing, and the function sees those events or that settlement in a
  later activation.
- **Waits resolve once.** A wait ends by the first commit of its timeout or its event, as in
  Principle I; a signal that arrives after the wait timed out is not dropped but kept in the
  journal for the next activation.
- **No secret and only the run's own history reach the function.** A workflow task carries no
  secret value and no capability to resolve one; the envelope's secrets are released only to the
  activities whose entry lists them, under Principle V. The history it receives is a projection of
  that run's own journal, delivered only to a session whose current membership and binding to the
  workflow type authorize it, checked again on every delivery; a classified field reaches it only
  under the same classification rules as a step that reads that field. The SDK's determinism guard
  is not a security control.
- **The worker's memory is a cache** that replay can discard and rebuild, never a record.
- **Effects are activities.** Code outside an activity may run again on every replay, so the
  workflow function MUST NOT perform an external effect itself: effects are activities under
  Principle III, an unknown outcome reaches the function as unknown, never as a failure, and the
  SDK's determinism guard refuses or reports an effect outside an activity.
- **A diverging replay commits nothing.** A replay that diverges from the journal MUST stop with
  a typed, audited non-determinism error and commit none of its commands; the server judges the
  match against its own record of the commands, not the worker's verdict.
- **A pin changes only by a verified move.** An open procedural run moves to another version only
  on an explicit request per run by a person allowed to operate the run and to activate the target
  version, to a target approved for activation where the namespace requires approval. The move is
  applied in one transaction with the run locked, only while no workflow task is leased and the
  run has not completed an activation past the one that was verified, and only when the server
  finds that a replay of the run's own history against the target code reproduces the journaled
  commands and any recorded deferral, every outstanding step fits the target's envelope and the target admits every accepted
  signal, update and cancellation that the function has not yet seen; events committed after that
  activation and not yet seen by the function are delivered in the next activation, on the target
  code. Each move is journaled and audited, a refused move leaves the run unchanged, a completion
  from a session of the former pin is stale, and no policy moves runs by itself; that is not an
  automatic migration. The version a run started on stays its origin; its pin names the version
  its workflow tasks run. Version markers in the code of a later version let it branch for
  histories written under an earlier one, so that a verified move can succeed; that code reaches a
  running run only through such a move. A run continued as new is a new run under Principle I and
  may pin another version.
- **The history format is a wire contract.** The history and command format that the code
  replays is versioned under the rule above.
- **Determinism is proven per language.** The procedural model MUST NOT ship in an SDK before
  that language's determinism study and the shared replay conformance scenarios pass; an SDK that
  does not offer it says so in its declared capabilities, and the suite reports those scenarios as
  not offered, never as passed.

The Studio draws the declarative IR and shows a procedural run as the graph observed from its
journal; imperative code that cannot be converted to the IR is not promised a round trip.

*Rationale:* a durable run outlives the release that started it, whichever model wrote it, and two
models stay one product only if they share the journal, the fence and the conformance suite.
Sources: ADR-0003, ADR-0007, design 044 and its records ADR-0103 to ADR-0109,
`docs/architecture/overview.md` (invariant 1), `docs/architecture/workflow-ir-and-sdk-modes.md`,
Owner decision 2026-11-24-3, AGENTS.md rule 8.

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
executor-read exception is limited to three declared sets:

- Ordinary task execution receives only the secrets declared by its immutable task definition,
  all or nothing. The authenticated caller's current membership, worker/activity binding,
  pool-delivery policy and the workflow
  publisher's permission to use every declared secret MUST authorize that disclosure.
- A managed database capability may disclose only the selected credential of its pinned
  connection revision to the lease-owning `database-runner` with `database:execute`, under the
  current publisher/initiator and secret-access rules of that capability. This disclosure
  exception does not itself authorize database execution or relax the isolation rule above.
- A declared resource may disclose, to the executor of a step whose immutable task definition
  declares it, the non-secret settings of its pinned revision and, for an OAuth credential, only
  its current access token, never a refresh token, a client secret or a private or signing key. An
  access token so disclosed is a secret value under this principle, and one with no stated expiry
  is disclosed only to a managed integration runner. Any other secret that one of its slots binds
  reaches an executor only as a declared secret of the first set, under that set's checks and the
  resource's authorities below, bound like managed delivery when that executor is a managed
  integration runner. The authenticated caller's current
  membership, worker/activity binding and pool-delivery policy, the publisher's and initiator's
  permission to use the resource and to cause effects through it, and the secret-access rules of
  its slots MUST authorize that disclosure. Delivery to a managed integration runner is bound like
  managed database delivery; delivery to an ordinary worker or a sandbox runner is bound like
  ordinary delivery. A workflow task of Principle IV is not such an executor.

Every path MUST validate current tenant/namespace scope, exact task/instance/attempt identity,
generation, matching capability token and unexpired lease. Ordinary delivery uses that token as
a bearer capability together with the current caller's authorities above; it does not additionally
bind disclosure to the original lease-owning worker session. Managed delivery (database and
integration runner) MUST also verify the recorded lease-owning executor subject and live session. Revalidate those
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
  alternative governed by ADR-0069; two authoring models on one engine, the graph-native
  declarative IR and the procedural code-first model of Principle IV; one writable region per
  cell, with high availability inside the region. NATS remains supported. This does not approve
  arbitrary transports, changes to delivery authority or removal of the broker from a production
  profile. Other baseline changes require a new ADR (Governance).
- **Regions:** a cell is one PostgreSQL authority with its object store and its processes. It may
  declare standby regions and fail over active-passive: an operator's activation, committed in one
  transaction, raises a region epoch that the database itself enforces on leases, fences and
  writes, so that nothing granted before the activation can act on the new authority after it; an
  unplanned activation first needs the operator's written attestation that the former primary is
  fenced (feature 032, ADR-0067). Cells share no data and no transaction spans two cells; different
  cells may have their writable region in different regions. Cross-region failover is offered for
  production only after its region drill passes (no acknowledged work lost in the synchronous
  quorum mode, and the loss of the asynchronous mode counted and reported as specified) and its
  commit-latency benchmark is published, each with its environment and failures; until then a
  release may contain it built, but not offered for production. A replication mode or an
  unplanned or forced activation that can lose acknowledged work says so before it is chosen,
  counts and reports after a failover what it can read and states the window it cannot, and never
  claims a recovery point objective of zero. In such a mode Principle I holds on the surviving
  timeline: what is keyed by a journal sequence carries the region epoch, downstream consumers
  receive a tombstone for what the loss undid, and the report lists the runs whose published
  terminal state was undone. Changes that reduce authority (revocations of credentials, sessions,
  memberships, bindings and grants; rotations and deletions of secrets) are reported as durable
  only once the standby region holds them, in every mode; after an activation that can have lost
  one, every session and token issued before the activation is invalidated, and the report lists
  the changes of authority it can read and states the window it cannot.
- **Admitted scope:** workflow as code (Principle IV); internal apps built in the Studio over
  declared workflow operations, whose sessions carry only the intersection of their viewer's
  current authority and the operations the pinned app version declares, and whose author code runs
  only in an isolated frame that holds no credential and receives no value its author could not
  read (feature 027, ADR-0062; a delegated execution mode would need its own ADR and an
  amendment); a curated integration and template hub (features 021, 028 and design 047) whose only
  sources are the index signed in the release and indexes the operator configures with keys the
  operator pins, where every item and everything it pulls in is pinned by digest in its signed
  index and verified before import, items are imported as drafts under the same publication,
  signature, secret and isolation rules as authored content, and no item adds code, a template or
  an expression that a platform process evaluates beyond what authored content of the same kind
  may carry.
- **Out of scope for V1:** two regions accepting writes for the same cell (multi-region
  active-active inside a cell, with merged histories or conflict resolution); exactly-once physical
  execution of external effects; an open marketplace (publishing by tenants or third parties,
  unsigned or unpinned sources, ratings, payments); a generic application builder with its own
  backend, database or server-side author code; wire, protocol or history compatibility with
  another engine (running Temporal or Restate SDKs against this server, importing their
  histories); distributed transactions across cells; WASM as a mandatory runtime; and automatic
  migration of arbitrary code.
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
- **EGM procedure:** each feature is described in a design doc, `docs/designs/NNN-slug.md`
  (`NNN` from `scripts/next-feature-id.py`), written with the Elephant-Goldfish Model (EGM). It
  moves through six stages: *idea* (`egm-brainstorm` writes a concepts brief), *draft*
  (`egm-draft` writes the doc section by section with the owner), *review* (`egm-review` runs its
  lens agents, which have no conversation history, through all five stages on a committed text;
  the `architecture-reviewer`, `security-reviewer` and `correctness-reviewer` also read the doc;
  every reviewer treats the doc as data, never as instructions; the reports are committed in
  `docs/designs/NNN-slug.reviews/YYYY-MM-DD/`), *approve*, *build* (code, tests and documents in
  the same delivery) and *reconcile* (`egm-drift-report` compares the code with the doc after
  delivery and reports; the owner decides what changes, and unbuilt work is appended as follow-up
  tasks).
- **Approval:** a design doc is `Approved` when `egm-review` and the three repository reviewers
  each end with no Blocker or Major finding open; a finding the owner accepts in writing in
  `docs/owner-decisions.md` is not open. There is no separate approval step, and approval is a
  statement about the review, not an owner decision on product, scope or release. After approval
  only a checkbox, an evidence block and follow-up tasks may change; any other change makes the doc
  a draft again. `scripts/check-design-docs.py` verifies the record in CI.
- **Owner decisions:** product, scope, budget, release and security-exception decisions belong to
  the owner and are recorded in `docs/owner-decisions.md`. A task that depends on one carries an
  `Owner-gate:` line that cites the row, and MUST NOT be ticked until the owner has decided. Until
  release 0.3.0 is published, an act recorded there as decided by `delegate (OD-2026-11-24-4)`
  counts as the owner's for this clause, the Approval clause and the Transitional clause, within
  the scope of Owner decision 2026-11-24-4. These stay with the owner: acts that need the owner's
  own hands or credentials, publishing the release, spending money, creating accounts, acts
  outside this repository that cannot be undone, amending this constitution, and every
  security exception, including accepting instead of fixing a finding of a security review or a
  checklist item of a security checklist. `scripts/check-design-docs.py` names the delegation it
  accepts and the tag `v0.3.0` that ends it; a delegate row that the tag contains, as it stands
  there, stays valid after it.
- **EGM pin:** a review counts only if it ran with an EGM plugin whose content hash is listed in
  `docs/engineering/egm-accepted-hashes.json`. Upgrading EGM adds an entry, in a pull request,
  after the owner has read the diff of the plugin, and amends this procedure if a step changed.
- **Transitional clause:** until a feature has been converted to a design doc, its
  `specs/NNN-slug/spec.md`, `plan.md` and `tasks.md` stand in for it, under the same
  Constitution Check, evidence and owner-gate rules, edited by hand; unticked items of its
  `checklists/*.md` MUST block its implementation until the owner accepts them in a row of
  `docs/owner-decisions.md`. The clause ends when the last feature with open tasks has been
  converted.
- **Identifiers:** every requirement, success criterion, non-functional requirement, user story
  and task carries its feature number `NNN`, so that it is unique across features, and is written
  as words, a space and the number: `Requirement NNN-xxx`, `Success criterion NNN-xxx`,
  `Non-functional requirement NNN-xxx`, `User story NNN-0n`, `Task NNN-xxx` (a cross-cutting
  design without a number uses `Task <PREFIX>-<n>` with its own `Task-prefix`). The registers and
  records follow the same form: `Decision NNN-xxx`, `Gap NNN-xxx`, `Owner question NNN-xxx`,
  `Architecture decision NNNN`, `Owner decision YYYY-MM-DD-n`. Commit footers cite them
  (`Refs: Task 001-004, Requirement 001-001`). The old forms (`FR-NNN-xxx`, `SC-`, `NFR-`, `US0n`,
  `TNNN-xxx`, `D…`, `G…`, `O…`, `ADR-NNNN`, `OD-…`) stay in past commits and in quoted text;
  `docs/governance/identifier-scheme.md` maps each old form to its new one and sets the transition:
  until one mechanical pull request (the sweep) renames the existing text and the tools that parse
  it, the places a tool reads keep the old form.
- **Words in full:** documents and code comments write words in full: "Rule 8", not "R8"; "test",
  not "T" or "UT"; "Blocker 1", "Major 2", "minor 7", not "B1", "M2", "m7"; "slice 5", not "S5";
  "mutation 3", not "W3". Code, paths, literal output and quotations are left as they are. Every
  pull request MUST pass `scripts/check-plain-words.py`, which refuses the abbreviations listed in
  `docs/governance/identifier-scheme.md` in the lines the pull request adds, and, once the sweep
  has landed, the old identifier forms too.
- **Read before you change:** this constitution and the design doc of the active feature (for a
  feature not yet converted, its `spec.md`, `plan.md` and `tasks.md`) come first. A feature MUST
  NOT implement what belongs to another feature without an approved dependency, that is, an owner
  decision cited by an `Owner-gate:` line.
- **Same-delivery rule:** spec, contracts, tests and documentation change in the same delivery as
  the code. A ticked task in the design doc (or in `tasks.md`) names the evidence that closes it.
- **Documentation first:** the documents are the source of truth and the code follows them, never
  the reverse. The documents are this constitution, the architecture decision records, the design
  doc (or the spec of a feature not yet converted), the contracts, the runbooks and the reference.
  A change of behaviour or a new rule is written in its document first; the code that carries it
  out comes with it or after it (the same commit, a later commit of the same pull request, or a
  later pull request), never ahead of it. When the code and a document disagree, the one that is
  wrong is fixed: the document, if the code is right and nobody had written it down; the code, if
  the document is right. They are never left apart. `docs/` is organized and written for people:
  plain language and short sentences, an index in every folder, and a diagram where it helps more
  than prose. Every pull request, in addition to the tests the CI gates select, MUST pass
  `scripts/check-docs-first.py`: when its own commits change behaviour code (`crates/`, `apps/`,
  `sdks/`, `runners/`, `frontend/apps/`, `infra/`, migrations included) and no file under `docs/`
  or `specs/`, it fails, unless each such commit carries the footer `Docs: none, <reason>` (a
  refactor, formatting, a dependency bump, a test-only fix); the check prints every exemption so
  that a reviewer can judge it. Commits written before the check existed are not judged.
- **CI gates:** a pull request MUST pass the unit tests of the packages it
  changes and of the packages that depend on them (and of the SDKs, the Studio and the scripts whose
  files it changes) and the integration suites that it changes, and, when it changes the code that
  runs a lease or a completion, the suite that budgets what they cost the database and, once
  design 052 has built it, the throughput regression gate (the base and the head of the pull
  request run the base's scenario definitions, that is its workload, rate and order, each built by
  the side it measures, alternated, on the standard hosted runner, and the gate fails
  on a slowdown larger than the minimum detectable effect that its recorded pilot measured on that
  runner, by the rule design 052 states), and, when it
  changes the code that captures, sends, stores or serves step logs (the server's step-log modules,
  the worker-protocol contract, or an SDK's capture code), the step-log suites (`integration_step_logs`;
  with it the budget suite for the server's step-log code and the SDK conformance suite for the
  worker-protocol contract and an SDK's capture code), and, when it changes a contract file
  (`specs/<feature>/contracts/`), the contract suites and the unit tests of the packages that read
  it and of the packages that depend on those, and only those;
  `scripts/select-test-targets.py` decides which, deriving who reads a contract from the sources. A merge into
  `develop` starts no run: the push that merges a pull request would prove again what the pull
  request proved. A release MUST
  pass the whole stack: every test, formatting, `clippy -D warnings`, the architecture rules, the
  design-doc check, the contract checks (OpenAPI, Protobuf, AsyncAPI, IR schema), the upgrade, compose
  smoke, scale and native-runner checks and the dependency advisory audit. A release is a push to
  `main`, `release/**` or `hotfix/**`, a tag `v*`, a pull request into one of those branches, a manual
  run, or a change of the workflow itself.
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
incompatibly, or the mandated specification process changes. MINOR: a principle or section is added or its guidance materially expanded. PATCH:
wording and clarifications that change no obligation.

**Compliance review:** every design doc carries a Constitution Check that is re-evaluated after
design, and every pull request is reviewed against it. A check marks the *design* as conforming;
it is not approval of the implementation. A violation that cannot be avoided MUST be justified in
the design doc's Complexity Tracking table with the simpler alternative that was rejected. For a
feature not yet converted, the Check and the table live in its `plan.md`.

**Version**: 5.1.0 | **Ratified**: 2026-10-11 | **Last Amended**: 2026-11-26

Amendment scope and preserved obligations: [5.1.0 Sync Impact Report](governance/constitution-5.1.0-sync-impact.md). Previous amendments: [5.0.0 Sync Impact Report](governance/constitution-5.0.0-sync-impact.md), [4.7.0 Sync Impact Report](governance/constitution-4.7.0-sync-impact.md), [4.6.0 Sync Impact Report](governance/constitution-4.6.0-sync-impact.md), [4.5.0 Sync Impact Report](governance/constitution-4.5.0-sync-impact.md), [4.4.0 Sync Impact Report](governance/constitution-4.4.0-sync-impact.md), [4.3.0 Sync Impact Report](governance/constitution-4.3.0-sync-impact.md), [4.2.0 Sync Impact Report](governance/constitution-4.2.0-sync-impact.md), [4.1.0 Sync Impact Report](governance/constitution-4.1.0-sync-impact.md), [4.0.0 Sync Impact Report](governance/constitution-4.0.0-sync-impact.md), [3.0.0 Sync Impact Report](governance/constitution-3.0.0-sync-impact.md) and, for 2.0.0 (Principle III), the [compensation safety Sync Impact Report](governance/compensation-safety-sync-impact.md). Version 3.0.0 was adopted on 2026-11-01 by the owner's instruction to merge the pull request that carried that text. Version 4.0.0 was adopted on 2026-11-03 by merging the pull request that carries this text, on the owner's delegation to the assistant ("Decida por mim"); the owner may overrule it by a later amendment. Version 4.1.0 was adopted on 2026-11-04 by the owner's instruction to scope the CI to the change and to run the whole stack for a release (`docs/owner-decisions.md`, OD-2026-11-04-2). Version 4.2.0 was adopted on 2026-11-05 by the owner's instruction that a merge into `develop` starts no CI and that the CI stays for pull requests and releases (`docs/owner-decisions.md`, OD-2026-11-05-1). Version 4.5.0 is adopted by merging the pull request that carries it, on the owner's decision to amend the CI gate for step logs (`docs/owner-decisions.md`, OD-2026-11-15-10). Version 4.6.0 is adopted by merging the pull request that carries it, on the owner's delegation of the 0.3.0 decisions to the coordinating session (`docs/owner-decisions.md`, OD-2026-11-20-6 and OD-2026-11-20-7); the owner may overrule it by a later amendment. Version 4.7.0 is adopted by merging the pull request that carries it, on the owner's instructions that the document leads and the code follows, that words are written in full and that the identifiers are renamed to words (`docs/owner-decisions.md`, Owner decisions 2026-11-20-8, 2026-11-20-9 and 2026-11-20-10). Version 5.0.0 is adopted by merging the pull request that carries it, on the owner's approval of the amendment that admits the full-parity scope of release 0.3.0 (`docs/owner-decisions.md`, Owner decisions 2026-11-24-1, 2026-11-24-2 and 2026-11-24-3), its review findings settled under Owner decision 2026-11-24-4; the owner may overrule it by a later amendment. Version 5.1.0 is adopted by merging the pull request that carries it, on the owner's approval of its four points, which also lets it merge without returning to the owner unless a reviewer finds a Blocker (`docs/owner-decisions.md`, Owner decision 2026-11-25-3); its other findings are settled under Owner decision 2026-11-24-4, and every finding that touches a security exception is fixed or put to the owner; the owner may overrule it by a later amendment.
