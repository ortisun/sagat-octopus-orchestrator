# Feature Specification: Durable workflow primitives and practical SDK examples

**Feature Branch**: `feature/workflow-primitives`
**Created**: 2026-10-14
**Revised**: 2026-10-15, after validation against accepted ADRs and canonical contracts ([analysis](./analysis.md))
**Status**: Architecture accepted in ADR-0030/0031/0032; runtime implementation partial
**Input**: Specify additional workflow primitives with Spec Kit and improve practical workflow examples in TypeScript, Go, Python, C# and Java.

## Context and delivery boundary

Developers need to route orders, process a runtime-sized collection, repeat work until a business
condition is met, request a person's decision, inspect/change a workflow's business state and
continue a long-lived process without an ever-growing individual run.

The accepted implementation scope includes US01–US07. US01 supplies practical examples of
**existing** behavior. Bounded switch, run-addressed declared Query/Update and root approval have
public runtime evidence. Continuation has restricted public paths with an activity followed by
continuation, an activity and condition choosing continuation or a terminal end, or an activity
and signal-controlled `waitEvent` followed by input-only continuation. Direct and conditional
continuation may transfer the exact declared state snapshot. A continuation may also start with a
`waitEvent` (followed by an input-only continuation, or one condition choosing it or an `end`): the successor
is created already waiting, in the hand-off transaction.
Collections have a restricted public path: an optional prefix of ordinary root source activities,
then one or more root `forEach` nodes ending at `end`, with each referenced body containing a chain of one to eight activities followed by one `return`; a body
activity may declare `compensateWith`, `reconcileWith`, `irreversible` and `highRisk`, and a failed or
cancelled scope then compensates its completed iterations (FR-014-005, FR-014-006). A source prefix
may supply an inline array or its own committed blob through `steps.fetch.output`; source-prefix
error edges, compensation/reconciliation, effect policies and managed database operations remain
closed. Producer failure enters no collection, and an uncommitted/foreign blob is not a source
capability. Admission remains
disabled until an administrator sets the tenant-wide iteration-concurrency quota. A restricted
public loop with an activity/return body has focused PostgreSQL restart and rollback evidence.
Scoped scheduler expiry/retry, worker discovery/claim, heartbeat and inline completion/failure
now use exact-path transactions, including compensation/reconciliation and fenced provider output
scans. Scoped secret delivery and blob capability checks also use current exact-path state;
secret resolution closes its transaction around provider reads and rechecks current authority before
auditing a handoff. Public scoped uploads skip the root-only managed database probe and obtain their
current exact-path capability before reserving object bytes; a skipped probe grants no access.
Ordinary forward scoped activities may complete with their own uploaded blob: current exact-path
lease authority precedes catalog access after a receipt miss, while an exact receipt remains
replayable after expiry under current role/binding checks. The exact descriptor, catalog commit,
quota, journal, outbox and receipt share one transaction. Known produced results may be reported
during cancellation to account for their effects. Derived/declared-secret blob outputs remain
refused. Broader body planning and the large resource gates remain separate work; these focused
bounded paths do not establish the production ceilings. A new child is
admitted under its parent's shard lease only inside its uncommitted zero-event creation transaction;
a reopened child requires its own shard lease for subsequent scheduler mutations.
Child reservation uses a parent write lock compatible with foreign-key key-share checks; tree
and active-run admission precede input/initial-state payload retention. This ordering applies
to existing child creation and evaluated continuation, independently of nested collection capacity.
Parent cancellation and final notification use a matching child-chain link alone; legacy child
flag writes are restricted to children without a link and with the exact own-parent association.
ADR-0037 retains the existing early parent key-share guard solely for identity protection under
immutable initial/head/link ancestry. It grants no parent mutation authority; the remaining
lock graph and nested-capacity transactions are still pending.
Exact-path storage entry also retains an applicable chain lock before its own run, matching full
run entry. This storage invariant alone does not enable chain+collection graphs or new finality.
Declared Updates addressed by run also acquire an applicable membership chain before the exact
run; they keep their original address and never forward across continuation.
Approval open/expire/decide also take an applicable member chain before their exact run.
This preserves current-user authorization and never forwards a request to another segment.
Collection and loop own-run writer helpers also acquire the applicable chain first, including
collection drain. This storage prefix grants no new publication or nested execution capability.
Chain handoff/finalization validates scheduler expiry on the database clock after its SHARE
lock wait, in the same transaction. Collection and loop ports use that same transaction-typed check and preserve their existing
wrong-shard and oversized-generation errors. Approval open/expire use the same post-lock check;
a human decision continues to use its separate current-user authorization. Collection entry
writes a supplied audit once through its atomic version commit.

