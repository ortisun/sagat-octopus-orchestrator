# Goldfish Review Report

**Target:** `docs/designs/041-step-logs.md` (`Lines 2064-3214`, "Detailed Implementation Plan" to the end), at commit `ff8033cd933d0c6adf972706eb1bb38407bdb7c6`. This is EGM stage 9, the Implementation Readiness Test. It reviews a revision of the design that was approved at `f9d4a65f`.

Counting rule: a finding about unchanged text counts only if the revision made it wrong. Findings already raised at stages 5 to 8 (Z-1 to Z-6, D-1 to D-12, I-1 to I-8 and their Nits) are not repeated. Two items about unchanged text that later code changes broke, not the revision, are listed after the Nits under "Not counted".

The synthesizer checked two claims against the target before writing this report: R-1 (the carry cases in T041-5, lines 2550-2558) and R-3 (line 2531 against line 1405-1406). Both hold.

## Reviewer Status

| Reviewer | Status |
| --- | --- |
| `egm-implementation-readiness` | completed |
| `egm-codebase-details` | completed (about 200 references checked; most exist as described or are marked new) |
| `egm-style` | completed (every D, G, O, FR, SC, NFR, ADR and section id cited in the range exists) |

Migration numbering (codebase-details): the newest migration is `20270201000100_step_log_quotas_validate`. The tasks use `<timestamp>` placeholders, so nothing clashes. The next number must sort after `20270201000100`.

## Prioritized Findings (Blockers -> Major -> Nits)

### R-1. T041-5 and T041-6 test a carry that only T041-36 builds - Severity: Blocker
* **Reviewer Lens:** `egm-implementation-readiness`
* **Affected Scope:** `Lines 2550-2559` (T041-5), `2629-2631` (T041-6), `3168` (Verification Plan). New text.
* **Discrepancy Description**: These tests need a carry:
  * T041-5's held-lock tests: "past *b* a carry answers `tenant_quota_busy` and its report commits"; "the ensure gives a carry its fast path"; "a reserved carry writes with no subtransaction"; "a carry with nothing to charge opens no subtransaction".
  * T041-6's lease test: "K − 1 ingests and a carrying report commit while one ingest is held after T8".

  The carry function `sagat_step_log_carry` (line 2761), the carrying entry points (2751) and `StepLogCarryTx` (2780) all arrive with T041-36. T041-36 lands after T041-5, T041-6, T041-32, T041-10, T041-7 and T041-8. The prototype versions of these tests in `tests/performance/scalability-ha/step_log_leases_tests.rs` run against `spike_041`, not the built schema. So neither task can meet its "Done when", and the tests-first rule (2078-2082) cannot be followed.
* **Rationale & Recommendation**: Move every carry case of T041-5, and the carrying-report half of T041-6's lease test, into T041-36's test list. In T041-5 and T041-6, keep only the cases a plain INSERT or an ingest can exercise. Fix row 3168 to match.
* Recommended disposition: fix. As written, two tasks have done conditions that their own slice cannot meet.

### R-2. The purge pass's call to the idle reclaim has no task that can build it - Severity: Major
* **Reviewer Lens:** `egm-implementation-readiness`, `egm-codebase-details`, `egm-style`
* **Affected Scope:** `Lines 2543, 2572, 2596-2606` (T041-5, new); `2682-2697` (T041-10, unchanged but made incomplete).
* **Discrepancy Description**: T041-5 builds "the purge pass calling the idle reclaim one tenant a transaction". The step-log purge pass only exists from T041-10, which lands later, and `MaintenanceStore` (`crates/application/src/maintenance.rs:134`) has no step-log method. T041-5's Files leave out `crates/scheduler/src/passes.rs`, `crates/application/src/maintenance.rs` and `crates/infra-postgres/src/maintenance.rs`. T041-10 never mentions `sagat_quota_step_log_idle_tenants`, `sagat_quota_step_log_reclaim_idle`, the per-tenant loop, the rule to skip when the fence is held, or any test for them. T041-5's test "a purge of them releases the same" (2572) names a process that does not exist yet. Without this call, the quota leases of idle or ended backends are never reclaimed.
* **Rationale & Recommendation**: Move the call into T041-10 and specify:
  * the `MaintenanceStore` method (name and signature);
  * the per-pass bound passed to `idle_tenants(max)`;
  * the `{kind="idle"|"ended"}` metrics;
  * the files;
  * the two prototype tests, ported: `the_idle_reclaim_folds_and_deletes_only_leases_idle_past_the_interval` and `the_lease_of_an_ended_backend_is_reclaimed_at_once`.

  In T041-5, write "a `DELETE` of them" in the test.
