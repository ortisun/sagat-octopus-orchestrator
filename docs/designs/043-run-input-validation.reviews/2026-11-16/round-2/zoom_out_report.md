# egm-review Stage 5 (Initial Zoom Out), round 2

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: b9b0f1b56d121ba3f977e2cea888305402f2eb47
Stage reviewer: egm-zoom-out (zoom-out-standard, EGM 0.1.0)
Note: condensed by the author; every finding and severity is kept.

## Major

1. The Technical Plan and several FRs read as a description of what was built (internal symbols, constants, migration
   ids, engine settings, test names in acceptance criteria, lint file paths); rules are stated in two to four places.
   Fix: components, interfaces, data flow and outcomes in Technical Plans 1 to 8; each rule once; symbols and constants
   to the tasks or `research.md`; acceptance names suites; FR-043-016 as one sentence, the seven rules in T043-4.

## Nit

2. Rules 2, 6 and 7 make up for debt; rule 7 has no follow-up (gate as a constructor argument, T043-28).
3. `ScheduleService` is said to hold `unwired()` and then to hold no gate.
4. "Appendix A" is cited at line 517 and no longer exists.
5. The fair queue is justified by an unmeasured starvation scenario; let T043-27 decide it.
6. A scheduled action validates twice; pass the first `CheckedRunInput` to the start.
7. Complexity Tracking and Alternatives tell the drafts' history.

Counts: Blocker 0, Major 1, Nit 6
