# Runbook: step logs (feature 041)

Operational notes for the step logs of design 041 (`docs/designs/041-step-logs.md`): the lines a step's code writes to
its logger, stored per attempt beside the journal with ceilings and a retention period. The architecture decisions are
ADR-0084, ADR-0085, ADR-0095 and ADR-0097. This runbook is completed by the design's tasks T041-10 (the purge, the
retention rule and the deployment switch), T041-30 (the delete route and the incident path) and T041-25 (the alerts and
their entries); until then it holds the sizing and operating facts the owner and the design decided.

## 1. What capture costs

Capturing step logs adds work to every report that carries records: the report's transaction stores the attempt's last
lines (ADR-0095) and charges them to its writer's step-log quota lease (ADR-0097). Task T041-35 measured this cost on the
hosted runners (4 cores, saturated by the benchmark itself; run 44834024933, recorded in
`specs/041-step-logs/validation-report.md`, decision D041-122), with every step writing ten `info` lines:

| Fleet | Rate with capture on, against capture off | Completion p99 against the allowance | Rate when the output stores nothing new | Log tickets used (of half the pool) |
|---|---|---|---|---|
| 1 API replica, 16 workers | 82.0% | +8.6 ms over it | 87.1% | 7 |
| 8 API replicas, 128 workers | 93.1% | 17.0 ms under it | 91.5% | 5 |

The allowance is the capture-off p99 plus the larger of 10% of it and 2 ms (OD-2026-11-20-1, OD-2026-11-20-5).

The owner accepted the measured cost ("Accept the measured cost", OD-2026-11-20-13). What it means for you:

- **A small fleet on a saturated host pays about 18% of its step rate for capture.** When the database host and the
  replicas share few cores and run near their limit, expect roughly one step in five fewer per second, and a completion
  p99 about 9 ms longer, with capture on. A larger fleet (8 replicas, 128 workers) stays within 10% of its rate.
- **If the rate matters more than the lines**, lower the cost per namespace: the attempt ceilings and the capture switch
  are namespace settings (OD-2026-11-16-9), and a namespace with capture off pays nothing. Capture is on by default at
  `info` (OD-2026-11-13-14).
- **Sizing:** give the database host headroom above the benchmark's saturation point, or scale out replicas, before
  turning capture off.

The release keeps these figures only while task T041-9, on the built carry, meets the per-size criterion of
SC-041-007 (decision D041-126) as amended by task T041-38: at 8 × 128 rate floors at the fourth run's worst valid rounds
(rate at least 88.6%, rate with output `{}` at least 83.3%, OD-2026-11-21-8) and a p99 floor at most 112.9 ms over the
allowance (OD-2026-11-21-1); at 1 × 16 a floor at the fourth run's figures less the spread of its eight valid rounds
(rate at least 80.4%, p99 at most 29.1 ms over the allowance, rate with output `{}` at least 83.5%), judged on the median
of all valid rounds of a twelve-round run, at least seven (decisions D041-135 and D041-136). The p99 floors are a guard
against gross regressions only (they admit about +94% over capture off at 1 × 16 and +33.4% at 8 × 128); task T041-9
reports to the owner a median p99 above +73.8% (1 × 16) or +29.9% (8 × 128), the worst valid rounds of the build the
owner accepted (decision D041-138), and these figures are then restated from its record.

## 2. Lines not stored because the log service was busy

Each database backend that writes step logs for a tenant holds a small *quota lease* of that tenant's step-log room. A
carry charges its lease; when the lease is short, the carry refills it from the tenant's usage row inside the report
(the *fallback refill*), waiting at most 200 ms for each lock of that row, and only while it holds one of the tenant's
*wait slots* and one of the replica's on that replica (3 and 5 with a pool of 20, 1 and 2 with the binary's default
of 10). A carry that cannot get the
row within 200 ms, or finds the slots taken,
drops its records; the report always lands. The reader sees "not stored: the log service was busy (not your quota)" on
the stream (its field `notStoredReason` is `service_busy`, never a ceiling) and in Studio, and the drop is counted
`step_log_lines_dropped_total{reason="service_busy"}` and `step_log_quota_refusals_total{cause="busy"}`. The owner
accepted this visible loss as built ("Accept the visible loss", OD-2026-11-20-14; gap G041-026). Task T041-35's fourth
run measured 1 such carry of 864,000, far from the limit with direct connections. It is more frequent near the
tenant's limit and behind a transaction pooler (gap G041-027). If it exceeds 0.1% of carries in a regime that task
T041-37 counts for this rule, the question goes back to the owner.

