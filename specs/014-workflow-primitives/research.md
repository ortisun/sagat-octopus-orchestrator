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
- **D480 — Chain command keys stay 8..160 (D293 built).** The code already used the shared
  `Idempotency-Key` rule; `chains_http::chain_commands_take_a_key_of_eight_to_one_hundred_sixty_...` pins
  7/8/160/161 and a space, and the contract test ties both chain commands to the shared parameter. T014-037's
  "stable chain keys (1..128)" should read "(8..160)"; `tasks.md` is the coordinator's to amend.
- **D481 — A run-addressed operation on a continued segment is `CONFLICT`.** A cancel of an earlier segment,
  or of a chain that is already latched or finished, tried to latch the chain, found nothing to latch and
  reported corruption (`INTERNAL` 500). The `chain-three-segments` scenario first recorded that 500 as the
  expected result. Only the open head of an open chain latches now; any other run is refused by the reducer
  as an ended run is.
- **D482 — Ordered races use held locks, not sleeps.** A test transaction holds a lock; each competitor is
  started and the test waits until `pg_stat_activity` shows the backend blocked on a lock before it starts
  the next; releasing the gate commits them in arrival order. A hand-off reads in one transaction and writes
  in another, so there are two gates: the chain row (who looks first) and the quota advisory lock the write
  takes after the chain and run locks (a hand-off already committed to winning). A dropped connection is
  `pg_terminate_backend` of a waiting hand-off.
- **D483 — Chain runs compare as one instant in the SDK goldens.** A run with a `chain` has its `deadline`
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

## Decisions of the scoped compensation increment

Tasks T014-014 to T014-021 (the compensation and reconciliation clauses). This is the design note that
precedes the code. It extends the feature 003 saga machinery to the members of a collection or a
loop; it adds no engine beside it. Where a sentence says "built", the evidence is named in the
section "As built" at the end; everything else is the design the code is held to. It keeps the IR at
`v1` (ADR-0032), adds no top-level `Error.code`, and changes no behavior of a body that declares no
compensation, irreversible or high-risk activity.

The rule it implements is FR-003-002 as refined by ADR-0030 item 7 and FR-014-005: inside one
iteration, compensation runs in reverse topological order of the body's compensable steps; between the
iterations of a collection there is no order (they ran concurrently); the completed iterations of a
loop compensate in reverse iteration order.

- **D380 — A scope whose body can be compensated is its own saga.** A `forEach`, `while` or
  `repeatUntil` owner whose body has an activity with `compensateWith`, `irreversible` or `highRisk`
  (a *compensating scope*) acts as the coordinator of its iterations: when the scope stops for a
  failure of an iteration, a storage-quota stop, an outer cancellation or an expired deadline, the
  iterations that completed (and the ones whose last attempt had an unknown outcome) are compensated
  after the existing drain and before the scope finishes. No enclosing `saga` node is required (the
  public collection and loop subsets have none). The policies are the saga defaults:
  `onCompensationFailure: manual_review` and `cancelPolicy: compensate_completed` (in-flight attempts
  are asked to stop by the drain, completed ones are compensated). A scope with none of the three
  declarations behaves exactly as before: it fails or is cancelled with its code, and there is nothing
  to compensate. *Why:* ADR-0030 item 7 says a failed compensation or irreversible effect "preserves
  MANUAL_REVIEW/PARTIAL rather than being overwritten by a simple scope failure", and data-model.md
  says a parent scope cannot claim drain complete while a child instance has an unresolved
  compensation; both only make sense if the scope owns the compensation. *Open (O380):* whether the
  policies should be author-settable on the scope node (the fields exist on `saga` only). They are not
  added here; adding them is an additive IR change that needs its own decision.
- **D381 — Completed members are read from the instance state, and the plan is recorded once.** No new
  list of completed members is kept. The forward instance `(path, node)` is *completed* when its step
  is `SUCCEEDED`, and *unknown* when it ended `CANCELLED` or `FAILED` with a code that leaves the
  outcome unknown (`FailureCode::leaves_outcome_unknown`, the same predicate the root saga uses). When
  the drain has settled every iteration, one journal event, `scope.compensation_planned`, addressed
  at the owner, records the plan: for each entry the forward instance and the forward instances whose
  compensations must finish first, plus the instances that cannot be undone. The entries and their
  `after` links are projected into `scope_saga_entries` so a restart restores them; the replay of the
  journal must give the same projection (`verify_run`). The event is what `saga_plans` is for roots:
  the plan a *run* follows is decided from what actually completed, not from the definition.
- **D382 — Stable compensation identity.** The compensation of the forward instance
  `(scope path, iteration index, node)` is the instance `(same path, node~c)` and its lookup is
  `(same path, node~r)`: the derived nodes already exist in the body graph
  (`derive_companions`) and `ExecutionInstanceId::compensation()` already keeps the path. The step
  UUID is the existing path-aware `instance_step_id`. The idempotency key of a compensation is
  `compensation:{id}:{run}` where `id` is a version-8 UUID of SHA-256 over a domain-separated,
  length-delimited encoding of the workflow version, the path segments and the forward node; for a
  root instance the existing derivation is unchanged byte for byte. The key of a lookup is
  `reconciliation:{run}:{step uuid of the forward instance}`. The key the forward instance's own
  attempts carried travels as `target_idempotency_key`; the target node is the forward node name and
  its path is the task's own path, so worker protocol 1.5 needs no new field.
- **D383 — The order, as data.** Within an iteration, `after` is `reverse_dependencies` over the body
  graph restricted to the entries of that iteration (the function is generalised from the root
  `WorkflowDefinition` to any graph that can answer `node` and `scope_chain`; the body graph
  implements it). Across the iterations of a collection there is no link; a release cap of
  `maxConcurrency` iterations with a compensation in flight bounds the fan-out, as data-model.md says
  ("may compensate concurrently within the concurrency limit"). Across the iterations of a loop, every
  entry of iteration `k` waits for the entries that run last in the nearest later iteration that has
  entries (so the order is last iteration first, and a link set linear in the entries). Completion
  sequence is evidence, never the order.
- **D384 — How it composes with each drain.** Nothing in the drains changes until their iterations
  are settled. *Collection fail-fast / storage-quota / cancel* (`collection.drain_started` with
  reason `Failed`, `StorageQuotaExceeded`, `Cancelled`): the iterations drain as today, an iteration
  whose member waits for a lookup or an operator is not drained (the existing `unresolved_derived`
  rule, extended to `RECONCILIATION` and `OPERATOR` waits), then the scheduler plans. *Loop failure
  drain* (`loop.drain_started` with `Failed`, `TimedOut`, `Cancelled`): the active iteration drains
  as today, then the scheduler plans. The scope owner stays `WAITING` on `COLLECTION` or `LOOP`
  (nothing around it can see it finished) and the run status is derived: `COMPENSATING` while a
  compensation can run, `MANUAL_REVIEW` when one waits for a person and nothing else can move.
  *Admission:* each iteration reserves `all_nodes - 1` instances (the return, the compensation and
  the lookup of every activity) when it is admitted, so exhausted forward budget or a tenant
  reduction cannot stop the compensation (the reservation is released at drain only for a scope that
  has no compensation to run). *Outer cancellation / deadline:* the run's first stop wins
  (`AlreadyStopping` for the second); a stop that lands while the scope compensates changes nothing:
  compensations are leased while the run is stopping (derived nodes already are) and are never
  cancelled by the stop.
- **D385 — The scope never ends in a simpler status than its compensation.** When every compensation
  is terminal, `scope.compensated` records the outcome from how they ended, as for a saga:
  `COMPENSATED`; `PARTIAL` (an irreversible effect completed, an unknown outcome had nothing to undo
  it, or an operator skipped a compensation); `COMPENSATION_FAILED` (a compensation failed or was
  cancelled for good). The scope then finishes and the run ends with the worse of the stop and the
  outcome (`RunState::final_status_for`): a failure whose completed iterations were all undone ends
  `COMPENSATED` (the root rule, `successors_after`); a cancellation ends `CANCELLED` only if
  everything was undone, otherwise `PARTIAL` or `COMPENSATION_FAILED`. A loop with `onError` takes the
  edge only after `COMPENSATED` (D66: a failure that was not undone is unhandled), otherwise the run
  ends with the outcome.
- **D386 — A compensation that keeps failing waits for a person, and a stop does not give up on that
  person.** A scoped compensation has the compensation retry budget (five attempts unless
  `compensationRetry`), the lease, fence, attempt timeout and breaker of every compensation. When the
  budget is spent the step waits on `OPERATOR` and the run is `MANUAL_REVIEW`; nothing is assumed
  done. A cancellation or a deadline that arrives while a scoped compensation waits for a person
  leaves it waiting: the status stays `MANUAL_REVIEW` and a person decides (retry, skip with a
  justification, resolve). *This is stricter than the root rule of D66,* which gives up on a
  compensation that waits for a person when the run is cancelled and ends the saga
  `COMPENSATION_FAILED`; the instruction for this increment is that a stop during the drain must not
  overwrite `MANUAL_REVIEW` or `PARTIAL`. A *forward* member that waits for a person when the run is
  cancelled is given up on and compensated defensively, as D66 says. *Open (O381):* whether the root
  rule should also change to match; recommended, but it is a change to released behavior and is not
  made here.
- **D387 — Unknown outcomes are looked up before anything is repeated or compensated.** An attempt of a
  scoped activity that ends `LEASE_EXPIRED`, `ATTEMPT_TIMEOUT` or `OUTCOME_UNKNOWN` follows
  `end_attempt` of the root: with `reconcileWith`, `reconciliation.requested` and the lookup
  `node~r` at the same path; without it, a `highRisk` step waits for a person and an ordinary one is
  retried. A step whose outcome stayed unknown when the scope compensates is compensated defensively
  (G53: a compensation tolerates an effect that never happened). Applied, not applied and unknown
  verdicts conclude the forward step exactly as at the root, and a not-applied verdict never retries a
  forward step in a scope that is stopping (`may_retry_forward`).
- **D388 — Public bodies.** The published collection and loop body widens from "one activity then
  return" to "a chain of one or more activities then return"; each activity may declare
  `compensateWith`, `reconcileWith`, `compensationRetry`, `compensationTimeoutMs`, `irreversible`
  and `highRisk`, and may not declare `onError`. Branching (`parallel`, `condition`, `switch`),
  `saga`, timers, child workflows and nested scopes inside a body stay closed: the scoped planner
  (`plan_addressed`) routes activities only, and the claim of such a node has no evidence. A chain is
  exactly what makes "reverse topological within an iteration" observable at runtime (the
  compensations of the later activities finish first); a diamond cannot be published, so the
  independence of parallel branches inside one iteration is proven in the domain only (the
  `reverse_dependencies` property tests) and is not claimed at runtime.
- **D389 — One reducer, addressed.** The scoped events reuse the events of the root saga where the
  meaning is the same (`reconciliation.requested`, `reconciliation.concluded`, `step.needs_review`,
  `step.retry_requested`, `step.skipped`, `step.resolved`), addressed at the instance's path; only
  the two events that name a scope (`scope.compensation_planned`, `scope.compensated`) are new. The
  six arms of the root reducer are extracted into one function that takes an `ExecutionInstanceId`,
  so a root and a scoped step cannot drift apart; root errors keep their variants. The scheduler does
  not decide compensation by SQL: a pure function of `(definition, state, stop)` returns the next
  addressed event, as `sagas::advance` does for roots, and the store only persists what the reducer
  accepted in the same transaction as the lease, the outbox hint and the audit.

