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

**State.** Slice 0 and the server half of slice 1 are built (research Decisions 042-100 to 042-160), with two exceptions
the code shows: the registry capability `collection-operators-v1` is not yet a value of `Capability::ALL`
(`crates/application/src/instances.rs`), so the publication refusal that ADR-0153 decision 4 asks for is not built
(Task 042-56 is open); and a blob source over the chain input ceiling is stopped through `stop_collection_for_source`,
which does not count `workflow_primitive_rejections_total`, while the same code from an inline list is counted (Task
042-8 routes it through the counted path). The SDK half of slice 1 is open (Gaps 042-113 and 042-131); slices 2 to 5 are
not built. Tasks stay unticked until the design is approved, as the rules of `docs/designs/README.md` require. The
registers are in `specs/042-stream-operators/research.md`; how each draft and review changed this text is in the section
Revisions at the end.

A word on names: "Streams" already means the workflow Streams of feature 026 (ADR-0068). To keep the two apart, the
tables and types of this design say *operator chain*, never *stream*.

### Terms

- **Slice.** A unit of delivery of the Detailed Implementation Plan: slices 0 and 1 are the collection half, slices 2 to
  5 the trigger half.
- **Chain.** The ordered list of operator stages declared on a `forEach` or on a trigger, with, on a trigger, its `key`
  and its chain-level limits.
- **Keyed stage.** `distinct`, `skip` or `take`: a stage whose state is kept per key.
- **Window.** A row of `operator_windows` that gathers the elements of one key and, once sealed, starts one run. A chain
  with `window` or `debounce` gathers elements into open windows; every other stateful chain writes each emission as a
  window sealed at once with its one member.
- **Emission.** What one sealed window asks for: one run, or one run per element after `flatMap`.
- **Summary.** The `window` object of a run input that a window started (section 5a).
- **Horizon.** The transaction horizon of ADR-0153: the newest transaction id the pass saw when a window's deadline
  passed; the window is not sealed before every transaction up to it has ended (FR-042-017).
- **Backlog hold.** The rule that keeps a due window open while a pending event that could join it waits in the inbox,
  for at most `maxHoldMs` past the deadline (FR-042-017).
- **Slot.** A position in the lock order of ADR-0050 decision 7 (slot 2 the request receipt, slot 3 the run-creating
  aggregates, slot 6 the new run, slot 8a bookkeeping, slot 8b usage rows).
- **Receipt.** The idempotency record of the admission port (slot 2) under which a repeated start finds the run the
  first one created.
- **Admission port.** The one path every run creator uses (feature 022, FR-022-018 and FR-022-041): `RunStore::create`
  with 022's gates.

## The Problem

**What exists on the collection side** (feature 014, `docs/runbooks/workflow-primitives.md`). The collection entry of the
scheduler (`crates/scheduler/src/passes.rs`) reads the run data under the fence, drops that
transaction, evaluates `items` through `ExpressionEvaluator::collection_source`, and then:

- for an inline list, stores the whole membership as one snapshot payload, refused above `MAX_PAYLOAD_BYTES` (256 KiB)
  with `RUN_DATA_LIMIT`, and refuses more elements than the node's `maxItems` with `ITERATION_LIMIT`;
- for a blob reference, materialises the blob bounded by the node's `maxItems` (`canonical_items`,
  `crates/application/src/collection_source.rs`: 8 MiB blob, 256 KiB per item, 4 MiB in all) and stores a
  `SourceManifest` (blob, count, items digest) as the snapshot; `enter_collection` and the restore in
  `crates/infra-postgres/src/primitives.rs` require the manifest's count and digest
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
- `JsonEvaluator`'s `environment` parses the run input, every step output of the path and the state from
  bytes each time an expression is evaluated. Evaluating a stage once per element through the present port would parse
  the run's data once per element.
- The IR schema is closed and a stored definition is validated again when read (`parse_stored`, `crates/infra-ir/src/
  lib.rs`). Both `parse` and `parse_stored` reach `convert::published_collection_definition`. A check placed in `convert::*` therefore runs on read as well as on publication.
- The scheduler is sharded by a fence (ADR-0020). The event dispatcher is not shard-fenced; it holds a 60-second claim
  per event, and claiming does not increase `attempts` (`crates/infra-postgres/src/dispatch.rs`, `claim_events`).
- `payloads` is append-only: a trigger refuses `UPDATE` and `DELETE` (`20261009000100_execution.up.sql`, the trigger `payloads_append_only`).
- Feature 022 owns *when a requested run may start*: concurrency by key and, in its 0.3.0 slice, a rate limit per key
  whose excess is buffered durably (US07, ADR-0088). Feature 018 owns schedules and their overlap policies. This design
  must not grow a second rate limiter or a second buffer of runs.

## Requirements

**Vocabulary and language**

- **FR-042-001** The operator set is closed and named in the IR schema: `filter`, `map`, `flatMap`, `skip`, `take`,
  `distinct`, `window`, `debounce`, `ignoreElements` (the last two on triggers only): a collection has seven of them, a trigger all nine, both
  in release 0.3.0. A definition that uses any other name, or an operator on a surface that does not have it, MUST be
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
  `inputSchema` is refused at publication (`OPERATOR_INPUT_SCHEMA`) unless that schema accepts every run input the chain can
  emit (FR-042-032), checked against a one-member sample of each shape.
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
  of at most 512 bytes in its canonical form; absent means one key for the whole trigger), `operators`, one to eight
  stages, and `chainLimits {maxKeys, maxMarks, keyIdleMs, maxHoldMs}` (the parameter table of section 2). The stages
  are in canonical order: any number of `filter` and `map`; then at most one each of `distinct`, `skip` and `take`, in
  that order; then at most one of `debounce` or `window`; then at most one of `flatMap` or `ignoreElements`. A chain out
  of order is refused at publication (`OPERATOR_ORDER`), as is a chain that can never emit (`OPERATOR_NEVER_EMITS`:
  `ignoreElements` without `debounce` or `window`; `take {count: 0}`). The existing `filter` field of a trigger keeps its
  meaning and runs first. The element at stage 0 is the event document that trigger filters read today
  (`event_document`, `crates/application/src/dispatch.rs`: `{event: {id, source, type, subject, correlationKey,
  occurredAt, receivedAt}, data}`); each stage sees it as `element` (after a `map`, the value the `map` produced) and the
  event document itself as `event` and `data`; `position` is not available (a trigger stage sees one event). A key is
  compared in the canonical form of FR-042-011 (so `81` and `81.0` are one key, `"81"` another).
