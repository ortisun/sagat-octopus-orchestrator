Re-read by the `correctness` reviewer of the fixes at `3f1f0aba` (answering its review of `4d74b27a`). Saved verbatim.

# Re-read of design 041's hot-row revision at 3f1f0aba (correctness)

The original Blocker and all five Majors are closed. The fixes introduce one new Major: in the steady state of a tenant that logs near its limit, a carrying report's refill answers `contended` and loses the attempt's last lines without counting them. The new Minors and Nits are below.

I read `resolution.md`, D041-112, `fix-docs.diff` and `fix-code.diff`, and the current files in `/Users/marcelo/Documents/Projects/sagat-wt-041`. I ran nothing. Line numbers refer to the files at 3f1f0aba.

## Status of the earlier Blocker and Major findings

| Finding | Status | Evidence |
|---|---|---|
| **B1** A limit lowered below S + G (but above the usage) refuses every charge for good | **Closed** | A charge that finds S + G > L now goes to the refill (data-model `:238-241`; prototype `step_log_spike.rs:366`). A change of the limit or its ceiling reclaims every lease under its exclusive fence (ADR-0097 `:94-97`; data-model `:251-253`; prototype `:405`). The exclusive fence does guarantee that no lease is held: every charger, refill and release keeps the step-log fence shared until its transaction ends. `put_ceiling` takes the fence for byte resources (`crates/infra-postgres/src/quotas.rs:912-913`). Test: `step_log_leases_tests.rs:678`. A gap left by the fix is N4. |
| **M1** The release folds leases "under the usage row" (deadlock risk) | **Closed** | The release takes no lease. Under L4 it reads Σ `used`, refuses a U underflow, and sets S −= released (ADR-0097 `:90-93`; data-model `:257-260`). This is exact: under L4, Σ `used` can only rise through uncommitted fast-path charges, so the check is conservative. Test: `a_release_never_waits_for_a_lease_and_never_goes_below_zero`, which also exercises a negative S. |
| **M2** Lease contention aborts the report; a refill waits under the run lock; the savepoint rejection is stale | **Closed as specified** | CC to CE now run in one `BEGIN … EXCEPTION` block, and the refill takes L4 `NOWAIT` (design `:1006-1011`, `:1044-1048`; ADR-0095 `:81`, `:98-100`, `:118-119`). Tests at `:761` and `:799`. See N1: the fix moves the cost elsewhere. |
| **M3** A release waits for every held lease (purge latency, hot row comes back) | **Closed** | Same change as M1. No transaction waits for a lease (design `:863-864`, `:868`). |
| **M4** T041-6 and the verification row still hold ingests "after the quota flush" | **Closed** | New hooks at design `:2258` ("hold after T8 … hold inside a refill"); SC-041-001 `:313`; T041-6 `:2261`; verification row `:2735`. One leftover Risks row is listed under N5. |
| **M5** ADR-0095 still describes the charge at commit | **Closed** | Decisions 5 and 7, options 6 and 7, and the Consequences are revised (ADR-0095 `:81`, `:96-100`, `:118-123`, `:133-134`). |

## New findings introduced by the fixes

### N1 [Major] A tenant that logs near its limit loses carries uncounted, because the `NOWAIT` refill answers `contended` and records nothing

**Evidence.**
- Design `:1047-1048`: "a refill that finds the usage row held raises `lock_not_available` (`NOWAIT`) … returns `contended` with nothing written".
- Design `:1169-1172`: below 64 MiB free, "a refill comes about once per charge, each holding the usage row to the end of its commit … a carry's refill then answers `contended`".
- Prototype `step_log_spike.rs:558-559`: `WHEN lock_not_available THEN RETURN 'contended'`.

**Failing scenario.**
- A tenant whose log volume times retention exceeds L lives at its limit in steady state: the purge frees expired lines and writers fill the room back.
- In that state the free room is small, so the refill path is the common path, not a rare one.
- L4 is held:
  - by every report with output, through its content finalizer (measured busy 24% for off-1K, `validation-report.md`);
  - by every ingest or marker refill, until its commit;
  - by other carries' refills, until their reports commit.
