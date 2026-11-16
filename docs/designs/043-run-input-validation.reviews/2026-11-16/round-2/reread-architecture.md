# Targeted re-read of design 043 for approval: architecture (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: 4b6eb1a649b281660c4622e20d0b348322a7d8a4
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The fourth targeted
re-read of the changes since 2606e1d5, on the approval path the owner chose (OD-2026-11-16-3: fix, re-read the fixes,
accept the rest), not a full review. The reviewer read files only and ran nothing.

The history of this re-read: `reread-1-architecture.md` (Majors 5), `reread-2-architecture.md` (1), `reread-3-architecture.md`
(0), each fixed before the next; this is the last. No layering, ADR, compatibility or scope problem; the Nits of re-read 3
are closed. The gate record of re-read 1 (Major 5) is not counted open: nothing but the record is missing, and it must name
this commit, carry the conditions listed in `reread-3-architecture.md`, and add the unit tests of `apps/control-plane` and
`crates/infra-postgres`, the 0.2.0 inventory test run as the runtime role, and how the new unit test was seen failing.

## Nit

1. D043-188 claims more than the code shows about panic output: a `str` slice off a boundary quotes the string, and a
   panic inside a dependency is not covered; narrow the sentence.

Counts: Blocker 0, Major 0, Nit 1
