# Targeted re-read 1 of design 054: correctness (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: ca46330f11d64ee1962ac6fec6e5e26e0fe7a397
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. A targeted re-read of the
fixes of `../correctness.md` and of the text changed since (the approval rule of Owner decision 2026-11-25-2, step 4), not a full
review. The reviewer read files only and ran nothing.

Findings 1 to 5 and 9 of the correctness review: resolved (the reviewer checked that the handoffs compute sequences from the
locked run version, so the tolerance causes no collision); 6 and 7 resolved in intent, 8 and 10 in part (new Nits 2 to 5).

Counts: Blocker 0, Major 0, Nit 6

## Nit

1. The predecessor can write three settling events, numbered as two; Requirement 054-010 omits `move_limit`; `moves` has no
   check. *Answer: fixed:* one event per non-zero kind, numbered on from `run_version + 1`; the cause listed; the check added.
2. Pending bytes "by joining each settled sequence" does not fit the count-only `signal.moved` and `signal.discarded`. *Answer:
   fixed:* the reducer empties the pending set on them and checks that the counts add up; the rebuild predicate drops them.
3. Under the code-run exception pending bytes may exceed 2 MiB. *Answer: fixed:* new signals of any name are refused while the run's
   pending bytes are above 2 MiB, so the exception cannot add to it.
4. The store-side tests are pointed at Task 054-3; `assert_inbox_invariant` has no task; "after every commit" overstates it.
   *Answer: fixed:* both point to Task 054-9; the helper is Task 054-6's, a deferred constraint trigger in the test database, so
   background commits are checked too.
5. "Read the method instead" contradicts the pending-only read; `retained` against `pending`; `TakenSignal` lacks a digest.
   *Answer: fixed.*
6. The source rule over `"run.finished"` literals would catch outbox and telemetry writes. *Answer: fixed:* it covers journal
   appends; an allowlist entry may give "not a journal write" as its reason.
