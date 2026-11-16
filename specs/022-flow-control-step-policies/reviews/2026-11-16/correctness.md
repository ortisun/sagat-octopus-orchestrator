# Review: feature 022, 0.3.0 slice, second build — correctness

- **Change:** `feature/022-throughput-control`, commits `fe8b2d57..162ec3aa` (T022-002, T022-013, T022-069, T022-095)
- **Reviewer:** `correctness-reviewer` (independent read-only agent, given the changed files and the spec), 2026-11-16
- **Recorded by:** the author, from the reviewer's report. No Blocker; 2 Major, 6 minor, 2 nits. Each fix has a test seen
  failing under a hand mutation of the fix.

| Finding | Severity | Answer |
|---|---|---|
| F1 — a retiring row that vanishes between resolution and lock is filed under the active digest at the old digest's position, breaking the one lock order (deadlock with a concurrent filer) | Major | Fixed: `file_keys` refuses with the retryable `KEY_ROW_VANISHED`; the retry resolves the key to its active digest. Test `migrations::a_retiring_row_that_goes_away_while_it_is_filed_is_a_retryable_refusal` (a held release, the decider blocked on it via `pg_blocking_pids`, then the refusal and the retry); red when the vanished row is filed anyway |
| F2 — the retire trigger serializes every key deletion of a rotation on one row and counts the retiring rows on each | Major (plausible) | Fixed by design change (D022-043): retirement is lazy, no trigger; the in-force function answers NULL for a retiring version no row uses (two index probes). Test: `a_rotation_runs_with_live_rows_and_retires_the_old_version_with_its_last_row` (out of force with its last row, RETIRED by the next activation); red with the version kept in force while unused |
| F3 — stale RETIRING rows after the upgrade can split a key | minor | Fixed with security S2 (unique index, migration retires them); test named there |
| F4 — spec, data model and T022-007's evidence still describe the drain-first rotation | minor | G1195 (approved texts change with the owner's answer to O1170 / ADR-0093). T022-007's evidence names a test that was rewritten as `a_rotation_runs_with_live_rows_and_retires_the_old_version_with_its_last_row`; its block is history of 2026-11-15 and is not edited |
| F5 — "run waiters per key always below the effective `queued_runs`" is not enforced | minor | Fixed: `FlowControlCeilings::effective_key_waiters`. Test `one_key_never_holds_every_waiting_run_of_the_namespace` |
| F6 — a retried publication is refused when the ceiling was lowered; a failed warning read fails the publication | minor | Fixed: the node-declarations refusal goes through the replay (as D042-105); a failed read gives no warning (logged). Test `publication::a_publication_retried_after_the_ceiling_was_lowered_is_replayed`; red without the routing |
| F7a — the vanished-row path is untested | minor | Fixed with F1 |
| F7b — "two transactions deleting the last two rows cannot both miss the retirement" is untested | minor | Moot: the trigger is gone (F2); lazy retirement has no such race (no row can be created under a retiring version) |
| F7c — the rotation migration's down guard is untested | minor | Fixed: `the_rotation_migration_retires_what_the_old_command_left_and_its_down_refuses_during_a_rotation` |
| F7d — the active-version join, a same-tenant namespace and the rate-limit kind of the node ceiling are untested | minor | Fixed: `publication::the_warning_reads_this_namespace_and_the_active_versions_only`, `publication::the_rate_limit_declarations_and_the_default_ceilings_are_bounded_too` |
| F7e — the bound tests prove `maximum()` only, never the default ceiling without a grant | minor | Fixed: the default ceiling accepted and one more refused without an operator's grant, in the rate-limit test above |
| F7f — the contract test's check that a namespace warning is proved passes on the helper's literal | nit | Fixed: it now looks for the code inside the named test's own body |
| F8 — `contracts/errors.md` and a doc comment disagree with the code used for the node-declarations ceiling | minor | The doc comment is fixed; the contract row is G1195 |
| F9 — `admits_waiter` would refuse a run already counted at its other gate | nit | Fixed: `already_queued`; asserted in `a_waiter_and_a_key_are_admitted_below_their_ceilings_and_refused_at_them` |
| F10 — byte order vs SQL collation of group names | nit | Recorded in D022-043 |