- **FR-042-014** An admitted event MUST be applied to a trigger's chain at most once. A chain of only `filter` and `map`
  is applied as a trigger is today: its stages run before the start with no lock and no state, and the start writes the
  `event_deliveries` row in the run's transaction under `start_key`, with outcome `STARTED`; only the input changes
  (FR-042-032). Every other chain (one with a keyed stage, `window`, `debounce` or `flatMap`) is applied in one
  transaction that takes the chain row `FOR SHARE`, then, only when the element is a new key, the chain's usage row
  (FR-042-021), then the key row when the chain has a keyed or windowed stage, then the open windows it changes; it
  applies the stages, writes each emission as an `operator_windows` row (a chain with `window` or `debounce` gathers
  into open windows; any other chain's emission is a window sealed at once with its one member), and inserts the
  `event_deliveries` row for that trigger *with its final outcome* (FR-042-031); a conflict on that insert rolls the
  transaction back and changes nothing. The apply never creates a run: the window start does (FR-042-019). A delivery
  row is never updated; it is deleted with the inbox row as today, or by the delivery retry of FR-042-033. A chain of
  only `filter`, `map` and `flatMap` takes no key row and no key lock. An apply that finds its chain `DRAINING` or gone
  rolls back without counting an attempt; the retried transaction reads the trigger row again and applies the event to
  the trigger's current chain, or, when the trigger is gone, writes `SUPPRESSED` with code `TRIGGER_REMOVED`.
- **FR-042-015** Keyed operators hold their state per chain and key. `distinct {by, horizonMs}` passes an element whose
  canonical `by` value has not passed for this key within `horizonMs`; `skip {count}` drops the key's first `count`
  elements; `take {count}` passes the key's first `count`. Counts belong to the key's *epoch*, which ends when the key
  has had no element for `keyIdleMs` (a chain-level limit of every keyed chain, 1 hour by default). An expression that
  fails ends the element's delivery with outcome `FAILED` and its code; it never passes it. `distinct` marks are rows of
  their own, bounded per key (FR-042-021), and do not hold a key slot.
- **FR-042-016** `window` gathers a key's passing elements. Kinds: `tumbling {sizeMs}`, `session {gapMs, maxWaitMs}` and
  `count {count, maxWaitMs}`. An element's time is its `received_at`.
  - *Tumbling.* The spans are aligned to the epoch of the database clock; a window's `opened_at` is its span's start and
    its deadline the span's end. An element joins the `OPEN` window of its span, or opens it.
  - *Session.* A window's `opened_at` is the least `received_at` of its members and its deadline `closes_at` is
    `min(greatest member received_at + gapMs, opened_at + maxWaitMs)`. An element with time `r` joins every `OPEN`
    window of its key whose interval `[opened_at - gapMs, closes_at]` contains `r`, whether or not that deadline has
    already passed on the database clock; when it falls into two or more, they are merged into the earliest opened (its
    `window_id` survives, the members of the others move to it, the others are deleted, being `OPEN` and without a run),
    and the survivor's `opened_at` and `closes_at` are computed again. With no such window it opens one. So the windows
    a burst forms do not depend on the order in which its events are applied.
  - *Count.* A window's `opened_at` is its first member's `received_at`; it seals when it holds `count` members (at most
    `maxCount`) or at `opened_at + maxWaitMs`, whichever comes first. Its membership follows the order in which events
    are applied, which is the one order-dependent kind; each element still lands in exactly one window.
  - Any change of a window's deadline clears its recorded horizon (FR-042-017).
  - `emit` is `all` (the default: the elements ordered by `(received_at, source, event_id)`), `first` or `last` (the
    least or greatest element in that order). `debounce {quietMs, maxWaitMs}` is `window session` with `gapMs = quietMs`
    and `emit: last`. `ignoreElements` after either makes the emission carry only the summary, and then no element value
    is stored.
  - `overflow` is true on a window sealed by a ceiling (`maxCount`, or the byte ceiling, under `onFull: emit`), false on
    one sealed by its deadline, by `count` or by a flush.
- **FR-042-017** Time is the database clock; an element's time is its `received_at`; `occurred_at` drives no timer. A
  window whose deadline `D` has passed MUST NOT be sealed before every transaction that was running when the pass first
  saw `D` passed has ended: the pass records `seal_horizon = pg_snapshot_xmax(pg_current_snapshot())` on the window in a
  statement whose deadline predicate compares with `now()` (the start of its transaction, earlier than the statement's
  snapshot), never with `clock_timestamp()`, and seals only once `pg_snapshot_xmin(pg_current_snapshot()) >=
  seal_horizon`; the admission statement takes its transaction id before it evaluates `received_at` (ADR-0153, refined
  by ADR-0155). After that it MUST NOT be sealed while the tenant and namespace hold a `PENDING` inbox event of a source
  and type the trigger matches, received at or before `D` (whatever its attempt count; the backlog hold). Both waits end
  at `D + maxHoldMs` (180 seconds by default, at least 90 seconds, which is the dispatcher's claim lease of 60 seconds,
  `CLAIM_LEASE` in `crates/application/src/dispatch.rs`, plus a margin of 30 seconds, and at most 10 minutes): past it
  the window is sealed anyway and counted `sealed_past_hold`. An element applied after the window that covered it was
  sealed is *late*: it joins an `OPEN` window of its key whose span or interval covers it, or opens a new one with the
  span (tumbling) or the interval (session, count) its own `received_at` gives, whose deadline may already be past; it
  is flagged and counted, and never dropped for being late.
- **FR-042-018** Windows are sealed by a scheduler pass fenced as ADR-0020 prescribes, which takes the chain row
  `FOR SHARE`, then the key row, then the `OPEN` window. Sealing and applying are ordered by the key lock; whichever
  commits first wins and the other re-reads (a seal that finds the deadline moved does nothing and is counted). A
  sealed window's membership never changes. An apply, a seal and a flush lock only `OPEN` windows; a discard locks an
  `OPEN` or `REFUSED` one; a retry a `REFUSED` one.
- **FR-042-019** A sealed window starts exactly one run (one per element after `flatMap`, FR-042-020), through the one
  admission port of feature 022 (FR-022-018, FR-022-041): the idempotency scope is the trigger's (`trigger:{id}`) and
  the key is `window-` followed by the first 40 hexadecimal characters of the SHA-256 of the window's `window_id` (and,
  after `flatMap`, of its index), the same form and length as `start_key`; the fingerprint is the SHA-256 of the
  members' `(source, event_id)` in emit order. The run starts on the workflow version whose chain gathered the window,
  with the input of FR-042-032; design 043's check of that input runs in the port as for any start. The start is not
  fenced: it is idempotent by the port's slot-2 receipt and by the guarded write of its window. It reads the held values
  and the window's state in one statement and stops when the state is not `SEALED` or a value is missing; it reads the
  chain's state, and a `REMOVED` chain makes it refuse the window (below); then one transaction: the port takes its
  receipt and creates the run, and in the run's open transaction `RunTx::record_window_start` sets the window `STARTED`
  with the run id, deletes its held values and must update exactly one row (`WHERE window_id = $1 AND state =
  'SEALED'`), else the transaction rolls back and no run is created. When the port answers a replay, a short
  transaction sets `STARTED` with the replayed run id under the same guard. Each outcome has one window outcome:

  | Port or start outcome (feature 022, as on its branch on 2026-11-26; re-checked against the merged port by Task 042-22) | Window |
  |---|---|
  | The run is created, buffered by 022 or not, or created already `FAILED` for a failed key | `STARTED` (a run exists) |
  | The request's key is found (`Admitted::Replayed` or `Admitted::Existing`) | `STARTED` with that run id |
  | `CONCURRENCY_QUEUE_FULL`, `FLOW_CONTROL_LIMIT`, `BinaryCapabilityMissing`, a quota refusal, `DatabaseUnavailable`, `DatabaseLimit` (audited as today's `DatabaseDenied`), any other transient error | stays `SEALED`; retried with a back-off under the same key |
  | Design 043 refuses the input (`RUN_INPUT_INVALID`) | `REFUSED` with `RUN_INPUT_INVALID`, audited, held values deleted |
  | Design 043 cannot use the version's schema (`RUN_INPUT_SCHEMA_UNUSABLE`) | `REFUSED` with `RUN_INPUT_SCHEMA_UNUSABLE`, audited, held values deleted |
  | The version is no longer stored (`MissingReference`) | `REFUSED` with `VERSION_UNAVAILABLE`, audited, held values deleted |
  | The chain is `REMOVED` (its trigger was removed) | `REFUSED` with `TRIGGER_REMOVED`, audited, held values deleted |
  | At the seal of a `flatMap` window: an index input over `MAX_PAYLOAD_BYTES`, or the recorded list over the held-bytes quota | `REFUSED` with `RUN_DATA_LIMIT` or `QUOTA_EXCEEDED`, audited, held values deleted, no run |
  | Still `SEALED` at `start_expires_at` (24 hours after the seal, or after the last retry) | `REFUSED` with `START_EXPIRED`, audited; held values kept until the window is purged (FR-042-022) |

  Only a `START_EXPIRED` window is retryable (back to `SEALED`, by an operator, once the cause is fixed; the retry sets
  `start_expires_at` 24 hours ahead and resets the back-off); the other refusals cannot be changed by a retry, since the
  version and its schema are pinned. An operator may discard any `REFUSED` window. A start that read the chain as live
  before the trigger was removed may still create its one run; that is stated, not prevented. A window sealed at once is
  started by the dispatcher right after its apply commits, in a second transaction, with the same start code as the
  pass, and by the pass when that start does not happen. A chain of only `filter` and `map` keeps today's `start_key`.
- **FR-042-020** `flatMap {select, maxFanOut}` on a trigger starts one run per element of the list `select` yields for an
  emission. `select` is evaluated once: at the seal for a window (with `element` bound to the envelope of FR-042-032
  without the `flatMap` fields), at the apply for a window sealed at once (with `element` bound to the chain's element).
  The list is recorded on the window with `fan_out`, its length; at the same point each index's input is checked with
  design 043's validator against the version's schema. A `select` error or a value that is not a list (`EXPRESSION_ERROR`),
  more than `maxFanOut` elements (at most 100; `OPERATOR_FAN_OUT`) or an index whose input the validator refuses
  (`RUN_INPUT_INVALID`), or an index input over `MAX_PAYLOAD_BYTES` (`RUN_DATA_LIMIT`), makes the window `REFUSED` at
  once with that code, and no run starts. The recorded list is held bytes: the seal (and a flush, and the apply of a
  window sealed at once) checks it under the tenant's `operator_held_bytes` lock, taken last, as an element is checked
  (FR-042-021), and refuses the window `QUOTA_EXCEEDED` (final, held values deleted, audited) when it would pass the
  quota. A seal that only the commit-time flush of the tenant content quota refuses is rolled back, counted `quota_wait`
  and tried again by a later pass; a window still `OPEN` 24 hours after its deadline is refused `START_EXPIRED`. The start then runs one
  transaction per index under the key suffixed with the index, records `started_count`, and becomes `STARTED` when
  `started_count = fan_out`; a crash between two indexes resumes at the next one, and the port's replay answers an index
  already created. A final refusal at index `k` (the version removed, or a refusal the earlier check did not foresee)
  makes the window `REFUSED` with `started_count = k < fan_out`: runs 0 to `k - 1` exist and are named in the audit
  record. A fan-out is not all-or-nothing.
- **FR-042-021** Ceilings each have one outcome. A window reaching `maxCount` (at most 1,000) or its byte ceiling (the
  inline run-input ceiling, 256 KiB, `MAX_PAYLOAD_BYTES` in `crates/application/src/execution.rs`, measured on the input
  FR-042-032 would build) follows `onFull`: `emit` (seal now; the default) or `drop` (the element is dropped with outcome
  `SUPPRESSED`, reason `window_full`). Window inputs stored as blobs are not in release 0.3.0 (Gap 042-015). A new key
  beyond the chain's `maxKeys` (default 10,000, at most 10,000) ends that event's delivery to that trigger at once with
  outcome `FAILED` and code `KEY_LIMIT`: the apply reserves a key slot by `UPDATE operator_chain_usage SET key_count =
  key_count + 1 WHERE chain_id = $1 AND key_count < max_keys`, which serialises new keys of one chain and nothing else;
  it is not retried by the dispatcher, it does not hold the event pending, it is counted once per chain and minute in a
  durable row and audited once per closed minute with that count (section 6), and it can be applied again by an
  operator (FR-042-033). `distinct` marks are bounded per key, under the key lock the apply already holds, by
  `floor(maxMarks / maxKeys)` (a keyless chain's one key gets the whole `maxMarks`; `maxMarks` 100,000 by default and at
  most); a mark that would pass the ceiling fails the element with `KEY_LIMIT` as well. Held bytes are bounded per tenant
  by a quota of their own, `operator_held_bytes`, 1 GiB (1,073,741,824 bytes) by default, which an operator may change
  like any quota: an element that would pass it ends with `FAILED`/`QUOTA_EXCEEDED` and holds nothing, a final outcome,
  not the dispatcher's wait for room. Held values also count against the tenant's retained content
  (`tenant_content_bytes`), which the existing delta triggers enforce at commit: a refusal there is the dispatcher's
  wait for room (the event stays `PENDING` with its attempt count unchanged and is offered again), not a final outcome,
  and while it waits it holds the windows that cover it until `D + maxHoldMs` like any pending event (FR-042-017). The
  number of chains (`operator_chains`, 50 per tenant), the sum of `maxKeys` (`operator_keys`, 100,000 per tenant) and
  the sum of `maxMarks` (`operator_marks`, 1,000,000 per tenant) are tenant quotas checked at publication and at
  `switch`: under the per-scope advisory lock of `crates/infra-postgres/src/quotas.rs` (`lock`), the transaction sums
  the tenant's registered chains and their limits after its own registration and refuses with `QUOTA_EXCEEDED` when a
  sum passes its limit. Window durations are 1 second to 24 hours; a `distinct` horizon is at
  most 7 days.
- **FR-042-022** Chain state is purged by a bounded pass that takes the chain row `FOR SHARE`: a key row that has no
  `OPEN` or `SEALED` window and whose last element is older than `keyIdleMs` (its usage slot released); a mark at its
  horizon; a `STARTED`, `REFUSED` or `DISCARDED` window seven days after it became final (a `REFUSED` window's held
  values with it); a `KEY_LIMIT` count row once its audit record is written. A chain's identity is (trigger, chain
  digest), the digest covering `key`, the stages and the chain-level limits: a republication with the same digest keeps
  the chain and its state; one with another digest registers a new chain and the old one takes no new element while its
  open windows drain at their own deadlines onto the version that gathered them (`DRAINING`); a rollback whose digest
  matches a `DRAINING` chain still stored makes it `ACTIVE` again. When the trigger is removed its chain becomes
  `REMOVED`: its `OPEN` windows are discarded, each audited and counted, its `SEALED` windows are refused by their start
  (FR-042-019), and the chain row goes once nothing is left.
- **FR-042-023** Every row carries tenant and namespace and every statement is scoped by them. Listing chains and
  windows needs `event:read`; a flush, a window retry and a delivery retry need `run:start` and `run:operate`; a
  discard needs `run:operate`. Every such act is audited with its caller; the caller of a flush or a window retry is
  also kept on the window (`requested_by`) only so that the start's `RunStarted` audit record says "requested by X at
  T". The starter of every run a window starts is the engine, as for today's trigger starts, so a caller never becomes
  the initiator of a run created later, after their rights may have changed. Another tenant's chain, window or event
  is indistinguishable from a missing one (`404` with the same body) and the attempt is audited as a probe.
- **FR-042-024** Held element values and `distinct` values are event data, personal data as a whole for feature 016.
  Element values are held in a column of the chain's own table, classified as content, counted in the tenant's retained
  bytes, and deleted in the transaction that starts the window's run (or discards it); they never go to the append-only
  `payloads` table. No key text is stored: a key and a `distinct` value are stored only as a digest of their canonical
  form (FR-042-011, section 4), keyed into their rows, deleted with them; no digest leaves the database in an API
  response, a log, a metric, a span attribute, an audit record or an error. The API names a key and a window by random
  surrogate ids (`key_id`, `window_id`). A chain with a `key`, `distinct`, `skip` or `take` is *value-dependent*: what
  happens to one of its events depends on the values of the others. For such a chain, whatever tells a match from a miss
  or events apart by their key or `distinct` value is itself personal data: a delivery's outcome, error code, reason and
  `windowId`; the chain's `keyCount` and `openWindows`; and every window resource (its `keyId`, members, count,
  instants, deadline, flags and run id). These are returned only to a request that asks `reveal=pii`, through the same
  projector as an event read (`crates/application/src/event_payloads.rs`), which needs `payload:read-internal` and the
  namespace reveal grant and writes `payload.pii_revealed` or `payload.reveal_denied` (feature 016). Without it a
  delivery to a value-dependent chain reads `outcome: "WITHHELD"` with no code, reason or window id, and the chain
  resource answers only that the chain exists, its declaration and its state, with no window list. For a chain that is
  not value-dependent (only `filter`, `map`, `window` or `debounce` without a key, `ignoreElements`, `flatMap`), an
  event's outcome does not depend on another event's value, so its deliveries and windows show outcome, counts,
  instants, flags and run id without reveal, and only the stored reason and `windowId` need `reveal=pii`. The control
  is on reads only: a caller who can publish to a value-dependent trigger and see the runs it starts (`run:read`, or
  only their timing) still learns whether a value was seen before (`distinct`) or whether a key opened a new window;
  that is the semantics of deduplication and grouping, as the inbox's deduplication by event id is, and it is stated as
  Gap 042-018, a residual risk for the owner. None of these values appears in a log, a metric, a span attribute, an
  audit record or an error.

- **FR-042-031** Every way an event's delivery to a chained trigger can end has one outcome, on both paths:

  | Case | Path | Delivery outcome and code |
  |---|---|---|
  | The trigger's own `filter` is false or fails | both | no row, as today |
  | A stage `filter` is false | both | `SUPPRESSED`, no code (reason `filtered`) |
  | A stage expression fails, or a stage `filter` is not boolean | both | `FAILED`, `EXPRESSION_ERROR` |
  | The `key` expression fails, is not a scalar, or is over 512 bytes | locked | `FAILED`, `OPERATOR_KEY_INVALID` |
  | A new key beyond `maxKeys`, or a mark beyond the key's ceiling | locked | `FAILED`, `KEY_LIMIT` |
  | The element would pass the held-bytes quota | locked | `FAILED`, `QUOTA_EXCEEDED` (final) |
  | `distinct`, `skip` or `take` drops the element | locked | `SUPPRESSED` (reason `distinct`, `skip` or `take`) |
  | The element joins or opens a window, or is written as a window sealed at once | locked | `HELD` (also for a window sealed at once, whose run starts right after) |
  | The window is full and `onFull` is `drop` | locked | `SUPPRESSED` (reason `window_full`) |
  | The same `(source, event_id)` is already a member of the window (a re-admission after a short dedupe window) | locked | `SUPPRESSED` (reason `duplicate_member`); the member insert is `ON CONFLICT DO NOTHING` |
  | The chain is `DRAINING` or gone | locked | rolled back, no attempt counted; the retry applies to the current chain, or `SUPPRESSED` with `TRIGGER_REMOVED` when the trigger is gone |
  | This process does not use `trigger-operators-v1` (FR-042-034) | locked | rolled back, no attempt counted; the event stays pending for a process that does |
  | A transient store error | both | rolled back; the dispatcher's existing retry |
  | A chain of only `filter` and `map` starts its run | stateless | `STARTED` |

  `event_deliveries` gains `outcome`, `window_id` and `error_code`, all nullable: `NULL` for a delivery of a trigger
  without a chain, of a wait, of design 043's refused delivery, and for every row an older writer inserts. `outcome` is
  checked to be one of `STARTED`, `HELD`, `SUPPRESSED`, `FAILED`; `error_code` one of `EXPRESSION_ERROR`,
  `OPERATOR_KEY_INVALID`, `KEY_LIMIT`, `QUOTA_EXCEEDED`, `TRIGGER_REMOVED`; a reason column the closed values `filtered`,
  `distinct`, `skip`, `take`, `window_full`, `duplicate_member`. For a value-dependent chain (FR-042-024) the outcome,
  code, reason and `window_id` are shown only under `reveal=pii`, otherwise the delivery reads `outcome: "WITHHELD"`;
  for another chain the stored reason and `window_id` are shown only under `reveal=pii`, otherwise the reason of a
  `SUPPRESSED` delivery reads `operator` and `windowId` is absent.
