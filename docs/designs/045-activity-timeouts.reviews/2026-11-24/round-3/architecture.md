# Architecture review of design 045, round 3 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 7675358f
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author; every finding and its severity kept, with the author's answer (applied in the fourth revision). The
reviewer read the working tree at `e5219dfe`, which differs from 7675358f only in `docs/owner-decisions.md`.

Counts: Blocker 0, Major 9, Nit 16

Round 2: Majors 1, 3, 9 and 10 closed; Majors 2 and 4 partly; Major 5 not soundly answered; Nits 7 and 15 not fully closed.

## Major

1. **The `Owner-gate:` answer is not sound**: the cited row cannot open a gate, the design still says "none open", and other tasks
   build into feature 006 without a gate. *Answer: accepted*: "Owner decisions needed" lists one open item for the coordinating
   session (an owner row whose `Covers` names Tasks 045-1, 045-7, 045-10, 045-13, 045-16 and 045-19), and the six carry the gate line.
2. **The domain sees a protocol minor written outside the events.** *Answer: accepted*: `StepState` has no protocol number; the domain
   predicate reads the node's required host features against the session's; the column is an adapter-only hint, covered by the parity
   test built through the real release statements.
3. **No single read path for "a level-0 version is read without the fields".** *Answer: accepted* (security round 3, Major 5).
4. **A second mechanism for a rule Architecture decision 0098 already solved.** *Answer: accepted*: the hold follows
   `lease_withdrawal_required()` (the requirement's row `FOR SHARE`, both kinds checked) and the floor is a rule of 0098's replay-floor
   trigger with its `READ COMMITTED` check; the misquotation of 0082 is gone.
5. **Three branches replace the same registry objects.** *Answer: accepted*: no operator function is replaced; the migration is
   timestamped after the latest migration on `develop` and on the branches it composes with at merge, restates the capability lists
   in force, and a platform-foundation test asserts every feature's capability and refusal after all migrations.
6. **Leases capped by the close deadline near the cutoff are never provisional.** *Answer: accepted as a stated consequence*: an attempt
   leased less than 18 s before its close deadline is not provisional; Architecture decision 0110 and Gap 045-005 say so, and the
   text no longer claims every lease can be provisional.
7. **The check on the attempt deadline cannot work as written.** *Answer: accepted*: the domain computes the attempt deadline at every
   level (the same value at level 0), carried by `LeaseGrant`, journaled only at levels 1 and 2; the check forbids interval arithmetic
   assigned to the deadline columns outside the named resume statement.
8. **Only the overlay rule is shared between the server, six SDKs and the Studio.** *Answer: accepted*: one fixture of every refusal
   and reason under `tests/contract/sdk-conformance/`, read by the domain tests and each SDK; a domain test that the reserved set
   contains `outcome_unknown_codes()`.
9. **The gate contract with feature 022 is one-sided.** *Answer: accepted*: Task 045-10 owns the gate work in 022's crates once 022
   lands (no counterpart task is assumed), `step.failed` from a gate's `WAITING` is listed among the new transitions, and the pointer
   in Architecture decision 0089 says "close cutoff".

## Nit (each accepted)

1. The register's superseded entries are marked. 2. "At or after the cutoff is final" everywhere; Architecture decision 0110's
decisions 2 and 6 aligned. 3. The misquotation is gone. 4. The skips reuse `worker_claim_skipped_total{reason}`. 5. The protocol
capability is listed when the negotiated ceiling reaches the minor, with a unit test. 6. The overlay rule belongs to the domain.
7. A partial index on `workflow_versions (activity_timeouts_level) WHERE activity_timeouts_level > 0`. 8. The reserved list is "codes
decided before row 3 or reported only by the platform". 9. The inventory is a task. 10. The 30,000 ms threshold is a named constant.
11. The two capabilities are entries of `control-plane capability-activate`. 12. The pause at 10,000 path rows is measured. 13. The
enforcing tests are named in Architecture decision 0110. 14. Architecture decision 0145 is cited. 15. A step whose cutoff is at or
before `db_now` is ended before it is enqueued at a gate. 16. A publication's hold that hits its `lock_timeout` answers `UNAVAILABLE`
with the reason `REQUIREMENT_BUSY`.