Other collection and loop graphs and nested scopes remain closed for general publication. The
remaining acceptance and release evidence is tracked
in [the validation report](./validation-report.md).

Publication diagnostics preserve schema-first rejection. Cyclic, escaping, overlapping or
recursive graphs that pass the schema return `INVALID_SCOPE`; unsupported expression literals
and effective expression byte-limit failures return `EXPRESSION`. An ordinary dangling root
edge retains `GRAPH`. A rejected draft creates no published version. These diagnostics do not
widen the accepted runtime shapes.
[ADR-0030](../../docs/adr/ADR-0030-structured-instances-and-continuation-chains.md) accepts the
execution design and [ADR-0031](../../docs/adr/ADR-0031-declared-operation-result-disclosure.md)
accepts the declared-result disclosure policy. [ADR-0032](../../docs/adr/ADR-0032-additive-workflow-ir.md)
keeps the existing v1 IR and adds capabilities without introducing a new public revision. These
decisions do not make unimplemented operations available. `condition`, static `parallel` with `all`/`any`, timers,
`waitEvent`, child workflows and sagas already exist. `parallel.any` means first **successful**
branch, failing only when all branches fail; it is not a new first-completion race primitive.
A signal-based sample approval is not a native approval with approver authorization.

The accepted primitives build on these existing rules, which they do not change: a signal is accepted
only by a step that is waiting for it and is consumed in the same transaction, never buffered
(`signalRun` in the canonical OpenAPI); compensation runs in reverse topological order and respects branch independence
(FR-003-002, ADR-0005); general payload inspection discloses only authorized structure (ADR-0028),
while explicit declared operation results follow ADR-0031;
roles are the built-in ones, with no custom roles (G39); errors use the closed `Error.code` set with
`details.reason` refinements; `Run.version`/`expectedRunVersion` is the run's optimistic-concurrency
token; and one child-run tree holds at most 1,024 runs at depth 8 or less.

## Domain vocabulary

“Workflow primitives” is the product-level term for the added capabilities, not a replacement Rust
abstraction. The existing definition model remains `WorkflowDefinition`, `Node` and `NodeKind`;
execution records each activation as `StepState` inside `RunState`. Extend those models where the
new behavior belongs. The cohesive `execution/instance.rs` module introduces instance identity,
not a generic `Primitive` object or an alternate workflow engine.

`ExecutionInstanceId` is the **run-local** key `{path, node}` because `RunState` already owns the run
ID. The complete persisted identity is `(tenantId, namespaceId, runId, path, definitionNodeId)`;
the application/store supplies the owning context. Do not duplicate tenant/run fields inside every
run-local key or confuse definition nodes with their repeated runtime activations. Approval, state
operations and chains can use separately named cohesive modules when implemented.

## User Scenarios & Testing *(mandatory)*

### US01 — Learn realistic workflows in any official language (Priority: P1)

A developer can copy three order examples, understand their decisions and run the same business
cases in any of the five SDKs.

**Why this priority**: Delivers useful examples now without depending on an unimplemented primitive.

**Independent Test**: Build each example with the checked-in SDK, execute all examples against the
real engine and compare workflow definitions and normalized terminal observations across languages.

**Acceptance Scenarios**:

1. **Given** an order with amount 100, **when** `order-routing` validates and routes it, **then**
   its approved branch runs and the rejected end is never reached; an amount of -1 takes the
   rejected branch and notification never runs. The rule (`amount > 0` accepts, so 0 is rejected)
   lives in each example's `order.validate` activity; the shared suite exercises 100 and -1 only.
