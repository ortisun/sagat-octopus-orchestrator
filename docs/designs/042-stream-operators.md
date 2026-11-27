# Stream operators

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

Reactive-stream operators (the ReactiveX vocabulary: debounce, window, map, flatMap, filter, ignoreElements, skip,
distinct, take) as declared data in the workflow IR, on two surfaces:

- **on a `forEach`**, between the `items` expression and the membership snapshot: the operators shape the captured
  list (keep some, project, flatten, drop duplicates, group N elements per iteration). **Release 0.3.0.**
- **on a trigger**, between the inbox and the start of a run: an *operator chain* decides which events become a run and
  with what input ("one run per 30-second burst of webhook events for the same order"). **Release 0.3.0 too**, since
  Owner decision 2026-11-24-1.

Every predicate, key and projection is an expression of the existing bounded language, evaluated by the engine. No user
code runs. Every piece of state is a PostgreSQL row; a debounce or a time window is a durable deadline closed by a fenced
scheduler pass; a closed window starts exactly one run because the run's idempotency key is derived from the window.

The owner asked for it on 2026-11-13, as more features wanted in release 0.3.0, right after asking for throughput control
and buffering: "Debounce, Window, Map, FlatMap, Filter, IgnoreElements, Skip, Distinct entre outros" (OD-2026-11-13-13).
The sentence names operators and no surface. The first draft stated a reading and nine questions (O042-001 to O042-009),
each with a recommendation; the owner answered them the same day with "Aceitar todas" (OD-2026-11-13-15), so every
recommendation is the decision, and accepted ADR-0086 and ADR-0087 with "Aceito tudo" (OD-2026-11-13-17). In particular:

- **The collection operators** were planned for release 0.3.0 from the start: slice 0 (the collection half of the
  foundation) and slice 1. One small migration (the registry capability of Task 042-56), no table, no new pass or
  loop. Slice 1 changes the collection entry of the
  existing scheduler pass (section 3).
- **The trigger operators** were first left for a later release (O042-002) and then put in release 0.3.0 by the owner
  on 2026-11-24 ("Inclua tudo na 0.3.0", Owner decision 2026-11-24-1): slices 2 to 5 (`debounce`, `window`, `ignoreElements` and the rest
  on triggers, their state, the pass, the surfaces). Their fields enter the IR schema with slice 2 (Task 042-43).

This is the second draft. The architecture, security and correctness reviewers read the first at `540154c8` on
2026-11-13 (between them 6 Blocker and 29 Major findings, on the text; nothing was built). Their findings and how this
draft answers each are in `docs/designs/042-stream-operators.reviews/2026-11-13/`. The changes of substance: the
collection chain is bounded by running ceilings on elements and bytes, not by a count of evaluations; a blob source has
a manifest of its own for a chained scope; the capability is checked at publication, never on read; a chain that fails
at entry fails the run; the stage variables are `element` and `position`; held trigger values live in a deletable column
of the chain, keyed by HMAC, never in the append-only payload table.

This is the third revision (2026-11-26). It brings slices 2 to 5 to build after the re-read of Task 042-44 against
`develop`, under the owner's delegation for release 0.3.0 (Owner decision 2026-11-24-4). What changed: a window is sealed
only past the transaction horizon of its deadline instead of after a fixed admission grace, which on PostgreSQL 16 no
timeout bounds
([Architecture decision 0153](../adr/ADR-0153-a-window-is-sealed-only-past-the-transaction-horizon-of-its-deadline.md),
refining ADR-0086); both switches become capabilities of the instance registry, which now exists (ADR-0082), so the
fleet rule is enforced for every registered process (a binary from before the registry is still kept out only by the
upgrade rule, Gap 042-006); the seal and the start stay two transactions (Gap 042-002 closed); the
refinements of O042-008 and O042-009 and the numbers the owner had not answered (`maxMarks`, the held-bytes quota,
`maxHoldMs`) are decided as delegate rows (Owner decisions 2026-11-26-130 to 2026-11-26-136); and the start of a
window's run depends on feature 022's admission point, which is built on the branch `feature/022-throughput-control`
and not yet merged, so slice 3's start waits for it (section 8). The revision of 2026-11-27 answers the architecture
review of the third revision (`docs/designs/042-stream-operators.reviews/2026-11-26/round-1/architecture.md`): every
emission of a stateful chain is a window, so the apply never creates a run (FR-042-014); the start writes its window
in the run's transaction, in ADR-0050's slots, with one outcome per answer of 022's port (FR-042-019); the trigger rules
get a pure domain module and a store port (section 1); the HMAC key is used through one definer function (section 4);
and the chain lifecycle follows every change of a trigger row.

Slice 0 and the server half of slice 1 are built (research Decisions 042-100 to 042-160); the SDK half of slice 1 is
open (Gaps 042-113 and 042-131); slices 2 to 5 are not built. Tasks stay unticked until the design is approved, as the
rules of `docs/designs/README.md` require. The registers are in `specs/042-stream-operators/research.md`.

A word on names: "Streams" already means the workflow Streams of feature 026 (ADR-0068). To keep the two apart, the
tables and types of this design say *operator chain*, never *stream*.

## The Problem

**What exists on the collection side** (feature 014, `docs/runbooks/workflow-primitives.md`). The collection entry of the
scheduler (`crates/scheduler/src/passes.rs`, from about line 1094) reads the run data under the fence, drops that
transaction, evaluates `items` through `ExpressionEvaluator::collection_source`, and then:

- for an inline list, stores the whole membership as one snapshot payload, refused above `MAX_PAYLOAD_BYTES` (256 KiB)
  with `RUN_DATA_LIMIT`, and refuses more elements than the node's `maxItems` with `ITERATION_LIMIT`;
- for a blob reference, materialises the blob bounded by the node's `maxItems` (`canonical_items`,
  `crates/application/src/collection_source.rs`, about line 304: 8 MiB blob, 256 KiB per item, 4 MiB in all) and stores a
  `SourceManifest` (blob, count, items digest) as the snapshot; `enter_collection` and the restore in
  `crates/infra-postgres/src/primitives.rs` (about lines 1571-1583 and 1430-1435) require the manifest's count and digest
  to equal the stored items.

`enter_collection` then opens its own transaction and checks the scheduler fence. A failing `items` expression stops the
run with `EXPRESSION_ERROR` through `stop_with_code_routing(..., None)`: a collection's `onError` is not taken at entry
(only an unentered loop's is). `map` is an SDK alias of the same `forEach` node. There is no way to drop elements, flatten
a list of lists, remove duplicates or run the body on groups of N; an author does it in an activity before the
collection, which costs a worker round trip and puts list shaping in code the Studio cannot draw.

**What exists on the event side** (feature 004, `docs/runbooks/events.md`). An event is admitted into `inbox_events`
under `(tenant, namespace, source, event_id)`, with `received_at = clock_timestamp()` at insert and an optional
provider-set `occurred_at`. The dispatcher (`crates/scheduler/src/events.rs`, `EventDispatcher::tick`) claims pending
events and, for each trigger of `workflow_triggers` that matches source and type, evaluates the trigger's one `filter`
and starts one run under the idempotency scope `trigger:{id}` and the key `start_key` derives from the admission
(`crates/application/src/dispatch.rs`). `event_deliveries` records that a consumer received an event, once, and is
deleted with the inbox row by its foreign key. So today one event is one run; deduplication is by event id only
(ADR-0012); nothing is ordered (gap G74 of feature 004).

**What bounds any answer.**

- The expression language (`crates/infra-expr`, used through `JsonEvaluator` in `crates/infra-ir/src/evaluator.rs`) is a
  bounded CEL subset: comparison, arithmetic, boolean operators, the ternary, field access, indexing, list literals,
  `has`, `size`, `string`, `int`, `double`, `contains`, `startsWith`, `endsWith`. No object literal, macro, regular
  expression, hash, clock or randomness. Limits: 1,024 bytes of source, 256 tokens, depth 16, 4,096 constructed list
  elements, 64 KiB strings (frozen by ADR-0032, decision 7). It is deterministic and runs in the engine process. A
  projection can select a value or compute a scalar or a list; it cannot build an object (G042-001).
- `JsonEvaluator`'s `environment` (lines 33-49) parses the run input, every step output of the path and the state from
  bytes each time an expression is evaluated. Evaluating a stage once per element through the present port would parse
  the run's data once per element.
- The IR schema is closed and a stored definition is validated again when read (`parse_stored`, `crates/infra-ir/src/
  lib.rs` lines 144-150). Both `parse` and `parse_stored` reach `convert::published_collection_definition` (lines
  262-271). A check placed in `convert::*` therefore runs on read as well as on publication.
- The scheduler is sharded by a fence (ADR-0020). The event dispatcher is not shard-fenced; it holds a 60-second claim
  per event, and claiming does not increase `attempts` (`crates/infra-postgres/src/dispatch.rs`, about line 227).
- `payloads` is append-only: a trigger refuses `UPDATE` and `DELETE` (`20261009000100_execution.up.sql`, lines 242-245).
- Feature 022 owns *when a requested run may start*: concurrency by key and, in its 0.3.0 slice, a rate limit per key
  whose excess is buffered durably (US07, ADR-0088). Feature 018 owns schedules and their overlap policies. This design
  must not grow a second rate limiter or a second buffer of runs.

## Requirements

**Vocabulary and language**

- **FR-042-001** The operator set is closed and named in the IR schema: `filter`, `map`, `flatMap`, `skip`, `take`,
  `distinct`, `window`, `debounce`, `ignoreElements` (the last two on triggers only). Release 0.3.0 has the seven that a
  collection has. A definition that uses any other name, or an operator on a surface that does not have it, MUST be
  refused at publication with a typed error that names the node or trigger and the stage index.
- **FR-042-002** Every predicate, key and projection MUST be an expression of the existing bounded language, validated
  at publication against the existing limits and evaluated by the engine. No operator takes code, a script reference or
  an activity. The language is not extended by this design.
- **FR-042-003** The declarations are additive fields of IR `sagat-octopus-orchestrator.io/v1`. The codec always decodes
  them; whether a deployment may publish them is checked by the definition service at publication, as `lake.enabled`
  is (`crates/application/src/definitions.rs`, `with_lake_enabled` and `refused_lake_node`), under two switches,
  `collection-operators` (slice 1) and `trigger-operators` (its fields enter the schema with slice 2), each also a
  capability of the instance registry that the definition service requires before it publishes (ADR-0153). A trigger
  chain whose emission is a window (`window`, `debounce`, a keyed stage or `flatMap`) on a version that declares an
  `inputSchema` is refused at publication (`OPERATOR_INPUT_SCHEMA`) unless that schema accepts the window envelope of
  FR-042-019, checked against an emission with an empty `events` list and the summary.
  A switch that is off MUST refuse publication and MUST NOT make a stored definition unreadable. A binary that predates
  a field refuses the stored document on read; the fleet rule of section 10 keeps such a binary out of a deployment
  that has published one.

**Collections (release 0.3.0)**

- **FR-042-004** A `forEach` MAY declare `operators`, one to eight stages in any order, each over the list the previous
  stage produced: `filter {where}` keeps the elements for which `where` is `true` (any other value, `false` included,
  drops it; a non-boolean is an error); `map {select}` replaces each element by the value of `select`; `flatMap {select}`
  requires `select` to yield a list per element and concatenates those lists in order, one level; `skip {count}` drops
  the first `count` (0 to 1,000); `take {count}` keeps the first `count` (0 to 1,000); `distinct {by?}` keeps the first
  occurrence of each canonical value of `by` (of the element when absent); `window {count}` replaces the list by
  consecutive lists of `count` elements (1 to 1,000; 0 is refused at publication), the last one possibly shorter. An
  empty output is allowed and gives an empty collection, as an empty `items` does today.
- **FR-042-005** Stage expressions see `element` (the current element of the stage's input), `position` (its 0-based
  index in that input) and the variables `items` sees (`input`, `steps`, `run`, `state`). A stage expression that names
  `item`, `index`, `iteration`, `carry` or `bodyResult` MUST be refused at publication, so the names stay free for an
  enclosing iteration when nesting exists (design 039). The body sees, as today, `item` and `index`, where `index` is the
  element's position in the chain's *output*: after a `filter` or a `skip` the body's indices are 0 to n-1 with no gap.
- **FR-042-006** The chain MUST be evaluated by one call of the evaluator port that builds the run's environment once
  (input, step outputs, state parsed once) and applies every stage, off the async runtime (a blocking task) under a
  cooperative wall-time budget of 1 second checked between elements. It MUST evaluate stage by stage: every element of a
  stage's input before the next stage starts. An error MUST name the first failing stage, then the first failing
  position in it, with the operator name, and never a value.
- **FR-042-007** Ceilings MUST be enforced while a stage's output is produced, failing at the first crossing:
  (a) with operators present, the chain's input (the list `items` yields, or the elements a blob source holds) MUST NOT
  have more than 1,000 elements, the ceiling published for collections in 0.3.0 (OD-2026-11-13-1); a blob source is
  materialised with that bound instead of the node's `maxItems`; (b) no stage output MUST hold more than 1,000
  elements; (c) no element MUST serialise to more than 256 KiB (`MAX_PAYLOAD_BYTES`); (d) no stage output MUST
  serialise to more than 4 MiB in all (`MAX_RUN_OUTPUT_BYTES`); (e) the wall-time budget of FR-042-006. With these the
  evaluations are at most stages times 1,000. Each ceiling maps to exactly one code (table in section 3).
- **FR-042-008** A chain that fails at entry (a ceiling, an expression error, a value of the wrong shape) MUST fail the
  run with the typed code through the path a failing `items` takes today; the collection's `onError` is not taken
  (G042-008). Nothing is truncated silently.
- **FR-042-009** The chain's output MUST be recorded once, by the transaction that enters the scope, as the membership
  the snapshot and its digest describe. An evaluation whose entry does not commit (the fence moved, the revision
  changed, the blob source was unavailable, storage quota made the entry wait) is discarded; the next entry evaluates
  again over the same immutable run data and gets the same output. Once the entry has committed nothing evaluates the
  chain again: not a replay, a retry, a restart, a reconciliation or a read. No journal event type is added.
- **FR-042-010** For a blob source with operators the snapshot MUST be a chained manifest recording the blob, the
  source count and digest, the chain's digest, and the output count and digest; `enter_collection` and the restore MUST
  check the stored items against the output count and digest. An inline source's output is stored as today's snapshot
  array, under today's 256 KiB snapshot ceiling.
- **FR-042-011** `distinct` MUST compare canonical values: an integral double within plus or minus 2^53 is written as
  the integer, `-0` as `0`, object keys sorted by code point, whatever the `serde_json` features of the build. Two
  values `==` calls equal within that range have the same canonical form.
- **FR-042-012** A `forEach` without `operators` MUST behave and cost as it does today (same statements, same
  snapshot bytes), held by `collection_entry_without_operators_is_unchanged` in `integration_workflow_primitives`.

**Triggers (release 0.3.0, slices 2 to 5)**

- **FR-042-013** A trigger of kind `event`, `webhook` or `message` MAY declare `key` (an expression that yields a scalar
  of at most 512 bytes; absent means one key for the whole trigger) and `operators`, one to eight stages in canonical
  order: any number of `filter` and `map`; then at most one each of `distinct`, `skip`, `take`; then at most one of
  `debounce` or `window`; then at most one of `flatMap` or `ignoreElements`. A chain out of order is refused at
  publication (`OPERATOR_ORDER`), as is a chain that can never emit (`OPERATOR_NEVER_EMITS`: `ignoreElements` without
  `debounce` or `window`; `take {count: 0}`). The existing `filter` field of a trigger keeps its meaning and runs first.
  Stage expressions see `element`, `position` is not available (a trigger stage sees one event), and the event
  document `event` and `data` as filters do today.
- **FR-042-014** An admitted event MUST be applied to a trigger's chain at most once. A chain of only `filter` and `map`
  is applied as a trigger is today: its stages run before the start with no lock and no state, and the start writes the
  `event_deliveries` row in the run's transaction under `start_key`, so only the input changes. Every other chain (one
  with a keyed stage, `window`, `debounce` or `flatMap`) is applied in one transaction that locks the chain row, then
  (when the chain has a keyed or windowed stage) the key row, applies the stages, writes each emission as an
  `operator_windows` row (a chain with `window` or `debounce` gathers into open windows; any other chain's emission is a
  window sealed at once with its one member), and inserts the `event_deliveries` row for that trigger *with its final
  outcome* (`HELD`, `SUPPRESSED`, or `FAILED` with its code); a conflict on that insert rolls the transaction back and
  changes nothing. The apply never creates a run: the window start does (FR-042-019). A delivery row is never updated;
  it is deleted with the inbox row as today, or by the retry action of FR-042-021. A chain of only `filter`, `map` and
  `flatMap` takes no key row and no key lock.
