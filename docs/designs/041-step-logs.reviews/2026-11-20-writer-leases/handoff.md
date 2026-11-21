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

## Update after decision D041-121 (2026-11-20, head `77fcb33d`)

Steps 1 to 3 of "Left to do" above are done:
- The lease tests were rerun.
- Hand mutations 16 to 21 are recorded in validation-report.md.
- The Studio types were regenerated, and they are identical to the hand edit.

The review rounds are closed:
- The second re-reads are in `rereview-2/`.
- The confirmation re-reads and the design critic's two narrow re-reads are in `rereview-3/`.
- Each folder has a `resolution.md`.
- The answers are decision D041-121, documents first, each code fix shown by a failing test.

The fourth run was started at `77fcb33d`: `perf/041-carry-spike` was fast-forwarded and the run is 44834024933. Its
record is decision D041-122 (the coordinating session had expected D041-121, which the review answers took).

How to read the run: by task T041-35's fourth-run rules, as restated by D041-121. The judge prints the (2b)
attribution medians: `g2b attribution refill share 8`, `i 8` and `ii 8`.

Not merged: develop moved to `a1d77778`. It merges cleanly, but was left out so that the measured build stays the
declared one.

## The fourth run, recorded (decision D041-122)

**Result.** Run 44834024933 failed at 1 × 16 on gates (1), (2b) and (3) and passed all four gates at 8 × 128. Gap
G041-025 is closed, and gap G041-026 was measured at 1 carry of 864,000. The full record is in validation-report.md.

**What is left is the owner's, through the coordinating session:**
- the environment question (decision D041-109) for the 1 × 16 failures; task T041-5 waits for it;
- questions O041-040, O041-041 and O041-042 (ADR-0097);
- question O041-038 (ADR-0095), which waits for a pass.


## Update after the owner's answers and the approval run's first round (2026-11-20)

**Done:**
- The owner's answers are recorded (commit `ac55c32b`): OD-2026-11-20-13 to -16 in `docs/owner-decisions.md`, decision
  D041-123, O041-038 / -040 / -041 / -042 answered, ADR-0095 and ADR-0097 Accepted, ADR-0027's Amendment of the
  Amendment written, ADR-0084 and ADR-0027 marked partly superseded, `docs/runbooks/step-logs.md` created with the
  1 × 16 cost, `docs/releases/unreleased.md` entry. The design is still `Draft`.
- The approval path is OD-2026-11-20-17, "Full review, capped" (commit `ff8033cd`): the egm-review on the changed text
  and the three repository reviewers at the final commit; if Majors remain after the first round, fix the "fix" items,
  re-read them briefly, accept the rest as recommended with one owner row each citing OD-2026-11-20-17 (written like
  OD-2026-11-16-200 and following); a Blocker is only fixed.
- The first round ran on `ff8033cd`: all five egm-review stages in order (EGM hash
  `003679ce…83f9`, on the accepted list) and the three repository reviewers. Reports and the triage are in
  `docs/designs/041-step-logs.reviews/2026-11-20-approval/` (`consolidated_review_report.md` lists every Blocker and Major
  with its class: text, delegate or owner). Totals: 3 Blockers (A-1, D-1, R-1), 68 Majors, 86 Nits.

**Left, in order:**
1. The coordinating session settles the owner and delegate rows of `consolidated_review_report.md` (A-1 with Z-1 and
   I-3, C-6 and A-2 / D-4 need the owner; D-1, D-2, S-1, C-3, C-4, I-5, I-7 are delegate decisions).
2. Answer every Blocker and Major, documents first, each in a new `resolution.md` of the approval folder; commit.
3. Short re-reads of the fixes at the final commit by the three repository reviewers and one egm-review summary for the
   approval block, each with the header lines `check-design-docs.py` reads (`Doc:`, `Reviewed-commit:` the final commit,
   `Reviewer:` `egm-review` / `architecture` / `security` / `correctness`, `EGM-hash:` for egm-review, `Agent-blob:` the
   blob of `.claude/agents/<lens>-reviewer.md` at that commit, one `Counts:` line, no absolute machine path). The blobs at
   `ff8033cd`: architecture `c6fbfd6f…`, security `c02123dd…`, correctness `d1de651d…`.
4. Owner rows for each finding accepted as recommended (Z-2, D-12 and any Major still open), anchored
   `<report path>#<n>` as the 2026-11-16 rows are; then the approval block (Status Approved, Reviewed-commit, EGM-hash,
   Claude-Code 2.1.329, Reports, Open findings "Blocker 0, Major 0", Accepted), the tick of task T041-35 with its
   evidence (run 44834024933, OD-2026-11-20-13), and `check-design-docs.py` passing.
