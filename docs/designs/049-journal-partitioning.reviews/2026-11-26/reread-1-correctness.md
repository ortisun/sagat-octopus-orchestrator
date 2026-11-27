# Targeted re-read 1 of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 4060f927e65a4fe70c1af8c1f893a88b8e84bab8
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the
fixes of the full round (`correctness-full.md`, at 66634fb9), not a full review (Owner decision 2026-11-25-2). The
reviewer read files only and ran nothing.

Full-round findings 1 to 20: all resolved (3, 6, 7 and 17 with the new findings below).

## Major

A. A plain `DETACH PARTITION` takes `ACCESS EXCLUSIVE` on the parent; the drop transaction holds both parents while it
   waits for the second and then for each tenant's usage row, so the bound of Success criterion 049-002 is wrong, and a
   completion writing `run_payloads` then `run_events` can deadlock with it. Fix: lock both parents first in the hot
   path's order; release the quota in a following idempotent transaction; restate the bound.
B. Two runs of one tree purged concurrently each see the other unpurged, so neither deletes the tree's `run_payloads`.
   Fix: decide "last" under a tree lock (the root's row), or a sweep.
C. After `--abandon` the runs are stubs whose partitioned rows nothing deletes. Fix: a content-pending state that feature
   020 sweeps, or forbid abandon after step 2.

## Nit

D. The mark-only form would refuse itself under the "retirement active" rule. Fix: exempt it.
E. A purge that reads the retirement state in the statement that waits for the shared lock uses a snapshot from before
   the retirement's commit. Fix: a separate statement after the lock, tested with a barrier.
F. A missing earlier partition that was never created (a root just before a month's start) is answered `RUN_PURGED`.
   Fix: `RUN_PURGED` only for a month marked done in `journal_retirements`.

Counts: Blocker 0, Major 3, Nit 3

All six were fixed at `12e3fe58`: step 4 locks both parents in the completion's order and only detaches and drops, step 5
releases idempotently, the bound is 2 × 2 = 4 s plus catalog time (A); the root's run row decides the last purge (B);
abandon only before step 2, resume after (C); the mark-only form is exempt (D); the check is a separate statement after
the lock (E); `RUN_PURGED` only for a month marked dropped, and the previous month's partition is created (F).
