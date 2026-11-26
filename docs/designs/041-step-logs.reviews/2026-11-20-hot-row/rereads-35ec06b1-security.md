Second re-read by the `security` reviewer of the fixes at `35ec06b1` (answering its re-read of `3f1f0aba`). Saved verbatim.

I've re-read 35ec06b1. Here is the full report.

# Security re-read: design 041 hot-row revision, fixes at 35ec06b1

All four of my findings on 3f1f0aba are closed. The new functions add one Minor and two Nits, and leave one suspicion for the correctness reviewer. The Minor: `sagat_quota_step_log_reserve` relies on its caller already holding the quota fences, which is the same pattern as the Major I raised last time, now in a smaller form.

Worktree: `/Users/marcelo/Documents/Projects/sagat-wt-041`; paths below are relative to it. Read:
- `docs/designs/041-step-logs.reviews/2026-11-20-hot-row/resolution.md`, section "Re-reads of 3f1f0aba"
- `fix2-code.diff` and `fix2-docs.diff`
- the current `specs/041-step-logs/data-model.md` §3b and §4
- the current `docs/designs/041-step-logs.md`, section 2a (`:1007-1024`) and T041-5 (`:2205-2249`)
- the migrations directory

## Status of my findings on 3f1f0aba

**Major 1 (the limit-change reclaim relied on its caller's fence and could not be called by the runtime role): closed.**
- The reclaim is now an `ENABLE ALWAYS` `SECURITY DEFINER` row trigger on `quota_limits` and `quota_ceilings`, executable by no one (`data-model.md:259-262`, `:309-310`; `041-step-logs.md:2249`).
- It takes the step-log fence exclusively itself. Then it locks the tenant's leases, then the usage row, and only then sums and zeroes (prototype `fix2-code.diff:291-297`).
- The leases are locked before the sum, so the race I described (sum, then a charge commits, then zero) cannot happen.
- `quotas.rs` no longer calls it.
- The reclaim's safety now rests on one invariant: whoever holds a lease also holds the step-log fence. The new reserve function breaks that invariant outside the carry; see New Minor 1.

**Minor 1 (T041-4's non-negative CHECK on S): closed.** No migration under `crates/infra-postgres/migrations/` defines `tenant_retained_usage_step_log_nonnegative` any more. Two places still say "dropped before T041-4 merges" in the past tense, which is now stale but harmless: the comment at `data-model.md:210-211` and the text at `041-step-logs.md:2208`.

**Nit 1 (T041-5's checks): closed.**
- The source-text check now names the refill, the release branch and the limit-change trigger (`041-step-logs.md:2218-2219`).
- The runtime-executable list now names the allowance, reserve, usage and counters functions (`:2233-2234`).

**Nit 2 (the handler caught every SQLSTATE 54000): closed.** It now matches the two constraint names and `55P03`, and re-raises everything else (`fix2-code.diff:401-406`). One remainder is noted as New Nit 1.

**Earlier closures still hold:**
- No transaction waits for a lease.
- The release takes no lease.
- A charge with S + G > L goes to the refill.
- The `sagat.step_log_charge_nowait` setting still decides nothing about quota, now with a test that a session-wide value admits nothing more (`041-step-logs.md:2217-2218`).
- The refill now locks the usage row first and inserts it only when it is absent (`fix2-code.diff:214-226`), so `NOWAIT` cannot be defeated by an `INSERT … ON CONFLICT` that waits.

## New findings

### Minor 1: `sagat_quota_step_log_reserve` can hold a lease without the fences, which can make limit changes fail

**Evidence**
- The function is granted to the runtime role and takes the tenant as an argument (`data-model.md:253-258`):
  - "`sagat_quota_step_log_reserve(tenant, bytes)` (`EXECUTE` to `sagat_app` …): after the fences the carry holds, … one lease with room ≥ bytes locked `FOR UPDATE SKIP LOCKED` for the transaction"
  - "It can lock a lease of any tenant it is given for the rest of the caller's transaction"
  - grant at `data-model.md:310-311`
- The prototype takes no fence (`fix2-code.diff:261-272`).
- The reclaim assumes no lease can be held while it holds the fence exclusively (`data-model.md:260-261`: "…exclusively itself (at once when the editor holds it, so no lease can be held), then the tenant's leases `FOR UPDATE NOWAIT`"). The prototype trigger does this at `fix2-code.diff:293-294`.

**Scenario**
1. Any runtime transaction calls `sagat_quota_step_log_reserve(T, 0)` without holding T's fences. That could be a code path other than the carry, a later refactor that moves the call ahead of CA, or a compromised runtime role.
2. That transaction now holds one of T's leases with no step-log fence.
3. An admin then changes `tenant_step_log_bytes` or its ceiling for T. The editor gets the exclusive fence at once, because nobody holds it shared.
4. The reclaim trigger's `FOR UPDATE NOWAIT` on T's leases hits the reserved lease and raises 55P03, so the admin's quota change fails.
5. Across pooled connections, such a caller can also hold many of T's leases. T's charges then fall to refills and `held` refusals.

**Impact**
- No overshoot and no change to the counters; exactness is intact.
- Availability: authorized quota and policy changes can be blocked, and one tenant's logging can be degraded by a runtime caller that names it.
- It is the pattern of my earlier Major: a definer function granted to the runtime role whose safety depends on a lock the caller is expected to hold.

**Fix**
- Inside `sagat_quota_step_log_reserve`, try the three class fences of the tenant, shared and in ADR-0027's order (`pg_try_advisory_xact_lock_shared`), before locking a lease, and return `false` when any is refused.
- The carry already holds them, so the call costs no wait and the same statement count plus at most one. After that, holding a lease always means holding the step-log fence, and the reclaim's `NOWAIT` cannot fire.
- Add to T041-5's runtime-role tests: as `sagat_app`, call reserve and hold the transaction open; from another session, change the limit; assert the change waits and then commits with the leases reclaimed, with no 55P03.

### Nit 1: The handler's `55P03` also matches lock-timeout cancellations

**Evidence**
- The handler matches `v_state <> '55P03'` (`fix2-code.diff:401-405`).
- PostgreSQL reports a `lock_timeout` cancellation with that same SQLSTATE, `lock_not_available`.

**Impact**
- If a report transaction ever runs with a `lock_timeout`, any lock wait inside the block that times out is recorded as a quota drop (`tenant_quota`, cause `busy`) instead of failing the carry.
- This is telemetry integrity, not a bypass.

**Fix**
- Have the refill catch its own `NOWAIT` failure and re-raise it under a constraint name of its own (for example `tenant_step_log_usage_busy`).
- Make the handler match on constraint names only.

### Nit 2: The new sequences are not in the explicit revoke list

**Evidence**
- `data-model.md:277-280` says the refill and the release advance the sequences `step_log_quota_refills_seq` (one per kind) and one per refusal cause.
- §4 (`:301-313`) revokes the lease table and the functions explicitly but does not mention these sequences.

**Impact**
- In a deployment whose default privileges grant sequences to the runtime role, `sagat_app` could `setval` them and falsify the operators' quota metrics.

**Fix**
- Add `REVOKE ALL ON SEQUENCE … FROM sagat_app` for each of them in the `DO $grants$` block.
- Include them in the privilege audit of `runtime_privileges.rs`.

## Suspicion (not verified; for the correctness reviewer)

The fast path assumes the planned charge equals the measured charge.
- The carry reserves a lease with room for its planned charge: `v_kept_bytes + 128 * v_kept` in the prototype (`fix2-code.diff:389-392`).
- CD's charge is measured by `sagat_quota_measure` over `message`, `exception`, `fields` and `level`, plus the marker row (`data-model.md:165-167`).
- If the built function's plan leaves out any of these, the measured charge can exceed the reserved room. CD's charge then needs another lease or a `NOWAIT` refill, outside any `EXCEPTION` block.
- A refusal or a 55P03 there would abort the report transaction, against "a reserved carry's charge cannot be refused" (`data-model.md:256`).
- What to require: a property test in T041-36 that the plan's charge equals the trigger's measurement for every carry shape (marker, fields, exception, redaction-lengthened text). Alternatively, keep the store's `StepLogRefused` mapping as a backstop.

## New surface judged with no finding

- **`sagat_quota_step_log_reserve`: no bypass and no tenant-facing oracle.**
  - It writes nothing. The charge in CD still takes its fences and re-checks S + G ≤ L itself (`data-model.md:236-241`), so a reservation cannot admit anything.
  - Its boolean answer (S + G ≤ L, and whether a lease has at least `bytes` free) reveals no more than the runtime role can already read: it has `SELECT` on every tenant's usage row (`durable_quotas.up.sql:566`). Only the lease room is finer-grained.
  - No route exposes the function. The carry passes the report's tenant, which the server derives.
- **`sagat_quota_step_log_counters()`: no finding.** It returns global sequence values and a global lease-row count, with no tenant identifier. They are published as metrics with closed labels and no tenant (`data-model.md:275-280`), as constitution VI requires.
- **Fast path and lock order: no finding.** A transaction's own reserved lease is not skipped by its charge. A reserved lease left unused is released at commit. The carry, the refill, the release and the reclaim trigger still follow fences → leases (tried only) → usage row, so no cycle appears.

## Count

- Blocker: 0
- Major: 0 (all four of my findings on 3f1f0aba closed)
- Minor: 1 (new)
- Nit: 2 (new)
- Suspicion: 1, not counted