2. **Given** `order-fulfillment`, **when** inventory reservation and shipping quotation run in static
   parallel branches, **then** notification happens only after both succeed.
3. **Given** `order-reminder`, **when** the timer expires, **then** notification runs after the wait.
4. **Given** the documented permanent-failure input, **when** the reminder notification fails
   permanently, **then** the run finishes FAILED, the `notify` step's terminal failure carries
   `NOTIFICATION_REJECTED`, no `notify` completion is journaled and the program exits 1 without
   reporting success. (The run resource exposes status only; the typed code is on the step failure.)
5. **Given** the five language implementations, **when** the shared example suite runs, **then** their
   canonical definitions, business results and errors match; documentation states which behaviors
   were exercised locally and which require Docker or a language toolchain.

### US02 — Process a collection with bounded concurrency (Priority: P1)

An author processes each order line independently, limits simultaneous work and receives results
in input order regardless of completion order.

**Why this priority**: Runtime-sized collections cannot be represented by enumerating static branches.

**Independent Test**: Process 17 elements with concurrency 3, force reverse completion order and a
restart, then assert 17 distinct logical instances, ordered results and at most 3 active iterations.

**Acceptance Scenarios**:

1. **Given** a collection and an acyclic body, **when** `forEach`/`map` enters, **then** collection
   membership and indices become fixed; a retry or restart neither rereads a changed source nor
   creates a second logical instance for the same index.
2. **Given** zero elements, **when** the collection enters, **then** it succeeds with an empty result;
   exceeding its item limit fails before any body work is dispatched.
3. **Given** a permanent failure in one iteration, **when** it commits, **then** new iterations stop,
   active iterations are cancelled/drained, and the enclosing failure/compensation policy applies:
   within an iteration in reverse topological order, with iterations as independent branches.
4. **Given** duplicate completion or an expired attempt, **when** it arrives, **then** it cannot append
   a second result, release another concurrency slot or activate an extra iteration.

### US03 — Repeat work durably within a budget (Priority: P1)

An author polls a business condition or retries a business process using explicit iteration and
deadline limits, without a worker sleeping between iterations.

**Why this priority**: A durable loop must survive a restart and must not monopolize capacity.

**Independent Test**: Execute a three-iteration loop, restart after its second checkpoint and verify
that only the third iteration remains; exercise zero iterations and budget exhaustion separately.

**Acceptance Scenarios**:

1. **Given** `while` whose initial condition is false, **when** entered, **then** no body executes;
   `repeatUntil` executes its body once before its first condition check.
2. **Given** a completed body, **when** the next decision is committed, **then** its result, new carried
   state, condition outcome and next iteration/wait are durable together.
3. **Given** an unmet condition after the final allowed iteration, **when** another iteration would
   be needed, **then** the loop fails with `ITERATION_LIMIT`; it never silently reports success.
4. **Given** a timer, signal or cancellation racing a loop transition, **when** one transition commits,
   **then** competing stale transitions cannot create another iteration.

### US04 — Request an authorized human decision (Priority: P1)

An author pauses a workflow for a named approval policy; an eligible person approves or rejects it,
with an expiry route and durable evidence of who decided.

**Why this priority**: Business approval needs identity and audit guarantees beyond an event wait.

**Independent Test**: Open an approval, restart the engine, submit unauthorized and authorized
responses, then race a valid response against expiry and observe exactly one chosen route.

**Acceptance Scenarios**:

1. **Given** a pending approval, **when** an eligible person approves/rejects, **then** exactly the
   corresponding route becomes eligible, with the decision and audit record persisted together.
2. **Given** a revoked role, another tenant's identity or an excluded initiator, **when** that person
   attempts a decision, **then** no decision is applied and the denial is audited without disclosure.
3. **Given** a pending approval at its deadline, **when** expiry and response race, **then** first
   commit wins; a duplicate returns the original outcome and a conflicting decision is rejected.