- **FR-042-032** The run input of a trigger chain is one JSON object of a shape fixed per emission kind. The summary is
  `{"windowId": uuid, "keyId": uuid or null (keyless chain), "openedAt": RFC 3339 UTC, "sealedAt": RFC 3339 UTC,
  "count": integer, "firstReceivedAt": RFC 3339 UTC, "lastReceivedAt": RFC 3339 UTC, "late": boolean, "overflow":
  boolean}`. A member is `{"source": string, "eventId": string, "receivedAt": RFC 3339 UTC, "value": the element}`.

  | Emission | Run input |
  |---|---|
  | A chain of only `filter` (no `map`) | today's event document, unchanged |
  | A chain of only `filter` and `map` | `{"value": <the map's value>}` |
  | `emit: all` | `{"window": <summary>, "events": [<member>, …]}` in emit order |
  | `emit: first` or `last` | `{"window": <summary>, "event": <member>}` |
  | After `ignoreElements` | `{"window": <summary>}` |
  | `flatMap`, index `i` | `{"window": <summary>, "index": i, "value": <element i>}` (a window sealed at once has a one-member summary) |

  For example, CUJ 3's run receives `{"window": {"windowId": "0193…", "keyId": "0193…", "openedAt":
  "2026-12-01T10:00:00.120Z", "sealedAt": "2026-12-01T10:00:33.500Z", "count": 10, "firstReceivedAt": "…",
  "lastReceivedAt": "…", "late": false, "overflow": false}, "event": {"source": "shop", "eventId": "u-12",
  "receivedAt": "…", "value": {"event": {…}, "data": {"orderId": 81, …}}}}`. Publication (`DefinitionService`, slice 2)
  refuses a chain whose version declares an `inputSchema` that does not accept a one-member sample of every shape the
  chain can emit, with `value` null and every instant the epoch (`OPERATOR_INPUT_SCHEMA`); design 043's check of the real
  input at the start stays. The `window`, `event` and `events` fields of these inputs, and the `value` of a `flatMap`
  index, are `pii` whatever the version's schema declares, as every field of a database result is
  (`crates/domain/src/classification.rs`, `DATABASE_RESULT`): a constant `OPERATOR_WINDOW_INPUT` beside it fixes their
  level, and a looser annotation in the schema does not lower it (the stricter level wins, so no schema is refused for
  it), so a run read without the reveal grant never shows a `keyId` or a member list.
- **FR-042-033** A delivery retry applies one event again to one trigger. It accepts only a `FAILED` delivery of the
  named trigger, addressed by namespace, source, event id and trigger id; it needs `run:start` and `run:operate`. A
  control plane that does not use `trigger-operators-v1` (FR-042-034) refuses it with the switch's typed error
  (`OPERATOR_UNSUPPORTED`). One transaction locks the inbox row `FOR NO KEY UPDATE` (which serialises retries and blocks
  the purge, and does not conflict with the `FOR KEY SHARE` a dispatcher's delivery insert takes through the foreign key,
  so a retry and a re-dispatch of the same event cannot deadlock), checks that the delivery is `FAILED` (otherwise
  `409`, `DELIVERY_NOT_RETRYABLE`), deletes it and applies the event to that trigger alone with the dispatcher's
  application method; it never returns the inbox row to `PENDING` and never touches another consumer of the event. No
  inbox row in the caller's scope (purged, never admitted, another tenant's), or no delivery row of the named trigger,
  answers the uniform `404` and writes the probe audit. The act is audited `operator.delivery_retried` with its
  caller; the run it may cause is started by the engine (FR-042-023).
- **FR-042-034** The two switches are capabilities of the instance registry under design 053 (Requirements 053-006 and
  053-007). The engine's trigger switch is `SAGAT_TRIGGER_OPERATORS_ENABLED`, beside the control plane's
  `SAGAT_COLLECTION_OPERATORS_ENABLED`; a process uses `trigger-operators-v1` only when `ServingGate::uses` says so
  (design 053 adds `uses`, Task 053-5; until it merges only `ServingGate::requires` exists, and slices 2 to 5 wait for
  it, section 10). The capability gates only what creates new chain state: a new apply, a delivery retry, a new
  publication or `switch`. A dispatcher that does not use it never starts a run for a chained trigger: that trigger's
  dispatch of the event rolls back without counting an attempt (FR-042-031). Sealing, starting, the `TRIGGER_REMOVED`
  refusal, the expiry and the purge are not gated: the `operator_windows` pass runs them in every binary that has the
  code, whatever the switch, so a deactivation can drain and held values reach their deletion. Publication and `switch`
  of a version with a chained `forEach` or a trigger chain take the capability's fence of form (a) of Requirement
  053-006 in their transaction: an owner-owned definer function per capability that takes the requirement row `FOR KEY
  SHARE` and answers whether it is `active`. The `FOR KEY SHARE` is design 053's (Requirement 053-006): the code's
  `lease_withdrawal_required()`, the precedent, takes `FOR SHARE` until Task 053-5 changes it. The control plane refuses
  to start with the trigger switch on until Task 042-22's start is built, as D042-103 did for collections.

**Surfaces**

- **FR-042-025** The collection chain counts elements per operator and outcome in
  `operator_elements_total{surface="collection", operator, outcome}`; its failure codes are members of
  `WorkflowPrimitiveRejectionCode::ALL` (`crates/telemetry/src/metrics.rs`) and of the run failure codes of
  `events.asyncapi.yaml` and `docs/sdk-compatibility.md`. For triggers: an event's delivery carries the outcome of
  FR-042-031, the pass has a lag metric, and every metric of section 9 has closed labels.
- **FR-042-026** The management API lists a namespace's chains and a chain's windows (state, key id, count, deadline,
  the events it holds by reference), flushes, discards or retries one window, and retries one delivery (section 9).
- **FR-042-027** The Studio edits a trigger's key, chain and chain-level limits as an ordered list of typed stages with
  the publication rules applied as the author types, shows open windows with their deadline and the pass's lag, and
  shows on an event's page the outcome of each delivery. Collection operators are shown on a `forEach` node once the
  Studio knows that node (G042-004).
- **FR-042-028** The six SDKs build both declarations with one set of names from an operator factory (`ops.filter(...)`
  in each language's casing), carried by each SDK's collection options without breaking a present caller, emit
  identical IR for the same chain, and pass common conformance scenarios.
- **FR-042-029** A chain decides *which events ask for a run and with what input*; features 022 and 018 decide *when a
  requested run may start*. No operator declares a rate or a limit (section 8).
- **FR-042-030** A trigger without `key` and `operators` MUST cost what it costs today: no added statement, row or lock,
  held by a budget test.

**Success criteria**

- **SC-042-001** With `debounce {quietMs: 30000, maxWaitMs: 300000}` and the default `maxHoldMs` (180 seconds), 500
  events for one key received within 3 seconds start exactly one run whose input carries the event with the greatest
  `(received_at, source, event_id)`, when every event is dispatched twice and the process is killed once at each of the
  failpoints of Task 042-27: before and after the apply's key insert, the window write and the delivery insert; between
  the seal's horizon record and the seal; between the port's receipt and `record_window_start`; after the claim of the
  first batch, so that its events are stranded for the claim lease while a second replica applies later ones; and with
  two replicas applying interleaved batches, forced with a barrier.
- **SC-042-002** The seal and an arriving element are forced in both orders with a held lock, and each order ends in the
  state FR-042-018 states, with no element lost or in two windows; two session windows bridged by one element are merged
  into one.
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
  rest end `FAILED`/`KEY_LIMIT` at once, no event stays pending for it, no deadlock is reported, a second tenant's
  dispatch latency at the 99th percentile during the flood stays within twice its value without the flood and at most
  1 second, on the stated environment of Task 042-31, and no table grows beyond its ceiling. Then 2,000 keys arriving one
  after another, each idle past `keyIdleMs`, are all served.
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
  (O042-005). A stateless chain's `map` value is wrapped as `{"value": …}` (FR-042-032).
- **G042-002** Whether the seal and the run start can share one transaction through the admission port, or must be two
  with the window `SEALED` between them. Closed 2026-11-26: two transactions (Decision 042-162, section 8).
- **G042-003** `published_collection_definition` refuses `triggers` on a workflow with a `forEach`, so a window cannot
  yet feed a collection; that gate belongs to design 039, which takes it over (Gap 039-006 there).
- **G042-004** The Studio's node registry does not know `forEach`, so collection operators have no canvas yet.
- **G042-005** The dispatcher is one queue for all tenants (G88 of feature 004); a key flood is bounded per event, not
  isolated. The held-bytes quota check serialises, per tenant, the applies that add held bytes (one advisory lock per
  tenant, section 5), and the new-key reservation serialises the new keys of one chain. SC-042-006 and Task 042-33
  measure both.
- **G042-006** A binary from before the instance registry does not register, so the registry cannot refuse it. Narrowed
  2026-11-27: under design 053 the upgrade into release 0.3.0 is a stopped upgrade and a rollback never crosses it, so
  such a binary meets a chain only in a deployment that breaks that procedure; for it the runbook's rule is the only
  guard (an older dispatcher would start one run per event).
- **G042-007** One hot key is serial by construction (one row lock per element), and the new keys of one chain are
  serial on its usage row (FR-042-021). The rate one key and one chain sustain is reported (Task 042-33).
- **G042-008** A collection that fails at entry stops the run; its `onError` is not taken, as for a failing `items`
  today. Routing entry failures to `onError` is a follow-up of its own for feature 014's collections, with or without
  operators.
- **G042-009** The wall-time budget is the one ceiling that depends on the machine: on a starved replica it could fail a
  chain another replica would finish. The deterministic ceilings bound the work so that it should never fire; when it
  does it is counted separately (`reason="wall_time"`) and treated as a defect to explain.
- **G042-010** Beyond plus or minus 2^53 an integer and a double that `==` calls equal (it compares through `f64`) may
  have different canonical forms, so `distinct` keeps both, and two such keys are two keys.
- **G042-011** Feature 022's D022-018 says a window whose run 022 buffers "stays sealed". This design records it
  `STARTED`, because the run exists (FR-042-019); the 022 text is to be aligned (T042-3).
- **G042-012** The back-off of a collection entry that waits for storage quota is a per-replica hint (section 3); a
  replica that loses it evaluates the chain once more, which costs time and changes no outcome.
- **G042-013** `take` after `flatMap` does not rescue a `flatMap` whose output crosses a ceiling: each stage's output is
  materialised and bounded before the next stage runs (FR-042-006).
- **G042-014** The chain input ceiling (1,000) is below the 10,000 elements validation accepts for `maxItems`, so a
  `filter` cannot reduce a blob of 5,000 elements to 500. Raising it waits for the measurement of T014-042 that
  OD-2026-11-13-1 deferred.
- **G042-015** A trigger window's input is bounded by the inline run-input ceiling (256 KiB); a window input stored as a
  blob is not in release 0.3.0 (Owner decision 2026-11-27-427). A follow-up designs the blob path (object-store write
  before the run transaction, its failure and its quota).
- **G042-016** The owner row OD-2026-11-13-15 has the Status "Decided; not built", which `scripts/check-design-docs.py`
  does not read as `Decided`, so it cannot open a gate. The tasks it gated cite Owner decision 2026-11-27-432, which
  records the same answers; the owner may correct his row, after which the gates may cite it again.
- **G042-017** A key or a `distinct` value is kept as a digest (section 4). A reader of the database or of a backup can
  recover a low-entropy one (an order number, an email address) by trying values; once the event's inbox row is gone and
  no run input holds the value, the digest is its only trace, for as long as its row lives (a key row until idle past
  `keyIdleMs` with no `OPEN` or `SEALED` window, a mark at most 7 days, a window's digest 7 days after it became final).
  The tenant and the chain in the digest's input stop linkage across tenants and chains and precomputed tables, not a
  targeted guess. A security residual risk for the owner (Decision 042-180).
- **G042-018** A value-dependent chain (FR-042-024) answers the same to a read without the reveal grant whether a value
  matched or not, but the run it starts does not hide itself: a caller who can publish to the trigger and see the runs
  it starts (with `run:read`, or only their timing) learns whether a `distinct` value was seen within the horizon (no
  run) or whether a key opened a new window (an earlier run, or none until the window seals). This is the semantics of
  deduplication and grouping, as the inbox's deduplication by event id already tells a publisher whether an event id
  was seen; no read control removes it. A security residual risk for the owner (Decision 042-181).

## Owner decisions needed

**Answered.** The owner answered O042-001 to O042-009 on 2026-11-13 with "Aceitar todas": each recommendation of the
first draft is the decision, recorded as OD-2026-11-13-15, and accepted ADR-0086 and ADR-0087 with "Aceito tudo",
recorded as OD-2026-11-13-17. Tasks that depend on an answer carry `Owner-gate: OD-2026-11-27-432`, the delegate row
that covers them (Gap 042-016).

| Question | Decision (OD-2026-11-13-15) | Options not taken |
|---|---|---|
| O042-001 Which surface the sentence means | (c) Both: triggers and `forEach`, one vocabulary, two switches | (a) events only; (b) collections only |
| O042-002 What goes in release 0.3.0 | (b) The collection operators only: slices 0 and 1; slices 2 to 5 a later release. Widened by the owner on 2026-11-24 to (d), all of them (Owner decision 2026-11-24-1) | (a) nothing; (c) collections and trigger debounce |
| O042-003 Operators on `waitEvent` | (a) Not in the first delivery; `filter` on a wait as a follow-up | (b) `filter` now; (c) the whole chain |
| O042-004 The time base | (a) Arrival time on the database clock only | (b) also event time with an allowed lateness |
| O042-005 Object construction in `map` | (a) The language stays as it is; `map` selects or computes | (b) an object literal |
| O042-006 The first set and what is deferred | The nine operators of FR-042-001; the deferred list of the catalogue | — |
| O042-007 The numbers | Chain length 8; `maxCount` 1,000; window durations 1 s to 24 h; `maxKeys` 10,000 per chain; 50 chains and 100,000 keys per tenant; `maxFanOut` 100; `distinct` horizon up to 7 days; published limits a measurement may raise | — |
| O042-008 Showing a key | (a) Not the key, unless the caller may reveal personal data. Refined on 2026-11-27: no key text is stored; the caller who may reveal reads it from a member event (Owner decision 2026-11-27-424) | (b) the key to any `event:read` holder |
| O042-009 A new key beyond `maxKeys` | (a) Fail the delivery, visible and replayable | (b) a run per event; (c) drop |

**Decided under the owner's delegation** (Owner decision 2026-11-24-4); each is a delegate row of
`docs/owner-decisions.md` with its reason:

| Item | Decision | Why |
|---|---|---|
| The refinement of O042-009 (Owner decision 2026-11-26-130, refined by 2026-11-27-429) | A new key beyond `maxKeys` ends that event's delivery to that trigger at once with `FAILED`/`KEY_LIMIT`, visible, retryable by an operator as a targeted apply of that trigger alone (FR-042-033); no dispatcher retry, no dead letter | A retry would hold the event pending for the dispatcher's whole schedule; returning the event to `PENDING` would offer it to every other consumer again |
| The refinement of O042-008 (Owner decision 2026-11-26-131, refined by 2026-11-27-424) | The API names a key by a random surrogate id; no key text and no digest is shown or stored | A digest of a low-entropy key is reversed by trying values; a caller who may reveal reads the key from an event, audited |
| `maxMarks` (Owner decision 2026-11-26-132, refined by 2026-11-27-426) | 100,000 per chain by default and at most, bounded per key by `floor(maxMarks / maxKeys)`; at most 1,000,000 over a tenant's chains | A per-key bound is checked under the key lock the apply holds, so no chain-wide row is written per element |
| The held-bytes quota (Owner decision 2026-11-26-133) | `operator_held_bytes`, 1 GiB per tenant by default | 4,096 windows at their 256 KiB ceiling; an operator raises it like any quota |
| `maxHoldMs` (Owner decision 2026-11-26-134, refined by 2026-11-27-425) | 180 seconds by default, at least 90, at most 10 minutes; it caps the horizon wait as well as the backlog hold, and the hold covers every pending event | The claim lease is 60 seconds, so a shorter hold lets a crashed apply's event miss its window; an uncapped horizon lets one long transaction hold every window |
| `admissionGraceMs` (Owner decision 2026-11-26-135) | Withdrawn: a window is sealed past the transaction horizon of its deadline (ADR-0153, Owner decision 2026-11-26-136) | On PostgreSQL 16 no timeout bounds an admission transaction, so no fixed grace is a proof |
| The start's dependency on feature 022 (Owner decision 2026-11-27-420) | Tasks 042-3 and 042-22 start after feature 022's admission port merges | A second creator would bypass 022's limits |
| Every stateful emission is a window (Owner decision 2026-11-27-421) | The apply never creates a run | The apply's delivery row and today's start would write the same row |
| The HMAC key (Owner decision 2026-11-27-422, superseded by 2026-11-27-424) | No keyed hash and no definer function for it: SHA-256 digests over the tenant, the chain and the value, which never leave the database (refined by Decision 042-180) | The keyed hash's key lived in the same database, so it did not stop a database reader; the residual of a low-entropy value is stated in Gap 042-017 |
| Gap 042-006 (Owner decision 2026-11-27-423) | Reopened, then narrowed by design 053's stopped upgrade into 0.3.0 | A binary from before the registry does not register |
| Window inputs (Owner decision 2026-11-27-427) | Bounded by the inline run-input ceiling; blob inputs deferred (Gap 042-015) | The CUJs fit in 256 KiB; a blob path is a design of its own |
| ADR-0155 (Owner decision 2026-11-27-428) | Accepted: window identity, session merging, the unfenced start, plain digests, the usage row, the capped hold | It corrects decisions of ADR-0086 and ADR-0153 that the reviews of 2026-11-27 found unsound |
| Session windows (Owner decision 2026-11-27-430) | An element joins every open window of its key whose interval covers it, and bridged windows merge | Otherwise the order in which events are applied splits one burst into two runs |
| Task 042-56's gate (Owner decision 2026-11-27-431) | Its own delegate row | OD-2026-11-26-136 does not name it |
| Retention and idle keys (Owner decision 2026-11-27-433) | `keyIdleMs` 1 hour by default for every keyed chain; final windows purged 7 days after they end | Without them a key row and its windows would live for ever |
| Fan-out (Owner decision 2026-11-27-434) | `select` evaluated and each index's input checked at the seal; a later refusal leaves a partial fan-out, recorded | One transaction per index cannot be all-or-nothing |
| The trigger switch and the cut order (Owner decision 2026-11-27-435) | The control plane refuses the trigger switch until the start is built; a cut order inside slices 2 to 5 | A release that cannot start a window must not hold events |

**Still needed from the owner.** Two security residual risks are the owner's to accept, and no delegate row accepts
them: Gap 042-017 (a database or backup reader can recover a low-entropy key or `distinct` value from its digest by
trying values) and Gap 042-018 (a publisher who sees the runs a value-dependent chain starts learns whether a value was
seen or a key is new). The design is otherwise `Approved` by review (constitution, Approval), not by an owner
decision.

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
    [{debounce: {quietMs: 30000, maxWaitMs: 300000}}]}` with the default chain limits (`maxHoldMs` 180 seconds,
    `keyIdleMs` 1 hour), and a provider that sends twelve updates for order 81 in three seconds, two of them twice.
