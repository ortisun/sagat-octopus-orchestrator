# Goldfish Review Report

**Target:** `docs/designs/constitution-4.0.0-egm.md` (`908-1216`, from `## Detailed Implementation Plan` to the end of the file; bounds confirmed by the synthesizer and by every reviewer)

**Mode:** REVIEW. **Stage:** 9, implementation readiness test. **Round:** 1.
**Repository checked against:** `<worktree>` (worktree of `develop` at `df59eb0e` plus design-doc commits that touch only `docs/designs/` and `tasks/reviews/`).
**Standard:** `~/.claude/plugins/cache/egm/egm/0.1.0/resources/document_standards/implementation-readiness-standard.md` (Zero-Decision Rule, phasing, Target Scope, Actionable Steps, Test Plan). The style lens has no standard.

## Verdict

The plan does not pass the standard. A competent engineer who does not know this design cannot execute it and produce the correct change on the first attempt. Eight Blockers sit in the rules that `scripts/check-design-docs.py` must enforce and in the order of the pull requests: as written, the new check cannot be coded without design choices, and several PRs would fail CI or fail the check they introduce. The file lists, line numbers and counts of the plan are mostly accurate (see "Checked and consistent"); the gaps are in the behavior the plan leaves to the implementer.

## Reviewer Status

| Reviewer | Status |
| --- | --- |
| `egm-implementation-readiness` | completed (two independent instances, A and B; both reported) |
| `egm-codebase-details` | completed (two instances on disjoint scopes: A on lines 908-1005, B on lines 1006-1216; both reported) |
| `egm-style` | completed (one instance) |

No reviewer failed, timed out or returned an empty report, so no retry was needed and none is recorded as `not run`.

## Severity breakdown

| Severity | Count | `readiness` | `codebase` | `style` |
| --- | ---: | ---: | ---: | ---: |
| Blocker | 8 | 8 | 0 | 0 |
| Major | 31 | 19 | 5 | 7 |
| Nit | 19 | 0 | 9 | 10 |
| Total | 58 | 27 | 14 | 17 |

Rules applied by the synthesizer:

*   Findings reported by more than one reviewer or instance are merged into one finding. Its severity is the highest any reviewer assigned, and the finding says where the ratings differed. No finding was lowered.
*   The id prefix is the lens that reported the finding at its highest severity (the readiness lens on a tie). `readiness`, `codebase` and `style` stand for `egm-implementation-readiness`, `egm-codebase-details` and `egm-style`.
*   "TP n" means "Technical Plan section n" of the TargetFile, which lies before the target scope. Line numbers without a file name are lines of the TargetFile. Evidence cited from the repository is the reviewers' unless the finding says the synthesizer confirmed it.

## Prioritized Findings (Blockers -> Major -> Nits)

### readiness-1-1. The `Reviewed-blob` fallback cannot be implemented as written - Severity: Blocker
*   **Id:** readiness-1-1
*   **Reviewer Lens:** `readiness` (instances A and B)
*   **Affected Scope:** `929-936` (item 1: reachability and the rebase, amend and shallow-clone tests); TP 3 and 4 at `410-411`, `427-428`, `477-483`
*   **Discrepancy Description**: `Reviewed-blob` is "git blob id of this doc at the reviewed commit" and lives in the doc's own status block. The fallback accepts a doc whose reviewed commit is no longer an ancestor "when the blob at this path equals `Reviewed-blob`". The author writes the block after the review (`421`), so the blob at `HEAD` always differs from the blob at the reviewed commit by at least the block, and by every tick afterwards. A doc cannot contain the id of its own blob. The fallback therefore never passes, and the rebase and amend tests named in item 1 have no outcome that can be written as "pass". Without the reviewed commit the check also has nothing to diff, so the plan does not say what happens to a ticked doc whose commit is unreachable. The synthesizer confirmed lines `410-411`, `427-428` and `481-482`.
*   **Rationale & Recommendation**: Define the stored value as a hash of the doc body below the status block with a stated normalization (or drop the fallback). State the outcome for each case: unreachable commit with an unticked doc, and with a ticked doc. Give the expected result of each of the three disposable-repository tests.

### readiness-1-2. The findings footer has no grammar and no producer - Severity: Blocker
*   **Id:** readiness-1-2
*   **Reviewer Lens:** `readiness` (A and B); also `codebase` (instance B, Major)
*   **Affected Scope:** `930-931` (item 1, "report hashes and footer counts"); `1091-1097` (item 22); `984-992` (item 8); TP 3 at `384-392`
*   **Discrepancy Description**: Item 22 says only "`id, reviewer, severity, status`". No delimiter or marker, id format, status vocabulary, position in the report or machine-readable shape (fenced block, table, JSON) is given, yet PR 1 parses it and PR 2 emits it, with no shared definition in PR 1. Further gaps:
    *   The severity map high/medium/low to Blocker/Major/Nit fits `security-reviewer` only (`.claude/agents/security-reviewer.md:19`). `correctness-reviewer.md:20` reports confidence and no severity (TP 389 maps it to Major, item 22 omits that, so it can never emit Blocker or Nit). `architecture-reviewer.md:19` says only "Say which findings are blocking".
    *   The three reviewers have `tools: Read, Grep, Glob` (line 4 of each), so they cannot write the `architecture.md`, `security.md` and `correctness.md` reports. The plan does not say who saves them or who appends the footer.
    *   The `egm-review` consolidated report comes from the pinned third-party plugin; no item names who appends its footer or how that fits "reports are immutable" (`422`).
*   **Rationale & Recommendation**: Put the footer grammar and a worked example in item 8 (`docs/designs/README.md` and `TEMPLATE.md`) and cite it from items 1 and 22. State the status values, who writes each report file, who appends the footer of the EGM report, and the severity map for all three reviewers.

### readiness-1-3. The `Owner-gate` pointer does not resolve against `docs/owner-decisions.md`, and no syntax exists for a decision not yet taken - Severity: Blocker
*   **Id:** readiness-1-3
*   **Reviewer Lens:** `readiness` (A and B); also `codebase` (instances A and B, Major)
*   **Affected Scope:** `930-931` (item 1); `1111-1117` (item 25); TP 5 at `491-498`; `docs/owner-decisions.md`
*   **Discrepancy Description**: The pointer is `#YYYY-MM-DD/<n>`, "the number of the row under that dated heading", and the script must check "its Refs column" and "a decided status". Against the real file:
    *   Dated headings repeat: `2026-11-01` heads five sections (lines 7, 19, 142, 154, 164), `2026-10-27` three (57, 125, 132), `2026-10-26` two (75, 89), `2026-10-29` two (33, 174). `#2026-11-01/2` is ambiguous. The plan's own example `#2026-10-29/1` (`464`) resolves to the ChildRoot O705 row (line 37), not to feature 018's entry (lines 174-190).
    *   No table has a `Refs` column (columns are `# | Item | Decision | Recorded in | Status` at line 13, and `Item | Approved direction | Record / remaining work` at line 25, which has no `#` column). Several sections are prose or bullets with no rows (142-190).
    *   Status is free prose ("Decided; not built", "Waiting for the owner", "Closed"), so "decided" has no vocabulary to test.
    *   A task waiting on a decision not yet taken cannot cite a row that does not exist. Conversions of 019-032, 035 and 036 are exactly that case.
    *   Item 25 adds "a new dated section" with no row layout, so the entries that gates and `Owner-accepted:` fields point at have no defined shape.
*   **Rationale & Recommendation**: Define the heading regex, the rule that numbers rows when dates repeat (an id column or an ordinal across same-date headings), the column that carries the reference, the status strings that count as decided, a way to reference a pending decision, and the format of an acceptance entry (`report-path#finding-id`, `392`). Say whether the old prose sections are addressable.

### readiness-1-4. The forward-only evidence rule fails every conversion that carries ticked tasks - Severity: Blocker
*   **Id:** readiness-1-4
*   **Reviewer Lens:** `readiness` (A Blocker; B Major, merged at the higher rating)
*   **Affected Scope:** `928-933` (item 1); `1131-1162` (PRs 17, 18, 20, 21); TP 4 at `436-437` and `455-460`
*   **Discrepancy Description**: The rule applies to "a task that is newly ticked in the diff against the merge base". A conversion PR adds the doc as a new file, so every moved tick can count as newly ticked. The sources do not carry the markers: of the 88 ticked tasks in 014, 018, 033 and 037, only 33 have a recognized marker on the task line (13 in 018, 20 in 037); 55 have none (37 in 014, 6 in 033, 12 in 037). 014 uses an `_Evidence: ..._` continuation line (44 occurrences in `specs/014-workflow-primitives/tasks.md`, confirmed by the synthesizer) that is not one of the three markers at `456-457`, although the text says they are "the forms used today". `specs/033-redis-streams/tasks.md:3-14` has no evidence at all. A "task" is one checkbox line in Terms (line 67), but the rule works on the whole list item (`455`), and 033 tasks wrap onto several lines. Read one way, PRs 17, 18, 20 and 21 fail; read the other way, the 88 ticks are never checked.
*   **Rationale & Recommendation**: State that tasks present in the frozen source are exempt from the rule and are checked only by `--conversion`. State how the rule treats a new file. Add `_Evidence:` as a recognized marker or require its conversion, and say which. Define a task as the whole list item.

