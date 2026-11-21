# egm-review Stage 8 (Intermediate Zoom Out), round 2

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: b9b0f1b56d121ba3f977e2cea888305402f2eb47
Stage reviewer: egm-zoom-out (zoom-out-standard), full file, reading the Stage 5 to 7 reports of this round
Note: condensed by the author; every finding and severity is kept.

## Major

1. Fix the comprehension and style findings by removing detail (NFR-043-BOUND one sentence linking the subset contract,
   `details.rule` a link, mechanics of Technical Plan 5 to the tasks, one name for the validator), not by adding terms.
2. A second hand-written limiter for the pre-check; take its permit from the existing `Permits` pool and delete it.
3. The per-tenant cache share is unmeasured; drop it for a plain weight-bounded LRU, or accept the single-tenant limit
   and let T043-27 decide; no work-conserving share.
4. The inventory gaps by wording: CUJ 5 expects only versions nobody still starts; exit 1 persists for superseded
   versions; the Risks row cites `report` and the failure codes, not the inventory.

## Nit

5. An engine creator should try the permit once and leave the item on `Busy`, not wait 5 s inline.
6. `ScheduleService` described two ways.
7. "Appendix A" does not exist.
8. Exactly-once audit mechanics for the legacy cron pass that is being retired.
9. "Triggered action" means a buffered trigger-now released and refused.
10. The Overview never mentions `enforce`/`report`.
11. The holder audit in `report` mode is machinery for a transition aid; recommend counting only.

Counts: Blocker 0, Major 4, Nit 7
