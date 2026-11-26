Second re-read by the `egm-design` reviewer of the fixes at `35ec06b1` (answering its re-read of `3f1f0aba`). Saved verbatim.

# Second re-read of design 041 at 35ec06b1: are N1 and N2 closed?

**Reviewer:** egm-design (Design Critic Goldfish), mode REVIEW, read-only.

**Scope:** the Majors of my re-read of 3f1f0aba (N1 and N2), plus the Minors N3 and N4 it raised, checked against:
- `resolution.md` ("Re-reads of 3f1f0aba") and D041-113
- `fix2-docs.diff` and `fix2-code.diff`
- the files at 35ec06b1: ADR-0095 decision 5; ADR-0097 decisions 1, 3, 4, 6 and 8, option 10 and the Consequences; design §2a and §2b and the Risks; `data-model.md` §3b, §4 and §8a; `step_log_spike.rs` (`step_log_reserve`, `step_log_refill`, `step_log_reclaim`, `step_log_carry`, `step_log_carry_write`); the validation report's mutations R7 to R9

**Verdict:** N1 and N2 are closed, and so are N3 and N4. The fixes introduce no Blocker and no Major. They introduce four Minor findings and two Nits, mostly around the new reservation (the fast path) and the way the counters are published.

## Status of the earlier findings

| Finding | Status | Evidence |
|---|---|---|
| **N1.** The handler was not in the spec, lost a long step's mark, and caught a whole SQLSTATE class | **Closed** | data-model.md §8a ("The fast path and the block") now describes the block. Its handler reads `GET STACKED DIAGNOSTICS … RETURNED_SQLSTATE, CONSTRAINT_NAME` and re-raises anything other than `tenant_step_log_quota_exceeded`, `tenant_step_log_quota_held` and `55P03`. On an existing stream it takes the row again `FOR UPDATE SKIP LOCKED` and updates it, answering `contended` only when that row is held. The stale re-run text is gone; any other raise is mapped to `StepLogRefused` and counted `failed`. Prototype: step_log_spike.rs carry handler (fix2-code.diff:401-491). Test `a_long_steps_carry_refused_by_the_limit_records_its_mark_on_its_stream`; mutation R9 (validation-report.md). |
| **N2.** A subtransaction on every carrying report | **Closed** | A carry first reserves a lease (`sagat_quota_step_log_reserve`, data-model.md §3b). With a reservation, CC to CE run with no subtransaction. Only a carry that needs a refill enters the block: about 1 in 300 far from the limit, most carries at the limit. Recorded in ADR-0097 option 10 and its Consequences, and in a risk row (design, Risks: "A carry without a reserved lease opens a subtransaction"); T041-9 reports the XID rate and `pg_stat_slru`. Test `only_a_carry_that_needs_a_refill_opens_a_subtransaction`; mutation R8. |
| **N3** (Minor). A held usage row loses carries uncounted | **Closed**, with a new labelling issue (see M3) | A held row now produces a mark-only final mark (cause `busy`). `contended` is answered only when the stream's own row is held. |
| **N4** (Minor). The reclaim relies on its caller's fence | **Closed** | ADR-0097 decision 6 and the spike's `step_log_reclaim`: the trigger takes the fence exclusively itself, then the leases `NOWAIT`, then the usage row, and sums the leases only after locking them. Its `NOWAIT` assumes something new code can break (see M1). |

---

## New findings

### Minor

#### M1. The reservation can be called by the runtime role but does not take the fence itself. This contradicts ADR-0097 decision 1 and can make a limit change fail.

**Evidence**
- ADR-0097:68 (decision 1): "the runtime role MUST have no privilege on the lease table, the new column's writes or the functions".
- data-model.md §3b and §4: `sagat_quota_step_log_reserve`, `sagat_quota_step_log_usage` and `sagat_quota_step_log_counters` are granted `EXECUTE` to `sagat_app`. The reservation takes its lease "after the fences the carry holds", and "it can lock a lease of any tenant it is given for the rest of the caller's transaction".
- step_log_spike.rs `step_log_reserve` (fix2-code.diff:261-272): it takes no advisory lock.
- The reclaim trigger takes the tenant's leases `FOR UPDATE NOWAIT` (fix2-code.diff:294). That is correct only if every holder of a lease also holds the step-log fence.

**Why**
- Decision 1 forbids, with a MUST, the grants that §4 now makes.
- The reservation is the first function the runtime role can call that holds a lease. Whether its caller holds the fence now depends on the call site, not on the function.
- Any call without the carry's CA (a future caller, or a refactoring of the carry) holds a lease outside the fence. A concurrent change of `tenant_step_log_bytes` then fails on `lock_not_available`, and the reservation's unlocked check of S + G ≤ L loses its basis.

**Fix**
- Make the reservation try the step-log fence shared itself (`pg_try_advisory_xact_lock_shared`; this is re-entrant when CA already holds it) and return false when it cannot get it.
- Restate decision 1 so that it names the three functions `sagat_app` may execute.
- Add a runtime-role test: the reservation called without CA, while a limit change runs.

