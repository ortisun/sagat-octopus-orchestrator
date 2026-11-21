# Review: feature 022, 0.3.0 slice foundation — correctness

- **Change:** `feature/022-throughput-control`, commits `4d044013..b5d071b3`
- **Reviewer:** `correctness-reviewer` (independent read-only agent, given the changed files and the spec), 2026-11-15
- **Recorded by:** the author, from the reviewer's report.

None of the findings was reachable in this build (every gated definition is refused at admission, D022-037); all are in
what T022-022 and T022-096 build on.

| Finding | Severity | Answer |
|---|---|---|
| M1 — a step (or a run's start) became READY when the other gate of the same attempt had timed out or been dequeued | Major | Fixed in `Gates::apply`: a wait one gate gave up on is not granted or queued by the other. Test `a_grant_after_the_other_gate_gave_up_is_refused` (red first) |
| M2 — a permit could be released while its attempt still ran, and an unleased slot returned while the step was still READY; existing tests treated both as legal | Major | Fixed in `apply_gate`: a permit is given back once its attempt ended or the step left READY unleased, a slot only in the second case; `StepLeased` refuses an attempt whose gate is not held. The planner now gives back what ended attempts hold (`give_back_ended_gates`). The two tests were corrected; new test `a_permit_or_slot_is_given_back_only_when_its_attempt_no_longer_needs_it` (red first) |
| M3 — a new attempt's grant overwrote a permit the last attempt still held; a run could finish with a step's permit held | Major | Fixed: a new attempt does not decide while the last one holds a permit; `check_finish` refuses while any step gate waits or holds a permit. Test `a_held_permit_is_neither_overwritten_nor_left_behind_at_the_end` (red first) |
| m4 — the successor guard checked run-level gates only | Minor | Fixed (see the security review) |
| m5 — restoring a run from its projection drops its gates | Minor | Left, recorded G1188: the projection of gate state is written with T022-022/T022-096; no stored run can have gates in this build (admission refuses them, and a gated step's wait is refused by the projection writer) |
| m6 — the counter triggers deleted an emptied key immediately: a release then a grant of the same key in one transaction failed; delete-under-KEY-SHARE can deadlock | Minor | Fixed the ordering trap: the empty row is deleted by a `DEFERRABLE INITIALLY DEFERRED` constraint trigger at commit. Test `a_key_emptied_and_filled_in_one_transaction_keeps_its_row` (red first: FK violation). The deadlock needs a writer that skips the key-row lock; the lock order every later writer must keep is recorded as D022-038 |
| m7 — `grants_total` restarts when a window row is deleted, so invariant 9 cannot hold across a purge | Minor | Recorded D022-039: `grants_total` counts one life of the window row; T022-079 checks invariant 9 per life |
| m8 — the down migrations' refusal on a journal event is untested; `start_run_gated` did not check its decisions against the definition | Minor | `start_run_gated` fixed (each declared run-level gate decided once; test `an_admission_decides_exactly_the_gates_the_definition_declares`, red first). The journal-event refusal of the downs is left untested (G1191) |

Checked with no finding: the codec's closed detail, the reset exclusion, the combination rule at publication only, the runtime role's grants.

Counts: B0 M3 m5 — all fixed or recorded (G1188, G1191, D022-038, D022-039).
