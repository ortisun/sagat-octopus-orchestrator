# Data-platform and PostgreSQL performance review of O460, O463, O464 and T014-042 — 2026-11-04

Written by a `general-purpose` agent briefed as a data-platform and PostgreSQL performance expert (read-only, nothing was run). It was given the questions and the files, not the assistant's leaning, and it did not see the other two reviews. Paths are relative to the repository at `54570349`. The assistant spot-checked three of its claims before relying on them: the idle-tick figures in `tests/performance/reports/raw/workflow-primitives/20261017-waits-runtime/waits.md` (p50 20.8 ms, p99 38.5 ms with 10,000 approvals and 10,000 timers pending, but 220.8 ms for 10,000 approvals before `ANALYZE`); that the `node_kind` column exists (migration `20261205000000`) while `collection_candidates` still parses the stored workflow JSON per PENDING root step (`crates/infra-postgres/src/maintenance.rs`); and the declared maxima of 10,000 for `maxItems` and `maxIterations` (`crates/domain/src/workflow.rs`). The rest is the reviewer's reading and is unverified.

# Expert opinion (data platform / PostgreSQL): O460, O463, O464 and the closing of T014-042

Reviewer role: data-platform and PostgreSQL performance engineer, deciding for the owner.
Scope: read-only review of the snapshot ROOT
`<repo>/`
(HEAD `54570349`, 2026-11-04). Nothing was built or run; every number below is read from the report, the raw
fragments, the research notes or the code, and "estimate" always means "not measured".
All paths are relative to ROOT. `R` = `tests/performance/reports/workflow-primitives.md`,
`RS` = `specs/014-workflow-primitives/research.md`, `RAW` = `tests/performance/reports/raw/workflow-primitives`.

---------------------------------------------------------------------------------------------------

## 0. Decisions in one screen

| # | Question | Decision |
|---|---|---|
| 1a | Published `maxItems` (collections) | **10,000** supported, declaration maximum unchanged at 10,000. Condition: the HEAD re-measurement of section 4.A passes. Fallback if it fails: publish 1,000. |
| 1b | Published `maxIterations` (loops) | **1,000** supported. The declaration maximum stays 10,000. Raise the published number to 10,000 only when one 10,000-iteration loop completes on HEAD in <= 1,800 s with flat per-transition rows (4.B). No 10,000-iteration loop has ever completed (R:1216, best 8,806 in 1,800 s). |
| 1c | Wall time per run at the ceiling | Collection, 10,000 items: **860 to 1,387 s (14 to 23 min)** measured on the bounded-worker source (debug build, runtime role, in-process, loaded laptop); best at width 16 (860 s); plan for 15 to 25 min. 1,000 items: about 1 to 2 min. Loop, 1,000 iterations: 98 s measured on the older (whole-run worker) source; 10,000 iterations would be about 12 to 17 min, an estimate. |
| 1d | G520 (worker `complete`/`poll` restore the whole run) | **Not a release blocker.** Resolved for scoped (iteration) tasks in HEAD; remains a documented limitation for root-task reports and whole-run API reads (about 4 microseconds per step row, about 0.4 s and about 400 MB at 100,000 instances, an estimate). Becomes a blocker for the 10,000-item ceiling only if 4.A4 fails on HEAD. |
| 1e | Must validation/defaults enforce it? | **No code change for the declared maxima.** `maxItems`/`maxIterations` have no default (schema `required`); the failure mode at the maxima is "slow and correct, bounded by the mandatory `timeoutMs` and by the closed-by-default tenant `iteration_concurrency` quota", not unsafe. What must change is documentation (runbook, release notes, report) per ADR-0040 point 4. A hard server cap is possible; its blast radius is listed in section 7 and I advise against it for 0.3.0. |
| 2a | Accepted ceiling of pending waits per scheduler replica | **20,000** (any mix of approvals, timers, event waits) at the default 500 ms interval. 20,000 is the largest population actually measured on the integrated code (10,000 approvals + 10,000 timers). |
| 2b | Enforcement / observation | No hard gate (the default `ActiveRuns` quotas of 10,000 per namespace and 50,000 per tenant are the only bound and `docs/owner-decisions.md:159` keeps them). Observe with the existing `runs_active{status="WAITING"}` / `scheduler_replicas`, and **add** `scheduler_tick_seconds{outcome}` plus alerts `SagatSchedulerTickSlow` and `SagatWaitsPerReplicaHigh` (section 3). `SagatSchedulerLate` cannot see this problem. |
| 2c | Index-only discovery before 0.3.0? | **Yes, required**, small: two discovery queries still parse the stored workflow JSON per PENDING root step (`crates/infra-postgres/src/maintenance.rs:203`, `loop_progress.rs:1150-1171`) although the `node_kind` column exists since `20261205000000`. Not needed to defend the 20,000 figure; needed because the cost is O(PENDING roots x document size) and the harness only uses tiny documents. |
| 3 | Closing T014-042 | Section 4: thresholds on HEAD, release build, `sagat_app`, separate-host PostgreSQL, concurrent workers, production tick loop, 2 repetitions. The nested 100,000-instance workload **cannot be measured** while nested scopes are closed; do not fake it. Close an amended T014-042 (flat 100,000-instance workload) and open a new task for the nested one, by an owner amendment recorded in `docs/owner-decisions.md`. |

---------------------------------------------------------------------------------------------------

## 1. Provenance of the numbers: what is stale, older, or not reproducible

### 1.1 Which source each measurement came from

