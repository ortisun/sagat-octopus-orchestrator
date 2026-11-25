# Re-reads of commit 78139ffb (2026-11-20)

Nine reviewers re-read the redesign at `78139ffb`: the three repository reviewers and six EGM lenses. Each treated `resolution.md` as the author's claim and checked the text and the code itself. This file holds each reviewer's open findings and closures as the coordinator relayed them. Line numbers refer to `docs/designs/041-step-logs.md` at `78139ffb` unless another file is named.

| Reviewer | Blocker | Major | Nit |
|---|---|---|---|
| security | 0 | 0 | 0 |
| correctness | 0 | 3 | 0 |
| architecture | 0 | 4 | 0 |
| EGM stage 7, design critic | 0 | 4 | 0 |
| EGM stage 7, structural | 0 | 3 | 5 |
| EGM stage 7, style | 0 | 5 | 0 |
| EGM stage 9, implementation readiness | 0 | 2 | 1 |
| EGM stage 9, codebase details | 0 | 2 | 0 |
| EGM stage 9, style | 0 | 5 | 1 |

## Security: closed

S-M1 and S-M2 are closed, and no new Blocker or Major was found.

Three items stay below Major:
- C0 checks the token's role and not the membership, so a membership demoted to runner after the token was issued still passes.
- Workload (h) is recorded with no threshold, and only at 1 × 16.
- G041-022: the database's round trip is not measured.

## Correctness

C-M1 to C-M6 are closed. The six equalities match the test's measure (`work_budget.rs:37-43`), with rows counted only for statements that begin with INSERT or UPDATE.

- **New 1: a scoped report whose output holds a declared secret still stores its carried lines.** `apply_path_report`'s `leaked` branch (`scoped_reports.rs:274-287`) goes to `finish_path_report` (`:414`), which the table at 899-905 gives a full carry. ADR-0095 decision 5 and line 903 say no line of such an attempt is stored.
  - Fix: pass `MarkOnly` when `leaked` is set, add the row to the table, and add a scoped secret case to T041-36.
- **New 2: T041-36's quota-crossing test cannot reach the quota re-run.** Its barrier is a BEFORE INSERT trigger waiting on an advisory lock (2091-2093). That trigger runs inside CD, after CA has set `lock_timeout = 200ms`, so any hold over 200 ms aborts CD with 55P03 and the report takes the `failed` path.
  - Fix: pause between CB and CD in application code, through a test wrapper of `StepLogCarryTx`, or hold both reports before CA.
- **New 3: a mark-only carry in a second transaction can itself fail, and nothing catches it.** This covers the content-quota fallbacks (`fail_success_for_storage_quota`, `fail_report_for_storage_quota`, `refuse_scoped_content`) and the step-log quota re-run. Each runs CA and CC under the 200 ms timeout, but "a re-run is never re-run" and `StepLogCarry` escapes as `AppError::Unavailable`.
  - That breaks FR-041-005 beyond G041-021, its one stated residual.
  - Fix: when a carry statement fails in a re-run or a fallback, run that transaction once more with `None`, and add a T041-36 case of a content fallback while an ingest is held after T7.

## Architecture

A-B1, A-M1, A-M2, A-M6, A-M7, A-M8 and A-M9 are closed.

- **A-M3, still open: the cost bound is checked only for a root completion.** ADR-0095 decision 2 is a MUST for every carrying report, but the equalities are measured on root completions only (976-979; T041-36 2078-2080).
  - Fix: one test per call site (`FailStep`, `finish_path_report`, the mark-only fallbacks) that asserts the cost with a carry minus the cost without equals the table's row.
- **A-M4, still open and narrower: places that record a leaked secret still get a full carry.**
  - the root `fail` with a secret in its failure code (`worker.rs:2415`, reaching `finish` at `:2436`);
  - the scoped report path, for a secret in the output and in the failure code (`scoped_reports.rs:274-287`, reaching `finish_path_report` at `:414`).

  Fix: add these rows as mark-only and test each one.
- **A-M5, still open and narrower: the re-run has more refusals than lease expiry.** The re-run goes back through `attempt_authority_fence` and `ensure_bound` (`worker.rs:2820-2838`), which also refuse when, between the two transactions, a cancel was requested, the run started stopping (for a forward step), the run finished, or the binding or membership was revoked. ADR-0095 decision 4 still says, with no exception, that a carry "MUST NOT change a report's outcome".
  - Fix: list every such refusal in G041-021 and FR-041-005, and put the exception in decision 4.
