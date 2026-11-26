Doc: docs/designs/041-step-logs.md
Reviewed-commit: c07372fd
Reviewer: correctness
Counts: Blocker 0, Major 1, Nit 4

I found no Blocker. There is one Major, about the hand-off to T041-5, and four Nits. The claim holds: one `SELECT` gives one snapshot, so a view answers either the pre-revoke pair or "withheld". The race test's order is set by locks and `pg_stat_activity`, not by a sleep. It cannot pass vacuously, and it should still catch the main regression after T041-5.

I could not run `git`, so I read the files at their current state. I did not see the pre-change `used`. To confirm the equivalence below against the old code, run `git -C /Users/marcelo/Documents/Projects/sagat-wt-041 diff 28fe969e c07372fd -- crates/application/src/quotas.rs crates/infra-postgres/src`.

## What I checked and found sound

- **Other resources and the two kinds of view.** `used` (`crates/application/src/quotas.rs:652-658`) keeps the general rule for every resource except `TenantStepLogBytes`.
  - The step-log branch (`:640-651`) skips `has_usage()` and the scope check. That is equivalent: the resource has usage (domain test `quota.rs:746`) and is tenant-scoped.
  - In a namespace view, `list` skips tenant-scoped resources before calling `used` (`:399`).
  - After `set` with a namespace, `quota::validate` refuses `NotPerNamespace` before anything is written.
  - `clear(Some(ns), TenantStepLogBytes)` is not validated and returns the gated tenant usage. That is the same as the old tenant-scope arm and is older than this diff.
- **No membership:** no row, so withheld. **No namespace:** the `EXISTS` is false, so withheld (not tested, which D041-140 item 4 accepts). **Role parse failure:** an error that fails the whole view, as `lock_membership_roles` did before; the `memberships_role_valid` CHECK makes it unreachable.
- **Snapshot semantics.** Under read committed the statement takes its relation locks first: in Parse, or in Bind's `GetCachedPlan` when the statement is cached. `PortalStart` then takes a fresh snapshot. So a statement that waited behind `LOCK TABLE` sees the revoke's commit. The role-change trigger deletes grants in the same transaction, so leaving the membership row unlocked is safe.
- **Test ordering.** Each test gets its own database (`fresh_db` creates `t_<uuid>`), so `datname = current_database()` isolates it.
  - On the clear path, the only statement whose text contains `step_log` is the combined `SELECT`. `put_setting` takes only the advisory policy lock and touches nothing in `tenant_retained_usage`. The audit insert carries "tenant_step_log_bytes reset" as a bound value, not as query text.
  - The 10 ms sleep is only a poll interval; the lock decides the order.
  - On the new code the answer is always `(None, true)`. Both accepted answers are correct pairs, and an implementation that always withholds fails at line 809. The poll runs as `postgres`, a superuser, so it can see the runtime role's query text.

## Major

**M1. T041-5's hand-off is not written down: where the usage read now lives, and how volatile `sagat_quota_step_log_usage` must be.** Plausible; the drift itself is confirmed in the documents.

- **Where the read lives.** The design's T041-5 text (`docs/designs/041-step-logs.md:2614`) says the readers to move are `crates/infra-postgres/src/quotas.rs` `:1228-1235`. Its Files list (`:2730`) names `{identity.rs,quotas.rs,pool.rs}`. After this diff the view's read is inline in `crates/infra-postgres/src/reveal_grants.rs:91`. The `usage` arm at `quotas.rs:1236-1243` is now dead for the view.
  - Scenario: someone implementing T041-5 by its text moves the dead arm. The view then keeps reading S alone, not S + Σ `used`. That under-reports, and S can be negative under leases (`data-model.md:421`). A negative value hits `reveal_grants.rs:108-110` and makes the whole list, `set` or `clear` view return 500.
  - The only safety net is T041-5's planned check on the Rust source text, if it is built to cover `reveal_grants.rs`.
