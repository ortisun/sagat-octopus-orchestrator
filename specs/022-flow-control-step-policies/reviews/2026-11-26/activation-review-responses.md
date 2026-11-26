# Responses to the reviews of `efabe42d..d70a60940` (2026-11-26)

The findings are in `activation-review-correctness-reviewer.md` and `activation-review-security-reviewer.md`. The
rulings are decisions D022-146 to D022-149 of `research.md`; every security finding is fixed, none accepted. Each new
test was seen failing on the code before the fix, for the reason named.

| Finding | Fix | Test |
|---|---|---|
| Correctness Major 1: the purge (and a decision's reclaim) deletes the grant of a step still `LEASED`; a later withdrawal clears nothing, and every give-back of the run fails on the missing row | D022-146: the one predicate `purgeable_grant!` keeps a step grant whose root step is `LEASED` at the grant's attempt; the purge and the reclaim hold the grant's window lock, which the withdrawal now takes before it changes the grant (D022-148), so neither judges a grant across a withdrawal in flight; a withdrawal whose grant is gone fails (`corrupt`) unless the same commit gives the attempt back | `withdrawn_leases::the_purge_keeps_the_grant_of_a_step_still_leased_and_a_pause_after_its_withdrawal_succeeds`: lease, record, purge as of the lease plus the hour plus 61 seconds (0 deleted), withdraw, pause (succeeds, `rate_limit.returned`); once the attempt ran and ended the same purge deletes the grant. Red first: the purge deleted it (1) |
| Correctness minor 1, security minor 1: the give-back lost `AND leased_at IS NULL` | D022-147: the read of a step's give-back matches `leased_at IS NULL` or an attempt this commit withdraws; the delete is strict again, because the withdrawn attempt's instant is cleared before it in the same pass | `withdrawn_leases::a_give_back_of_a_recorded_grant_whose_lease_was_not_withdrawn_is_refused`: a recorded instant on an unleased grant, then a pause: refused, nothing returned, the grant still counts. Red first: the pause gave the slot back |
| Correctness minor 2: `key_consistency` ignores the lease instant and the unleased slots' rows | D022-149 item 4: two checks, "a step grant whose lease instant is not its attempt's last lease" (empty, or the last `step.leased` of the attempt with no withdrawal after it) and "a granted, unleased step slot without its grant row" | every `assert_consistent` of the target runs them; they pass on the whole target |
| Correctness minor 3: the pause test does not test what its comment says | D022-149 item 5: its comment now says what it proves (a pause after a withdrawal gives back the cleared slot); the same-commit give-back of a recorded instant is the cancellation test's | `withdrawn_leases::a_pause_after_a_withdrawal_gives_back_the_permit_and_the_slot`, `withdrawn_leases::a_withdrawal_that_cancels_the_step_gives_back_its_slot_in_the_same_commit` |
| Correctness minor 4: failures dismissed in the evidence | Not chased here (another agent of the coordinator examines them); task T022-117 lists the tests seen failing first for D022-138, D022-139 and D022-146 to D022-149 | `tasks.md`, T022-117 |
| Correctness nit 1: the race test's barrier releases on any lock waiter | The barrier waits until a backend is blocked by the holder's backend (`pg_blocking_pids`); the same helper serves the new lock tests | `withdrawn_leases::a_lease_pass_does_not_write_a_grant_a_withdrawal_changed_after_the_pass_read_it` |
| Correctness nit 2: the down migration's comment is wrong | D022-149 item 3: the comment says that after a rollback the older lease pass records an attempt's first lease, the withdrawn one, so such a slot stops counting early | `20270217000000_flow_control_withdrawn_lease.down.sql` |
| Security minor 2: G022-041 is reachable at will by a host of protocol 1.8 | D022-148: a withdrawal of a rate-gated lease whose recorded slot has stopped counting is refused like one withdrawn as often as it may be (`capped`: a lease not sent lapses, a hand-back is the ordinary failure and the retry asks for a new slot). Checked before the command (`RunTx::withdrawal_slot_spent`) and, authoritative, when the commit clears the instant under the window's lock taken in the commit's sorted pass, which every decision that counts the window holds. G022-041 is closed; its cost is G022-042 (accepted). Gap, runbooks and release notes corrected | `withdrawn_leases::a_withdrawal_whose_slot_has_stopped_counting_is_refused` (a lease not sent: `Capped`, nothing changed; a hand-back of protocol 1.8: an ordinary failure, no withdrawal journaled). Red first: `Withdrawn`. `withdrawn_leases::a_withdrawal_whose_slot_stops_counting_while_it_waits_for_the_window_is_refused_under_the_lock` (the check before the command passes; the lease is recorded and its second passes while the commit waits for the window; the commit refuses: `Error`, nothing changed). Red first: the commit never waited for the window |
| Security nit 3: the guard lets the runtime role rewrite an instant | D022-149 item 2: the up migration's comment says the guard is a defence against bugs; the role already holds `DELETE` | `20270217000000_flow_control_withdrawn_lease.up.sql` |
| Security nit 4: the key version read does not serialize with key-version commands | D022-149 item 1: `ActivationPrerequisite::holds` reads the active version `FOR SHARE` | `activation::the_activation_waits_for_a_change_of_the_digest_key_version_and_judges_what_it_committed`: the activation is blocked by the transaction that changes the version row, and refused once it commits with no version active. Red first: the activation never waited |

Two existing tests followed the fixes, after their failure was read:

- `rate_gate::the_purge_keeps_a_short_grant_for_the_longest_period_its_window_has_seen` leased a step and purged its grant
  while the step was still `LEASED`, which D022-146 now forbids (the purge returned 0 where it expected 1). The test ends
  the attempt before the purges; what it measures (the window's longest period, not the grant's own) is unchanged.
- `withdrawn_leases::a_withdrawal_whose_slot_has_stopped_counting_is_refused` moves the grant's instants by hand to age
  it, which the new lease-instant check of `key_consistency` reports, rightly. The test checks the run's journal with
  `verify_run` only.

The privilege audit (`integration_platform_foundation::runtime_privileges`) found the new `FOR SHARE` read of
`flow_control_key_versions` outside the runtime role's privileges; it is operator only (the activation runs as the schema
owner), so it is a reasoned entry of `ALLOWED`, beside the activation functions.

## Evidence (2026-11-26, at the head of this branch)

- Red first, before the fixes: the five new tests named above failed for the reasons given (`withdrawn_leases`
  10 passed, 5 failed with `activation`).
- After them: `cargo test -p sagat-control-plane --locked` all passed; `integration_flow_control_step_policies` whole
  target 260 passed, 0 failed as the owner and 260 passed, 0 failed with `SAGAT_TEST_AS_RUNTIME_ROLE=1` (this run the
  batch-rerun and lock-order tests of the earlier evidence also passed; another agent examines them);
  `integration_platform_foundation -- runtime_privileges migrations` 70 passed; `sagat-infra-postgres` and
  `sagat-application` tests passed; `cargo clippy --workspace --all-targets --locked -- -D warnings` exit 0;
  `cargo fmt --check` clean.
