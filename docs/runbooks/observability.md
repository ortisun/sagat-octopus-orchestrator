# Runbook: observability (feature 008)

For the people who run the platform. It covers what the platform exports, how to see it, the conditions of the platform itself (not of a namespace: those are in [`alerts.md`](alerts.md)), and what to do when the telemetry is the thing that is wrong. Design: [ADR-0017](../adr/ADR-0017-telemetry-is-derived-from-the-journal-and-conditions-are-found-by-looking.md); decisions: [`specs/008-observability-operations/research.md`](../../specs/008-observability-operations/research.md).

## 1. What there is

| What | Where | For |
|---|---|---|
| **Traces** | OTLP/HTTP (JSON) to a Collector | One trace from the request that started a run, through the scheduler and the worker, to the provider the worker called |
| **Metrics** | `/metrics` of each process (admin listener, `SAGAT_ADMIN_ADDR`) | Rates, latencies, queues, leases, dead items, compensation, quotas |
| **Logs** | standard output, JSON | Events with `trace_id`, `run_id`, `tenant_id` when they happen inside a span that has them |
| **Timeline** | `GET /runs/{id}/events`, the Studio | Every transition of a run with its time, its cause and its trace |
| **Alerts** | `GET /namespaces/{id}/alerts`, webhooks, `/metrics` | Conditions that need a person, per namespace ([`alerts.md`](alerts.md)) |
| **Dashboards and rules** | `infra/observability/` | Grafana and Prometheus for the whole platform |

Locally: `docker compose -f infra/docker/compose.yaml -f infra/docker/compose.observability.yaml --env-file infra/docker/.env up -d --build` starts a Collector, Jaeger (`http://127.0.0.1:16686`), Prometheus (`:19091`) and Grafana (`:13000`).

## 2. Turning traces on

Nothing is exported until an endpoint is set, with the standard variables, on the control plane, the engine and the runner:

| Variable | Meaning |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Base URL (`/v1/traces` is added) or the full URL. Setting it turns the exporter on. |
| `OTEL_EXPORTER_OTLP_HEADERS` | `name=value,…`, for a backend's API key. Never logged. |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | Only `http/json` is spoken. Anything else is a **startup error**, not a silent fallback (G169). |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | Milliseconds, 100 to 60000. |
| `OTEL_SERVICE_NAME` | Defaults to `sagat-control-plane`, `sagat-execution-engine` or `sagat-sandbox-runner`. |
| `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG` | `always_on` (default), `always_off`, `traceidratio` with a ratio from 0 to 1. Decided from the trace id, so every process agrees. |
| `OTEL_SDK_DISABLED` | `true` turns it off whatever else is set. |
| `SAGAT_OTLP_QUEUE_CAPACITY` | Live spans waiting for the exporter (16 to 65536, default 2048). A full queue drops the span and counts it. |
| `SAGAT_TELEMETRY_POLL_MS`, `SAGAT_TELEMETRY_BATCH_RUNS`, `SAGAT_TELEMETRY_LAG_BUDGET_SECS` | The projector (section 4). |

The spans, and what hangs from what:

```
http.request (control plane)            the request that started the run; a child of the caller's span if it sent one
└─ workflow.run                          the run, from its creation to its end (exported when it ends)
   ├─ scheduler.action                   what the scheduler did to the run (a retry released, a lease taken back)
   ├─ outbox.publish                     the hint that a step is ready
   ├─ step.attempt  (one per attempt)    leased → ended; the span the worker is given in its lease
   │  └─ … the worker's spans and its calls to a provider, if it continues the lease's traceparent
   ├─ timer, wait.event                  where the time goes when no worker holds the run
   └─ webhook.delivery                   telling a subscriber the run finished; it sends a traceparent
```

Attributes are `sagat.tenant.id`, `sagat.namespace.id`, `sagat.run.id`, `sagat.workflow`, `sagat.workflow.version`, `sagat.node`, `sagat.attempt`, `sagat.generation`, `sagat.outcome`, `sagat.failure.code`, `sagat.schedule_latency_ms`, `sagat.run.status`. A person's subject is never an attribute: an operator's action is a span event that says only whether a person or the platform did it.

