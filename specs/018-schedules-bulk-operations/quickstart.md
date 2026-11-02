# Quickstart — feature 018 demo (planned)

**Nothing below runs yet.** The routes, CLI subcommands and test targets named here are proposed by
this feature and do not exist on `develop`. This page is the demo the Definition of Done requires; it
becomes runnable as the tasks land, and T018-075 records the actual results (including skips).

## 1. Prerequisites

The development stack and CLI of [docs/quickstarts/README.md](../../docs/quickstarts/README.md):

```sh
cp infra/docker/.env.example infra/docker/.env
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env --profile seed run --rm seed
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)
sagat new hello-world && sagat workflow publish hello-world/workflow.json
export NS=<namespace UUID of payments>     # from GET $SAGAT_SERVER/namespaces or the Studio
export WF=<workflow id printed by the publish command>
```

Serve the hello-world activities with canned answers as in the developer quickstart (§4 there), and
keep that worker running. For §4 below publish this workflow too (its entry waits for the signal):

```json
{
  "apiVersion": "sagat-octopus-orchestrator.io/v1",
  "kind": "Workflow",
  "metadata": {"name": "cart", "description": "Entity workflow: waits for items."},
  "entrypoint": "wait-item",
  "triggers": [{"kind": "manual"}],
  "inputSchema": {"type": "object", "properties": {"cartId": {"type": "string"}}},
  "nodes": [
    {"id": "wait-item", "type": "waitEvent", "eventType": "item-added",
     "correlationExpr": "input.cartId", "timeoutMs": 3600000, "next": "compose"},
    {"id": "compose", "type": "activity", "activity": "greeting.compose", "next": "deliver"},
    {"id": "deliver", "type": "activity", "activity": "greeting.deliver"}
  ]
}
```

## 2. Happy path — a schedule (US01, US02)

```sh
curl -sS -X POST "$SAGAT_SERVER/namespaces/$NS/schedules" \
  -H "Authorization: Bearer $SAGAT_TOKEN" -H 'Content-Type: application/json' \
  -H 'Idempotency-Key: demo-schedule-0001' -d @- <<EOF
{"name": "hello-every-minute",
 "spec": {"intervals": [{"everyMs": 60000}], "timezone": "UTC", "jitterMs": 5000},
 "policies": {"overlap": "SKIP"},
 "action": {"workflowId": "$WF", "input": {"name": "schedule"}}}
EOF
```

Expected: `201` with `revision: 1` and `nextActionAt`. Within two minutes:

```sh
sagat schedule get hello-every-minute --recent 5     # planned CLI
```

shows `STARTED` actions with run ids; each run has `startedBy: system:schedule:<id>` and
`schedule.nominalTime` on an exact minute, `actualTime` within 5 s after it. Then:

```sh
sagat schedule pause hello-every-minute --note "maintenance window 14:00-14:05"
sagat schedule trigger hello-every-minute            # runs once although paused
sagat schedule resume hello-every-minute --note "maintenance done"
sagat schedule backfill hello-every-minute --start <10 minutes ago> --end <now>
```

Expected: no `SCHEDULED` action while paused; one `TRIGGERED` action; after resume the next action is
the first minute after the resume instant; the backfill reports `ALREADY_ACTED` for minutes that ran
and `STARTED` (sequentially) for the paused minutes. The audit trail
(`GET /audit` as today) shows `schedule.paused`, `schedule.triggered`, `schedule.resumed`,
`schedule.backfill_requested` with the notes and actors.

## 3. Happy path — business identifier and delay (US03)

```sh
# `sagat workflow run` exists today; the flags below are planned (T018-069).
sagat workflow run hello-world --input '{"name":"order"}' --business-id order-123
sagat workflow run hello-world --input '{"name":"order"}' --business-id order-123            # 409 BUSINESS_ID_IN_USE
sagat workflow run hello-world --input '{"name":"order"}' --business-id order-123 --if-exists use-existing   # 200, same run
sagat workflow run hello-world --input '{"name":"later"}' --business-id reminder-9 --start-delay 2m
```

