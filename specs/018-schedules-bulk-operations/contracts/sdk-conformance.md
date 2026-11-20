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

| File | Covers | Assertions (normalized; identical across the five SDKs) |
| --- | --- | --- |
| `schedules-crud.json` | create with calendar + cron + interval, describe, update with stale revision, delete | revision numbers, `REVISION_CONFLICT`, next nominal times for a fixed spec and a fixed database instant supplied by the harness |
| `schedules-operate.json` | pause/resume with note, trigger-now while paused, replay of trigger with same key, backfill with `ALREADY_ACTED` | action records (kind, outcome, run started or not), remaining actions unchanged by trigger |
| `schedules-overlap.json` | `SKIP`, `BUFFER_ONE`, `ALLOW_ALL` with a blocking activity the harness controls | outcomes per nominal time |
| `business-ids.json` | `FAIL`, `USE_EXISTING`, each reuse policy, lookup | 409 reasons, `started` flag, binding fields |
| `start-delay.json` | delayed start, signal refused, cancel during delay | status `WAITING`, `startDelayedUntil` present, `NOT_WAITING`, final `CANCELLED` |
| `signal-with-start.json` | absent id (entry waits), open waiting run, open run not waiting, entry not waiting | `started`, `deliveredTo`, 409 reasons, no run created on refusal |
| `update-with-start.json` | applied, rejected by validator, undeclared operation, replay with same key | receipt status, `started`, revisions, replay equality |
| `batch-operations.json` | preview, create cancel job over seeded runs, list items, cancel job; `REPIN` refused without feature 019 | counts, item states and reasons, `BATCH_ACTION_UNAVAILABLE` |
| `reset.json` | list points, preview, reset `REQUIRE_TERMINAL` with `SIGNALS` reapply; with 022 present, a reset on a busy concurrency key | points, plan (`concurrencyPermit`), `resetFrom`, reapply report, `CONCURRENCY_PERMIT_UNAVAILABLE` |
| `errors-018.json` | each `details.reason` reachable from a client | code, reason, retryable |

A scenario runs only against a server that implements its routes; a server that does not answers
`NOT_FOUND` with reason `ROUTE_NOT_FOUND` (canonical `Error.details` rule), and the harness records the
scenario as skipped with that evidence, never as passed.
