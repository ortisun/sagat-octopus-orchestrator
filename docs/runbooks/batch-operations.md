# Runbook: batch operations (feature 018)

For the people who run the platform and for operators who apply one action to many runs ("cancel everything stuck since Tuesday"). It says what a job does and promises, how to turn it on, how to read what became of each run, and what to look at when a job does not move.

Design: [ADR-0050](../adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md); decisions, gaps and owner questions: [`specs/018-schedules-bulk-operations/research.md`](../../specs/018-schedules-bulk-operations/research.md). Schedules are in [`schedules.md`](schedules.md).

## 1. What a job is and what it promises

A batch job applies **one action to many runs, one run at a time**, at a rate, in the background. It is created from the run list in the Studio (*Batch operation…*, with the filters you set) or over REST (`POST /namespaces/{id}/batch-operations`, preview first at `…/preview`), and followed at `/batches`.

- **Actions**: `CANCEL`, `SIGNAL`, `RERUN`, `RETRY_STEP`, `PAUSE`, `RESUME`, `MARK_MANUAL_REVIEW`. `UPDATE` and `REPIN` are refused with `422 BATCH_ACTION_UNAVAILABLE` in this build.
- **Which runs**: a filter (status, workflow, version, origin, schedule, a created-after and created-before) or a list of run ids. The job takes the runs that existed when it was created, in pages, after every transaction that was open then has ended, so a run committed by one of those is not missed. A filter by business-id prefix is refused (`BATCH_SELECTOR_UNSUPPORTED`): business identifiers exist and the run list filters by them, but the job's selection does not match them yet; list the runs by prefix and give the job their ids ([business-ids.md](business-ids.md)).
- **Whose authority**: the creator's, **read again at every item**. The permission to run batch jobs (`run:batch`: admin, operator) *and* the permission of the single action are needed to create the job and still needed when each item is applied. A creator who loses either one fails the job (`AUTHORITY_REVOKED`). A run the creator may not see is skipped (`HIDDEN_FROM_CREATOR`), never described.
- **Exactly the single action**: each item goes through the same use case a person would use for that run, under the key `batch:{job}:{run}`, so a repeat after a crash does nothing twice and the audit trail shows the same record as the single action.
- **What it does not promise**: a job is not a transaction. Cancelling a job stops what has not run; **what was already applied stays applied**. An effect and the record of its item are two committed steps under one key (O1951 in the research register is the owner's question whether they must be one): after a crash an item can say less than happened to its run, never more, and the repeat finds the effect done.

## 2. Limits

| Limit | Default | Hard | Note |
|---|---|---|---|
| runs in one job (`maxItems`) | 10,000 | 100,000 | the selection stops there |
| rate (`itemsPerSecond`) | 20 | 200 | one item per slot; a job that ran late never catches up in a burst; a ceiling, not what a job gets (section 2.1: one replica applies about 73 items a second to all its jobs) |
| active jobs (`batch_jobs`, per namespace) | 2 | per `quota_ceilings` | a job counts until it ends |
| active jobs (`tenant_batch_jobs`, per tenant) | 8 | per `quota_ceilings` | |
| retries of an item refused for now | 10 within 1 hour | 50 within 24 hours | quota, rate, lock wait, a full queue |
| reason | 8 to 2,000 characters | | plain text, no new line, kept with the job |
| how long a job is kept (`expiresAt`) | 30 days | 90 days | stamped on the job; nothing purges it yet (section 6) |

### 2.1 The rate a job really gets

The rate is a **ceiling, never a promise** (NFR-018-005 promises nothing: the figures below are measurements of one machine on one day, not a guarantee). A job applies at most one item per slot (`1 / itemsPerSecond`) and never catches up in a burst after it ran late; how many it applies is the lesser of that and what the replica that holds its shard can do:

- **A lone job at the default rate (20 a second)** applied **1.0 item a second before 2026-11-09** and applies **18.4 items a second** (18.3 on a machine at four times the load; the 1.6 missing are the idle round that follows each item (a clock read and the scan, which also says the slot ahead) and the wake-up of the loop, about 4 ms an item) after the change that makes the engine's loop wake when the next slot comes instead of sleeping the whole `SAGAT_BATCHES_POLL_MS` (G2043, D2116; the measurement is the addendum "After the fixes of 2026-11-09" of [the report](../../tests/performance/reports/schedules-bulk-operations.md)). The rate was bounded by the poll before: the executor applies at most one item of a job in a step, the next slot of a job is 50 ms after an item, and a loop that found nothing due slept a second.
- **One replica applies about 73 to 75 items a second to all its jobs together** (13 ms an item, 7.4 ms of it the action itself; release build, PostgreSQL in Docker, no worker, no HTTP), and shares them fairly: sixteen jobs of one size ended within 1% of each other, none waited more than 0.4 s for its first item at the default rate, and a job of 900 runs beside fifteen of 30 did not hold the small ones back. So **sixteen jobs at the default rate (320 a second asked) got about 4.5 items a second each**, and a job at the highest rate (200 a second) got about 75 a second: more than one replica can apply is not applied. A second replica holds other shards, so it applies the items of other jobs; the batch pass on two replicas was not measured (G2050).
- **To find how long a job will take**, divide its runs by `min(itemsPerSecond, 73 / the number of active jobs on the replica)`; 10,000 runs at the default rate are about 9 minutes alone (18.4 a second) and about 36 minutes among sixteen jobs (4.6 a second each). The selection of the runs (0.3 s for 10,000) and the drain of a cancelled job (1.3 s for 10,000 items) are never the limit.

## 3. Turning it on

Off by default, in two places that **must agree**. Turn it on in the engines first and then in the control plane, and turn it off in the control plane first: the same order as the switch of the reset of an open run (ADR-0081), so that every job that is accepted has an engine to run it. The registry that would refuse a mismatch is not built (ADR-0050 decision 9):

| Where | Setting | Default | What it does |
|---|---|---|---|
| control plane | `SAGAT_BATCHES_ENABLED` | `false` | serves the routes (otherwise `404 ROUTE_NOT_FOUND`) |
| engine | `SAGAT_BATCHES_ENABLED` | `false` | runs the jobs of the shards the replica holds |
| engine | `SAGAT_BATCHES_BATCH` | 100 (1 to 5,000) | due jobs looked at in a round |
| engine | `SAGAT_BATCHES_ROUND_ACTIONS` | 100 (1 to 1,000) | steps one round takes over all its jobs |
| engine | `SAGAT_BATCHES_POLL_MS` | 1000 | pause when a round found nothing; the scan of the due jobs also says when the soonest next slot of a running job of the shards the replica holds comes, and the loop wakes then if that is sooner, and a due job that the executor finds not due yet (its clock reads a hair before the database's) is looked at again in 10 ms (G2043, section 2.1) |

With the routes on and no engine running jobs, a job is accepted and stays `SELECTING`, holding its place against the active-job limit, until an engine with the flag runs it; a cancellation waits for one too (G1965). Turning the flag off in the control plane stops the routes, not the jobs already running: the engine's flag is the switch.

## 4. Reading a job

`GET /namespaces/{id}/batch-operations/{id}` gives the state and the counters; `…/items` the outcome of each run, with a reason, in pages and by outcome.

- **Job states**: `SELECTING` (finding the runs), `RUNNING`, `CANCELLING` (marking the runs that have not run), `COMPLETED`, `CANCELLED`, `FAILED` (with `AUTHORITY_REVOKED` or `SELECTION_FAILED`).
- **Item outcomes**: `SUCCEEDED`, `SKIPPED`, `FAILED`, `CANCELLED`, `DEFERRED` (refused for now, tried again with a growing delay), `PENDING`.
- **Why skipped**: `ALREADY_TERMINAL` (the run had ended), `NOT_APPLICABLE` (the action does not apply to the run's state), `NOT_WAITING` (no step was waiting for the signal), `OPERATION_NOT_DECLARED`, `RUN_CONTINUED` (the run was handed to a continuation), `HIDDEN_FROM_CREATOR`.
- **Why deferred**: `QUOTA_EXCEEDED`, `RATE_LIMITED`, `CONCURRENCY_QUEUE_FULL`, `LOCK_WAIT`. An item refused too often or for too long fails with the code of the last refusal (`errorCode`).
- The payload of a signal and the arguments of an update are charged content: a read says only that the job carries one, never its value or its digest.

## 5. When a job does not move

1. **`SELECTING` for a long time.** The selection waits for every transaction that was open when the job was created. Look in the database for a transaction that has been open a long time (`pg_stat_activity.xact_start`); ending it lets the job go on. This is not the scheduler being late and no replica added will change it, so `scheduler_lag_seconds{pass="batches"}` does not count it (G1963: the wait has no deadline).
2. **`RUNNING` and nothing changes.** Is `SAGAT_BATCHES_ENABLED` on in an engine, does a replica hold the job's shard, is `scheduler_lag_seconds{pass="batches"}` high? Is the rate low (`itemsPerSecond`) for the number of runs, or are many jobs sharing the replica's 73 items a second (section 2.1)? Are the items `DEFERRED`? (`QUOTA_EXCEEDED` means the platform is at the limit of what the action needs, for example active runs of a rerun.)
3. **Many `FAILED`.** The `errorCode` of each is the single action's own code; the same failure would have come from doing it by hand.
4. **`FAILED` with `AUTHORITY_REVOKED`.** The creator no longer holds a permission the action needs. Create the job again as somebody who does.
5. **A cancelled job still shows runs that were cancelled by it.** That is right: the item that ran stays `SUCCEEDED`, the ones that had not run become `CANCELLED`.

| Signal | Meaning |
|---|---|
| `batch_jobs_active` | jobs that are selecting, running or being cancelled |
| `batch_items_total{action,outcome}` | what became of the items, counted once the outcome is committed |
| `batch_item_duration_seconds{action}` | how long applying the action to one run took |
| `scheduler_lag_seconds{pass="batches"}` | how long the oldest job the pass would take has waited; `SagatSchedulerLate` takes the maximum over every pass |
| `scheduler_fenced_total{kind="batch"}` | steps another replica or a lost lease took away |
| `SagatBatchJobsStalled` (alert) | jobs are active and no run has had an outcome from any of them for fifteen minutes: look first at the two flags (section 3), then at an old open transaction (point 1 above) |

No label is a tenant, a job or a run. The audit trail records `batch.created`, `batch.cancelled` and `batch.finished`, and each item's effect is audited as the single action is.

## 6. What is not built yet

- The purge of finished jobs and their items (T018-051, waiting on feature 020's selector).
- `REPIN` (feature 019) as a batch action. (`UPDATE` is built: each item applies the declared Update under the key of the item, `SUCCEEDED` when applied, `FAILED` with the `Error.code` of `updateRun` when a rule says no, `SKIPPED` with `ALREADY_TERMINAL`, `RUN_CONTINUED` or `OPERATION_NOT_DECLARED` otherwise.)
- A bounded wait on locks inside an item (G1958) and a deadline for the wait of the selection (G1963); the clock skew between the host that stamps `runs.created_at` and the database clock the selection uses (G1962).
