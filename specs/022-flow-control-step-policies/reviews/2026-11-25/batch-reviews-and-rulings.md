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

## Built (2026-11-25)

| Ruling | Change | Evidence |
|---|---|---|
| The `run` key rule | `flow_control::check_publication_keys`, called by `parse` only | `contract_flow_control_step_policies::ir::a_key_that_reads_the_runs_identity_is_refused_as_unbounded` now also restores the stored documents with `parse_stored`; red first (`parse_stored` refused them) |
| The reset's receipt and advice | the advisory check moved under the source's row, after the receipt; an error of the read is no advice | `run_creators::a_cancel_source_reset_retried_while_the_first_is_in_flight_answers_its_entry` (the first request held back writing its entry, the window filled, a retry with its key): red first with the earlier order (`SlotUnavailable`). The failing read has no test: every statement the read could be held on is read earlier by the request (the run's own read, the quota read), so no lock isolates it, and the application has no store double |
| `BINARY_CAPABILITY_MISSING` and a child | left for a replica that serves the gate; D022-080 amended | `child_starts::a_child_a_replica_cannot_serve_is_left_for_one_that_can`, red first (the node failed) |
| The body child's assertion | the code and the node asserted | `child_starts::a_child_node_inside_a_collection_body_is_refused_at_publication`: the refusal names the body `line` (`UNSUPPORTED_NODE`, kind `body`), not the child node, which is what the codec does |
| The reset's 403 and 404 before 409 | assertions in the full-window case | `run_creators::a_cancel_source_reset_on_a_full_window_is_refused_at_its_request_and_cancels_nothing`: another tenant's principal `NotFound`, a viewer `Forbidden`; green at once (the order was right; the case guards it) |
| D022-079's text | corrected; `START_WINDOW_SQL`'s unreachable branch removed | the suites unchanged |
| The Studio hint, D022-150 item 2, the codec's message | none lists `run` | `FlowControlFields.test.tsx` "says a step's key reads input, steps or state", red first |
| The deadline half | `DEADLINE_HALF_TEXT` beside a waiter table joined with `runs` | `TheEligibilityOfAWaiterIsOneText::test_the_deadline_half_written_out_beside_a_runs_join_is_refused` red first; `test_a_deadline_that_has_passed_is_not_the_eligibility` |
| One `keys_due` | `support::flow_control::keys_due`, the fault suite's margin | the suites |
| The test's text | `five_same_key_starts…` says it checks after each step | — |
