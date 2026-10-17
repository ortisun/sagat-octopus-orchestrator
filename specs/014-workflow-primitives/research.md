# Research: workflow primitives

**Date**: 2026-10-14. The design is accepted by ADR-0030/0031/0032; runtime implementation and release
evidence remain pending. The decisions below record the evidence and accepted bounded defaults.

## Baseline verified in this repository

| Evidence | Consequence for 014 |
| --- | --- |
| `crates/domain/src/workflow.rs`: `NodeKind`, acyclic validation, `MAX_BRANCHES=256`, `MAX_NODES=10000`, `MAX_WAIT=366 days` | Retain DAG invariants and upper bounds; dynamic bodies need explicit instance identity, not graph back-edges. |
| `crates/domain/src/execution/plan.rs`: `join_outcome`, `settle_joins` | Static `all` and first-success `any` already exist. Neither is newly implemented here. |
| `crates/domain/src/execution/state.rs`: run statuses and event reducer | No `CONTINUED` status exists; an additive completion kind avoids silently expanding the status enum seen by old clients. |
| `specs/001-platform-foundation/contracts/workflow.schema.json`, `crates/infra-ir/` | Wire IR remains separate from domain rules. ADR-0032 keeps additive v1 IR and independent worker 1.5; canonical schema/mapping implementation remains T014-006. |
| `crates/infra-expr/src/{limits,parser,eval}.rs` | Actual source limit is 1024 bytes, literals 64, constructed/concatenated lists 4096; no object/map literal. The domain 2048-byte wrapper is not the effective evaluator limit. |
| ADR-0002, ADR-0014, ADR-0020 | Reuse authoritative database transactions, cancellation-safe transaction start and scheduler owner fencing. |
| ADR-0015, `tests/contract/sdk-conformance/` | All five SDK public APIs must match a reviewed common golden, including faults. TypeScript is not a semantic reference. |
| `specs/012-developer-experience/`, `examples/hello-world/`, `examples/payment-saga/` | Add a practical three-workflow example on current APIs; do not fabricate a loop/approval API for demonstration. |
| FR-003-002, ADR-0005, ADR-0010, `crates/domain/src/saga.rs` (`reverse_dependencies`) | Compensation is planned in reverse topological order and respects branch independence; it is not reverse completion order. Iteration compensation refines this rule (R2). |
| ADR-0028, `Run` schema in `openapi.yaml` (no output field) | General inspection remains structural; ADR-0031 separately authorizes explicitly declared Query/Update results with closed schemas, operation permissions and access audit. |
| `Error` schema in `openapi.yaml`, `crates/api-http/src/error.rs` | `Error.code` is a closed enum of ten values; refinements are `details.reason` (`IDEMPOTENCY_KEY_REUSED`, `REVISION_CONFLICT`, `ALREADY_DECIDED`, `STATE_CONFLICT`, `UNSUPPORTED_NODE`…). New errors add reasons, not codes. |
| `crates/domain/src/identity.rs` (`Role`), feature 009 G39, feature 013 (federated login) | Six built-in roles, no custom roles; 013 excludes mapping identity-provider groups to roles. Approval eligibility can name subjects and built-in roles only (R3). |
| `Run.version` / `expectedRunVersion` in `openapi.yaml` | The run's existing optimistic-concurrency token counts every journal event. Update uses a state revision instead, so unrelated work does not cause conflicts (R4). |
| `crates/application/src/runs.rs` (`signal`), `signalRun` in `openapi.yaml` | A signal is accepted only by a waiting step and consumed in that transaction; it is never buffered. There is no pending-signal state for continuation to drain (R5). |
| `crates/application/src/execution.rs` (`MAX_TREE_RUNS` = 1024), `Run.depth` ≤ 8 | Every run created under one root counts. Children inside bodies and child-chain successors consume this budget (R5, R6). |
| `Run.rerunOf` (RERUN_AS_NEW), ADR-0012, ADR-0016, ADR-0017, ADR-0023, ADR-0027 | Existing rerun lineage, durable inbox, journal-derived graph and telemetry, blob claim-check and durable content/rate/retention quotas: reused, not replaced. |

