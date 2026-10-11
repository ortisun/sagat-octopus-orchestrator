# Runbook: execution engine (feature 002)

Operational procedures for the `execution-engine` process: the worker gRPC API, the scheduler loop,
the housekeeping and the NATS outbox dispatcher, over the same PostgreSQL database as the control
plane. Everything here was run against the code in this repository; what is **not** covered is
listed at the end so nobody assumes it. Read `control-plane.md` first: the database roles, the
OIDC settings and the backup procedure are the same.

## 1. What runs

| Piece | Where | Notes |
|---|---|---|
| Worker API | `execution-engine serve`, `SAGAT_GRPC_ADDR`, default `127.0.0.1:9000` | gRPC over HTTP/2. TLS (and mTLS) can be terminated by the engine itself with `SAGAT_GRPC_TLS_*`, or by the ingress or mesh in front (then say so with `SAGAT_GRPC_ALLOW_PLAINTEXT_SECRETS=true` if secrets are configured). Authenticated like the REST API: bearer JWT, member of a tenant, role `worker` (or `admin`), **and bound** to the namespace and activities it serves (`docs/runbooks/security-governance.md`) |
| Scheduler loop | same process | Fires timers, wait deadlines and retries, reclaims lost leases, enforces run deadlines, renews lost wake-up hints, creates child runs, reports ended children to their parents, cancels children of stopped parents. **Each replica works only on the shards of the work it holds a lease on** (64 shards, divided between the replicas alive), shares each pass between the tenants by weight, and fences every action against its lease: [`scaling.md`](scaling.md) |
| Shard keeper | same process, every `SAGAT_SHARD_LEASE_SECS / 3` (5 s) | Announces the replica, renews its leases, gives back the shards beyond its share and takes the free ones; gives them back on `SIGTERM`. Sets `scheduler_shards_owned`, `scheduler_replicas` |
| Housekeeping | same process, every `SAGAT_HOUSEKEEPING_INTERVAL_SECS` (300) | Purges idempotency records (7 d), expired worker sessions (1 h after expiry) and delivered outbox messages (24 h, 5000 per round) |
| Outbox dispatcher | same process, only when `SAGAT_NATS_URL` is set | Publishes `task.ready` hints (`sagat.v1.task.ready.<tenant>.<activity>`) and `run.finished` events (`sagat.v1.run.finished.<tenant>`) to JetStream, at-least-once |
| Telemetry projector | same process, one at a time (a lease row in `engine_leases`) | Turns what is new in the journal of each run into spans and measurements and hands the spans to the OTLP collector; settings `SAGAT_TELEMETRY_*`, see [`observability.md`](observability.md) |
| Monitor | same process, one at a time, every `SAGAT_MONITOR_INTERVAL_SECS` (30) | Finds the conditions that need a person (outbox stuck, leases not taken back, steps nobody serves, reviews waiting, dead items), keeps the alerts in step and sets the gauges; see [`alerts.md`](alerts.md) |
| Admin listener | `SAGAT_ADMIN_ADDR`, default `127.0.0.1:9091` | `/metrics`, `/livez`, `/readyz`, **without authentication**: keep it on loopback or restrict it with a `NetworkPolicy`; the container health check uses it from inside |
| PostgreSQL 16 | `DATABASE_URL` | Source of truth. TLS required outside `SAGAT_ENV=dev`. The runtime role `sagat_app` (the engine refuses to start as a role that could rewrite the audit trail, exit code 2) |
| NATS JetStream | `SAGAT_NATS_URL`, stream `SAGAT` (subjects `sagat.v1.>`) | **Hints only.** Losing it, or never configuring it, costs latency, never correctness: workers poll, and the outbox keeps what is owed |

```sh
execution-engine serve         # runtime role; environment variables listed by `execution-engine help`
execution-engine healthcheck   # exit 0 when /livez answers; used as the container HEALTHCHECK
```

The container image is `infra/docker/Dockerfile --target execution-engine`; `infra/docker/compose.yaml`
runs it next to PostgreSQL and NATS, and `scripts/smoke-compose.sh` checks it.

