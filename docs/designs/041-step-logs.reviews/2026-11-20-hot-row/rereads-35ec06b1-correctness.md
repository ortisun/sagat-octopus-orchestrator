Second re-read by the `correctness` reviewer of the fixes at `35ec06b1` (answering its re-read of `3f1f0aba`). Saved verbatim.

# Second re-read of design 041's hot-row revision at 35ec06b1 (correctness)

The N1 Major from the previous re-read is closed: no carry now loses its records uncounted. All seven earlier Minor and Nit items N2 to N8 are closed. The fixes introduce no new Blocker or Major. They do introduce four new Minors and two new Nits, and a slow-path refill whose usage row is busy now drops lines, counted but labelled as quota (C1).

I read the "Re-reads of 3f1f0aba" table in `resolution.md`, D041-113, `fix2-docs.diff` and `fix2-code.diff`, and the current files in `/Users/marcelo/Documents/Projects/sagat-wt-041`. I ran nothing. Line numbers refer to the files at 35ec06b1.

## Status of the findings of the previous re-read

| Finding | Status | Evidence |
|---|---|---|
| **N1 [Major]** Near the limit, a held usage row made carries `contended` with nothing recorded | **Closed** | A refill that finds the row held (`55P03`), or a refusal, now records a mark-only final mark, so the drops are counted: design `:1050-1054`, data-model `:519-525`, prototype `step_log_spike.rs:601-620`. `contended` remains only when an existing stream's own row is held. The refill now locks the row before it would ever insert it, so a holder that updated the row is not waited for either (prototype `:300-313`). The test's holder now updates the row, as real holders do (`step_log_leases_tests.rs`, `a_carry_never_waits_for_the_usage_row`). What remains is attribution, not lost counts; see C1. |
| N2 Down migration leaves S negative | Closed | data-model `:289` folds Σ `used` into S. T041-4's CHECK is gone from its migration (`20270201000000_step_log_quotas.up.sql:100`). |
| N3 Handler catches all of class 54; a long step's mark is lost | Closed | The handler matches the two constraint names and `55P03` and re-raises anything else (data-model `:519-524`, prototype `:601-606`). An existing stream is taken again `SKIP LOCKED` (prototype `:610-620`). New test: `a_long_steps_carry_refused_by_the_limit_records_its_mark_on_its_stream`. One residual is N-b. |
| N4 S + G > L path untested | Closed | `a_charge_past_the_leased_room_reclaims_before_it_refuses`, with the reclaim triggers disabled. |
| N5 Marker's held refusal counted `failed`; stale lines | Closed | Marker: data-model `:540-541`. Risks row: design `:1677`. One stale passage remains; see C2. |
| N6 Reclaim after L4 relies on its caller's fence | Closed | The trigger takes the fence exclusively itself, then the leases `NOWAIT`, then L4 (prototype `step_log_reclaim`; data-model `:259-261`; design `:1165-1167`). The reservation creates a new way to break the fence assumption; see C3. |
| N7 Enforcement off: the charge still routes to the refill | Closed for the charge (data-model `:251-252`) | The reservation does not follow it; see N-a. |
| N8 A subtransaction on every carrying report | Closed | Only the slow path opens the block. Test: `only_a_carry_that_needs_a_refill_opens_a_subtransaction`, which checks `xmin`. |

## New findings introduced by the fixes

### C1 [Minor] A refill that finds the usage row busy drops a carry's lines with reason `tenant_quota`, even far from the limit

**Evidence.**
- The handler writes `ceiling_reason = 'tenant_quota'` for the `busy` cause too (data-model `:522-525`; prototype `:607-614` writes `'tenant_quota'` unconditionally). The `busy` cause appears only in the return value and the metric.

**Failing scenario.**
- Tenant at 1% of its quota. A carry is the one in about 300 that needs a refill.
- L4 is held about 24% of the time by the content finalizers of reports with output (validation report, off-1K).
- So about 0.08% of such tenants' carries lose all their lines.
- The reader sees `ceiling_reason = tenant_quota`, which tells the user the quota is exhausted when it is not. That is a misleading counted drop, and a loss that did not exist at 4d74b27a, where this refill waited.
- Also, design `:1183-1184` still says "a carry's refill then answers `contended` when the row is held". That contradicts `:1053` and `:1185`.

