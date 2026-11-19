# Correctness confirmation check of design 041 (third re-read)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: f9d4a65fa278e1c782a59e3d15adadd6e764371a
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a fresh `correctness-reviewer`, asked to confirm the fix of the Major of `correctness-reread-2.md`; no shell. Recorded condensed. With `correctness-reread.md` and `correctness-reread-2.md`, this closes the targeted re-read of the owner's approval path (OD-2026-11-16-7).

The previous Major is fixed (the marker's timeout a store parameter, the two T041-13 tests that hold a lock named and given the long timeout, data-model.md §9 alike); the previous Nit is fixed.

## Blocker

None.

## Major

None.

## Nit

1. The forced-failure test's trigger fires before the marker reaches the tenant row, so "also while the tenant usage row is held" adds nothing; the lock-timeout branch needs a separate test without the trigger.
2. The barrier test's ingest keeps its fixed 200 ms lock timeout; "all commit" should include its resend after `RETRY_LATER`, or the test should set the ingest's timeout too.

Counts: Blocker 0, Major 0, Nit 2