## 2. Start, stop, scale, upgrade

- **Start order:** `control-plane migrate` (owner role), then engines and API with the runtime role.
- **Stop:** `SIGTERM` stops the gRPC server (in-flight calls drain), then the loops, then closes the
  pool. Workers whose call is cut off retry with the same idempotency key.
- **Scale:** run up to 64 replicas (a replica beyond the 64 shards holds none). The scheduler's work is
  divided between them by shard, so a second replica adds capacity, until the database is the limit;
  claims are `FOR UPDATE SKIP LOCKED` and the outbox is divided by shard as well. How many, how long a
  lease, what the autoscaler should watch and what each alert of the scheduler means:
  [`scaling.md`](scaling.md); what was measured: `tests/performance/reports/scalability-ha.md`.
- **Upgrade:** `migrate` first (additive migrations), then roll the engines. A run that is waiting
  for a timer, an event or a child during the roll continues with the new binary: its state is rows.
- **Upgrading from a build before feature 009:** after `migrate`, bind your worker identities (`PUT /namespaces/{id}/worker-bindings/{subject}`, see `security-governance.md`) before rolling the engines, or the workers are refused at registration.
- **Rolling back a migration** drops data (`.down.sql`): back up first.
- **Sagas (feature 003).** `SAGAT_COMPENSATION_BREAKER_THRESHOLD` (5; `0` turns it off), `SAGAT_COMPENSATION_BREAKER_OPEN_SECS` (30) and `SAGAT_COMPENSATION_BREAKER_MAX_OPEN_SECS` (300) set when the compensations of an activity that keeps failing stop being handed out; see [`sagas.md`](sagas.md). Runs in flight when the migration is applied are unaffected.

## 3. Health and diagnosis

| Probe | Meaning |
|---|---|
| `GET /livez` | The scheduler loop finished a tick within `max(10 x SAGAT_SCHEDULER_INTERVAL_MS, SAGAT_LIVENESS_GRACE_SECS)` (30 s). Failing means the loop is stuck: restart the process |
| `GET /readyz` | `/livez` and the database answers and every table of the execution schema exists. Failing before the first `migrate` is expected; it becomes ready by itself once the migration ran |

### Symptom table