4. **Given** a raw event/signal with an approval-looking payload, **when** delivered, **then** it cannot
   substitute for the authorized decision operation.

### US05 — Choose one of several routes (Priority: P2)

An author declares ordered business cases and a default, avoiding chains of binary conditions.

**Why this priority**: Improves readability with a small independent extension to existing routing.

**Independent Test**: Evaluate overlapping true cases, no matching case and an invalid predicate,
then verify one recorded route or a typed failure and identical behavior after replay.

**Acceptance Scenarios**:

1. **Given** two true cases, **when** `switch` evaluates, **then** only the first in declaration order
   is selected; when none matches, its mandatory default is selected.
2. **Given** an invalid/non-boolean predicate, **when** it is evaluated before a match, **then** routing
   fails with `EXPRESSION_ERROR`; later cases are not a fallback for evaluation failures.
3. **Given** a persisted choice, **when** restarted, **then** it reuses the choice without evaluating
   predicates against changed business state.

### US06 — Inspect and change declared workflow state (Priority: P2)

A caller reads a named business query or submits a named, validated update and receives its durable
result, rather than guessing state from activity logs or treating a signal acknowledgement as a result.

**Why this priority**: Enables interactive workflows while keeping state changes explainable.

**Independent Test**: Query a waiting run, atomically change its declared state, duplicate the update
and race two updates with the same expected state revision; assert one mutation and stable replayed
response.

**Release gate**: ADR-0031 accepts explicit declared-result disclosure separately from ADR-0028
structural inspection. The bounded run-addressed slice is enabled after authorization, audit,
bounded result, receipt/replay, transaction-fault and all-five conformance tests passed. The
chain-addressed Query/Update routes have chain-before-run locking, authorization, audit, head-CAS,
handoff-race and stable-receipt evidence from seeded chain fixtures. A direct
activity-to-continuation workflow can now publish and transfer its exact declared snapshot;
a single condition may also select that transfer or a successful end. Shared conformance in all
five SDKs covers Query and Update on the successor, exact Update replay, a stale expected-head
Query and final Query after conditional state transfer. Broader fault/race combinations remain
open. Other combinations of
state with structured primitives remain disabled until separate evidence passes.

**Acceptance Scenarios**:

1. **Given** a declared query, **when** called, **then** it returns a consistent committed snapshot with
   the observed run version and state revision, without changing workflow state or scheduling work.
2. **Given** a valid declared update, **when** accepted, **then** state, returned value, deduplication
   receipt and journal are committed together. An update schedules no work by itself; the workflow
   sees the new state at its next expression evaluation.
3. **Given** the same update key and different content, **when** retried, **then** it is rejected;
   identical content returns the original durable result even after a lost response.
4. **Given** a business validator rejection or revision conflict, **when** submitted, **then** no
   business state changes; a caller sees a stable typed rejection rather than an activity retry.
5. **Given** an attempt to use a query/update handler for arbitrary code, network access or waiting,
   **when** published, **then** validation rejects it.

### US07 — Continue a process in a fresh run (Priority: P3)

An author ends a completed processing segment and explicitly hands selected state to a successor,
keeping the chain discoverable and the old history intact.

**Why this priority**: Useful for long-lived processes after ordinary durable primitives are stable.

**Independent Test**: Continue through three segments, crash around the handoff transaction and
verify one successor per segment, one current chain member and independently inspectable histories.

**Acceptance Scenarios**:

1. **Given** a quiescent run at `continueAsNew`, **when** its handoff commits, **then** the old run is
   terminal, a linked successor is ready and the chain points at that successor atomically.
2. **Given** duplicate handoff execution or a lost broker notification, **when** recovered, **then**
   the existing successor is resumed and no additional successor is created.
3. **Given** active work, unresolved compensation, approval or subscriptions, **when** handoff is
   requested, **then** it is refused; work and external effects are never silently abandoned.
4. **Given** a run pinned to a version, **when** it continues, **then** its successor pins the same
   version and only explicitly selected input/state transfers; publication of a newer version has
   no effect on either run.