* Recommended disposition: fix. No task owns a piece the quota-lease design needs in order to free room.

### R-3. T041-5 drops the stream run index that section 3 and data-model.md keep - Severity: Major
* **Reviewer Lens:** `egm-implementation-readiness`
* **Affected Scope:** `Lines 2531-2532`. New text. Compare line 1405-1406, `specs/041-step-logs/data-model.md:93` and line 2297.
* **Discrepancy Description**: T041-5 says "on streams no run index ... D041-111". Section 3 says "the table keeps its run index (D041-112 ...)". data-model.md creates `step_log_streams_run_idx` "-- kept (D041-112)", and T041-35's third run records the index as kept. `listRunLogStreams` (T041-11) depends on that index.
* **Rationale & Recommendation**: Rewrite T041-5 as "on streams the run index of data-model.md §2 (kept, D041-112)".
* Recommended disposition: fix. As written, the migration would contradict both the design and the data model.

### R-4. T041-36's migration is missing from its Files, from the upgrade-boundary move and from the exact privilege list - Severity: Major
* **Reviewer Lens:** `egm-implementation-readiness`, `egm-style`
* **Affected Scope:** `Lines 2761-2764, 2869-2879`. New text. Compare 2510, 2569-2570, 2580-2582.
* **Discrepancy Description**: T041-36 creates `<timestamp>_step_log_carry`. Three things are missing:
  * Its Files list no `.up.sql` or `.down.sql`.
  * It does not say that the upgrade-boundary oracle moves to the new migration, and it does not list the three `release_upgrade*.rs` oracle files. `scripts/check-upgrade-boundary.py` requires those oracles to name the newest migration, and the check is not in Done-when.
  * It grants `EXECUTE` on `sagat_step_log_carry` to `sagat_app`. T041-5 makes the privilege audit in `tests/integration/platform-foundation/runtime_privileges.rs` check "exactly that list" of functions, so the new grant breaks it, and T041-36 does not list that file.
* **Rationale & Recommendation**: Add the two migration files, the oracle move with its three files, `runtime_privileges.rs` with `sagat_step_log_carry` added to the list, and `python3 scripts/check-upgrade-boundary.py` to Done-when.
* Recommended disposition: fix. The checker and the privilege audit would both fail on T041-36's pull request.

### R-5. A T041-36 test reads a route that T041-11 builds later - Severity: Major
* **Reviewer Lens:** `egm-implementation-readiness`, `egm-style`
* **Affected Scope:** `Lines 2778-2779`. New text.
* **Discrepancy Description**: The test expects "its stream says `tenant_quota_busy` on the streams route". `listRunLogStreams` arrives in T041-11 (Slice C), after T041-36. T041-6 avoids the same problem on purpose by asserting through SQL "(the read routes come in T041-11)".
* **Rationale & Recommendation**: In T041-36, assert `step_log_streams.ceiling_reason = 'tenant_quota_busy'` by SQL. Move the route-level assertion into T041-11's tests.
* Recommended disposition: fix. The test cannot run when its task lands.

### R-6. The store seam of the ensure is not specified - Severity: Major
* **Reviewer Lens:** `egm-implementation-readiness`
* **Affected Scope:** `Lines 2765-2773`. New text. Context at 1240-1246 and 1255-1263.
* **Discrepancy Description**: T041-36 refers to "the store's method that begins a carrying report ... returns the closed `CarryRoom`". It never gives:
  * the method's name or signature;
  * which traits carry it. Today a report begins through `RunStore::begin(tenant, run, lock)` (`crates/application/src/runs.rs:479`), and the scoped path begins through `begin_path` and `begin_current_path` (`crates/application/src/execution.rs:747-758`);
  * how the `Tx` and the `CarryRoom` come back together;
  * where the per-(backend pid, tenant) room map lives, its type, its eviction rule and its concurrency control;
  * how the store learns the backend pid of a pooled connection before that connection's first carry without a statement of its own (line 1262). sqlx pooled connections hold no user state.
