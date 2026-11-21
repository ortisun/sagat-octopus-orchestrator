# Journal, run fields, AsyncAPI and protocol — feature 018 (DRAFT; the run fields and the AsyncAPI additions are as built, see their sections)

Mostly built; the sections "Run resource additions" and "AsyncAPI" say what is as built. T018-006 adds the domain events, T018-010 merges the AsyncAPI fragment and
regenerates `specs/001-platform-foundation/contracts/events.baseline.json` on purpose
(`docs/event-schema-evolution.md`). Journal `detail` shapes are closed and bounded; they carry
identifiers, times and closed codes only, never payloads or business identifiers, so `run_events`
stays a protected system record (ADR-0027).

## New journal event types

| `event_type` | When | `detail` (closed) | Reducer effect |
| --- | --- | --- | --- |
| `run.start_delayed` | In the creation transaction of a run with `startDelayMs`, right after `run.started` | `{"until_us": integer}` | Run is `WAITING`; no step is released. |
| `run.start_released` | Delayed-start pass, once, at or after `until_us` | `{"until_us": integer}` | Entry is released as `start_run` would have; duplicate is refused by the reducer. |
| `run.reset` | In the reset transaction, at sequence N+1 after the re-emitted prefix `1..N` | `{"source_run_id": uuid, "source_sequence": N, "prefix_sha256": hex64, "reapply": "NONE"\|"SIGNALS"\|"UPDATES"\|"ALL"}` | Marks the restored prefix and clears any restored run-permit state (022); a fresh `concurrency.acquired` follows when the version declares top-level `concurrency`; the settle reduction then releases the successors of the reset point. |

`run.reset` is never the first event: a reset run's sequence 1 is the re-emitted `run.started` of
the source. `runs.reset_sequence = N` lets every reader tell restored sequences from new ones.

## Additive `detail` fields on existing events

| Event | Field | Values | Written by |
| --- | --- | --- | --- |
| `wait.satisfied` | `via` (optional) | `signal`, `event`, `signal_with_start` | API signal and chain signal (`signal`), event dispatch (`event`), Signal-with-Start whether it created the run or found it (`signal_with_start`), reset reapply (`signal`). Older rows have no `via` and decode unchanged; a name this version does not know decodes as none. **Written since T018-043 (D1975).** |
| `run.started` | `schedule` (optional) | `{"schedule_id", "nominal_at_us", "kind", "revision"}` | **Not written, and not planned (D2147, 2026-11-10).** The schedule action that started a run is on the run record (`runs.schedule_id`, `schedule_nominal_at`, `schedule_action_kind`, `schedule_revision`, written in the creation transaction and immutable) and on the action row; the journal detail of `run.started` does not carry it (`RunStarted { entrypoint }`), so `verify_run` does not replay it. Why: a reset copies the journal verbatim and the provenance of the new run is not the source's, and a successor of a chain copies no schedule column (D2144), so a journal copy would claim what the record does not. What it would cost, if replay equality of the columns is wanted: a `RunEvent` detail, its codec, a rule for `RESTORE`/`REMATERIALIZE`, a replay test (G2068). |
| `state.updated` | `retained` (optional) | `true`, `false` | Every Update applied since T018-043: `true` when its declaration opts in with `reapplyOnReset` and its arguments were kept, `false` when it does not. Older rows have none: a reset reports them `NOT_REAPPLIED` / `LEGACY_EVENT`, and an Update that did not opt in `NOT_REAPPLIED` / `NOT_RETAINED`. |
| every re-emitted row | `restored` | `true` | Reset re-emission; the source rows are not changed. |
| re-emitted `run.started` | `reset_of` | `{"source_run_id", "source_sequence"}` | Reset re-emission, so a reader knows from sequence 1. |
| `run.started` | `delayed` (optional) | `true` | Starts with `startDelayMs`. |