Open decisions for the owner (options, recommendation, effect on what is built):

- **O380 — Scope-level policy fields.** *Options:* (a) defaults only, as built; (b) `onCompensationFailure`
  and `cancelPolicy` on the scope nodes, additive in IR v1; (c) require an enclosing `saga`.
  *Recommendation:* (a) now, (b) when a user needs `fail` or `abort_no_compensation`; (c) would close the
  public subsets. *Effect:* none on the code; the policy lookup is one function.
- **O381 — Give up on a person when a compensation waits and the run is cancelled.** *Options:* (a) hold
  (built for scopes); (b) give up and end `COMPENSATION_FAILED` (the root rule, D66). *Recommendation:*
  (a) for both; changing the root is a behavior change that needs its own ADR note.
- **O382 — Run deadline over an open collection.** `plan()` fails closed with
  `UnsupportedStructuredNode` when `TIMED_OUT` stops a run whose collection is still `OPEN`
  (existing, G343 for loops). Supporting it needs a `TimedOut` drain reason in the collection tables.
  *Recommendation:* add it with its own migration after this increment; not claimed here.
- **O383 — Branches and nested scopes in bodies.** Closed (D388). Opening them needs the scoped
  planner to route conditions, switches and parallel joins, and nested low-capacity admission tests
  (T014-014, T014-021). *Recommendation:* separate increment.

### Decisions made while building

- **D390 — A loop that stops at a limit is stopped at the result boundary of its last iteration.** The
  checkpoint that records `ITERATION_LIMIT` or `INSTANCE_LIMIT` finishes the loop at once, which would
  leave the completed iterations un-undone. For a compensating scope the scheduler does not commit that
  checkpoint: it starts the failure drain of the iteration that met the limit (the same drain an
  expression error at the result boundary uses), the loop compensates last iteration first, and then
  fails with the limit's code. Consequences: the journal shows `loop.drain_started` with the limit code
  where a non-compensating loop shows `loop.iteration_checkpointed` with continuation `iteration_limit`;
  the result and carry of that last iteration are not recorded (the loop failed, nothing reads them);
  the iteration's own effects are undone because they completed. The store re-proves the claim: the
  limit is accepted as a drain code only for a pending `return`, `iteration + 1 >= maxIterations` for
  `ITERATION_LIMIT`, and consumed instances plus the body charge above the run's budget for
  `INSTANCE_LIMIT`. A loop with no compensation keeps the old checkpoint and journal.
- **D391 — The next body is charged with its derived nodes.** The instance charge of a loop iteration is
  `all_nodes` (an activity with `compensateWith` and `reconcileWith` costs four), as admission already
  charged. The limit decision of the scheduler and the proof of the checkpoint used the declared nodes
  only, so a loop near its budget could be refused by the store after the scheduler had decided to
  continue and then be unable to stop. Both use `all_nodes` now; for a body with no derived node nothing
  changes.
- **D392 — Operator decisions address the iteration.** `POST /runs/{id}/operations` takes an optional
  `instancePath` (OpenAPI `OperationRequest.instancePath`, items `ApprovalScope`) for `RETRY_STEP`,
  `RETRY_COMPENSATION`, `SKIP_COMPENSATION`, `RESOLVE_STEP` and `MARK_MANUAL_REVIEW`; it is refused for
  the operations that apply to the whole run. Permissions, reason, idempotency key, audit and the 409
  rules are the root's. The audit resource names the instance (`{run}/lines[1]/reserve~c`).
- **D393 — A compensation answer is not retained, so a closed storage quota cannot stall it.** Tested:
  with the tenant content quota closed at its current use, every compensation of a failed collection
  completes and the run ends `COMPENSATED`. A forward step whose answer the quota refuses still fails
  with `STORAGE_QUOTA_EXCEEDED`, as at the root.

### As built