## R1 — Structured bodies, not structural cycles

**Decision**: Add named, closed acyclic bodies and durable scope/iteration identities. A body may
contain ordinary nodes and nested bounded scopes; its completion uses a body return boundary, not a
workflow `end`. Each activation is uniquely `(run, enclosing instance path, definition node)`.

**Rationale**: Today's reducer keys ordinary steps by node identity. Reusing a node id for every
iteration would merge results, leases and compensation records. Explicit identities let replay
retain previous iterations without mutating an immutable definition.

**Alternatives**: Graph back-edges weaken publication validation and require a different execution
model. Expanding all iterations at publication cannot handle runtime inputs. Hosting an imperative
loop in an activity hides checkpoints and cancellation from the engine. ADR-0030 records
identity/storage cost and accepts the direction; implementation and measurements remain pending.

## R2 — Collection semantics and loop budgets

**Decision**: `forEach` is the collection node; `map` is an SDK convenience selecting each body
result. Both use stable input indices, an immutable input snapshot, bounded concurrency and fail-fast
admission with drain. `while` is pre-test and `repeatUntil` post-test, both sequential with explicit
carried state, `maxIterations`, deadline and optional between-iteration timer.

**Rationale**: Ordered results and per-instance effect keys remove dependence on completion order.
Empty collections return `[]`; false initial `while` returns the initial carry; exhausted limits
are failures. Slots remain occupied until the iteration is terminal or safely reconciled, so a
lost lease cannot exceed concurrency through premature release. No collect-all failure mode, break,
continue, mutable iterator source or streaming collection is promised in the first increment.

Compensation keeps FR-003-002 instead of inventing a global completion order: within an iteration,
the body's reverse topological order; between collection iterations, none (they are independent
branches and may compensate concurrently within the concurrency limit); between loop iterations,
reverse iteration order, since each depends on the previous carry. ADR-0030 accepts this refinement of ADR-0005.
Forward instance admission reserves its possible derived compensation/reconciliation identities
inside the lifetime maxInstances ceiling so budget exhaustion cannot prevent these obligations.

