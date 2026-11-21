# Resolution of the review of the quota view's one snapshot (2026-11-21)

The `security-reviewer` and the `correctness-reviewer` read commits `9eb1ea52` (documents) and `c07372fd` (code), the
change of decision D041-141 (design 041, follow-up task T041-39), against the design; their reports are `security.md`
and `correctness.md`, verbatim. No Blocker; one Major (correctness) and seven Nits. Each is answered in decision
D041-141 of `specs/041-step-logs/research.md`, documents first:

| Finding | Answer | Test |
|---|---|---|
| correctness Major 1, the hand-off to task T041-5 (where the read lives, the volatility of `sagat_quota_step_log_usage`) | D041-141 (a), T041-39 and data-model.md §3b: T041-5 calls `sagat_quota_step_log_usage(m.tenant_id)` inside the one `SELECT` of `reveal_grants.rs`, the function `STABLE`, checked by the privilege audit and by a hand mutation to `VOLATILE` | the race test, at T041-5 |
| correctness Nit 2 and security Nit 2, the general usage read of `tenant_step_log_bytes` left unguarded | D041-141 (b): `QuotaStore::usage` refuses it with `StoreError::Internal` | `the_store_refuses_the_step_log_usage_without_the_grant` (seen failing: it read 0) |
| correctness Nit 1, a negative usage fails the view | D041-141 (c): kept, fail-closed, as D041-140 item 6 | none (unreachable today: nothing writes the usage column before T041-5; from T041-5 the view reads S + Σ `used`, which ADR-0097's invariants keep at 0 or more, while S alone may go below 0) |
| correctness Nit 3, the waiting predicate | the poll requires the revoke's backend among `pg_blocking_pids` of the waiter | the race test |
| correctness Nit 4 and security Nit 1, the narrative says "withholds" where the test accepts either consistent pair | D041-141 and T041-39 now say "the grant with the usage of the same snapshot, or no usage"; the test keeps both pairs, since a `STABLE` function after T041-5 answers the first; the test comment gives the real reason for the `clear` | the race test |
| security Nit 3, the trigger named as if a test | the comment says "the trigger" | none |

Hand mutations, each failing and restored with a fresh build: the grant flag ignored (the grant-in-every-namespace
test and the race test fail); the membership role ignored (the grant-in-every-namespace test fails); the read split
back into two statements (the race test fails with `(Some(7777), false)`, under the tightened predicate).

The correctness reviewer re-read the fixes at `7d34e23b` (`correctness-rereview.md`): the Major and the Nits resolved,
no new Blocker or Major. Its one new Nit (task T041-5's approved text still names `quotas.rs:1228-1235`, has no
volatility check and no `reveal_grants.rs` in its files) is answered in T041-39, since an approved task's text cannot
change: T041-39 states that it replaces that sentence, adds the file and adds the `provolatile = 's'` check to T041-5.
