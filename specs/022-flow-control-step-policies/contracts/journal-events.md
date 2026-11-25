# Feature 022 — journal events (draft)

Additive `run_events.event_type` values, written by the reducer in the transaction of the transition
(transaction numbers refer to the plan). Every event is payload-free: a value is referenced by the
existing `payload_ref` column (SHA-256 of a payload the run retains), never placed in `detail`. `detail`
keys are snake_case and closed, like the existing ones (`due_at_us`, `before_revision`); instants are
integer microseconds (`*_us`). Old readers treat an unknown event type as "something happened" (Studio
shows it as an unsupported event, never an error). `verify_run` replays them.

| Event | Transaction | `detail` (closed) | Reducer effect |
|---|---|---|---|
| `step.cache_hit` | T1 | `entry_id`, `source_run_id`, `source_node`, `source_attempt`, `expires_at_us`; `payload_ref` set | followed by `step.completed` with `source: "CACHE"`, attempt 0; no lease; a `step_cache_hit_provenance` row is written with it |
| `step.cache_missed` | T1 (added 2026-11-24, decision D022-203) | `outcome` ∈ `miss`, `expired`, `invalidated`, `unauthorized`, `bypassed` | the step leaves `WAITING` (`CACHE_LOOKUP`) for `READY`, or for its gates' round in the same commit; no payload |
| `step.mocked` | T16 or when reached | `source` ∈ `MOCK`, `RUN_PIN`; `source_run_id`, `floor_level` (both optional); `payload_ref` set | step `SUCCEEDED` with origin `MOCK`; excluded from compensation/reconciliation planning |
| `result.returned` | T6 | `result_source: "EARLY_RETURN"`; `payload_ref` set | sets the run result once; status unchanged |
| `end.reached` (existing) | existing | adds optional `result_already_returned: true` | no second result |
| `failure_handler.started` | T7 | `pending_status`, `form` ∈ `NODE`, `CHILD`; `failure_kind`, `failure_code` | enters handler phase |
| `failure_handler.draining` | T32 (added 2026-11-25, decision D022-246) | `cause` ∈ `CANCELLED`, `TIMEOUT`, `REGION_FAILED`, `STORAGE_QUOTA`, `RUN_DATA_LIMIT`, `EXPRESSION_ERROR` (one cause: ~~`later_causes`~~ removed 2026-11-25, decision D022-261; a further stop request while draining is refused `409`) | the phase drains: `failure_handler_state` `TIMING_OUT` for `TIMEOUT`, `CANCELLING` otherwise; the region's attempts and the child asked to stop; every stop source of the phase maps here, never to the run's stop |
| `failure_handler.finished` | T8 | `outcome` ∈ `SUCCEEDED`, `FAILED`, `FAILURE_HANDLER_TIMEOUT`, `CANCELLED` | followed by `run.finished` with the pending status |
| `concurrency.queued` | T10 | `pool`, `scope: "RUN"`, `group`, `key_label`, `queue_deadline_us` | run `WAITING` (wait kind `CONCURRENCY`) |
| `concurrency.acquired` | T10, T11, T13 | `pool`, `scope`, `group`, `key_label`, `generation` (step scope) | holder open |
| `concurrency.released` | T8, T14 | `pool`, `scope`, `group`, `key_label` | holder closed |
| `concurrency.wait_timed_out` | T12 or step timeout | `pool`, `scope`, `group`, `key_label` | run or step fails `CONCURRENCY_WAIT_TIMEOUT` |
| `concurrency.dequeued` | cancellation, run deadline | `pool`, `group`, `key_label` | waiter removed; no permit |

| `rate_limit.queued` | T19, T22 | `pool`, `scope` ∈ `RUN`, `STEP`; `group`, `key_label`, `limit`, `period_ms`, `queue_deadline_us` | run or step `WAITING` (wait kind `RATE_LIMIT`); a step spends no attempt |
| `rate_limit.granted` | T19, T20, T22 | `pool`, `scope`, `group`, `key_label`, `limit`, `period_ms`, `granted_at_us`, `from_queue` (boolean) | run: first tasks follow in the same transaction; step: `READY` |
| `rate_limit.wait_timed_out` | T21 | `pool`, `scope`, `group`, `key_label` | run or step fails `RATE_LIMIT_WAIT_TIMEOUT` |
| `rate_limit.dequeued` | T24 | `pool`, `scope`, `group`, `key_label` | waiter removed; no slot consumed |
| `rate_limit.returned` | T29 (review 2026-11-13) | `pool`, `scope: "STEP"`, `group`, `key_label`, `attempt` | a granted, unleased step's slot is returned; the step waits again or ends |