**Alternatives**: Best-effort cancellation plus immediate success could conceal effects. Unbounded
fanout cannot respect tenant capacity. Merely reusing `parallel` does not give dynamic membership or
bounded admission. AWS's documented Inline Map distinguishes per-item iteration and a concurrency
limit; this supports separating membership from admission but its limits/implementation are not
copied into Sagat. [AWS Inline Map](https://docs.aws.amazon.com/step-functions/latest/dg/state-map-inline.html).

## R3 — Approval is a first-class authorized decision

**Decision**: One eligible decision wins. Snapshot the allowed subjects/role identifiers and
initiator at activation; evaluate current membership and authorization when deciding. Exclude the
initiator by default. A timer makes expiry eligible; response/expiry/cancel serialize on the same
request/run. First committed outcome wins, including a response after the nominal deadline if the
expiry transition has not committed. This is deliberate first-commit behavior, not wall-clock
priority. Decision routes are approve/reject/expire, with cancellation handled by the enclosing run.

**Rationale**: A generic signal proves neither the decision-maker's permission nor separation of
duties. A persisted policy prevents a later definition publication from altering a pending request;
current RBAC prevents a revoked member from using an old pending request.

**Eligibility vocabulary**: roles are the six built-in ones (G39 accepted no custom roles) and
feature 013 does not map identity-provider groups to roles. A policy therefore lists subjects and,
optionally, `admin`/`developer`/`operator`; a business approver group ("finance approvers") is
expressed by listing its subjects until a role or group feature exists. New permissions
`approval:read`/`approval:decide` follow the `resource:action` form; ADR-0030 accepts read grants for
admin/developer/operator/viewer and decision grants for admin/developer/operator, still subject to
eligibility. Worker and runner receive neither.

**Alternatives**: Quorum/delegation/escalation introduces additional identity and race rules and is
out of scope. Existing publishing approval (ADR-0018) approves a document, not a running workflow;
its resource/permissions must not be reused as if they were interchangeable.

## R4 — Query and Update are declarative state operations

**Decision**: Define a schema-checked business state initialized at run creation. Query evaluates a
bounded side-effect-free expression against one committed snapshot. Update validates arguments,
optionally checks an expected state revision, computes a new state and return value, and commits
state, receipt and journal atomically. Neither runs arbitrary SDK code. Reads work on terminal runs
while retained; new updates are allowed only in RUNNING/WAITING.

**Corrections from validation**: (1) the concurrency token is the state revision, not the run's
journal position: `Run.version`/`expectedRunVersion` already exists and moves on every activity
completion or timer, so using it would make interactive updates conflict with unrelated work.
Query returns both values under their existing/explicit names. (2) An Update schedules nothing: no
node waits on a state predicate and no node assigns state, so "work enabled by an update" had no
mechanism. Whether to add either is a pending decision, not part of the first increment. (3) Query
and Update return business values through the API, which ADR-0028 does not permit today; ADR-0031 accepts
explicit `declared-business-result-v1` disclosure, while implementation/security/conformance gates
still block US06 release.

**Rationale**: This follows the graph-native boundary (ADR-0003) without inventing replay of arbitrary
procedural code or executing customer code in a scheduler. Valid business rejections are durable
receipts, enabling safe retries after lost responses. Authorization is rechecked before exposing a
stored receipt. Query's read audit is separate from the workflow journal and cannot activate work. Namespace
run:query/run:update grants cover all published operation names, with no per-name ACL. Each
operation explicitly declares its disclosure policy and closed bounded result schema. The existing
expression subset can select existing objects such as args/state but cannot construct object
literals; this increment does not add that grammar.

**Alternatives**: Async Update handlers that launch activities/wait would need a new operation
lifecycle, cancellation protocol and pending-handler shutdown policy. Temporal documents handler
concurrency and the need to finish handlers before workflow exit; that is a useful hazard inventory,
not evidence that Sagat implements Temporal's semantics. The first Sagat increment therefore uses
only bounded atomic reducers. [Temporal message handling](https://docs.temporal.io/handling-messages).

## R5 — Continue-As-New closes a segment without changing old status enums

**Decision**: A `continueAsNew` boundary ends the old run as `SUCCEEDED` with additive
`completionKind=CONTINUED`, and atomically creates a new run in the same chain with the same pinned
version. Normal success uses `completionKind=FINAL`. Link predecessor/successor and update the head
in that transaction. Child completion notification is suppressed until the chain's final outcome.
Quiescence excludes outstanding activities, timers, subscriptions, approvals, unresolved outcomes
and compensation obligations. Only explicitly transferred input/state is copied. The original chain
deadline and tenant accounting survive; a chain has a finite configured segment limit.

Signals need no drain at the boundary: a signal is accepted only by a step waiting for it and is
consumed in that transaction, so an open wait (which already blocks handoff) is the only message
obligation; a chain-targeted signal to a head that is not waiting is refused, never held. A child
chain's successors are runs of the parent's tree and consume its 1,024-run budget. RERUN_AS_NEW of
any segment starts an unrelated run with `rerunOf`; chain lineage is a separate link.

**Rationale**: A terminal run never reopens. Existing clients can still parse `SUCCEEDED`, but must
use the additive chain fields to determine whether the business process finished. This limitation
is documented and tested before enabling a workflow that continues. Version migration is excluded
until its compatibility and authorization contract is separately approved.

**Alternatives**: A new terminal status is a possible future major-contract choice but breaks clients
that exhaustively parse the present enum. Resetting the same run erases history. Automatically
following old-run writes risks directing an operation to an unintended segment. Temporal describes
fresh histories and different run IDs within a chain; we adopt the explicit-chain concept, not its
infinite continuation/versioning guarantees. [Temporal Continue-As-New](https://docs.temporal.io/workflow-execution/continue-as-new).

## R6 — Limits are explicit design values, not capacity claims

**Decision**: Optional top-level `limits` defaults to `maxInstances=100000`, `maxDepth=8`;
tenant policy may lower them. Forward activation reserves its declared derived identities against
that lifetime budget, retry reuses them, and derived drain honors the reservation after a policy
reduction. Hard publication ceilings: 10,000 items/loop iterations, concurrency 256, nesting depth
8, 100,000 activated node instances per run, 32 cases per switch, 32 operations of each Query/Update
kind, 32 approval principals/roles per policy, 10,000 segments per chain, and the existing 366-day
wait ceiling. Tenant policies may lower these. Runtime inline state/results retain existing limits
(256 KiB value, 4 MiB retained run data, 8 MiB task context); large collections use authorized blob
references without bypassing byte/node budgets. Run creation captures effective limits; policy
reductions block additional admissions but never erase accepted work or prevent draining it.
CEL source remains limited to 1024 bytes, literal lists to 64 and constructed lists to 4096;
a 10,000-item reference must pass bounded materialization and byte tests before release. Nested
scope capacity also needs low-quota/policy-reduction evidence so suspended ancestors do not consume
all capacity their descendants require. The existing child-run tree budget (1,024 runs, depth 8) still applies, so a collection whose body
starts a child is effectively bounded by it, whatever `maxItems` allows.

**Rationale**: Explicit values make rejection/overflow tests possible. They are conservative design
ceilings to validate before implementation release; T014-042 must measure cost and may propose
lower values, with contracts/tests updated together. No throughput claim is inferred from a ceiling.

## R7 — Compatibility and release sequence

**Decision**: Preserve accepted IR v1 forms and stored-run behavior. ADR-0032 adds new node shapes,
scoped identity and declared state to `sagat-octopus-orchestrator.io/v1`, superseding the separate
revision allocation. Worker minor 1.5 remains independent after checking the 1.4 baseline. Existing root-node identities and older-minor workers remain supported for old
workflows. New scoped tasks require a supporting worker minor; they are never flattened and handed
to an old worker. Claim and replay use the pinned workflow definition and explicit stored capability data, not
a distinction between public workflow revisions.

**Rationale**: Neither a TypeScript-only builder nor a silently permissive parser establishes five
SDK parity. Architecture acceptance is recorded; canonical contracts, migrations, fault suites and all-five
conformance still precede advertising each primitive. Examples using today's APIs can be delivered independently now.

## Accepted bounded product decisions

The authorized implementation scope adopts these defaults through ADR-0030/0031/0032. They define the
first increment; their runtime evidence is still required:

1. Collections fail fast on the first permanent iteration failure; there is no collect-all mode.
2. One eligible decision settles an approval; no quorum, delegation or escalation.
3. The initiator is excluded from deciding by default.
4. An approval decision carries an optional `reasonCode` (1..64 ASCII), never free text.
5. Query/Update are declarative expressions only; no asynchronous handler that runs activities.
6. Only Updates change declared state; no node assigns state or waits on it.
7. Continue-As-New keeps the same pinned version; no cross-version continuation.
8. Ceilings: 10,000 items/iterations, concurrency 256, depth 8, 100,000 instances per run, 32 cases,
   32 queries and 32 updates, 32 approval principals, 10,000 segments per chain.
9. Implement all six runtime capabilities through dependency-ordered slices with per-primitive
   release gates; completing a subset does not complete the full feature.

## Remaining gates, not unanswered semantics

ADR-0030 accepts scoped identity, compensation, lock order, worker negotiation, budget reserves
and the defaults above. ADR-0031 separately accepts explicit result disclosure and operation grants.
Production ceilings still need measurement; canonical contracts, engine/SDK code, nested admission,
10,000-item materialization, migration/replay and runtime fault/security/conformance evidence remain
pending. Architectural acceptance proves none of them. External sources were read on 2026-10-14;
local ADRs remain the normative Sagat decisions.

## Decisions of 2026-10-17

Taken while closing the remaining tasks. D294–D298 are the owner's; the others follow the accepted
contracts (`contracts/README.md`, the OpenAPI) and only align the code or the task text with them.

- **D291 — A conflicting declared Update answers `CONFLICT` (409).** The code answered
  `VALIDATION_FAILED` with status 409 for a revision conflict, a reused update key and a run that is
  not mutable; the contract says `CONFLICT`. The code is corrected and the reviewed
  `state-operations` golden is re-reviewed, not re-blessed blindly.
- **D292 — The four reasons the code already uses join the contract table.** `INVALID_ARGUMENTS`,
  `INVALID_UPDATE_KEY`, `INVALID_DECISION_KEY` and `INVALID_REASON_CODE` are added to
  `contracts/README.md` §5 with their remediation rows, and a contract test ties the table to the
  OpenAPI and to `crates/api-http/src/error.rs`.
- **D293 — Chain command keys follow the existing `Idempotency-Key` rule (8..160).** The task text
  said 1..128; the contract and the shared header win, and T014-037 is amended.
- **D294 (owner) — A loop's `onError` routes every loop failure.** A permanent body failure, the loop
  timeout and an expression error route to `onError` after the active iteration drains, as the
  iteration limit already does.
- **D295 (owner) — Approval combines with declared state and with chains; not inside bodies.**
  Approvals inside collection or loop bodies wait for scoped compensation.
- **D296 (owner) — A continuation boundary waits for quiescence.** Reaching `continueAsNew` with open
  work waits until it settles (or the run's deadline ends it `TIMED_OUT`);
  `CONTINUATION_NOT_QUIESCENT` is not a terminal outcome.
- **D297 (owner) — The top-level `limits` block is publishable.** An author may lower
  `maxInstances` and depth below the platform and tenant ceilings, never raise them; checked at
  publication and enforced at admission (WP-12).
- **D298 (owner) — Retention of finished runs stays a pending decision.** Nothing is deleted. Task
  clauses about cleanup are amended to "nothing is removed; a future retention must keep chain,
  receipt and blob references", and `NOT_FOUND after retention` is out of scope.
- **D299 — `GET /chains/{chainId}/runs` is built with cursor paging,** as the contract plans.
- **D300 — Task text names the files where the work lives.** Where a task names a file that was
  never created (`primitives.rs`, `transactions.rs`, `collections.rs`, `approvals.rs`, `chains.rs`,
  `chain.rs`), the task is amended to the actual file when it is closed; no empty files are created.
- **D301 — Starting a continuation chain as a child is refused with a typed error** until child
  chains are enabled, instead of failing with `INTERNAL`.

## Decisions of the loop onError increment

Implements D294. Evidence is in `crates/domain/tests/primitive_loops.rs`,
`tests/integration/workflow-primitives/{loop_progress,loops_http}.rs` and
`tests/fault/workflow-primitives/loops.rs`.

- **D340 — A loop that declares `onError` drains on the loop scope alone.** A permanent body
  failure, the loop timeout and an expression error write no `run.stop_requested` and do not move the
  run to `CANCELLING`. The drain start, the iteration drain and the finish keep the same
  transactions, fence checks and idempotency receipts as before; the run keeps the status its steps
  imply (`RUNNING`/`WAITING`). Whether a drain takes this path is derived from the pinned immutable
  definition (the owner declares `onError`), not stored, so no column is added and journals written
  before this change replay unchanged. A loop without `onError` still stops the run with
  `FAILED` and the failure code, then fails it after the drain.
- **D341 — The finish settles the owner and takes the edge in one transaction.** After the last
  iteration is drained, `finish_drain` ends the loop scope `FAILED`, settles the owner step `FAILED`
  (code retained, `routed`), journals `loop.finished`, `step.routed` and then the successor's own
  events, and reuses `route_loop_successor`: an activity target becomes `READY` with its
  `task.ready` outbox row, an `end` target finishes the run through the normal terminal path
  (`end.reached`, `run.finished`, the `run.finished` hint and the outbound fan-out), any other kind
  becomes `PENDING` with a `loop.finished` hint whose status is now `FAILED` for an error edge. No
  event type is added; the domain reducer and `RunState::replay` already accept the sequence.
  Tenant iteration usage and run instance reservations are released by the iteration drain, once.
  A duplicate delivery of the finish reports the same run version by reading the journal.
- **D342 — A cancel that lands during a failure drain wins.** A run-level cancellation always ends
  the run `CANCELLED` and never takes the edge. If it commits while a failure drain is in flight,
  the drain finishes under it: the loop still ends `FAILED` with its code, the run ends `CANCELLED`.
- **D343 — Discovery now surfaces the loops the scheduler must drain.** `loop_candidates` found only
  a loop with a returned body or a due delay, so a failed body, an expired loop deadline, a
  cancelled run and an unfinished drain were never seen by the scheduler. It now also returns a
  `RUNNING` scope with a failed member or an expired deadline, any `DRAINING` scope, and a not yet
  draining scope of a run stopped by `CANCELLED`. This also makes the no-edge failure path reach
  the scheduler.
- **D344 — The stopping planner leaves a draining loop owner alone.** `plan()` on a stopping run
  emitted `StepCancelled` for a loop owner that was `WAITING` on its drain, taking the drain from
  under the scheduler. A draining owner is now skipped, as a draining collection already was.
- **D345 — Publication accepts an `end` as the loop `onError` target.** The public loop subset is a
  loop chain ending at `end`, so only an `end` target can be published and run over HTTP. The
  activity target is proven at the storage level (including a worker completing the recovery
  activity and the run finishing), not through publication.
- **G340 — An `onError` edge to an activity is not publishable.** Widening the public loop subset to
  allow root activities after a loop is a separate publication decision; nothing here changes it.
- **G341 — An expression error at loop entry is not an iteration failure.** Evaluating the initial
  carry or condition fails before a scope or an iteration exists, so there is no drain to wait for.
  It still stops the run `FAILED` with `EXPRESSION_ERROR`. Routing it over `onError` needs a way
  to fail a `PENDING` owner (a new journal event or a scope created already failed); decide before
  building it.
- **G342 — Any other stop landing during a failure drain fails closed.** Only `CANCELLED` is
  handled (D342). A different stop reason (for example a storage-quota stop) during a failure drain
  leaves the finish refused with `InvalidTransition` rather than inventing a final status.
- **G343 — A public cancel of a run whose loop is active returns 500.** `plan()` fails closed with
  `UnsupportedStructuredNode` for a `READY`/`RUNNING`/`DELAYING` loop under a stop request, the
  cancel transaction rolls back and the API answers `INTERNAL`. This predates D294 and is the open
  "outer cancel through the domain planner" part of T014-019. The storage-level cancel drain is
  proven; the public cancel is not.

## Decisions of the limits/projection increment

Taken while implementing D297 (publishable top-level `limits`), the byte ceilings of T014-008 and
the graph/telemetry projection of T014-045. D298 is respected: nothing is deleted, and a test
asserts that no scheduled job removes primitive data.

- **D310 — The planner enforces the author's instance limit on root activations.** When a
  definition lowers `maxInstances` below the platform's 100000, the planner counts the instances the
  run has charged (every scoped instance, and each root instance that left `Pending` other than by
  cancellation, the `end` node included) and, instead of activating the next pending root node,
  stops the run with the durable code `INSTANCE_LIMIT`. The check is skipped while the limit is the
  platform's, so no old document pays for it and none changes behavior. The store's trigger stays
  the last guard for budgets that tenant policy lowered.
- **D311 — Run creation captures the effective limits; they only go down.** The row of
  `run_instance_usage` starts at 100000/8 and is set to `least(row, author)` when the run is
  created, the chain successor included. A later definition can never raise what tenant policy
  lowered. `maxInstances` bounds each segment of a chain; a chain-wide instance budget is not
  claimed.
- **D312 — Collections are refused before they start.** Entry refuses a collection whose
  iterations cannot all be admitted (`consumed + 1 + items x body nodes > budget`) with
  `INSTANCE_LIMIT`, and a collection given more items than `maxItems` with `ITERATION_LIMIT`; both
  stop the run at entry, with nothing admitted. A tenant policy that lowers the budget *after*
  entry pauses further admission (counted in `scheduler_admission_paused_total`), keeps the
  accepted work and resumes if the budget is raised; it is not a failure.
- **D313 — A loop stops on the instance budget with its own checkpoint continuation.**
  `LoopContinuation::InstanceLimit` is recorded, like `IterationLimit`, when the next body (or, once
  the loop is done, the node that follows it) cannot be admitted; the completed iterations are kept.
  The scheduler decides it from the author's limit before any delay is scheduled, and falls back to
  it when the store reports a budget tenant policy lowered. The store re-proves it against the real
  budget. It is not routed to `onError`, which would need an instance of its own, and it reuses the
  `loop.iteration_limit` hint (AsyncAPI description updated) so no new subject appears.
- **D314 — Inline values keep the existing 256 KiB / 4 MiB / 8 MiB ceilings.** A collection snapshot,
  an iteration result, the aggregate, a loop's initial carry, result and carry are each at most
  256 KiB; the aggregate or the carry that ends a loop must also fit with the outputs the run
  retains (4 MiB). The aggregate is checked as a dry run when the last iteration is checkpointed,
  while that iteration can still fail; nothing oversize is written. Over a ceiling the failure is
  `RUN_DATA_LIMIT`. Member outputs were already held to 256 KiB and 4 MiB by the worker path, for
  scoped instances too; this is now tested. A scoped task context holds the run input, state, the
  item or carry and the iteration's own outputs, so it stays far below 8 MiB (tested with
  250 KiB input and carry).
- **D315 — A result that cannot be produced fails its iteration instead of retrying forever.** A
  collection whose `return` expression fails, or whose result is over a ceiling, drains with that
  iteration's `EXPRESSION_ERROR` or `RUN_DATA_LIMIT`; the store accepts the claim when the
  iteration's entry succeeded and its `return` is still pending, as it already did for loops.
- **D316 — The graph names approval routes and decisions, and the projector names what it cannot
  read.** Approval nodes get `approve`, `reject` and `expire` edges and an `approvalDecision`
  (`PENDING`, `APPROVED`, `REJECTED`, `EXPIRED`, `CANCELLED` with the pinned target), never the
  approver, key or reason. `waitingOn` also reports `collection`. A journal entry of an unknown kind
  becomes one `workflow.unsupported` span and one `unsupported` transition (the closed label sets
  grow by one value each); its type and payload are never copied.
- **G294 — The approval decision fails under the least-privilege runtime role.**
  `approval_progress.rs` reads the decider's membership with `SELECT ... FOR SHARE`, which needs
  `UPDATE` on `memberships`; `sagat_app` has `SELECT` only, so a decision over HTTP answers 500
  (`permission denied for table memberships`) when the API connects as the runtime role. The
  decision tests connect as the owner. The fix needs a decision: grant the lock, or check
  eligibility without it.
- **G295 — The iteration-limit checkpoint that takes an `onError` edge needs a spare instance.**
  The loop store charges the error route's node when the checkpoint for `ITERATION_LIMIT` routes
  there (`terminal.is_some()` in `checkpoint`), so with the run's instance budget exhausted that
  checkpoint cannot commit and the scheduler retries it. D340 (the drain routes) was built on the
  base branch and this increment did not change that charge; `INSTANCE_LIMIT` itself is never
  routed. If the edge must survive an exhausted budget, the route needs a reserved instance.
- **G296 — Mid-flight collection exhaustion by other work waits.** Collection entry proves the whole
  collection fits, so only a tenant reduction or other root work spending the budget can pause
  admission later; the public shapes cannot do the latter. If a shape ever can, the pause needs a
  deadline-bound failure.
- **G297 — A collection `result` expression cannot read `item` or `index` once the iteration is
  done.** `contracts/README.md` says the expression reads `bodyResult`, `item` and `index`, but the
  run data loads scope inputs only for iterations still `ADMITTED`
  (`infra-postgres/src/execution.rs`, `collection_inputs`), so `result: "index"` fails on every
  finished member. Before this increment the finish retried that for ever; it now fails the scope
  with `EXPRESSION_ERROR` at the last checkpoint (D315). Making the contract true needs the
  scope inputs of checkpointed iterations loaded too (bounded by the 256 KiB snapshot); it is left
  for the collection owner.

