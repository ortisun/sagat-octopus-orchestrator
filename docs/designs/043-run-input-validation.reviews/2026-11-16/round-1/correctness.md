# Correctness review of design 043, round 1 (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: f562614874016262dd9812e241f734038d4d70de
Reviewer: correctness (agent blob d1de651dc4a3676084974c440b0684eed2cd4888)
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Read files only.

## Major

1. A panicking compile or validation is not transient for the caller that runs it; in the engine it stops the process.
2. A `CANCEL_SOURCE` reset cancels a live source without checking its input, then ends `ADMISSION_REFUSED`.
3. Row 11: the "a peer already created the child" lookup is check-then-act; the step failure does not depend on it.
4. The conformance golden does not show `failureCode` `CONTINUATION_INPUT_INVALID`.

## Nit

5. A scheduled refusal without a pause and a backfill ended for its input write no audit row (Constitution row VI).
6. In `report` mode a recheck that reports is not counted.
7. A batch `RERUN` item refused for lack of room is a lock wait and can end `RETRIES_EXHAUSTED`.
8. A buffered backfill step refused at its release leaves the backfill running.
9. Row 10 can audit a refusal for a tick whose run exists (a reporting peer that stopped).
10. The contract and comments describe graded disclosure; row 8 can also answer `NOMINAL_TIME_FIELD_INVALID`.
11. FR-043-007's acceptance test is not built as stated.
12. Row 14 in `report`: a reported refusal is accepted without an audit.
13. All 24 tasks are unticked and none names its evidence.

Counts: Blocker 0, Major 4, Nit 9