- **FR-042-015** Keyed operators hold their state per chain and key. `distinct {by, horizonMs}` passes an element whose
  canonical `by` value has not passed for this key within `horizonMs`; `skip {count}` drops the key's first `count`
  elements; `take {count}` passes the key's first `count`. Counts belong to the key's *epoch*, which ends when the key
  has had no element for `keyIdleMs` (required with `skip` or `take`). An expression that fails drops the element with
  outcome `FAILED` and the code; it never passes it. `distinct` marks are rows of their own and do not hold a key slot.
- **FR-042-016** `window` gathers a key's passing elements. Kinds: `tumbling {sizeMs}` (boundaries aligned to the epoch of
  the database clock), `session {gapMs, maxWaitMs}` and `count {count, maxWaitMs}`. A key MAY have several open
  windows, told apart by the instant they opened. An element joins the open window whose span covers its `received_at`;
  a session window's deadline moves to `received_at + gapMs` (bounded by `opened_at + maxWaitMs`) only while that
  deadline has not elapsed on the database clock; an element received after an elapsed deadline opens a new window.
  `emit` is `all` (the elements ordered by `(received_at, source, event_id)`), `first` or `last` (the least or greatest
  element in that order). `debounce {quietMs, maxWaitMs}` is `window session` with `emit: last`. `ignoreElements`
  after either makes the emission carry only the summary, and then no element value is stored.
- **FR-042-017** Time is the database clock; an element's time is its `received_at`; `occurred_at` drives no timer. A
  window whose deadline `D` has passed MUST NOT be sealed before every transaction that was running when the pass first
  saw `D` passed has ended: the pass records `seal_horizon = pg_snapshot_xmax(pg_current_snapshot())` on the window and
  seals only once `pg_snapshot_xmin(pg_current_snapshot()) >= seal_horizon`, and the admission statement takes its
  transaction id before it evaluates `received_at` (an event's `received_at` is taken at insert and becomes visible at
  commit; ADR-0153, which replaces the first draft's `admissionGraceMs`). After that it MUST NOT be sealed while the tenant and namespace hold a pending event with
  `attempts = 0` that the trigger could match and that was received at or before `D`, unless `D + maxHoldMs` has
  passed (`maxHoldMs` 60 s by default, at most 10 minutes), because claiming does not increase `attempts`. An element
  applied after the window that covered it was sealed is *late*: it joins an open window or opens one, flagged and
  counted, and is never dropped for being late.
- **FR-042-018** Windows are sealed by a scheduler pass fenced as ADR-0020 prescribes, which locks the chain row, then
  the key row, then the window. Sealing and applying are ordered by the key lock; whichever commits first wins and the
  other re-reads (a seal that finds the deadline moved does nothing and is counted). A sealed window's membership never
  changes.
- **FR-042-019** A sealed window starts exactly one run (one per element after `flatMap`, FR-042-020), through the one
  admission port of feature 022 (FR-022-018, FR-022-041): the idempotency scope is the trigger's and the key is derived
  from the window's recorded identity (namespace, chain, key HMAC, the instant it opened, its sequence), with a
  fingerprint of its membership. The run starts on the workflow version whose chain gathered the window. Its input is
  `{window: {keyId, sequence, openedAt, sealedAt, count, firstReceivedAt, lastReceivedAt, late, overflow}, events: [...]}`
  for `emit: all`, the element with the summary for `first` and `last`, and the summary alone after `ignoreElements`;
  publication checks that the version's `inputSchema`, when it has one, accepts that shape (FR-042-003,
  `OPERATOR_INPUT_SCHEMA`). The start is one transaction: the port takes its receipt and creates the run, and in the
  run's open transaction the start sets the window `STARTED` with the run id and deletes its held values. Each outcome
  of the port has one window outcome:

  | Port outcome (feature 022, as on its branch on 2026-11-26; re-checked against the merged port by Task 042-22) | Window |
  |---|---|
  | The run is created, buffered by 022 or not, or created already `FAILED` for a failed key | `STARTED` (a run exists) |
  | The request's key is found (a replay) | `STARTED` with the replayed run id |
  | `CONCURRENCY_QUEUE_FULL`, `FLOW_CONTROL_LIMIT`, `BinaryCapabilityMissing`, a quota refusal, any transient error | stays `SEALED`; retried with a back-off under the same key |
  | The run-input check of design 043 refuses the input | `REFUSED` with `RUN_INPUT_INVALID`, audited, held values deleted |
  | The version can no longer run (removed or not runnable) | `REFUSED` with its code, audited, held values deleted |
  | Still `SEALED` 24 hours after it was sealed | `REFUSED` with `START_EXPIRED`, audited; held values kept until the tenant's event retention or a discard |

  Only a `START_EXPIRED` window is retryable (back to `SEALED`, by an operator, once the cause is fixed); the other
  refusals cannot be changed by a retry, since the version and its schema are pinned. An operator may discard any
  `REFUSED` window. A chain of only `filter` and `map` keeps today's
  `start_key`; a window sealed at once is started by the dispatcher right after its apply commits, in a second
  transaction with the same code as the pass, and by the pass when that start does not happen.
- **FR-042-020** `flatMap {select, maxFanOut}` on a trigger starts one run per element of the list `select` yields for an
  emission, in order, under the window's key suffixed with the index, one start transaction per index (FR-042-019); the
  window records `started_count`, becomes `STARTED` when every index has its run and deletes its held values then; a
  crash between two indexes resumes at the next one, and the port's replay answers an index already created. More than
  `maxFanOut` (at most 100) elements, or a value that is not a list, fails the emission as a whole.