#### M2. The fast path's guarantee that "the charge cannot be refused" depends on the planned bytes bounding the measured bytes. That equality is not tested, and a mismatch has an unspecified failure mode.

**Evidence**
- data-model.md §8a: reserve with "<the kept rows' text bytes plus 128 a row>", "After item 5's plan and before item 3's write". But for an existing stream, item 5's plan needs item 4's counters, and item 4 comes after the write of item 3.
- ADR-0095 decision 5: "with one it writes with no subtransaction, and its charge cannot be refused".
- The planned sizes come from C3 (`p_sizes`, data-model.md §8a signature). The charge is `sagat_quota_measure` over `message`, `exception` and `fields` (data-model.md:145-166).
- On the fast path the transaction has set `sagat.step_log_charge_nowait`, and no block runs.

**Why**
- The guarantee holds only if (a) C3's sizes equal what the database measures on the stored rows, and (b) the reserved amount bounds an existing stream's plan.
  - (b) holds when the new-stream plan is used as the bound: a ceiling marker replaces at least one dropped row of at least 128 bytes. The spec does not say that the new-stream plan is the bound.
  - Nothing tests (a).
- If either fails, CD's charge on the fast path goes to a refill outside any block. A held usage row then raises `55P03`, which is mapped to `StepLogCarry` and re-runs the report with `None`: the records and the final mark are lost. A refusal re-runs the report, which is the transaction the revision set out to remove.

**Fix**
- State in §8a that the reservation is made from the new-stream plan, as an upper bound of any plan for the attempt.
- Add a T041-5 property test that `sagat_quota_measure` of the stored rows equals C3's planned sizes plus 128 a row (Unicode, JSON fields, NUL-replaced text).
- Say what a fast-path refusal or `55P03` does. Better: have the charge raise a dedicated guard ("charge exceeds reservation") so that a mismatch shows up as a defect, not as a re-run.

#### M3. A transient "busy" usage row is shown to readers as `tenant_quota`, permanently, on a tenant that may be far below its limit.

**Evidence**
- data-model.md §8a: for each caught case, including `55P03`, the handler records "the records counted dropped by the server (`tenant_quota`) and `ceiling_reason = 'tenant_quota'`"; the cause (`limit`, `held` or `busy`) is only returned.
- G041-023 (research.md) covers only held room.
- validation-report.md:500: completions that store output keep the usage row busy 24.0% of the time.

**Why**
- Far from the limit, about 1 carry in 300 needs a refill, and about a quarter of those find the row held. Those streams display "tenant quota reached" for good.
- CUJ 8 asks an administrator to bound cost from what the streams show. A false `tenant_quota` invites raising a limit that was never reached.
- Held room is a documented limit (G041-023); "busy" is not documented anywhere.

**Fix:** record a drop reason of its own (for example `contended` or `busy`) and leave `ceiling_reason` unset for it. Otherwise, add the busy case to G041-023 and to the runbook, with its expected rate.

#### M4. Counters published as deltas from a sharded purge pass will be multiplied by the number of replicas.

**Evidence**
- data-model.md §3b: the sequences are read by "`sagat_quota_step_log_counters()` … the purge pass calls it and publishes the deltas", and the lease-row count is also published as a gauge.
- The scheduler's passes run on every replica (shard holders, `crates/scheduler/src/passes.rs:51-74`).

**Why**
- The sequences are global to the database. Each replica that reads them and adds the delta to its own process counter counts every event once, so the cluster-wide sum of `step_log_quota_refills_total`, `step_log_quota_refusals_total` and `step_log_purge_failures_total` is N times the truth for N replicas.
- `SagatStepLogQuotaHeldRefusals` is a ratio "above 1% of charges", so its denominator and numerator must come from the same source.
- A gauge of lease rows summed across replicas is likewise wrong.

**Fix:** publish the raw sequence values (monotonic) as a gauge or counter that is aggregated with `max` across replicas, or have exactly one replica publish (for example the holder of a fixed shard). State the aggregation in the alert's expression.

### Nit

#### N-a. The handler on an existing stream does not recheck what CC′ checked.
After the block's rollback, the stream row is unlocked until the handler takes it again. A purge mark (it takes rows `SKIP LOCKED`, and the advisory stream lock does not stop it) can set `removed_at` in that window. data-model.md §8a says only "updated the same way". The spike filters on `final_at IS NULL` only (fix2-code.diff:485-489). Have the handler recheck removed or expired (answer `removed`) and final (answer `final_recorded`) under the re-taken lock.

#### N-b. A reserved carry can hold two leases.
The charge chooses "one lease with room" by the rotation order. If another refill gave room to a lease earlier in that order after the reservation, the charge takes that one too, so the transaction holds two leases and other charges skip both. Pass the reserved lease number to the charge (for example as a transaction-local setting) and try it first.

---

**Count of new findings:** Blocker 0, Major 0, Minor 4, Nit 2.
**Earlier findings:** N1 and N2 (Major) closed; N3 and N4 (Minor) closed.