### readiness-1-5. The duplicate-number rule fails every conversion PR - Severity: Blocker
*   **Id:** readiness-1-5
*   **Reviewer Lens:** `readiness` (B Blocker; A Major, merged at the higher rating)
*   **Affected Scope:** `924-936` (item 1); `984-992` (item 8); `1131-1162`; Goal 9 at `150-151`
*   **Discrepancy Description**: Goal 9 says the check "fails if two docs or a doc and a `specs/` directory share a number" (confirmed by the synthesizer). Item 8 (`987-988`) and PRs 3 to 22 create `docs/designs/NNN-slug.md` beside `specs/NNN-slug/` with the same number, and `NNN-slug.part-K.md` files share the index doc's number. Read literally, the check fails in PR 3.
*   **Rationale & Recommendation**: State the exemptions in item 1: same number and same slug, a `.part-K` file of the same doc, a superseded source. Add tests for each. State that the glob is `docs/designs/*.md`, so the `*.reviews/` reports are not read as docs.

### readiness-1-6. The extended backticked-path check turns CI red in PR 1, and its grammar and allowlist are undefined - Severity: Blocker
*   **Id:** readiness-1-6
*   **Reviewer Lens:** `readiness` (B Blocker; A Major; `codebase` instance A Major; merged at the higher rating)
*   **Affected Scope:** `967-972` (item 6); `994-998` (item 9); TP 8 at `617-621`
*   **Discrepancy Description**: Item 6 widens the scan to `README.md`, `SPECIFICATION-INDEX.md`, `specs/README.md`, `.claude/agents` and `.claude/skills` and runs it in CI in PR 1, which changes no text. The plan does not define a path, the base of resolution, how a directory matches `git ls-files` (which lists files), the allowlist file's name, location, format or first content, or how the existing tests (a `tempfile.mkdtemp()` root that is not a git repository, `scripts/tests/test_check_agent_config.py:20-22`) keep passing. Item 6 names only "placeholders, gitignored files and build outputs". Present spans that fit none of those or are not resolved by the plan:
    *   `README.md:527` (`registry.example.com/team/`, a registry host).
    *   `SPECIFICATION-INDEX.md:116` (`contracts/journal-and-events.md`, which exists only under `specs/018-schedules-bulk-operations/contracts/`) and `:141` (`P2/later`). Confirmed by the synthesizer.
    *   `docs/engineering/architecture-principles.md:31` (`rest/conformance.rs`, which exists only under `tests/contract/*/rest/`) and `:16` (a path without its `.rs` extension).
    *   `AGENTS.md:7` (`.specify/feature.json`, gitignored), `AGENTS.md:29` (`feature/<id>-<slug>`, `bugfix/<slug>`), `AGENTS.md:31` (`compose.observability.yaml`, which exists only under `infra/docker/`), `.claude/skills/adr/SKILL.md:9` (`origin/*`).
    *   All ten `speckit-*` skills, which cannot be edited because `.specify/integrations/claude.manifest.json` hashes them: `speckit-tasks/SKILL.md:25` (`.specify/extensions.yml`, which does not exist), `speckit-checklist/SKILL.md:146`, `speckit-converge/SKILL.md:143` (`US1/AC2`), and every `.specify/memory/constitution.md` path, which fails again in PR 2 when the file moves.
*   **Rationale & Recommendation**: State the path grammar, the resolution base, the directory rule, the allowlist file with its exact initial contents (or the PR 1 text edits that remove each hit), an explicit exemption for the `speckit-*` skills until PR 23, and how `check(root)` finds tracked files in a non-git test root (the fixtures must `git init` and `git add`). Add a note for sibling branches, which meet the wider scan on rebase.

### readiness-1-7. Open questions Q1 to Q5 gate items 1, 11, 12 and 25, and nothing marks them as owner actions - Severity: Blocker
*   **Id:** readiness-1-7
*   **Reviewer Lens:** `readiness` (B Blocker; A Major; `codebase` instance B Major; `style` Major on wording; merged at the higher rating)
*   **Affected Scope:** `166-187` (Open Questions); `928-957` (item 1: Q3 changes the status block that PR 1 parses); `1008-1023` (items 11 and 12); `1111-1117` (item 25)
*   **Discrepancy Description**: Item 11 sets the footer to "the version of Q5", item 12 names a file `constitution-4.0.0-sync-impact.md` and also says "the file name follows the version chosen in Q5" (4.0.0 or 3.1.0), and item 25 records "the owner's answers to Q1 to Q5 and to the contested findings", which do not exist (`168` says the answers are not applied in this design). An implementer cannot author the owner's answers into a register. Each answer changes the work:
    *   Q2 adds a `CODEOWNERS` file and a branch-protection setting; no item creates either, and the second is an owner action in GitHub.
    *   Q3 adds a `Scope-approval:` field to the status block, which is PR 1 grammar, so Q3 must be answered before PR 1.
    *   Q4 moves PR 23 and drops the transitional clause.
    *   Q5 renames the Sync Impact Report and the version. If the owner keeps 4.0.0, the unchanged Versioning clause (marked KEEP at `323`) still yields MINOR, so a MAJOR bump has no amended clause behind it.
*   **Rationale & Recommendation**: Mark items 1, 11, 12 and 25 as needing the owner's written answer first. State when each answer is needed (Q3 before PR 1; Q1, Q4, Q5 before PR 2), the default the draft applies, and what changes if the answer differs. Make the item 12 heading and file name agree with the chosen version, and record the answers under their own date.

