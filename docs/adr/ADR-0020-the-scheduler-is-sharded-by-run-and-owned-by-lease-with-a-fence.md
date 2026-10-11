# ADR-0020 — The scheduler's work is sharded by run and owned by lease with a fence in the action's transaction, a pass shares its batch between tenants by weight, and what scales is measured before it is promised

Status: **Accepted** | Date: 2026-10-12 | Feature: 010 | Builds on ADR-0002 (PostgreSQL is the authority and the outbox carries the wake-ups), ADR-0004 (external effects are at least once), ADR-0009 (single region first: this is high availability inside one region) and ADR-0017 (conditions are found by looking, metrics carry no tenant, run or activity)

## Context

Until feature 010 every engine replica ran every scheduler pass over every run. That was *correct* (a candidate that
another replica or a caller had already handled is a harmless skip: the domain re-checks under the run's lock) but it
was not *scalable*: the replicas read the same oldest candidates and the losers did the work of looking and skipping, so
a second replica added a cost and no capacity. Feature 002's report measured about 145 timers per second per scheduler
loop and put the cost of a burst of 100 000 due timers at seventeen minutes, whatever the number of replicas. Three
more things were left for this feature: candidates were taken oldest first, so a tenant's burst held every other
tenant back until it had drained; an engine that was cut off from the database or paused for longer than any timeout
went on acting when it came back, on the strength of what it had believed before; and nothing said how many
replicas, how many workers or how much database a given number of waits needed.

The temptations, and why each fails:

1. **Advisory locks or `FOR UPDATE SKIP LOCKED` per run as the only partitioning.** They stop two replicas doing the same
   *action*, not the same *looking*: the replicas still read the same index entries. A session-level advisory lock
   belongs to a connection, so a pooler or a dropped connection moves ownership without anyone deciding to.
2. **Leader election outside the database** (a Kubernetes `Lease`, etcd, a ZooKeeper). It adds a second source of truth
   for "who may act", and the action's transaction cannot ask it a question: a replica that was paused after it won
   the election and before its commit acts as the leader of a term that is over. The check has to be in the same
   transaction as the change, so the authority has to be in the same database.
3. **Shards per namespace or per tenant.** One busy namespace is one hot shard, and the number of shards would follow
   the customers. The work that matters (timers, retries, leases, deadlines) belongs to a run, and the identity of a
   run is random.
4. **Fairness by quota alone.** A quota bounds what a tenant can *hold* (active runs); it does not say whose timers a
   pass fires first when thousands fall due at the same second.
5. **Autoscaling and capacity numbers without a harness.** A figure in a runbook that nobody measured is read as a
   promise.

## Decision

**1. The scheduler's work is divided in 64 fixed shards by the run's identifier** (`run_shard(uuid)`, the last byte of
the UUID modulo 64: a UUIDv7's last byte is random, so the shards are even). The same function is in the domain
(`Shard::of`) and in SQL (`IMMUTABLE`, the leading column of partial indexes), and a test holds the two to the same
answer. The number is fixed for the life of a database because it is in the indexes; 64 is the largest set that is one
machine word (`ShardSet` is a `u64`), and more replicas than shards is refused by the chart. A child that ended is
reported to its parent, so its shard is the parent's.

**2. A replica works on the shards it holds a lease on.** The lease is a row (`scheduler_shards`: owner, generation,
expiry) and every time in it is the database's clock. A replica announces itself (`scheduler_members`) apart from the
leases, so one that holds nothing yet is counted and the others give it its share: `ceil(64 / replicas alive)`. The keeper (a loop
in every engine, every `lease / 5`, so that two rounds in a row can fail before a lease ends: the pause comes after a
round, and with its jitter a third would leave room for one) renews what it holds, gives back the highest shards beyond its
share and takes the lowest free ones. The renewal, like the takeover and the release, skips a row that is locked instead of
waiting for it, and a shard it skipped is renewed in the same round when the keeper adopts the leases its snapshot does not
know (a lock held by an orphan of a cut-off connection must not fail the renewal of every other shard). A **takeover raises the generation by one; a renewal does not**, and a lease that ran out cannot be
renewed, only taken again with a new generation. A clean stop gives the shards back so that the others take them at
once. The lease is 15 s by default (3 to 600): a crashed replica's work waits that long and no longer.

**3. Every action is fenced inside its own transaction.** Before the scheduler changes a run it reads the shard's row
`FOR SHARE` and proceeds only if the owner, the generation *and* the expiry are what it believes. A takeover is an
`UPDATE` of that row that **skips a row that is locked**: it cannot overtake a transaction that found the lease good
(that one commits first), it takes the shards that are free in the same call, and it asks for the skipped one again at
the next round; the transactions that start after it find the new generation. It does not wait, because a node lost in
the middle of an action leaves its transaction, and the lock with it, on the server until the server ends it, and a
takeover that waited would wait as long: the pools cap how long a transaction may sit idle (`idle_in_transaction_session_timeout`, 60 s),
which is the longest a shard can be held by an action that never finished. The owner in the check is the replica's name with a random suffix chosen at start, so a
generation that is reused (a database restored to an earlier point, a promoted asynchronous standby that lost the last
takeover) can never be mistaken for the one it replaced. A replica that cannot reach the database stops by itself: its
snapshot of what it holds expires on its own monotonic clock a little before the database would let another replica
take the lease. **The fence bounds how much redundant work a split brain can do; it is not what makes the result correct**
(the passes are idempotent under the run's lock, as before), which is why an engine of the previous version can keep
running during an upgrade.

**4. The passes read their own shards, shard first, and their cost follows what is due and not what waits.** Each
candidate query asks every owned shard for its oldest due entries through a partial index whose leading column is the
shard, and takes the oldest of those (`LATERAL ... LIMIT`), which is exactly what one query over all the shards would
return. The bound of the range is `now()` and not `clock_timestamp()`: the planner can use a stable function as the end
of an index range and cannot use a volatile one, so with `clock_timestamp()` a pass with nothing due read every waiting
step in order to find that none was due, and the cost of an idle tick grew with the number of waits. A test refuses a
comparison with `clock_timestamp()` in these files. The indexes of feature 002 are kept; dropping them is a decision
for evidence (research G240).

**5. A pass shares its batch between tenants by weighted max-min fairness, and is work-conserving.** If the first page
of candidates is short, everything was taken and nobody can be hiding behind anybody (one query, the common case).
Otherwise the pass pages through the tenants, leaving out of each next query the ones that already show their share, at
most 16 pages, and then shares the batch: a tenant with less than its share gets all it has, the rest is split by
weight, a lone tenant gets the whole batch, and inside a tenant the oldest goes first (what is left over after the weights are applied goes to the largest fractions, so a
tenant whose share is 0.96 of a candidate is not left with none at every pass by tenants whose shares are 15.31). Weights are 1 to 16, default 1, set
by the platform operator (`control-plane scheduling-weight`, audited) and read by the engine with the leases. The share
is a budget **per pass**, not a rate: the quotas of feature 009 bound what a tenant may hold and are unchanged. When more
tenants than a pass can find have work, the ones not found are found as the others drain
(`scheduler_fairness_exhausted_total` says so); that is stated, not hidden. `SAGAT_SCHEDULER_FAIRNESS=false` takes the
oldest candidates only, as a diagnostic for measuring what fairness does.

**6. The candidates of a pass are acted on concurrently** (`SAGAT_SCHEDULER_CONCURRENCY`, 8), each in its own
transaction on its own connection, under the run's lock as before. The outbox dispatcher is divided the same way: it
looks at the due messages of its shards, chooses them fairly and claims the ones it chose by id with `SKIP LOCKED`,
so the messages of one run are published, normally, by one dispatcher in the order they were written (a failed
publish that backs off, or a handover while a message is claimed, can reorder them: the hints are idempotent and
nothing may rely on the order), and a message claimed by a dispatcher that died is published by the next owner after
its visibility timeout, as before.

**7. What scales is observed without a tenant, a run or an activity on a metric** (ADR-0017). The monitor sets
`scheduler_lag_seconds{pass}` (how long the oldest overdue item of each pass has been overdue), the keeper sets the
shards held, the replicas alive and the handovers, the passes count what the fence refused and what fairness held
back, and a **pool** of workers is a name the operator gives to a set of activities (`SAGAT_WORKER_POOLS`) whose queue
(`pool_ready_steps`, `pool_queue_lag_seconds`) the monitor measures across tenants. A monitor that loses its lease sets
its gauges to zero, so two replicas never export two answers to one question, and one whose sample fails sets them to zero
too (a lag frozen at its last value would scale the engine for as long as the database cannot answer); the outbox backlog
is one of them and is counted by the monitor only, not by every dispatcher at every round. The engine's `/livez` fails when
the keeper stops rounding, as it does when the scheduler stops ticking (a round that fails counts: the loop is alive and
the database is what is down). The chart (`infra/helm`) scales the
engine with KEDA on the lag and the pools on their queue, never past 64 replicas, one at a time and slowly, with
a disruption budget that keeps a replica, and refuses at render time what the platform would refuse at start.

**8. Nothing here is a promise of capacity.** The load harness (`tests/performance/scalability-ha`) makes its
populations through the platform's own services, records the hardware and the database's configuration with every
figure, distinguishes the capacity to wait, the throughput of transitions and the latency under backlog, and says when a
load level of the specification was not reached. Backup, point-in-time recovery and failover are drilled with real
clusters (`infra/dr/drill.sh`), and the RPO and RTO of the drill are reported as what the procedure took on that
machine, not published as an objective: that needs the operator's replication and restore.

**9. The quota of active runs is read from counters the database keeps, not counted.** Every start counted the
runs that had not finished, under a lock that serialises the starts of a namespace, so the cost of a start grew with
the number of runs waiting: the benchmark could not make a hundred thousand waits in the time it should, and a million
not at all. A trigger on `runs` keeps sixteen counters per namespace in the same transaction as the change they count
(`run_active_counts`, by the run's identifier, so runs that finish together do not wait for one row); the check reads
at most sixteen rows. The lock stays, so the quota is still exact when starts race for the last places, and the
runtime role can read the counters and cannot write them (the function is `SECURITY DEFINER`, with a search path that
names `pg_temp` last, every name qualified, and no `EXECUTE` for anybody else: a role that could put a temporary table of
the same name in front of the counters, or attach the function to a table of its own, would write them with the rights
of the owner; the security review found it and a test holds it).

**10. A pass spends on looking for other tenants a bounded share of its time.** Leaving a tenant out of a query filters
its rows rather than skipping them, so a search that has to get past the backlog of the tenants it left out reads the whole
of that backlog; repeated at every tick of a burst of a hundred thousand timers that is quadratic. After a search the
pass rests it (shares the first page only) for a time that depends on what the search found and cost:

- **found nobody new** (no tenant the first page did not already show) or **failed** (a page timed out on a backlog too
  large to read in the time the pool allows; the pass shares what the pages before it showed and does not take nothing):
  the longer of `SAGAT_SCHEDULER_DISCOVERY_REST_MS` (two seconds) and ten times what the search took, up to a minute, so
  the search costs a tenth of the pass's time at most;
- **found somebody**: nothing while the search was cheap (under 100 ms), so a tenant behind a burst is served at the next
  tick; twice what it took when it was not, so that the search costs a third of the time at most while the burst and the
  tenant behind it are both served.

A tenant that arrives behind a burst while the search is resting waits for the rest to be over. The first version rested
after any search whose last page was empty, which is every search that ends, and starved the tenant it had just found
until the rest was over (the benchmark's noisy-neighbour scenario showed it). What remains is a read of the backlog of the
tenants left out per search, a limit that is stated (research G264) and is what a tenant-first index would remove.

## Consequences

- A second engine replica now adds capacity to the scheduler: the work is divided by shard, and the measured
  curve (in the report) shows where the database becomes the limit. The limit is PostgreSQL's commit rate, which more
  replicas do not raise; read replicas and partitioning of the journal are the next levers and are not built.
- A handover costs time: up to a lease after a crash, one keeper round after a clean stop or a join. Nothing is lost
  meanwhile: a timer is a row and fires late.
- The fence adds one indexed read with a shared row lock to every scheduler action, and a shard held by an action in
  flight is taken at a later round. Both are measured in the report.
- Fairness changes the order in which a pass takes work and nothing else: every candidate is still applied under the
  run's lock and re-checked by the domain. A tenant that has the whole batch to itself still gets it.
- The shard indexes are in addition to the old ones, so a database carries both until the old ones are dropped.
- A rollback of an upgrade is a rollback of the engines; the schema stays. The migration is additive.
- The generation is not a global clock: it counts takeovers of one shard. It is meaningful only with its owner.
- Whether the old indexes go, whether the journal should be partitioned by time, and what a read replica could take
  off the primary are open (research G239-G246).