* **Rationale & Recommendation**: Name the method on `RunStore` and `PathExecutionStore`, for example `begin_carrying(tenant, run, lock, planned: u64) -> Result<Option<(Self::Tx, CarryRoom)>, StoreError>`. Name where the room map lives and what it looks like. State the first-use rule: an unknown pid counts as "unknown" and the ensure runs.
* Recommended disposition: fix. These are API design choices that would otherwise fall to each implementer.

### R-7. The test wrapper of `StepLogCarryTx` has no way into the report path - Severity: Major
* **Reviewer Lens:** `egm-codebase-details`
* **Affected Scope:** `Lines 2780-2782, 2794-2795, 2857-2860`. New text.
* **Discrepancy Description**: A wrapper in `tests/support/step_log_carry.rs` is supposed to pause before the carry call, and the order-guard test depends on that pause. The only implementations of `RunTx` and `PathExecutionTx` are `PgRunTx` (`crates/infra-postgres/src/execution.rs:4153`) and `PgPathExecutionTx` (`execution/path.rs:5`). The only implementations of `RunStore` and `WorkerStore` are on `PgRunStore`. `WorkerService` gets its transaction from the real store, so the wrapper has nowhere to sit. The only seam of this kind is `HookSlot` / `with_report_hook` (`execution.rs:149-177`, `report_hook.rs:54`), and T041-36 deletes it.
* **Rationale & Recommendation**: Decide the seam in the design. Either keep a test-only slot on `PgRunStore` modelled on `HookSlot`, or pause on the database side with a `BEFORE INSERT` trigger waiting on an advisory lock (the pattern at 2820-2822). The second option pauses inside the call rather than before it, so the order test would need restating.
* Recommended disposition: fix. As written, the order-guard test cannot be built.

### R-8. The Rust side of the quota-lease metrics has no task - Severity: Major
* **Reviewer Lens:** `egm-implementation-readiness`
* **Affected Scope:** `Lines 2528-2606` (T041-5), `2682-2697` (T041-10), `2775-2778` (T041-36), `3115` (T041-25). New text. Context at 1373-1382 and 1840-1843, and data-model.md §3b and §7.
* **Discrepancy Description**: data-model.md §7 gives T041-5 the lease-row gauge and the refill counters. They are counted inside SQL and published only by the replica holding shard 0, through `sagat_quota_step_log_counters()`. T041-5 builds only the SQL function. No task names `crates/telemetry/src/metrics.rs` or builds the shard-0 publisher. These metrics have no task that emits them:
  * `step_log_quota_refills_total{kind}`;
  * `step_log_quota_lease_rows`;
  * `step_log_purge_failures_total{cause}` (absent from T041-10);
  * `step_log_quota_refusals_total{cause}` (absent from T041-36).

  Without them, T041-25's check that every alert names an emitted metric fails for `SagatStepLogQuotaHeldRefusals`.

  A related gap: T041-36 counts a batch's refill `55P03` as `caller="ingest"`, but data-model.md gives the `caller="ingest"` counters to T041-6, whose metric list has neither `fallbacks_total` nor `refill_budget_exceeded_total`. No task says how an ingest learns that its plain INSERT refilled.
* **Rationale & Recommendation**: Assign each metric to a task, with its file. Describe the shard-0 publisher: the pass, the interval, and how the monotonic values are exposed. Settle who owns the `caller="ingest"` counters and define the refill signal that the ingest reads.
* Recommended disposition: fix. The observability contract test would fail, and the ADR-0097 operational signals would be missing.