| Symptom | Likely cause | What to do |
|---|---|---|
| Exit code 2 at start, message names a role | The database role can rewrite the audit trail | Use the runtime role `sagat_app` (the owner role is for `migrate`) |
| Exit code 2: `SAGAT_ALLOW_INSECURE_JWKS_HTTP is only allowed when SAGAT_ENV=dev` | Plain-http key URL outside dev | Use an `https://` JWKS URL |
| Workers get `UNAUTHENTICATED` | Token invalid/expired, tenant claim unknown, subject not a member of the tenant, or the worker session expired (idle `SAGAT_SESSION_TTL_SECS`) | Check the token; re-register after a session expiry (`PollTasks` answers `UNAUTHENTICATED`) |
| Workers get `PERMISSION_DENIED` (`x-sagat-error: FORBIDDEN`) | The identity's role in the tenant is not `worker`/`admin` | Grant the `worker` role to the worker identity |
| Workers get `PERMISSION_DENIED` with `x-sagat-error: NOT_BOUND` | The identity holds the role but is not bound to the namespace and activities it asked to serve (or its binding was removed) | Bind it: `docs/runbooks/security-governance.md`. The audit action `worker.denied` names the activity |
| Workers get `RESOURCE_EXHAUSTED` with `retry-after-ms` | Per-identity or per-tenant budget (`SAGAT_SUBJECT_RATE_LIMIT_*`, `SAGAT_RATE_LIMIT_*`), or the engine is full (`grpc_shed_total`; `grpc_in_flight` close to `SAGAT_GRPC_MAX_IN_FLIGHT`) | Poll less often (use the `retry_after_ms` the server returns when idle); when the engine is full add replicas or raise `SAGAT_GRPC_MAX_IN_FLIGHT` after checking the database pool can serve it. Heartbeats and reports are shed last, so leases are not lost while polls are refused |
| New worker connections hang | `SAGAT_GRPC_MAX_CONNECTIONS` connections are open: the next one is accepted when one closes. Connections that send nothing are closed after `SAGAT_GRPC_FIRST_BYTE_TIMEOUT_SECS` (10), silent ones after `SAGAT_GRPC_IDLE_TIMEOUT_SECS` (300), and one address holds at most `SAGAT_GRPC_MAX_CONNECTIONS_PER_PEER` (256; workers behind one NAT or load balancer share an address) | Find the clients holding connections (one per process is enough: a gRPC channel multiplexes), then raise the limit or add replicas. `grpc_connections_refused_total` and `grpc_connections_closed_total` say which rule acted |
| Workers get `ABORTED` with `x-sagat-error: STALE_ATTEMPT` | The lease was reclaimed, the run was cancelled or finished, or the report is a duplicate with a new key | Stop working on that task; this is not an error to retry |
| Tasks sit `READY` and nobody claims them | No worker advertises that activity in that namespace, or workers are at `max_inflight` | `SELECT activity, count(*) FROM steps WHERE status = 'READY' GROUP BY 1`; check `grpc_requests_total{method="PollTasks"}` |
| Runs stay `WAITING` past their timer | The scheduler is not running, is stuck, or the database clock is wrong | `/livez`; `scheduler_actions_total`; compare `steps.due_at` with `clock_timestamp()` |
| `outbox_backlog` grows | NATS is down or unreachable, or `SAGAT_NATS_URL` is unset | Nothing is lost: the backlog drains after recovery. Check `outbox_publish_failed_total{code}` and the `last_error_code` column |
| `QUOTA_EXCEEDED` (REST 429) on a signal, or a step failed with `RUN_DATA_LIMIT` | The run retains 4 MiB of outputs and signals (every lease carries them) | Return less from the activities (references, not documents); this is a limit of the run, retrying will not help |
| A step failed with `CHILD_LIMIT_EXCEEDED` | The tree of child runs reached 1024 runs, usually a workflow that starts itself without a base case | Fix the recursion; the tree has already stopped growing |
| `RegisterWorker` answers `RESOURCE_EXHAUSTED` | The identity already holds 32 sessions under other names, or its worker names change on every start | Use a stable worker name (a restart then replaces its own session), or one identity per fleet |
| A worker's token stops working right after another worker registered | Two workers use the same identity and the same worker name, so the second replaced the first's session | Give every process its own `worker_name` |
| `PollTasks` answers an error while other tenants' polls work | A run of this tenant cannot be leased (a failing statement on it); healthy runs are still served, the error appears only when nothing else is claimable | Find it in the engine log (`leasing the steps of a run failed`, with the run id) and cancel or repair that run |
| A child run never starts | The scheduler cannot create it (target workflow missing is reported to the parent as `CHILD_NOT_FOUND`; other failures are logged as `starting a child run failed`) | Look at `scheduler_actions_total{kind="child_start",outcome="failed"}` and the engine log |

Useful read-only queries (as the owner or `sagat_app`):

```sql
-- runs and what they wait for
SELECT status, count(*) FROM runs GROUP BY 1;
SELECT waiting_kind, count(*) FROM steps WHERE status = 'WAITING' GROUP BY 1;
-- leases about to be reclaimed, and retries backing off
SELECT count(*) FROM steps WHERE status IN ('LEASED','RUNNING') AND lease_expires_at < now();
SELECT count(*) FROM steps WHERE status = 'RETRY_WAIT';
-- the journal of one run
SELECT seq, event_type, node, attempt, actor, occurred_at FROM run_events WHERE run_id = '<uuid>' ORDER BY seq;
-- outbox
SELECT count(*) FILTER (WHERE delivered_at IS NULL) AS pending, max(attempt_count) FROM event_outbox;
```

## 4. Metrics (admin listener)

