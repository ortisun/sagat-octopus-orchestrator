# egm-review for the approval of design 043 (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: 4b6eb1a649b281660c4622e20d0b348322a7d8a4
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary. The last complete run of the five stages of `egm-review` is round 2 (2026-11-16, at
`b9b0f1b5`; its stage reports and `egm-review.md` beside this file). The owner chose to approve after fixing the findings
marked for a fix, re-reading the fixes, and accepting the rest (OD-2026-11-16-3), so no complete run follows the last fix:
the fixes were re-read four times by EGM lenses (comprehension and implementation readiness; `reread-1-egm-review.md` to
`reread-3-egm-review.md`), each Major fixed before the next, and this summary records the last re-read and the findings
of round 2 that stay open because the owner accepted them.

## Open, accepted by the owner (round 2)

1. Major (docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/zoom_out_report.md#1): the Technical Plan and several requirements read as a record of what was built.
   Accepted, OD-2026-11-16-100.
2. Major (docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/critic_report.md#2): the tenant share of the cache is not work-conserving. Accepted, OD-2026-11-16-101
   (and OD-2026-11-16-6; T043-28 measures it).
3. Major (docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/critic_report.md#4): inputs inside the subset that their schema refuses are not visible before the upgrade.
   Accepted with the Risks row reworded, OD-2026-11-16-102.
4. Major (docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/critic_report.md#5): an engine pass can wait on permits inline. Accepted, OD-2026-11-16-103.
5. Major (docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/intermediate_zoom_out_report.md#3): the per-tenant cache share is unmeasured. Accepted, OD-2026-11-16-104.
6. Major (docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/implementation_readiness_report.md#1): the phase order of the tasks. Accepted, OD-2026-11-16-105.
7. Major (docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/implementation_readiness_report.md#7): the gate's signatures are elided in the plan. Accepted,
   OD-2026-11-16-106.

Every other Major of round 2 was fixed and its fix re-read (the map is `owner-questions.md` with D043-173 to D043-188).

## The last re-read (egm-comprehension, with readiness checks of T043-24)

The Major of re-read 3 (a 0.2.x database said to both have and lack the schedule tables) is resolved, and its four Nits.
No new Blocker or Major. Nits: 8. which 0.2.x build has the schedules is not said, and the runbook still says a 0.2.x
database has none; 9. "the runtime role holds all five" is the wrong reason the inventory runs before the migration;
10. Technical Plan 8's list of exit-2 causes omits the panic; 11. "each inventory run as the runtime role" covers two
tests that run no inventory; 12. T043-24 does not cite the test of the panic-to-`Interrupted` step.

Counts: Blocker 0, Major 7, Nit 5