| Measurement | Source commit | Ancestor of HEAD? | Build | Worker path | Result |
|---|---|---|---|---|---|
| Series 1: 10,000 items c256 full run (R:610-651) | `e3e3979` | yes | release (R:45) | whole-run | 6,815 s |
| "After G461" (R:1128-1288) | `e8ccca8b` (R:1150) | yes | release | whole-run (G520) | 10,000 items c256 1,405 s (R:1213); loop 10,000 not finished, 8,806 iterations in 1,800 s (R:1216) |
| Frozen diagnostic, owner role (R:1323-1351) | `025ce8d4` | **no** | debug (`RAW/20261017-025ce8d4/manifest.json`) | exact-path | c1 1,580.7 s, c16 980.4 s, c256 1,163.4 s |
| Frozen runtime-role collections (R:1410-1438) | `b1855cbe` | **no** | debug (`RAW/20261017-runtime-flat/manifest.json`) | exact-path | c1 1,238.8 s, c16 860.1 s, c256 1,387.1 s |
| Runtime-role 20,000 waits (R:1393-1407) | `60dfc824` | **no** | debug | n/a | idle tick p50 20.8 ms at 20,000 waits |
| Flat rows at 160/320/640 (R:1290-1320) | `5ba1c2bc` | **no** | debug | n/a | 44 / 67 rows per transition |

I checked ancestry with `git merge-base --is-ancestor` on the snapshot: `e3e3979`, `e8ccca8b`, `42375f3`, `d8b9d2e` are ancestors of HEAD;
`025ce8d4`, `b1855cbe`, `60dfc824`, `5ba1c2bc` are present in the object store but are **not** ancestors. The exact-path worker code is in HEAD
(introduced by `482979a6`, 2026-10-24, `feat(016): integrate payload classification and runtime follow-ups`: `crates/application/src/worker/scoped_claims.rs:12`,
`scoped_reports.rs:119`, `crates/infra-postgres/src/execution.rs:744` `begin_current_path`), but **no performance run exists on any commit at or after it**. The harness
files (`tests/performance/workflow-primitives/*.rs`) were last touched by that same commit. The EGM drift report says the same (`specs/014-workflow-primitives/evidence/egm-drift-20261104/2026-11-04_DRIFT_REPORT.md:213`)
and `specs/014-workflow-primitives/tasks.md:209-212` itself requires the gate to run "on the integrated source after the nesting work". Nested capacity, control entry, scoped timers,
event deadlines and the vacate/acquire sweeper (D714, `RS:1798-1800`) all landed after the measurements and add work to transitions or to the idle tick.

### 1.2 Stale text inside the report (the numbers are right, the conclusions are not)

- R:14-34 "What this report concludes", R:871-881 "Defects and anomalies" 1, R:919-934 "Ceilings", R:936-945 "Not measured" are the pre-G461 text. Only R:21 carries a one-line
  supersession note. R:930 still recommends "cap at 1,000 / 47 times / 6,815 s". R:927 still says the idle tick "grows (G468)", contradicted by R:1249 and `RAW/20261017-waits-runtime/waits.md:45-48`.
- RS:1675-1686 (G520) and `specs/014-workflow-primitives/plan.md:292` ("The production worker still uses the existing full transaction path") are false for scoped tasks at HEAD:
  `scoped_claims.rs:12-100` and `scoped_reports.rs:119-` run in one `begin_current_path` transaction ("never restores the full run history", `crates/infra-postgres/src/execution.rs:829`).
  They are still true for root tasks (`crates/application/src/worker.rs:958` calls `begin`, and `output.rs:92,146` route only non-root paths to `report_path`).
- Evidence that G520 is gone for scoped tasks, same code family: `complete` p50 **14.8 ms** at 10,000 items c256 (`RAW/20261017-runtime-flat/collection-10000-c256.md:37`), 14.6 ms c16 (`...c16.md:37`),
  15.5 ms c1 (`...c1.md:37`), against **78.4 ms** in G520 (RS:1678-1679) and 18.5 to 21.4 ms at 1,000 items on the older source (RS:1679). Lease cost is flat too: 278.8 ms / 16 leases = 17.4 ms (c16, `...c16.md:38`),
  1,135.9 ms / 64 = 17.7 ms (c256, `...c256.md:38`).
- Section 1 of the report compares unlike states: R:1243-1251 sets the "before" 10-tick row ("statistics as the load left them") against the "after" 50-tick row taken after `ANALYZE`; 11.2 ms is *below* the empty-database
  control of 26.1 ms in the same table, which is only possible because the rows are different states, not because 11,000 waits are cheaper than none. The control the harness provides
  (`SAGAT_PERF_WITHOUT_OPEN_INDEXES=1`, `tests/performance/workflow-primitives/main.rs:281-288`) was never reported (RS:1695-1696 says "repeat on a quiet host").
  The like-for-like data is the runtime-role run: 21.1 ms empty, **220.8 ms** at 10,000 approvals with stale statistics, **33.9 ms** after `ANALYZE`, **20.8 ms (p99 38.5)** at 10,000 approvals + 10,000 timers
  (`waits.md:45-48`). So G522 is explained at the level of "planner statistics": the slow window is the minutes between a bulk load and autovacuum's first analyze. The 185.9 -> 320.8 ms delta of R:1250 is still unexplained (one run each, load 10 to 40).

### 1.3 Not reproducible as written

