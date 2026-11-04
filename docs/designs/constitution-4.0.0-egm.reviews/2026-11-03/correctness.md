# Correctness review: `docs/designs/constitution-4.0.0-egm.md` (commit c3f25a41)

Reviewer: `correctness-reviewer` (read-only; the author saved this report).

I had only Read, Grep and Glob, so I ran no scripts. Counts come from regex searches over `specs/*/tasks.md`, `docs/owner-decisions.md`, `.claude/`, `.git/` refs and the installed EGM plugin. They are exact for the patterns stated.

Real data used:
- **Task lines.** `^\s*[-*] \[[ xX]\] T\d{3}-\d+` matches 1,946 lines in 35 files: 678 ticked, 1,268 open. This agrees with the doc's claim.
- **Evidence markers.** The four marker forms in section 4 match about 141 lines in total: `*Done D.*` 11 (all in 018), `**Evidence (D)**:` 20 (all in 037), `_Evidence:` 110 (014, 016 and 2 in 001). `Partly done` matches 0.
- **Owner decisions.** `docs/owner-decisions.md` has 64 table rows, no id column, and no common status column.

## Findings

### Blocker

**B1. The doc's own section 4 example fails the check that PR 1 adds to CI (lines 398-401, R5 at 373, R4 at 371).**
- The example sits in a ```text fence at column 0. It matches the task regex: `- [x] T018-012 ...`.
- It carries `Owner-gate: OD-2026-12-01-1`. No such row exists in `docs/owner-decisions.md`.
- The grammar says nothing about fences. As written, R5 fails on this doc in PR 1's CI.
- If R4's "no other design doc defines the same id" scans unnumbered docs, it also blocks the later conversion of 018, which will define T018-012.
- Confidence: certain as written.
- Recommendation: make the task scanner fence-aware, or write the example without a leading `- [x]`. Add a test with a task inside a fence.

### Major

**M1. The header boundary can be moved to exempt the body from R3 (lines 355 and 404-408).**
- The header is "every line before the first line of 20 or more hyphens". The doc has hyphen lines at 7, 679 and 933.
- Input: delete line 7 in the working tree. The header now ends at 679. Lines 8-678 (Overview to Risks) become "header", and the diff examines only the body.
- The deleted hyphen line is in neither region. Edits to the former body pass.
- The header is also unconstrained text. Anything can be put before the first hyphen line.
- Confidence: certain from the wording.
- Recommendation: take the boundary from the Reviewed-commit version only. Require the hyphen line to be unchanged and the header to be exactly the ten status fields.

**M2. A task's `Owner-gate:` line can be stripped after approval (lines 389-395, 404-408, 428-433).**
- The evidence block runs from the first marker to the end of the item. The grammar does not say where `Owner-gate:` goes.
- Approved doc:
  ```
  - [ ] T018-012 x
    *Partly done 2026-12-01.* ...
    Owner-gate: OD-2026-12-01-1
  ```
- Change to `- [x] T018-012 x` plus the marker line, with the gate line deleted.
- R3 passes: the removed line was inside an evidence block in the old version. R5 passes: no gate. R7 passes: a block exists.
- Real files put progress notes before trailing metadata, for example 018 T018-007.
- Confidence: certain.
- Recommendation: compute gates from the Reviewed-commit version as well. Require that any gate present there is still present when the task is ticked. Exclude `Owner-gate:` lines from evidence blocks.

**M3. The inline marker form cited as "in use today" cannot pass R3, and it unprotects the task text (lines 389-395, 404-408).**
- 037 has 20 ticked tasks with `**Evidence (2026-11-01)**:` on the task line itself, for example T037-020.
- "The first line that contains a marker" is the task line. So the whole task line, description, Accept and Verify included, is an evidence block.
- Any rewrite of a 037-style task description passes R3 once the marker is on the line.
- The tick itself replaces the whole line, so the `-` and `+` lines differ by more than the checkbox. The old line has no marker, so it is not in a block in its own version.
- The rule says nothing about removed lines. Taken literally, the 037 form can never be ticked after approval.
- Confidence: certain.
- Recommendation: require the marker to start a continuation line and not the task line. Define pairing and removed-line handling explicitly.

**M4. The marker forms do not match the real files, and "Partly done stays unticked" is not enforced (lines 389-395).**
- 018 uses `*Part done 2026-10-29 (task stays open).*` on 7 open tasks. These are not `*Partly done YYYY-MM-DD.*`.
- 018 T018-026 uses `*Done 2026-10-30; the scenarios live in ...*`. The `;` before the closing `*` breaks `*Done YYYY-MM-DD.*`.
- 529 ticked tasks in 15 files have no marker. 033 is one of them: its 6 ticked tasks carry only `Verify:` lines.
- Ticking T018-007 needs the stale "task stays open" text edited. That text is not an evidence block, so R3 forbids the edit.
- Nothing in R1-R8 enforces "a task with only a Partly done marker stays unticked". R7 accepts any marker, so a ticked task with only `*Partly done*` passes.
- Recommendation: a single marker regex that covers the real forms, and a rule that a ticked task needs a `Done` or `Evidence` marker.

**M5. Evidence can be deleted from an already-ticked task (R7, lines 376-377; R3 at 404-408).**
- R7 applies only to tasks newly ticked relative to the merge base.
- Input: base has `- [x] T1` plus `*Done ...* Evidence: ...`. The PR deletes the evidence lines and keeps the tick.
- R3 allows it because the removed lines were in an evidence block. R7 skips it because the task was ticked at the base.
- A bare `**Evidence (2026-12-03)**:` with nothing after it also satisfies "has an evidence block".
- Confidence: certain.
- Recommendation: a ticked task needs a non-empty block at the working tree. Evidence lines may not be removed from a ticked task.

**M6. The diff rule contradicts "Amending an approved doc" (lines 418-420 against 404-410).**
- The prose says a change of whitespace or Markdown layout keeps `Approved`.
- R3 says "Anything else fails" and the diff is plain `git diff --unified=0`, with no `-w` and no normalisation.
- A re-indented table or re-wrapped paragraph fails.
- Recommendation: either delete the exemption or specify the normalisation. Note that `-w` would also hide indentation changes that move lines in or out of a task item.

**M7. Tasks in unnumbered docs are unenforced, and ticking them breaks R3 (lines 336-337, 386-387, 371).**
- Unnumbered docs are "exempt from FR/SC/T ids". A task item requires a `T\d{3}-\d+` id.
- A checkbox without an id is not a task item. R5 and R7 never see it.
- In an Approved unnumbered doc, ticking it is a changed non-task line, so R3 fails. Progress can never be recorded without a re-review.
- This design's own Detailed Implementation Plan (items 1-26) has no checkboxes. The 26 deliverables get no tick or evidence trail, although `AGENTS.md` rule 17 and this doc's Goal 3 demand one.
- Recommendation: give unnumbered docs a task-id scheme (for example `T000-n`), or treat any checkbox line as a task.

**M8. OD row status is a gate predicate, but it changes over a row's life (lines 369, 373, 428-433).**
- The file's own convention rewrites status as work proceeds. Sections 2026-10-16 and 2026-10-17 go from "Decided; not built" to "Done (...)", "Closed" and "Waiting for the owner".
- Once the owner updates a cited row to "Done" or "Closed", every ticked task and every `Accepted:` citation of it fails R2 or R5, retroactively and permanently.
- The "status cell" is not located. Tables in the file have different columns: `| # | Item | Decision | Recorded in | Status |` and `| Item | Approved direction | Record / remaining work |` (no Status column).
- Duplicate ids are not forbidden. Two same-day branches will both mint `OD-YYYY-MM-DD-1` with no conflict.
- Recommendation: the gate should test "row exists" only, or accept a status whitelist (Decided, Done, Closed). Define the column by header name. Fail on duplicate ids.

