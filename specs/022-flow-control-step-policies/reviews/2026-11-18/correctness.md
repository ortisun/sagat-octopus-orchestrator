# Correctness review of T022-096 and T022-076 (2026-11-19)

Reviewer: `correctness-reviewer` subagent, read-only, given the diff `16b0aae8..3366aeb3` with the spec and ADRs.

## Findings

1. **Major: the gate locks can deadlock against releases and dequeues.** `decide_gates` locks its keys before `persist`;
   `persist_gate_returns` then deletes holder and waiter rows whose triggers lock key rows in event order (a stop
   dequeues the window before the semaphore). Scenario: `x (G1) → y (G2) → z (G1)` in two runs.
2. **Major: a `forEach` or loop finish makes a gated successor READY with no grant** (`primitives.rs`, `loop_progress.rs`
   insert the step row `READY` directly). G1197's text was wrong.
3. **Major: a paused run cannot persist a gated step's retry**: the planner is idle before asking the gates, `persist`
   refuses the undecided wait, and the retry pass fails on it every tick.
4. **Major (evidence): `readiness.rs` asserts `leased_at IS NOT NULL` after a poll, which no code writes**: "readiness 10
   passed" could not come from the committed tree.
5. **minor: the new `verify_run` checks have no failing test of their own** (the seeded breach matches the older
   projection check).
6. **minor: no held-lock test of two transactions deciding one key; FR-022-046 (pause returns an unleased grant) is not
   built (T022-100).**
- Checked and fine: journal-alone and gated replays agree for single, dual, queued and retried gates; `holds_all` at
  `READY` and at lease; the restore is complete; counting across periods; several signal targets; the hint guard; the key
  failure route; continue-as-new, reset rules, the test feature; the budget lines.

## Resolution (author, 2026-11-19)

| Finding | Resolution | Evidence |
|---|---|---|
| 1 | Fixed: one sorted lock pass over what the round gives back and decides, returns first; `persist`'s own returns locked sorted before their deletes | `readiness::a_release_and_a_decision_lock_their_keys_in_one_sorted_order` and `…_counted_once`, both red with the old order |
| 2 | Fixed (fail closed): both paths refuse a gated successor (`RuntimeNotIntegrated`); G1197 corrected. Not yet tested against a collection | `primitives.rs`, `loop_progress.rs`; G1197 |
| 3 | Fixed: a paused run asks the gates of what the retry pass or an operator made ready | `readiness_gate::a_retry_due_in_a_paused_run_is_decided_in_its_commit`, red without the fix |
| 4 | Real: the assertion had not been rerun after the claim-time lease recording was removed (an edit that did not apply). Corrected to `is_none` and the suite rerun | `integration_flow_control_step_policies` rerun after the fix |
| 5 | Accepted as covered in the domain (`a_step_left_ready_without_a_grant_is_a_breach_of_invariant_11`); a store-level seed that only those checks catch is left | — |
| 6 | Held-lock test added (finding 1); FR-022-046 stays with T022-100 | — |
