# Correctness review of the built change (PR 1 and PR 2)

Doc: docs/designs/constitution-4.0.0-egm.md
Reviewed-commit: 072481c8673916b5a5b870a6e9f43b37d4a5b4f6
Reviewer: correctness
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files only and ran nothing.

## Major

1. **R1 read fenced and commented lines.** *Resolved* in 5f9a71a3 (fenced lines and HTML comments are dropped before the section checks).
2. **Indented sibling tasks were swallowed by the previous task.** *Resolved* in 5f9a71a3 (an item ends at a sibling task line).
3. **R8 could be satisfied by relabelling a principle or by choosing `Previous-source`.** *Resolved* in 5f9a71a3 for the second part: when the constitution differs from the base ref, `Previous-source` must be the base text and the version must increase. *Not changed:* a REWRITE disposition is allowed for a principle, because a MAJOR amendment may change one; the owner reviews the dispositions. Goal 1 holds for this change because all eight rows are KEEP and the hashes are equal.
4. **Failure lines echoed doc content.** *Resolved* in 5f9a71a3 (plain values only, otherwise "(malformed)").
5. **The accepted-hash list was validated only when a doc was approved.** *Resolved* in 5f9a71a3 (validated on every run; a missing list fails).
6. **Two paths skipped rules silently** (glob characters in the root; an unreadable file treated as missing). *Resolved* in 5f9a71a3 (`glob.escape`, `listdir`, an unreadable file fails).
7. **`egm-tree-hash.py` swallowed an unreadable directory.** *Resolved* in 88dcaa88 (exit 2).
8. **The seven-file wording test did not exist.** *Resolved* in cf528bee, with the missing wording in `security-reviewer.md`.

## Nit

1. Tests that cannot fail for the right reason, and tests listed in the design but absent. *Partly resolved:* the 40/41-line pair, more than four reports, 999 taken and the `crates/x/src/lib.rs:42` token were added; the remaining weak cases (deleted hyphen line, option-shaped commit) still assert only the exit code and an earlier rule.
2. A ticked checkbox that is not a task line was invisible. *Resolved* (see the security report, Major 1).
3. The follow-up region accepted unreviewed prose. *Resolved:* only blanks, "None yet." and tasks are dropped.
4. `path_exists` is more permissive than the spec (a directory in the middle of a path). *Resolved in the design:* section 8 now says a directory matches whole or as a suffix; `..` can no longer climb out of the repository.
5. `next-feature-id.py` edge cases (Unicode digits, `fullmatch`, git failure). *Resolved.*
6. R2 did not validate `Author` or `Date`. *Resolved.*
7. The hook's name filter admits short instruction-shaped slugs. *Recorded* as residual risk.
8. A CRLF draft failed with misleading messages. *Resolved:* a carriage return fails R9 with its own message.

Counts: Blocker 0, Major 8, Nit 8
