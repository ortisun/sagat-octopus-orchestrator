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