- The wait command in R:77 and the rest of R:57-78 omit `SAGAT_TEST_AS_RUNTIME_ROLE=1`; `measure_waits` asserts the role is `sagat_app` and panics otherwise
  (`tests/performance/workflow-primitives/main.rs:267-273`). The test names in the command block still match as substrings of the current names
  (`measure_durable_waits_and_cross_run_tenant_slot_contention`, main.rs:1014; `measure_chain_segments`, chains.rs:200;
  `measure_cross_run_tenant_slot_bounds_with_a_bursting_neighbour`, tenants.rs:219).
- The frozen runs used a manually started container, private worktrees (a private worktree outside the repository) and binaries under `/tmp` that are not retained
  (`RAW/20261017-025ce8d4/manifest.json`: `binary_retained_after_run: false`). The harness comment itself says later acceptance must use the repository-declared fixture (R:1348-1349).
- Run-to-run noise is about +/-25 percent on the same code: c1 1,580.7 s (owner role) vs 1,238.8 s (runtime role); c256 1,163.4 s vs 1,387.1 s; host load 6.9 to 24.9 (R:1332-1336, `RAW/.../collection-10000-c*.md:3`).
  c16 was fastest in both pairs (980 and 860 s). Width 1 vs 256 is inside the noise, so no ordering between them can be claimed.
- Two stalls in the runtime-role c256 fragment are environmental, not algorithmic: `admit` max 7,612.9 ms and `complete` max 7,611.1 ms (`...c256.md:32,37`) are the same 7.6 s event, and one `poll` took 92.8 s (`...c256.md:38`) with a
  single-threaded driver that cannot contend with itself. R:1429-1431 states "no latency SLO" but does not list them as unexplained. Queue delay p99 190 s (`...c256.md:40`) is an artifact of one serial driver completing
  64 per poll, not a platform property.

### 1.4 What the harness does not measure (so the numbers cannot be read as product behaviour)

1. **One thread plays scheduler and worker**: `advance_collection` then poll then complete, in sequence (`tests/performance/workflow-primitives/collections.rs:130-200`). In production every report, claim and scheduler
   transition of a scope takes the same `runs ... FOR UPDATE` lock (`crates/infra-postgres/src/execution.rs:696-715`, `lock_path_revision`), so concurrent workers and the scheduler queue on one row. The harness has zero
   such contention. This is the largest gap.
2. **The production tick is not used for collections/loops** (D463, RS:1371-1375): the harness calls `advance_collection` in a tight loop. In production a run gets at most one transition per tick (G463, RS:1415-1417), a tick that worked
   is followed immediately by another, an idle tick by a 500 ms +/-20 percent pause (`apps/execution-engine/src/loops.rs:150-181`, `settings.rs:511-514`), and a tick costs 11 to 21 ms idle before the transition itself.
3. `ANALYZE` every 50 transitions (`collections.rs:20,136`, D462 RS:1368-1370) is not what autovacuum does (about one analyze per minute after a threshold); it keeps plans better than production would for 10,000-item runs.
4. In-process means no network: a collection transition issues 49 (checkpoint) to 57 (admit) statements (`...c256.md:49-75` sample rows). The 20,002 transitions of a 10,000-item run are about 1.05 million statements. Each extra 0.3 ms of
   round trip adds about 315 s to a run from transitions alone (estimate: 1.05 M x 0.3 ms), before counting the worker's statements. This is the single biggest uncertainty in the wall times.
5. PostgreSQL is `shared_buffers=128MB`, 20,000 step rows, a dataset that lives in cache; nothing here shows behaviour on tables of millions of rows.
6. Event waits as a population, loops parked in a durable delay (`loops.rs` has `delay_ms` in `loop_document` but no test that creates a population, G465), approval decision throughput, more than one replica, HTTP/gRPC, TLS: not measured (R:936-945).

---------------------------------------------------------------------------------------------------

## 2. Decision 1: ceilings for collections and loops (O460, O464, G520)

### 2.1 Reasoning

- O460 asked "is a quadratic collection acceptable". It no longer is one: after D520-D526 the scheduler transition reads 44 rows (collection) and 65 rows (loop) at every size
  (R:1173-1195; `RAW/20261017-runtime-flat/collection-10000-c1.md` first/last probe 41/41 and 45/45 rows) and the 10,000-item transition p50 by tenth is flat or falling (c1 15.7 -> 11.5 ms, c16 11.8 -> 8.8 ms; c256 13.8 -> 24.4 ms is
  inside the host noise and the O(maxConcurrency) frontier term, rows 797 -> 65 depend on iterations in flight).
- O464 asked whether the worker path is in scope. On the integrated source it already is done for scoped tasks (section 1.2). With it, a 10,000-item collection **completed in 6 of 6 recorded full runs on the bounded-worker source**
  (3 owner role on `025ce8d4`, 3 runtime role on `b1855cbe`) and the only thing left is wall time.
- Wall time is not an algorithm problem any more; it is per-run serialization. Per item the run's row lock is held for admit 13 + checkpoint 13 + lease 17 + complete 15 = about 58 ms of p50 work (c16 fragment), i.e. about 17 items/s
  upper bound, 11.6 items/s observed at c16, 8.1 at c1, 7.2 at c256. Width above about 16 buys nothing and c256 is worse (queue delay and poll tails, section 1.3). A per-run throughput of **about 7 to 12 items/s is the number to publish**,
  and the lever for more is more runs (parallel child runs/shards), not more width.
