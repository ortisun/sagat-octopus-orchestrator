Re-read by the `architecture` reviewer of the fixes at `3f1f0aba` (answering its review of `4d74b27a`). Saved verbatim.

# Re-read (architecture): `3f1f0aba` against the review of `4d74b27a`

Paths are relative to the worktree root `/Users/marcelo/Documents/Projects/sagat-wt-041`.

## Verdict

- **Blocker B1:** closed.
- **Majors:** M1 and M3 to M9 are closed. M2 is still open.
- **New:** the fixes introduce one new Major. When the usage row's holder has already updated the row, which every real holder does, the carry's `NOWAIT` refill still waits under the run lock. The prototype test that should catch this holds the row in a way real holders never do.

## What I read

- `resolution.md` and D041-112.
- The current ADR-0097, ADR-0095, ADR-0084 (its new Note), the ADR-0027 Note and `docs/adr/README.md`.
- `docs/engineering/architecture-principles.md:31`.
- Design 041 §2, §2a, §2b, §3, §4, the open owner questions, T041-5, T041-6, T041-35, the Risks, the Constitution Check and Complexity Tracking.
- data-model §2, §3b, §4 and §7.
- The validation report's mutation record.
- The prototype as committed: `step_log_spike.rs` and `step_log_leases_tests.rs`.
- To check the fence that the reclaim relies on: `crates/infra-postgres/src/content_usage.rs:54-103`.

I skipped `docs/owner-decisions.md` and design 043, as asked. Nothing was built or run.

---

## Status of the earlier Blocker and Majors

| Finding | Status | Evidence |
|---|---|---|
| B1: ADR-0084 departed from without saying so | **Closed** | ADR-0097:14-18 names Revision decision 3's usage-row clause and the ingest's reading of decision 11. ADR-0084:251-258 adds a Note. ADR-0097:22 promises the "partly superseded" marker. The README line names ADR-0084. |
| M1: the release waits for every lease | **Closed** | ADR-0097:90-93: the release "MUST NOT wait for a lease" and reads Σ `used` under the usage row. Data-model `:257-260` and the prototype `step_log_release` match. Test `a_release_never_waits_for_a_lease_and_never_goes_below_zero` and mutations R2 and R3 (`validation-report.md:583-584`). Allowing S to be negative is sound: U = S + Σ `used` ≤ S + G holds whatever the sign of S. ADR-0097:13 states that ADR-0027's non-negative rule is superseded for this class. |
| M2: acceptance route and README rule 4 | **Still open** | See R1 below. |
| M3: ADR-0095 inconsistent; catching the refusal in the function | **Closed** | ADR-0095 decisions 5 (`:78-84`) and 7 (`:89-98`), options 6 and 7, and the Consequences are revised. Design `:1000-1006` and `:1041-1049` describe the `BEGIN … EXCEPTION` block, and `:993` is corrected. Its effectiveness depends on N1 below. |
| M4: evidence misattributed | **Closed** | ADR-0097:159-166 quotes the paired figures exactly (0.988, p99 +3.4 ms), calls the slots a proxy, cites the 8 × 128 lock timeouts with their caveat, and leaves the lease charge's own cost to the third run. |
| M5: usage readers do not know about leases | **Closed** | ADR-0097:98-99. data-model `:254-256` adds `sagat_quota_step_log_usage`. T041-5 now modifies `quotas.rs:1228-1235` and `guards.rs:98` (design `:2228-2234`). The prototype carry reads `step_log_usage`. |
| M6: a down migration that never runs | **Closed** | ADR-0097:152-153 and data-model `:277-278`: the down refuses while a line exists and otherwise drops the leases. |
| M7: no telemetry; held refusals look like the limit | **Closed in vocabulary, open in wiring** | ADR-0097:102-104, data-model `:266-268` and `:354-355`, and the alert `SagatStepLogQuotaHeldRefusals` exist. How the counters are emitted is not specified (N3). |
| M8: the lock-order fitness function would reject the order | **Closed** | Design §2: slots for L1, L1b, L2, L3 and L3b; tries are exempt and proved never to wait with `pg_stat_activity`; a case with a refill held. |
| M9: the stream run index dropped without evidence | **Closed** | data-model `:93`, `step_log_streams_run_idx … -- kept (D041-112)`. The prototype no longer drops it. |

### R1 (Major, M2 still open). The acceptance route is still stated two ways, and O041-040 was not restated

