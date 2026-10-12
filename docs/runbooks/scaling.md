# Runbook: scaling and high availability of the engine (feature 010)

For the people who run the execution engine at more than one replica. It says how the scheduler's work is divided between replicas, what to change to scale it, how to upgrade it, and what each alert of the scheduler means and what to do. The numbers in it are defaults and starting points: what was measured, on what hardware, is in [`tests/performance/reports/scalability-ha.md`](../../tests/performance/reports/scalability-ha.md), and **no figure here is a promise of capacity** (`NFR-010-PERF`).

Design: [ADR-0020](../adr/ADR-0020-the-scheduler-is-sharded-by-run-and-owned-by-lease-with-a-fence.md); decisions: [`specs/010-scalability-ha/research.md`](../../specs/010-scalability-ha/research.md); the engine in general: [`execution-engine.md`](execution-engine.md); alerts of namespaces: [`alerts.md`](alerts.md); backup and restore: [`disaster-recovery.md`](disaster-recovery.md).

## 1. How the work is divided

A suspended run costs one row, so what grows with the number of waits is not memory but the work of finding what is due. The scheduler's passes (timers, event-wait deadlines, retries, lost leases, run deadlines, hints, child runs) look for it with an index scan. To let several replicas share that work without looking at the same rows, it is divided in **64 shards**, by the last byte of the run's id (a UUIDv7's last byte is random, so the shards are even).

- **A replica works only on the shards it holds.** The lease on a shard is a row of `scheduler_shards` (`owner`, `generation`, `expires_at`). Every time in it is the database's clock, so replicas with skewed clocks agree on who holds what.
- **The share is `ceil(64 / replicas alive)`.** A replica announces itself in `scheduler_members` every round, so one that holds nothing yet is counted and the others give it its share: with two replicas, 32 and 32; with three, 22, 22 and 20.
- **A takeover raises the generation**; a renewal does not. The keeper (one loop per engine, every `lease / 5`) renews what it holds, gives back the highest shards beyond its share, and takes the lowest free ones up to it.
- **Every action is fenced.** The transaction in which a replica changes a run first reads the shard's row `FOR SHARE` and proceeds only if it still holds the generation it believes it does. A takeover is an `UPDATE` of that row that skips it while it is locked: it never overtakes a transaction that found the lease good, it takes the free shards of its call, and it takes the skipped one at the next round; the transactions that start after it find the new generation. A replica that was paused for longer than its lease, or cut off from the database, finds out when it wakes up, and changes nothing (`scheduler_fenced_total` counts the attempts).
- **A replica that cannot reach the database stops by itself.** What it believes it holds expires on its own clock a little before the database would let another take it, so it selects no work without having to be told.
- **The passes are idempotent under the run's lock regardless.** The fence bounds how much redundant work a split brain could do; it is not what makes the result correct. An engine without shard ownership support that is still running during an upgrade works on every shard, and the result is the same: only redundant.

What a handover costs, in time, is on the next table (the measured values are in the report):

| Event | The work of the shards waits |
|---|---|
| a replica stops cleanly (SIGTERM) | one keeper round of the others, `lease / 5` (3 s by default) |
| a replica crashes or is partitioned | up to `lease` (15 s by default) plus one round; a shard whose row an action of the dead replica still holds locked is taken only when the server ends that transaction (`idle_in_transaction_session_timeout`, 60 s) |
| a replica joins | until the others' next round (`lease / 5`) |

Nothing is lost while a shard waits: timers are rows, the passes catch up as soon as the shard has an owner, and a timer fires late and not never.

## 2. Scaling

| Setting | Default | Change it when |
|---|---|---|
| engine replicas | 2 | `scheduler_lag_seconds` stays high with every shard held, and the database has room (see below). Never more than **64**: a replica beyond the shards holds none (the chart refuses). |
| `SAGAT_SHARD_LEASE_SECS` | 15 | Shorter takes over sooner and loses shards to a pause of the process; longer waits longer after a crash. 3 to 600. |
| `SAGAT_SCHEDULER_CONCURRENCY` | 8 | Candidates of one pass acted on at the same time, each in its own transaction on its own connection. Raise it with a burst of due timers to let a replica use the database; never above what `SAGAT_DB_MAX_CONNECTIONS` can carry. |
| `SAGAT_SCHEDULER_BATCH` | 200 | Candidates examined per pass per tick. |
| `SAGAT_DB_MAX_CONNECTIONS` | 10 (the chart sets 20) | `replicas × connections` must stay under the server's `max_connections`, with room for the control plane and the migrations. The engine uses no more concurrency than this less two (it says so when it clamps). Put a pooler in front of the server before the product passes it. |
| `SAGAT_SCHEDULER_DISCOVERY_REST_MS` | 100 | The least a pass (and the outbox dispatcher) rests its search for other tenants after one that found nobody new; it rests ten times what the search took if that is longer; see below. 0 never rests. |

**The database is the ceiling.** Every transition is a commit with its journal rows and its outbox row; adding engine replicas moves the work between processes and does not make the server's commits cheaper. When the passes are late and the replicas are not busy, look at the database (`db_pool_connections{state="in_use"}`, the server's CPU and WAL write rate) before adding replicas.

