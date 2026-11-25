# Security re-review: task T022-078 (the rate gate's reads, after the fixes)

Reviewer: `security-reviewer` (read-only, diff and spec only), 2026-11-20, over the uncommitted fixes on top of 5b2e6de4
on `feature/022-throughput-control`. Text as returned; resolutions in `rate-reads-responses.md`. The diff handed to the
reviewers was taken while a hand mutation was applied to the working tree (see the responses, row one).

I found no Blocker in the security classes. The fixes for the earlier review's Major 1 and its Minors 2 and 3 are in the code, and I checked each one. There is one Major: the working tree does not match the diff I was given, and in both versions the `ahead` count is missing a filter. There is also one Minor (the new cost test can't catch the regression it is meant to pin), one Minor suspicion and two Nits. I ran nothing; everything here comes from reading the code.

## Major

**1. The `ahead` filter of `RATE_PLACE_SQL` is missing a condition in both the diff and the working tree, and the two disagree.** Confidence: high on what the text says, medium on the cause.
- **Where:** `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/src/flow_control.rs:1009`.
- **The base** (the "-" side of the diff) has `AND x.deadline_at > me.t AND r.status IN ('RUNNING', 'WAITING') AND NOT r.paused`.
- **The diff** changes it to `AND r.status IN ('RUNNING', 'WAITING') AND NOT r.paused`, which drops the deadline filter.
- **The working tree** now reads `AND x.deadline_at > me.t AND NOT r.paused`, which drops the status filter.
- **Why it matters:** the doc comment above it, the plan, D022-067 item 2 and the contract all say "not past their deadline, their run running or waiting and not paused". The test `the_waiters_the_pass_skips_are_not_ahead_and_have_no_estimate` expects position 1 for run D. Either version gives 2.
- **Likely cause:** each looks like a hand mutation in progress, captured at two different moments.
- **Security impact:** none across tenants, because every term is tied to `me.tenant_id` and `me.namespace_id`. The position is wrong, though.
- **Scenario (tree as it is now):** a cancelled run whose waiter row is left behind counts as ahead. Every reader of the run behind it sees position 2 instead of 1, and its estimate is pushed one counted lease later or withheld.
- **Fix:** put both conditions back exactly as on the base line. Then confirm with `git -C /Users/marcelo/Documents/Projects/sagat-wt-022 diff -- crates/infra-postgres/src/flow_control.rs`: the diff should show no change to that line except the `me` join. Then run `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_flow_control_step_policies rate_rest:: -- --nocapture`.

## Minor

**1. The cost test cannot detect the scan of aged grants it is meant to prevent.** Confidence: high.
- **Where:** `/Users/marcelo/Documents/Projects/sagat-wt-022/tests/integration/flow-control-step-policies/rate_rest.rs:910` and `:953`.
- **What it checks:** each node's `Actual Rows`, the rows a node returns. That count leaves out rows removed by a filter.
- **Scenario:** put back the old, non-indexable predicate `g.leased_at + make_interval(...) > now()`. The grant scan then walks all 600 aged grants as a filter (`Rows Removed by Filter: 600`) and still returns one row, so `read <= 1` passes.
  - The 256-waiter bound is pinned, because the old `DISTINCT` returned 600 rows.
  - The grant half of the earlier Minor 2 is not pinned.
- **Fix:** run `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)`, and on every `rate_limit_grants` and `rate_limit_waiters` node assert that `node["Rows Removed by Filter"]` is absent or 0. Asserting that the grant node's `Index Cond` names `leased_at` would also work.
- **Confirm:** apply that hand mutation and check that the test now fails.

**2. Suspicion, not verified: the `counted` subquery of the place read may still scan the purge backlog.**
- **Where:** `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/src/flow_control.rs:1002`. This code was not changed in this diff.
- **Why:** `(g.leased_at IS NULL OR g.leased_at > me.t - ...)` is an OR. The planner may run it as a filter over every grant of the key, aged ones included, rather than as two index ranges.
- **Impact:** at run scope `leased_at` is never NULL, so the `IS NULL` branch is dead. The new 2 s timeout now bounds the cost, and a failure only hides the place, so this is cost, not exposure.
- **Fix:** at run scope, use just `g.leased_at > me.t - make_interval(...)`.
- **Confirm:** run `EXPLAIN (ANALYZE, BUFFERS)` of `RATE_PLACE_SQL` on a run key seeded with aged grants, the same way the listing's test is seeded.

