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
| `step.mocked` | T16 or when reached | `source` ∈ `MOCK`, `RUN_PIN`; `source_run_id`, `floor_level` (both optional); `payload_ref` set | step `SUCCEEDED` with origin `MOCK`; excluded from compensation/reconciliation planning |
| `result.returned` | T6 | `result_source: "EARLY_RETURN"`; `payload_ref` set | sets the run result once; status unchanged |
| `end.reached` (existing) | existing | adds optional `result_already_returned: true` | no second result |
| `failure_handler.started` | T7 | `pending_status`, `form` ∈ `NODE`, `CHILD`; `failure_kind`, `failure_code` | enters handler phase |
| `failure_handler.finished` | T8 | `outcome` ∈ `SUCCEEDED`, `FAILED`, `FAILURE_HANDLER_TIMEOUT`, `CANCELLED` | followed by `run.finished` with the pending status |
| `concurrency.queued` | T10 | `pool`, `scope: "RUN"`, `group`, `key_label`, `queue_deadline_us` | run `WAITING` (wait kind `CONCURRENCY`) |
| `concurrency.acquired` | T10, T11, T13 | `pool`, `scope`, `group`, `key_label`, `generation` (step scope) | holder open |
| `concurrency.released` | T8, T14 | `pool`, `scope`, `group`, `key_label` | holder closed |
| `concurrency.wait_timed_out` | T12 or step timeout | `pool`, `scope`, `group`, `key_label` | run or step fails `CONCURRENCY_WAIT_TIMEOUT` |
| `concurrency.dequeued` | cancellation, run deadline | `pool`, `group`, `key_label` | waiter removed; no permit |

`run.started` (existing) gains optional provenance `handler_for`, `origin` and `test_tree`. `key_label` is
the 12-hex-character prefix of the keyed digest (D1168), never the key value.

## Reset rules (feature 018 registry)

Feature 018 keeps the reset-rule registry in `specs/018-schedules-bulk-operations/contracts/journal-and-events.md`
and refuses a reset point whose prefix contains an event type without a declared rule (coordinator
decision C1). Rows re-emitted by a reset carry 018's `restored: true` marker; journal-derived projections,
including ADR-0056's holder count, skip them. Rules for this feature's event types:

| Event | Rule | What the reset transaction does, or which points are refused |
|---|---|---|
| `step.cache_hit` | `REMATERIALIZE` | inserts a `step_cache_hit_provenance` row for the reset run (same source run, entry and payload), so 020 erasure still finds the copy; no cache read |
| `step.mocked` | `REFUSE` | only test-tree runs carry it and they are not reset sources (`RESET_SOURCE_TEST_RUN`) |
| `result.returned` | `REMATERIALIZE` | sets `runs.result_digest`, `result_source = EARLY_RETURN` and `result_available_at` on the reset run from the restored event; no `run.result_returned` delivery or hint is sent again |
| `failure_handler.started` | `REFUSE` | every reset point after it is inside the handler phase or its terminal tail and is refused; reset to a point before the failure instead |
| `failure_handler.finished` | `REFUSE` | as above (it follows `failure_handler.started`) |
| `concurrency.queued` | `REMATERIALIZE` | a point after `concurrency.queued` and before its `concurrency.acquired` is refused (the run never started work); otherwise history |
| `concurrency.acquired`, `concurrency.released` | `REMATERIALIZE` | restored rows are history; after `run.reset` the reset transaction acquires a fresh run permit through the permit port (ADR-0056 decision 3) and never queues, else the reset is refused (`CONCURRENCY_PERMIT_UNAVAILABLE`, 018's code); a point with an open step-scope acquisition is not eligible (no attempt in flight, ADR-0050 decision 5) |
| `concurrency.wait_timed_out`, `concurrency.dequeued` | `REFUSE` | points after them lie in a terminal tail of a run that never started work |
| `run.started` provenance (`handler_for`) | `REMATERIALIZE` | the reset run does not inherit `handler_for` (exactly one handler run per failure, unique index); its link to the source is 018's reset provenance; `origin` and `test_tree` are copied (test trees are refused as sources) |

## Spans

Journal-derived spans (ADR-0017) map `step.cache_hit` and `step.mocked` to a zero-duration attempt span with
`sagat.outcome = cache_hit | mocked`, and the handler phase to a `workflow.failure_handler` span under the
run span.