### R-9. Removing the `perf-spike` feature in T041-36 breaks files that no task lists - Severity: Major
* **Reviewer Lens:** `egm-implementation-readiness`, `egm-codebase-details`
* **Affected Scope:** `Lines 2216, 2422-2424` (T041-35 Files), `2796-2797, 2869-2879` (T041-36). New text.
* **Discrepancy Description**: T041-36 removes "T041-35's report hook, its features and the driver's carry mode". These files are not listed:
  * `crates/infra-postgres/src/report_hook.rs`, whose own header says T041-36 removes it;
  * `.github/workflows/perf-spike.yml:60`, which builds with `--features perf-spike`. Its header comment (line 3) never says how the file is kept out of `develop`;
  * `tests/performance/scalability-ha/run.sh:93`, `carry_spike_rounds.sh:12` and `carry_diag_rounds.sh:12`;
  * `step_log_diag.rs`, which has `cfg(feature = "perf-spike")` at :33, :91, :241 and :251;
  * `step_log_leases_tests.rs`, gated at `step_log_spike.rs:744-745`.

  Once the feature leaves `tests/Cargo.toml:18`, the leftover `cfg` lines should trigger `unexpected_cfgs`, which fails clippy under `-D warnings`. The workflow stops building. The 34 prototype lease tests stop compiling, although T041-5 cites them as its pattern and `validation-report.md:568` runs them. The plan does not say whether `carry_spike_judge.py` survives.
* **Rationale & Recommendation**: List each file for T041-36 as deleted, changed or kept, and correct T041-35's Files for the record. Say what `perf-spike.yml` runs afterwards, and what happens to the prototype lease tests once T041-5's built tests replace them.
* Recommended disposition: fix. Following the task as written would break the build.

### R-10. T041-9's regimes have no environment, setup or round rule - Severity: Major
* **Reviewer Lens:** `egm-implementation-readiness`, `egm-codebase-details`
* **Affected Scope:** `Lines 3059-3104`, especially `3073-3101`. New text. This finding is about how to set the regimes up and run them, which I-2 and D-1 did not cover.
* **Discrepancy Description**:
  * The "behind a transaction pooler" regime needs PgBouncer in transaction mode. Nothing in the repository installs it, and no version, `pool_mode` or pool size is given.
  * The regimes with 100 and 1,000 tenants, and the usage row held for 150 ms and 400 ms, need driver setup that is not described.
  * Unlike T041-35, T041-9 does not say where it runs (the hosted runner of D041-102 or another machine).
  * It gives no rounds, no validity rule and no median rule.
  * Its Files are only `main.rs`, `step_log_spike.rs` and `validation-report.md`, with no `run.sh` or workflow.
* **Rationale & Recommendation**: State the environment, the rounds and the validity and median rules (or "as T041-35's fourth run"), the PgBouncer setup with its files, and how multiple tenants and the held usage row are set up.
* Recommended disposition: fix. The criterion fixed in this task cannot be judged until the protocol is fixed too.

### R-11. The SDK fault plans need a "hold the answer" action that no task defines - Severity: Major
* **Reviewer Lens:** `egm-implementation-readiness`
* **Affected Scope:** `Lines 2976-2978` (Slice D preamble), `2985-2993` (T041-14); the scenario at 1703-1708. Changed text.
* **Discrepancy Description**: `step-logs-transport-faults` now holds a batch's answer at the fault proxy, and the report may only be sent once the answer is released or 2 seconds have passed. No SDK fault plan has a hold action today. Go's `faults.go` supports only `failBefore`, `status` and `dropResponse`. The preamble lists only the `verdict` fault. T041-14 does not define a hold action, how it is released, or how the harness observes when the report is sent relative to the release.
* **Rationale & Recommendation**: Define the action in T041-14: its name, its JSON fields, its release (by time or by a harness op), and the evidence the golden asserts. List it beside `verdict` in the preamble.
* Recommended disposition: fix. Otherwise each of the six SDKs would invent its own version, which goes against rule 8 of AGENTS.md (conformance without forking).

### R-12. T041-6's "hold inside a refill" hook cannot sit on a `StepLogTx` method boundary - Severity: Major
* **Reviewer Lens:** `egm-implementation-readiness`
* **Affected Scope:** `Line 2627`; used at `2630-2631`. New text.
* **Discrepancy Description**: The wrapper hooks between the adapter's statements. A refill runs in plpgsql, called from T8's statement trigger, so Rust has no point inside it where it can pause. The plan also does not say how the fixture forces T8 to refill instead of charging a free quota lease.
* **Rationale & Recommendation**: Rename the hook "hold after a T8 that refilled (the usage row held)", and state how the fixture guarantees a refill. Two options: a limit set so that the writer has no quota lease, or a reclaim beforehand.
* Recommended disposition: fix. The hook cannot be built as described.