- **Volatility.** `data-model.md:355` and D041-141 say "one snapshot" but not `STABLE`. A plpgsql or SQL function is `VOLATILE` by default. Called from the combined `SELECT`, a volatile function gives its inner queries fresh snapshots. The grant (outer snapshot) and the usage (inner snapshot) can then straddle a revoke commit, and the race is back.
- **The race test would catch it.**
  1. The outer text still contains `step_log` through the function's name.
  2. The outer snapshot is taken before the commit, because the outer statement no longer locks `tenant_retained_usage`.
  3. The inner read waits on the table lock, then sees 7777, so the answer is `(Some(7777), false)`.
  - A `STABLE` function gives `(Some(0), false)` instead, which the test accepts. So the test survives the move, but the rule should not be learned from a failing test.
- **Fix:**
  - In T041-39 and D041-141, state that T041-5 replaces the subquery at `reveal_grants.rs:91` with `sagat_quota_step_log_usage(m.tenant_id)` inside the same `SELECT`, and that the function is `STABLE` (`SECURITY DEFINER`, never `VOLATILE`).
  - Add `provolatile = 's'` for that function to the privilege audit.
  - Change the `TenantStepLogBytes` arm of `usage` to return the same `StoreError::Internal` the admission-policy arms return (see N2).
  - When T041-5 lands, run a hand mutation that declares the function `VOLATILE` and check that `a_view_that_reads_after_a_revoke_commits_never_shows_the_step_log_usage` fails.

## Nits

**N1. Negative usage now fails the view instead of reading 0** (`reveal_grants.rs:108-110` against `infra-postgres/src/quotas.rs:1318`, `u64::try_from(count).unwrap_or(0)`). Confirmed in code; not reachable today.
- Scenario: `step_log_bytes = -1` gives a revealed caller a 500 for every resource in `list`. The response to a `set` or `clear` that already committed is also an error. Both are idempotent, so a retry is harmless.
- An error is arguably the honest answer to a broken invariant, but it differs from the content and log resources and is not recorded as a decision.
- Fix: keep the error and record it in D041-140 item 6, or clamp the same way as the other resources. After T041-5 the value read is S + Σ `used`, which should never be negative.

**N2. An unguarded reader of the step-log usage is left in place** (`infra-postgres/src/quotas.rs:1236-1243`). Confirmed in code.
- Any future `store.usage(t, None, TenantStepLogBytes)`, for example from a guard or a metric, skips the grant check.
- Fix: as in M1, make that arm return `Internal("read through step_log_usage_if_revealed")`.

**N3. The waiting predicate could be tighter** (`tests/integration/security-multitenancy/quotas.rs:852-856`). Plausible, future only.
- It matches any backend in the database that waits on any lock and has `step_log` in its text.
- After T041-5, the row trigger `sagat_quota_step_log_reclaim` on `quota_limits` locks the usage row. If anyone adds a `set` before the race, the `clear`'s `DELETE FROM quota_limits` would wait on the held table under text without `step_log`. The poll would then time out: a loud failure, not a vacuous pass.
- Fix: read `pg_backend_pid()` inside the revoke transaction and add `AND $revoke_pid = ANY(pg_blocking_pids(pid))` to the predicate.

**N4. Stale narrative** (test comment at `quotas.rs:774-777`; D041-141 at `research.md:152`; T041-39 at `041-step-logs.md:3594-3595`). Confirmed in the documents.
- The comment's "reads the grant before it first touches the usage table" describes the old two-statement code. The real reason to use `clear` rather than `list` is different: `list` blocks first on the content and log reads, whose text has no `step_log`.
- D041-141 and T041-39 say "the view withholds". With a `STABLE` function after T041-5 the answer becomes `(Some(0), false)`, so the text should say "either consistent pair", as the assertion already does.

## Evidence to collect (I cannot run these)

T041-39 requires the test to pass as the owner role and as the runtime role, and it is not yet ticked:

```
cargo test -p sagat-tests --test integration_security_multitenancy quotas::
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_security_multitenancy quotas::
```

Files:
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/application/src/quotas.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/src/reveal_grants.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/src/quotas.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/tests/integration/security-multitenancy/quotas.rs
- /Users/marcelo/Documents/Projects/sagat-wt-041/docs/designs/041-step-logs.md
- /Users/marcelo/Documents/Projects/sagat-wt-041/specs/041-step-logs/data-model.md
- /Users/marcelo/Documents/Projects/sagat-wt-041/specs/041-step-logs/research.md
