Second re-read by the `architecture` reviewer of the fixes at `35ec06b1` (answering its re-read of `3f1f0aba`). Saved verbatim.

# Second re-read (architecture): `35ec06b1` against the re-read of `3f1f0aba`

Paths are relative to the worktree root `/Users/marcelo/Documents/Projects/sagat-wt-041`.

## Verdict

- **R1 (the acceptance route):** closed. A few lines elsewhere still name the owner as the one who accepts; they are listed in M1 below.
- **N1 (the carry waited for an updated usage row):** closed.
- **The fast path with no subtransaction:** sound. The reserved lease is locked by the same transaction, so the charge finds it, and S + G ≤ L cannot change between the reservation and the charge while the carry holds the class fences. Two rules it rests on are not stated as MUST and have no test: m1 and m2.
- **New findings:** none Blocker or Major. Five Minor and three Nit.
- **Earlier Minors N2 to N9:** all addressed.

## What I read

- `resolution.md` ("Re-reads of 3f1f0aba"), D041-113, and both diffs.
- The current ADR-0097 header, decisions and options; ADR-0095's header, decision 5 and Enforced by; the ADR-0027 Note.
- `architecture-principles.md:31`.
- data-model §3b, §4, §7 and §8a.
- Design 041 §2's lock order, §2a, §2b, the open owner questions, the Risks, the Constitution Check and T041-5.
- The validation report's mutation table.
- The prototype as committed: `step_log_spike.rs` and `step_log_leases_tests.rs`.
- The edited T041-4 migrations.

Nothing was built or run.

---

## The earlier Majors

### R1, the acceptance route: closed

- **ADR-0097:4-8** gives one route. The coordinating session accepts the ADR under OD-2026-11-20-6 once the re-review has no Blocker or Major and T041-35's third run is recorded, whatever its result, because the decision rests on D041-106's ceiling and not on the 1 × 16 gate. It is accepted together with the restated O041-040. ADR-0095 is accepted the same way, but only with a pass. README rule 4 is answered explicitly by the delegation.
- **ADR-0095:4-7** now says the same.
- **O041-040 is restated** in design `:429-435` and in `research.md:202`. Option (a) accepts the statement-level measurement, with the charge going to a lease, and the per-row release. The deferred flush "being replaced by ADR-0097". Accepting (a) no longer contradicts ADR-0097.
- **O041-038** in design `:436` gives the same route.

The ADR now records the coordinating session's acceptance consistently.

### N1, the carry waited for an updated usage row: closed

- The refill now locks first: `SELECT … FOR UPDATE [NOWAIT]` (`step_log_spike.rs:216-226`). It runs `INSERT … ON CONFLICT DO NOTHING` only when no row exists, then selects again.
- data-model `:416-419` and ADR-0097 decision 4 say the same and give the reason.
- The test's holder now updates the row (`step_log_leases_tests.rs`, `UPDATE tenant_retained_usage SET step_log_bytes = step_log_bytes …`). That is what real holders do.
- Mutation R7, "the refill inserts the usage row before its NOWAIT lock", fails that test (validation-report table).

### The fast path: sound, with gaps m1 and m2

**Why it holds.**
1. The carry tries the three class fences in CA (shared), before it reserves.
2. `step_log_reserve` locks a lease that has room for the planned charge, `SKIP LOCKED`.
3. The charge's scalar subquery skips leases that other transactions hold, but not the ones this transaction holds, so the reserved lease stays available to it.
4. Nobody can reduce that lease's room while it is held.
5. S + G ≤ L cannot become false in between:
   - a limit change needs the step-log fence exclusively, and the carry holds it shared;
   - other refills keep S + G ≤ L (I4);
   - reclaims and releases only lower S + G.

So, provided the reserved bytes are at least the bytes the trigger measures, CD's charge cannot be refused and cannot reach the refill.

**Evidence.** The test `only_a_carry_that_needs_a_refill_opens_a_subtransaction` uses `xmin` to show that the fast path writes with the transaction's own id. Mutation R8 fails it.

**Recorded.** Option 10 (ADR-0097:178-181) and the Risks row (design `:1699`) record the subtransaction trade-off.

