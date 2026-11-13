# Stream operators

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-13

--------------------------------------------------------------------------------

## Overview

Reactive-stream operators (the ReactiveX vocabulary: debounce, window, map, flatMap, filter, ignoreElements, skip,
distinct, take) as declared data in the workflow IR, on two surfaces:

- **on a trigger**, between the inbox and the start of a run: an *operator chain* decides which events become a run and
  with what input ("one run per 30-second burst of webhook events for the same order");
- **on a `forEach`**, between the `items` expression and the membership snapshot: the same operators shape the captured
  item list (keep some, project, flatten, drop duplicates, group N items per iteration).

Every predicate, key and projection is an expression of the existing bounded language, evaluated by the engine. No user
code runs. Every piece of state is a PostgreSQL row; a debounce or a time window is a durable deadline closed by a fenced
scheduler pass; a closed window starts exactly one run because the run's idempotency key is derived from the window.

The owner asked for it on 2026-11-13, as more features wanted in release 0.3.0, right after asking for throughput control
and buffering: "Debounce, Window, Map, FlatMap, Filter, IgnoreElements, Skip, Distinct entre outros" (OD-2026-11-13-13).
The sentence names operators and no surface. The first draft stated a reading and nine questions (O042-001 to O042-009),
each with a recommendation; the owner answered them the same day with "Aceitar todas" (OD-2026-11-13-15), so every
recommendation is now the decision. In particular:

- **Release 0.3.0 carries the collection operators only**: slice S0 (the collection half of the foundation) and slice
  S1. No migration, no scheduler change.
- **The trigger operators are a later release**: slices S2 to S5 (`debounce`, `window`, `ignoreElements` and the rest
  on triggers, their state, the pass, the surfaces). They are designed here so that the vocabulary is one; none of
  their fields is in the 0.3.0 schema.

Nothing is built. The registers are in `specs/042-stream-operators/research.md`.

A word on names: "Streams" already means the workflow Streams of feature 026 (ADR-0068). To keep the two apart, the
tables and types of this design say *operator chain*, never *stream*.

## The Problem

**What exists on the event side** (feature 004, `docs/runbooks/events.md`). An event is admitted into `inbox_events`
under the key `(tenant, namespace, source, event_id)`, with `received_at` set by the database and an optional
provider-set `occurred_at`. The dispatcher (`crates/scheduler/src/events.rs`, `EventDispatcher::tick`) claims pending
events and, for each trigger of `workflow_triggers` that matches source and type, evaluates the trigger's one `filter`
(`event_filter`, over `{event, data}`) and starts one run under the idempotency scope `trigger:{id}` and the key
`start_key` derives from the admission (`crates/application/src/dispatch.rs`). `event_deliveries` records that a
consumer received an event, once. So today:

- one event is one run. A provider that sends twelve `order.updated` webhooks in three seconds starts twelve runs; the
  only defence is a `filter`, which cannot see the other eleven;
- deduplication is by event id only (ADR-0012). Two events with different ids and the same business content both act;
- nothing is ordered (gap G74 of feature 004), so "the last event of a burst" has no definition yet;
- a filter that fails to evaluate counts as no match and leaves only a metric (`trigger_events_total{outcome=filter_error}`).

**What exists on the collection side** (feature 014, `docs/runbooks/workflow-primitives.md`). `forEach` evaluates `items`
once, snapshots the membership (`iteration_scopes`, `collection_scope_items`) and runs `body` per item; `map` is an SDK
alias of the same node. There is no way to drop items, flatten a list of lists, remove duplicates or run the body on
groups of N. An author does it in an activity before the collection, which costs a worker round trip and puts list
shaping in code the Studio cannot draw.

**What bounds any answer.**

- The expression language (`crates/infra-expr`, used through `JsonEvaluator` in `crates/infra-ir/src/evaluator.rs`) is a
  bounded CEL subset: comparison, arithmetic, boolean operators, the ternary, field access, indexing, list literals,
  `has`, `size`, `string`, `int`, `double`, `contains`, `startsWith`, `endsWith`. It has no object literal, no macro,
  no regular expression, no hash, no clock and no randomness. Limits: 1,024 bytes of source, 256 tokens, depth 16,
  4,096 constructed list elements, 64 KiB strings (frozen by ADR-0032, decision 7). It is deterministic and runs in the
  engine process. So a predicate and a key are fully expressible; a projection can *select* a value or compute a scalar
  or a list, and cannot *build* an object (G042-001).
- The IR schema is closed (`additionalProperties: false`, a closed `oneOf` of node kinds), and a stored definition is
  validated again when it is read (`parse_stored`). A reader that does not know a field refuses the document; it never
  drops the field (ADR-0032, decisions 2 and 8).
- The scheduler is sharded by a fence (ADR-0020: 64 shards, `scheduler_shards`, each action reads its shard row
  `FOR SHARE` in its own transaction). The event dispatcher is not shard-fenced; it holds a 60-second claim per event.
- A run's inline input is at most 256 KiB (`MAX_PAYLOAD_BYTES`); an event body is at most 240 KiB. A window of whole
  events therefore does not fit a run input unless it is projected first or handed over as an input blob.
- Feature 022 owns *when a requested run may start*: concurrency by key and, in its release 0.3.0 slice, a rate limit
  per key whose excess is buffered durably (US07, proposed ADR-0088). Feature 018 owns schedules and their overlap
  policies. This design must not grow a second rate limiter or a second buffer of runs.

## Requirements

**Vocabulary and language**

- **FR-042-001** The operator set is closed and named in the IR schema. The first delivery has nine operators: `filter`,
  `map`, `flatMap`, `skip`, `take`, `distinct`, `window`, `debounce`, `ignoreElements` (the last two on triggers only).
  Release 0.3.0 has the seven that a collection has. A definition that uses any other name, or an operator on a surface that does not have it, MUST be refused at
  publication with a typed error that names the node or trigger and the position in the chain.
- **FR-042-002** Every predicate, key and projection MUST be an expression of the existing bounded language, validated
  at publication against the existing limits and evaluated by the engine. No operator takes code, a script reference or
  an activity. The language is not extended by this design.
- **FR-042-003** The declarations are additive fields of IR `sagat-octopus-orchestrator.io/v1`, behind two capabilities
  enabled separately, `collection-operators` (release 0.3.0) and `trigger-operators` (later; its fields enter the schema
  with slice S2, not before). With a capability off, publication refuses the field; a binary that predates the field
  refuses the stored document on read. Enabling `trigger-operators` requires
  every scheduler replica to run a binary that knows it (see Technical Plan, Rollout).

**Collections**

