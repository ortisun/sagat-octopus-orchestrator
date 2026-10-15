# Feature Specification: Durable workflow primitives and practical SDK examples

**Feature Branch**: `feature/014-workflow-primitives`
**Created**: 2026-10-14
**Revised**: 2026-10-14, after validation against accepted ADRs and canonical contracts ([analysis](./analysis.md))
**Status**: Specified; proposed runtime semantics, implementation pending
**Input**: Specify additional workflow primitives with Spec Kit and improve practical workflow examples in TypeScript, Go, Python, C# and Java.

## Context and delivery boundary

Developers need to route orders, process a runtime-sized collection, repeat work until a business
condition is met, request a person's decision, inspect/change a workflow's business state and
continue a long-lived process without an ever-growing individual run.

This delivery specifies those additions and supplies practical examples of **existing** behavior.
Only US01 is an implementation slice in this delivery. US02–US07 are future implementation work;
this document does not make them available. `condition`, static `parallel` with `all`/`any`, timers,
`waitEvent`, child workflows and sagas already exist. `parallel.any` means first **successful**
branch, failing only when all branches fail; it is not a new first-completion race primitive.
A signal-based sample approval is not a native approval with approver authorization.

The proposed primitives build on these existing rules, which they do not change: a signal is accepted
only by a step that is waiting for it and is consumed in the same transaction, never buffered
(`signalRun` in the canonical OpenAPI); compensation runs in reverse topological order and respects branch independence
(FR-003-002, ADR-0005); the API discloses no payload value, only authorized structure (ADR-0028);
roles are the built-in ones, with no custom roles (G39); errors use the closed `Error.code` set with
`details.reason` refinements; `Run.version`/`expectedRunVersion` is the run's optimistic-concurrency
token; and one child-run tree holds at most 1,024 runs at depth 8 or less.

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

**Release gate**: Query and Update results disclose business values through the API, which ADR-0028
does not permit today. US06 is not released before a value-disclosure decision is accepted (T014-002).

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
  its declared result schema. Returning values requires the accepted value-disclosure decision
  that refines ADR-0028.
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
  lag; a Studio without support for a new revision shows those nodes as unsupported, never fails.

### Non-functional requirements

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
- Pending decisions, not settled by this document: the value-disclosure decision for Query/Update
  results (refines ADR-0028); the compensation rule for iterations (refines ADR-0005); the role
  grants of the new permissions; whether workflow nodes may assign declared state and whether a
  node may wait on a state predicate (neither exists in the first increment). The product choices
  made while specifying are listed for confirmation in [research.md](./research.md#product-decisions-to-confirm).
- Loops and fanout operate on bounded captured data; streaming/infinite iterators are out of scope.
- Automatic migration to a new definition, arbitrary procedural replay, external exactly-once effects,
  distributed/multi-region execution and automatic history deletion are out of scope. Explicit
  cross-version continuation requires a later approved contract and migration design.