---

## New findings

### Major

None.

### Minor

#### M1. Residual text still names the owner as the one who accepts

**Evidence**
- `docs/adr/ADR-0027-…md:169`: "Proposed amendment of the Amendment (2026-11-20), **for the owner's acceptance** of its wording (O041-040)". Lines `:171-172` add "changes nothing above until the owner accepts it".
- `docs/designs/041-step-logs.md:2113`: "Waits for O041-038 (the owner's acceptance of ADR-0095, asked once T041-35 passes)". Line `:2396` repeats "the owner's acceptance of ADR-0095".
- `specs/041-step-logs/research.md` O041-038 row: "Asked once T041-35 passes … Open". It does not mention the delegated route of D041-113.

**Why it matters.** These lines describe a different acceptor from ADR-0097 and ADR-0095. ADR-0027 is the accepted ADR a future reader will open first.

**Fix.** Change these five places to the D041-113 route: "accepted by the coordinating session under OD-2026-11-20-6 …".

#### M2. The reservation's byte count is not tied to the trigger's measure

**Evidence**
- data-model `:517`: `sagat_quota_step_log_reserve(p_tenant, <the kept rows' text bytes plus 128 a row>)`.
- D041-113 claims this is "the statement trigger's measure".
- The trigger sums what `sagat_quota_measure` gives for `message`, `exception` and `fields` (canonical JSON), plus 128 a row, including the planned marker row (data-model `:164-168`).
- The prototype reserves `octet_length(m) + 128` of the messages only.
- No rule or test requires the two figures to be equal.

**Why it matters.** If the reservation is smaller than the measured charge, CD's charge can miss the reserved lease and fall into the refill on the fast path, where no block runs. A held usage row (`NOWAIT`) or a refusal then fails the report's transaction. ADR-0095 decision 5 sends that to the downgrade re-run, with its G041-021 window. Design `:1009-1012` says such a charge "cannot be refused", which would be false.

**Fix.**
1. State as a MUST in ADR-0097 decision 3 and in data-model §8a that the reservation is computed from the planned rows with the same `sagat_quota_measure` expressions the trigger uses, the marker row included.
2. Add a property test in T041-36 that the reserved bytes equal the trigger's charge for generated carries: Unicode, JSON fields, NUL-replaced text, a ceiling marker.
3. Optionally, have the charge raise a distinct guard if a reserving transaction ever reaches the refill, so that a mismatch shows up as a typed error instead of a silent re-run.

#### M3. `sagat_quota_step_log_reserve` is callable on any tenant with any byte count, and does not check the fences it relies on

**Evidence**
- data-model `:432-437`: `EXECUTE` to `sagat_app`. It "can lock a lease of any tenant it is given for the rest of the caller's transaction", and runs "after the fences the carry holds".
- The prototype's function takes no fence and accepts `p_bytes = 0`, which matches every lease, since `granted - used >= 0`.

**Why it matters**
- Any runtime-role code path (a bug or a compromised replica) can lock all 256 leases of another tenant, one call each, inside one long transaction. That tenant's charges then all go to the refill: the hot row returns, and near the limit they become `held` refusals.
- A lease held without the step-log fence also makes a limit change fail: the reclaim trigger takes leases `NOWAIT` (ADR-0097 decision 6).

**Fix.**
1. Have the reservation try the three class fences shared itself, and return false if they are held.
2. Refuse `bytes ≤ 0` and anything above the largest carry charge.
3. Return false without locking when the transaction already holds a reserved lease.
4. List it in the runtime-role bypass tests of T041-5.

#### M4. The block's handler does not re-check the existing stream after taking it again

**Evidence**
- Design `:1049-1053` and data-model `:513-515`: when the handler catches a refusal for an existing stream, the stream is "taken again `FOR UPDATE SKIP LOCKED` and updated the same way".
- The prototype re-checks only `final_at IS NULL`.

**Why it matters.** The block's rollback released CC′'s row lock. Between the rollback and the second lock, the purge can mark the stream `removed_at`; a final mark could also have been written, or the order guard changed. Updating without CC′'s checks sets `final_at` and drop counts on a stream whose removal has begun, which design §4's "no batch or carry can add a line to a stream whose removal has begun" rules out for the counters as well.

