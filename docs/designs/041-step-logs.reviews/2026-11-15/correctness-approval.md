# Correctness review of design 041, the approval run

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 882e56fcd513fa4d0de4205b95532113522f223a
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a fresh `correctness-reviewer`, given the committed design, its companions, the ADRs, the code and its own `correctness-final.md`; no shell. Recorded condensed.

All findings of `correctness-final.md` answered; end rule, lock order and counting verified sound.

## Blocker

None.

## Major

1. The SC-041-001 barrier test describes a state PostgreSQL cannot reach: with K ingests each running `SET CONSTRAINTS ALL IMMEDIATE`, the second and third block on the first's usage-row lock, so only one is "held after its flush". *Fix:* hold one after the flush, let the other K-1 queue with a raised lock timeout and be seen in `pg_locks`, then start the completion and assert the commit order.

## Nit

2. Which `bytes` the stream holds is ambiguous (step 8 returns "charged sizes"; data-model says text bytes); following steps 8-9 literally brings back the 128-byte ceiling problem.
3. "No `SECURITY DEFINER`" (design 671-672, 1097) against the two accessors.
4. SC-041-003 does not cover a stream removed (expired or deleted) while its lease lives.

Counts: Blocker 0, Major 1, Nit 3