**M9. Accepted findings are not bound to any OD row (lines 366-369 and 319).**
- R2 requires `sum(B+M) <= Accepted n` and that each listed OD id exists and is "Decided".
- Input: reports with `Counts: Blocker 3, Major 10, Nit 2`, and `Accepted: 13 findings; OD-2026-11-03-1`, pointing at any unrelated Decided row of this very PR. It passes.
- "Open findings: Blocker 0, Major 0" is checked for presence only.
- It is unclear which reports are listed. A `review_summary.md` from the synthesizer double-counts, and earlier-stage reports of a fix-and-rerun flow carry fixed findings.
- Recommendation: each accepted finding is cited as `report#id -> OD-id`, and the OD row names the reports it covers. Validate `Open findings` against the sums.

**M10. The Counts mapping cannot be produced by two of the three reviewers (lines 328-331 and 843-852; agent files read).**
- `architecture-reviewer.md` has no severity scale, only "Say which findings are blocking". The high/medium/low map has nothing to map from.
- `correctness-reviewer.md` reports confidence as "confirmed in code / plausible". The mapping uses certain/likely/plausible. The mapping gives Major or Nit only, so correctness can never raise a Blocker.
- Item 19 does not say to change those output formats.
- Approval hangs on these counts.
- Recommendation: define one severity scale for all three agents in item 19 and change their report formats.

**M11. The backticked-path check as specified fails on bare names, and the allowlist hides stale pointers (item 25, lines 888-899; section 8).**
- Tokens resolve "against the repository root". Real tokens in scanned files that fail:
  - `research.md` in about 8 files, `validation-report.md`, `quickstart.md`, `SKILL.md`, `openapi.yaml`, `workflow.schema.json`
  - `observability.rs`, `approval_telemetry.rs`, `scoped_compensation.rs`, `chain_progress.rs`
  - `storage-classification.json` (relative to `crates/infra-postgres/`), `CLAUDE.local.md`, the lone `/` in `CLAUDE.md:9` and `migration/SKILL.md:9`
  - `rediss://` (README:453), `registry.example.com/team/` (README:527), `contracts/journal-and-events.md`
