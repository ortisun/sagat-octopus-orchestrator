# Implementation Plan: Durable workflow primitives and practical SDK examples

**Branch**: `feature/workflow-primitives` | **Date**: 2026-10-14 | **Spec**: [spec.md](./spec.md)

**Input**: `specs/014-workflow-primitives/spec.md`

Release 0.2.0 follows [ADR-0040](../../docs/adr/ADR-0040-release-checkpoints-preserve-incomplete-feature-boundaries.md) for the
implemented-subset checkpoint. General nesting and the eight open feature-014 tasks remain
pending; publication capabilities, authority checks and required technical gates are unchanged.

## Summary

The owner requested completion of the remaining implementation for release 0.3 on
2026-10-26. [The 0.3.0 implementation ledger](./implementation-0.3.md) records the
ordered remaining increments and actual verification. Existing unchecked tasks and
publication gates remain authoritative until their acceptance evidence is obtained.

Deliver current-API order examples for all five SDKs (US01), then implement bounded dynamic
collections, sequential loops, native approval, switch, declarative Query/Update and run-chain
continuation (US02–US07) through accepted dependency-ordered slices. US01, bounded switch,
run-addressed Query/Update and root approval have public runtime evidence. Chain-addressed Query and
Update have transaction and HTTP evidence, and public Query/Update after a stateful handoff pass the
five-SDK conformance. Direct public continuation transfers the declared state snapshot atomically.
Wider chain semantics (child continuation, wait-entry continuation) are built and tested; wider graphs and the remaining primitive release
tasks stay open until their named tests pass.
The design retains an acyclic immutable IR, adds durable scoped instance identities and reuses the
pure reducer, PostgreSQL transaction boundary, leases, quotas and outbox.

T014-004 publication fixtures use the public HTTP draft validation/publication paths under the
runtime database role. Preserve named-body provenance at the IR adapter and typed expression
byte-limit causes in the domain; the HTTP adapter classifies these without matching message text.
Check schema-first precedence, root graph compatibility, trace identity and absence of a published
version after rejection. Five-SDK authoring/runtime cases remain a separate common contract.

During B0 inventory, keep chained child settlement separate from legacy child-row flag updates.
Prove both cancellation policies and final notification with a database trigger that rejects any
legacy-flag UPDATE of an already-linked child; retain existing ordered handoff races, legacy
children, journal consistency and exact command audits. This does not accept the
complete group transaction design. ADR-0037 separately accepts the existing early stable-parent
identity guard under its immutable ancestry precondition; the remaining B0 inventory stays open.
Align exact-path begin_current_path/begin_path with full-run optional chain-before-run entry.
Prove both Wait/SkipLocked behaviors and same-transaction lock lifetime with PostgreSQL barriers;
keep snapshot CAS/time/bounded mutations and public shape restrictions unchanged.
Align direct run-addressed declared Updates with the same chain prefix, resolving membership
by tenant/namespace/run rather than assuming run ID equals chain ID. Preserve pinned addressing,
receipt replay and terminal errors; verify public successor updates and lock lifetime.
Reuse the transaction-typed post-lock scheduler-time helper for chain handoff/finalization,
preserving wrong-shard/oversized-generation errors and the later evaluated deadline clock.
Use unchanged-tuple expiry barriers and the actual runtime pool. Collection and loop now have
independent entry barriers, atomic unchanged-state checks, fresh-authority/replay controls and
generation-error compatibility. Collection entry retains one supplied audit via finish_version.
Approval open and genuinely due expiry now have independent unchanged-row barriers; preserve
current-user decision authorization and the non-idempotent opening port behavior.

## Technical Context

**Language/Version**: Rust 1.98.0 from `rust-toolchain.toml`; existing locked TypeScript, Go, Python,
.NET/C# and Java SDK toolchains; do not update toolchains for this feature.

**Primary Dependencies**: Existing Tokio, Axum/tonic adapters, SQLx PostgreSQL store, NATS JetStream,
expression adapter and OTel crates. No new framework and no infrastructure dependency in domain.

**Storage**: PostgreSQL authoritative state/journal/receipts/chain links; authorized claim-check
blob storage for large payloads; NATS carries redeliverable references only.

**Testing**: Existing Rust unit/property tests, real PostgreSQL integration/fault suites and public
five-language SDK conformance. Contract lint/schema compilation and example execution are separate
checks; a static example check is not proof of runtime behavior.

**Target Platform**: Existing single-region Linux containers, local Docker Compose and supported SDK
platforms. No new service/process is necessary.

**Project Type**: Modular monolith with shared contracts and five SDKs.

**Performance Goals**: Measure 10,000 suspended approvals, a 10,000-item collection with concurrency
1/16/256, and nested loops reaching the 100,000-instance ceiling. Report environment, p50/p95/p99
schedule delay, memory, transaction duration, journal bytes, retries and quota rejections. Acceptance
is correctness of limits and zero per-wait worker ownership; no unmeasured latency SLO is promised.

**Constraints**: Bounded declarative operations only; no user code in scheduler/control plane.
Domain uses typed values/ports, not `serde_json` as business semantics. Errors reuse the closed
`Error.code` enum with new `details.reason` values; concurrency reuses `Run.version` naming.
Existing payload limits and tenant quotas apply to snapshots, retained results, state, command
receipts and continuation handoffs.

**Scale/Scope**: Six accepted runtime capabilities plus the immediately deliverable three-workflow
example family. A single approval decision, fail-fast collection policy, finite loop/chain budgets
and same-version continuation define the first increment.

