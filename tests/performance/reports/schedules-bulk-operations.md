# Performance report: feature 018 (schedules and bulk run operations)

Measured by `tests/performance/schedules-bulk-operations/` (the Cargo target `perf_schedules_bulk_operations`), run by its
`run.sh`, on 2026-11-09 at commit `345dd70e` (the benchmark files themselves are not committed yet). **These are observations of
one machine on one day, not promises**: NFR-018-005 says that the firing lag, the batch item rate, the business-id claim latency and
the reset transaction time are "measured on a declared environment; nothing is promised", and this is that measurement. Every
scenario states the bounds it asserts before it runs and **fails when the data exceeds one**, after it has written its numbers:
30 of the 51 repetitions failed that way, and the failures are the findings, listed below with what they mean. No bound was loosened to make a run green, and no
production file and no ceiling was changed (D2112). Every scenario was repeated three times (N = 3) on a fresh database; a figure is
the median of the three with the smallest and the largest in brackets.

## What this report concludes

1. **One engine replica starts about 61 runs a second, so a burst is late by its size over that (G2044).** 1,000 schedules due at the
   same instant: the lag is 8.2 s at the median, 15.3 s at p95 and 16.2 s at the most (64 actions a second); 10,000: 82 s, **157 s**
   and 165 s (61 a second). The bound "p95 within 60 s" (the threshold of `SagatSchedulerLate`) held at 1,000 and was broken at
   10,000 in 3 of 3 repetitions. A second replica on the same database doubles the rate (131 to 135 a second) and halves the lag
   (p95 7.6 s at 1,000 and 69.8 s at 10,000: still broken at 10,000). Spread evenly over a minute, 1,000 schedules are kept up with
   (p95 1.0 s, 17 arrivals a second) and 10,000 (167 a second) are not (p95 100 s). The product's own histogram, the database's
   `decided_at` and an outside observer polling every 100 ms agree to within a second at every size. `SagatSchedulerLate` (60 s for 5
   minutes) would not have fired in any repetition (the gauge was above 60 s for 110 s at the most), though the gauge the replicas
   scale on would have moved. The round ceilings are not a throughput lever: `SAGAT_SCHEDULES_ROUND_ACTIONS` 1,000 with
   `SAGAT_SCHEDULES_BATCH` 500 gave the same 157 s and 60.6 actions a second, with rounds of 8.3 s instead of 1.6 s. An action costs
   about 16 ms, of which a commit is 0.5 ms.
2. **A lone batch job applies one item a second, not the 20 it asks for by default (G2043).** 120 runs at the default rate: 0.97 to
   0.98 items a second, the gap between two items 1,036 ms at the median, which is the poll of the engine's loop (1,000 ms): the
   executor never catches up in a burst (by design), a step applies at most one item of a job, and the loop sleeps its poll when a
   round finds nothing due. With the poll at 50 ms the same job applies 13.4 items a second. At the highest rate a job may ask for
   (200 a second) 1,000 runs were applied at 75 a second, which is the cost of an item (13 ms, 7.4 ms of it the action). The bound
   "reaches 80% of the rate it asked for" was broken in 3 of 3 repetitions at both rates. The effective rate of a job also depends on
   how many other jobs there are: sixteen jobs at once get 4.5 items a second each at the default rate, because the other jobs keep
   the loop busy.
3. **One replica applies about 73 to 75 batch items a second to all its jobs together, and shares them fairly.** Sixteen jobs of one
   size end within 1% of each other (the last job's end over the first's: 1.00 to 1.01), none waited more than 0.4 s for its first
   item at the default rate (4.1 s at the highest rate, where the first round takes sixteen selections), and a job of 900 runs
   beside fifteen of 30 did not hold the small ones back (they ended at 6.5 to 6.9 s, 0.45 of the allowance of the bound). The
   aggregate is bounded by the replica (320 items a second asked for at the default rate, 72 applied; 3,200 asked for at the highest,
   75 applied), so "every job reaches half its rate" was broken in 3 of 3 repetitions at both rates.
4. **The selection and the drain of a batch job are not a limit.** 10,000 runs are selected in 0.32 s (31,000 runs a second) and the
   10,000 items of a cancelled job are drained in 1.3 s (7,500 a second), both far inside the N / 200 s that the highest rate would take
   (50 s). The selection of 100,000 runs (`maxItems` hard ceiling) was not measured (G2050).
5. **The business-id claim costs 7% over a plain start and holds under 32-way contention.** In 50 rounds of each of the four shapes
   (1,600 starts a repetition, three repetitions), exactly one run was created for a hot identifier in every round (policy that
   refuses: 31 `BUSINESS_ID_IN_USE`; `USE_EXISTING`: 31 answered with the one run), a start that waits for the holder is answered in 32 ms
   at the median and 50 ms at p99 (max 62 ms), and 32 starts with 32 different identifiers cost 117 ms at the median against 109 ms for
   32 plain starts (p95 206 against 191 ms). The three bounds held in 3 of 3 repetitions.
6. **A reset at the ceiling holds the row of its source for 4.65 s, and replaying a long journal costs more than linearly (G2046,
   G2047).** The reset of a prefix of 5,000 events (`RESET_PREFIX_EVENTS`, the largest accepted) takes 4.7 s and keeps the source row locked
   for 4.65 s (0.93 ms an event; 0.95 s at 1,000 events; about 19 s at the hard ceiling of 20,000 events that the plan names, a linear
   extrapolation because no code path accepts it): the bound of 1 s was broken in 3 of 3 repetitions, the bound of 5 s for the whole
   reset held by 0.15 to 0.24 s. The points of a run, and its preview, replay the whole journal: `reset_points` 9 ms at 1,002 events,
   89 ms at 5,002 and 1,313 ms at 20,002; the preview of the same 5,000-event prefix 98 ms on the short journal and 1,367 ms on the long
   one, and the reset of it 6.0 s (the bound of 5 s broken in 3 of 3). The refusal of a point past the ceiling (5,004 and 20,000) is
   typed `RESET_PREFIX_TOO_LONG`, creates no run and is answered in 46 ms at the median (158 ms the slowest). Replay equals projection
   (`verify_run`) held for all 45 runs the resets made, up to 5,000 restored events (20 ms to check).
7. **The handover costs that G2023 estimated, measured.** The convergence count takes 6 ms at 10,000 triggers and 193 ms at 100,000
   with every trigger unconverged; a window of the drift 7 ms and 46 ms (336 ms the slowest); a whole sweep of 100,000 triggers 9.1 s
   (4.1 s once converged); every statement is inside the 4 s timeout, and the one below is the statement that comes close. The reconciliation drains a backlog at **30 to 77 schedules a second, not the 100 it
   advertises**: 100,000 triggers of two tenants take 27 minutes under the default settings (62 a second) and 21 with the tenant cap
   raised to 100 (77 a second), against the 17 minutes of the estimate (G2048). **The check the handover makes under its lock took
   2.85 s at 10,000 triggers** (1.0 s at 5,000, 28 ms at 15,000, 44 ms at 20,000, 111 ms at 50,000, 280 ms at 100,000): the planner
   chose a nested loop that rescans `schedules` for every trigger at 5,000 and 10,000 and the unique index from 15,000 on, so its cost
   follows the plan and not the size (G2049).
8. **A burst of schedules in one namespace meets the default quota of 10,000 active runs** and the pass keeps refusing what it cannot
   admit (G2045): the first launch of the 10,000-schedule scenario stopped at 9,998 for that reason.

What the figures do not support: any statement about another machine, a quiet machine, a database on another host, runs that execute
(none ran), the HTTP API in front of these services, or more than one tenant (G2050).

## Environment

| Item | Value |
| --- | --- |
| Machine | Apple Mac15,9, Apple M3 Max, 16 logical cores (12 performance, 4 efficiency), 48 GiB RAM |
| OS | macOS 26.3 (25D125), Darwin 25.3.0 arm64 |
| Container runtime | Docker Desktop 29.9.2 (client and server), its Linux VM (kernel 7.0.14-linuxkit, overlayfs): 16 CPUs and 7.75 GiB RAM (8,318,709,760 bytes). The VM also ran about thirty containers of other projects (several PostgreSQL servers among them) throughout |
| PostgreSQL | `postgres:16-alpine` by digest (`sha256:cf78e76683b9ca8c5733cbbdce6c9262b45b6767934dd0a95e671f9a0fc20685`), PostgreSQL 16.15 on aarch64-unknown-linux-musl, in a container `run.sh` starts for the run and removes after it (`--shm-size=1g`, no CPU or memory limit, published on the loopback only) |
| PostgreSQL settings | The image's defaults except `-c max_connections=200`: `shared_buffers` 128 MB (163,848 kB), `work_mem` 4 MB, `effective_cache_size` 5,242,888 kB, `synchronous_commit` on, `fsync` on, `full_page_writes` on, `max_wal_size` 1 GB, `wal_level` replica, `random_page_cost` 4, `checkpoint_timeout` 300 s, `wal_buffers` 5,128 kB. (The Testcontainers default starts this image with `fsync` off; this run does not.) **One commit costs 0.5 ms**: a one-row insert in its own transaction, a hundred of them at the start of every repetition, median 0.48 to 0.87 ms over all the fragments (0.5 in all but a few), p95 0.6 to 2.0 ms |
| Build | `cargo test -p sagat-tests --test perf_schedules_bulk_operations --release --locked`, rustc 1.98.0, the workspace's release profile (thin LTO, symbols stripped) |
| Process | One test binary per scenario. The engine's own loops (`sagat_execution_engine::loops::run_schedules`, `run_batches` and `run_triggers` with their `*_round` functions and their poll: 1,000 ms for the schedule and batch passes, 5,000 ms and the pause after a round that wrote for the triggers) drive the passes; the services are those of a world (run service, schedule service, batch service) as the **runtime role `sagat_app`** (`SAGAT_TEST_AS_RUNTIME_ROLE=1`); a pool of 16 connections for each engine replica, 16 for the world, 8 for the owner and 48 for the claim scenario. No HTTP, no TLS, no worker (the runs the scenarios start never run), a Docker port forward between the process and the database. Nothing else of the platform runs in it |
| Commit | HEAD `345dd70e` (`docs(specs): T018-074 narrowed and unticked, ...`). The benchmark files are not committed yet (a new directory under `tests/performance/`, one `[[test]]` entry, one paragraph of `docs/test-strategy.md`); every fragment says `dirty: true` for that reason. No production file differs from `345dd70e` |
| Concurrent load | Not controlled: the machine ran other agents' builds, tests and containers. The one-minute load average before and after every repetition is in the last table: 3 to 13 for most of the run, 13 to 24 during the repetitions of the 50 ms poll variant and of the steady trigger state at 20,000 and 50,000 triggers. `docker stats` of the PostgreSQL container was taken at the start and the end of every repetition (`load.log`: it says the container was idle then, which is all it can say) |

## Method

**N and the spread.** Every scenario is its own process on a fresh database (created and migrated for the repetition and dropped after it) and is repeated **three times** (N = 3); every figure in the tables is the median of the repetitions with the smallest and the largest in brackets, and a bound is reported as "held in k of 3". Nothing here is a single run. Three is a median and a spread, not a distribution: a p99 is the tail of one repetition, and the figure of the tables is the median of three tails.

