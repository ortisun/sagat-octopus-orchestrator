# Correctness review of design 044, round 2 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 19d226028a2a387d77c637ac27b7bc0f88cf5f8a
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The first correctness
review of this design (the session's reviewer limit refused it in round 1). The reviewer read files only and checked
engine claims in `crates/`.

## Blocker

1. The engine reads every step from the published graph (`def.node_for_instance`, the claim under the run lock in
   `crates/application/src/worker.rs:1170-1184`, `declared_secrets_of`, `fail_step` returning `UnknownStep`,
   `retry_review`, `NoNodes` refusing an empty definition); a code version has no nodes, so nothing would ever be leased.
   A per-run definition overlay built from the stored fragments (with the derived companions) is needed, with its load
   cost; rewrite the "unchanged" claims.
2. A run moved off a review is left waiting: the fifth lease cleared the pending flag, the failed activation's events
   stay unanswered, and the move makes the step `READY` only "if events are pending"; and a move of a `READY` step emits
   no hint for the new name. Derive pending from the journal or leave it set on failure; a move off a review always makes
   the step `READY` with a hint.
3. An output too large to retain is a successful effect whose result is unknown (`STORAGE_QUOTA_EXCEEDED`,
   `crates/application/src/worker/output.rs:336-352`), not a known failure; telling the function it failed breaks
   principle III. Follow feature 037.

## Major

4. Events produced inside the completion itself (a `waitEvent` satisfied by look-back, a cancellation of a `READY` step)
   do not set pending; decide the step's status from what the transaction wrote.
5. A command overtaken by an event in flight (cancel a timer that fired) would go under review; make such commands
   audited no-ops when their target is terminal; barrier tests in both orders.
6. "Visible" includes the completion's own events, so read literally every completion re-arms itself; define waking
   events apart from visible events.
7. The visible count is a counter kept apart from the rows it counts; compare the journal sequence of the last visible
   event, or derive the count with one filter; a property test.
8. The protocol-minor gate is per run (`crates/infra-postgres/src/execution.rs:4253-4271`); raising it would hide the
   `c-<n>` activities from old-minor activity workers; gate per task kind.
9. Derived compensation and lookup steps inherit the forward node's secrets (`crates/domain/src/workflow.rs:2857-2869`)
   and their names are not checked against the envelope.
10. A lost answer to a completion with eager leases makes every eager step expire as an unknown outcome; an "offered, not
    started" state or re-issued tokens.
11. A move does not clear the sticky fields, and the eager workflow task checks no digest or binding.
12. Nothing wakes other workers when the sticky preference expires or the sticky session ends; a delayed hint.
13. `UNHANDLED_EVENTS` can block the end of a run indefinitely (activity events still arrive); bound it; say how
    continue-as-new and the cancellation acknowledgement are covered.
14. The closing window of 1,024 events is smaller than the 2,048 outstanding outcomes it must absorb.
15. A terminal command ends the run while leased activities may still act; use the cancelling path so leased work
    settles or goes to feature 037 first.
16. Continue-as-new while work is outstanding has no defined cause.
17. `CHAIN_MISMATCH` and `NONDETERMINISM_RISK` are often a worker's fault but allow no retry.
18. The replay check does not say where replay stops; applying pending events after the last completed start gives false
    mismatches.
19. Reset of a code run is underspecified (the cut drops the completion's command events; the chain at the cut; the
    outstanding steps; when the new run exists).
20. Nothing stores the count of consecutive failures.
21. The cancellation grace is not a timer committed in the request's transaction; no terminal check in statement 1; no
    race test.
22. An update result outside its schema has no defined outcome.
23. The bounds leave no time to read 50 pages and replay a long history within a 10 s task timeout.
24. Several planned tests cannot fail (the property test checks one function against itself), the barrier test has no
    owner, only one waking path has an integration test; the statement budget omits the reads.

## Nit

25. A column revoke does nothing against a table-level `UPDATE ON runs` grant; and run creation can insert any
    `code_version_id`.
26. The idempotency lookup must come before the fence check.
27. Keying the digests by the seed protects nothing (the seed and fragments are stored); drop the claim.
28. Hash the bytes received, after checking they are canonical.
29. A timed-out workflow task has two contradictory rules.
30. Register entries replaced later are not marked as superseded (Decisions 044-003, 044-026, 044-040).
31. Idempotent signal retries must be answered before the bounds.
32. `ctx.now()` can go backwards; take the later of the previous start's time and `clock_timestamp()`.

Counts: Blocker 3, Major 21, Nit 8