### R-13. T041-5's per-replica `application_name` has no file, no value and no caller - Severity: Major
* **Reviewer Lens:** `egm-codebase-details`, `egm-implementation-readiness`, `egm-style`
* **Affected Scope:** `Lines 2545-2546, 2596-2606`. New text.
* **Discrepancy Description**: `crates/infra-postgres/src/pool.rs:70` hard-codes `.application_name("sagat-octopus-orchestrator")`. `PoolConfig` (`pool.rs:30-52`) has no field for a name or a replica id. Neither `pool.rs` nor the apps that build `PoolConfig` are in T041-5's Files, and no test is named. The style lens adds that this Rust work sits inside the migration's parenthetical list, along with the quota readers and the purge-pass call.
* **Rationale & Recommendation**: Name the value (for example the instance id from `20270113000000_instance_registry`), the `PoolConfig` field, its callers and a test, and add the files. Split T041-5 into "Migration:", "Rust:" and "Tests:" paragraphs.
* Recommended disposition: fix. Otherwise the implementer must invent the replica identity that the lease attribution depends on.

### R-14. The `CeilingReason` fitness test is unlisted, never extended and not selected by CI - Severity: Major
* **Reviewer Lens:** `egm-codebase-details`, `egm-implementation-readiness`
* **Affected Scope:** `Lines 2495-2496, 2516-2518, 2521-2527` (T041-4), `2528-2606` (T041-5), `2662-2675` (T041-32). New and changed text.
* **Discrepancy Description**: The test `the_contracts_ceiling_reasons_are_the_domains`, in `tests/contract/platform-foundation/step_log_ceiling.rs`, run by target `contract_platform_foundation`, is not in T041-4's Files or Done-when. T041-4 says "T041-5 extends it to the migration's `ceiling_reason` CHECK", but T041-5 never mentions it. None of T041-32's selector rules picks `contract_platform_foundation` when `crates/domain/src/step_logs.rs` changes, so a pull request that adds a variant would not run the guard.
* **Rationale & Recommendation**: Add the file and the target to T041-4. Add the extension to T041-5's text and Files. Add `contract_platform_foundation` to T041-32's first selector entry.
* Recommended disposition: fix. As written, the guard against drift between the domain and the contract never runs on the change it protects against.

### R-15. Verification Plan row 3168 says a carry's refill answers `contended` at once - Severity: Major
* **Reviewer Lens:** `egm-codebase-details`, `egm-style`, `egm-implementation-readiness`
* **Affected Scope:** `Line 3168`. Changed text. Compare T041-5 at 2550-2552 and T041-36 at 2829-2833.
* **Discrepancy Description**: The row keeps the NOWAIT refill of D041-112, which D041-117 and D041-119 replaced. The tasks and the prototype (`step_log_leases_tests.rs:918` and `:1432`, with `lock_timeout` 200 ms at `step_log_spike.rs:913`) say the refill waits at most *b*, after which the report answers `tenant_quota_busy` and commits. `contended` is the answer when a fence or the stream lock is held. I-1 covered rows that are missing, not this contradiction.
* **Rationale & Recommendation**: Restate the row: "a carry's fallback refill waits at most *b* and then answers `tenant_quota_busy`, the report landing (T041-5, T041-36)".
* Recommended disposition: fix. Anyone writing tests from the table would assert the wrong answer.

