# Correctness review of the EGM design v3

Doc: docs/designs/constitution-4.0.0-egm.md
Reviewed-commit: b3693e737138c966e688b248d0d028f6adcb48e1
Reviewer: correctness
Note: condensed by the author from the reviewer's output. Every finding, severity and location is kept; the evidence prose is shortened. The reviewer could not run code and reasoned from the text and the files.

Resolved from the earlier round: the fenced example, the marker on the task line, the whitespace exemption, unnumbered tasks, owner-decision status and ids, evidence deletion, allocator branches and worktrees, test git configuration, the reviewer severity scale. Line numbers of T000-7, T000-14..23 and the hook plan, the T000-27 file list and the 016 quickstart edit were checked and hold.

## Blocker

1. **R3's header rule contradicts the status block, so the first approval always fails (357-375, 435-439).** The hyphen line moves from line 7 to line 14. Fix: find the first hyphen line separately in each version, compare the header against a field whitelist, compare bodies by content.
2. **R4's cross-document uniqueness conflicts with `T000` being the prefix of every unnumbered doc (353-355, 394-395, 747).** `spec-conversion.md` would collide with T000-1..27. Fix: scope uniqueness to numbered ids or derive the prefix from the stem.

## Major

1. **An evidence marker placed above the description satisfies R7 and passes R3 with no evidence (407, 415-422, 442-443).** Inserting `Evidence (date):` as the first continuation line turns the task's own description into an "evidence block". Fix: lines present at the Reviewed-commit are never evidence; the marker must come after the last of them.
2. **The backticked-path rule fails on slash commands and on T000-15's own new text (529-536, 876-877).** `/verify`, `/adr`, `/doctor`, `/egm:egm-draft`, `rest/conformance.rs`, `contracts/`, `checklists/` would all fail. Fix: exclude `^/[a-z][a-z0-9:-]*$` and define relative resolution.
3. **R8's "principle block" has no definition (164-165, 283, 401-403, 834).** Heading, trailing blanks, newline form and encoding are unstated. Fix: state the byte range.
4. **T000-14 misses the constitution path on `AGENTS.md` line 3.**
5. **Deleting `.specify/.gitignore` un-ignores `feature.json` (969-970).** Fix: add the ignore to the root `.gitignore` in T000-27.
6. **The new path check does not fit `check()` and its tests (536, 950-959).** 17 existing tests use non-git fixtures. Fix: a separate function called from `main()`, or git fixtures.
7. **The CI step cannot tell a pull request from a push (448-450, 816).** Fix: read `GITHUB_EVENT_NAME`; unset means pull-request behaviour.
8. **A test for T000-2 already exists and encodes behaviour the doc does not state (36, 759, 780-785, 494-500).** Symlink refusal, missing-directory exit and glob semantics. Fix: state them; change "Create" to "Complete".

## Nit

1. Header blank lines contradict the grammar of section 3.
2. R2's "Open findings = sum minus n, floored" is dead: the sum is already at most n.
3. `docs/owner-decisions.md` is not strictly newest-first.
4. The README sentence T000-16 deletes runs to line 723, not 722.
5. `correctness-reviewer.md` line 10 has no `.specify/memory/...` path to replace.
6. The transitional clause as drafted names `/speckit-*`, which Goal 4's search forbids in the constitution.
7. The "Enforced by" extraction is unspecified; line 27 also backticks `architecture` and `cargo metadata`.
8. Goal 1 counts Governance as bullets; it is paragraphs.
9. `Follow-up tasks` is described as at the end of the plan; it is followed by two sections and holds "None yet.".

Counts: Blocker 2, Major 8, Nit 9