- So a large share of carries become `contended`. Their lines are lost and counted nowhere. The stream shows "ended without its final flush", which blames the worker for what is quota pressure.
- Before this revision, such a carry waited at its commit and was either stored or counted `tenant_quota`. ADR-0095's Consequences still call contention "rare" (`:127`). That justification (the SDK's 2-second wait) does not cover this cause.

**Why the tests miss it.** `a_carry_never_waits_for_the_usage_row` (`:761`) asserts only `contended` with nothing stored. No test, and no T041-35 row, measures the contended rate near the limit.

**Fix.**
1. In the `lock_not_available` handler, record the stream's final mark as a mark-only carry that counts the records as dropped. Use `tenant_quota`, or a new reason `contended_quota`. This write charges nothing and needs no L4. For an existing stream, retake the stream row `SKIP LOCKED`: the rolled-back subtransaction released it.
2. Optionally, cut how often this happens: give the carry's refill a few-millisecond `lock_timeout` instead of `NOWAIT`, or top leases up ahead of need from the ingest and marker paths, which may wait.
3. Add a test: with the usage row held, a carry on a new stream and one on an existing stream each record the final mark with the drops counted. Have T041-35's third run, or T041-9, report the contended rate when free room is below 64 MiB.

**Confidence:** plausible. The mechanism is confirmed in the text and in the prototype; the rate is not measured.

### N2 [Minor] The down migration leaves S negative, so T041-4's down can never run afterwards

**Evidence.**
- data-model `:277-278`: "refuses while a row of `step_log_lines` exists; otherwise U = 0, and it drops the leases, the column". ADR-0097 `:152-153` says the same.
- S may now be negative (data-model `:210-211`). With U = 0, S = −Σ `used`.
- T041-4's down `crates/infra-postgres/migrations/20270201000000_step_log_quotas.down.sql:11` refuses while `step_log_bytes <> 0`.

**Failing scenario.** Lines are released while their bytes are still in a lease's `used`, so S = −5,000 and Σ `used` = 5,000. T041-5's down drops the leases and leaves S = −5,000. T041-4's down then refuses forever. A T041-4-level usage view would read a negative value.

**Fix.** Before dropping the leases, the down sets `step_log_bytes = step_log_bytes + Σ used`, which is 0 because U = 0. Test it with `undo_to` after a release that drove S below zero.

**Confidence:** confirmed in the text.

### N3 [Minor] The carry's refusal handler catches the whole SQLSTATE class 54; for a long step it reports `mark_only` without recording the mark

**Evidence.**
- Prototype `step_log_spike.rs:546` uses `WHEN program_limit_exceeded`. In PL/pgSQL, a class code such as 54000 matches every error in its class, so this also catches `statement_too_complex` (54001, which includes "stack depth limit exceeded") and 54011/54023.
- For an existing stream, the handler's `INSERT … ON CONFLICT … DO NOTHING` (`:556`) inserts nothing and still returns `mark_only`. Design `:1045-1046` says the handler "writes the stream's final mark".
- Test `:799` covers only a new stream.

**Fix.**
- In the built function, read `GET STACKED DIAGNOSTICS … CONSTRAINT_NAME`. Handle only `tenant_step_log_quota_exceeded` and `tenant_step_log_quota_held`, and re-raise anything else.
- For an existing stream, retake the row `SKIP LOCKED` and apply CE as mark-only.
- Add T041-36 cases: the refusal on an existing stream, and a non-quota error of class 54 reaching `StepLogCarry`.

