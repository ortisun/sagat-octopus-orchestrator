# Stage 8 (Intermediate Zoom Out) review of design 045, round 4 (2026-11-26)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 4d1f87ea
Reviewer: egm-review, stage 8 (the egm-zoom-out lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author; every finding and its severity kept, with the author's answer.

Counts: Blocker 0, Major 3, Nit 3

The design still addresses the root problem within the owner's scope; the Stage 5 findings were applied or contested soundly.

## Major

1. **Technical Plan 15 overrode text it did not update** (the withdrawal window, the pause, the first beat, the metric names,
   `pausedAtUs`, the lock, the feature 022 counterpart). *Answer: fixed*: each rule is merged into the requirement or section it
   changes; Technical Plan 15 keeps only the details stated nowhere else.
2. **The Technical Plan slipped back into code-level detail.** *Answer: settled* (Owner decision 2026-11-26-561): the names were
   asked for by the implementation-readiness and codebase-details reviews of rounds 2 and 3, which failed the plan without them; the
   design keeps them where a builder needs them, and the contracts stay at the altitude of the requirements.
3. **The scope should be split into phases.** *Answer: fixed*: Phase A (level 1, no protocol change) and Phase B (protocol 1.9, level
   2), each with its slices and success criteria; the integration rows are built by whichever feature lands second.

## Nit

4. Design 044's envelope mapping and the later-feature rule are defined here. *Answer: settled* (Owner decision 2026-11-26-561): the
   coordinating session asked this design to accept `timeoutLevel` and to own the column (Gaps 044-020 and 044-026).
5. The inventory is a new command. *Answer: fixed*: a mode of design 043's inventory module.
6. Requirement 045-001's refusals as a table. *Answer: settled* (Owner decision 2026-11-26-561): the shared refusal fixture is the
   single source the tests read; the prose states each rule once.
