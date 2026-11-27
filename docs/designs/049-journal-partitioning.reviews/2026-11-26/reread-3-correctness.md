# Targeted re-read 3 of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: b2edd5bcf279155c974f6ef43b69d076e9ac782a
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A third targeted re-read
of the fixes of re-read 2 (changes since 12e3fe58), not a full review. The reviewer read files only and ran nothing.

Re-read 2: Majors 1 to 4 resolved (the per-table transaction takes the parent before the partition, the same order as
every platform transaction in PostgreSQL 16; the sweep selects runs by `purged_at`, which commits with each chunk);
Nits 1 to 3 resolved.

## Major

1. Nothing serializes the retirement functions with the abandon, and an abandon is not terminal once it starts: a crash
   during an abandon followed by a resume releases the step-3 bytes again (double release), and a purge chunk racing the
   sweep leaves rows nothing deletes. Fix: every retirement function locks the retirement's row `FOR UPDATE` and checks
   its state; the abandon commits `ABANDONING` first, which the others refuse, and is refused after a drop; tests of both
   interleavings.

## Nit

1. An anti-wraparound vacuum of the old month is not cancelled, so locking the partition can hold the parent for the
   whole lock timeout on each attempt. Fix: lock the partition `NOWAIT`.
2. `journal_retirements` is keyed by bucket; whether an abandoned month can be retired again is not stated.
3. A per-second counter cannot show one statement waiting more than 4 s (use the maximum statement latency); ADR
   decision 6's wording of the sweep differs from the design's.

Counts: Blocker 0, Major 1, Nit 3

All four were fixed at `b7a3126d`: the retirement's row serializes every function, the abandon commits `ABANDONING`
first and is refused after a drop, with the two interleaving tests in Task 049-14; `NOWAIT` on the partition; a later
start reactivates an abandoned row; the bound is read from the maximum statement latency; the ADR's wording aligned.