**Sizes.** Firing at 1,000 and 10,000 schedules, due at the same instant or spread evenly over a minute; batch jobs of 120 (default rate), 1,000 (highest rate) and 10,000 (selection and drain) runs, sixteen jobs of 300 and of 500 runs, and one job of 900 runs beside fifteen of 30; 32-way claims in 50 rounds (1,600 starts of each of the four shapes in a repetition, after three rounds of warm-up); resets from journals of 1,000, 5,000 and 20,000 events; 5,000 to 100,000 cron triggers. The sizes are set by the environment of `run.sh`; the defaults of the target are a smoke run.

**What is read, and from where.** The product measures itself (`schedule_action_lag_seconds` and `batch_item_duration_seconds`, recorded in the process by a recorder installed for the scenario; `scheduler_lag_seconds{pass="schedules"}` as the monitor reads it, once a second); the database says when each decision and each item was committed (`schedule_actions.decided_at`, `batch_items.updated_at`); and an outside observer (a separate connection polling every 100 ms) says when the runs became visible. The three agree to within a second (the lag at p95 at 10,000 schedules due together: 157 s, 157 s and 157 s), so none of the findings rests on the process that is measured. The poll of the observer and the sampler are a few millisecond-statements a second and are part of what is measured. Latencies of the claim are taken from the release of 32 tasks at a barrier to each answer. The time a reset keeps the row of its source locked is read by a probe that asks for the row `FOR UPDATE SKIP LOCKED` in a tight loop on a connection of its own while the reset runs (a resolution of one round trip, about a millisecond; each probe takes the row for the instant of its statement).

**What is set up, and how.** The schedules are made through the schedule service as the runtime role (a transaction each, with its revision and audit record): one-minute intervals with an offset within the minute (zero for all: the same instant; spread evenly over 60 s: the spread case), `ALLOW_ALL`, one action each (`remainingActions` 1), then re-timed by one `UPDATE` to a whole minute 10 to 70 s ahead by the clock of the database (the nominal times must be occurrences of the spec, which the firing recomputes: a first attempt with times off the grid acted on nothing), with the passes already up and idle. A schedule that the pass reaches more than a minute after its time has its second time due as well and the firing records it as one `COALESCED` row in the same step (6,400 of the 10,000 at the same instant; the runs started are exactly 10,000). The runs of a batch job are started through the run service, sixteen at a time. The sources of a reset are chains of activities completed one after the other by a worker (four events a step; 8 s for 250 steps, 49 s for 1,250 and 286 s for 5,000: the cost of a step grows with the journal, which is not a finding about this feature). The trigger tables are seeded in bulk as the owner (`INSERT ... SELECT` over `generate_series`, with the foreign keys and the user triggers of the three tables off for the writing session, as `trigger_scale.rs` does): in the steady state a schedule equal to every trigger but one in four thousand, 200 invented tenants; after an upgrade no schedule for any trigger, the triggers copied from the one a real publication made for each of the two real tenants, with the guards on, so that the reconciliation really writes the schedules. The quotas of active runs are raised for every scenario (the default of 10,000 a namespace is met by the scenarios, G2045).

**Bounds.** Every scenario states the bounds it asserts before it runs and fails when the data exceeds one, after it has written its fragment (D2106). A failed repetition is therefore a finding, listed with its exit status in the last table (exit 101: a bound did not hold), not a harness error. The spec promises nothing (NFR-018-005), so each bound is either one the repository already states (the 60 s and 5 minutes of `SagatSchedulerLate`, the 4 s statement timeout, the 5 s statement timeout of the pool, the 2 s and 1 s of `trigger_scale.rs`) or one a reader would hold the feature to; the reason of each is in its module.

**How the run went, because it is part of the result.** The first launch of the firing scenario at 10,000 schedules did not end: the 10,000 runs it started never finish (no worker), and a namespace admits 10,000 active runs by default, so the pass stopped at 9,998 schedules acted and went on refusing admissions (10,500 refusals in 426 s; the engine answers a refused admission with `ADMISSION_RETRY` and tries again). The harness now raises the active-run quotas, as an operator would for a platform that runs this many (G2045); nothing of that launch is reported. The main run (13:46 to 15:25 local) then ran every scenario and was stopped while the trigger backlog was on its first repetition, which had failed on an assertion of the harness: the count the product reads stops at 100,000 and there were 100,002 triggers (off by two), and when counted exactly, by one (a round also repairs the schedule of a trigger that a real publication made). The assertion was corrected; the steady-state scenario was extended with the converged phase (it had shown a statement whose cost depended on the size in a way nobody expected, and the converged fleet is the state the handover needs); the outcomes of the schedule actions were added to the firing fragments; two variants of the settings were added (the 50 ms poll, the hard ceilings of a round); and the trigger scenarios, the two variants and the firing scenario at the same instant were run again with that binary, then the steady state at 5,000, 15,000, 20,000 and 50,000 triggers. The fragments they replaced were set aside and are not used; the first three repetitions of the firing at the same instant (same code, without the outcomes) gave p95 16.0 s and 158 s and 62 to 63 actions a second, within 6% of the ones reported.


## The numbers

Every table below is the output of `summarize.py`, unedited: the median of the three repetitions with the smallest and the largest in brackets.

### Firing lag

| scenario | N | lag p50 s (product) | p95 s (product) | max s (product) | p95 s (database) | 95% visible s (outside) | 100% visible s (outside) |
|---|---|---|---|---|---|---|---|
| firing-instant-1000 | 3 | 8.16 [7.86 - 8.29] | 15.3 [15.1 - 15.4] | 16.2 [15.9 - 16.2] | 15.3 [15.1 - 15.4] | 15.4 [15.2 - 15.5] | 16.2 [15.9 - 16.3] |
| firing-instant-10000 | 3 | 82.4 [80.9 - 82.4] | 157 [156 - 157] | 165 [164 - 166] | 157 [156 - 157] | 157 [156 - 158] | 165 [164 - 166] |
| firing-instant-10000-round1000-batch500 | 3 | 82.1 [81.7 - 85.1] | 157 [157 - 160] | 165 [165 - 168] | 157 [157 - 160] | 157 [157 - 160] | 166 [165 - 168] |
| firing-instant-two-replicas-1000 | 3 | 4.19 [4.14 - 4.38] | 7.62 [7.33 - 7.71] | 8.22 [7.73 - 8.58] | 7.61 [7.32 - 7.71] | 7.70 [7.34 - 7.73] | 8.24 [7.75 - 8.62] |
| firing-instant-two-replicas-10000 | 3 | 36.7 [36.0 - 37.2] | 69.8 [69.0 - 70.5] | 74.3 [74.2 - 74.9] | 69.8 [69.0 - 70.5] | 69.9 [69.0 - 70.6] | 74.3 [74.2 - 75.0] |
| firing-spread-1000 | 3 | 0.53 [0.51 - 0.53] | 1.04 [1.00 - 1.05] | 1.21 [1.19 - 1.22] | 1.03 [0.99 - 1.04] | 57.8 [57.1 - 57.8] | 60.9 [60.8 - 61.1] |
| firing-spread-10000 | 3 | 52.1 [51.1 - 53.3] | 100 [98.6 - 102] | 106 [104 - 107] | 100 [98.6 - 102] | 157 [156 - 159] | 166 [164 - 167] |

Lag is the time from the nominal time to the decision (`schedule_action_lag_seconds`, recorded in the engine process); `database` is `decided_at` less the nominal time; `outside` is when a separate connection polling every 100 ms first saw a run for that share of the schedules, counted from the first nominal time.

| scenario | actions/s (burst) | wall s | schedules created in s | gauge max s | s above 60 | longest stretch above 60 s | SagatSchedulerLate fires | rounds that worked (replica 1) | work round p50 ms | work round max ms |
|---|---|---|---|---|---|---|---|---|---|---|
| firing-instant-1000 | 63.8 [63.7 - 64.0] | 82.3 [34.3 - 82.9] | 2.8 [2.8 - 3.0] | 15.5 [15.0 - 16.1] | 0.0 [0.0 - 0.0] | 0.0 [0.0 - 0.0] | 0 of 3 | 10.0 [10.0 - 10.0] | 1,578 [1,552 - 1,584] | 1,662 [1,639 - 1,670] |
| firing-instant-10000 | 60.9 [60.6 - 61.1] | 176 [174 - 232] | 32.1 [30.5 - 34.9] | 165 [164 - 165] | 105 [104 - 105] | 105 [104 - 105] | 0 of 3 | 101 [101 - 101] | 1,631 [1,629 - 1,646] | 1,858 [1,853 - 2,016] |
| firing-instant-10000-round1000-batch500 | 60.6 [59.6 - 60.7] | 205 [196 - 225] | 33.1 [30.8 - 33.5] | 165 [165 - 167] | 106 [106 - 110] | 106 [106 - 110] | 0 of 3 | 21.0 [21.0 - 21.0] | 8,252 [8,242 - 8,330] | 8,711 [8,691 - 9,210] |
| firing-instant-two-replicas-1000 | 131 [119 - 138] | 45.6 [44.9 - 74.8] | 2.7 [2.6 - 2.7] | 7.4 [7.0 - 8.5] | 0.0 [0.0 - 0.0] | 0.0 [0.0 - 0.0] | 0 of 3 | 6 [5 - 6] | 1,442 [1,370 - 1,456] | 1,539 [1,528 - 1,600] |
| firing-instant-two-replicas-10000 | 135 [134 - 136] | 91.1 [89.0 - 91.6] | 32.4 [31.9 - 35.3] | 74.2 [74.1 - 74.7] | 15.0 [14.0 - 15.0] | 15.0 [14.0 - 15.0] | 0 of 3 | 52.0 [51.0 - 52.0] | 1,449 [1,441 - 1,469] | 1,759 [1,717 - 1,767] |
| firing-spread-1000 | 16.5 [16.5 - 16.6] | 128 [125 - 129] | 2.8 [2.7 - 2.8] | 1.1 [1.1 - 1.2] | 0.0 [0.0 - 0.0] | 0.0 [0.0 - 0.0] | 0 of 3 | 144 [142 - 147] | 74.3 [74.0 - 76.3] | 382 [344 - 387] |
| firing-spread-10000 | 60.4 [59.9 - 61.0] | 189 [186 - 192] | 34.6 [31.1 - 34.7] | 106 [104 - 107] | 71.0 [68.0 - 73.0] | 71.0 [68.0 - 73.0] | 0 of 3 | 102 [101 - 102] | 1,640 [1,623 - 1,664] | 1,844 [1,823 - 2,010] |

### Batch rate

| scenario | N | asked items/s | achieved items/s | selection s | wall s | gap p50 ms | gap p95 ms | gap max ms | item duration p50 s | p95 s |
|---|---|---|---|---|---|---|---|---|---|---|
| batch-one-job-default-rate-120-poll50 | 3 | 20.0 [20.0 - 20.0] | 13.4 [13.3 - 13.6] | 0.11 [0.10 - 0.11] | 8.9 [8.8 - 9.1] | 73.7 [73.1 - 74.9] | 85.8 [83.9 - 85.9] | 108 [93.5 - 200] | 0.0100 [0.0099 - 0.0101] | 0.0128 [0.0120 - 0.0137] |
| batch-one-job-default-rate-120 | 3 | 20.0 [20.0 - 20.0] | 1.0 [1.0 - 1.0] | 0.11 [0.10 - 0.11] | 122 [122 - 123] | 1,036 [1,022 - 1,037] | 1,205 [1,194 - 1,214] | 1,229 [1,216 - 1,231] | 0.0117 [0.0114 - 0.0117] | 0.0175 [0.0175 - 0.0182] |
| batch-one-job-highest-rate-1000 | 3 | 200 [200 - 200] | 74.7 [73.5 - 77.2] | 0.10 [0.10 - 0.10] | 13.5 [13.1 - 13.7] | 13.0 [12.5 - 13.1] | 16.0 [15.7 - 17.3] | 46.5 [45.0 - 48.2] | 0.0074 [0.0071 - 0.0074] | 0.0096 [0.0095 - 0.0098] |

