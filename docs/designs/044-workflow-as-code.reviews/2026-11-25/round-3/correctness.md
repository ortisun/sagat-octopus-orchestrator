# Correctness review of design 044, round 3 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 2249e90100039f5c012b6217e4e5bb454ace1296
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer judged round
2's findings 1, 3, 9 to 13, 17, 18, 20 to 23 and Nits 25, 27, 28, 31, 32 closed, and 2, 4 to 6, 14, 16, 19, 24 and Nit 30
partly closed (raised again below).

## Blocker

1. A review is stored as `WAITING` on the operator (`state_effects.rs:192-193`), and the waking rule and the reconciler
   make any waiting step `READY`, so a review is released by the next event; test only `WAITING` with kind `DECIDER`.
2. An audited no-op journals nothing, so every cold replay after the race reports `NONDETERMINISM`; journal a command
   record, count and chain it.
3. Saga dependencies are off by one: a step issued in the activation that delivered an earlier outcome depends on it;
   use journal sequences (command event after outcome event).

## Major

4. The claim leases by the node's `NodeKind::Activity`, not by `steps.activity`; a session of another digest or a
   non-sticky session can lease the workflow task; give the claim its own workflow-task arm.
5. The claim gives an activity the whole run document as input; a code step must receive its literal input; define the
   derived steps' input.
6. Twelve raw `INSERT INTO run_events` in seven files, some writing kinds as strings; one typed append and a check.
7. `is_waking` on the kind alone cannot tell a command's record from an outcome (`step.cancelled`).
8. No state for the workflow-task step after a terminal or continue-as-new command; completions during a stop drain.
9. `StopReason` has no success variant and `run.stop_requested` carries no output; the verifier reads `end.reached`.
10. Continue-as-new waiting for quiescence hangs on a losing timer or an untimed wait.
11. `same` keeps the start version, not the current one, after a move.
12. The headroom is undefined; the 56,320 maximum is not proven.
13. Steps have no not-before column, so the 100 ms rule needs a claim change or an extra pass.
14. A sticky worker with a late completion applies a window twice; the lease must carry its window start.
15. The chain has no fragment for set-state, update results, markers and terminal outputs.
16. Signals consumed on delivery are lost when unhandled at continue-as-new.
17. The move's envelope check skips completed steps of open saga scopes and their derived entries.
18. A reset of a moved run: start version, pin, copied move events; signal reapply.
19. A Python finder sees only modules not in `sys.modules`; a preloaded `uuid` passes and preloaded dependencies miss
    the digest; isolate the workflow modules' imports.
20. TypeScript and C# keep the process time zone for local-time methods; require UTC or hash it.
21. The pause reaches only sessions registered later; check it in the claim and the eager path.
22. `OUTSTANDING_OBLIGATIONS` is counted as a failure; hold the terminal command instead.
23. Changes to the workflow-task step have no event of their own; reducer rules; the move function writes outside the
    typed append; run the verifier after every scenario.
24. Engine-driven stops meet open code sagas; their cancel policy is undefined.

## Nit

25. Look the idempotency key up again under the lock.
26. The all-set-or-null check rejects normal code runs.
27. Put the exclusion in `is_derived` and `forward`.
28. `required_worker_protocol_minor` is in `infra-postgres` and per run.
29. Crash points before a statement of one transaction always leave nothing; useful points are after commit; name the
    oracles.
30. Four supersessions not marked in the register.
31. The drain waits for the workflow task's own lease.
32. A batch item's outcome commits in the move's transaction; re-applying is a no-op.
33. The eager count may leave out the acknowledgement commit.

Counts: Blocker 3, Major 21, Nit 9