### R-16. "Until it passes" is never resolved for a run that was accepted rather than passed - Severity: Major
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** `Lines 2190-2191, 2248-2252, 2419, 2438-2441, 2443, 2487-2496`. New text.
* **Discrepancy Description**: T041-35 holds T041-3, T041-4 and the later tasks "until it passes", and puts ADR-0095 off until T041-35 passes (D041-115). The run failed at 1 × 16, and the owner accepted the cost (OD-2026-11-20-13). Yet T041-3 records ADR-0095 as accepted, and T041-4 says nothing about whether it may now be ticked. A reader must guess whether the acceptance counts as a pass.
* **Rationale & Recommendation**: In the Settled evidence, state that OD-2026-11-20-13 stands in for a pass for every gate that T041-35 held, and name those gates (T041-3, T041-4, T041-5, O041-038). Give T041-4 the same "No longer waits" line.
* Recommended disposition: fix. One sentence fixes the gate status of three tasks.

### R-17. T041-5 drops a constraint that no longer exists - Severity: Major
* **Reviewer Lens:** `egm-style`, `egm-implementation-readiness`, `egm-codebase-details`
* **Affected Scope:** `Lines 2546, 2600-2601`. New text.
* **Discrepancy Description**: The migration's content list includes "T041-4's `tenant_retained_usage_step_log_nonnegative` dropped". Files says that constraint was already removed from T041-4's migrations on 2026-11-20 (D041-113). `20270201000000_step_log_quotas.up.sql:99-101` indeed has no such CHECK, and `data-model.md:210` agrees. A literal `DROP CONSTRAINT` would fail. Files also lists T041-4's migrations as modified by T041-5, although they land in an earlier pull request. That migration's comment still says "ADR-0097 (Proposed)" and cites D041-112.
* **Rationale & Recommendation**: Delete the clause from line 2546, and remove "T041-4's migrations" from T041-5's Files. State once, in T041-4, that its migrations never create the constraint. Fix the migration comment.
* Recommended disposition: fix. Followed literally, the step fails the migration.

### R-18. "16 to 21 of the writer leases" is undefined, and the label's scope is unclear - Severity: Major
* **Reviewer Lens:** `egm-style`, `egm-implementation-readiness`, `egm-codebase-details`
* **Affected Scope:** `Lines 2558-2560, 2567`. New text.
* **Discrepancy Description**: The phrase refers to hand mutations numbered at `validation-report.md:920-929` and in a review handoff file. Neither is cited. The colon after "a caller's lower `lock_timeout` is kept:" leaves open whether the label covers that one test or every held-lock test before it. `step_log_leases_tests.rs` is cited without its path.
* **Rationale & Recommendation**: Close the held-lock list before the label. Then write "each held-lock test shown failing under its hand mutation (mutations 16 to 21 of `validation-report.md`, D041-120)", and give the path `tests/performance/scalability-ha/step_log_leases_tests.rs`.
* Recommended disposition: fix. The evidence that closes T041-5 depends on knowing which mutations apply.

### R-19. One T041-6 test has two possible outcomes - Severity: Major
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** `Line 2639`. New text.
* **Discrepancy Description**: "then the resend's `FINAL_ONLY` at the limit, or its charge once held room is freed" gives one test two results and does not say which setup produces which.
* **Rationale & Recommendation**: Make two cases. In the first, the resend is at the limit and gets `FINAL_ONLY`. In the second, the resend comes after the holder of the room commits, and is stored and charged.
* Recommended disposition: fix. As written, a test can pass either way, so it proves nothing.

### R-20. "Lease" means both the task lease and the quota lease - Severity: Major
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** Task lease at `2636, 2729, 2861, 3068, 3080`. Quota lease at `2627, 2629, 2836, 3168`. The task-lease uses are unchanged but invalidated by the revision; the quota-lease uses are new.
* **Discrepancy Description**: ADR-0097 introduced a quota lease beside the worker's task lease, and the range uses the bare word for both. In T041-36, "a short lease" (2836) means quota room, while "the lease expired between its two transactions" (2861) means the task lease. T041-9 mixes "lease calls" (worker RPCs) with "the quota leases' regimes".
* **Rationale & Recommendation**: Write "quota lease" and "task lease" throughout the range.
* Recommended disposition: fix. The terms are fixed mechanically, while a misreading would lead to the wrong test.