- **A-NM1, new: T041-35 depends on code that cannot be merged before it.** The hook plans with T041-4's `plan_batch` (1759-1760), but D041-091 keeps T041-4 unmerged until T041-35 passes, and T041-4 must still remove `final_batch` and add `in_order` (1853-1855).
  - Fix: have the hook plan its two cases inline, as T041-31's spike did, or revise T041-4 first and stack T041-35 on it.

## EGM stage 7, design critic

K-1, K-2 and K-6 to K-9 are closed. K-1's workload (h) is recorded with no threshold.

- **K-3, still open: round trips to a remote database.** It is stated as G041-022, and the carry now has five statements. The reason given for not combining them (writing statements never begin with WITH, so the budget test can count their rows) is a test-harness convenience that adds hot-path cost. Neither T041-35 nor T041-9 measures a remote database.
- **K-4, still open: collections and loops are not in the gate** (G041-019). The path where many reports contend for one run row is first measured after the schema is built, in T041-9.
- **K-5, still open: the purge's share of the tenant usage row.** T041-35 runs no purge (line 1793), and the only purge risk row is "Purge falls behind ingest" (line 1481).
- **N-1, new: a mark-only carry still waits on step-log locks, so a re-run or a content fallback can fail the report.** This is the same defect as correctness New 3.
  - There is a second-order effect: while a step-log quota change waits on its lock, every carrying report of that tenant waits 200 ms, fails and repeats its whole transaction.
  - Fix: do not abort on contention. Take the policy locks with `pg_try_advisory_xact_lock_shared`, lock an existing stream row with `FOR UPDATE SKIP LOCKED` after `INSERT ... ON CONFLICT DO NOTHING`, and when a lock is not available skip the carry in the same transaction and count it as `contended`. At the least, re-runs and content fallbacks should downgrade to no carry, with G041-021 and lines 960-962 corrected.

## EGM stage 7, structural

ST-1 to ST-6, ST-8, ST-9 and ST-11 are closed.

- **ST-7, still open in the linked files: expiry is still defined by "the last recorded batch".** This is in `data-model.md` lines 63-65 (`first_at`, `last_at`, `expires_at`) and in `contracts.md` line 236 (`expiresAt`). The `contracts.md` text is what T041-3 copies into openapi, so the public contract describes a short step's expiry wrongly.
- **ST-10, still open and narrower.** Constitution V (1494) still says "Pass with conditions, each accepted by the owner", lists only G041-001, -008 and -017, and ends "so G041-015 is closed". But G041-018 is an open gap on the same principle and quota. Risks row 1483 also says "G041-015 closed".
  - Fix: add G041-018 to V as an open condition not yet accepted, and qualify row 1483.
- **N-1, new: contracts.md §4 (336-337) contradicts the report-path table (899-905) and ADR-0095 decision 5 (74-75).** contracts.md says a refused output loses its carry and the stream shows "ended without its final flush". The design says a refused output writes a mark-only carry, so the stream is `ended`.
  - Fix: limit "lost" to a stale-lease refusal.
- **Nits:**
  - Risks rows 1451 and 1462 use the old definition of a short step.
  - O041-021 sits in the 2026-11-16 group (475).
  - "five methods" should be six (175, 917, data-model 325).
  - Risks row 1457's mixed-rollout claim holds only for mixed binaries.
  - SC-041-003 states the G041-014 case twice (319-323).

## EGM stage 7, style

SY-1 and SY-3 to SY-19 are closed, except as listed below.

- **SY-2, still open.** Risks lines 1451 and 1462 use the old conditions. Rewrite them as "A short step (see Terms) adds none" and "Only a long step waits".
- **SY-11, still open.** Lines 312 and 1450 still say "the time bound of SC-041-007". ADR-0095 line 23 says "the time bound of decision 2", but decision 2 states only statement counts. Match line 841: "The effect of these waits on time is what SC-041-007 bounds (completion p99 within 10% of its value with capture off)."
- **SY-12, partly open.** The trigger of O041-026 is stated three ways, at 405, 410 and 1453. Use the wording of 1570-1571 in all three places.
- **N-1, new: "each bounded at 200 ms" reads as covering the commit wait** (Overview 24-26 and line 701). The usage-row wait at commit has no 200 ms bound, because the timeout is restored before the commit.
  - Fix: "...the tenant's usage row at the commit, which SC-041-007 covers, and, for a carrying report, the step-log policy fences and its own attempt's stream row, each of these two waits bounded at 200 ms". At 701: "its lock waits before the commit are bounded at 200 ms each".