5. Step 2 of the coordinator's brief: merge the held tasks T041-3 and T041-4 and build through T041-36.

## Update after the approval run's second and third rounds (2026-11-21, head after `c52b5d37`)

**Done (all pushed to `feature/041-step-logs-server`):**
- `4d6fe4cc`: every first-round Blocker and Major answered, documents first (`2026-11-20-approval/resolution.md`):
  owner rows OD-2026-11-20-18 ("Top up after the commit") and OD-2026-11-20-19 ("Hide it without the grant"),
  decisions D041-124 to D041-133, owner questions O041-043 and O041-044, ADR-0097 decision 3b amended, ADR-0095
  decisions 2 to 5 restated (decision 4 holds again with a Note), new task T041-37, T041-31 withdrawn, the carry tests
  moved to T041-36. The re-run tier of ADR-0095 decision 5 (a guard's raise re-runs with a mark-only carry, reason
  `failed`) is kept: removing it (finding Z-3) would change an accepted ADR's decision.
- Second round (short re-reads by the seven reviewers at `4d6fe4cc`; reports not saved, answered in resolution.md
  "Second round"): `a9265cd5`, decisions D041-134 (replica-wide wait slots Q = floor(N/4), tenant slots
  P = ceil(Q/2), through the per-connection setting `sagat.step_log_wait_slots`; the top-up tries the usage row
  `NOWAIT` only and never reclaims) and D041-135 (SC-041-007 at 1 x 16 on the median of all valid rounds; gate (4) at
  the measured cap and tickets 10; the eight valid rounds' table in validation-report.md).
- Develop merged (`5a86aaa1`, develop `566ca492`); `docs/runbooks/README.md` indexes step-logs.md (`c52b5d37`;
  `python3 scripts/check-docs-index.py` now runs before every push).

**Third round at `c52b5d37` (architecture, security, correctness, egm-design for stages 5 to 8, egm readiness;
reports not saved): still open, to fix before the final re-read:**
- **Blocker (correctness B-1, design Major 1): the "median of all valid rounds" rule was written for both sizes.** At
  8 x 128 the eight valid rounds' (2b) median is (-2.19 + 6.30) / 2 = +2.06 ms > 0, so the accepted build fails;
  4 of 8 valid rounds are above 0, so even the fourth run's own statistic (median of the first three valid rounds) is
  close to a coin flip for this build. Needs the coordinating session (and possibly the owner, OD-2026-11-20-1 owns the
  allowance): keep the fourth run's statistic at 8 x 128 with its false-stop rate stated, or a floor derived as at
  1 x 16.
- **Connection hand-off (architecture N-2, correctness M-1, readiness M-1 and M-2):** sqlx 0.9's
  `PoolConnection::close_on_drop()` cannot be cleared; use a wrapper with an `open` flag whose `Drop` calls
  `close_on_drop()` only while a statement or `COMMIT`/`ROLLBACK` state is unknown; a clean drop rolls back and returns
  the connection (no churn on contended claims and refused reports, test: same `pg_backend_pid()` across them); it
  applies to every `PgRunTx` command (`open_run`, admission); list the files `begin_in_transaction`
  (`primitives.rs`, `chain_progress.rs`), `breaker.rs`, `declared_operations.rs`, `execution/scoped_events.rs`,
  `execution/database.rs`.
- **Q behind a transaction pooler (architecture N-3, security NF-1, design Major 3, readiness M-3, correctness
  M-4):** a custom `sagat.*` setting does not reach PgBouncer's server backends; pass Q per transaction
  (`set_config('sagat.step_log_wait_slots', $q, true)` in the `BEGIN` the store sends) or state Q = 1 there; the
  floor is per client pool, so add a server-pool sizing rule (server pool >= sum over replicas of floor(N/2) + Q plus
  a reserve); run the three-tenant regime behind the pooler; record `SHOW POOLS` waits.
- **Benchmark replicas share one `application_name` (readiness M-4):** `tests/support/engine.rs:756-765`; name each
  replica and assert distinct names in T041-37's setup.
- **The replica cap couples tenants through loss (correctness M-3):** state the channel in 2b and the risk row; give B
  slot-needing carries in the three-tenant regime with B's busy <= 0.1%; exclude A1 to A3 from the counted rule.
- **Top-up answers (correctness M-2):** test `limit` (share 0) before `room`; define or remove `held`.
- **False-stop rate (correctness N-2 remainder):** require at least seven valid rounds (3.85% at m = 8; 5.49% at
  m = 6); state the per-release rate (union bound 11.5% at m = 8); say what happens with too few valid rounds; define
  the median of an even count.
- **Stale text after D041-134 (design Major 2 and Nits):** section 2's L4 and the Overview still say the top-up waits
  with a tenant slot; section 2a's table (top-up "reclaims"); data-model.md §3b line 247; `docs/adr/README.md:39`;
  ADR-0097 header note should name D041-134; "default pool of 20" is the chart's (the binary's is 10: Q = 2, P = 1,
  reserve 3); the newest revision note should list the second round.