## Constitution Check

These are design checks, not implementation approvals or successful runtime test results.

| Principle | Before research | After design | Evidence / future gate |
| --- | --- | --- | --- |
| I — durable atomic transitions | Pass | Pass | `data-model.md` transaction table; T014-007, T014-040. |
| II — pure domain / inward dependencies | Pass | Pass | Existing reducer and typed instance ids; T014-005, T014-043. |
| III — at-least-once effects / honest compensation | Pass | Pass | FR-014-005; scope drain and FR-003-002 reverse topological compensation with independent collection iterations; T014-017, T014-021. |
| IV — pinned versions / five SDKs | Pass | Pass | Contract compatibility section; T014-006, T014-039, T014-041. |
| V — isolated code / tenant and secret boundary | Pass | Pass | Declarative-only Query/Update, tenant-scoped FKs, RBAC at decision; T014-008, T014-040. |
| VI — OTel, audit, typed errors | Pass | Pass | Contract telemetry/error catalogue; T014-009 and each story's adapter task. |
| VII — inbox/outbox and reconciliation | Pass | Pass | All scheduling/chain writes share transaction; T014-007, T014-040. |
| VIII — failure evidence / verified contracts | Pass | Pass | Fault matrix and unchecked future tests; T014-004, T014-040–044. |

**Governance gate**: T014-002 is accepted through
[ADR-0030](../../docs/adr/ADR-0030-structured-instances-and-continuation-chains.md) and separate
[ADR-0031](../../docs/adr/ADR-0031-declared-operation-result-disclosure.md), with additive-v1
compatibility accepted by [ADR-0032](../../docs/adr/ADR-0032-additive-workflow-ir.md). They accept structured
identity, compensation dependencies, additive v1/worker 1.5, lock order, product defaults and role grants,
and explicit declared-result disclosure. The [original proposal](./adr-proposal-structured-primitives.md)
links to those decisions. Canonical contracts/code and release evidence remain pending; accepting
the architecture does not prove them. Any later divergence requires a new ADR and dependent updates.

## Project Structure

### Documentation (this feature)

```text
specs/014-workflow-primitives/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── adr-proposal-structured-primitives.md
├── contracts/
│   ├── README.md
│   └── conformance.md
├── checklists/requirements.md
└── tasks.md
```

### Existing source boundaries and planned additions

```text
examples/order-processing/{typescript,go,python,dotnet,java}/   # this delivery
examples/order-processing/README.md                           # this delivery
tests/e2e/developer-experience/                              # example integration (existing suite)
crates/domain/src/workflow.rs                                 # later: structured definition rules
crates/domain/src/execution/{state,event,plan,command,error}.rs  # later: instance-aware reducer
crates/domain/src/execution/instance.rs                        # scoped run-local identity
crates/application/src/primitives.rs                          # later: use cases / store ports
crates/infra-ir/src/                                          # later: additive wire mapping
crates/infra-postgres/{src/primitives.rs,migrations/}            # later: durable adapter
crates/api-http/src/{routes/primitives.rs,dto_primitives.rs}    # later: narrow authorized API
crates/api-grpc/src/                                          # later: scoped task wire identity
crates/telemetry/src/{metrics,span}.rs                         # later: emitted observations
sdks/{typescript,go,python,dotnet,java}/                       # later: builders and operations
tests/{contract,integration,fault,e2e}/workflow-primitives/   # later: new suites
tests/contract/sdk-conformance/{scenarios,expected}/          # later: shared public-API parity
tests/performance/reports/workflow-primitives.md             # later: actual measurements
```

Extend `WorkflowDefinition`/`Node`/`NodeKind` and `RunState`/`StepState` where behavior belongs.
The domain `execution/instance.rs` owns the run-local `{path,node}` identity; owning run/tenant/
namespace context belongs to the enclosing application/store. Product “primitives” does not name a
replacement Rust `Primitive` model. Approval, operations and chains may use separately named
cohesive modules; the proposed application/store modules are ports/adapters, never empty mappers.
Exact migration timestamps are allocated at implementation time to avoid collision with parallel work.

## Phase 0 — Research and decisions

`research.md` records repository evidence and alternatives. Accepted decisions are nested acyclic
bodies, immutable collection membership, index-ordered results, first-commit approval outcome,
declarative atomic updates and same-version continuation with an additive completion kind.
ADR-0032 supersedes the original separate-revision allocation with additive v1.
ADR-0030/0031/0032 resolve value disclosure, iteration compensation, permission grants and bounded product
defaults. Effective CEL source is limited to 1024 bytes, list literals to 64, constructed lists to
4096; no object literal is added. Bounded 10,000-item reference materialization, nested low-capacity
admission and all benchmark/failure evidence remain release gates, not architectural results.

## Phase 1 — Design and contracts

- `data-model.md` defines identities, fields, lifecycle, resource limits and atomic boundaries.
- `contracts/README.md` records the accepted IR/API/worker design and error/telemetry rules. Canonical
  machine contracts remain T014-006 work and the design cannot be advertised as deployed.
- `contracts/conformance.md` maps the shared five-SDK cases and failure injection points.
- `quickstart.md` separates checks runnable today from future target suites.

## Phase 2 — Dependency-ordered implementation

1. **Current examples / US01**: tests and shared expected workflows; five language implementations;
   documentation and real execution evidence. Uses only approved features 001/002/006/012.