5. **Given** a parent awaiting a child that continues, **when** the old child segment ends, **then**
   the parent waits for the chain's final outcome instead of treating the handoff as child success.
6. **Given** a continuation transfer expression that cannot be evaluated on a committed run
   snapshot, **when** the scheduler advances it, **then** the current segment and chain finish once
   with durable `EXPRESSION_ERROR`, journal, audit and outbox in the same fenced transaction.

### Edge Cases

- Empty and oversized collections; duplicate values at different indices; nested scopes exhausting a
  shared run budget; deterministic ordering with out-of-order completions.
- Failure after an external effect but before its checkpoint; retries reuse that logical instance's
  effect key or reconcile an unknown outcome; cancellation is not proof an effect did not happen.
- Restart after collection snapshot, body completion, predicate choice, decision or handoff commit;
  broker loss and stale scheduler ownership; a failed compensation requires manual review.
- Approval expiry vs response vs cancellation; permission revoked after opening the request;
  two eligible people submit different decisions; duplicate key with different content.
- Query sees a consistent older snapshot while another transaction commits; update conflicts with
  timer/signal/worker completion; terminal/paused/compensating runs reject new updates.
- Continue-As-New races a run-targeted update/signal/cancel; exhausted chain or tenant quota; missing
  target payload; old-version clients see the completed old run but cannot infer chain completion.
- A chain-targeted signal whose current head is not waiting for it is refused, as for a run; it is
  never held for a successor. RERUN_AS_NEW of a chain segment starts an unrelated run (`rerunOf`).
- Child runs started inside bodies, and successors of a continuing child, exhaust the 1,024-run tree.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-014-001**: Provide three runnable examples in all five official SDK languages using only
  released/current repository behavior: `order-routing`, `order-fulfillment`, `order-reminder`, with
  documented approved/rejected inputs and injected permanent failure.
- **FR-014-002**: Validate the five examples through public SDK APIs against shared canonical
  definitions and normalized results, and document prerequisites, credentials from the environment,
  terminal assertions, observability and the boundary between available and proposed primitives.
- **FR-014-003**: Reject structural cycles, escaping/overlapping bodies, unbounded repetition and
  unsupported primitive contracts before publication; every dynamic instance has a durable identity.
- **FR-014-004**: `forEach` and value-returning `map` MUST use the same bounded collection semantics:
  snapshot membership once, stable indices, configurable concurrency, deterministic input-order
  aggregation and an empty result for an empty collection.
- **FR-014-005**: Collection failure/cancellation MUST stop admission, fence late attempts, drain
  active work and apply existing effect reconciliation and saga semantics without claiming rollback.
  Compensation keeps FR-003-002: reverse topological order within an iteration, no order between
  collection iterations (independent branches), and reverse iteration order for sequential loops.
- **FR-014-006**: `while` MUST evaluate before its first body and `repeatUntil` after each body; both
  MUST persist each carried-state/result/decision checkpoint, enforce explicit iteration/deadline
  limits and support a durable between-iteration delay without holding a worker.
- **FR-014-007**: Native approval MUST bind the request to its workflow instance, eligibility policy,
  initiating identity and deadline, and expose approve/reject/expire outcomes without a worker lease.
  Eligibility names subjects of the run's namespace and, optionally, built-in roles; there are no
  custom roles or identity-provider groups to name (G39; feature 013 excludes group mapping).
- **FR-014-008**: Approval decisions MUST authorize the caller at decision time, deduplicate responses,
  select exactly one outcome transactionally and reject raw signals as decisions; default policy
  excludes the initiator (the run's recorded `startedBy`; a continuation successor keeps the chain's
  original one) and cross-tenant requests reveal no resource existence.
- **FR-014-009**: `switch` MUST select the first true predicate in declaration order, require a default,
  record the selected route and reject predicate failures without falling through.
- **FR-014-010**: Workflow-defined Query MUST be a bounded, side-effect-free read over one committed
  state snapshot, return the observed run version (`Run.version`) and state revision and expose only
  its declared closed result schema under explicit `disclosure: declared-business-result-v1`
  (ADR-0031). Ordinary graph, journal, SSE and structural inspection retain their existing
  payload/value boundaries; namespace operation grants cover all declared operation names.