**The alert** `SagatStepLogServiceBusyDrops` fires when `step_log_quota_refusals_total{cause="busy"}` is above 0.1% of
`step_log_carries_total` for ten minutes. The server writes a structured warning naming the tenant at most once per
tenant per minute on each replica, with the count since the last one. Look at who holds the tenant's usage row (a long
transaction, the purge, a change of the tenant's step-log limit), at `step_log_quota_fallbacks_total` against
`step_log_quota_top_ups_total`, and at whether the tenant is near its limit.

**Wait slots** (decisions D041-125 and D041-134). A wait on a tenant's usage row through step logs (a carry's
fallback, a batch, a script marker; the top-up never waits) holds a tenant slot and a replica slot. A replica has
*Q* = max(1, ⌊N / 4⌋) slots, N its pool `SAGAT_DB_MAX_CONNECTIONS` (10 by default, 20 in the Helm chart), and a tenant
*P* = ⌈*Q* / 2⌉ of them: 5 and 3 with a pool of 20, 2 and 1 with 10 or 8. So one tenant's waits hold at most *P* of a
replica's connections and all tenants' at most *Q*; with the Logs cap (half the pool) that leaves at least
N − ⌊N / 2⌋ − *Q* connections (5 of 20, 3 of 10) that step-log waits and log calls never hold, for task leases,
heartbeats and reports that do not wait; with a pool of 2 or less nothing is left, so give a replica that serves step
logs a pool of at least 3 (which leaves 1). *Q* follows the pool size: the server computes it from its own pool and sends it with each
transaction that may wait, so raising `SAGAT_DB_MAX_CONNECTIONS` raises it, behind a pooler too. Raising
`SAGAT_STEP_LOGS_TENANT_CALLS` does not change the slots. While other tenants hold all *Q* slots, a carry of a quiet
tenant whose own usage row is busy at that moment answers busy at once: one tenant's load can cause another's (rare)
busy drops, never a wrong charge. Reports of one run that queue on the run row while a sibling's fallback waits are
not covered by this reserve.

## 3. How the quota leases are kept topped up

A backend's lease is topped up *after* a carrying report commits, never before it: when the carry left less than 64 KiB
(65,536 bytes) in the lease, the server runs one short statement on the same connection, under no run lock, and the
report's answer does not wait for it (decision D041-124, OD-2026-11-20-18). The top-up never waits: when the tenant's
usage row is busy it answers `busy` and gives up; it takes no wait slot and never takes another backend's room
(decision D041-134). Far from the limit a lease is granted 1 MiB,
so a backend tops up about once every 300 ten-line carries. The first carry of each backend for a tenant, after a
deploy, a pool recycle or an idle reclaim (10 minutes), finds no lease and refills inside its report: that is the
expected cost of a cold start. Counted in `step_log_quota_top_ups_total{outcome}` and `step_log_quota_fallbacks_total`.