2. **Runtime foundation**: ADR/dependency acceptance; additive IR and worker compatibility; scoped
   identities with replay; additive data migrations; quota, audit and scheduler transaction support;
   runtime-graph/telemetry projection of scoped instances and safe Studio degradation (T014-045).
3. **US02 + US03**: bounded collection and sequential loop scopes; compensation/reconciliation and
   limits. Both depend on the instance foundation, not on each other's public API.
4. **US04**: approval policy, authorized command, request/expiry race and SDK operations.
5. **US05**: switch routing can be implemented independently after additive contracts/reducer.
6. **US06**: schema-checked business state, snapshot queries, atomic update receipts and auth;
   released only after the accepted ADR-0031 policy passes its security and conformance gates.
7. **US07**: chain links and atomic continuation after US06 state serialization; parent-child waiting,
   old-run command behavior, chain retention and quota/accounting.
8. **Release gates**: all-five golden/fault parity, upgrades with old active runs, architecture/contract
   checks, security tests, reproducible benchmarks and documentation. Ship a primitive only after its
   own gates; unchecked items are not converted into promises by completing this spec.

## Transaction and scheduling design

Run creation and evaluated continuation acquire applicable tree and active-run admission before
retaining input/initial-state payloads. Child reservation uses `FOR NO KEY UPDATE` on the parent:
it excludes cancellation and competing reservation writes while permitting the root/parent
foreign-key key-share checks of another creator. A depth-two handoff/root-child barrier and a
novel-payload root-start/handoff barrier exercise these edges; they do not implement capacity
groups or resolve every outstanding child-link/chain ordering question.

Planner decisions remain pure and bounded. Application obtains a committed snapshot/revision,
evaluates only trusted bounded expression interpreters, and proposes a typed transition. The store
rechecks run sequence, the scheduler owner/fence for scheduler actions, and the worker lease for
worker reports before commit. Authenticated HTTP commands serialize on run/chain and current policy;
they do not acquire an unrelated scheduler lease. Evaluation
is repeated only after CAS conflict against a fresh snapshot; once committed, replay uses recorded
results. No network calls or arbitrary user code run while holding a transaction.

Each iteration admission is a separate bounded scheduling transition; never create an unbounded
fanout in one transaction. Allocate only available slots and charge run/tenant budget in the same
transaction that creates tasks/outbox. Reconciliation recovers committed unsent work. A transaction
failure must roll back quota counters and receipts along with tasks. All new PostgreSQL transactions
use `BeginSafely::begin_safely` per ADR-0014.

Optional top-level `limits` defaults to `maxInstances=100000`, `maxDepth=8`; tenant policy may lower them.
Instance admission reserves the declared compensation/reconciliation identities of each forward
instance inside the same lifetime ceiling. Derived activation uses that reservation, and accepted
obligations remain drainable after budget exhaustion or policy reduction. Reserve descendant
capacity or distinguish suspended ancestor slots from tenant execution capacity; prove nested
low-capacity admission before enabling nesting.

Use chain-before-current-run locking for chain-capable runs and retain existing ownership/drain/session/activity
fence ordering. Handoff inserts a successor without taking two existing run mutation locks. Its stable-parent
identity KEY SHARE guard precedes head/link under ADR-0037's immutable ancestry precondition.
Parent and child transitions meet through a durable child-instance link: each mutates only its
own chain/run then the link, which holds cancellation/final-outcome/settlement state. The child's handoff checks its link
latch under lock; final notification is applied once under the parent's shard fence. Keep legacy
parent-first writers separate until explicitly migrated. Starting a child uses an explicit parent-shard
creation fence before the child's first event. The adapter derives that parent from immutable
admission metadata and refuses that fence on reopened or already-mutated transactions; normal
full/bounded mutations continue to require their own run's shard. Owner/generation/expiry remain
locked and checked at current database time. Both paths use IR v1; select the protocol
from immutable definition capabilities and explicit stored chain/identity data. `data-model.md`
and ADR-0030 specify the order, with the compatibility refinement in ADR-0032.

## Rollout, compatibility and rollback

Expand schema first; backfill old root instances with the root path and existing journal sequence
without rewriting historic events. Dual-read old/new event encodings using the pinned workflow
definition and explicit stored capability data; write scoped encodings only for runs using the
corresponding capabilities. Enable publication per primitive after all-five conformance;
unsupported kinds/capabilities fail before publication or run creation. Old workers retain old
workloads and must not receive scoped tasks they cannot identify.

Disable new publication/start to roll back capability. Continue serving existing runs using the new capabilities
on a compatible engine until drained; never downgrade a reader that cannot read their events or
remove their rows/enum support. Do not delete old history during continuation. Retention must keep
chain links, live parent references and deduplication receipts for the documented retention window.
No automated conversion of active old runs or cross-version continuation is in this increment.

## Risks and mitigations

| Risk | Mitigation / evidence required |
| --- | --- |
| Instance identity touches worker and storage contracts | Additive negotiation, root compatibility, migration/replay and stale-fence tests before loop execution. |
| Fanout multiplies bytes, nested scope work or compensation | Shared instance/byte/depth limits and incremental admission; reverse topological compensation within each iteration, independent collection iterations, and reverse sequential-loop iteration order. |
| Approval policy becomes stale | Snapshot policy identifiers; check current tenant/namespace authorization and eligibility per decision. |
| Query/Update accidentally executes arbitrary code | Publication grammar/schema validation and capability-free bounded evaluator; negative tests. |
| Continuation makes a child appear finished early | Child observer follows chain and emits final outcome once; duplicate/race integration tests. |
| Examples pass via canned outputs without real execution | Shared canonical definitions plus actual engine terminal states/activity observations in US01 gate. |
| Query/Update disclose payload values the API never returned before | ADR-0031 explicit disclosure, closed bounded result schemas, current permission on replay and access audit; release only after tests. |
| New error codes break SDKs that parse `Error.code` exhaustively | No new code; new meaning in `details.reason` with remediation rows; WP-13 checks it. |
| Children in bodies or child chains exhaust the 1,024-run tree | Budget documented in limits; WP-12 boundary cases; typed tree-limit failure. |