- **FR-042-021** Ceilings each have one outcome. A window reaching `maxCount` (at most 1,000) or its byte ceiling (the
  run input ceiling inline, 4 MiB as an input blob) follows `onFull`: `emit` (seal now; the default) or `drop` (the
  element is dropped with outcome `SUPPRESSED`, reason `window_full`). A new key beyond the chain's `maxKeys` ends that
  event's delivery to that trigger at once with outcome `FAILED` and code `KEY_LIMIT`: it is not retried by the
  dispatcher, it does not hold the event pending, it is audited once per chain and minute with a count, and it can be
  applied again by an operator through the delivery's retry action. The retry is one transaction: it locks the inbox
  row `FOR UPDATE` (absent, because the dedupe window ended and the row was purged with its deliveries, the action
  fails with `DELIVERY_EVENT_PURGED`, `409`, not retryable), deletes that trigger's delivery row, and returns the event
  to `PENDING` with its attempt count unchanged; the dispatcher then offers it to every trigger again, the others find
  their delivery rows and change nothing, and this trigger applies it once more (an audit record `operator.delivery_retried`). `distinct` marks are bounded per chain by
  `maxMarks` (100,000 by default and at most; the sum over a tenant's chains at most 1,000,000); a mark that would pass
  the ceiling fails the element with `KEY_LIMIT` as well. Held bytes are bounded per tenant by a quota of their own,
  `operator_held_bytes`, 1 GiB (1,073,741,824 bytes) by default, which an operator may change like any quota: an element
  that would pass it fails with `QUOTA_EXCEEDED` and holds nothing. The sum of `maxKeys`, the sum of `maxMarks` and the
  number of chains are tenant quotas checked at publication. Window durations are 1 second to 24 hours; a `distinct` horizon is at most 7 days.
- **FR-042-022** Chain state is purged by a bounded pass that locks the chain row first: a key row with no open window
  whose epoch has ended; a mark at its horizon; a started, refused or discarded window after the tenant's event
  retention. When a workflow is republished with a different chain, the old chain takes no new element and its open
  windows drain at their own deadlines onto the version that gathered them; when the trigger is removed, its open
  windows are discarded, each audited and counted.
- **FR-042-023** Every row carries tenant and namespace and every statement is scoped by them. Listing chains and
  windows needs `event:read`; `flush` and `retry` need `run:start` and `run:operate`, `discard` needs `run:operate`; the
  caller of a flush or a retry is recorded as the starter of the run it causes. Every such act is audited. Another
  tenant's chain or window is indistinguishable from a missing one and the attempt is audited.
- **FR-042-024** Held element values, key text and `distinct` values are event data, personal data as a whole for
  feature 016. Element values are held in a column of the chain's own table, classified as content, counted in the
  tenant's retained bytes, and deleted in the transaction that starts the window's run (or discards it); they never go
  to the append-only `payloads` table. Keys and `distinct` values are stored as HMAC-SHA-256 under a per-tenant key
  that never leaves the database role; the API names a key and a window by random surrogate ids, never by a digest, and
  shows the key text only to a caller who may reveal personal data. None of these values appears in a log, a metric, a
  span attribute, an audit record or an error.

**Surfaces**

- **FR-042-025** The collection chain counts elements per operator and outcome in
  `operator_elements_total{surface="collection", operator, outcome}`; its failure codes are members of
  `WorkflowPrimitiveRejectionCode::ALL` (`crates/telemetry/src/metrics.rs`) and of the run failure codes of
  `events.asyncapi.yaml` and `docs/sdk-compatibility.md`. For triggers: an event's delivery says `STARTED`, `HELD`,
  `SUPPRESSED` or `FAILED` with a code, the pass has a lag metric, and every label is a closed enumeration.
- **FR-042-026** The management API lists a namespace's chains and a chain's windows (state, key id, count, deadline,
  the events it holds by reference), and flushes, discards or retries one window.
- **FR-042-027** The Studio edits a trigger's key and chain as an ordered list of typed stages with the publication
  rules applied as the author types, shows open windows with their deadline and the pass's lag, and shows on an event's
  page where it is held. Collection operators are shown on a `forEach` node once the Studio knows that node (G042-004).
- **FR-042-028** The six SDKs build both declarations with one set of names from an operator factory (`ops.filter(...)`
  in each language's casing), carried by each SDK's collection options without breaking a present caller, emit
  identical IR for the same chain, and pass common conformance scenarios.
- **FR-042-029** A chain decides *which events ask for a run and with what input*; features 022 and 018 decide *when a
  requested run may start*. No operator declares a rate or a limit (section 8).
- **FR-042-030** A trigger without `key` and `operators` MUST cost what it costs today: no added statement, row or lock,
  held by a budget test.

**Success criteria**

- **SC-042-001** 500 events for one key inside one quiet period start exactly one run whose input is the event with the
  greatest `(received_at, source, event_id)`, when every event is dispatched twice and the process is killed once at
  each write of the apply, seal and start paths.
- **SC-042-002** The seal and an arriving element are forced in both orders with a held lock, and each order ends in the
  state FR-042-018 states, with no element lost or in two windows.
- **SC-042-003** One collection chain per operator, and one of eight stages, produce the same snapshot digest from the
  six SDKs. After the entry has committed, the engine's evaluator is replaced by one that panics on the chain call and
  the scheduler is restarted: the run completes on the recorded membership with zero chain evaluations, for an inline
  and for a blob source (the blob deleted from the store after entry), and `verify_run` passes.
- **SC-042-004** For each ceiling of FR-042-007 one test crosses it by one and fails the run with its code, and one test
  stays exactly at it and succeeds; an empty output succeeds with an empty result.
- **SC-042-005** Cancellation and entry are forced in both orders with a held lock, and a shard lease is taken away while
  the chain is held at a barrier inside the evaluator: each order ends with the run cancelled or entered exactly once,
  and the stale entry records nothing.
- **SC-042-006** 50,000 events with 50,000 distinct keys against a chain of `maxKeys` 1,000: 1,000 keys are served, the
  rest end `FAILED`/`KEY_LIMIT` at once, no event stays pending for it, another tenant's dispatch latency stays within
  the stated bound, and no table grows beyond its ceiling.
- **SC-042-007** A restart with 1,000 open windows, some past their deadline and some with pending events behind them,
  ends with every window started once and every event in exactly one window.
- **SC-042-008** The budgets of NFR-042-001 hold on the stated environment.

## Goals

- An author shapes a collection's list in the definition (keep, project, flatten, deduplicate, chunk) without an
  activity, in release 0.3.0, with every failure typed and nothing re-evaluated after entry.
- With slices 2 to 5, also in release 0.3.0 (Owner decision 2026-11-24-1), an author says "one run per burst per
  order" in the definition, and the platform keeps that promise across duplicate delivery, a crash and a restart.
- The same words mean the same thing in six SDKs, in the Studio and on both surfaces.

## Non-Goals

- **No user code in an operator**, and no extension of the expression language (decided, O042-005).
- **No rate limit and no concurrency limit.** `throttle`, and anything that paces run starts, is feature 022's
  (`concurrency`, and `rateLimit` with buffering, US07 and ADR-0088). This design neither admits nor queues a run.
- **No trigger operator in slices 0 and 1.** O042-002 kept trigger operators out of release 0.3.0; Owner decision
  2026-11-24-1 brought slices 2 to 5 into it, after slices 0 and 1.
- **No `onError` routing of a collection that fails at entry** in 0.3.0 (G042-008).
- **No event-time semantics** (decided, O042-004). No watermark, no allowed lateness, no out-of-order correction.
- **No ordering guarantee between keys or between triggers.**
- **No operators on `waitEvent`** in the first delivery (decided, O042-003; `filter` on a wait is a follow-up), none
  on outbound subscriptions, none on connector admission (feature 021's FR-021-029 filters there, before the inbox).
- **No multi-stream operators** and no accumulators (`scan`, `reduce`).
- **No streaming or unbounded iterators** in collections (feature 014 rules them out); nesting stays in feature 039.
- No change to the inbox key, the dedupe window, the dead-letter rule or `start_key` for a chain without a window.

## Open Questions

The owner questions are answered (next section). Technical questions, registered as gaps:

- **G042-001** `map` cannot build an object, only select or compute. A stated limit: the owner kept the language
  (O042-005).
- **G042-002** Whether the seal and the run start can share one transaction through the admission port, or must be two
  with the window `SEALED` between them. Closed 2026-11-26: two transactions (Decision 042-162, section 8).
- **G042-003** `published_collection_definition` refuses `triggers` on a workflow with a `forEach`, so a window cannot
  yet feed a collection; that gate belongs to design 039, which takes it over (Gap 039-006 there).
- **G042-004** The Studio's node registry does not know `forEach`, so collection operators have no canvas yet.
- **G042-005** The dispatcher is one queue for all tenants (G88 of feature 004); a key flood is bounded per event, not
  isolated. SC-042-006 measures it.
- **G042-006** Nothing enforces the fleet rules of section 10 until the instance registry (ADR-0082) can. Narrowed
  2026-11-26 and reopened 2026-11-27: the registry carries both capabilities (ADR-0153, Task 042-54) and keeps a
  registered process that lacks one from serving, but a binary from before the registry does not register, so for it
  the runbook's upgrade rule is the only guard; an older dispatcher in a mixed fleet would start one run per event.
- **G042-007** One hot key is serial by construction (one row lock per element). The rate one key sustains is reported.
- **G042-008** A collection that fails at entry stops the run; its `onError` is not taken, as for a failing `items`
  today. Routing entry failures to `onError` is a follow-up of its own for feature 014's collections, with or without
  operators.
- **G042-009** The wall-time budget is the one ceiling that depends on the machine: on a starved replica it could fail a
  chain another replica would finish. The deterministic ceilings bound the work so that it should never fire; when it
  does it is counted separately (`reason="wall_time"`) and treated as a defect to explain.
- **G042-010** Beyond plus or minus 2^53 an integer and a double that `==` calls equal (it compares through `f64`) may
  have different canonical forms, so `distinct` keeps both.
- **G042-011** Feature 022's D022-018 says a window whose run 022 buffers "stays sealed". This design records it
  `STARTED`, because the run exists (FR-042-019); the 022 text is to be aligned (T042-3).
- **G042-012** The back-off of a collection entry that waits for storage quota is a per-replica hint (section 3); a
  replica that loses it evaluates the chain once more, which costs time and changes no outcome.
- **G042-013** `take` after `flatMap` does not rescue a `flatMap` whose output crosses a ceiling: each stage's output is
  materialised and bounded before the next stage runs (FR-042-006).
- **G042-014** The chain input ceiling (1,000) is below the 10,000 elements validation accepts for `maxItems`, so a
  `filter` cannot reduce a blob of 5,000 elements to 500. Raising it waits for the measurement of T014-042 that
  OD-2026-11-13-1 deferred.

## Owner decisions needed

**Answered.** The owner answered O042-001 to O042-009 on 2026-11-13 with "Aceitar todas": each recommendation of the
first draft is the decision, recorded as OD-2026-11-13-15, and accepted ADR-0086 and ADR-0087 with "Aceito tudo",
recorded as OD-2026-11-13-17. Both rows are added to `docs/owner-decisions.md` on `develop` by the owner's session, not by
this branch. Tasks that depend on an answer carry `Owner-gate: OD-2026-11-13-15`.

| Question | Decision (OD-2026-11-13-15) | Options not taken |
|---|---|---|
| O042-001 Which surface the sentence means | (c) Both: triggers and `forEach`, one vocabulary, two switches | (a) events only; (b) collections only |
| O042-002 What goes in release 0.3.0 | (b) The collection operators only: slices 0 and 1; slices 2 to 5 a later release. Widened by the owner on 2026-11-24 to (d), all of them (Owner decision 2026-11-24-1) | (a) nothing; (c) collections and trigger debounce |
| O042-003 Operators on `waitEvent` | (a) Not in the first delivery; `filter` on a wait as a follow-up | (b) `filter` now; (c) the whole chain |
| O042-004 The time base | (a) Arrival time on the database clock only | (b) also event time with an allowed lateness |
| O042-005 Object construction in `map` | (a) The language stays as it is; `map` selects or computes | (b) an object literal |
| O042-006 The first set and what is deferred | The nine operators of FR-042-001; the deferred list of the catalogue | — |
| O042-007 The numbers | Chain length 8; `maxCount` 1,000; window durations 1 s to 24 h; `maxKeys` 10,000 per chain; 50 chains and 100,000 keys per tenant; `maxFanOut` 100; `distinct` horizon up to 7 days; published limits a measurement may raise | — |
| O042-008 Showing a key | (a) Not the key, unless the caller may reveal personal data | (b) the key to any `event:read` holder |
| O042-009 A new key beyond `maxKeys` | (a) Fail the delivery, visible and replayable | (b) a run per event; (c) drop |

**Decided on 2026-11-26 under the owner's delegation** (Owner decision 2026-11-24-4), after the re-read of Task 042-44;
each is a delegate row of `docs/owner-decisions.md` with its reason:

| Item | Decision | Why |
|---|---|---|
| The refinement of O042-009 (Owner decision 2026-11-26-130) | A new key beyond `maxKeys` ends that event's delivery to that trigger at once with `FAILED`/`KEY_LIMIT`, visible, retryable by an operator; no dispatcher retry, no dead letter | A retry would hold the event pending for the dispatcher's whole schedule and every window of the namespace behind the backlog rule; a dead letter would take the event from its other triggers |
| The refinement of O042-008 (Owner decision 2026-11-26-131) | The API names a key by a random surrogate id and shows its text only to a caller who may reveal personal data; no digest is shown | A digest of a low-entropy key (an order number) is reversed by trying values |
| `maxMarks` (Owner decision 2026-11-26-132) | 100,000 per chain by default and at most; at most 1,000,000 over a tenant's chains | Ten marks per key at the key ceiling of 10,000; a mark row takes about 250 bytes with its index, so a full tenant holds about 250 MB of marks |
| The held-bytes quota (Owner decision 2026-11-26-133) | `operator_held_bytes`, 1 GiB per tenant by default | 256 windows at their 4 MiB ceiling; an operator raises it like any quota |
| `maxHoldMs` (Owner decision 2026-11-26-134) | 60 seconds by default, at most 10 minutes, per chain | Long enough for a dispatcher restart, short enough that a stuck event delays a burst's run by a minute |
| `admissionGraceMs` (Owner decision 2026-11-26-135) | Withdrawn: a window is sealed past the transaction horizon of its deadline (ADR-0153, Owner decision 2026-11-26-136) | On PostgreSQL 16 no timeout bounds an admission transaction, so no fixed grace is a proof; the horizon is exact and needs no tuning |

**Nothing is still needed from the owner.** This design is `Approved` by review (constitution, Approval), not by an
owner decision.

## Critical User Journeys (CUJs)

### CUJ 1: A collection shaped in the definition (release 0.3.0)

*   **Given**: `forEach {items: "steps.fetch.output.orders", operators: [{filter: {where: "has(element.total) &&
    element.total > 0"}}, {distinct: {by: "element.customerId"}}, {window: {count: 50}}], maxItems: 100, …}` over 900
    small orders of 600 customers, 700 of them with a positive total.
*   **When**: the scope is entered, and the scheduler restarts before the first iteration.
*   **Then**: the snapshot holds the chunks of the first order of each customer with a positive total, 50 per chunk; the
    body sees them as `item` with `index` 0, 1, 2 …; after the restart the body runs on the same chunks without the
    chain being evaluated again; results come back in that order.

### CUJ 2: A chain that cannot fit (release 0.3.0)

*   **Given**: `operators: [{map: {select: "steps.big.output"}}]` over 1,000 elements, where `steps.big.output` is 200 KiB.
*   **When**: the scope is entered.
*   **Then**: the twenty-first element brings the stage past 4 MiB; the chain stops there, the run fails with
    `COLLECTION_OPERATOR_BUDGET`, the failure detail says stage 0, `map`, position 20, and no value appears anywhere.

### CUJ 3: One run per burst per order (slices 2 to 5)

*   **Given**: a trigger `{kind: webhook, endpoint: shop, eventType: order.updated, key: "data.orderId", operators:
    [{debounce: {quietMs: 30000, maxWaitMs: 300000}}]}` and a provider that sends twelve updates for order 81 in three
    seconds, two of them twice.
*   **When**: the events are admitted, and one scheduler replica is killed while applying the seventh.
*   **Then**: the inbox absorbs the two duplicates; ten deliveries say `HELD`; thirty seconds after the last arrival,
    once the transactions running at the deadline have ended, the pass seals the window and one run starts with the update of the greatest receipt as
    input and `window.count` 10. Each update's event page names the window by its id, and the window names the run.

### CUJ 4: A batch every five minutes (slices 2 to 5)

*   **Given**: a `message` trigger with `operators: [{filter: {where: "data.kind == 'reading'"}}, {map: {select:
    "data.value"}}, {window: {kind: tumbling, sizeMs: 300000, maxCount: 1000, emit: all}}]` and no key.
*   **When**: 2,400 readings arrive in one five-minute interval.
*   **Then**: three runs start, two with 1,000 values (sealed by `onFull: emit`, `window.overflow` true) and one with
    400 at the boundary. Each is admitted by feature 022's rules like any other start.

### CUJ 5: A flood of keys (slices 2 to 5)

*   **Given**: a chain with `maxKeys` 1,000 and a caller that sends events with a random key each.
*   **When**: the thousand-and-first key arrives.
*   **Then**: that event's delivery to the trigger ends `FAILED`/`KEY_LIMIT` at once and the event leaves the backlog;
    the thousand keys keep their windows; one audit record per minute counts the refusals; other tenants' events are
    still dispatched.

## Technical Plan (Architecture)

### 1. Declaration and validation

Two optional fields are added to `specs/001-platform-foundation/contracts/workflow.schema.json`: `operators` on the
`forEach` node in 0.3.0, and `key` and `operators` on a trigger item with slice 2. A stage is an object with exactly one
property, the operator name, whose value is that operator's closed parameter object with its ranges. The domain gains
`OperatorChain` and `Operator` (in `crates/domain`, beside `workflow.rs` and `triggers.rs`): pure types and the rules
that need no I/O (surface, ranges, chain length, canonical order on triggers, "can emit"). `crates/infra-ir` decodes them
on every path and checks each expression against the present limits and the variable rule of FR-042-005. A chain has a
canonical encoding and a `chain_digest`.

The trigger side (slices 2 and 3) places its business rules as Principle II asks: a pure module
`crates/domain/src/operators/window.rs` decides every transition from the rows it is given and the database time it is
told (which window an element joins or opens, a session deadline that moves, `onFull`, a late element, the epochs of
`skip` and `take`, the horizon of `distinct`, whether a due window may seal), with property tests over generated
element sequences and no I/O, clock or SQL; `ExpressionEvaluator` gains one method, `trigger_stage(stage, element,
event)`, for a trigger chain's stage, beside the collection method of section 3; and `crates/application/src/operators.rs`
holds the store port `OperatorChainStore` (apply in the dispatcher's transaction, due windows, seal, start, purge,
retry) that the dispatcher and the pass call and `crates/infra-postgres` implements. No rule lives in SQL beyond the
statements that read and write the rows the module decided.

**The chain lifecycle follows every change of a trigger row, not only publication.** Activation, rollback and promotion
rewrite `workflow_triggers` through `triggers_of` (`crates/application/src/publishing.rs`), so the registration and
draining of chains, the tenant quotas of FR-042-021 and the capability requirement of section 10 run in `switch` as well
as at publication; a rollback onto an older chained version registers its chains (or is refused by a quota or the
requirement) and drains the newer ones, tested. The publication switch itself is still read only by a new publication
(below).

**The switches are checked at publication only.** `DefinitionService` gains `with_collection_operators_enabled` and
(with slice 2) `with_trigger_operators_enabled`, beside `with_lake_enabled`, and refuses a definition that uses a switched-off
field with `IrError::CollectionOperatorsDisabled { node }` (and its trigger twin), in the same place `refused_lake_node`
is checked. Nothing in `convert::*` looks at a switch, because `parse_stored` goes through it: switching a capability off
stops new publications and never makes a stored definition unreadable (tested through `parse_stored` with the switch
off). Amended after the slice 0 review (D042-105, D042-106): only a new publication reads a switch. A publication asked again
with its own idempotency key is answered by the store's replay before the switch is read, and activating, rolling back
or promoting to a namespace that already runs the document move a pointer to an existing version and never read it.
Until T042-8 applies a chain at entry, the control plane refuses to start with `collection-operators` on, and the
scheduler fails a chained `forEach` closed at entry with `COLLECTION_OPERATORS_UNSUPPORTED` (D042-103, G042-100).

**Old readers.** The schema is closed, so a binary without the field fails `parse_stored` with `IrError::Schema`: it
refuses, it does not ignore. That is why section 10 forbids a mixed fleet.

### 2. The catalogue

| Operator | Trigger (slices 2 to 5) | Collection (0.3.0) | State | Emits |
|---|---|---|---|---|
| `filter {where}` | yes | yes | none | the element, when `where` is `true` |
| `map {select}` | yes | yes | none | the value of `select` |
| `flatMap {select}` | yes, last stage, fan-out of runs | yes, flattens one level | none | each element of the list |
| `skip {count}` | yes, per key epoch | yes, 0 to 1,000 | a counter per key | elements after the first `count` |
| `take {count}` | yes, per key epoch | yes, 0 to 1,000 | a counter per key | the first `count` elements |
| `distinct {by, horizonMs}` | yes | yes (no horizon) | a mark per value | the first occurrence |
| `window` | tumbling, session, count | `{count}` 1 to 1,000 | windows and their members | a list, or first or last |
| `debounce {quietMs, maxWaitMs}` | yes | no | a window holding one element | the greatest element |
| `ignoreElements` | yes, after a window | no | a count | the summary only |

`key` is Rx's `groupBy`, as an attribute of the chain rather than an operator.

Deferred, with the reason:

| Rx name | Why not now |
|---|---|
| `throttle`, `rateLimit` | Pacing run starts is feature 022's. `throttle` with a trailing value is `window tumbling, emit: last` |
| `sample`, `auditTime` | Expressible as `window tumbling` with `emit: last` |
| `buffer` | The same as `window` with `emit: all`; one name is kept |
| `delay` | A timer before a start is a schedule (feature 018) or a `window count {count: 1}` |
| Sliding and hopping windows | One element in several windows multiplies members and run starts |
| `distinctUntilChanged` | Cheap, deferred only to keep the first set small |
| `takeWhile`, `skipWhile`, `takeUntil`, `skipUntil` | A predicate that ends an epoch needs an epoch reset rule |
| `scan`, `reduce`, `count`, `min`, `max` | An accumulator needs object construction (G042-001) and a reducer contract |
| `merge`, `zip`, `combineLatest`, `withLatestFrom`, `join` | State across triggers |
| `switchMap`, `concatMap`, `exhaustMap` | An overlap policy (feature 018) or a concurrency rule (022) |
| `retry`, `catch`, `timeout` | The engine's own retries, `onError` and wait timeouts |
| Event-time windows, watermarks | O042-004 |

### 3. Collections (release 0.3.0)

**One port call.** `ExpressionEvaluator` gains `collection_chain(items, chain, data, ceilings) -> Result<ChainOutput,
ChainFailure>`. `JsonEvaluator` implements it by building the environment once (the variables the `items` expression and
every stage name, parsed once from `RunData`), evaluating `items`, and then each stage with `element` and `position`
bound per element on top of that environment. The scheduler calls it inside `tokio::task::spawn_blocking`, so a long
chain does not hold a runtime worker; the blocking task checks a deadline of 1 second between elements, since it cannot
be aborted from outside (a single evaluation is already bounded by the expression limits). For a blob source the port
is called on the materialised elements instead of evaluating `items` again; the method takes either.

**Stage by stage, bounded while produced.** Each stage consumes its whole input before the next starts. While a stage
produces, the running count and the running serialised bytes are checked after every element; the first crossing stops
the chain. Each element is serialised once, when it is produced, which both measures it and gives the bytes the
snapshot stores. Peak memory is the environment plus two stage lists of at most 4 MiB serialised each, each element held
both as a value and as its bytes: several times the serialised size (T042-9 measured the resident memory; see the
validation report). A replica evaluates at most two chains at once and one tick enters at most four (D042-153); an
element nests at most 120 levels and one evaluation copies at most about 512 KiB (D042-151, D042-155).

| Ceiling | Value | Code | Where |
|---|---|---|---|
| Chain input elements (inline list or blob elements, with operators) | 1,000 | `COLLECTION_OPERATOR_BUDGET` | before the first stage; `materialize` bounded by it |
| Elements of a stage output | 1,000 | `COLLECTION_OPERATOR_BUDGET` | while produced |
| One element | 256 KiB | `COLLECTION_OPERATOR_BUDGET` | while produced |
| One stage output in all | 4 MiB | `COLLECTION_OPERATOR_BUDGET` | while produced |
| Wall time | 1 s | `COLLECTION_OPERATOR_BUDGET` (reason `wall_time`) | between elements |
| Expression error, `filter` not boolean, `flatMap` not a list | — | `EXPRESSION_ERROR` | the stage |
| Output elements over the node's `maxItems` | `maxItems` | `ITERATION_LIMIT` (today's) | after the chain |
| Inline output snapshot | 256 KiB | `RUN_DATA_LIMIT` (today's) | after the chain |
| Blob source size, one source item, source total | 8 MiB, 256 KiB, 4 MiB | `RUN_DATA_LIMIT` (today's) | `materialize` |

Every crossing is checked by the scheduler before `enter_collection` is called, so the store never sees an item it would
refuse with the untyped `InvalidTransition` (which retries every tick). The failure detail carries the stage index, the
operator name and the position, never a value. `ChainFailure` is a closed enum the scheduler maps to the codes above.

**Failure at entry fails the run.** The scheduler routes a `ChainFailure` through `stop_collection_for_source` with its
code, the path a failing `items` takes, which stops the run through `stop_with_code_routing(..., None)`. The collection's
`onError` is not taken (G042-008).

**What a commit records.** The output is recorded by `enter_collection`, which opens its own transaction and checks the
fence and revision. When that entry does not commit (the fence moved, the revision changed, `SourceUnavailable`, or the
storage-quota path that answers `Waiting`), the evaluation is discarded and the next pass evaluates again; over the same
immutable run data and the same digest-checked blob it gets the same output, so what is finally recorded does not depend
on how many tries it took. A run whose entry answered `Waiting` for storage quota is not re-entered before a back-off
that doubles from one pass interval to 30 seconds, held per replica as a hint (G042-012), and every re-entry is counted
`collection_chain_reentries_total{reason}`. Once committed, nothing evaluates the chain again.

**The snapshot.** For an inline source the output is today's snapshot array (one payload, at most 256 KiB), and the
present checks of `enter_collection` and the restore apply unchanged. For a blob source with operators the snapshot is a
chained manifest under a key of its own, `{"$collectionChainSource": {"blob": …, "sourceCount": n, "sourceSha256": …,
"chainSha256": …, "count": m, "itemsSha256": …}}`; `count` and `itemsSha256` describe the output items stored per item.
`enter_collection` (about lines 1571-1583) and the restore (about lines 1430-1435) learn the variant and check the stored
items against the output count and digest; the source fields are provenance, since the restore never reads the blob
again. `SourceManifest::parse` returns `None` for the new key (it requires one top-level key, `$collectionSource`, with
three fields), so an old binary that met such a scope would fail its restore with an invalid-storage error rather than
misread it; section 10 keeps such a binary out.

**`distinct`.** The canonical form of FR-042-011 is written by a function of its own over `serde_json::Value`, which
sorts object keys itself and so does not depend on the `preserve_order` feature; strings are compared as they are (no
Unicode normalisation, as `==` does). The digest is SHA-256 of the canonical bytes, kept in memory only.

**Cost.** At most eight stages over at most 1,000 elements: at most 8,000 evaluations, each linear in at most 256
tokens, over at most 4 MiB per stage. It runs after the fence read and before `enter_collection`'s transaction, so it
lengthens no lock. T042-9 measures the worst case (eight `map` stages at the byte ceiling) and its effect on
`scheduler_tick_seconds`. No migration and no new journal event type.

**Telemetry.** `operator_elements_total{surface="collection", operator, outcome}` with `outcome` in `kept`, `dropped`,
`produced`; the existing `scheduler_collection_source_failures_total{reason}` gains the reasons of the table; the new
code joins `WorkflowPrimitiveRejectionCode::ALL`; a test feeds marked values through a passing and a failing chain and
asserts that no log line, span attribute or metric label contains them.

**SDKs.** `forEach` and `map` return the workflow builder and `map` is already the collection alias, so operators are
carried by the collection options, built from an operator factory, without breaking a present caller:

| SDK | Where the chain goes | Factory |
|---|---|---|
| TypeScript | optional `operators` in the `CollectionOptions` interface | `ops.filter(where)`, … |
| Go | variadic `operators ...ops.Operator` after `maxConcurrency` in `ForEach` and `Map` | package `workflow/ops` |
| Python | keyword-only `operators=None` in `for_each` and `map` | module `ops` |
| C# | optional init property `Operators` of the `CollectionOptions` record | static class `Ops` |
| Java | overloads `forEach(..., CollectionOptions, Operators)` and `map(...)`; the record keeps its two components | class `Ops` |
| Rust | new methods `for_each_with_operators` and `map_with_operators`; `CollectionOptions` (a public struct without `#[non_exhaustive]`) is unchanged | module `ops` |

The builders only assemble the list; the server validates.

### 4. Trigger chains: rows (slice 2)

One additive migration. All tables carry `tenant_id` and `namespace_id` first in their primary key.

| Table | Key | Holds | Removed |
|---|---|---|---|
| `workflow_triggers` (columns added) | — | `chain` (canonical), `chain_digest`, `chain_id` | with the trigger |
| `operator_chains` | `chain_id` | workflow, trigger, version, `chain_digest`, state `ACTIVE` or `DRAINING`, `key_count`, `mark_count`, shard | drained and empty |
| `operator_keys` | `chain_id, key_hmac` | `key_id` (random), key text (content), epoch start, counters, last element time, next sequence | idle past its epoch |
| `operator_marks` | `chain_id, key_hmac, value_hmac` | `expires_at` | at `expires_at` |
| `operator_windows` | `chain_id, key_hmac, opened_at, sequence` | `window_id` (random), state, `closes_at`, `max_close_at`, `seal_horizon` (`xid8`, ADR-0153), count, bytes, first and last `received_at`, late and overflow flags, `version_id`, `run_id`, attempts, last error code, shard | after event retention |
| `operator_window_members` | `window_id, source, event_id` | `received_at`, late flag, `value` (bytea, the projected element; content) | `value` deleted at start; the row with the window |

The HMAC key is per tenant, generated for a new tenant by the migration's function and held in `operator_hmac_keys`,
which the runtime role cannot read. The one way to use it is `operator_key_hmac(tenant_id, namespace_id, value text)
returns bytea`, a `SECURITY DEFINER` function owned by the schema-owning role, with `search_path` pinned to `pg_catalog`
and the schema, executable by `sagat_app`, returning only the HMAC-SHA-256 (`hmac(value, key, 'sha256')` of the
`pgcrypto` extension, which the migration installs with `CREATE EXTENSION IF NOT EXISTS pgcrypto`; it is a trusted
extension since PostgreSQL 13). The instance registry's migration is the precedent for such a function
(`20270113000000_instance_registry`); the privilege audit gains its rows. The key never leaves the database. The API and the Studio name keys
and windows by `key_id` and `window_id`. `value` is deleted (set to `NULL` and its bytes released from the tenant's
usage) in the transaction that starts or discards the window; with `emit: first` or `last` one member row exists per
window, and after `ignoreElements` none has a value. `event_deliveries` gains `outcome`, a `window_id` and an error code,
written once by the insert and never updated; it keeps its key, its cascade from `inbox_events` and therefore its
lifetime, while a member row keeps its own `(source, event_id, received_at)` so a window outlives its events' inbox rows.

Window states: `OPEN` → `SEALED` → `STARTED`; `SEALED` → `REFUSED` (only `START_EXPIRED` is retryable, back to
`SEALED`); `OPEN` or `REFUSED` → `DISCARDED`. Only `OPEN` accepts members. A `SEALED` window is written only by its
start (and by the pass's back-off and expiry), never by an operator action, which is what lets the start write it in
the run's transaction (section 6). `operator_windows` gains `started_count` for `flatMap` (FR-042-020) and
`next_attempt_at` for the back-off.

Grants to `sagat_app` in the migration's grant block; the down migration refuses while a row exists;
`storage-classification.json` gets an entry per variable-size column (key text, member value, error code) and the
content tables the quota-delta triggers (`.claude/skills/migration`). Held bytes count against the tenant's retained
bytes and against a quota of their own. The backlog rule of section 6 reads the oldest pending event with
`attempts = 0` per (tenant, namespace); the existing `inbox_events_backlog_idx (tenant_id)` cannot serve it, so the
migration adds the partial index `inbox_events_unattempted_idx (tenant_id, namespace_id, received_at) WHERE status =
'PENDING' AND attempts = 0`. It costs one index entry per admitted event, removed at its first attempt, and is
measured in the statement budget of Task 042-19 (one more index write on admission, no more statements).

### 5. Trigger chains: applying an event (slices 2 and 3)

A chain of only `filter` and `map` is evaluated before today's start and changes nothing else (FR-042-014). For any
other chain, in `EventDispatcher`, one transaction:

1. evaluate the trigger `filter`, then `key`, then the stateless prefix (`filter`, `map`), with no lock held;
2. lock the `operator_chains` row `FOR SHARE` (a draining or removed chain takes no element);
3. if the chain has a keyed or windowed stage: lock the key row, inserting it first when new under `key_count` (at the
   ceiling the outcome is `FAILED`/`KEY_LIMIT` and nothing else is written); apply `distinct`, `skip`, `take`; then, for
   a chain with `window` or `debounce`, find the open window covering `received_at` or open one, add the member, move a
   session deadline that has not elapsed, and seal at once when `count` or `onFull: emit` says so; for any other chain,
   write the emission as a window sealed at once with its one member;
4. insert the `event_deliveries` row for `trigger:{id}` with the outcome (`HELD`, `SUPPRESSED`, or `FAILED` with its
   code); on a conflict, roll back: the admission was already applied.

When step 3 sealed a window, the dispatcher starts it right after the commit, in a second transaction with the start
code of section 6; a crash or a refusal there leaves the window `SEALED` for the pass. These locks are slot 3 of
ADR-0050 decision 7 (run-creating aggregates: operator chain row, then key row, then window rows), registered there by
Task 042-13; the apply takes no lower slot and creates no run. A failure here leaves the event pending and the
dispatcher's existing retry applies, except `KEY_LIMIT`, which is an outcome and not a failure.

### 6. Trigger chains: the pass (slice 3)

A pass `operator_windows` in `crates/scheduler/src/passes.rs`, on the shards the replica holds. Each tick, in bounded
batches per tenant:

- **seal**: for `OPEN` windows whose deadline `D` (`closes_at` or `max_close_at`) has passed on the database clock:
  record `seal_horizon` when it is still null, and go on only once `pg_snapshot_xmin(pg_current_snapshot())` has
  reached it (ADR-0153; counted `held_by_horizon`); for each (tenant, namespace, trigger match) compute once the oldest
  pending inbox event with `attempts = 0`, scoped by tenant and namespace (the index of section 4); skip the windows
  that event is not past unless `D + maxHoldMs` has passed (counted `held_by_backlog`); for the others, in one
  transaction each: read the shard row `FOR SHARE`, lock the chain row, the key row and the window, and seal it if it
  is still due (a moved deadline is counted `lost_race`);
- **start**: for `SEALED` windows whose `next_attempt_at` has passed, read the held values (a sealed window's
  membership never changes, so no lock is needed to read them) and build the input; then one transaction: read the
  shard row `FOR SHARE` (ADR-0020), call the admission port with the window's key and fingerprint, which takes the
  receipt (slot 2) and creates the run (slot 6) with 022's rows (slots 7 and 8); in the run's open transaction, update
  the window `WHERE state = 'SEALED'` to `STARTED` with `run_id` (or advance `started_count` after `flatMap`) and delete
  its held values. The window row is the only operator row the start touches, and it takes it after the run: it is
  registered in ADR-0050 as an 8a row (bookkeeping of the run the transaction creates). That is safe because a
  `SEALED` window is written only by its own start (section 4), whose duplicates serialize first on the slot-2 receipt,
  so the update never waits on a transaction that holds another slot; the start holds no chain or key row while the run
  is created, so no apply on that key waits behind it. A retryable outcome of the table of FR-042-019 rolls the
  transaction back and a short transaction on the window alone advances `attempts` and `next_attempt_at` (back-off
  doubling from 1 second to 5 minutes); a final outcome sets `REFUSED` in that short transaction;
- **purge**: lock the chain row first, then keys, marks and finished windows past their retention, and `DRAINING` chains
  with nothing left.

Nothing is held in memory between ticks. The lag of the pass is the age of the oldest due window that is neither held
by the horizon nor by the backlog rule: `scheduler_lag_seconds{pass="operator_windows"}`, so the alert does not fire on
the waits the design asks for; `operator_seal_horizon_age_seconds` shows the age of the oldest recorded horizon.

### 7. Semantics under failure, stated once

| Case | Outcome |
|---|---|
| Same event id inside the dedupe window | Absorbed by the inbox; the chain never sees it |
| An admission dispatched twice | One application: the delivery row's key; the second transaction rolls back |
| Crash during the apply transaction | Nothing applied; the event is dispatched again |
| Crash between seal and start | The window is `SEALED`; the pass starts it under the same key |
| Start retried after an unknown commit | The admission port finds the key; one run |
| Events out of order | Each joins the window covering its `received_at`; `emit` order is `(received_at, source, event_id)` |
| An event applied after its window sealed | Late: an open window or a new one, flagged, counted |
| Deadline and element at the same instant | The key lock orders them; both orders are valid and tested |
| An event committed just after the deadline with an earlier `received_at` | Covered by the transaction horizon (ADR-0153); were it applied after the seal, it is late, never lost |
| An event that stays claimed and never attempted | Holds sealing for at most `maxHoldMs` |
| Scheduler down past many deadlines | On return the backlog rule lets pending events in, within `maxHoldMs` |
| The run a window started fails, or times out in 022's buffer | A run like any other; the window stays `STARTED` |

Collection side (0.3.0):

| Case | Outcome |
|---|---|
| Crash or fence loss while the chain is evaluated | Nothing recorded; the next owner evaluates once more and records once |
| Cancellation racing the entry | First commit wins: cancelled with no scope, or entered and then cancelled |
| Storage quota at entry | `Waiting`, re-entered after the back-off, counted |
| Restart after entry | The snapshot (or the chained manifest and its items) is read; no chain evaluation |

### 8. Composition with features 022 and 018

Feature 022's 0.3.0 slice (US02 and US07; ADR-0056 and ADR-0088) limits how many runs or steps of a key are in flight
(`concurrency`) and how many start per interval (`rateLimit`, a strict sliding window of durable grants, the excess
buffered as `WAITING` runs and granted by a fenced pass). This design is upstream of it:

| Stage | Owner | Unit | What it holds | What it decides |
|---|---|---|---|---|
| Inbox admission | 004, 009 | an event | nothing; refuses with `429` | whether the event is recorded |
| Operator chain | 042 | an event, per chain and key | elements, before any run exists | which events ask for a run, and its input |
| Schedule overlap | 018 | a schedule action | actions, before a run exists | whether a fire starts a run now |
| Run admission | 022 | a run, per concurrency or rate key | runs that exist and wait | when a requested run starts |

1. **One creator port.** The window start uses the one admission port (FR-022-018, FR-022-041) and joins 022's
   run-creator test (T022-086) when slice 3 is built. On 2026-11-26 that port is built on the branch
   `feature/022-throughput-control` (the run gates decided by `decide_run_gates` in `crates/application/src/execution.rs`
   of that branch, with the creators of `tests/integration/flow-control-step-policies/rate_creators.rs`) and is not on
   `develop`. Slice 2 does not need it; the start of slice 3 (Task 042-22) and the composition test (Task 042-3) start
   only after that branch merges, and are written against the port as merged. Should 022 slip, slice 3 does not start
   a run through any other path (a second creator would bypass 022's limits).
2. **A created run is started.** A run 022 buffers exists, has its journal and counts against 022's ceilings, so the
   window is `STARTED`. 022's D022-018 says the window "stays sealed" in that case; both texts agree that a repeated
   request finds the run, and 022's wording is to be aligned on 022's branch by its owner (G042-011, T042-3).
   The seal and the start are two transactions (Gap 042-002, decided 2026-11-26, Decision 042-162): the seal is short
   and fenced; the start takes 022's gate rows and may wait on a buffer; a crash between them leaves the window `SEALED`,
   which the next pass starts under the same key. The start needs no change to 022's port: `RunStore::create` already
   returns the run's open transaction (`crates/application/src/runs.rs`), in which the start writes its window.
3. **A full buffer is a retry**, as 022 leaves an event start pending (FR-022-042, O1165): the window stays `SEALED`.
4. **No operator paces.** `throttle` is `rateLimit`. The chain's key sees the event; 022's key sees the run input.
5. **Two things named window.** 022's window is an interval of grants; this design's is a set of gathered elements.
6. **Lock order.** The apply's chain, key and window rows are ADR-0050 slot 3 and are never held when a run is created;
   the start takes no chain or key row and writes only its `SEALED` window, after the run, as an 8a row (section 6).
7. **Collections (0.3.0).** A chain changes how many iterations a `forEach` has and nothing about their admission; 022
   refuses `rateLimit` on a body node in 0.3.0 (`SCOPED_RATE_LIMIT`).

### 9. Surfaces (slice 5, except the collection telemetry of section 3)

- **REST** (additive): `GET /v1/namespaces/{ns}/operator-chains`, `GET …/operator-chains/{chainId}/windows`,
  `GET …/windows/{windowId}`, `POST …/windows/{windowId}:flush`, `:discard`, `:retry`, and a retry of one delivery;
  permissions as FR-042-023. Typed errors with `code`, `retryable`, `traceId`.
- **Audit**: `operator.window_flushed`, `operator.window_discarded`, `operator.window_retried`,
  `operator.window_refused`, `operator.delivery_retried`, `operator.key_limit` (collapsed per chain and minute, with a
  count).
- **Metrics** (closed labels): `operator_elements_total{surface, operator, outcome}`, `operator_windows_total{outcome}`
  (`outcome` in `started`, `refused`, `discarded`, `retried`, `expired`), `operator_open_windows`,
  `operator_window_seal_total{outcome}` (`outcome` in `sealed`, `held_by_horizon`, `held_by_backlog`, `lost_race`,
  `sealed_on_count`, `sealed_on_full`), `operator_window_emit_lag_seconds`, `operator_seal_horizon_age_seconds`, and
  `scheduler_lag_seconds{pass="operator_windows"}` (defined in section 6).
- **Spans**: `operator.apply` on the event's trace; `operator.window.start` with links to at most 32 member traces. The
  run's first journal event records the window that started it (chain, window id, count).
- **Studio**: the `Triggers` editor in `features/workflows/WorkflowSettings.tsx` gains the key and the stage list; a
  window list per workflow; the event page shows the delivery outcome.
- **SDKs**: the trigger builders reuse the operator factory of section 3. Scenarios: `collection-operators` (build),
  `collection-operators-runtime`, `collection-operators-limits` in 0.3.0; `trigger-operators` (build),
  `trigger-debounce-runtime` and `trigger-window-runtime` with slice 5 (which close windows with `:flush`, not by
  waiting).

### 10. Rollout

Capabilities are compiled into a binary and switched per deployment. **Release 0.3.0:** publication of a chained
`forEach` or of a trigger chain requires every scheduler and control-plane replica to run 0.3.0, which the upgrade
procedure already requires (`docs/runbooks/scaling.md`: mixed old and new writers are unsupported; publication and
starts of a new capability wait until its engines are available). An old replica that met such a definition would fail
`parse_stored` with a schema error and leave the run alone; one that met a chained blob scope would fail its restore
with an invalid-storage error. Neither misreads; both are covered by an old-reader test (T042-50).

**What the instance registry enforces, and what it cannot** (ADR-0153, ADR-0082). The switches are also capabilities
of the registry, `collection-operators-v1` and `trigger-operators-v1`. Under ADR-0082 a process lists a capability for
what its configuration runs, not for what the binary could do: a control plane lists one when its definition service
has the switch on, and an execution engine when its dispatcher and its `operator_windows` pass run with the switch on.
The operator records the requirement for both kinds (`platform_requirement_activate`), which the registry refuses while
a live, registered process lacks the capability; the definition service refuses to publish a chained `forEach` or a
trigger chain, and `switch` to activate one, while its requirement is not recorded (Task 042-54). Publication reads the
requirement without a lock and the guard is the registry's removal check: the `verify` step of `remove`
(`crates/infra-postgres/src/instances.rs`) refuses to remove `trigger-operators-v1` while an `operator_chains` row is
`ACTIVE` or `DRAINING` or a window is not final, and `collection-operators-v1` while an active version has a chained
`forEach` or a chained blob scope is open; no definer function is needed for publication.

A binary older than the registry (every binary before 0.3.0) does not register, so the registry cannot see it: an older
dispatcher would read the trigger row without the chain and start a run per event. For those binaries the runbook's
upgrade rule is the only guard, and the over-delivery is the residual risk of a deployment that breaks it (Gap
042-006, reopened). Rollback: switching publication off stops new chains; stored definitions stay readable (section 1)
and open windows drain. Rolling the binary back below a recorded requirement is unsupported (ADR-0082): the operator
first switches publication off, lets the chains drain or removes their triggers, removes the requirement (which
`verify` then allows), and only then rolls the binary back.

### 11. Decisions recorded

- **ADR-0087 (accepted, OD-2026-11-13-17; revised after review)**: collection operators are applied once at scope
  entry by one port call under running ceilings, and the snapshot is the record. Release 0.3.0.
- **ADR-0086 (accepted, OD-2026-11-13-17; revised after review)**: an operator chain on a trigger is declared data
  applied by the engine; its state is rows per chain and key keyed by HMAC; time is the database clock at admission; a
  fenced pass seals a window, and a sealed window starts one run by its identity through the one admission port.

- **ADR-0153 (accepted, delegate, Owner decision 2026-11-26-136)**: a window is sealed only past the transaction
  horizon of its deadline, replacing ADR-0086's admission grace; both switches are capabilities of the instance
  registry. Slices 2 to 5.

Each ADR states what the review changed in a section of its own; the owner accepted the earlier text of ADR-0086 and
ADR-0087.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A chain exhausts memory or time at entry | Running ceilings on elements and bytes per stage, one environment, a blocking task with a deadline (FR-042-006, 007); a test per ceiling |
| A stored definition becomes unreadable when a switch is off | Switches are checked at publication only; a `parse_stored` test with the switch off |
| A blob scope's checks reject a chained membership | A chained manifest with the output's count and digest, checked at entry and restore |
| A replay sees another list | The output is recorded once; SC-042-003 runs the restart with an evaluator that panics |
| `distinct` disagrees with `==` | A canonical form with tests on `1` and `1.0`, `-0` and `0`, key order |
| Stage names collide with nesting | `element` and `position`; `item` and `index` refused in a stage |
| An SDK change breaks callers | Options and overloads that leave every present call compiling |
| A window that never closes holds events | `maxWaitMs` required; `maxHoldMs` caps the backlog rule; flush and discard |
| Held personal data outlives its use | A deletable column, deleted at start; HMAC keys; surrogate ids; a held-bytes quota |
| A key flood stalls a namespace | `KEY_LIMIT` is terminal at once and leaves the backlog; audit collapsed |
| Two rate limiters appear | No operator paces; section 8 |
| The trigger half drifts before it is built | Re-read against `develop` on 2026-11-26 (Task 042-44); its fields enter the schema only with slice 2 |
| Feature 022's admission point is not merged when slice 3 starts | Slice 3's start waits for it and no other creator path is used (section 8); slices 2, 4's fault tests of the apply path and 5's read surfaces proceed |
| A long transaction delays every window | It delays, never loses (ADR-0153); the pass's lag metric and alert show it |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Collections: the output is recorded once by the entry transaction and never recomputed (FR-042-009). Triggers: all state is rows; an application commits with its delivery row; seal and start are idempotent by the window's identity (FR-042-014, 018, 019) |
| II. Modular domain, ports and adapters | Pass | `OperatorChain` and the window module `crates/domain/src/operators/window.rs` are pure (no serde_json, SQL or clock); one evaluator port method per surface; the store port `OperatorChainStore` in `crates/application`; statements in `infra-postgres`; the switch in the definition service |
| III. External effects are at-least-once; sagas compensate | Pass | No external effect; runs start through the ordinary path |
| IV. Compatibility is versioned and pinned | Pass with conditions | Additive v1 fields; switches never make a stored definition unreadable; six SDKs without breaking a caller. Condition: a registered process without the capability is kept from serving by the registry's requirements; a binary from before the registry is kept out only by the upgrade rule (section 10, Gap 042-006) |
| V. Code, tenant boundary and secrets are isolated | Pass | Bounded expressions evaluated by the engine, off the async runtime under ceilings; tenant and namespace in every key; held data classified and deletable; HMAC keys and surrogate ids (FR-042-002, 007, 023, 024) |
| VI. Observability and audit are part of the feature | Pass | Typed codes in the closed sets, counters with closed labels, a no-value-leak test, audit of every manual act (FR-042-025) |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | The chain sits after the inbox and applies once per admission; `KEY_LIMIT` is an outcome, not a held event |
| VIII. Failure is tested and contracts are verified, honestly | Pass | SC-042-003 to 005 with slices 0 and 1 (panicking evaluator, a test per ceiling, races with barriers); SC-042-001, 002, 006, 007 with slices 2 to 5, all in release 0.3.0 |
| Delivery Constraints | Pass | One writable region per cell, PostgreSQL as authority, no new component; budgets are targets on a stated environment |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A port method for the whole chain | The present per-expression call parses the run's data each time | Calling `condition` per element: GiBs of parsing on a large run |
| Running ceilings per stage | A count of evaluations bounds neither memory nor bytes | One evaluation counter: `map` over a large step output passes it |
| A chained blob manifest | The present checks compare stored items to the blob's | Re-reading the blob at restore: a read the design forbids |
| A blocking task with a deadline | The chain is CPU work on the scheduler | Running inline on the runtime: one chain stalls the pass |
| Six tables of chain state (slice 2) | A debounce that survives a restart is a row with a deadline | State in memory: lost on restart, wrong with two replicas |
| A fenced pass with the transaction horizon and the hold rule (slice 3) | A deadline must fire with no event, and a commit can land after it | Sealing on the clock alone: a burst split by commit latency |
| HMAC keys behind one definer function, and surrogate ids (slice 2) | A digest of a short key can be reversed, and the runtime role must not read the key | Plain SHA-256 digests shown in the API; or the key readable by the runtime role |
| Every stateful emission as a window (slice 2) | A run is created only by the start, so the delivery row and the run never collide | Applying and starting in one transaction: the apply's delivery row and today's start write the same row |
| A canonical stage order on triggers (slice 2) | Arbitrary orders give each pair its own stateful semantics | Any order |
| A capability requirement before publication | Readers refuse unknown fields by design, so an old replica must not serve once a chain is published | A runbook rule alone: nothing enforces it (Gaps 042-006 and 042-140) |

## Alternatives Considered

- **Operators as code** (a sandboxed function per stage). A round trip per element, nothing the Studio can draw,
  nondeterminism to police. Rejected by principle V and rule 7.
- **An evaluation counter as the only ceiling** (the first draft). It bounds neither bytes nor memory; replaced.
- **The switch in the codec** (the first draft). It would make stored definitions unreadable when switched off.
- **`onError` at entry in 0.3.0.** Collections do not take it today for a failing `items`; adding it for chains only
  would make two behaviours. A follow-up for both (G042-008).
- **Lazy stages fused across the chain** (so `take` limits `flatMap`). Less work in some chains, a harder error rule
  and harder ceilings; stage by stage is simpler to state and test (G042-013).
- **Debounce as a concurrency policy of 022** ("a new start withdraws the queued one"). It covers "last wins" only.
- **A separate stream resource outside the workflow version.** A second, unpinned place where a workflow's behaviour lives.
- **Holding trigger values in `payloads`.** Append-only, so they could never be deleted.
- **Windows keyed on `occurredAt`.** O042-004.

## Non-Functional Requirements

- **NFR-042-001 Performance** (targets on a stated environment, not promises): a collection chain of eight stages over
  1,000 small elements adds at most 20 ms to scope entry at the 95th percentile, and the worst case at the ceilings is
  reported with its effect on `scheduler_tick_seconds` (T042-9). Slices 2 to 5: an event joining an open window costs at most 6
  statements in its one transaction; sealing and starting a window at most 8 beyond the run start; a trigger without a
  chain costs zero more (FR-042-030); 1,000 events per second across 10,000 keys with the pass's lag under 2 seconds at
  the 99th percentile (T042-33).
- **NFR-042-002 Security**: one `SECURITY DEFINER` function, `operator_key_hmac` (section 4), with a pinned
  `search_path`, and no other; no new role; the runtime role gets only the grants the statements need and no read of
  `operator_hmac_keys`, checked by the privilege audit; the cross-tenant battery covers every new endpoint, table and
  the function.
- **NFR-042-003 Operability**: the runbooks say what each metric and code means, the 0.3.0 upgrade rule, what a held
  window is and how to flush or discard one; an alert rule on the pass's lag ships with slice 3.
- **NFR-042-004 Compatibility**: every existing definition, trigger and run behaves as before; the OpenAPI and IR schema
  changes are additive and pass the contract baselines.
- **NFR-042-005 Storage**: with the default quotas a tenant's chain state is bounded, and the bound is stated in bytes
  in the runbook after measurement.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

**Release 0.3.0 (OD-2026-11-13-15): slices 0 and 1**, from `develop` after pull request 218. **Slice 0** foundation, collection
half: T042-1, 4, 5, 6, 47. **Slice 1** collection operators: T042-7 to 12, 45, 46, 48 to 52, and 56 (the one small migration of slice 1, which adds
its capability to the registry). No table, pass or loop; the collection entry of the scheduler pass changes.

**Release 0.3.0 since Owner decision 2026-11-24-1: slices 2 to 5.** **Slice 2** chain state and the operators without
time (Tasks 042-2, 3, 13 to 19, 43, 44, 53, 54); **slice 3** windows and the pass (Tasks 042-20 to 26 and 55); **slice
4** failure and capacity (Tasks 042-27 to 33); **slice 5** surfaces and close (Tasks 042-34 to 42). ADR-0086 and
ADR-0153 are accepted; Task 042-44's re-read is the third revision of this design.

**Two lanes for two builders, with no crate in common:**

- **Lane A, declarations and surfaces:** `crates/domain` (the trigger rules), `crates/infra-ir` (the schema and the
  codec), `crates/api-http` (the REST resources), the six SDKs and the Studio. Tasks 042-43 (first), then 042-10 and
  042-11 (the SDK half of slice 1 still open, Gaps 042-113 and 042-131), 042-36, 042-37, 042-34, 042-17's response
  shape, 042-38, 042-39.
- **Lane B, state and execution:** `crates/application` (the definition service's switch and capability, the
  dispatcher's apply, the window start), `crates/infra-postgres` (the one migration and every statement),
  `crates/scheduler` (the dispatcher and the pass) and the test targets. Tasks 042-2, 13, 14, 54, 56, 15, 16, 18, 19,
  53, 55, 20, 21, 23 to 26, 3, 22, 27 to 33, 35, and the `trigger-operators` switch of Task 042-43 in
  `DefinitionService`.
- **Shared files:** Task 042-43 is split by crate: lane A writes the domain rules, the schema and the codec, lane B the
  switch in `crates/application`; Task 042-18's REST retry action is written by lane A in Task 042-34 over lane B's
  application method. Both lanes append rows to `specs/042-stream-operators/research.md` from disjoint ranges (lane A
  Decision 042-300 upward, lane B Decision 042-400 upward), so a merge keeps both sides.
- **Order points between the lanes:** Task 042-43 (lane A) merges before Task 042-15 (lane B) applies an event; the
  migration of Task 042-13 (lane B) merges before lane A's REST resources (Task 042-34) are wired to lane B's services,
  which lane B exposes as application methods first; Tasks 042-3 and 042-22 (lane B) start after feature 022's
  throughput branch merges (section 8); Tasks 042-40 to 042-42 close both lanes.

- [ ] T042-1 Confirm that OD-2026-11-13-15 and OD-2026-11-13-17 are rows of `docs/owner-decisions.md` on `develop` and
  that the `Covers` of OD-2026-11-13-15 names every task gated on it here, before any of them is ticked.
  Refs: FR-042-001
- [ ] T042-2 Spike, in `specs/042-stream-operators/research.md`: the statement count of today's dispatch (the baseline
  of FR-042-030) and the input-blob path for a window above the inline ceiling. Seal and start are two transactions
  (Decision 042-162; Gap 042-002 closed).
  Refs: FR-042-019, FR-042-030
- [ ] T042-3 Prove the composition with feature 022 as built: the window start goes through the one admission port and
  is in the run-creator test; a window whose run 022 buffers is `STARTED`; a full buffer leaves it `SEALED`; the start
  holds no chain or key row and writes its window after the run, with a barrier test that an apply on the same key is
  not held behind a start; ask 022's owner session to align D022-018's wording on 022's branch (G042-011); this task
  edits no register of feature 022.
  Refs: FR-042-019, FR-042-029
  Owner-gate: OD-2026-11-27-420
- [ ] T042-4 Have ADR-0087 accepted before slice 1 merges and ADR-0086 before slice 2 starts; amend the design if either changes.
  Note (2026-11-13): done, not ticked. Both ADRs were set to Accepted in commit 540154c8 on the owner's "Aceito tudo"
  (OD-2026-11-13-17, the row added on develop by the owner's session), and revised after the reviews of the same day
  before any code (each ADR's Revision section). Rule 9 of `scripts/check-design-docs.py` refuses a tick in a doc that
  is not Approved, so the tick and its evidence block wait for approval.
  Refs: FR-042-003, FR-042-017
- [ ] T042-5 Add `OperatorChain` and `Operator` to the domain with the collection rules (the seven operators, parameter
  ranges, `window {count: 0}` refused, chain length), and property tests of the rules.
  Refs: FR-042-001, FR-042-004
  Owner-gate: OD-2026-11-13-15
- [ ] T042-6 Extend the IR schema and the codec with `forEach.operators` and the expression and variable checks
  (`item`, `index`, `iteration`, `carry`, `bodyResult` refused in a stage); fixtures prove every earlier document still
  decodes, an unknown operator is refused and a trigger operator is refused.
  Refs: FR-042-002, FR-042-005, NFR-042-004
  Owner-gate: OD-2026-11-13-15
- [ ] T042-7 Add `collection_chain` to the evaluator port and `JsonEvaluator`: one environment, stage-by-stage
  evaluation, `element` and `position`, the first failing stage and position reported, the canonical form of
  `distinct`, and a property test against a reference implementation over random chains and lists (including `1` and
  `1.0`, `-0` and `0`, objects with keys in different orders).
  Refs: FR-042-004, FR-042-005, FR-042-006, FR-042-011
  Owner-gate: OD-2026-11-13-15
- [ ] T042-8 Enforce the running ceilings and map each to its one code; call the chain in a blocking task with its
  deadline; route a failure through `stop_collection_for_source`; one test per ceiling crossing it by one and one at
  it; an empty output.
  Refs: FR-042-007, FR-042-008, SC-042-004
- [ ] T042-9 Measure scope entry with and without a chain and the worst case at the ceilings, with its effect on
  `scheduler_tick_seconds` and the count of re-entries, on a stated environment.
  Refs: NFR-042-001, SC-042-008
- [ ] T042-10 Add the operator factory and the collection options of section 3 to the six SDKs, with a compile test
  per SDK that every present call still builds.
  Refs: FR-042-028
- [ ] T042-11 Add the scenarios `collection-operators`, `collection-operators-runtime` and `collection-operators-limits`
  with reviewed goldens.
  Refs: FR-042-028, SC-042-003
- [ ] T042-12 Document collection operators, their ceilings and codes, body `index` after `filter`/`skip`, and the 0.3.0
  upgrade rule in the workflow-primitives runbook, the reference and the traceability map.
  Refs: FR-042-005, FR-042-007, NFR-042-003
- [ ] T042-13 Write the migration (tables, columns, indexes including `inbox_events_unattempted_idx`, grants,
  classification entries, quota triggers, the held-bytes quota, `trigger-operators-v1` in the registry's closed
  capability checks, a down that refuses data loss) with the migration checklist; register the operator rows in ADR-0050
  decision 7 (slot 3 for the apply and the seal, 8a for the start's window) with the slot test.
  Refs: FR-042-021, FR-042-023, FR-042-024, NFR-042-002
  Owner-gate: OD-2026-11-26-133
- [ ] T042-14 Register a chain when a version is published, activated, rolled back or promoted (in `switch` as well as
  at publication), drain the old one, discard on removal, and check the tenant quotas and the capability requirement on
  each of those paths; a test rolls back onto a chained version over a quota and sees it refused, and one under the quota
  registers its chains.
  Refs: FR-042-021, FR-042-022
- [ ] T042-15 Write `crates/domain/src/operators/window.rs` (the transitions, with property tests), the evaluator's
  `trigger_stage` method and the store port `OperatorChainStore`; then apply an event in one transaction: stateless
  prefix without locks, chain row, key row when needed, every emission of a stateful chain written as a window, the
  delivery row inserted once with its outcome, and the start of a window sealed at once right after the commit; a test
  that a `distinct` chain without a window starts its run, and that a chain of only `filter` and `map` behaves as today.
  Refs: FR-042-014
- [ ] T042-16 Add `distinct`, `skip` and `take` with epochs, marks and `maxMarks`.
  Refs: FR-042-015, FR-042-021
  Owner-gate: OD-2026-11-26-132
- [ ] T042-17 Expose the delivery outcome on the event resource.
  Refs: FR-042-025
- [ ] T042-18 Enforce `maxKeys` at key creation with the terminal `KEY_LIMIT` outcome and the collapsed audit; the
  delivery retry as an application method (delete the delivery row, return the event to `PENDING`, refuse
  `DELIVERY_EVENT_PURGED` when the inbox row is gone), with tests of both; its REST action is lane A's (Task 042-34).
  Refs: FR-042-021, FR-042-023
  Owner-gate: OD-2026-11-13-15, OD-2026-11-26-130
- [ ] T042-19 Hold FR-042-030 with a budget test: a trigger without a chain adds no statement.
  Refs: FR-042-030, NFR-042-001
- [ ] T042-20 Open, extend and seal-on-count windows in the apply transaction (tumbling, session, count; several open
  per key; all, first, last by `(received_at, source, event_id)`; `onFull`), with the numbers of O042-007.
  Refs: FR-042-016, FR-042-021
  Owner-gate: OD-2026-11-13-15
- [ ] T042-21 Add the `operator_windows` pass: fenced seal past the transaction horizon (ADR-0153, Task 042-55), the
  scoped backlog rule over `inbox_events_unattempted_idx` and `maxHoldMs`, with the seal outcomes and the lag of
  section 6.
  Refs: FR-042-017, FR-042-018
  Owner-gate: OD-2026-11-26-134, OD-2026-11-26-136
- [ ] T042-22 Start a sealed window's run by its identity, on its version, through the one admission port, with the
  three input shapes and the `flatMap` fan-out; the window written in the run's open transaction; held values deleted
  at start; each row of FR-042-019's outcome table (re-checked against 022's port as merged, and amended here first if
  it differs), with a test per row, the 24-hour expiry included; the input check of design 043 refusing with
  `RUN_INPUT_INVALID`.
  Refs: FR-042-019, FR-042-020, FR-042-024, FR-042-029
  Owner-gate: OD-2026-11-27-420
- [ ] T042-23 Add `debounce` and `ignoreElements` over the window machinery.
  Refs: FR-042-016
- [ ] T042-24 Add `flatMap` fan-out on a trigger with its ceiling and all-or-nothing start.
  Refs: FR-042-020
- [ ] T042-25 Mark and count late elements; test an event retried after its window sealed.
  Refs: FR-042-017
- [ ] T042-26 Purge keys, marks, finished windows and drained chains in bounded batches, chain row first.
  Refs: FR-042-022, NFR-042-005
- [ ] T042-27 Fault test: kill at each write of apply, seal and start; 500 events, one key, one run.
  Refs: SC-042-001
- [ ] T042-28 Fault test: every admission dispatched twice, and a lost broker hint, change nothing.
  Refs: SC-042-001, FR-042-014
- [ ] T042-29 Race test: seal against element in both orders, forced with a held lock, never a sleep.
  Refs: SC-042-002, FR-042-018
- [ ] T042-30 Fault test: restart with 1,000 open windows, deadlines passed and events pending.
  Refs: SC-042-007
- [ ] T042-31 Capacity test: 50,000 keys against `maxKeys` 1,000, with a second tenant's latency measured.
  Refs: SC-042-006
- [ ] T042-32 Cross-tenant and privilege tests for every new table and endpoint, also as the runtime role.
  Refs: FR-042-023, NFR-042-002
- [ ] T042-33 Measure NFR-042-001 for triggers on a stated environment and write the dated validation report.
  Refs: SC-042-008, NFR-042-001
- [ ] T042-34 Add the REST endpoints with the permissions of FR-042-023 and the audit actions, with the contract
  baseline.
  Refs: FR-042-026, FR-042-023
- [ ] T042-35 Add the trigger metrics, spans, the journal note on a window-started run and the lag alert rule.
  Refs: FR-042-025, NFR-042-003
- [ ] T042-36 Add the trigger builders to the six SDKs over the operator factory.
  Refs: FR-042-028
- [ ] T042-37 Add the scenarios `trigger-operators`, `trigger-debounce-runtime` and `trigger-window-runtime`.
  Refs: FR-042-028
- [ ] T042-38 Studio: the key and stage editor on a trigger with the publication rules.
  Refs: FR-042-027
- [ ] T042-39 Studio: the window list, the lag, the event page's outcome, flush and discard; keys and windows named by
  their ids, the key text only with the reveal right (O042-008).
  Refs: FR-042-027, FR-042-024
  Owner-gate: OD-2026-11-13-15, OD-2026-11-26-131
- [ ] T042-40 Write the events runbook section, the quickstart of CUJ 3 with its failure scenario, and the reference.
  Refs: NFR-042-003
- [ ] T042-41 Update `docs/traceability.md`, the roadmap row and the release notes; register the remaining gaps.
  Refs: NFR-042-004
- [ ] T042-42 Write the trigger rollout procedure (readers first, fleet-wide before `trigger-operators`, drain on
  rollback) and rehearse it on the compose stack.
  Refs: FR-042-003
- [ ] T042-43 Extend the domain rules, the IR schema and the codec with a trigger's `key` and `operators` (canonical
  order, "can emit", ceilings) and the `trigger-operators` switch. (First task of slice 2.)
  Refs: FR-042-003, FR-042-013
  Owner-gate: OD-2026-11-13-15
- [ ] T042-44 Before slice 2 starts, re-read sections 4 to 10 against `develop` (the dispatcher, the start path, feature 022
  as merged, the instance registry), decide the refinements of O042-008 and O042-009 and the new numbers (`maxMarks`,
  held-bytes quota, `maxHoldMs`, the admission grace), and amend the design.
  Note (2026-11-26): done by the third revision under Owner decision 2026-11-24-4 (Owner decisions 2026-11-26-130 to
  2026-11-26-136); not ticked, because the doc is not Approved.
  Refs: FR-042-014, FR-042-017, FR-042-021
- [ ] T042-45 Record a blob source's chained manifest: `materialize` bounded by the chain input ceiling, the variant
  read by `enter_collection` and the restore, and tests of a blob with a chain at entry and at restore with `verify_run`.
  Refs: FR-042-010, FR-042-007
  Owner-gate: OD-2026-11-13-15
- [ ] T042-46 Check the output ceilings in the scheduler before `enter_collection`, so the store never answers a chained
  entry with `InvalidTransition`; a test that a chain at each ceiling reaches the store with a valid entry.
  Refs: FR-042-007, FR-042-010
- [ ] T042-47 Add the `collection-operators` switch to `DefinitionService` beside `lake.enabled`, refusing publication
  with a typed error; a test that a stored chained definition parses through `parse_stored` with the switch off.
  Refs: FR-042-003
  Owner-gate: OD-2026-11-13-15
- [ ] T042-48 Race tests with barriers: cancellation against entry in both orders; the shard lease taken while the
  chain is held inside the evaluator. A test that after entry an evaluator that panics on the chain call sees zero
  calls across a restart, for an inline and a blob source.
  Refs: SC-042-003, SC-042-005, FR-042-009
- [ ] T042-49 Telemetry: `operator_elements_total{surface="collection"}`, the code in
  `WorkflowPrimitiveRejectionCode::ALL`, the codes in `events.asyncapi.yaml` and `docs/sdk-compatibility.md`, and a test
  that no element value reaches a log, a span or a label.
  Refs: FR-042-025
- [ ] T042-50 An old-reader test: a binary of the release before 0.3.0 (or its codec and restore as a fixture) refuses a
  chained definition and a chained blob scope without misreading them; the runbook states the fleet rule.
  Refs: FR-042-003, NFR-042-004
- [ ] T042-51 Hold FR-042-012 with `collection_entry_without_operators_is_unchanged`: same statements and snapshot bytes
  as before the change for a `forEach` without operators.
  Refs: FR-042-012
- [ ] T042-52 Back off the re-entry of a collection that waits for storage quota and count re-entries by reason.
  Refs: FR-042-009
- [ ] T042-53 The per-tenant HMAC key of trigger chains: `pgcrypto`, `operator_hmac_keys`, the definer function
  `operator_key_hmac` with its pinned `search_path`, no read path for the runtime role (privilege audit rows), and a test
  that no digest of a key leaves the database.
  Refs: FR-042-024, NFR-042-002
- [ ] T042-54 Add the capabilities to the instance registry (`Capability::ALL`; `collection-operators-v1` in the small
  migration of Task 042-56, `trigger-operators-v1` in Task 042-13's), list each only where the configuration runs it
  (section 10), refuse publication and activation of a chained `forEach` or a trigger chain while the requirement is not
  recorded, and define the `verify` of their removal; tests of both refusals, of an activation refused while a live
  process lacks the capability, and of a removal refused while a chain or chained version exists.
  Refs: FR-042-003
  Owner-gate: OD-2026-11-26-136
- [ ] T042-55 Seal past the transaction horizon: `operator_windows.seal_horizon`, recorded at the first pass after the
  deadline; the admission statement takes `pg_current_xact_id()` before `received_at` in one statement; a race test that
  holds an admission transaction open past the deadline with a held lock and sees the window stay open until it commits,
  and a budget test that the admission issues the same statements as before. The admission statement orders the two
  reads with `MATERIALIZED` common table expressions in sequence (the transaction id first), and
  `scripts/check-architecture.py` refuses an `INSERT INTO inbox_events` outside `admit_within`.
  Refs: FR-042-017, FR-042-030
  Owner-gate: OD-2026-11-26-136
- [ ] T042-56 A small migration, delivered with slice 1, that adds `collection-operators-v1` to the registry's closed
  capability checks, so that slice 1 is gated whether or not slice 2 ships.
  Refs: FR-042-003
  Owner-gate: OD-2026-11-26-136

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the three
checkers, the unit tests of the crates it changes and the suites it touches, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`
where it adds a statement. A test is seen failing for the right reason before the code that passes it; races use held
locks, barriers or failpoints, never sleeps.

- **Slice 0** (0.3.0): the IR contract tests with the earlier fixture corpus unchanged; domain property tests; the
  `parse_stored` test with the switch off (T042-47).
- **Slice 1** (0.3.0): `integration_workflow_primitives` with unmodified tests still passing and the new tests by name
  (T042-8, 45, 46, 48, 51); the property test of T042-7; `sdk_conformance` for the three collection scenarios in six
  languages; the no-value-leak test (T042-49); the old-reader test (T042-50); a hand mutation per operator (invert the
  predicate, drop the last chunk, flatten two levels, compare `distinct` by raw bytes) to prove the tests can fail.
- **Slices 2 and 3** (0.3.0): a new target `integration_stream_operators` on real PostgreSQL; the events suites of
  feature 004 unmodified; the budget tests of Tasks 042-19 and 042-55; the horizon race test of Task 042-55; the
  capability tests of Task 042-54; the migration's up, down and privilege audit.
- **Slice 4** (0.3.0): the fault and race tests of T042-27 to T042-32 by name; the capacity and performance runs state the
  machine, its load, the PostgreSQL version and configuration, and what was skipped.
- **Slice 5** (0.3.0): the OpenAPI baseline (additive, never blessed blind), `npm run api && npm run check:api` in the Studio,
  the trigger conformance scenarios, and the quickstart run end to end including its failure scenario.

Size, stated plainly: 55 tasks, all in release 0.3.0. Slices 0 and 1 are eighteen (Tasks 042-1, 4 to 12, 45 to 52),
most of the server half built and waiting for approval to be ticked, the SDK half open; they touch no table and add no
loop. Slices 2 to 5 are thirty-seven (Tasks 042-2, 3, 13 to 44, 53 to 55): one migration, a change in the dispatcher's
hot path, a new pass and its fault battery, the surfaces in six SDKs and the Studio; comparable to the first delivery of
feature 018, in two lanes.