### R-21. T041-1 is ticked although its done condition is false now, and no task owns this revision's review - Severity: Major
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** `Lines 2097-2103`; header line 4 (`Status: Draft`). New text.
* **Discrepancy Description**: T041-1's done condition is an approval block with `Status: Approved`, but the document is now Draft. T041-33's appeal to R9 lets tasks ticked on develop stand. Still, no task covers the current re-review and approval pull request, so a reader must guess which task this review counts against.
* **Rationale & Recommendation**: Add an evidence line to T041-1 saying that this revision's re-review runs under it and will be cited by the new approval block. Alternatively, open a new task for the review and record it in the Owner-gates list of additions.
* Recommended disposition: fix. One evidence line makes the tick honest under rule 17 of AGENTS.md (no claim without evidence).

### Nits

1. **Which migration the oracle moves to** (`egm-implementation-readiness`), lines 2509-2510, changed. T041-4 now creates two migrations. Name `<timestamp>_step_log_quotas_validate` as the one the oracle moves to.
2. **Wrong line for the selector rule** (`egm-codebase-details`), line 2666, new. `scripts/select-test-targets.py:71` is `SDK_LANGUAGES`. The budget rule is at :85-86, and it also covers `runs.rs`.
3. **Unstated change to `retained_usage`** (`egm-codebase-details`), lines 2602-2603, new. T041-5 modifies `content_usage.rs` `retained_usage` (:146-173) and `RetainedUsage` without saying how. Nothing calls the function, and it raises when a counter is negative, while S may go below zero under ADR-0097. Say what changes, or drop the entry.
4. **Two places hold the switch** (`egm-codebase-details`), lines 2622 and 2755-2756, new. Say that `tests/support/api.rs` copies `Options.step_logs_enabled` into `worker: WorkerSettings` (`api.rs:248`), so the two cannot disagree.
5. **T041-25 starts two more rule tests** (`egm-codebase-details`), line 3107. Running every `*_test.yaml` also starts `instance_registry_test.yaml` and `trigger_schedules_test.yaml`, which CI does not run today. Name them.
6. **No duration rows for the new targets** (`egm-codebase-details`). `integration_step_logs` (T041-6) and `e2e_step_logs` (T041-26) have no row in `scripts/integration-suite-durations.toml`, so the selector plans each at 600 s. Add the rows in those tasks.
7. **SDKs still at 1.5** (`egm-codebase-details`), T041-36 and T041-7. `sdks/rust/src/worker.rs:468` refuses `accepted > 6`. T041-7 raises it, but the Go, Python, .NET, Java and TypeScript constants stay at 1.5 until their SDK tasks. Say that `protocol-negotiation.json` must stay valid for SDKs at 1.5.
8. **Emphasis eats path globs** (`egm-style`), lines 2106 and 2109. Put `2026-11-15/*-approval*.md` and `2026-11-16/*-reread*.md` in backticks.
9. **Mixed size notation** (`egm-style`), lines 2181 and 2427-2429. "1 x 16" sits beside "1 × 16". Use × throughout.
10. **S has two meanings** (`egm-style`), line 2544. S is the service time at 2164, 2233 and 2248, and the folded usage here. Write "the folded usage S (section 2b)".
11. **The delta sentence misparses** (`egm-style`), lines 2492-2494. It reads as removing `validate_carry`. Split it into "removes ..." and "adds `validate_carry`, `in_order` and `CeilingReason::TenantQuotaBusy`".
12. **"No longer waits:" is undefined and placed inconsistently** (`egm-style`), lines 2071-2073, 2443, 2593 and 2746. Define the line in the Owner-gates rule and place it consistently.
13. **T041-6's hook labels are stale** (`egm-style`), lines 2625-2627, unchanged but invalidated. "Hold after T3" leaves out the stream lock that T3 now takes, and "hold after T7" no longer names where the attempt is serialized. Restate both against section 2 (T3, L1b).
14. **The second run's rules are misreported** (`egm-style`), lines 2263-2264 and 2292. The second run claims "those of the first run" but gives 45 minutes and "not quiet", against the first run's 10 minutes and "not run". The third run inherits "four runners", although the second run used one job of three rounds, and D041-104 is not cited.
15. **A switch is missing from the Switches list** (`egm-style`), lines 2194-2195. Add `SAGAT_PERF_STEP_LOG_CARRY_RUN=3`, used at 2285.
16. **Forward references** (`egm-style`), lines 2203 and 2211-2212. "Gates (1) to (3)" and the workload letters are used before their definitions at 2217-2238.
17. **Rewrap artifacts and over-long lines** (`egm-style`). Broken wraps at 2212-2213, 2215-2216 and 2402-2403. Over-long lines at 2197, 2254, 2268, 2413, 2418, 2496, 2536-2537, 2546, 2567-2568, 2575, 2583, 2780, 2784 and 2805.
18. **Paths that are not repository-relative** (`egm-style`), against the Paths rule at line 2093. `pool.rs:70` (2545), `run.sh` (2277), `carry_spike_judge.py` (2280 and 2364), `step_log_spike.rs` (2236).
19. **One amendment, four names** (`egm-style`), lines 2124, 2534, 2536 and 2747. Name ADR-0027's amendment of 2026-11-20 once and reuse that name.
20. **One actor, three names** (`egm-style`), lines 2131, 2273, 2306, 2324 and 2377. "Coordinator", "coordinating session" and "delegate" (undefined) mean one actor. Pick one name and define it.
21. **T041-25's Files omit `docs/runbooks/scaling.md`** (`egm-style`). The body changes it at 3110-3111.
22. **Stray article** (`egm-style`), lines 2541-2542: "the `sagat_quota_step_log_reclaim`".
23. **The Follow-up tasks section only repeats line 2077** (`egm-style`), lines 3149-3152. Write "None".
24. **T041-36 repeats its ensure tests and leaves out the warm and cold pins** (`egm-style`). Lines 2778-2779 repeat the tests at 2826. "The six equalities" (2799) leave out the warm and cold pins that section 2a requires (1157-1159).
25. **Cryptic phrases** (`egm-style`). "T9's is T041-6's" (2791), "so the wait is the test's" (2822-2823), "an ensure whose usage row" (2828), "which each call returns" (2768), "takes the block" (2836), "(backends × tenants first charges)" (3078).
26. **Passive voice hides the actor** (`egm-style`). At 2446-2447, name the compatibility test's baseline update. At 2701, say who holds the release.
27. **"Row" is overloaded** (`egm-style`), lines 2355 and 3074. Write "measurement" for the benchmark sense.
28. **Verification row 3187 ignores T041-36** (`egm-style`), unchanged but invalidated. The row says every function keeps `SECURITY DEFINER`, but `sagat_step_log_carry` is `SECURITY INVOKER`. Add T041-36's grants.
29. **"Section 1" next to "contracts.md §1"** (`egm-style`), line 2453, unchanged but invalidated. Write "section 1 of this design".

