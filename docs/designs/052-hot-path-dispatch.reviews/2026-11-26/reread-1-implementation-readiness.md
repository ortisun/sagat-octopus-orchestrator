# Targeted re-read 1 of design 052: egm-implementation-readiness (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 2803afaeb9b2fd167e5afd293b481308af8e20b6
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`implementation_readiness_report.md`, not a full review.

Full round: the Blocker and Majors 1 to 5 and 7 to 9 resolved; Major 6 partly (below); Nits 2 to 8 resolved, Nit 1
partly.

## Major

1. The wake tests have no target: `crates/infra-nats/tests/` does not exist and the selector has no rule for it. Fix: a
   new target `integration_hot_path_wake` and its selector rule with a test.
2. The Redis transport has no test (every transport test is NATS-shaped). Fix: a cross-instance Redis wake test and an
   open Redis test, in `integration_redis_streams` or the new target.

## Nit

1. Test seams: a constructor for the instance identifier and a hook marking a namespace contended; the offsets at least K
   apart; round 2 while round 1's locks are held.
2. A dispatched pilot workflow must first be on the default branch; which commits are A and A′.
3. Task 052-2's counters' test is unnamed.

Counts: Blocker 0, Major 2, Nit 3

All five were fixed at the next commit.
