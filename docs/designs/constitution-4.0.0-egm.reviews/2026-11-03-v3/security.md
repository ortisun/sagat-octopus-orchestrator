# Security review of the EGM design v3

Doc: docs/designs/constitution-4.0.0-egm.md
Reviewed-commit: b3693e737138c966e688b248d0d028f6adcb48e1
Reviewer: security
Note: condensed by the author from the reviewer's output. Every finding, severity and location is kept; the evidence prose is shortened. Paths of the reviewer's machine are removed.

Previous round (8 Major): resolved M1 (header boundary), M6 for the commit format, argument order and leaf path, N4 (JSON fails closed); partly resolved M2, M3, M4, M7, M8; not resolved M5.

## Major

1. **R3 can be switched off with `.gitattributes` (doc 435).** The command has no `--text`; a pull request that adds `docs/designs/*.md -diff` makes git print "Binary files differ" and R3 finds no changed lines. Fix: `--text --no-renames`, treat `Binary files` as a failure, add a test.
2. **R2 does not check how many reports there are or who wrote them (345-347, 368-369, 385-392).** One hand-written file listed four times passes. Fix: a `Reviewer:` line in every report; the set of reviewers must be exactly the required ones, with distinct paths.
3. **"Pinned by the reviewed commit" is not enforced for the reviewer definitions (341-343).** R3 diffs only the design doc; editing `security-reviewer.md` to report nothing passes. Fix: compare the agent files with the Reviewed-commit, or record their blob ids in the report.
4. **Owner rows can be reused for acceptances and gates (391, 396-397, 470-475, 849-852; decision N).** Rows -1..-8 cover the whole doc and satisfy any acceptance; ids need not be distinct; substring matching lets `T018-1` match `T018-12`; `Status begins with Decided` accepts "Decided against". Fix: distinct ids, whole-token task id for gates, a finding anchor for acceptances, a closed status vocabulary, narrow `Covers`.
5. **The owner-decisions parser can disagree with what a reviewer sees (380-381, 470-473).** HTML comments, `~~~` fences, indented code and `\|` escapes are not handled; a hidden or shifted row can read `owner` where the page shows `delegate`. Fix: exact cell count, defined escapes, fail on a row inside a comment or fence.
6. **Decision N is a label, not a control, and CI runs the pull request's own checker.** The assistant writes the rows; the script, the hash list, the rows and the settings change in one pull request. Fix: base-ref rule for rows, run the base branch's checker, reword decision N; use `GITHUB_BASE_REF` in R9.
7. **The pin is not guarded and the marketplace source has no ref (489-493, T000-4).** Nothing checks the settings entry after the first commit; a later `autoUpdate: true` stays green. Fix: a rule in `check-agent-config.py`; set `ref` or say the pin is detection only; verify the keys against the schema.
8. **The plan enables the plugin before the controls exist (section 9, T000-4 against T000-14 and T000-25).** Between the two merges contributors are offered a third-party plugin with write-capable agents and no warning. Fix: move T000-4 into PR 2.
9. **The hash does not fully cover the plugin and its first entry cannot be checked (494-503, T000-3).** Exclusion anchoring is unstated (`**/assets/**` could hide an executable), symlink handling is unstated, the executable bit is not hashed, and the first entry is the hash of one cached copy. Fix: narrow the exclusions, refuse symlinks, hash the mode, reproduce the hash from the upstream commit.
10. **The plugin's agents are exposed in project scope and the compensating control cannot be applied (313, 716-721, 602).** The synthesizer holds `Agent`, `Write`; no plugin instruction says to treat the reviewed document as data; the optional path-wide `permissions.deny` would block the implementation work itself. Fix: commit to one outcome, no path-wide denies, state that synthesizer output is data and recount the findings.

## Nit

1. Path confinement checks only the leaf (387-388): use `realpath`/`commonpath` and git mode 100644.
2. Regex details: use `fullmatch`, `[0-9]`, bound the digits; the `Doc` line has no status-block counterpart; say "per line".
3. `git cat-file -e <sha>` also succeeds for a tree or blob: use `<sha>^{commit}`.
4. Wrong citation (716, decision O): "plugins are supply chain" is `docs/engineering/ai-engineering.md:50`, not `AGENTS.md` rule 20.
5. A committed report already contains an absolute local path; the rule is prose only. Fix: R2 rejects an absolute path prefix and the report is cleaned.
6. The branch-name filter still passes instruction-shaped names and the hook prints the current branch unfiltered (T000-25).
7. R8 proves self-consistency only; the hashes sit in the report the same pull request writes.
8. Hook robustness: `installed_plugins.json` entries are a list per plugin; parse and hash errors must be caught.
9. `git ls-tree` ref argument: use full ref names and `--end-of-options`.

Counts: Blocker 0, Major 10, Nit 9