**Fix.**
- Record the busy case under a reason of its own (`ceiling_reason = 'quota_busy'` or similar, added to `step_log_streams_ceiling_valid`, the contracts and the reader's labels). Or keep `tenant_quota` and state it, with its rate, in G041-023.
- Optionally cut the loss: before the mark-only, retry the reservation once, since another carry's refill may just have granted a lease. This adds no wait.
- Correct design `:1183-1184`.

**Confidence:** confirmed in the text and the prototype; the rate is estimated.

### C2 [Minor] The reservation's amount is placed "after item 5's plan and before item 3's write", but an existing stream's plan exists only after CC′; nothing requires charge ≤ reservation

**Evidence.**
- data-model `:515-517`: "After item 5's plan and before item 3's write … `sagat_quota_step_log_reserve(p_tenant, <the kept rows' text bytes plus 128 a row>)`". But item 5 (the plan of an existing stream, `:494-506`) follows item 4 (CC′, `:491-493`), which follows item 3 (CC).
- The fast path's CD runs outside the block. If CD's charge exceeds the reserved room and no other free lease has room, the refill runs unguarded. Its `NOWAIT` (`55P03`) or a refusal then aborts the report and leads to a `StepLogCarry` or `StepLogRefused` re-run: the failure mode M2 removed.
- The prototype is safe because it reserves the sum of every sent record plus 128 a row before any write (prototype `:580`). The design text does not say that.

**Fix.**
- State that the reservation, computed before CC, is an upper bound of CD's charge for both stream cases: every sent record's measured size plus 128 a row, plus 128 for a possible marker. Item 5 plans a marker only when a record is dropped, so the bound holds without that extra 128; adding it is cheap insurance.
- Add to T041-36's property test that the charge CD measures never exceeds the reserved bytes, over new and existing streams and each ceiling.
- Optionally, make the fast path's charge never call the refill (raise a guard instead), so any violation is caught by T041-36.

**Confidence:** confirmed in the text; the abort is plausible only if built as worded.

### C3 [Minor] The reservation is callable without the fences and for any tenant, so it breaks the reclaim's "no lease is held"

**Evidence.**
- data-model `:253-258`: `sagat_quota_step_log_reserve` has `EXECUTE` to `sagat_app` and is "after the fences the carry holds". It takes no fence itself (prototype `:369-379`), and "it can lock a lease of any tenant it is given".
- The reclaim trigger relies on its exclusive fence meaning that no lease is held, and takes the leases `NOWAIT` (data-model `:259-261`).

**Failing scenario.**
- Any runtime-role transaction calls `sagat_quota_step_log_reserve(T, 0)` without holding T's fences, and stays open.
- A change of T's limit then fails with `lock_not_available` from the trigger's `NOWAIT`, which the API reports as an internal error.
- Repeated calls with different byte sizes can lock several of another tenant's leases. That pushes the victim's charges to refills on L4, and pushes the victim's carries to the busy mark-only of C1.

**Fix.**
- Have the reservation take T's step-log fence itself with `pg_try_advisory_xact_lock_shared`. That is re-entrant for the carry, which already holds it. Return false when the fence is not free.
- Refuse `bytes <= 0`.
- Add to T041-5's runtime-role tests: a reservation without the fence returns false, and a limit change commits while a carry holds a reserved lease.

**Confidence:** confirmed in the text.

### C4 [Minor] data-model `:210` still says T041-4's CHECK "is dropped before T041-4 merges"

The resolution and `20270201000000_step_log_quotas.up.sql:100` show it is already removed from T041-4's migration.

**Fix.** Reword the comment to say T041-4 adds no such CHECK.

**Confidence:** confirmed.

### N-a [Nit] The reservation ignores the enforcement switch

**Evidence.** The reservation returns false whenever S + G > L (prototype `:375`; data-model `:254`). With enforcement off and usage past L, every carry therefore takes the slow path: a subtransaction plus a `NOWAIT` refill on L4, with the busy mark-only of C1. The charge, by contrast, now skips its S + G ≤ L test when enforcement is off (data-model `:251-252`).

**Fix.** Skip the reservation's limit test when enforcement is off, as the charge does.

### N-b [Nit] The handler's retake of an existing stream skips the removal and expiry checks

**Evidence.** The block's rollback releases the stream row. Between that rollback and the handler's `SKIP LOCKED` retake, the purge can mark the stream removed: the attempt's stream lock (L1b) does not exclude the purge. The prototype's retake checks only `final_at IS NULL` (`:618`). So it would set `final_at` and count drops on a stream whose removal has begun. data-model `:522-524` does not say the retake rechecks.

**Fix.** In the retake, apply CC′'s conditions again (`removed_at IS NULL AND expires_at > clock_timestamp()`), and answer `removed` otherwise.

Count: Blocker 0; Major 0 (N1 closed); Minor 4 (C1, C2, C3, C4); Nit 2.
