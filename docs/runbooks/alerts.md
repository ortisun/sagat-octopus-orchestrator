# Runbook: operational alerts (feature 008)

For the people who operate a namespace. An **alert** is a condition the platform found that needs a person: it is raised by the engine's monitor, listed by `GET /namespaces/{namespaceId}/alerts` (permission `ops:read`), delivered to the namespace's webhook subscriptions as `ops.alert.fired` and `ops.alert.resolved`, and counted on `/metrics`. Every alert carries a `runbook` that points to a section below. An alert carries counts, ages and names — never data of a run.

Design: [ADR-0017](../adr/ADR-0017-telemetry-is-derived-from-the-journal-and-conditions-are-found-by-looking.md); decisions: [`specs/008-observability-operations/research.md`](../../specs/008-observability-operations/research.md); the engine's own runbook: [`execution-engine.md`](execution-engine.md); traces and dashboards: [`observability.md`](observability.md).

## How alerts work

- One **monitor** looks every `SAGAT_MONITOR_INTERVAL_SECS` (30). One monitor at a time works (a lease row, `engine_leases`); a second engine waits and takes over when the lease runs out (60 s).
- A condition has a **threshold**: how long a thing may wait before it is a condition. The defaults are several times what a healthy deployment needs, so an alert is a condition and not a blip. They are set with `SAGAT_ALERT_OUTBOX_AGE_SECS` (120), `SAGAT_ALERT_LEASE_GRACE_SECS` (60), `SAGAT_ALERT_UNCLAIMED_SECS` (300) and `SAGAT_ALERT_REVIEW_SECS` (900). The three flow-control alerts of feature 022 reuse two of them: `RATE_LIMIT_GRANT_LAGGING` the lease grace (the scheduler has not done what a tick does) and `CONCURRENCY_QUEUE_STUCK` the unclaimed threshold (work waits while other work is served); `RATE_LIMIT_QUEUE_SATURATED` is a share of a ceiling (80%), not a time.
- An alert is one row per namespace, kind and subject. It is `FIRING` while the condition is seen and `RESOLVED` when it is not; coming back is the same row with `firedCount` one more. Resolved alerts are kept `SAGAT_ALERT_RETENTION_DAYS` (30).
- **A look that fails resolves nothing.** If the monitor cannot run the query for a kind (`alert_checks_failed_total{kind}` rises), the alerts of that kind stay as they were.
- At most 25 alerts of one kind per namespace per look (the oldest): a namespace with a thousand unserved activities has 25 alerts, and the rest are in the gauges.
- A change of state and the obligation to tell the subscribers are written in one transaction, so a subscriber is told once per change however many engines are running. Subscribe with `events: ["ops.alert.fired", "ops.alert.resolved"]` on `POST /namespaces/{namespaceId}/webhook-subscriptions`.
- `SAGAT_RUNBOOK_BASE_URL` (default `docs/runbooks/alerts.md`) is what the `runbook` field starts with, so it can point to wherever you keep these pages.

## outbox-stuck

**`OUTBOX_STUCK`, critical.** Messages of the transactional outbox have waited longer than `SAGAT_ALERT_OUTBOX_AGE_SECS`. The outbox carries the "a step is ready" hints to workers and the `run.finished` events to subscribers. The state of every run is safe (it is in the journal); what is late is the telling.

- **Confirm:** `outbox_oldest_age_seconds` and `outbox_backlog` on the engine's `/metrics`; `outbox_publish_failed_total{code}` says why. In SQL: `SELECT event_type, count(*), min(created_at) FROM event_outbox WHERE delivered_at IS NULL GROUP BY 1;`
- **Likely causes:** NATS is down or refuses the engine (`code` `BROKER_UNAVAILABLE`); `SAGAT_NATS_URL` is not set (the engine says so at start and the outbox only grows; workers then have to poll); the engine is not running.
- **Do:** bring NATS back, or set `SAGAT_NATS_URL`; start the engine. Nothing is lost: the dispatcher publishes what waits, with the same message ids, and the broker drops duplicates. Workers poll anyway, so runs go on at the pace of the polling while the hints are late.
- **Over when:** `outbox_oldest_age_seconds` is under the threshold; the alert resolves at the next look.

## lease-not-reaped

**`LEASE_NOT_REAPED`, critical.** An attempt's lease ran out more than `SAGAT_ALERT_LEASE_GRACE_SECS` ago and nobody took the attempt back. A healthy scheduler does it within a tick (half a second). The attempt of a step whose worker died is waiting for the scheduler that would retry it.

- **Confirm:** `leases_expired` (the number right now) on `/metrics`; the engine's `/livez` fails when the scheduler stops ticking. In SQL: `SELECT namespace_id, count(*), min(lease_expires_at) FROM steps WHERE status IN ('LEASED','RUNNING') AND lease_expires_at < now() GROUP BY 1;`
- **Likely causes:** no engine is running; the engine cannot reach the database (`db_pool_connections`, the `scheduler_actions_total{outcome="failed"}` counter); the scheduler loop is stuck (the orchestrator restarts a process whose `/livez` fails).
- **Do:** make an engine run and reach the database. The scheduler takes the attempts back by itself, retries them under the step's policy and the journal keeps every attempt.
- **Over when:** `leases_expired` is 0.

## step-unclaimed

**`STEP_UNCLAIMED`, warning.** Steps have been `READY` for longer than `SAGAT_ALERT_UNCLAIMED_SECS` and no worker takes them. The **subject** is the activity name.

