# Errors — feature 018 (DRAFT)

No new `Error.code` (the closed set in `sdks/error-catalog.json` and the canonical `Error` schema
stays as it is). Feature 018 adds `details.reason` refinements only, each with one remediation row
(`crates/api-http/src/error.rs` and `docs/quickstarts/errors.md` gain them in T018-044). Every error
carries `code`, `retryable`, `traceId` and never a payload, an input value, a business id of another
caller or another tenant's identifier. A foreign resource is `NOT_FOUND` without a reason.

| `code` | HTTP | `details.reason` | Retryable | Raised by | Remediation (one sentence, closed table) |
| --- | --- | --- | --- | --- | --- |
| `VALIDATION_FAILED` | 422 | `SCHEDULE_NAME_RESERVED` | no | create/update schedule | The name is one a schedule of a cron trigger has or may fall back to (`trigger-` and 32 hex digits, optionally `-` and digits); choose another. |
| `VALIDATION_FAILED` | 422 | `SCHEDULE_SPEC_INVALID` | no | create/update schedule (also the action's business id: `details.field` is `/action/businessIdPrefix` for a prefix that is not 1 to 160 visible ASCII characters, and `/action/businessIdConflictPolicy` or `/action/businessIdReusePolicy` for a policy that has no prefix to belong to; the text is never echoed) | A spec field is outside its domain or ceiling; `details.field` names the JSON pointer. |
| `VALIDATION_FAILED` | 422 | `SCHEDULE_NEVER_FIRES` | no | create/update | The spec has no occurrence between `startAt` and `endAt`. |
| `VALIDATION_FAILED` | 422 | `SCHEDULE_TOO_FREQUENT` | no | create/update | Two occurrences are closer than the minimum spacing; widen the spec. |
| `VALIDATION_FAILED` | 422 | `SCHEDULE_ZONE_UNKNOWN` | no | create/update | Use an IANA zone name known to the platform. |
| `VALIDATION_FAILED` | 422 | `SCHEDULE_INPUT_INVALID` | no | create/update, action time | The input does not match the target version's input schema (no excerpt). |
| `VALIDATION_FAILED` | 422 | `NOMINAL_TIME_FIELD_INVALID` | no | create/update | The field is not a declared top-level string property of the input schema. |
| `VALIDATION_FAILED` | 422 | `BACKFILL_RANGE_INVALID` | no | backfill | `start` must be before `end` and `end` not after now. |
| `VALIDATION_FAILED` | 422 | `BACKFILL_LIMIT` | no | backfill | The range holds more nominal times than one backfill may act on; split it. |
| `CONFLICT` | 409 | `REVISION_CONFLICT` (existing reason) | no | update/delete schedule | Read the schedule again and resubmit with its current revision. |
| `CONFLICT` | 409 | `SCHEDULE_TRIGGER_OWNED` | no | update/delete of an origin `TRIGGER` schedule | Change the cron trigger in the workflow definition and publish it. |
| `CONFLICT` | 409 | `SCHEDULE_DELETED` | no | operate a deleted schedule | The schedule no longer exists for new actions. |
| `UNAVAILABLE` | 503 | `BINARY_CAPABILITY_MISSING` (shared with 029 and 031) | yes | any API call but `GET /v1/health`, `GET /v1/ready` included, served by a process whose binary lacks a capability recorded as required for its kind in the instance-capability registry (the process learns of a requirement at its next heartbeat, at most about 10 s; a stream ends with a `reconnect` event) | The service cannot serve this call now: try again. If it keeps happening, tell the operator of the platform. |
| `CONFLICT` | 409 | `SCHEDULE_TRIGGER_GONE` | no | trigger-now (run now) of a `TRIGGER` schedule whose cron trigger is no longer in force (an older control plane dropped it) | The cron trigger of this schedule is no longer in force; the schedule is removed with it. Publish the trigger again to have it back. |
| `CONFLICT` | 409 | `SCHEDULE_TRIGGER_HANDOVER_PENDING` | no | pause/resume/trigger of a `TRIGGER` schedule before the operator handover | The legacy cron pass still runs; the operator completes the handover once every engine is upgraded. |
| `QUOTA_EXCEEDED` | 429 | `SCHEDULE_LIMIT`, `BACKFILL_CONCURRENCY`, `BATCH_LIMIT`, `PENDING_RESET_LIMIT` | no | create | A namespace or tenant ceiling is reached; free capacity or ask an administrator. |
| `VALIDATION_FAILED` | 422 | `BUSINESS_ID_INVALID` | no | start options | 1 to 200 visible ASCII characters. |
| `CONFLICT` | 409 | `BUSINESS_ID_IN_USE` | no | start, rerun/reset with `KEEP`, Update-with-Start with `FAIL` | Another open execution holds this id; `details.runId` names it when the caller may read runs. |
| `CONFLICT` | 409 | `BUSINESS_ID_REUSE_REJECTED` | no | start | The reuse policy forbids a new run while the previous one is remembered; `details.reusableAfter` says until when. |
| `VALIDATION_FAILED` | 422 | `START_DELAY_INVALID` | no | start | 1 ms to 366 days. |
| `VALIDATION_FAILED` | 422 | `START_DELAY_WITH_SIGNAL` | no | Signal-with-Start | A delayed run cannot accept a signal at creation; start, then signal later. |
| `QUOTA_EXCEEDED` | 429 | `CONCURRENCY_QUEUE_FULL` (feature 022's reason, reused) | no | synchronous starts, with-start, rerun of a version whose key queue is full | Retry when the key's queue has room. |
| `CONFLICT` | 409 | `NOT_WAITING` | no | Signal-with-Start on an open holder; existing `signalRun` keeps `STATE_CONFLICT` | No step of the run waits for this signal now; signals are never buffered. |
| `CONFLICT` | 409 | `SIGNAL_NOT_ACCEPTED_AT_START` | no | Signal-with-Start creating a run | The workflow's entry does not wait for this signal, or feature 022 would queue the new run; nothing was created. |
| `VALIDATION_FAILED` | 422 | `INVALID_OPERATION_STATE`, `INVALID_ARGUMENTS`, `INVALID_UPDATE_KEY` (existing reasons) | no | Update-with-Start | As for `updateRun`; nothing was created. |
| `CONFLICT` | 409 | `IDEMPOTENCY_KEY_REUSED` (existing reason) | no | every keyed operation | Use a new key for a different request. |
| `VALIDATION_FAILED` | 422 | `BATCH_SELECTOR_INVALID` | no | batch preview/create | The filter or run-id list is malformed or over its ceiling. |
| `VALIDATION_FAILED` | 422 | `QUERY_UNSUPPORTED_OPERATOR` (feature 020's reason, reused) | no | batch with `query` while 020's capability is off | Use the structured filter. |
| `VALIDATION_FAILED` | 422 | `BATCH_ACTION_UNAVAILABLE` | no | batch action whose owning feature is not enabled (`REPIN` without 019) | Enable the feature or choose another action. |
| `VALIDATION_FAILED` | 422 | `BATCH_LIMITS_INVALID` | no | batch create | `maxItems` is 1 to 100,000 and `itemsPerSecond` above 0 and at most 200. |
| `VALIDATION_FAILED` | 422 | `BATCH_REASON_INVALID` | no | batch create/cancel | The reason is 8 to 2,000 characters of plain text once trimmed: no control character, a new line included. |
| `VALIDATION_FAILED` | 422 | `BATCH_PAYLOAD_INVALID`, `BATCH_PAYLOAD_TOO_LARGE` | no | batch create with `signal` or `update` | The payload must be a JSON object (a signal) and at most 256 KiB. |
| `VALIDATION_FAILED` | 422 | `BATCH_SELECTOR_UNSUPPORTED` | no | reserved: nothing in this build raises it (`businessIdPrefix` selects runs since D2177; the `020` query is `QUERY_UNSUPPORTED_OPERATOR`); kept because the set only grows | This deployment cannot evaluate that selector; select by the other filters or by run ids. |
| `VALIDATION_FAILED` | 422 | `BATCH_ACTION_PARAMETERS` | no | batch create | The action needs (or refuses) `stepId`, `signal` or `update`; a signal name is 1 to 128 characters of letters, digits and `_ . : -`; a step id is a step of the workflow. |
| `CONFLICT` | 409 | `BATCH_NOT_CANCELLABLE` | no | batch cancel | The job already ended. |
| `VALIDATION_FAILED` | 422 | `RESET_NOT_SUPPORTED_FOR_SOURCE` | no | reset points/preview/reset | Child runs, members of a continuation chain and runs of a database execution cannot be reset in this version. |
| `VALIDATION_FAILED` | 422 | `RESET_POINT_NOT_ELIGIBLE` | no | preview/reset | Choose a sequence listed by `listResetPoints`. |
| `VALIDATION_FAILED` | 422 | `RESET_PREFIX_TOO_LONG` | no | preview/reset | The prefix exceeds the reset ceiling; choose an earlier point. |
| `VALIDATION_FAILED` | 422 | `RESET_REEXECUTION_NOT_ACKNOWLEDGED` | no | reset | Steps that completed in the source, or that are in flight in a run being cancelled, will run again; set `acknowledgeReexecution`. |
| `CONFLICT` | 409 | `RESET_SOURCE_NOT_TERMINAL` | no | reset with `REQUIRE_TERMINAL` | Wait for the run to end, or ask for `sourcePolicy: CANCEL_SOURCE` to cancel it and have the run created when it has ended. |
| `VALIDATION_FAILED` | 422 | `RESET_CANCEL_SOURCE_NOT_AVAILABLE` | no | reset with `CANCEL_SOURCE` of a run that has not ended, on a deployment that does not serve it | Wait for the run to end and reset it then, or ask the operator of the platform to enable resets that cancel their source. |
| `VALIDATION_FAILED` | 422 | `RESET_RULE_MISSING` | no | reset points/preview/reset whose prefix holds an event type without a rule in the reset-rule registry (`details.eventType`) | Reset to an earlier point; the owning feature must declare the rule. |
| `VALIDATION_FAILED` | 422 | `RESET_REAPPLY_NOT_AVAILABLE` | no | preview/reset with `reapply` other than `NONE` on a deployment that does not serve it | Ask for `reapply: NONE`, or ask the operator of the platform to enable reset reapply. |
| `VALIDATION_FAILED` | 422 | `RESET_REAPPLY_TOO_MANY` | no | preview/reset with `reapply` | The run received more than 256 signals and retained Updates after that point; choose a later point or `reapply: NONE`. |
| `CONFLICT` | 409 | `CONCURRENCY_PERMIT_UNAVAILABLE` | no | reset of a version with run-level `concurrency` whose key has no free permit (a reset cannot queue) | Retry the reset when a holder of the key has finished. |
| `VALIDATION_FAILED` | 422 | `RESET_SOURCE_TEST_RUN` | no | reset points/preview/reset of a 022 draft test run | Test runs are re-run through feature 022's test API. |
| `CONFLICT` | 409 | `RUN_PURGED` (feature 020's code and reason, reused) | no | reset or rerun of a purged source, or a purge committing during the reset | The run's history is archived or erased; it cannot be restarted from the hot store. |

## Per-item and per-action reason codes (not HTTP errors)

Batch item `reason` (outcome `SKIPPED`): `NOT_APPLICABLE`, `NOT_WAITING`, `OPERATION_NOT_DECLARED`,
`RUN_CONTINUED`, `ALREADY_TERMINAL`, `HIDDEN_FROM_CREATOR`, `TEST_RUN`. Item `DEFERRED` causes:
`QUOTA_EXCEEDED`, `RATE_LIMITED`, `CONCURRENCY_QUEUE_FULL`, `LOCK_WAIT`. Batch item `errorCode` (outcome `FAILED`):
the canonical `Error.code` the single action returned. Batch job `failureReason`:
`AUTHORITY_REVOKED`, `SELECTION_FAILED` (the platform could not select the runs: the eleventh selection round in a row
failed, G2093; the read says the reason and nothing of the rounds, the count or the cause).

Schedule action `reason`: `TRACKING_LIMIT`, `VERSION_MISSING`, `ADMISSION_REFUSED`, `AUTHORITY_REVOKED`,
`BUSINESS_ID_IN_USE` (the outcome `BUSINESS_ID_CONFLICT` under `FAIL`, and `ALREADY_ACTED` with the holder as `runId` under
`USE_EXISTING`) and `BUSINESS_ID_REUSE_REJECTED` (the outcome `BUSINESS_ID_CONFLICT` when a closed holder inside its reuse window and the
reuse policy refuse the id, added by D2173). Never the business id, the prefix or the holder's identifier of another tenant.

Reset `failureReason`: `BOUNDARY_NO_LONGER_ELIGIBLE`, `ADMISSION_REFUSED`, `BUSINESS_ID_IN_USE`,
`CONCURRENCY_PERMIT_UNAVAILABLE`, `RUN_PURGED`, and the reason of a `REMATERIALIZE` rule that could not
be applied (`RESOURCE_UNAVAILABLE`, `VARIABLE_NOT_FOUND`, `SNAPSHOT_RETIRED`, as their owners define).
`CANCEL_SOURCE` (D1996..D2005) adds three, written by the pass that completes a pending reset:
`AUTHORITY_REVOKED` (the member who asked no longer holds `run:operate`, `run:start` or what the reapply needs, or is
no longer a member), `REEXECUTION_NOT_ACKNOWLEDGED` (steps completed after the point and the request did not
acknowledge the re-execution) and `REAPPLY_TOO_MANY` (the source received more to deliver again than a reset may).
The expiry of a reset that waits (D2059, OD-2026-11-08-1) adds one, written by the same pass and not by its judgement of the final
journal: `SOURCE_DID_NOT_END` (the source run had not ended when the time a reset waits for it was over: 24 hours after the request
unless the operator of the platform set another time, from 1 hour to 7 days; no run was made and the place in the namespace is free;
the run stays cancelled; end the run, then reset it again).
Reapply `reason`: `NO_WAITING_STEP`, `AFTER_UNDELIVERED_SIGNAL`, `UPDATE_REJECTED`, `NOT_RETAINED`,
`LEGACY_EVENT`, `REAPPLY_LIMIT`.

These lists are closed and grow only; SDKs treat an unknown value as "another reason".