- **FR-014-011**: Workflow-defined Update MUST be a bounded declarative validation/state-change/result
  operation with run-scoped serialization, durable idempotency, typed rejection and optional expected
  state revision; it MUST NOT execute user code, perform external effects, wait inside the operation
  or schedule work. In the first increment only Updates change declared state.
- **FR-014-012**: Continue-As-New MUST atomically close one segment, create exactly one successor,
  advance a durable chain and persist transferred input/state; it MUST require quiescence and preserve
  the original version, parent relationship and original chain deadline.
- **FR-014-013**: Callers MUST be able to inspect individual segments and chain links; writes targeting
  an old run MUST NOT silently move to its successor; explicit chain commands MUST atomically resolve
  the current run and deduplicate across rollover; existing completion waits remain segment waits
  and a new explicit chain wait follows FINAL; a parent awaiting a child MUST follow the chain
  until its final segment, and a terminal segment MUST never reopen.
- **FR-014-014**: All new transitions MUST preserve journal order, scheduler ownership, worker fencing,
  inbox/outbox deduplication and transactional scheduling/timer/subscription boundaries through crashes.
- **FR-014-015**: All primitives MUST enforce tenant-scoped admission, size, rate, concurrency and
  retention limits; nested scopes share run budgets and Continue-As-New MUST NOT reset chain-wide
  deadline, authorization or tenant usage accounting. The existing child-run tree budget (1,024
  runs, depth 8) counts children started inside bodies and every successor of a continuing child.
- **FR-014-016**: Published definitions MUST be immutable and feature compatibility explicitly
  versioned; existing runs and SDK protocol minors retain their semantics, and all five SDKs MUST
  pass the same reviewed canonical contract and fault scenarios before a primitive is advertised.
- **FR-014-017**: Every primitive MUST expose correlated traces, bounded-cardinality metrics, redacted
  structured logs, typed errors (existing `Error.code` plus a `details.reason`) and durable audits
  for decisions, denials and manual interventions. The journal-derived runtime graph and telemetry
  projection (ADR-0016/0017) MUST show scoped instances, chosen routes and chain links with their
  lag; a Studio without support for a new capability shows those nodes as unsupported, never fails.

### Non-functional requirements

The accepted evaluator is the existing bounded subset: effective source limit 1024 bytes, 256
tokens, depth 16, list literals at most 64, constructed/concatenated lists at most 4096, generated
strings at most 64 KiB, and no object/map literal or mutation operator. Expressions may select an
existing object (for example `args` or `state`) without inventing object-construction syntax.
Collections may reference arrays of up to 10,000 input/blob elements only within the documented
capture/value/context budgets and after bounded materialization tests; this is not permission to
construct a 10,000-element CEL list. Optional top-level `limits` defaults to `maxInstances: 100000` and
`maxDepth: 8`; forward admission reserves declared derived compensation/reconciliation identities
within that shared lifetime budget so exhaustion or policy reduction cannot prevent their drain.
The IR remains `sagat-octopus-orchestrator.io/v1` with additive capability declarations; scoped
worker identity uses independent minor 1.5. Preserve previously accepted definitions and stored
runs. Publication checks enabled capabilities; the API version alone does not select runtime behavior.

- **NFR-014-001 (durability)**: A successfully committed logical instance, approval, update or handoff
  survives process/broker restart; duplicate delivery cannot create a second transition. External
  effects remain at-least-once and require idempotency/reconciliation where critical.
- **NFR-014-002 (security)**: Tenant/namespace identity comes from authorization context; every command,
  snapshot, chain link and payload reference enforces that boundary. Secrets never enter journal,
  audit, telemetry or responses. Authorization fails closed.
- **NFR-014-003 (resource bounds)**: Publication and runtime reject declared/executed budgets above
  configured limits. Benchmark the bounded fanout, nested-loop and 10,000-suspended-approval cases on
  a documented environment; publish queue delay, memory, journal growth and failures as measurements,
  not capacity promises.
