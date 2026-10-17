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


## Runtime privilege audit

G294 showed that the suites cannot see a missing privilege: they connect as the database owner, and
`sagat_app`, the role the control plane and the engine are required to run as, was refused a row lock
that the owner takes without noticing. This increment asks the same question of every statement the
stores issue instead of waiting for the next one.

- **D350 — The privileges the runtime role needs are derived from the statements, not from a list
  kept by hand.** `tests/integration/platform-foundation/runtime_privileges.rs` reads the SQL
  literals out of every non-test `src` directory of the workspace (`rust_source.rs`: `\`
  continuations, raw strings, `concat!`, `macro_rules!` select templates, `const` fragments named in
  `format!`), turns each statement into the privileges it asks for (`sql_needs.rs`: SELECT, INSERT and
  UPDATE per column, DELETE, TRUNCATE, row locks, function calls, `ON CONFLICT DO UPDATE`,
  `RETURNING`, data-modifying CTEs), and asks the catalog of the fully migrated schema whether
  `sagat_app` holds each one. A new statement or a migration that drops a grant fails the test with the
  file, line and statement.
- **D351 — A row lock is modelled as UPDATE on at least one column, and the model is checked against the
  server.** For every need the test also runs the smallest equivalent statement as `sagat_app` inside a
  transaction it rolls back (`SELECT 1 FROM t WHERE false FOR NO KEY UPDATE`, `UPDATE t SET c = DEFAULT
  WHERE false`, `INSERT INTO t (c) VALUES (DEFAULT)`, ...) and reads SQLSTATE `42501` as a refusal. Any
  disagreement between the catalog and the server fails the test, so the audit cannot drift from what
  PostgreSQL does. 564 of 593 distinct needs have a probe; function calls and unresolved placeholders are checked by the catalog only.
- **D352 — What the role is not meant to run is listed with a reason, and an entry that excuses nothing
  fails.** `ALLOWED` holds `provision-tenant` and `seed-dev` (`tenants`, `memberships`), `scheduling-weight`
  (`tenant_scheduling`, G260), the unused `apply_content_delta` (`tenant_retained_usage`), and two
  statements assembled at run time. Nothing else is excused.
- **D353 — The triggers a write fires are held to the same rule.** A trigger function that is not
  `SECURITY DEFINER` runs with the caller's privileges, so its body is read with the same scanner, and
  the triggers it fires in turn are followed. The functions that CHECK constraints and column defaults
  call, and the sequences behind `serial` defaults, are checked for EXECUTE and USAGE. Triggers that are
  `SECURITY DEFINER` run as the owner and are listed, not followed.
- **D354 — The audit proves it catches the defect it exists for.** A second test feeds it the statement
  of G294 (`FOR SHARE` on `memberships`), an `UPDATE`, a `DELETE` and a `TRUNCATE` of `audit_records`, a
  write to a table the role may not insert into, a column that does not exist, a renamed table, a
  `SECURITY DEFINER` function the role may not call and a `serial` key whose sequence the role may not use,
  and requires each to be reported; a third feeds it statements the role is meant to run and requires
  silence.
- **D355 — Representative write paths run as the runtime role, and the API-backed suites can be re-run
  as it.** Loop entry and checkpoint, collection entry, admission, checkpoint and finish, chain handoff and
  finalization, an applied declared Update and the secret store (put, rotate, grant, revoke, delete) each
  have a test whose store is connected as `sagat_app` while the fixture is seeded by the owner.
  `SAGAT_TEST_AS_RUNTIME_ROLE=1` connects the API of every API-backed test as `sagat_app`.

Result of the first run: no statement outside `ALLOWED` lacks a privilege. G294 was the only gap of its
class in the statements the stores issue today; the audit exists for the next one.

- **G350 — The extractor cannot see SQL that is not a literal in the Rust source.** The two statements
  built from a `format!` argument at run time (`operations.rs` `{groups}`, `telemetry.rs` `{condition}`)
  are reported as unresolved and allowlisted; the pieces they are made from are literals that are audited
  on their own. A statement assembled by `push_str` from parts that each look like a clause would be
  missed unless the part carries a verb or a lock clause, which the fragment test (`no_literal_outside_a_
  statement_carries_a_lock_clause_or_a_write`) then rejects. Statements of a kind the audit does not model
  (`LOCK TABLE`, `COPY`, DDL, `NOTIFY`) fail the test when they appear in capitals in the sources; there is
  none today.
- **G351 — The audit proves the migrations, not the deployed database.** It uses the grants the
  migrations make. A role widened or narrowed by hand afterwards, a restore that loses the grants, or a
  `sagat_app` created after `migrate` (the grants are skipped when the role does not exist) is not seen;
  `\dp` on the deployed database after those events is the check (control-plane runbook, sections 1 and 7).
- **G352 — Not audited: the bodies of `SECURITY DEFINER` functions, foreign-key checks, column-only
  `SELECT` grants.** Definer functions and the referential checks run as the owner; the audit checks that
  the role may call the definer functions the stores call and nothing about what they do. A table the role
  can read only through column grants would be reported as lacking `SELECT`; there is none.
- **G353 — `control-plane scheduling-weight` needs the owner role** (G260), and `apply_content_delta` is
  dead code that would fail as the runtime role. Both are allowlisted with that reason. If the weight
  should be settable with the runtime role, `tenant_scheduling` needs a column-level `INSERT`/`UPDATE` and
  a `DELETE` grant in a new migration and the entries come out of `ALLOWED`.

## Decisions of the SDK scenario increment

Taken while extending the shared five-SDK conformance (WP-03, WP-05, WP-06, WP-14). Each one comes from
a scenario or a fault test that failed first; none changes a contract, they make the runtime do what the
contracts already say.

- **D330 — The scheduler releases the route a settled approval chose.** A decision or an expiry commits
  the chosen route as a `PENDING` root step and nothing else; before this the successor never ran in a
  served engine, so a run stayed `RUNNING` after its approval was decided. The `approval_routes` scheduler
  pass finds those steps and releases them through the planner (`release_pending`), once, under the shard
  lease; a crash between the outcome and the release is found again by the next pass.
- **D331 — A settled request answers from its outcome whatever the run did next.** A decision for a
  request that has its outcome replays the receipt (same key, same body), answers `IDEMPOTENCY_KEY_REUSED`
  (same key, other body) or `ALREADY_DECIDED`, also when the run already ended. Only a request still open
  on an ended run stays `UNAVAILABLE`.
- **D332 — A collection's `result` expression reads `item` and `index`.** The contract allows it; the
  aggregate was built from run data that no longer held the ended iterations' inputs. The scheduler restores
  them from the captured snapshot before evaluating.
- **D333 — A run stopped by its own transaction is ended by the scheduler.** A loop at its iteration limit
  records `CANCELLING` with its code and leaves the draining to the planner; the `stopped_runs` pass settles
  such runs when no attempt holds them, so the run ends `FAILED` with `ITERATION_LIMIT`.
- **D334 — A task in flight is keyed by its task id and its instance path.** The legacy task id is
  `run/node/generation`; iterations of one scope that are leased together share it. The Java host dropped
  the second lease as already in flight (it lapsed and was redelivered), the TypeScript host counted one
  slot for two. Both now include the instance path (protocol 1.5).
- **D335 — An approval deadline cannot be moved by the control endpoint.** The deadline is part of the
  journal's `approval.opened` event, which the projection check compares; changing the row alone makes the
  request unreadable. Expiry is observed with a short real timeout, and the order "decision first" with a
  later run that expires after it.
- **D336 — The shared suite pins what a revoked member sees.** After its membership is removed the member
  is `UNAUTHENTICATED` (401) for listing and deciding, and no approval audit row is written for it because
  the call never reaches the approval. The store-level fault test keeps the `NOT_ELIGIBLE` denial for a
  principal that is resolved but not eligible.

Open, recorded rather than closed:

- **G320 — Report idempotency keys of scoped iterations.** `sdks/SPEC.md` derives `done-<taskId>` from the
  legacy task id, which iterations of one scope share. The server accepts the reports (its idempotency is
  scoped by step identity), so nothing fails today; the key should carry the instance path in the next
  protocol minor.
- **G321 — A failing collection `result` expression still stalls the run.** The expression error is
  returned to the scheduler pass, logged and retried; it neither fails the run nor reaches the caller.
  D332 removes the one cause the scenarios found; the general case needs a durable failure.
- **G322 — The commit order of a decision racing an expiry is not observable through an SDK.** The shared
  scenarios cover both orders by sequence; the race itself is exercised by the barrier tests in
  `tests/fault/workflow-primitives/approvals.rs`.
- **G323 — Not done in this increment:** scoped compensation and reverse-iteration compensation (WP-04,
  WP-05), limit scenarios (WP-12), child chains (WP-11), direct-root `waitEvent`, and `onError` routing of
  loop body failures (D294).

## Decisions of the operations fault increment

Task T014-033. The fault cases live in `tests/fault/workflow-primitives/operations.rs` and its child
modules in `tests/fault/workflow-primitives/operations/` (`races`, `crash`, `mutability`, `query_invariance`,
`redaction`, with the shared fixture in `engine`). They do not repeat an integration test: each is a
separate scenario against real PostgreSQL.

- **D360 — A race is ordered by the database's lock queue, never by a sleep.** The test takes the row
  lock of the run in a transaction of its own, starts the first contender, waits until PostgreSQL reports a
  backend in a `Lock` wait (`pg_stat_activity.wait_event_type`), starts the second, waits for two, and only
  then lets go. The first contender is queued ahead of the second, so both commit orders are forced and
  each is observable: an Update that ran first returns `0` for a step that completed after it and `7` for
  one that completed before, and the journal carries `state.updated` before or after `step.completed`,
  `timer.fired` or `wait.timed_out`. Contenders: Update against a worker completion, a firing timer and a
  wait timeout (both orders each), and two deliveries of one update key. A contender that finishes while
  the row is held fails the test, so a contender that never queued cannot pass.
- **D361 — A crash is the two ways the database sees a client die.** (a) The Update future is dropped
  after its Nth pending poll for every N from one until the update completes (33 cut points today, 10 of
  them after the commit and 23 before), on a one-connection pool, each on a run of its own. (b) The
  connection is ended with `pg_terminate_backend`, once while the Update waits for the run lock and once
  inside the serialized section after the decision is made (a gate in the evaluate callback, on a thread
  and runtime of its own because the gate blocks its thread). After every case: the state revision,
  receipt, `state.updated` event, outbox row and applied-update audit row stand or fall together, no
  session is idle in a transaction and `now()` moves on the connection, and a new pool and store replay the
  same key to exactly one receipt (`replayed` is true exactly when the first attempt had committed) and
  `verify_run` agrees.
- **D362 — Query invariance is proven against a control run, not against a list of columns.** Two runs
  of one workflow take the same three writers (a completion, a firing timer, the final completion); the
  probed run has a burst of Queries queued on the run lock together with each writer and so races it. After
  every writer the probed run's status, run version, journal, step rows, row count of every table with a
  `run_id` column, outbox and non-Query audit rows equal the control's. With the writers quiet, a further
  burst leaves every row of every such table, and the run row, byte for byte as it was (`updated_at`
  included). Every answer is a committed version and one coherent snapshot: the completed result shows
  from the version that committed it on. A second test shows the two detectors see a one-microsecond
  timestamp change and one extra journal event.
- **D363 — The Query access audit collapses repeats.** `insert_audit` writes an identical repeated access
  once, so a burst of Queries leaves one `run.queried` row per operation asked, not one per Query. The
  test asserts that (one row for `balance.read`, one for `work.read`, none for the control run). The audit
  therefore cannot be used to count how often a run was read.
- **D364 — A refused Update is not remembered, and a receipt outlives the run.** In every status except
  `RUNNING` and `WAITING` (taken from `RunStatus::ALL`, so a new status is tried automatically) the HTTP
  answer is `409 CONFLICT` with reason `RUN_NOT_MUTABLE`, and the run version, state revision, receipts,
  journal, `state.updated` events, outbox and `run.updated` audit rows are unchanged; a Query still
  answers from the last snapshot. The same key is refused again for the same reason, and once the run is
  back in `WAITING` it applies; after the run ends, that applied key replays its receipt.
- **D365 — The redaction sweep reads stored bytes and proves it can.** A worker resolves a real secret
  for a step of the run that is then queried and updated, and a credential-shaped marker goes through
  every refusal path (query argument, operation name, update property, value, key, validator, stale
  revision, key reuse). Neither string appears in a response or error body, in any table (searched as text
  and as the hex a `bytea` prints, so the payload store is covered), or in the spans (with their events),
  log lines or metrics of the requests. The control: the run's own input, stored in `payloads`, is found by
  the same sweep. The fake collector gained `install_with_logs`, which adds a log buffer to the span
  exporter in one subscriber.

- **G360 — No process death, no server crash.** A test cannot kill its own process, and the container
  is shared. The dropped future and the ended connection are what PostgreSQL observes of a dead client; a
  crash of the PostgreSQL server (recovery from the write-ahead log) and a power loss are not exercised
  here. The failover suite (`tests/fault/failover`) is where a server restart belongs.
- **G361 — Update against a worker, a timer and a wait timeout is proven at the store and scheduler, not
  over HTTP.** The Update runs through `PgDeclaredOperationStore` and the production evaluator exactly as
  the route calls them (apply, then the retained result); the worker completion and the scheduler tick are
  the production services. The HTTP-level Update against a signal is the existing
  `update_and_signal_race_serialize_without_losing_either_transition`.
- **G362 — The order of two lock waiters is PostgreSQL's, not a contract.** The tests depend on a row
  lock being granted to waiters in the order they queued. That holds for the row lock queue today; if it
  stopped holding, the order assertions (the `0`/`7` result and the journal position) fail instead of
  passing, so the dependency is loud.
- **G363 — Telemetry text is what the test process exports.** Spans reach a fake OTLP collector, log lines
  are captured on the test's own runtime thread, and metrics are read from the process-wide registry. A real
  collector, a log shipper and a metrics scrape are not in the loop.
- **G364 — Non-mutable statuses are set with SQL.** The engine's own paths into each status (pause,
  cancel, compensation, manual review, each terminal) are proven by their features' tests. What is proven
  here is that the Update refuses every status and writes nothing, not that the engine reaches it.


## Decisions of the chains increment

Taken while closing US07 (T014-034..039). D296, D298, D299 and D301 are the owner's earlier decisions
(see "Decisions of 2026-10-17"); the entries below say how they were built and what the tests found.

- **D370 — A continuation boundary waits for open work, and says which.** `Quiescence::of_run`
  (`crates/domain/src/execution/continuation.rs`) reads the run's own state: every step other than the
  boundary must be finished and routed and every saga done. The most significant open obligation is the
  blocker, in the fixed order compensation, unknown effect, approval, child, lease, timer, subscription,
  sibling work. `ChainError::is_transient()` is true for `NotQuiescent` and for nothing else. The
  scheduler returns `ContinuationAdvanceError::Waiting`, counts it in
  `Report.continuations_waiting` (not in `did_work`) and in
  `scheduler_continuation_waits_total{blocker}`, and logs nothing at error level. The store proves
  quiescence again under the chain and run locks and names the exact blocker (`typed_segment_blocker`).
  `CONTINUATION_NOT_QUIESCENT` is a wait reason, never stored; the contract table row says so. The run's
  original deadline still ends the chain `TIMED_OUT` (`quiescence::the_original_deadline_ends_...`).
- **D371 — Candidates that cannot hand off yet are not candidates.** `continuation_candidates` leaves out a
  boundary whose run has open steps, an unfinished saga, a pending approval or an unsettled child, so a
  blocked boundary no longer sits at the head of the oldest-first list and starves the ones behind it
  (`chains::a_blocked_boundary_does_not_hold_back_...`, red with `batch = 1` before the filter).
- **D372 — Waiting for room is the same wait.** A namespace or tenant without room for the successor makes
  the boundary wait (`blocker="capacity"`) instead of failing every tick. The successor replaces the
  predecessor, so a limit equal to the runs now active is enough and a limit of 0 waits
  (`chains::a_continuation_never_outgrows_the_active_run_quota_...`; the write re-proves it under the lock,
  `fault chains::a_quota_lowered_while_the_hand_off_waits_...`).
- **D373 — A continuation workflow runs as a child (D301 built, not only refused).** No typed refusal was
  needed: the creating transaction writes the chain, its first segment and the `child_chain_links` row
  (named by the parent's child step) together, and creates the chain's runs settled so the ordinary
  child-completion pass ignores them. The link, not the run, tells the parent: `ended_children` reports a
  chain only when its final outcome is set, its link is unsettled and the parent step still waits for it
  (`child = chain id`, as the step stored it). The ordinary `ChildFinished` path then settles the link. The
  narrow `settle_child_chain` prototype (a parent that is itself a chain, one activity successor, `SUCCEEDED`
  only) and its three seeded tests are removed; `chains::the_parent_hears_only_the_final_outcome_...`,
  `fault chains::the_final_outcome_of_a_child_chain_reaches_the_parent_once_...` and `..._a_lost_wake_...`
  replace them with the real path. The chain deadline is the smallest of the parent's, the node's and the
  chain's required timeout.
- **D374 — Parent cancellation latches the link under the parent's lock only.** `release_child` and
  `settle_child` set `child_chain_links.cancel_requested` (and `settled` once the final outcome is applied);
  the hand-off reads the latch `FOR UPDATE` under the chain and run locks and refuses; the cancel pass
  (`children_to_cancel`) also picks the current head of every latched chain. `DETACH` leaves the chain running
  and tells nobody (`chains::a_detached_child_chain_...`); `REQUEST_CANCEL` keeps the parent waiting until the
  chain settles (`chains::request_cancel_makes_the_parent_wait_...`). Both orders of cancel against hand-off
  are tested with held locks (`fault chains::parent_cancellation_and_a_child_hand_off_...`).
- **D375 — Lock order is parent, chain, head run (found by the race above).** The first run of the ordered
  race ended with `deadlock detected`: the successor insert takes a key-share lock on the parent row (the
  foreign key) after the hand-off holds the chain and the head run, while the parent holds its own row and
  wants the child's. Every other writer of a child locks the parent first. `hold_parent` now takes the parent
  key-share lock in `lock_chain`, before the head run is locked. Red before (the parent's cancel answered
  `UNAVAILABLE`), green after.
- **D376 — The tree budget ends a chain with `CHAIN_LIMIT`.** `MAX_TREE_RUNS` (1,024) moved to the domain
  and `ChainError::TreeBudgetExhausted` is the store's answer when the next successor would exceed it. Unlike
  an active-run limit, a full tree never frees, so waiting would only wait for the deadline; the scheduler
  finalizes the chain `FAILED` with `CHAIN_LIMIT`, the code the segment limit already uses, and the parent
  hears it once as a child failure (`chains::the_last_run_the_tree_budget_allows_...`: 1,023 members accept
  the successor, 1,024 refuse it and leave no run).
- **D377 — Lifetime counters are derived from rows nothing deletes.** The chain's segment count
  (`execution_chains.segment_count`, `CHECK <= max_segments`) and a tree's run count (`runs.root_run_id`) only
  grow, and this is asserted rather than assumed: no housekeeping pass changes the row counts of runs, chain,
  segments, links, payloads or declared states with every retention window at zero, and the runtime role has
  no `DELETE` on them (`chains::finished_chains_links_and_payloads_survive_every_housekeeping_pass`). No
  counter table was added. A future retention (D298) must keep chain, link and payload references and move
  the counts it removes into a tombstone first.
- **D378 — `GET /namespaces/{namespaceId}/chains/{chainId}/runs` is built (D299).** Bounded page (default
  50, clamped to 1..200), segment order, a two-byte opaque cursor that is the last index returned (a segment
  is only appended, so a page never repeats or skips one), `run:read`, the denial audit and the single 404 of
  chain inspection, never cached, no payload. The path carries the namespace, as the other chain operations
  do; `contracts/README.md` said `/chains/{chainId}/runs`. `ChainSegment.index` in the OpenAPI was tightened
  from 65,535 to 9,999 to match the 10,000-segment ceiling.
- **D379 — A run read names its segment.** `Run.chain` (`chainId`, `segmentIndex`, `completionKind`,
  `successorRunId`) is additive and absent outside a chain, so the segment wait of the contract returns
  `SUCCEEDED` with `CONTINUED` and a client that predates it sees an ordinary succeeded run
  (`chains_http::a_continued_segment_is_an_ordinary_succeeded_run_...` compares the key sets). The five SDKs
  parse it and expose `listChainRuns` (`listAll` follows the cursor).
- **D380 — Chain command keys stay 8..160 (D293 built).** The code already used the shared
  `Idempotency-Key` rule; `chains_http::chain_commands_take_a_key_of_eight_to_one_hundred_sixty_...` pins
  7/8/160/161 and a space, and the contract test ties both chain commands to the shared parameter. T014-037's
  "stable chain keys (1..128)" should read "(8..160)"; `tasks.md` is the coordinator's to amend.
- **D381 — A run-addressed operation on a continued segment is `CONFLICT`.** A cancel of an earlier segment,
  or of a chain that is already latched or finished, tried to latch the chain, found nothing to latch and
  reported corruption (`INTERNAL` 500). The `chain-three-segments` scenario first recorded that 500 as the
  expected result. Only the open head of an open chain latches now; any other run is refused by the reducer
  as an ended run is.
- **D382 — Ordered races use held locks, not sleeps.** A test transaction holds a lock; each competitor is
  started and the test waits until `pg_stat_activity` shows the backend blocked on a lock before it starts
  the next; releasing the gate commits them in arrival order. A hand-off reads in one transaction and writes
  in another, so there are two gates: the chain row (who looks first) and the quota advisory lock the write
  takes after the chain and run locks (a hand-off already committed to winning). A dropped connection is
  `pg_terminate_backend` of a waiting hand-off.
- **D383 — Chain runs compare as one instant in the SDK goldens.** A run with a `chain` has its `deadline`
  normalized to `<chain-deadline:N>` by the instant, not `<createdAt+Ns>`: every segment must show the same
  one, and a successor that moved it would show another. The server state of a scenario is listed by creation
  time, because the first run of a child chain has an id derived from its parent's that sorts anywhere among
  time-ordered ids.

- **G370 — No SDK scenario cancels a parent mid-chain.** The harness has no way to hold a segment open
  deterministically (a worker behaviour is the same for every segment, and the child's chain id is not an
  observation). Parent cancellation and both race orders are proved in Rust only (D374).
- **G371 — `RERUN_AS_NEW` has no SDK method**, so there is no five-SDK scenario for it; the unrelated run with
  its own chain, its own fresh deadline and `rerunOf` is proved by `chains::rerun_as_new_of_a_segment_...`.
- **G372 — A continuation whose entrypoint is a `waitEvent` (`chain-root-wait.json.pending`) is still closed.**
  The successor is created with an activity step and its `task.ready` hint; a wait needs the evaluated
  correlation key, the deadline, the subscription rows and `wait.started` written in the same transaction, a
  new `ContinuationSelection` carrying them, the publication shape widened, and the same through five SDKs.
  Not small, so it was not started.
- **G373 — A wait-shaped chain cannot race a signal against the hand-off.** Until the signal is consumed the
  boundary is not pending, and once it is there is no wait left to signal; the meaningful races (signal first,
  hand-off first) are on the activity-shaped chain and the wait-shaped signal-then-hand-off order is exercised
  by the existing `signalled_wait_handoff_rolls_back_with_outbox_and_retries_once`.
- **G374 — `contract_grpc_durable_execution::protocol_compatibility::mixed_workers_only_lease_run_input_blobs_to_protocol_1_3`
  fails on the base branch** (expects 1.4, gets 1.5), with and without this increment. It is not a chain test.
- **G375 — `inspectChain` still returns every segment.** At most 10,000 small entries, but unbounded for a
  long chain; clients that only need lineage should page `GET .../runs`. Bounding inspection would change an
  existing response and was left alone.
- **G376 — Studio has the generated types, not a screen.** `frontend/apps/studio/src/api/schema.d.ts` follows
  the OpenAPI; no component lists a chain's runs.

### Merge of `develop` (owner decisions) into this branch, 2026-10-17

- **G377 — The pool check of G48 took a row lock the runtime role cannot take.** The privilege audit
  (`runtime_privileges`, G294) found `SELECT ... FOR SHARE` on `worker_binding_pools` in
  `crates/infra-postgres/src/secrets.rs`: row locks need UPDATE, which `sagat_app` does not hold, so
  a secret delivery to a worker would have answered `permission denied`. The grants are now held by
  `pg_advisory_xact_lock_shared` on the key that replacing or revoking a binding already takes
  exclusively (`binding:{tenant}:{namespace}:{subject}`), which has the same effect. The audit's
  extractor now expands a `macro_rules!` that takes no argument (`pool_refused!()`), so the claim query
  is audited instead of reported as unresolved.
- The reported-lease record (G141) and the declared secrets of a step (G48) are now addressed by
  execution instance, so a step inside a loop or a collection has both (`ReportedLease.instance`,
  `reported_by(instance, ...)`, the scoped step insert writes `secrets`).
- `Core` gained `record_access_denied` (G9) for the two 014 API states; `Outcome::DoesNotFit` is
  treated as a skip wherever the scheduler only handles `Condition::Always`.
