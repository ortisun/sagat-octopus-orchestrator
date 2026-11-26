Re-read by the `egm-design` reviewer of the fixes at `3f1f0aba` (answering its review of `4d74b27a`). Saved verbatim.

# Re-read of design 041 at 3f1f0aba: are the critique's findings closed?

**Reviewer:** egm-design (Design Critic Goldfish), mode REVIEW, read-only.

**Scope:** my review of 4d74b27a (`egm-design.md`, 2 Blocker, 7 Major), checked against:
- `docs/designs/041-step-logs.reviews/2026-11-20-hot-row/resolution.md`
- D041-112
- the diffs `fix-docs.diff` and `fix-code.diff`
- the files at 3f1f0aba: the design, ADR-0095 and ADR-0097, `data-model.md` §3b and §8a, `validation-report.md` (the hand-mutation record), `step_log_spike.rs`, `step_log_leases_tests.rs`

**Verdict:** both Blockers and all seven Majors from my review are closed. Two answers carry limits that are accepted and stated (M2's NOWAIT trade-off and M4's partial attribution). The fix for the old M2 (the carry's `BEGIN … EXCEPTION` block, with its refill taking the usage row `NOWAIT`) introduced two new Major findings, three Minor and one Nit.

## Status of the earlier Blocker and Major findings

| Finding | Status | Evidence |
|---|---|---|
| **B1.** A lowered limit strands room, and the tenant is refused for good | **Closed** | ADR-0097:96-100 (decision 6): a change of the limit or the ceiling reclaims every lease under the fence it holds exclusively. ADR-0097 decision 3 and data-model.md:241: a charge that finds S + G > L goes to the refill, and the refill reclaims. Test `a_limit_lowered_above_the_usage_still_admits_what_fits`; mutations R1 and R6 (validation-report.md:582, 587). The mismatch between T6's allowance and refusals of held room is now documented (design:831-835). The robustness of the reclaim is a new Minor (N4). |
| **B2.** The cut is not shown to recover the gates, and the next step is not said | **Closed** (as a design obligation) | design:2066-2079 records, before any round, the expected effect of each change on each gate, taken from the breakdown: (1) and (3) at most about 2.5 points; (2b) at most 3.6 ms, with the lines' 12.2 ms left untouched; (4) about 2.0 ms of S p99. It says 1 × 16 is "not expected to pass", and what follows a third failure (escalation, then chunk rows or fewer line indexes as a redesign). Residual, not a finding: the coordinator is running a gate it predicts will fail. The expected-effect list now also includes an unmeasured negative, the subtransaction (see N2). |
| **M1.** The release waits for and holds every lease | **Closed** | ADR-0097 decision 5 and data-model.md:257-260: the release takes no lease, reads Σ `used` after locking the usage row, refuses an underflow of U, and lets S go negative. The release's own fold is gone (option 9, ADR-0097). Test `a_release_never_waits_for_a_lease_and_never_goes_below_zero`; mutations R2 and R3. Purge transactions are now one tenant each (design:1258). |
| **M2.** At the limit the leases fall back to one usage-row lock per charge | **Closed** for waits under the run lock | design:1169-1173 states the regime; a carry's refill is `NOWAIT` (data-model.md:243); T041-9 measures the regime with a purge running. The trade-off (lost carries in place of waits) is N3. |
| **M3.** The cost evidence is misquoted | **Closed** | ADR-0097, Evidence: the paired figures are quoted exactly (0.988, p99 +3.4 ms), the slots are labelled a proxy, and the 8 × 128 lock timeouts are cited with their caveat. |
| **M4.** No attribution between the charge and the storage | **Closed as accepted** | design:2070-2072 reports the (g)/`g-slot` S p99 proxy (28.0 against 26.1 ms) and says "full attribution is not claimed". The third run now changes three things (leases, storage, subtransaction), so a failure will be harder to read. |
| **M5.** Readers of the usage see S only | **Closed** | `sagat_quota_step_log_usage` (data-model.md:254-256), and T041-5's file list includes `quotas.rs` and `guards.rs` (design, T041-5). |
| **M6.** No telemetry, and refusals of held room look like the limit | **Closed** | ADR-0097 decision 8; design §9 adds the alert `SagatStepLogQuotaHeldRefusals`; refusals of held room have a constraint name of their own (`tenant_step_log_quota_held`). Readers still see `tenant_quota`, which is stated (design:1150-1151). |
| **M7.** The lease table bloats | **Closed** | data-model.md:220 sets fill factor 50 and per-table autovacuum settings; the risk row now names the lease table, and (d) records its size. |

Minor findings from the first review that were taken: m2 (the probe order; data-model.md:239 is correct), m3 (stated in the ADR-0097 Consequences), m4 and m5 (costs stated, measured in T041-9 and T041-10), m6 (run index kept), m7, m8, m9, and most of m10. m1 (the constants) is only partly answered: the regime boundary and the bound per grant are stated, but 256 is still not derived from the connection budget. These are not re-graded.

---

## New findings

### Major

#### N1. The carry function's binding spec does not describe the new exception block. The prototype's handler loses a long step's final mark and catches too broad an error class.

**Evidence**
- data-model.md:502-506, §8a, which is the body spec T041-36 builds from, still says: "Any raise of the step-log charge in CD … is mapped to `StoreError::StepLogRefused`, which the application answers with a re-run of the report". §8a items 1-7 (data-model.md:463-500) never mention the `BEGIN … EXCEPTION` block.
- design:1045-1049 and ADR-0095 decision 5 say the block "writes the stream's final mark as a mark-only carry … and returns `mark_only`".
- step_log_spike.rs, the handler in `install_third_run_storage` (fix-code.diff:651-665):
  - `WHEN program_limit_exceeded THEN INSERT INTO spike_041.step_log_streams … ON CONFLICT (…) DO NOTHING; RETURN 'mark_only';`
  - `WHEN lock_not_available THEN RETURN 'contended'`.
- The only refusal test, `a_carry_refused_by_the_limit_records_its_mark_in_the_same_call` (step_log_leases_tests.rs:799-810), uses a new stream.

**Why**
1. The spec an implementer follows and the decision contradict each other: one says re-run, the other says caught in the same call.
2. For an existing stream (a long step), rolling back the subtransaction also releases the stream-row lock that CC′ took. The handler's `INSERT … ON CONFLICT DO NOTHING` then writes nothing, yet returns `mark_only`. The final mark and the `tenant_quota` counts are lost, the outcome metric is wrong, and the stream shows "ended without its final flush". That contradicts design:1045-1049.
3. `program_limit_exceeded` is SQLSTATE 54000, which is also raised by unrelated limits (for example a btree index row that is too large). Those would be recorded as quota refusals. The design itself says each refusal has "a constraint name of its own".

**Fix**
- Rewrite §8a:
  - put CC to CE inside the block;
  - catch only `tenant_step_log_quota_exceeded` and `tenant_step_log_quota_held` (by constraint name or a dedicated SQLSTATE) and the refill's `lock_not_available`, and re-raise anything else;
  - for an existing stream, re-take its row `SKIP LOCKED` inside the handler, then apply the mark-only plan (or answer `contended` when the row cannot be had).
- Remove data-model.md:502-506.
- Add an existing-stream refusal case to T041-36 and to the prototype's test.

#### N2. Every carrying report now opens a subtransaction. Its known PostgreSQL costs are unmeasured, and an alternative without one exists.

**Evidence**
- design:1008-1013: "CC to CE run in one `BEGIN … EXCEPTION` block of the function (one subtransaction …)".
- ADR-0095 option 6 (revised): "its cost is measured by T041-35's third run".
- design:2071: "the subtransaction costs an unmeasured amount".
- The (d) workload holds a transaction open for the whole measurement (design:1959-1960).

**Why**
- The block writes rows, so each carrying report gets an extra subtransaction id. Transaction-id consumption and freeze pressure grow with the completion rate.
- Rows written by subtransactions bring in `pg_subtrans` lookups. These are known to degrade under long-running transactions, and on hot standbys especially (`SubtransSLRU` waits). The design's own risk table expects long transactions (risk row "A long or prepared transaction holds the read horizon").
- This is on the hot path of every carrying completion. It was added to answer a refusal path that, apart from the limit, is rare.
- The gates are already predicted to fail at 1 × 16 (B2), and this adds an unquantified negative.

**Fix**
- At minimum, have the third run's (d) and its 8 × 128 rows record `SubtransSLRU`/`SubtransBuffer` waits and transaction-id consumption per completion. State the PostgreSQL version and its `subtransaction_buffers`.
- Preferably, remove the subtransaction:
  - before CC, the carry calls a `SECURITY DEFINER` try-charge with the planned sizes (C3 already computes `p_sizes`). It returns `ok`, `limit`, `held` or `contended` instead of raising, and records the prepaid bytes in a transaction-local setting;
  - CD's statement trigger then charges only the measured bytes minus the prepaid bytes, and raises only on a mismatch, which can only be a defect;
  - the database still measures the stored rows (ADR-0027), and the refusal paths need no exception block.

### Minor

#### N3. A held usage row on a refill makes the carry write nothing at all, not even its final mark.

**Evidence**
- design:1047-1049: "a refill that finds the usage row held raises `lock_not_available` (`NOWAIT`), caught the same way, and the function returns `contended` with nothing written".
- validation-report.md:500: with completions that store output, the row is busy 24.0% of the time.
- design:1169-1173: at the limit there is about one refill per charge.

**Why**
- Away from the limit, a carry needs a refill about once in 300, and about a quarter of those find the row held. That is a small loss of records that would have fit.
- At the limit, where nearly every carry refills, a large share of carries are lost as `contended`. They leave no stream, no drop counts and no `tenant_quota` reason: only G041-014's "no stream", which is a weaker diagnostic than the mark-only stream the design gives a refused carry.

**Fix:** on `lock_not_available`, write the mark-only stream, which charges nothing. Give it a reason of its own, or `tenant_quota`, and count it. Quantify the expected `contended` rate in T041-9.

#### N4. The reclaim on a limit change is correct only if its caller holds the fence, and the trigger neither takes nor checks it.

**Evidence**
- step_log_spike.rs, `spike_041.step_log_reclaim` (fix-code.diff:495-514): it is a row trigger on any `INSERT`/`UPDATE`/`DELETE` of `quota_limits` or `quota_ceilings`. It reads `sum(used)` before it updates the leases. Its comment says "the editor holds the step-log fence exclusively".
- data-model.md:251-253.

**Why:** suppose some writer of those tables does not hold the fence (a backfill, a test fixture, a path added later). The reclaim's `UPDATE` of a lease waits for an open charge on that lease. Re-evaluated after that charge commits, it sets `used` to 0 after S was already computed from the old Σ `used`. Those committed bytes then disappear from U: usage is under-counted, and an overshoot becomes possible.

**Fix:** have the reclaim take the step-log fence exclusively itself (idempotent when the editor already holds it), lock the tenant's leases `FOR UPDATE` before reading Σ `used`, and test a reclaim fired without the editor.

#### N5. Stale text left after the fix.

**Evidence**
- design:2734 (Verification Plan): "A lease charge's raise at the tenant's step-log limit and a failing carry statement each re-run the report". A quota refusal is now answered in the same call, with no re-run.
- design:1662 (Risks): "The tenant row only at commit".
- The Constitution Check row VI lists the old alerts without `SagatStepLogQuotaHeldRefusals`.

**Fix:** update the three rows.

### Nit

#### N6. ADR-0097's probe-order expression is missing the `+ 256` that the spec has.
ADR-0097:79-80 gives the order as "`(lease − pg_backend_pid() mod 256) mod 256`". PostgreSQL's `%` keeps the dividend's sign, so if this is implemented literally the key is just `lease − p`, and the order collapses back to lease order. data-model.md:239 and the prototype have the correct `(lease - pg_backend_pid() % 256 + 256) % 256`. Copy that form into the ADR.

---

**Count of new findings:** Blocker 0, Major 2, Minor 3, Nit 1.
**Earlier findings:** B1, B2 and M1 to M7 closed (M2 and M4 closed with stated limits).