## Complexity Tracking

No constitutional exception is proposed. Scoped identities and chain links add necessary independent
semantics; their alternative would merge iterations or lose history. Architecture is accepted; implementation and measurements
remain pending and do not permit fake implementations of these guarantees.


### Integrated bounded effect checkpoint (2026-10-17)

The bounded exact-path store now shares pure effect/review reducers and scoped command rules
with full replay. Global work facts from other paths preserve the same run status policy without
restoring historical steps. The read snapshot and mutation version fence include that outside
work; partial indexes support the existential queries. Real PostgreSQL coverage uses the runtime
role and exercises unknown/applied lookup results and retained outputs. Exact commands and
failures are in `tasks/project-completion/integration-notes.md`.

The production worker still uses the existing full transaction path. Complete blob/provider/
control/drain/finalization integration and the 10,000/100,000 resource/fault criteria remain
required; this checkpoint is not a substitute for those accepted tasks.


### Current scoped capability increment (2026-10-17)

Scoped secret handoff and blob authority now consume current exact-path state. Secret handoff
keeps one shared Root/scoped application policy with their existing transaction lifecycles,
commits before provider I/O, and rechecks live/current authority and publisher inputs before
committing the handoff audit. Scoped audit deduplication addresses the complete logical step UUID.
Blob authorization preserves scanning refusals and does not enable scoped blob output completion.
The real-provider/history-denial and current regression evidence is recorded in
[validation-report.md](./validation-report.md). The larger graph, descendant admission, resource,
compatibility and release tasks retain their existing acceptance criteria and remain open.


### Public producer-to-collection increment (2026-10-17)

Publication now permits an ordinary activity prefix before the existing root collection chain.
The existing root worker/output transaction and collection-source capture port handle execution;
no alternate runtime, wire field or adapter query is introduced. Root producers with error edges,
compensation/reconciliation, effect policies or managed database operations remain closed, as do
activity successors and nested bodies. Real public inline/blob pipelines and equal root/body node
ids are covered in native and all-five SDK fixtures; current evidence is in the validation report.


### Scoped forward blob completion increment

Add ordinary forward blob completion to the existing exact-path report transaction, sharing
Root BlobCommit semantics. Preserve current authority before receipt lookup, cancellation-tolerant
live fencing before catalog access on misses, and exact receipt replay after expiry. The adapter
must bind the persisted descriptor to its blob commit and roll back both with journal failures.
Public native failure tests and the all-five `collection-blob-output` case cover this slice;
derived/declared-secret blobs, broader bodies/nesting and full resource gates remain closed.


Approval's own-run writer helper now shares the existing declared Run Update membership guard:
chain before exact run, held through commit. The guard resolves immutable tenant/namespace/run
membership and grants no head/finality authority. New-run creation commits initial membership
atomically; live reparent/membership migration is not supported by this prefix. Native ordered
wait/final-audit barriers accompany existing public successor Update and approval regressions.
Collection entry/drain and loop helpers now use the shared transaction-typed prefix, with
independent native wait/own-run-free barriers. Complete B0/group ordering remains pending.


B0 includes the existing009 production incident writer. ADR-0038 refines ADR-0022's explicit
serialization clause after a deterministic incident/report storage deadlock. Correct only
chain→ownrun→binding→ownstep order; preserve attribution and after-commit ACK semantics.
Require both native report-prefix counterexample and a real public leased script-chain barrier.


## Worker session renewal after lock waits (2026-10-18)

The existing009 WorkerStore renewal acquires its matching tenant/id/token session row before
sampling the database clock, then renews in the same transaction only if still live. A session
that expires during an unchanged-row wait returns the existing unauthenticated outcome without
changing expiry, leases, run version, journal or outbox. Fresh registration remains supported.
This is post-lock freshness, not continuous validity through commit; TTL/wire policy is unchanged.
Full/path claim-room checks now share a transaction-typed post-lock clock check: expired
authority supplies zero room without renewal, retaining the guard through count/claim commit.
Managed-database ownership separately retains its session SHARE guard and samples time after
acquisition; the complete B0/group lock graph remains pending.
Runtime-role public-poll RED/GREEN and exact source/commands are retained under
`tasks/project-completion/evidence/session-renewal-postwait-20261018/`. No canonical task closes.


## Integrated managed-session expiry correction (2026-10-18)

The existing managed executor owner check now reads the matching session expiry under its
SHARE guard, then samples the database clock on the owning run transaction. A session that
expired during an unchanged-row wait cannot authorize heartbeat renewal. Keep the existing
binding→session→namespace secret/catalog quota→current dedicated membership order and all
tenant/namespace/rootnode/protocol filters. No TTL, wire, role or provider policy changes;
the guarantee is validity at that post-lock sample, not continuously until commit.
This is a focused existing-writer correction required by014 B0, not nested execution or a new
017 delivery gate. Real public managed lease/heartbeat RED/GREEN and reconnect controls use
PostgreSQL/OpenBao in tasks/project-completion/evidence/database-session-postwait-20261018/.
Standalone reference evidence remains historical; integrated release gates remain separate.