- **NFR-014-004 (compatibility)**: Declarative expression semantics, numbers, UTC instants, errors,
  cancellation and normalized outcomes are identical across the five SDKs and survive supported
  upgrades with active old-version runs.

### Key Entities

- **Primitive definition / body**: immutable, bounded instruction and its closed acyclic body.
- **Execution instance**: a particular node within a run, scope and iteration; separate from a retry.
- **Iteration scope**: fixed collection or sequential carried state, cursor, budgets and ordered results.
- **Approval request / decision**: policy snapshot, deadline, eligible decision and audited outcome.
- **Workflow state / Query / Update receipt**: versioned business state, declared operation and durable
  mutation result tied to a caller-provided deduplication key.
- **Execution chain**: ordered run segments, one current member, predecessor/successor links and shared
  lifetime/deadline, distinct from an individual segment's immutable history.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-014-001**: Each of the five language examples demonstrates all three workflows and the same
  approved/rejected/failure outcomes through a reproducible automated check.
- **SC-014-002**: Every acceptance scenario US02–US07 has automated happy/failure evidence before its
  primitive is released; every official SDK produces matching reviewed observations for each.
- **SC-014-003**: Fault injection at each decision/handoff boundary produces no duplicate logical
  iteration, accepted approval, applied update or successor, and loses no acknowledged transition.
- **SC-014-004**: All cross-tenant, revoked-role, over-budget and secret-marker negative cases are
  denied or redacted; errors include a stable code and trace correlation without sensitive content.
- **SC-014-005**: Existing workflows remain executable after upgrade while active new primitive runs
  resume after restart with unchanged selected version and recorded decisions.
- **SC-014-006**: The resource benchmark reports observed concurrency within configured limits,
  zero dedicated workers for suspended approvals/loop delays and measured resource use for all
  three workloads, with reproducible environment and failures disclosed.

## Assumptions and dependencies

- The user-authorized examples extend feature 012 and reuse approved 001/002/006 contracts. Future
  runtime changes require the dependency/ADR review in T014-002 before changing those contracts.
- Runtime foundations are features 001 (contracts/security), 002 (durability), 003 (sagas), 004
  (events), 006 (SDK conformance), 008 (telemetry), 009 (security/quotas), 010 (scheduler ownership)
  and 011 (versioning). Studio authoring/visualization changes belong to feature 007 and need a
  separately approved dependent delivery; the runtime journal remains the source of graph truth.
- First native approval release supports one eligible decision, not quorum or delegation. Query and
  Update use declared expressions over persisted state, not arbitrary language callbacks.