- **FR-042-004** A `forEach` MAY declare `operators`, a list of one to eight stages in any order, each over the list the
  previous stage produced. The chain is applied exactly once, in the transaction that snapshots the scope's membership;
  the snapshot records the output and is the only thing a replay, a retry or a restart reads. Stage semantics:
  `filter {where}` keeps the items for which `where` is true; `map {select}` replaces each item by the value of
  `select`; `flatMap {select}` requires `select` to yield a list per item and concatenates those lists in order, one
  level; `skip {count}` drops the first `count`; `take {count}` keeps the first `count`; `distinct {by?}` keeps the
  first occurrence of each value of `by` (of the item when absent), compared by canonical digest; `window {count}`
  replaces the list by consecutive lists of up to `count` items, the last one possibly shorter. Expressions see `item`
  and `index` (the position in the stage's input) and the variables `items` already sees.
- **FR-042-005** `maxItems` bounds the chain's output. The chain's input is bounded by the present source ceilings
  (10,000 items; the blob ceilings of `collection_source`), and the chain by a work ceiling of 20,000 expression
  evaluations per scope entry. Exceeding either, or an expression that fails, fails the `forEach` at entry with a typed
  error (`COLLECTION_OPERATOR_BUDGET`, `COLLECTION_OPERATOR_FAILED`) through the same path as a failing `items`, so
  `onError` applies; nothing is truncated silently. Body `index` and result order follow the output list.

**Triggers**

- **FR-042-006** A trigger of kind `event`, `webhook` or `message` MAY declare `key` (an expression that yields a scalar
  of at most 512 bytes; absent means one key for the whole trigger) and `operators`, one to eight stages in canonical
  order: any number of `filter` and `map`; then at most one each of `distinct`, `skip`, `take`; then at most one of
  `debounce` or `window`; then at most one of `flatMap` or `ignoreElements`. A chain out of order is refused at
  publication (`OPERATOR_ORDER`), as is a chain that can never emit (`OPERATOR_NEVER_EMITS`: `ignoreElements` without
  `debounce` or `window`; `take {count: 0}`). The existing `filter` field of a trigger keeps its meaning and runs first.
- **FR-042-007** An admitted event MUST be applied to a trigger's chain at most once, in the transaction that writes its
  `event_deliveries` row for that trigger, under a row lock on the chain's key. The order in which applications commit
  under that lock is the order of the key's elements (its *position*); it is recorded and never recomputed. A second
  dispatch of the same admission finds the delivery row and changes nothing.
- **FR-042-008** Keyed operators hold their state per chain and key. `distinct {by, horizonMs}` passes an element whose
  `by` value has not passed for this key within `horizonMs` and drops the others; `skip {count}` drops the key's first
  `count` elements; `take {count}` passes the key's first `count` and drops the rest. Counts belong to the key's
  *epoch*, which ends when the key has had no element for `keyIdleMs` (declared; required with `skip` or `take`). An
  expression that fails drops the element and records the failure on the delivery row; it never passes it.
- **FR-042-009** `window` gathers a key's passing elements and emits when it closes. Kinds: `tumbling {sizeMs}`
  (boundaries aligned to the epoch of the database clock, so they do not depend on the first event), `session {gapMs,
  maxWaitMs}` (closes when no element arrived for `gapMs`, or `maxWaitMs` after it opened) and `count {count, maxWaitMs}`
  (closes at `count` elements, or `maxWaitMs` after it opened, emitting what it has). `emit` is `all` (the list, in
  position order), `first` or `last` (one element). `debounce {quietMs, maxWaitMs}` is `window session` with
  `emit: last`. `ignoreElements` after either makes the emission carry only the summary (key, count, first and last
  arrival), and then no element value is stored at all. An empty window does not exist and emits nothing.
- **FR-042-010** Time is the database clock. An element's time is its `received_at`; `occurred_at` is data and drives no
  timer in the first delivery. A window whose deadline has passed MUST NOT close while the namespace's inbox still holds
  a never-attempted pending event that the trigger could match and that was received at or before the deadline. An
  element applied after its window was sealed (possible only for an event whose first dispatch failed and was retried)
  is *late*: it joins the key's open window, or opens one, at its position, is marked late and counted. It is never
  dropped for being late.
- **FR-042-011** Windows are sealed by a scheduler pass fenced as ADR-0020 prescribes. Sealing and applying take the
  same key lock; whichever commits first wins, the other re-reads and acts on what it finds (an application that finds
  its window sealed opens the next; a seal that finds the deadline moved does nothing), and the lost race is counted.
  A sealed window's membership never changes.
- **FR-042-012** A sealed window starts exactly one run: the idempotency scope is the trigger's and the key is derived
  from the window's recorded identity (namespace, chain, key digest, the instant it opened, its sequence), with a
  fingerprint of its membership. The run starts on the workflow version whose chain gathered the window. Its input is
  `{window: {key, sequence, openedAt, sealedAt, count, firstReceivedAt, lastReceivedAt, late, overflow}, events: [...]}`
  for `emit: all`, the element with a `window` summary beside it for `first` and `last`, and the summary alone after
  `ignoreElements`. A chain without `window` or `debounce` keeps today's key (`start_key`), so adding a `filter` or a
  `distinct` to a trigger does not change which run an event starts.
- **FR-042-013** `flatMap {select, maxFanOut}` on a trigger starts one run per element of the list `select` yields for
  an emission, in order, each under the emission's key suffixed with its index. More than `maxFanOut` (at most 100)
  elements, or a value that is not a list, fails the emission as a whole; no partial fan-out is started.

**Limits, retention, boundary**

- **FR-042-014** Ceilings are declared and enforced, and each has one stated outcome. A window reaching `maxCount` (at
  most 1,000) or its byte ceiling (the run input ceiling inline, 4 MiB as an input blob) follows `onFull`: `emit` (seal
  now; the next element opens a new window; the default) or `drop` (the element is dropped and recorded). A new key
  beyond the chain's `maxKeys` (at most 10,000) fails that event's delivery with `QUOTA_EXCEEDED`, which the dispatcher
  retries with its backoff and then dead-letters; existing keys keep working. The sum of `maxKeys` over a tenant's
  chains and the number of chains are tenant quotas checked at publication. Window durations are between one second
  and 24 hours; a `distinct` horizon is at most 7 days.
- **FR-042-015** Chain state is purged by a bounded pass: a key row when it has no open window and its epoch and
  horizons have ended, a `distinct` mark at its horizon, a started, refused or discarded window after the tenant's
  event retention. When a workflow is republished with a different chain, the old chain takes no new element and its
  open windows drain at their own deadlines onto the version that gathered them; when the trigger is removed, its open
  windows are discarded, each audited and counted.
- **FR-042-016** Every row carries tenant and namespace and every statement is scoped by them. Reading chains and
  windows needs `event:read`; flushing, discarding or retrying a window needs `event:manage` and is audited. Another
  tenant's chain or window is indistinguishable from a missing one and the attempt is audited.
- **FR-042-017** Held element values, key text and `distinct` values are event data: they are stored as classified
  content (feature 016 treats event data as personal data as a whole), counted in the tenant's retained bytes, revealed
  only by the `reveal` rules of feature 016, and never appear in a log, a metric, a span attribute, an audit record or
  an error.

**Surfaces**

- **FR-042-018** What became of an event is answerable from durable rows: its delivery to a trigger says `STARTED` (the
  run), `BUFFERED` (the window), `SUPPRESSED` (the operator and why) or `FAILED` (the typed error). The pass has a lag
  metric; chains and collections count elements per operator and outcome; labels are closed enumerations only.
- **FR-042-019** The management API lists a namespace's chains and a chain's windows (state, key digest, count,
  deadline, the events it holds by reference), and flushes, discards or retries one window.
- **FR-042-020** The Studio edits a trigger's key and chain as an ordered list of typed stages with the publication
  rules applied as the author types, shows open windows with their deadline and the pass's lag, and shows on an event's
  page where it is held. Collection operators are shown on a `forEach` node once the Studio knows that node (G042-004).
- **FR-042-021** The six SDKs build both declarations with one set of names, emit identical IR for the same chain, and
  pass common conformance scenarios for the builders and for the runtime.
- **FR-042-022** A chain decides *which events ask for a run and with what input*; features 022 and 018 decide *when a
  requested run may start*. A sealed window's start is a run creator like any other and MUST go through the one
  admission port of feature 022 (FR-022-018, FR-022-041). When that port buffers the run (a concurrency queue, or a
  rate limit: `WAITING`, wait kind `RATE_LIMIT`), the run exists and the window is `STARTED`; what happens to the run
  afterwards, its queue timeout included, is the run's. When the port's buffer is full, the window stays `SEALED` and
  is retried, as 022 leaves an event start pending (FR-022-042). Only a refusal that no retry can change makes the
  window `REFUSED`. No operator declares a rate or a limit.
- **FR-042-023** A trigger without `key` and `operators`, and a `forEach` without `operators`, MUST cost what they cost
  today: no added statement, row or lock, held by a budget test.

**Success criteria**

- **SC-042-001** 500 events for one key inside one quiet period start exactly one run whose input is the last event,
  when every event is dispatched twice and the process is killed once at each write of the apply, seal and start paths.
- **SC-042-002** The seal and an arriving element are forced in both orders with a held lock, and each order ends in the
  state FR-042-011 states, with no element lost or in two windows.
- **SC-042-003** One collection chain per operator, and one of eight stages, produce the same snapshot digest from the
  six SDKs, and a crash between the chain and the first iteration resumes on the recorded snapshot without evaluating
  an expression again.
- **SC-042-004** 50,000 events with 50,000 distinct keys against a chain of `maxKeys` 1,000: 1,000 keys are served,
  the rest fail with `QUOTA_EXCEEDED` and are dead-lettered, another tenant's dispatch latency stays within the stated
  bound, and no table grows beyond its ceiling.
- **SC-042-005** A restart with 1,000 open windows, some past their deadline and some with pending events behind them,
  ends with every window started once and every event in exactly one window.
- **SC-042-006** The budgets of NFR-042-001 hold on the stated environment.

## Goals

- An author says "one run per burst per order" in the definition, and the platform keeps that promise across duplicate
  delivery, a crash and a restart.
- The same nine words mean the same thing in six SDKs, in the Studio and on both surfaces.
- An operator can see every held event, why it is held and until when, and can flush or discard it.

## Non-Goals

- **No user code in an operator**, and no extension of the expression language (decided, O042-005).
- **No rate limit and no concurrency limit.** `throttle`, and anything that paces run starts, is feature 022's
  (`concurrency`, and `rateLimit` with buffering, US07 and ADR-0088). This design neither admits nor queues a run.
- **No trigger operator in release 0.3.0** (decided, O042-002).
- **No event-time semantics** (decided, O042-004). No watermark from `occurred_at`, no allowed lateness, no
  out-of-order correction.
- **No ordering guarantee between keys or between triggers.** The order is per chain and key, as recorded.
- **No operators on `waitEvent`** in the first delivery (decided, O042-003; `filter` on a wait is a follow-up), none
  on outbound subscriptions, none on connector admission (feature 021's FR-021-029 filters there, before the inbox).
- **No multi-stream operators** (`merge` across triggers, `zip`, `combineLatest`, `join`) and no accumulators (`scan`,
  `reduce`).
- **No streaming or unbounded iterators** in collections (feature 014 rules them out); nesting stays in feature 039.
- No change to the inbox key, the dedupe window, the dead-letter rule or `start_key` for a chain without a window.

## Open Questions

The owner questions are answered (next section). Technical questions the build has to answer, registered as gaps:

- **G042-001** `map` cannot build an object, only select or compute. Enough for "the order id and the amount as a list";
  not for a renamed structure. A stated limit: the owner kept the language as it is (O042-005).
- **G042-002** Whether the seal and the run start can share one transaction on the present start path, or must be two
  with the window `SEALED` between them. The design is correct either way (the key is the window's); T042-2 decides.
- **G042-003** `published_collection_definition` refuses `triggers` on a workflow with a `forEach`. A windowed trigger
  whose list feeds a `forEach` is the obvious composition and needs that gate opened by feature 014 or 039.
- **G042-004** The Studio's node registry does not know `forEach` (only the unsupported legacy `foreach`), so collection
  operators have no canvas to appear on until it does.
- **G042-005** The dispatcher is one queue for all tenants (gap G88 of feature 004). A tenant flooding keys costs every
  tenant dispatch time, bounded per event but not isolated. SC-042-004 measures it; it is not fixed here.
- **G042-006** A mixed fleet, with a scheduler replica that predates the capability, would start one run per event for a
  chain's trigger. The rollout rule forbids it; nothing enforces it until the instance registry (ADR-0082) does.
- **G042-007** A single hot key is serial by construction (one row lock per element). The budget states what one key
  sustains; a workload above it needs a finer key, not a faster lock.

## Owner decisions needed

**Answered.** The owner answered O042-001 to O042-009 on 2026-11-13 with "Aceitar todas": each recommendation of the
first draft is the decision, recorded as OD-2026-11-13-15 (the row is added to `docs/owner-decisions.md` on `develop` by
the owner's session, not by this branch). Tasks that depend on an answer carry `Owner-gate: OD-2026-11-13-15`.

| Question | Decision (OD-2026-11-13-15) | Options not taken |
|---|---|---|
| O042-001 Which surface the sentence means | (c) Both: triggers and `forEach`, one vocabulary, two capabilities | (a) events only; (b) collections only |
| O042-002 What goes in release 0.3.0 | (b) The collection operators only: S0 and S1. S2 to S5 are a later release | (a) nothing; (c) collections and trigger debounce; (d) all |
| O042-003 Operators on `waitEvent` | (a) Not in the first delivery; `filter` on a wait as a follow-up | (b) `filter` now; (c) the whole chain |
| O042-004 The time base | (a) Arrival time on the database clock only | (b) also event time with an allowed lateness |
| O042-005 Object construction in `map` | (a) The language stays as it is; `map` selects or computes | (b) an object literal |
| O042-006 The first set and what is deferred | The nine operators of FR-042-001; the deferred list of the catalogue | — |
| O042-007 The numbers | Chain length 8; `maxCount` 1,000; window durations 1 s to 24 h; `maxKeys` 10,000 per chain; 50 chains and 100,000 keys per tenant; `maxFanOut` 100; `distinct` horizon up to 7 days; published limits a measurement may raise | — |
| O042-008 Showing a key | (a) Its digest only, unless the caller may reveal personal data | (b) the key to any `event:read` holder |
| O042-009 A new key beyond `maxKeys` | (a) Fail the delivery, retry, then dead-letter | (b) a run per event; (c) drop |

**Still needed.**

- Accept ADR-0087 (collection operators; before S1 merges) and ADR-0086 (operator chains on triggers; before S2
  starts). Both are written and `Proposed`.
- The release that carries S2 to S5. Not decided; nothing in 0.3.0 depends on it.
- This design is `Approved` by review (constitution, Approval), not by an owner decision.

## Critical User Journeys (CUJs)

### CUJ 1: One run per burst per order

*   **Given**: a trigger `{kind: webhook, endpoint: shop, eventType: order.updated, key: "data.orderId", operators:
    [{debounce: {quietMs: 30000, maxWaitMs: 300000}}]}` and a provider that sends twelve updates for order 81 in three
    seconds, two of them twice.
*   **When**: the events are admitted, and one scheduler replica is killed while applying the seventh.
*   **Then**: the inbox absorbs the two duplicates; ten deliveries say `BUFFERED`; thirty seconds after the last arrival
    the pass seals the window and one run starts with the last update as input and `window.count` 10. The event page of
    each update names the window, and the window names the run.

### CUJ 2: A batch every five minutes

*   **Given**: a `message` trigger with `operators: [{filter: {where: "data.kind == 'reading'"}}, {map: {select:
    "data.value"}}, {window: {kind: tumbling, sizeMs: 300000, maxCount: 1000, emit: all}}]` and no key.
*   **When**: 2,400 readings arrive in one five-minute interval.
*   **Then**: three runs start, two with 1,000 values (sealed by `onFull: emit`, `window.overflow` true) and one with
    400 at the boundary. Each run is admitted by feature 022's rules like any other start.

### CUJ 3: A collection shaped in the definition

*   **Given**: `forEach {items: "steps.fetch.output.orders", operators: [{filter: {where: "item.total > 0"}}, {distinct:
    {by: "item.customerId"}}, {window: {count: 50}}], maxItems: 100, …}` over 3,000 orders of 1,200 customers.
*   **When**: the scope is entered, and the scheduler restarts before the first iteration.
*   **Then**: the snapshot holds 24 items, each a list of up to 50 orders; after the restart the body runs on those 24
    without the chain being evaluated again; results come back in that order.

### CUJ 4: A held event is found

*   **Given**: an operator asked why an order's webhook "did nothing".
*   **When**: they open the event in the Studio.
*   **Then**: it shows the delivery to the trigger as `BUFFERED`, the window, its count and the time it is due, with the
    pass's lag; the key appears as a digest unless they may reveal personal data; they can flush the window, which
    starts the run now and is audited.

### CUJ 5: A flood of keys

*   **Given**: a chain with `maxKeys` 1,000 and a caller that sends events with a random key each.
*   **When**: the thousand-and-first key arrives.
*   **Then**: its delivery fails with `QUOTA_EXCEEDED`, is retried and dead-lettered; the thousand keys keep their
    windows; the tenant's audit shows the quota event; other tenants' events are still dispatched.

## Technical Plan (Architecture)

### 1. Declaration and validation

Two optional fields are added to `specs/001-platform-foundation/contracts/workflow.schema.json`: `operators` on the
`forEach` node, and `key` and `operators` on a trigger item. A stage is an object with exactly one property, the operator
name, whose value is that operator's closed parameter object. The domain gains `OperatorChain` and `Operator` (in
`crates/domain`, beside `triggers.rs` and `workflow.rs`): pure types and the rules that need no I/O (canonical order,
"can emit", ceilings, which operator a surface has). `crates/infra-ir` decodes them and asks the evaluator to check
every expression against the present limits; the two publication gates (`convert::definition` for triggers,
`convert::published_collection_definition` for collections) refuse the field when its capability is off. A chain has a
canonical encoding and a `chain_digest`.

Old readers: the schema is closed, so a binary without the field fails `parse_stored` on such a document with
`IrError::Schema`. It refuses; it does not ignore. Trigger rows are the exception, because the dispatcher reads
`workflow_triggers` without parsing the IR: see Rollout.

### 2. The catalogue

| Operator | Trigger | Collection | State | Emits |
|---|---|---|---|---|
| `filter {where}` | yes | yes | none | the element, when `where` is true |
| `map {select}` | yes | yes | none | the value of `select` |
| `flatMap {select}` | yes, last stage, fan-out of runs | yes, flattens | none | each element of the list |
| `skip {count}` | yes, per key epoch | yes | a counter per key | elements after the first `count` |
| `take {count}` | yes, per key epoch | yes | a counter per key | the first `count` elements |
| `distinct {by, horizonMs}` | yes | yes (no horizon) | a mark per value | the first occurrence |
| `window` | tumbling, session, count | `count` only | a window and its members | a list, or first or last |
| `debounce {quietMs, maxWaitMs}` | yes | no | a window holding one element | the last element |
| `ignoreElements` | yes, after a window | no | a count | the summary only |

`key` is Rx's `groupBy`, as an attribute of the chain rather than an operator: it never changes what is emitted, only
which state an element meets.

Deferred, with the reason:

| Rx name | Why not now |
|---|---|
| `throttle`, `rateLimit` | Pacing run starts is feature 022's. `throttle` with a trailing value is `window tumbling, emit: last` |
| `sample`, `auditTime` | Expressible as `window tumbling` with `emit: last`; no new name until someone needs the alias |
| `buffer` | The same as `window` with `emit: all`; one name is kept |
| `delay` | A timer before a start is a schedule (feature 018) or a `window count {count: 1}`; no clear need |
| Sliding and hopping windows | One element in several windows multiplies members and run starts; needs its own ceilings |
| `distinctUntilChanged` | Cheap (one value per key), deferred only to keep the first set small |
| `takeWhile`, `skipWhile`, `takeUntil`, `skipUntil` | A predicate that ends an epoch needs an epoch reset rule |
| `scan`, `reduce`, `count`, `min`, `max` | An accumulator needs object construction (G042-001) and a reducer contract |
| `merge`, `zip`, `combineLatest`, `withLatestFrom`, `join` | State across triggers; a workflow with several triggers already merges |
| `switchMap`, `concatMap`, `exhaustMap` | What happens to the previous run is an overlap policy (feature 018) or a concurrency rule (022) |
| `retry`, `catch`, `timeout` | The engine's own retries, `onError` and wait timeouts |
| Event-time windows, watermarks | O042-004 |

### 3. Collections

`collection_source` yields the list `items` evaluates to. With `operators`, the application applies the chain to that
list before `iteration_scopes` and `collection_scope_items` are written, in the same transaction, and the snapshot digest
covers the output. Nothing else of feature 014 changes: admission, iteration, result aggregation, compensation and the
capacity rules see an ordinary snapshot. Replay is deterministic for the reason it is today: the snapshot is read, never
recomputed.

The cost is bounded and paid inside the engine: at most eight stages over at most 10,000 input items, cut off at 20,000
evaluations, each linear in at most 256 tokens. The chain runs before the scope's transaction takes its locks where the
present code allows (evaluate, then lock and write), so a long chain does not lengthen a lock hold; T042-9 measures it.
`distinct` uses the digest that `collection_scope_items.item_digest` already uses. No migration.

### 4. Trigger chains: rows

One additive migration. All tables carry `tenant_id` and `namespace_id` first in their primary key.

| Table | Key | Holds | Removed |
|---|---|---|---|
| `workflow_triggers` (columns added) | — | `chain` (canonical), `chain_digest`, `chain_id` | with the trigger |
| `operator_chains` | `chain_id` | workflow, trigger, version that declared it, `chain_digest`, state `ACTIVE` or `DRAINING`, `key_count`, shard | when drained and empty |
| `operator_keys` | `chain_id, key_digest` | key text (content), epoch start, seen and taken counters, last element time, open window sequence, next sequence | idle past epoch and horizons |
| `operator_marks` | `chain_id, key_digest, value_digest` | first seen, `expires_at` | at `expires_at` |
| `operator_windows` | `chain_id, key_digest, opened_at, sequence` | state, `closes_at`, `max_close_at`, count, bytes, first and last `received_at`, late and overflow flags, `version_id`, `run_id`, attempts, last error code, shard | after event retention |
| `operator_window_members` | window key, `position` | source, event id, `received_at`, late flag, digest of the projected value in the payload store | values at start; references with the window |

`chain_id` is a digest of the trigger id and the `chain_digest`, so a republished chain is a new chain and the old one
drains. `event_deliveries` gains `outcome`, a window reference and an error code; it keeps its key, which is what makes
an application happen once. Members hold no foreign key to `inbox_events`: an inbox row leaves at the end of its dedupe
window (as short as a minute), and a window may outlive it, which is why a member keeps the projected value and not a
pointer. With `emit: first` or `last` one member row exists per window; after `ignoreElements` none does.

Window states: `OPEN` → `SEALED` → `STARTED`, or `SEALED` → `REFUSED` (retryable back to `SEALED`), or any open state →
`DISCARDED`. Only `OPEN` accepts members.

Indexes: `operator_windows (shard, closes_at) WHERE state = 'OPEN'`, `(shard) WHERE state = 'SEALED'`, and the purge
orders. Grants to `sagat_app` in the migration's grant block; the down migration refuses while a row exists;
`storage-classification.json` gets an entry per variable-size column (key text, member value digest reference, error
code) and the content tables get the quota-delta triggers (`.claude/skills/migration`).

### 5. Trigger chains: applying an event

In `EventDispatcher`, for a trigger with a chain, the present step "filter, then start" becomes one transaction:

1. insert the `event_deliveries` row for `trigger:{id}`; a conflict means the admission was already applied: stop;
2. evaluate the trigger `filter`, then `key`, then the stateless prefix (`filter`, `map`); a false predicate ends with
   `SUPPRESSED`, a failure with `FAILED`, both recorded on the row just inserted;
3. lock the `operator_keys` row (insert it first when new, under the chain's `key_count`, which is where `maxKeys` is
   enforced exactly; only key creation and removal touch that counter);
4. apply `distinct`, `skip`, `take` against the key's state and the marks;
5. with no window: the element is the emission, started as today under `start_key` (and per element for `flatMap`);
   with a window: find or open the key's window from `received_at`, append the member (or replace it, for `last`),
   move `closes_at` for a session, and seal it at once when `count` or `onFull: emit` says so.

The transaction either commits all of it or none; a crash anywhere leaves the event pending and the claim expires. An
event that fails here is retried by the dispatcher's existing backoff and dead-lettered by its existing rule.

### 6. Trigger chains: the pass

A new pass, `operator_windows`, in `crates/scheduler/src/passes.rs`, on the shards the replica holds (the window's shard
is derived from chain and key, as feature 018 does for schedules). Each tick, in bounded batches:

- **seal**: for `OPEN` windows with `closes_at` or `max_close_at` at or before the database clock, compute once per
  namespace and event type the oldest never-attempted pending inbox event; skip the windows it is not past (counted
  `held_by_backlog`); for the others, in one transaction each: read the shard row `FOR SHARE`, lock the key row, re-read
  the window, and seal it if it is still due. A deadline that moved is a no-op, counted `lost_race`;
- **start**: for `SEALED` windows: build the input from the members (inline, or an input blob above the inline ceiling),
  start the run through the ordinary start path with the window's key and fingerprint, record `run_id`, set `STARTED`
  and release the member values. A retryable refusal leaves it `SEALED` with a backoff; a final one sets `REFUSED`;
- **purge**: keys, marks and finished windows past their retention, and `DRAINING` chains with nothing left.

Nothing is held in memory between ticks: a replica that dies mid-tick leaves rows another replica acts on, and a stale
replica's transaction fails its fence. The lag is `scheduler_lag_seconds{pass="operator_windows"}`: the age of the oldest
window that is due and not held by the backlog.

### 7. Semantics under failure, stated once

| Case | Outcome |
|---|---|
| Same event id inside the dedupe window | Absorbed by the inbox; the chain never sees it |
| Same event id after the window | A new admission and a new element, as today |
| Same business content, different ids | Two elements, unless `distinct` is declared |
| An admission dispatched twice (lost claim, lost broker hint, two replicas) | One application: the delivery row's key |
| Crash during the apply transaction | Nothing applied; the event is dispatched again |
| Crash between seal and start | The window is `SEALED`; the pass starts it under the same key |
| Start retried after an unknown commit | The start path finds the key; one run |
| Events out of order | Position is the order of application under the key lock; `occurredAt` travels as data |
| An event applied after its window sealed | Late: next window, flagged, counted |
| Deadline and element at the same instant | The key lock orders them; both orders are valid and tested |
| Scheduler down past many deadlines | On return the backlog rule lets pending events in before the windows seal |
| Publication of a different chain | The old chain drains on its version; the new one starts empty |
| The run a window started fails | It is a run like any other; the window stays `STARTED` and is not re-emitted |

Replay: a run's input is materialised at its start and the run never reads chain state, so replaying the run does not
depend on what the chain holds later. The chain is not replayed; it is the record.

### 8. Composition with features 022 and 018

Feature 022's release 0.3.0 slice (`specs/022-flow-control-step-policies`, US02 and US07; ADR-0056 and the proposed
ADR-0088) limits how many runs or steps of a key are in flight (`concurrency`) and how many start per interval
(`rateLimit`: a strict sliding window of durable grants on the database clock, the excess buffered as `WAITING` runs
or steps and granted by a fenced scheduler pass). It says what it is not: the tenant rate quotas of feature 009
(`events_per_second`), which refuse and never buffer, and 018's overlap policies, which buffer a schedule's actions
before a run exists. This design is a third thing, upstream of all of them:

| Stage | Owner | Unit | What it holds | What it decides |
|---|---|---|---|---|
| Inbox admission | 004, 009 | an event | nothing; refuses with `429` | whether the event is recorded |
| Operator chain | 042 | an event, per chain and key | elements, before any run exists | which events ask for a run, and its input |
| Schedule overlap | 018 | a schedule action | actions, before a run exists | whether a fire starts a run now |
| Run admission | 022 | a run, per concurrency or rate key | runs that exist and wait | when a requested run starts |

The rules that follow:

1. **One creator port.** The pass that starts a sealed window is a run creator and uses the one admission port
   (FR-022-018, FR-022-041). When S3 is built it is added to 022's run-creator test (T022-086) or to its successor.
2. **Buffered is started.** A run that 022 buffers exists, has its journal (`rate_limit.queued`) and counts against
   022's ceilings. The window is `STARTED` the moment the run exists. A queue timeout (`RATE_LIMIT_WAIT_TIMEOUT`) fails
   that run; the window is not re-emitted and its elements are not gathered again.
3. **A full buffer is a retry.** 022 leaves an event start pending under its retry when its buffer is full
   (FR-022-042, O1165). The same holds for a window: it stays `SEALED`, with a backoff, under the same key. A
   chain with `flatMap` treats its fan-out as all-or-nothing against that answer.
4. **No operator paces.** `throttle` and anything that spreads starts over time is `rateLimit`. An author who wants
   "at most 10 bursts a second per customer" declares `debounce` here and `rateLimit` there; the keys are independent
   expressions and may differ (the chain's key sees the event, 022's key sees the run input, which for a window is
   the shape of FR-042-012).
5. **Two things named window.** 022's window is an interval over which grants are counted; this design's window is a
   set of gathered elements. The tables differ (`operator_windows` here) and the runbooks say which is meant.
6. **Lock order.** ADR-0088, decision 4, puts its window rows after every entity lock and the concurrency semaphore,
   and forbids locking a run after them. The start transaction of this design locks the chain's key row and window
   first, as entity locks, then creates the run through the port. It never touches a chain row after the port has
   taken a semaphore or a rate row. ADR-0086 records this; T042-3 proves it with a barrier.
7. **Collections (0.3.0).** A chain changes how many iterations a `forEach` has and nothing about their admission.
   022 refuses `rateLimit` on a body node in its 0.3.0 slice (`SCOPED_RATE_LIMIT`), so the two features do not meet
   inside a collection in this release.

A schedule's fire is not an event and passes no chain; a schedule's overlap policy and a chain's window are independent.

### 9. Surfaces

- **REST** (`specs/001-platform-foundation/contracts/openapi.yaml`, additive): `GET /v1/namespaces/{ns}/operator-chains`,
  `GET …/operator-chains/{chainId}/windows`, `GET …/windows/{windowId}`, `POST …/windows/{windowId}:flush`, `:discard`,
  `:retry`; the event resource's deliveries gain `outcome`. Typed errors with `code`, `retryable`, `traceId`.
- **Audit** (`AuditAction`): `operator.window_flushed`, `operator.window_discarded`, `operator.window_retried`,
  `operator.window_refused`, and the existing quota event for `maxKeys`. Publication is already audited.
- **Metrics** (closed labels): `operator_elements_total{surface, operator, outcome}`,
  `operator_windows_total{outcome}`, `operator_open_windows`, `operator_window_seal_total{outcome}`,
  `operator_window_emit_lag_seconds`, and the pass's `scheduler_lag_seconds`.
- **Spans**: `operator.apply` on the event's trace; `operator.window.start` with links to at most 32 member traces. The
  run's first journal event records that a window started it (chain, sequence, count), so the Studio's graph stays a
  projection of the journal.
- **Studio**: the `Triggers` editor in `features/workflows/WorkflowSettings.tsx` gains the key and the stage list; a
  window list per workflow; the event page shows the delivery outcome.
- **SDKs**: chainable builders on the trigger and on `forEach`/`map` in the six SDKs (`filter`, `map`, `flatMap`, `skip`,
  `take`, `distinct`, `window`, `debounce`, `ignoreElements`, `keyBy`, in each language's casing), which only assemble
  the list; the server validates. Scenarios in `tests/contract/sdk-conformance/scenarios`: `collection-operators`
  (build), `collection-operators-runtime`, `collection-operators-limits`, `trigger-operators` (build),
  `trigger-debounce-runtime` and `trigger-window-runtime` (which close their windows with `:flush`, not by waiting).

### 10. Rollout

Per ADR-0032, decision 8: ship readers first with both capabilities off; enable `collection-operators`; enable
`trigger-operators` only when every scheduler replica runs the new binary, because an older dispatcher reads the trigger
row without the chain and would start a run per event (G042-006). That failure over-delivers and loses nothing, which is
why the chain is stored on the trigger row and not in a table an old binary would not look at. Rollback disables
publication and keeps the pass running until open windows drain.

### 11. Decisions recorded

- **ADR-0086 (proposed)**: an operator chain on a trigger is declared data applied by the engine; its state is rows per
  chain and key; time is the database clock at admission; a fenced pass seals a window, and a sealed window starts one
  run by its identity through the one admission port. For slices S2 to S5.
- **ADR-0087 (proposed)**: collection operators are applied once at scope entry and the snapshot is the record. For
  release 0.3.0.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A window that never closes holds events for ever | `maxWaitMs` is required on session and count windows; durations have a ceiling; the lag metric and the window list show a held window; flush and discard exist |
| The backlog rule lets a stuck dispatcher hold every window | Only never-attempted events hold; `held_by_backlog` is counted; a dispatcher that is down is already an incident with its own lag metric |
| A hot key serialises a tenant's events | Stated (G042-007) and budgeted; `debounce` and `emit: last` update one row |
| Chain state grows without bound | Every table has a ceiling and a purge with an owner; SC-042-004 |
| Personal data sits in a buffer nobody knows about | Classified as content, counted in retained bytes, listed by the API, released at start; `ignoreElements` stores none |
| An old scheduler replica ignores the chain | Rollout rule; the failure direction is over-delivery, not loss (G042-006) |
| The collection chain makes scope entry slow | Work ceiling, evaluation before the locks, a measured budget (T042-9) |
| Two rate limiters appear (here and in 022) | No operator paces a start; section 8 states the boundary against 022's US07; T042-3 tests it |
| Nine operators on two surfaces multiply the tests | A canonical order on triggers; one evaluator; conformance scenarios shared by six SDKs |
| The trigger half is designed long before it is built and drifts | Its fields stay out of the 0.3.0 schema; T042-44 re-reads this design against `develop` before S2 starts |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | All chain state is rows; an application commits with its delivery row; seal and start are idempotent by the window's identity; deadline against element is first commit wins, the loser counted (FR-042-007, 011, 012) |
| II. Modular domain, ports and adapters | Pass | `OperatorChain` and its rules are domain types without serde_json, SQL or a clock; statements in `infra-postgres`; decoding in `infra-ir` |
| III. External effects are at-least-once; sagas compensate | Pass | A chain has no external effect; it starts runs through the ordinary path and promises one start per window, not one execution of a step |
| IV. Compatibility is versioned and pinned | Pass with conditions | Additive v1 fields behind two capabilities; a window starts on the version that gathered it; six SDKs share scenarios. Condition: a mixed scheduler fleet is unsupported until the capability is fleet-wide (Complexity Tracking, G042-006) |
| V. Code, tenant boundary and secrets are isolated | Pass | Only bounded expressions, evaluated by the engine, as trigger filters are today; no user code, no secret; tenant and namespace in every key; held data classified (FR-042-002, 016, 017) |
| VI. Observability and audit are part of the feature | Pass | Delivery outcome per event, lag per pass, counters with closed labels, audit of every manual act, the run's journal names its window (FR-042-018) |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | The chain sits after the inbox and applies once per admission; a ceiling is backpressure and then dead-letter; a lost hint changes nothing because the pass reads rows |
| VIII. Failure is tested and contracts are verified, honestly | Pass | SC-042-001 to 005 and tasks T042-27 to T042-32: crash at each write, duplicate and lost delivery, both race orders with a held lock, a key flood, restart with open windows |
| Delivery Constraints | Pass | Single region, PostgreSQL as authority, no new component; budgets are targets on a stated environment |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Five tables of chain state | A debounce that survives a restart is a row with a deadline | State in the dispatcher's memory: lost on restart, wrong with two replicas |
| A new fenced pass | A deadline must fire with no event to trigger it | Sealing only when the next event arrives: the last burst never starts |
| The backlog rule on sealing | Without it a slow dispatcher splits a burst in two runs | Sealing on the clock alone: the result depends on dispatcher lag |
| A canonical stage order on triggers | Arbitrary orders give each pair of operators its own semantics and tests | Any order: `skip` after `window` and before it mean different things, and both need state |
| Two capabilities | The collection half has no migration and can ship alone | One switch: the small half waits for the large one |
| A mixed fleet unsupported for trigger chains | The dispatcher reads trigger rows without the IR | A registry check at publication: it does not exist yet (ADR-0082) |

## Alternatives Considered

- **Operators as code** (a function per stage, run in the sandbox). The most expressive and the reverse of what the
  platform is: a round trip per event, no Studio drawing, nondeterminism to police. Rejected by principle V and rule 7.
- **Debounce as a concurrency policy of feature 022** ("a new start cancels the queued one"). It covers "last wins" and
  nothing else (no window, no distinct, no list), and it makes a start request something that can be withdrawn.
- **A separate "stream" resource with its own REST lifecycle**, bound to workflows. More flexible (one chain feeding two
  workflows) and a second place where a workflow's behaviour is defined, outside the pinned version.
- **Windows keyed on `occurredAt`.** O042-004.
- **Buffering by reference to the inbox row.** The row may expire before the window closes; lengthening the dedupe
  window to fit couples two unrelated settings.
- **Emitting in the dispatcher's transaction when a count is reached.** A second start path; sealing there and starting
  in the pass keeps one.
- **Collection operators as engine-run "virtual activities".** A journal entry per stage for work that is a pure
  function of one input; the snapshot already records the result.

## Non-Functional Requirements

- **NFR-042-001 Performance** (targets on a stated environment, not promises; measured by T042-33): an event joining an
  open window costs at most 6 statements in the one transaction that also records its delivery; sealing and starting a
  window at most 8 statements beyond the run start; a trigger without a chain costs zero more (FR-042-023, by equality
  in a budget test in the style of design 038); 1,000 events per second across 10,000 keys with the pass's lag under 2
  seconds at the 99th percentile; the rate one key sustains is reported, not targeted; a collection chain of 8 stages
  over 1,000 items adds at most 20 ms to scope entry at the 95th percentile.
- **NFR-042-002 Security**: no `SECURITY DEFINER`, no new role; the runtime role gets only the table grants the
  statements need, checked by the privilege audit; the cross-tenant battery covers every new endpoint and table.
- **NFR-042-003 Operability**: the runbook says what each metric means, what a held window is and how to flush or
  discard one; an alert rule on the pass's lag ships with it.
- **NFR-042-004 Compatibility**: every existing definition, trigger and run behaves as before; the OpenAPI and IR schema
  changes are additive and pass the contract baselines.
- **NFR-042-005 Storage**: with the default quotas a tenant's chain state is bounded (100,000 keys; open windows no more
  than keys; members no more than `maxCount` per window; marks purged at their horizon), and the bound is stated in the
  runbook in bytes after measurement.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

**Release 0.3.0 (OD-2026-11-13-15): S0 and S1.** **S0** foundation, collection half (T042-1, 4, 5, 6); **S1**
collection operators (T042-7 to 12). No migration and no scheduler change.

**A later release, not yet named: S2 to S5.** **S2** chain state and the operators without time (T042-2, 3, 13 to 19,
43, 44); **S3** windows and the pass (20 to 26); **S4** failure and capacity (27 to 33); **S5** surfaces and close (34
to 42). None of it starts before ADR-0086 is accepted.

- [ ] T042-1 Confirm that OD-2026-11-13-15 is a row of `docs/owner-decisions.md` on `develop` and that its `Covers`
  names every task gated on it here, before any of them is ticked.
  Refs: FR-042-001
- [ ] T042-2 Spike, in `specs/042-stream-operators/research.md`: whether seal and start can share a transaction through
  the admission port (G042-002), the input-blob path for a window above the inline ceiling, and the statement count of
  today's dispatch. (Later release.)
  Refs: FR-042-012, FR-042-023
- [ ] T042-3 Prove the composition with feature 022 as built: the window start goes through the one admission port and
  is in the run-creator test; a window whose run is buffered by a rate limit is `STARTED`; a full buffer leaves it
  `SEALED`; the chain rows are locked before the semaphore and the rate rows, with a barrier test of the reversed order.
  (Later release.)
  Refs: FR-042-022
- [ ] T042-4 Have ADR-0087 accepted before S1 merges and ADR-0086 before S2 starts; amend the design if either changes.
  Refs: FR-042-003, FR-042-010
- [ ] T042-5 Add `OperatorChain` and `Operator` to the domain with the collection rules (the seven operators, chain
  length, ceilings), and property tests of the rules.
  Refs: FR-042-001, FR-042-004
  Owner-gate: OD-2026-11-13-15
- [ ] T042-6 Extend the IR schema and the codec with `forEach.operators`, the expression checks and the
  `collection-operators` gate; fixtures prove every earlier document still decodes, an unknown operator is refused and
  a trigger operator is refused.
  Refs: FR-042-002, FR-042-003, NFR-042-004
  Owner-gate: OD-2026-11-13-15
- [ ] T042-7 Apply a collection chain at scope entry, before the snapshot, with the seven collection operators.
  Refs: FR-042-004
  Owner-gate: OD-2026-11-13-15
- [ ] T042-8 Enforce the collection ceilings and typed errors, with `onError` reached.
  Refs: FR-042-005
- [ ] T042-9 Test replay and restart on the recorded snapshot, and measure scope entry with and without a chain.
  Refs: SC-042-003, NFR-042-001
- [ ] T042-10 Add the collection builders to the six SDKs.
  Refs: FR-042-021
- [ ] T042-11 Add the scenarios `collection-operators`, `collection-operators-runtime` and `collection-operators-limits`
  with reviewed goldens.
  Refs: FR-042-021, SC-042-003
- [ ] T042-12 Document collection operators in the workflow-primitives runbook, the reference and the traceability map.
  Refs: FR-042-004, NFR-042-003
- [ ] T042-13 Write the migration (tables, columns, indexes, grants, classification entries, quota triggers, a down that
  refuses data loss) with the migration checklist.
  Refs: FR-042-016, FR-042-017, NFR-042-002
- [ ] T042-14 Register a chain when a version is published, drain the old one, discard on removal, and check the tenant
  quotas at publication.
  Refs: FR-042-014, FR-042-015
- [ ] T042-15 Apply an event to a chain in the delivery transaction: key, stateless prefix, key row, position.
  Refs: FR-042-007
- [ ] T042-16 Add `distinct`, `skip` and `take` with epochs and marks.
  Refs: FR-042-008
- [ ] T042-17 Record the delivery outcome and expose it on the event resource.
  Refs: FR-042-018
- [ ] T042-18 Enforce `maxKeys` at key creation and the dead-letter path beyond it (O042-009).
  Refs: FR-042-014
  Owner-gate: OD-2026-11-13-15
- [ ] T042-19 Hold FR-042-023 with a budget test: a trigger without a chain adds no statement.
  Refs: FR-042-023, NFR-042-001
- [ ] T042-20 Open, extend and seal-on-count windows in the apply transaction (tumbling, session, count; all, first,
  last; `onFull`), with the numbers of O042-007.
  Refs: FR-042-009, FR-042-014
  Owner-gate: OD-2026-11-13-15
- [ ] T042-21 Add the `operator_windows` pass: fenced seal with the backlog rule.
  Refs: FR-042-010, FR-042-011
- [ ] T042-22 Start a sealed window's run by its identity, on its version, through the one admission port, with the
  three input shapes; `SEALED` with a backoff on a full buffer, `REFUSED` and retry.
  Refs: FR-042-012, FR-042-022
- [ ] T042-23 Add `debounce` and `ignoreElements` over the window machinery.
  Refs: FR-042-009
- [ ] T042-24 Add `flatMap` fan-out on a trigger with its ceiling and all-or-nothing start.
  Refs: FR-042-013
- [ ] T042-25 Mark and count late elements; test an event retried after its window sealed.
  Refs: FR-042-010
- [ ] T042-26 Purge keys, marks, finished windows and drained chains in bounded batches.
  Refs: FR-042-015, NFR-042-005
- [ ] T042-27 Fault test: kill at each write of apply, seal and start; 500 events, one key, one run.
  Refs: SC-042-001
- [ ] T042-28 Fault test: every admission dispatched twice, and a lost broker hint, change nothing.
  Refs: SC-042-001, FR-042-007
- [ ] T042-29 Race test: seal against element in both orders, forced with a held lock, never a sleep.
  Refs: SC-042-002, FR-042-011
- [ ] T042-30 Fault test: restart with 1,000 open windows, deadlines passed and events pending.
  Refs: SC-042-005
- [ ] T042-31 Capacity test: 50,000 keys against `maxKeys` 1,000, with a second tenant's latency measured.
  Refs: SC-042-004
- [ ] T042-32 Cross-tenant and privilege tests for every new table and endpoint, also as the runtime role.
  Refs: FR-042-016, NFR-042-002
- [ ] T042-33 Measure NFR-042-001 on a stated environment and write the dated validation report.
  Refs: SC-042-006, NFR-042-001
- [ ] T042-34 Add the REST endpoints and audit actions for chains and windows, with the contract baseline.
  Refs: FR-042-019, FR-042-016
- [ ] T042-35 Add the metrics, spans, the journal note on a window-started run and the lag alert rule.
  Refs: FR-042-018, NFR-042-003
- [ ] T042-36 Add the trigger builders to the six SDKs.
  Refs: FR-042-021
- [ ] T042-37 Add the scenarios `trigger-operators`, `trigger-debounce-runtime` and `trigger-window-runtime`.
  Refs: FR-042-021
- [ ] T042-38 Studio: the key and stage editor on a trigger with the publication rules.
  Refs: FR-042-020
- [ ] T042-39 Studio: the window list, the lag, the event page's outcome, flush and discard; a key shown as its digest
  unless the caller may reveal personal data (O042-008).
  Refs: FR-042-020, FR-042-017
  Owner-gate: OD-2026-11-13-15
- [ ] T042-40 Write the events runbook section, the quickstart of CUJ 1 with its failure scenario, and the reference.
  Refs: NFR-042-003
- [ ] T042-41 Update `docs/traceability.md`, the roadmap row and the release notes; register the remaining gaps.
  Refs: NFR-042-004
- [ ] T042-42 Write the rollout procedure (readers first, fleet-wide before `trigger-operators`, drain on rollback) and
  rehearse it on the compose stack.
  Refs: FR-042-003

- [ ] T042-43 Extend the domain rules, the IR schema and the codec with a trigger's `key` and `operators` (canonical
  order, "can emit", ceilings) and the `trigger-operators` gate. (Later release, first task of S2.)
  Refs: FR-042-003, FR-042-006
  Owner-gate: OD-2026-11-13-15
- [ ] T042-44 Before S2 starts, re-read sections 4 to 10 against `develop` (the dispatcher, the start path, feature 022
  as merged, the instance registry) and amend the design, which makes it a draft again if it was approved.
  Refs: FR-042-007, FR-042-022

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the three
checkers, the unit tests of the crates it changes and the suites it touches, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`
where it adds a statement. A test is seen failing for the right reason before the code that passes it.

- **S0** (0.3.0): the IR contract tests with the earlier fixture corpus unchanged; domain property tests.
- **S1** (0.3.0): `integration_workflow_primitives` (unmodified tests still pass; new tests for each operator, the ceilings and
  restart on the snapshot); `sdk_conformance` for the three collection scenarios in six languages; a hand mutation per
  operator (invert the predicate, drop the last chunk, flatten two levels) to prove the tests can fail.
- **S2 and S3** (later): a new target `integration_stream_operators` on real PostgreSQL; the events suites of feature 004
  unmodified; the budget test of T042-19; the migration's up, down and privilege audit.
- **S4**: the fault and race tests of T042-27 to T042-32 by name, with barriers and held locks; the capacity and
  performance runs state the machine, its load, the PostgreSQL version and configuration, and what was skipped.
- **S5**: the OpenAPI baseline (additive, never blessed blind), `npm run api && npm run check:api` in the Studio, the
  trigger conformance scenarios, and the quickstart run end to end including its failure scenario.

Size, stated plainly: 44 tasks, about sixteen pull requests. Release 0.3.0 takes ten tasks (T042-1, 4 to 12), about
five pull requests, and touches no table and no scheduler loop. S2 to S4 are the bulk (a migration, a change in the
dispatcher's hot path, a new pass and its fault battery), comparable to the first delivery of feature 018, and belong
to the later release.