## 3. Metrics

Labels are closed sets; none is a tenant, a namespace, a run, a node or an activity. The ones the dashboard draws:

`runs_terminal_total{status}`, `run_duration_seconds`, `runs_active{status}`, `step_attempts_total{outcome}`, `step_attempt_duration_seconds{outcome}`, `step_schedule_latency_seconds`, `ready_steps`, `queue_lag_seconds`, `leases_active`, `leases_expired`, `outbox_backlog`, `outbox_oldest_age_seconds`, `inbox_pending_events`, `inbox_oldest_age_seconds`, `dead_items{kind}`, `compensating_runs`, `workers_registered`, `alerts_firing{kind}`, `alerts_fired_total{kind}`, `scheduler_lag_seconds{pass}`, `scheduler_shards_owned`, `scheduler_replicas`, `scheduler_fenced_total{kind}`, `pool_queue_lag_seconds{pool}`, `pool_ready_steps{pool}` (feature 010: [`scaling.md`](scaling.md)), `telemetry_projection_lag_seconds`, `telemetry_projection_pending_runs`, `telemetry_projector_active`, `telemetry_spans_exported_total{outcome}`, `otel_spans_total{outcome}`, plus the request, gRPC, saga, event and sandbox series of the earlier features (see each runbook).

`runs_terminal_total`, `step_attempts_total` and the duration histograms are made by the **projector**, from the journal, in the process that holds its lease (section 4): they are the same whatever order requests arrived in, and they lag the transitions by `telemetry_projection_lag_seconds`. The gauges (`ready_steps`, `leases_*`, `outbox_*`, `dead_items`, …) are sampled by the **monitor** every `SAGAT_MONITOR_INTERVAL_SECS`.

The Prometheus rules (`infra/observability/alerts.yaml`) are about the whole deployment; each has a `runbook` annotation to a section here or in [`alerts.md`](alerts.md). `promtool check rules` passes on them and a test holds every expression to a metric the code really emits.

## 4. The telemetry projection

The write path notes, in the transaction that changes a run, how far its journal has got (`telemetry_pending`). **One projector** (a lease row in `engine_leases`, 30 s) reads the notes, derives the spans and measurements of what is new, hands the spans to the collector and, only when it has them, records the run's progress. Nothing it does takes a lock on a run.

- `telemetry_projection_pending_runs` and `telemetry_projection_lag_seconds`: how many runs have changes not projected and how old the oldest change is.
- A collector that is down: the spans wait in the journal (`telemetry_projection_errors_total{reason="collector"}`), the lag grows, nothing is lost. When it is back the projector catches up.
- The projector that was working dies: another takes over when the lease runs out; spans exported but not recorded are exported again with the same ids.
- Without an endpoint the measurements are still made and no span is exported.
- A note that claims more journal than there is (a restore from a backup, a journal trimmed by a retention policy) is settled to what it claimed, with a warning and `telemetry_projection_errors_total{reason="inconsistent"}`; a run that is *still* behind after that is behind for another reason.

```sql
-- runs that have changes not projected, oldest first
SELECT run_id, head_seq, applied_seq, now() - behind_since AS behind FROM telemetry_pending WHERE applied_seq < head_seq ORDER BY behind_since LIMIT 20;
```

## 5. Operating a run, and the workers that serve it

Every action is a command that the platform checks, journals and audits; none edits state. They go through `POST /runs/{id}/operations` (`run:operate`; Operator and Admin) unless said otherwise, and each needs a `reason` of 8 to 2000 characters, which is written to the audit trail.

| Action | When it applies | What it does |
|---|---|---|
| `RETRY_STEP` | The step is waiting for a decision after a failure, **or is waiting for its next automatic retry** (`RETRY_WAIT`) | A new attempt now. The attempts before it stay in the journal, with their outcome and their trace. If the scheduler's own retry commits first, the operator's request gets `409`: only one attempt starts |
| `MARK_MANUAL_REVIEW` | A step that is `READY` or in `RETRY_WAIT`, and not a lookup | Holds it for a person (code `MARKED_FOR_REVIEW`); it is then decided like any other review (`RETRY_STEP`, skip, abort). Nothing runs on it until then |
| `RERUN_AS_NEW` | The run has **ended** | A new run of the same version with the same input; it says where it came from (`rerunOf`). Idempotent by the request's key. A run that is still going is refused: cancel it first |
| `PAUSE`, `RESUME`, `CANCEL`, `SIGNAL` | As before (feature 002) | |

