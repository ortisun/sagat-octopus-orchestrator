# Correctness re-review of the follow-up (`295b158b..40854f4d`)

Reviewer: `correctness-reviewer` (read-only), 2026-11-20. Ran nothing. (Condensed.)

Blocker: none. **Major 1:** FR-022-022 says the namespace view MUST list keys with their oldest wait; making
`oldestWaitSince` and `oldestHolderSince` `null` for every caller without a reveal grant (`operations.rs`) leaves most
operators unable to see that a queue is stuck. Fix: amend the requirement with an owner decision, or give such callers a
coarse age (whole minutes). *Resolved after this review: the instants are cut to the minute for such callers (D022-060
revised), asserted in `rest::a_key_label_is_shown_only_to_a_caller_who_may_reveal_personal_data`.*

Fixed and verified: the cursor bound (364 ≤ 384; the longest real group is 129 bytes), sealing under the active version
read in the snapshot, the real-engine start check, the one reveal rule (the demoted-viewer test fails without the role
filter), the migration-based schema decision of `key_consistency` (constants match the migrations), the `run_creators`
wait on the database clock (a bounded condition; the delay's end really is the process clock's, outside 022's scope).

minor: m1 a key file with every switch off was parsed and wired, contrary to the record (*resolved: not read with every
switch off*); m2 the contract's "such a caller" reads backwards (*resolved*); m3 sealing under the active version and the
503 have no end-to-end test; m4 the `must(...)` checks have no test (rename `oldest_wait_at`); m5 `tasks.md` evidence of
T022-026 and T022-028 is stale; m6 the startup check is not exercised for a missing retiring version or the control
plane's path. nit: the "largest instant" comment, the `group` patterns, the loosened fixture assertion, `concat!` evades
`reveal_rule_errors`.
