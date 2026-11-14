# Quickstart — feature 018 demo

This page is the demo the Definition of Done asks for. It was run for real twice against the development stack of this
repository, by following it from the top: on 2026-11-09 (commit `98818160` plus the changes of T018-075) and again on
**2026-11-11 (commit `83447218`, the images built from that tree)**, which is the run the outputs below come from unless a
line says otherwise. The second pass added what the tree gained since the first (the provenance of a run, the business-id
prefix of a schedule's action and of a batch selector, the bound on a selection that keeps failing). Each output below is the
one the command gave, trimmed only for length (an id or a time differs from run to run). What was run, step by step, with the
steps that were skipped and the defects the demo found, is recorded in [verification.md](verification.md) (section (b), two
logs).

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

```text
ok    address      http://127.0.0.1:18080/v1
ok    token        expires in 1 h 0 min for dev-admin
...
ok    studio       http://127.0.0.1:18080/studio/
nothing to fix
```

Two things of this page need more than the four lines, and neither is needed by the rest. **Step 2.4 (the webhook of a finished
run)** needs the secret store, which exists only when the control plane and the engine are given a key ring: append
`SAGAT_SECRETS_KEYS=k1:<base64 of 32 random bytes>` to `infra/docker/.env` too (`openssl rand -base64 32`), the same line for both,
which `compose.yaml` passes on. **The two rows of section 6 about a selection that keeps failing and its alert** read the engine's metrics port and Prometheus: the repository's
`infra/docker/compose.observability.yaml` is the way (the collector, Jaeger, Prometheus and Grafana, the processes' `/metrics`
listening on the container's address); the second pass ran a cut-down copy of it as a second override file, because the pinned
images of the collector, Jaeger and Grafana were not in the local daemon and nothing was pulled: the two admin listeners
published on the loopback of this machine, and the repository's own Prometheus image, `prometheus.yml` and `alerts.yaml`:

```yaml
# obs.yaml, given as one more -f of the compose command
services:
  control-plane:   {environment: {SAGAT_ADMIN_ADDR: "0.0.0.0:9090"}, ports: ["127.0.0.1:19090:9090"]}
  execution-engine: {environment: {SAGAT_ADMIN_ADDR: "0.0.0.0:9091"}, ports: ["127.0.0.1:19092:9091"]}
  prometheus:      # the service of compose.observability.yaml, the same image and the same two files
    image: prom/prometheus:v3.15.0
    command: [--config.file=/etc/prometheus/prometheus.yml, --storage.tsdb.retention.time=1d]
    volumes: ["<repo>/infra/observability/prometheus.yml:/etc/prometheus/prometheus.yml:ro", "<repo>/infra/observability/alerts.yaml:/etc/prometheus/alerts.yaml:ro"]
    ports: ["127.0.0.1:19091:9090"]
```

The second pass also gave the stack **its own project name and image names** (`-p sagat018qs2`, a second override file that
renames the three built images), so that it moved no tag and no volume of another checkout; use the plain command of the
developer quickstart on your own machine.

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

Expected, and seen: `HTTP 201` with `revision: 1`, `state: ACTIVE` and `nextActionAt` (the start of the next minute). The answer is
the schedule; its id is `scheduleId` (`sagat schedule create --definition FILE` makes one from the same document and prints it). Keep it:

```sh
export SCHEDULE=$(sagat schedule list | awk '$2=="hello-every-minute" {print $1}')
sagat schedule list
```

```text
01a1ca36-31fc-706e-8daf-2238a92c24cd  hello-every-minute           ACTIVE  revision 1  API
```

Within two minutes the schedule has acted. `sagat schedule actions` lists its decisions, newest first, and
`GET /runs?scheduleId=` the runs it started:

```sh
sagat schedule actions $SCHEDULE
curl -sS "$SAGAT_SERVER/runs?scheduleId=$SCHEDULE&limit=2" -H "$AUTH" -H 'X-Octopus-Namespace: payments' | jq -c '.items[] | {runId, status, startedBy}'
```

```text
2026-11-11T10:47:00Z  SCHEDULED STARTED                01a1ca37-e5f2-77a0-81a1-641e835d44d7
2026-11-11T10:46:00Z  SCHEDULED STARTED                01a1ca36-f5af-70f6-bbdc-7f5dde861dc6
{"runId":"01a1ca37-e5f2-77a0-81a1-641e835d44d7","status":"SUCCEEDED","startedBy":"system:schedule:01a1ca36-31fc-706e-8daf-2238a92c24cd"}
{"runId":"01a1ca36-f5af-70f6-bbdc-7f5dde861dc6","status":"SUCCEEDED","startedBy":"system:schedule:01a1ca36-31fc-706e-8daf-2238a92c24cd"}
```

Each run has `startedBy: system:schedule:<id>` and, since G2057 was closed, the schedule action that made it: `schedule {scheduleId, nominalTime,
kind, revision}` in `GET /runs/{id}` and in every item of `GET /runs`. Its **nominal** time is the exact minute (the first column above), the time the
schedule calculated, to the microsecond; its actual time is the start of the run, within the 5 s of jitter after it, and stays on the action
(`GET /namespaces/$NS/schedules/$SCHEDULE/actions`: `"nominalTime":"2026-11-11T10:46:00Z","actualTime":"2026-11-11T10:46:01.905586Z"`), because the
contract gives the run no `actualTime`. The field is absent, not `null`, for a run no schedule started (the second pass checked the
keys of the answers: a run nobody scheduled has no `schedule` and no `resetFrom`, and a run a schedule started has `schedule` and no `resetFrom`).

```sh
curl -sS "$SAGAT_SERVER/runs?scheduleId=$SCHEDULE&limit=1" -H "$AUTH" -H 'X-Octopus-Namespace: payments' | jq -c '.items[] | {runId, startedBy, schedule}'
sagat workflow inspect 01a1ca36-f5af-70f6-bbdc-7f5dde861dc6     # a line after "started …": started by schedule <id> (scheduled), nominal time …, revision 1
```

This is **step 2.4 of the first pass, which failed on 2026-11-09 and was fixed afterwards without a rerun on the stack (G2066); the second pass
ran it on the rebuilt stack, and it passes**. The two commands, as run:

```text
{"runId":"01a1ca36-f5af-70f6-bbdc-7f5dde861dc6","startedBy":"system:schedule:01a1ca36-31fc-706e-8daf-2238a92c24cd","schedule":{"scheduleId":"01a1ca36-31fc-706e-8daf-2238a92c24cd","nominalTime":"2026-11-11T10:46:00Z","kind":"SCHEDULED","revision":1}}
run 01a1ca36-f5af-70f6-bbdc-7f5dde861dc6  SUCCEEDED  workflow 01a1ca35-f0bb-7754-b75d-d61663a08e99  version 01a1ca35-f0cd-72c4-b751-73d3a77200bc
  started 2026-11-11T10:46:01.905586Z by system:schedule:01a1ca36-31fc-706e-8daf-2238a92c24cd, finished 2026-11-11T10:46:01.99984Z
  started by schedule 01a1ca36-31fc-706e-8daf-2238a92c24cd (scheduled), nominal time 2026-11-11T10:46:00Z, revision 1
  trace f62bdd54f9fa44bfafdbc3e955bcae1e
```

The run made by a *run now* has `"kind":"TRIGGERED"` and, as nominal time, the instant of the request to the microsecond; one a backfill made says
`BACKFILL`. The Studio's run page says "Started by schedule *name* (scheduled), nominal time …" with a link to the schedule (read in the browser: section 2.5),
and the six SDKs read it as `Run.schedule`.

`sagat schedule get $SCHEDULE` says when it acts next (the jittered times) and how many of the runs it started are still open (a run that has ended is not listed, whether or not the schedule has taken its next step; before T018-075 it was, for as long as the schedule was paused):

```text
schedule 01a1ca36-31fc-706e-8daf-2238a92c24cd  hello-every-minute  ACTIVE  revision 1
  next: 2026-11-11T10:47:02.534Z
  next: 2026-11-11T10:48:03.791Z
  next: 2026-11-11T10:49:03.438Z
  next: 2026-11-11T10:50:01.775Z
  next: 2026-11-11T10:51:00.089Z
  recent: 2026-11-11T10:46:00Z  STARTED
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
schedule 01a1ca36-31fc-706e-8daf-2238a92c24cd is now PAUSED                     (paused 10:47:24)
2026-11-11T10:47:00Z  SCHEDULED STARTED                01a1ca37-e5f2-77a0-81a1-641e835d44d7      (the actions after 150 s: nothing for 10:48 and 10:49)
2026-11-11T10:46:00Z  SCHEDULED STARTED                01a1ca36-f5af-70f6-bbdc-7f5dde861dc6
schedule 01a1ca36-31fc-706e-8daf-2238a92c24cd was triggered: STARTED, run 01a1ca3a-80ea-759f-bcc6-00235c491f3a
schedule 01a1ca36-31fc-706e-8daf-2238a92c24cd is now ACTIVE                     (resumed 10:49:54)
```

After the resume, `schedule get` lists the next time as the first minute after the resume instant (`next: 2026-11-11T10:50:01.775Z`
for a resume at 10:49:54). The pause and the resume leave a **note, who wrote it and when on the schedule** (`note`,
`noteBy`, `noteAt` of `GET …/schedules/$SCHEDULE`: `{"state":"ACTIVE","note":"maintenance done","noteBy":"dev-admin","noteAt":"2026-11-11T10:49:54.171783Z"}`).

Backfill acts on the nominal times of a past range that have no decision yet. `--to` must not be later than now
(`422 BACKFILL_RANGE_INVALID: start must be before end and end not after now`); the range is half open; times are RFC 3339:

```sh
sagat schedule backfill $SCHEDULE --from 2026-11-11T10:46:00Z --to 2026-11-11T10:49:56Z
# backfill 01a1ca3a-8902-7366-8d41-e0aeb1799d45 of schedule … is PENDING: 2026-11-11T10:46:00Z to 2026-11-11T10:49:56Z
curl -sS "$SAGAT_SERVER/namespaces/$NS/schedules/$SCHEDULE/backfills" -H "$AUTH" | jq -c '.items[]'
```

```text
{"backfillId":"01a1ca3a-8902-7366-8d41-e0aeb1799d45","start":"2026-11-11T10:46:00Z","end":"2026-11-11T10:49:56Z","cursor":"2026-11-11T10:49:00Z","state":"COMPLETED","overlap":"BUFFER_ALL","counts":{"ALREADY_ACTED":2,"BUFFERED":1,"STARTED":1}}
2026-11-11T10:49:00Z  BACKFILL  STARTED                01a1ca3a-a1dc-74a3-b431-d948168138ec
2026-11-11T10:48:00Z  BACKFILL  STARTED                01a1ca3a-8a76-7191-888f-af8fc2d2e1cc
```

The two minutes that had run (10:46, 10:47) are `ALREADY_ACTED` and **are counted in the backfill, not listed as
actions**; the two paused minutes (10:48, 10:49) are started one after the other (`BUFFER_ALL`: the second waited, `BUFFERED`,
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
...
```

(the other schedules of this page, section 3.1, add their own `schedule.created`, `schedule.updated` and `schedule.triggered` rows; `schedule.paused` and `schedule.created` of this schedule are further down the list)

### 2.4 The webhook of a run a schedule started (G2066)

`run.finished` goes to a subscriber with `data.scheduleId` and `data.nominalTime` when a schedule started the run, and with neither for any other
run; no event carries a business identifier. A subscription needs a signing secret and an egress rule for the host (needs the key ring of section 1);
**a receiver cannot be run on this machine** (the events runbook says the platform never delivers to a loopback or a private address, and nothing was sent to the internet),
so the subscription points at a name that does not resolve, and the body is read from the row of the delivery, which holds the bytes a receiver is sent
(the delivery stays `PENDING` and is retried: `last_outcome` `NETWORK`):

```sh
curl -sS -X PUT "$SAGAT_SERVER/namespaces/$NS/secrets/hook-signing" -H "$AUTH" -H 'Content-Type: application/json' -H 'Idempotency-Key: qs-secret-1' -d '{"value":"whsec-<32 random characters>"}'
curl -sS -X PUT "$SAGAT_SERVER/namespaces/$NS/egress-policy" -H "$AUTH" -H 'Content-Type: application/json' -H 'Idempotency-Key: qs-egress-1' -d '{"rules":["receiver.example.com"],"ports":[443],"allowHttp":false}'
curl -sS -X POST "$SAGAT_SERVER/namespaces/$NS/webhook-subscriptions" -H "$AUTH" -H 'Content-Type: application/json' -H 'Idempotency-Key: qs-sub-1' \
  -d '{"name":"qs-hook","url":"https://receiver.example.com/octopus","secretName":"hook-signing","events":["run.finished"]}'      # 201
# a run a schedule started, under a business id (the prefixed schedule of section 3.1), cancelled so that it finishes; and a run nobody scheduled, with a business id of its own
sagat schedule trigger $MC ; sagat workflow cancel <that run> --reason "demo: finish the run so that its webhook is made"
sagat workflow run hello-world --input '{"name":"plain"}' --business-id plain-webhook-1 --wait
docker compose … exec -T postgres psql -U sagat -d sagat -At -c "SELECT kind, status, attempts, last_outcome, convert_from(payload,'UTF8') FROM webhook_deliveries ORDER BY created_at"
```

```text
run.finished | PENDING | 1 | NETWORK | {"data":{"namespaceId":"01a1ca35-911f-74df-94cf-7b5cfbbe29d3","nominalTime":"2026-11-11T10:49:12.695266Z","runId":"01a1ca39-def9-76ba-87cd-b96262411251","scheduleId":"01a1ca36-d6f8-71f1-b98b-ce5be49d8f75","status":"CANCELLED","tenantId":"01a1ca35-9108-71a7-a7df-0a5f7d72e2bb","traceId":"a385c2bf91ad4b53a62378eac34ce59c","workflow":"cart"},"id":"01a1ca39-df14-70bd-b28f-014913be25c5","time":"2026-11-11T10:49:12.721506Z","type":"run.finished"}
run.finished | PENDING | 1 | NETWORK | {"data":{"namespaceId":"01a1ca35-911f-74df-94cf-7b5cfbbe29d3","runId":"01a1ca39-df25-76c5-b589-e5dda880559c","status":"SUCCEEDED","tenantId":"01a1ca35-9108-71a7-a7df-0a5f7d72e2bb","traceId":"5c59dd72a49a4751a6e7b8679e9ac641","workflow":"hello-world"},"id":"01a1ca39-e0e0-76a5-8e5e-f1f1628a2203","time":"2026-11-11T10:49:13.175363Z","type":"run.finished"}
```

The first body carries `scheduleId` and `nominalTime` (the run was bound to `mc-2026-11-11T10:49:12Z` and the body says nothing of it); the second, of a run that had
a business id of its own (`plain-webhook-1`) and no schedule, carries neither. (A later segment of a chain a schedule started carries
neither: G2063; the second pass did not run a chain.)

### 2.5 The Studio, in a browser (what the feature added)

Open `http://127.0.0.1:18080/studio/`, choose the token sign-in and paste the output of `sagat dev token` (the token stays in the page's memory, so
move between pages by the Studio's own links: a reload signs you out). The second pass read these on the stack (no screenshot was kept):

- **Schedules, then a schedule with a prefix** (`hourly-cart`, after the update of section 3.1): *Business id prefix* `hcx, followed by a dash and the nominal time of
  the action`; *If an open run has the id* `Record that run and start nothing`; *If a closed run remembers the id* `Start a new run`. The decisions table has a *Why*
  column (`The business id was held by an open run.` on the `Already acted on` row of the second *run now*, with the run's short id) and the outcome filter has
  `Business id conflict`.
- **Edit** the same schedule: the field *Business id prefix* ("1 to 160 letters, digits and punctuation. 3 of 160 characters."), the two policies as choices
  (`Record a conflict and start nothing (default)`/`Record that run and start nothing`; `Start a new run (default)`/`Start a new run unless it succeeded`/`Record a
  conflict and start nothing`) and the line **`The next action would start its run under the business id hcx-2026-11-11T11:00:00Z.`**, which is the id the next
  hour would make. Nothing was saved.
- **Runs**: the filter *Business id starts with* (`mc-`); **Batch operation…** opens a dialog that says `A batch job applies one action to the runs with business id
  starting with "mc-"`, and *Count the runs* answered `15 runs match. For example 50fde4e5, f05968fe, 03c1b0fd.` (the runs under `mc-` at that moment, cancelled ones included). No job was started
  from the dialog.
- **A run page**: `STARTED BY SCHEDULE minute-cart (scheduled), nominal time Nov 11, 2026, 7:50:00 AM` (the time of the browser's zone, UTC-3) beside `BUSINESS ID
  mc-2026-11-11T10:50:00Z`; for a run a reset made, `RESET FROM run e9a16884; this run keeps the first 1 event of it`.
- **Batches** lists the jobs with their progress (`12 of 12`, `2 of 2`, `0 of 0`); it does not show the selector or the prefix of a job (G2084). The page of a job that ended
  `SELECTION_FAILED` (section 6) says `Failed`, `The job stopped. The platform could not select the runs.` and `0 of 0 runs have an outcome`.

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
run 01a1ca36-6335-77bd-9267-573b10873be4 started: cart v1
sagat: another open execution holds this business identifier (CONFLICT, BUSINESS_ID_IN_USE). Another open execution holds this id; `details.runId` names it when the caller may read runs. Trace 5e0356cd6bac4d628f127d39277283fc.            (exit 1)
run 01a1ca36-6335-77bd-9267-573b10873be4 already holds that business id: nothing was started            (exit 0, HTTP 200)
```

(Each success also prints a line `follow it with: sagat workflow inspect <run>`, and an error a second line `sagat: trace <id>`; they are left out here and below.)

A start can be held. `--start-delay` is in **seconds** (from 1, up to 366 days); the run is `WAITING` with `startDelayedUntil`,
holds its identifier, and refuses a signal:

```sh
sagat workflow run hello-world --input '{"name":"later"}' --business-id reminder-9 --start-delay 120
```

```text
run 01a1ca36-6376-77cd-913c-b25f150ef2fe started: hello-world v1
  its first step is held until 2026-11-11T10:47:24.470692Z
```

```sh
RUN=$(sagat workflow run hello-world --input '{"name":"later"}' --business-id reminder-10 --start-delay 120 | awk 'NR==1 {print $2}')
curl -sS "$SAGAT_SERVER/runs/$RUN" -H "$AUTH" -H 'X-Octopus-Namespace: payments' | jq -c '{status, startDelayedUntil, businessId}'
curl -sS -X POST "$SAGAT_SERVER/runs/$RUN/signals/item-added" -H "$AUTH" -H 'X-Octopus-Namespace: payments' \
  -H 'Content-Type: application/json' -H 'Idempotency-Key: demo-signal-delayed-1' -d '{"payload":{"sku":"A1"}}' -w '\nHTTP %{http_code}\n'
```

```text
{"status":"WAITING","startDelayedUntil":"2026-11-11T10:47:24.498488Z","businessId":"reminder-10"}
{"code":"CONFLICT","message":"the run is not waiting for that signal",...,"details":{"reason":"STATE_CONFLICT"}}
HTTP 409
```

After two minutes the first step is released once, within about a second of the instant (`run.start_released` appears once in
`sagat workflow inspect $RUN`; the run here ended at 10:47:25.03, 0.5 s after `startDelayedUntil`):

```text
     1  run.started
     2  run.start_delayed
     3  run.start_released
     4  step.released  compose
     ...
    12  run.finished  [SUCCEEDED]
```

### 3.1 The runs of a schedule under a business id (FR-018-016)

The step of the first demo that was refused (`422`, "business identifiers on schedules are not available yet") now makes the id. **Run on the
stack on 2026-11-11 (second pass).** Two schedules over `cart`, whose runs stay open: `minute-cart` fires every minute and lets runs overlap
(`ALLOW_ALL`), with the default policy `FAIL`; `hourly-cart` has the policy `USE_EXISTING`:

```sh
cat > minute-cart.json <<EOF
{"name": "minute-cart",
 "spec": {"intervals": [{"everyMs": 60000}], "timezone": "UTC"},
 "policies": {"overlap": "ALLOW_ALL"},
 "action": {"workflowId": "$CART", "input": {"cartId": "scheduled"}}}
EOF
cat > hourly-cart.json <<EOF
{"name": "hourly-cart",
 "spec": {"intervals": [{"everyMs": 3600000}]},
 "action": {"workflowId": "$CART", "input": {"cartId": "scheduled"}}}
EOF
sagat schedule create --definition minute-cart.json --business-id-prefix mc
sagat schedule create --definition hourly-cart.json --business-id-prefix hc --if-exists use-existing
export MC=$(sagat schedule list | awk '$2=="minute-cart" {print $1}') HC=$(sagat schedule list | awk '$2=="hourly-cart" {print $1}')
sagat schedule get $MC
```

```text
schedule 01a1ca36-d6f8-71f1-b98b-ce5be49d8f75 created: minute-cart (revision 1)
schedule 01a1ca36-d70a-729b-adae-aba3ed5167a2 created: hourly-cart (revision 1)
schedule 01a1ca36-d6f8-71f1-b98b-ce5be49d8f75  minute-cart  ACTIVE  revision 1
  next: 2026-11-11T10:46:00Z
  ...
  business id: mc-<time of the action>  (held by an open run: FAIL; remembered by a closed one: ALLOW_DUPLICATE)
  0 open run(s), 0 buffered
```

`hourly-cart` says `business id: hc-<time of the action>  (held by an open run: USE_EXISTING; remembered by a closed one: ALLOW_DUPLICATE)`. **The id of the runs.** At the
next minute `minute-cart` acted by itself; the run is bound to `<prefix>-<nominal time>` (RFC 3339 UTC, whole seconds, ending in `Z`), the run list finds it by the prefix, and the
binding names its holder:

```sh
sagat schedule actions $MC
curl -sS "$SAGAT_SERVER/runs?businessIdPrefix=mc-" -H "$AUTH" -H 'X-Octopus-Namespace: payments' | jq -c '.items[] | {runId, status, businessId, schedule}'
curl -sS "$SAGAT_SERVER/namespaces/$NS/business-ids/mc-2026-11-11T10:46:00Z" -H "$AUTH" | jq -c .
```

```text
2026-11-11T10:46:00Z  SCHEDULED STARTED                01a1ca36-f138-7698-8f15-a95599e0dd1c
{"runId":"01a1ca36-f138-7698-8f15-a95599e0dd1c","status":"WAITING","businessId":"mc-2026-11-11T10:46:00Z","schedule":{"scheduleId":"01a1ca36-d6f8-71f1-b98b-ce5be49d8f75","nominalTime":"2026-11-11T10:46:00Z","kind":"SCHEDULED","revision":1}}
{"businessId":"mc-2026-11-11T10:46:00Z","executionId":"01a1ca36-f138-7698-8f15-a95599e0dd1c","currentRunId":"01a1ca36-f138-7698-8f15-a95599e0dd1c","open":true,"workflowId":"01a1ca35-f0f4-76a2-bb1d-e605534b42dc","boundAt":"2026-11-11T10:46:00.763108Z"}
```

**Run now twice in the same second** (the id of *run now* is the prefix and the second of the request; the second call is given `--overlap allow-all`, because under the
default `SKIP` the open run of the first would skip the time before its id is asked for). The two calls were made right after a second began so that both fell in it. With
`FAIL` the second is **recorded `BUSINESS_ID_CONFLICT`, names no run and makes none**; with `USE_EXISTING` it is **`ALREADY_ACTED` and names the run that holds the id**:

```sh
sagat schedule trigger $MC --overlap allow-all ; sagat schedule trigger $MC --overlap allow-all
sagat schedule actions $MC --limit 3
sagat schedule trigger $HC --overlap allow-all ; sagat schedule trigger $HC --overlap allow-all
sagat schedule actions $HC --limit 2
```

```text
schedule 01a1ca36-d6f8-71f1-b98b-ce5be49d8f75 was triggered: STARTED, run 01a1ca37-a5ff-743a-b71a-2e26f8b20428
schedule 01a1ca36-d6f8-71f1-b98b-ce5be49d8f75 was triggered: BUSINESS_ID_CONFLICT
2026-11-11T10:46:47.063093Z  TRIGGERED BUSINESS_ID_CONFLICT   -  BUSINESS_ID_IN_USE
2026-11-11T10:46:47.037922Z  TRIGGERED STARTED                01a1ca37-a5ff-743a-b71a-2e26f8b20428
2026-11-11T10:46:00Z  SCHEDULED STARTED                01a1ca36-f138-7698-8f15-a95599e0dd1c
schedule 01a1ca36-d70a-729b-adae-aba3ed5167a2 was triggered: STARTED, run 01a1ca37-a9e7-7015-931b-48438b501295
schedule 01a1ca36-d70a-729b-adae-aba3ed5167a2 was triggered: ALREADY_ACTED, run 01a1ca37-a9e7-7015-931b-48438b501295
2026-11-11T10:46:48.058338Z  TRIGGERED ALREADY_ACTED          01a1ca37-a9e7-7015-931b-48438b501295  BUSINESS_ID_IN_USE
2026-11-11T10:46:48.037981Z  TRIGGERED STARTED                01a1ca37-a9e7-7015-931b-48438b501295
```

(`sagat schedule trigger` says only the outcome; the reason, `BUSINESS_ID_IN_USE`, is on the decision, in `schedule actions`. The run list under `mc-` then held two runs, `mc-2026-11-11T10:46:00Z`
and `mc-2026-11-11T10:46:47Z`: the conflicting time made none. The schedule did not pause and `remainingActions` was not used up.)

**An update that changes the prefix.** `schedule update` takes the document again, so the action is what the file and the flags say; with no `--business-id-prefix` and none in the
file the action **has no prefix any more**. Runs made earlier keep the id they were bound to; the run a later action makes is under the new prefix and the revision it was made under:

```sh
sagat schedule update $HC --definition hourly-cart.json --revision 1 --business-id-prefix hcx --if-exists use-existing     # schedule … updated: revision 2
sagat schedule get $HC | grep 'business id'                                    # business id: hcx-<time of the action>  (held by an open run: USE_EXISTING; …)
sagat schedule trigger $HC                                                    # SKIPPED_OVERLAP: the open run of the first trigger (the default overlap is SKIP)
sagat schedule update $HC --definition hourly-cart.json --revision 1 --business-id-prefix hcy      # a stale revision: 409 REVISION_CONFLICT, nothing changes
sagat schedule update $HC --definition hourly-cart.json --revision 2                               # revision 3: no prefix flag, none in the file
sagat schedule get $HC | grep 'business id' || echo '(no business id line)'                         # (no business id line)
sagat schedule update $HC --definition hourly-cart.json --revision 3 --business-id-prefix hcx --if-exists use-existing   # revision 4
sagat schedule trigger $HC --overlap allow-all                                # STARTED, run 01a1ca37-f9b3-7508-a500-85cd221f57bf
curl -sS "$SAGAT_SERVER/runs?businessIdPrefix=hc" -H "$AUTH" -H 'X-Octopus-Namespace: payments' | jq -c '.items[] | {businessId, status, revision: .schedule.revision}'
```

```text
{"businessId":"hcx-2026-11-11T10:47:08Z","status":"WAITING","revision":4}
{"businessId":"hc-2026-11-11T10:46:48Z","status":"WAITING","revision":1}
```

**The refusals.** A policy with no prefix, and a prefix that is not an id, are `422 VALIDATION_FAILED`, reason `SCHEDULE_SPEC_INVALID`, and the body names the **field** and never
echoes the text (the test text, `has space CANARYPFX`, was searched for in the body: 0 matches; the 161-character prefix likewise). The CLI refuses a bad prefix before it sends anything
(exit 2, then its usage):

```sh
curl -sS -X POST "$SAGAT_SERVER/namespaces/$NS/schedules" -H "$AUTH" -H 'Content-Type: application/json' -H 'Idempotency-Key: k1' -d "{\"name\":\"refused\",\"spec\":{\"intervals\":[{\"everyMs\":3600000}]},\"action\":{\"workflowId\":\"$CART\",\"input\":{},\"businessIdConflictPolicy\":\"USE_EXISTING\"}}"
curl -sS … -d "{…,\"action\":{…,\"businessIdPrefix\":\"has space CANARYPFX\"}}"                      # and with a prefix of 161 characters
sagat schedule create --definition hourly-cart.json --business-id-prefix "has space"
```

```text
{"code":"VALIDATION_FAILED","message":"the schedule is not acceptable: SCHEDULE_SPEC_INVALID","details":{"field":"/action/businessIdConflictPolicy","reason":"SCHEDULE_SPEC_INVALID"}}      HTTP 422
{"code":"VALIDATION_FAILED","message":"the schedule is not acceptable: SCHEDULE_SPEC_INVALID","details":{"field":"/action/businessIdPrefix","reason":"SCHEDULE_SPEC_INVALID"}}      HTTP 422
sagat: --business-id-prefix is 1 to 160 visible ASCII characters      (exit 2)
```

The tests that stand behind these lines are `rest::a_prefix_that_is_not_one_is_refused_with_its_place_and_never_echoed`, `scheduled_business_ids::` (the same second, `FAIL` and `USE_EXISTING`), and the CLI's
`a_schedule_with_a_prefix_says_so_…`.

**A batch job by the same prefix.** The preview counts the runs the prefix reaches (a literal; it counts the runs, whatever their state), alone or joined to another filter, and a prefix that
matches nothing counts zero and a job over it completes with no item. A job over `hc` cancelled the two runs of the hourly schedule (`hc-…` and `hcx-…` both begin with `hc`):

```sh
sagat batch preview --business-id-prefix mc-                       # 3 run(s) match (with the ids of the first ones)
sagat batch preview --business-id-prefix hc                        # 2 run(s) match
sagat batch preview --selector "{\"filter\":{\"businessIdPrefix\":\"mc-\",\"workflowId\":\"$CART\",\"status\":[\"WAITING\"]}}"       # 3 run(s) match
sagat batch preview --selector '{"filter":{"businessIdPrefix":"mc-","status":["SUCCEEDED"]}}'                                           # 0 run(s) match
sagat batch preview --business-id-prefix nomatch-                  # 0 run(s) match
sagat batch create --business-id-prefix nomatch- --action CANCEL --reason "demo: a prefix that matches nothing"
sagat batch create --business-id-prefix hc --action CANCEL --reason "demo: stop the runs of the hourly cart schedule" --rate 5
```

```text
batch job 01a1ca38-ce4c-725e-bcdb-e30cbf1d71f3  CANCEL  COMPLETED
  0 selected: 0 succeeded, 0 skipped, 0 failed, 0 cancelled, 0 deferred
batch job 01a1ca38-ce4c-725e-bcdb-e30cbf1d71f3 has no items
batch job 01a1ca38-de2e-7424-97a2-19bc98bc7395  CANCEL  COMPLETED
  2 selected: 2 succeeded, 0 skipped, 0 failed, 0 cancelled, 0 deferred
     1  01a1ca37-a9e7-7015-931b-48438b501295  SUCCEEDED
     2  01a1ca37-f9b3-7508-a500-85cd221f57bf  SUCCEEDED
```

and the two runs were `CANCELLED` (`hcx-2026-11-11T10:47:08Z`, `hc-2026-11-11T10:46:48Z`). The job's **cancel** is in section 6 (a job over `mc-` at one item a second, cancelled by hand). The
prefix is customer text, kept in the job and in no read of it (`batch get` and `batch items` do not return it; the batch tests: `batch_business_id_selector::`, `privacy::`).

## 4. Happy path — Signal-with-Start (US04)

```sh
sagat workflow signal-with-start cart --business-id cart-42 --input '{"cartId":"cart-42"}' \
  --signal item-added --payload '{"sku":"A1"}'
sagat workflow signal-with-start cart --business-id cart-42 --input '{"cartId":"cart-42"}' \
  --signal item-added --payload '{"sku":"A2"}'
```

```text
run 01a1ca36-a49e-7510-a4c6-76904914338e was started and signalled: 1 step(s) woke
nothing was signalled: the run is not waiting for that signal (CONFLICT, NOT_WAITING). No step of the run waits for this signal now; signals are never buffered. Trace 2c2863acd85b43b1ba0cb647c34b071b.            (exit 1)
```

The REST call says the same with `started` and `deliveredTo`:

```sh
curl -sS -X POST "$SAGAT_SERVER/runs/signal-with-start" -H "$AUTH" -H 'X-Octopus-Namespace: payments' \
  -H 'Content-Type: application/json' -H 'Idempotency-Key: demo-sws-43' -w '\nHTTP %{http_code}\n' \
  -d "{\"workflowId\":\"$CART\",\"businessId\":\"cart-43\",\"input\":{\"cartId\":\"cart-43\"},\"signal\":{\"name\":\"item-added\",\"payload\":{\"sku\":\"B1\"}}}"
# {"runId":"01a1ca36-a4c8-77b9-857a-71f5093f4835","started":true,"deliveredTo":["wait-item"]}
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
sleep 1      # createdBefore is the whole second `date` gives: a run created in that same second is after it, and the job would not take it
SELECTOR="{\"filter\":{\"workflowId\":\"$CART\",\"status\":[\"WAITING\"],\"createdBefore\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"}}"
sagat batch preview --selector "$SELECTOR"
BATCH=$(sagat batch create --selector "$SELECTOR" --action CANCEL --reason "faulty greeting service, incident 42" --rate 5 | awk '/created/ {print $3}')
sagat batch get $BATCH
sagat batch items $BATCH --limit 20
```

```text
12 run(s) match
  01a1ca3b-dd63-769f-833c-a63b830ae22a
  ...
batch job 01a1ca3c-88cd-7150-a194-f79e53bb55d2  CANCEL  COMPLETED
  12 selected: 12 succeeded, 0 skipped, 0 failed, 0 cancelled, 0 deferred
     1  01a1ca3b-dd63-769f-833c-a63b830ae22a  SUCCEEDED
     ...
```

The preview count equals the matched runs: **whatever `cart` runs are waiting when you run it** (twelve here, because the open runs of the earlier sections had been cancelled by then; on
the first try, with the second schedule of section 3.1 paused and no `sleep 1`, the same selector matched the twelve *older* open runs and none of the twelve new ones, because they had been created in the
same second as `createdBefore`: **DOC FIXED**, the line above is the `sleep`). The job cancels at about the rate asked for (5 a second: 12 in about 2.4 s, so it had completed when read
5 s later; read after one second it said `RUNNING`, `12 selected: 1 succeeded`); every item is `SUCCEEDED` (a run that had already ended would be `SKIPPED` with `ALREADY_TERMINAL`) and the
counters equal the items.

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
run 01a1ca3c-1187-76ac-9593-379ce9a16884 can be reset to:
       1  the start of the run  (2026-11-11T10:51:36.711923Z)
       7  after step compose  (2026-11-11T10:51:37.081132Z)
      11  after step deliver  (2026-11-11T10:51:37.115704Z)
a reset of run 01a1ca3c-… to point 1 would:
  keep nothing: the new run starts from the beginning
  run again: wait-item
  run again: compose (it completed in this run: running it again repeats what it did)
  run again: deliver (it completed in this run: running it again repeats what it did)
  deliver again what the run received after that point, in order:
    signal item-added (event 3): will be tried
  the reset asks you to say you know that: add --acknowledge
reset 01a1ca3c-1da1-74bc-a085-c6d161213d9e: run 01a1ca3c-1d98-70e0-84d1-b184d209df35 was made from point 1 of run 01a1ca3c-1187-76ac-9593-379ce9a16884
  delivered again, in the order the run received them:
    signal item-added (event 3): delivered
```

Expected, and seen: a new run with `rerunOf` the source (`GET /runs/<new>` has `"rerunOf":"01a1ca3c-1187-…"` and the same
business id), whose journal begins with the restored prefix and `run.reset` (`run.started`, `run.reset`, `wait.started`,
`wait.satisfied`…); the preview lists the signal the run received after the point as `will be tried`, and the reset reports it
`delivered` (it would say `rejected`, with the reason, for a signal nobody waited for yet, and `not delivered` for one that is no
longer kept); the steps run again (`compose` and `deliver` here); the source stays terminal. `sagat workflow resets <source>`
lists the resets made from it. Resetting to the end of `compose` (`--sequence 7`) keeps `compose` and `wait-item` as they were
and runs `deliver` again.

**Where the new run says it came from** (new on the second pass). `GET /runs/<new>` carries `resetFrom {runId, sequence}` as well as `rerunOf`, and `sagat workflow inspect` says so in words; the
source, a run nobody reset, has neither key, and `resetFrom` is absent (not `null`) on a plain rerun:

```sh
curl -sS "$SAGAT_SERVER/runs/<new>" -H "$AUTH" -H 'X-Octopus-Namespace: payments' | jq -c '{status, rerunOf, resetFrom, businessId}'
sagat workflow inspect <new> | head -4
```

```text
{"status":"SUCCEEDED","rerunOf":"01a1ca3c-1187-76ac-9593-379ce9a16884","resetFrom":{"runId":"01a1ca3c-1187-76ac-9593-379ce9a16884","sequence":1},"businessId":"reset-1"}
run 01a1ca3c-1d98-70e0-84d1-b184d209df35  SUCCEEDED  workflow 01a1ca35-f0f4-76a2-bb1d-e605534b42dc  version 01a1ca35-f0fc-753e-9294-6a297929ef72
  started 2026-11-11T10:51:39.802834Z by dev-admin, finished 2026-11-11T10:51:40.283722Z
  reset from run 01a1ca3c-1187-76ac-9593-379ce9a16884: it keeps the first 1 event of that run
  trace a9984751fce6410cbd27fe935e323cd5
```

The Studio's run page says `RESET FROM run e9a16884; this run keeps the first 1 event of it` (section 2.5).

Delivering again what the run received after the point is the deployment setting `SAGAT_RESET_REAPPLY_ENABLED` of section 1
([run-reset.md](../../docs/runbooks/run-reset.md) 4.1).

## 6. Failure scenarios (each was demonstrated, or is marked as not demonstrable and why)

"Shown" is what the command gave on the stack of this page; "Test" is the automated evidence, which also covers what a demo cannot
(a connection killed after any statement). Codes are `reason`s of the answer's `details`, as in [contracts/errors.md](contracts/errors.md).

| Scenario | How | Shown | Test |
| --- | --- | --- | --- |
| Engine killed while schedules are due | `docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env kill execution-engine` a second before a minute, then `up -d execution-engine` 20 s later | the time that fell due while the engine was down (23:37:00, killed at 23:36:59, back at 23:37:21) was started once on its return and none was doubled or lost (`catchup-demo`, with `ALLOW_ALL`: 23:36, 23:37 and 23:38 `STARTED`, one run each); `hello-every-minute` (`SKIP`) recorded `SKIPPED_OVERLAP` at 23:38 because its 23:37 run was still open: **the worker had stopped with the engine** (start it again). The crash inside a firing, after each of its statements, is the test's **Second pass (2026-11-11):** `catchup-demo` (`ALLOW_ALL`, a catch-up window of 120 s) was created at 10:52:5x; the engine was killed at 10:52:59 and brought back at 10:53:21: the time that fell due meanwhile, 10:53:00, was started once on the return (`10:53:00 STARTED`, `10:54:00 STARTED`, one run each, none doubled or lost); the worker had stopped with the engine (`the platform refused to hand out tasks (Unavailable): tcp connect error`, G2060 again) and was started again. | `fault_schedules_bulk_operations firing::`, `upgrade::` |
| Outage longer than the catch-up window | a schedule with `"catchupMode":"ONE","catchupWindowMs":120000` (`sagat schedule create --definition FILE`), `docker compose … stop execution-engine` for 5 min 15 s, then `up -d execution-engine` | `23:42:00 STARTED` (the earliest time inside the window), `23:43:00 COALESCED` (`summaryCount: 1`), `23:39:00 MISSED_CATCHUP_WINDOW` (`summaryCount: 3`, last `23:41:00`: the times older than the window, in one summary) **Second pass (2026-11-11):** `catchup-demo`, the engine stopped from 10:54:44 to 11:00:00 (5 min 16 s): `10:59:00 STARTED` (the earliest time inside the window), `11:00:00 COALESCED` (`summaryCount: 1`), `10:55:00 MISSED_CATCHUP_WINDOW` (`summaryCount: 4`, `summaryLastNominalTime` `10:58:00`: the times older than the window, in one summary). | `catchup::` |
| Authority revoked | a member (`dev-dana`, `developer`) creates a schedule; the operator then removes the member (no REST route does: `DELETE FROM memberships …` as the database owner) | next action `AUTHORITY_REVOKED`, schedule `PAUSED` with `note: "the authorizing subject can no longer start runs"`, `noteBy: system`, audit `schedule.auto_paused` by `system:schedule:<id>` ("authority revoked") **Second pass (2026-11-11):** `dev-dana` (developer) created `by-dana` at 11:00:37 and was removed at once with `DELETE FROM memberships …` as the database owner; at 11:01:00 `SCHEDULED AUTHORITY_REVOKED`, the schedule `PAUSED`, `note: "the authorizing subject can no longer start runs"`, `noteBy: system`, audit `schedule.auto_paused` by `system:schedule:<id>` (`authority revoked`). | `authority::` |
| Signal-with-Start on a workflow whose entry does not wait | `sagat workflow signal-with-start hello-world --business-id sws-hello-1 --input '{"name":"x"}' --signal item-added` | `409`, `details.reason` `SIGNAL_NOT_ACCEPTED_AT_START`: "the new run would not accept that signal: nothing was created"; `GET …/business-ids/sws-hello-2` is `404` **Second pass (2026-11-11):** `409 SIGNAL_NOT_ACCEPTED_AT_START` (CLI exit 1: `the new run would not accept that signal: nothing was created`); `GET …/business-ids/sws-hello-1` is `404`. | `with_start::`, `with_start_rest::` |
| Batch cancelled half way | a job at 1 item a second over twelve waiting runs, `sagat batch cancel <id> --reason "stop now please"` after 5 s | the job `CANCELLING`, then `CANCELLED` (by REST, `POST …/cancel` with `{"reason":"stop now please"}`: `12 selected: 5 succeeded, 0 skipped, 0 failed, 7 cancelled`; with the fixed CLI, over 22 waiting runs: `22 selected: 6 succeeded, 0 skipped, 0 failed, 16 cancelled`); the runs it had cancelled stay cancelled. Cancelling again: `409 BATCH_NOT_CANCELLABLE` ("The job already ended."); without `--reason` the CLI says `--reason is required` (exit 2). **The first demo failed here: the CLI sent no body and the platform answered `400 content type must be application/json` (defect, fixed in T018-075)** **Second pass (2026-11-11):** a job over the prefix `mc-` (12 runs) at one item a second, cancelled by `sagat batch cancel <id> --reason "stop now please"` after four seconds: `is CANCELLING: what is in flight finishes, the rest is cancelled`, then `CANCELLED`, `12 selected: 4 succeeded, 0 skipped, 0 failed, 8 cancelled, 0 deferred` (items 1 to 4 `SUCCEEDED`, 5 to 12 `CANCELLED`; the eight runs of the cancelled items were still `WAITING`); a second cancel `409 BATCH_NOT_CANCELLABLE` ("The job already ended."); without `--reason`, `sagat: --reason is required`, exit 2. The fixed CLI works on the stack. | `batch::`, `schedule_and_batch::a_job_is_read_listed_its_items_shown_and_cancelled` |
| Reset to a sequence that is not a point | `sagat workflow reset <run> --sequence 5 --acknowledge --reason "choose a sequence that is not a point"` | `422`, `RESET_POINT_NOT_ELIGIBLE`: "Choose a sequence listed by `listResetPoints`." **Second pass (2026-11-11):** `RESET_POINT_NOT_ELIGIBLE` (CLI exit 1): "Choose a sequence listed by listResetPoints." | `reset::` |
| Reset that delivers again on a deployment without the setting | the control plane started with `SAGAT_RESET_REAPPLY_ENABLED=false`, `--reapply all` | `422`, `RESET_REAPPLY_NOT_AVAILABLE`: "Ask for `reapply: NONE`, or ask the operator of the platform to enable reset reapply."; nothing is made **Second pass (2026-11-11):** the control plane recreated with `SAGAT_RESET_REAPPLY_ENABLED=false`: `RESET_REAPPLY_NOT_AVAILABLE`, and `sagat workflow resets <source>` still lists only the one reset it had (none was made); the switch was turned on again afterwards. | `reapply::` |
| Reset that would deliver more than 256 things | `many-ticks`, a run of 257 sequential waits for `tick`, sent 257 signals (about 11 s, the script is in verification.md), then `reset … --sequence 1 --reapply signals` | `422`, `RESET_REAPPLY_TOO_MANY` ("The run received more than 256 signals and retained Updates after that point; choose a later point or `reapply: NONE`"), for the preview as well; `sagat workflow resets` says no reset was made **Second pass (2026-11-11):** `many-ticks`, 257 signals sent in 11 s (11:02:42 to 11:02:53, then `SUCCEEDED`); `reset-points` lists only the start; the preview and the reset of point 1 with `--reapply signals`: `RESET_REAPPLY_TOO_MANY`; `no reset was made from run …`. | `reapply::the_ceiling_is_the_most_a_reapply_holds_and_one_more_refuses_the_preview_and_the_reset` |
| Reset of a run that has not ended | `sagat workflow reset <run> --sequence 1 --reason "…" --acknowledge --cancel-source` on a waiting `cart` run, with the setting on in the control plane and the engine | without the flag: `409 RESET_SOURCE_NOT_TERMINAL`. With it `202`, `state: PENDING_SOURCE`, `newRunId: null`; the run is `CANCELLED`; within three seconds the engine made the run and `sagat workflow resets` says `was made` (`"state":"CREATED"`) **Second pass (2026-11-11):** without the flag `409 RESET_SOURCE_NOT_TERMINAL` ("Wait for the run to end, or ask for `sourcePolicy: CANCEL_SOURCE`…"); with `--cancel-source` the CLI says `… is being cancelled; the new run is made from point 1 when it has ended, if the point is still one`, and within four seconds `sagat workflow resets` says `was made`; the source was `CANCELLED`. | `cancel_source::`, `cancel_source_rest::an_engine_started_with_the_flag_creates_the_run_of_the_reset_the_api_accepted` |
| Reset of an open run with a step in flight and no acknowledgement | a `cart` run signalled while the worker serves `compose` with `delayMs: 10000` (the most a mock may take), then `sagat workflow reset <run> --sequence 1 --reason "…" --cancel-source` | `422`, `RESET_REEXECUTION_NOT_ACKNOWLEDGED`; the run stays `RUNNING` (not cancelled) and no entry is written **Second pass (2026-11-11):** `RESET_REEXECUTION_NOT_ACKNOWLEDGED`; the run stayed `RUNNING` (not cancelled) and ended `SUCCEEDED` when its ten-second step did. | `cancel_source_rest::a_step_in_flight_needs_the_acknowledgement_and_a_request_without_it_cancels_nothing` |
| Reset of an open run on a deployment without the setting | the control plane with `SAGAT_RESET_CANCEL_SOURCE_ENABLED=false`, `--cancel-source` | `422`, `RESET_CANCEL_SOURCE_NOT_AVAILABLE`: "Wait for the run to end and reset it then, or ask the operator of the platform to enable resets that cancel their source."; the run stays `WAITING`, no reset was made **Second pass (2026-11-11):** the control plane with `SAGAT_RESET_CANCEL_SOURCE_ENABLED=false`: `RESET_CANCEL_SOURCE_NOT_AVAILABLE`; the run stayed `WAITING`, no reset was made. | `cancel_source::` |
| Reset that waited and was killed | kill the connection after any statement of the request or of the engine's completion | NOT DEMONSTRATED: it needs fault injection inside a transaction (a trigger that kills the backend), which only the test has | `fault_schedules_bulk_operations cancel_source` |
| Reset killed while it delivers | kill the connection after any statement of the transaction | NOT DEMONSTRATED: same | `fault_schedules_bulk_operations reset` |
| Pause a trigger-owned schedule before the handover | publish a workflow with `"triggers":[{"kind":"cron","cron":"* * * * *","timezone":"UTC"}]` (its schedule is named `trigger-<id>` and listed with origin `TRIGGER`), `sagat schedule pause <id> --note "test"` | `409`, `SCHEDULE_TRIGGER_HANDOVER_PENDING`: "The legacy cron pass still runs; the operator completes the handover once every engine is upgraded."; the tick keeps firing once per minute; editing its spec is `409 SCHEDULE_TRIGGER_OWNED` ("Change the cron trigger in the workflow definition and publish it.") **Second pass (2026-11-11):** `cron-hello` published at 11:08:10 (its schedule `trigger-<id>`, origin `TRIGGER`): `409 SCHEDULE_TRIGGER_HANDOVER_PENDING`; an edit of its spec `409 SCHEDULE_TRIGGER_OWNED`. | `trigger_schedules::pause_resume_trigger_and_backfill_are_refused_until_the_handover_and_a_spec_edit_always` |
| Hand the cron triggers over | `docker compose … run --rm --no-deps migrate schedule-trigger-handover --operator demo --expect-live 1 --apply` (the service `migrate` has the owner role; the engine runs `SAGAT_SCHEDULES_ENABLED=true`) | `handed over: the cron triggers fire from the schedule pass now (1 live execution engine(s) list schedules-v1, requirement epoch 1). The legacy cron pass of every engine stops within about two seconds. There is no command to go back…`; `schedule_trigger_handover` is `HANDED_OVER` by `demo` as database role `sagat`; the schedule could then be paused (`schedule … is now PAUSED`) and run now (`was triggered: STARTED`), and no tick fired after the pause **Second pass (2026-11-11):** the command with `--operator demo --expect-live 1 --apply` at 11:11:36 (`handed over: … requirement epoch 1 …`); `schedule_trigger_handover` says `HANDED_OVER  demo  2026-11-11 11:11:36.243266+00  sagat`; the schedule was then paused (`is now PAUSED`) and run now (`was triggered: STARTED`), and no tick fired after the pause (no action and no run in the next two minutes). | `trigger_handover::`, `trigger_firing::after_the_handover_the_legacy_pass_starts_nothing_even_when_it_still_runs_and_the_schedule_pass_does` |
| Hand over while a publication is open, or a cron trigger has no schedule | the command while a publication of a cron workflow is uncommitted | NOT DEMONSTRATED: it needs a publication transaction held open for two seconds | `trigger_round3::a_long_publication_makes_the_command_try_again_and_does_not_hold_the_writers_behind_it`, `trigger_round3::the_refusal_for_a_trigger_that_is_not_planned_says_which_trigger_and_what_to_do` |
| Run now, or backfill, a trigger schedule whose trigger is gone | an older control plane drops the cron trigger | NOT DEMONSTRATED: there is no older control plane in the stack | `trigger_round3::a_backfill_or_a_run_now_of_a_schedule_whose_trigger_is_gone_is_refused_with_a_documented_reason` |
| Hand over with an engine that does not run the schedule pass | a second engine started with `docker compose … run -d --no-deps --name … -e SAGAT_SCHEDULES_ENABLED=false execution-engine`, then the command with `--expect-live 2 --dry-run` | `error: 1 live execution engine(s) do not list schedules-v1 (instance ids: 934c24b5-74af-4ba1-8fae-872e5715d26f). An engine lists it only when it runs the schedule pass: start it with SAGAT_SCHEDULES_ENABLED=true (or stop it), wait for it to register, and run the command again. Nothing was changed.`; after the second engine was stopped the dry run said `dry run: nothing stops the handover` **Second pass (2026-11-11):** a second engine (`run -d --no-deps --name … -e SAGAT_SCHEDULES_ENABLED=false`) and `--expect-live 2 --dry-run`: `error: 1 live execution engine(s) do not list schedules-v1 (instance ids: 3f08c90d-…)… Nothing was changed.` (exit 2); with that engine removed (`docker rm -f` of that one container) the dry run with `--expect-live 1`: `dry run: nothing stops the handover (1 live execution engine(s), all listing schedules-v1, and this database role may run it). Nothing was changed.` | `trigger_handover::the_handover_refuses_while_an_engine_lacks_the_capability_and_names_it` |
| Rolling upgrade on one tick | the legacy pass and the schedule pass reach one tick, in either order, one killed after each write | PARTLY: before the handover both passes ran on every tick, and every tick of `cron-hello` has one run (`startedBy: system:engine`) and one action `ALREADY_ACTED`; the kills after each write are the test's **Second pass (2026-11-11):** before the handover both passes reached each tick and every tick of `cron-hello` has one run and one action: `11:09:00` `STARTED` by the schedule pass, `11:10:00` and `11:11:00` `ALREADY_ACTED` (the legacy pass had started the run); the kills after each write are still the test's. | `fault_schedules_bulk_operations upgrade::*` |
| Quota at zero | `PUT /namespaces/$NS/quotas/schedules` with `{"limit":0}`, then create a schedule | `429`, `QUOTA_EXCEEDED`, `details.reason` `SCHEDULE_LIMIT` ("schedules in the namespace"); the existing schedule fired at the next minute; `DELETE …/quotas/schedules` put the default back **Second pass (2026-11-11):** `PUT …/quotas/schedules` with `{"limit":0}` answered `{"resource":"schedules","limit":0,"source":"namespace","ceiling":200,"used":4}`; `sagat schedule create` then `429 QUOTA_EXCEEDED`, `details` `{"limit":"schedules in the namespace","reason":"SCHEDULE_LIMIT"}`; `DELETE` put the default (200) back and the create worked. | `limits::` |
| A schedule's action whose business id is held (new) | run now twice in one second on `minute-cart` (policy `FAIL`) and on `hourly-cart` (`USE_EXISTING`): section 3.1 | `BUSINESS_ID_CONFLICT` with the reason `BUSINESS_ID_IN_USE`, no run (the run list under the prefix kept only the first run); `ALREADY_ACTED` naming the run that holds the id; the schedule stayed `ACTIVE` | `scheduled_business_ids::`, `fault … firing::` |
| A batch selection that keeps failing, `SELECTION_FAILED` (new) | in this demo's database only, as its owner: `CREATE FUNCTION … PERFORM pg_sleep(30)` and `CREATE TRIGGER … BEFORE INSERT ON batch_items`, so that the insert of the round runs into the round's own ten-second statement timeout (a real `57014`); then `sagat batch create --business-id-prefix cs- --action CANCEL --reason "demo: a selection that keeps failing"`; the trigger and the function are dropped at the end | The job stayed `SELECTING` while its rounds failed, one about every 10 s plus the wait: failures 1, 2 and 3 on record at 11:03:10, 11:03:25 and 11:03:45; the wait is the database's `next_slot_at`, 5, 10 and 20 s (`19.5 s` ahead right after the third); `sagat batch get` says `SELECTING` and `0 selected`; the REST read has no count, no cause and no selector; the engine log has one `WARN` for each failure carrying **the job id, `failures` and `sqlstate: 57014` and nothing else** (a search of the whole engine log for the prefix found no line); `batch_selection_failures_total{outcome="retry"}` was 3 on the engine's metrics port, 4 after the fourth. **The 25 minutes of the real backoff were not waited for:** the owner set `selection_failures` to 10 (`UPDATE batch_jobs …`), and the next failed round ended the job `FAILED` with `SELECTION_FAILED` (`failures` 11, `finished_at` set, audit `batch.finished` by `system:engine` with the reason `SELECTION_FAILED`, the line `a batch job failed: its selection rounds kept failing`, `{outcome="failed"} 1`; `sagat batch get`: `failed: SELECTION_FAILED`). A second job over the failing selection, **cancelled while it waited**, went `CANCELLING` and `CANCELLED` within two seconds (the cancel does not wait for the backoff); with the trigger dropped, a job over the same prefix completed (`2 selected: 1 succeeded, 1 skipped`) | `batch_selection_failures::`, `fault … batch_selection::` |
| The alert for a job that ended `SELECTION_FAILED`, on the stack's Prometheus (new) | `SagatBatchSelectionFailed`: `sum(increase(batch_selection_failures_total{outcome="failed"}[15m])) > 0`; Prometheus is the repository's image, `prometheus.yml` and `alerts.yaml` (section 1) | Prometheus loaded the 34 rules with no error (`health: ok`), the two alerts of the feature among them. **First run: the alert stayed `inactive` after a job had ended `SELECTION_FAILED`**: the series `{outcome="failed"}` did not exist before the job and appeared with the value 1 (its samples `1,1,1,1,1,1`), which `increase` does not count (`sum(increase(…))` was 0), so the alert would have fired for the second job an engine ended and never for the first (**FAILED, then DEFECT FIXED**: G2102). After the fix (the two series are created at zero when an executor is built) and the engine rebuilt from the tree: before any job `batch_selection_failures_total{outcome="retry"} 0` and `{outcome="failed"} 0` on the engine's port; after a job ended `SELECTION_FAILED` the `increase` was 1.02 and the alert was **`firing`**. The same run showed that the job's `finishedAt` had been the instant its last round began, up to ten seconds before the end (**DEFECT FIXED**: G2103; after the fix `finished_at` 11:18:45.932 and the log line 11:18:45.936) | `batch_selection_failures::the_two_series_of_the_selection_counter_exist_at_zero_before_the_first_failure`, `batch_selection_failures::a_job_that_fails_its_selection_is_finished_when_…`, `infra/observability/tests/batch_stalled_test.yaml` |

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