| Metric | Labels | Meaning |
|---|---|---|
| `grpc_requests_total` | `method`, `code` | Worker API calls by outcome (closed sets) |
| `grpc_request_duration_seconds` | `method` | Latency of each call |
| `grpc_in_flight` | | Calls running now, all connections (limit `SAGAT_GRPC_MAX_IN_FLIGHT`) |
| `grpc_shed_total` | `class` | Calls refused because the engine was full: `discovery` (`RegisterWorker`, `PollTasks`, shed above 3/4 of the limit) or `lease` (`Heartbeat`, `CompleteStep`, `FailStep`, shed only when it is completely full) |
| `grpc_connections_total` | | Connections accepted (at most `SAGAT_GRPC_MAX_CONNECTIONS` are open at once) |
| `grpc_connections_refused_total` | `reason` | Connections turned away because the address already holds `SAGAT_GRPC_MAX_CONNECTIONS_PER_PEER` |
| `grpc_connections_closed_total` | `reason` | Connections closed because the peer was silent (`silent`: never sent a byte; `idle`: silent for `SAGAT_GRPC_IDLE_TIMEOUT_SECS`) |
| `scheduler_actions_total` | `kind`, `outcome` | What the loop did: `timer`, `wait_timeout`, `retry`, `lease`, `deadline`, `rehint`, `child_start`, `child_finish`, `child_cancel`; `done`, `skipped` (someone else got there first), `failed` |
| `child_runs_started_total` | | Child runs created |
| `scheduler_actions_total{outcome="fenced"}`, `scheduler_fenced_total` | `kind` | Actions refused because the replica no longer holds the run's shard at the generation it believed (feature 010) |
| `scheduler_shards_owned`, `scheduler_replicas` | | Shards this replica holds (their sum over the replicas is 64), and replicas alive |
| `scheduler_shards_acquired_total`, `scheduler_shards_released_total`, `scheduler_shards_lost_total`, `scheduler_keeper_failures_total` | | Handovers, leases lost while away, rounds of the keeper that failed |
| `scheduler_lag_seconds` | `pass` | Set by the monitor: how long the oldest overdue item of each pass (`timers`, `wait_deadlines`, `retries`, `leases`, `deadlines`) has been overdue |
| `scheduler_fairness_limited_total`, `scheduler_fairness_exhausted_total` | `pass` | Tenants given less than they showed in a pass; passes that ran out of pages looking for tenants |
| `pool_ready_steps`, `pool_queue_lag_seconds` | `pool` | Set by the monitor for the pools of workers the operator names in `SAGAT_WORKER_POOLS`: steps ready for their activities, across tenants, and how long the oldest has waited |
| `saga_transitions_total`, `saga_outcomes_total`, `compensation_attempts_total`, `reconciliations_total`, `operator_decisions_total`, `compensation_breaker_opened_total` | closed sets | Sagas (feature 003); meaning in [`sagas.md`](sagas.md) |
| `ready_steps`, `queue_lag_seconds`, `leases_active`, `leases_expired`, `outbox_backlog`, `outbox_oldest_age_seconds`, `inbox_pending_events`, `inbox_oldest_age_seconds`, `dead_items{kind}`, `runs_active{status}`, `compensating_runs`, `workers_registered`, `alerts_firing{kind}` | | Gauges set by the monitor (feature 008) |
| `step_attempts_total{outcome}`, `step_attempt_duration_seconds`, `step_schedule_latency_seconds`, `runs_terminal_total{status}`, `run_duration_seconds` | | Made by the projector from the journal; they lag the transitions by `telemetry_projection_lag_seconds` |
| `telemetry_projection_pending_runs`, `telemetry_projection_lag_seconds`, `telemetry_projector_active`, `telemetry_spans_exported_total{outcome}`, `telemetry_projection_errors_total{reason}` | | The projection and its collector |
| `worker_polls_drained_total` | | Polls answered with nothing because the identity is being drained |
| `outbox_published_total` | `event` | Messages sent to NATS (`task.ready`, `run.finished`, `other`) |
| `outbox_publish_failed_total` | `code` | `BROKER_UNAVAILABLE`, `REJECTED` |
| `outbox_backlog` | | Messages owed |
| `db_pool_connections`, `auth_failures_total`, `jwks_refresh_total`, `http_rate_limited_total{scope}` | | As in the control plane |