- A 10,000-iteration loop has no completed run on any source and its per-iteration path is the same 4 round trips in strict sequence (admit/checkpoint ~17 ms, claim ~23 ms, complete ~15 ms, nothing-to-advance and empty poll ~9 ms),
  so the floor is about 65 to 100 ms per iteration: 650 to 1,000 s for 10,000 if everything is flat (estimate). That is plausible but unproven; 1,000 iterations (98 s, R:1215) is proven.
- Safety is not what limits the ceiling. A slow 10,000-item or 10,000-iteration run is correct, tenant-bounded (the 279-round two-tenant test, R:234-244) and fairness-preserving (quiet tenant ratio 1.00). Collection admission is closed until an operator sets
  `iteration_concurrency` (`docs/runbooks/workflow-primitives.md:22-33`; migration `20261029000000_collection_progress.up.sql:88-90`, default operator ceiling 1,024), loops require `timeoutMs` (`crates/domain/src/workflow.rs:1829`), instances are bounded at 100,000 per run
  (`workflow.rs:1021,1051`). So I do not need to lower the validation to protect the platform; I need to stop the document from promising what it did not measure (ADR-0040 point 4, `docs/adr/ADR-0040-...:16`).

### 2.2 What to publish

| Item | Declared maximum (validation) | Published supported ceiling | Expected wall at the ceiling | Evidence status |
|---|---|---|---|---|
| `forEach.maxItems` | 10,000 (`crates/domain/src/workflow.rs:1797`, `crates/application/src/primitives.rs:121`, `specs/001-platform-foundation/contracts/workflow.schema.json:1118-1122`; structural limit `MAX_ITERATION_INDEX = 9_999`, `crates/domain/src/execution/instance.rs:12`) | **10,000** | 14 to 23 min (c16 best, 860 s); 1,000 items about 1 to 2 min | measured on a non-HEAD source, debug, in-process: **provisional until 4.A passes on HEAD** |
| `while`/`repeatUntil` `maxIterations` | 10,000 (`workflow.rs:1823`, `crates/application/src/loop_progress.rs:210`, schema :1171-1175 and :1223-1227) | **1,000** | about 1 to 1.7 min (98 s measured, older source); 10,000 would be 12 to 17 min (estimate) | 1,000 measured; 10,000 never completed |
| `maxConcurrency` | 256 (`workflow.rs:1797`) | recommend **16**; 256 is allowed, supported, not faster | | c16 fastest in both frozen pairs |
| Instances per run | 100,000 default, author can lower (`workflow.rs:1021,1051`, `crates/infra-ir/src/convert.rs:1052-1079`) | 100,000 only after 4.E1 | | no measurement above 20,002 step rows |

The legacy `foreach` schema entry still says `maxItems` maximum **100,000** (`specs/001-platform-foundation/contracts/workflow.schema.json:621-625`). It is a placeholder that publication refuses
(`crates/infra-ir/src/convert.rs:74,85`), but a reader of the schema will take 100,000 as a promise: lower it to 10,000 or add a description saying it is refused.

### 2.3 G520 classification

Decision: **documented limitation, not a blocker, not deferred work for 0.3.0.** Scoped tasks are done. Root-task reports (`worker.rs:958`) and whole-run readers (cancel, signal, `GET run`, `verify_run`, scope enter/finish/compensation, G521) still restore the whole run.
Cost per step row read from the older source: 78.4 ms / about 20,000 rows = about 4 microseconds, and sampled RSS grew from 38 to 120 MiB with 20,002 step rows (`...c256.md:24-26`), about 4 KB per row. For the 100,000-instance ceiling that is about **0.4 s and about 400 MB per whole-run read** (estimate), which is a
control-plane memory amplification and not a worker-throughput problem; test it in 4.E1, document it in the runbook (`docs/runbooks/workflow-primitives.md` has no capacity guidance, drift report :227).
G520 would become a blocker for the 10,000-item ceiling only if 4.A4 (worker `complete` p50 <= 40 ms at 10,000 items) fails on HEAD; then publish 1,000 for collections too. That fallback needs **no code change** because the declared maximum is unchanged.

### 2.4 What must change (files)

No change to validation. Documents and evidence:
- `docs/runbooks/workflow-primitives.md`: add a "Measured capacity" section (the table in 2.2, per-run throughput 7 to 12 items/s, width 16 recommendation, `iteration_concurrency` sizing, whole-run read cost).
- `docs/releases/0.3.0.md` (release notes): state declared maxima and supported ceilings separately (ADR-0040 point 4).
- `tests/performance/reports/workflow-primitives.md`: replace R:14-34, R:871-881, R:919-945 with a post-fix table; mark every historical section with its source SHA and build profile.
- `specs/014-workflow-primitives/research.md`: answer O460 (RS:1450), O464 (RS:1724) and O463 (RS:1460); update G520 (RS:1675) to "scoped tasks resolved at 482979a6, root/API reads documented".
- `docs/owner-decisions.md`: record the three answers.
- `specs/014-workflow-primitives/plan.md:292` and `specs/014-workflow-primitives/quickstart.md`: remove the false "full transaction path" sentence; keep quickstart examples at small maxima (they use 3, `quickstart.md:332`).

---------------------------------------------------------------------------------------------------

## 3. Decision 2: pending waits per scheduler replica (O463, G468, G522)

### 3.1 What the evidence says