| scenario | N | selection s | runs selected/s | drain s | items drained/s | runs started in s (setup) |
|---|---|---|---|---|---|---|
| batch-selection-and-drain-1000 | 3 | 0.05 [0.05 - 0.05] | 18,785 [18,600 - 18,932] | 0.06 [0.06 - 0.06] | 16,196 [15,970 - 16,474] | 6.0 [5.9 - 6.2] |
| batch-selection-and-drain-10000 | 3 | 0.32 [0.27 - 0.38] | 31,262 [26,024 - 37,376] | 1.34 [1.25 - 1.65] | 7,488 [6,046 - 7,980] | 56.9 [56.4 - 58.2] |

| scenario | N | demand items/s | aggregate items/s | slowest job items/s | median job | fastest job | latest first item s | first job ends s | last job ends s | last/first end | wall s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| batch-one-big-fifteen-small-30 | 3 | 320 [320 - 320] | 73.6 [71.8 - 73.9] | 4.6 [4.5 - 4.6] | 4.6 [4.5 - 4.7] | 4.7 [4.5 - 4.7] | 0.3 [0.3 - 0.4] | 6.5 [6.4 - 6.7] | 6.7 [6.7 - 6.9] | 1.03 [1.03 - 1.03] | 6.8 [6.7 - 6.8] |
| batch-sixteen-jobs-default-rate-300 | 3 | 320 [320 - 320] | 72.5 [71.7 - 72.8] | 4.5 [4.5 - 4.5] | 4.5 [4.5 - 4.6] | 4.6 [4.5 - 4.6] | 0.4 [0.4 - 0.4] | 65.8 [65.8 - 66.6] | 66.3 [66.0 - 67.1] | 1.01 [1.00 - 1.01] | 66.5 [66.3 - 67.2] |
| batch-sixteen-jobs-highest-rate-500 | 3 | 3,200 [3,200 - 3,200] | 75.1 [74.6 - 75.5] | 4.7 [4.7 - 4.7] | 4.8 [4.7 - 4.8] | 4.9 [4.8 - 4.9] | 4.1 [4.0 - 4.4] | 107 [106 - 107] | 107 [106 - 107] | 1.00 [1.00 - 1.00] | 107 [106 - 108] |

### Business-id claim, 32-way