**Autoscaling.** The chart (`infra/helm`) can scale the engine with KEDA on `max(scheduler_lag_seconds)` (the oldest overdue item of any pass, in seconds; the threshold is `engine.autoscaling.keda.lagThresholdSeconds`), or with an HPA on CPU. The lag is a measure of lateness and not of load, so this is a coarse controller: it adds replicas while the scheduler is late and removes them slowly (one every two minutes, after five minutes of calm), because a shard handover costs a lease. Prefer to fix the number of replicas to what the report shows is enough for your peak, and use the autoscaler for the surprises. The lag is a signal a tenant can raise (many timers due together), so the ceiling matters: `engine.autoscaling.keda.maxReplicaCount` times `database.maxConnections` must be a number of connections the server holds, and `database.serverMaxConnections` makes the chart refuse a ceiling that is not. Set `metrics.selector` (e.g. `namespace="sagat"`) when more than one install shares the Prometheus, or the lag of staging scales production. The chart also isolates egress: the control plane reaches DNS, PostgreSQL and OIDC; the engine additionally reaches configured NATS; workers reach DNS and the release's control-plane and engine pods. Declare any collector or other infrastructure dependency in the role-specific extra rules instead of broadening the core rules.

**Pools of workers.** A pool is a deployment of your own image whose program serves some activities. Name the pools in `workerPools` (chart) or `SAGAT_WORKER_POOLS=name=act.a,act.b;name=act.c` (engine) and the monitor sets `pool_ready_steps{pool}` and `pool_queue_lag_seconds{pool}`: the steps ready for any of the pool's activities, across tenants, and how long the oldest has waited. The pool's `ScaledObject` scales on both: replicas for every `readyPerReplica` ready steps, and more when the oldest has waited longer than `lagThresholdSeconds`. Nothing routes work to a pool: a step goes to any worker that serves its activity, in any pool. The gauges count the ready steps of **every** tenant for the pool's activities, with or without a worker that could take them, so a tenant can keep a pool at its ceiling: `maxReplicaCount` is what you are willing to spend on that, and a tenant you do not trust with it gets its own install.

**Per-activity concurrency.** `activity_concurrency` is the number of tasks of one activity in one tenant namespace that may hold an unexpired lease at once. Set a generic namespace or tenant value as the default for its activities, and use an activity override when one downstream dependency needs a smaller budget. A missing setting remains effectively unlimited. A value of zero pauses new claims for that activity without changing ready work; removing an override restores the generic value. Forward, compensation and reconciliation tasks use the activity name they advertise to workers; they share a budget only when those names are the same.

The generic default uses the existing quota endpoints (`PUT /quotas/activity_concurrency` or `PUT /namespaces/{namespaceId}/quotas/activity_concurrency`). Activity overrides use `GET /namespaces/{namespaceId}/activity-quotas` and `PUT`/`DELETE /namespaces/{namespaceId}/activity-quotas/{activity}` with the same `{"limit": n}` body as other quotas. Reads need `quota:read`; changes need `quota:manage`, are ceiling checked and audited. The list contains explicit overrides; the answer to a delete has `overridden: false` and shows the inherited generic limit.

