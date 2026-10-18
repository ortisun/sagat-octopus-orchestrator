# Implementation Plan: Durable workflow primitives and practical SDK examples

**Branch**: `feature/workflow-primitives` | **Date**: 2026-10-14 | **Spec**: [spec.md](./spec.md)

**Input**: `specs/014-workflow-primitives/spec.md`

## Summary

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
implementation or activation changed. Own-child cancellation/compensation, detached progress
and terminal fanout remain pending. Evidence: tasks/project-completion/evidence/
ordinary-child-parent-flags-20261018/manifest.json.