Detail keys are snake_case like the existing ones (`due_at_us`, `before_revision`). Decoders read fields
by name (`crates/infra-postgres/src/event_codec.rs`), so older readers ignore the new optional ones.
The retained Update arguments are the canonical arguments of the Update as a `payloads` row, charged to
`tenant_content_bytes` like any payload and referenced by the **receipt** of the Update
(`operation_update_receipts.arguments_digest`, migration `20270105000000`), not by the `payload_digest` of
the journal row: that column of a `state.updated` row already references the new state (D1975). The receipt
goes with the run, is written in the transaction of the row, and is found by `(resolved_run_id, run_version)`,
the sequence of the row. They are not a classification carrier (FR-016-004), so a journal disclosure of that
sequence answers `NO_SCHEMA` as today.

## Re-emitted prefix of a reset run

Events `1..N` of the source are decoded with the source's codec and re-encoded for the new run:
event type, node, attempt, `detail`, `payload_digest`, `actor`, `trace_id` and `occurred_at` are
kept; run-identity fields inside `detail` (for example a run id in `run.started`) are rewritten to the
new run. `prefix_sha256` is computed over the canonical encoding of the source rows `1..N`
(sequence, type, node, attempt, detail, payload digest), so a reader can verify the copy against a
retained source (the markers above are added after digesting). The telemetry projector (ADR-0017)
starts the reset run's cursor at N and emits a span link to the source run's span instead of
re-exporting attempts that happened in the source; feature 020's analytics projector MUST likewise skip
(or flag) rows with `restored: true`.

Feature interplay of the re-emitted prefix:

- **022 run permits.** Restored `concurrency.acquired`/`concurrency.queued`/`concurrency.released` rows
  are history; ADR-0056's holder count considers only sequences after `runs.reset_sequence`. The reset
  transaction acquires a permit through 022's admission path after `run.reset` and cannot queue
  (`CONCURRENCY_PERMIT_UNAVAILABLE`). Step permits are never open at an eligible point.
- **020 search attributes.** The reset transaction inserts the reset run's `run_search_attributes` rows
  by folding the re-emitted `search_attributes.upserted` events (same sequences, so `set_seq` matches),
  keeping 020's invariant that the rows equal the fold of the run's journal.
