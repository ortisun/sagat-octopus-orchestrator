# Quickstart — feature 018 demo

This page is the demo the Definition of Done asks for, and everything on it was run for real on 2026-11-09 against the
development stack of this repository (commit `98818160` plus the changes of T018-075), by following it from the top. Each
output below is the one the command gave, trimmed only for length. What was run, step by step, with the steps that were
skipped, the one that failed and the defects the demo found, is recorded in [verification.md](verification.md).

Three things to know before you start:

- The schedule, batch and reset routes and passes are **off by default**, in the binaries and in the compose file. Section 1
  says which lines turn them on.
- The workflows of the demo are `hello-world`, which finishes in about 0.3 s once a worker serves it, and `cart`, which waits
  for a signal and so stays open for as long as you want. A demo about a run that is *open* needs `cart`.
- Times are UTC. The commands that wait (a schedule that fires every minute) need minutes: be patient.

## 1. Prerequisites

The development stack and CLI of [docs/quickstarts/README.md](../../docs/quickstarts/README.md), with three additions: the
lines that turn the feature on, a second workflow and its ids.

```sh
cp infra/docker/.env.example infra/docker/.env
```

Append these four lines to `infra/docker/.env`. `infra/docker/compose.yaml` passes them to the control plane (which serves the
routes) and to the engine (which runs the passes), and **the two must agree**; each is `false` when the line is absent, which
is what the binaries do too ([schedules](../../docs/runbooks/schedules.md#2-turning-it-on),
[batch operations](../../docs/runbooks/batch-operations.md#3-turning-it-on),
[reset](../../docs/runbooks/run-reset.md#4-doing-it-and-reading-what-became-of-it) say what each does and in which order to
turn it on and off):

```sh
SAGAT_SCHEDULES_ENABLED=true            # the schedule routes (control plane) and the schedule pass (engine)
SAGAT_BATCHES_ENABLED=true              # the batch routes and the batch pass
SAGAT_RESET_REAPPLY_ENABLED=true        # a reset may deliver signals and Updates again (control plane only)
SAGAT_RESET_CANCEL_SOURCE_ENABLED=true  # a reset may cancel a run that has not ended (control plane and engine)
```

Then the stack, the seed, the CLI and the token, as in the developer quickstart (the build of the images takes a few minutes
the first time):

```sh
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env --profile seed run --rm seed
cargo install --path apps/cli --locked
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)
sagat doctor            # ends with: nothing to fix
```

The two workflows. `hello-world` is the template; `cart` waits for a signal named `item-added` before it composes and
delivers the greeting (its entry waits, which is what Signal-with-Start needs):

```sh
sagat new hello-world
mkdir cart && cat > cart/workflow.json <<'EOF'
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
EOF
sagat workflow publish hello-world/workflow.json     # published  hello-world v1
sagat workflow publish cart/workflow.json            # published  cart v1
```

In a second terminal, serve the two activities (both workflows use them) with canned answers, and leave it running; it needs
its own environment and a token of its own, and it **stops when the engine does** (`the platform refused to hand out tasks:
transport error`): start it again after you restart the engine.

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)
sagat worker dev --mocks hello-world/mocks.yaml --workflow hello-world/workflow.json --bind
```

Back in the first terminal, the ids the REST calls below need (`sagat workflow publish` does not print them; `jq` reads them):

```sh
AUTH="Authorization: Bearer $SAGAT_TOKEN"
export NS=$(curl -sS "$SAGAT_SERVER/namespaces" -H "$AUTH" | jq -r '.items[] | select(.slug=="payments") | .id')
export WF=$(curl -sS "$SAGAT_SERVER/workflows" -H "$AUTH" -H 'X-Octopus-Namespace: payments' | jq -r '.items[] | select(.name=="hello-world") | .id')
export CART=$(curl -sS "$SAGAT_SERVER/workflows" -H "$AUTH" -H 'X-Octopus-Namespace: payments' | jq -r '.items[] | select(.name=="cart") | .id')
```

(Every request of the REST API that reads or writes inside a namespace names it, by the path as the schedule routes do or by the
header `X-Octopus-Namespace: payments`; every write carries an `Idempotency-Key`.)

## 2. Happy path — a schedule (US01, US02)

```sh
curl -sS -X POST "$SAGAT_SERVER/namespaces/$NS/schedules" \
  -H "$AUTH" -H 'Content-Type: application/json' \
  -H 'Idempotency-Key: demo-schedule-0001' -w '\nHTTP %{http_code}\n' -d @- <<EOF
{"name": "hello-every-minute",
 "spec": {"intervals": [{"everyMs": 60000}], "timezone": "UTC", "jitterMs": 5000},
 "policies": {"overlap": "SKIP"},
 "action": {"workflowId": "$WF", "input": {"name": "schedule"}}}
EOF
```

Expected: `201` with `revision: 1`, `state: ACTIVE` and `nextActionAt` (the start of the next minute). The answer is the schedule;
its id is `scheduleId` (`sagat schedule create --definition FILE` makes one from the same document and prints it). Keep it:

```sh
export SCHEDULE=$(sagat schedule list | awk '$2=="hello-every-minute" {print $1}')
sagat schedule list
```

```text
01a1c29f-8f88-76c1-8728-ddd50c7ffe1d  hello-every-minute           ACTIVE  revision 1  API
```

Within two minutes the schedule has acted. `sagat schedule actions` lists its decisions, newest first, and
`GET /runs?scheduleId=` the runs it started:

```sh
sagat schedule actions $SCHEDULE
curl -sS "$SAGAT_SERVER/runs?scheduleId=$SCHEDULE&limit=2" -H "$AUTH" -H 'X-Octopus-Namespace: payments' | jq -c '.items[] | {runId, status, startedBy}'
```

```text
2026-11-09T23:26:00Z  SCHEDULED STARTED                01a1c2a2-0649-757f-80fa-96d3ba3079b3
2026-11-09T23:25:00Z  SCHEDULED STARTED                01a1c2a1-22a6-761b-9094-aa8e6321677a
2026-11-09T23:24:00Z  SCHEDULED STARTED                01a1c2a0-3e44-74ae-bc92-011a1275ebf8
{"runId":"01a1c2a2-0649-757f-80fa-96d3ba3079b3","status":"SUCCEEDED","startedBy":"system:schedule:01a1c29f-8f88-76c1-8728-ddd50c7ffe1d"}
```

Each run has `startedBy: system:schedule:<id>`. Its **nominal** time is the exact minute (the first column) and its actual time
is the start of the run, within the 5 s of jitter after it: `GET /namespaces/$NS/schedules/$SCHEDULE/actions` has both,
`"nominalTime":"2026-11-09T23:24:00Z","actualTime":"2026-11-09T23:24:04.040211Z"`. **The run resource itself does not carry
the schedule's nominal time, kind or revision** (FR-018-016 asks that it does; this is the gap G2057 and step 2.4 of
verification.md): read them from the action, which names the run.

`sagat schedule get $SCHEDULE` says when it acts next (the jittered times) and how many of the runs it started are still open (a run that has ended is not listed, whether or not the schedule has taken its next step; before T018-075 it was, for as long as the schedule was paused):

```text
schedule 01a1c29f-8f88-76c1-8728-ddd50c7ffe1d  hello-every-minute  ACTIVE  revision 1
  next: 2026-11-09T23:25:01.801Z
  ...
  recent: 2026-11-09T23:24:00Z  STARTED
  0 open run(s), 0 buffered
```

Then pause, run now, resume, and backfill. Pause, wait two minutes, and look at the decisions: none for the minutes it was
paused. **Run now** acts once although the schedule is paused (kind `TRIGGERED`, the instant of the request):

```sh
sagat schedule pause $SCHEDULE --note "maintenance window 14:00-14:05"        # schedule … is now PAUSED
sleep 150
sagat schedule actions $SCHEDULE --limit 3                                    # nothing since the pause
sagat schedule trigger $SCHEDULE                                              # runs once although paused
sagat schedule resume $SCHEDULE --note "maintenance done"                     # schedule … is now ACTIVE
```

```text
schedule 01a1c29f-… was triggered: STARTED, run 01a1c2a4-5050-712c-abe5-e7c104e73c5a
2026-11-09T23:28:30.798523Z  TRIGGERED STARTED                01a1c2a4-5050-712c-abe5-e7c104e73c5a
2026-11-09T23:26:00Z  SCHEDULED STARTED                01a1c2a2-0649-757f-80fa-96d3ba3079b3
```

After the resume, `schedule get` lists the next time as the first minute after the resume instant (`next: 2026-11-09T23:29:03.714Z`
for a resume at 23:28:37). The pause and the resume leave a **note, who wrote it and when on the schedule** (`note`,
`noteBy`, `noteAt` of `GET …/schedules/$SCHEDULE`: `"note":"maintenance done","noteBy":"dev-admin"`).

Backfill acts on the nominal times of a past range that have no decision yet. `--to` must not be later than now
(`422 BACKFILL_RANGE_INVALID: start must be before end and end not after now`); the range is half open; times are RFC 3339:

```sh
sagat schedule backfill $SCHEDULE --from 2026-11-09T23:24:00Z --to 2026-11-09T23:28:37Z
# backfill 01a1c2a4-81b3-7280-835c-70a98ebe65d2 of schedule … is PENDING: 2026-11-09T23:24:00Z to 2026-11-09T23:28:37Z
curl -sS "$SAGAT_SERVER/namespaces/$NS/schedules/$SCHEDULE/backfills" -H "$AUTH" | jq -c '.items[]'
```

```text
{"backfillId":"01a1c2a4-81b3-7280-835c-70a98ebe65d2","start":"2026-11-09T23:24:00Z","end":"2026-11-09T23:28:37Z","cursor":"2026-11-09T23:28:00Z","state":"COMPLETED","overlap":"BUFFER_ALL","counts":{"ALREADY_ACTED":3,"BUFFERED":1,"STARTED":1}}
2026-11-09T23:28:00Z  BACKFILL  STARTED                01a1c2a4-9b0e-77c5-a196-9c53c98748ce
2026-11-09T23:27:00Z  BACKFILL  STARTED                01a1c2a4-8469-7446-be28-3e87cedcfab6
```

The three minutes that had run (23:24, 23:25, 23:26) are `ALREADY_ACTED` and **are counted in the backfill, not listed as
actions**; the two paused minutes (23:27, 23:28) are started one after the other (`BUFFER_ALL`: the second waited, `BUFFERED`,
until the first run ended).

The audit trail, `GET /audit`, has the decisions with their actors. It does **not** carry the note (a note is customer text and
is kept on the schedule, not copied into the audit):

```sh
curl -sS "$SAGAT_SERVER/audit?limit=100" -H "$AUTH" -H 'X-Octopus-Namespace: payments' | jq -c '.items[] | select(.action | startswith("schedule.")) | [.action, .actor, .reason]'
```

```text
["schedule.backfill_requested","dev-admin","backfill requested"]
["schedule.resumed","dev-admin","schedule resumed"]
["schedule.triggered","dev-admin","trigger-now"]
["schedule.paused","dev-admin","schedule paused"]
["schedule.created","dev-admin","schedule created"]
```

## 3. Happy path — business identifier and delay (US03)

A business id is held by **one open run**. `hello-world` finishes in 0.3 s, so a second start a moment later may find it
closed and start a new run (the default policy allows a duplicate once the holder has ended); to see the conflict, use `cart`,
which stays open:

```sh
sagat workflow run cart --input '{"cartId":"order-123"}' --business-id order-123
sagat workflow run cart --input '{"cartId":"order-123"}' --business-id order-123
sagat workflow run cart --input '{"cartId":"order-123"}' --business-id order-123 --if-exists use-existing
```

```text
run 01a1c29f-fb74-7377-b7f7-9cba6529af9a started: cart v1
sagat: another open execution holds this business identifier (CONFLICT, BUSINESS_ID_IN_USE). Another open execution holds this id; `details.runId` names it when the caller may read runs. Trace af75e4bc9ff84c07a8f016d74e6d39a7.            (exit 1)
run 01a1c29f-fb74-7377-b7f7-9cba6529af9a already holds that business id: nothing was started            (exit 0, HTTP 200)
```

A start can be held. `--start-delay` is in **seconds** (from 1, up to 366 days); the run is `WAITING` with `startDelayedUntil`,
holds its identifier, and refuses a signal:

```sh
sagat workflow run hello-world --input '{"name":"later"}' --business-id reminder-9 --start-delay 120
```

```text
run 01a1c2a0-13ef-7381-a414-634e11b58516 started: hello-world v1
  its first step is held until 2026-11-09T23:25:53.199646Z
```

```sh
RUN=$(sagat workflow run hello-world --input '{"name":"later"}' --business-id reminder-10 --start-delay 120 | awk 'NR==1 {print $2}')
curl -sS "$SAGAT_SERVER/runs/$RUN" -H "$AUTH" -H 'X-Octopus-Namespace: payments' | jq -c '{status, startDelayedUntil, businessId}'
curl -sS -X POST "$SAGAT_SERVER/runs/$RUN/signals/item-added" -H "$AUTH" -H 'X-Octopus-Namespace: payments' \
  -H 'Content-Type: application/json' -H 'Idempotency-Key: demo-signal-delayed-1' -d '{"payload":{"sku":"A1"}}' -w '\nHTTP %{http_code}\n'
```

```text
{"status":"WAITING","startDelayedUntil":"2026-11-09T23:25:53.199646Z","businessId":"reminder-9"}
{"code":"CONFLICT","message":"the run is not waiting for that signal",...,"details":{"reason":"STATE_CONFLICT"}}
HTTP 409
```

After two minutes the first step is released once, within about a second of the instant (`run.start_released` appears once in
`sagat workflow inspect $RUN`, 0.9 s after `startDelayedUntil` here):

```text
     1  run.started
     2  run.start_delayed
     3  run.start_released
     4  step.released  compose
```

## 4. Happy path — Signal-with-Start (US04)

```sh
sagat workflow signal-with-start cart --business-id cart-42 --input '{"cartId":"cart-42"}' \
  --signal item-added --payload '{"sku":"A1"}'
sagat workflow signal-with-start cart --business-id cart-42 --input '{"cartId":"cart-42"}' \
  --signal item-added --payload '{"sku":"A2"}'
```

```text
run 01a1c2a1-1047-7612-846d-e7c3ca965fe2 was started and signalled: 1 step(s) woke
nothing was signalled: the run is not waiting for that signal (CONFLICT, NOT_WAITING). No step of the run waits for this signal now; signals are never buffered. Trace 29a7cfbf5576426aa6685845bd51a8af.            (exit 1)
```

The REST call says the same with `started` and `deliveredTo`:

```sh
curl -sS -X POST "$SAGAT_SERVER/runs/signal-with-start" -H "$AUTH" -H 'X-Octopus-Namespace: payments' \
  -H 'Content-Type: application/json' -H 'Idempotency-Key: demo-sws-43' -w '\nHTTP %{http_code}\n' \
  -d "{\"workflowId\":\"$CART\",\"businessId\":\"cart-43\",\"input\":{\"cartId\":\"cart-43\"},\"signal\":{\"name\":\"item-added\",\"payload\":{\"sku\":\"B1\"}}}"
# {"runId":"01a1c2a1-25a4-77cf-b610-ae8314152731","started":true,"deliveredTo":["wait-item"]}
# HTTP 202
```

The run proceeds to `compose` and `deliver`. Repeating the call while the run is past `wait-item` is `409 NOT_WAITING` and changes
nothing. **That depends on the run still being open**: once the worker has finished it, the identifier is free and the call
starts a new run and signals it (`started: true`); with `hello-world` the run is over in 0.3 s.

## 5. Happy path — batch and reset (US05, US06)

The runs to act on: twelve more `cart` runs, all waiting. A job selects the runs of a **selector**, which is JSON: a filter
(`workflowId`, `versionId`, `status`, `origin`, `scheduleId`, `createdAfter`, `createdBefore`) or a list of run ids. The status
of a run that waits for a signal is `WAITING` (`RUNNING` is a run whose step is in flight):

```sh
for i in 1 2 3 4 5 6 7 8 9 10 11 12; do sagat workflow run cart --input "{\"cartId\":\"batch-$i\"}" | head -1; done
SELECTOR="{\"filter\":{\"workflowId\":\"$CART\",\"status\":[\"WAITING\"],\"createdBefore\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"}}"
sagat batch preview --selector "$SELECTOR"
BATCH=$(sagat batch create --selector "$SELECTOR" --action CANCEL --reason "faulty greeting service, incident 42" --rate 5 | awk '/created/ {print $3}')
sagat batch get $BATCH
sagat batch items $BATCH --limit 20
```

```text
13 run(s) match
  01a1c29f-fb74-7377-b7f7-9cba6529af9a
  01a1c2a2-c386-70c3-9f8d-74ac4d741061
  ...
