# Security re-read of the fixes `343bbe406..0f04b4429` (decisions D022-146 to D022-149), 2026-11-26

The repository's read-only `security-reviewer` read the fixes of the activation review against its findings
(`activation-review-security-reviewer.md`). The coordinator ruled that every security finding is fixed. The responses are
rows of `activation-review-responses.md`.

## Verdicts on the earlier findings

| Earlier finding | Verdict |
|---|---|
| Minor 1, the give-back filter | Met (D022-147) |
| Minor 2, gap G022-041 | Not met for a key whose grants have different periods: Medium 1 below |
| Nit 3, the guard's comment | Met |
| Nit 4, the key version under a share lock | Met in the code; the test does not show it at the read under the epoch lock: nit 2 below |

## Medium 1. Gap G022-041 is not closed for a key whose grants have different periods

- **Where:** `crates/infra-postgres/src/execution/readiness_gate.rs:1700` (`rate_slot_spent`) and `:1781`
  (`clear_withdrawn_grants`) judge "stopped counting" by the grant's own `period_ms`; every decision counts a grant over the
  deciding waiter's period (`readiness_gate.rs:909-914`, `:990-991`). Decision D022-009 allows mixed `per` in one group;
  publication only warns (`RATE_LIMIT_MISMATCH`).
- **Scenario:** grant G, per hour, leased at T. A per-second waiter with limit 1 on the same key is granted at T + 1.5 s
  (G no longer counts for it). At T + 2 s the host hands G back: by G's own hour it still counts, so the withdrawal is
  accepted, and G runs again: two calls within one second on a key limited to one.
- **Asked:** decide exactly, under the window lock: the slot is spent when another grant on the same key was made after
  the point where that grant's own gate stopped counting this one (`o.granted_at >= g.leased_at + o.period`), or judge
  against the shortest period the key counts with; take the exact rule if it holds under the lock; a test with two
  definitions, per hour and per second; correct "closed" in G022-041, D022-148 and the runbooks only once it holds.

## Nit 1. The privilege allowance does not name the function

- **Where:** `tests/integration/platform-foundation/runtime_privileges.rs`, the entry `ROW LOCK flow_control_key_versions`.
- **Asked:** name `ActivationPrerequisite::holds` in its reason.

## Nit 2. The share-lock test does not block where the finding was

- **Where:** `activation::the_activation_waits_for_a_change_of_the_digest_key_version_and_judges_what_it_committed`.
- **Scenario:** it blocks at the first read, not at the read under the epoch lock (`crates/infra-postgres/src/instances.rs:530`),
  and changes the row with a hand-written `UPDATE` instead of the real key command.
- **Asked:** block at the read under the epoch lock and use `flow_control_key_activate`.
