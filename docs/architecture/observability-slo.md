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
- **Alerts.** Deployment-wide Prometheus rules declared in [alerts.yaml](../../infra/observability/alerts.yaml), including the later shard ownership, scheduler lag/fencing, keeper, pool backlog and fairness rules and nine conditions the engine finds itself per namespace (`docs/runbooks/alerts.md`): outbox stuck, lease not taken back, step nobody serves, review waiting, dead items, and the four flow-control alerts of feature 022 (below). Not built: replication lag, disk and memory (the platform does not see them), webhook signature spikes, tenant skew.
- **SLOs.** None of the proposed targets is measured as an SLO. `tests/performance/reports/observability-operations.md` measures what observability costs the paths it watches (the note a writer pays, the projector's rate and lag, the monitor's rounds), on one machine.
- **Runbooks.** [`observability.md`](../runbooks/observability.md) and [`alerts.md`](../runbooks/alerts.md); the five scenarios above are in [`sagas.md`](../runbooks/sagas.md), [`events.md`](../runbooks/events.md) and the control-plane and engine runbooks.

## Flow control: concurrency keys and rate limits (feature 022)

The closed vocabulary is `crates/telemetry/src/flow_control.rs` (FR-022-033, FR-022-049; plan "Observability" and
"Observability (rate limits)" of [feature 022](../../specs/022-flow-control-step-policies/plan.md)). No label takes a
tenant, a namespace, a run, a workflow, a group or a key: those are on spans and in the journal only. A span names a
group (`sagat.concurrency.group`, `sagat.rate_limit.group`), never a key's value or its label.

| Series | Kind | Labels | What it counts |
|---|---|---|---|
| `concurrency_acquisitions_total` | counter | `scope` (`run`, `step`); `outcome` (`acquired`, `queued`, `refused_full`, `refused_reset`, `granted`, `skipped_ahead`, `timed_out`, `dequeued`) | each decision of a permit, after its commit |
| `concurrency_wait_seconds` | histogram | `scope` | how long a waiter granted from the queue waited (a timeout or a dequeue is counted by its outcome only) |
| `concurrency_waiters` | gauge | `scope` | waiters of every key, deployment-wide |
| `rate_limit_decisions_total` | counter | `scope`; `outcome` (`granted`, `queued`, `granted_from_queue`, `refused_full`, `refused_reset`, `skipped_ahead`, `timed_out`, `dequeued`, `slot_returned`) | each decision of a slot, after its commit |
| `rate_limit_wait_seconds` | histogram | `scope` | how long a waiter granted from the queue waited (a timeout or a dequeue is counted by its outcome only) |
| `rate_limit_waiters` | gauge | `scope` | waiters of every key, deployment-wide |
| `rate_limit_grant_lag_seconds` | histogram | none | from the instant a slot was free with a waiter present to the grant |
| `rate_limit_unleased_grants` | gauge | none | step grants not leased yet |
| `scheduler_actions_total{kind="gate_grant"}`, `{kind="gate_timeout"}`, `{kind="rate_lease"}`, `{kind="rate_purge"}` | counter | `kind`, `outcome` | the grant pass, the timeout routine, the lease recording and the purge of old grants (existing family; outcomes in the [flow-control runbook](../runbooks/flow-control.md), section 15) |

Spans: `workflow.concurrency.acquire`, `workflow.concurrency.grant`, `workflow.rate_limit.admit`,
`workflow.rate_limit.grant`.

As built on this tree (decisions D022-045 and D022-083; gap G022-001 closed for the telemetry): every committed outcome
of a step's gates and of a run's start is counted at its scope (`step`, `run`), so are the refusals of a run's start
(`refused_full`, `refused_reset`), and the four spans name the group, never the key. The two wait histograms observe each waiter the
grant routine grants, from its queueing to its grant, after the commit; a timeout or a dequeue is counted by its outcome
only. The grant lag is measured from the key's next-eligible hint as the pass selected it (or the waiter's queueing if
later) to the grant: the hint is early and never late, so the figure is an upper bound of the true lag. The two waiter
gauges and `rate_limit_unleased_grants` are set by the engine's monitor every look, deployment-wide, from the key rows'
counters. Evidence: `crates/telemetry/tests/flow_control_vocabulary.rs` (an unknown label or value is refused);
`readiness_gate::tests::{each_committed_gate_outcome_is_counted_in_the_closed_vocabulary,
a_refused_start_is_counted_and_typed, a_grant_from_the_queue_records_its_wait_and_a_slot_its_lag}` in
`crates/infra-postgres`. Task T022-082, which builds them, is not ticked until its gates and review are recorded.

Alerts, found per namespace by the engine's monitor and listed with the others in the
[alerts runbook](../runbooks/alerts.md), one per namespace and group, the group the subject:

| Alert | Severity | Condition | Threshold |
|---|---|---|---|
| `CONCURRENCY_QUEUE_STUCK` | warning | a key with no room for its oldest eligible waiter, which queued more than the threshold before the key's oldest current holder took its permit | `SAGAT_ALERT_UNCLAIMED_SECS` (300) |
| `RATE_LIMIT_GRANT_LAGGING` | critical | a key with waiters, due for longer than the threshold, and no grant of it in that time | `SAGAT_ALERT_LEASE_GRACE_SECS` (60) |
| `RATE_LIMIT_QUEUE_SATURATED` | warning | a run-scope key whose waiters are above 80% of the namespace's effective `flow_control_key_waiters` | 80%, no setting |
| `FLOW_CONTROL_WAITER_ORPHANED` | critical | a waiter row of a run that ended (an invariant breach) | none: one is the condition |

The conditions are read in `crates/infra-postgres/src/operations.rs` and proved by
`integration_flow_control_step_policies::alerts::*` (each raised and resolved). What they mean and what to do: the
[flow-control runbook](../runbooks/flow-control.md), sections 8 to 10 and 14. A database clock set back pauses grants
without any alert (gap G1183; the runbook's section 12 gives the query).

## Dormant leaf source certification observations (feature014)

Operational questions: did this returned native attempt certify, replay or refuse; did storage fail
or leave COMMIT uncertain; how long did the attempt take; which qualified original source does its
trace identify? These observations describe the native metadata transaction, not workflow attempt
history. Journal-derived workflow spans remain unchanged.

The dormant port emits `workflow.collection.leaf.certify` under the caller's trace, with
`sagat.tenant.id`, `sagat.namespace.id`, `sagat.run.id`, `sagat.collection.owner_node`,
`sagat.collection.index` and `sagat.outcome`. Error status is fixed for storage/Unknown, without an
exported error description. Direct calls without caller trace still emit metrics. Filtering this
child span cannot write an outcome/status onto the caller's span.

`workflow_collection_leaf_certification_attempts_total{outcome}` and
`workflow_collection_leaf_certification_duration_seconds{outcome}` count/time returned attempts.
The closed outcomes are `certified`, `replayed`, `refused`, `storage_error`, `unknown`. No tenant,
source, authority or error identifier is a metric label. Cancellation returns no outcome and is
not counted. `workflow_collection_leaf_certified_total` has no labels and counts only acknowledged
first commits. A server commit whose reply was lost is not inferred as a confirmed transition;
its reconciliation replay does not increment that counter. The immutable canonical table and
atomic audit record are authoritative, including when process metrics reset or miss Unknown.

The fixed debug completion event has only the closed `outcome`; no payload, digest, audit ID,
scheduler owner/generation or raw driver diagnostic is included. Released INSERT and a production
caller remain gated; these observations grant no source preservation or nested execution authority.