## Integrated managed report replay ordering (2026-10-18)

An existing managed report receipt checks current binding before retaining the namespace
Secrets quota and dedicated membership guards. Durable attempt ownership, permission, role,
receipt fingerprint and binding remain required; a live session is not required to replay the
accepted report. Live/missing-receipt and ordinary paths keep their existing behavior.
A real PostgreSQL/OpenBao published-query fixture proves actual app-role binding wait while
Secrets remains free, then exact receipt replay after session removal with unchanged execution,
audit and credential-pin counts. Reconnect and subsequent public polling remain supported.
This is the existing017 writer dependency of014 B0, not a new capability or full graph proof.
The ordered barrier does not claim a reproduced three-party deadlock. Evidence:
tasks/project-completion/evidence/managed-replay-prefix-20261018/.

Rerun source admission correction: ADR-0039, exact tenant/source identity KEY SHARE after replay
and before admission. Evidence: tasks/project-completion/evidence/rerun-source-identity-20261018.
Do not generalize this root-rerun writer exception to child ancestry or nested group eligibility.

B1a scoped entry decision is reviewed: one existing group NOWAIT after current chain/own-run
entry, whole transaction rollback on contention; unknown COMMIT outcome reconciles atomic
receipt/state. See tasks/project-completion/capacity-transaction-entry.md. No SQL lifecycle,
own-member transition authority, B1 completion or nested runtime activation is accepted.

A1 storage prerequisite: pure checked CapacityGroup::restore with exact holder fact and bounded
header, fixed-seed model round trips, invalid snapshot and exhausted-counter controls. No SQL
port/eligibility/OTel acceptance follows from pure domain tests. See capacity-header-restore evidence.

B1b.1 child-root Finish authority is reviewed: exact current own-chain/head/segment FINAL, matching
terminal outcome, guarded member binding and current own scheduler fence. Never infer finality
from CONTINUED, historical run status or parent terminality. See capacity-member-finality.md in
tasks/project-completion; bounded SQL schema/receipts/port and other B1 families remain pending.

B2a scoped storage contract is reviewed (capacity-finish-storage.md). Refine the local B2 dependency:
the dormant exact child-root Finish proof depends on A1+B0+B1a+B1b.1 only and cannot activate
runtime, close full B1/B2, create/reserve groups or claim complete capacity accounting. All other
B1 decisions still precede their corresponding persistence/runtime integrations. Implement
vocabulary → schema → exact port → real barriers/recovery → observations in separate increments;
retain B1a's actual retained-usage gate for a later command with a real delta.


B2c schema is implemented and tested only with explicit owner fixtures and real runtime-role
schema probes. No production creator/caller is introduced. Preserve permanent member/receipt
chain references until C0 defines compatible tombstone/replay/history retention; current schema
pins chain/run cleanup. B2d exact native Finish remains the next implementation unit, followed by
B2e barriers/recovery and B2f observations. See capacity-finish-schema-20261018 evidence.


B2d.1 application contract is defined with exact identity/revision groups, renewable own scheduler
fence, versioned fixed-width fingerprint, typed outcomes and immutable receipt result. Cancellation
can leave a COMMIT outcome unknown; callers retain the entire original command and future production
callers require durable intent. This API-only increment enables no runtime caller or SQL operation.


B2d.2 exact PG adapter and real current child FINAL/refusal/replay evidence are implemented.
B2e must now test actual contention, postwait expiry, confirmed failure atomicity and both unknown
COMMIT outcomes; B2f adds closed observations. These checkpoints cannot enable a production caller
before durable intent, C0 retention/old-writer compatibility and remaining lifecycle decisions.


B2e.1 native G/prefix/fence and confirmed abort barriers have focused real PG evidence in both
suite modes, with explicit runtime-role adapters. Keep B2e open for actual unknown-COMMIT outcomes,
same-command prefix reconciliation versus distinct-member G Busy, handoff/discovery race and
same-tenant multiple-namespace membership; no production activation or quota proof follows.


B2e.2 actual unresolved COMMIT response-loss evidence covers committed and aborted outcomes,
same-command prefix timeout versus distinct-member G Busy, and same-key fresh transaction recovery.
Production adapter unchanged; no durable caller/process restart or quota acceptance. B2e.3 still
must cover real handoff/discovery and same-tenant multiple namespace membership; B2f remains open.


B2e.3 real handoff/discovery and same-tenant two-namespace tests close the local B2e proof.
B2f observes only returned native attempts, fixed safe identity spans under caller trace and
finite outcome labels. The transition counter follows acknowledged first commit only; unknown
and replay rely on durable receipt evidence. Preserve the unchanged transaction helper and
ensure a filtered-out child span cannot mutate caller span fields. This closes no full B1/B2
or canonical runtime task and introduces no production caller, eligibility or nesting.


B1b.2a defines only required exact scoped-path settlement evidence. Existing checkpoint/drain and
inactive ancestry may precede compensation creation; they cannot authorize Finish. Require an
explicit source-closure proof and all recorded obligation dispositions, then separately review
one leaf collection source writer, scoped schema/retention and broader closure/quiescence. Full
B1b.2 stays open; the ChildRoot adapter/schema cannot be repurposed by changing a tag. See
tasks/project-completion/capacity-path-settlement.md.


