# Stage 8 (Intermediate Zoom Out) review of design 054 (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: 84e80ef86140e46ba58a03dd967960512a073f01
Reviewer: egm-review, stage 8 (the egm-zoom-out lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. The author's answer
follows each finding.

Counts: Blocker 0, Major 2, Nit 4

Scope verdict of the reviewer: still right; the fixes of stages 5 to 7 mostly reuse what exists (the byte ledger, the
correlation's evaluation, one discard helper, the shared hold). Nothing new to raise about reuse.

## Major

1. **Senders acting within the declared bounds can make a later step fail.** Pending payloads count in the 4 MiB, an output that
   does not fit fails its step `RUN_DATA_LIMIT`, and publication allowed declarations summing to the whole 4 MiB; the Risks row
   said "never the run". *Answer: fixed:* the byte sum of the declarations is capped at 2,097,152, half of a run's retained bytes,
   so step outputs always keep the other half; Requirement 054-001, Requirement 054-007, the Overview, the Risks row, the Studio,
   Architecture decision 0111 decision 4 and Decision 054-005 (revised) say so.
2. **The bound promised to authors is false for code runs**, which design 044 refuses from 3 MiB (`HISTORY_HEADROOM`), at a
   visible count and while a closing window is open. *Answer: fixed:* Requirement 054-007 states the bound for graph runs and
   defers code runs to design 044's headroom; the 2 MiB cap fits under it; Gap 054-008 asks design 044 to apply the cap and state
   its headroom.

## Nit

3. **`SIGNAL_ENTRY_WAIT_CONTINUES` is a refusal caused by the handoff opening a wait itself.** *Answer: fixed as proposed:* the
   refusal stays for 0.3.0; Gap 054-009 records the fix in the handoff; the guide explains why a segment starts with an activity
   (Task 054-15).
4. **Two orderings (entry time for live delivery, node id within a transaction).** *Answer: fixed:* the lowest node id everywhere;
   the derivation is gone (Requirement 054-004, Technical Plan 12, Architecture decision 0111 decision 2, Decision 054-003 revised).
5. **Three reason codes for one refusal across designs 044, 045 and 054.** *Answer: fixed:* `CAPABILITY_NOT_ACTIVE` with the
   capability named, from the shared hold of Task 054-18 (Decision 054-014; Gap 054-010 for design 045).
6. **The invariant that holds the design together is not written.** *Answer: fixed:* stated in Terms (a waiting root wait of a
   declared name and a pending signal of that name never coexist at a commit), and checked at every commit by the replay corpus
   (Requirement 054-020, Task 054-3).
