# SDK surface and shared conformance scenarios — feature 018 (DRAFT)

The five official SDKs (TypeScript, Go, Python, C#, Java) expose the same operations with the same
semantics (ADR-0007, ADR-0015, `sdks/SPEC.md` §3). No worker-side change exists: every scenario is a
client scenario run against the real engine by the existing `sdk_conformance` target
(`tests/contract/sdk-conformance`). Scenario files go to
`tests/contract/sdk-conformance/scenarios/`; normalized expectations to `.../expected/`.

## Client operations (names in TypeScript style; each SDK uses its own casing)

| Operation | Request | Success |
| --- | --- | --- |
| `schedules.create(namespaceId, write, options)` | `POST /namespaces/{ns}/schedules`, `Idempotency-Key` | `201` Schedule |
| `schedules.get(namespaceId, id, {nextCount, recentCount})` | `GET …/schedules/{id}` | `200` ScheduleDescription |
| `schedules.list(namespaceId, filter)` / `listAll` | `GET …/schedules` | page; `listAll` follows opaque cursors |
| `schedules.update(namespaceId, id, write, expectedRevision)` | `PUT …/schedules/{id}` | `200` Schedule |
| `schedules.delete(namespaceId, id, expectedRevision)` | `DELETE …/schedules/{id}` | `204` |
| `schedules.pause/resume(namespaceId, id, note)` | `POST …/pause` / `…/resume` | `200` Schedule |
| `schedules.trigger(namespaceId, id, {overlap})` | `POST …/trigger` | `202` ScheduleActionRecord |
| `schedules.backfill(namespaceId, id, start, end, {overlap})`, `cancelBackfill` | `POST …/backfills`, `…/{b}/cancel` | `202` Backfill |
| `schedules.listActions(namespaceId, id, filter)` | `GET …/actions` | page |
| `start(workflowId, input, {businessId, businessIdConflictPolicy, businessIdReusePolicy, startDelay})` | `POST /runs` | `202` new run, or `200` existing run (`started: false` in the typed result) |
| `signalWithStart(workflowId, businessId, input, signalName, payload, options)` | `POST /runs/signal-with-start` | `202` `{runId, started, deliveredTo}` |
| `updateWithStart(workflowId, businessId, input, updateName, updateKey, args, options)` | `POST /runs/update-with-start` | `200` `{runId, started, update}` |
| `businessIds.get(namespaceId, businessId)` | `GET /namespaces/{ns}/business-ids/{id}` (URL-encoded) | `200` BusinessIdBinding |
| `batch.preview(namespaceId, selector, maxItems)` | `POST …/batch-operations/preview` | `200` BatchPreview |
| `batch.create(namespaceId, request)`, `get`, `list`, `cancel`, `listItems` | `…/batch-operations…` | as in `rest.openapi.yaml` |
| `resetPoints(runId)`, `previewReset(runId, sequence, reapply)`, `reset(runId, request)`, `getReset` | `/runs/{runId}/reset-points`, `/reset-previews`, `/resets` | as in `rest.openapi.yaml` |

Rules shared with `sdks/SPEC.md` §3:

- Keys: the SDK accepts or generates one `Idempotency-Key` per call and keeps it with the exact body
  across retries; `updateKey` is the caller's and required.
- `startDelay` given as a duration is converted to integer milliseconds once, before the first
  attempt; the SDK never sends a local-offset instant.
- A business id is sent as given (no trimming, no case folding); `businessIds.get` URL-encodes it.
- Unknown enum values in answers (`ScheduleActionOutcome`, item reasons, reapply outcomes) are kept as
  raw strings, never an error; typed values keep `raw` JSON.
- `200` versus `202` of `start` is surfaced as `started: false | true`; old SDK versions never send a
  conflict policy and therefore never see `200`.

## Scenarios

Nine scenario files, built from the table above and blessed from the TypeScript SDK after every observation was read
against the REST contract; the other five languages hold their output equal to the same goldens. (The draft listed
`errors-018.json`; every `details.reason` reachable from a client is instead asserted in the scenario of the operation
that raises it, so the reason and the call stay together.)

| File | Covers | Assertions (normalized; identical across the SDKs) |
| --- | --- | --- |
| `schedules-crud.json` | create with a calendar, a cron string and an interval (bounded to January 2099, so the next times do not depend on the day it runs), describe, update with a stale and a current revision, list with filters and pages, duplicate name, invalid spec, unknown zone, never fires, too frequent, delete with a stale and a current revision, the name freed, another tenant, a viewer | revisions, `REVISION_CONFLICT`, the next nominal times as epoch milliseconds, `SCHEDULE_*` reasons, 403 and 404 |
| `schedules-operate.json` | pause and resume with a note, trigger while paused, replay of a trigger with the same key, the same key with another body, a backfill of two whole UTC hours of an hourly cron, a second backfill of the same range, a future range, then the run the trigger started and the run a backfill started read back (G2057) | action records (kind, origin, outcome, `backfillId`, run started or not), `remainingActions` unchanged by a trigger, backfill counts (`STARTED` 2, then `ALREADY_ACTED` 2), `BACKFILL_RANGE_INVALID`, and on each of the two runs `schedule {scheduleId, nominalTime (an instant), kind TRIGGERED or BACKFILL, revision 1}` |
| `schedules-overlap.json` | `SKIP`, `BUFFER_ONE` (buffered, then buffer full), `ALLOW_ALL`, an unknown policy, with a run the harness leaves open; every action is a trigger, so no outcome depends on the clock | outcome per action, counters, `bufferedCount` |
| `business-ids.json` | `FAIL`, `USE_EXISTING`, each reuse policy, the lookup of an open and of a closed holder, an invalid identifier, a policy without an identifier, an unknown policy, a replay | `409 BUSINESS_ID_IN_USE` / `BUSINESS_ID_REUSE_REJECTED`, `started`, binding fields, `422 BUSINESS_ID_INVALID` |
| `start-delay.json` | a held start, a signal while held (`409 STATE_CONFLICT`: nothing waits for it), cancel during the delay, a short delay released by the scheduler, zero and too long, the longest, a fraction, the same key with another delay | status `WAITING`, `startDelayedUntil` present, journal events, `START_DELAY_INVALID`, `IDEMPOTENCY_KEY_REUSED` |
| `signal-with-start.json` | no holder (the entry waits), an open waiting run, an open run that does not wait for that name (`NOT_WAITING`), an entry that is not a wait (`SIGNAL_NOT_ACCEPTED_AT_START`, no run created), a replay, a different body under the same key, an invalid identifier, a closed holder with `REJECT_DUPLICATE` | `started`, `deliveredTo`, reasons |
| `update-with-start.json` | applied at creation, replay, same key with another request, applied to an open holder, rejected by the validator, stale revision, `FAIL` on an open holder, undeclared operation, arguments outside the declaration, rejected at creation, with a start delay | receipt revisions, `outcome` and `reason` of a rejection, `started`, replay equality |
| `batch-operations.json` | preview (count, cap, filter), create `CANCEL` over three open runs, the same key and body, the same key with another body, await `COMPLETED`, items (all and filtered, paged), a second job over the ended runs, cancel of an ended job, `REPIN`, empty selector, short reason, business id prefix, query selector | counts and item states, `ALREADY_TERMINAL`, `BATCH_NOT_CANCELLABLE`, `BATCH_ACTION_UNAVAILABLE`, `BATCH_SELECTOR_INVALID`, `BATCH_REASON_INVALID`, `BATCH_SELECTOR_UNSUPPORTED`, `QUERY_UNSUPPORTED_OPERATOR` |
| `reset.json` | list points (and a page of one), preview at the start and after the first step, a sequence that is not a point, a `reapply` policy the server does not know and a `sourcePolicy` it does not know, no acknowledgement, a short reason, the reset, its replay, the same key with another sequence, the new run finishing, the register, an open source, another tenant, a viewer | points, plan (`restored`, `reexecuted`, `requiresAcknowledgement`, `reapply`), `RESET_REEXECUTION_NOT_ACKNOWLEDGED`, `RESET_POINT_NOT_ELIGIBLE`, `RESET_SOURCE_NOT_TERMINAL`, `rerunOf` and `resetFrom {runId, sequence}` of the new run (G2057), `run.reset` in its journal; a run that was signalled is reset with `reapply: SIGNALS` (the preview lists the signal `WILL_TRY`, the report says `REAPPLIED`, the same key answers the same report) and reset to its start (the signal finds nothing waiting: `REJECTED` `NOT_WAITING`); the server runs with `SAGAT_RESET_REAPPLY_ENABLED` (`server.resetReapply`); `CANCEL_SOURCE` of a run held in flight answers `PENDING_SOURCE` (the same key the same entry, another request under it `IDEMPOTENCY_KEY_REUSED`, the register and the read by id show it, the source `CANCELLING`) and of a run that ended makes the run at once; the server runs with `SAGAT_RESET_CANCEL_SOURCE_ENABLED` (`server.resetCancelSource`), and the source never ends, so the observations are the same on every run (the engine's pass is proved by the integration suite) |

The concurrency-permit reset of feature 022 is not a scenario: the permit is not built (T018-053), and a scenario for it
would assert nothing.

### Steps and server settings the scenarios use

The step operations are those of `tests/contract/sdk-conformance/README.md`, with the feature 018 ones listed there
(`scheduleCreate` ... `getReset`, `awaitSchedule`, `awaitBackfill`, `awaitBatch`). A parameter of a step may be a
reference: `{{step:<id>:<field>}}`, `{{workflow:<key>}}`, `{{namespaceId}}`, `{{now:<offsetMs>}}` or
`{{hour:<offsetHours>}}` (the start of a UTC hour). A scenario that needs the engine's schedule or batch passes says so
in `server` (`"schedules": true`, `"batches": true`): those passes are off in an engine until told to run, and a
scenario that waits on them without the setting would only time out.

A scenario runs only against a server that implements its routes; a server that does not answers
`NOT_FOUND` with reason `ROUTE_NOT_FOUND` (canonical `Error.details` rule), and the harness records the
scenario as skipped with that evidence, never as passed.
