# Correctness review of design 041 (first draft)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 2e90dbb854291a319ec94bc53b65ccef5fddcc0f
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output was relayed by the coordinator in condensed form, grouped by subject across the three reviewers; this record lists the findings attributed to this reviewer, in the relay's words, with the answer of the revision. Findings the relay did not describe are listed as such and are not claimed answered.

## Major

1. M1. The tail can end before a late batch commits (shared with architecture M1).
   *Answer:* xid first in ingest, end rule on `xmax`, held-transaction test (D041-019, T041-12).
2. M2. Size, scrub and cut in the wrong order; counters from the batch, not the rows stored; markers carry log bytes.
   *Answer:* lock stream, drop known sequences, scrub and cut, measure as `sagat_quota_measure` (fields as canonical JSON text), attempt/run/tenant allowances, insert `RETURNING`, counters from returned rows; markers have null text (section 2, section 3, D041-017; test at the tenant boundary with a value redaction lengthens, T041-8).
3. M3. The tenant usage row serialises one tenant's ingest and delays completions (shared with architecture B1).
   *Answer:* as architecture B1 (D041-013).
4. M4. Deadlock between ingest and the script marker (shared with architecture M2).
   *Answer:* one lock order; marker does not charge `run_log_usage` (D041-020, T041-13).
5. M5. `DROPPED` counts can be double counted or inflated (shared with security M7).
   *Answer:* sequence ranges, unseen sequences only, saturating, clamped (D041-018).
6. M6. Test placement (shared with architecture M6).
   *Answer:* as architecture M6 (D041-023).

## Nit

7. N1. One `last_sequence` for worker and server ranges. *Answer:* `last_sequence` and `next_marker_sequence` (section 3).
8. N3. `xid8` and the horizon in the same statement. *Answer:* section 5, D041-019.
9. N4. Counters versus the horizon. *Answer:* FR-041-011 says counters can be ahead.
10. N6. Markers past the run ceiling multiply. *Answer:* run-ceiling and tenant drops only on the stream row (FR-041-006).
11. N7. `final` lost when the batch is past a ceiling. *Answer:* `final_at` recorded regardless (section 2 step 9).
12. N8. No concurrency tests at the ceilings. *Answer:* T041-8.
13. N10. SC-041-001 wording. *Answer:* reworded to commits under lock timeouts.
14. N11. `UNIMPLEMENTED` stops capture for the session. *Answer:* for the attempt only (FR-041-014).
15. N12. `/logs/streams` has no paging. *Answer:* `cursor` and `limit` (FR-041-011).
16. N13. Inconsistent status wording. *Answer:* as architecture M9.
17. N2, N5, N9, N14. Not described in the relay; not claimed answered.

Counts: Blocker 0, Major 6, Nit 14
