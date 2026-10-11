# Observability, SLOs and operations

## OTel telemetry

- Instrument HTTP/gRPC ingress, DB transaction, lease, outbox publish, NATS delivery, step attempt, external HTTP, webhook delivery, saga compensation.
- Attributes: `tenant.id` (hash/internal code depending on cardinality protection), `workflow.id`, `workflow.version`, `run.id`, `step.id`, `attempt`, `worker.pool`, `queue`, `connector.type`, `trace_id`.
- Avoid high-cardinality IDs in metrics; they stay in logs/spans/exemplars. JSON logs with redaction.

## Priority metrics

`runs_started_total`, `runs_terminal_total{status}`, `run_duration_seconds`, `step_attempts_total{outcome}`, `step_schedule_latency_seconds`, `queue_lag_seconds`, `leases_expired_total`, `outbox_pending`, `reconciliation_repairs_total`, `compensation_backlog`, `webhook_delivery_failures_total`, `sandbox_oom_total`, `tenant_quota_rejections_total`, `journal_write_duration_seconds`.

## Proposed initial SLOs (validation in reference environment)

- Durability: no confirmed Run lost in fault injection scenarios with storage preserved; a functional correctness goal, not an availability percentage.
- Admission API: p95 < 250 ms without inline builds, in a reference scenario.
- Queueing start→ready: p95 < 500 ms with a queue without backlog and baseline health.
- Worker pickup READY→RUNNING: p95 < 1 s with available workers and queue within provisioning.
- Recovery after worker failure: detection and requeue within 2x lease TTL + the configured reconciliation cycle.
- Targets are conditional on capacity, hardware, load and integration coverage; they are not production guarantees.

## Alerts

Journal write errors, pool starvation, queue backlog, stuck leases, outbox age, DLQ growth, compensation_failed, webhook signature spikes, sandbox escape indicators, tenant skew/fairness, memory, disk, replication lag.

## Runbooks

1. DB unavailable → pause admission/execution; never confirm without commit.
2. NATS unavailable → record in outbox, retry/backoff, reconcile after it returns.
3. Worker crash loop → quarantine pool/version, drain, review artifacts/quotas.
4. Compensation failed → incident, block unsafe automation, audited manual resolution.
5. Payload inconsistency → fail closed, preserve evidence, block dangerous retries.

## As built (feature 008)

What the sections above propose is partly built; this says which part, and where the names differ.

- **Traces.** Not produced on the write path: the write path leaves one note per run and a projector derives the spans from the journal ([ADR-0017](../adr/ADR-0017-telemetry-is-derived-from-the-journal-and-conditions-are-found-by-looking.md)). One trace follows a request through admission, the scheduler, the worker and the provider it calls; the live spans (HTTP, gRPC, scheduler action, outbox publish, webhook delivery) are exported as they end, the run and its attempts when the journal says they ended. Export is OTLP/HTTP with JSON only (G169).
- **Attributes.** `sagat.tenant.id`, `sagat.namespace.id`, `sagat.run.id`, `sagat.workflow`, `sagat.workflow.version`, `sagat.node`, `sagat.attempt`, `sagat.outcome`, `sagat.failure.code`. A person's subject is never one. Identifiers are on spans and logs, never on metrics.
- **Metrics, as named in the code.** `runs_terminal_total{status}`, `run_duration_seconds`, `step_attempts_total{outcome}`, `step_attempt_duration_seconds`, `step_schedule_latency_seconds`, `queue_lag_seconds`, `ready_steps`, `leases_active` / `leases_expired` (gauges), `outbox_backlog`, `outbox_oldest_age_seconds`, `dead_items{kind}`, `compensating_runs`, `alerts_firing{kind}`; the proposals `reconciliation_repairs_total`, `sandbox_oom_total`, `tenant_quota_rejections_total` (as `quota_exceeded_total{limit}`) and `journal_write_duration_seconds` exist under the names of their features, or not at all (the last is not built).
- **Alerts.** Seventeen Prometheus rules for the whole deployment (`infra/observability/alerts.yaml`) and five conditions the engine finds itself per namespace (`docs/runbooks/alerts.md`): outbox stuck, lease not taken back, step nobody serves, review waiting, dead items. Not built: replication lag, disk and memory (the platform does not see them), webhook signature spikes, tenant skew.
- **SLOs.** None of the proposed targets is measured as an SLO. `tests/performance/reports/observability-operations.md` measures what observability costs the paths it watches (the note a writer pays, the projector's rate and lag, the monitor's rounds), on one machine.
- **Runbooks.** [`observability.md`](../runbooks/observability.md) and [`alerts.md`](../runbooks/alerts.md); the five scenarios above are in [`sagas.md`](../runbooks/sagas.md), [`events.md`](../runbooks/events.md) and the control-plane and engine runbooks.
