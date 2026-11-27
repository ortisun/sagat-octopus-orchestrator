# Security review of design 045, round 4 (2026-11-26)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: fd79fb39
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author; every finding and its severity kept, with the author's answer. Every security finding is fixed, as
Owner decision 2026-11-25-2 requires.

Counts: Blocker 0, Major 2, Nit 4

Round 3: Majors 1, 3, 4, 5 and 6 and Nits 2 to 8 closed; Major 2 partly closed and Nit 1 not closed (below).

## Major (each fixed)

1. **Task 045-5 still described a separate hold method.** Reworded: the store's `publish` calls the hold inside its own transaction,
   through one shared helper; no port method holds anything.
2. **The shared function's body was not in the doc, and the creator was unclear.** The body, identical to design 044's, is written in
   Technical Plan 13; the first of Tasks 044-6, 045-6 and 054-18 to merge creates it and the others compare and fail on a difference.

## Nit (each fixed)

1. The generic parameter lets the runtime role hold any requirement's row for one transaction: the bounded effect is stated and
   recorded in `docs/architecture/security-and-sandbox.md` by Task 045-1.
2. The `lock_timeout` is set by one shared store helper that every publication uses.
3. A server-side bound: a renewal of a level-1 or level-2 lease within 500 ms of its previous renewal is refused
   `HEARTBEAT_TOO_FREQUENT`, so the server carries at most two beats per second per such lease.
4. The reserved set is built from every code the domain builds with `FailureCode::system`, and a domain test lists them all.