claim-thirty-two-way-50 (N = 3 repetitions; latencies in ms, median of the repetitions' medians [min - max])

| shape | samples per repetition | p50 ms | p95 ms | p99 ms | max ms |
|---|---|---|---|---|---|
| one identifier, policy refuses: the start that wins | 50.0 [50.0 - 50.0] | 23.0 [22.8 - 23.3] | 28.2 [27.4 - 30.7] | 36.2 [30.4 - 36.2] | 36.2 [30.4 - 36.2] |
| one identifier, policy refuses: the 31 refused | 1,550 [1,550 - 1,550] | 32.1 [31.7 - 32.9] | 43.6 [43.1 - 46.4] | 49.7 [49.1 - 51.9] | 58.9 [56.4 - 61.5] |
| one identifier, USE_EXISTING: the run created | 50.0 [50.0 - 50.0] | 23.6 [23.2 - 24.0] | 29.6 [28.8 - 29.6] | 30.8 [30.5 - 32.6] | 30.8 [30.5 - 32.6] |
| one identifier, USE_EXISTING: the 31 answered with it | 1,550 [1,550 - 1,550] | 35.9 [35.3 - 36.3] | 47.8 [47.5 - 49.8] | 57.0 [52.8 - 59.8] | 66.3 [56.7 - 67.5] |
| 32 identifiers, no contention | 1,600 [1,600 - 1,600] | 117 [116 - 118] | 206 [203 - 206] | 231 [229 - 253] | 332 [274 - 407] |
| 32 plain starts | 1,600 [1,600 - 1,600] | 109 [108 - 109] | 191 [190 - 196] | 216 [215 - 230] | 262 [246 - 268] |

Claim over plain start: p50 x1.07 [1.07 - 1.08], p95 x1.07 [1.05 - 1.08], p99 x1.07 [1.06 - 1.10]; a refused start over a plain one (p50) x0.29 [0.29 - 0.30].

### Reset

| journal (events, rounded) | N | events | built in s | points ms | largest accepted point | preview p50 ms | preview max ms | reset p50 ms | reset max ms | source row locked max ms | rows copied | verify_run p50 ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1000 | 3 | 1,002 [1,002 - 1,002] | 8 [8 - 8] | 9 [8 - 9] | 1,000 [1,000 - 1,000] | 8 [8 - 8] | 10 [9 - 12.8] | 919 [909 - 930] | 968 [934 - 970] | 948 [915 - 952] | 1,000 [1,000 - 1,000] | 6 [6 - 7] |
| 5000 | 3 | 5,002 [5,002 - 5,002] | 48.8 [48.0 - 50.2] | 89.2 [88.7 - 90.8] | 5,000 [5,000 - 5,000] | 98.1 [94.9 - 98.8] | 103 [102 - 106] | 4,702 [4,694 - 4,727] | 4,762 [4,754 - 4,848] | 4,652 [4,643 - 4,737] | 5,000 [5,000 - 5,000] | 19.7 [19.0 - 20.5] |
| 20000 | 3 | 20,002 [20,002 - 20,002] | 286 [273 - 286] | 1,313 [1,273 - 1,328] | 5,000 [5,000 - 5,000] | 1,367 [1,351 - 1,373] | 1,465 [1,444 - 1,474] | 5,994 [5,993 - 6,004] | 6,092 [6,071 - 6,124] | 4,702 [4,656 - 4,739] | 5,000 [5,000 - 5,000] | 20.5 [19.5 - 21.6] |

| journal (events, rounded) | request | sequence | reason | N | preview p50 ms | preview max ms | reset p50 ms | reset max ms |
|---|---|---|---|---|---|---|---|---|
| 20000 | first past the ceiling | 5,004 [5,004 - 5,004] | RESET_PREFIX_TOO_LONG | 3 | 45.8 [44.5 - 46.7] | 55.6 [49.6 - 63.2] | 47.5 [47.4 - 48.4] | 120 [82.8 - 126] |
| 20000 | near 20000 | 20,000 [20,000 - 20,000] | RESET_PREFIX_TOO_LONG | 3 | 45.8 [45.0 - 47.5] | 97.5 [53.6 - 102] | 48.8 [47.9 - 51.0] | 78.5 [57.4 - 158] |

### Trigger handover costs

| scenario | N | seeded in s | unconverged | count p50 ms | count p95 ms | count max ms | EXISTS check p50 ms | drift view count p50 ms | windows | window p50 ms | window p95 ms | window max ms | sweep s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| triggers-steady-5000 | 3 | 0 [0 - 0] | 1 [1 - 1] | 7 [6 - 7] | 9 [9 - 10] | 9 [9 - 10.1] | 843 [642 - 913] | 5 [4 - 5] | 3 [3 - 3] | 10.2 [10 - 11.9] | 10.7 [10.4 - 13.7] | 10.7 [10.4 - 13.7] | 0.0 [0.0 - 0.0] |
| triggers-steady-10000 | 3 | 0 [0 - 0] | 2 [2 - 2] | 10.1 [10 - 10.4] | 13.3 [13.2 - 15.1] | 14.7 [14.2 - 17.5] | 380 [44.2 - 501] | 19.3 [18.8 - 19.9] | 5 [5 - 5] | 13.3 [12.9 - 14.0] | 52.9 [52.0 - 53.1] | 52.9 [52.0 - 53.1] | 0.1 [0.1 - 0.1] |
| triggers-steady-15000 | 3 | 0 [0 - 0] | 3 [3 - 3] | 28.1 [25.9 - 28.9] | 39.2 [38.3 - 42.4] | 43.0 [41.2 - 45.6] | 4.4 [2.3 - 17.7] | 24.5 [23.8 - 30.5] | 8 [8 - 8] | 20.5 [19.9 - 22.9] | 46.0 [44.0 - 48.6] | 46.0 [44.0 - 48.6] | 0.2 [0.2 - 0.2] |
| triggers-steady-20000 | 3 | 0 [0 - 1] | 5 [5 - 5] | 29.2 [28.5 - 31.6] | 43.4 [42.3 - 45.3] | 46.1 [44.5 - 52.8] | 7.0 [6.6 - 16.2] | 29.9 [28.1 - 32.8] | 10.0 [10.0 - 10.0] | 48.5 [48.3 - 54.0] | 56.4 [53.1 - 56.9] | 56.4 [53.1 - 56.9] | 0.5 [0.5 - 0.5] |
| triggers-steady-50000 | 3 | 1 [1 - 1] | 12.0 [12.0 - 12.0] | 195 [183 - 199] | 219 [216 - 226] | 229 [221 - 239] | 6.8 [5.7 - 7.9] | 177 [171 - 184] | 25.0 [25.0 - 25.0] | 49.3 [47.1 - 49.6] | 56.7 [55.2 - 58.6] | 61.5 [55.8 - 62.3] | 1.3 [1.1 - 1.3] |
| triggers-steady-100000 | 3 | 2 [2 - 2] | 25.0 [25.0 - 25.0] | 314 [304 - 322] | 380 [371 - 393] | 404 [395 - 419] | 4.2 [4.1 - 7.9] | 238 [233 - 241] | 50.0 [50.0 - 50.0] | 86.4 [84.6 - 88.9] | 603 [600 - 637] | 632 [609 - 643] | 9.1 [9.0 - 9.2] |

The same fleets once every trigger has a schedule (the state the handover needs; the check under its lock must then look at every trigger and find none):

| scenario | N | handover check (EXISTS) p50 ms | max ms | count p50 ms | windows | window p50 ms | window max ms | sweep s |
|---|---|---|---|---|---|---|---|---|
| triggers-steady-5000 | 3 | 1,009 [999 - 1,014] | 1,035 [1,033 - 1,039] | 6 [6 - 6] | 3 [3 - 3] | 10 [9 - 10.8] | 11.1 [9 - 11.7] | 0.0 [0.0 - 0.0] |
| triggers-steady-10000 | 3 | 2,853 [2,851 - 2,856] | 2,880 [2,853 - 2,890] | 11.5 [10.5 - 11.5] | 5 [5 - 5] | 13.6 [11.4 - 14.4] | 25.4 [21.0 - 26.2] | 0.1 [0.1 - 0.1] |
| triggers-steady-15000 | 3 | 28.0 [27.3 - 28.0] | 30.2 [29.0 - 30.3] | 25.3 [25.0 - 27.2] | 8 [8 - 8] | 18.5 [17.6 - 20.2] | 23.1 [21.7 - 25.1] | 0.2 [0.2 - 0.2] |
| triggers-steady-20000 | 3 | 43.5 [38.2 - 48.9] | 46.1 [41.2 - 49.8] | 29.5 [29.0 - 33.5] | 10.0 [10.0 - 10.0] | 16.8 [16.0 - 18.0] | 20.8 [20.6 - 21.5] | 0.2 [0.2 - 0.2] |
| triggers-steady-50000 | 3 | 111 [108 - 135] | 119 [114 - 145] | 186 [183 - 187] | 25.0 [25.0 - 25.0] | 40.3 [39.6 - 44.3] | 47.2 [47.1 - 49.4] | 1.0 [1.0 - 1.1] |
| triggers-steady-100000 | 3 | 280 [269 - 282] | 344 [340 - 347] | 321 [312 - 339] | 50.0 [50.0 - 50.0] | 79.0 [78.1 - 79.8] | 114 [108 - 155] | 4.1 [4.0 - 4.2] |

| scenario | N | seeded in s | unconverged | count p50 ms | count max ms | first window p50 ms | first window max ms |
|---|---|---|---|---|---|---|---|
| triggers-backlog-10000 | 3 | 2 [2 - 2] | 10,002 [10,002 - 10,002] | 6 [5 - 6] | 10 [8 - 11.0] | 7 [6 - 7] | 10 [9 - 10] |
| triggers-backlog-100000 | 3 | 125 [121 - 127] | 100,000 [100,000 - 100,000] | 193 [187 - 201] | 259 [232 - 375] | 46.1 [45.3 - 52.3] | 336 [328 - 348] |

| scenario | settings | N | window s | schedules made | writes/s | rounds that wrote | round p50 ms | round max ms | backlog drains in (minutes, extrapolated) |
|---|---|---|---|---|---|---|---|---|---|
| triggers-backlog-10000 | defaults (batch 200, 20 a tenant, window 2,000) | 3 | 60.0 [60.0 - 60.0] | 1,802 [1,642 - 1,802] | 30.0 [27.4 - 30.0] | 46.0 [42.0 - 46.0] | 151 [149 - 154] | 181 [174 - 193] | 6 [6 - 6] |
| triggers-backlog-10000 | tenant cap 100 (a round fills its batch of 200) | 3 | 60.0 [60.0 - 60.0] | 4,131 [4,091 - 4,215] | 68.8 [68.2 - 70.2] | 23.0 [23.0 - 23.0] | 639 [629 - 644] | 772 [771 - 815] | 2 [2 - 2] |
| triggers-backlog-100000 | defaults (batch 200, 20 a tenant, window 2,000) | 3 | 60.1 [60.0 - 60.2] | 3,722 [3,722 - 3,762] | 62.0 [61.9 - 62.6] | 94.0 [94.0 - 95.0] | 209 [206 - 213] | 515 [500 - 549] | 26.9 [26.6 - 26.9] |
| triggers-backlog-100000 | tenant cap 100 (a round fills its batch of 200) | 3 | 60.0 [60.0 - 60.0] | 4,640 [4,440 - 4,640] | 77.3 [74.0 - 77.3] | 24.0 [23.0 - 24.0] | 677 [672 - 685] | 770 [760 - 782] | 20.8 [20.8 - 21.7] |

### Bounds asserted and whether they held

| scenario | bound | asserted | held in | observed (each repetition) |
|---|---|---|---|---|
| batch-one-big-fifteen-small-30 | a_small_job_does_not_wait_for_a_big_one | end of each job over three times its own time at its rate plus ten seconds <= 1 x | 3 of 3 | 0.464 x; 0.451 x; 0.453 x |
| batch-one-big-fifteen-small-30 | no_job_waits_for_the_others | seconds to the first item of the job that waited longest <= 10 s | 3 of 3 | 0.337 s; 0.343 s; 0.363 s |
| batch-one-job-default-rate-120 | never_exceeds_the_rate_it_asked_for | items a second achieved <= 22 items/s | 3 of 3 | 0.980 items/s; 0.974 items/s; 0.970 items/s |
| batch-one-job-default-rate-120 | reaches_the_rate_it_asked_for | items a second achieved >= 16 items/s | 0 of 3 | 0.980 items/s; 0.974 items/s; 0.970 items/s |
| batch-one-job-default-rate-120 | selection_is_not_the_limit | seconds to select the runs <= 0.6 s | 3 of 3 | 0.104 s; 0.106 s; 0.105 s |
| batch-one-job-default-rate-120-poll50 | never_exceeds_the_rate_it_asked_for | items a second achieved <= 22 items/s | 3 of 3 | 13.616 items/s; 13.447 items/s; 13.255 items/s |
| batch-one-job-default-rate-120-poll50 | reaches_the_rate_it_asked_for | items a second achieved >= 16 items/s | 0 of 3 | 13.616 items/s; 13.447 items/s; 13.255 items/s |
| batch-one-job-default-rate-120-poll50 | selection_is_not_the_limit | seconds to select the runs <= 0.6 s | 3 of 3 | 0.104 s; 0.106 s; 0.107 s |
| batch-one-job-highest-rate-1000 | never_exceeds_the_rate_it_asked_for | items a second achieved <= 220.00000000000003 items/s | 3 of 3 | 77.231 items/s; 74.706 items/s; 73.510 items/s |
| batch-one-job-highest-rate-1000 | reaches_the_rate_it_asked_for | items a second achieved >= 160 items/s | 0 of 3 | 77.231 items/s; 74.706 items/s; 73.510 items/s |
| batch-one-job-highest-rate-1000 | selection_is_not_the_limit | seconds to select the runs <= 5 s | 3 of 3 | 0.103 s; 0.104 s; 0.105 s |
| batch-selection-and-drain-1000 | drain_is_not_the_limit | seconds to cancel the items that were not applied <= 5 s | 3 of 3 | 0.061 s; 0.062 s; 0.063 s |
| batch-selection-and-drain-1000 | selection_is_not_the_limit | seconds to select the runs <= 5 s | 3 of 3 | 0.053 s; 0.053 s; 0.054 s |
| batch-selection-and-drain-10000 | drain_is_not_the_limit | seconds to cancel the items that were not applied <= 50 s | 3 of 3 | 1.654 s; 1.335 s; 1.253 s |
| batch-selection-and-drain-10000 | selection_is_not_the_limit | seconds to select the runs <= 50 s | 3 of 3 | 0.268 s; 0.384 s; 0.320 s |
| batch-sixteen-jobs-default-rate-300 | every_job_reaches_half_its_rate | items a second of the slowest job >= 10 items/s | 0 of 3 | 4.540 items/s; 4.524 items/s; 4.473 items/s |
| batch-sixteen-jobs-default-rate-300 | jobs_of_one_size_end_together | the last job's end over the first job's end <= 2 x | 3 of 3 | 1.003 x; 1.008 x; 1.007 x |
| batch-sixteen-jobs-default-rate-300 | no_job_waits_for_the_others | seconds to the first item of the job that waited longest <= 10 s | 3 of 3 | 0.393 s; 0.448 s; 0.408 s |
| batch-sixteen-jobs-highest-rate-500 | every_job_reaches_half_its_rate | items a second of the slowest job >= 100 items/s | 0 of 3 | 4.688 items/s; 4.712 items/s; 4.653 items/s |
| batch-sixteen-jobs-highest-rate-500 | jobs_of_one_size_end_together | the last job's end over the first job's end <= 2 x | 3 of 3 | 1.001 x; 1.001 x; 1.001 x |
| batch-sixteen-jobs-highest-rate-500 | no_job_waits_for_the_others | seconds to the first item of the job that waited longest <= 10 s | 3 of 3 | 3.994 s; 4.061 s; 4.362 s |
| claim-thirty-two-way-50 | a_start_that_waits_for_the_holder_is_answered_within_a_second | p99 of the starts of a held identifier <= 1000 ms | 3 of 3 | 49.694 ms; 51.921 ms; 49.052 ms |
| claim-thirty-two-way-50 | the_claim_costs_little_at_the_median | p50 of a start with its own identifier over a plain start <= 1.5 x | 3 of 3 | 1.076 x; 1.070 x; 1.070 x |
| claim-thirty-two-way-50 | the_claim_costs_little_at_the_tail | p95 of a start with its own identifier over a plain start <= 2 x | 3 of 3 | 1.076 x; 1.050 x; 1.070 x |
| firing-instant-1000 | gauge_does_not_stay_above_the_alert_threshold_for_five_minutes | longest stretch of scheduler_lag_seconds{pass="schedules"} above 60 s <= 299 s | 3 of 3 | 0.000 s; 0.000 s; 0.000 s |
| firing-instant-1000 | nothing_refused_or_failed | steps refused or failed <= 0 steps | 3 of 3 | 0.000 steps; 0.000 steps; 0.000 steps |
| firing-instant-1000 | p95_lag_within_the_alert_threshold | p95 of the lag between the nominal time and the decision <= 60 s | 3 of 3 | 15.107 s; 15.419 s; 15.324 s |
| firing-instant-10000 | gauge_does_not_stay_above_the_alert_threshold_for_five_minutes | longest stretch of scheduler_lag_seconds{pass="schedules"} above 60 s <= 299 s | 3 of 3 | 105.000 s; 105.000 s; 104.000 s |
| firing-instant-10000 | nothing_refused_or_failed | steps refused or failed <= 0 steps | 3 of 3 | 0.000 steps; 0.000 steps; 0.000 steps |
| firing-instant-10000 | p95_lag_within_the_alert_threshold | p95 of the lag between the nominal time and the decision <= 60 s | 0 of 3 | 157.474 s; 156.975 s; 155.681 s |
| firing-instant-10000-round1000-batch500 | gauge_does_not_stay_above_the_alert_threshold_for_five_minutes | longest stretch of scheduler_lag_seconds{pass="schedules"} above 60 s <= 299 s | 3 of 3 | 106.000 s; 110.000 s; 106.000 s |
| firing-instant-10000-round1000-batch500 | nothing_refused_or_failed | steps refused or failed <= 0 steps | 3 of 3 | 0.000 steps; 0.000 steps; 0.000 steps |
| firing-instant-10000-round1000-batch500 | p95_lag_within_the_alert_threshold | p95 of the lag between the nominal time and the decision <= 60 s | 0 of 3 | 156.926 s; 159.977 s; 157.278 s |
| firing-instant-two-replicas-1000 | gauge_does_not_stay_above_the_alert_threshold_for_five_minutes | longest stretch of scheduler_lag_seconds{pass="schedules"} above 60 s <= 299 s | 3 of 3 | 0.000 s; 0.000 s; 0.000 s |
| firing-instant-two-replicas-1000 | nothing_refused_or_failed | steps refused or failed <= 0 steps | 3 of 3 | 0.000 steps; 0.000 steps; 0.000 steps |
| firing-instant-two-replicas-1000 | p95_lag_within_the_alert_threshold | p95 of the lag between the nominal time and the decision <= 60 s | 3 of 3 | 7.707 s; 7.615 s; 7.319 s |
| firing-instant-two-replicas-10000 | gauge_does_not_stay_above_the_alert_threshold_for_five_minutes | longest stretch of scheduler_lag_seconds{pass="schedules"} above 60 s <= 299 s | 3 of 3 | 15.000 s; 15.000 s; 14.000 s |
| firing-instant-two-replicas-10000 | nothing_refused_or_failed | steps refused or failed <= 0 steps | 3 of 3 | 0.000 steps; 0.000 steps; 0.000 steps |
| firing-instant-two-replicas-10000 | p95_lag_within_the_alert_threshold | p95 of the lag between the nominal time and the decision <= 60 s | 0 of 3 | 69.810 s; 70.486 s; 68.984 s |
| firing-spread-1000 | gauge_does_not_stay_above_the_alert_threshold_for_five_minutes | longest stretch of scheduler_lag_seconds{pass="schedules"} above 60 s <= 299 s | 3 of 3 | 0.000 s; 0.000 s; 0.000 s |
| firing-spread-1000 | nothing_refused_or_failed | steps refused or failed <= 0 steps | 3 of 3 | 0.000 steps; 0.000 steps; 0.000 steps |
| firing-spread-1000 | p95_lag_within_the_alert_threshold | p95 of the lag between the nominal time and the decision <= 60 s | 3 of 3 | 0.989 s; 1.040 s; 1.033 s |
| firing-spread-10000 | gauge_does_not_stay_above_the_alert_threshold_for_five_minutes | longest stretch of scheduler_lag_seconds{pass="schedules"} above 60 s <= 299 s | 3 of 3 | 73.000 s; 68.000 s; 71.000 s |
| firing-spread-10000 | nothing_refused_or_failed | steps refused or failed <= 0 steps | 3 of 3 | 0.000 steps; 0.000 steps; 0.000 steps |
| firing-spread-10000 | p95_lag_within_the_alert_threshold | p95 of the lag between the nominal time and the decision <= 60 s | 0 of 3 | 101.634 s; 98.610 s; 100.197 s |
| reset | a_reset_at_the_ceiling_of_a_journal_twice_as_long_completes_within_the_statement_timeout | slowest reset near the ceiling <= 5000 ms | 0 of 3 | 6123.950 ms; 6071.118 ms; 6091.826 ms |
| reset | a_reset_at_the_ceiling_of_a_journal_twice_as_long_holds_the_source_row_briefly | longest hold of the source row near the ceiling <= 1000 ms | 0 of 3 | 4739.365 ms; 4656.005 ms; 4701.941 ms |
| reset | the_largest_accepted_reset_completes_within_the_statement_timeout | slowest reset near the ceiling <= 5000 ms | 3 of 3 | 4762.397 ms; 4753.523 ms; 4848.281 ms |
| reset | the_largest_accepted_reset_holds_the_source_row_briefly | longest hold of the source row near the ceiling <= 1000 ms | 0 of 3 | 4652.205 ms; 4643.334 ms; 4737.031 ms |
| reset | the_refusal_at_20000_events_is_answered_within_a_second | slowest refusal (preview or reset) at 20,000 events <= 1000 ms | 3 of 3 | 157.752 ms; 102.292 ms; 57.389 ms |
| triggers-backlog-10000 | a_full_round_keeps_half_the_advertised_pace | schedules written a second by a reconciliation whose rounds are full >= 50 writes/s | 3 of 3 | 70.249 writes/s; 68.181 writes/s; 68.847 writes/s |
| triggers-backlog-10000 | a_window_stays_within_the_statement_timeout | slowest first window of a backlog <= 4000 ms | 3 of 3 | 9.825 ms; 8.978 ms; 9.710 ms |
| triggers-backlog-10000 | no_write_fails_or_is_refused | writes that failed or were refused <= 0 writes | 3 of 3 | 0.000 writes; 0.000 writes; 0.000 writes |
| triggers-backlog-10000 | the_count_stays_within_the_statement_timeout | slowest convergence count of a backlog <= 4000 ms | 3 of 3 | 10.993 ms; 9.786 ms; 8.031 ms |
| triggers-backlog-100000 | a_full_round_keeps_half_the_advertised_pace | schedules written a second by a reconciliation whose rounds are full >= 50 writes/s | 3 of 3 | 77.330 writes/s; 73.998 writes/s; 77.330 writes/s |
| triggers-backlog-100000 | a_window_stays_within_the_statement_timeout | slowest first window of a backlog <= 4000 ms | 3 of 3 | 328.159 ms; 348.036 ms; 336.083 ms |
| triggers-backlog-100000 | no_write_fails_or_is_refused | writes that failed or were refused <= 0 writes | 3 of 3 | 0.000 writes; 0.000 writes; 0.000 writes |
| triggers-backlog-100000 | the_count_stays_within_the_statement_timeout | slowest convergence count of a backlog <= 4000 ms | 3 of 3 | 258.692 ms; 231.913 ms; 374.973 ms |
| triggers-steady-10000 | a_window_stays_within_a_second | slowest window of the sweep <= 1000 ms | 3 of 3 | 52.946 ms; 52.023 ms; 53.131 ms |
| triggers-steady-10000 | a_window_stays_within_the_statement_timeout | slowest window of the sweep <= 4000 ms | 3 of 3 | 52.946 ms; 52.023 ms; 53.131 ms |
| triggers-steady-10000 | the_count_stays_within_the_statement_timeout | slowest convergence count <= 4000 ms | 3 of 3 | 14.171 ms; 14.700 ms; 17.546 ms |
| triggers-steady-10000 | the_count_stays_within_two_seconds | p95 of the count <= 2000 ms | 3 of 3 | 13.273 ms; 13.194 ms; 15.149 ms |
| triggers-steady-10000 | the_handover_check_on_a_converged_fleet_stays_within_the_statement_timeout | slowest EXISTS check (what the handover asks under its lock) when every trigger has a schedule <= 4000 ms | 3 of 3 | 2889.904 ms; 2853.486 ms; 2880.440 ms |
| triggers-steady-100000 | a_window_stays_within_a_second | slowest window of the sweep <= 1000 ms | 3 of 3 | 609.347 ms; 632.157 ms; 643.260 ms |
| triggers-steady-100000 | a_window_stays_within_the_statement_timeout | slowest window of the sweep <= 4000 ms | 3 of 3 | 609.347 ms; 632.157 ms; 643.260 ms |
| triggers-steady-100000 | the_count_stays_within_the_statement_timeout | slowest convergence count <= 4000 ms | 3 of 3 | 418.822 ms; 403.850 ms; 394.729 ms |
| triggers-steady-100000 | the_count_stays_within_two_seconds | p95 of the count <= 2000 ms | 3 of 3 | 393.052 ms; 371.490 ms; 380.241 ms |
| triggers-steady-100000 | the_handover_check_on_a_converged_fleet_stays_within_the_statement_timeout | slowest EXISTS check (what the handover asks under its lock) when every trigger has a schedule <= 4000 ms | 3 of 3 | 340.091 ms; 347.197 ms; 344.439 ms |
| triggers-steady-15000 | a_window_stays_within_a_second | slowest window of the sweep <= 1000 ms | 3 of 3 | 43.997 ms; 45.998 ms; 48.607 ms |
| triggers-steady-15000 | a_window_stays_within_the_statement_timeout | slowest window of the sweep <= 4000 ms | 3 of 3 | 43.997 ms; 45.998 ms; 48.607 ms |
| triggers-steady-15000 | the_count_stays_within_the_statement_timeout | slowest convergence count <= 4000 ms | 3 of 3 | 45.593 ms; 41.174 ms; 43.019 ms |
| triggers-steady-15000 | the_count_stays_within_two_seconds | p95 of the count <= 2000 ms | 3 of 3 | 42.366 ms; 39.155 ms; 38.261 ms |
| triggers-steady-15000 | the_handover_check_on_a_converged_fleet_stays_within_the_statement_timeout | slowest EXISTS check (what the handover asks under its lock) when every trigger has a schedule <= 4000 ms | 3 of 3 | 28.974 ms; 30.277 ms; 30.209 ms |
| triggers-steady-20000 | a_window_stays_within_a_second | slowest window of the sweep <= 1000 ms | 3 of 3 | 56.878 ms; 56.385 ms; 53.062 ms |
| triggers-steady-20000 | a_window_stays_within_the_statement_timeout | slowest window of the sweep <= 4000 ms | 3 of 3 | 56.878 ms; 56.385 ms; 53.062 ms |
| triggers-steady-20000 | the_count_stays_within_the_statement_timeout | slowest convergence count <= 4000 ms | 3 of 3 | 52.850 ms; 46.090 ms; 44.475 ms |
| triggers-steady-20000 | the_count_stays_within_two_seconds | p95 of the count <= 2000 ms | 3 of 3 | 45.327 ms; 43.448 ms; 42.320 ms |
| triggers-steady-20000 | the_handover_check_on_a_converged_fleet_stays_within_the_statement_timeout | slowest EXISTS check (what the handover asks under its lock) when every trigger has a schedule <= 4000 ms | 3 of 3 | 49.784 ms; 41.198 ms; 46.058 ms |
| triggers-steady-5000 | a_window_stays_within_a_second | slowest window of the sweep <= 1000 ms | 3 of 3 | 10.444 ms; 13.707 ms; 10.731 ms |
| triggers-steady-5000 | a_window_stays_within_the_statement_timeout | slowest window of the sweep <= 4000 ms | 3 of 3 | 10.444 ms; 13.707 ms; 10.731 ms |
| triggers-steady-5000 | the_count_stays_within_the_statement_timeout | slowest convergence count <= 4000 ms | 3 of 3 | 9.325 ms; 10.084 ms; 9.054 ms |
| triggers-steady-5000 | the_count_stays_within_two_seconds | p95 of the count <= 2000 ms | 3 of 3 | 8.681 ms; 9.865 ms; 8.730 ms |
| triggers-steady-5000 | the_handover_check_on_a_converged_fleet_stays_within_the_statement_timeout | slowest EXISTS check (what the handover asks under its lock) when every trigger has a schedule <= 4000 ms | 3 of 3 | 1035.343 ms; 1039.087 ms; 1032.999 ms |
| triggers-steady-50000 | a_window_stays_within_a_second | slowest window of the sweep <= 1000 ms | 3 of 3 | 61.466 ms; 62.297 ms; 55.783 ms |
| triggers-steady-50000 | a_window_stays_within_the_statement_timeout | slowest window of the sweep <= 4000 ms | 3 of 3 | 61.466 ms; 62.297 ms; 55.783 ms |
| triggers-steady-50000 | the_count_stays_within_the_statement_timeout | slowest convergence count <= 4000 ms | 3 of 3 | 229.059 ms; 239.195 ms; 220.905 ms |
| triggers-steady-50000 | the_count_stays_within_two_seconds | p95 of the count <= 2000 ms | 3 of 3 | 216.223 ms; 225.854 ms; 218.945 ms |
| triggers-steady-50000 | the_handover_check_on_a_converged_fleet_stays_within_the_statement_timeout | slowest EXISTS check (what the handover asks under its lock) when every trigger has a schedule <= 4000 ms | 3 of 3 | 118.529 ms; 114.494 ms; 144.714 ms |

### Load of the machine (1-minute load average, before / after each repetition: lowest-highest)

| scenario | N | before / after |
|---|---|---|
| batch-one-big-fifteen-small-30 | 3 | 4.7-5.6 / 5.7-8.7 |
| batch-one-job-default-rate-120 | 3 | 4.6-6.2 / 4.4-6.4 |
| batch-one-job-default-rate-120-poll50 | 3 | 17.6-24.1 / 15.2-22.1 |
| batch-one-job-highest-rate-1000 | 3 | 4.0-5.2 / 4.1-4.9 |
| batch-selection-and-drain-1000 | 3 | 4.5-6.4 / 4.4-6.6 |
| batch-selection-and-drain-10000 | 3 | 4.4-6.6 / 6.4-6.9 |
| batch-sixteen-jobs-default-rate-300 | 3 | 5.3-6.7 / 5.0-7.2 |
| batch-sixteen-jobs-highest-rate-500 | 3 | 4.2-5.0 / 4.5-5.0 |
| claim-thirty-two-way-50 | 3 | 5.9-8.8 / 6.3-7.4 |
| firing-instant-1000 | 3 | 5.4-11.3 / 4.3-7.6 |
| firing-instant-10000 | 3 | 4.3-7.6 / 4.8-5.5 |
| firing-instant-10000-round1000-batch500 | 3 | 4.6-13.6 / 4.6-5.5 |
| firing-instant-two-replicas-1000 | 3 | 4.3-8.6 / 4.1-5.8 |
| firing-instant-two-replicas-10000 | 3 | 4.1-5.8 / 5.0-7.3 |
| firing-spread-1000 | 3 | 4.3-4.9 / 3.1-7.1 |
| firing-spread-10000 | 3 | 3.1-7.1 / 4.3-4.8 |
| reset | 3 | 6.1-7.3 / 6.1-7.0 |
| triggers-backlog-10000 | 3 | 2.8-4.8 / 4.0-5.6 |
| triggers-backlog-100000 | 3 | 4.0-5.6 / 2.7-3.9 |
| triggers-steady-10000 | 3 | 4.6-5.1 / 4.0-4.7 |
| triggers-steady-100000 | 3 | 4.0-4.7 / 4.8-5.2 |
| triggers-steady-15000 | 3 | 3.2-6.2 / 3.2-6.0 |
| triggers-steady-20000 | 3 | 13.8-23.6 / 13.1-21.9 |
| triggers-steady-5000 | 3 | 3.2-4.1 / 3.2-6.2 |
| triggers-steady-50000 | 3 | 13.1-21.9 / 11.8-19.0 |

### Repetitions and their exit status (0 = every bound held)

| scenario | repetition | exit status | seconds |
|---|---|---|---|
| firing-spread | 1 | 101 | 361 |
| firing-spread | 2 | 101 | 353 |
| firing-spread | 3 | 101 | 360 |
| firing-two-replicas | 1 | 101 | 208 |
| firing-two-replicas | 2 | 101 | 177 |
| firing-two-replicas | 3 | 101 | 178 |
| batch-one-default | 1 | 101 | 124 |
| batch-one-default | 2 | 101 | 127 |
| batch-one-default | 3 | 101 | 127 |
| batch-one-highest | 1 | 101 | 21 |
| batch-one-highest | 2 | 101 | 22 |
| batch-one-highest | 3 | 101 | 23 |
| batch-selection | 1 | 0 | 68 |
| batch-selection | 2 | 0 | 68 |
| batch-selection | 3 | 0 | 69 |
| batch-16-default | 1 | 101 | 95 |
| batch-16-default | 2 | 101 | 95 |
| batch-16-default | 3 | 101 | 96 |
| batch-16-highest | 1 | 101 | 151 |
| batch-16-highest | 2 | 101 | 152 |
| batch-16-highest | 3 | 101 | 152 |
| batch-big-and-small | 1 | 0 | 18 |
| batch-big-and-small | 2 | 0 | 18 |
| batch-big-and-small | 3 | 0 | 19 |
| claim | 1 | 0 | 30 |
| claim | 2 | 0 | 30 |
| claim | 3 | 0 | 29 |
| reset | 1 | 101 | 425 |
| reset | 2 | 101 | 423 |
| reset | 3 | 101 | 409 |
| batch-one-default-poll50 | 1 | 101 | 13 |
| batch-one-default-poll50 | 2 | 101 | 13 |
| batch-one-default-poll50 | 3 | 101 | 12 |
| firing-instant-round1000 | 1 | 101 | 263 |
| firing-instant-round1000 | 2 | 101 | 242 |
| firing-instant-round1000 | 3 | 101 | 235 |
| triggers-steady | 1 | 0 | 61 |
| triggers-steady | 2 | 0 | 64 |
| triggers-steady | 3 | 0 | 55 |
| triggers-backlog | 1 | 0 | 378 |
| triggers-backlog | 2 | 0 | 382 |
| triggers-backlog | 3 | 0 | 376 |
| triggers-steady-mid | 1 | 0 | 21 |
| triggers-steady-mid | 2 | 0 | 21 |
| triggers-steady-mid | 3 | 0 | 19 |
| firing-instant | 1 | 101 | 311 |
| firing-instant | 2 | 101 | 297 |
| firing-instant | 3 | 101 | 297 |
| triggers-steady-small | 1 | 0 | 30 |
| triggers-steady-small | 2 | 0 | 30 |
| triggers-steady-small | 3 | 0 | 26 |


## Failures, and what they mean

A repetition that exits with 101 broke at least one bound; a bound that held is not listed. Which ones broke, in how many of the
three repetitions, and where it is explained:

| Scenario | Bound that broke | Repetitions | Observed (median) | What it means |
| --- | --- | --- | --- | --- |
| firing-instant-10000, -two-replicas-10000, -spread-10000, -round1000 | p95 of the lag within 60 s | 3 of 3 each | 157 s, 69.8 s, 100 s, 157 s | A replica starts about 61 runs a second; a burst of 10,000 is late by that much (G2044). The round ceilings do not change it |
| batch-one-default-rate-120 | reaches 80% of the rate asked | 3 of 3 | 0.98 items/s for 20 asked | The loop's poll bounds a lone job at one item per poll (G2043) |
| batch-one-default-rate-120-poll50 | reaches 80% of the rate asked | 3 of 3 | 13.4 items/s for 20 asked | With a poll of 50 ms the job gets 67% of its rate: the poll is the cause, the slot (50 ms) and the loop's overhead take the rest. The machine's load was 15 to 24 |
| batch-one-job-highest-rate-1000 | reaches 80% of the rate asked | 3 of 3 | 74.7 items/s for 200 asked | One replica applies an item in about 13 ms; 200 a second is above what it can do |
| batch-sixteen-jobs-default-rate-300, -highest-rate-500 | every job reaches half its rate | 3 of 3 each | 4.5 and 4.7 items/s | The aggregate is the replica's (72 and 75 a second), shared fairly (G2044) |
| reset | holds the source row for no more than 1 s | 3 of 3 (5,002 and 20,002 events) | 4.65 s and 4.70 s | The restore writes the events one statement at a time inside one transaction (G2046) |
| reset | a reset of a journal four times the ceiling within 5 s | 3 of 3 | 6.0 s | The whole journal is read and replayed before the transaction starts (G2047) |

Everything else held in 3 of 3: the claim (exactness, p99 of a waiting start within 1 s, overhead within 1.5x at the median and 2x at the
tail), the selection and the drain of 1,000 and 10,000 runs, the small jobs beside a big one, the equal jobs ending together, no job
waiting more than 10 s for its first item, the refusal at 20,000 events within 1 s, the reset of the largest accepted prefix within 5 s,
every statement of the trigger scenarios within the 4 s timeout (the count within 2 s, a window within 1 s), no step refused or failed,
and the reconciliation keeping half its advertised pace when its rounds are full. **A bound that held with little margin** is the reset
within 5 s (0.15 to 0.24 s) and the handover check within 4 s at 10,000 triggers (1.1 s).

## Ceilings the data supports or contradicts

No ceiling was changed (D2112). Each row says what the data says about it and what would change it: a new decision (the next free
number after D2115) that quotes the figures here, with its tests in both roles.

| Ceiling (plan, "Finite ceilings") | Default / hard | What the data says | Verdict | Would change it |
| --- | --- | --- | --- | --- |
| Schedule actions per pass round (`SAGAT_SCHEDULES_ROUND_ACTIONS`), schedules looked at (`_BATCH`) | 100 / 1,000 | It bounds the length of a round (1.6 s at 100, 8.3 s with 500 schedules and 1,000 actions), not the rate (61 a second either way) | Supports it as a bound on a round and on the fairness search; contradicts the plan's reading of it as the answer to "many due at one instant" | G2044: capacity statement, or a design for firing in parallel |
| Schedules per namespace | 200 / 10,000 | 10,000 were made through the service in 32 s (310 a second) and all fired | Supports the ceiling | |
| Active runs per namespace | 10,000 | Met by a burst of 10,000 schedules; the pass refuses and retries without end | Works as designed; undocumented | G2045: runbook |
| Batch item rate per job | 20 / 200 a second | A lone job at the default gets 1 a second, at the highest 75; sixteen jobs get 4.5 each; one replica applies 73 to 75 in all | Contradicts both as a description of what a job gets; 200 is above what one replica applies | G2043 (the loop) and G2044 (capacity) |
| Active batch jobs per namespace | 2 / 16 | Sixteen at once end together and none waits more than 4.4 s for its first item | Supports 16 | |
| Items per job (`maxItems`) | 10,000 / 100,000 | Selection of 10,000 in 0.32 s, drain in 1.3 s; 100,000 not measured | Supports 10,000; 100,000 unmeasured | G2050 |
| Reset prefix (`RESET_PREFIX_EVENTS`) | 5,000 / 20,000 | 4.65 s of lock at 5,000 (0.93 ms an event), 6.0 s for the whole reset on a long journal; about 19 s at 20,000 by extension | Supports 5,000 as the most the lock time should allow; contradicts 20,000 as safe unless the restore is paged | G2046, G2047 |
| Business-id contention (32-way, one hot identifier) | one run | Exactly one run in 150 rounds; p99 of a waiting start 50 ms; claim +7% | Supports | |
| Reconciliation: window 2,000, batch 200, 20 a tenant, 100 writes a second | | Windows 7 ms to 632 ms; sweep of 100,000 triggers 9.1 s; drain 30 to 77 writes a second | Supports the windows and the bounds on statements; contradicts 100 a second as a pace (it is a ceiling the pause makes unreachable) | G2048 |
| The check under the handover's lock | 4 s statement timeout | 2.85 s at 10,000 triggers, 1.0 s at 5,000, under 0.3 s from 15,000 | Holds in all 18 repetitions; depends on the plan | G2049 |

## Not measured

The selection of 100,000 runs; the cross-tenant fairness of the batch and the schedule passes (one tenant in every scenario but the
trigger ones); the batch pass on two replicas; the claim across processes (one process with a pool of 48 held the 32); the HTTP API in
front of any of it; workers that run what is started; a reset with reapplied signals or Updates, with a business id, or `CANCEL_SOURCE`
and its pass; a reset of more than 5,000 events (no code path accepts it); backfills, catch-up storms and the release of delayed
starts; PostgreSQL on another machine or another version; a quiet machine; and any scenario longer than the hour it takes here
(G2050).

## Reproducing

```sh
export CARGO_TARGET_DIR=<a directory outside the repository> CARGO_BUILD_INCREMENTAL=false
SAGAT_PERF_REPS=3 SAGAT_PERF_OUT=<a directory> tests/performance/schedules-bulk-operations/run.sh
python3 -I tests/performance/schedules-bulk-operations/summarize.py <the directory>   # the tables above
```

The tables above are the output of `summarize.py` over the fragments of seven invocations of that script with the same sources, the
first without the later additions listed under "How the run went": the whole script (13:46 to 15:25, local time; it was stopped while
the trigger backlog was on its first repetition), then `SAGAT_PERF_ONLY=poll50`, `=round1000` and `=triggers` (15:42 to 16:18), then
`SAGAT_PERF_ONLY==triggers-steady-mid` and `==firing-instant` (16:19 to 16:36; a leading `=` selects one label exactly) and
`SAGAT_PERF_ONLY==triggers-steady-small` (16:36). The scenarios of the first invocation that the tables use are the firing at spread and
with two replicas, the six batch scenarios, the claim and the reset; the rest come from the later ones.


## After the fixes of 2026-11-09

Everything above is what the code did at commit `345dd70e`, and it is not rewritten: the tables, the bounds and the findings stay as they were measured. This section is what the same benchmark measured after the two defects it found were fixed (G2043 and G2049; decisions D2116 and D2117 in `specs/018-schedules-bulk-operations/research.md`), and it says what each change did and what it did not do. **These are, as above, observations of one machine on one day, not promises** (NFR-018-005).

**What changed in the product.** (1) *The batch loop (G2043).* A round that found nothing due now asks the database for the soonest slot of an active job of the shards the replica holds (one statement of at most 64 index probes, answered by the database's clock), and `run_batches` rests the lesser of its poll and that time (never less than 1 ms, stop-aware) instead of the whole poll; a round that did work is followed at once, as before; the executor, the fairness and the ceilings are untouched. (2) *The drift view (G2049).* `schedule_trigger_drift` looks the schedule of each trigger up by `trigger_id` (a `LATERAL ... LIMIT 1` over `schedules_trigger_uniq`) and reads the singleton `schedule_trigger_handover` by a scalar subquery, so the check the handover makes under its lock has one plan, an index probe for each trigger, whatever the statistics say (in place in `20270114000000_trigger_handover`, which has not been applied outside tests or released).

**Method.** The same `run.sh`, scenarios, image (by digest) and settings as above, release build, runtime role, N = 3, one label of `run.sh` at a time (`SAGAT_PERF_ONLY==batch-one-default`, `=triggers-steady-small`, `=triggers-steady`, `=triggers-steady-mid`, `=batch-big-and-small`, `=batch-one-highest`, `=batch-16-default`, and again `=batch-one-default`), on 2026-11-09 from 20:11 to 20:25 UTC (17:11 to 17:25 local); the commit is `11d4bec1` with the working tree of the follow-up (every fragment says `dirty: true`, and the files of the product that differ from `345dd70e` are `crates/scheduler/src/batches.rs`, `crates/application/src/batch.rs`, `crates/infra-postgres/src/batch.rs`, `apps/execution-engine/src/loops.rs` and the migration; the benchmark's own files are as they were). **The firing, the claim, the reset, the selection and drain scenarios were not rerun**: nothing they read changed. The reconciliation's backlog is in its own subsection below. The tables are the output of `summarize.py` over the fragments of each run, unedited, apart from the "before" columns, which are quoted from the tables above.

**Load of the machine.** The machine ran other agents' work and the Docker VM about thirty containers of other projects, as before. The first run of the lone job (`batch-one-default`) began seconds after the release build's last link and the one-minute load average was **20.6 to 34.1** (the poll50 variant above was at 15 to 24); it was therefore run again on a quieter machine (**4.8 to 5.7**) and both are shown. The other scenarios ran at 5.5 to 26.7 (the sixteen jobs began a repetition at 26.7, a rebuild having run before it; the one-minute figure before and after every repetition is in the table at the end of this section).

### The lone job at the default rate (G2043)

| | before (the run above) | after, first run (load 20.6 to 34.1) | after, second run (load 4.8 to 5.7) |
| --- | --- | --- | --- |
| asked, items a second | 20 | 20 | 20 |
| **achieved, items a second** | **1.0** [1.0 - 1.0] (0.97 to 0.98) | **18.3** [18.3 - 18.3] | **18.4** [18.4 - 18.4] |
| gap between two items, p50 | 1,036 ms [1,022 - 1,037] | 54.6 ms [54.5 - 54.7] | 54.4 ms [54.3 - 54.4] |
| gap p95 / largest | 1,205 / 1,229 ms | 56.2 / 57.6 ms | 56.0 / 57.3 ms |
| wall time of 120 runs | 122 s | 6.6 s | 6.6 s |
| item duration p50 / p95 | 11.7 / 17.5 ms | 8.7 / 10.9 ms | 8.8 / 12.0 ms |
| bound "reaches 80% of the rate asked" (16 a second) | broken in 3 of 3 | held in 3 of 3 | held in 3 of 3 |
| bound "never more than 110%" (22 a second) | held in 3 of 3 | held in 3 of 3 | held in 3 of 3 |

The loop was the whole difference: a job that asks for 20 items a second now gets 92% of that and not 5% (the variant with a poll of 50 ms above gave 13.4 and is obsolete). The 8% that is missing is not the poll: the gap is 54.4 ms for a slot of 50 ms, and the extra 4.4 ms is the round that finds nothing due and asks for the next slot (a clock read, the due scan and the new statement) and the wake-up of the loop, which D2116 and G2051 say a round that knew its own slot could save. Two runs a load of 25 apart gave the same figure within 0.5%.

### What must not change did not (fairness and the replica's capacity)

| Scenario | Metric | before | after |
| --- | --- | --- | --- |
| sixteen jobs at the default rate (300 runs each) | aggregate, items a second | 72.5 [71.7 - 72.8] | 73.5 [71.9 - 73.6] |
| | slowest job, items a second | 4.5 [4.5 - 4.5] | 4.6 [4.5 - 4.6] |
| | last job's end over the first's | 1.01 [1.00 - 1.01] | 1.00 [1.00 - 1.01] |
| | the job that waited longest for its first item | 0.4 s | 0.4 s (0.375 to 0.433) |
| | bound "every job reaches half its rate" (10 a second) | broken in 3 of 3 | broken in 3 of 3 (the replica's capacity, G2044) |
| | bounds "end together" and "no job waits 10 s" | held in 3 of 3 | held in 3 of 3 |
| one job of 900 runs beside fifteen of 30 | aggregate, items a second | 73.6 [71.8 - 73.9] | 72.0 [70.2 - 72.5] |
| | last job's end over the first's | 1.03 [1.03 - 1.03] | 1.04 [1.04 - 1.06] |
| | end of each job over its allowance (bound <= 1) | 0.464, 0.451, 0.453 | 0.460, 0.457, 0.468 |
| | the job that waited longest for its first item | 0.337 to 0.363 s | 0.328 to 0.445 s |
| a lone job at the highest rate (200 a second asked, 1,000 runs) | achieved, items a second | 74.7 [73.5 - 77.2] | 73.1 [72.6 - 75.0] |
| | bound "reaches 80% of the rate asked" (160 a second) | broken in 3 of 3 | broken in 3 of 3 (13 ms an item is the replica's capacity) |

With several jobs, or one job faster than the replica can apply, the loop is never idle, so the change does not touch them: one replica still applies about 73 items a second to all its jobs, shares them fairly, and the bounds that were broken because of that stay broken. They are the replica's capacity (G2044), not the loop.

### The check under the handover's lock (G2049)

**What the plan analysis found** (`EXPLAIN (ANALYZE, BUFFERS)` on the database of the scenario at 10,000 triggers, the benchmark's image and settings, run by hand after the scenario with the tables as the benchmark leaves them):

- *The statistics of the bulk insert are no part of it.* The three tables were analyzed after the load and again after the stragglers were given a schedule (`last_analyze` a few seconds before; `n_mod_since_analyze` 0 for `workflow_triggers` and 2 for `schedules` and `schedule_revisions`). What had no statistic was the singleton `schedule_trigger_handover` (`reltuples = -1`; it is never analyzed, in a deployment as here: one row inserted by a migration and updated once).
- *The plan:* `Nested Loop Left Join ... Join Filter: (s.trigger_id = t.id)` over a `Materialize` of a sequential scan of `schedules`, `Rows Removed by Join Filter: 99990000`, 30,497 buffers, 3.5 s under `ANALYZE` (3.15 s in the harness run that made the database). The planner estimated 6,069,500 rows for the join: 10,000 triggers times 610 rows for the unanalyzed singleton (its default for a table of unknown size) times 0.995 (its estimate of `CASE ... IS NOT NULL`). An `EXISTS` is planned for its first row, so with six million estimated rows the cost of a plan is its start-up cost, and the nested loop, which has none (estimated total cost 1.7 million), beat a hash join, which must first read all of `schedules` (a scan costed at 423) before it returns a row.
- ***`ANALYZE schedule_trigger_handover` alone, nothing else, brought the same statement to 21.5 ms*** with `Index Scan using schedules_trigger_uniq` (60,174 buffers): from 15,000 triggers up the planner reaches the same plan unaided, which is why the cost looked like it followed the size.
- *The new view, with the singleton still unanalyzed:* `Nested Loop Left Join` over `Seq Scan on workflow_triggers`, a `Limit` over `Index Scan using schedules_trigger_uniq` and `Index Scan using schedule_revisions_pkey` for each trigger (`loops=10000`), the singleton an `InitPlan` that is never executed on a fleet with no paused schedule, 60,173 buffers, **19.6 ms**; no plan that scans `schedules` again is possible in that form.

**The check on a converged fleet, every trigger with a schedule** (`SELECT schedule_trigger_unconverged()` as the runtime role, median of the three repetitions, in brackets the smallest and the largest; five samples a repetition; before = the tables above):

| Triggers | before, p50 | before, slowest sample (median of the repetitions) | after, p50 | after, slowest sample (median of the repetitions) |
| --- | --- | --- | --- | --- |
| 5,000 | 1,009 ms [999 - 1,014] | 1,035 ms | **10 ms** [9 - 10.3] | 11.3 ms [10.4 - 11.3] |
| 10,000 | 2,853 ms [2,851 - 2,856] | 2,880 ms | **19.7 ms** [19.0 - 20.8] | 22.2 ms [20.1 - 22.8] |
| 15,000 | 28.0 ms [27.3 - 28.0] | 30.2 ms | 28.2 ms [27.5 - 32.2] | 30.1 ms [29.1 - 33.8] |
| 20,000 | 43.5 ms [38.2 - 48.9] | 46.1 ms | 39.2 ms [38.2 - 46.4] | 46.6 ms [39.4 - 47.8] |
| 50,000 | 111 ms [108 - 135] | 119 ms | 112 ms [112 - 121] | 122 ms [118 - 138] |
| 100,000 | 280 ms [269 - 282] | 344 ms | **263 ms** [263 - 271] | **324 ms** [291 - 335] |

Where the old plan was already an index plan (15,000 and above) the new one costs the same, as it should: it is the same lookups. At 5,000 and 10,000 it is 100 and 140 times faster. **The check stays under 300 ms at the median at every size; its slowest single sample at 100,000 triggers was 335 ms**, 12% over the 300 ms that was asked for and 3% under what the old index plan gave there (344 ms): the cost is now linear, about 2.6 microseconds a trigger (G2052), and 4 s would be reached at about 1.5 million triggers by extrapolation, which nobody measured. The statement timeout of 4 s holds with a margin of more than ten at 100,000 and of two orders of magnitude at the sizes the planner used to get wrong; the 2 s lock timeout is the writers' wait for the lock and is not touched. The bound "the handover check on a converged fleet stays within the statement timeout" held in 3 of 3 repetitions at every size, before and after; what changed is the margin (2,890 ms of 4,000 at 10,000 before, 22.8 ms after).

**The other statements that read the view** (median of three repetitions; "count" is `schedule_trigger_convergence`, the gauge's and the dry run's, at the state of the steady scenario; "window" is one window of the reconciliation, 2,000 triggers; "view count" is an unbounded `SELECT count(*)` of the view, which nothing in the product issues):

| Triggers | count, before → after | window p50, before → after | whole sweep, before → after | `count(*)` of the view, before → after |
| --- | --- | --- | --- | --- |
| 5,000 | 7 → 11.0 ms | 10.2 → 10.9 ms | 0.0 → 0.0 s | 5 → 10 ms |
| 10,000 | 10.1 → 19.0 ms | 13.3 → 13.7 ms | 0.1 → 0.1 s | 19.3 → 30.2 ms |
| 15,000 | 28.1 → 31.0 ms | 20.5 → 15.4 ms | 0.2 → 0.1 s | 24.5 → 41.2 ms |
| 20,000 | 29.2 → 43.2 ms | 48.5 → 15.3 ms | 0.5 → 0.2 s | 29.9 → 47.7 ms |
| 50,000 | 195 → 119 ms | 49.3 → 32.3 ms | 1.3 → 0.7 s | 177 → 236 ms |
| 100,000 | 314 → 288 ms | 86.4 → 48.3 ms | 9.1 → 2.4 s | 238 → 395 ms |

**What it cost, said plainly.** The windows of the reconciliation and a whole sweep are faster from 15,000 triggers up (a sweep of 100,000: 9.1 s to 2.4 s), because a window of 2,000 triggers is now 2,000 index probes and no longer a join of the whole window against `schedules`. The count of the gauge is slower up to 20,000 triggers (11 to 43 ms against 7 to 29 ms: a hash join over a small fleet beats the probes) and faster from 50,000 up (119 ms against 195, 288 against 314); it is a bounded statement of tens of milliseconds at the sizes where it is slower. The unbounded `count(*)` of the view, the statement an operator might type, is 1.3 to 2 times slower at every size (395 ms against 238 ms at 100,000); nothing in the product reads the view unbounded (the gauge and the dry run read it through a `LIMIT`, the reconciliation through a window, the handover through `EXISTS`).

### Bounds, before and after

| Scenario | Bound | before | after |
| --- | --- | --- | --- |
| lone job at the default rate | reaches 80% of the rate asked | broken in 3 of 3 | **held in 3 of 3** (twice) |
| lone job at the default rate | never more than 110%; selection is not the limit | held | held |
| handover check at 5,000 to 100,000 triggers | within the 4 s statement timeout | held in 3 of 3 at the six sizes (1.0 s at 5,000 and 2.9 s at 10,000 left 75% and 28% of the margin) | held in 3 of 3 at the six sizes (the slowest sample at any size 335 ms) |
| sixteen jobs; a lone job at the highest rate | every job half its rate; the highest rate reached | broken in 3 of 3 | broken in 3 of 3: the replica's capacity |
| the rest of the scenarios rerun (fairness, first item, equal jobs end together, small beside big) | | held in 3 of 3 | held in 3 of 3 |

The bounds that fail now fail for the reason the report gave for them above (a replica applies about 73 items and starts about 61 runs a second), not because of the loop or the plan.

### The reconciliation's backlog, once

The windows of the reconciliation read the same view, so the scenario that drains a backlog after an upgrade (`triggers-backlog`: 10,000 and 100,000 triggers with no schedule, two tenants, the engine's own loop for 60 s) was run **once** (N = 1, `SAGAT_PERF_REPS=1`, 6.5 minutes, load 3.1 to 4.8): a point and not a median, so the figures have no brackets and a difference of a few percent between the two runs is not a finding.

| | before (N = 3, median) | after (N = 1) |
| --- | --- | --- |
| 100,000: convergence count with every trigger unconverged, p50 / largest | 193 ms / 259 ms | **59.9 ms / 82.5 ms** |
| 100,000: first window of a sweep, p50 / largest | 46.1 ms / 336 ms | **11.9 ms / 16.9 ms** |
| 10,000: convergence count p50 / first window p50 | 6 ms / 7 ms | 7 ms / 8 ms |
| writes a second, 100,000 triggers, the defaults (20 an entry a tenant) | 62.0 [61.9 - 62.6] | 66.7 |
| writes a second, 100,000 triggers, the per-tenant cap at 100 | 77.3 [74.0 - 77.3] | 77.3 |
| writes a second, 10,000 triggers, the defaults / the cap at 100 | 30.0 [27.4 - 30.0] / 68.8 [68.2 - 70.2] | 26.7 / 69.0 |
| the backlog of 100,000 drains in (extrapolated) | 26.9 minutes (defaults), 20.8 (cap at 100) | 25.0 minutes, 20.7 |
| bounds (the count and a window within the 4 s timeout, a full round keeps half the advertised pace, no write failed) | held in 3 of 3 | held (exit 0) |

Nothing is slower; the count and the first window of a backlog are about three to four times faster at the median at 100,000 (twenty times at the largest first window) because a trigger with no schedule is now one probe of an index that finds nothing. The drain is bounded by the pause after a round that writes, not by the reads, so its pace is what G2048 says (21 to 27 minutes for 100,000 triggers of two tenants; this run: 25 and 21).

### Repetitions and their exit status (0 = every bound held), and the load of the machine

| Scenario (`run.sh` label) | Repetitions: exit status | One-minute load average before / after each repetition (lowest - highest) |
| --- | --- | --- |
| batch-one-default, first run | 0, 0, 0 | 23.6 - 34.1 / 20.6 - 29.1 |
| batch-one-default, second run | 0, 0, 0 | 4.9 - 5.4 / 4.8 - 5.7 |
| batch-big-and-small | 0, 0, 0 | 6.4 - 7.9 / 6.8 - 7.9 |
| batch-one-highest | 101, 101, 101 (the highest rate is not reached: the replica's capacity) | 6.0 - 6.8 / 5.9 - 6.6 |
| batch-16-default | 101, 101, 101 (every job does not reach half its rate: the replica's capacity) | 6.3 - 26.7 / 5.5 - 8.7 |
| triggers-steady-small (5,000 and 15,000 triggers) | 0, 0, 0 | 14.7 - 19.2 / 13.7 - 17.7 |
| triggers-steady (10,000 and 100,000) | 0, 0, 0 | 9.0 - 13.4 / 8.6 - 13.4 |
| triggers-steady-mid (20,000 and 50,000) | 0, 0, 0 | 7.3 - 9.8 / 7.3 - 9.8 |
| triggers-backlog (N = 1) | 0 | 3.1 - 4.8 |

### Reproducing the addendum

```sh
export CARGO_TARGET_DIR=<a directory outside the repository> CARGO_BUILD_INCREMENTAL=false
for label in batch-one-default triggers-steady-small triggers-steady triggers-steady-mid batch-big-and-small batch-one-highest batch-16-default; do
  SAGAT_PERF_REPS=3 SAGAT_PERF_ONLY="=$label" SAGAT_PERF_OUT=<a directory for $label> tests/performance/schedules-bulk-operations/run.sh
done
SAGAT_PERF_REPS=1 SAGAT_PERF_ONLY==triggers-backlog SAGAT_PERF_OUT=<a directory> tests/performance/schedules-bulk-operations/run.sh
python3 -I tests/performance/schedules-bulk-operations/summarize.py <each directory>   # the tables above
```

(A leading `=` in `SAGAT_PERF_ONLY` selects one label exactly, so `batch-one-default` does not also run `batch-one-default-poll50`.) The tests of the two changes are in `apps/execution-engine/src/loops.rs` (the loop under paused time) and `tests/integration/schedules-bulk-operations/` (`batch_pass.rs`: the store, the pass and the engine's own loop; `trigger_plan.rs`: the plan of the check in three states of the statistics and the equality of the view with its previous definition), in both roles.


## After the review fixes of 2026-11-09

Two independent reviews of the change above found two ways in which the stall of G2043 would have come back at random, by reading the code (neither was reproduced, and the figures above do not show them: G2056). They are fixed (D2123 to D2125 in `research.md`), and the statement that the first addendum describes ("a round that found nothing due asks the database for the soonest slot, one statement of at most 64 index probes") is no longer how it works.

- **A slot could come between the two statements of a round.** The scan of the due jobs found a job's slot a millisecond ahead, the round did nothing, and by the time it asked for the next slot the slot had come, so it answered nothing and the loop slept its whole poll while an item was due; the exposure is highest at 100 to 200 items a second. The slot ahead is now the second half of the scan of the due jobs itself: one statement that reads the database's clock once, at its start; a slot that comes while it runs, or after it, is ahead of that instant and is a zero, which the loop floors to 1 ms. There is no second statement (one round trip less for an idle round, 64 index probes more for every round, in the same statement).
- **The executor and the scan can disagree about the time.** The scan says a job is due by the database's clock; the executor decides by its own (`DatabaseClock`, behind by up to half a round trip and by what it drifted since). A due job the executor found not due yet made the round idle with nothing to wake for, and the loop slept a whole poll. A round with a due job that a step found nothing to do for now comes back in 10 ms (at most a hundred rounds a second, and only while the clocks disagree).

The scenarios that the change touches were run again with the same `run.sh`, the same release build profile, image and settings, N = 3 each, on 2026-11-09 between 21:56 and 22:05 UTC (18:56 to 19:05 local), the tree of the review fixes (every fragment says `dirty: true`):

| Scenario | Metric | the first design (above) | after the review fixes |
| --- | --- | --- | --- |
| lone job at the default rate, run on a machine at load 4.5 to 6.4 | achieved, items a second | 18.4 [18.4 - 18.4] | **18.3** [18.3 - 18.4] |
| | gap between two items, p50 / p95 | 54.4 / 56.0 ms | 54.6 / 56.0 ms |
| | bound "reaches 80%" (16 a second) | held in 3 of 3 | held in 3 of 3 |
| lone job at the default rate, run at load 22.7 to 34.3 (right after the build's last link) | achieved, items a second | 18.3 [18.3 - 18.3] | 18.3 [18.2 - 18.3], gap p50 54.7 ms |
| sixteen jobs at the default rate | aggregate, items a second | 73.5 [71.9 - 73.6] | 72.2 [71.8 - 72.9] |
| | slowest job; last job's end over the first's | 4.6; 1.00 | 4.5; 1.01 [1.00 - 1.01] |
| | the job that waited longest for its first item | 0.4 s | 0.4 s (0.368 to 0.427) |
| | bounds | the half-rate bound broken 3 of 3 (the replica's capacity), the others held | the same |
| one job of 900 runs beside fifteen of 30 (load 11 to 21) | aggregate, items a second | 72.0 [70.2 - 72.5] | 72.3 [66.9 - 72.3] |
| | last job's end over the first's | 1.04 [1.04 - 1.06] | 1.05 [1.04 - 1.06] |
| | bounds | held in 3 of 3 | held in 3 of 3 |
| a lone job at the highest rate (load 8 to 10.7) | achieved, items a second | 73.1 [72.6 - 75.0] | 74.5 [73.9 - 76.1] |
| | bound "reaches 80%" (160 a second) | broken in 3 of 3 (the replica's capacity) | the same |

The lone job stays at 18.3 items a second, as the first design gave (18.3 and 18.4), at both loads: the fold neither helped nor cost it, and the fairness and the capacity of the replica are as they were. The one repetition of one big job beside fifteen small ones that gave 66.9 items a second ran on a machine at a load of 11 to 21 (this scenario gave 71.8 to 73.9 at a load of 6 to 8, above), and no bound depends on the aggregate there; I did not rerun it on a quieter machine. Exit statuses: the lone job 0, 0, 0 in both runs; the small beside the big 0, 0, 0; the highest rate 101, 101, 101 and the sixteen jobs 101, 101, 101 (the replica's capacity, as above). The one-minute load average before and after every repetition: 25.9 to 34.3 and 22.7 to 29.6, then 4.5 to 6.4 and 4.5 to 6.2 (the lone job); 13.0 to 21.0 and 11.2 to 17.4 (small beside big); 9.4 to 10.7 and 8.0 to 10.5 (highest rate); 5.1 to 7.5 and 5.5 to 6.0 (sixteen jobs). The firing, the claim, the reset, the selection and drain, the reconciliation and the handover's check were not rerun: nothing they read changed in the review fixes except the comments of the view's migration.