Domain (`crates/domain`): `scoped_sagas.rs` (`plan_scope_compensation`), `ScopeSagaState` and the two scope
events in `state.rs`/`event.rs`, the shared effect arms (`apply_effect_event`), `fail_instance` and the
instance forms of the operator commands in `command.rs`, `DependencyGraph` in `saga.rs`,
`WorkflowDefinition::scope_compensates`. Application: instance-addressed compensation and lookup keys
(`execution.rs`), the scoped lease/complete/fail paths (`worker.rs`), `RunService::review_instance`.
Storage (`crates/infra-postgres`): migration `20261112000000_scope_compensation` (`scope_sagas`,
`scope_saga_entries`, runtime-role grants, `storage-classification.json`), the journal codec, the
projection of the scope saga and of scoped steps, kept reservations in the collection and loop drains,
`scope_compensation.rs` (the finished saga and the stopped run's status). Scheduler: `passes.rs` plans
and advances the compensation in the draining branches of both scopes. Publication: a body is a chain of
one to eight activities (`MAX_BODY_CHAIN`, `infra-ir`). HTTP: `instancePath`.

Evidence, by clause (test names; the files are under `crates/domain/tests`, `tests/integration/workflow-primitives`,
`tests/fault/workflow-primitives` and `tests/contract`):

| Clause | Evidence |
| --- | --- |
| Reverse topological order inside an iteration | `scoped_compensation::compensation_in_one_iteration_runs_in_reverse_topological_order`, `scope_compensation_http::a_failed_collection_undoes_what_its_iterations_completed_in_reverse_order`, scenario `collection-compensation` (refund before release) |
| No order between collection iterations, bounded by `maxConcurrency` | `iterations_of_a_collection_are_independent_branches`, `compensation_fan_out_is_bounded_by_the_scope_concurrency`, `scope_compensation_http::a_collection_undoes_no_more_iterations_at_once_than_it_may_run` |
| Loop iterations in reverse order | `completed_iterations_of_a_loop_compensate_in_reverse_iteration_order`, `a_failed_loop_undoes_its_completed_iterations_last_first` (carries `[2,1,1,0,0]`), `a_loop_stopped_by_its_iteration_limit_undoes_its_iterations_last_first`, `a_loop_stopped_by_the_instance_budget_undoes_its_iterations_last_first`, scenario `loop-compensation` |
| Unknown effect reconciled before repeating or compensating | `an_attempt_with_an_unknown_outcome_is_looked_up_before_anything_is_repeated`, `a_lookup_that_found_the_effect_makes_the_step_compensable`, the three verdicts over HTTP (`an_effect_of_unknown_outcome_is_looked_up_and_found_applied_before_anything_repeats`, `an_effect_found_not_applied_is_tried_again_with_the_same_key`, `a_lookup_that_cannot_say_waits_for_a_person_who_declares_what_happened`), scenario `collection-compensation` (`reconciled`) |
| Irreversible effect ends PARTIAL | `an_irreversible_effect_ends_the_scope_partial_after_the_rest_is_undone`, `scope_compensation_http::an_irreversible_effect_ends_the_run_partial_and_is_never_called_compensated`, scenario `collection-compensation` (`partial`) |
| A compensation that keeps failing waits for an audited decision | `a_compensation_that_keeps_failing_waits_for_a_person_and_is_never_assumed_done`, `scope_compensation_http::a_compensation_that_keeps_failing_waits_for_a_person_who_can_retry_it`, `a_person_can_skip_a_compensation_and_the_run_ends_partial`, `a_person_can_declare_a_compensation_failed_and_the_run_ends_compensation_failed` (with the `saga.step_*` audit), scenario `collection-compensation` (`stuck`) |
| A failure or cancel during the drain never overwrites the outcome | `a_cancellation_that_lands_during_compensation_changes_nothing`, `a_stop_from_outside_never_gives_up_on_a_compensation_that_waits_for_a_person`, `a_cancellation_never_gives_up_on_a_compensation_that_waits_for_a_person` (HTTP) and `fault_workflow_primitives` `scope_compensation::a_cancellation_racing_the_scheduler_that_stops_the_scope_for_a_failure_ends_consistently` |
| Lease, fence, duplicates, restart, lost hint | `tests/fault/workflow-primitives/scope_compensation.rs` (duplicate and stale report, lost hint, connection killed in each transaction, scheduler cut at every await, outbox and audit failure, replica takeover) |
| Storage quota | `a_collection_stopped_by_the_storage_quota_still_undoes_its_completed_iterations` (domain), `a_closed_storage_quota_does_not_stop_the_compensation` (HTTP) |
| Five SDKs | scenarios `collection-compensation` and `loop-compensation`, goldens in `expected/` read by hand |

### Gaps recorded by this increment

- **G380 — The reservation integrity check of the loader was relaxed, not made definition-aware.** The
  loader accepted `remaining_reservations > 0` for an admitted iteration and `= 0` for a checkpointed or
  drained one; a scope with kept compensation reservations needs `>= 0`. The exact count is a function of
  the body (`all_nodes - 1` minus what ran) and is checked at write time, not on load. A definition-aware
  load check is open.
- **G381 — A run deadline over an open collection or loop is unsupported (O382).** Unchanged. The loop
  case is pinned by `the_run_deadline_over_an_active_loop_still_fails_closed`.
- **G382 — The SDKs cannot send `instancePath`.** `operate` in the five SDKs, the CLI and Studio do not
  carry it; a scoped decision is made over HTTP. Adding it is additive but touches all five clients and
  their goldens, so it was not done in this increment.
- **G383 — The chain bound of eight activities is a design value.** `MAX_BODY_CHAIN` has no load or
  latency evidence behind the number; it bounds the instance charge and the plan size. Branches, timers
  and nested scopes inside a body stay closed (O383).
- **G384 — Hold versus the root rule of D66 (O381).** A scoped compensation that waits for a person is
  kept through a cancellation; the root gives up on it. Recommended: change the root; not done.
- **G385 — The collection-level storage-quota drain with compensation has planner evidence only.** Making
  the scheduler's own quota stop (`CollectionDrainReason::StorageQuotaExceeded`) fire needs a checkpoint
  that retains new content, which the test documents do not produce; the planner treats the reason like
  the others and has a domain test, but no integration test reaches that exact reason.
- **G386 — Restart at every step is sampled, not exhaustive.** The cut sweep covers every await of the
  scheduler tick at a stride and the replica takeover covers a lapsed lease; a crash after every
  individual commit of a long compensation is not enumerated.
- **G387 — SDK `operate` and Studio show a scoped step by its id only.** The graph lists
  `lines[2]/reserve~c`; there is no screen that groups a scope's compensations.

## Decisions of the public loops increment (D400+)

Closes the public parts of T014-019..021 that D340-D345 left open (G340, G341, G343) and adds the
zero-worker delay evidence. Evidence is in `tests/integration/workflow-primitives/loops_http.rs`,
`tests/fault/workflow-primitives/loops.rs`, `tests/contract/workflow-primitives/loops.rs`,
`crates/domain/tests/primitive_loops.rs` and the scenarios `loop-cancel` and `loop-error-activity`.

- **D400 — A loop's `onError` edge may lead to a chain of root activities ending at an `end`
  (closes G340).** Publication (`published_loop_definition`) allows an `Activity` only when it is
  reached by a loop's `onError` edge (directly or through further activities) and the chain ends at
  an `end`. Such an activity declares no `onError`, `compensateWith`, `reconcileWith`, `irreversible`
  or `highRisk`: root compensation of a recovery activity is not part of the proven slice. An
  activity after a loop's success edge stays unsupported, and an activity nothing reaches is refused
  earlier by the structure check (`Unreachable`). The recovery activity is an ordinary root task, so
  the worker protocol is the same as at the root. D294 is unchanged: every loop failure drains the
  active iteration first and then follows the edge.
- **D401 — An expression error at loop entry follows `onError` (closes G341).** No new journal event
  and no owner decision was needed: `step.failed` already applies to a `PENDING` step. When the
  loop declares `onError`, the scheduler calls the new domain command `fail_unentered_loop`, which
  applies `step.failed` (`EXPRESSION_ERROR`, no fence) to the pending owner and settles; the planner
  then routes it like any failed step with an edge, in the same transaction. No scope, iteration,
  reservation or tenant usage exists to release. A loop without `onError` still stops the run
  `FAILED` with `EXPRESSION_ERROR`, so no existing journal changes. A stop already in flight is never
  overwritten (`AlreadyStopping`).
- **D402 — A delay that cannot end inside the loop budget is not scheduled; the loop times out at its
  deadline.** The scheduler asked the store for a timed-out drain as soon as `now + delay` passed
  the deadline, but the store (rightly) accepts `TimedOut` only at or after the deadline, so every
  tick counted a failed action (`InvalidTransition`) until the deadline arrived. The scheduler now
  leaves the returned iteration waiting and the existing deadline check ends the loop with
  `RUN_TIMED_OUT`. The outcome is the same as before, minus the failing ticks. See O400.
- **D403 — The public cancel and pause of an active loop are proven (closes G343).** G343 was stale:
  commit 76d13b7 had already made the planner leave a cancelled active loop to the scheduler's
  drain (`plan()` only fails closed for the run deadline, G381). What was missing was the proof
  through the public API and the real scheduler; it now exists for a leased body (the attempt is
  told to stop, `cancelRequested` on the heartbeat), a loop waiting on its delay (timer
  `CANCELLED`), and a loop not yet entered (owner cancelled, no scope). Each ends `CANCELLED` with
  `run_instance_usage.reserved_instances` and `tenant_iteration_usage.active_iterations` at zero,
  and `verify_run` rebuilds the projection. A cancel that commits before or after the timeout drain
  starts wins either way and the edge is not taken (D342, now also over HTTP).
- **D404 — The zero-worker delay is proven in both loop kinds over the public path.** `while` and
  `repeatUntil` with `delayMs` wait with no `READY`/`LEASED`/`RUNNING` step and only a `WAITING`
  `loop_timers` row, the scheduler fires it once, and a worker that registers only afterwards takes
  iteration 1. The five-SDK `loop-semantics` scenario already pins the same behaviour from outside.

### Gaps and owner questions of this increment

- **G400 — Delay and timeout tests use real clocks.** The public delay tests sleep between 1.5 and
  9.3 s (the margin between delay and budget is 4 s in the timeout test). They passed under a loaded
  shared machine, but a much slower host could make the margins too tight; the timeout test fails
  loudly (not silently) in that case. Nothing in the production code depends on these values.
- **G401 — A returned iteration whose delay overruns the budget is re-evaluated on every tick.**
  D402 returns `Waiting`, so the checkpoint expressions are computed again each scheduler tick until
  the deadline (nothing is written). It is bounded by the loop `timeoutMs` and has no measurement
  behind it; a cheaper wait (a due time for the scheduler to skip until) is not built.
- **G402 — `onError` to another loop is accepted by publication and not tested.** The public kind
  check allows `while`/`repeatUntil`/`end`/recovery activities at the root, so an edge from one
  loop to another loop publishes. Only `end` and activity targets are proven.
- **G403 — Updates cannot race the loop timer publicly.** A declared state document is not
  publishable with a loop, so a loop run has no Update to send; the Update-versus-timer race is
  proven only at store level (`timer_delivery_fails_closed_after_cancel_signal_or_update_commit`). A
  signal to a loop run is refused with `CONFLICT` and writes nothing (tested during the delay and
  racing the fire).
- **G404 — The entry-failure route has no cut-at-every-await sweep.** `fail_unentered_loop` runs in
  the generic engine transaction (journal, projection, outbox, audit in one commit); it is proven by
  the domain tests (replay included) and three HTTP tests, not by a crash at each await of that one
  transaction. The generic engine fault tests cover the transaction, not this command.
- **G405 — Pause is proven over the delay only.** Pausing a run while a body attempt is leased, and
  the completion of that attempt while paused, are not exercised for loops.
- **G406 — A recovery activity is not compensable.** An activity after a failed loop cannot declare
  compensation, so a failure after recovery has nothing to undo; the combination of loops, scoped
  compensation and root recovery effects is not designed (O401).
- **G407 — `scope_compensation::a_scheduler_replica_that_takes_over_mid_compensation_finishes_it` is
  flaky (2 of 6 runs alone, 2 of 4 inside the full fault target).** It is not touched by this
  increment. The likely cause is the one fixed in the new replica tests: a replica with another
  owner name makes the shard keeper rebalance, so the replica holds only part of the shards and the
  run's shard may stay with the other owner; the test never runs another keeper round. Not fixed
  here (another increment's file); a deterministic version keeps the owner name and bumps the fence
  generation, as `a_replica_that_lost_its_shards_cannot_fire_the_timer` does.
- **O400 — Should a delay that can never fit fail the loop at once?** Recommendation: no. Failing
  early would need a distinct code (not `RUN_TIMED_OUT`, which means the deadline was reached) and a
  new drain reason in the store; waiting for the deadline costs only idle time and keeps one meaning
  for the timeout code. Owner to confirm, or ask for `DELAY_EXCEEDS_TIMEOUT` checked at publication
  when `delayMs >= timeoutMs` (the cheapest honest fix: refuse the document).
- **O401 — Which root graph may follow a loop?** Today only `end`, and recovery activities after an
  error edge. Recommendation: allow activities after the success edge too, with the same plain
  recovery rules, once the owner decides how a root activity after a loop is compensated if a later
  step fails (a loop saga boundary).

- **G407 resolved (2026-10-17).** `a_scheduler_replica_that_takes_over_mid_compensation_finishes_it` failed
  about half of its runs because the dead scheduler's membership row was still unexpired: the new replica
  counted two live replicas, was entitled to about half of the shards, and the run's shard was in the
  other half. The test now lapses `scheduler_members` with the shard leases, which is what a gone
  process does. Eight repeated runs pass; before the change 3 of 6 failed.

## Decisions of the approval combinations increment

Implements D295 (an approval combines with declared state and with chains, not inside bodies). Evidence is
in `tests/integration/workflow-primitives/{approval_combinations,approval_telemetry,chains}.rs`,
`tests/fault/workflow-primitives/approval_combinations.rs`, `crates/infra-ir/tests/{codec,continuation}.rs`
and the shared scenarios `approval-state` and `approval-chain`. The combination was far smaller than the
task text suggested: the only gates were two refusals in the codec. The work was in what the combination
exposed once it ran (D443-D445, D447).

- **D440 — An approval publishes beside declared state, and its routes are the decision alone.** The codec
  refused `state` with an `approval` node; the gate is gone (`published_declared_state_definition` shares the
  root-graph builder with the plain path, so an approval still cannot meet `parallel`). While a request is
  open the run is `WAITING` and Update and Query work; the decision and the Update both take the run's row
  lock and write one gap-free journal in either order (`a_decision_and_a_declared_update_racing_...`, six
  rounds). No node expression reads the state: the design says only Updates change it and no node reads or
  waits on it (accepted decision 6), so the "state visible to the approval's route" of the task text is
  not a feature; see G442 for what a node expression naming `state` does today.
- **D441 — The published approval-and-chain shape.** An entry `activity`, then an `approval` (no `onError`)
  whose `onApprove` is the `continueAsNew` boundary (transferring the exact declared state when the document
  declares one) and whose `onReject` and `onExpire` are `end` nodes without output, nothing else in the
  graph (`continuation::parse_published`). Each segment is a new run with the same graph, so each opens a
  request of its own; a decision belongs to the segment that opened it. The entry stays an activity because
  successor creation assumes one (G441). Other approval-and-continuation graphs stay closed
  (`publication_keeps_other_approval_continuation_shapes_closed`).
- **D442 — An approval in a collection or loop body is refused with its dependency named.** It was refused
  as a `GRAPH` error or as "a body is one to eight activities" (the validation order decided). Publication
  now checks the bodies first and answers `VALIDATION_FAILED`/`UNSUPPORTED_NODE` with `details.node`,
  `details.kind: "approval"` and the additive `details.dependency: "SCOPED_COMPENSATION"`
  (`IrError::Pending`, `contracts/README.md` §5, OpenAPI `Error.details`). No new `details.reason`: the
  table stays closed (O440).
- **D443 — A stop the platform makes is audited as the engine's act.** A run with an open approval whose
  deadline passed could never end: cancelling the request needed the operator's run audit, the deadline
  pass has none, so the action failed on every tick and the run stayed `WAITING` with a `PENDING` request
  (`the_chains_original_deadline_settles_an_open_approval` failed with `approval cancellation requires its
  enclosing run audit`). It affected every approval under a run or chain deadline and every system cancel
  (a parent cancelling its child). The cancellation now writes `approval.cancelled` as the engine when no
  operator audit comes with the stop.
- **D444 — A state revision handed to a successor is readable.** `declared_state` accepted only a
  `state.updated` entry as the origin of a revision above zero, so the state an Update wrote in segment N
  could not be read or updated in segment N+1 (404). The successor's `execution_chain.successor_linked`
  entry, which carries the transferred digest, is accepted as the origin too. It never showed because the
  published shapes gave no window for an Update before the hand-off; an open approval is that window
  (`the_state_an_update_wrote_during_the_approval_is_what_the_next_segment_starts_with`).
- **D445 — A hand-off past the chain's original deadline is skipped, not failed.** It was refused as
  `RuntimeNotIntegrated`, which logged an error and counted a failed scheduler action until the deadline
  pass ended the run in the same tick. It is `ChainError::DeadlineElapsed` and the continuation pass counts a
  skip (`an_approved_boundary_past_the_chains_deadline_is_ended_by_the_deadline_not_failed`; found by the
  deadline race).
- **D446 — The approval routes keep no series of their own.** `workflow_approval_http_reads_total` and
  `workflow_approval_http_decisions_total` were outside the declared vocabulary (their `outcome` values
  `listed` and `replayed` are not in the closed set) and nothing in the repository reads them. They are
  removed. A refused decision counts once in `workflow_primitive_rejections_total{kind="approval",code}`
  with its closed code (`ApiError::primitive_rejection`, shared with the declared operations); committed
  outcomes are the journal-derived `workflow_primitive_transitions_total` and `workflow_approval_wait_seconds`.
- **D447 — A cancelled request records its wait.** `workflow_approval_wait_seconds{outcome="cancelled"}` was
  never recorded; it is now, when (and only when) the cancelling transaction commits.
- **D448 — Open work at a boundary is proven for approvals with a seeded request.** No published graph has
  an open approval at its boundary (the approval settles first, and `parallel` is closed with approvals), so
  `chains::a_boundary_with_an_open_approval_waits_until_the_decision_settles_the_request` and
  `the_store_proves_an_open_approval_again_under_the_locks` open a real request on a synthetic step and
  assert the `Approval` blocker in the scheduler and, again, in the store under its locks, as the existing
  tests do for the other blockers. They prove the rule, not a reachable run.

Task clauses of T014-024 and the tests that prove them (the storage is `approval_progress.rs`, D300):

| Clause | Evidence |
|---|---|
| Transactional request, expiry, decision, audit storage | `approval_progress::{settlement_outbox_failures_roll_back_decision_and_expiry, outbox_failure_rolls_back_request_timer_journal_projection_and_audit, a_decision_cut_off_at_any_await_commits_whole_or_not_at_all}`, fault `approvals::{an_audit_failure_rolls_back_a_decision_..., ..._an_expiry_..., ..._opening_...}`; with a stop: `approval_combinations::the_chains_original_deadline_settles_an_open_approval` |
| Authorized operations (initiator, eligibility, tenant) | `approvals_http::permission_eligibility_initiator_and_tenant_boundaries_are_opaque_and_audited`, `approval_combinations::a_viewer_reads_the_state_but_cannot_decide_a_stateful_approval`, scenario `approval-security` |
| SDK builders and client calls (five SDKs) | scenarios `approval-builders`, `approval-decisions`, `approval-settlement`, `approval-security`, `approval-state`, `approval-chain`; the per-SDK unit suites |
| Declared telemetry (spans, metrics, trace id) | `approval_telemetry::*` (span under the run's trace, closed attributes and outcomes, no person or key; refusal series; cancelled wait), `approvals_http` (wait histogram), `projection::approval_routes_and_the_recorded_decision_are_projected_over_http` |
| Typed errors | `approvals_http::out_of_bound_decision_keys_and_reason_codes_...`, the reason table test (`contract_workflow_primitives::errors`), `approval_combinations` (`UNSUPPORTED_NODE`+`dependency`, `REVISION_CONFLICT`, `RUN_NOT_MUTABLE`, `RUN_CONTINUED`, `ALREADY_DECIDED`) |

### Gaps and questions recorded by this increment

- **G440 — An approval inside a collection or loop body stays unsupported.** It waits for scoped
  compensation (D295); the refusal now names that (D442). Nothing else was built.
- **G441 — An approval cannot be a chain's entrypoint, and only one approval per segment is published.**
  Successor creation assumes an activity entrypoint (the same limit as `chain-root-wait.json.pending`), and
  the published shape has one approval, no condition after it and no way for an approval to end the chain
  except rejection or expiry (a final approval round ends only by `maxSegments` or by a rejection).
- **G442 — A node expression that names `state` compiles and evaluates to null.** The codec only checks the
  syntax of node expressions, so `end.outputExpr: "state.balance"` publishes and the run ends `SUCCEEDED`
  with a null output (observed; not pinned by a test because it is undesirable). Any root the engine does
  not provide behaves the same way, so this is general, not approval-specific (O442).
- **G443 — A cancelled or stopped request has no span.** The telemetry spans are derived from journal
  entries and a cancellation writes no approval entry, so only the metric (D447) shows it; the step shows
  `CANCELLED` in the graph.
- **G444 — The race tests assert invariants, not the commit order.** Contenders are released together by a
  barrier and each of the six rounds checks one outcome, one gap-free journal and no second successor; which
  contender wins is the database's. The deadline race moves the decision around the real deadline
  (-400..+200 ms) and the expiry race around a 1.2 s timeout, so both depend on the machine's clock
  resolution, not on a fake clock. They passed three consecutive runs here; a loaded CI may need wider offsets.
- **G445 — The OTLP export of approval spans is not exercised end to end here.** The tests stop at the
  projector's `derive`, which is where the span and its trace id are decided; the exporter has its own suites.
- **G446 — Approvals are listed per run.** A chain's open request is found through the chain head
  (`GET /chains/{id}` then `GET /runs/{head}/approvals`); there is no chain-level listing.
- **G447 — The `continue` command path keeps `RuntimeNotIntegrated` for a hand-off past the deadline.**
  Only the evaluated path the scheduler uses was changed (D445).
- **O440 — Should `details.dependency` be the contract's way to say "not built yet", or should each
  dependency get its own `details.reason`?** The first keeps the reason table closed and adds one optional
  field (the choice made); the second adds a row and a remediation per dependency.
- **O441 — Should an approval be allowed as the first node of a chain?** It needs successor creation for a
  non-activity entrypoint, the work `chain-root-wait.json.pending` also waits for.
- **O442 — Should publication refuse a node expression that names a root the engine never provides
  (`state`)?** It would turn G442 into a typed error at publication.
- **O443 — Is anything outside this repository reading `workflow_approval_http_*`?** Nothing in the repository
  does (D446); restore them under a declared name if something does.

## Decisions of the collection sources increment (blob item sources, ceilings, audit)

Taken while closing the rest of T014-016 (captured arrays and blob references), T014-004 (ceilings and
the 10,000-item reference test) and the audit of T014-014/015/017. Numbering: D420-D429, G420-G429,
O420-O421 (D410+ and G410+ are free for the peer increments).

- **D420 — The item source of a collection may be a blob; the membership is read once at entry.** When
  the `items` expression yields `{"$blob": {id,size,sha256,contentType}}` (the run's bound input, e.g.
  `items: "input"` after `inputBlobRef`, or a committed step output) the scheduler asks a
  `CollectionItemSource` (`BlobService` in production, wired in `apps/execution-engine` only when a
  blob store is configured) for the array, once, in the transaction-less step before the entry
  transaction. Elements are stored as canonical compact JSON under their own digests exactly as for an
  inline array; nothing reads the blob again for that scope (tested: the bytes may leave the store
  after entry and the collection finishes on the same items).
- **D421 — A descriptor in a value is not a capability.** The reader accepts a blob only if the catalog
  says it is `COMMITTED`, in the run's own tenant **and** namespace, the run's bound input
  (`RunData.input_blob`, which rerun and child inheritance also set) or the committed output of a step of
  this run, and the descriptor's id, size and digest equal the catalog's. Anything else (another tenant,
  another namespace, another run's blob, an unbound upload, an unknown id, a wrong size or digest) is the
  same `NotFound` and fails the run `BLOB_UNAVAILABLE`; it cannot tell a client whether the blob exists
  elsewhere.
- **D422 — Byte ceilings of a blob source.** Blob at most 8 MiB (`MAX_SOURCE_BLOB_BYTES`), each element
  at most 256 KiB (a payload), canonical elements together at most 4 MiB (the retained-run-data
  ceiling), count at most `maxItems`. Reasons: an element is a payload; the whole membership is retained
  data of the run; 8 MiB leaves room for pretty-printing. Not derived from a benchmark (G422).
- **D423 — The snapshot payload of a blob-backed scope is a manifest.** `{"$collectionSource":
  {blob, count, itemsSha256}}`: the descriptor read, the number of elements and the digest of the
  canonical array. An array of 10,000 references (1.8 MiB) does not fit the 256 KiB payload that the
  inline snapshot is. `collection_progress` verifies count and digest of the stored items against it on
  every full load. `iteration_scopes.source_blob_id` (migration `20261121000000`) is a restricted
  foreign key to `blobs` in the same tenant and namespace: a live membership keeps its blob, whatever
  retention or sweep is added later. Today nothing deletes a committed blob (D298, ADR-0023).
- **D424 — Typed failures, and what only waits.** Run failure codes: `BLOB_UNAVAILABLE` (not visible to
  the run, or the store lost the bytes), `BLOB_CORRUPT` (size/digest mismatch while reading),
  `ITERATION_LIMIT` (more elements than `maxItems`), `RUN_DATA_LIMIT` (a byte ceiling),
  `EXPRESSION_ERROR` (not a JSON array). They are run failure codes (an open set), not `Error.code`
  values. A store or catalog outage, or a replica without a source, only leaves the run waiting
  (`CollectionAdvanceError::SourceUnavailable`, retried every pass), never fails it.
- **D425 — A non-array inline value now fails the run.** An `items` expression that did not yield an
  array used to be retried by the scheduler for ever (counted as a failed action each pass); it now
  stops the run with `EXPRESSION_ERROR` at entry, like the blob cases. Behavior change.
- **D426 — The engine's read is audited in the entry transaction.** `blob.read` on the blob (actor: the
  engine) commits with the scope, so a rolled-back entry leaves no audit and a replayed entry adds none
  (tested). `scheduler_collection_source_failures_total{reason}` counts the refusals (reasons:
  `expression`, `unavailable`, `corrupt`, `not_an_array`, `too_many_items`, `too_large`).
- **D427 — A transition reads one item, not the membership.** The scheduler read and digested every item
  and result of the scope on every transition and scanned all instances once per admitted iteration. It
  now reads the scope identity, one item to admit it, the full progress only for the last result and the
  finish, and indexes failed steps and pending returns in one pass (`collection_scope`,
  `collection_item` on `CollectionStore`). See G421 for what remains linear.
- **D428 — The SDK runners can upload exact bytes.** `inputBlobText` on `startWithBlob` (all five
  runners) and an opt-in `sourceEvidence` in the harness (run status, stop code, whether the collection
  came from a blob, number of audited reads). Scenarios `collection-blob-source` and `collection-limits`.
- **D429 — The 10,000-iteration end-to-end run is `#[ignore]`.** It takes tens of minutes (G421), so it
  runs on request; the entry of 10,000 references, the count and content of the membership, `maxItems`
  + 1 and the byte ceilings run with the suite.

### Gaps recorded by this increment

- **G420 — A step output cannot yet be a published source.** The public path starts at the collection
  owner (D300 note in the spec), so no published workflow puts an activity before a collection. The
  reader's rule for a step output (committed, same run) is tested against a real committed output
  (`a_step_output_blob_is_a_source_of_its_own_run_only`), but no end-to-end run uses it. A collection after
  a collection reads `steps.<c>.output`, which is an inline array of at most 256 KiB.
- **G421 — A transition is still linear in the size of the run.** Each transition restores the run's
  whole `RunState` (two instances per admitted iteration) and re-reads its data, and one transition
  admits or checkpoints one iteration. After D427 the 10,000-iteration run still slows as iterations
  accumulate; measurements are in "Evidence" below. Batching admissions and checkpoints per transaction,
  or restoring only the live instances, are the fixes; neither is attempted here.
- **G422 — The source ceilings (8 MiB, 4 MiB) have no benchmark behind them,** and the read and JSON parse
  of up to 8 MiB run on the scheduler's async task without `spawn_blocking`. Entry of the 1.8 MiB array of
  10,000 references took about 4 s in a debug build including the 10,000-row insert.
- **G423 — Unused reservations of a drained iteration are returned.** `data-model.md` says reservations
  are "not reused even if the derived work is never needed"; the store hands back the reservations the
  drain does not keep for compensation (`reserved_instances - (remaining - kept)`, D380). The test
  (`collection_budgets`) pins "never more than accepted", not equality. Either the sentence or the code is
  to be aligned (O421).
- **G424 — No retention for committed blobs.** The foreign key protects a live membership; a retention
  policy for finished runs (D298) must still decide when `iteration_scopes` rows, and with them the
  reference, go.
- **G425 — The SDK builders cannot set the top-level `limits`.** A scenario's `limits` key is ignored by
  all five runners, so WP-12's instance budget, depth and node ceilings are not exercised through the
  SDKs (`loop-runtime.json` carries a `limits` that has no effect). Rust covers them (`limits.rs`,
  `ceilings.rs`).
- **G426 — WP-02 server-side rejection and WP-13 object-literal rejection are not SDK-visible.** Publishing a
  document is not an SDK operation (the harness publishes), so rejection of invalid, cyclic or escaping bodies
  and of unsupported expressions is proven by the Rust contract suites, not by the five SDKs.
- **G427 — Nested descendants at low capacity are not exercised** because nested scopes are closed (O383); the
  low-quota case covers the iteration's own instances (a capacity of one runs four iterations to the end).

### Open questions for the owner

- **O420 — Raise or lower the blob-source ceilings?** *Options:* (a) keep 8 MiB / 256 KiB / 4 MiB; (b) allow
  a larger membership by retaining it outside `payloads`. *Recommendation:* (a) until G421 is fixed: a bigger
  membership only makes each transition slower.
- **O421 — Reservations returned at drain: align the data model or the code?** *Recommendation:* the code
  (it returns budget the run can no longer use); amend the sentence in `data-model.md`.

### Evidence by clause: T014-014, T014-015, T014-017 (audit of 2026-10-17)

Paths are under `crates/domain/tests` (domain), `crates/application/src` (unit), `tests/integration/workflow-primitives`
(int), `tests/fault/workflow-primitives` (fault), `tests/contract/workflow-primitives` (contract) and
`tests/contract/sdk-conformance/scenarios` (scenario). "new" marks a test written by this increment; "no red"
means the behavior already existed and the test only proves it (stated, not hidden). D300: the tasks name
`primitive_collections.rs`, `collections.rs` and `fault/.../collections.rs`; the evidence lives in the files below.

**T014-014 (failing cases, limits, contention)**

| Clause | Tests |
| --- | --- |
| `maxItems` and `maxIterations` are integers in 1..10000 | domain `primitive_collections::{max_items_is_an_integer_from_one_to_ten_thousand, max_iterations_is_an_integer_from_one_to_ten_thousand}` (new, no red); `structured_workflow::collection_limits_and_effective_expression_bytes_are_validated`; contract `ceilings::item_and_iteration_limits_are_one_to_ten_thousand_and_concurrency_one_to_256` (new, no red) and `revision::publication_shape_enforces_hard_limits_and_rejects_unknown_fields`; runtime int `limits::more_items_than_the_collection_limit_fail_the_run_with_iteration_limit`, `limits::a_loop_that_needs_more_than_its_iteration_limit_fails_with_iteration_limit`, `collection_10k::entry_of_ten_thousand_items_is_one_exact_membership` (10,000 accepted, 10,001 refused) |
| `maxConcurrency` is 1..256 | domain `primitive_collections::max_concurrency_is_an_integer_from_one_to_256` (new, no red); contract `ceilings::item_and_iteration_limits_…` (same test); never exceeded at run time: domain `collection_reducer::concurrency_one_requires_a_durable_checkpoint_before_next_admission`, int `collection_budgets::a_tenant_capacity_of_one_runs_every_iteration_to_the_end` (new), `scope_compensation_http::a_collection_undoes_no_more_iterations_at_once_than_it_may_run`, ignored `collection_10k::ten_thousand_items_are_admitted_concurrently_aggregated_in_order_and_bounded` (peak active at 256) |
| Snapshot immutability | int `primitive_transactions::a_collection_entered_once_keeps_its_membership_against_any_later_entry` (new, no red: exact replay is a no-op, another array or scope is `DuplicateMismatch`, no journal event), `primitive_transactions::collection_payloads_require_same_tenant_and_matching_content_digests`, `collection_sources::the_membership_is_fixed_at_entry_and_the_blob_behind_it_is_protected` (new, red first), fault `collection_sources::two_attempts_to_enter_the_same_collection_leave_one_membership` (new), `collections::bounded_public_collection_finishes_in_input_order_after_restart_and_lost_hints`; domain `collection_restore_state::*` |
| Shared run and tenant budgets | int `limits::a_collection_that_cannot_be_admitted_within_the_instance_limit_fails_before_it_starts`, `limits::a_tenant_ceiling_below_the_authors_limit_is_never_raised`, `primitive_transactions::persisted_tenant_capacity_serializes_admission_across_runs`, `security::tenant_collection_capacity_serializes_admission_across_runs`, `scoped_worker::automatic_collection_progress_rechecks_the_tenant_capacity`, `collections_http::iteration_capacity_is_tenant_scoped_authorized_audited_and_reducible` |
| Descendant admission with low quotas | int `collection_budgets::a_tenant_capacity_of_one_runs_every_iteration_to_the_end` (new, no red: four iterations of two activities run one at a time, no slot cycle), `security::collection_admission_derives_its_reservation_from_the_pinned_definition`. Nested scopes are closed (O383), so descendants are the iteration's own instances (G427) |
| Derived-instance reservations survive policy reduction | int `collection_budgets::a_budget_reduced_to_what_is_accepted_still_lets_the_accepted_failure_be_undone` (new, no red), `limits::a_policy_reduction_midway_pauses_admission_without_failing_or_killing_accepted_work`, `scoped_worker::failed_derived_obligations_keep_member_capacity_reserved` |

**T014-015 (implementation)**

| Clause | Tests |
| --- | --- |
| Typed entry | domain `collection_reducer::starting_a_collection_parks_the_owner_for_its_transactional_scheduler`; int `primitive_transactions::collection_admission_writes_expected_journal_projection_and_usage_atomically`, `collection_sources::a_run_input_blob_array_is_the_membership_and_each_item_reaches_its_iteration` (new, red first) |
| Admission | domain `collection_reducer::{replay_uses_only_admitted_instances_and_recorded_checkpoints, duplicate_admission_is_rejected_without_changing_state}`; int `primitive_transactions::admission_owner_must_match_the_stored_scope_owner` |
| Ordered aggregation | domain `collection_reducer::checkpoint_results_remain_in_input_order_after_reverse_completion`; fault `collections::bounded_public_collection_finishes_in_input_order_after_restart_and_lost_hints`; scenario `collection-ordering`; ignored int `collection_10k::ten_thousand_items_…` (10,000 results in input order, completed in reverse poll order) |
| Fail-fast drain | domain `collection_reducer::permanent_failure_stops_admission_and_finishes_only_after_drain`; int `scoped_worker::exhausted_member_failure_starts_fail_fast_drain_and_survives_restart` |
| Each admitted logical instance is charged once | int `collection_budgets::a_retried_iteration_is_charged_once_and_a_derived_activation_uses_its_reservation` (new, no red), `primitive_transactions::failed_outbox_insert_rolls_back_admission_and_every_counter` |
| Slots released only after terminal or reconciled drain | domain `collection_reducer::{drain_refuses_to_hide_an_attempt_that_is_still_in_flight, drain_refuses_to_hide_failed_derived_obligations}`; int `scoped_worker::{failed_derived_obligations_keep_member_capacity_reserved, cancellation_drains_members_atomically_and_finishes_after_restart}` |

**T014-017 (faults and compensation)**

| Clause | Tests |
| --- | --- |
| Crash and restart recovery | fault `collections::bounded_public_collection_finishes_in_input_order_after_restart_and_lost_hints`, `scope_compensation::{a_connection_that_dies_inside_each_transaction_of_the_compensation_loses_nothing, the_scheduler_cut_at_every_await_point_of_the_compensation_resumes_to_the_same_end, a_scheduler_replica_that_takes_over_mid_compensation_finishes_it}`, `collection_sources::a_failure_after_the_read_and_before_the_commit_leaves_nothing_and_the_retry_enters_once` (new); int `scoped_worker::{a_scoped_lost_lease_rolls_back_then_recovers_after_restart, a_late_journal_failure_rolls_back_the_whole_scoped_report}` |
| Duplicate reports and deliveries | fault `scope_compensation::a_compensation_reported_twice_is_applied_once_and_a_stale_report_is_refused`, `collection_sources::two_attempts_to_enter_the_same_collection_leave_one_membership` (new); domain `collection_reducer::duplicate_admission_is_rejected_without_changing_state` |
| Stale fence | int `primitive_transactions::stale_scheduler_fence_rejects_entry_and_admission`, `scoped_worker::a_scoped_failure_retries_after_restart_and_rejects_the_old_fence`; fault `collection_sources::{a_scheduler_that_loses_its_shard_after_reading_commits_nothing, a_replica_without_a_current_lease_does_not_even_read_the_blob}` (new) |
| Lost broker reconciliation | int `scoped_worker::stale_hints_are_renewed_for_each_scoped_instance`; fault `collections::bounded_public_…_lost_hints`, `scope_compensation::a_compensation_whose_hint_never_arrived_is_leased_from_the_database` |
| Cancellation | int `scoped_worker::{cancellation_drains_members_atomically_and_finishes_after_restart, a_scoped_lease_observes_cancellation_and_acknowledges_it_before_restart}`, `scope_compensation_http::a_cancelled_collection_undoes_what_completed_and_ends_cancelled`; domain `collection_reducer::cancellation_drains_only_admitted_work_before_finishing`; fault `scope_compensation::a_cancellation_racing_the_scheduler_that_stops_the_scope_for_a_failure_ends_consistently`, `collection_sources::a_run_cancelled_before_entry_never_reads_the_blob` (new) |
| Unknown and failed compensation | int `scope_compensation_http::{an_effect_of_unknown_outcome_is_looked_up_and_found_applied_before_anything_repeats, an_effect_found_not_applied_is_tried_again_with_the_same_key, a_lookup_that_cannot_say_waits_for_a_person_who_declares_what_happened, a_compensation_that_keeps_failing_waits_for_a_person_who_can_retry_it, a_person_can_skip_a_compensation_and_the_run_ends_partial, a_person_can_declare_a_compensation_failed_and_the_run_ends_compensation_failed}`; scenario `collection-compensation` |
| Compensation order (FR-003-002) | domain `scoped_compensation::{compensation_in_one_iteration_runs_in_reverse_topological_order, iterations_of_a_collection_are_independent_branches, compensation_fan_out_is_bounded_by_the_scope_concurrency}`; int `scope_compensation_http::a_failed_collection_undoes_what_its_iterations_completed_in_reverse_order` |
| All-five WP-03/04 | scenarios `collection-ordering`, `collection-compensation`, plus `collection-runtime`, `scoped-identity`, `collection-blob-source` and `collection-limits`, all matching reviewed goldens in the five SDKs |

**T014-004 remainder.** Ceilings: `ceilings.rs` (6 tests) at the schema and at the publication codec; the
4096-element constructed list stays in `identity_limits::effective_expression_limits_remain_bounded_for_additive_v1`.
Shared cases: WP-02 `scoped-identity` (+ `collection-builders`); WP-03 `collection-ordering`, `collection-runtime`,
`collection-blob-source`; WP-04 `collection-compensation`; WP-05 `loop-semantics`, `loop-compensation`, `loop-runtime`;
WP-06 `approval-decisions`, `approval-settlement`; WP-07 `switch-builders`; WP-08/09 `state-operations`;
WP-10 `chain-three-segments`, `chain-final-success`, `chain-state-final-success`; WP-11 `chain-commands`,
`chain-inspect-cancel`, `chain-child`, `chain-query-handoff`; WP-12 `collection-limits` (item limit only, G425);
WP-13 `protocol-negotiation`; WP-14 `approval-security` and the opaque-descriptor case of `collection-blob-source`.
Added here: `collection-blob-source`, `collection-limits`. Not coverable through the SDKs: WP-02 server rejection of
invalid bodies and WP-13 unsupported object literals (publishing is not an SDK operation, G426), WP-12 instance
budget, depth, node, case and operation ceilings (no builder for `limits`, G425; Rust covers them).

### Evidence: the 10,000-item collection (T014-004, D429)

`collection_10k::ten_thousand_items_are_admitted_concurrently_aggregated_in_order_and_bounded` (ignored by default),
2026-10-17, macOS arm64, **debug build**, one PostgreSQL 16 container, the machine shared with the other agents' builds
and with the SDK and gate suites for most of the run (so times are an upper bound, not a benchmark). The source is
a run-input blob of 10,000 blob references (1,834 KiB), `maxItems` 10000, `maxConcurrency` 256, tenant iteration
capacity 256; the worker is in process (`WorkerService` over the platform's store, protocol 1.5) and completes each
poll of up to 256 tasks in reverse order.

- Entry (read of the blob, parse, 10,000 payload rows, scope, audit): about 4 s (`entry_of_ten_thousand_items_is_one_exact_membership`,
  which runs with the suite and also checks the content of every index and the refusal of 10,001).
- Whole run: **7,926 s (2 h 12 min)**, 10,000 completions, 20,002 collection transitions (10,000 admissions, 10,000
  checkpoints, entry and finish): 1.3 iterations/s on average. Peak active iterations 256, never more.
  Aggregate of 10,000 results in input order (position *i* holds *i*) and `verify_run` agree.
- Peak resident set of the test process (it hosts the API, the scheduler and the worker side): 117 MiB.
- The cost of one transition is not flat: 0.07 s for the first 250 transitions, 0.4 s around 8,000,
  0.7 s near 18,000 (rows of the table in the log: 250 -> 18 s, 2,250 -> 203 s, 8,250 -> 1,665 s,
  14,250 -> 4,231 s, 18,250 -> 6,645 s). Before D427 the same run took 275 s for its first 1,500 transitions
  (0.18 s each, growing 0.0001 s per transition); after it 119 s. What is left is the restore of the run's
  state (G421). **A collection of 10,000 items therefore completes, correct and in order, but not quickly,
  and no throughput claim follows from it.**
- Ceilings that ran for real in that run: the 256 KiB result and the 4 MiB retained-output checks of the last
  checkpoint (`aggregate_refusal`, 10,000 results of up to 5 digits), the 256 concurrent iterations, and the
  tenant capacity; the negative cases (element over 256 KiB, elements over 4 MiB, blob over 8 MiB, aggregate over
  256 KiB) are in `collection_sources` and `byte_ceilings` and do not need 10,000 iterations.

### Verification of this increment (2026-10-17)

`cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `scripts/check-architecture.py`
pass. On the final tree: contract 44, integration 263 (+1 ignored: the 10,000-iteration run), platform_foundation 89, durable_execution 178, sagas_compensations 74 pass, and fault 69 of 70 (the failure is G428, pre-existing). `operations_http::outbox_failure_rolls_back_the_http_update_and_the_same_key_can_retry` failed once under load and passed alone and in the final full run.

- **G428 — `scope_compensation::a_scheduler_replica_that_takes_over_mid_compensation_finishes_it` is flaky when its
  module runs in parallel.** It failed in 3 of 4 runs of `scope_compensation` (7 of 8 tests passing, the run still
  `COMPENSATING` after the test's 40 ticks) and also on the base commit `a49b228` (1 of 2 runs, before any change of
  this increment); alone it passes. It is not caused by the blob source or by D427. Probable cause: the loop ticks
  without waiting for the 100 ms compensation back-off. Not fixed here.

## Decisions of the compatibility and performance evidence increment

Written 2026-10-17 for T014-041 and T014-042. The coordinator ticks the tasks; nothing here marks one.

### Decisions

- **D460 — Compatibility is proven at the PostgreSQL adapter, with the older reader injected where
  production injects its codec.** `compatibility.rs` runs the real migrations, the real worker service and
  the real store. The reader of an earlier release is a codec handed to `DefinitionCache`; it returns the
  `Schema` error a pre-014 schema gives for the new top-level fields (`additionalProperties: false`). It
  is a simulation of a build, not a second build. The facts that needed a real second build were taken
  from one: the canonical digests of eight pre-014 documents were computed by the build at commit
  `07d673f` (it changes no file under `crates/`) and are pinned in
  `pre_primitive_documents_keep_the_digest_the_pre_014_build_computed`; the same build was run on the nine
  documents of the SDK builder goldens and refused every one with `IrError::Schema`, which is why the
  injected reader returns that error. The probe is manual (`docs/sdk-compatibility.md`, matrix 2); the
  old build cannot be compiled from this tree.
- **D461 — Old-worker evidence runs concurrently with scoped-ready work.** The `1`, `1.0`–`1.4` offers
  are registered on the scoped-capable adapter against a run whose scoped iterations are ready and whose
  activity names the old worker advertises, so a flattened scoped task would match it. A `1.5` worker then
  leases the same two tasks, which proves the test would have seen them.
- **D462 — Per-transition cost is measured in rows and statements, not in time.** The host was at load
  10–47 during this work, so wall time is recorded but never used as evidence of an algorithm. Each
  sampled collection or loop transition is bracketed by `pg_stat_statements` (rows returned or affected,
  statement count), which does not depend on load. The benchmark therefore runs against a dedicated
  PostgreSQL 16 container started with `shared_preload_libraries=pg_stat_statements`
  (`SAGAT_TEST_DATABASE_URL`), with `fsync` on. Planner statistics are refreshed with `ANALYZE` every 50
  transitions (what autovacuum does); without it the planner chose plans for empty tables and buffer
  counts were meaningless.
- **D463 — Collections and loops are driven through the production transition, chains and tenants
  through the production tick.** `advance_collection` / `advance_loop` isolate one durable transition;
  `tick()` is used where fairness or the continuation pass is the subject. A production tick adds the
  cost of the other passes (about 25–50 ms idle in these runs). `LoopAdvance` and `LoopAdvanceError` are
  now exported by `sagat-scheduler` because a public method returned types a caller could not name.
- **D464 — The tenant bound is asserted against two figures.** After every scheduler round for each
  tenant the usage counter and the count of iterations with `slot_held` must both be at most the policy
  limit and equal each other. The starvation bound (the quiet tenant needs at most 4x its solo rounds) is
  the benchmark's own threshold, not a product promise.
- **D465 — Zero dedicated resources per wait is asserted, not only reported.** With 10,000 approvals and
  1,000 timers pending the process may not gain more than four threads, PostgreSQL connections may not
  exceed the pool of eight plus the probe, no connection may be idle in a transaction, and no worker
  session may exist.
- **D466 — The two quadratic findings are pinned by `#[ignore]`d red tests.** `red_*` in
  `tests/performance/workflow-primitives/` fail today and must be un-ignored when G461 is fixed. They sit in
  the performance target so that the suites the coordinator gates stay green.

### Gaps

- **G460 — A reader that cannot parse a pinned definition fails the whole poll with `Internal`.** Observed
  in `a_reader_without_the_capability_leaves_an_active_structured_run_untouched_until_it_returns`. State
  is preserved (the test proves it), but the answer is an internal error, not a typed refusal, and
  `claim` allows three failed runs per poll before it gives up, so three unreadable runs ahead of
  readable work could starve it. The second half is inferred from `MAX_CLAIM_FAILURES` and not tested.
  A real rolled-back build negotiates at most `1.4` and would not see scoped steps, but chain successors
  and approval runs have ready root steps whose definitions it cannot read.
- **G461 — One collection or loop transition reads work proportional to the progress of the scope, so the
  scope costs quadratic database work.** Evidence: statement count is constant (60–65 per collection
  transition, 83 per loop transition) while rows returned grow linearly: about 3 rows per transition index
  in a collection (1,536 at transition 166, 7,014 at 1,992 for 1,000 items) and 5 per iteration in a loop
  (561 at iteration 100, 5,067 at 1,000). The statements responsible are the restore of every step of
  the run, the load of every item payload of the collection, every iteration row, every step output and
  every checkpoint. `advance_collection_in_view` also walks `0..next_admission` per call. Two red tests
  fail: `red_collection_transition_rows_do_not_grow_with_progress` (218 -> 1,146 rows over 160 items) and
  `red_loop_transition_rows_do_not_grow_with_progress` (91 -> 841 over 160 iterations). Consequence in
  the 10,000-item workload: at maxConcurrency 1, 16 and 256 it did not complete in 600 s, and a full-length
  run at 256 completed in 6,815 s (1 h 54 min, 47 times the 1,000-item run), the p50 of a transition
  rising from 44 ms to 678 ms and the rows read per transition from 15,000 to 70,000. Not fixed here.
- **G462 — A conditional chain cannot have more than about 127 segments.** The accepted expression subset
  cannot construct an object, so the first input carries the rest of the chain as a nested `next`; the
  run input must parse as a JSON object, and the JSON parser's recursion limit of 128 stops it. 120
  segments ran; the start of 300 was refused with `the run input must be a JSON object`. `maxSegments`
  allows 10,000. An unconditional `continueAsNew` with `"input": "input"` has no such limit and ends
  `FAILED` when the limit is reached (1,000 segments measured).
- **G463 — A collection or loop advances one durable transition per scheduler tick per run.** Per run
  the throughput ceiling is the tick rate times one transition, plus two worker round trips per item.
  Measured: 5–8 items/s at 1,000 items. Across runs the scheduler works in parallel (`concurrency` 8).
- **G464 — `enter` of a 10,000-item collection is one 3.6 s transaction** in the 10,000-item run (206–240
  ms at 1,000). It is the only transition that is proportional to the whole input by design.
- **G465 — Not measured:** loops parked in a durable delay as a population (timers were measured, a loop
  delay uses the same timer wait but its own path); nested loops reaching the 100,000-instance ceiling
  (nesting is closed); approval decision throughput; more than one scheduler replica; target hardware;
  production-sized PostgreSQL; client network latency (everything is in-process).
- **G466 — Matrix cells without a test.** `docs/sdk-compatibility.md` marks them "not tested": loops,
  approvals, `switch` at `1.5`, declared Query/Update and chains with an old or `1.5` worker, the root-only
  adapter at `1.0`–`1.3`, task streaming at `1.1`–`1.3` and `1.5`.
- **G468 — An idle scheduler tick costs work proportional to the pending waits, and it grew with
  feature 014.** 26 ms with no waits, 42 / 75 / 142 / 186 ms with 500 / 2,000 / 5,000 / 10,000 pending
  approvals (about 16 to 30 microseconds per pending approval per tick). Built from the harness commit
  `42375f3` and from this branch and run alternately on the same container, the idle tick over 10,000
  approvals and 1,000 timers took 118.7 and 118.1 ms before and 198.0 and 197.6 ms now (+67 percent) while
  run creation did not change (194 and 203 against 191 and 191 runs/s). The new collection, loop and
  continuation passes each add a discovery read. `pg_stat_statements` over the idle ticks of a 2,000-approval
  run (80 calls each, 0 rows returned) puts the collection discovery query at 14.8 ms per call and the loop
  discovery query at 14.7 ms per call, against 7.3 ms for the approval-opening query and about 3 ms for the
  next one: the two new passes are about half of a 59 to 75 ms idle tick. Both scan the pending root steps of
  every run and read each run's stored document as JSON to find a `forEach` node; the 10,000-approval
  population was not repeated under statement accounting. At the default 500 ms
  idle pause a replica would be busy for a whole interval at roughly 20,000 to 30,000 pending approvals if
  the growth stays linear (extrapolated, not measured).
- **G467 — Three conformance scenarios failed once under host load and passed alone.** In the first full
  run (host load 30–47) `run-input-blobs` (java, deadline `+59s` against `+60s`), `scoped-identity` (go
  `the worker log has 3 entries, not 6`; python `RUNNING` against `WAITING`) and `secrets` (typescript
  and go, event 3 `step.completed` against `step.started`) differed. Each passed alone minutes later, and
  the final full run is recorded in the matrix. These look like observation races (a snapshot taken
  before the worker finished) but the cause was not investigated; they are not hidden.

### Owner questions

- **O460 — Is a quadratic collection acceptable for the first release?** With G461 open, a published
  `maxItems` of 10,000 is correct but slow (one run took 1 h 54 min, the three budgeted runs did not finish
  in 10 minutes). Options:
  publish a lower ceiling until the transition is incremental, or schedule the fix before enabling
  collections. The recommendation is to cap `maxItems` and `maxIterations` at the largest size the report
  completed (1,000) until G461 is fixed.
- **O461 — Should an unreadable pinned definition quarantine the run?** Today it fails the poll (G460).
  A typed per-run skip with an alert would keep rollback safe.
- **O462 — Should long chains be supported beyond 127 segments?** Needs an object-free way to carry
  per-segment data (for example the declared state), or a documented ceiling of 127 for conditional chains.
- **O463 — Is the idle tick cost an accepted ceiling on pending waits per replica?** G468 puts it near
  20,000 to 30,000 approvals per replica at the default interval. Either publish that as the ceiling, or
  make the discovery passes index-only and cheaper, or both.

## Decisions of the wait-entry continuation increment (D500+)

Closes the open parts of T014-038: G372 (a continuation whose entrypoint is a `waitEvent`), G370 (a parent
cancelled mid-chain in the five SDKs) and G371 (`RERUN_AS_NEW` in the SDKs); G375 is decided, not built.
Evidence is in `tests/integration/workflow-primitives/chain_wait.rs`, the `wait`-chain tests at the end of
`tests/fault/workflow-primitives/chains.rs`, `crates/infra-ir/tests/continuation.rs` and the scenarios
`chain-root-wait`, `chain-cancel-parent` and `chain-rerun`.

- **D500 — The successor of a wait-entry chain is created already waiting, in the hand-off transaction.** The
  scheduler evaluates the entry node's correlation expression over the successor's input and state (a
  `RunData` with no outputs), and `ContinuationSelection::with_correlation_key` carries the key to the write.
  The write reads the definition's entrypoint and refuses a selection that does not match it
  (`RuntimeNotIntegrated`: a key for an activity entry, or none for a wait). For a wait it inserts the run
  `WAITING`, the entry step `WAITING/EVENT` with `due_at`, `event_type`, `correlation_key` and `event_since`
  (the row the event dispatcher and the chain signal read: there is no separate subscription table), and
  `wait.started` as the third journal event instead of `step.released`. No `task.ready` outbox row is written
  (a wait is not a task). Everything else of the hand-off is unchanged (predecessor `SUCCEEDED`, lineage, quota,
  audit, `run.continued` outbox). `a_continuation_that_starts_with_a_wait_hands_off_to_a_successor_that_already_waits`
  pins the rows, the journal (`run.started`, `execution_chain.successor_linked`, `wait.started`) and the absence of
  a hint; `the_store_refuses_a_selection_that_does_not_match_the_entry_the_definition_declares` pins the refusal.
- **D501 — Published shapes: `waitEvent -> continueAsNew`, and `waitEvent -> condition -> continueAsNew | end`.**
  Closed with a typed `Unsupported` at publication: `onTimeout`, a look-back above 0, declared state, a wait
  followed by another wait, an activity, an approval or any extra node, and every other entry kind (the message
  now says "an activity or an event wait"). Look-back stays closed on purpose: events are delivered per consumer
  (`wait:{run}/{node}`), so a successor that looked back would be a new consumer of the very event that ended its
  predecessor's wait and would be woken by it again. (`publishes_an_event_wait_entrypoint_followed_by_a_continuation_or_one_condition`,
  `publication_keeps_every_other_wait_entrypoint_shape_closed_with_a_typed_refusal`.)
- **D502 — The wait is measured from the write transaction's clock; the run keeps the chain's original deadline.**
  `event_since` and the wait deadline use the `clock_timestamp()` read inside the write, not the scheduler's
  earlier read, so an event admitted between the two is not older than the wait it finds. An event (or signal)
  that arrived before the hand-off is not held back for the successor: it is not matched by a wait that does not
  look back, which is the documented non-buffered rule
  (`an_event_admitted_before_the_hand_off_does_not_wake_the_successor_and_one_after_does`); one admitted after it
  finds the wait through the dispatcher's unscanned-wait scan. The brief for this increment said the deadline
  written is the chain's original one; that is the run's `deadline_at`, which the successor inherits
  (asserted equal to `execution_chains.original_deadline_at`). The wait's own deadline is the node's `timeoutMs`
  from the hand-off, as for any wait. It is deliberately not capped at the chain's deadline: both would be due
  at the same instant and the wait timeout (`FAILED`, `WAIT_TIMED_OUT`) would race the deadline pass
  (`TIMED_OUT`); uncapped, the deadline is strictly earlier whenever it is the binding one
  (`the_original_deadline_ends_a_waiting_successor_and_its_chain_as_timed_out`,
  `the_wait_of_the_successor_times_out_by_its_own_timeout_like_any_other_wait`). See O500.
- **D503 — A correlation expression that fails over the successor's input ends the chain, like a failing transfer
  expression.** The scheduler reuses the continuation-expression path: the predecessor's run stops `FAILED` with
  `EXPRESSION_ERROR`, the segment is `FINAL` and the chain `FAILED`, in one fenced transaction, and no successor
  exists (`a_successor_whose_correlation_cannot_be_computed_ends_the_chain_with_a_typed_failure`).
- **D504 — Signal and hand-off, both orders, are decided by the chain lock and never buffered.** At a boundary the
  predecessor's wait is already satisfied, so a signal fenced to the old head is refused (`not waiting`) when it
  is first and `CHAIN_HEAD_CHANGED` when the hand-off is first; one that names no head and queues behind the
  hand-off reaches the wait the hand-off opened, once. Held locks, not sleeps (D482):
  `a_signal_and_the_hand_off_of_a_wait_chain_commit_in_either_order_and_the_next_signal_is_not_lost`.
  A hand-off that fails after it wrote the step row leaves no subscription and the retry writes exactly one
  (`a_failed_hand_off_of_a_wait_chain_leaves_no_subscription_and_the_retry_writes_one`); after a restart a new
  scheduler and a new dispatcher find the wait in the database and wake it once
  (`a_restart_between_the_hand_off_and_the_event_still_wakes_the_successor_once`). A duplicate event id and a
  replayed signal key wake the successor once (`a_duplicate_delivery_wakes_the_successor_once`).
- **D505 — The SDK harness learned three things, all additive.** `list` takes `waitForCount` (list again until a
  hand-off has produced that many runs in a status, the way `inspectChain` already waits for a head) and
  `countOnly` (answer `{count}`): the ids of the first run of a child chain are derived from the parent's and
  `GET /runs` orders by id, so a list of runs mixing them has no stable order across languages (the same cause as
  D483). A step whose result has `newRunId` and no `runId` (a rerun) can be named by `run`. The goldens of every
  earlier scenario are unchanged.
- **D506 — G370 is closed by `chain-cancel-parent`.** The chain waits on an event the scenario publishes, so a
  segment is held open without a sleep: one hand-off is awaited by counting `SUCCEEDED` runs, the parent is
  cancelled while the second segment waits (`propagate`: the head is cancelled, the parent ends `CANCELLED`), and a
  parent that `detach`es its chain is cancelled while the chain keeps going (an event still hands it off).
  `request_cancel` has no SDK scenario (G502).
- **D507 — G371 is closed: `RERUN_AS_NEW` is public, so the SDKs name it.** `operate` accepts the action,
  `rerunAsNew(runId, reason, options)` is its shorthand (like `cancel`), `OperationResult.newRunId` is set only
  for it and `Run.rerunOf` only for a rerun, in TypeScript, Python, Go, .NET and Java. Java and .NET keep the old
  constructors/positional shapes, so nothing that compiled stops compiling. `chain-rerun` shows the rerun of a
  continued segment as an unrelated run with a chain, a fresh deadline (`<chain-deadline:2>`) and `rerunOf`, a rerun
  of a run that has not ended refused `CONFLICT`, and the original chain untouched. The command line gains
  `workflow rerun <RUN> --reason <TEXT>` (exit 1 on 409); the Studio already had the operation.
- **D508 — G375 is not built: `inspectChain` keeps returning every segment (at most 10,000 small entries).**
  Bounding it by default changes the answer of every existing client with a long chain, and an opt-in `limit` /
  `cursor` on the same route would duplicate `GET …/runs`, which is the paged read (D378). The OpenAPI description
  of `inspectExecutionChain` now says so and points to it. Revisit if a client ever needs the head and the page in
  one answer.

### Gaps and owner questions of this increment

- **G500 — The correlation key is not covered by the transfer digest.** The store checks the input and state
  bytes against their digests under the locks, but it cannot evaluate the correlation expression, so it trusts
  the key the fenced scheduler selected. Only the scheduler holds that fence, the key is journaled in
  `wait.started`, and a wrong key would only make the wait unreachable (a deadline ends it), but the store does
  not prove it.
- **G501 — `MARK_MANUAL_REVIEW` has no SDK method either.** It has the same shape as `RERUN_AS_NEW` (an
  `operate` action); it was not part of G371.
- **G502 — A parent cancelled with `request_cancel` has no five-SDK scenario.** The behaviour (the parent waits
  until the chain settles) is proved in Rust (`request_cancel_makes_the_parent_wait_until_the_child_chain_has_settled`).
- **G503 — A wait-entry chain cannot use declared state, `onTimeout`, a look-back or a longer graph.** See O501.
- **G504 — Pre-existing, seen while running the SDK suites.** `mypy` reports five errors in
  `sdks/python/conformance/runner.py` (the `listChainRuns` code) and `ruff format --check` lists seven files, both
  on the base commit too. The .NET test `Protocol14StreamsTwoFramesDispatchesBeforeEofAndCancelsTheIdleReceiveOnStop`
  failed once in a full run on a loaded machine and passes alone (a timing assertion). `sagat-cli`'s
  `diagnostics::every_code_the_platform_returns_has_help_...` fails on the base commit too: nine domain codes
  (`BODY_DEPTH_EXCEEDED`, `DUPLICATE_BODY`, `INVALID_CONTINUATION_POLICY`, `INVALID_SCOPE`, `INVALID_SWITCH`,
  `INVALID_WORKFLOW_LIMIT`, `RECURSIVE_BODY`, `UNKNOWN_BODY`, `UNREACHABLE_BODY`) have no help text in the CLI catalog.
  Two `fault_workflow_primitives` loop tests (`loops::a_cancel_that_...`) failed once at a load average above 20
  and passed in two repeats and in the full rerun (82 of 82).
- **G505 — An approval entrypoint (O441) is still closed.** The wait entry shows the pattern (open the step and
  its durable row in the hand-off transaction); an approval additionally needs its request row and
  `approval.opened` journaled the same way. Not started.
- **O500 — Should the wait of a successor count its `timeoutMs` from the chain's start rather than from the
  hand-off?** Recommendation: keep it (D502). The chain's own `timeoutMs` already bounds the whole process, and a
  per-segment wait that counted from the chain start would make the wait of a late segment shorter than the
  author wrote.
- **O501 — Should a wait-entry chain accept `onTimeout` (to an `end`) and declared state?** Recommendation:
  `onTimeout` to an `end`, yes, later: it is the natural way for an idle chain to finish `SUCCEEDED` instead of
  `FAILED` with `WAIT_TIMED_OUT`, and it needs only a test that the routed `end` finalizes the chain. Declared
  state is independent of the entry kind and could be allowed with the same state-transfer tests as the
  activity shapes. Look-back stays closed (D501).

## Decisions of the scale increment (G461, G468, G460)

Written 2026-10-17 on `feature/014-scale`. The coordinator ticks the tasks; nothing here marks one.
The decisions below were written before the code and are amended at the end of this section where the
implementation or a measurement changed them.

### Decisions

- **D520 — A collection or loop transition reads a frontier, not the run.** Today every scheduler
  transition calls `RunStore::begin`, which restores the whole `RunState` (every step, every collection
  iteration with its checkpoint, every loop iteration, every step output) and then discards almost all
  of it: the decision reads the scope's counters, the iterations in flight (at most `maxConcurrency`
  for a collection, one for a loop) and the steps of those iterations. A new port,
  `ScopeFrontierStore`, returns exactly that: `CollectionFrontier` / `LoopFrontier`, which carry the run
  header (version, stop reason, namespace, trace, definition, the database clock), the scope counters,
  the active members with the steps of their paths, a flag for other open root work, and a `RunData`
  limited to the run input, the declared state and the outputs and scope inputs of the active members.
  The domain reducer is not restructured: the scheduler transitions never ran it (they call the
  store's own transactions, which were already incremental), they only used `RunState` as a read model.
  The pure domain crate is untouched; `RunState`, `CollectionState` and `LoopState` keep holding the
  whole scope because replay, `verify_run`, the API projections and scoped compensation need it.
- **D521 — The reference path stays and is the oracle.** The transitions that are proportional to the
  scope by nature keep using the whole-run read: entering a collection or loop (snapshot of the items),
  the last checkpoint of a collection (the aggregate is checked against its ceilings), the finish (the
  aggregate is produced), and scoped compensation (the planner reads every completed iteration). Each
  runs once per scope. The frontier path returns "needs the whole run" for them and the scheduler runs
  the previous code unchanged, now named `advance_*_from_run`. It is public so the differential test
  can drive the same scope through both and compare the journals.
- **D522 — The instance charge comes from the store's own counter.** The loop's affordability check
  used `RunState::charged_instances()` (rows of `steps` that are scoped or left `Pending`). The store
  keeps the same number in `run_instance_usage.admitted_instances` (it is incremented by the same events:
  an admission, a materialized reservation and the root admission trigger); `reserved_instances` is the
  separate, not yet materialized part. The frontier reads one row. A test compares both numbers after
  every loop transition, so a divergence is a failing test, not a silent change of limit behavior.
- **D523 — The frontier is a snapshot read with no row lock.** `REPEATABLE READ READ ONLY`, one
  transaction, so the counters, the members and the steps are from one snapshot. The run row is not
  locked: every write of a transition already takes `FOR UPDATE` on the run, checks the fence and
  compares `expected_run_version` (the frontier carries the version it read), so a concurrent change
  makes the write return `RevisionConflict`, which the pass already counts as skipped. The previous
  path held a lock only while it read and released it before the write, so this does not weaken the
  protocol; it removes a lock acquisition from the read.
- **D524 — The integrity audit of a whole-run restore is not run on these transitions.** `begin`
  re-proves every collection and loop projection against the steps (identity, counters, checkpoints) each
  time it runs. The frontier checks what it uses (a counter read from the scope must agree with the rows
  fetched for the active members: `active_count` against the number of admitted rows; the member rows
  against the steps of their path) and the writes re-check the rows they change, as they did. The full
  audit remains available to every other reader of the run and to `verify_run`, and the tests run it
  after the transitions. This is the one place the change gives up a continuous self-check; it is
  recorded as G524, not hidden.

#### Amendments to D520-D524 made while building

- D520 as built: `CollectionFrontier` / `LoopFrontier` live in `sagat-application::scope_frontier`; the
  adapter is `PgRunStore` (`infra-postgres/src/scope_frontier.rs`); the scheduler's `Scheduler` needs
  the new bound `ScopeFrontierStore` (only `PgRunStore` implements it). The domain crate is untouched.
  The frontier path decides: the open-phase failure scan, the pending-return checkpoint (except the last),
  admission, drain of an iteration, drain finish (except a compensating scope) and, for a loop, cancel,
  deadline, delay, failure, checkpoint and drain. The whole-run path (public `advance_collection_from_run`
  and `advance_loop_from_run`) decides entering, the last checkpoint, the finish and compensation. The
  loop checkpoint tail (`commit_loop_checkpoint`) is one function used by both paths, so the instance
  limit, the delay-versus-deadline rule, the byte ceilings and the quota drain have one implementation.
- D522 as built: the number is `run_instance_usage.admitted_instances`. `RunState::charged_instances()`
  and that column are compared after every transition of a collection and of a loop in
  `the_frontier_equals_the_restored_scope_at_every_step` and its loop twin; they were not run (see G525).
- D523 as built: `SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY` as the first statement of a
  `begin_safely` transaction; nothing is locked or written.

### Decisions (continued)

- **D525 — Two partial indexes make the open steps of a run cheap to find.** `finish_version` (every
  collection and loop write) asks `EXISTS (steps ... status IN (PENDING, READY, LEASED, RUNNING,
  RETRY_WAIT))` and then `EXISTS (... status = WAITING)`. Without an index on status that walks the run's
  steps in primary-key order until it meets one, and the open ones are the newest, so it read every
  finished iteration: the residual buffer growth after the frontier (210 / 237 / 262 buffer accesses at
  160 / 320 / 640 items) disappeared with the indexes (225 / 247 / 217). The same indexes make the
  collection and loop discovery passes read only open steps. Migration `20261122000000_steps_open_index`
  (up and down; indexes only, so `storage-classification.json` has nothing to add). The cost is one index
  entry per step while it is open, removed when it closes; it was not measured on the write path beyond
  the runs above (G523).
- **D526 — The G461 tests run in every suite and are asserted on rows and buffers, not time.** The two
  `red_*` tests are no longer ignored and pass; `collection_transition_cost_is_flat_across_sizes` and
  `loop_transition_cost_is_flat_across_sizes` run a scope at 160, 320 and 640 and assert that the median
  late transition does not move (rows within 20 percent + 4, buffers within a third + 40). The shared test
  PostgreSQL container preloads `pg_stat_statements` and the performance platform creates the extension
  in its database, so they run on the default container; on a server that does not preload it they fail
  with a message that says so. `SAGAT_PERF_REFERENCE_PATH=1` and `SAGAT_PERF_WITHOUT_OPEN_INDEXES=1` are
  the controls (the whole-run path; no open-step indexes).
- **D527 — An unreadable pinned definition is skipped for the poll, typed, counted and logged (G460).**
  `StoreError::UnreadableDefinition` is what the definition cache returns when a stored version no longer
  parses (it was `Internal`); every other caller still maps it to `AppError::Internal`, so no response
  changes. The worker `claim_run` treats it as "not this poll's run": it counts
  `worker_claim_skipped_total{reason="definition_unreadable"}` (one label value, so bounded), logs a
  warning with the run id, leaves the run exactly as it was and does not spend the three failures a poll
  tolerates. Status: implemented and **not compiled or run** when this was written (the disk was below
  the build threshold); the test is `runs_a_reader_cannot_read_do_not_starve_the_ones_it_can`.

### Gaps (continued)

- **G520 — The worker's `complete` and `poll` (`claim`) still restore the whole run, so a large scope is
  faster but not linear.** `claim_run` calls `RunStore::begin` and walks every instance for READY steps;
  `complete` runs the domain reducer on the whole `RunState`. Measured after G461: in the 10,000-item
  collection (c256) `complete` p50 78.4 ms (82.4 before; 18.5 to 21.4 ms at 1,000 items, 11.0 to 14.3 ms
  at 100) and the 64-task `poll` 384.3 ms; 10,000 x 78.4 ms is 784 s of the 1,405 s wall. The 10,000-
  iteration loop still did not finish in 1,800 s (8,806 iterations; `complete` 74.3 ms, `poll` 78.9 ms).
  Unlike the scheduler transitions, these do need the reducer: completing a step derives the run's
  status from every step. The smallest restructuring that keeps `RunState` equal to a replay is to let
  the reducer run on a window and give it the aggregates it needs from the run (open-step count, the
  charged instances, the settled-scope summaries) from the store, which is a change to the domain that
  needs its own decision and its own proof; it was not attempted. A cheaper first step is a
  ready-step index so `claim` does not walk the run.
- **G521 — Four transitions are still proportional to the scope, once per scope each:** entering (it
  snapshots the items, G464), the last checkpoint (the aggregate is checked against its ceilings over
  every result), the finish (the ordered aggregate), and compensating a scope (the planner reads every
  completed iteration). At 10,000 items the finish took 218.0 ms and enter 3,343.9 ms. A draining scope
  with other root work open is decided from the frontier (it reads root steps only).
- **G522 — The first 10 idle ticks after a bulk load of 10,000 approvals were slower after the change
  (185.9 to 320.8 ms, one run each, host load 10 to 40) while the same ticks after `ANALYZE` were far
  faster (197.6 to 11.2 ms).** The 10-tick row is taken before the tables have statistics. Not explained;
  the harness now measures the 10 ticks before and after `ANALYZE` and `SAGAT_PERF_WITHOUT_OPEN_INDEXES=1`
  gives the control on the same binary. Repeat on a quiet host before reading anything into the 320.8 ms.
- **G523 — The write cost of the open-step indexes was not measured separately.** Run creation and
  step updates were within the noise of the runs above (the wait harness created 10,000 approval runs at
  143 runs/s at host load 10 to 40, against 191 to 203 runs/s in the earlier A/B at load 8 to 25), which is
  not evidence either way.
- **G524 — The continuous projection audit of a whole-run restore is not run on these transitions (D524).**
  The frontier checks the counters it uses (active count against the rows holding a slot, the running
  loop's active iteration against its row, the entry path against the iteration index); the writes
  re-check what they change; `verify_run` and every other reader of the run still run the whole audit.
  A corrupt projection of an iteration that has already ended is no longer noticed by these transitions.
- **G525 — Verification that was not done in this session because the disk fell below the build
  threshold (15 GB free, down to 6 GB):** the integration differential tests
  (`scope_frontier.rs`) were compiled up to one missing import that was then fixed, and never run; the
  G460 change was never compiled; `cargo clippy --workspace --all-targets`, the suites listed in the
  task (integration/fault/contract workflow primitives, platform foundation with the runtime-privilege
  audit and `SAGAT_TEST_AS_RUNTIME_ROLE=1`, durable execution, sagas, security-multitenancy) and the
  five-SDK collection and loop scenarios were not run after the change. What was run: `cargo check
  --workspace --all-targets` on the frontier code, the four G461 performance tests, the harness
  measurements in the report, and the discovery queries under `EXPLAIN (ANALYZE)`.
- **G526 — A poll that has more than 64 unreadable runs ahead of the first readable one still serves
  nothing.** `claimable_runs` hands `claim` at most 64 candidates, oldest first. G460's skip makes each
  unreadable run cost one failed restore instead of ending the poll, but 64 of them still fill the window.
  A negative cache of unreadable versions consulted by `claimable_runs`, or a candidate query that
  excludes them, would close it; both need a decision about where the "this build cannot read version V"
  knowledge lives.

### Owner questions (continued)

- **O464 — Is the worker report (`complete`, `poll`) in scope of G461?** The scheduler transition is
  linear now; the 10,000-item collection takes 23 minutes instead of 114, and the 10,000-iteration loop
  still does not finish in 30 minutes. If a 10,000-item scope has to finish in minutes the worker path
  (G520) is the next fix and it is a domain change. If not, the recommendation of O460 changes from
  "cap `maxItems` at 1,000" to "10,000 is correct but takes minutes per run; 1,000 takes about one minute".
- **O465 — May the shared test container preload `pg_stat_statements`?** It is how the G461 tests count
  rows. It changes the server flags of every suite's default container (no data effect).