Admission is exact across engine replicas: the claim and the count of unexpired `LEASED`/`RUNNING` steps commit under the activity's database lock. When the limit is full, the candidate remains ready and the poll continues through its bounded scan for other activities. A crashed worker stops consuming the budget when its lease expires. Watch `quota_exceeded_total{limit="activity_concurrency"}` and the ready/lag gauges of the affected pool; the metric deliberately carries no tenant, namespace or activity label. If an unrelated activity behind a very large capped backlog waits to be discovered, poll again or separate the worker bindings so its session searches a smaller queue set.

**Fairness between tenants.** A pass takes up to `batch` candidates, not the `batch` oldest: it shares them between the tenants that have work due by weighted max-min fairness, so a tenant with a burst of ten thousand timers does not hold back a tenant with five. A tenant that has less than its share gets all it has; the rest go to the others in proportion to their weights; a lone tenant gets the whole batch. Weights are 1 to 16, default 1, and are set by the platform operator:

```sh
SAGAT_WEIGHT_TENANT_ID=<tenant id> SAGAT_WEIGHT_VALUE=4 control-plane scheduling-weight     # 1..16, or default
```

It writes a table the runtime role cannot, so it is run with the owner role from an operator's machine, as `quota-ceiling` is, and not from a pod of the chart. The weights are read by the engine every keeper round (5 s). Leaving a tenant out of a query filters its rows instead of skipping them, so a search that finds nobody new has read the whole backlog of the tenants it left out; after such a search (or one that failed, for the backlog was too large to read in the time the pool allows) the pass rests it for `SAGAT_SCHEDULER_DISCOVERY_REST_MS` or ten times what the search took (at most a minute), and shares only the first page of the oldest meanwhile. A search that found somebody is not rested while it is cheap (under 100 ms) and is rested for twice what it took when it is not. The search therefore costs a tenth of the pass's time at most when nobody else has work and a third when somebody does, and a tenant that arrives behind a burst waits up to one rest to be found. If a quiet tenant must not wait that long behind a burst, lower the rest and accept the reads, or give the burst's tenant a lower weight than the others. The share is a budget **per pass**, not a rate limit; the quotas of a tenant (active runs per namespace and per tenant) are where they were, in [`control-plane.md`](control-plane.md).

## 3. Upgrade

Chart `0.4.0` adds required, fail-closed NetworkPolicy peers without changing the application image
version. Before upgrading from chart `0.3.x`, populate `networkPolicy.apiFrom`, `workersFrom` and
`egress.{dnsTo,postgres.to,oidc.to}`; also populate `egress.nats.to` when `nats.url` is set. Render the
release first. The chart refuses the upgrade if a peer is missing, empty or global.

1. Run the migrations as the database owner, once, **before** any new engine starts: `control-plane migrate` (a Job in Kubernetes; the chart does not run it, so that a rolling upgrade never races its own schema change). The migrations of feature 010 only add tables, triggers and indexes; the old indexes stay, so engines of the previous version keep working while the upgrade rolls. Two things to know on a large database: the eleven shard indexes are built inside the migration (build them `CONCURRENTLY` first, same names, and check none is invalid: `data-model.md`), and the migration that adds the counters of active runs takes a `SHARE ROW EXCLUSIVE` lock on `runs` while it counts the unfinished ones (reads go on, writes wait for the count: a few seconds for a million runs; run it in a quiet moment). Engines of the previous version go on counting the runs for their quota, which is correct and slow; the counters are read by the new ones.
2. Roll the engines (the chart surges one at a time, `maxUnavailable: 0`). A new replica announces itself and takes shards as the old ones give theirs back or lose them with their pods. During the roll, old and new replicas work on the same shards: that is redundant and correct.
3. Roll the control plane.