B1b.2a.1a chooses a separate own-run certification transaction after successful CollectionFinished,
restricted first to a depth-1 ordinary activity-chain body without derived/effect producers. Exact
admission/checkpoint/return/reduction/finish references and terminal routed local rows are required;
no aggregate/history hydration. Before SQL, add the versioned pure shape predicate and review
source-certification record/audit/retention/API, then prove actual producer preservation and restart.
The reviewed source-writer choice is not current implementation evidence. See
capacity-leaf-success-certification.md under tasks/project-completion.


B1b.2a.1b now has the pure version1 LeafCollectionSuccessShape classifier and8focused tests.
It follows actual root/body routes, qualifies local IDs and explicitly detects lowered scripts,
derived producers and effect settings. It scans bounded static definition metadata, not runtime
history, and proves no publication/version pin/source finality. API/record/source-writer and real
producer/restart gates remain next; do not activate scoped capacity from this pure result.


B1b.2a.1c.1 fixes the immutable source-certification port: canonical tenant/namespace/original-run/
depth-1 path is its own receipt for fixed OrdinaryLeafV1, with no caller business parameters.
Current own-run scheduler authority precedes canonical replay; first creation CAS follows receipt
miss and precedes actual pinned source proof. Existing-group mutations retain B1a key/fingerprint
rules. Before source SQL, add exact application types/structural checks, then review immutable
schema/retention/implicit locks and truthful audit identity. See leaf-certification-port.md.

B1b.2a.1c.2 adds application::leaf_certification command/identity/profile/record/result/error and
store port, with7focused structural/privacy tests. Depth1, qualified return-path binding and
positive signed-journal sequence relationships are checked; actual version/source/fence/CAS
proof remains the future adapter's responsibility. No schema, adapter or production caller exists.

B1b.2a.1c.3 freezes a dormant immutable canonical snapshot, exact namespace/run FK, no expiry under
D298, unconditional source deletion guards and original-run identity preservation including FK
replication-role bypass. Source UPDATE closure remains a producer proof. A dedicated noncollapsing
audit action and injective natural resource identity commit with first certification only. No hint
or execution journal event is introduced. Guarded down takes runs before the certificate table and
refuses retained rows. Schema/audit implementation and real tests are next; see
tasks/project-completion/leaf-certification-storage.md and leaf-source-retention-inventory.md.

B1b.2a.1c.3a now implements dormant migration20261211000000, bounded storage classification,
startup append-only role checks and five real PostgreSQL schema/retention/downgrade tests in
default/runtime-role modes. Existing chain obstruction fixtures reuse retained rows. Dedicated
audit vocabulary and the real bounded source certifier/producer proofs remain next. No source
adapter, INSERT runtime grant, production caller, scoped Finish or nested execution is enabled.

B1b.2a.1c.3b adds the dedicated noncollapsing collection.leaf_success_certified audit action and
leaf_collection_success resource type. Domain audit8/8 checks vocabulary/dedupe policy only;
first-record/atomic audit/replay/source correctness still requires the native adapter and tests.

## Dormant scoped Finish schema increment — 2026-10-18

The fixed OrdinaryLeafV1 member/receipt binding is now represented by additive migration
20261212000000. Total discriminated CHECKs and exact tenant/group/path FKs preserve ChildRoot
metadata, original20-column runtime INSERT, immutable identity and permanent tombstones. Actual
capacity Finish regression passes30tests, including five new schema probes. Conditional downgrade
execution passed in a separate bounded unit, including native down/up replay and observed writer
lock wait/refusal. This introduces no scoped native writer or production caller;
B1/C0/general nested publication and canonical runtime gates remain open.


## Dormant ordinary-child source characterization — 2026-10-18

The next bounded evidence unit is the already reachable OrdinaryLeafV1 source in a genuine
ordinary unchained child Run, with selected root path lines/0 closed while later child work
continues. The existing source certifier has no parent/chain-context rejection; its current
producer-preservation evidence remains limited until the new tests pass. No new source enum,
schema, API, caller, grant or publication shape is introduced. Scoped Finish continues to refuse
any parent/chain context, including a genuine child certificate.

The reviewed contract is [leaf-child-source-preservation.md](../../tasks/project-completion/leaf-child-source-preservation.md).
The first baseline requires genuine public creation/closure, metadata-only exact certification,
missing-closure/current-fence/qualified-identity controls, cold original-record replay and a genuine
certificate that still cannot authorize Scoped Finish. The baseline passes both focused tests in
default and runtime-role modes; exact evidence is in ordinary-child-source-baseline-20261018. Actual parent flags
(PROPAGATE/DETACH/REQUEST_CANCEL), own-child cancellation/drain and terminal fanout are separate
subsequent proofs. Nonjournaled parent flag changes do not imply a stale child Run revision.
General nesting, current-member authority, C0/mixed writers, resource ceilings and release gates
remain unaccepted. This is a bounded evidence refinement under ADR-0030/0031/0032.


The ordinary-child incoming parent flag family now has real PROPAGATE/DETACH/REQUEST_CANCEL
queue-order and actual certifier-COMMIT guard evidence. The four-test child suite passes in
default/runtime-role modes, with seven recorded three-PID observations per mode. No runtime
implementation or activation changed. Own-child cancellation/compensation
and terminal fanout remain pending. Evidence: tasks/project-completion/evidence/
ordinary-child-parent-flags-20261018/manifest.json.