*Revised after the review of 2026-11-13 (D022-019, D022-029):* every step-scope `concurrency.*` and
`rate_limit.*` event carries `attempt` (the attempt the grant is for, the step's current attempt plus one);
`concurrency.queued` and `concurrency.wait_timed_out` also exist with `scope: "STEP"` (a step waiting for a
permit is `WAITING`, wait kind `CONCURRENCY`); `concurrency.acquired` of a step is written at its grant (T26,
T28), not at its claim; `concurrency.released` is also written by T29.

The four `rate_limit.*` rows belong to the release 0.3.0 slice (US07, D022-012). A rate slot has no
release event: it stops counting by time, which replay never needs to know (FR-022-045).

*Amended 2026-11-24 (decisions D022-203 to D022-216; ADR-0135, ADR-0136 and ADR-0137 proposed):* the transaction
numbers refer to the as-built table of `plan.md` ("Transactions, as built"). `step.cache_hit` follows the lookup round
(`Evaluation::CacheLookup`), asked before the gates; its `payload_ref` is the entry's payload row, referenced, not copied.
A step of a node that declares `cache` waits `WAITING` (`CACHE_LOOKUP`) between its first `step.released` and its
`step.cache_hit` or `step.cache_missed`. `step.mocked` of a downstream pin is written in the commit that releases the
node, before any lookup or gate. `failure_handler.started` replaces `run.finished` until `failure_handler.finished`, which
commits with the start's `concurrency.released` and `run.finished`. `result.returned` follows the round
`Evaluation::EarlyResult`.

*Revised 2026-11-25 (decisions D022-224, D022-228, D022-230, D022-233, D022-238):* `step.cache_missed` is one of the
task-ready hint's arms and writes the lookup's key digest and version on the step projection (`steps.cache_lookup`), never
in `detail`; no step is released while its run is paused, so no lookup waits across a commit. An upstream pin commits
`step.mocked` and the existing `step.routed` with no activation, which releases no successor. `failure_handler.started`
moves the stop into `pending_terminal_status` and clears it; a cancellation, the run deadline or the handler's timer
during the phase drains it (`failure_handler_state` `CANCELLING` or `TIMING_OUT`) before `failure_handler.finished`, whose
`outcome` `FAILURE_HANDLER_TIMEOUT` the column records as `TIMED_OUT`; a phase never opens a second one. `run.started`
also carries `start_at` (a draft test's start node), `handler_tree` and `handler_kind` (`DISPATCHED`, `REGION_CHILD`).

*Revised 2026-11-25 after the re-reviews (decisions D022-242, D022-246, D022-253), and after the third reads (decision
D022-260: the `run.finished` row is appended after the two dispatch statements, its detail taken from what they did; the
reducer ignores it; the terminal path updates no `run_events` row):* `run.finished` gains an optional
`namespace_dispatch` detail (`PENDING` or `SUPPRESSED_LINEAGE`) when its commit inserted a dispatch row, so a suppressed
dispatch is on the run's journal; `failure_handler.finished`'s `outcome` is `FAILED` after a draining cause other than
`CANCELLED` and `TIMEOUT`; a reset run's lineage is always its source's.

`run.started` (existing) gains optional provenance `handler_for`, `origin` and `test_tree`. `key_label` is
the 12-hex-character prefix of the keyed digest (D1168), never the key value.

## Reset rules (feature 018 registry)

*Implemented 2026-11-15 (D022-035):* until a reset admits its run through the gate (T022-064, T022-086), the registry
declares all ten `concurrency.*` and `rate_limit.*` types `EXCLUDE`, so no point after one is offered. The table below is
the rule those tasks install. *2026-11-20 (T022-064, D022-053):* the five `concurrency.*` types have the rules of the
table; the five `rate_limit.*` types stay `EXCLUDE` until T022-086. *2026-11-21 (T022-086, D022-069):* the five
`rate_limit.*` types have the rules of the table too, and `RATE_LIMIT_SLOT_UNAVAILABLE` is registered in 018's contract.
A restored gate event carries `restored: true` in its otherwise closed detail. *D022-036:* every one of the ten carries `scope` in its detail (also `concurrency.dequeued`)
and, at step scope, `attempt`.

Feature 018 keeps the reset-rule registry in `specs/018-schedules-bulk-operations/contracts/journal-and-events.md`
and refuses a reset point whose prefix contains an event type without a declared rule (coordinator
decision C1). Rows re-emitted by a reset carry 018's `restored: true` marker; journal-derived projections,
including ADR-0056's holder count, skip them. Rules for this feature's event types:

| Event | Rule | What the reset transaction does, or which points are refused |
|---|---|---|
| `step.cache_hit` | `REMATERIALIZE` | inserts a `step_cache_hit_provenance` row for the reset run (same source run, entry and payload), so 020 erasure still finds the copy; no cache read |
| `step.cache_missed` | `REMATERIALIZE` | *Revised 2026-11-25 (decision D022-253; was `COPY`):* the reset copies `steps.cache_lookup` from the source step, so the reset run's next completion can write its store obligation |
| `failure_handler.draining` | `REFUSE` | added 2026-11-25 (decision D022-246): a point inside the handler phase, as the phase's other events |
| `step.mocked` | `REFUSE` | only test-tree runs carry it and they are not reset sources (`RESET_SOURCE_TEST_RUN`) |
| `result.returned` | `REMATERIALIZE` | sets `runs.result_digest`, `result_source = EARLY_RETURN` and `result_available_at` on the reset run from the restored event; no `run.result_returned` delivery or hint is sent again |
| `failure_handler.started` | `REFUSE` | every reset point after it is inside the handler phase or its terminal tail and is refused; reset to a point before the failure instead |
| `failure_handler.finished` | `REFUSE` | as above (it follows `failure_handler.started`) |
| `concurrency.queued` | `REMATERIALIZE` | a point after `concurrency.queued` and before its `concurrency.acquired` is refused (the run never started work); otherwise history |
| `concurrency.acquired`, `concurrency.released` | `REMATERIALIZE` | restored rows are history; after `run.reset` the reset transaction acquires a fresh run permit through the permit port (ADR-0056 decision 3) and never queues, else the reset is refused (`CONCURRENCY_PERMIT_UNAVAILABLE`, 018's code); a point with an open step-scope acquisition is not eligible (no attempt in flight, ADR-0050 decision 5); the permit of the attempt whose completion is the point is history, retired by `run.reset` (D022-053) |
| `concurrency.wait_timed_out`, `concurrency.dequeued` | `REFUSE` | points after them lie in a terminal tail of a run that never started work |
| `rate_limit.queued` | `REMATERIALIZE` | a point after a run-scope `rate_limit.queued` and before its `rate_limit.granted` is refused (the run never started work); otherwise history |
| `rate_limit.granted` | `REMATERIALIZE` | restored rows are history and create no grant row; after `run.reset` the reset transaction takes a fresh run-scope slot through the admission port and never waits, else the reset is refused (`409 CONFLICT`, `RATE_LIMIT_SLOT_UNAVAILABLE`, 018's contract; rate first when the start also takes a permit, D022-069); a point with a step waiting for a slot or granted and not yet completed is not eligible (no attempt in flight, ADR-0050 decision 5) |
| `rate_limit.wait_timed_out`, `rate_limit.dequeued` | `REFUSE` | run scope: points after them lie in a terminal tail of a run that never started work; step scope: reset to a point before the step asked for its slot |
| `rate_limit.returned` | `REMATERIALIZE` | history; it creates no grant row, and a point between it and the step's next grant is not eligible (no attempt in flight) |
| `run.started` provenance (`handler_tree`, `handler_kind`, `start_at`; added 2026-11-25) | `REMATERIALIZE` | a reset of a handler-tree source is a handler-tree run and inherits no `handler_kind`; `lineage_run_id` is the source's, whoever starts the reset (decision D022-242, which reverses D022-241); `start_at` is copied with the test tree, which is refused as a source anyway |
| `run.started` provenance (`handler_for`) | `REMATERIALIZE` | the reset run does not inherit `handler_for` (exactly one handler run per failure, unique index); its link to the source is 018's reset provenance; `origin` and `test_tree` are copied (test trees are refused as sources) |

## Spans

Journal-derived spans (ADR-0017) map `step.cache_hit` and `step.mocked` to a zero-duration attempt span with
`sagat.outcome = cache_hit | mocked`, and the handler phase to a `workflow.failure_handler` span under the
run span.