- **020 purge.** The re-emitted rows reference the source's payload rows by foreign key; a purge that
  deleted them makes the reset fail with `CONFLICT`/`RUN_PURGED` (020's code).

## Reset-rule registry (normative; coordinator decision C1)

Feature 018 owns this registry. Every journal event type has exactly one rule; a draft that adds an
event type declares its rule here in the same delivery and confirms it in its own contract. A reset
point whose prefix contains an event type with **no** row is not listed by `listResetPoints` and an
explicit request for it is refused `VALIDATION_FAILED`/`RESET_RULE_MISSING` (`details.eventType`).
The implementation keeps the rules in an exhaustive table in `crates/domain/src/execution/reset.rs`
keyed by `RunEvent::name()`; a test fails when an event type has no rule (D1605).

Rules:

- **`COPY`** — re-emitted under the new run with `restored: true`; nothing else.
- **`REMATERIALIZE`** — re-emitted, and the reset transaction re-creates the side-table rows the event
  implies for the new run, in the slots of ADR-0050's admission order, or refuses the reset with the
  owner's reason when it cannot (for example a retired version or a busy permit).
- **`EXCLUDE`** — must not be in a prefix: every point at or after it is ineligible (a terminal tail or a
  state the reset run cannot inherit).
- **`REFUSE`** — opens a window: points after it are ineligible until the closing event named in the
  row (none named: until the end, the same as `EXCLUDE` for eligibility).

State-based eligibility (FR-018-035: no attempt in flight, no waiting step, no active scope, not
paused, no compensation, no unknown outcome or manual review, no later compensation of a restored
instance) applies in addition to these rules.

### Existing event types (002, 003, 014; owned by 018 for this registry)

| Event type | Rule | Notes |
| --- | --- | --- |
| `run.started` | `COPY` | identity fields rewritten, `reset_of` added; provenance fields of drafts: see their rows |
| `step.routed`, `step.released`, `step.leased`, `step.started`, `step.completed`, `step.retry_scheduled`, `step.retry_due`, `step.failed`, `step.cancel_requested`, `step.cancelled` | `COPY` | an attempt open at the point makes it ineligible (state rule) |
| `condition.evaluated`, `switch.evaluated`, `parallel.forked`, `parallel.joined` | `COPY` | recorded choices are history; replay never re-evaluates |
| `timer.started`, `timer.fired`, `wait.started`, `wait.satisfied`, `wait.timed_out` | `COPY` | an open timer or wait at the point makes it ineligible |
| `approval.opened`, `approval.decided`, `approval.expired` | `COPY` | a decided approval is restored (preview lists it); an open one makes the point ineligible; 026 side rows below |
| `collection.*`, `loop.*` (entered, iteration admitted/returned/checkpointed, drain started, iteration drained, finished; loop delay elapsed) | `COPY` | an active scope at the point makes it ineligible; points are only at `collection.finished`/`loop.finished`. *As built (D2156, G1980 (1)):* the entry of a collection, and every row below the root, make every later point ineligible, so no `COLLECTION_FINISHED` or `LOOP_FINISHED` point is offered until the side tables of a scope are restored |
| `child.started`, `child.finished` | `COPY` | a child in flight makes the point ineligible; the reset run starts no child again for a restored one |
| `saga.entered`, `saga.completed` | `COPY` | |
| `saga.aborting`, `compensation.planned`, `saga.compensated`, `scope.compensation_planned`, `scope.compensated` | `EXCLUDE` | the source was compensating; reset to a point before the failure |
| `reconciliation.requested` | `REFUSE` until `reconciliation.concluded` of the same step | an unknown outcome is open |
| `reconciliation.concluded`, `step.needs_review`, `step.retry_requested`, `step.skipped`, `step.resolved` | `COPY` | an open review at the point makes it ineligible (state rule) |
| `state.updated` | `COPY` | declared state restored; the row keeps its `retained` marker, and the retained arguments (`reapplyOnReset`) stay where the receipt of the source run references them (D1975; whether a reset of the reset run finds them is G1978) |
| `run.paused`, `run.resumed` | `COPY` | a point at which the run is paused is ineligible (state rule) |
| `run.stop_requested`, `end.reached`, `run.finished` | `EXCLUDE` | terminal tail |
| `execution_chain.started`, `execution_chain.successor_linked`, `run.continued`, `continuation.selected` | `EXCLUDE` | sources outside continuation chains only (G1001) |

### Event types added by 018

| Event type | Rule | Notes |
| --- | --- | --- |
| `run.start_delayed` | `REFUSE` until `run.start_released` | a reset run never inherits a delay |
| `run.start_released` | `COPY` | |
| `run.reset` | `COPY` | resetting a reset run is allowed only at points after its own `run.reset` |
| `wait.satisfied` `via`, `run.started` `schedule`/`delayed`/`reset_of` detail | `COPY` | additive detail of existing types |

### Event types and side tables added by other drafts (declared by their owners)

| Owner | Event type or side table | Rule | Declared in (confirmed 2026-10-27) |
| --- | --- | --- | --- |
| 019 | `run.started` + `dispatch` | `COPY` (fairness key re-read from the source run row) | declared by 019 — `specs/019-worker-deployments-dispatch/contracts/errors-and-events.md` |
| 019 | `run.deployment_pinned`, `run.deployment_repinned` | `REMATERIALIZE` (re-create `run_deployment_pins` at the folded version under 019's fences; a retired version pins at the first claim with `source: reset`) | declared by 019 — same file |
| 019 | `step.leased` + dispatch detail, `step.detached` | `COPY` (an open detached attempt is an attempt in flight) | declared by 019 — same file |
| 020 | `search_attributes.upserted` | `REMATERIALIZE` (fold into `run_search_attributes`, same `set_seq`) | declared by 020 — FR-020-006, D1618 |
| 020 | `search_attributes.rejected` | `COPY` | declared by 020 — D1618 |
| 021 | (no event type) side tables `run_resource_bindings`, `run_variable_bindings` | `REMATERIALIZE` (same revisions after rechecking status and permissions; refused `RESOURCE_UNAVAILABLE` / `VARIABLE_NOT_FOUND`) | declared by 021 — `specs/021-integration-resources-triggers/spec.md` "Reset of runs that use resources" |
| 021 | side table `step_resource_credentials` | not copied (attempt evidence of the source) | declared by 021 — same section |
| 022 | `step.cache_hit`, `result.returned`, `concurrency.acquired`, `concurrency.released`, `run.started` provenance (`handler_for` dropped) | `REMATERIALIZE` | declared by 022 — `specs/022-flow-control-step-policies/contracts/journal-events.md` |
| 022 | `concurrency.queued` | `REMATERIALIZE`; a point between it and its `concurrency.acquired` is refused | declared by 022 — same file |
| 022 | `step.mocked`, `failure_handler.started`, `failure_handler.finished`, `concurrency.wait_timed_out`, `concurrency.dequeued` | `REFUSE` (until the end) | declared by 022 — same file |
| 024 | `data.inputs_pinned`, `data.snapshot_committed` | `REMATERIALIZE` (re-create input pins / snapshot refs; refused `SNAPSHOT_RETIRED`) | declared by 024 — `specs/024-data-engines-pipelines/data-model.md` |
| 024 | `data.published`, `data.publish_skipped`, `data.observation_recorded` | `COPY` (never re-published) | declared by 024 — same file |
| 025 | `agent.started`, `agent.turn_scheduled`, `agent.turn_completed`, `agent.tool_planned`, `agent.tool_refused`, `agent.tool_completed`, `agent.repair_scheduled`, `agent.budget_exhausted` | `REFUSE` until `agent.finished` of the same step; after it `REMATERIALIZE` (agent rows copied, no call repeated, no budget charged) | declared by 025 — FR-025-046, D1635 |
| 025 | `agent.finished` | `REMATERIALIZE` | declared by 025 — D1635 |
| 026 | (no event type) `run_stream_events` and stream heads | not copied: the reset run starts with an empty stream log | declared by 026 — `specs/026-invocation-human-surfaces/data-model.md` |
| 026 | `approval.decided` decision input; `approval.opened` side rows (summary, deliveries, decision links) | `COPY`; side rows `REMATERIALIZE` (belong to the source request; a reopened approval gets new ones) | declared by 026 — same file |
| 030 | `service_call.scheduled`, `service_call.started`, `service_call.cancel_requested`, `service_call.finished` | `COPY` when the call's `service_call.finished` is in the prefix (no call record, message or handler run is created); `REFUSE` while the call is open | declared by 030 — FR-030-038, `specs/030-cross-namespace-services/data-model.md` (Reset rules, D1516) |
| 030 | `service_operation.cancel_requested`, `service_operation.completed` | `EXCLUDE` | declared by 030 — FR-030-038 |
| 030 | handler `run.started` `cause` | `COPY` as provenance; the reset run backs no operation | declared by 030 — FR-030-038 |

Drafts 023, 027, 028, 029, 031 and 032 add no journal event type in their drafts of 2026-10-27; any they
add later has no rule until declared here, so resets over it are refused rather than guessed.

## Run resource additions (REST `Run`)

`businessId`, `startDelayedUntil`, `schedule {scheduleId, nominalTime, kind, revision}`,
`resetFrom {runId, sequence}` — all optional, absent for existing runs. `rerunOf` is also set on a
reset run (it names the source, as for `RERUN_AS_NEW`). `startedBy` of a scheduled run is
`system:schedule:<scheduleId>`; trigger-owned schedules keep `system:engine`.

**As built (2026-11-10, G2057, D2141 to D2146).** All four are in the `Run` schema of `openapi.yaml`, read by `getRun` and
`listRuns` (the list returns `Run` items, so the list carries them too) from the columns the run was created with, in the
statement that loads the run: no query per run. Each is **absent, not `null`**, for a run that has none.
`schedule.kind` is the kind of the **action** (`SCHEDULED`, `TRIGGERED` for run now, `BACKFILL`, as `runs.schedule_action_kind` and
`ScheduleActionRecord.kind` say), not the schedule's origin (`API`/`TRIGGER`), which stays on the schedule; `nominalTime` is UTC
to the microsecond and equal to the `nominalTime` of the action; `revision` is the revision the action was taken under.
`resetFrom.runId` equals `rerunOf` and `resetFrom.sequence` is `runs.reset_sequence`; a run started again by `RERUN_AS_NEW` has
`rerunOf` and no `resetFrom`. The run has no `actualTime` (the action has it). What carries no `schedule`: a run the legacy cron pass
started for a trigger (it writes no provenance) and the later segments of a continuation chain (G2063).
**The provenance lives on the run record, not in the journal (D2147):** the `schedule` of the `run.started` detail declared above is
not written, a reset copies the journal verbatim, and `verify_run` (replay equals projection) does not reproduce the four
columns; what replay equality would take is G2068. **`schedule.kind` is a closed enum in the response schema** (the three values
the column admits): the description tells a client to read another value as "another kind" and the six SDKs keep it as sent, but
adding a fourth kind later is a compatibility decision of its own (a migration of the check first, the schema, the SDK and Studio
words), and a server that read a kind it does not know in `get` or `list` (a mixed-version fleet after that migration) would fail
the read as stored data it cannot decode rather than skip it (G2067, accepted).

## AsyncAPI (`events.asyncapi.yaml`)

`RunFinishedWebhook.payload.data` and `RunFinishedBroker` data gain two optional properties:

```yaml
scheduleId: {type: string, format: uuid, description: 'Feature 018: the schedule that started the run, when one did.'}
nominalTime: {type: string, format: date-time, description: 'Feature 018: the nominal time of the schedule action.'}
```

Both are generated values. `businessId` is **not** added: it is caller text and would turn
`event_outbox.payload` into content (ADR-0027). No new message, channel or header.

**As built (2026-11-10, T018-010, D2143).** `RunFinishedWebhook.payload.data` has the two optional properties, and
`RunFinishedBroker` takes that payload by reference, so a broker destination is given the same body. The four code paths that finish a
run and tell its subscribers (`RunTx`, the collection finish, the loop terminal path, the chain's final head) add them through one
function, `add_schedule_origin`, and only for a run a schedule started (`runs.schedule_id`); the body of every other run is what it was.
**What is tested, and what is not.** The run transaction's writer: a real firing, its webhook, and the same run stamped with a
business identifier (the canary of ADR-0027 is in no body and no hint, on the branch that adds properties). The collection, the
loop and the chain-finalization writers: on runs **stamped by hand** with the provenance columns (the guard trigger switched off for
one statement), with and without a schedule, which proves each writer carries the function's data and not that the firing reaches
it. And a rule of `scripts/check-architecture.py` that fails when a `"type": "run.finished"` body is built in the adapter without
`add_schedule_origin` before it in the same function (D2148). **What the product does on a real chain is different, and it is the
open item against FR-046:** the final head of a chain a schedule started is a successor that copies no schedule column, so its
`run.finished` carries no `scheduleId` (a real chain from a real firing, `chain_handoff.rs`; D2144, G2063). The hint on NATS or Redis (`RunFinished`, and `event_outbox.payload`) is
unchanged. `events.baseline.json` was regenerated on purpose: the only change is the two properties on `RunFinishedWebhook.payload`
(`data.scheduleId`, `data.nominalTime`; the shape tool does not follow the `$ref` of the broker message, so its entry is unchanged);
`info.version` stays `0.3.0`, the release in development. `worker.baseline.json` and `workflow.schema.json` are untouched by
this change (`git diff` empty; `workflow.schema.json` differs from `develop` only by `reapplyOnReset`, T018-078).

## The event document of trigger-owned schedules

Unchanged from feature 004: `{event: {source: "cron", type: "cron.fired", id: "cron:{trigger}:{epoch}",
occurredAt, receivedAt}, data: {scheduledAt, firedAt}}`. API schedules give the run their configured
input object (plus `nominalTimeField` when set), not an event document.

## Worker protocol and workflow IR

- **Worker protocol (`worker.proto`): no change.** Runs created by schedules, with-start, batch reruns
  and resets are ordinary runs; their tasks carry the existing fields. `worker.baseline.json` must stay
  byte-identical (checked by T018-010).
- **Workflow IR (`workflow.schema.json`): one optional additive field.** `$defs/update` gains
  `reapplyOnReset` (boolean, default false; `workflow-ir.fragment.schema.json`), capability-gated under
  ADR-0032. Schedules are not part of a definition; trigger-owned schedules reuse the existing
  `triggers[].cron`/`timezone`. Policy fields for definition triggers are deferred (G1000).