The separate DETACH later-worker-progress characterization passes its focused test in default
and runtime-role modes, including unchanged settled parent/source and cold original-CAS replay.
No whole-child completion or terminal-result/fanout guarantee follows. The newly identified
collection End/owner-result retention defect remains an explicit .1d.0 prerequisite requiring
genuine RED and a reviewed correction. Evidence: tasks/project-completion/evidence/
ordinary-child-detached-progress-20261018/manifest.json.

## .1c acceptance checkpoint (2026-10-18)

Two real PostgreSQL tests pass in default mode (42.77 s wall / 4.74 s harness) and explicit runtime-role mode (5.30 / 4.66); same two names in both, no aggregate four-test claim. Actual three-PID witnesses establish both original-child writer COMMIT orders. After real public PROPAGATE, cancellation, drain and known-effect refund, selected source/certificate/audit remain unchanged and the complete original record replays cold. The child remains unfinished; End/result/fanout remain separate. Strict Clippy passed 11.69 s, fmt 4.36 s, architecture 0.34 s / 24 crates. Final 1108 source inputs unchanged. See evidence/ordinary-child-own-cancel-20261018 manifest/review/reproduce and retained initial failures. B1b.2c.1c closes only; parent umbrella, .1d/.1d.0 and canonical 014 tasks stay open.

Implementation/evidence commit: `de64eafc`. Prior child-suite, detached-progress and this two-test selection are separate source revisions and invocations, not a new combined suite. The independent collection End result and instance-limit RED regressions remain unresolved at `6ef22a85`; these successful cancellation tests stop before that path.

## Shared End preparation checkpoint — 2026-10-18

`da9201b2` closes local .1d.0b only: shared bounded End evaluator and optional closed answer on CollectionFinish, default None. Existing real result gRPC tests passed5/5 in each separate mode, final1108 inputs; strict application Clippy/fmt/architecture passed. See tasks/project-completion/evidence/collection-end-preparation-20261018/manifest.json. No native collection consumer activated; collection End/instance-limit RED and .0Q remain unresolved. No canonical task closure.

## Data-limit vocabulary and domain checkpoint — 2026-10-18

Q1 closed RunDataLimit vocabulary/domain/codec preparation and Q1a four new pure domain cases have bounded evidence in tasks/project-completion/evidence/collection-data-limit-vocabulary-20261018 and collection-data-limit-domain-20261018. Codec9/9; updated domain targets17/17,6/6,30/30 in one invocation. Strict relevant Clippy/fmt/architecture pass. Native reason production explicitly remains RuntimeNotIntegrated before transaction; full/path readers, additive schema, effective legacy accounting, real report/compensation/upgrade and native End correction are pending. No public capability/shape/grant or canonical task closure. Public empty-collection and End result/instance-limit regressions remain genuine RED.

## Collection data-limit schema checkpoint — 2026-10-18

`55ea9423` adds migration20261213000000 without changing applied migrations, grants or writer activation. Five behavioral PostgreSQL schema tests pass separately in default and runtime-role modes (49.04s/4.85s and5.14s/4.82s wall/harness), including total CHECK, old-form preservation, guarded down/up and observed down-before-writer rollback. Strict integration Clippy/fmt/architecture pass; final1112 inputs unchanged. See tasks/project-completion/evidence/collection-data-limit-schema-20261018/manifest.json, including the separately retained initial compilation failure. New-reason rows are explicit static fixtures, not native producer, replay or actual old/current upgrade evidence. Schema down eligibility never authorizes old execution binaries. Native producer, effective accounting, report preservation, scheduler lifecycle and End correction remain pending; no canonical task closes.

## Collection reader and known-effect prerequisites — 2026-10-18

`c620c2bd` and `7212a4b1` add strict new-reason full, owned-path and descendant-ancestor readers. Four focused SQL tests pass in each access mode, including ten static malformed projection cases and native finish no-write rejection; executed behavior pins and subsequent lint-only revision are separately recorded in collection-data-limit-readers-sql-20261018. The dormant path-owned nested shape is not exercised by these public depth1 fixtures, and native new-reason production/replay remains pending. Existing runtime byte_ceilings19/19 passes.

`a7dc19b3` adds the metadata-only effective-byte helper, verified read-only legacy overlay and full positive-charge guard. Bounded reviews found no remaining issue, but actual frozen old-producer overlay/guard/upgrade acceptance is still pending. It is preparation, not a deployable bridge.

`c04b786d` adds the root-only preparation command for refused known-success scoped outputs. Pure domain scoped_compensation34/34 passes, including four new ordinary/Applied identity, prior-stop, version and compensation-plan tests. PostgreSQL report atomicity/authorization and the matching scheduler consumer remain pending. `ffec26d5` adds typed audit/evidence with audit9/9 and constructor3/3, plus strict relevant Clippy/fmt/architecture. Final source pins are1114 for the domain command and1115 for audit/evidence. See their separate manifests under tasks/project-completion/evidence. No public shape, grant, production caller or canonical task closes.

## Effective-byte/native preparation checkpoint — 2026-10-18

`b0a7ea5e` shares verified E=L+V with exact-path scalar/positive guards and ScopeRun in its existing repeatable-read snapshot. Strict integration Clippy and existing runtime byte_ceilings19/19 pass, final1115 inputs unchanged. Actual old E>L snapshot/CAS/report acceptance remains pending. `012d746f` uses the effective scalar in four existing full preflights; existing real gRPC result5/5 passes separately per access mode and strict all-target application Clippy/fmt/architecture pass. These existing regressions do not establish new legacy signal/chain acceptance. See the effective-output-path-frontier-20261018 and effective-output-preflights-20261018 manifests.

