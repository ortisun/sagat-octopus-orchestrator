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

`runs_terminal_total{status}`, `run_duration_seconds`, `runs_active{status}`, `step_attempts_total{outcome}`, `step_attempt_duration_seconds{outcome}`, `step_schedule_latency_seconds`, `ready_steps`, `queue_lag_seconds`, `leases_active`, `leases_expired`, `outbox_backlog`, `outbox_oldest_age_seconds`, `inbox_pending_events`, `inbox_oldest_age_seconds`, `dead_items{kind}`, `compensating_runs`, `workers_registered`, `alerts_firing{kind}`, `alerts_fired_total{kind}`, `telemetry_projection_lag_seconds`, `telemetry_projection_pending_runs`, `telemetry_projector_active`, `telemetry_spans_exported_total{outcome}`, `otel_spans_total{outcome}`, plus the request, gRPC, saga, event and sandbox series of the earlier features (see each runbook).

`runs_terminal_total`, `step_attempts_total` and the duration histograms are made by the **projector**, from the journal, in the process that holds its lease (section 4): they are the same whatever order requests arrived in, and they lag the transitions by `telemetry_projection_lag_seconds`. The gauges (`ready_steps`, `leases_*`, `outbox_*`, `dead_items`, …) are sampled by the **monitor** every `SAGAT_MONITOR_INTERVAL_SECS`.

The Prometheus rules (`infra/observability/alerts.yaml`) are about the whole deployment; each has a `runbook` annotation to a section here or in [`alerts.md`](alerts.md). `promtool check rules` passes on them and a test holds every expression to a metric the code really emits.

## 4. The telemetry projection

The write path notes, in the transaction that changes a run, how far its journal has got (`telemetry_pending`). **One projector** (a lease row in `engine_leases`, 30 s) reads the notes, derives the spans and measurements of what is new, hands the spans to the collector and, only when it has them, records the run's progress. Nothing it does takes a lock on a run.

- `telemetry_projection_pending_runs` and `telemetry_projection_lag_seconds`: how many runs have changes not projected and how old the oldest change is.
- A collector that is down: the spans wait in the journal (`telemetry_projection_errors_total{reason="collector"}`), the lag grows, nothing is lost. When it is back the projector catches up.
- The projector that was working dies: another takes over when the lease runs out; spans exported but not recorded are exported again with the same ids.
- Without an endpoint the measurements are still made and no span is exported.

```sql
-- runs that have changes not projected, oldest first
SELECT run_id, head_seq, applied_seq, now() - behind_since AS behind FROM telemetry_pending WHERE applied_seq < head_seq ORDER BY behind_since LIMIT 20;
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