**Behind a connection pooler.** Session pooling is not required. Behind a pooler in transaction mode the top-up may
run on another backend than the next report, so more refills happen inside reports: about the fourth run's share,
0.31% of ten-line carries and 2.0% of 64-record carries far from the limit (gap G041-027, accepted with ADR-0097,
OD-2026-11-20-15). The pooler advice of `docs/runbooks/scaling.md` stands as written: run a pooler in transaction mode
when replicas × pool size approaches `max_connections`; expect more fallback refills, and watch
`step_log_quota_fallbacks_total` and `SagatStepLogServiceBusyDrops` (decision D041-130). **Size the pooler's server
pool** for the database at least at the sum over the replicas of (half the replica's pool + its *Q*), plus one a
replica: 8 replicas of 20 need 8 × (10 + 5) + 8 = 128. Below that, the replicas' step-log waits and log calls can take
every server backend, and heartbeats and task leases then queue in the pooler (`SHOW POOLS`: `cl_waiting`,
`maxwait`). The pool's statement timeouts travel as the startup `options` parameter; if the pooler refuses or drops it,
set the same timeouts on the database role (`ALTER ROLE … SET`).

## 4. Log admission: the Logs class and the tickets

A long step streams its earlier lines through `AppendStepLogs`, admitted in a class of its own per replica:
- `SAGAT_GRPC_MAX_LOG_CALLS`, the class cap, defaults to half of `SAGAT_DB_MAX_CONNECTIONS` rounded down, at least 1
  (10 of a pool of 20, 5 of the binary's default of 10): at most that many connections are in a log transaction at
  once. Below a pool of 16 one tenant's tickets (at most the cap) equal the cap, so one tenant can fill the class on a
  replica; the others' batches then answer `RETRY_LATER`.
- `SAGAT_STEP_LOGS_TENANT_CALLS`, a tenant's tickets per replica, defaults to 7, at most the cap. It comes from the
  fourth run (decision D041-127): ⌈batches per second × the batch's p99 service time / 0.7⌉ gave 7 with 1 replica and
  16 workers and 5 per replica with 8 replicas and 128 workers.

A batch that finds no ticket free is answered `RETRY_LATER` at once and resent; it is lost only after 10 seconds.
**Raise `SAGAT_STEP_LOGS_TENANT_CALLS`** (and the cap with it, never above half the pool) when one tenant's long steps
see sustained `step_log_batches_total{outcome="retry_later"}` and `SagatStepLogIngestShedding` fires while the database
has headroom: two of the fourth run's eight valid rounds needed 10 tickets for a tenant at the benchmark's saturation.

## 5. Who sees the step-log quota's usage

The quota route shows `used` of `tenant_step_log_bytes` only to a caller who holds `run:logs:read` and the PII reveal
grant in every namespace of the tenant. Anyone else sees the resource, its limit and `usageWithheld: true`, never the
live figure, which moves with what other runs log (decision D041-133, "Hide it without the grant", OD-2026-11-20-19).
To let an administrator see it, give them the grant in every namespace. The grant and the usage are read together, in
one snapshot, so a view that reads after a revoke commits withholds the figure at once (decision D041-141).

## 6. PostgreSQL logging for the runtime role

A carry puts line text into the parameters of a call on the report path. Set, for the role `sagat_app`,
`log_parameter_max_length = 0` and `log_parameter_max_length_on_error = 0` (no parameter value is logged, not even a
prefix), and `log_error_verbosity = terse`, so that a database error never copies line text into PostgreSQL's own log
(security review Nit S-5).

## 7. Retention and the purge

A stream takes the namespace's `step_log_retention_days` when it is created and keeps it. Its expiry is the server's
time of its last recorded batch or carry plus that retention, so a step that keeps logging keeps its stream. **Lowering
the retention applies only to streams created after the change**: it never shortens the expiry of a stored stream
(ADR-0027's retention rule). To remove lines early, use `DELETE /runs/{runId}/logs`.

The housekeeping pass of every execution engine removes expired lines in two steps, each in short transactions:
1. **Mark**: at most 500 expired streams a transaction get `removed_at`. From then on readers show them as `expired`
   with no line, and a worker's next batch on one is answered `STOP_ATTEMPT` (`REMOVED`). A stream whose row an ingest
   holds is skipped and marked by the next pass.
2. **Delete**: at most 500 lines of one tenant a transaction; a stream with no line left gets `purged_at`. Each delete
   releases its bytes from the tenant's `tenant_step_log_bytes` usage at its commit.

Watch `step_log_purge_backlog` (the streams expired or being removed and not yet purged, counted at the start of each
pass) and `step_log_purged_lines_total`. A backlog that grows across passes means the purge does not keep up with what
expires: shorten `SAGAT_HOUSEKEEPING_INTERVAL_SECS` (300 by default) or look for a long transaction holding the streams
(`database_oldest_transaction_age_seconds`). `step_log_purge_failures_total{cause="underflow"}` counts a delete whose
release would have taken the tenant's usage below zero; it never happens unless the accounting is broken, so open an
incident, keep the lines (the delete was rolled back) and compare `sagat_quota_step_log_usage(tenant)` with the sum of
the tenant's stored line charges. The stream rows themselves stay until the run is deleted (feature 020, gap G041-004).

## 8. The quota reclaims and their counters

A backend's quota lease that has not been charged for 10 minutes, or whose backend has ended (a pool recycle, a
restart), is folded back into the tenant's usage by the same pass: at most 100 tenants a pass, one tenant a
transaction. A tenant whose step-log fence is held at that moment (a change of its limit in progress) is skipped and
tried again at the next pass. A change of the tenant's `tenant_step_log_bytes` limit or ceiling reclaims every quota
lease of the tenant at once.

`step_log_quota_refills_total{kind}` (`refill`, `reclaim`, `idle`, `ended`) and the gauge `step_log_quota_lease_rows`
come from counters the database keeps and are published only by the replica that holds shard 0, so a sum across
replicas counts each event once. The counters count attempts: a refill whose transaction rolled back is counted too. A
replica that loses shard 0 sets its gauge to 0 and stops publishing; the next holder starts from its own first read. A
steady rise of `step_log_quota_lease_rows` means backends that charge and then live on idle: it is bounded by the
backends that charged in the last 10 minutes.
