# Correctness review of the digest-key force (T022-108, ADR-0093 decision 5; 2026-11-20)

Reviewer: `correctness-reviewer` subagent, read-only, given the working-tree diff with the spec, the research register,
the earlier migrations, `flow_control.rs`, `readiness_gate.rs`, `gate_queue.rs` and the tests (not the author's
conclusions).

## Findings

1. **Major (M1): the over-admission is up to the old row's limit for as long as it has waiters**, not the holders at the
   force: the grant pass grants the old row's waiters within its own limit (a dual-gated head refused only by its other
   gate is the example). The tests use limit 1 with a full old row, so the two readings cannot be told apart.
2. **Major (M2): after a force, an existing key can fail `FLOW_CONTROL_LIMIT`**: its only row is under the forced
   version, so it is "new", and the live count includes both rows; a namespace above half its ceiling turns waits into
   step failures.
3. **Major (M3): rate windows never drain in this build** (no lease recording or purge, G022-003): a forced rate
   over-admission is permanent, the old digests stay, and an ordinary rotation with rate rows never ends; the listing
   shows completed runs with grants.
4. **Major (M4): repeated forces give more than two rows per key**, and the runbook's emergency script commits nothing
   useful when another version is already retiring.
5. minor (m1): the live-keys texts disagree ("their number" against "minus one"); a code comment still says "their
   number"; a key judged existing can be pruned and re-created uncounted, so "no new key is created while it is" is false.
6. minor (m2): spec drift (FR-022-056, `data-model.md`, ADR-0093's evidence names, T022-095's evidence not updated).
7. minor (m3): the bound check covers semaphores only at limit 1; the wait loops have no deadline (a broken lock hangs).
8. nits: the reason padding; the actor's checks; the migration credits the insert trigger for "gains no new waiter".
- Checked with no finding: lock order (the force's `EXCLUSIVE` against the deciders' `FOR SHARE`, no deadlock);
  agreement of the in-force function, the insert trigger and the force; releases, grants and timeouts work from the
  stored digests; no row under the forced version after the force; the down migration.

## Resolution (author, 2026-11-20)

| Finding | Resolution | Evidence |
|---|---|---|
| M1 | Restated, not changed: the excess is what the old row holds at each moment, never more than its limit, until its holders and waiters drain (ADR-0093 decision 5, D022-046, runbook section 3); the order across the two rows is the stated exception to ADR-0088 decision 2 and ADR-0056 decision 5. The exact "holders at the force" reading needs a new journal transition; O1174 asks the owner, recommending this reading. The test's bound now checks each row against its own limit and that the old rows gain no waiter, for semaphores and windows | `rotation::a_forced_retirement_over_admits_a_key_by_at_most_its_old_rows_holders_until_they_drain` |
| M2 | Fixed: the readiness round counts the rows of the versions in force only (`refused_by_ceilings`), so the ceiling is passed by at most the rows a force left, until they drain, and a key filed again is decided as before | `rotation::a_key_filed_again_after_a_force_is_not_refused_by_the_live_keys_ceiling`, red before the fix (the key failed `FLOW_CONTROL_LIMIT`) |
| M3 | Documented: ADR-0093 (consequences), D022-046 and the runbook (sections 2 and 3) say rate rows drain only with T022-074 (G022-003), and that a rotation with rate rows ends only by the force | review only (T022-074 not built) |
| M4 | Fixed: `FORCED_VERSION_DRAINING`; the runbook gives the order for each case and says to read every outcome before `COMMIT` | `migrations::the_force_is_the_operators_audited_and_retires_only_a_retiring_version`, red before the fix; mutation (guard off) caught |
| m1 | Fixed: the gap row, the code comment, D022-047 and the runbook say "the new keys the others create (their number minus one when each creates one key)"; the runbook no longer claims no key is created above the ceiling by a prune and re-create race | review only |
| m2 | Fixed in the register: G1195 names FR-022-056's split clause, SC-022-028 and the new table (the approved texts change when they are next re-approved); the force is follow-up task T022-108 with its own evidence; ADR-0093's evidence names the new tests | tasks.md, research.md |
| m3 | Fixed: the bound helper checks semaphores and windows; every wait has a thirty-second failure deadline, and the lock barrier matches the statement it waits for | `rotation` tests |
| nits | Fixed (padding, actor); the migration's comment now credits the in-force function and the rule that only the gate's two files write waiters | `migrations::the_force_is_…` |