- **N-2, new: the port is said to have five methods, but section 2a defines six statements** (CA, CB, CR, CC, CD, CE). This appears at FR-041-008 (175), 917 and ADR-0095 decision 3.
  - Fix: "six methods, one statement each; at most five run for one report", and give CR its own bullet.

## EGM stage 9, implementation readiness

IR-1 to IR-5 and IR-7 to IR-11 are closed.

- **IR-6, still open: two test mechanisms do not work as written.**
  - The order guard (2100-2102) has no barrier that holds the report after its transaction id and before CC while a later ingest commits.
  - The tenant-limit barrier (2091-2092) waits inside CD, under the 200 ms lock timeout. This is the same defect as correctness New 2.
  - The test of a change to 0 (2084-2086) accepts either of two outcomes.
  - Fix: name barriers outside the carry's lock timeout, for example pause points in a test wrapper of `StepLogCarryTx` before CA and before CC, and give each test one expected outcome.
- **N-1, new: a carry that fails inside a re-run turns the report into an error.** This is the same defect as correctness New 3. Fix: retry that transaction once with `CarryPlan::None`, count it `failed`, and test that the outcome is unchanged. Otherwise state it as a gap and correct lines 960-962.
- **Nit:** line 1866 says the down migration restores three constraints (there are now five), and line 1880 does not list `<timestamp>_step_log_quotas_validate`.

## EGM stage 9, codebase details

CD-1 to CD-8 are closed. The six equalities are consistent with `work_budget.rs:37-43`.

- **N1, new: the leaked-secret rule applies only to root completions.** A scoped completion with a leaked secret reaches `finish_path_report` (`scoped_reports.rs:274-287`, `:414`), and a root `FailStep` whose failure code held a secret (`worker.rs:2412-2418`) reaches `finish`. Both get a full carry. This is the same defect as correctness New 1 and A-M4.
- **N2, new: `StoreError::StepLogCarry` cannot be told apart where the re-run decides.** `finish` and `finish_path_report` return `AppError` and convert with `?` (`worker.rs:2889-2890`, `scoped_reports.rs:442-443`). The wrapper therefore sees `AppError::Unavailable`, which is the same value as an ordinary refusal: secret versions changed (`scoped_reports.rs:247-249`), the refusal paths (`scoped_output_refusal.rs:54-56, 219-221`) and a database outage.
  - Either a carry failure is not re-run, or unrelated refusals are re-run with no carry.
  - Fix: carry the distinction across the boundary, with an internal `AppError` variant that the API edge maps to `Unavailable`, or with a typed result from `finish` and `finish_path_report`.
- **Minor:** `apps/execution-engine/src/settings.rs:462-487` builds the full `WorkerSettings` literal and needs the new field, but it is not in T041-36's Files. Some line references in T041-3 are stale.

## EGM stage 9, style

SY9-1 to SY9-11, SY9-13, SY9-15 and SY9-16 are closed.

- **SY9-14, still open.** Line 2415 says "35 tasks ... and 34 pull requests". With T041-34 also withdrawn, 36 task numbers give 34 tasks. Line 1617 still says "each task is one pull request".
- **SY9-17, still open.** Line 1809 adds `StepLogCarry` to "the three `skip_debug` lists", but line 1819 says "add the four messages and the `oneof` of section 1". It is unclear whether `StepLogCarry` is one of the four, and which "section 1" is meant.
- **SY9-18, still open.** T041-5 names its accessor nowhere (lines 1891, 1893, 1901).
- **N-1, new: one ticket budget is tied to three pools.** T041-31 uses half of `SAGAT_GRPC_MAX_LOG_CALLS`. T041-35 (g) uses "half the pool of 20 of `API_POOL`". The Then step uses a share of `SAGAT_DB_MAX_CONNECTIONS`. T041-7 says "the defaults T041-35 set" without naming the key.
  - Fix: name one pool, and say how it relates to the gRPC limit.
- **N-2, new: T041-35 depends on T041-4's revised code without saying so.** This is the same finding as architecture A-NM1.
- **Nit:** SY9-12, "the canary" is used before it is defined (2045, 2132).
