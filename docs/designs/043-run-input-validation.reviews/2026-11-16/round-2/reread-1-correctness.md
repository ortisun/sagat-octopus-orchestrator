# Targeted re-read 1 of design 043: correctness (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: a763d9d01d36acea3bb3cad1e6e861d7c4f5ce74
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the
changes since b9b0f1b5 (OD-2026-11-16-3), not a full review. The reviewer read files only and ran nothing.

Round-2 Major 2: fixed. Round-2 Major 1: partly fixed (finding 2).

## Major

1. A pre-check with no permit gets the wrong answer on three paths: the dry run answers 200 with a `VALIDATOR_BUSY`
   problem (`definitions.rs` validate), a publication replay answers 500 (`publishing.rs`), a promotion reads the busy
   parse of a clean draft as not clean and answers 409 `DRAFT_HAS_CHANGES`. No test sends `ValidatorBusy` through a caller.
2. An accepted action that was buffered is counted twice (`check_before_its_start` at the decision and at the release);
   section 7 still says a recheck counts only a refusal.
3. The inventory ignores schedules that pin a version: a quarterly or paused pinned schedule makes its version superseded.
4. The pin test leaves out `MAX_POINTER_BYTES`, `MAX_REPETITION`, `MAX_PATTERN_NESTING`; the pointer cap is not in the
   subset contract.
5. The unit test of the pre-check bound can fail when the new pool test runs beside it (the static counter is raised by
   any pool).

## Nit

6. Row 3: `event_type LIKE 'signal%'` matches no journal event; count `wait.satisfied`.
7. Two test doc comments are swapped in `run_input.rs`.
8. `AtStart` and `Ahead` never count `deadline` or `interrupted`.
9. `acquire_blocking` has no test of the race between its timeout and a late grant.
10. The inventory test has no finished run created within 30 days.
11. The row 13 sentinel never checks that the log capture saw anything.
12. D043-175 says every other trigger update names the tenant; the advance and delay updates do not.

Counts: Blocker 0, Major 5, Nit 7