batch job 01a1c2a2-e070-71b5-8381-c3af2942662c created: CANCEL (SELECTING)
batch job 01a1c2a2-e070-71b5-8381-c3af2942662c  CANCEL  RUNNING
  13 selected: 12 succeeded, 0 skipped, 0 failed, 0 cancelled, 0 deferred
batch job 01a1c2a2-e070-71b5-8381-c3af2942662c  CANCEL  COMPLETED
  13 selected: 13 succeeded, 0 skipped, 0 failed, 0 cancelled, 0 deferred
     1  01a1c29f-fb74-7377-b7f7-9cba6529af9a  SUCCEEDED
     ...
```

The preview count equals the matched runs (the twelve and the open `order-123` run of section 3); the job cancels them at about the
rate asked for (5 a second: 12 of 13 in about three seconds); every item is `SUCCEEDED` (a run that had already ended would be
`SKIPPED` with `ALREADY_TERMINAL`) and the counters equal the items.

Resetting a run to a point makes a new run from a prefix of its journal. The source must have ended: use the `cart` run of
the signal (start one, send it `item-added`, wait for it to finish):

```sh
RUN=$(sagat workflow run cart --input '{"cartId":"reset-1"}' --business-id reset-1 | awk 'NR==1 {print $2}')
curl -sS -X POST "$SAGAT_SERVER/runs/$RUN/signals/item-added" -H "$AUTH" -H 'X-Octopus-Namespace: payments' \
  -H 'Content-Type: application/json' -H 'Idempotency-Key: demo-signal-reset-1' -d '{"payload":{"sku":"A1"}}'