**Evidence**
- ADR-0097:4-7: the coordinating session "accepts it under the same delegation when design 041's re-review closes and T041-35's third run is recorded, together with O041-038 (ADR-0095) and the **restated** O041-040 (README rule 4: … OD-2026-11-20-6)".
- O041-040 has not been restated anywhere:
  - design `:429-435` still asks to accept a text "with one deferred flush queued by the transaction's delta row", and still says "(a) Accept the wording … Recommended (a)";
  - `specs/041-step-logs/research.md:202` is the same.
  - Accepting (a) would accept the deferred flush that ADR-0097:11-12 removes.
- The triggers disagree:
  - ADR-0097 says "when … the third run is recorded";
  - O041-038 and O041-040 say "asked … once T041-35 passes" (design `:429`, `:436`; research `:200`, `:202`);
  - ADR-0095:4-5 says "for the owner to accept (O041-038) once design 041's spike T041-35 passes";
  - design `:2097` and `:2377` say "the owner's acceptance of ADR-0095".
- If the third run fails, the documents do not say whether ADR-0097 is accepted on its own (its case does not depend on the 1 × 16 gate, D041-106) or waits for O041-038, which is never put.

**On the coordinating session's statement.** Inside its own header, ADR-0097 records consistently that the coordinating session accepts it under OD-2026-11-20-6, and it names README rule 4. That part is acceptable. What is still inconsistent is everything around it: the O041-040 wording, ADR-0095's header and the design's "owner's acceptance" lines. The only record of the rule-4 exception is a parenthesis inside the ADR. The rule-2 exception for ADR-0027's Amendment had an owner decision of its own (OD-2026-11-15-16).

**Fix**
1. Restate O041-040 in the design and in research: option (a) becomes "accept the first and third bullets, with the charge as ADR-0097 states it".
2. Choose one trigger and write it everywhere: either "re-review closed and third run recorded" or "third run passes". Say what happens to ADR-0097 if the third run fails.
3. Change ADR-0095's "Decided by" and the design's "the owner's acceptance" lines to the delegated route.
4. Add a one-line Note under README rule 4, or to the OD-2026-11-20-6 row, saying that accepting a Proposed ADR is a delegated act until 0.3.0.

---

## New findings introduced by the fixes

### N1 (Major). The carry's `NOWAIT` refill still waits for the usage row whenever the holder has updated it

**Evidence**
- In the prototype refill, `INSERT INTO public.tenant_retained_usage (tenant_id) VALUES (p_tenant) ON CONFLICT (tenant_id) DO NOTHING` (`tests/performance/scalability-ha/step_log_spike.rs:307`) runs **before** `… FOR UPDATE NOWAIT` (`:310`).
- data-model `:242-243` specifies the same order: "`INSERT … ON CONFLICT DO NOTHING` of the tenant's usage row and its `SELECT … FOR UPDATE` (`FOR UPDATE NOWAIT` when …)".
- The test `a_carry_never_waits_for_the_usage_row` holds the row with a bare `SELECT 1 FROM tenant_retained_usage … FOR UPDATE` (`step_log_leases_tests.rs:773`).

**Why it matters.** PostgreSQL's pre-check for `ON CONFLICT` (`ExecCheckIndexConstraints` → `check_exclusion_or_unique_constraint` with `CEOUC_WAIT`) reads the conflicting row with a dirty snapshot.
- If the row's xmax is an in-progress transaction that **updated** it, the insert waits for that transaction to end.
- If the row was only locked (`FOR UPDATE`, which sets a lock-only xmax), it does not wait.

Every real holder of the usage row updates it before committing: a refill, the content and log finalizer (held to the end of its WAL flush), a release and the reclaim on a limit change. So in practice a carry that needs a refill waits under the run lock, with no lock timeout, behind exactly the holders the fix meant to skip. That contradicts ADR-0097:82-83 ("`NOWAIT` … as a carry does"), ADR-0095 decision 7 ("a refill's usage row only `NOWAIT`") and design `:1000-1006` ("a carry adds no wait under the run lock"). The test passes only because its holder never updates the row. Mutation R4 (no `NOWAIT`) cannot detect this.