*   **When**: the events are admitted, and one dispatcher replica is killed while applying the seventh, after it had
    claimed the seventh to the twelfth.
*   **Then**: the inbox absorbs the two duplicates; the first six deliveries say `HELD`; the seventh to the twelfth wait
    in the inbox for the claim lease (60 seconds) and, being pending and received before the deadline, hold the window
    (the backlog hold); once they are applied they join the same session window, which then holds ten updates; thirty
    seconds after the last of them was received, once the transactions running at the deadline have ended and nothing
    is pending, the pass seals the window and one run starts with the update of the greatest receipt as `event` and
    `window.count` 10. Each update's event page shows its delivery `HELD` with the window's id, and the window names the
    run. Without the kill the run starts about thirty seconds after the last arrival.

### CUJ 4: A batch every five minutes (slices 2 to 5)

*   **Given**: a `message` trigger with `operators: [{filter: {where: "data.kind == 'reading'"}}, {map: {select:
    "data.value"}}, {window: {kind: tumbling, sizeMs: 300000, maxCount: 1000, emit: all}}]` and no key.
*   **When**: 2,400 readings arrive in one five-minute interval.
*   **Then**: three runs start, two with 1,000 values (sealed by `onFull: emit` at `maxCount`, `window.overflow` true) and
    one with 400 at the boundary (`window.overflow` false). Each is admitted by feature 022's rules like any other start.

### CUJ 5: A flood of keys (slices 2 to 5)

*   **Given**: a chain with `maxKeys` 1,000 and a caller that sends events with a random key each.
*   **When**: the thousand-and-first key arrives.
*   **Then**: that event's delivery to the trigger ends `FAILED`/`KEY_LIMIT` at once and the event leaves the backlog;
    the thousand keys keep their windows; one audit record per chain and minute counts the refusals; other tenants'
    events are still dispatched. An hour after a key's last element, with no open or sealed window left, its row is
    purged and its slot serves a new key.

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
told (which windows an element joins, a merge, a session deadline that moves, `onFull`, a late element, the epochs of
`skip` and `take`, the horizon of `distinct`, the per-key mark ceiling, whether a due window may seal), with property
tests over generated element sequences and no I/O, clock or SQL; `ExpressionEvaluator` gains one method,
`trigger_chain(chain, event) -> Result<TriggerChainOutput, ChainFailure>`, which parses the event document once,
evaluates the trigger's key and its stateless stages, and answers the key's canonical bytes, the element that reaches
the first stateful stage (or that the chain drops, with the stage), and, for `distinct`, the canonical `by` value; its
compiled expressions are cached per chain digest (at most 256 chains per process); the dispatcher calls it in a
blocking task under the same 1-second budget as the collection chain. `crates/application/src/operators.rs` holds the
store port `OperatorChainStore` (apply in the dispatcher's transaction, due windows, seal, start, flush, discard, retry,
purge) that the dispatcher, the pass and the management API call and `crates/infra-postgres` implements. No rule lives
in SQL beyond the statements that read and write the rows the module decided.

**The chain lifecycle follows every change of a trigger row, not only publication.** Activation, rollback and promotion
rewrite `workflow_triggers` through `register_triggers` (`crates/infra-postgres/src/definitions.rs`), fed by
`triggers_of` (`crates/application/src/dispatch.rs`), whose `TriggerRegistration` gains the chain and its digest. The
registration and draining of chains (FR-042-022), the tenant quotas of FR-042-021 and the capability fence of
FR-042-034 run in `switch` as well as at publication; a rollback onto an older chained version reactivates its chain
when it is still stored, registers it otherwise (or is refused by a quota or the fence) and drains the newer one,
tested. The dispatcher reads a trigger's chain from `workflow_triggers.chain`, decoded by the codec of Task 042-43.

**The switches.** `DefinitionService` gains `with_collection_operators_enabled` (built) and, with slice 2,
`with_trigger_operators_enabled`, beside `with_lake_enabled`, and refuses a definition that uses a switched-off field
with `IrError::CollectionOperatorsDisabled { node }` (and its trigger twin), in the same place `refused_lake_node` is
checked. Nothing in `convert::*` looks at a switch, because `parse_stored` goes through it: switching a capability off
stops new publications and never makes a stored definition unreadable (tested through `parse_stored` with the switch
off). Amended after the slice 0 review (D042-105, D042-106): on the control plane only a new publication reads the
publication switch. A publication asked again with its own idempotency key is answered by the store's replay before the
switch is read, and activating, rolling back or promoting to a namespace that already runs the document move a pointer
to an existing version and never read it. On the execution engine, `SAGAT_TRIGGER_OPERATORS_ENABLED` turns on the
dispatcher's apply and the capability listing only: the process lists `trigger-operators-v1` only then (FR-042-034),
and a dispatcher without it never starts a chained trigger per event. The `operator_windows` pass (seal, start,
refusal, expiry and purge) runs in every binary that has the code, whatever the switch.

**Old readers.** The schema is closed, so a binary without the field fails `parse_stored` with `IrError::Schema`: it
refuses, it does not ignore. That is why section 10 forbids a mixed fleet with a binary from before 0.3.0.

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

The parameters of a trigger chain (lane A's schema, lane B's apply; durations in milliseconds):

| Parameter | Where | Required, default | Range |
|---|---|---|---|
| `key` | trigger | optional; absent is one key for the trigger | an expression; its value a scalar of at most 512 bytes in canonical form |
| `chainLimits.maxKeys` | trigger | default 10,000 | 1 to 10,000 |
| `chainLimits.maxMarks` | trigger | default 100,000 | 1 to 100,000; per key `floor(maxMarks / maxKeys)`, at least 1 |
| `chainLimits.keyIdleMs` | trigger | default 3,600,000 (1 hour) | 1,000 to 604,800,000 (7 days) |
| `chainLimits.maxHoldMs` | trigger | default 180,000 | 90,000 to 600,000 |
| `filter {where}`, `map {select}` | stage | `where`, `select` required | expressions |
| `distinct {by, horizonMs}` | stage | `by` optional (the element), `horizonMs` required | 1,000 to 604,800,000 |
| `skip {count}`, `take {count}` | stage | `count` required | 0 to 1,000 (`take {count: 0}` refused) |
| `window {kind: tumbling, sizeMs}` | stage | `sizeMs` required | 1,000 to 86,400,000 |
| `window {kind: session, gapMs, maxWaitMs}` | stage | both required, `gapMs` at most `maxWaitMs` | 1,000 to 86,400,000 each |
| `window {kind: count, count, maxWaitMs}` | stage | both required, `count` at most `maxCount` | `count` 1 to 1,000; `maxWaitMs` 1,000 to 86,400,000 |
| `window` and `debounce`: `maxCount`, `onFull`, `emit` | stage | `maxCount` default 1,000; `onFull` default `emit`; `emit` default `all` (`debounce` is always `last`) | `maxCount` 1 to 1,000; `onFull` `emit` or `drop`; `emit` `all`, `first`, `last` |
| `debounce {quietMs, maxWaitMs}` | stage | both required, `quietMs` at most `maxWaitMs` | 1,000 to 86,400,000 each |
| `flatMap {select, maxFanOut}` | stage | `select` required; `maxFanOut` default 100 | 1 to 100 |
| `ignoreElements {}` | stage | no parameter | after `window` or `debounce` only |

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

**Failure at entry fails the run.** The scheduler routes a `ChainFailure` through `stop_collection_for_chain` with its
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
`enter_collection` and the restore learn the variant and check the stored
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
`scheduler_tick_seconds`. No table and no new journal event type; the one small migration is Task 042-56's
capability.

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

One additive migration (expand only, Requirement 053-001). All tables carry `tenant_id` and `namespace_id` first in their
primary key. A chain's and a window's shard is `run_shard` of its id (the last byte of a UUIDv7 modulo 64, as for runs;
`Shard::of` in the domain), so the replica that holds the shard serves it.

| Table | Key | Holds | Removed |
|---|---|---|---|
| `workflow_triggers` (columns added, nullable) | — | `chain` (canonical), `chain_digest`, `chain_id` | with the trigger |
| `operator_chains` | `chain_id` (random at first registration) | trigger, workflow, the version that registered it, `chain_digest`, state `ACTIVE`, `DRAINING` or `REMOVED`, `max_keys`, shard | drained or removed and empty |
| `operator_chain_usage` | `chain_id` | `key_count`, `max_keys` | with the chain |
| `operator_keys` | `chain_id, key_digest` | `key_id` (random), epoch start, `skip` and `take` counters, `mark_count`, last element time | no `OPEN` or `SEALED` window and idle past `keyIdleMs` |
| `operator_marks` | `chain_id, key_digest, value_digest` | `expires_at` | at `expires_at` |
| `operator_windows` | `window_id` (a UUIDv7 drawn when the window opens) | `chain_id`, `key_digest`, `key_id`, state, `opened_at`, `closes_at`, `max_close_at`, `seal_horizon` (`xid8`), count, bytes, first and last `received_at`, late and overflow flags, `version_id`, `run_id`, `fan_out`, `fan_out_values` (bytea, content), `started_count`, attempts, `next_attempt_at`, `start_expires_at`, last error code, `requested_by`, `final_at`, shard | seven days after `final_at` |
| `operator_window_members` | `window_id, source, event_id` | `received_at`, late flag, `value` (bytea, the projected element; content) | `value` deleted at start; the row with the window |
| `operator_key_limit_counts` | `chain_id, minute` | the number of `KEY_LIMIT` refusals in that minute | once its audit record is written |

Indexes: the open windows of a key (`chain_id, key_digest` where `state = 'OPEN'`), the due windows per shard
(`shard, closes_at` where `state = 'OPEN'`), the windows to start per shard (`shard, next_attempt_at` where `state =
'SEALED'`), the final windows per shard (`shard, final_at`). On `inbox_events`, the partial index
`inbox_events_pending_match_idx (tenant_id, namespace_id, source, event_type, received_at) WHERE status = 'PENDING'`
serves the backlog hold; it costs one index entry per admitted event, kept while the event is pending (also while it
waits for room with its attempt count unchanged), and is measured in the statement budget of Task 042-19 (one more
index write on admission, no more statements).

**Digests, not keyed hashes.** A key and a `distinct` value are stored only as a digest: SHA-256 over the
length-prefixed parts `tenant_id`, `chain_id` and the canonical bytes of the value (FR-042-011; the absent key is the
canonical `null`), each part preceded by its length as eight big-endian bytes, as `start_key` builds its digest
(`crates/application/src/dispatch.rs`). The tenant and the chain in the input mean that one value gives different
digests in two tenants or two chains, so digests cannot link tenants and one table of precomputed digests serves one
chain only. No key text is stored. The digests are never returned by an API, written to a log, a metric, a span or an
audit record, or put in an error, and they are deleted with their rows. The threat model, stated as it is: the API
never shows a digest, so no caller can reverse one through it; a reader of the database or of a backup can recover a
low-entropy key or `distinct` value (an order number, an email address) from its digest by trying values, and once
the event's inbox row is gone (its dedupe window ended) and, for a suppressed element, no run input holds it, the
digest is the only trace of that value. That trace lasts only as long as its row: a key row until it is idle past
`keyIdleMs` with no `OPEN` or `SEALED` window, a mark until its horizon (at most 7 days), a window's `key_digest` until
the window is purged 7 days after it became final. This is a residual risk (Gap 042-017), not a guarantee. No
`pgcrypto`, no key table and no `SECURITY DEFINER` function of this design computes the digests; the apply computes them
in the process.

`value` and `fan_out_values` are deleted (set to `NULL` and their bytes released from the tenant's usage) in the
transaction that starts the window's run, or the last index's run after `flatMap`, or discards or refuses it, except a
`START_EXPIRED` refusal, whose values stay until the window is purged; with `emit: first` or `last` one member keeps a
value, and after `ignoreElements` none has one. `event_deliveries` gains `outcome`, `window_id`, `error_code` and
`reason` (nullable, FR-042-031), written once by the insert and never updated; it keeps its key, its cascade from
`inbox_events` and therefore its lifetime, while a member row keeps its own `(source, event_id, received_at)` so a
window outlives its events' inbox rows.

Window states: `OPEN` → `SEALED` → `STARTED`; `SEALED` → `REFUSED`; a `REFUSED` window with `START_EXPIRED` → `SEALED`
(a retry); `OPEN` or `REFUSED` → `DISCARDED`; an `OPEN` window merged into another is deleted. Only `OPEN` accepts
members. A `SEALED` window is written only by its start and by the pass's short back-off, expiry and refusal
transactions, which hold no other operator row; this is what lets the start write it in the run's transaction (section
6, ADR-0155).

Grants to `sagat_app` in the migration's grant block; the down migration refuses while a row exists;
`storage-classification.json` gets an entry per variable-size column, with its class among the four the file defines:
the member `value` and the `fan_out_values` are `content` (counted in the tenant's retained bytes by the quota-delta
triggers, `.claude/skills/migration`); the digest columns (`key_digest`, `value_digest`, 32 bytes each, fixed size) are
`protected_system` with the bound of their fixed length, and are personal data deleted with their rows (FR-042-024);
`error_code` and `reason` are `protected_system` (closed vocabularies). **The held-bytes counter.** `operator_held_usage
(tenant_id, held_bytes)` holds one row per tenant. It is maintained only by an owner-owned trigger function,
`operator_held_usage_delta()`, `SECURITY DEFINER` with `search_path` pinned to `pg_catalog, pg_temp` and every name
schema-qualified, which fires on every insert, update of `value` or `fan_out_values` and delete of
`operator_window_members` and `operator_windows` and adds the change of held bytes to a per-transaction delta row
(`operator_held_deltas (transaction_id, tenant_id, held_delta)`, keyed by `pg_current_xact_id()` as
`sagat_quota_add_delta` keys `quota_transaction_deltas`), never to the counter itself; a deferred constraint trigger,
`operator_held_flush()`, owner-owned and of the same shape as `sagat_quota_flush_deltas()`, adds each pending delta to
the counter row under that row's lock at commit and deletes the delta. So no transaction locks the counter before
commit, the counter row is the last lock a transaction takes (slot 8b, recorded beside the usage rows in ADR-0050's
recorded deviation for design 042), and the start, a discard, a refusal or the purge that deletes held values writes
only a delta row of its own transaction. `sagat_app` has `SELECT` on the counter and no write on it or on the delta
table. The apply checks it at step 5 (section 5): the committed counter row plus this transaction's pending delta,
read once. The capability fences of FR-042-034 are two
owner-owned definer functions, `collection_operators_required()` and `trigger_operators_required()`, of the shape of
`lease_withdrawal_required()` (`search_path` pinned to `pg_catalog, pg_temp`, every name schema-qualified, revoked from
`PUBLIC`, executable by `sagat_app`), each taking its requirement row `FOR KEY SHARE`. These four (the two fences, the
delta trigger function and its flush) are the only definer functions of this design and are in the privilege audit's
allow-list.