### readiness-1-8. The PR 23 scoped search fails in its own pull request, and the work that would make it pass is not listed - Severity: Blocker
*   **Id:** readiness-1-8
*   **Reviewer Lens:** `readiness` (B Blocker; A Major, merged at the higher rating)
*   **Affected Scope:** `1164-1183` (items 28 and 29); `1008-1016` (item 11); `967-972` (item 6); Goal 4 at `133-138`; TP 8 and 9 at `614-616` and `637-639`
*   **Discrepancy Description**: Item 11 puts the transitional clause into `docs/constitution.md`, and the clause names the `/speckit-*` skills (`637-639`). `docs/constitution.md` is a live surface of Goal 4 (`136`), and the search turns on in PR 23. PR 23 lists only deletions, so the search fails on the constitution. Removing the clause is a normative amendment that needs a version bump and a Sync Impact Report (the version-line check requires the version to equal the newest report); none is listed. The scan also covers `scripts/`, and the search code and its tests must contain the two literals, so it matches itself. Item 6 says the search "stays off" in PR 1 and item 29 says it "turns on", but neither names the switch (constant, flag or list of surfaces), the file PR 23 edits, or a test for both states. The synthesizer confirmed Goal 4 and TP 9.
*   **Rationale & Recommendation**: List in PR 23 the constitution edit, its version and report, and any transitional wording removed from `AGENTS.md` and `CLAUDE.md`. Name the switch and the exclusions (the scanner's own files). Add tests for the off and on states.

### readiness-1-9. `check-design-docs.py` has no interface specification - Severity: Major
*   **Id:** readiness-1-9
*   **Reviewer Lens:** `readiness` (A and B); also `codebase` (instance A)
*   **Affected Scope:** `924-936` (item 1); `994-998` (item 9); TP 3 at `421-442`
*   **Discrepancy Description**: Only `--conversion NNN` is named. Missing:
    *   Flags, exit codes and message text. The model scripts take a root or `--metadata FILE`, print `error: ...` and exit 1 (`scripts/check-agent-config.py:203`), and the tests assert on message fragments. The plan does not say how tests point the script at a disposable repository (root argument or working directory).
    *   The base for "the merge base with the target branch". CI runs on `push` to `main`, `develop`, `release/**` and `hotfix/**` and on any `pull_request` (`.github/workflows/ci.yml:3-6`); the `architecture` job passes no `env`. No variable (`GITHUB_BASE_REF`), default ref (`origin/develop`) or push behavior is named.
    *   What counts as a design doc. `docs/designs/constitution-4.0.0-egm.reviews/2026-11-02/*.md` are Markdown without a status block; only `TEMPLATE.md` and `README.md` are skipped, "by name" (which could be read as skipping every `README.md`).
    *   The grammar of the status block: whether `Owner-accepted:` repeats or takes a list, how `Review-reports:` and `Repo-reviewer-reports:` separate path and hash, where the block ends. The block at `406-419` is an illustration.
    *   Where the minimum Claude Code version `2.1.271` is stored (script constant or JSON).
    *   How "the newest Sync Impact Report" is chosen: `docs/governance/` holds `constitution-3.0.0-sync-impact.md` and `compensation-safety-sync-impact.md`, and PR 2 adds `sync-impact-rust-sdk-2026-10-27.md`; only one name carries a version.
    *   Which paths count as "Enforced by" paths in the constitution.
*   **Rationale & Recommendation**: Add an interface subsection to item 1: CLI, exit codes, message format, base-ref resolution for pull request, push and local runs, the doc glob, the status-block grammar, the location of the version floor, and the report-name pattern.

### readiness-1-10. The required-section list and the Check-row matching rule are not stated, and this design doc may fail its own check - Severity: Major
*   **Id:** readiness-1-10
*   **Reviewer Lens:** `readiness` (B)
*   **Affected Scope:** `927-928` (item 1); `984-992` (item 8); TP 3 at `394-401`
*   **Discrepancy Description**: The check "requires" the added sections, but no list exists. The design doc under review has no heading "Owner decisions needed" (it appears only in prose at `399`; the synthesizer confirmed), and has "Open Questions" in its place. Item 8 adds "Requirements" to the template, while `399-400` exempts an unnumbered doc from the Requirements section. Heading level and Check-row matching are unspecified: the table at `694` writes "I. State and retry semantics are explicit and durable", the constitution heading is "I. State and Retry Semantics Are Explicit and Durable" (`.specify/memory/constitution.md:5`; the synthesizer confirmed). Whether `Draft` docs are checked is not stated.
*   **Rationale & Recommendation**: Give the exact headings per kind of doc (numbered, unnumbered, part), shared by `TEMPLATE.md` and the check; the row-match rule (numeral only, or numeral plus normalized title); and whether Draft docs are checked. Run the rule on this doc.

### readiness-1-11. The coverage rule and the `research.md` append-only rule cannot be coded from the text - Severity: Major
*   **Id:** readiness-1-11
*   **Reviewer Lens:** `readiness` (A and B)
*   **Affected Scope:** `930-933` (item 1); TP 3 and 10 at `433-435` and `653`
*   **Discrepancy Description**: Real tasks cite requirements in several compressed forms: `FR-014-001–017` with an en dash (`specs/014-workflow-primitives/tasks.md:12`), `FR-014-003,005,007–008` (`:14`), `FR-019-001..005, 012, 027` and `Refs: all FR` (`specs/019-worker-deployments-dispatch/tasks.md:23,15`). "Every FR and SC is cited" needs one range-expansion rule, a rule for "all FR", and the definition-site pattern for FR, SC and NFR ids. The rule applies to docs "converted after the first conversion", but nothing says how the check learns which docs those are. The `research.md` ids come in three layouts and two schemes (`| D123 |` rows, `- **D1040 — ...**` bullets in `specs/019.../research.md:77`, `D035-001` in `specs/035.../research.md:11`, `D1702` in 036), so the append-only check needs an id grammar, a row-boundary rule, and a definition of "renumbered" versus an allowed status update.
*   **Rationale & Recommendation**: State the grammar and the expansion rule in item 1 with fixtures taken from real files, and the marker that tells the check which docs are subject to the rule. Say whether PR 1 exits 0 on the 35 existing specs.

### readiness-1-12. The `--conversion NNN` contract is undefined - Severity: Major
*   **Id:** readiness-1-12
*   **Reviewer Lens:** `readiness` (A and B)
*   **Affected Scope:** `932-936` (item 1); `1131-1137`; TP 7 at `566-570`
*   **Discrepancy Description**: Missing: the banner's exact text, position and parse rule; the commit "X" (it cannot be the commit that adds the banner, so the plan must say which commit and that it stays reachable); what "body" means for "bodies change after X"; the task-text normalization (re-wrapping, whitespace, checkbox state, evidence lines, `Owner-gate:` lines, `[P]` and `[US01]` markers); the format of "amendments listed in the PR" (a CI check cannot read a PR description); how the check knows which conversion is the first; what it compares for a split doc; and where the result of this one-off comparison is recorded.
*   **Rationale & Recommendation**: Give the banner regex and placement, the definition of X, the normalization function, an amendments file or flag, and fixtures.

### readiness-1-13. The test plan names categories, not cases, and the failing-first discipline is not checkable - Severity: Major
*   **Id:** readiness-1-13
*   **Reviewer Lens:** `readiness` (A)
*   **Affected Scope:** `924-936`, `938-944`, `959-972`, `974-982`, `1194-1212`
*   **Discrepancy Description**: "One passing case and one failing case for each rule" covers about 20 rules, but only the evidence rule (`469-470`) and reachability have named cases. Inputs and outcomes (pass or fail, message) are absent. Boundary cases are not stated: Claude Code 2.1.270 against 2.1.271, a report hash that differs by one byte, a count that differs by one, an owner-accepted finding with no register row, a gate pointing at a non-decided row, a task prefix that differs from the doc number. "Seen failing before the rule is written" (`936`, `1200`) is a sentence, not a procedure. The hook (`965`) and `check-agent-config.py` (`972`) cases are listed by theme only.
*   **Rationale & Recommendation**: List each rule with its inputs and expected outcome. Make failing-first checkable, for example one commit per rule with the test first, or a table in the PR with the failure output seen before the rule.

### readiness-1-14. The conversion mapping and the disposition of every artifact type are not specified - Severity: Major
*   **Id:** readiness-1-14
*   **Reviewer Lens:** `readiness` (A and B); also `codebase` (instance A on item 8, instance B on the unlisted files)
*   **Affected Scope:** `984-992` (item 8); `1131-1162`; `1185-1192`; TP 7 at `548-571`
*   **Discrepancy Description**: The only mapping is one sentence (spec to Overview, Problem, Goals, Non-Goals, stories as CUJs and requirements; plan to Technical Plan and Check; tasks to the Detailed Implementation Plan). Not stated: whether text is copied or rewritten; where SC and NFR ids, edge cases, key entities, assumptions and clarifications go; whether sections with no source (Alternatives, Risks, Non-Functional Requirements) are required and who writes them; what happens to the non-task parts of `tasks.md` (dependencies, parallel opportunities, coverage tables, checkpoints); and whether a conversion PR merges as `Draft` or must be `Approved` (the plan expects ESCALATE findings, so a stalled PR would leave a `Draft` doc beside a bannered `tasks.md`).
    *   Item 8's artifact table lists seven artifacts. `specs/AGENTS.md:3` lists ten (adds `data-model.md`, `analysis.md`, `validation-report.md`), and 014 and 037 have an `evidence/` directory. TP 7 (`563-565`) names `data-model.md`, `evidence/` and `validation-report.md` as staying in `specs/`; item 8 does not say whether its table is exhaustive.
    *   The "read and not changed" list (`1187-1190`) is closed. `specs/014-workflow-primitives/` has 25 other Markdown files (3,575 lines) outside it: `implementation-ledger.md`, `analysis.md`, six `capacity-*.md`, nine `scoped-event-*.md`, four `release-upgrade-*.md`, `control-entry-admission.md`, `scoped-timer-runtime.md`, `zero-iteration-policy.md`, `adr-proposal-structured-primitives.md`. `docs/owner-decisions.md:37-38` links `capacity-coordinator.md` as the "Coordinator plan". Also unlisted: `analysis.md` in 016 and 037, `contracts.md` and `released-callback-cutover.md` in 033, and the `examples/` directory of 016 (read by `tests/e2e/payload-classification/quickstart.rs:17`).
    *   The "14,004 lines of markdown in 42 files" figure for 014 is correct, but `spec.md`, `plan.md` and `tasks.md` total 1,725 lines; `validation-report.md` (4,385) and `research.md` (1,820) stay live. "Split by increment" is justified by directory size, not by the size of the conversion input; 014 tasks are grouped by Phase, not by plan increment.
*   **Rationale & Recommendation**: Put a heading-level mapping table and a disposition for every artifact type (including those above) in `docs/designs/README.md` (item 8) and cite it from the PR 3-22 block. State the split rule against the real structure, and the merge state of a conversion PR.

### readiness-1-15. Which tasks carry an `Owner-gate` in a conversion is left to the executor - Severity: Major
*   **Id:** readiness-1-15
*   **Reviewer Lens:** `readiness` (B)
*   **Affected Scope:** `1131-1162`; TP 7 at `583-586`
*   **Discrepancy Description**: A task that depends on a scope decision carries a gate (`585`), with no criterion. For 019-032, 035 and 036 the scope is unapproved, so arguably every task depends on a decision; `specs/019-worker-deployments-dispatch/tasks.md:15` shows one task that names owner answers and the rest do not. The entries mostly do not exist and the grammar cannot point at a future row (readiness-1-3). Adding a gate line changes the task's text, which the per-task hash then flags (readiness-1-12).
*   **Rationale & Recommendation**: State the selection rule (for example, tasks that cite an open `O` row of the feature's `research.md`), where the gate points, and that gate lines are excluded from the hash.

### readiness-1-16. "Updates its row in `docs/traceability.md` and `SPECIFICATION-INDEX.md`" refers to rows that mostly do not exist - Severity: Major
*   **Id:** readiness-1-16
*   **Reviewer Lens:** `readiness` (A and B); also `codebase` (instance B)
*   **Affected Scope:** `1062-1066` (item 18); `1133-1135`
*   **Discrepancy Description**: `docs/traceability.md` has a feature table only for 001-012 and 016 (lines 5-19), sections for 014 and 037 (lines 402, 1108), and nothing for 018-032 and 035-036 (033 appears inline at 1048-1063). `SPECIFICATION-INDEX.md` has rows for 014 and 033 in "Feature records" (lines 24, 27) and a paragraph for 037 (32-39), which has no Status column; rows for 018-032 sit in another table with columns Feature, Scope and Proposed ADRs (88-104); 035 and 036 do not appear. `docs/roadmap.md:22-23` carries a status row and calls `tasks.md` the live progress record, and no item names it.
*   **Rationale & Recommendation**: State per feature which file and table receive a row, its columns, the status text and the link targets; say whether rows are created or updated; add `docs/roadmap.md` or give the reason it is out of scope.

### readiness-1-17. Conversion preconditions, ordering, sizing and owner actions are not procedural - Severity: Major
*   **Id:** readiness-1-17
*   **Reviewer Lens:** `readiness` (A and B); also `style` (Major, wording of the PR 23 precondition); also `codebase` (instance B, Nit on the 036 worktree)
*   **Affected Scope:** `1131-1162`; `1182-1183`; `1211-1212`; TP 7 and 9 at `573-600` and `635`
*   **Discrepancy Description**:
    *   No command proves "no open branch still ticks its `tasks.md`" (CUJ 5). Only 018 and 035 carry a "convert after the branch merges" precondition; 014, 036 and 037 are described as in flight or having worktrees and carry none. The table lists 037 and 036 before 014 and 018, while `1136-1137` says features with open branches come last.
    *   The line ceiling "is set after PR 3" (`989`) with no criterion and no PR that edits `docs/designs/README.md`; PR 20 (014) depends on it.
    *   PR 3 "records" cost and the stop thresholds (27 runs, 10 acceptances) are "proposals for the owner to set" (`598-600`); no file, place or actor is named, and the thresholds must be set before PR 3 ends.
    *   PR 23 waits for sibling branches "to have rebased" (`1182`, `635`): no actor, base, check or completion test.
    *   Owner or external actions other than Step 0 and the stop decision are unmarked: the Claude Code plugin install and marketplace trust, the merge method, acceptance of contested findings.
*   **Rationale & Recommendation**: Add the branch-detection command and a precondition per PR; key the split rule to Phase or `[US]` labels; give the ceiling procedure; name the file for the PR 3 record; mark the thresholds, the plugin trust and the merge method as owner actions; state PR 23's entry condition as a testable statement with an actor.

### readiness-1-18. The id allocator (item 7) has no algorithm, inputs or output format - Severity: Major
*   **Id:** readiness-1-18
*   **Reviewer Lens:** `readiness` (A and B); also `codebase` (instance A)
*   **Affected Scope:** `974-982` (item 7); TP at `346-350`
*   **Discrepancy Description**: "Next free number" is undefined. `SPECIFICATION-INDEX.md:78-79` says missing numbers are not available by default (013 and 015 are taken elsewhere), so the rule must be maximum plus one, not the lowest gap. The inputs listed are the `specs/` and `docs/designs/` directories; used D, G, O and ADR numbers live in `research.md` files and `docs/adr/`, in three id forms (`G1000` in `specs/018.../research.md:327`, `D035-001`, unprefixed `D714`/`O705` in 014). The prose to turn into a table (`SPECIFICATION-INDEX.md:136-137`) reads "ADR-0050..ADR-0068; D/G/O 1000–1599, overflow 1600–1674": one shared D/G/O pool, not separate `D`, `G`, `O` blocks as `980` states. Reserved ranges are also recorded elsewhere (for example `specs/036-elastic-scaling/research.md:3` reserves D1700-D1739 and G1700-G1718). The table columns, the output format and the source of the "last fetch" time (`FETCH_HEAD` lives in the common git directory in a worktree) are not given.
*   **Rationale & Recommendation**: State the algorithm, the sources per number kind, the table schema with its seeded rows, the output format and the fetch-time source. Correct the description of the existing prose.

### readiness-1-19. The hash script and the accepted-hash list leave choices to the implementer - Severity: Major
*   **Id:** readiness-1-19
*   **Reviewer Lens:** `readiness` (A and B); also `codebase` (instance A, two Major findings merged)
*   **Affected Scope:** `938-950` (items 2 and 3); TP 6 at `516-529`
*   **Discrepancy Description**:
    *   TP 6 gives an inclusion list (`.claude-plugin/`, `agents/`, `resources/`, `SKILL.md` and `references/` per skill, `docs/claude.md`, `docs/harnesses.md`) and an exclusion list (`TEST.md`, `test_scenarios.yaml`, `assets/`, `site/`, `tests/`), which give different digests unless one rule is chosen. The installed copy at `~/.claude/plugins/cache/egm/egm/0.1.0` has content in neither list: `distributions/claude/` (a generated runtime copy with its own agents, skills and docs), a root `plugin.json`, `.agents/`, `skills/egm-review/agents/openai.yaml`, `scripts/`, `pyproject.toml`, `TESTING.md`.
    *   Test case 4 names `TEST.md` as outside the set; it exists only as `skills/<name>/TEST.md` (the root file is `TESTING.md`), so under an inclusion rule the case passes trivially.
    *   The digest is undefined: hash function, how paths and file hashes are joined, separators, line endings, symlinks, modes, and the printed form (bare hex or prefix).
    *   `egm-accepted-hashes.json` has no schema (top-level shape, key names, hash and date formats). The plugin commit is written `f0ba151b`; the full value is in `~/.claude/plugins/installed_plugins.json:46` (`f0ba151b252c9abf1318d9fe857a43265d06de89`), not in the plugin directory (which has no `.git`). Whether 8 or 40 characters are stored is open, and nothing tells the implementer to read that field, stop if it differs, and hash the cache copy rather than the marketplace clone. The install is user-scope on the owner's machine, so another account yields another hash.
*   **Rationale & Recommendation**: Fix the set as one rule, the byte layout, the output form and the JSON schema, with a worked example. Add an owner or implementer step that verifies the commit before the first hash is recorded. Correct test case 4.

### readiness-1-20. The `.claude/settings.json` change (item 4) is not exact, and its permission entry has an unresolved owner choice - Severity: Major
*   **Id:** readiness-1-20
*   **Reviewer Lens:** `readiness` (A and B); also `codebase` (instance A)
*   **Affected Scope:** `952-957` (item 4); Non-Functional Requirements at `874-876`
*   **Discrepancy Description**: Neither key exists in the file today (confirmed), so "Add" is right, but the JSON is not given: the nesting of `source`, `repo` and `autoUpdate` inside `extraKnownMarketplaces.egm`, the value of `enabledPlugins["egm@egm"]` and the placement (the file uses 2-space indentation and the order `$schema`, `env`, `permissions`, `hooks`). A misplaced `autoUpdate` may be ignored without error, and no check or test parses the result. The Non-Functional Requirements say the owner may skip the `permissions.allow` entry, while item 4 makes it mandatory; `AGENTS.md` rule 20 says not to widen permissions. Existing entries are exact strings (`Bash(python3 scripts/check-architecture.py)`, `.claude/settings.json:13-14`), so the copy would still prompt for `--conversion NNN`. Enabling a third-party plugin in a committed project file needs each user to trust the marketplace and install the plugin, which an agent cannot do; no item assigns that to the owner.
*   **Rationale & Recommendation**: Give the literal JSON and its position. Mark the permission entry as an owner decision with a default and say whether `--conversion` is covered. Assign the trust and install step to the owner or users. Add a parse test or a `check-agent-config.py` check.

### readiness-1-21. The session-start hook change (item 5) leaves its output, pattern and tests loose - Severity: Major
*   **Id:** readiness-1-21
*   **Reviewer Lens:** `readiness` (B); also `codebase` (instance A)
*   **Affected Scope:** `959-965` (item 5); TP 8 at `611-613`; `.claude/hooks/session-start.py`
*   **Discrepancy Description**: Today the hook prints `Branch:`, then `Active feature (.specify/feature.json): …` (`session-start.py:27`), then `Other branches mention …` (`33-35`), then a disk line (`36-38`). The plan does not say what the "Active feature" line becomes, what a numbered branch prints, or the exact message. The branch pattern is not given (`feature/NNN-slug` only, or `bugfix/NNN-`; digit width; the current regex is `specs/(\d{3})-`, line 28). "Keep the warning" keeps a quirk: the test `fid in b and b != branch` over `git branch -a` makes a pushed `feature/014-x` warn about its own `origin/feature/014-x`, so the test "a branch that other branches mention" needs that decided. The disk warning fires below 20 GiB, so a "no warning" test that asserts empty output can fail on a loaded machine. A subprocess test needs a repository with at least one commit (an unborn `HEAD` breaks `rev-parse --abbrev-ref HEAD`), the working directory set to it, and `CLAUDE_PROJECT_DIR` if used.
*   **Rationale & Recommendation**: State the message text, the regex, the treatment of `origin/` copies of the current branch, and assert on substrings, not on empty output.

### readiness-1-22. After PR 2, pointer text conflicts with the transitional clause for features not yet converted, and item 20 gives no replacement text - Severity: Major
*   **Id:** readiness-1-22
*   **Reviewer Lens:** `readiness` (B); also `codebase` (instance B); also `style` (Major, wording of item 20)
*   **Affected Scope:** `1036-1054` (items 15, 16); `1075-1082` (item 20); `1099-1109` (items 23, 24); TP 9 at `637-639`
*   **Discrepancy Description**: PR 2 rewrites `AGENTS.md` rule 1 and line 11, `CLAUDE.md` line 10, the `verify` and `parallel-agents` skills and the PR template to say "the design doc". Design docs exist only per conversion (PRs 3 to 22) and never for the 15 features with no open task (Non-Goals, `159`), while the transitional clause keeps `tasks.md` canonical and lets features use `/speckit-*` until PR 23. After PR 2, `AGENTS.md` line 9 and `CLAUDE.md` line 7 no longer mention those skills. No item gives wording for an unconverted or never-converted feature (for example `docs/engineering/sdd-workflow.md` "Changing something that exists", line 42, says "amend the spec in the same pull request"). Item 20 says only "Retitled for design docs" (no title), "are adapted" (no change) and "`tasks.md` read as ..." (replace or annotate?).
*   **Rationale & Recommendation**: Give the exact replacement text for each edited line, including the dual form for unconverted features ("or `specs/NNN/tasks.md` until converted"), say who removes it in PR 23, and give the new title and the edit for each section of item 20.

### readiness-1-23. Between PR 2 and PR 23, the Spec Kit skills read the constitution from a path that no longer exists - Severity: Major
*   **Id:** readiness-1-23
*   **Reviewer Lens:** `readiness` (A)
*   **Affected Scope:** `1008-1016` (item 11); `1164-1183`; TP 9 at `637-639`
*   **Discrepancy Description**: The transitional clause lets features keep using the skills. They load the constitution "IF EXISTS" at `.specify/memory/constitution.md` (`speckit-plan/SKILL.md:62`, `speckit-specify/SKILL.md:117`, `speckit-tasks/SKILL.md:65`). After PR 2 the file is gone, so the skills continue without it, and `speckit-constitution/SKILL.md:129` would recreate a stale copy at the old path.
*   **Rationale & Recommendation**: State what the implementer does: edit the ten paths (they are hashed by `integrations/claude.manifest.json`), leave a stub at the old path, or accept the gap as a recorded risk.

### readiness-1-24. Item 11 supplies no clause text, and its footer, links and KEEP rows are incomplete - Severity: Major
*   **Id:** readiness-1-24
*   **Reviewer Lens:** `readiness` (A and B); also `codebase` (instance B); also `style` (Major on the "Compliance review" wording, see style-1-2)
*   **Affected Scope:** `1008-1016` (item 11); TP 1 at `307-337`
*   **Discrepancy Description**: Item 11 asks for new or rewritten clauses (the EGM procedure, the approval rule, the owner-decision clause, the transitional clause, the EGM pin rule) and gives no text. Goal 1 defines an obligation as a statement that uses MUST, so which sentences use MUST is a decision. The footer at `.specify/memory/constitution.md:225` has two relative links (`../../docs/governance/constitution-3.0.0-sync-impact.md` and `../../docs/governance/compensation-safety-sync-impact.md`) that resolve outside the repository after the move to `docs/`; "the link at its end is corrected" names neither and does not say whether the first now points at the 4.0.0 report. Nothing would catch a missed link (`check-agent-config.py:188-189` does not scan `docs/constitution.md`). The existing sentence "Adopted on 2026-11-01 by the owner's instruction to merge the pull request that carries this text" has no replacement, and "the merge date" cannot be written in the PR that is merged. The disposition row "Development Workflow: CI gates" is KEEP but "gains the design-doc CI check" (TP 1), which item 11 does not mention; a bullet that gains text is not KEEP.
*   **Rationale & Recommendation**: Supply the clause text (or an authoritative draft in this doc), the exact footer wording with a date source that exists at merge time, both link targets, and the corrected disposition of the CI-gates row.

### readiness-1-25. Item 12 leaves the unit granularity and the report format open - Severity: Major
*   **Id:** readiness-1-25
*   **Reviewer Lens:** `readiness` (A and B); also `codebase` (instance B)
*   **Affected Scope:** `1018-1023` (item 12); Goal 1 at `121-125`; TP 1 table at `311-325`
*   **Discrepancy Description**: Item 12 says "one row per unit of 3.0.0". Goal 1 defines a unit as one heading or one bullet; the TP 1 table groups "Core Principles I to VIII", "Delivery Constraints" and four Governance paragraphs into single rows, and Governance consists of bold-lead paragraphs, not bullets (`.specify/memory/constitution.md:202-221`). Finer units (the two bullets inside Principle V, lines 95-103, and four Delivery Constraints bullets) are not listed. The 3.0.0 report format is `Old unit | Action | Result` with its own action vocabulary (`constitution-3.0.0-sync-impact.md:30-37`) and has a "Scope and why MAJOR" section (line 13) that item 12 omits, although that justification is what Q5 contests. "The file name follows the version chosen in Q5" repeats readiness-1-7.
*   **Rationale & Recommendation**: Say whether the report has one row per unit or the grouped rows of TP 1, define the unit for Governance paragraphs, give the table columns, and add the "why MAJOR (or MINOR)" section.

### readiness-1-26. PR 2 pointer edits miss lines that still name the moved constitution or the old workflow - Severity: Major
*   **Id:** readiness-1-26
*   **Reviewer Lens:** `readiness` (A Major; B Nit); also `codebase` (instances A and B, Nit)
*   **Affected Scope:** `1036-1054` (items 15, 16), `1068-1073` (item 19), `1075-1082` (item 20), `1091-1097` (item 22), `1105-1109` (item 24); `1000-1004` (item 10)
*   **Discrepancy Description**: Lines not covered by any item:
    *   `AGENTS.md:10` ("look at other open branches before taking the next number"), which TP at `346-350` replaces with `scripts/next-feature-id.py`; `specs/AGENTS.md:8` and `docs/engineering/sdd-workflow.md:19` carry the same wording.
    *   `.github/pull_request_template.md:12` lists the two existing checks and not `check-design-docs.py` (item 24 does not add it).
    *   `scripts/README.md:9-12` and `30-35` (inventory of the fitness scripts) and the script table at `README.md:684-693`; `AGENTS.md` rule 10 requires documentation in the same delivery.
    *   `.claude/agents/architecture-reviewer.md:3` ("when a `plan.md` Constitution Check is written") and `:17` ("without an approved dependency", which rule 2 is changing); `security-reviewer.md:3`; `correctness-reviewer.md:17-18` ("Spec drift"). Line 10 differs per file and `correctness-reviewer.md:10` has no `.specify` path to replace (it says "Read the constitution"), so "line 10 of each ... instead of `.specify/memory/...`" does not apply to it.
    *   `docs/engineering/ai-engineering.md:31` ("re-read the active `tasks.md`", the wording `CLAUDE.md` line 10 changes) and `README.md:248` (links `specs/014-workflow-primitives/tasks.md` as the open feature tasks).
    *   A conversion PR has no step to retarget links from live docs to the design doc.
*   **Rationale & Recommendation**: List every line to edit and give the new sentence for each. The backticked-path check will catch the path cases, but only after readiness-1-6 is fixed; the prose cases need the list.

### readiness-1-27. The Verification Plan lacks the specifics needed to run it - Severity: Major
*   **Id:** readiness-1-27
*   **Reviewer Lens:** `readiness` (B Major; A and `codebase` instance B Nit); also `style` (Major, passives that hide the actor)
*   **Affected Scope:** `1194-1212`
*   **Discrepancy Description**:
    *   "CUJ 1 to 5 are run once each on a scratch branch" (`1207`): CUJ 1 is a full five-stage review (at least 15 EGM runs plus three repository reviewer runs). No scratch doc, runner, cost or PR after which they run is named; CUJ 5 does not say which feature it converts.
    *   "PR 3 records ..." and "the stop condition ... is applied before PR 4" (`1211-1212`): a PR does not record anything; the file and the actor are not named (see readiness-1-17).
    *   The quickstart command (`1205`) omits `--locked` (used at `AGENTS.md:44`), the Docker requirement, the `DOCKER_HOST` note from `AGENTS.md` and `tests/AGENTS.md`, and the filter and `-- --test-threads=1` that `specs/016-payload-classification/quickstart.md:23` documents. The 014 quickstart edit has no verification step.
    *   The `architecture` CI job also runs `check-capacity-activation.py`, `check-upgrade-boundary.py` and `check-vendor.py` (`ci.yml:173-175`); the "all exit 0" list (`1202-1203`) names three scripts. "Scoped search" has no command or exit code of its own.
*   **Rationale & Recommendation**: Name the scratch doc, the runner, the expected output and the file for the PR 3 record; give the full commands; align the script list with the CI job.

### codebase-1-1. Step 0 claims that the evidence manifests of 014 and 037 record the constitution hash "at a base commit"; 014's do not - Severity: Major
*   **Id:** codebase-1-1
*   **Reviewer Lens:** `codebase` (instance A)
*   **Affected Scope:** `917-921` (Step 0)
*   **Discrepancy Description**: The sentence names no file or commit. 037 holds: `specs/037-compensation-safety/evidence/resume-tree-manifest.json:2,5` and `d1-source-tree.json:2,7` give base `6b6f0fa4…` with the constitution hash, and the `develop` reflog shows a fast-forward to that commit. 014 does not: `specs/014-workflow-primitives/evidence/released-v02-20261027/current-source.json:146-148` (and the same file in the `applied` and `inflight` directories) is a bare array of path and sha256 with no commit. The only 014 commit is `currentCommit` `33ab9da5…` in the sibling `build.json:24`, which `implementation-ledger.md:1377` calls the "compiled parent" of a "dirty fixture input", so the manifest hashes a dirty working tree, not a base commit. `33ab9da5` appears in no ref log or packed-refs the reviewer could read, so its presence on `develop` is unproven. The synthesizer confirmed the manifest and `build.json` lines. The sentence is the plan's reason why the tag is optional.
*   **Rationale & Recommendation**: Cite the files, state the 037 base commit, and reword the 014 clause (its manifest hashes a working tree and names its commit in `build.json`), or drop the clause.

### codebase-1-2. Item 15 omits the constitution path on `AGENTS.md` line 3 - Severity: Major
*   **Id:** codebase-1-2
*   **Reviewer Lens:** `codebase` (instance B); also `readiness` (A)
*   **Affected Scope:** `1036-1046` (item 15)
*   **Discrepancy Description**: The item says "Line 3 drops '(Spec Kit)'". `AGENTS.md:3` also reads "The constitution (`.specify/memory/constitution.md`) outranks this file", a path that is dead once item 11 moves the file. Items 15 and 17 list the other `.specify` mentions (line 7, rule 1 at line 16, `README.md:681`, `README.md:721`) but not this one, so the implementer finds it only when the new backticked-path check fails.
*   **Rationale & Recommendation**: Add the path on line 3 to the item.

### codebase-1-3. Item 21 names "its section on plugins as supply chain" in `ai-engineering.md`, which does not exist - Severity: Major
*   **Id:** codebase-1-3
*   **Reviewer Lens:** `codebase` (instance B Major; `readiness` A and B Nit)
*   **Affected Scope:** `1084-1089` (item 21)
*   **Discrepancy Description**: `docs/engineering/ai-engineering.md` has sections 1 to 7 and "Sources". The only supply-chain text is one bullet in "6. Security of agentic work" (line 50: "MCP servers, skills, hooks and plugins are supply-chain: pin and review them like dependencies"). The synthesizer confirmed the heading list. The plan does not say whether the EGM pin text extends that bullet or becomes a new bullet or section. Line 63 does carry the Spec Kit reference.
*   **Rationale & Recommendation**: Name the target (the bullet at line 50 or a new bullet in section 6) and give the sentence to add.

### codebase-1-4. Item 27 (quickstarts): the calls span two lines, the prose around them describes the removed command, and two `analysis.md` files repeat the call - Severity: Major
*   **Id:** codebase-1-4
*   **Reviewer Lens:** `codebase` (instance B Major; `readiness` A and B Nit)
*   **Affected Scope:** `1124-1129` (item 27); Verification Plan at `1205`
*   **Discrepancy Description**: The cited lines are right (`specs/014-workflow-primitives/quickstart.md:23`, `specs/016-payload-classification/quickstart.md:46`), but each call starts one line earlier with the `SPECIFY_FEATURE_DIRECTORY=...` line (lines 22 and 45), so replacing only the cited line leaves a dangling variable. In 016 the prose at line 35 ("The specification prerequisite command below passed") and lines 51-53 ("The first command lists the documents the feature has") describes the removed command. "A plain `ls` of the files it checked" gives no command or file list. The same call appears in `specs/014-workflow-primitives/analysis.md:30` and `specs/016-payload-classification/analysis.md:109`, which are neither under Modify nor under "read and not changed". The test `tests/e2e/payload-classification/quickstart.rs:174-193` acts only on commands that start with `sagat` or `curl`, so the edit is safe for it; the 014 edit has no verification.
*   **Rationale & Recommendation**: Give the exact replacement lines for both files, the prose changes for 016, and the disposition of the two `analysis.md` files.

### codebase-1-5. The disposable-git-repository tests lack fixture rules - Severity: Major
*   **Id:** codebase-1-5
*   **Reviewer Lens:** `codebase` (instance A)
*   **Affected Scope:** `934-936` (item 1); `994-998` (item 9)
*   **Discrepancy Description**: The only test in `scripts/tests` that touches git is `test_release_upgrade_profile.py:69`, which runs `git init -q` and `git add` and never commits. The new tests need commits, rebases, amends and shallow clones. Nothing says how they set a commit identity (the `architecture` job at `ci.yml:165-176` configures none), the default branch name, or `file://` URLs (a local-path clone ignores `--depth`). A green local run may fail on the CI runner.
*   **Rationale & Recommendation**: Specify the fixture helper: identity through environment variables or `-c`, `init.defaultBranch`, `file://` clones for the shallow case, and a CI run of the tests before the rules.

### style-1-1. Item 28 contradicts its own list and mixes full and bare paths - Severity: Major
*   **Id:** style-1-1
*   **Reviewer Lens:** `style`; also `codebase` (instance B, Nit), `readiness` (A and B, Nit)
*   **Affected Scope:** `1166-1175` (item 28)
*   **Discrepancy Description**: "Delete every file listed here" is followed by `memory/constitution.md` and `memory/sync-impact-rust-sdk-2026-10-27.md` marked "(already moved)", and the heading says "21 tracked files". PR 23 deletes 19. Only the first path carries the `.specify/` prefix; `init-options.json`, `integrations/...` and `memory/...` are bare.
*   **Rationale & Recommendation**: "Delete the 19 files below. The 2 files marked moved are already gone." Give every path in full, or state once that all paths are relative to `.specify/`.

### style-1-2. "Governance's Compliance review renamed to the design doc" describes a rename that does not happen - Severity: Major
*   **Id:** style-1-2
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `1014` (item 11); TP 1 table at `324`
*   **Discrepancy Description**: The disposition table (confirmed by the synthesizer) changes the wording from "every `plan.md` carries a Constitution Check" to "every design doc carries a Constitution Check and a Complexity Tracking section". Nothing is renamed.
*   **Rationale & Recommendation**: "In Governance, Compliance review, replace `plan.md` with 'design doc' and require a Complexity Tracking section."

### style-1-3. Bare "section N" collides with the constitution's own sections - Severity: Major
*   **Id:** style-1-3
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `1011-1016` (item 11)
*   **Discrepancy Description**: After one "Technical Plan section 1" the item uses "section 2", "3", "5", "9" and "6" in a sentence that also names the constitution's sections "Development Workflow" and "Governance".
*   **Rationale & Recommendation**: Write "Technical Plan section N" every time, as items 1, 2, 6, 8 and 22 and line `1212` do.

### style-1-4. Bare file names and referents that more than one file or thing could satisfy - Severity: Major
*   **Id:** style-1-4
*   **Reviewer Lens:** `style` (two findings merged)
*   **Affected Scope:** `934`, `987`, `1087`, `1065`; `1087-1089` (item 21)
*   **Discrepancy Description**: Four `README.md` files are in play: the root one (item 17), `docs/designs/README.md` (item 8), `specs/README.md` (item 19), `docs/engineering/README.md` (item 21). "`docs/designs/TEMPLATE.md` and `README.md` are skipped by name" (`934`) can be read as skipping every file named `README.md`. In item 21, "describe the guide as the design-doc workflow" follows a Type / Scope that names "the AI-engineering guide", but the intended referent is `sdd-workflow.md`.
*   **Rationale & Recommendation**: Use the full path at every use; name the file in item 21.

### style-1-5. Proposed Changes mix imperative, present-tense and noun-phrase forms; several can be read as the current state of the file - Severity: Major
*   **Id:** style-1-5
*   **Reviewer Lens:** `style`
*   **Affected Scope:** items 15, 17, 24, 26 (present tense: "Line 3 drops ...", "Line 677 describes ... and adds `docs/designs/`", "The description on line 3 says ... instead of ..."); items 11, 16, 18, 19; item 12 (noun phrase); item 20 (participles); item 17 `1059-1060`
*   **Discrepancy Description**: Items 4 to 6, 9, 10 and 27 use the imperative; the others do not. Item 17 is the clearest case: "adds `docs/designs/`" does not say to what, and "drop the Spec Kit and `feature.json` text" does not say whether to delete the line or the mention.
*   **Rationale & Recommendation**: Write every Proposed Change in the imperative with the object and the replacement ("Replace X with Y on line N of F").

### style-1-6. Item 25's list of recorded decisions is telegraphic and mixes dates - Severity: Major
*   **Id:** style-1-6
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `1114-1117` (item 25)
*   **Discrepancy Description**: "the tool only" is a fragment (line `261` says "only the tooling is removed and `specs/` stays"). "the architecture decisions of this design" has no checkable scope and collides with the "architecture check" of Goal 7. "The decisions of 2026-11-02" includes "the owner's answers to Q1 to Q5 and to the contested findings", which `168` says are not applied yet, so they cannot carry that date.
*   **Rationale & Recommendation**: Copy the decision wording from lines `260-267`, enumerate the architecture decisions, and record the answers under their own date.

### style-1-7. Step 0 states the owner's authorization as a fact - Severity: Major
*   **Id:** style-1-7
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `917-921` (Step 0)
*   **Discrepancy Description**: "This is an external action and the owner authorizes it" can be read as already granted. The last sentence has two "which" with unclear referents (see also codebase-1-1).
*   **Rationale & Recommendation**: "Push the tag only after the owner authorizes it." Move the explanation into a separate sentence.

### codebase-1-6. Item 9: the job spans lines 165-176, and the `fetch-depth: 0` pattern already exists - Severity: Nit
*   **Id:** codebase-1-6
*   **Reviewer Lens:** `codebase` (A; `readiness` A Nit)
*   **Affected Scope:** `994-998` (item 9)
*   **Discrepancy Description**: The `architecture` job runs from line 165 to 176 (`check-agent-config.py` is on 176; 177 is blank). The range still lands on the right job. The checkout step (`ci.yml:169`) has no `with:` block yet; `fetch-depth: 0` is used at `ci.yml:103-106` (job `released-upgrade`). The job has no `setup-python`, so it uses the runner's system `python3`.
*   **Rationale & Recommendation**: State "lines 165 to 176" and copy the pattern from `ci.yml:103-106`.

### codebase-1-7. Item 10: placement in the command chain, and `verify` does not run `unittest discover` - Severity: Nit
*   **Id:** codebase-1-7
*   **Reviewer Lens:** `codebase` (A)
*   **Affected Scope:** `1000-1004` (item 10)
*   **Discrepancy Description**: The plan does not say where in the one-line chain `python3 scripts/check-architecture.py && python3 scripts/check-agent-config.py` (`AGENTS.md:43`, `.claude/skills/verify/SKILL.md:12`) the new command goes, and `verify` does not run `unittest discover` while CI does (`ci.yml:171`).
*   **Rationale & Recommendation**: Give the exact new line and say whether to add the test step to `verify`.

### codebase-1-8. Item 16: `/egm:<name>` is ambiguous against the plugin's skill names - Severity: Nit
*   **Id:** codebase-1-8
*   **Reviewer Lens:** `codebase` (B)
*   **Affected Scope:** `1052-1054` (item 16)
*   **Discrepancy Description**: The plugin's skill names start with `egm-`, and `docs/claude.md:19` of the plugin writes `/egm:egm-brainstorm`. The quoted text to be written into `CLAUDE.md` can be read as `/egm:brainstorm`. `CLAUDE.md:8` ("give them the diff and the spec") is outside the item, which touches only lines 7 and 10.
*   **Rationale & Recommendation**: Write the invocation form with a real example (`/egm:egm-brainstorm`).

### codebase-1-9. Item 19: `specs/AGENTS.md` lines 3, 5 and 9 are described imprecisely - Severity: Nit
*   **Id:** codebase-1-9
*   **Reviewer Lens:** `codebase` (B)
*   **Affected Scope:** `1068-1073` (item 19)
*   **Discrepancy Description**: The plan says lines 3 and 9 stop saying "the directories" are generated. Only line 3 describes the directories as "produced by Spec Kit"; line 9 says the skills and `.specify/integrations/*` are generated and names `/speckit-constitution`. Line 5 ("a plan ... carries the Constitution Check") is not addressed.
*   **Rationale & Recommendation**: Describe each line's change separately and cover line 5.

### codebase-1-10. Item 26: the new description is true only for converted features - Severity: Nit
*   **Id:** codebase-1-10
*   **Reviewer Lens:** `codebase` (B)
*   **Affected Scope:** `1119-1122` (item 26)
*   **Discrepancy Description**: `tests/Cargo.toml:3` reads "(paths follow specs/*/plan.md)" as the plan says. "paths follow `docs/designs/*.md`" is true only for converted and new features; the 15 features that stay in `specs/` keep `plan.md`.
*   **Rationale & Recommendation**: Reword to cover both locations.

### codebase-1-11. Item 28: deleting `.specify/.gitignore` un-ignores `.specify/feature.json` - Severity: Nit
*   **Id:** codebase-1-11
*   **Reviewer Lens:** `codebase` (B; `readiness` A Nit)
*   **Affected Scope:** `1174-1175` (item 28)
*   **Discrepancy Description**: `.specify/.gitignore:6` is what ignores `feature.json`; the root `.gitignore` has no entry. After PR 23 the file shows as untracked in every checkout and worktree and can be staged by `git add -A`. The plan says "untracked", which is right, but not that it is no longer ignored.
*   **Rationale & Recommendation**: Add an ignore entry for `.specify/feature.json` to the root `.gitignore`, or tell people to delete the file before they pull.

### codebase-1-12. Step 0: tag name, timing and push command - Severity: Nit
*   **Id:** codebase-1-12
*   **Reviewer Lens:** `codebase` (A); also `readiness` (B)
*   **Affected Scope:** `915-921` (Step 0); TP 9 at `625-626`
*   **Discrepancy Description**: The tag `speckit-last-state` is created before PR 1, but the transitional clause (`637-639`) keeps Spec Kit in use until PR 23, so it marks the first state of the change, not the last. The text does not say the tag exists before PR 1 merges, nor the remote or the push command. `guard-bash.py` does not block `git push origin <tag>`; no tag of that name exists today.
*   **Rationale & Recommendation**: Rename the tag or say what it marks, state the timing, and give the commands.

### codebase-1-13. The claim that 036 "has an open worktree" is not shown by the repository - Severity: Nit
*   **Id:** codebase-1-13
*   **Reviewer Lens:** `codebase` (B)
*   **Affected Scope:** `1157` (PR 19 row); TP 7 at `590`
*   **Discrepancy Description**: `.git/worktrees/` holds only `sagat-octopus-egm`. A branch `feature/036-elastic-scaling` exists locally and on `origin`. The reviewers could not check the worktree claims for 014, 016, 018, 035 or 037, or the two ticks on `feature/035-iceberg-write-steps` (needs `git show`).
*   **Rationale & Recommendation**: Say "has an open branch" unless a worktree is confirmed, and let the precondition command of readiness-1-17 decide.

### codebase-1-14. Item 17: the `README.md` sentence continues past line 722 - Severity: Nit
*   **Id:** codebase-1-14
*   **Reviewer Lens:** `codebase` (B)
*   **Affected Scope:** `1059-1060` (item 17)
*   **Discrepancy Description**: `README.md:722-723` is one sentence ("...or `SPECIFY_FEATURE_DIRECTORY`; feature numbers are shared by every branch, so check `git branch -a`"); the plan cites lines 721 and 722. Neither `AGENTS.md` line 10 nor `README.md:723` is updated to mention `scripts/next-feature-id.py`, and the Install line has no stated position in `AGENTS.md`.
*   **Rationale & Recommendation**: Cite 721-723 and say where the Install line goes.

### style-1-8. Vague quantifiers and verbs - Severity: Nit
*   **Id:** style-1-8
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `1043`, `1065`, `1033`, `1016`, `1011`, `1127`, `979`, `981-982`, `1136`, `990`
*   **Discrepancy Description**: "A short 'Install' line", "A short paragraph", "One line near the top", "the link at its end is corrected", "a pure `git mv`", "a plain `ls`", "turns its prose ... into a table", "a day", "at conversion time", "to its place" give no size, position or event.
*   **Rationale & Recommendation**: Give sizes and positions ("after the title"), "24 hours", and name the event ("when the conversion PR opens"). Drop "pure" and "plain".

### style-1-9. Synonym redundancy - Severity: Nit
*   **Id:** style-1-9
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `1071`, `935-936`, `1200`, `1174`
*   **Discrepancy Description**: "produced and generated by Spec Kit"; "Each rule has a case that must fail, seen failing before the rule is written" (said twice, with "one failing case for each rule" on `935`); "ignored `.specify/feature.json`" where the doc says "gitignored" at `619`.
*   **Rationale & Recommendation**: "Write a failing test for each rule before writing the rule", once; one verb; "gitignored".

### style-1-10. Unresolved pronouns and referents - Severity: Nit
*   **Id:** style-1-10
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `1052`, `1097`, `1127`, `971`, `950`, `981`, `978`, `1034`, `997`, `965`
*   **Discrepancy Description**: "It says:" (line 7 or `CLAUDE.md`); "They are part of the approval rule" (reviewers or footer); "the files it checked" (the call or the script); "which stays off until PR 23" (check or search); "accepts it" (entry or hash); "its last `git fetch`" (the script does not fetch); "of each" (no antecedent); "The line 46 path" (no file); "after `check-agent-config.py`" (should read "after the `check-agent-config.py` step"); "a branch that other branches mention" (should be "a feature number that another branch mentions").
*   **Rationale & Recommendation**: Replace each pronoun with its noun.

### style-1-11. Undefined abbreviations and terms - Severity: Nit
*   **Id:** style-1-11
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `933`, `987`, `919`, `922`, `971`, `989`, `932`, `929`
*   **Discrepancy Description**: "PR" is not in Terms, first appears at `388`, and alternates with "pull request" (`1004`, `1183` against `950`, `1066`, `1133`); `AGENTS.md` always writes "pull request". `X` in "unchanged since X" (`933`) is defined only at `559` and `567`; `K` in `part-K` (`987`); "SHA-256" (`919`) against "sha256" (`415`); "normative text" (`922`); "scoped search" (first used at `971`, defined by Goal 4); "line ceiling" (defined only at `574`); "forward-only" (defined only at `433`); "grammar" (`929`).
*   **Rationale & Recommendation**: Define "PR n" once in Terms and use one form; define or link the other terms where first used.

### style-1-12. One term with two names, or two meanings - Severity: Nit
*   **Id:** style-1-12
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `922`, `1114`, `950`, `1072`, `1188`, `1187`, `1190`, `1093`, `1209`, `961`, `1136`, `929`, `1155`, `931`, `1210`
*   **Discrepancy Description**: "tooling" (new scripts in "PR 1. The tooling"; Spec Kit files elsewhere; "the tool only" in `1114`); "accept/accepted" (owner accepts a finding; the accepted list of hashes; the owner "reads and accepts" a hash entry); "live" (instruction surfaces; a live doc; files that "stay live"), with `specs/001-platform-foundation/research.md` both "live" (`1187`) and a "dated record" (`1190`); "read-only reviewers", "agents" and "repository reviewers" for the same three agents; "SessionStart hook" (`961`) against "session-start hook" (`99`, `611`); "conversion comparison" against `--conversion`; "Check table" and "Check rows" against "Constitution Check"; "one definition site" against "defined in one place" (`431`); "this design" against "design doc".
*   **Rationale & Recommendation**: Pick one name per concept and use it in this scope.

### style-1-13. Requirements mixed with explanations - Severity: Nit
*   **Id:** style-1-13
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `918-921`, `1028`, `1052-1054`, `1126-1129`, `1182-1183`
*   **Discrepancy Description**: The Step 0 rationale about the manifests; "It is the only record ... and would be deleted with `.specify/`"; the claim about the `Skill` tool is inside the text to be written into `CLAUDE.md`; the explanation of the quickstart test with "that test runs after the edit" instead of an instruction pointing at `1205`; "so no session is left with ...".
*   **Rationale & Recommendation**: Move rationale to a separate sentence or drop it.

### style-1-14. Item 4 mixes notations for settings keys - Severity: Nit
*   **Id:** style-1-14
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `955-957` (item 4)
*   **Discrepancy Description**: `extraKnownMarketplaces.egm` (dot) against `enabledPlugins["egm@egm"]` (brackets); "`autoUpdate` false" has the value outside the code span; `enabledPlugins["egm@egm"]` has no value. See readiness-1-20 for the missing JSON.
*   **Rationale & Recommendation**: Show the JSON fragment, or use one notation with values.

### style-1-15. Inconsistent path and file naming - Severity: Nit
*   **Id:** style-1-15
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `1091`, `1103`, `1062`, `1099`, `910`, `980`, `1158`
*   **Discrepancy Description**: Item 22 gives one full path and then abbreviates (`security-reviewer.md`); item 28 uses bare names; item 23 writes `tasks.md`/`spec`/`plan` for `spec.md` and `plan.md`; files edited twice are labeled inconsistently ("second edit" in items 18 and 23, not in items 10 and 15); line numbers refer to `develop` at `df59eb0e` (`910`) and drift for the files edited twice (`SPECIFICATION-INDEX.md` line 71, `verify` line 28); "1000 to 1674" without the thousands separator used in "14,004" and "1,268"; "markdown" against "Markdown" (`67`, `95`).
*   **Rationale & Recommendation**: Use full paths, state that line numbers are hints and the quoted text locates the edit, and format numbers and capitalization consistently.

### style-1-16. Inconsistent headings and labels; item 1 is one run-on sentence - Severity: Nit
*   **Id:** style-1-16
*   **Reviewer Lens:** `style`
*   **Affected Scope:** `915`, `922`, `1006`, `1164`, `1008`, `1025`, `1177`, `1194-1212`, `927-936`
*   **Discrepancy Description**: Step headings differ in form ("Step 0 (optional). Tag", "PR 1. The tooling (...)", "PR 2. The constitution and the pointers", "PR 23. Removal"). The verbs go beyond the section title "Create/Modify/Delete" ("Move from ..., then Modify", "Move from", "Enable", "second edit"). Verification Plan labels mix Title Case ("Unit Tests", "Compilation/Build"), an ampersand ("Static Analysis & Linting") and sentence case ("Quickstart test", "Independent read"). Item 1 (`927-936`) is one run-on sentence of about twelve semicolon-separated checks.
*   **Rationale & Recommendation**: Use one heading form and one label case, and split item 1 into a list so each check is visible.

### style-1-17. PR table notes mix status and instruction and use undefined status terms - Severity: Nit
*   **Id:** style-1-17
*   **Reviewer Lens:** `style`; also `readiness` (A, Nit, on the order sentence)
*   **Affected Scope:** `1136-1160`
*   **Discrepancy Description**: "In specification only", "Closing out" and "In construction" are not defined (`580` uses "not started (no ticked task)"). Rows 17 and 18 end with a period or a semicolon inconsistently. Rows 21 and 22 are instructions while the others describe status. "Used to measure the cost" (row 3) does not say which cost. Row 22's "0 on `develop`, 2 on its branch" breaks the numeric column. "features that nobody is changing first" (`1136-1137`) is unmeasurable and conflicts with the order of rows 18 (037) and 19 (036), which have worktrees (`590`), before 014 and 018.
*   **Rationale & Recommendation**: Use a testable criterion such as "no ticked task and no open branch", define the status terms, and keep notes to one kind of content.

## Checked and consistent

Reported by the reviewers; the synthesizer re-confirmed the first four groups.

*   **Bounds:** the target runs from line 908 (header) to line 1216 (closing "Drafted with EGM" line). The plan proper ends at line 1212.
*   **Counts:** the open total is 1,268 and the ticked total 678 across 20 `tasks.md` files. All 20 per-feature counts in the table match `specs/*/tasks.md` (the two ticks of 035 on its branch were not checked). The 20 directory names match `specs/` and the 15 other feature directories have no open task. Every checkbox line begins with a `Tnnn-nnn` id whose prefix equals the feature number.
*   **`.specify/`:** it holds exactly the 21 tracked files item 28 names. `.specify/feature.json` is absent and ignored by `.specify/.gitignore:6`. There are exactly ten `.claude/skills/speckit-*/SKILL.md` with the ten names given, and nothing else in `.claude/agents`, `.claude/hooks`, `.claude/settings.json`, `.github` or `scripts` names `speckit`.
*   **Line numbers cited and found right:** `AGENTS.md` lines 3, 7, 9, 11, 12, 16, 17 and 43; `CLAUDE.md` 7 and 10; `README.md` 677, 681, 721, 722; `SPECIFICATION-INDEX.md` 71; `specs/AGENTS.md` 3 and 9; `specs/README.md` 5; `docs/engineering/README.md` 3 and 7; `ai-engineering.md` 63; line 10 of each reviewer; `verify/SKILL.md` 28; `parallel-agents/SKILL.md` 13 and 19; PR template 3, 19, 25; `tests/Cargo.toml` 3; `constitution-3.0.0-sync-impact.md` 46; the quickstart lines 23 and 46 (the calls start one line earlier).
*   **Other repository facts:** `tests/Cargo.toml:322-324` defines `e2e_payload_classification`, and `quickstart.rs` ignores non-`sagat`/`curl` commands. No test or script reads a `specs/*/{spec,plan,tasks}.md`. `AGENTS.md` has 56 lines against a 160-line budget (`check-agent-config.py:22`). `sdd-workflow.md` has the nine-row table, the eight "what the repository adds" items, the two named sections and the anti-pattern line 55. `.specify/memory/constitution.md` is version 3.0.0 with Principles I to VIII at lines 5-156, "Compliance review" at line 218 and "features 001–012" at lines 183-184. `docs/governance/` holds only the 3.0.0 and compensation-safety reports. `.specify/memory/sync-impact-rust-sdk-2026-10-27.md` is linked from nowhere in the repository. `.github/workflows/ci.yml` job `architecture` starts at line 165 and runs `unittest discover` at 171. `guard-bash.py` does not block `git tag`. All plugin skills carry `disable-model-invocation: true`. `google/elephant-goldfish-model` and `egm@egm` match the plugin metadata under `~/.claude/plugins/`.

## Could not verify

*   Anything that needs a command: `cargo metadata --locked`, the unit tests, the three check scripts "exit 0 on the merged tree", the `e2e_payload_classification` run, and the new checks (no reviewer has a shell).
*   Git-object facts: that the two ticks of 035 exist on `feature/035-iceberg-write-steps`; that `33ab9da5…` (014's `currentCommit`) is reachable from `develop`; which sibling worktrees exist.
*   Whether the 35 existing specs satisfy the new id-uniqueness and one-definition-site rules (needs running the script), and the full number of backticked-path hits beyond the samples read.
*   The Claude Code schema for `extraKnownMarketplaces`, `enabledPlugins` and `autoUpdate`, and the plugin claims of TP 6 (upstream tag distance, "four plus four files" difference between the marketplace clone and the cache copy); GitHub settings (merge method, branch protection, `CODEOWNERS`).

## Notes

*   The harness flagged three reviewer outputs (`settings-json` and `permissions-allow-deny` patterns) because they quote entries of `.claude/settings.json`. The synthesizer read them as findings and found no directive in them; none was acted on.
*   The synthesizer edited no file other than this report.

## Findings footer

The footer uses the form that TP 3 (`384-392`) proposes. Its grammar is itself open (readiness-1-2); the status of every finding is `open` in round 1.

| id | reviewer | severity | status |
| --- | --- | --- | --- |
| readiness-1-1 | readiness | Blocker | open |
| readiness-1-2 | readiness | Blocker | open |
| readiness-1-3 | readiness | Blocker | open |
| readiness-1-4 | readiness | Blocker | open |
| readiness-1-5 | readiness | Blocker | open |
| readiness-1-6 | readiness | Blocker | open |
| readiness-1-7 | readiness | Blocker | open |
| readiness-1-8 | readiness | Blocker | open |
| readiness-1-9 | readiness | Major | open |
| readiness-1-10 | readiness | Major | open |
| readiness-1-11 | readiness | Major | open |
| readiness-1-12 | readiness | Major | open |
| readiness-1-13 | readiness | Major | open |
| readiness-1-14 | readiness | Major | open |
| readiness-1-15 | readiness | Major | open |
| readiness-1-16 | readiness | Major | open |
| readiness-1-17 | readiness | Major | open |
| readiness-1-18 | readiness | Major | open |
| readiness-1-19 | readiness | Major | open |
| readiness-1-20 | readiness | Major | open |
| readiness-1-21 | readiness | Major | open |
| readiness-1-22 | readiness | Major | open |
| readiness-1-23 | readiness | Major | open |
| readiness-1-24 | readiness | Major | open |
| readiness-1-25 | readiness | Major | open |
| readiness-1-26 | readiness | Major | open |
| readiness-1-27 | readiness | Major | open |
| codebase-1-1 | codebase | Major | open |
| codebase-1-2 | codebase | Major | open |
| codebase-1-3 | codebase | Major | open |
| codebase-1-4 | codebase | Major | open |
| codebase-1-5 | codebase | Major | open |
| style-1-1 | style | Major | open |
| style-1-2 | style | Major | open |
| style-1-3 | style | Major | open |
| style-1-4 | style | Major | open |
| style-1-5 | style | Major | open |
| style-1-6 | style | Major | open |
| style-1-7 | style | Major | open |
| codebase-1-6 | codebase | Nit | open |
| codebase-1-7 | codebase | Nit | open |
| codebase-1-8 | codebase | Nit | open |
| codebase-1-9 | codebase | Nit | open |
| codebase-1-10 | codebase | Nit | open |
| codebase-1-11 | codebase | Nit | open |
| codebase-1-12 | codebase | Nit | open |
| codebase-1-13 | codebase | Nit | open |
| codebase-1-14 | codebase | Nit | open |
| style-1-8 | style | Nit | open |
| style-1-9 | style | Nit | open |
| style-1-10 | style | Nit | open |
| style-1-11 | style | Nit | open |
| style-1-12 | style | Nit | open |
| style-1-13 | style | Nit | open |
| style-1-14 | style | Nit | open |
| style-1-15 | style | Nit | open |
| style-1-16 | style | Nit | open |
| style-1-17 | style | Nit | open |