**Fix.** After taking the row again, re-apply CC′'s checks (removed or expired → `removed`; already final → `final_recorded`; out of order) before updating. Add this case to T041-36 with the purge marking the stream between the two locks, using a hold trigger as the existing tests do.

#### M5. A carry far from the limit can lose its records under the label `tenant_quota`

**Evidence**
- The handler answers the refill's `lock_not_available` with a mark-only carry whose `ceiling_reason = 'tenant_quota'` (data-model `:511-515`; prototype handler). The metric's cause is `busy`.
- A refill happens about once per 300 carries far from the limit (design `:1169-1173`).
- The usage row is busy about 23-24% of the time under completions that store output (D041-106).

**Why it matters.** Roughly one carry in a thousand or so, on a tenant with plenty of quota left, drops its last records. The reader of the stream sees "tenant quota" as the reason, which is false. Only the metric says `busy`. G041-023 describes held room, not this case, and neither the ADR nor the design states the rate.

**Fix.**
1. Give this case its own closed `ceiling_reason` and drop reason (for example `quota_busy`), so the stream does not claim the quota was reached.
2. Add a gap row (or extend G041-023) with the expected rate: refill rate × the row's busy fraction.
3. Have T041-35 and T041-9 report the share of `busy` carries.
4. Optionally, reduce the rate by refilling a lease while it still has room for one more carry, so the refill falls on the ingest path, which may wait, rather than on a carry.

### Nit

#### N1. The lock-order text is stale
- Design `:876` still says a carrying report "tries … L3b at CD".
- On the fast path the lease is now tried by the reservation, after CA and before CC and CC′.
- **Fix:** update the sentence. Tries stay exempt from the order (M8).

#### N2. The option numbering is out of order
- In ADR-0097, option 10 is inserted before option 9 (Considered options, after option 8).
- **Fix:** renumber.

#### N3. The refill can still wait once per tenant, and its loop does not check the second select
- The first refill of a tenant whose usage row does not exist yet runs `INSERT … ON CONFLICT DO NOTHING`, which waits for a concurrent inserter of the same row even under `NOWAIT`. That happens once in a tenant's life.
- The loop at `step_log_spike.rs:216-226` also continues with `v_s` and `v_g` null if the second select finds nothing.
- **Fix:** create the usage row with the tenant, or in T041-5's migration for existing tenants, so a refill never inserts it. Raise when the row is still not found after the loop.

---

## The earlier Minors (N2 to N9 of the first re-read)

- **N2** (the handler): addressed by D041-113 (2), subject to M4 above.
- **N3** (telemetry wiring): addressed. Sequences are read as deltas through `sagat_quota_step_log_counters`, the carry returns its cause, and `contended` is now in the carries' outcomes (data-model `:451-455`, `:492`).
- **N4** (the reclaim): addressed. It is a trigger that takes the fence exclusively itself, then the leases `NOWAIT`, then the usage row.
- **N5:** addressed, subject to M5 above.
- **N6:** addressed. Option 10 and the Risks row record the subtransaction cost.
- **N7** (stale text): addressed. The ADR-0027 Note, `architecture-principles.md:31`, ADR-0095's Enforced by and G041-023 are updated.
- **N8:** addressed. ADR-0097 decision 4 states what the flag can and cannot do.
- **N9:** addressed. `step_log_purge_failures_total{cause="underflow"}`.

The edits to T041-4's unmerged migrations (removing `tenant_retained_usage_step_log_nonnegative` from the up migration, its validation and the down migration) follow D041-091's "hold unmerged and merge with whatever the review changes". Nothing else in the repository refers to that constraint except the spike's `DROP CONSTRAINT IF EXISTS` (`step_log_spike.rs:274`).

## Count

| | Blocker | Major | Minor | Nit |
|---|---|---|---|---|
| Earlier findings still open | 0 | 0 | — | — |
| New findings | 0 | 0 | 5 (M1 to M5) | 3 (N1 to N3) |