- Settled on 2026-10-14: the value-disclosure policy for Query/Update results (ADR-0031, refining
  ADR-0028), the compensation rule for iterations and the role grants of the new permissions
  (ADR-0030, refining ADR-0005) and the additive v1 IR (ADR-0032). The bounded product defaults are
  listed in [research.md](./research.md#accepted-bounded-product-decisions). Accepting them proves no
  runtime behavior; each primitive still needs its release evidence in `tasks.md`.
- Still pending, outside the first increment: whether workflow nodes may assign declared state and
  whether a node may wait on a state predicate (neither exists today).
- Loops and fanout operate on bounded captured data; streaming/infinite iterators are out of scope.
- Automatic migration to a new definition, arbitrary procedural replay, external exactly-once effects,
  distributed/multi-region execution and automatic history deletion are out of scope. Explicit
  cross-version continuation requires a later approved contract and migration design.


Production script incident attribution follows the same applicable chain/own-run entry under
ADR-0038, then retains its existing binding EXCLUSIVE and own-step guards through audit commit.
This refines ADR-0022's reproduced opposite binding/run edge. Incident wire, current fenced
validation and audit-only semantics remain the existing feature009 dependency; no nested
execution capability or complete group lock graph is accepted.


Existing009 worker renewal must sample expiry after acquiring its session row, retaining the
guard through renewal commit. The public poll counterexample and correction are tracked in
B0. Complete and exact-path claim-room checks also sample time after their own session lock,
with zero capacity after expiry and the count/lease serialized through commit. Managed database
ownership separately checks time after its session SHARE acquisition; the complete group lock
graph remains pending.

The existing017 managed owner session SHARE guard also samples expiry after acquisition.
Its real managed-heartbeat counterexample/correction is tracked in B0 alongside renewal and
claim-room evidence; completion of those checks does not accept the full group lock graph.

Existing017 managed report receipt replay retains binding before catalog/current-role guards.
It preserves durable ownership and current authorization after session clearance; its focused
ordering barrier contributes to B0 and does not prove general nested/group ordering.

ADR-0039 advances the unrelated rerun source identity guard before creation admission without
authorizing source business state. Its runtime-role PostgreSQL/OpenBao race reproduces and fixes
the source/catalog deadlock; nested groups and remaining B0/B1 work stay unaccepted.

Capacity recovery has checked pure header restoration preserving revision/epoch and verifying
count/possession/holder consistency. This does not establish database or own-member authority;
SQL lifecycle, B1 completion and nested execution remain pending.

The first dormant capacity storage proof is restricted to Finish of an existing ChildRoot member
after exact current chain/head/segment FINAL under its own fence. Groups are fixture-seeded only;
no production creator, caller, admission/accounting or execution capability is accepted. It leaves
terminal run journals unchanged and requires an atomic group receipt/journal, truthful audit and
reference hint. The scoped contract is tasks/project-completion/capacity-finish-storage.md.

### Dormant capacity schema checkpoint — 2026-10-18

The fixture-only capacity header/member/Finish-journal schema is present with protected bounded
metadata, exact composite identity FKs, deferred active-holder consistency, immutable tombstones,
append-only receipts and narrow runtime column privileges. Empty downgrade is reversible; any
retained row prevents downgrade under table locks. This is no native Finish or nested execution
capability. Permanent chain FKs currently pin chain/run history; C0 must resolve retention/replay
compatibility before production activation. Evidence: tasks/project-completion/evidence/
capacity-finish-schema-20261018. T014-014..017/040 and broader acceptance remain open.

### Dormant native capacity Finish checkpoint — 2026-10-18

An exact internal ChildRoot Finish adapter is implemented without a production call site. It
proves current own child-chain/head/segment FINAL under current scheduler fencing, then atomically
finishes one membership with receipt/audit/outbox. Replay returns the original receipt under renewed
current authority. It never changes the terminal execution journal or other members' business
state. Native contention/fault/unknown-COMMIT evidence and observations remain separate B2e/f work;
no creation/admission/eligibility, retention rollout or nested runtime is enabled.


### Dormant native capacity observations checkpoint — 2026-10-18

Local B2e now has focused native contention, fresh fence, rollback, actual uncertain-COMMIT,
handoff and namespace boundary evidence. B2f observes returned outcomes with closed finite
metrics and a caller-traced workflow.capacity.finish span. Only an acknowledged first commit
increments its transition counter; receipt replay and uncertain acknowledgement do not. Durable
receipts remain authority and abandoned calls have no returned outcome. These are internal
dormant observations, with no production caller or activation. Full B1/B2, scoped finality,
retention/mixed writers, nested runtime and broader acceptance gates remain pending.


The initial leaf-success source proof has a pure versioned definition-shape predicate: a selected
ordinary activity-chain body with one to eight activities/return in a frozen root collection chain.
This is definition metadata only, with owner/body-qualified IDs and no persisted version/run or
settlement authority. Matching pinned source facts, immutable certification storage and actual
producer race/restart evidence remain prerequisites before any scoped Finish.

## Dormant scoped Finish metadata status — 2026-10-18

The fixed OrdinaryLeafV1 capacity member and receipt schema now retains a qualified original Run,
root owner and iteration index, with closed ChildRoot/ScopedPath variants. The extension preserves
the existing native ChildRoot writer and denies the new scoped INSERT columns to sagat_app.
Conditional downgrade is verified against retained scoped rows and actual native ChildRoot replay.
This is dormant storage metadata only; native scoped source consumption, production intent/
permissions and broader nested execution remain separately gated.


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