**Then:** the final short re-read at the final commit by the three repository reviewers and the egm stage agents; save
each report verbatim in `2026-11-20-approval/` with the header lines `check-design-docs.py` reads (`Doc:`,
`Reviewed-commit:`, `Reviewer:`, `Agent-blob:` = blob of `.claude/agents/<lens>-reviewer.md` at that commit, or
`EGM-hash:` for the egm summary, one `Counts:` line, no absolute path); write `egm-review-final.md` (Reviewer:
egm-review) whose Counts are the open findings after the re-read; one acceptance row per finding still open and
recommended "accept" (expected: Z-3's kept tier and D-12), citing OD-2026-11-20-17 and anchored `<report path>#<n>`;
the approval block; tick T041-35 with evidence (run 44834024933, OD-2026-11-20-13). The build (step 2) has not
started.

## Update after the approval (2026-11-21)

**Done:**
- `e8b70cd2`: the part of task T041-4 that makes the contract and the domain agree (`CeilingReason` without
  `TenantQuotaBusy`, `NotStoredReason::ServiceBusy`, the fitness test of both enums), which CI had flagged.
- `54341713`: the third round answered, documents first: the 8 × 128 gate (2b) floor (OD-2026-11-21-1, decision
  D041-136), decision D041-137 (the owned connection, *Q* per transaction, the pooler's sizing rule, the top-up's
  answers, per-replica pool names), the owner rows OD-2026-11-21-1 to -3, the second- and third-round reports saved
  in `2026-11-20-approval/round-2/` and `round-3/`.
- The final read at `54341713` (`2026-11-20-approval/final/`): no Blocker, six Majors accepted under the cap
  (OD-2026-11-21-200 to -205), Z-2 with them; the Nits are listed in `resolution.md` "Final read" with the task that
  settles each. The design is Approved (Reviewed-commit `54341713`); T041-35 and T041-33 are ticked.

**For the owner, through the coordinating session:** the release-level false-stop estimate (up to about 34% at eight
valid rounds, about 15 points of it from gates (1) and (3) at 8 × 128, which keep the fixed 90%); the p99 report
threshold (OD-2026-11-21-200); heavy multiplexing behind a transaction pooler (OD-2026-11-21-201); ADR-0097's
delegate amendments D041-134 and D041-137, accepted with this approval.

**Next:** the build: merge the held tasks T041-3 and T041-4, then the tasks in the design's order through T041-36 and
T041-37. The body of the approved design may now change only by ticks, evidence and follow-up tasks.

## Update after the owner's answers and task T041-4 (2026-11-21)

**Done:**
- `19cc381d`: the owner's answers of 2026-11-21, documents first: rows OD-2026-11-21-8 ("Derive them too"), -9
  ("Confirm the amendments"), -10 ("Confirm"); O041-048 to O041-050; decision D041-138 (gates (1) and (3) at 8 × 128
  at least 88.6% and 83.3%; the release's union bound at most 26.9%; the owner report on the p99 derived again on its
  own statistic, +73.8% at 1 × 16 and +29.9% at 8 × 128, from the fourth run's `verdict.txt`); D041-139 (the protocol
  order: #260, then 1.7 here, then #264's 1.8); follow-up task T041-38 carries the amended bars, since the approved
  body of SC-041-007 cannot change (`check-design-docs.py` Rule 3); ADR-0097 cites OD-2026-11-21-9; the runbook and
  the validation report restate the bars. The derivation script and its input are in the scratchpad
  (`041-floors.py`, `041-verdict-44834024933/`), not in the repository; T041-38 asks the judge of T041-9 to encode the
  bars with a test that derives them.
- `c1fc1206` and its review fixes: task T041-4 ticked (the carry's plan in the domain, `DropReason`,
  `MarkOnlyReason`, `validate_carry`, the withheld usage of D041-133); the review of the slice in
  `2026-11-21-domain-and-quota-view/` with D041-140.

**Next (not started):** T041-5 (the quota-lease migration, the §3b functions, `PoolConfig.application_name`, *Q* from
the pool, the per-replica names, the isolation tests), then T041-6, T041-32, T041-10 in the design's order. Hold
T041-3, T041-36, T041-7 and every task that touches `crates/api-grpc/src/service.rs`, `crates/application/src/worker.rs`
or the SDKs' worker hosts until the coordinator says #260 merged; then merge origin/develop (never rebase).
For T041-6: raise `step_log_lines_dropped_total{reason}` by the saturated change of the stream's counts (D041-140 (7)).