**Draining a worker** (`POST /namespaces/{id}/workers/{subject}/drain`, `worker:manage`; undo with `…/undrain`) stops an identity from being given new attempts in that namespace. What it already holds goes on until it ends; it can still register and heartbeat, so a worker that restarts stays drained. `GET /namespaces/{id}/workers` shows `draining` and `inFlight`: when `inFlight` is 0 it is safe to stop the process. Use it before a deploy, or to take a bad version out of rotation without cancelling anything. It is by **identity and namespace**: two processes that share an identity are drained together (G184).

## 6. Reading the audit trail

`GET /audit` (Admin) returns the tenant's records, newest first, narrowed by `action`, `resourceType`, `resourceId`, `actor`, `from`, `to`; pages by cursor.

- **Reading it is recorded** (`audit.read`, with the filters). The same reading by the same person within five minutes is recorded once; after that it is a new reading and is on the trail again.
- A reason that has the shape of a credential (a token, `Bearer …`, the password in a URL, a key with a well-known prefix) is replaced on the way out; the trail keeps what was written.
- **Denied attempts to reach another tenant's resource (`access.denied.cross_tenant`) are not returned.** They are written only when the identifier exists elsewhere, so listing them would answer "does this exist?" for whoever could read them. They stay in the table for you:

```sql
SELECT created_at, actor, resource_type, resource_id, trace_id FROM audit_records
WHERE action = 'access.denied.cross_tenant' ORDER BY created_at DESC LIMIT 100;
```

## engine-down

**`SagatEngineDown`.** Prometheus cannot scrape any execution engine. Nothing schedules: leases are not taken back, retries and timers do not fire, the outbox and the telemetry are not moving. The state of every run is safe in the journal; it resumes when an engine runs. Check the process (`/livez`), the database, the orchestrator's restarts.

## telemetry-behind

**`SagatTelemetryBehind`, `SagatNoProjector`.** The spans and the measurements derived from the journal are more than a minute behind, or no engine holds the projector's lease. This delays traces and the series made from them; the runs themselves are not affected.

- **Confirm:** `telemetry_projection_lag_seconds`, `telemetry_projection_pending_runs`, `telemetry_projector_active` (1 on the engine that holds the lease), the SQL above.
- **Likely causes:** the collector is down or slow (`telemetry_projection_errors_total{reason="collector"}`); the database is slow; no engine is running; a burst of runs larger than one projector takes in a round (`SAGAT_TELEMETRY_BATCH_RUNS`, default 50 per second).
- **Do:** bring the collector back; raise `SAGAT_TELEMETRY_BATCH_RUNS` if it is a burst; start an engine. The projector catches up by itself.

## collector-refusing

**`SagatCollectorRefusing`, `SagatSpansDropped`.** The collector answers with an error that sending the same batch again would not change (`telemetry_spans_exported_total{outcome="rejected"}`), or live spans are dropped because the queue is full or the collector cannot be reached (`otel_spans_total{outcome="dropped"|"lost"}`). Spans of the journal are not lost to an outage (section 4), but a batch that is **refused** is dropped.

- Check the collector's log for why it refuses (a request too large, a malformed attribute, an authentication header missing: `OTEL_EXPORTER_OTLP_HEADERS`).
- A collector behind a gateway that wants `http/protobuf` cannot take this exporter (G169): put a Collector with an `otlp` receiver in front.

## monitor-blind

**`SagatMonitorBlind`.** The monitor cannot run the query for some kind of alert (`alert_checks_failed_total{kind}`); the alerts of that kind stay as they were, neither raised nor resolved. Look at the engine's log (`could not look for an alert condition`), the database's load and its statement timeout.
