# Performance and scalability report: feature 010 (scalability and high availability)

Measured by `tests/performance/scalability-ha/main.rs`, run by `tests/performance/scalability-ha/run.sh`
(`SAGAT_PERF_MAX_WAITS` = 1000000). **These are measurements of one machine on one day, not promises** (`NFR-010-PERF`):
each section says what it measured and on what. The machine was shared with other work, which the load
average of each section shows; absolute figures vary from run to run, and the claims that matter are the
structural ones (what grows with what).

The three things the numbers must not be confused with: the **capacity to wait** (how many suspended runs
the database holds, and what each costs), the **active throughput** (step transitions per second), and the
**latency under backlog** (how late a timer fires when many fall due together).

## Reading the numbers

**What ran on which commit.** The sections for waits and for transitions ran at `9092aa1`; the sections for firing and for
the noisy tenant were run again at `603662e`, because that commit changed the floor of the fairness search's rest (from
two seconds to a tenth of a second, decision D260) and the way the noisy-neighbour scenario orders its timers. Nothing
else between the two commits touches what the waits and the transitions measure. Each section states its own commit
and the load average of the machine when it began (between 8 and 18 on 16 logical cores: other work was running).

**What the figures say, and what they do not.**

1. *Waiting costs the same at a million as at ten thousand* (the claim of US02 and of FR-010-006's 1M waits). A million
   suspended runs were made through the platform's own services in about 16 minutes (9, 79 and 851 s for the three steps) at a steady 1,060 starts per second
   (1,097 at ten thousand, 1,137 at a hundred thousand, 1,057 at a million; the start's p99 is 248, 93 and 105 ms), and
   a scheduler tick with nothing due costs 0.8 ms at the median and 3.3 ms at p99 at a million waits, as at ten
   thousand: the plan reads the shard-first index with the bound `due_at <= now()` as an index condition and never
   looks at a wait that is not due. A wait costs about 2.8 KB on disk with its indexes (2,844 bytes at a million; feature
   002's report measured 2,459 at its size; the shard-first index adds about 40 bytes per wait, 38.7 MiB at a million,
   and the rest of the difference comes from what later features write for a run and was not taken apart). Before the
   counters of active runs (decision D247) the same load fell to about 125 starts per second and could not make a million.
2. *A due timer is fired fast, and replicas add capacity until the machine is the limit.* One scheduler loop fires
   1,179 timers per second with one candidate at a time and 2,721 with eight at a time (feature 002 measured 145 per
   second per loop); four replicas fire 4,228 and eight 4,236, so past four replicas on this machine the database and the
   cores, not the number of schedulers, are the limit. A burst of 2,000 timers that fall due together is fired within
   0.5 to 1.7 s, p99 lateness 0.40 to 1.64 s. This is *redundant work removed*, not a promise about another machine: the
   replicas here are in one process on the machine that runs PostgreSQL.
3. *Fairness serves the quiet tenant without costing the noisy one its median.* With tenant A holding 80% of 10,000 timers
   due at the same instant and B's the younger, oldest-first (the behaviour before feature 010) fires B's at a median of
   3.86 s and a p99 of 4.64 s, after A's. With equal weights B's p99 is 0.89 s and A's median is unchanged (1.24 s
   against 1.27 s); with B weighing 4, B's p99 is 0.61 s. The whole set also drained faster with fairness on (2.0 s against
   4.6 s). The cause was not isolated: with oldest-first the eight candidates acted on at once are all one tenant's, and a
   batch that mixes two may contend less for rows each tenant has one of; that is a hypothesis, not a finding. Fairness
   has a cost, stated in ADR-0020 (decision 10): the search for other tenants reads the due backlog of the tenants it
   leaves out (about 10 ms at 8,000 due timers), so a pass rests it for ten times what it cost (100 ms at the least), and
   a tenant behind a burst waits that long to be found.