To go back, roll the engines back. The previous version does not use the new tables and the old indexes are still there, so leave the schema as it is: the down migration exists and is tested, but no command runs it, and undoing a schema is a decision to take with whoever owns the database.

## 4. On Kubernetes: the chart

`infra/helm` installs the control plane, the engine and, if you list them, pools of your workers; PostgreSQL, NATS and the identity provider are yours. Start from [`infra/helm/examples/ha.yaml`](../../infra/helm/examples/ha.yaml).

```sh
helm lint infra/helm -f infra/helm/examples/ha.yaml
helm install sagat infra/helm -f my-values.yaml          # after the migrations (section 3)
```

- It **refuses at render time** what the platform would refuse at start: a database Secret or OIDC settings missing, more than 64 engine replicas (or an autoscaler allowed to go past 64), a lease outside 3 to 600 s, both autoscalers, TLS and plaintext secrets at once, a pool named twice or serving nothing, and unsafe NetworkPolicy inputs. Metrics, REST, worker API, DNS, PostgreSQL and OIDC peers must be explicit; NATS peers are required when NATS is configured. Empty selectors, zero-prefix CIDRs, missing peers or ports, named ports and protocols other than TCP/UDP are refused.
- Pods run as a non-root user with a read-only root filesystem, no capabilities, the runtime-default seccomp profile and no service-account token. The engine's pods are told their own name (`SAGAT_REPLICA_ID`), surge one at a time, are spread over zones and nodes, and are held by a disruption budget that keeps one replica.
- `metrics.selector` adds label matchers (`namespace="prod"`) to every query the autoscalers ask, so that two installs that share a Prometheus do not drive each other. The ports 9090 and 9091 answer only the explicit peers in `networkPolicy.metricsFrom`.
- Kubernetes NetworkPolicy cannot select an external service by hostname or SNI. Use operator-owned CIDRs, keep DNS restricted to the selected resolver, and remember that policies selecting the same pod are additive. Start from the explicit selectors in `examples/ha.yaml`; never substitute `0.0.0.0/0` or `::/0`.
- `contract_scalability_ha` renders and reads back the whole chart. The security verifier applies the rendered policies and service accounts, validates PodSpecs derived from every workload template under restricted Pod Security, and runs controlled TCP canaries on Cilium. It does not install the full platform, exercise KEDA, or prove real PostgreSQL/OIDC protocols (G247); run those deployment checks in a non-production cluster.

## shards-unowned

**`SagatShardsUnowned`.** `sum(scheduler_shards_owned)` is below 64 for two minutes: some shards are held by no replica, and the timers, retries, waits and deadlines of the runs in them are not being fired. Runs are safe; they are late.

- **Confirm:** `scheduler_shards_owned` by `instance`; `SELECT shard, owner, generation, expires_at FROM scheduler_shards WHERE expires_at < now() OR owner IS NULL ORDER BY 1;` and `SELECT * FROM scheduler_members;` for who is alive.
- **Likely causes:** a shard held by an action that never finished (a node lost in the middle of one: `SELECT pid, state, now() - xact_start FROM pg_stat_activity WHERE state = 'idle in transaction'`; the server ends it after 60 s); no engine is running (`SagatEngineDown` fires too); the engines cannot reach the database (`SagatKeeperFailing`); the lease is shorter than the pauses of the process; all the replicas are starting (the first round takes a few seconds).
- **Do:** start an engine and give it the database. Nothing needs to be repaired: a free shard is taken at the next round. If the replicas are alive and the shards stay free, read the engines' log for `could not keep the shard leases`.
- **Over when:** the sum is 64.

## shards-overlap

**`SagatShardsOverlap`.** `sum(scheduler_shards_owned)` is above 64 for two minutes: two replicas report holding the same shard. The lease row allows one owner at a time, so a replica is holding a snapshot it should have dropped: it is partitioned from the database, or paused, and its own clock has not expired it yet. A replica does expire it after at most one lease, and every action it takes in the meantime is refused by the fence.

