# Targeted re-read of design 046, sixth pass: security (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: 7248175e95e446acae9f2c2fd4a5d9eb72f70b8e
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A confirmation of the fixes of
`reread-4-security.md`; the reviewer read the diff of the fixes, the design and its contracts, and ran nothing.

## The previous findings

All five answered: the switch never refuses or holds on staleness; `RUNNING` jobs count and the heartbeat answers `409`;
UPDATE of `steps_failed_at` only with its trigger; `EXECUTE` revoked from `PUBLIC`; the runner principal in the audit.
The suspicion is answered for the barrier (a `SHARE` table lock waits its turn and new writers wait behind it), but the
same starvation applies to the switch (finding 1). The barrier's definer function, the column-only grant, the `409` path
and the audit actor add no tenant oracle and no privilege.

## New findings

1. Major. The switch still takes the snapshot row `FOR UPDATE` while every writer holds it `FOR SHARE` to commit; a new
   `FOR SHARE` on an already share-locked row does not wait behind a waiting `FOR UPDATE`, so an attacker with
   `script:publish` keeping author-lock publications overlapping makes the switch time out and retry forever, with only
   an alert, contradicting Decision 046-077. Fix: a lock queued in order that conflicts with the writers' first lock, for
   example writers taking `pg_advisory_xact_lock_shared` on an ecosystem key through the definer function and the switch
   `pg_advisory_xact_lock` first, keeping the writers' order; `the_switch_is_not_starved_by_overlapping_writers`. The
   PostgreSQL behaviour comes from how it handles row locks, not from a run here; the held-lock test would confirm it.

Counts: Blocker 0, Major 1, Nit 0

## The author's answer (fixed; Decision 046-078)

The snapshot lock is a transaction-level advisory lock per ecosystem: decisions (resolve completion, author-lock
publication, finish, re-evaluation page) take it shared through `sagat_lock_advisory_snapshot_share(ecosystem)`, the switch
takes it exclusively and then the row; the lock order, the data model, ADR-0121 and the test
`the_switch_is_not_starved_by_overlapping_writers` say so.