- The 20,000 to 30,000 figure of G468 is a linear extrapolation of the **stale-statistics, pre-index** curve (500 ms / 16 to 30 microseconds, RS:1427-1440, R:105-120). It is not a statement about the current code.
- Integrated, runtime role, 10,000 approvals + 10,000 timers = **20,000 waits, measured, not extrapolated**: idle tick p50 20.8 / p95 36.4 / p99 38.5 ms over 50 ticks after `ANALYZE`; 21.1 ms with no waits
  (`RAW/20261017-waits-runtime/waits.md:45-48`). 38.5 ms is 7.7 percent of the 500 ms interval. 0 skipped, 0 fenced, 0 failed scheduler outcomes (`waits.md:72`).
- The binding case is the **stale-statistics window** after a bulk load: 220.8 ms at 10,000 approvals (`waits.md:46`), 320.8 ms once on the older source (R:1250). That is 22 to 32 microseconds per approval, so about 15,600 to 22,600 approvals
  loaded in one burst take the tick past 500 ms until autovacuum analyzes. Exceeding the interval is not a failure (a tick that works is followed immediately by the next, `loops.rs:176-181`), it makes due timers late by at most one tick; but it is a 100 percent duty cycle on one replica.
- Feature 010 already proved the platform standard: ADR-0020 point 4 says the cost of a pass "follows what is due and not what waits" and `docs/traceability.md:344` records an idle tick of 0.8 ms at a million waits for timers/retries/leases/deadlines. The 014 discovery passes were built without that property,
  which is why G468 existed; D525's partial indexes restored it for WAITING steps.
- What D525 did not fix: `collection_candidates` (`crates/infra-postgres/src/maintenance.rs:176-260`) and `loop_candidates` (`crates/infra-postgres/src/loop_progress.rs:1119-1200`) filter `steps` on `status='PENDING'` and root path
  and then evaluate `jsonb_array_elements(convert_from(version.document,'UTF8')::jsonb)` **per candidate row** (maintenance.rs:203; loop_progress.rs:1150,1164,1171) *before* `ORDER BY ... LIMIT`. The comment at maintenance.rs:172-175 says
  the query "does not imply a shard-first index". The open-step indexes (`20261122000000_steps_open_index.up.sql:9-12`) are keyed `(tenant_id, run_id)`, not shard-first, so the per-shard `LATERAL` over 64 owned shards rescans the open set. Migration
  `20261205000000_bounded_execution_discovery.up.sql:1-2` states that "runtime discovery ... does not parse workflow documents" and adds `steps.node_kind` for it; these two queries do not use it.
  Cost per tick is O(PENDING root steps in owned shards x workflow document size) per pass, twice (collections and loops). With the benchmark's tiny documents that is invisible (EXPLAIN 1.3 and 0.6 ms, R:1238-1241). With 5,000 PENDING root starts of a
  100 KB definition (a burst of approvals before they open, which runs at 227 to 282 per second, `waits.md:55`, R:179) it is gigabytes of JSON parsed per tick. This is inferred from the query text and **not measured**; criterion 4.C6 measures it.

### 3.2 The ceiling

**20,000 pending waits per scheduler replica**, counted as runs in `WAITING` owned by the replica's shards, default tick interval 500 ms (`apps/execution-engine/src/settings.rs:511-514`).
It is the largest measured population, it keeps the idle tick at 7.7 percent of the interval in steady state, and it sits just below the point where the stale-statistics window reaches the interval. Waits divide by shard (ADR-0020), so the platform figure is `20,000 x replicas` (up to 64 replicas).
Beyond 20,000 means "not measured", not "not supported"; raise it only with the evidence in section 6.

### 3.3 Enforcement and observation

- Enforcement: none new. The existing default quotas (10,000 active runs per namespace, 50,000 per tenant, `docs/owner-decisions.md:159`) already bound it, and a single tenant at the default can exceed 20,000 on one replica, so the runbook must say "provision
  `ceil(expected waits / 20,000)` replicas".
- Existing signals: `runs_active{status="WAITING"}` (`crates/scheduler/src/monitor.rs:270`), `scheduler_replicas` and `scheduler_shards_owned` (`crates/scheduler/src/shards.rs:327-328`). Existing alert `SagatSchedulerLate` (`infra/observability/alerts.yaml:166-172`,
  `max(scheduler_lag_seconds) > 60 for 5m`) covers only the passes `timers, wait_deadlines, retries, leases, deadlines` (`monitor.rs:259-263`); it is blind to the collection, loop, continuation and approval passes and cannot fire until something is a minute late. No tick-duration metric exists (grep of `crates/` and `apps/` for a tick histogram finds none).
- Proposed, to add before 0.3.0:
  1. Histogram `scheduler_tick_seconds{outcome="idle"|"worked"}` recorded around `tick().await` in `run_scheduler` (`apps/execution-engine/src/loops.rs:174`). No tenant/run labels (ADR-0017).
  2. Alert `SagatSchedulerTickSlow` in `infra/observability/alerts.yaml`: `histogram_quantile(0.95, sum by (le) (rate(scheduler_tick_seconds_bucket{outcome="idle"}[10m]))) > 0.25` for 10m, warning (duty cycle over 33 percent); same expression `> 0.5`, critical.
  3. Alert `SagatWaitsPerReplicaHigh`: `sum(runs_active{status="WAITING"}) / clamp_min(sum(scheduler_replicas), 1) > 16000` for 15m, warning (80 percent of the ceiling).
  4. Runbook entries in `docs/runbooks/scaling.md` (next to `#scheduler-late`, line 146) and the metric in the dashboard `infra/observability/dashboards/sagat-overview.json:1575`.
  5. Document the autovacuum setting for the stale-statistics window (G522): `autovacuum_analyze_scale_factor = 0.02` on `steps`, `runs`, `approval_requests`, `approval_timers`; or an explicit `ANALYZE` after a planned bulk start.