- **Confirm:** which `instance` reports shards that `scheduler_shards` gives to another `owner`; `scheduler_fenced_total` rising on it.
- **Do:** nothing is corrupted. Find why the replica cannot reach the database or was paused (node pressure, a stop-the-world pause, a network partition) and fix that; restart the pod if it does not recover.
- **Over when:** the sum is 64.

## scheduler-late

**`SagatSchedulerLate`.** `scheduler_lag_seconds{pass}` has been above 60 seconds for five minutes: an item of that pass (`timers`, `wait_deadlines`, `retries`, `leases`, `deadlines`) has been overdue for that long. The run it belongs to is waiting, not lost.

- **Confirm:** `scheduler_lag_seconds` by `pass`; `scheduler_shards_owned` (all 64 held?); `scheduler_actions_total{outcome}` (failing, or being fenced?); the CPU of the engines; `db_pool_connections{state="in_use"}` against the pool size; the server.
- **Likely causes:** shards unowned (the alert above); a burst of due items larger than the passes take (`SAGAT_SCHEDULER_BATCH` per pass per tick, `SAGAT_SCHEDULER_CONCURRENCY` at a time); a slow database; fewer replicas than the peak needs.
- **Do:** with shards unowned, that section. With a burst, wait for the backlog to drain: fairness gives every tenant its share meanwhile (`scheduler_fairness_limited_total`). If it does not drain, add replicas (up to 64) or raise the concurrency, and look at the database first.
- **Over when:** every pass's lag is under the threshold.

## shard-fencing

**`SagatShardFencing`.** `scheduler_fenced_total` has been rising for ten minutes: a replica keeps finding that the shard of a run it was about to change is no longer its own. Once is a handover (a shard was taken while a pass was selecting); continuously is a replica that cannot drop what it no longer holds.

- **Confirm:** which `instance` and `kind`; `scheduler_shards_lost_total` on it; its log (`shard leases were lost`).
- **Likely causes:** the replica is slower than its lease (CPU throttling, a saturated connection pool, long pauses), so it loses its leases on every round and takes them again.
- **Do:** give the pods the CPU they ask for, lengthen `SAGAT_SHARD_LEASE_SECS`, or reduce `SAGAT_SCHEDULER_CONCURRENCY` so the keeper's own queries are not queueing behind the passes. No run is harmed: the fence refused each of these actions.

## keeper-failing

**`SagatKeeperFailing`.** `scheduler_keeper_failures_total` rises: a replica's keeper cannot announce itself, renew or take shards. It keeps what it holds until the leases run out (at most `lease`), and then it works on nothing.

- **Confirm:** the engine's log (`could not keep the shard leases`) has the cause; `/readyz` of that pod reports whether the database is usable.
- **Do:** restore the database connection; the keeper retries every `lease / 5` by itself.

## pool-backlog

**`SagatPoolBacklog`.** `pool_queue_lag_seconds{pool}` has been above five minutes for ten: the oldest step ready for the pool's activities has waited that long.

- **Confirm:** `pool_ready_steps{pool}`; the pool's pods (`kubectl get pods -l sagat.io/pool=<pool>`) and its `ScaledObject`; `workers_registered`; the namespace's alerts (`STEP_UNCLAIMED` names the activity, see [`alerts.md`](alerts.md#step-unclaimed)).
- **Likely causes:** the pool is at its `maxReplicaCount`; its workers are not registered or not bound to the activities (they start and serve nothing); a worker is busy for longer than the queue allows; KEDA cannot reach Prometheus.
- **Do:** raise the pool's maximum, fix the binding, or add capacity. The steps wait without cost and start when a worker takes them.

## fairness-exhausted

**`SagatFairnessExhausted`.** Passes ran out of the pages they may read (`scheduler_fairness_exhausted_total`) while looking for the tenants that have work: more tenants than a pass can see, each with more than its share, have work due. The tenants not seen in a pass are seen in a later one, as the ones that were seen drain; none is skipped for good. It is information, not a fault.

- **Do:** nothing, unless `SagatSchedulerLate` fires with it. Then the batch is too small for the number of tenants that are busy at once: raise `SAGAT_SCHEDULER_BATCH`, or add replicas, which divide the tenants' work by shard.