**Fix**
1. Take the row with `SELECT … FOR UPDATE NOWAIT` first. Only when no row exists, run `INSERT … ON CONFLICT DO NOTHING` and then the `NOWAIT` select again. Better still, guarantee the row exists before any charge (when the tenant is created or by T041-4's backfill) and drop the insert from the refill.
2. Make the test's holder do what real holders do: `UPDATE tenant_retained_usage SET step_log_bytes = step_log_bytes WHERE tenant_id = $1`, held open. Add a case where the holder is a real refill or a content finalizer held before its commit.
3. Record the new mutation ("insert before the `NOWAIT` select") in the validation report.

### N2 (Minor). The refusal handler loses the final mark of an existing stream and catches too broad a condition

**Evidence**
- The prototype's `WHEN program_limit_exceeded` handler (`step_log_spike.rs:550-560`) writes the mark-only stream with `INSERT … ON CONFLICT … DO NOTHING`. For a long step whose stream already exists, it records nothing (no final mark, no drop count) and still returns `mark_only`.
- The design's text (`:1045-1047`, "the function writes the stream's final mark as a mark-only carry") does not say that rolling back the subtransaction also releases the `FOR UPDATE SKIP LOCKED` row lock that CC′ took inside it. The mark-only write must therefore take the stream row again (`SKIP LOCKED`, `contended` if held). A plain `UPDATE` there would wait.
- `program_limit_exceeded` (54000) is a whole SQLSTATE that PostgreSQL also raises for unrelated limits. The design gives each refusal a constraint name of its own (T041-5).

**Why it matters.** Built as prototyped, a long step that meets the limit breaks "lines stored + dropped = `last_sequence`" and shows "ended without its final flush" instead of `tenant_quota`.

**Fix.** In §2a and data-model §8a, specify:
- the handler matches by constraint name (`GET STACKED DIAGNOSTICS … CONSTRAINT_NAME`) and re-raises anything else;
- for an existing stream it takes CC′ again with `SKIP LOCKED` and updates the final mark and drops; for a new stream it inserts.

Add a T041-36 test for a long step refused at the limit.

### N3 (Minor). Telemetry wiring is unspecified for anything counted inside SQL

**Evidence**
- `step_log_quota_refills_total{kind}` (ADR-0097:102; data-model `:354`) counts events that happen inside trigger and definer functions, which the application never sees.
- A carry's refusal is now caught inside `sagat_step_log_carry` (ADR-0095 decision 5), so its `cause` (`limit` or `held`) never reaches the application unless the function returns it.
- The gauge `step_log_quota_lease_rows` is "published by the purge pass" (data-model `:267-268`), but the purge runs as `sagat_app`, which has "none, revoked explicitly" on `step_log_quota_leases` (`:286`).
- `step_log_carries_total{outcome}` (`:346-347`) has no `contended` value, even though `contended` is now also the result of a refill that finds the row held.

**Why it matters.** The obvious implementation of a counter, an `UPDATE` of a counter row inside the refill, would bring back a hot row. Without a stated mechanism, M7's metrics and alert cannot be built as specified.

**Fix**
1. Count refills and their kind with sequences (`nextval`, which takes no row lock, as `step_log_diag`'s `slot_fallbacks` did), read as deltas by a definer accessor that the purge pass calls; that accessor also returns the lease-row count.
2. Make the carry function return the refusal cause in its result.
3. Add `contended` to the closed set of `step_log_carries_total{outcome}`.

### N4 (Minor). The limit-change reclaim relies on a fence the caller must take, and the spec disagrees on who calls it

**Evidence**
- ADR-0097:94-95 and data-model `:251-253`: the reclaim runs "holding the step-log fence exclusively (so no lease is held)".
- The prototype implements it as a trigger on `quota_limits` and `quota_ceilings` (`step_log_spike.rs:394` onward) that does not take the fence itself, and resets leases with a plain `UPDATE … SET granted = 0, used = 0`.
- T041-5 (design `:2230-2231`) says `quotas.rs` calls `sagat_quota_step_log_reclaim`, yet the function is revoked from `sagat_app` (data-model `:296-297`; design §3 Grants), and `quotas.rs` runs as `sagat_app` under `SAGAT_TEST_AS_RUNTIME_ROLE=1`.

**Why it matters.**
- Any writer of those rows that does not take `quota-policy:tenant_step_log_bytes:<tenant>` exclusively (the editor does take it, `content_usage.rs:92`) would make the reclaim's `UPDATE` wait on a held lease, breaking "leases are never waited for". Worse, it would zero a `used` that was committed after the `Σ used` it had already folded, so those bytes would drop out of U.
- As T041-5 is written, the call either fails as the runtime role or invites granting `EXECUTE`.

**Fix.** Keep the trigger form. Have the trigger function take the step-log fence exclusively itself (re-acquiring an advisory lock the transaction already holds returns at once), and compute the fold in the same statement as the reset (`UPDATE … RETURNING used`). Correct T041-5 to "the trigger on `quota_limits` and `quota_ceilings`".

### N5 (Minor). Near the limit, `NOWAIT` turns quota drops into silent `contended`

**Evidence**
- ADR-0097:141-143 and design `:1169-1173`: at the limit "about one refill per charge", and "a carry's refill then answers `contended` when the row is held".
- A `contended` carry "writes nothing more" (design `:1047-1049`, and the prototype's `WHEN lock_not_available THEN RETURN 'contended'` at `:562`).

**Why it matters.** At or near its limit, a busy tenant's carries are dropped with no mark, no drop count and no `ceiling_reason`. The reader sees "ended without its final flush" rather than the quota. At the moment it matters most, the reason disappears, along with the counting rule's completeness for those attempts.

**Fix.** On `lock_not_available` from the refill, write a mark-only stream that charges nothing, with a closed reason (for example `quota_busy`), counted in `step_log_lines_dropped_total{reason}`. Alternatively, when CB's allowance read shows the tenant within one grant of its limit, skip CD and write `tenant_quota` mark-only. Test it with the usage row held by an updating holder (see N1).

### N6 (Minor). One subtransaction per carrying report is not recorded as a risk

**Evidence**
- ADR-0095:85-87 and design `:1000-1006`: "one subtransaction" on every carrying report.
- Its cost is left to the third run, but the Risks table (design §Risks) has no row for it.

**Why it matters.** A subtransaction that writes takes an XID of its own, so carrying reports roughly double their XID use, which adds freeze and wraparound pressure. Subtransactions are also PostgreSQL's known source of `pg_subtrans` SLRU contention under a long-running transaction, most of all on hot standbys. At hundreds of carries a second this becomes the platform's highest-rate source of subtransactions.

**Fix.** Add a Risks row naming the XID use and the `pg_subtrans` behaviour. Have T041-35's (d), the held-transaction workload, or T041-9 report `pg_stat_slru` for `subtransaction` and the XID rate, and state the deployment guidance (PostgreSQL 17+ `subtransaction_buffers`, standbys).

### N7 (Minor). Stale text left by the revision

- **ADR-0027:199-204 (Note):** "the finalizer's step-log branch, reached then only by a release, folds the tenant's leases first". That is now the opposite of ADR-0097 decision 5. The Note also does not mention that ADR-0097 supersedes the non-negative counter rule for this class.
- **`architecture-principles.md:31`:** says "only a refill or a release locks the row", without the limit change (ADR-0097:28 lists it). It also lacks the two new rules worth a check: a carry's refill is `NOWAIT`, and a release never takes a lease.
- **ADR-0095 "Enforced by" (`:12-24`):** still lists only "the re-run tests of decision 5". It needs the in-call refusal test and the `NOWAIT` test.
- **G041-023 (`research.md:151`):** still says only "counted `tenant_quota`", with no `cause="held"` and no `tenant_step_log_quota_held`.

**Fix:** update all four.

### N8 (Nit). The caller-settable `NOWAIT` flag is not described against ADR-0027's rule on session flags

- `sagat.step_log_charge_nowait` is set by the `SECURITY INVOKER` carry and read by a `SECURITY DEFINER` refill. ADR-0027:53 forbids caller-controlled flags that authorize a bypass.
- This flag can only turn a wait into a refusal. ADR-0097 decision 4 should say so, and that the flag is set transaction-locally.
- A T041-5 test should show that setting it session-wide admits nothing more and only makes ingests refuse instead of wait.

### N9 (Nit). An underflow refusal is not counted

`step_log_usage_underflow` (data-model `:259`) fails the purge transaction, and the next pass retries it, possibly forever if the accounting is wrong. Count it, for example as `step_log_purge_failures_total{cause="underflow"}`, or name it in the `SagatStepLogPurgeBehind` runbook entry.

---

## Checked with no new finding

- The third run's protocol: the added declarations (the scenario's step-log limit, the review fixes, the predicted effect on each gate, what follows a failure, the preview run 44753470534) were written before any gated round. Gates, thresholds and the validity rule are unchanged.
- Considered options 4, 5, 8 and 9: fairly stated now.
- The lease table's fill factor and autovacuum settings, and the foreign key to the usage row.
- The mutation record (`validation-report.md:564-593`).
- Layering: still nothing in the domain, and the carry still goes through one port.

## Count

| | Blocker | Major | Minor | Nit |
|---|---|---|---|---|
| Earlier findings still open | 0 | 1 (M2, as R1) | — | — |
| New findings | 0 | 1 (N1) | 6 (N2 to N7) | 2 (N8, N9) |

M7 counts as closed in substance; its remaining wiring gap is N3.
