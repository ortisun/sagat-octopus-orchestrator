# Targeted re-read of design 047, third pass: security (2026-11-26)

Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: 1d93c787153fb256a6ab7f23f2709e594466dffa
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A confirmation of the fixes of
`reread-2-security.md`; the reviewer read the design, the diff of the fix and `research.md`, and ran nothing.

## The previous findings

1. Answered: the sweeper revokes only rows of disconnecting or disconnected credentials; a stale row of a live credential
   is adopted, or abandoned and destroyed in the vault once replaced, under Technical Plan 4's guard; the test is named.
2. Answered for the common case: an unmarked row is taken only after the lease and store deadlines; the test is named.

## New findings

1. Nit (suspicion). Neither the sweeper nor the adopt step claims the write-once vault reference; each waits or probes,
   and only the provider call is measured on `CLOCK_BOOTTIME`. A holder suspended after inserting its `PENDING` row past
   every deadline, a disconnect, the sweeper closing the row with no vault entry, then a late write and a crash leave a
   live token untracked with `REVOKED` recorded; the adopt step's "absent when probed" has the same gap. Fix: claim the
   reference with a write-once tombstone before closing or abandoning a row, or measure the store deadline on
   `CLOCK_BOOTTIME`; a test with a holder suspended between the insert and the write.

Counts: Blocker 0, Major 0, Nit 1

## The author's answer (fixed; Decision 047-063)

The adopt step and the sweeper claim the reference with a check-and-set tombstone, the same kind of write as the
holder's, before abandoning or closing an unmarked row, so the late write fails;
`a_suspended_holder_cannot_write_after_its_row_was_closed`. The clock alone was rejected: a suspension between its check
and the write still escapes it.
