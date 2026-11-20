# Handoff: design 041, task T041-35, decision D041-120 (2026-11-20)

This note is for the agent that continues this work. The worktree is `/Users/marcelo/Documents/Projects/sagat-wt-041` and
the branch is `feature/041-step-logs-server` (pull request #241). The commit this note describes is the one that adds it.
The commit before it is `f8c855a4` (the prototype of decision D041-120), and the documents of decision D041-120 are in
`d1b113a9`.

## Where things stand

The coordinating session approved decision D041-120 (points 1 to 8), including the second setup step of the fourth run.
Two owner rules apply to all new writing:

- **OD-2026-11-20-8:** documents are the source of truth. Change the document first, then the code, and cite the
  document's section in each commit's `Refs:` line.
- **OD-2026-11-20-9:** write words in full, not abbreviations ("Major 3", "mutation 16", "decision D041-120"). The
  review reports are verbatim records and stay as written.

Done and pushed:

- **Documents of decision D041-120** (commit `d1b113a9`):
  - ADR-0097: decisions 1, 3b and 4, the held-room bound, options 1g and 1h, the consequences, and the acceptance clause, now question O041-042.
  - ADR-0095: decisions 2, 4, 5 and 7, and the consequences.
  - Design 041:
    - the overview, the revision header, FR-041-008 and FR-041-005;
    - "What a report can wait for" and the lock order;
    - section 2b: the ensure, the fallback, the budget, the held-room bound, idle and ended leases, regimes, observability;
    - section 8: the Studio texts;
    - section 9: the alerts;
    - the budget table, with an ensure row and warm and cold equalities;
    - the Risks and Constitution Check rows and Complexity Tracking;
    - tasks T041-5, T041-9 (lease scenarios with criteria) and T041-36 (the ensure item, the ensure tests and the lock test);
    - task T041-35's fourth run declaration, described below.
  - Data model: sections 1, 2, 3b (ensure, idle tenants, reclaim of idle and ended leases), 4 (one privilege list), 7 and 8a (the carry's return gains `cause`, `writer` and `room`).
  - `contracts.md`: the `ceiling` value `tenant_quota_busy`, with its description.
  - The planned `specs/001-platform-foundation/contracts/openapi.yaml` (line 7636) and `frontend/apps/studio/src/api/schema.d.ts` (line 6080), both edited by hand. `npm run api` and `npm run check:api` have NOT been run: there is no `node_modules` in `frontend/apps/studio`.
  - `research.md`: decision D041-120, gaps G041-025 (restated), G041-026 and G041-027, questions O041-041 and O041-042, and corrections to decision D041-117 and gap G041-023.
  - `docs/engineering/architecture-principles.md`: the step-log row.
  - `docs/adr/README.md`, and the notes in ADR-0027 and ADR-0084.
- **Prototype** (commit `f8c855a4`, `tests/performance/scalability-ha/step_log_spike.rs`):
  - `spike_041.step_log_ensure(tenant, bytes)` returns `(outcome, writer, room)`. The outcomes are `invalid`, `contended`, `room`, `limit`, `made`, `busy` and `held`. It never raises for these.
  - `spike_041.step_log_idle_tenants(max)`.
  - `spike_041.step_log_reclaim_idle(tenant)` also folds the leases of ended backends (the writer's pid is not in `pg_stat_activity`). It checks before taking any lock and tries the fence.
  - `step_log_refill_wait()` is `STABLE`.
  - A carry with `v_kept = 0` writes with no block.
  - `prewarm` (setup, when `PREWARM_LEASE` is set) folds ended leases, leases each filled connection 1 MiB, and charges it the share *u* = ((*i* + 1) × 2,654,435,761) mod 2^20, where *i* is the connection's index in that pool's fill.
  - `RowResult.lease_rows`.
  - The gate JSON (`main.rs`, `spike-carry-json`) gains `carry_over_p99_by_path` (fast, refill, mark-only), `carry_refill_max_ms`, `carry_mark_max_ms`, `lease_rows` and `carry_errors`.
- **Tests**, in `tests/performance/scalability-ha/step_log_leases_tests.rs`:
  - New: `the_ensure_answers_without_raising`, `the_lease_of_an_ended_backend_is_reclaimed_at_once`, `a_carry_with_nothing_to_charge_opens_no_subtransaction`, `a_callers_lower_lock_timeout_is_kept`.
  - `the_ensure_before_a_report_gives_the_carry_its_fast_path` now checks the outcome, the pid and that an ensure charges nothing.
  - The staged tests' budget is replaced by 300 s through `without_the_refill_bound`, so that mutation 21 exceeds the 60 s test bound.
  - The whole performance test binary passed: 50 tests, 10 ignored benchmarks. The change from 60 s to 300 s was made after that run; it passed `fmt` and was committed without a rerun.

## Left to do, in order

1. **Rerun the lease tests**, because the change to 300 s has not been run yet.
2. **Run the hand mutations of decision D041-120.** Each must fail its named test; restore the file after each one.
   Record them in `specs/041-step-logs/validation-report.md`, after the table of mutations 9 to 15, with words in
   full, then commit. Each mutation replaces one text exactly once in `step_log_spike.rs`:

   | Mutation | Change | Test that must fail |
   |---|---|---|
   | 16 | The ensure no longer catches `lock_not_available` | `the_ensure_answers_without_raising` |
   | 17 | The ensure waits for the fence (`pg_advisory_xact_lock_shared`) instead of trying it | `the_ensure_answers_without_raising` (hangs until the bound) |
   | 18 | The ensure charges: `step_log_refill(p_tenant, p_bytes, true, true)` | `the_ensure_before_a_report_gives_the_carry_its_fast_path` |
   | 19 | The reclaim's loop drops the `NOT EXISTS (… pg_stat_activity …)` clause | `the_lease_of_an_ended_backend_is_reclaimed_at_once` |
   | 20 | `IF v_kept = 0 OR spike_041.step_log_reserve(…)` becomes `IF spike_041.step_log_reserve(…)` | `a_carry_with_nothing_to_charge_opens_no_subtransaction` |
   | 21 | `IF v_prev = '0' OR v_prev::interval > v_bound THEN` becomes `IF true THEN` | `a_callers_lower_lock_timeout_is_kept` |

   A script for these was written at
   `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/mut120.sh`.
   It uses `mut117.py` beside it, which replaces exactly one occurrence. The scratchpad may have been cleared; the
   table above is enough to redo them.
3. **The four reviewers' re-read of the changed parts.** Run `correctness-reviewer`, `security-reviewer`,
   `architecture-reviewer` and `egm:egm-design` on `f8c855a4` (or the head), pointing each at its earlier re-read in
   this folder (`rereads-fc4adff7-*.md`) and at decision D041-120. Save each report verbatim as
   `rereads-<sha>-<reviewer>.md`. Answer every Blocker and Major (documents first) until none remains, and write the
   answers in `resolution.md`.
4. **The gate run.**
   - Before any round, confirm that the fourth run's declaration in design 041 (task T041-35, "Fourth run") still
     matches the prototype.
   - Then push `feature/041-step-logs-server` and fast-forward `perf/041-carry-spike` to the head:
     `git push origin HEAD:perf/041-carry-spike`. Never force; the hook refuses it.
   - The workflow `.github/workflows/perf-spike.yml` runs 4 shards × 3 rounds on hosted runners, then `judge`
     (`tests/performance/scalability-ha/carry_spike_judge.py`). `SAGAT_PERF_STEP_LOG_CARRY_RUN=3` is the default in
     `carry_spike_rounds.sh`. A run takes about 70 to 90 minutes.
   - Download the artifacts with `gh run download <id> -D <a new scratchpad folder>`. They are `carry-spike-<sha>-{1..4}` and `carry-spike-<sha>-verdict` (`verdict.txt` and `second-round-N.log`). The `spike-carry-json` lines carry everything.
5. **Record the run.**
   - In `validation-report.md`, as the third run was recorded: the environment, the CPUs and PostgreSQL versions, the 12 rounds with validity and reasons, the gate values and medians for each size, and carries by path and cause.
   - Add the new fields: carries above p99 by path, the longest refill and mark-only waits, the lease rows and the refill share against the expected 0.31% for (a) and 2.0% for (h).
   - Read the result by the declared rules. Add the next decision number (D041-121) in `research.md`.
   - Tick T041-35 only on a pass.
6. **Report to the coordinating session**, with the gate values and medians for each size, the measured
   `mark_only:busy` rate (which goes to the owner with question O041-041) and the refill share.

## The fourth run's declaration (design 041, task T041-35)

- **Setup additions**, approved and declared:
  1. Each row's pool fill folds the leases of ended backends, then leases every filled connection 1 MiB (`step_log_ensure`). This stands in for the built ensure for the first charge only, so the spike is conservative.
  2. Each connection is charged the share *u* = ((*i* + 1) × 2,654,435,761) mod 2^20 bytes. This models a fleet in steady state.
- **Expected refill share:** with the room spread uniformly over (0, 1 MiB], expected refills per row equal the bytes charged divided by 1 MiB, whatever the number of backends.
  - (a): 9,000 × 3,280 = 29,520,000 bytes; ÷ 1,048,576 = 28.2 refills = 0.31% of carries, at both sizes.
  - (h): 9,000 × 64 × 328 ÷ 1,048,576 = 180.2 refills = 2.0%.
- **Expected result:** no `mark_only` answer. (2b) at 8 × 128 is close and may go either way (about +56 ms against an allowance near 61 ms; the comparison is across builds). (1), (2b) and (3) are not expected to pass at 1 × 16. (4) is expected to stay within 10.
- **Reading rules, one consequence for each gate at each size:**
  - (1) or (3) failing at 1 × 16: the carry's known cost on that runner. It goes to the owner as the environment question (decision D041-109).
  - (2b) failing at either size: attributed by the share of `written:refill` and `mark_only` carries among the carries above p99. Above one half, the lever is the refill; below, the lines' p99.
  - (1) or (3) failing at 8 × 128: a design change.
  - (4) failing: the ticket sizing of task T041-31.
  - A round void only because of `mark_only:busy`: recorded as a failure of the refill design, not re-run.
  - Gap G041-025 closes when refills stay within twice the expected share and no carry is answered `mark_only:*`.
  - Gap G041-026 is measured, not closed.
  - Whatever the result, ADR-0097 (question O041-042), ADR-0095 and questions O041-038, O041-040 and O041-041 go to the owner with the record.

## Open items for the owner (the coordinating session puts them)

- **Question O041-041 (gap G041-026):** what happens to a carry's records when its fallback cannot get the usage row within 200 ms. The options are (a) accept the visible loss (built), (b) fail and resend (breaks ADR-0084 decision 7), or (c) write the records after the commit (needs a durable spool or the worker's resend). It goes to the owner with the fourth run's measured busy rate. No recommendation is recorded.
- **Question O041-042:** accept ADR-0097, which stays Proposed.
- **Also with the record:** ADR-0095 (question O041-038) and question O041-040.
- **Gap G041-027:** the ensure needs session affinity. Behind a pooler in transaction mode it degrades to fallbacks. Task T041-9 measures this.

## Commands

All builds use a private target and no debug information. Run `df -h /` first, and run at most two heavy builds at once.

```sh
cd /Users/marcelo/Documents/Projects/sagat-wt-041
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/.cargo-target-041b CARGO_PROFILE_DEV_DEBUG=0 RUST_TEST_THREADS=2
# a local PostgreSQL 18 started by hand for these tests (pg_stat_statements preloaded, track=all, max_connections=400):
#   data directory <scratchpad>/pgsmoke/data, port 55491, binaries /opt/homebrew/opt/postgresql@18/bin, LC_ALL=C
export SAGAT_TEST_DATABASE_URL="postgres://postgres:<the local smoke password>@127.0.0.1:55491/postgres"
cargo fmt --check
cargo clippy -p sagat-tests --all-targets --features perf-spike --locked -- -D warnings
cargo clippy -p sagat-tests --all-targets --locked -- -D warnings
cargo test -p sagat-tests --test perf_scalability_ha --features perf-spike --locked leases_tests   # 34 lease tests
cargo test -p sagat-tests --test perf_scalability_ha --features perf-spike --locked              # 50, 10 ignored
python3 scripts/check-design-docs.py && python3 scripts/check-agent-config.py && python3 scripts/check-architecture.py
```

Stop the local server when done:
`LC_ALL=C /opt/homebrew/opt/postgresql@18/bin/pg_ctl -D <scratchpad>/pgsmoke/data -m fast stop`.

## Things not written elsewhere

- **Performance measurements:** never run them on the laptop, which is shared (load 60 to 85). Measure only on the hosted runners. Docker Desktop is hung; do not use, restart or prune it.
- **Diagnosis runs**, not gates: push to `perf/041-diag-split` (set `split`, `carry_diag_rounds.sh`).
  - Its summary script was in the scratchpad (`tools/split_summary.py`). It reads `spike-diag-json` lines of `*/diag.log` and prints, per label and size, rates, path counts, times by path and lease snapshots.
  - The gate's table script was `tools/gate3_tables.py`. It reads `verdict.txt` and `second-round-N.log` of the verdict artifact.
- **`check-design-docs.py`:** compares against the tip of `origin/develop` (Rule 8 on the constitution). If develop changes the constitution again, the check fails until develop is merged. The coordinating session approved merging develop (merge, never rebase). The last merge was `5458820b` (develop `99992c70`).
- **Commits:** no `Co-Authored-By` (a hook refuses it). Use Conventional Commits with a `Refs:` line.
- **Prototype test helpers:**
  - Writers use their own connections outside the pool (`writer(pool)`), because a lease belongs to a backend and the test pool has 8 connections.
  - Tests that stage held locks replace the budget function in their own database (`without_the_refill_bound`).
- **Pending stale text** that the re-reads may still find:
  - task T041-3 does not say that the planned openapi.yaml now carries `tenant_quota_busy`;
  - the lock-order test's set of writers (design, "lock-order test") does not name the holders of `sagat_quota_lock_log_usage`;
  - the T041-36 test text "a carry statement that raises" is unchanged.