No label ever carries a tenant, run, node, subject or user-supplied text.

## 5. Logs and secrets

Logs are JSON (`SAGAT_LOG_FORMAT=text` for people), carry `trace_id` and `span_id` and, inside a span that has them, `run_id`; values are redacted on the way out by the name they are logged under and by the shape of a credential (a token, `Bearer …`, the password of a URL, a key with a well-known prefix), and a line is cut at 2048 characters. They never contain lease tokens, session tokens, bearer tokens, payloads, outputs
or the database/NATS URLs. Validation messages name the field, never its value. A worker's free-text
error message is dropped (decision D31); the journal holds a bounded failure code.

## 6. Failure behaviour (what to expect)

| Failure | Behaviour |
|---|---|
| Worker dies mid-step | Its lease expires (`SAGAT_LEASE_TTL_SECS`, 30 s without a heartbeat); the scheduler records `LEASE_EXPIRED`, backs off and releases a retry with a higher generation; the dead worker's late report is refused as stale |
| Engine dies mid-transaction | The transaction rolls back; nothing half-applied exists. The candidate is found again by the next tick of any replica |
| Engine dies between publishing and confirming an outbox message | The message becomes visible again after 30 s and is republished with the same id; NATS drops the duplicate inside its window |
| NATS down | Publishing fails and backs off (1 s doubling to 60 s with jitter); the outbox grows; workers that poll are unaffected |
| PostgreSQL down | Calls fail with `UNAVAILABLE` and `retry-after-ms`; the loops log and retry; `/readyz` fails; nothing is acknowledged that was not committed |
| Timer, cancellation and signal race | The first commit wins under the run's lock; the loser is a benign no-op |
| An engine replica dies | Its shards have no owner until its lease runs out (`SAGAT_SHARD_LEASE_SECS`, 15 s) and another replica takes them; the timers, retries and deadlines of those runs fire late and once. A clean stop (`SIGTERM`) gives the shards back and costs one keeper round |
| An engine replica is paused or cut off past its lease | It stops selecting work by itself when its own clock says the lease ran out, and anything it had already chosen is refused by the fence (`scheduler_fenced_total`); it takes shards again with a new generation when it can |
| The database fails over | The pools replace their connections; replicas whose leases are not the new primary's take them again (the fence names the owner as well as the generation, so a generation that repeats is harmless); [`disaster-recovery.md`](disaster-recovery.md) |

## 7. Not covered (read before relying on this)

- **Fairness between tenants is of the scheduler's passes and the outbox (feature 010, ADR-0020).** The
  worker claim path is still oldest-first across the tenant's own steps (a worker belongs to one tenant),
  and the database pool is shared by everything in a process. Quotas per queue (per activity) do not
  exist (G245).
- Certificate-bound tokens, that is, tying a client certificate to the worker identity in the token (G44 of feature 009).
- A blob store for payloads over 256 KiB (G19), run results (G21), `foreach` and `humanApproval` nodes (G20).
- Partitioning and retention of `run_events` (G28) are not built: the journal grows with the runs
  (feature 010 measured the cost of a wait and of a transition, `tests/performance/reports/scalability-ha.md`,
  and left the partitioning for evidence, G241).
- Sagas and compensation (feature 003), inbound events and webhooks (004): the engine only emits
  `task.ready` and `run.finished`, and signals arrive through the REST API.
- OTLP span export is OTLP/HTTP with JSON only (G169); a backend that takes only protobuf needs a Collector in front. One projector and one monitor cap their own throughput (G177): sharding is feature 010. The audit trail has no retention (G182, G28).
- Failover of PostgreSQL itself, backups of the NATS stream (it holds hints only) and capacity
  planning per tier (spec open question).
