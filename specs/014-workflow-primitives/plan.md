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
fence ordering. Handoff inserts a successor rather than locking two existing runs. Parent and child
transitions meet through a durable child-instance link: each locks only its own chain/run then the
link, which holds cancellation/final-outcome/settlement state. The child's handoff checks its link
latch under lock; final notification is applied once under the parent's shard fence. Keep legacy
parent-first writers separate until explicitly migrated. Both paths use IR v1; select the protocol
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