Expected: the delayed run is `WAITING` with `startDelayedUntil`; `POST /runs/{id}/signals/item-added` against it is 409;
after two minutes its first step is released once.

## 4. Happy path — Signal-with-Start (US04)

```sh
sagat workflow signal-with-start cart --business-id cart-42 --input '{"cartId":"cart-42"}' \
  --signal item-added --payload '{"sku":"A1"}'
```

Expected: `started: true`, `deliveredTo: ["wait-item"]`; the run proceeds to `compose`. Repeating
it while the run is past `wait-item` returns 409 `NOT_WAITING` and changes nothing.

## 5. Happy path — batch and reset (US05, US06)

```sh
sagat batch preview --workflow hello-world --status RUNNING --created-before <now>
sagat batch create --workflow hello-world --status RUNNING --action CANCEL \
  --reason "faulty greeting service, incident 42" --rate 5
sagat batch get <batchId> --items
```

Expected: the preview count equals the matched runs; the job cancels them at about 5 per second;
items are `SUCCEEDED` or `SKIPPED` (`ALREADY_TERMINAL`); counters equal items.

```sh
sagat workflow reset points <failedRunId>
sagat workflow reset preview <failedRunId> --sequence <after compose> --reapply SIGNALS
sagat workflow reset apply <failedRunId> --sequence <after compose> --acknowledge-reexecution \
  --reason "deliver fixed in release 1.2"
```

Expected: a new run with `rerunOf` and `resetFrom` whose journal begins with the restored prefix and
`run.reset`; `deliver` runs again; the source stays terminal.

## 6. Failure scenarios (each must be demonstrated)

| Scenario | How | Expected |
| --- | --- | --- |
| Engine crash during a firing | `docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env kill execution-engine` while a schedule is due, then `up -d execution-engine` | one run per nominal time; no orphan action (test: `fault_schedules_bulk_operations firing`) |
| Outage longer than the catch-up window | stop the engine for 5 min with a 1-minute schedule and `catchupWindowMs: 120000` | one `STARTED` (mode `ONE`) for the earliest time inside the window, the others `COALESCED`, older ones in one `MISSED_CATCHUP_WINDOW` summary |
| Authority revoked | remove the developer who created the schedule from the namespace | next action `AUTHORITY_REVOKED`, schedule `PAUSED` with a system note, audit `schedule.auto_paused` |
| Signal-with-Start on a workflow whose entry does not wait | use `hello-world` | 409 `SIGNAL_NOT_ACCEPTED_AT_START`; `GET business-ids/<id>` is 404 (nothing created) |
| Batch cancelled half way | `sagat batch cancel <id> --reason "stop now please"` | remaining items `CANCELLED`, applied cancellations stay applied |
| Reset of a run with an attempt in flight | choose a sequence not listed by `points` | 422 `RESET_POINT_NOT_ELIGIBLE` |
| Pause a trigger-owned schedule before the handover | `sagat schedule pause <trigger-owned> --note "test"` while `schedule_trigger_handover` is `LEGACY` | 409 `SCHEDULE_TRIGGER_HANDOVER_PENDING`; the tick keeps firing once |
| Quota at zero | set `schedules` quota of the namespace to 0, create a schedule | 429 `QUOTA_EXCEEDED` reason `SCHEDULE_LIMIT`; existing schedules keep firing |

## 7. Automated evidence (planned targets)

```sh
cargo test -p sagat-tests --test integration_schedules_bulk_operations --locked
cargo test -p sagat-tests --test fault_schedules_bulk_operations --locked
cargo test -p sagat-tests --test contract_schedules_bulk_operations --locked
cargo test -p sagat-tests --test sdk_conformance --locked
```

Contracts referenced: [rest.openapi.yaml](contracts/rest.openapi.yaml),
[schedule-spec.schema.json](contracts/schedule-spec.schema.json),
[journal-and-events.md](contracts/journal-and-events.md), [errors.md](contracts/errors.md),
[sdk-conformance.md](contracts/sdk-conformance.md); data in [data-model.md](data-model.md).
