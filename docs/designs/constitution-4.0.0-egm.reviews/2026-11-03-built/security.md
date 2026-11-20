# Security review of the built change (PR 1 and PR 2)

Doc: docs/designs/constitution-4.0.0-egm.md
Reviewed-commit: 072481c8673916b5a5b870a6e9f43b37d4a5b4f6
Reviewer: security
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files only and ran nothing.

## Major

1. **A ticked task in an unrecognised shape escapes R4, R7, R9 and R3** (`- [x] T018-3: done`, `- [x] **T-1**`, two spaces, `+ [x]`), and a backtick fence opener with a backtick in its info string hides every task up to the next fence line.
   *Resolution:* fixed in 5f9a71a3. A checkbox line that is not a task fails R4; the fence rule follows CommonMark. Tests: `test_a_ticked_line_that_is_not_a_task_fails`, `test_a_backtick_fence_with_a_backtick_in_its_info_string_is_not_a_fence`.
2. **Plugin tampering that makes the hash script exit non-zero is shown as "skipped"** (a symlink, an unreadable file, a non-UTF-8 file name that made the script raise).
   *Resolution:* fixed in 88dcaa88. The hook reports a plugin that cannot be hashed as not accepted; the script handles names as bytes and exits 2 on an unreadable directory.

## Nit

1. Machine paths are committed in earlier review reports. *Resolved:* scrubbed from all reports.
2. Unbounded digits crash `next-feature-id.py` and the semver parser. *Resolved:* six ASCII digits at most; git failure exits 2.
3. Report paths are used as git pathspecs. *Resolved:* `--literal-pathspecs`.
4. Doc-controlled strings are echoed into failure lines. *Resolved:* only plain values of at most 80 characters are printed.
5. The branch-name filter is a shape check. *Recorded:* residual risk; the test was renamed and says so.
6. Dot files in `docs/designs` were never checked. *Resolved:* listed and checked.

Suspicions: the marketplace source has no `ref` (no tag or branch fixes the accepted commit; the design calls the pin detection only); `__pycache__` and the top-level `tests`, `site`, `distributions` are excluded from the hash (no Python is imported at session time in plugin 0.1.0); `Bash(git branch:*)` was already in the allow list. None changed.

Counts: Blocker 0, Major 2, Nit 6