### 5. Trigger chains: applying an event (slices 2 and 3)

A chain of only `filter` and `map` is evaluated by `trigger_chain` before today's start and changes nothing else
(FR-042-014). For any other chain, in `EventDispatcher`, one transaction, in this lock order (slot 3 of ADR-0050
decision 7: chain, usage, key, windows; then slot 8b):

1. call `trigger_chain` (the trigger `filter`, the `key`, the stateless prefix), with no lock held; a refusal there ends
   with the outcome of FR-042-031;
2. take the `operator_chains` row `FOR SHARE` and read its state (`DRAINING`, `REMOVED` or gone: FR-042-014);
3. if the chain has a keyed or windowed stage: read the key row `FOR UPDATE`; when it does not exist, take a savepoint,
   reserve a slot on `operator_chain_usage` by the conditional update of FR-042-021 (none left: `FAILED`/`KEY_LIMIT`,
   the minute's count upserted, nothing else written) and insert the key row with `ON CONFLICT DO NOTHING RETURNING`;
   when nothing is returned (a concurrent apply of the same new key committed first), roll back to the savepoint, which
   releases the reservation, and read the key row `FOR UPDATE` again. No unique violation is raised, so no key digest
   can reach the server log, and the event uses no attempt; apply `distinct` (its mark under the key's ceiling), `skip`
   and `take`, in that order;
4. for a chain with `window` or `debounce`: lock the `OPEN` windows of the key that the element falls into (by span or
   interval, FR-042-016) `FOR UPDATE` in `window_id` order, merge them when there are several, or open one; insert the
   member (`ON CONFLICT DO NOTHING`); update the count, the bytes, the deadline (clearing `seal_horizon` when it moves)
   and the flags; seal at once when `count` or `onFull: emit` says so (and, with `flatMap`, evaluate `select` and record
   the fan-out, FR-042-020); for any other chain, write the emission as a new window sealed at once with its one member;
5. when the element adds held bytes, take the tenant's `operator_held_bytes` lock (the advisory lock of
   `crates/infra-postgres/src/quotas.rs`, slot 8b, last) and check, under it, the committed `operator_held_usage` row
   plus this transaction's pending delta in `operator_held_deltas` (read once, never counted twice) against
   `operator_held_bytes`, and the committed tenant content usage plus
   this transaction's pending content delta against the tenant content quota; when either would be passed, the
   transaction rolls back to a savepoint taken before step 3 and records `FAILED`/`QUOTA_EXCEEDED`, so nothing is held.
   A refusal that only the commit-time flush of the tenant content quota finds (a concurrent transaction of the tenant
   committed in between) is the dispatcher's wait for room, not a final outcome (FR-042-021);
6. insert the `event_deliveries` row for `trigger:{id}` with the outcome; on a conflict, roll back: the admission was
   already applied.

Statements of the common case, an element of an existing key joining an open window: the chain row, the key row, the
window lock, the member insert, the window update, the held-bytes check, the delivery insert: seven, at most eight with
the evaluation's own read; a new key adds the usage update and the key insert; a merge adds one update per merged window.
When step 4 sealed a window, the dispatcher starts it right after the commit, in a second transaction with the start of
section 6; a crash or a refusal there leaves the window `SEALED` for the pass. The apply takes no slot below 3 and
creates no run. A transient failure leaves the event pending and the dispatcher's existing retry applies; every final
outcome is a delivery row.

### 6. Trigger chains: the pass (slice 3)

A pass `operator_windows` in `crates/scheduler/src/passes.rs`, on the shards the replica holds. Each tick, in bounded
batches per tenant:

- **seal**: for `OPEN` windows whose deadline `D` (`closes_at` or `max_close_at`) is at or before `now()` (the start of
  the statement's transaction; never `clock_timestamp()` in the statement that reads the snapshot, checked by
  `scripts/check-architecture.py`): record `seal_horizon = pg_snapshot_xmax(pg_current_snapshot())` when it is null; go
  on only once `pg_snapshot_xmin(pg_current_snapshot())` has reached it (counted `held_by_horizon`); for each (tenant,
  namespace, source, type) the chain matches, read once the oldest `PENDING` inbox event (the index of section 4) and
  skip the windows that event is not past (counted `held_by_backlog`); both holds end at `D + maxHoldMs`, after which the
  window is sealed anyway (counted `sealed_past_hold`). For the others, one transaction each: read the shard row `FOR
  SHARE` (ADR-0020), take the chain row `FOR SHARE`, the key row and the window `FOR UPDATE` while it is `OPEN`, and seal
  it if it is still due (a moved deadline is counted `lost_race`); with `flatMap`, evaluate `select` and check each
  index's input there (FR-042-020), each index input also against `MAX_PAYLOAD_BYTES` (over it the window is refused
  `RUN_DATA_LIMIT`, a row of FR-042-019's table), then, when the recorded `fan_out_values` add held bytes, take the
  tenant's `operator_held_bytes` lock last (slot 8b) and check them as step 5 of section 5 checks an element: over the
  quota the window is refused `QUOTA_EXCEEDED` in that transaction (final, its held values deleted, audited
  `operator.window_refused`). A seal that only the commit-time flush of the tenant content quota refuses rolls back,
  leaves the window `OPEN` to be sealed again by a later pass, is counted `quota_wait`, and is bounded: a window still
  `OPEN` 24 hours after `D` is refused `START_EXPIRED` by the pass, as a start that cannot happen is;
- **start**: for `SEALED` windows whose `next_attempt_at` has passed, run the start of FR-042-019. It is not fenced
  (ADR-0050 decision 6 keeps fence reads to passes' own aggregates, and the dispatcher, which starts windows sealed at
  once, holds no shard): one statement reads the state, the held values and the chain's state; then one transaction
  calls the admission port, which takes the receipt (slot 2) and creates the run (slot 6) with 022's rows (slots 7 and
  8), and in the run's open transaction `RunTx::record_window_start` (a new method beside `record_delivery`) updates
  the window `WHERE window_id = $1 AND state = 'SEALED'` to `STARTED` with `run_id` (or advances `started_count` after
  `flatMap`), deletes its held values and fails the transaction unless exactly one row changed. The window row is the
  only operator row the start touches, and it takes it after the run: the recorded deviation of ADR-0050 decision 7
  (ADR-0155). That is safe because a `SEALED` window is written only by its starts and by the pass's short back-off,
  expiry and refusal transactions, none of which holds another row, and no transaction that holds a slot-3 row locks a
  `SEALED` window (apply, seal and flush lock only `OPEN` ones, discard `OPEN` or `REFUSED` ones, retry `REFUSED` ones);
  so the update never waits on a transaction that holds another slot, and the start holds no chain or key row while the
  run is created. A retryable outcome rolls the transaction back and a short transaction on the window alone advances
  `attempts` and `next_attempt_at` (back-off doubling from 1 second to 5 minutes); a final outcome sets `REFUSED` (with
  `final_at`) in that short transaction; a window still `SEALED` at `start_expires_at` is refused `START_EXPIRED` there;
- **audit of `KEY_LIMIT`**: for each `operator_key_limit_counts` row of a minute that has ended, one transaction writes
  `operator.key_limit` with the chain and the count and deletes the row;
- **purge**: take the chain row `FOR SHARE`, then remove idle keys (releasing their usage slots with `key_count =
  key_count - 1`), marks past their horizon, and windows seven days after `final_at`; a `DRAINING` or `REMOVED` chain
  with nothing left is taken `FOR UPDATE` and deleted with its usage row.

