# Architecture review of design 045, round 4 (2026-11-26)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 99b551c1
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author; every finding and its severity kept, with the author's answer.

Counts: Blocker 1, Major 5, Nit 7

Round 3: Majors 3, 4, 8 and 9 closed; Majors 1, 2, 5, 6 and 7 and Nit 1 not fully closed (below).

## Blocker

1. **The gate row could not open the gate** (`Decided by` was plain `delegate`). *Answer: fixed*: Owner decision 2026-11-26-560 is
   `delegate (OD-2026-11-24-4)`, with its reasoning, under a 2026-11-26 heading; `check-design-docs.py` passes.

## Major (each fixed)

1. Two refinements of Architecture decision 0098 (decision 3, the kept `ready_at`, now only at levels 1 and 2; decision 8, the
   replay-floor rule) are listed in Architecture decision 0110 and get a pointer in Task 045-1.
2. The store-side release paths call one pure domain function, `StepTimeouts::release_deadlines(db_now)`, and only bind its output.
3. The static check bans interval or clock arithmetic on the deadline columns outside a named allowlist, with passing and failing
   fixtures; Architecture decision 0110's enforcement says the same.
4. The shared hold function's life cycle: compare or create, fail on a difference, never dropped; the stale clause is gone.
5. Design 044's workflow-task step is excluded from every deadline, with a test; Gaps 044-020 and 044-026 are registered as answered.

## Nit (each fixed)

1. The lease predicate takes the node and compares host features; one function maps a feature to a minor. 2. The register's
superseded entries point to the current text. 3. Architecture decision 0110 decision 1 matches its consequences. 4.
`DOCUMENT_NEEDS_REPUBLISH` is an application refusal. 5. The new closed values of the existing metric families are named. 6. The
existing capability-list check is extended, not duplicated. 7. The shared refusal fixture has a path, read by Tasks 045-2 and 045-14.