### Not counted (unchanged text broken by later code changes, not by the revision)

- **CI references in the Gates rule, T041-11 and T041-25** (`egm-codebase-details`), lines 2083-2085, 2904-2906 and 3106. The `rust` job no longer runs integration suites; they run in the sharded `integration` job (`.github/workflows/ci.yml:130-219`). The `contains(format(...))` form is now `matrix.shard.targets` (:155, :166, :173). The promtool step is at :682-687, not :617-622. The integration preflight steps run selected suites as the runtime role on pull requests, which the Gates sentence denies. T041-11's new `horizon_` step therefore belongs in the `integration` job. The Elephant may want to fix this alongside R-9, because an implementer of T041-11 would otherwise edit the wrong job.
- **Stale line numbers** (`egm-codebase-details`):
  - `tests/support/mod.rs` `eventually` is at :413-429, not :383-399 (line 2092);
  - `apps/execution-engine/src/lib.rs` builds `WorkerGrpc` at :788, not :775, and `tests/support/api.rs` at :632, not :616 (line 2621);
  - `crates/infra-postgres/src/quotas.rs` holds the secrets-lock test at :1329-1357, not :1323-1344 (line 2613);
  - `crates/api-grpc/build.rs` calls `compile_fds_with_config` at :37-40 (line 2459);
  - `tests/contract/payload-classification/openapi.rs` has the additions at :349-361 and the digest at :390-391 (lines 2466 and 2954);
  - `scripts/select-test-targets.py:71` (line 3159).

Counts: Blocker 1, Major 20, Nit 29