### 3.4 Index-only discovery: required, with a bounded scope

Required before 0.3.0 because the cost model is wrong in kind (document-size dependent) and the harness cannot show it. Minimal change, one migration plus two queries:
`node_kind IN ('forEach')` / `IN ('while','repeatUntil')` instead of the JSON predicate (keep the JSON test only for rows still labelled `legacyControl`, written by `ensure_node_kind`, `crates/infra-postgres/src/execution.rs:3347`), plus a shard-first partial index
`ON steps (run_shard(run_id), updated_at) WHERE status='PENDING' AND cardinality(instance_path_nodes)=0` (the same shape as `steps_timer_shard_idx`, `20261017000000_scheduler_shards.up.sql:45`), which also serves `approval_candidates`, `approval_routes`
(`crates/infra-postgres/src/approval_scheduler.rs:118-170`) and `continuation_candidates` (`maintenance.rs:105-170`). Follow `.claude/skills/migration` (additive, grants, down). Acceptance: 4.C6 and `EXPLAIN (ANALYZE, BUFFERS)` showing no `jsonb_array_elements` in the plan.

---------------------------------------------------------------------------------------------------

## 4. Decision 3: acceptance criteria that close T014-042 honestly

The task text: `specs/014-workflow-primitives/tasks.md:222`; plan goals `plan.md:80-81`; NFR-014-003 and SC-014-006 `spec.md:411-413,444`. Everything below is run on **one** release-candidate SHA on `develop`
(`git merge-base --is-ancestor <sha> origin/develop` must succeed), and the fragments, manifest (SHA, binary SHA-256, flags, host description, load average at start) and raw output are committed under
`tests/performance/reports/raw/workflow-primitives/<date>-<sha>/`.

### 4.0 Preconditions (a run that misses one is a diagnostic, not acceptance)

- P1 Release build (`cargo test --release`), `SAGAT_TEST_AS_RUNTIME_ROLE=1`, the repository-declared PostgreSQL fixture (Rule 16), tree clean, no other project's containers on the host. Debug builds and the owner role are never acceptance.
- P2 Reference hardware R1 (if the owner names a different machine, record it and rescale nothing): engine host 8 vCPU / 16 GiB Linux; PostgreSQL 16 on a **separate** host or VM, 8 vCPU / 32 GiB, local NVMe, `fsync=on`, `synchronous_commit=on`,
  `shared_buffers=8GB`, `effective_cache_size=24GB`, `max_wal_size=8GB`, `random_page_cost=1.1`, default autovacuum, `pg_stat_statements` preloaded; engine-to-database round trip <= 0.5 ms (record `ping` and a measured `SELECT 1` p50); 1-minute load <= 1.0 on both hosts at the start of every run.
  If only the laptop profile exists, the report must say "laptop profile" in its title and the thresholds below are claimed for that profile only.
- P3 Production shape: the scheduler runs through `Scheduler::tick()` in the engine loop with defaults (batch 200, concurrency 8, interval 500 ms, `discovery_rest` 100 ms) and **at least min(width, 32) worker sessions** polling and completing concurrently, so run-row lock contention is exercised. A single serial driver may only be used for a labelled diagnostic.
  No periodic manual `ANALYZE` (D462); statistics come from autovacuum, and a second run with `ANALYZE` every 50 transitions may be added as a labelled control.
- P4 Two repetitions per cell; report min / median / max. Because noise on this host class is +/-25 percent, a cell passes only if its **worse** repetition passes.

### 4.A Collections: 10,000 items at maxConcurrency 1 / 16 / 256 (and 1,000 at the same widths)

| ID | Threshold | Evidence today (non-HEAD) |
|---|---|---|
| A1 | Run `SUCCEEDED`; 10,000 checkpoints; aggregate in input order; `verify_run` agrees; scheduler errors 0; client retries 0; peak active iterations observed equals the width (not width minus 1) | holds, but 255 observed at c256 on `025ce8d4` (R:1338-1339) |
| A2 | Wall <= **1,800 s at c16**; c1 and c256 must complete in <= 3,600 s (the harness budget, `tests/performance/workflow-primitives/common.rs:385-391`) and are reported as is | 860 / 1,239 / 1,387 s (debug, loopback) |
| A3 | Rows per sampled transition: last-decile median <= 1.25 x first-decile median + 20 (admit and checkpoint separately) at c1 and c16; c256 <= 1,000 rows absolute; SQL statements <= 65 per transition | 41/41 and 45/45 (c1), 49 and 57 statements; c256 rows 797 to 65 |
| A4 | Worker `complete` p50 <= 40 ms and p99 <= 500 ms at 10,000 items, and p50 of the last decile <= 1.5 x first decile (this is the G520 invariant); lease cost p50 per task <= 40 ms | 14.8 to 15.5 ms; 17.4 to 22.8 ms |
| A5 | Tails: scheduler transition p99 <= 1 s, max <= 10 s; poll-with-leases p99 <= 5 s, max <= 30 s. A max above the limit passes only with a recorded explanation (host stall evidence) and a clean rerun | c256: admit max 7.6 s, poll p99 15.1 s, max 92.8 s: **would fail**, must be explained or fixed |
| A6 | Engine process RSS peak (native `time -l` / `/usr/bin/time -v`, not sampled) <= 512 MiB; threads <= start + 4; PostgreSQL connections <= pool + probe; journal bytes per item <= 2,500; relation + index bytes per item <= 12 KB (not measured today) | 177 MiB native max RSS (`RAW/.../process-c256.txt`), 2,334 B/item |
| A7 | 1,000 items: wall <= 180 s at every width | 53.8 to 94.5 s (older source, R:1210-1212) |
| A8 | Production-path ratio: wall / (20,002 x p50 tick) <= 1.5 shows the run is tick-bound, not lock-bound; report both | not measured |