4. *Transitions: 1,513 per second at best on this machine; 5,000 per second was **not reached**.* The best figure is
   16 workers on one API replica; it falls to 514 per second with 128 workers on eight replicas. The sample of what
   the database's active backends were doing says why this is not the database's ceiling: 34 to 57% of them were waiting for the
   *client's* next statement (`Client:ClientRead`), 23 to 54% were running, and commit flushes (`WALWrite`) 5 to 17%.
   The workers, up to eight API replicas and the load generator are tasks in one process on eight threads, on a machine at
   a load of 12 from other work, so the ceiling measured is the harness's. A figure for the platform needs the workers and
   the database on different machines, with a network between them (research G249, G270); until then the honest statement
   is that 1,000 per second was reached closed-loop and 5,000 was not tried under conditions that could reach it.

**Not measured:** a network between the services and the database; replication (the synchronous commit of a standby
is the cost that matters for RPO zero, and the drill measures failover and not throughput); the cost of the journal and
the projections at the sizes of a long-lived system (the journal is partitioned by nothing yet, G241); the loss of 30% of
the workers under load (the chaos suite kills workers and counts what is lost, it does not time throughput); 5,000
transitions per second; and PostgreSQL 16 (CI's) at these sizes.

## Waits at scale (BL01, BL02, BL03)

- CPU: Apple M3 Max (16 logical cores), 48 GiB of memory
- OS: Darwin 25.3.0 arm64
- Load average when the scenario began: 18.17 13.16 13.06 (other work on the machine adds noise)
- Build profile of the test process and the services: release
- Commit: 9092aa1
- PostgreSQL 18.6 (Homebrew) on aarch64-apple-darwin25.6.0
- PostgreSQL configuration: `checkpoint_timeout=900s`, `effective_cache_size=20971528kB`, `fsync=on`, `full_page_writes=on`, `max_connections=400`, `max_wal_size=16384MB`, `random_page_cost=4`, `shared_buffers=2621448kB`, `synchronous_commit=on`, `wal_level=replica`, `work_mem=16384kB`
- Topology: PostgreSQL and the services on one machine (no network between them and the client); each engine replica is a scheduler with its own connection pool; HA of the database is not part of the measurement.

Runs of `timer(1h) -> activity`, started through the run service with the number of callers below; every one is suspended on its timer. The population grows in one database, so each size includes the earlier ones.

Callers starting runs at the same time: 80.

| Suspended runs | Loaded in | Starts/s | Start p50 / p95 / p99 | Bytes per run (rows and indexes) | Idle tick p50 | p95 | p99 | max |
|---:|---:|---:|---|---:|---:|---:|---:|---:|
| 10000 | 9 s | 1097 | 66 / 87 / 248 ms | 3219 | 0.9 ms | 2.3 ms | 3.5 ms | 13.7 ms |
| 100000 | 79 s | 1137 | 69 / 80 / 93 ms | 2880 | 0.9 ms | 1.6 ms | 2.9 ms | 5.8 ms |
| 1000000 | 851 s | 1057 | 74 / 90 / 105 ms | 2844 | 0.8 ms | 1.4 ms | 3.3 ms | 6.6 ms |

Size of the indexes the passes read, at the largest population:

- `steps_ready_idx`: 0.0 MiB
- `steps_timer_idx`: 21.4 MiB
- `steps_timer_shard_idx`: 38.7 MiB

Plan of the due-timer query over four shards, 10000 suspended runs:

```
Limit (actual rows=0.00 loops=1)
  Buffers: shared hit=8
  ->  Sort (actual rows=0.00 loops=1)
        Sort Key: steps.due_at
        Sort Method: quicksort  Memory: 25kB
        Buffers: shared hit=8
        ->  Nested Loop (actual rows=0.00 loops=1)
              Buffers: shared hit=8
              ->  Function Scan on unnest sh (actual rows=4.00 loops=1)
              ->  Limit (actual rows=0.00 loops=4)
                    Buffers: shared hit=8
                    ->  Index Scan using steps_timer_shard_idx on steps (actual rows=0.00 loops=4)
                          Index Cond: ((((get_byte(uuid_send(run_id), 15) & 63))::smallint = sh.s) AND (due_at <= now()))
                          Index Searches: 4
                          Buffers: shared hit=8
Planning:
  Buffers: shared hit=151
Planning Time: 0.425 ms
Execution Time: 0.019 ms
```

Plan of the due-timer query over four shards, 100000 suspended runs:

```
Limit (actual rows=0.00 loops=1)
  Buffers: shared hit=12
  ->  Sort (actual rows=0.00 loops=1)
        Sort Key: steps.due_at
        Sort Method: quicksort  Memory: 25kB
        Buffers: shared hit=12
        ->  Nested Loop (actual rows=0.00 loops=1)
              Buffers: shared hit=12
              ->  Function Scan on unnest sh (actual rows=4.00 loops=1)
              ->  Limit (actual rows=0.00 loops=4)
                    Buffers: shared hit=12
                    ->  Index Scan using steps_timer_shard_idx on steps (actual rows=0.00 loops=4)
                          Index Cond: ((((get_byte(uuid_send(run_id), 15) & 63))::smallint = sh.s) AND (due_at <= now()))
                          Index Searches: 4
                          Buffers: shared hit=12
Planning:
  Buffers: shared hit=105
Planning Time: 0.434 ms
Execution Time: 0.030 ms
```

Plan of the due-timer query over four shards, 1000000 suspended runs:

```
Limit (actual rows=0.00 loops=1)
  Buffers: shared hit=12
  ->  Sort (actual rows=0.00 loops=1)
        Sort Key: steps.due_at
        Sort Method: quicksort  Memory: 25kB
        Buffers: shared hit=12
        ->  Nested Loop (actual rows=0.00 loops=1)
              Buffers: shared hit=12
              ->  Function Scan on unnest sh (actual rows=4.00 loops=1)
              ->  Limit (actual rows=0.00 loops=4)
                    Buffers: shared hit=12
                    ->  Index Scan using steps_timer_shard_idx on steps (actual rows=0.00 loops=4)
                          Index Cond: ((((get_byte(uuid_send(run_id), 15) & 63))::smallint = sh.s) AND (due_at <= now()))
                          Index Searches: 4
                          Buffers: shared hit=12
Planning:
  Buffers: shared hit=78
Planning Time: 0.400 ms
Execution Time: 0.051 ms
```

The same query as a prepared statement with a generic plan (what a connection settles on after a few executions):

```
Limit (actual rows=0.00 loops=1)
  Buffers: shared hit=12
  ->  Sort (actual rows=0.00 loops=1)
        Sort Key: steps.due_at
        Sort Method: quicksort  Memory: 25kB
        Buffers: shared hit=12
        ->  Nested Loop (actual rows=0.00 loops=1)
              Buffers: shared hit=12
              ->  Function Scan on unnest sh (actual rows=4.00 loops=1)
              ->  Limit (actual rows=0.00 loops=4)
                    Buffers: shared hit=12
                    ->  Index Scan using steps_timer_shard_idx on steps (actual rows=0.00 loops=4)
                          Index Cond: ((((get_byte(uuid_send(run_id), 15) & 63))::smallint = sh.s) AND (due_at <= now()))
                          Filter: (tenant_id <> ALL ($3))
                          Index Searches: 4
                          Buffers: shared hit=12
Planning:
  Buffers: shared hit=51
Planning Time: 0.452 ms
Execution Time: 0.078 ms
```

## Firing a burst of timers: concurrency and replicas (US01)

- CPU: Apple M3 Max (16 logical cores), 48 GiB of memory
- OS: Darwin 25.3.0 arm64
- Load average when the scenario began: 13.75 12.01 11.20 (other work on the machine adds noise)
- Build profile of the test process and the services: release
- Commit: 603662e
- PostgreSQL 18.6 (Homebrew) on aarch64-apple-darwin25.6.0
- PostgreSQL configuration: `checkpoint_timeout=900s`, `effective_cache_size=20971528kB`, `fsync=on`, `full_page_writes=on`, `max_connections=400`, `max_wal_size=16384MB`, `random_page_cost=4`, `shared_buffers=2621448kB`, `synchronous_commit=on`, `wal_level=replica`, `work_mem=16384kB`
- Topology: PostgreSQL and the services on one machine (no network between them and the client); each engine replica is a scheduler with its own connection pool; HA of the database is not part of the measurement.

20000 suspended runs of `timer(1h) -> activity`; for each configuration 2000 of the timers are made due at the same instant and the replicas tick until nothing is left. `lateness` is the time from the timers becoming due to their `timer.fired` event in the journal. One replica with a concurrency of 1 is the closest to the scheduler of feature 002 (one transaction at a time); its report measured 145 timers/s.

| Replicas | Concurrency | Timers/s | Drained in | Tick p50 / p95 / p99 | Lateness p50 | p95 | p99 |
|---:|---:|---:|---:|---|---:|---:|---:|
| 1 | 1 | 1179 | 1.70 s | 161 / 201 / 201 ms | 0.82 s | 1.58 s | 1.64 s |
| 1 | 8 | 2721 | 0.74 s | 68 / 75 / 75 ms | 0.30 s | 0.63 s | 0.65 s |
| 1 | 32 | 2148 | 0.93 s | 83 / 102 / 102 ms | 0.37 s | 0.79 s | 0.80 s |
| 2 | 8 | 3233 | 0.62 s | 98 / 123 / 123 ms | 0.25 s | 0.46 s | 0.48 s |
| 4 | 8 | 4228 | 0.47 s | 90 / 196 / 196 ms | 0.22 s | 0.38 s | 0.40 s |
| 8 | 8 | 4236 | 0.47 s | 89 / 353 / 353 ms | 0.23 s | 0.39 s | 0.41 s |

Replicas are schedulers with a connection pool of their own in one process; they contend for one PostgreSQL on the same machine, so the curve shows how the work divides and where the database becomes the limit, not what separate machines would do.

## A noisy tenant and a quiet one (BL06, US04)

- CPU: Apple M3 Max (16 logical cores), 48 GiB of memory
- OS: Darwin 25.3.0 arm64
- Load average when the scenario began: 13.92 12.16 11.27 (other work on the machine adds noise)
- Build profile of the test process and the services: release
- Commit: 603662e
- PostgreSQL 18.6 (Homebrew) on aarch64-apple-darwin25.6.0
- PostgreSQL configuration: `checkpoint_timeout=900s`, `effective_cache_size=20971528kB`, `fsync=on`, `full_page_writes=on`, `max_connections=400`, `max_wal_size=16384MB`, `random_page_cost=4`, `shared_buffers=2621448kB`, `synchronous_commit=on`, `wal_level=replica`, `work_mem=16384kB`
- Topology: PostgreSQL and the services on one machine (no network between them and the client); each engine replica is a scheduler with its own connection pool; HA of the database is not part of the measurement.

Tenant A starts 8000 timers and tenant B 2000 (B's are the younger), and every one of them falls due at the same instant: A has 80% of the work. One replica, concurrency 8, batch 200. `lateness` is the time from becoming due to the `timer.fired` event. With fairness off a pass takes the oldest 200 candidates, as before feature 010. The table is vacuumed before each policy, so that the dead index entries of the timers the previous policy fired do not weigh on the next one.

| Policy | Tenant | Timers | Lateness p50 | p95 | p99 | Last fired after |
|---|---|---:|---:|---:|---:|---:|
| fairness off (oldest first) | A (noisy) | 8000 | 1.24 s | 2.99 s | 3.21 s | 3.23 s |
| fairness off (oldest first) | B (quiet) | 2000 | 3.86 s | 4.63 s | 4.64 s | 4.64 s |
| fairness on, equal weights | A (noisy) | 8000 | 1.27 s | 1.94 s | 2.00 s | 2.01 s |
| fairness on, equal weights | B (quiet) | 2000 | 0.48 s | 0.85 s | 0.89 s | 0.89 s |
| fairness on, B weighs 4 | A (noisy) | 8000 | 1.26 s | 1.93 s | 1.99 s | 2.00 s |
| fairness on, B weighs 4 | B (quiet) | 2000 | 0.33 s | 0.57 s | 0.61 s | 0.61 s |

## Step transitions per second (BL02, BL04)

- CPU: Apple M3 Max (16 logical cores), 48 GiB of memory
- OS: Darwin 25.3.0 arm64
- Load average when the scenario began: 12.02 9.45 9.95 (other work on the machine adds noise)
- Build profile of the test process and the services: release
- Commit: 9092aa1
- PostgreSQL 18.6 (Homebrew) on aarch64-apple-darwin25.6.0
- PostgreSQL configuration: `checkpoint_timeout=900s`, `effective_cache_size=20971528kB`, `fsync=on`, `full_page_writes=on`, `max_connections=400`, `max_wal_size=16384MB`, `random_page_cost=4`, `shared_buffers=2621448kB`, `synchronous_commit=on`, `wal_level=replica`, `work_mem=16384kB`
- Topology: PostgreSQL and the services on one machine (no network between them and the client); each engine replica is a scheduler with its own connection pool; HA of the database is not part of the measurement.

3000 runs of three chained activities per configuration; a worker polls up to 16 tasks and completes each one (a complete is one transition: the commit, the routing and the next step becoming ready). Workers are tasks in this process with sessions of their own, spread over identities (a session budget is 32 per identity). The worker API is served by 1 to 8 replicas, each with a pool of 20 connections (the engine's default is 20): a worker's calls go to the replica its number selects. Closed loop: the workers take work as fast as they can, so the figure is the ceiling of this machine, not a rate that was asked for.

| API replicas | Workers | Transitions | Transitions/s | Poll per task p50 / p95 / p99 | Complete p50 / p95 / p99 | What the active backends of the database were doing (share of 25 ms samples) |
|---:|---:|---:|---:|---|---|---|
| 1 | 4 | 9000 | 1444 | 1 / 2 / 3 ms | 1 / 2 / 3 ms | CPU:running 54%, Client:ClientRead 34%, LWLock:WALWrite 7%, IO:WalWrite 3% |
| 1 | 16 | 9000 | 1513 | 3 / 28 / 73 ms | 2 / 4 / 4 ms | CPU:running 43%, Client:ClientRead 42%, LWLock:WALWrite 7%, IO:WalWrite 4% |
| 1 | 64 | 9000 | 1014 | 26 / 179 / 211 ms | 3 / 6 / 8 ms | Client:ClientRead 44%, CPU:running 39%, LWLock:WALWrite 10%, IO:WalWrite 5% |
| 1 | 128 | 9000 | 829 | 79 / 278 / 307 ms | 3 / 8 / 12 ms | Client:ClientRead 51%, CPU:running 38%, IO:WalWrite 5%, LWLock:WALWrite 3% |
| 2 | 64 | 9000 | 907 | 34 / 164 / 196 ms | 7 / 9 / 11 ms | Client:ClientRead 53%, CPU:running 35%, LWLock:WALWrite 5%, IO:WalWrite 3% |
| 2 | 128 | 9000 | 663 | 64 / 288 / 363 ms | 8 / 12 / 16 ms | Client:ClientRead 53%, CPU:running 25%, LWLock:WALWrite 13%, IO:WalWrite 4% |
| 4 | 64 | 9000 | 746 | 47 / 209 / 243 ms | 13 / 18 / 22 ms | Client:ClientRead 57%, CPU:running 23%, LWLock:WALWrite 8%, Lock:transactionid 7% |
| 4 | 128 | 9000 | 696 | 68 / 276 / 355 ms | 14 / 19 / 27 ms | Client:ClientRead 53%, CPU:running 24%, LWLock:WALWrite 13%, Lock:transactionid 6% |
| 8 | 128 | 9000 | 514 | 207 / 285 / 389 ms | 26 / 34 / 46 ms | Client:ClientRead 56%, CPU:running 23%, Lock:transactionid 13%, LWLock:WALWrite 5% |

The load levels of the specification (FR-010-006), against the best closed-loop figure on this machine:

- 100 transitions/s: **reached** (best measured 1513/s)
- 1000 transitions/s: **reached** (best measured 1513/s)
- 5000 transitions/s: **not reached** (best measured 1513/s)

`transitions/s` is measured from the start of the workers to the last completion. The workers, the API replicas and the database share the machine.