`9fd9405f` adds the guarded internal native drain writer. C1 owner/checkpoint gaps were corrected; fresh bounded C2 on e0928f4f found no required issue. Strict PostgreSQL library Clippy/fmt/architecture pass with1117 unchanged inputs; executed native behavior tests are zero. The old unconditional RuntimeNotIntegrated refusal has been replaced by checked audit/evidence plus locked source/budget/stop predicates; normal scheduler/report producers remain absent. Native positive/replay/corruption/fault/runtime acceptance and End correction remain pending. This intermediate source is not a deployable bridge.

`46d3824f` corrects Q7b's unconditional report-outbox assumption to existing durable collection discovery. Report refusal creates no outgoing obligation or task release; its expected new outbox count is zero, with journal/projection/audit/receipt/telemetry atomic. The actual later drain retains its own outbox. Fresh bounded amendment review found no issue under Constitution I/VII and ADR-0002/0017. Cold normal-tick discovery without hints remains a required unexecuted gate. No public message, grant or shape, ADR divergence or canonical task closure follows.


## Actual legacy-output compatibility acceptance — 2026-10-18

Q5 is accepted with source `ce0cb786` and immutable old producer `6c44a0e4`: the complete
old/current coordinator passed in133.91s. Each of the two independent role passes contains one
real public old-producer test and one current matrix test with five ordered cases, not five tests.
The old producer always uses the runtime role; only the current native store pool varies between
owner and `sagat_app`. Read-only overlay/per-owner accounting, exact migration/read-only witnesses,
isolated corruption refusals, unclamped E>L, fitting L-only charge, zero-delta access above the
limit and positive-delta rollback passed. Stored `legacyControl` discovery metadata is allowed
only with the unchanged pinned rootForEach/full source qualification; no old source/result is
rewritten. Strict Clippy/fmt/architecture passed with1117 unchanged native inputs.

Known-success report baseline tests in `03a0043a` independently reproduce actual FAILED acknowledgements
for a real successful forward effect and a genuine Applied reconciliation; their correction and
later compensation assertions remain pending. Q8 second-owner discovery compatibility and actual
producer composition, native End/scheduler activation, full old-binary rollout, source/fanout,
nested capacity and final release gates remain open. No canonical runtime task is closed by Q5.
Evidence: `tasks/project-completion/evidence/legacy-output-view-20261018/reproduce.md` and
`scoped-output-refusal-red-20261018/reproduce.md`.


### 0.3 control entry admission increment

[The bounded admission contract](./control-entry-admission.md) extends collection entry to
Condition/Switch through an explicit Activity/Scheduler dispatch intent. It reuses native
chain/run→shard→policy/usage→scope admission, charges the entry and reservations exactly once,
and discovers PENDING control work from durable state without a task hint. The additive
20261221000000 migration preserves old activity writers by default and retains immutable
scheduler admission evidence for rollback refusal. Named native tests cover transaction
rollback, replay/mismatch, revision, capacity reduction, cold discovery, cancellation,
CHECK-valid projection corruption and frozen OrdinaryLeafV1 exclusion. All general task and
release gates remain open. Old scheduler authority must be fenced/drained before admitting
new definitions; released-profile upgrade evidence is required before advertising 0.3.


### 0.3 scoped timer runtime increment

Use shared explicit-clock full/path commands, native waiting_kind/due_at projection and a
retained timer read-set (status/routed/deadline). Reuse scheduler admission's SCHEDULER intent
and qualified timer discovery, with no fake activity/task hint. Durable collection discovery
resumes succeeded/unrouted members after pause; safe drain cancels validated waiting timers.
Full/path/frontier readers and both writers enforce the same closed lifecycle and physical
metadata. Test journal rollback, stale snapshots, real cancellation/fire lock order, cold resume,
source-profile exclusion and five existing SDK sleep builders. See scoped-timer-runtime.md.
No new migration, protocol minor or nested capacity activation is needed for this increment.

### Explicit zero tenant iteration policy (O701)

[zero-iteration-policy.md](./zero-iteration-policy.md) implements the decided admission-only
meaning of zero. Keep default/absent versus explicit tenant policy distinct, reuse the current
policy/run/usage locks and admission comparison, and preserve accepted progress and drain.
An additive positive→nonnegative CHECK has a retained-zero downgrade guard; deploy schema
first and fence zero-capable writers before rollback. This adds no group runtime or counter.

### Scoped event-wait recorded preparation

[scoped-event-waits.md](./scoped-event-waits.md) separates pure recorded lifecycle/branch
selection from native correlation, clock and delivery authority. Root/full/path planners share
recorded branch selection; Pending waits remain scheduler-owned. Public wait bodies stay closed
under a contract fitness test until qualified dispatcher identity, persistence/receipt/value
atomicity, races and all-five runtime evidence pass. Native/group activation is not inferred.


### Qualified event receiver implementation checkpoint — 2026-10-27

The bounded contracts in scoped-event-scheduling.md, scoped-event-deadlines.md and
scoped-event-receiver.md now have current native caller evidence. External delivery uses
current exact-path receipt/value transactions; lookback uses bounded tenant-qualified traversal.
Index-only migration20261223000000 retains legacy lookup support and documents build locking.
This does not activate public scoped event bodies or entry admission. Released-v0.2 cutover,
remaining node kinds/nesting/capacity and canonical release acceptance remain pending in the
implementation ledger and validation report.
