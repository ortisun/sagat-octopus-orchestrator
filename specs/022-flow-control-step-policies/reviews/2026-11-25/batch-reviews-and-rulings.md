# Reviews of the batch `c72ae86e..c6cf8200` (tasks T022-075, 101, 102, 103, 106, 110, 112, 113, 114, 115), and the rulings (2026-11-25)

The repository's read-only `correctness-reviewer` and `security-reviewer` read the code at `c6cf8200`. Neither found a
Blocker or a Major. Correctness: 4 minors and nits; security: 2 minors and 2 nits. Every ruling is delegate
(OD-2026-11-24-4) and is built with a test seen failing first.

| Finding | Ruling |
|---|---|
| Correctness minor 1, security nit 1: the `run` key rule (`FLOW_CONTROL_KEY_UNBOUNDED`, `crates/infra-ir/src/flow_control.rs:222`) also runs when a stored version is parsed (`parse_stored`, `lib.rs:180-182`), against the rule that a stored version is restored as published (`lib.rs:170-175`); a version published by an earlier build would stop every pass of its runs | The check moves to the publication-only path; test: a stored document whose step key reads `run` parses with `parse_stored` and is refused by `parse` |
| Correctness minor 2, security minor 2: the advisory window check of a `CANCEL_SOURCE` reset (`pending_reset.rs:344-357`) runs before the receipt replay (`:391`) and fails the request when its read fails | The receipt is replayed first, so a duplicate request with the same key answers the first one's entry; any error of the advisory read is "no advice" and the request goes on. Tests: two requests with one key under a held source lock with the window filled between them; a failing advisory read |
| Correctness minor 3: a child start refused by one replica's configuration fails the child node permanently (`passes.rs:6731-6740`), while the grant pass leaves such a key for a replica that can serve it | `BINARY_CAPABILITY_MISSING` (this process lacks the capability) leaves the child for a replica that has it, as the grant pass does; `CAPABILITY_DISABLED` and `FLOW_CONTROL_ACTIVATION_PENDING` (an operator's switch, which the runbook says to drain before flipping) fail the node through its route, as D022-080 decided. D022-080 gains this amendment; a `child_starts` case with a store that does not serve the gate |
| Correctness minor 4: `child_starts.rs:217-224` accepts any `InvalidDefinition` | The test asserts the code and the node (`UNSUPPORTED_NODE` at the body child) |
| Security minor 1: no test pins that the reset's `409` comes after the `403` and `404` checks | In the same test, while the window is full, a principal of another tenant gets `NotFound` and a viewer gets `Forbidden`, never `SlotUnavailable` |
| Security nit 2: D022-079 says "a step's unleased grant included"; `START_WINDOW_SQL` counts run-scope rows only | The text of D022-079 is corrected (run-scope windows only; step and run windows are separate) and the unreachable branch removed |
| Correctness nit: the Studio hint (`FlowControlFields.tsx:205`), decision D022-150 item 2 and the codec's message (`flow_control.rs:216`) still say a key may read `run` | All three say what a key may read now |
| Correctness nit: the eligibility rule of `scripts/check-architecture.py:541` matches one spelling of the run half only | The rule also matches the deadline half (`deadline_at` compared with an instant beside a `runs` join), with negative fixtures |
| Correctness nit: the two `keys_due` helpers differ (`rate_lifecycle.rs:60` against `rate_limits.rs:1851`) | One helper, shared, with the margin the fault suite needs |
| Correctness nit: `five_same_key_starts…` claims "at no instant" but checks between steps | The test's text says what it measures |