Outcome rule: A1-A6 pass at c16 and c1/c256 complete, then publish `maxItems` 10,000; otherwise publish 1,000 (A7 must then pass).

### 4.B Loops

| ID | Threshold |
|---|---|
| B1 | 1,000 iterations: `SUCCEEDED`, wall <= 150 s. (98.2 s older source, R:1215) |
| B2 | 10,000 iterations: `SUCCEEDED`, wall <= 1,800 s, `complete`/lease p50 limits as A4, rows per transition <= 70 and statements <= 90 at every decile (65 rows measured at iterations 1,000 to 7,000, R:1193-1195) |
| B3 | Parked delay population: 1,000 loops with `delayMs` >= 1 h, zero threads/connections/leases per loop (D465 style assertions), idle tick as in C2. (`loop_document` supports it, `loops.rs:20-33`; no test exists, G465) |
Outcome: B2 passes -> publish `maxIterations` 10,000; fails -> keep 1,000 and report the largest completed size.

### 4.C Waits (approvals, timers, event waits)

| ID | Threshold |
|---|---|
| C1 | Populations: 10,000 approvals + 10,000 timers (existing) and 10,000 approvals + 10,000 timers + 10,000 event waits (new workload; `waitEvent` correlated, never signalled). Assertions as D465 (RS:1380-1383): threads <= start + 4, PostgreSQL connections <= configured pools + probe, 0 idle in transaction, 0 worker sessions, no lease/worker identity on any wait step. |
| C2 | Steady state (>= 120 s after load, statistics from autovacuum, 50 ticks): idle tick p95 <= 100 ms at 20,000 waits and <= 125 ms at 30,000, and <= 3 x the empty-database tick. (Observed 36.4 ms p95 at 20,000.) |
| C3 | Stale-statistics window (first 10 ticks after load, no ANALYZE): p95 <= 500 ms (the interval) at 20,000 waits (observed 233.9 ms at 10,000 approvals); returns under C2 within 180 s without manual action. |
| C4 | Slope: p50(30,000) - p50(10,000) <= 20 ms. (Detects a return of G468.) |
| C5 | Lateness: 1,000 timers made due in the same second with 30,000 other waits pending are all fired within 15 s of due (batch 200 x concurrency 8 -> about 5 ticks). |
| C6 | Discovery cliff: 5,000 approval runs started within 60 s from a definition of >= 100 KB. All opened within 120 s; no tick longer than 5 s; `EXPLAIN (ANALYZE, BUFFERS)` of the collection, loop and approval candidate queries shows no JSON parsing. (Fails today by inference; passes after 3.4.) |
| C7 | A/B control on one container with `SAGAT_PERF_WITHOUT_OPEN_INDEXES=1`, reported next to the main run with the same tick counts and statistics state (closes the open comparison of R:1243-1251 and G522). |
| C8 | `scheduler_tick_seconds` and `SagatSchedulerTickSlow`/`SagatWaitsPerReplicaHigh` exist and fire in a test at 2 x their thresholds (section 3.3). |
| C9 | Cross-tenant bound as today (counter == holders every round, <= limit; quiet tenant <= 4 x solo rounds; 8 of 64 under a held lock, 56 typed refusals), repeated on HEAD with concurrent workers. Two scheduler replicas on the same 20,000 waits: no double transition, per-replica idle tick <= single-replica p50 (shard division works). |

### 4.D Reporting hygiene (blocks closing)

- Every table row names the SHA, build profile, role, host profile. Sections measured on an older source are moved to an appendix titled "historical, not acceptance".
- R:14-34, R:871-881, R:919-945 rewritten; the Commands block includes `SAGAT_TEST_AS_RUNTIME_ROLE=1` and the real test names; the raw fragments are in the repository, not in `/tmp`.
- Failures stay in the report; a failed cell is a finding, not a retry (Rule 17).

### 4.E The 100,000-instance workload while nested scopes are closed

Nested scopes are closed at publication (`docs/adr/ADR-0040-release-checkpoints-preserve-incomplete-feature-boundaries.md:6` names the refusal test `collection_publication_keeps_nested_bodies_closed`; drift report :211: no nested workload exists in `tests/performance/workflow-primitives/`, the only `100_000` is a limit
argument at `main.rs:804`). A nested-loop 100,000-instance run therefore cannot be produced without a fake path (Rule 11). My ruling:

- **E1 (flat, supported, required to close):** one `forEach` of 10,000 items whose body has 9 activities in sequence plus the `return` (about 10 instances per item, 100,000 charged), default `limits.maxInstances` 100,000.
  Pass: completes in <= 4 h on R1 (estimate 100,000 x about 65 ms = 1.8 h); `run_instance_usage.admitted_instances` equals the number of scoped plus pending step rows (D522 invariant, RS:1605-1610) at every 10,000th instance; per-transition rows/statements flat (A3); a second run with `limits.maxInstances = 99,999`
  fails the 100,000th instance with the typed limit error and `verify_run` still agrees (no rows beyond the ceiling); whole-run reads on the finished run (`GET run`, `cancel` of a copy, `verify_run`): p95 <= 5 s and engine RSS delta <= 1 GiB (predicted 0.4 s and about 400 MB, 2.3);
  database size growth <= 1.5 GB including indexes; compensation of a failed 10,000-item scope (G521: the planner reads every completed iteration) completes and is measured, because it is the one path never measured at scale.
