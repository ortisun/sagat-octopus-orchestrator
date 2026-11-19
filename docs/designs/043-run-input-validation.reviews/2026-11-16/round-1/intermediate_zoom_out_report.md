# egm-review Stage 8 (Intermediate Zoom Out), round 1

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: f562614874016262dd9812e241f734038d4d70de
Stage reviewer: egm-zoom-out (zoom-out-standard), full file, reading the Stage 5 to 7 reports of this round
Note: condensed by the author; every finding and severity is kept.

Overall: Stage 5 asks the design to shed detail; Stages 6 and 7 mostly ask it to add definitions, summaries and
guarantees. Applied as written, the document grows and some remedies add code paths where a deletion or a sentence
would do. The findings steer those remedies.

## Major

1. The context remedies (comprehension 7 to 10, 19, 23; critic 5) would make the document grow back. Fix: one "Terms
   and references" block of about eight one-line entries with links (holder and `USE_EXISTING`, repeat, `decide` /
   `BUFFERED` / owed pointing to design 018, requirement recording and the registry's closed gate renamed, ADR-0090,
   ADR-0092, the constitution); one sentence pointing to ADR-0092's Considered options as the fallback; no summaries.
2. Stage 5 and Stage 7 give opposite remedies for Appendix A. Fix: state each bound once in NFR-043-BOUND with the test
   that holds it; move Appendix A to `research.md` beside D043-064, D043-069, D043-071, D043-159; drop the pointers.
3. Fix the compile-panic case by simplifying: a panicking compile is deterministic, so remember it as `Unusable`
   (changing D043-106); `report` then admits it; correct CUJ 5; delete the Risks row; no transient bypass in `report`.

## Nit

4. Tie the charges, the budget and the pre-check limits to `run-input-validation-v1` in one sentence; no new versioning.
5. State the permits, the waits, the cache size and the budget as compile-time constants; scale out by replicas;
   G043-006 stays the open measurement.
6. Answer the 0.2.x readers question with a short table per new stored value; a test only where versions coexist.
7. Number the architecture rules only after deciding Stage 5 Major 2 and Nit 4.
8. Rename the `deadline` label `unavailable`; keep the cause in the log, not in a label; alert guidance in the runbook.
9. The per-answer audit of a holder answer in `report` is machinery for a transition aid; offer "count only" in
   ADR-0092 decision 5.
10. Tick the built tasks with their evidence; remove per-symbol line numbers; reconcile the phase paragraph.
11. The remaining earlier remedies are deletions or one-clause fixes: graded disclosure in `errors.md` and T043-5;
    row 12 checks after the lock is released; the CLI help's owner role; T043-3's tokio; the inventory needs no
    migration; "no remote reference"; "proportional", not "exact"; delete the Overview history; state the tuple
    consequence honestly.
12. A correction to Stage 5 Nit 9: dropping `shared: false` changes accepted ADR-0090 decision 6; withdraw it or hand it
    to the owner as an ADR-0092 option.

Counts: Blocker 0, Major 3, Nit 9