- The implementer would have to allowlist generic basenames. A stale `tasks.md` or `spec.md` pointer, the very thing the check exists to catch, is indistinguishable from `research.md`.
- `git ls-files` is new to this script. The existing tests in `scripts/tests/test_check_agent_config.py` use plain `mkdtemp` fixtures, not git repos.
- Recommendation: resolve bare names by basename lookup, resolve relative to the containing file's directory, add `://` and `:line` to the exclusions, and decide behaviour outside a git repo.

**M12. The allocator misses local branches and breaks in worktrees (section 7, lines 466-470).**
- It reads the working tree and `origin/*` only. The repo has about 13 local-only branches, such as `feature/016-*` and `feature/014-bounded-execution`. `AGENTS.md` rule "Start here" says to run `git branch -a`, and the session hook already does.
- `.git/FETCH_HEAD` does not exist where `.git` is a file (a worktree). FETCH_HEAD is also per-worktree, so a fetch run in a sibling worktree is invisible.
- The script would either crash or always warn.
- Confidence: certain for the file layout, likely for the symptom.
- Recommendation: include `refs/heads/*`, and use `git rev-parse --git-path FETCH_HEAD` or the mtime of `packed-refs` and `refs/remotes/origin`.

**M13. The tests described for PR 1 will fail in CI (lines 683-686, 698-700, 722-726, 880-886).**
- Tests run `git commit` in temp repos. `ubuntu-latest` has no `user.name` or `user.email`. A developer's `commit.gpgsign=true` can also hang or fail them.
- Item 1's R7 tests also need an `origin/develop` ref.
- Recommendation: pass `-c user.name -c user.email -c commit.gpgsign=false` and `init.defaultBranch`, and create the remote ref with `update-ref`.

**M14. After PR 2, agents working on the 20 in-flight features are told to read documents that do not exist (item 13, lines 791-802).**
- `AGENTS.md` line 7, rule 1 and line 11 are rewritten to "the design doc `docs/designs/NNN-slug.md` of the active feature".
- For 35 features, including the 20 with 1,268 open tasks, no such doc exists until the follow-up design lands. Decision B says they "tick it by hand".
- Only `specs/AGENTS.md` (item 17) says so, and it loads only when working in `specs/`.
- No check covers ticks in `specs/*/tasks.md`.
- Rule 2 also changes "approved dependency" to "a dependency recorded in the design doc", which is weaker than the constitution sentence that is kept.
- Recommendation: keep a root-level sentence for unconverted features, and keep the word "approved".

**M15. Goal 1 is not checked by anything (lines 158-162, 378-379).**
- R8 checks that the eight headings exist and that "Enforced by" paths exist. Nothing compares the KEEP text with 3.0.0.
- The dispositions live in a prose table.
- The table counts a unit as "one heading or one bullet", but Governance and the Principles are paragraphs.
- Recommendation: put a per-unit hash of each KEEP unit in the Sync Impact Report and have R8 compare it with the constitution.

### Nit

- **N1. Line 35 contradicts the approval rule.** "The Status line reads Draft because nothing is built." Under section 3, Draft means the reviews are not clean.
- **N2. Decision count.** Item 12 (lines 783-789) says `OD-2026-11-03-1..12` hold the 2026-11-02 decisions plus A-L. Lines 72-82 list 8 owner decisions, plus 12 decisions A-L, which makes 20 rows. The rows are dated 2026-11-03 under a heading for that date but record 2026-11-02 decisions. The file's convention is newest-first, and item 12 appends at the end.
- **N3. Goal 9 (line 178) contradicts section 5 (line 433).** Goal 9 reads "cannot be ticked without it". Section 5 says a missing gate passes.
- **N4. Unrecognised Status values skip R2 and R3.** `Aproved`, `**Approved**` and `Approved 2026-11-05` are not `Approved`. The check "never skips a rule" (line 360) but silently skips here. Fail on any value outside Draft, Proposed, In Review or Approved.
- **N5. "Exactly one `Counts:` line" is undefined** (line 328). Is it anchored? Existing reports quote `**Counts:**` mid-bullet. Reports reviewing this doc will quote the format in blockquotes or fences. Anchor it to the full-line regex `^Counts: Blocker \d+, Major \d+, Nit \d+$`, outside fences.
- **N6. R8 "newest" sync-impact is undefined** (line 378). A string sort puts `10.0.0` before `4.0.0`. The 2.0.0 report is named `compensation-safety-sync-impact.md` and falls outside the glob.
- **N7. The status block parser is unspecified** (lines 342-353). The doc itself contains a second `**Status:**` block inside a fence in the body. Read fields from the header only.
- **N8. Rollback claim.** "PR 1 and PR 2 can each be reverted alone" (line 494) is false for PR 1 after PR 2, because `AGENTS.md`, the constitution and `verify` then cite a missing script.
- **N9. Decision B's claim (lines 95-96) leaves out local branches.** There are `bugfix/egm-documentation-bootstrap`, `bugfix/egm-open-task-corrections` and `bugfix/egm-readme-drift-corrections` on origin. Check them for overlap with PR 2's files.
- **N10. Review text must be committed first.** The doc never says the reviewed text must be committed before the review, so that `Reviewed-commit` equals the reviewed text.

Counts: Blocker 1, Major 15, Nit 10
