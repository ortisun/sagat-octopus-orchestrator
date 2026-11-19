# Correctness review of design 043, round 2 (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: b9b0f1b56d121ba3f977e2cea888305402f2eb47
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept, numbered across the
report. The reviewer read files only and ran nothing.

Every row commits its refusal in one transaction; every refusal path looks the key up first; transient refusals are
never remembered.

## Major

1. In `report` mode one start is counted `reported` twice (the check before `decide` and the recheck; a
   `CANCEL_SOURCE` request and its completion); a buffered trigger-now three times.
2. `row_3_with_start_is_refused_for_its_input_even_with_an_open_holder` cannot fail: the holder no longer waits for the
   signal when the refused request arrives.

## Nit

3. `A && B || C` in `one_scheduled_action_is_one_validation_in_two_spans` passes on one condition.
4. "Uses up none of its attempts" is not asserted for a trigger waiting for the validator.
5. `database_executions_total{outcome="refused"}` counts a transient refusal.
6. The `report` audit of a plain holder answer is best effort and can be lost.
7. The conformance golden normalises `message`, so it cannot show "never the value".
8. Row 8 omits `NOMINAL_TIME_FIELD_INVALID`.
9. The claim-race test's stand-in peer does not take the runs row as `claim_reservation` does.

Counts: Blocker 0, Major 2, Nit 7
