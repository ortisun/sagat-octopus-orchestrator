# egm-review, style lens (Stages 7 and 9) of design 050 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 4ac4065d
Reviewer: egm-review (Stages 7 and 9, egm-style)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author to one paragraph per finding, every severity, location and fix kept. Wrapping was excluded from the brief as a known, settled finding.
Counts: Blocker 0, Major 8, Nit 6

1. **Major. Requirement 050-007, Technical Plan 4 "Bodies", CUJ 1, Task 050-8.** Three rules for a body without its `return` (required, refuses to save, shows a problem), and a double negative in the acceptance. Fix: one rule everywhere; the draft still saves.
2. **Major. Requirement 050-022.** Prohibitions and required supports in one list read as banning the three themes. Fix: a MUST NOT list and a MUST list.
3. **Major. Requirements 050-009, 050-014; Technical Plan 5.** "Intent" undefined; "a changed decision MUST never be refused for its key" compressed; the test name promises acceptance the server does not give; "a resend is the person's". Fix: define intent; restate; rename the test; "only the person resends".
4. **Major. Terms, Requirements 050-001, 050-003, Technical Plans 3 and 4.** "Capability or advertisement" and the drivers list as a fourth meaning; the registry field's type is unclear. Fix: a defined `requires` shape; "capability" only for names of `GET /version`.
5. **Major. Requirement 050-012, Technical Plan 6.** "Tab" means the inbox tab or the browser tab. Fix: "inbox tab" and "browser tab"; two tests.
6. **Major. Requirements 050-008, 050-010, 050-011; Technical Plans 3, 5 and 6.** Long sentences each carrying several route and port rules. Fix: a lead sentence and one rule per bullet; a table for the iteration route.
7. **Major. Technical Plan 3, the instance-registry paragraph.** "This work" has no referent; member, beat, epoch and closed gate undefined; "it" unclear; feature 035's sentence repeated. Fix: define, name the noun, delete the repeat.
8. **Major. Requirements 050-003, 050-008, 050-012, Non-functional requirement 050-002, Decisions.** "Owner node", "the validation delay", a circular "frame budget", "within a bound" and "Stage N review" undefined. Fix: define or state each.
9. **Nit. Overview.** An ambiguous attachment of "lake writes" and a three-rule first bullet. Fix: split.
10. **Nit. Identifiers.** "Owner decision 2026-11-24-4" and "OD-2026-11-24-4" for one record; a hard sentence in Open Questions; three self-references. Fix: one form; rewrite.
11. **Nit. Technical Plan 6, "Ended runs" and "Run ends".** "The adapter" ambiguous; a passive sentence hides the probe test. Fix: name the file and the actor.
12. **Nit. Technical Plans 6 and 7, Decision 050-012.** A garbled audit sentence, "the loop events of the same names", "a fifth of that". Fix: rewrite, list the events, name the figure.
13. **Nit. Overview, The Problem, Decision 050-001.** "Small", "the products the owner wants to surpass", "not an accident", an unlinked assessment. Fix: drop or name.
14. **Nit. Technical Plan 4.** "It changes to keep each" means `documentIO.ts`. Fix: name it.