- **Confirm:** `ready_steps` and `queue_lag_seconds`; `workers_registered`; the namespace's worker bindings (`GET /namespaces/{id}/worker-bindings`).
- **Likely causes:** no worker serves that activity (a deploy removed it, a typo in the name); the worker's identity is not bound to the namespace or the activity (`FORBIDDEN` in the worker's log); the workers are all busy (`max_inflight`); the workers are draining.
- **Do:** start or fix the worker pool; bind it; scale it. The step waits without cost and starts when a worker takes it.
- **Over when:** the step is leased (the alert resolves) or the run is cancelled.

## review-pending

**`REVIEW_PENDING`, warning.** Runs have waited for a person (`MANUAL_REVIEW`) longer than `SAGAT_ALERT_REVIEW_SECS`. A saga could not decide by itself whether an effect happened or could not be undone.

- **Confirm:** `GET /runs?status=MANUAL_REVIEW`; the runtime graph shows the step that waits and why.
- **Do:** decide as [`sagas.md`](sagas.md) section 3 describes: retry, skip a compensation or declare the step done or not done, each with a reason that goes to the audit trail.
- **Over when:** no run of the namespace is in `MANUAL_REVIEW`.

## dead-items

**`DEAD_ITEMS`, warning.** Things ran out of attempts and wait to be redriven or discarded. The **subject** says which: `events` (admitted events the dispatcher could not process), `webhooks` (deliveries to a subscriber that kept failing) or `connectors` (broker messages that could not become events).

- **Confirm:** `dead_items{kind}`; `GET /namespaces/{id}/events/…`, `…/webhook-subscriptions/{id}/deliveries?status=DEAD`, `…/connectors/{id}/dead-letters`.
- **Do:** find out why (the receiver's answer is on every delivery attempt; a dead letter has its reason), fix it, then redrive (`POST …/redrive`) or discard. [`events.md`](events.md) describes each.
- **Over when:** the namespace has none of that kind.

## concurrency-queue-stuck

**`CONCURRENCY_QUEUE_STUCK`, warning (feature 022).** Built by task T022-082: this build does not raise it yet. A concurrency key has no room and its oldest eligible waiter queued more than `SAGAT_ALERT_UNCLAIMED_SECS` (300) before the key's oldest current holder took its permit: what it waits for keeps going to others. The **subject** is the key's group, never a key's value or label.

- **Confirm:** `GET /namespaces/{id}/concurrency-keys?group=<group>` (`holders`, `waiters`, `oldestWaitSince`, `oldestHolderSince`); the SQL of [`flow-control.md`](flow-control.md) section 8.
- **Likely causes:** definitions sharing an explicit group with different limits (`CONCURRENCY_LIMIT_MISMATCH` at publication): an older waiter refused by its own lower limit does not block younger ones; a lowered namespace `concurrency_limit`.
- **Do:** give the group one limit, or the lower-limit definition a group of its own; cancel the waiting run with its owner's agreement, or let it reach its deadline. Never edit the `concurrency_*` rows by hand. Details: [`flow-control.md`](flow-control.md) section 8.
- **Over when:** no key of the group is in the condition.

## rate-limit-grant-lagging

**`RATE_LIMIT_GRANT_LAGGING`, critical (feature 022).** Built by task T022-082: this build does not raise it yet. A rate-limit key has waiters, has been due (its next-eligible hint passed) for longer than `SAGAT_ALERT_LEASE_GRACE_SECS` (60), and no grant of it was made in that time: the grant pass is not serving it. The **subject** is the key's group.

- **Confirm:** `rate_limit_grant_lag_seconds` once built; `scheduler_actions_total{kind="gate_grant",outcome}`; the SQL of [`flow-control.md`](flow-control.md) section 9.
- **Likely causes:** no engine runs or none holds the key's home shard; the engine that holds it has no flow-control key ring (it logs that it grants no queued step); the pass fails every tick.
- **Do:** make an engine that holds the key ring run and reach the database. Nothing is lost: waiters stay buffered with their deadlines and the pass grants in admission order, at most `limit` a window. A database clock that went back pauses grants without this alert ([`flow-control.md`](flow-control.md) section 12).
- **Over when:** no window of the namespace has been due for longer than the threshold.

## rate-limit-queue-saturated

**`RATE_LIMIT_QUEUE_SATURATED`, warning (feature 022).** Built by task T022-082: this build does not raise it yet. A rate-limit key's buffer of run starts holds more than 80% of the namespace's effective `flow_control_key_waiters` (100 by default, 10,000 at most). When it is full, a synchronous start is refused `429 QUOTA_EXCEEDED` with `RATE_LIMIT_QUEUE_FULL`, and event and cron starts stay pending under their retry. The **subject** is the key's group.

- **Confirm:** `GET /namespaces/{id}/rate-limit-keys` (`waiters`, `queuedRuns`, `nextSlotAt`); `rate_limit_decisions_total{scope="run",outcome}` (`queued` against `granted_from_queue`, then `refused_full`).
- **Likely causes:** arrivals above the key's limit (a backfill, a replay, a schedule that fires faster than the limit); a lowered limit or ceiling.
- **Do:** with the workflow's owner, raise the limit or the namespace's `rate_limit_per_*` ceiling only as far as the downstream system accepts, or raise `flow_control_key_waiters` to absorb a longer burst; otherwise the refusals are the gate working. Details: [`flow-control.md`](flow-control.md) section 10.
- **Over when:** no run-scope key of the namespace is above 80% of its ceiling.