## Nits

1. **The new warning logs the error text, not just the kind.** `/Users/marcelo/Documents/Projects/sagat-wt-022/crates/application/src/runs.rs:2403-2407` logs `error = %error`, the full message of the underlying database error. D022-068 item 2 says "the error's kind".
   - Nothing can leak today: the statement binds only the tenant and run identifiers and reads no digest or value, and the parsing failures (`corrupt`) are fixed texts.
   - The same text is already logged by `record` (`identity.rs:756`), so each failure is logged twice.
   - If you want the log to match the decision, map the error with `store_error_without_text` (the error code only) or log a fixed kind.
2. **Each place read now takes five round trips instead of one** (begin, two settings, the query, commit). That is performance, not exposure. Merging `SET TRANSACTION READ ONLY` and the timeout into one statement would save one.

## Classes checked with no finding

- **Tenant boundary:**
  - The new join in `RATE_PLACE_SQL` ties the run to `w.tenant_id`, and `w` is filtered on `$1`. If the run row is missing the join returns nothing, so no place is shown.
  - The new `nextSlotAt` subqueries (the 256-waiter slice and the grant probe) tie every term to `w.tenant_id`, `w.namespace_id` and the rest of the key. `w` itself is filtered on `$1` and `$2`.
  - The place read only runs after `store.get(tenant, id)` has found the run, so another tenant's run is still the audited 404, with the same body and timing class as before.
- **Estimate withheld:** `runs.rs:2376` sets `estimated_grant_at = None` whenever `slot.label()` is none. The label is set only by `reveal_permit_labels`, and that check fails closed: a storage error fails the whole read rather than revealing. `get` is the only caller of `slot.place`, so `listRuns` never gets a place. Position is still shown to every reader; D022-067 item 8 accepts that under G022-014.
- **Minute truncation:** unchanged. `operations.rs:776-777` cuts `oldestWaitSince` and `nextSlotAt` to the minute for callers without the reveal grant, after the read and before anything is returned.
- **Resource exhaustion:**
  - A page now costs at most 201 keys × (256 waiter rows + at most 3 single-row grant probes + the key's counted grants).
  - The counted-grants part is still up to the 10,000 hard ceiling a key. The plan accepts this, and the 2 s timeout bounds it.
  - The place read has its own 2 s statement timeout, which also covers lock waits; the test with held tables shows that.
  - `getRun` takes its connections one after another (the run, the reveal check, the place) and never holds two at once. The transaction is rolled back when it is dropped on an error path, and `begin_safely` handles a cancelled begin.
- **Runtime role:** `SET TRANSACTION READ ONLY` and `SET LOCAL statement_timeout` need no privilege. `sagat_app` has table-level SELECT on `rate_limit_waiters`, `rate_limit_grants` and `rate_limit_windows` (`20270130000100_flow_control_rate_limits.up.sql:249-255`), and on `runs`. There is no row lock and no write.
- **Injection and parsing:** every value is bound. `LIMIT 256` is a literal tied to `GRANTS_PER_KEY_PER_PASS` by a check at build time. There is no new input, regular expression, JSON or outbound call.
- **Races:** a run granted between its row and its place gets no place, which the contract states. The reveal check is read without a lock, as already accepted for labels.
- **Test-only constructs in production:**
  - The new `#[doc(hidden)] pub use` of the two SQL constants exposes only fixed text.
  - The direct SQL writes, the held locks and `ALTER TABLE ... DISABLE TRIGGER` exist only in the tests.
  - There is no new environment switch or test-only branch in production code.
- **Key values in bodies and logs:** `routes/rate_limits.rs` is now under `key_value_free_errors`, with a test showing it is checked. The digest prefix in the consistency check is test output only.

Files referenced:
- /Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/src/flow_control.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022/crates/application/src/runs.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022/crates/application/src/operations.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022/tests/integration/flow-control-step-policies/rate_rest.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/src/identity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022/crates/infra-postgres/migrations/20270130000100_flow_control_rate_limits.up.sql
