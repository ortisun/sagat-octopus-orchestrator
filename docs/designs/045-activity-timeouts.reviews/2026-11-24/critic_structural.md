# Stage 7 (Critic Review), structural lens, of design 045 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 67cc649e
Reviewer: egm-review, stage 7 (the egm-structural lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author; every finding and its severity kept, with the author's answer. The reviewer read the working
tree, which differed from 67cc649e only in other features' files.

Counts: Blocker 0, Major 4, Nit 12

Every section of the template is present, in order, with its levels; the Constitution Check rows start as the checker expects;
every link resolves.

## Major

1. **Identifier format is not the template's** (`FR-NNN-xxx` and the others). *Answer: contested*: constitution 4.7.0
   (Development Workflow, "Identifiers") and `docs/governance/identifier-scheme.md` make the word forms ("Requirement 045-001") the
   rule for new prose; only the places a tool parses keep the old form (the task lines, which do). The template's comment predates
   4.7.0; the identifier sweep will update it.
2. **A cross-reference points to the wrong option of ADR-0110.** *Answer: accepted*: renumbered.
3. **Risks has no security row.** *Answer: accepted*: rows for the worker's delay, a worker sending a platform code, the heartbeat
   rate and the requirement race (from the security review).
4. **"One migration" contradicts "a second migration".** *Answer: accepted*: two migrations, everywhere.

## Nit

5. Task references in prose ("Task 045-17") versus the task lines (`T045-17`). *Answer: contested*: the word form is the rule for
   prose (identifier scheme); the task line keeps the parsed form until the sweep.
6. Owner questions skip 045-007 and 045-008, and the section lists only decided items. *Answer: accepted*: said, with "none open".
7. Inconsistent list items in that section. *Answer: accepted*.
8. The buffered-signals scope note sits under The Problem. *Answer: accepted*: moved to Non-Goals.
9. Alternatives are scattered and rely on the ADR. *Answer: accepted*: each alternative gives its reason in place.
10. Acceptance criteria point to Technical Plan 4, which lists statuses. *Answer: accepted*: they point to Technical Plan 7.
11. Gaps placed inconsistently. *Answer: accepted*: Open Questions lists only open gaps; settled ones are in the register.
12. "Slices 1 to 6" used before the slice table. *Answer: accepted*: the Risks row names what ships instead.
13. Index definitions broken across lines. *Answer: accepted*: a fenced `sql` block.
14. The `json` blocks are fragments. *Answer: accepted*: labelled as fragments.
15. Links and reference names styled inconsistently. *Answer: accepted in part*: architecture decisions on `develop` are linked;
    owner decisions keep the word form of the identifier scheme.
16. New failure codes are not declared in one place. *Answer: accepted*: Technical Plan 12 lists them.
