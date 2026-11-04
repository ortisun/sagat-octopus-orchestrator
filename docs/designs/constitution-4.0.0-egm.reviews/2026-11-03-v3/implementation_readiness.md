# Implementation-readiness review of the EGM design v3

Doc: docs/designs/constitution-4.0.0-egm.md
Reviewed-commit: b3693e737138c966e688b248d0d028f6adcb48e1
Reviewer: egm-implementation-readiness
Note: condensed by the author from the reviewer's output. Every finding, severity and location is kept; the evidence prose is shortened. Every sampled file and line number of the plan matched the working tree. Files that appeared untracked in the worktree during the review (the first scripts) were excluded from the judgement.

## Blocker

**B1. R3's header rule contradicts the status-block flow, so every `Approved` doc fails R3 (357-359, 374-375, 436-438; CUJ 1 and 2).** Fix: locate the hyphen line independently, allow header differences only in field lines and blanks, drop the line-number requirement.

## Major

1. **Ticking PR 1 tasks needs `Approved`, but the approval machinery arrives later (R9, T000-20, T000-8, T000-3).** Nothing says how this doc is approved. Fix: a bootstrap paragraph.
2. **R3 cannot tell a pull request from a push (448-450, 774-775).** Name the variables and the default.
3. **R8's "principle block" and "Enforced by" path are undefined (401-403, 834-836, 283).**
4. **R4 breaks on the planned follow-up design (355-356, 394-397, 747).**
5. **R2's report set and counting are ambiguous (344-349, 368-369, 389-392).** A summary file repeats the lens findings, so summing double-counts. Fix: state exactly which files are listed and carry `Counts:`.
6. **The allocator's output about branch names is contradictory (516-520, 585, 800-802, decision I).** Define the empty case and the 999 case.
7. **The hook's inputs are unspecified (504-506, 941-948).** The shape of `installed_plugins.json`, where the hook finds the script and the list, the warning wording, deduplication.
8. **The path check does not fit `check-agent-config.py` (529-536, 950-959).** `check()` returns a list and `main` exits 0 or 1; the existing tests use non-git directories; fence and span handling is unstated.
9. **T000-26 and T000-27 must finish last and the allowlist has no admission rule (950-971).** Allowlisting "tokens that still fail" makes Goal 4 trivially true.
10. **Deleting `.specify/.gitignore` un-ignores `feature.json` (969-970).**
11. **T000-3 cannot be reproduced without the owner's install (485-486, 500-503, 787-791).**
12. **T000-4's `permissions.deny` step is undefined and conflicts with the plan (718-721, 796).**
13. **The `#### Follow-up tasks` region has no defined end (423-424, 444, 972-976).**

## Nit

1. Field formats: `Claude-Code`, `Accepted` when `n` is 0, meaning of `Date`; `Open findings` is redundant.
2. T000-9 must follow T000-8's final text; "Last Amended the merge date" is unknowable.
3. The 014 quickstart has no sentence describing the call, only the fenced command.
4. T000-13 omits the `docs/adr/README.md` index entry and the fitness-check registration.
5. T000-27's title says 21 files and the body lists 19.
6. Hash grammar: glob dialect, anchoring, symlinks, hex case, sort order.
7. The transitional clause as drafted contains `/speckit-*`.
8. The Verification Plan depends on non-deterministic reviewer output and gives no pass criterion.
9. `concepts_brief.md` appears in two places; "preset 0 or 1" is never defined.
10. The check's CLI default and failure-line format are unspecified.
11. T000-19 says "adapt the sections" without a target.

Counts: Blocker 1, Major 13, Nit 11
