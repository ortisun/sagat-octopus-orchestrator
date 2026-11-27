# Re-read of the fourth-round fixes of design 045 (2026-11-26)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 1fd99ff5
Reviewer: security (re-read of every fix of the four round-4 reports)
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author; every finding and its severity kept, with the author's answer. This is the one short re-read of
Owner decision 2026-11-25-2; its findings were fixed and are not re-read again.

Counts: Blocker 0, Major 1, Nit 5

The fixes of security Majors 1 and 2 and Nit 1, the shared hold's body (types, qualification, the race in both orders), the gate row,
correctness Major 1 and architecture Majors 2, 3 and 5 hold.

## Major

1. **The renewal bound derived the previous renewal from the expiry**, which a renewal capped by the attempt deadline defeats.
   *Answer: fixed*: a seventh column, `steps.lease_renewed_at`, written by each renewal; a test with a capped expiry.

## Nit (each fixed)

1. A level-2 host's first beat would be refused: the claim does not write the column, so an attempt's first beat is never refused,
   and the host honours the retry hint. 2. The bound limits writes, not requests: said. 3. The system codes: one `const SYSTEM_CODES`
   that `FailureCode::system` checks, and a test over every literal. 4. The `lock_timeout` is set just before the hold and restored
   after. 5. The creator of the shared hold is stated the same way as design 054's (Task 054-18 planned first; whichever merges first
   creates it if absent; the others compare).