sleep 3                                              # the worker completes compose and deliver
sagat workflow reset-points $RUN
sagat workflow reset-preview $RUN --sequence 1 --reapply signals
sagat workflow reset $RUN --sequence 1 --acknowledge --reapply signals --reason "deliver fixed in release 1.2"
```

```text
run 01a1c2a3-464c-73b2-b560-e48586d325b8 can be reset to:
       1  the start of the run  (2026-11-09T23:27:22.70169Z)
       7  after step compose  (2026-11-09T23:27:25.664775Z)
      11  after step deliver  (2026-11-09T23:27:25.698792Z)
a reset of run 01a1c2a3-… to point 1 would:
  keep nothing: the new run starts from the beginning
  run again: wait-item
  run again: compose (it completed in this run: running it again repeats what it did)
  run again: deliver (it completed in this run: running it again repeats what it did)
  deliver again what the run received after that point, in order:
    signal item-added (event 3): will be tried
  the reset asks you to say you know that: add --acknowledge
reset 01a1c2a3-8317-777e-999c-565ff401f905: run 01a1c2a3-830f-7401-9178-e20d2d9ba169 was made from point 1 of run 01a1c2a3-464c-…
  delivered again, in the order the run received them:
    signal item-added (event 3): delivered
```

Expected, and seen: a new run with `rerunOf` the source (`GET /runs/<new>` has `"rerunOf":"01a1c2a3-464c-…"` and the same
business id), whose journal begins with the restored prefix and `run.reset` (`run.started`, `run.reset`, `wait.started`,
`wait.satisfied`…); the preview lists the signal the run received after the point as `will be tried`, and the reset reports it
`delivered` (it would say `rejected`, with the reason, for a signal nobody waited for yet, and `not delivered` for one that is no
longer kept); the steps run again (`compose` and `deliver` here); the source stays terminal. `sagat workflow resets <source>`
lists the resets made from it. Resetting to the end of `compose` (`--sequence 7`) keeps `compose` and `wait-item` as they were
and runs `deliver` again.

Delivering again what the run received after the point is the deployment setting `SAGAT_RESET_REAPPLY_ENABLED` of section 1
([run-reset.md](../../docs/runbooks/run-reset.md) 4.1).

## 6. Failure scenarios (each was demonstrated, or is marked as not demonstrable and why)

"Shown" is what the command gave on the stack of this page; "Test" is the automated evidence, which also covers what a demo cannot
(a connection killed after any statement). Codes are `reason`s of the answer's `details`, as in [contracts/errors.md](contracts/errors.md).

| Scenario | How | Shown | Test |
| --- | --- | --- | --- |
| Engine killed while schedules are due | `docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env kill execution-engine` a second before a minute, then `up -d execution-engine` 20 s later | the time that fell due while the engine was down (23:37:00, killed at 23:36:59, back at 23:37:21) was started once on its return and none was doubled or lost (`catchup-demo`, with `ALLOW_ALL`: 23:36, 23:37 and 23:38 `STARTED`, one run each); `hello-every-minute` (`SKIP`) recorded `SKIPPED_OVERLAP` at 23:38 because its 23:37 run was still open: **the worker had stopped with the engine** (start it again). The crash inside a firing, after each of its statements, is the test's | `fault_schedules_bulk_operations firing::`, `upgrade::` |
| Outage longer than the catch-up window | a schedule with `"catchupMode":"ONE","catchupWindowMs":120000` (`sagat schedule create --definition FILE`), `docker compose … stop execution-engine` for 5 min 15 s, then `up -d execution-engine` | `23:42:00 STARTED` (the earliest time inside the window), `23:43:00 COALESCED` (`summaryCount: 1`), `23:39:00 MISSED_CATCHUP_WINDOW` (`summaryCount: 3`, last `23:41:00`: the times older than the window, in one summary) | `catchup::` |
| Authority revoked | a member (`dev-dana`, `developer`) creates a schedule; the operator then removes the member (no REST route does: `DELETE FROM memberships …` as the database owner) | next action `AUTHORITY_REVOKED`, schedule `PAUSED` with `note: "the authorizing subject can no longer start runs"`, `noteBy: system`, audit `schedule.auto_paused` by `system:schedule:<id>` ("authority revoked") | `authority::` |
| Signal-with-Start on a workflow whose entry does not wait | `sagat workflow signal-with-start hello-world --business-id sws-hello-1 --input '{"name":"x"}' --signal item-added` | `409`, `details.reason` `SIGNAL_NOT_ACCEPTED_AT_START`: "the new run would not accept that signal: nothing was created"; `GET …/business-ids/sws-hello-2` is `404` | `with_start::`, `with_start_rest::` |
| Batch cancelled half way | a job at 1 item a second over twelve waiting runs, `sagat batch cancel <id> --reason "stop now please"` after 5 s | the job `CANCELLING`, then `CANCELLED` (by REST, `POST …/cancel` with `{"reason":"stop now please"}`: `12 selected: 5 succeeded, 0 skipped, 0 failed, 7 cancelled`; with the fixed CLI, over 22 waiting runs: `22 selected: 6 succeeded, 0 skipped, 0 failed, 16 cancelled`); the runs it had cancelled stay cancelled. Cancelling again: `409 BATCH_NOT_CANCELLABLE` ("The job already ended."); without `--reason` the CLI says `--reason is required` (exit 2). **The first demo failed here: the CLI sent no body and the platform answered `400 content type must be application/json` (defect, fixed in T018-075)** | `batch::`, `schedule_and_batch::a_job_is_read_listed_its_items_shown_and_cancelled` |
| Reset to a sequence that is not a point | `sagat workflow reset <run> --sequence 5 --acknowledge --reason "choose a sequence that is not a point"` | `422`, `RESET_POINT_NOT_ELIGIBLE`: "Choose a sequence listed by `listResetPoints`." | `reset::` |
| Reset that delivers again on a deployment without the setting | the control plane started with `SAGAT_RESET_REAPPLY_ENABLED=false`, `--reapply all` | `422`, `RESET_REAPPLY_NOT_AVAILABLE`: "Ask for `reapply: NONE`, or ask the operator of the platform to enable reset reapply."; nothing is made | `reapply::` |
| Reset that would deliver more than 256 things | `many-ticks`, a run of 257 sequential waits for `tick`, sent 257 signals (about 11 s, the script is in verification.md), then `reset … --sequence 1 --reapply signals` | `422`, `RESET_REAPPLY_TOO_MANY` ("The run received more than 256 signals and retained Updates after that point; choose a later point or `reapply: NONE`"), for the preview as well; `sagat workflow resets` says no reset was made | `reapply::the_ceiling_is_the_most_a_reapply_holds_and_one_more_refuses_the_preview_and_the_reset` |
| Reset of a run that has not ended | `sagat workflow reset <run> --sequence 1 --reason "…" --acknowledge --cancel-source` on a waiting `cart` run, with the setting on in the control plane and the engine | without the flag: `409 RESET_SOURCE_NOT_TERMINAL`. With it `202`, `state: PENDING_SOURCE`, `newRunId: null`; the run is `CANCELLED`; within three seconds the engine made the run and `sagat workflow resets` says `was made` (`"state":"CREATED"`) | `cancel_source::`, `cancel_source_rest::an_engine_started_with_the_flag_creates_the_run_of_the_reset_the_api_accepted` |
| Reset of an open run with a step in flight and no acknowledgement | a `cart` run signalled while the worker serves `compose` with `delayMs: 10000` (the most a mock may take), then `sagat workflow reset <run> --sequence 1 --reason "…" --cancel-source` | `422`, `RESET_REEXECUTION_NOT_ACKNOWLEDGED`; the run stays `RUNNING` (not cancelled) and no entry is written | `cancel_source_rest::a_step_in_flight_needs_the_acknowledgement_and_a_request_without_it_cancels_nothing` |
| Reset of an open run on a deployment without the setting | the control plane with `SAGAT_RESET_CANCEL_SOURCE_ENABLED=false`, `--cancel-source` | `422`, `RESET_CANCEL_SOURCE_NOT_AVAILABLE`: "Wait for the run to end and reset it then, or ask the operator of the platform to enable resets that cancel their source."; the run stays `WAITING`, no reset was made | `cancel_source::` |
| Reset that waited and was killed | kill the connection after any statement of the request or of the engine's completion | NOT DEMONSTRATED: it needs fault injection inside a transaction (a trigger that kills the backend), which only the test has | `fault_schedules_bulk_operations cancel_source` |
| Reset killed while it delivers | kill the connection after any statement of the transaction | NOT DEMONSTRATED: same | `fault_schedules_bulk_operations reset` |
| Pause a trigger-owned schedule before the handover | publish a workflow with `"triggers":[{"kind":"cron","cron":"* * * * *","timezone":"UTC"}]` (its schedule is named `trigger-<id>` and listed with origin `TRIGGER`), `sagat schedule pause <id> --note "test"` | `409`, `SCHEDULE_TRIGGER_HANDOVER_PENDING`: "The legacy cron pass still runs; the operator completes the handover once every engine is upgraded."; the tick keeps firing once per minute; editing its spec is `409 SCHEDULE_TRIGGER_OWNED` ("Change the cron trigger in the workflow definition and publish it.") | `trigger_schedules::pause_resume_trigger_and_backfill_are_refused_until_the_handover_and_a_spec_edit_always` |
| Hand the cron triggers over | `docker compose … run --rm --no-deps migrate schedule-trigger-handover --operator demo --expect-live 1 --apply` (the service `migrate` has the owner role; the engine runs `SAGAT_SCHEDULES_ENABLED=true`) | `handed over: the cron triggers fire from the schedule pass now (1 live execution engine(s) list schedules-v1, requirement epoch 1). The legacy cron pass of every engine stops within about two seconds. There is no command to go back…`; `schedule_trigger_handover` is `HANDED_OVER` by `demo` as database role `sagat`; the schedule could then be paused (`schedule … is now PAUSED`) and run now (`was triggered: STARTED`), and no tick fired after the pause | `trigger_handover::`, `trigger_firing::after_the_handover_the_legacy_pass_starts_nothing_even_when_it_still_runs_and_the_schedule_pass_does` |
| Hand over while a publication is open, or a cron trigger has no schedule | the command while a publication of a cron workflow is uncommitted | NOT DEMONSTRATED: it needs a publication transaction held open for two seconds | `trigger_round3::a_long_publication_makes_the_command_try_again_and_does_not_hold_the_writers_behind_it`, `trigger_round3::the_refusal_for_a_trigger_that_is_not_planned_says_which_trigger_and_what_to_do` |
| Run now, or backfill, a trigger schedule whose trigger is gone | an older control plane drops the cron trigger | NOT DEMONSTRATED: there is no older control plane in the stack | `trigger_round3::a_backfill_or_a_run_now_of_a_schedule_whose_trigger_is_gone_is_refused_with_a_documented_reason` |
| Hand over with an engine that does not run the schedule pass | a second engine started with `docker compose … run -d --no-deps --name … -e SAGAT_SCHEDULES_ENABLED=false execution-engine`, then the command with `--expect-live 2 --dry-run` | `error: 1 live execution engine(s) do not list schedules-v1 (instance ids: 934c24b5-74af-4ba1-8fae-872e5715d26f). An engine lists it only when it runs the schedule pass: start it with SAGAT_SCHEDULES_ENABLED=true (or stop it), wait for it to register, and run the command again. Nothing was changed.`; after the second engine was stopped the dry run said `dry run: nothing stops the handover` | `trigger_handover::the_handover_refuses_while_an_engine_lacks_the_capability_and_names_it` |
| Rolling upgrade on one tick | the legacy pass and the schedule pass reach one tick, in either order, one killed after each write | PARTLY: before the handover both passes ran on every tick, and every tick of `cron-hello` has one run (`startedBy: system:engine`) and one action `ALREADY_ACTED`; the kills after each write are the test's | `fault_schedules_bulk_operations upgrade::*` |
| Quota at zero | `PUT /namespaces/$NS/quotas/schedules` with `{"limit":0}`, then create a schedule | `429`, `QUOTA_EXCEEDED`, `details.reason` `SCHEDULE_LIMIT` ("schedules in the namespace"); the existing schedule fired at the next minute; `DELETE …/quotas/schedules` put the default back | `limits::` |

## 7. Automated evidence

Each target below exists (`tests/Cargo.toml`); they run against a real PostgreSQL through Testcontainers and need Docker. The
runtime-role sweep is the same command with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.

```sh
cargo test -p sagat-tests --test integration_schedules_bulk_operations --locked      # service, store, REST, firing, batch, reset, triggers, registry
cargo test -p sagat-tests --test fault_schedules_bulk_operations --locked            # a kill after each write, stale fences, barriers, locks
cargo test -p sagat-tests --test contract_schedules_bulk_operations --locked         # the REST responses against the OpenAPI, the errors
cargo test -p sagat-tests --test e2e_schedules_bulk_operations --locked              # compiles; the browser journeys are frontend/e2e (below)
cargo test -p sagat-tests --test perf_schedules_bulk_operations --locked             # what a pull request runs of the benchmark (4 tests); the report is tests/performance/reports/schedules-bulk-operations.md
SAGAT_SDK_SCENARIO=schedules-crud cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture   # six languages, one scenario
cargo test -p sagat-tests --test e2e_workflow_studio --locked -- --ignored journeys  # the fourteen keyboard-only browser journeys of the Studio
cargo test -p sagat-domain --test schedules --test schedules_properties --test business_ids --test batch --locked
cargo test -p sagat-infra-ir --test schedule_calendar --locked
cargo test -p sagat-cli --locked
```

Contracts referenced: [rest.openapi.yaml](contracts/rest.openapi.yaml),
[schedule-spec.schema.json](contracts/schedule-spec.schema.json),
[journal-and-events.md](contracts/journal-and-events.md), [errors.md](contracts/errors.md),
[sdk-conformance.md](contracts/sdk-conformance.md); data in [data-model.md](data-model.md).