**Confidence:** confirmed (prototype; the built function's text does not say how it matches).

### N4 [Minor] The S + G > L path of the charge (B1's fix, and I5) is no longer exercised by any test

**Evidence.**
- Since a change of the limit reclaims every lease, after any limit change S + G = U. So both tests reach S + G > L never:
  - `a_limit_lowered_above_the_usage_still_admits_what_fits` (`:678`);
  - `a_lowered_limit_stops_charges_into_leases_with_room` (`:229`).
- `validation-report.md:582` records that R1 failed the test only "before the reclaim on a limit change existed". With the reclaim in place, mutations M3 (`:575`) and R1 would now survive.
- The path is still reachable without a limit edit: when the 1 GiB default changes in a release, or when enforcement is turned back on.

**Fix.** Add a test that produces S + G > L without the reclaim (disable the reclaim, or edit the usage row directly). Assert that a charge that fits after reclaiming idle leases is admitted, and that one that does not fit is refused. Rerun M3 and R1 against it.

**Confidence:** confirmed.

### N5 [Minor] A held-room refusal of the script marker is counted `failed`; two stale lines remain

**Evidence.**
- data-model `:515-516`: "any error other than `tenant_step_log_quota_exceeded` is caught … and counted as `failed`". Design `:1556-1561` says the same.
- The new `tenant_step_log_quota_held` (data-model `:250`) therefore lands in `failed`, while the ingest maps it to `RETRY_LATER` (design `:835-836`) and the carry to `tenant_quota`.
- data-model `:522` still says "`COMMIT`, with `tenant_step_log_quota_exceeded` counted as `skipped_quota`".
- The Risks row at design `:1662` still says "The tenant row only at commit".

**Fix.**
- Count `_held` as `skipped_quota`, or as a `skipped_held` outcome.
- Edit data-model `:522` to "the `script_marker` insert (its charge may raise …)".
- Rewrite the Risks row `:1662` for the lease charge.

**Confidence:** confirmed.

### N6 [Minor] The reclaim on a limit change locks lease rows in the waiting form after L4, against the stated rule and the planned fitness check

**Evidence.**
- Design `:868-869`: "after L4 no writer takes a lock it waits for". Design `:882`: the lock-order test "checks that after L4 … no writer takes a lock on an existing row".
- The reclaim does exactly that: it locks L4, then runs `UPDATE … step_log_quota_leases SET granted = 0, used = 0 WHERE tenant_id` (prototype `:404-405`; data-model `:251-253`).
- This is safe only because the caller holds the step-log fence exclusively. A future editor path, or an SQL fix applied by hand without the fence, would wait for a held lease while holding L4. The lease holder (a carrying report with output) waits for L4 at its commit, so the two deadlock.

**Fix.**
- Take the leases `FOR UPDATE NOWAIT` in the reclaim, or assert inside it (via `pg_locks`) that the transaction holds the fence exclusively.
- State the exemption in section 2 and in the lock-order test.

**Confidence:** confirmed in the text; the deadlock is plausible only on a path that lacks the fence.

### N7 [Nit] With enforcement off, every charge past the limit takes L4

**Evidence.** ADR-0097 `:99-101` says the refill grants without the limit check when enforcement is off. But the charge still routes every charge with S + G > L to the refill (data-model `:238-241`), and so to L4. With enforcement off and usage past L, every charge serializes on the usage row again.

**Fix.** The charge reads the enforcement flag and skips the S + G ≤ L test when enforcement is off.

**Confidence:** confirmed in the text.

### N8 [Nit] The block assigns a subtransaction XID to every carrying report that writes

**Evidence.** The block at design `:1008-1011` makes every carrying report that writes assign a subxid, and its rows carry that subxid as `xmin`. That doubles XID consumption for those transactions and adds `pg_subtrans` lookups for readers while they run. Design `:2067` defers the cost to the third run.

**Fix.** Have T041-35's third run also report XID consumption per committed report (`txid_current()` deltas), beside the rate.

**Confidence:** confirmed (PostgreSQL behaviour).

Count: Blocker 0; Major 1 (new; all earlier Blocker and Majors closed); Minor 5; Nit 2.