**Manual acts** (application methods of `OperatorChainStore`, wired by Task 042-34). A flush takes the chain row `FOR
SHARE`, the key row and the `OPEN` window `FOR UPDATE`, records `requested_by` and seals it now, bypassing the horizon
and the backlog hold (an event not yet applied becomes late and opens a new window; the runbook says so), with the
seal's `flatMap` evaluation and its two checks, the held-bytes lock taken last (a quota refusal answers `409
QUOTA_EXCEEDED` and refuses the window as the seal does); a discard
locks an `OPEN` or `REFUSED` window the same way, deletes its held values and sets `DISCARDED`; a window retry locks a
`REFUSED` window with `START_EXPIRED`, records `requested_by`, sets `SEALED`, resets `attempts` and `next_attempt_at` and
sets `start_expires_at` 24 hours ahead. Each act writes its own audit record with its caller. The start's run has the
engine as its starter, as today's trigger starts do; when the window carries `requested_by`, the start's `RunStarted`
audit record adds "requested by X at T", so the caller is traced without becoming the initiator of a run created later.
Delivery retries are not recorded on the window: the caller is in `operator.delivery_retried` only.

Nothing is held in memory between ticks. The lag of the pass is the age of the oldest due window past every hold, that
is, due and either not held or past `D + maxHoldMs`: `scheduler_lag_seconds{pass="operator_windows"}`, set by
`crates/scheduler/src/monitor.rs` from the pass's report. `operator_seal_horizon_age_seconds` shows the age of the oldest
recorded horizon, which any long transaction of the database cluster holds back; the runbook says the horizon is
cluster-wide and how to find the transaction that holds it (`pg_stat_activity` ordered by `backend_xid` age).

### 7. Semantics under failure, stated once

| Case | Outcome |
|---|---|
| Same event id inside the dedupe window | Absorbed by the inbox; the chain never sees it |
| The same event id admitted again after its dedupe window, into a window that holds it | `SUPPRESSED`, reason `duplicate_member` |
| An admission dispatched twice | One application: the delivery row's key; the second transaction rolls back |
| Crash during the apply transaction | Nothing applied; the event is dispatched again after the claim lease, and the backlog hold keeps its window open meanwhile |
| Events applied out of order, by two replicas or after a crash | A session window takes every element its interval covers and merges the windows one element bridges; tumbling windows are aligned; `emit` order is `(received_at, source, event_id)` |
| Crash between seal and start | The window is `SEALED`; the pass starts it under the same key |
| Start retried after an unknown commit | The admission port finds the key; one run; the guarded write sets `STARTED` once |
| An event applied after its window sealed | Late: an open window or a new one, flagged, counted |
| Deadline and element at the same instant | The key lock orders them; both orders are valid and tested |
| An event committed just after the deadline with an earlier `received_at` | Covered by the transaction horizon (ADR-0153); were it applied after the seal, it is late, never lost |
| A long transaction anywhere in the cluster | Holds the horizon until `D + maxHoldMs`, then the window seals; alerted |
| An event that stays pending (claimed, waiting for room or retried) | Holds sealing for at most `maxHoldMs` |
| Scheduler down past many deadlines | On return the backlog hold lets pending events in, within `maxHoldMs` |
| The run a window started fails, or times out in 022's buffer | A run like any other; the window stays `STARTED` |
| The trigger is removed | `OPEN` windows discarded; `SEALED` ones refused `TRIGGER_REMOVED` by their start |

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
   a run through any other path (a second creator would bypass 022's limits), and the control plane keeps refusing the
   trigger switch (FR-042-034).
2. **A created run is started.** A run 022 buffers exists, has its journal and counts against 022's ceilings, so the
   window is `STARTED`. 022's D022-018 says the window "stays sealed" in that case; both texts agree that a repeated
   request finds the run, and 022's wording is to be aligned on 022's branch by its owner (G042-011, T042-3).
   The seal and the start are two transactions (Gap 042-002, Decision 042-162): the seal is short and fenced; the start
   takes 022's gate rows and may wait on a buffer; a crash between them leaves the window `SEALED`, which the next pass
   starts under the same key. The start needs no change to 022's gates: `RunStore::create` already returns the run's
   open transaction (`crates/application/src/runs.rs`), in which the start writes its window through one additive
   `RunTx` method, `record_window_start`, as today's trigger start writes its delivery through `record_delivery`.
3. **A full buffer is a retry**, as 022 leaves an event start pending (FR-022-042, O1165): the window stays `SEALED`.
4. **No operator paces.** `throttle` is `rateLimit`. The chain's key sees the event; 022's key sees the run input.
5. **Two things named window.** 022's window is an interval of grants; this design's is a set of gathered elements.
6. **Lock order.** The apply's chain, usage, key and window rows are ADR-0050 slot 3 and are never held when a run is
   created; the start takes no chain or key row and writes only its `SEALED` window, after the run, as the recorded
   deviation of ADR-0050 decision 7 (ADR-0155). The held-bytes lock is slot 8b.
7. **Collections (0.3.0).** A chain changes how many iterations a `forEach` has and nothing about their admission; 022
   refuses `rateLimit` on a body node in 0.3.0 (`SCOPED_RATE_LIMIT`).

### 9. Surfaces (slice 5, except the collection telemetry of section 3)

**REST** (additive; every path under `/v1/namespaces/{namespaceId}`; permissions as FR-042-023; typed errors with
`code`, `retryable`, `traceId`):

| Method and path | Answer |
|---|---|
| `GET /operator-chains` | chains: `chainId`, `workflowId`, `triggerId`, `state`, `maxKeys`, `valueDependent`; `keyCount` and `openWindows` without reveal only for a chain that is not value-dependent, otherwise only with `reveal=pii` (FR-042-024) |
| `GET /operator-chains/{chainId}/windows?state=` | windows, paged: `windowId`, `state`, `count`, `openedAt`, `deadline`, `late`, `overflow`, `runId`, `startedCount`, `fanOut`, `lastErrorCode`; `keyId` only with `reveal=pii`; for a value-dependent chain the whole list only with `reveal=pii`, and without it the answer is the chain's declaration and state (FR-042-024) |
| `GET /operator-chains/{chainId}/windows/{windowId}` | one window; with `reveal=pii`, also its `keyId` and its members by reference (`source`, `eventId`, `receivedAt`), never their values |
| `POST /operator-chains/{chainId}/windows/{windowId}:flush` | `202` with the window in its shape without reveal (for a value-dependent chain, only its id and state); `409 WINDOW_NOT_OPEN` otherwise |
| `POST /operator-chains/{chainId}/windows/{windowId}:discard` | `200` with the window in its shape without reveal; `409 WINDOW_NOT_DISCARDABLE` unless `OPEN` or `REFUSED` |
| `POST /operator-chains/{chainId}/windows/{windowId}:retry` | `202` with the window in its shape without reveal; `409 WINDOW_NOT_RETRYABLE` unless `REFUSED` with `START_EXPIRED` |
| `POST /events/{source}/{eventId}/deliveries/{triggerId}:retry` | `202` with the delivery; `409 DELIVERY_NOT_RETRYABLE` unless `FAILED` |
| `GET /events/{source}/{eventId}` (existing) | gains, per delivery, `outcome`, `errorCode` and `reason` (`operator` for every operator suppression), and `WITHHELD` as the only outcome of a delivery to a value-dependent chain; with `reveal=pii`, the stored outcome, code, reason and `windowId` |

A chain, window or event that is missing or another tenant's answers `404` with the same body, audited as a probe. A
`reveal=pii` request goes through the event read's projector and writes `payload.pii_revealed` or
`payload.reveal_denied`, one record per request.

- **Audit**: `operator.window_flushed`, `operator.window_discarded`, `operator.window_retried`,
  `operator.window_refused`, `operator.delivery_retried`, `operator.key_limit` (one per chain and minute, with a count),
  each with its caller where a person acted; `RunStarted` of a window's run adds "requested by X at T" when the window
  was flushed or retried.
- **Metrics** (no tenant, namespace, chain or key label):

  | Name | Type | Labels (closed) | Meaning | Emitted by |
  |---|---|---|---|---|
  | `operator_elements_total` | counter | `surface` (`collection`, `trigger`), `operator` (the nine names), `outcome` (`kept`, `dropped`, `produced` for collections; `passed`, `suppressed`, `failed`, `held` for triggers) | elements per stage outcome | the scheduler pass (collections), the dispatcher (triggers) |
  | `operator_windows_total` | counter | `outcome` (`started`, `refused`, `discarded`, `retried`, `expired`, `merged`) | window transitions: `expired` is a refusal with `START_EXPIRED`, `retried` a retry back to `SEALED` | the pass and the management API |
  | `operator_window_seal_total` | counter | `outcome` (`sealed`, `held_by_horizon`, `held_by_backlog`, `sealed_past_hold`, `lost_race`, `sealed_on_count`, `sealed_on_full`, `flushed`, `quota_wait`) | seal attempts and their result | the dispatcher (on count and full) and the pass |
  | `operator_open_windows` | gauge | none | windows in `OPEN` on the replica's shards | the pass |
  | `operator_window_emit_lag_seconds` | histogram | none | from a window's deadline to its run's creation | the start |
  | `operator_seal_horizon_age_seconds` | gauge | none | age of the oldest recorded horizon on the replica's shards | the pass |
  | `operator_key_limit_total` | counter | none | `KEY_LIMIT` refusals | the dispatcher |
  | `scheduler_lag_seconds{pass="operator_windows"}` | gauge | `pass` | defined in section 6 | `crates/scheduler/src/monitor.rs` |

- **Alerts** (slice 3): `OperatorWindowsLag` when the pass's lag is over 30 seconds for 5 minutes;
  `OperatorSealHorizonOld` when `operator_seal_horizon_age_seconds` is over 300 for 5 minutes; `OperatorSealedPastHold`
  when `operator_window_seal_total{outcome="sealed_past_hold"}` increases for 15 minutes.
- **Spans**: `operator.apply` on the event's trace; `operator.window.start` on the run's trace with the window's id and
  its member count as attributes, and no link to the members' traces, since a link would put a window's membership,
  which FR-042-024 counts as personal data, in the trace backend. The run carries no journal addition: the window names
  its run.
- **Studio**: the `Triggers` editor in `features/workflows/WorkflowSettings.tsx` gains the key, the stage list and the
  chain limits; a window list per workflow; the event page shows each delivery's outcome.
- **SDKs**: the trigger builders reuse the operator factory of section 3. Scenarios: `collection-operators` (build),
  `collection-operators-runtime`, `collection-operators-limits` in 0.3.0; `trigger-operators` (build),
  `trigger-debounce-runtime` and `trigger-window-runtime` with slice 5 (which close windows with `:flush`, not by
  waiting).

### 10. Rollout

Capabilities are compiled into a binary and switched per deployment. **Release 0.3.0:** publication of a chained
`forEach` or of a trigger chain requires every scheduler and control-plane replica to run 0.3.0. Under design 053 the
upgrade into 0.3.0 is a stopped upgrade and a rollback never crosses it, so every release a rollback can reach knows
both fields. An old replica that met such a definition would fail `parse_stored` with a schema error and leave the run
alone; one that met a chained blob scope would fail its restore with an invalid-storage error. Neither misreads; both
are covered by an old-reader test (T042-50).

**The capabilities under design 053** (ADR-0153 decision 4, ADR-0082, Requirements 053-006 and 053-007). The switches are
the capabilities `collection-operators-v1` and `trigger-operators-v1`. A process lists one for what its configuration
runs: a control plane `collection-operators-v1` when its definition service has `SAGAT_COLLECTION_OPERATORS_ENABLED`
on, an execution engine `trigger-operators-v1` when `SAGAT_TRIGGER_OPERATORS_ENABLED` turns on its dispatcher's apply
(the switch turns on the apply and the capability listing only; the `operator_windows` pass runs whatever the switch),
and a control plane `trigger-operators-v1` when its definition service publishes trigger
chains. The operator records each requirement for both kinds (`platform_requirement_activate`), which the registry
refuses while a live registered process lacks the capability.

- *Descriptors.* `collection-operators-v1`: fence form (a) on publication and `switch` of a version with a chained
  `forEach` (`collection_operators_required()`); drain check: no active version has a chained `forEach` and no chained
  blob scope is open. `trigger-operators-v1`: fence form (a) on publication and `switch` of a version with a trigger
  chain (`trigger_operators_required()`); drain check: no `operator_chains` row is `ACTIVE` or `DRAINING`, no
  `operator_windows` row of any state is left (final ones included), no member holds a value, and no `operator_keys`
  or `operator_marks` row is left. While the capability is `draining`, the pass purges a final window at once instead
  of seven days after `final_at`, so the drain can finish and a binary rolled back after phase 2 holds no window, key
  digest or member identity it would not purge. The apply and the delivery retry write only for an existing chain and are gated on
  `uses`; the pass's sealing, starting, refusing, expiry and purge are not gated, so they finish the draining in every
  binary that has the code: a chain cannot be created after phase 2 of the deactivation, and the drain check refuses
  the removal while any state is left.
- *Late activation.* A process that was not live at the activation can serve for about one heartbeat before the
  registry refuses it. A control plane without the capability refuses chained publications on its own switch; an engine
  without it does not use the capability (`ServingGate::uses` is false), so its dispatcher leaves a chained trigger's
  event pending without counting an attempt and starts no run per event (FR-042-034); its `operator_windows` pass still
  seals, starts and purges what exists.
- *Deactivation.* By the two phases of Requirement 053-007: phase 1 stops new publications and activations (the fence
  answers inactive), the chains drain (their triggers removed or their versions replaced by unchained ones) and the
  windows reach a final state, sealed, started, refused or purged by the pass, which phase 1 does not stop; phase 2
  takes the requirement row as Requirement 053-007 prescribes for form (a) (it waits for every fence call that holds
  the row `FOR KEY SHARE`; the table locks in share mode are form (b)'s, which this design does not use), runs the
  drain check and removes the requirement. The writer that meets the fence retries once with the capability treated as
  inactive and is refused with the switch's typed error.

A binary older than the registry does not register; for it the stopped upgrade into 0.3.0 is the only guard (Gap
042-006). Rolling the binary back below a recorded requirement is unsupported (ADR-0082); the operator deactivates first.

**Cut order.** Should the trigger half not all fit in release 0.3.0, it ships in this order, each step usable without
the next: (1) `key`, `filter` and `map`, `window`, `debounce` and `ignoreElements`, with the apply, the pass, the start
and the surfaces; (2) `distinct`, `skip` and `take`, with marks and epochs; (3) `flatMap` on triggers. A later step's
operators are refused at publication (`OPERATOR_UNSUPPORTED`) until they ship, so no stored definition depends on them.

### 11. Decisions recorded

- **ADR-0087 (accepted, OD-2026-11-13-17; revised after review)**: collection operators are applied once at scope
  entry by one port call under running ceilings, and the snapshot is the record. Release 0.3.0.
- **ADR-0086 (accepted, OD-2026-11-13-17; revised after review)**: an operator chain on a trigger is declared data
  applied by the engine; its state is rows per chain and key; time is the database clock at admission; a fenced pass
  seals a window, and a sealed window starts one run by its identity through the one admission port. Decisions 2, 3, 6
  and 7 are superseded by ADR-0155.
- **ADR-0153 (accepted, delegate, Owner decision 2026-11-26-136)**: a window is sealed only past the transaction
  horizon of its deadline, replacing ADR-0086's admission grace; both switches are capabilities of the instance
  registry. Decisions 1 and 2 are refined by ADR-0155.
- **ADR-0155 (accepted, delegate, Owner decision 2026-11-27-428)**: a window is identified by a random id and session
  windows merge; the start is unfenced, writes its window after its run (a recorded deviation of ADR-0050 decision 7)
  and refuses a window of a removed trigger; keys and `distinct` values are plain digests that never leave the database;
  new keys are counted on a usage row of their own; the horizon is read against the transaction's start and both holds
  end at `D + maxHoldMs` over every pending event.

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
| One burst becomes two runs | Session windows merge whatever the order of the applies; the backlog hold covers every pending event for at least the claim lease plus a margin; SC-042-001's failpoints |
| Concurrent applies deadlock on a chain | No counter on the share-locked chain row: new keys on a usage row, marks per key under the key lock; a two-replica barrier test |
| A window that never closes holds events | `maxWaitMs` required; both holds end at `D + maxHoldMs`; flush and discard |
| A long transaction anywhere in the cluster delays every window | Both holds end at `D + maxHoldMs`; the `OperatorSealHorizonOld` and `OperatorSealedPastHold` alerts; the runbook names the holder |
| Held personal data outlives its use | A deletable column, deleted at start; no key text; digests never shown; idle keys and final windows purged; a held-bytes quota |
| A key flood stalls a namespace | `KEY_LIMIT` is final at once and leaves the backlog; counts collapsed per minute |
| An operator retry starts runs elsewhere | The delivery retry applies the event to one trigger only and never returns it to `PENDING` |
| A capability is removed while a chain is being published | The fence of form (a) on publication and `switch`, and the drain check in share mode (design 053) |
| Two rate limiters appear | No operator paces; section 8 |
| Feature 022's admission point is not merged when slice 3 starts | Slice 3's start waits for it and no other creator path is used (section 8); the control plane refuses the trigger switch until the start exists |
| The trigger half does not all fit in 0.3.0 | The cut order of section 10 |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Collections: the output is recorded once by the entry transaction and never recomputed (FR-042-009). Triggers: all state is rows; an application commits with its delivery row; the start is idempotent by the window's random id and the port's receipt, and its guarded write fails the run's transaction unless the window was `SEALED` (FR-042-014, 018, 019) |
| II. Modular domain, ports and adapters | Pass | `OperatorChain` and the window module `crates/domain/src/operators/window.rs` are pure (no serde_json, SQL or clock); one evaluator port method per surface; the store port `OperatorChainStore` in `crates/application`; statements in `infra-postgres`; the switch in the definition service |
| III. External effects are at-least-once; sagas compensate | Pass | No external effect; runs start through the ordinary path |
| IV. Compatibility is versioned and pinned | Pass with conditions | Additive v1 fields and expand-only columns; switches never make a stored definition unreadable; six SDKs without breaking a caller; both capabilities under design 053 with fences, descriptors and drain checks. Condition: a binary from before the registry is kept out only by the stopped upgrade into 0.3.0 (Gap 042-006) |
| V. Code, tenant boundary and secrets are isolated | Pass | Bounded expressions evaluated by the engine, off the async runtime under ceilings; tenant and namespace in every key; held data classified and deletable; no key text, digests never shown, surrogate ids; another tenant's chain, window or event answers `404` and is audited (FR-042-002, 007, 023, 024) |
| VI. Observability and audit are part of the feature | Pass | Typed codes in closed sets, a metric table with closed labels, three alerts, a no-value-leak test, audit of every manual act (FR-042-025, section 9) |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | The chain sits after the inbox and applies once per admission; `KEY_LIMIT` is an outcome, not a held event; a delivery retry is a targeted apply |
| VIII. Failure is tested and contracts are verified, honestly | Pass | SC-042-003 to 005 with slices 0 and 1 (panicking evaluator, a test per ceiling, races with barriers); SC-042-001, 002, 006, 007 with slices 2 to 5, with named failpoints, all in release 0.3.0 |
| Delivery Constraints | Pass | One writable region per cell, PostgreSQL as authority, no new component; budgets are targets on a stated environment |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A port method for the whole chain, on each surface | The present per-expression call parses the run's data, or the event, each time | Calling `condition` per element: GiBs of parsing on a large run; ten parses per event on the shared dispatcher |
| Running ceilings per stage | A count of evaluations bounds neither memory nor bytes | One evaluation counter: `map` over a large step output passes it |
| A chained blob manifest | The present checks compare stored items to the blob's | Re-reading the blob at restore: a read the design forbids |
| A blocking task with a deadline | The chain is CPU work on the scheduler | Running inline on the runtime: one chain stalls the pass |
| Seven tables of chain state (slice 2) | A debounce that survives a restart is a row with a deadline; a key ceiling needs a counter that is not on the share-locked row; a collapsed audit needs a durable count | State in memory: lost on restart, wrong with two replicas |
| A fenced pass with the transaction horizon and the hold rule (slice 3) | A deadline must fire with no event, and a commit can land after it | Sealing on the clock alone: a burst split by commit latency |
| Session windows that merge | The order of the applies must not decide how many runs a burst starts | Opening a window when no deadline covers the element: two runs after a crash |
| Every stateful emission as a window (slice 2) | A run is created only by the start, so the delivery row and the run never collide | Applying and starting in one transaction: the apply's delivery row and today's start write the same row |
| A canonical stage order on triggers (slice 2) | Arbitrary orders give each pair its own stateful semantics | Any order |
| Two capability fences (design 053 form (a)) | Publication and the capability's removal must not race | Reading the requirement without a lock: a chain can commit after the removal |

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
- **A keyed hash (HMAC) of keys behind a definer function** (the third revision). A database reader already holds the
  events and the held values, so it protected nothing the plain digest does not, at the cost of a key table, an
  extension and a definer function; replaced by digests that never leave the database (ADR-0155).
- **Counters on the chain row.** Concurrent applies holding it `FOR SHARE` deadlock when they update it; taking it
  `FOR UPDATE` serialises every element of a chain.
- **A fenced start.** The dispatcher, which starts windows sealed at once, holds no shard, and the port opens its own
  transaction; the guarded write and the receipt make the start safe without a fence.
- **Window inputs stored as blobs in 0.3.0.** A design of its own (object-store write before the run, failure, quota);
  the CUJs fit inline (Gap 042-015).

## Non-Functional Requirements

- **NFR-042-001 Performance** (targets on a stated environment, not promises): a collection chain of eight stages over
  1,000 small elements adds at most 20 ms to scope entry at the 95th percentile, and the worst case at the ceilings is
  reported with its effect on `scheduler_tick_seconds` (T042-9). Slices 2 to 5: an event joining an open window of an
  existing key costs at most 8 statements in its one transaction (section 5 lists them); sealing a window at most 6,
  and starting it at most 4 beyond the run start; a trigger without a chain costs zero more (FR-042-030); 1,000 events
  per second across 10,000 keys with the pass's lag under 2 seconds at the 99th percentile, the new-key and held-bytes
  serialisation measured (T042-33).
- **NFR-042-002 Security**: the only `SECURITY DEFINER` functions are the two capability fences of FR-042-034, of the
  shape of `lease_withdrawal_required()`, and the trigger functions `operator_held_usage_delta()` and
  `operator_held_flush()` that maintain the held-bytes counter through per-transaction deltas (section 4), each with `search_path` pinned to `pg_catalog, pg_temp`; no new role; the runtime role
  gets only the grants the statements need (no write on `operator_held_usage` or `operator_held_deltas`), checked by the privilege audit; no key
  text is stored, digests are tenant- and chain-separated and no digest leaves the database; what links events by key
  is returned only under `reveal=pii` and audited, and the residual of a publisher who watches runs is stated (Gap
  042-018); the cross-tenant battery covers every new endpoint and table.
- **NFR-042-003 Operability**: the runbooks say what each metric and code means, the 0.3.0 upgrade rule, what a held
  window is, how to flush or discard one, that the horizon is cluster-wide and how to find the transaction that holds
  it; the three alerts of section 9 ship with slice 3.
- **NFR-042-004 Compatibility**: every existing definition, trigger and run behaves as before; the OpenAPI and IR schema
  changes are additive and pass the contract baselines; the new columns are expand-only.
- **NFR-042-005 Storage**: with the default quotas a tenant's chain state is bounded, and the bound is stated in bytes
  in the runbook after measurement.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

**Release 0.3.0 (OD-2026-11-13-15): slices 0 and 1**, from `develop` after pull request 218. **Slice 0** foundation,
collection half: T042-1, 4, 5, 6, 47. **Slice 1** collection operators: T042-7 to 12, 45, 46, 48 to 52, and 56 (the one
small migration of slice 1, which adds its capability to the registry). No table, pass or loop; the collection entry of
the scheduler pass changes.

**Release 0.3.0 since Owner decision 2026-11-24-1: slices 2 to 5.** **Slice 2** chain state and the operators without
time (Tasks 042-2, 13 to 19, 43, 44, 53, 54, 57, 58); **slice 3** windows, the pass and the start (Tasks 042-3, 20 to 26,
55 and 59; Tasks 042-3 and 042-22 after feature 022's port merges); **slice 4** failure and capacity (Tasks 042-27 to
33); **slice 5** surfaces and close (Tasks 042-34 to 42). ADR-0086, ADR-0153 and ADR-0155 are accepted; Task 042-44's
re-read is the third revision of this design. The cut order of section 10 applies inside slices 2 to 5.

**Two lanes for two builders.** They share some files, listed below; everything else is one lane's.

- **Lane A, declarations, rules and surfaces:** `crates/domain` (the trigger rules and the pure window module),
  `crates/infra-ir` (the schema, the codec and the evaluator's `trigger_chain`), `crates/api-http` (the REST resources),
  the six SDKs and the Studio. Tasks 042-43 (first), then 042-15's domain module and `trigger_chain`, 042-10 and 042-11
  (the SDK half of slice 1 still open, Gaps 042-113 and 042-131), 042-36, 042-37, 042-34, 042-17's response shape,
  042-38, 042-39.
- **Lane B, state and execution:** `crates/application` (the definition service's switch and fences, the dispatcher's
  apply, the window start, the manual acts), `crates/infra-postgres` (the one migration and every statement),
  `crates/scheduler` (the dispatcher and the pass) and the test targets. Tasks 042-2, 13, 14, 54, 56, 15's apply, 16,
  18, 19, 53, 55, 57, 58, 20, 21, 23 to 26, 59, 3, 22, 27 to 33, 35, and the `trigger-operators` switch of Task 042-43 in
  `DefinitionService`.
- **Shared files:** Task 042-43 is split by crate: lane A writes the domain rules, the schema, the codec and the new
  `IrError` variants (`OPERATOR_ORDER`, `OPERATOR_NEVER_EMITS`, `OPERATOR_INPUT_SCHEMA`, `OPERATOR_UNSUPPORTED`), lane B
  maps them in `crates/application/src/definitions.rs` and adds the switch. Task 042-15 is split the same way: lane A
  delivers `crates/domain/src/operators/window.rs` and `trigger_chain` in `crates/infra-ir/src/evaluator.rs` with their
  tests, and lane B consumes them in the apply; Tasks 042-16, 042-20 and 042-23 add their rules to that module through
  lane A. Task 042-18's and Task 042-57's REST actions are written by lane A in Task 042-34 over lane B's application
  methods. Both lanes append rows to `specs/042-stream-operators/research.md` from disjoint ranges (lane A Decision
  042-300 upward, lane B Decision 042-400 upward), so a merge keeps both sides.
- **Order points between the lanes:** Task 042-43 (lane A) merges before Tasks 042-13, 042-14, 042-15 and 042-54 (lane
  B); Task 042-15's domain module and `trigger_chain` (lane A) merge before its apply (lane B); the migration of Task
  042-13 (lane B) merges before lane A's REST resources (Task 042-34) are wired to lane B's services, which lane B
  exposes as application methods first (Tasks 042-18 and 042-57); Tasks 042-3 and 042-22 (lane B) start after feature
  022's throughput branch merges (section 8); Tasks 042-13, 042-15, 042-18, 042-54 and 042-56 start after design 053's
  Task 053-5 merges (the `draining` state, `ServingGate::uses`, the capability descriptors, phase 2 of Requirement 053-007
  and `lease_withdrawal_required()` moved to `FOR KEY SHARE`), since the fences, the gating of FR-042-034 and the drain
  checks are built on them; Tasks 042-40 to 042-42 close both lanes.

- [ ] T042-1 Confirm that OD-2026-11-13-15, OD-2026-11-13-17 and OD-2026-11-27-432 are rows of `docs/owner-decisions.md`
  on `develop`, that the `Status` cell of every row a task here cites in `Owner-gate:` reads exactly `Decided` and that
  its `Covers` names every task gated on it, before any of them is ticked (Gap 042-016).
  Refs: FR-042-001
- [ ] T042-2 Spike, in `specs/042-stream-operators/research.md`: the statement count of today's dispatch (the baseline
  of FR-042-030). Seal and start are two transactions (Decision 042-162; Gap 042-002 closed); window inputs are inline
  (Gap 042-015).
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
  Owner-gate: OD-2026-11-27-432
- [ ] T042-6 Extend the IR schema and the codec with `forEach.operators` and the expression and variable checks
  (`item`, `index`, `iteration`, `carry`, `bodyResult` refused in a stage); fixtures prove every earlier document still
  decodes, an unknown operator is refused and a trigger operator is refused.
  Refs: FR-042-002, FR-042-005, NFR-042-004
  Owner-gate: OD-2026-11-27-432
- [ ] T042-7 Add `collection_chain` to the evaluator port and `JsonEvaluator`: one environment, stage-by-stage
  evaluation, `element` and `position`, the first failing stage and position reported, the canonical form of
  `distinct`, and a property test against a reference implementation over random chains and lists (including `1` and
  `1.0`, `-0` and `0`, objects with keys in different orders).
  Refs: FR-042-004, FR-042-005, FR-042-006, FR-042-011
  Owner-gate: OD-2026-11-27-432
- [ ] T042-8 Enforce the running ceilings and map each to its one code; call the chain in a blocking task with its
  deadline; route a failure through `stop_collection_for_chain`, a blob source over the chain input ceiling included, so
  that `workflow_primitive_rejections_total` counts it as it counts the same code from an inline list; one test per
  ceiling crossing it by one and one at it, the counter asserted for both sources; an empty output.
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
- [ ] T042-13 Write the migration of section 4 (tables, nullable columns of `workflow_triggers` and `event_deliveries`
  with their `CHECK`s, indexes including `inbox_events_pending_match_idx`, grants, classification entries, quota
  triggers, the counter `operator_held_usage`, the delta table `operator_held_deltas`, the owner-owned trigger
  function `operator_held_usage_delta()` that records a per-transaction delta and the deferred flush
  `operator_held_flush()` that adds it to the counter at commit (no write grant to `sagat_app` on either table; a test
  that the counter is not locked before commit), the quota resources `operator_held_bytes`, `operator_chains`, `operator_keys` and
  `operator_marks` with their defaults in `quota_limits` and `quota_ceilings`, `trigger-operators-v1` in the registry's
  closed capability checks,
  the fence function `trigger_operators_required()`, a down that refuses data loss) with the migration checklist;
  register the apply's and the seal's rows in ADR-0050 decision 7 (slot 3: chain, usage, key, windows; 8b the held-bytes
  lock) with the slot test of the apply and the seal in the harness `tests/support/lock_order.rs`.
  Refs: FR-042-021, FR-042-023, FR-042-024, FR-042-031, NFR-042-002
  Owner-gate: OD-2026-11-26-133
- [ ] T042-14 Register a chain when a version is published, activated, rolled back or promoted (through
  `register_triggers`, in `switch` as well as at publication), with the identity (trigger, chain digest) of FR-042-022:
  keep it on an equal digest, drain the old one on another, reactivate a stored `DRAINING` chain on a rollback, mark
  it `REMOVED` on the trigger's removal; check the tenant quotas and the capability fence on each of those paths; a
  test rolls back onto a chained version over a quota and sees it refused, one under the quota reactivates its chain
  with its state, and one republishes the same chain and keeps its keys.
  Refs: FR-042-021, FR-042-022
  Owner-gate: OD-2026-11-27-433
- [ ] T042-15 Write `crates/domain/src/operators/window.rs` (the transitions of FR-042-016 and FR-042-017, merges
  included, with property tests that any order of applying a generated burst gives the same windows for tumbling and
  session kinds), the evaluator's `trigger_chain` (one parse of the event, compiled expressions cached per chain digest,
  a typed answer) and the store port `OperatorChainStore`; then apply an event in one transaction in the lock order of
  section 5: the stateless prefix without locks, the chain row `FOR SHARE`, the usage row only for a new key, the key
  row, the windows; every emission of a stateful chain written as a window; the delivery row inserted once with the
  outcome of FR-042-031, and the start of a window sealed at once right after the commit; tests that a `distinct` chain
  without a window starts its run, that a chain of only `filter` and `map` behaves as today with the input of
  FR-042-032, that an apply meeting a `DRAINING` chain applies to the current one, and a two-replica barrier test with
  two new keys and two new marks on one chain that ends with no deadlock and exact counts; a barrier test of two applies
  of one new key that ends with one key row, one reserved slot, no unique violation and no attempt used (the key insert
  `ON CONFLICT DO NOTHING RETURNING` and the savepoint of section 5).
  Refs: FR-042-014, FR-042-031
- [ ] T042-16 Add `distinct`, `skip` and `take` with epochs ending after `keyIdleMs`, marks under the per-key ceiling
  `floor(maxMarks / maxKeys)`, and the purge of idle keys releasing their usage slots; a test that `2 × maxKeys` keys
  arriving one after another, each idle past `keyIdleMs`, are all served.
  Refs: FR-042-015, FR-042-021, FR-042-022
  Owner-gate: OD-2026-11-27-426, OD-2026-11-27-433
- [ ] T042-17 Expose each delivery's `outcome`, `errorCode` and `reason` on the event resource: for a value-dependent
  chain only `WITHHELD` without reveal; for another chain the operator reasons shown as `operator`; with `reveal=pii`,
  through the event read's projector writing `payload.pii_revealed` or `payload.reveal_denied`, the stored outcome,
  code, reason and `windowId`; tests: a caller without the reveal grant sees `WITHHELD` on a `distinct` chain and a
  keyed window chain and `operator` with no `windowId` on a keyless window chain, and its `reveal=pii` request is
  denied and audited; the publish-and-read case (a Developer without the grant publishes an event with a guessed key or
  `distinct` value and reads its delivery, the chain resource and its windows) sees the same answers for a match and a
  miss; what a run start shows to `run:read` is the stated residual of Gap 042-018 and is not asserted.
  Refs: FR-042-024, FR-042-025, FR-042-031
- [ ] T042-18 Enforce `maxKeys` at key creation through the usage row with the final `KEY_LIMIT` outcome, the
  per-minute count row and its collapsed audit; the delivery retry of FR-042-033 as an application method (only a
  `FAILED` delivery of the named trigger; a targeted apply in one transaction under the inbox row `FOR NO KEY UPDATE`;
  `409 DELIVERY_NOT_RETRYABLE`; the uniform `404` with the probe audit when no inbox row is in scope or no delivery row
  of that trigger exists; `OPERATOR_UNSUPPORTED` when the process does not use the capability; the caller only in the
  audit record), with tests that a retry touches no other trigger and no wait, never returns the event to `PENDING`, is
  refused for a `HELD` delivery, answers `404` for a trigger without a delivery row, and does not deadlock with a
  re-dispatch of the same event that holds the key row (a barrier between the two); its REST action is lane A's (Task
  042-34).
  Refs: FR-042-021, FR-042-023, FR-042-033
  Owner-gate: OD-2026-11-27-432, OD-2026-11-27-429
- [ ] T042-19 Hold FR-042-030 with a budget test: a trigger without a chain adds no statement.
  Refs: FR-042-030, NFR-042-001
- [ ] T042-20 Open, extend, merge and seal-on-count windows in the apply transaction (tumbling, session with merging,
  count; several open per key; all, first, last by `(received_at, source, event_id)`; `onFull` on count and bytes;
  `overflow`; the duplicate member), with the numbers of O042-007 and the parameter table of section 2.
  Refs: FR-042-016, FR-042-021
  Owner-gate: OD-2026-11-27-432, OD-2026-11-27-430
- [ ] T042-21 Add the `operator_windows` pass: fenced seal past the transaction horizon (ADR-0153 and ADR-0155, Task
  042-55), the backlog hold over `inbox_events_pending_match_idx` for every pending event of a matching source and type,
  both holds ending at `D + maxHoldMs`, with the seal outcomes, the lag of section 6 and the `KEY_LIMIT` audit of closed
  minutes.
  Refs: FR-042-017, FR-042-018
  Owner-gate: OD-2026-11-26-136, OD-2026-11-27-425
- [ ] T042-22 Start a sealed window's run by its `window_id` key, on its version, through the one admission port,
  unfenced, with the input shapes of FR-042-032; `RunTx::record_window_start` writing the window in the run's open
  transaction and failing it unless exactly one `SEALED` row changed; the replay's short guarded transaction; the read
  of state, values and chain in one statement; held values deleted at start; each row of FR-042-019's outcome table
  (re-checked against 022's port as merged, and amended here first if it differs), with a test per row, the expiry and
  its reset by a retry included; the slot test of the start in `tests/support/lock_order.rs` with the recorded deviation
  of ADR-0155, the held-bytes counter row asserted as the last lock (slot 8b, taken only by the commit-time flush).
  Refs: FR-042-019, FR-042-020, FR-042-024, FR-042-029, FR-042-032
  Owner-gate: OD-2026-11-27-420, OD-2026-11-27-428
- [ ] T042-23 Add `debounce` and `ignoreElements` over the window machinery.
  Refs: FR-042-016
- [ ] T042-24 Add `flatMap` fan-out on a trigger: `select` evaluated once at the seal (at the apply for a window sealed
  at once), `fan_out` and its values recorded, each index's input checked there, `REFUSED` with `EXPRESSION_ERROR`,
  `OPERATOR_FAN_OUT`, `RUN_INPUT_INVALID` or `RUN_DATA_LIMIT` before any run, the recorded list checked against the
  held-bytes quota under its lock in the seal, the flush and the apply (`QUOTA_EXCEEDED`), a commit-time content refusal
  counted `quota_wait` and retried by a later pass up to the 24-hour bound, and the partial fan-out of FR-042-020
  recorded and audited; tests of each refusal, of a `select` that doubles a 256 KiB envelope against a tenant at its
  quota (refused, nothing held), and of a version removed between two indexes.
  Refs: FR-042-020
  Owner-gate: OD-2026-11-27-434
- [ ] T042-25 Mark and count late elements, with the span or interval of FR-042-017; test an event retried after its
  window sealed.
  Refs: FR-042-017
- [ ] T042-26 Purge keys, marks, final windows seven days after `final_at` and drained or removed chains in bounded
  batches, chain row first.
  Refs: FR-042-022, NFR-042-005
  Owner-gate: OD-2026-11-27-433
- [ ] T042-27 Fault test in `fault_stream_operators`: kill the process at each failpoint of SC-042-001 (a held lock at
  the named point, then `pg_terminate_backend` of the connection), with 500 events for one key and one run; the
  stranded first batch and the two-replica interleaving forced with a barrier.
  Refs: SC-042-001
- [ ] T042-28 Fault test: every admission dispatched twice, and a lost broker hint, change nothing.
  Refs: SC-042-001, FR-042-014
- [ ] T042-29 Race test: seal against element in both orders, forced with a held lock, never a sleep; two session
  windows bridged by one element merge.
  Refs: SC-042-002, FR-042-018
- [ ] T042-30 Fault test: restart with 1,000 open windows, deadlines passed and events pending.
  Refs: SC-042-007
- [ ] T042-31 Capacity test: 50,000 keys against `maxKeys` 1,000, with a second tenant's dispatch latency measured
  against the bound of SC-042-006 on a stated environment.
  Refs: SC-042-006
- [ ] T042-32 Cross-tenant and privilege tests for every new table, function and endpoint, also as the runtime role.
  Refs: FR-042-023, NFR-042-002
- [ ] T042-33 Measure NFR-042-001 for triggers on a stated environment, the new-key and held-bytes serialisation
  included, and write the dated validation report.
  Refs: SC-042-008, NFR-042-001
- [ ] T042-34 Add the REST endpoints of section 9 with the permissions of FR-042-023, the error codes and the audit
  actions, with the contract baseline.
  Refs: FR-042-026, FR-042-023, FR-042-033
- [ ] T042-35 Add the trigger metrics of section 9's table, the spans (no link to member traces) and the three alert
  rules with their promtool cases.
  Refs: FR-042-025, NFR-042-003
- [ ] T042-36 Add the trigger builders to the six SDKs over the operator factory.
  Refs: FR-042-028
- [ ] T042-37 Add the scenarios `trigger-operators`, `trigger-debounce-runtime` and `trigger-window-runtime`.
  Refs: FR-042-028
- [ ] T042-38 Studio: the key, stage and chain-limit editor on a trigger with the publication rules.
  Refs: FR-042-027
- [ ] T042-39 Studio: the window list, the lag, the event page's delivery outcomes, flush and discard; windows named by
  their ids, never a key text; `keyId`, members and the stored reasons shown only after an explicit reveal, and for a
  value-dependent chain the windows, counts and delivery outcomes too (`WITHHELD` until revealed) (O042-008, refined;
  FR-042-024).
  Refs: FR-042-027, FR-042-024
  Owner-gate: OD-2026-11-27-432, OD-2026-11-27-424
- [ ] T042-40 Write the events runbook section (the horizon is cluster-wide and how to find its holder; flush, discard
  and both retries), the quickstart of CUJ 3 with its failure scenario, and the reference.
  Refs: NFR-042-003
- [ ] T042-41 Update `docs/traceability.md`, the roadmap row and the release notes; register the remaining gaps.
  Refs: NFR-042-004
- [ ] T042-42 Write the trigger rollout procedure under design 053 (activation after every engine and control plane
  lists the capability, deactivation by its two phases) and rehearse it on the compose stack.
  Refs: FR-042-003, FR-042-034
- [ ] T042-43 Extend the domain rules, the IR schema and the codec with a trigger's `key`, `operators` and
  `chainLimits` (canonical order with `distinct`, `skip`, `take` in that order, "can emit", the parameter table of
  section 2, `OPERATOR_UNSUPPORTED` for an operator the cut order has not shipped) and the `trigger-operators` switch,
  with the publication check `OPERATOR_INPUT_SCHEMA` of FR-042-032 in `DefinitionService`. (First task of slice 2.)
  Refs: FR-042-003, FR-042-013, FR-042-032
  Owner-gate: OD-2026-11-27-432
- [ ] T042-44 Before slice 2 starts, re-read sections 4 to 10 against `develop` (the dispatcher, the start path, feature 022
  as merged, the instance registry), decide the refinements of O042-008 and O042-009 and the new numbers (`maxMarks`,
  held-bytes quota, `maxHoldMs`, the admission grace), and amend the design.
  Note (2026-11-26): done by the third revision under Owner decision 2026-11-24-4 (Owner decisions 2026-11-26-130 to
  2026-11-26-136); not ticked, because the doc is not Approved.
  Refs: FR-042-014, FR-042-017, FR-042-021
- [ ] T042-45 Record a blob source's chained manifest: `materialize` bounded by the chain input ceiling, the variant
  read by `enter_collection` and the restore, and tests of a blob with a chain at entry and at restore with `verify_run`.
  Refs: FR-042-010, FR-042-007
  Owner-gate: OD-2026-11-27-432
- [ ] T042-46 Check the output ceilings in the scheduler before `enter_collection`, so the store never answers a chained
  entry with `InvalidTransition`; a test that a chain at each ceiling reaches the store with a valid entry.
  Refs: FR-042-007, FR-042-010
- [ ] T042-47 Add the `collection-operators` switch to `DefinitionService` beside `lake.enabled`, refusing publication
  with a typed error; a test that a stored chained definition parses through `parse_stored` with the switch off.
  Refs: FR-042-003
  Owner-gate: OD-2026-11-27-432
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
- [ ] T042-53 Keys and `distinct` values as digests: SHA-256 over the length-prefixed tenant, chain and canonical bytes
  (FR-042-011, section 4) computed in the process, the absent key as the canonical `null`, no key text in any column;
  tests that one value gives different digests in two tenants and in two chains, that no digest and no key text reaches
  an API response, a log, a span, a metric label, an audit record or an error, that `keyId` and window members are
  returned only under `reveal=pii` with its audit, and that the key-text column the third revision planned does not
  exist; `storage-classification.json` entries with the classes of section 4.
  Refs: FR-042-024, NFR-042-002
  Owner-gate: OD-2026-11-27-424
- [ ] T042-54 Add the capabilities to the instance registry (`Capability::ALL`; `collection-operators-v1` in the small
  migration of Task 042-56, `trigger-operators-v1` in Task 042-13's), list each only where the configuration runs it
  (section 10), the fences of form (a) on publication and `switch` with their descriptors and drain checks, and the
  dispatcher's behaviour without the capability (FR-042-034); tests of both refusals, of an activation refused while a
  live process lacks the capability, of a removal refused while a chain or chained version exists, a held-lock test of a
  publication racing the removal in both orders, a test that a dispatcher without the capability starts no run for a
  chained trigger while its pass still seals, starts and purges, and a deactivation test with a `SEALED` window of a
  removed trigger and a `START_EXPIRED` window whose phase 2 is refused until the pass has refused the first, purged
  both at once because the capability is `draining` (no seven-day wait), and removed every key and mark row, with a
  dispatcher whose switch is off running that pass.
  Refs: FR-042-003, FR-042-034
  Owner-gate: OD-2026-11-26-136
- [ ] T042-55 Seal past the transaction horizon: `operator_windows.seal_horizon`, recorded by a statement whose deadline
  predicate reads `now()`, cleared on every deadline change; the admission statement takes `pg_current_xact_id()` before
  `received_at` in one statement; a race test that holds an admission transaction open past the deadline with a held
  lock and sees the window stay open until it commits or `D + maxHoldMs` passes, and a budget test that the admission
  issues the same statements as before. The admission statement orders the two reads with `MATERIALIZED` common table
  expressions in sequence (the transaction id first), and `scripts/check-architecture.py` refuses an `INSERT INTO
  inbox_events` outside `admit_within`.
  Refs: FR-042-017, FR-042-030
  Owner-gate: OD-2026-11-26-136, OD-2026-11-27-425
- [ ] T042-56 A small migration, delivered with slice 1, that adds `collection-operators-v1` to the registry's closed
  capability checks with its fence function `collection_operators_required()`, so that slice 1 is gated whether or not
  slice 2 ships; `Capability::ALL` gains the value.
  Refs: FR-042-003, FR-042-034
  Owner-gate: OD-2026-11-27-431
- [ ] T042-57 The manual acts as application and store methods: list chains and windows, flush (`OPEN` only, bypassing
  both holds), discard (`OPEN` or `REFUSED`), window retry (`REFUSED` with `START_EXPIRED` only, `start_expires_at`
  reset), `requested_by` recorded for the start's audit line only (the run's starter is the engine), each with its
  `409` code and its audit action with its caller, and tests of each state precondition, of a flush racing an apply in
  both orders, and that a flushed window's run is started by the engine with "requested by" in its `RunStarted` audit
  record; their REST actions are lane A's (Task 042-34).
  Refs: FR-042-018, FR-042-019, FR-042-023, FR-042-026
- [ ] T042-58 Fitness checks in `scripts/check-architecture.py`, each with a negative fixture: the statement that writes
  `seal_horizon` uses `now()` and never `clock_timestamp()`; no transaction that locks an `operator_chains`,
  `operator_keys` or `operator_chain_usage` row locks a `SEALED` window; no statement updates `key_count` on
  `operator_chains`.
  Refs: FR-042-017, FR-042-018, FR-042-021
- [ ] T042-59 Trigger removal: the chain `REMOVED`, its `OPEN` windows discarded and audited, its `SEALED` windows refused
  `TRIGGER_REMOVED` by their start with held values deleted; a test that removing a trigger with a fan-out in progress
  starts no further index.
  Refs: FR-042-019, FR-042-022

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
  capability and fence tests of Task 042-54; the slot tests of Tasks 042-13 and 042-22; the fitness checks of Task
  042-58; the migration's up, down and privilege audit; hand mutations (put the counter back on the chain row, seal on
  `clock_timestamp()`, open a session window without merging, drop the guard of `record_window_start`) that the named
  tests catch.
- **Slice 4** (0.3.0): the target `fault_stream_operators` with the fault and race tests of T042-27 to T042-32 by name;
  the capacity and performance runs state the machine, its load, the PostgreSQL version and configuration, and what
  was skipped.
- **Slice 5** (0.3.0): the OpenAPI baseline (additive, never blessed blind), `npm run api && npm run check:api` in the Studio,
  the trigger conformance scenarios, and the quickstart run end to end including its failure scenario.

Size, stated plainly: 59 tasks, all in release 0.3.0. Slices 0 and 1 are nineteen (Tasks 042-1, 4 to 12, 45 to 52 and
56), most of the server half built and waiting for approval to be ticked, the SDK half and Task 042-56 open; they touch
no table and add no loop. Slices 2 to 5 are forty (Tasks 042-2, 3, 13 to 44, 53 to 55 and 57 to 59): one migration, a
change in the dispatcher's hot path, a new pass and its fault battery, the surfaces in six SDKs and the Studio;
comparable to the first delivery of feature 018, in two lanes.

## Revisions

- **First draft (2026-11-13).** Stated a reading of the owner's sentence and nine questions (O042-001 to O042-009); the
  owner answered them with "Aceitar todas" (OD-2026-11-13-15) and accepted ADR-0086 and ADR-0087 with "Aceito tudo"
  (OD-2026-11-13-17).
- **Second draft.** The architecture, security and correctness reviewers read the first at `540154c8` on 2026-11-13
  (between them 6 Blocker and 29 Major findings, on the text; nothing was built); their findings and how this draft
  answered each are in `docs/designs/042-stream-operators.reviews/2026-11-13/`. The changes of substance: the
  collection chain is bounded by running ceilings on elements and bytes, not by a count of evaluations; a blob source
  has a manifest of its own for a chained scope; the capability is checked at publication, never on read; a chain that
  fails at entry fails the run; the stage variables are `element` and `position`; held trigger values live in a
  deletable column of the chain, never in the append-only payload table.
- **Third revision (2026-11-26).** Brought slices 2 to 5 to build after the re-read of Task 042-44 against `develop`,
  under the owner's delegation (Owner decision 2026-11-24-4): a window sealed only past the transaction horizon of its
  deadline (ADR-0153, refining ADR-0086); both switches capabilities of the instance registry (ADR-0082); seal and start
  two transactions (Gap 042-002 closed); the refinements of O042-008 and O042-009 and the numbers the owner had not
  answered decided as delegate rows (Owner decisions 2026-11-26-130 to 2026-11-26-136); the window start depending on
  feature 022's admission point (section 8). Its revision of 2026-11-27 answered the architecture review
  (`docs/designs/042-stream-operators.reviews/2026-11-26/round-1/architecture.md`): every emission of a stateful chain
  a window; the start writing its window in the run's transaction; a pure domain module and a store port; the chain
  lifecycle following every change of a trigger row.
- **Fourth revision (2026-11-27).** Answers the security, correctness and EGM reviews of round 1
  (`docs/designs/042-stream-operators.reviews/2026-11-27/round-1/`; Decisions 042-168 to 042-179): new keys counted on
  a usage row of their own and marks bounded per key, so concurrent applies never deadlock; session windows that merge
  whatever the order of the applies; the horizon read against the transaction's start, and both holds ending at
  `D + maxHoldMs` over every pending event, with `maxHoldMs` above the claim lease; a window identified by a random id
  and started unfenced, its guarded write in the run's transaction; plain digests and no key text instead of a keyed
  hash and its definer function; a targeted delivery retry; the capabilities under design 053; normative run inputs and
  delivery outcomes; the fan-out checked at the seal and partial when it must be; inline window inputs; ADR-0155. Its
  security re-read (`docs/designs/042-stream-operators.reviews/2026-11-27/reread/security.md`, Decision 042-180) added:
  `keyId`, window members, a delivery's `windowId` and the operator reasons returned only under `reveal=pii`; the pass's
  sealing, starting and purge run whatever the switch, and the drain check waits for no held value, key or mark; the
  held-bytes counter and its trigger function, and the commit-time refusal of the tenant content quota as a wait;
  digests over the tenant, the chain and the value, with their stated residual (Gap 042-017); the delivery retry under
  `FOR NO KEY UPDATE`, `404` for a missing delivery and refused without the capability; the engine as the starter of
  every window's run, the caller kept in the audit; the order point on design 053.
  Its second security re-read (`docs/designs/042-stream-operators.reviews/2026-11-27/reread-2/security.md`, Decision
  042-181) added: value-dependent chains, whose delivery outcomes, counts and windows need `reveal=pii`, with the
  residual of a publisher who sees runs (Gap 042-018); the seal, the flush and the apply checking a `flatMap` list
  against the held-bytes quota and each index input against `MAX_PAYLOAD_BYTES`; the switch limited to the apply and
  the capability listing, and final windows purged at once while draining; the held-bytes counter fed by
  per-transaction deltas flushed at commit; the key insert without a unique violation; window inputs `pii` whatever
  the schema, no member span links, and action responses without reveal.