- **E2 (nested, blocked):** record in the report "not measured: nested scopes are closed (ADR-0040; ADR-0075 Proposed)" and add a harness test asserting that a nested publication is refused today, so the report cannot silently claim it.
  Write the target shape now so it is ready: outer loop 100 iterations x inner collection of 1,000 items (depth 2, 100,000 instances), same thresholds as E1.
- Owner amendment needed: `tasks.md:222`, `plan.md:80-81` and `spec.md:412` literally require "nested loops reaching the 100,000-instance ceiling". Without an amendment T014-042 can never be ticked honestly. Amend to "flat 100,000-instance workload (E1); nested workload tracked as a new
  task gating the nested release", record in `docs/owner-decisions.md`, and tick only the amended text with the E1 evidence.

---------------------------------------------------------------------------------------------------

## 5. Closing checklist, in order

1. Land the discovery change (3.4) and the tick metric/alerts (3.3). Both are small and independent of the ceiling decision.
2. Fix the harness: concurrent workers, production tick loop, no periodic ANALYZE, event-wait population, parked-loop population, the compensation-at-scale case, E1; add the nested-refusal assertion.
3. Run 4.A to 4.E on the reference profile on one SHA; commit raw evidence.
4. Rewrite the report and runbook; answer O460/O463/O464 in `research.md` with the measured figures; record the owner amendment.
5. Only then tick T014-042 (amended) with the evidence; keep T014-044 for last.

---------------------------------------------------------------------------------------------------

## 6. Biggest risk, and what would move a ceiling

**Biggest risk:** the 10,000-item ceiling rests on debug-build, in-process (no network), single-serial-driver numbers from commits that are not in HEAD. A collection transition is 49 to 57 statements; at a realistic 0.3 to 0.5 ms
round trip the transitions alone add about 315 to 525 s, and concurrent workers will add row-lock queueing that the harness cannot show. c1 and c256 can then leave the 1,800 s line even though c16 passes. This is why the 10,000 figure is conditional and has a no-code fallback to 1,000.

What would raise or lower a number:

| Change | Evidence that triggers it |
|---|---|
| Lower `maxItems` to 1,000 | HEAD c16 wall > 1,800 s on R1, or `complete` p50 > 40 ms / last-decile rows > 1.25 x first at 10,000 items, or whole-run read RSS at 100,000 instances > 1 GiB |
| Raise loops to 10,000 | B2 passes on HEAD (<= 1,800 s, rows <= 70 per transition at every decile) |
| Keep loops at 1,000 and also lower items | per-iteration p50 chain (admit/checkpoint + lease + complete + empty poll) > 150 ms on R1 |
| Raise waits ceiling to 100,000 per replica | C2 holds at 100,000 (p95 <= 100 ms) after index-only discovery, C3 p95 <= 500 ms, C4 slope <= 1 microsecond per wait, event waits included, two replicas divide the work (C9) |
| Lower waits ceiling to 10,000 per replica | C3 p95 > 500 ms at 10,000 approvals, or C6 fails after the index change, or HEAD idle tick at 20,000 waits > 100 ms in steady state (new passes since the measurement) |

---------------------------------------------------------------------------------------------------

## 7. If the owner prefers a hard server-side cap anyway (not recommended for 0.3.0)

Server-only enforcement, SDK pre-flight checks may stay at 10,000 (they only reject earlier values; the server would reject at publication):
`crates/domain/src/workflow.rs:1823` (loops) and, for items, `:1797`; `crates/application/src/loop_progress.rs:210`; `crates/application/src/primitives.rs:121`;
`specs/001-platform-foundation/contracts/workflow.schema.json:1118-1122,1171-1175,1223-1227`; tests `tests/contract/workflow-primitives/ceilings.rs:142-164`, `loops.rs:97,135`, `revision.rs:193`; authoring-only goldens
`tests/contract/sdk-conformance/expected/loop-builders.workflows.json:30` and its scenario `loop-builders.json:36`; storage constraint comments in `crates/infra-postgres/storage-classification.json` (index range 0..9999 stays valid).
Full enforcement in the six SDKs would also touch `sdks/go/workflow/workflow.go:348,460`, `sdks/python/sagat_octopus_orchestrator/workflow.py:469,569`, `sdks/typescript/src/workflow.ts:360,391`,
`sdks/rust/src/workflow/primitives.rs:277,296`, `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/workflow/Flow.java:414,451`, `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workflows/Flow.cs:249,291` and the 60-scenario conformance matrix. A published-versus-declared split costs none of this
and can be raised in a point release by editing documents only.

---------------------------------------------------------------------------------------------------

## 8. What I could not verify

- No build or run was made; none of the HEAD behaviour in section 4 is observed, only inferred from code and from the non-ancestor commits.
- That the non-ancestor commits and HEAD run equivalent worker code is inferred from `482979a6`'s message and the file contents; no diff of `b1855cbe` against HEAD for those files was made.
- Query-shape statements in 3.1 and 3.4 (per-row JSON parse before the limit, 64-shard rescans) come from reading the SQL; the planner may hoist or cache parts. C6 and C7 decide it.
- The memory estimate of 4 KB per step row is the growth of a *sampled* RSS in a test process (`...c256.md:24-26`), not an isolated measurement of `RunState`.
