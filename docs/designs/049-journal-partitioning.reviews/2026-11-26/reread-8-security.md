# Targeted re-read 8 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: a033fc72e30f152929239e2be0d63c4d810a9991
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fixes of re-read 7, not a full review. The reviewer read files only and ran nothing.

Re-read 7's two Nits are resolved: the ended-month branch refuses any isolation but `READ COMMITTED` and fails closed
with a message that carries no value; the static check counts every literal naming a journal table outside its owning
file and refuses assembled SQL. Checked with no finding: a literal naming a partition passes the static check, but the
runtime role has no grant on any partition, so it cannot read one by name.

No Blocker, no Major.

## Nit

1. The isolation refusals are tested only at `REPEATABLE READ`; a check written as `= 'repeatable read'` would pass and
   let `SERIALIZABLE`, with the same fixed snapshot, through. Fix: a `SERIALIZABLE` case in both refusal tests.

Counts: Blocker 0, Major 0, Nit 1

Fixed at `1f07592a`: both tests of Task 049-14 run at `REPEATABLE READ` and at `SERIALIZABLE`.
