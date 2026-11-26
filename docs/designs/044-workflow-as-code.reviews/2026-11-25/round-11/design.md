# egm-review Stage 7 design lens, targeted re-read, round 11 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 9bdb707d84f3fef3c190979cde128256f2b8159a
Reviewer: egm-design, feature-spec standard, FeatureRange from the start to the Detailed Implementation Plan
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 10's findings,
A, B, D and Nits 1 to 3 and 5 are closed; C and Nits 4 and 6 partly closed.

## Major

1. SDK-issued commands are still unbounded: update completions of accepted updates, one consumption per signal name,
   scope closes; bound accepted updates, one consumption command, count closes with aborts, carry overflow forward.
2. Re-emitting a deferred blob-carried value forces a re-upload and a double charge: refer to the held blob by its
   descriptor; net the charge; a test.
3. The continue-as-new chain check locks the chain after the run (reverse of the pass), and its reads are outside the
   statement budget: an unlocked pre-check with the pass as authority, or the chain lock first; count the reads.

## Nit

4. `WriteBlob` still lists a continue-as-new output.
5. Section 6.3 still says "count against its 1,024".
6. The continue-as-new `state` option has no carrier or size rule.
7. The 3 MiB headroom rule differs between Requirements 044-027, 044-014 and section 6.3.
8. `run.cancel_requested` is no longer journaled once per run.
9. No number for the workflow-task claim's statement budget.

Counts: Blocker 0, Major 3, Nit 6
