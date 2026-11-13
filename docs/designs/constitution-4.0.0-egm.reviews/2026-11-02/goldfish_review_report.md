# Goldfish Annotated Review Report

**Status: resolved** (2026-11-02). The owner replied "auto accept". Applied: the 35 `PROPOSE FIX` issues, in one rewrite of the target file (the file grew from 880 to about 1,216 lines). Not applied, as `egm-resolve` requires: 4 `CONTEST` (Issues 31, 32, 34, 35, awaiting the owner's written acceptance), 4 `ESCALATE` (Issues 5, 9, 14, 36, recorded as Open Questions Q2 to Q5 of the target) and 1 `DECLINE` (Issue 43). Several fixes were applied in the Detailed Implementation Plan as well as in the FeatureRange, because the issues changed the plan (the swap of PR 1 and PR 2, the file list per PR); otherwise the file would contradict itself. Nothing is re-audited yet: this was round 1 of the Critic Review.

**Target:** `docs/designs/constitution-4.0.0-egm.md` (`FeatureRange` lines 1-590 for the Critic round; stages 5 and 6 read the whole file)

**Sources.** Stage 5 `zoom_out_report.md` (12 findings: 6 Major, 6 Nit), Stage 6 `comprehension_report.md` (23 findings: 16 Major, 7 Nit) and Stage 7 round 1 `critic_report.md` (52 entries from 69 raw findings: 1 Blocker, 24 Major, 27 Nit). The three reports overlap heavily, so this report groups them into 44 issues. Each issue lists every source id. Severity is the highest severity among its sources, shown as High (Blocker), Medium (Major) or Low (Nit).

**Severity caveat.** The Critic synthesizer lowered nine `egm-style` findings from Major to Nit (`style-1-2`, `-3`, `-4`, `-5`, `-8`, `-9`, `-10`, `-16`, `-19`) and raised three Nits to Major, using a rubric of its own. This report does not change any severity, as `egm-resolve` requires. Where an issue below comes from a lowered finding, it is marked "(reviewer rated Major)". Under the approval rule of the design, a rule that lets an intermediate agent lower a Major is a gap: see Issue 2.

## Summary of suggestions

| Suggestion | Issues |
| --- | ---: |
| PROPOSE FIX | 35 |
| CONTEST | 4 |
| ESCALATE | 4 |
| DECLINE | 1 |
| Total | 44 |

Eight issues touch decisions the owner took (recorded in `concepts_brief.md`, "Decided in round 1" and "Decisions taken while drafting"). Issue 6 extends the reviewed-commit diff of drafting decision 2. Issues 31, 32, 34 and 35 are contested on the strength of the owner's decisions on conversion scope (round 1; Alternatives C and D), pure automatic approval (round 1; Alternative E), the id allocator (drafting decision 8) and the reviewed-commit mechanism (drafting decision 2; Alternative H). Issues 9 and 14 ask the owner to re-decide the owner gate and the timing of removal. A contested finding counts as settled only when the owner accepts the contest in writing.

The four ESCALATE issues are 5 (branch protection), 9 (a document-level scope gate), 14 (when the Spec Kit tooling is removed) and 36 (version bump).

## Prioritized Findings & AI Suggestions

### [Issue 1] [`egm-style`, `egm-design`, `egm-zoom-out`, `egm-comprehension`] - Severity: High
*   **Sources:** `style-1-1` (Blocker), `design-1-6`, zoom-2 (grammar part), comp-3
*   **Affected Scope:** CUJ 2 (135-138), section 3 (277-280), section 4 (288-291), 417, 496
*   **Discrepancy Description:** The grammar of an allowed post-approval edit is defined three ways ("a dated `Evidence:` line", "a date and an `Evidence:` line", "a line that begins with `*Evidence` or with a date and the words `partly done` inside a task") and matches no real task. Zero `specs/*/tasks.md` files have a line starting `*Evidence`. Real data has two forms: `*Done 2026-10-29.* Evidence: ...` (a multi-line paragraph, `specs/018-schedules-bulk-operations/tasks.md`) and `**Evidence (2026-11-01)**:` appended inline (`specs/037-compensation-safety/tasks.md`). No worked task example exists.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid, and I confirmed the data: no line starts with `*Evidence` (0 matches), and the inline form exists 20 times in 037. My own notes written this session on the 035 branch (`*Evidence, 2026-11-02:*`) use a fourth spelling and must be normalized at conversion. This rule is the only barrier between ticking a task and rewriting an approved design, so one grammar is needed.
*   ***Proposed Edit:***
    *   Location: section 4; then CUJ 2, section 3 and section 10 cite it by name
    *   Change: Define an **evidence block** once: the text inside a task's list item, after its checkbox line, that starts at a marker (`*Done YYYY-MM-DD.*`, `*Partly done YYYY-MM-DD.*` or an inline `**Evidence (YYYY-MM-DD)**:`) and runs to the end of that item, with a size cap (proposal: 40 lines) and no heading, table or code-fence marker. Add one worked task block showing id, checkbox, requirement citation, `Owner-gate:` and an evidence block. Require tests for multi-line evidence, design text smuggled in an evidence block, and a tick with no evidence.

### [Issue 2] [`egm-design`, `egm-structural`, `egm-style`, `egm-comprehension`] - Severity: Medium
*   **Sources:** `design-1-1`, `structural-1-8` (the `REV-003` and "Minor" parts), `style-1-11`, comp-4 (counts part)
*   **Affected Scope:** 239-247, 256-274; 243 ("Minor findings and below"); 265 (`REV-003`)
*   **Discrepancy Description:** "Their counts match the block" cannot be built from the repository reviewers' real output: `security-reviewer` reports high/medium/low, `correctness-reviewer` reports confidence, `architecture-reviewer` only says which findings block, and EGM's synthesizer normalizes "unconstrained natural language". Finding ids are review-local, so `Owner-accepted: REV-003` is ambiguous and "item N" is undefined. "Minor" is not in EGM's scale (Blocker, Major, Nit). `Open findings: Blocker 0, Major 0` sits beside a listed owner-accepted finding.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. The record's whole value is that a script compares counts, and today there is nothing machine-readable to compare. The synthesizer's severity changes (see the caveat above) show a second gap: counts must come from the reviewers' own severities.
*   ***Proposed Edit:***
    *   Location: section 3, "Record" and "Rule"; plan items 13 and 19
    *   Change: State the scale once (Blocker, Major, Nit; map security high/medium/low to Blocker/Major/Nit and correctness confidence to Major when stated). Require every report to end with a machine-readable footer of `id, reviewer, severity, status`. PR 1 amends the three repository reviewers' output contract. Counts use the highest severity any reviewer assigned; a synthesizer or author may not lower a Blocker or Major. Owner acceptances cite `report-path#finding-id`. Say that an owner-accepted finding is listed separately and is not counted as open.

### [Issue 3] [`egm-design`, `egm-style`, `egm-comprehension`] - Severity: Medium
*   **Sources:** `design-1-3`, `style-1-11`, comp-4 (which commit is "reviewed")
*   **Affected Scope:** 60, 73-78, 239-243, 256-266, 272-274, 362
*   **Discrepancy Description:** `egm-review` has four presets: Preset 2 skips Stage 9, Preset 3 runs only Stage 9, and only Preset 1 writes `consolidated_review_report.md`, the file the block cites. "Open", "settled", "accepted", "contested", "left unfixed" and "clean" overlap without definition. The block records neither the preset nor the round. With several rounds and fixes between them, it is unstated which commit is the reviewed one.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. I confirmed the preset behavior in `skills/egm-review/SKILL.md` (Presets 0 to 3). A record that cannot say which stages ran can be satisfied by a partial review.
*   ***Proposed Edit:***
    *   Location: section 3, "Rule" and "Record"
    *   Change: Require all five stages (Preset 0 or 1) and record the preset, the number of rounds and the stages run. Define "open" as raised in the last complete run of all four reviewers and not owner-accepted, so a final re-run follows the last fix. `Reviewed-commit` is the commit of that last complete run. Add a failing test for a Preset 2 record.

### [Issue 4] [`egm-design`, `egm-style`] - Severity: Medium
*   **Sources:** `design-1-2` (wording part), `style-1-6`
*   **Affected Scope:** 74-75, 131, 228, 241-243, 268-283, 325-328
*   **Discrepancy Description:** The text says "No person pressed an approve button" (131) and "No person is asked" (228) as absolutes, then requires the owner's written acceptance of contested findings. The author agent writes the status block, the EGM hash and the reports, so every field can be forged by the author. The design never says whether merging a design-doc PR is an owner act.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. The two absolutes contradict the owner-acceptance step in the same design, and the self-attestation limit is real but unstated.
*   ***Proposed Edit:***
    *   Location: CUJ 1 (131), section 2 stage 4 (228), section 10
    *   Change: Replace the absolutes with: "No separate approval step exists. The owner acts only to accept a contested or unfixed Blocker or Major finding." Say that merging a design-doc pull request is the normal merge under `AGENTS.md` rule 14 and not an approval act. Add to section 10: every field of the record can be written by the author. Make reports immutable after they are introduced, and put each report's SHA-256 in the block.

### [Issue 5] [`egm-design`] - Severity: Medium (ESCALATE)
*   **Sources:** `design-1-2` (protection part)
*   **Affected Scope:** 325-328, 341-343
*   **Discrepancy Description:** The repository has no `CODEOWNERS` file (confirmed by the Critic synthesizer's Glob), and the design names no branch protection or merge method for the files that carry authority: `docs/constitution.md`, `docs/owner-decisions.md`, `docs/engineering/egm-accepted-hashes.json`, `.claude/settings.json`, `.claude/agents/`.
*   ***AI Suggestion:*** ESCALATE
*   ***Justification:*** The critique is accurate, and the fix is a repository and GitHub setting that only the owner can decide and that the files I could read do not show. Whether to add `CODEOWNERS` and a branch-protection rule changes who can merge. I will not choose it.
*   ***Proposed Edit:***
    *   Location: section 10 and the Detailed Implementation Plan (PR 2)
    *   Change: If the owner wants it, add `CODEOWNERS` naming the owner for the listed files and require a review in branch protection. If not, record "no technical protection of these files; review by the owner is by convention" as an assumption and a risk.

### [Issue 6] [`egm-design`, `egm-zoom-out`, `egm-comprehension`] - Severity: Medium
*   **Sources:** `design-1-5`, zoom-2 (cost part), comp-4 (identity part)
*   **Affected Scope:** 287-296, 408, 498-499
*   **Discrepancy Description:** Section 9 step 5 requires sibling branches to rebase. A rebase, amend, force-push or a squash or rebase merge makes `Reviewed-commit` unreachable or a different commit, so the check fails on unchanged content and the only fix is a full re-review. The merge-commit rule is not enforceable from the repository: `AGENTS.md` rule 14 says `--no-ff`, but no setting is checked in. The unit-test model `test_check_architecture.py` uses synthetic JSON, while these rules need disposable git repositories.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid, and it extends owner decision 2 without reversing it (the owner chose the reviewed-commit diff knowing the squash risk, Alternative H). The three additions keep that mechanism and make its limits explicit.
*   ***Proposed Edit:***
    *   Location: section 4; Alternative H; plan item 19 (tests)
    *   Change: (a) Also record the blob SHA of the doc at the reviewed commit, and accept a reviewed commit that is no longer an ancestor if the blob at that path matches. (b) Document the GitHub merge setting that must be on and record that it is a convention and not enforced. (c) State that tests of the reviewed-commit rules use disposable git repositories (rebase, amend, shallow clone cases). Owner to accept (a), because it extends their decision.

### [Issue 7] [`egm-design`] - Severity: Medium
*   **Sources:** `design-1-7`
*   **Affected Scope:** 137-138, 230-232, 287-291
*   **Discrepancy Description:** Any edit other than a tick sets the doc back to `Draft`, including a typo and anything `egm-drift-report` finds. Converted docs run up to 4,000 to 14,000 lines, the cost is not estimated, and the rule pushes authors to hide edits in evidence blocks.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. Without an amendment path the rule rewards stale designs.
*   ***Proposed Edit:***
    *   Location: new subsection "Amending an approved doc" in section 4, and section 2 stage 6
    *   Change: Define two classes. A formatting-only change (whitespace and markdown layout that leave the words identical) keeps `Approved`. A substantive change sets the doc to `Draft`, and the re-review covers only the changed line range (`egm-review` accepts ranges) plus the three repository reviewers on the diff. The block records the range. Cost one such re-review in the cost model (Issue 14).

### [Issue 8] [`egm-style`, `egm-design`] - Severity: Medium
*   **Sources:** `style-1-14`, `design-1-23`
*   **Affected Scope:** 268-269, 279-280, 288-291, 353-354
*   **Discrepancy Description:** "Any other change sets the doc back to `Draft`" does not say whether the script rewrites the status or only fails. "Body of the doc" is undefined, and the status block, written after review, must be excluded from the diff. Line 279 compares against "the base branch" and line 288 against `Reviewed-commit`: two diff bases, and "base branch" has no meaning on push events (`ci.yml` lines 3-6 run on pushes as well as pull requests).
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid, and cheap to fix.
*   ***Proposed Edit:***
    *   Location: section 4, section 3 (forward-only rule)
    *   Change: Write "The script exits non-zero. It does not edit the doc. Until the doc is re-reviewed it is treated as `Draft`." Define "body" as everything below the status block. Use `Reviewed-commit` as the diff base for the tick rule on every event, and the merge base with the target branch only for the forward-only evidence rule, stating the base for each event type (pull request, push).

### [Issue 9] [`egm-design`, `egm-zoom-out`] - Severity: Medium (ESCALATE)
*   **Sources:** `design-1-8`, zoom-4 (leak part)
*   **Affected Scope:** 298-308, 362-366, 419-421, 460-466
*   **Discrepancy Description:** About 1,190 of the 1,268 open tasks are in drafts or features not started, and a clean review marks them `Approved`. The only barrier is an `Owner-gate:` that the mapping agent must judge for each task, and a missing gate passes the check. The Critic proposes inverting the default: add `Scope-approval: <owner-decisions ref> | none` to the block and fail any tick while it is `none`.
*   ***AI Suggestion:*** ESCALATE
*   ***Justification:*** The critique is accurate: the design itself admits the gate is "the only barrier" (section 10). But the proposed fix is a document-level scope gate, which is close to the "approval locked while an owner question is open" option the owner rejected in Alternative E, and it was not weighed as its own alternative. Whether to add it is the owner's call.
*   ***Proposed Edit:***
    *   Location: section 5 and Alternative E (or a new Alternative)
    *   Change: If the owner adopts it: add a `Scope-approval:` field to the status block, fail any tick of a task in a doc whose field is `none`, and add the option to Alternatives with the owner's reason. If not: keep section 5 and add the sentence "A missing gate on a task that needed one passes the check; the repository reviewers are asked to look for unmarked tasks".

### [Issue 10] [`egm-design`, `egm-style`, `egm-comprehension`] - Severity: Medium
*   **Sources:** `design-1-9`, `style-1-13`, comp-5
*   **Affected Scope:** 140-146, 265, 304-308
*   **Discrepancy Description:** `docs/owner-decisions.md` entries are table rows under dated headings with no anchors. A gate can need several answers (`T035-001` needs ADR-0071 plus O035-003, -004 and -005), so "the entry exists" does not show that it answers the task's question, and an agent could write the entry. CUJ 3 promises the check fails when an agent "tries to start or tick" a gated task, but section 5 enforces only ticks, and nothing detects a start.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid; CUJ 3 promises more than section 5 delivers.
*   ***Proposed Edit:***
    *   Location: section 5; CUJ 3
    *   Change: Fix the grammar `Owner-gate: docs/owner-decisions.md#YYYY-MM-DD/<n>` where `<n>` is the row number under that dated heading. The script parses the row, requires its Refs column to cite the task or doc, and requires a decided status. One entry per answer. Change CUJ 3 to "tries to tick". State that an agent can write an entry and that the owner's review of the PR is what stops it.

### [Issue 11] [`egm-design`, `egm-comprehension`, `egm-zoom-out`] - Severity: Medium
*   **Sources:** `design-1-10`, comp-6, zoom-3 (fidelity part)
*   **Affected Scope:** 86-88, 156-162, 340-361, 421-424, 518-523
*   **Discrepancy Description:** The design says tasks are "moved, not copied" (349), yet the superseded `tasks.md` keeps its full body (353-354) and the script compares against "the superseded `tasks.md` at X" (359-360). If tasks were moved out, there is nothing at X to compare. The check compares ids and counts only, and a per-id text hash was not weighed.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. The wording contradicts itself, and I confirmed it in the document (lines 349 and 359-360). Goal 5 rests on this check.
*   ***Proposed Edit:***
    *   Location: section 7; Goal 5; Alternative K; plan item 19
    *   Change: Reword to "moved into the live doc; the superseded copy stays frozen at X, and the comparison is against that frozen copy". The id-uniqueness rule excludes superseded files. Compare a per-id hash of the normalized task text, with only listed amendments exempt, and add "a scripted converter with a golden test" as an alternative considered and not chosen (the owner chose agent mapping).

### [Issue 12] [`egm-design`, `egm-style`, `egm-comprehension`] - Severity: Medium
*   **Sources:** `design-1-11`, `style-1-21`, comp-7
*   **Affected Scope:** 86, 259-260, 273-278, 347, 355-361, 430-431
*   **Discrepancy Description:** Large docs are split "by increment", with no threshold for "large", no definition of "increment", no naming scheme under `NNN-slug.md`, no statement of whether approval is per part, and no rule for how id uniqueness, coverage and the count comparison work across parts. Feature 014 (14,004 lines in 42 files) is the largest case and is in scope.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. Without a ceiling, "no Major open" can come from a sampled read, which section 10 already admits.
*   ***Proposed Edit:***
    *   Location: section 7; the README and TEMPLATE item (plan item 5)
    *   Change: Define "increment" as a group of tasks that the source plan labels as one increment. A split doc is `NNN-slug.md` (index and shared requirements) plus `NNN-slug.part-K.md`, with a status block per part, the shared id prefix `NNN`, and coverage computed per feature. The line ceiling of an approvable doc is set after PR 3 measures a real conversion and is then written in `docs/designs/README.md`. Move the ordering bullet to section 9. Define "not started" as no ticked task.

### [Issue 13] [`egm-design`, `egm-structural`, `egm-style`, `egm-comprehension`, `egm-zoom-out`] - Severity: Medium
*   **Sources:** `design-1-12`, `structural-1-3`, `style-1-17`, comp-8, zoom-12
*   **Affected Scope:** 183-231, 368-370, 376-411, 465-466, 561-567
*   **Discrepancy Description:** PR 1 lands a constitution whose rules name the check, hash list and template that arrive in PR 2. Section 8 says pointers change "in the same change as the constitution" and lists the hook and `check-agent-config.py`; section 9 puts both in PR 2. Sections 8 and 9 duplicate the Detailed Implementation Plan and the copies disagree (four versus five new checks; `/adr`, `/migration` and `traceability.md` named in section 8 and absent from PR 1 in the plan). Section 9 omits `TEMPLATE.md` and `README.md` that the plan puts in PR 1. "One wave" conflicts with one feature at a time.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid; I confirmed the section 8 versus section 9 disagreement. Zoom-12 offers the cleaner remedy: land the additive tooling before the normative text.
*   ***Proposed Edit:***
    *   Location: sections 8 and 9; the Detailed Implementation Plan headings
    *   Change: Swap the order. PR 1: `check-design-docs.py`, the extended `check-agent-config.py`, `egm-tree-hash.py`, the accepted-hash list, the settings entry, the hook, the id allocator, `TEMPLATE.md` and `README.md` (no normative change). PR 2: the constitution, its Sync Impact Report and every pointer fix. Add a transitional clause with a sunset: until a feature's conversion PR merges, its `tasks.md` is canonical and `/speckit-*` is allowed for it. Keep intent and constraints in sections 8 and 9 and leave file lists to the plan with a pointer. Replace "one wave" with "one pull request per feature, all 20 committed to". Rename PR labels everywhere.

### [Issue 14] [`egm-zoom-out`] - Severity: Medium (ESCALATE)
*   **Sources:** zoom-3 (sequencing part)
*   **Affected Scope:** "9. Landing order and rollback"; the PR 3-22 table; PR 23
*   **Discrepancy Description:** The deletion of `.specify/` and the skills is the owner's original request, and the design parks it at PR 23, behind 20 conversion PRs, though nothing technical makes the deletion depend on the conversions.
*   ***AI Suggestion:*** ESCALATE
*   ***Justification:*** The critique is accurate in what it says: removing the tooling does not break the existing `spec.md`, `plan.md` and `tasks.md`. But "delete last" also supports the transitional clause of Issue 13 (`/speckit-*` stays allowed for unconverted features), and the choice trades the owner's request against the in-flight features. It is the owner's call and I should not pick it.
*   ***Proposed Edit:***
    *   Location: section 9 and Detailed Implementation Plan, PR 23
    *   Change: Either (a) keep removal last, as drafted; or (b) remove `.specify/` and the ten skills right after the constitution PR and the sibling-branch rebases, drop the transitional clause, and let unconverted features keep their `tasks.md` by hand until converted. Record the choice and the reason in section 9.

### [Issue 15] [`egm-design`, `egm-style`, `egm-zoom-out`] - Severity: Medium
*   **Sources:** `design-1-4`, `style-1-23`, zoom-3 (cost part), comp-20 (units)
*   **Affected Scope:** 372-377, 468-475, 518-523, 564-568
*   **Discrepancy Description:** One pass costs 1+1+3+1+3 = 9 reviewer runs; "converged" needs two consecutive clean rounds per looped stage (minimum 15) and the 5-round cap gives 33 before synthesizer and resolve runs. The design's 15-27 band is the reviewers' estimate and its top is below the cap-derived worst case. The 60 repository-reviewer runs count one run per doc though re-runs follow fixes. The number of owner acceptances the rule will produce is not estimated. "Reviewer run" is undefined. "The plan is revisited" has no criterion.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid, and this review is itself evidence: this one doc needed three stages and produced 104 raw findings in round 1 (12 + 23 + 69), which became 87 entries after the Critic round merged duplicates.
*   ***Proposed Edit:***
    *   Location: section 7 (cost paragraph); NFR "Cost"; Verification Plan "Cost"; Alternative K
    *   Change: Define "reviewer run" as one reviewer agent started. Model it as rounds times reviewers per stage, with the cap-derived worst case. Count owner acceptances as a cost line. Set a stop condition after PR 3 (proposal: stop and ask the owner if PR 3 exceeds 27 EGM runs or 10 owner acceptances) before PR 4 starts. Both numbers are proposals for the owner to set.

### [Issue 16] [`egm-design`, `egm-style`, `egm-comprehension`] - Severity: Medium
*   **Sources:** `design-1-13`, `style-1-20`, comp-11
*   **Affected Scope:** 81-85, 395-398, 852-855, 866
*   **Discrepancy Description:** Goal 4 exempts whole directories (`specs/`, `tasks/reviews/`, `docs/adr/`, `docs/governance/`, `docs/owner-decisions.md`), the plan exempts specific files, and `docs/designs/` is on neither list though this doc contains `speckit` on 22 lines. The new check would fail on its own PR, or the exception list would grow ad hoc.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid; I counted 22 lines. Scoping the check to live instruction surfaces ends the growing exception list.
*   ***Proposed Edit:***
    *   Location: Goal 4; section 8 last bullet; Verification Plan; plan item 24
    *   Change: Scope the check to live instruction surfaces: `AGENTS.md`, `CLAUDE.md`, `README.md`, `.claude/`, `.github/`, `scripts/`, `docs/engineering/` and `docs/constitution.md`. Drop the historical allow-list. State the case rule for `speckit` against `Spec Kit`.

### [Issue 17] [`egm-design`, `egm-zoom-out`] - Severity: Medium
*   **Sources:** `design-1-14`, zoom-7
*   **Affected Scope:** 50-52, 81-85, 395-397; plan item 24
*   **Discrepancy Description:** The design says `check-agent-config.py` "checks relative links and rule numbering only". It also enforces line budgets, the `CLAUDE.md` import, skill and agent front matter, reviewer allow-lists, hook existence, the `.env` deny rule and the ADR format, and its link check scans only `AGENTS.md`, `CLAUDE.md`, `docs/engineering/*.md`, `docs/adr/README.md` and nested `AGENTS.md`. The backticked-path check has no rule for what counts as a path: `README.md` has 14 backticked path-like tokens and five do not exist in a clean checkout (`infra/docker/.env`, `infra/docker/runner.token`, `frontend/apps/studio/dist`, `target/sdk-packages`, `registry.example.com/team/`). Section 8 lists four new checks and plan item 24 lists five.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. The description of the current checker is wrong, and the extension must widen the scanned set.
*   ***Proposed Edit:***
    *   Location: "The Problem", pointers bullet; section 8; plan item 24
    *   Change: Describe the current checker accurately. Name the files the new check scans (add `README.md`, `SPECIFICATION-INDEX.md`, `specs/README.md`, `.claude/agents` and `.claude/skills`). Use `git ls-files` for existence, keep an explicit allowlist file for gitignored and build-output paths, and treat placeholders such as `specs/NNN-slug/` as non-paths. Make the list of checks identical in section 8 and in plan item 24. Add tests for placeholders and build outputs.

### [Issue 18] [`egm-design`, `egm-style`, `egm-zoom-out`] - Severity: Medium
*   **Sources:** `design-1-15`, `style-1-12`, `design-1-25`, zoom-1
*   **Affected Scope:** 249, 310-333; plan items 20 and 21
*   **Discrepancy Description:** The hash covers `plugin.json` plus `agents/`, `docs/`, `resources/` and `skills/`. Claude Code skips the root `agents/` because `.claude-plugin/plugin.json` lists adapters (`docs/claude.md` lines 108-121, confirmed), and the adapters in `.claude-plugin/agents/` carry the read-only `tools: [Read, Grep, Glob]` and `omitClaudeMd: true`. None of `.claude-plugin/` is hashed, so widening a reviewer's tools leaves the hash unchanged. The hashed set also includes non-behavioral files (`skills/*/TEST.md`, `test_scenarios.yaml`, `assets/`, `docs/privacy.md`). The field is defined as "files that were read" in one place and as a fixed set in another, and "the commit that ran the reviews" is called a hash entry. The record keeps neither the Claude Code version nor the model.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid; I confirmed the adapter behavior in the plugin's `docs/claude.md` and that `.claude-plugin/` exists. I had hashed the wrong directory.
*   ***Proposed Edit:***
    *   Location: section 6; the status block; plan items 20 and 21
    *   Change: Hash `.claude-plugin/**` (manifest and adapters), `agents/`, `resources/`, the `SKILL.md` and `references/` files of `skills/`, and only the `docs/` files the skills tell the agent to read; exclude `TEST.md`, `test_scenarios.yaml`, `assets/`, `site/` and `tests/`. Write one definition: "hash computed by `scripts/egm-tree-hash.py` over this set". Each list entry stores hash, commit and date. Record the Claude Code version and model in the block and enforce a minimum version (2.1.271 per the plugin's `docs/claude.md`). State how another harness's install is hashed, or that it is not accepted.

### [Issue 19] [`egm-design`, `egm-style`, `egm-zoom-out`] - Severity: Medium
*   **Sources:** `design-1-16`, `style-1-15` (pin claims), zoom-1 (CI part)
*   **Affected Scope:** 148-154, 233-235, 310-331, 415-417, 483, 556-557
*   **Discrepancy Description:** Settings cannot pin a version (318), so `claude plugin install egm@egm` installs whatever the marketplace holds today. A new contributor gets an unlisted hash and cannot produce an approvable review until the owner reads a 100+ file diff. Lines 233-235 and 483 say a step change is "caught by the check and routed to an amendment", but the check sees only a different hash; CI has no plugin installed, so it can only compare a string. Lines 556-557 say the pin "limits the plugin to files whose hash the owner accepted", while lines 316-318 and 415-417 say nothing enforces the version.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. The claims exceed the mechanism, and the design already knows it (section 10).
*   ***Proposed Edit:***
    *   Location: sections 2, 6, 10; NFR "Security and supply chain"
    *   Change: Reword to: "A review record that carries an unlisted hash fails the check. The check does not stop a developer from running another EGM version, and it cannot tell whether a step changed; a person reads the diff." Document how to install the accepted commit (the list entry records it) or state that no such way exists. Name who reads the diff, with a checklist limited to behavioral files. State that CI compares strings only.

### [Issue 20] [`egm-design`, `egm-zoom-out`] - Severity: Medium
*   **Sources:** `design-1-17`, zoom-9
*   **Affected Scope:** 316-318, 555-560; plan items 22 and 27
*   **Discrepancy Description:** Enabling the plugin in the committed project settings loads EGM into every contributor's session and adds nine agent types, including a synthesizer with Write and a bootstrap orchestrator with Bash and Edit. `check-agent-config.py` forbids those tools for repository reviewers, and `ai-engineering.md` and `AGENTS.md` rule 20 treat plugins as supply chain, so "widens no permission" holds only for `permissions.allow`. Separately, `.claude/settings.json` lines 13-15 allow-list the exact check commands, so the new `check-design-docs.py` has no entry and prompts on every run; the `AGENTS.md` Commands block omits it.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. The tool surface is a real change and the NFR understates it.
*   ***Proposed Edit:***
    *   Location: NFR "Security and supply chain"; Alternative I; plan items 6 and 22
    *   Change: Restate the NFR with the tool surface each added agent has. Add the alternative "enable at user scope and commit only the marketplace entry" to Alternative I, with the reason project scope was kept. Add the script to the `AGENTS.md` Commands block. Add one allow entry for `python3 scripts/check-design-docs.py`, in the style of the existing entries, and say plainly that it is a small widening; the owner may skip it and accept the prompt.

### [Issue 21] [`egm-design`, `egm-comprehension`, `egm-style`] - Severity: Medium
*   **Sources:** `design-1-18`, comp-9, `style-1-18`
*   **Affected Scope:** 44-45, 78-80, 103, 231, 349-352, 487
*   **Discrepancy Description:** Goal 3 promises a named carrier for each rule in "The Problem", but the design never gives the item-by-item mapping. The append-only registers have no check (section 7 only says they "must stay append-only"), and the home of a new feature's registers, data model, quickstart and evidence is not stated. Line 231 says `egm-drift-report` replaces `/speckit-converge` "in part only, and the design says so", and nothing says which part. Clarify, checklist and taskstoissues are not addressed at all.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. This is the document's thesis: nothing is lost silently. From the skill descriptions, `/speckit-converge` assesses the code against spec, plan and tasks and appends the unbuilt work as new tasks; `egm-drift-report` covers the assessment and not the appending.
*   ***Proposed Edit:***
    *   Location: after "Goals", a "Carrier of each rule" table; section 7; section 2 stage 6
    *   Change: Add a table with one row per item of "The Problem" and per Spec Kit skill (`constitution`, `specify`, `clarify`, `plan`, `checklist`, `tasks`, `analyze`, `implement`, `converge`, `taskstoissues`) giving the carrier (script, written rule, EGM step, or "none, and why"). State that no replacement exists for `checklist` (the 36 security checklist's 34 open items stay in `specs/`) and for `taskstoissues`, and that `converge`'s step of appending unbuilt work as tasks is done by the author by hand. For the registers, add a cheap check that existing D/G/O ids in a changed `research.md` are not deleted or renumbered against the base, or state "written rule, unchecked, because ...". State where a new feature's registers and contracts live (`specs/NNN-slug/`).

### [Issue 22] [`egm-structural`] - Severity: Medium
*   **Sources:** `structural-1-1`
*   **Affected Scope:** section 10 (413-430); NFR (553-568)
*   **Discrepancy Description:** The feature-spec standard requires "Risks & Mitigations". The document has no section with that role: the limits are in section 10, an H3 nested in the Technical Plan, and most bullets state a limit but no mitigation. Rollback appears in section 9 and again in the NFR.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. The content exists; its organization and the missing mitigations are the gap.
*   ***Proposed Edit:***
    *   Location: section 10 and NFR
    *   Change: Promote section 10 to an H2 "Risks and Mitigations" as a table of risk, mitigation and residual limit, and link the NFR items to it. Keep rollback in one place.

### [Issue 23] [`egm-structural`, `egm-comprehension`] - Severity: Medium
*   **Sources:** `structural-1-2`, comp-14
*   **Affected Scope:** 78-80, 249-255, 270-271
*   **Discrepancy Description:** The planned check requires every design doc to have a Constitution Check (a row for principles I to VIII and the Delivery Constraints) and a Complexity Tracking section. Section 3 exempts unnumbered cross-cutting docs from the `FR/SC/T` ids only, so this doc, which has neither section, would fail its own check once the check lands. It also amends the constitution, so a Check table fits it.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid; I confirmed the document has no such section.
*   ***Proposed Edit:***
    *   Location: new sections after "Alternatives Considered"; section 3
    *   Change: Add a Constitution Check table (one row per principle I to VIII and the Delivery Constraints, with the design's evidence) and a Complexity Tracking section (each new script with the simpler alternative rejected). State in section 3 and in the check's specification which sections an unnumbered doc is exempt from (only the `FR/SC/T` ids and Requirements).

### [Issue 24] [`egm-style`, `egm-comprehension`, `egm-structural`, `egm-design`] - Severity: Medium
*   **Sources:** `style-1-7`, comp-16, `structural-1-5`, `design-1-21`
*   **Affected Scope:** 69-72, 92-93, 188-209; row at 202; plan item 2
*   **Discrepancy Description:** Goal 1 lists four verdicts (KEEP, MOVE, REWRITE, DROP); section 1 says "plus the verdicts MOVE and DROP"; the table uses a fifth, ADD, and has no MOVE row, though the constitution file itself moves and the owner-decisions rule is really a move from an `AGENTS.md` bullet. "Unit" and "obligation" are undefined; rows differ in size; the footer has no row; the quoted sentence at row 202 is not verbatim (`.specify/memory/constitution.md` line 184 reads "Features 001-012 already exist; a new feature continues the sequence from 013"); "verdict" also names a review outcome at line 57.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. Goal 1 is the first checkable goal and cannot be checked without a unit list.
*   ***Proposed Edit:***
    *   Location: Goal 1; section 1 and its table
    *   Change: Define "unit" as one heading or one bullet of the 3.0.0 text and list the unit set once. List the full set KEEP, MOVE, REWRITE, DROP, ADD in Goal 1 and in section 1. Add MOVE rows (the constitution file; the owner-decisions rule) and a footer row, quote the sentence verbatim, rename the first column and call the table values "disposition" to avoid the clash with the review "verdict". Define "obligation" as a MUST statement of the 3.0.0 text.

### [Issue 25] [`egm-comprehension`] - Severity: Medium
*   **Sources:** comp-10
*   **Affected Scope:** Overview 22-27; Goals 1-7; 221-223, 404-406, 543-551, 775-784
*   **Discrepancy Description:** The id allocator, the EGM pin and the hook change are PR deliverables with plan items, but the Problem and Goals never state the numbering-collision problem, and the Overview's five items omit the pin and the owner gate. "Reserved ranges" are never explained.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. A reader cannot tell why these exist. The comprehension gap is not contestable.
*   ***Proposed Edit:***
    *   Location: "The Problem"; Overview; Goals; section 9
    *   Change: Add to "The Problem" the numbering collision (two branches took the same number; `AGENTS.md` records "a clash happened") and the unpinned plugin. Add goals for the allocator and the pin, and list all five mechanisms in the Overview. Explain "reserved ranges" (the `D`/`G`/`O` blocks and ADR numbers that `SPECIFICATION-INDEX.md` assigns to features) in one sentence.

### [Issue 26] [`egm-comprehension`, `egm-style`] - Severity: Medium
*   **Sources:** comp-1, comp-2, `style-1-2` (reviewer rated Major), `style-1-3` (reviewer rated Major), `style-1-24`, comp-17, comp-18, comp-19 (Nit)
*   **Affected Scope:** 17-19, 38, 58, 59-61, 101, 120, 224-226, 242, 313, 326, 388, 424, 430, 455, 470, 513, 574-575, 586, 697
*   **Discrepancy Description:** EGM vocabulary is used as if known (six skills, "seven lenses", "readiness standard", ESCALATE, "stage", "harness"); the "seven lenses" claim is the whole justification for the three extra reviewers. The evidence base is described inconsistently ("the one EGM review run" versus "the earlier EGM runs", "six reviewers" versus "seven lenses"), and the cited run is never named. Spec Kit, FR, SC, ADR, Complexity Tracking, Delivery Constraints, Sync Impact Report, Claude Code plugin settings and "EGM hash" are used before they are defined.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. Under `egm-resolve`, a comprehension gap is not contestable: if a fresh reader cannot follow, the document is missing context.
*   ***Proposed Edit:***
    *   Location: after the Overview, a "Terms" section; "The Problem"
    *   Change: Add a short glossary: EGM and its six skills and seven lenses, Goldfish and Elephant, Spec Kit and its artifacts, FR and SC, ADR, Sync Impact Report, Constitution Check, Complexity Tracking, Delivery Constraints, harness, ESCALATE, readiness standard, `extraKnownMarketplaces`, `enabledPlugins`, `ref`, `sha`, `autoUpdate`. Name the cited run once ("One EGM review of a prior document, `tasks/reviews/2026-10-30-open-tasks/`, ended with 0 Blocker and 6 Major findings") and use one term each for EGM's lens agents, the three repository agents and the brainstorm panel.

### [Issue 27] [`egm-comprehension`, `egm-style`] - Severity: Medium
*   **Sources:** comp-12, comp-21, `style-1-4` (reviewer rated Major), `style-1-5` (reviewer rated Major), `style-1-8` (reviewer rated Major), `style-1-9` (reviewer rated Major), `style-1-25`
*   **Affected Scope:** 37, 39, 111, 171, 199, 250, 287, 304-307, 348, 364, 421, 473-474, 513, 521, 566, 637
*   **Discrepancy Description:** "Owner gate" has four senses (the per-task `Owner-gate:`, the "Owner gates" section the new template adds, the EGM template having "no owner gates", and the drafting "section gates"), plus "gate" for a CI step and a review rule. "Draft" has four senses, "plan" has five, "the check" has several referents, "boxes" and "tasks" alternate, and one role and one artifact each have many names ("owner", "author", "agent", "person"; "approval record", "review record", "status block").
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid; these are comprehension gaps. Each fix is a rename.
*   ***Proposed Edit:***
    *   Location: throughout; the glossary of Issue 26
    *   Change: "Owner gate" means only the task prerequisite; call the template section "Owner decisions needed", the drafting checkpoint "section approval" and the CI step "CI check". Reserve `Draft` for the status value and say "feature in specification only" for 019-032 and 036. Say "Detailed Implementation Plan" for the task section every time. Write `check-design-docs.py` wherever that script is meant. Define "a task is one markdown checkbox line whose text begins with an id" and use "task" only. Define "author" (human or agent) once and use "status block" for the block and "approval record" for its data.

### [Issue 28] [`egm-comprehension`, `egm-zoom-out`] - Severity: Medium
*   **Sources:** comp-13, zoom-10
*   **Affected Scope:** 295-296, 598-601, 855-856
*   **Discrepancy Description:** The Step 0 tag is an external push, and the claim that it "keeps that commit reachable" is unexplained. The base commits cited by the 014 and 037 evidence manifests are on `develop` and stay reachable under the merge-commit rule. The manifests are never defined: they record a SHA-256 of `.specify/memory/constitution.md` at a base commit.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid. The reachability argument does not hold; the tag's real value is a named restore point.
*   ***Proposed Edit:***
    *   Location: Step 0; "Files that are read and not changed"; section 9 step 1
    *   Change: Define the evidence manifests in one sentence. Drop the "keeps that commit reachable" claim. Keep the tag only as a named restore point for the rollback amendment, mark it optional, and note that pushing it is an external action the owner authorizes.

### [Issue 29] [`egm-comprehension`] - Severity: Medium
*   **Sources:** comp-15
*   **Affected Scope:** 655-656; CUJ 1 (127)
*   **Discrepancy Description:** "EGM skills (which the `Skill` tool refuses, so they are read and followed in the same thread)" is not understandable: it does not say what is refused, who refuses it, or how the skills are actually run.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid, and I confirmed the behavior while running these skills: they carry `disable-model-invocation: true`, so the agent reads the `SKILL.md` and follows it in the same session.
*   ***Proposed Edit:***
    *   Location: plan item 7 (`CLAUDE.md`); CUJ 1
    *   Change: Write: "EGM skills are marked manual-only, so Claude Code's `Skill` tool does not start them. An agent reads the skill's `SKILL.md` and follows it in the same session, or the person starts the skill with its slash command `/egm:<name>`."

### [Issue 30] [`egm-zoom-out`] - Severity: Medium
*   **Sources:** zoom-6
*   **Affected Scope:** plan items 19 and 24; section 3 checks; section 8 last bullet; Verification Plan
*   **Discrepancy Description:** `check-design-docs.py` bundles more than ten rule families. Cross-doc id uniqueness duplicates "prefix equals doc number" plus unique doc numbers; the coverage rule may fail many converted drafts on first run; the superseded-body and conversion-fidelity checks are one-time migration scaffolding made permanent; report-count parsing reads free-text reports; the constitution checks (version line, eight headings, no-`speckit`) do not belong in `check-agent-config.py`.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid in part: the permanent versus one-time split and the misplaced checks are sound. Report-count parsing is fixed by Issue 2's findings footer.
*   ***Proposed Edit:***
    *   Location: section 3; plan items 19 and 24
    *   Change: Make the coverage rule forward-only (applies to docs converted after the first conversion measures it). Run the superseded-body and fidelity comparison once per conversion PR (`check-design-docs.py --conversion NNN`), and keep only a stub-unchanged check permanent. Move the version-line, eight-heading and "Enforced by" checks to the design-doc checker. Drop the permanent no-`speckit` check in favor of the scoped check of Issue 16.

### [Issue 31] [`egm-zoom-out`] - Severity: Medium
*   **Sources:** zoom-3 (scope part)
*   **Affected Scope:** Goals 5 and 6; section 7; section 9; the PR 3-22 table; Alternatives C and D
*   **Discrepancy Description:** The 20-feature, 23-PR plan is larger than the root problem needs: 15 of the 20 features have no ticked task, and the fidelity check protects ticked tasks in only five features. The reviewer recommends scoping this design to PR 1, PR 2 and the removal, with conversion decided later.
*   ***AI Suggestion:*** CONTEST
*   ***Justification:*** The scope is a recorded owner decision. The document states it in Alternatives C and D ("Reason for Rejection: The owner chose to convert the features with open tasks" and "The owner chose to convert all 20 features with open tasks in one wave. The accepted cost is the 300 to 540 reviewer runs"). The reviewer's own recommendation says the owner decisions "stay on record". The factual parts of this finding are handled elsewhere: the 033 heading problem in Issue 23 (the exemption), the cost stop condition in Issue 15, and the removal sequencing in Issue 14. The owner may overrule this contest.
*   ***Proposed Edit:***
    *   Location: none for the contest; the 033 point is in Issue 23
    *   Change: No change for the scope. For feature 033, whose plan has only a one-line "Constitution:" note and no Check table (confirmed), say that a conversion records "no Check in the source plan" in its Constitution Check section instead of authoring one.

### [Issue 32] [`egm-zoom-out`] - Severity: Medium
*   **Sources:** zoom-4 (policy part)
*   **Affected Scope:** section 3 (237-243, 249-254); section 5; section 7 last paragraphs; section 10; Alternative E
*   **Discrepancy Description:** A review-clean but unscoped draft becomes `Approved`, while `SPECIFICATION-INDEX.md` says the drafts are "none approved". The reviewer suggests leaving review-clean docs at `Proposed` and reserving `Approved` for the owner, using the template's four-value Status.
*   ***AI Suggestion:*** CONTEST
*   ***Justification:*** This is owner decision 1 and the document records it with the consequence stated plainly: section 7 says "The owner accepted this consequence", section 10 says "An `Approved` converted draft can be read as permission to build. The gate of section 5 is the only barrier", and Alternative E lists the two-state option as "Reason for Rejection: Owner decision: pure automatic approval". The reviewer asks that the owner "re-decide knowingly"; the document already states what is accepted. The owner may overrule this contest. The factual understatement about the template's Status field is fixed in Issue 33.
*   ***Proposed Edit:***
    *   Location: none for the policy
    *   Change: No change for the policy.

### [Issue 33] [`egm-structural`, `egm-zoom-out`] - Severity: Low
*   **Sources:** `structural-1-4`, `design-1-19`, zoom-4 (template part)
*   **Affected Scope:** 4, 249-266
*   **Discrepancy Description:** Line 249 says EGM's template has "no status block", but its header has `Status: [Draft | Proposed | In Review | Approved]` and this document uses it. The text does not say whether the new block replaces or extends the Author, Status and Date lines, the example puts a second `**Status:**` line in the doc, and it does not say what the check does with `Proposed` and `In Review`.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid and cheap.
*   ***Proposed Edit:***
    *   Location: section 3, "Template" and "Record"
    *   Change: State that the block extends the template header: the header `Status:` is the single status line, and the other fields follow it. Give the exact header layout the script parses. Say the check treats `Proposed` and `In Review` like `Draft` (not approved). Align the section title with the body order (Rule, Template, Record, checks).

### [Issue 34] [`egm-zoom-out`] - Severity: Medium
*   **Sources:** zoom-5
*   **Affected Scope:** section 2 stage 2; plan item 25; Alternative N; section 10
*   **Discrepancy Description:** The only Spec Kit allocator, `.specify/scripts/bash/create-new-feature.sh:111-134`, takes the highest number in local `specs/` plus one. Scanning `origin/*`, worktrees, a reserved-range table and the `D`/`G`/`O`/ADR blocks is a new capability; the reviewer recommends only a duplicate-number check.
*   ***AI Suggestion:*** CONTEST
*   ***Justification:*** This is owner decision 8, recorded in Alternative N ("Reason for Rejection: Owner decision: the allocator ships in PR 2"). The reviewer's premise that the manual rule suffices is answered by evidence in a file it could read: `AGENTS.md`, "Start here", says "look at other open branches before taking the next number (a clash happened)". The owner may overrule this contest.
*   ***Proposed Edit:***
    *   Location: none
    *   Change: No change. Issue 25 adds the numbering collision to "The Problem", which gives the allocator its stated motivation.

### [Issue 35] [`egm-zoom-out`] - Severity: Medium
*   **Sources:** zoom-2 (replacement part)
*   **Affected Scope:** section 4; Alternative H
*   **Discrepancy Description:** The reviewer recommends storing a hash of the normalized body (ticks reset, evidence stripped) in the status block, which removes git history from the check and the merge-commit and full-history requirements.
*   ***AI Suggestion:*** CONTEST
*   ***Justification:*** This option is Alternative H, which the document records and the owner rejected ("Reason for Rejection: Owner decision: a diff against the reviewed commit. Its weakness ... is handled by the merge-commit rule and the full-history checkout of section 4"). The Critic round shows a middle path that keeps the owner's mechanism: Issue 6 adds a blob-SHA identity as a fallback when the commit is unreachable. The grammar defects in the same finding are fixed in Issue 1. The owner may overrule this contest.
*   ***Proposed Edit:***
    *   Location: none
    *   Change: No change to the chosen mechanism beyond Issues 1, 6 and 8.

## Low severity (Nit)

### [Issue 36] [`egm-design`] - Severity: Low
*   **Sources:** `design-1-20`
*   **Affected Scope:** 203, 209
*   **Discrepancy Description:** "MAJOR as in 2.0.0 and 3.0.0" does not follow from the versioning rule as written. The 2.0.0 and 3.0.0 bumps relaxed or redefined principle obligations; here all eight principles are KEEP and the versioning clause is unchanged, so adding sections reads as MINOR under that clause.
*   ***AI Suggestion:*** ESCALATE
*   ***Justification:*** The critique is accurate about the written rule. The choice between a MINOR bump (3.1.0, as the clause reads) and a MAJOR bump (4.0.0, with an amendment to the clause) is the owner's. The owner's title "Constitution 4.0.0" in the approved Overview section was chosen before this argument was made.
*   ***Proposed Edit:***
    *   Location: section 1, the paragraph after the table; title
    *   Change: If MAJOR: add the argument ("the carrier of the Compliance review obligation changes and the process the constitution mandates changes") and amend the versioning clause so that a change to the mandated process is MAJOR. If MINOR: change the title and every `4.0.0` to `3.1.0`.

### [Issue 37] [`egm-design`] - Severity: Low
*   **Sources:** `design-1-22`
*   **Affected Scope:** 37-40, 46-47
*   **Discrepancy Description:** In `specs/035-iceberg-write-steps/plan.md` line 115, "Requires ADR" is the pre-design column and the post-design column reads "Pass pending ADR-0071 acceptance". The `ajv` command in `ci.yml` ends at line 341, not 340. "32 plans" is verifiable, so "(reported)" can go.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid and verifiable.
*   ***Proposed Edit:***
    *   Location: "The Problem", first two bullets
    *   Change: Quote both columns, correct the `ci.yml` range to 320-341, and drop "(reported)" from the count of 32 plans.

### [Issue 38] [`egm-design`, `egm-zoom-out`] - Severity: Low
*   **Sources:** `design-1-24`, zoom-8
*   **Affected Scope:** 384-386; plan item 23
*   **Discrepancy Description:** The hook keys on `feature/NNN-slug`, which this design's own branch does not match, so cross-cutting work gets no duplicate-work warning. The hook's valuable behavior is the warning, which needs only `NNN`; the design-doc lookup and the `specs/NNN-slug/` fallback add nothing to it. The hook is a top-level script, so its first test needs subprocess execution or a refactor.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid, and it reduces scope.
*   ***Proposed Edit:***
    *   Location: section 8 hook bullet; plan item 23
    *   Change: The hook reads the number from the branch name and keeps the warning about other branches that mention it. Drop the design-doc lookup and the fallback. A branch with no number gets no warning (state it). Tests run the script as a subprocess.

### [Issue 39] [`egm-design`, `egm-structural`] - Severity: Low
*   **Sources:** `design-1-26`, `structural-1-9`
*   **Affected Scope:** 432-551 (Alternatives A to N)
*   **Discrepancy Description:** 11 of 14 "Reason for Rejection" fields cite an authority ("Owner decision") rather than a reason, and the trade-off sits in Cons. The letters differ from the brief's. The 14 entries mix decisions about scope, approval, the record, the pin and the checks, with no grouping.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid; I flagged the first point myself at the Alternatives gate.
*   ***Proposed Edit:***
    *   Location: "Alternatives Considered"
    *   Change: Group the entries by decision (scope, approval, record, ticks, procedure, pin, owner decisions, conversion method, reviewers, allocator), state the option chosen for each, and put the technical trade-off in the rejection field next to "Owner decision". Add the concrete cost of two formats (double ticking, id clashes) for A, C and D, and the two alternatives named in Issues 6 and 11.

### [Issue 40] [`egm-design`, `egm-comprehension`] - Severity: Low
*   **Sources:** `design-1-27`, `style-1-15` (Goal 5 and 7 parts), comp-22
*   **Affected Scope:** 86-88, 91-93
*   **Discrepancy Description:** Only the "unchanged" half of Goal 7 is checkable; "and evolve" is not. Goal 5 is titled "The conversion loses no task" but is checked only by ids and counts. Goal 7 never says what `check-architecture.py` checks or why Spec Kit removal could affect it, and "no principle text loses an obligation" is not scriptable and overlaps Goal 1.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid.
*   ***Proposed Edit:***
    *   Location: Goals 5 and 7
    *   Change: Goal 5: "The conversion keeps every task id and the counts of ticked and open tasks." Goal 7: drop "and evolve", say that `check-architecture.py` checks crate layering and dependency direction and that this change does not touch crate dependencies, and move "no principle text loses an obligation" into Goal 1.

### [Issue 41] [`egm-structural`, `egm-style`] - Severity: Low
*   **Sources:** `structural-1-6`, `structural-1-7`, `structural-1-8`, `structural-1-10`, `structural-1-11`, `structural-1-12`, `style-1-26`, `style-1-29`
*   **Affected Scope:** 34-55, 69-93, 106-120, 166-181, 216-231, 249-256, 274, 323, 355, 551-553, 564, 568-570, 572-580, 582
*   **Discrepancy Description:** Two horizontal rules from the template are missing before the trailing sections. Cross-references use unqualified "section N" next to Goal N, Q1 to Q4 and PR N. Label styles are mixed and Open Questions are not all questions. Four lines exceed 120 columns. There is not one markdown link in the range, and the document criticizes unchecked pointers while its own are unchecked. The architecture diagram is misaligned and covers only the artifact layout.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid and cheap. Links need care: `check-agent-config.py` does not scan `docs/designs/`, so a link would be unchecked unless the scan is widened (Issue 17 does that).
*   ***Proposed Edit:***
    *   Location: throughout
    *   Change: Restore the two rules. Write "Technical Plan section N" at first use and align the section 9 list with the PR numbers. Pick one label style and phrase each Open Question as a question (move confirmations to "Owner confirmations"). Reflow the four long lines. Use relative links for existing repository files. Align the diagram columns and add a small flow diagram of the procedure and the approval check.

### [Issue 42] [`egm-style`, `egm-comprehension`] - Severity: Low
*   **Sources:** `style-1-10` (reviewer rated Major), `style-1-16` (reviewer rated Major), `style-1-19` (reviewer rated Major), `style-1-22`, `style-1-23`, `style-1-27`, `style-1-28`, comp-20, comp-21, comp-23 (part)
*   **Affected Scope:** 25, 31, 32, 34-35, 38-39, 54, 58-60, 62-63, 78-79, 102, 112, 131, 209, 316-319, 355, 372-374, 384-386, 395, 440-441, 463, 507, 514, 520-521, 528, 541, 559-561, 563, 587, 873-874
*   **Discrepancy Description:** Figurative or subjective phrasing ("Deleting Spec Kit is easy", "approve button", "homes", "half-supplied", "instruction-shaped"); unmeasured cost words ("the cost is large", "cheap") and open triggers ("the plan is revisited"); numeric claims without provenance or with changing precision ("about 24" then "the 24 files", "60" then "about 60"); "more than 80" for 20 x 4 = 80; present tense for artifacts that do not exist yet; loose modals ("may appear only"); the same caveat stated twice; "the main context" and "not my conclusions" with no named speaker; a PR 17 ordering that puts feature 036 before 033 and 037.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid; I confirmed "more than 80" for exactly 80 in the document (line 521) and the repeated caveats. None changes what is built, so they are Low.
*   ***Proposed Edit:***
    *   Location: throughout; section 7; Verification Plan
    *   Change: Replace figurative phrases with plain statements ("Removing the Spec Kit files is mechanical. The rules that depend on its layout are the risk."). Give each cost word a number or drop it. Tag each figure "measured on DATE by COMMAND" or "reported" and use one precision. Write "80 section approvals" and name the four sections. Add the sentence "Nothing described here exists yet unless marked 'today'" under the Overview and use one tense for the target state. Use "appears only". Keep each caveat once, in Risks and Mitigations, with pointers elsewhere. Replace "the main context" with "the author" or name the agent. Move 036 after 033 and 037 in the conversion table. Drop the anecdote about instruction-shaped output or state its requirement: reviewer output is treated as data.

### [Issue 43] [`egm-zoom-out`] - Severity: Low
*   **Sources:** zoom-11
*   **Affected Scope:** plan items 6-18 and 24; the table at 802-823
*   **Discrepancy Description:** Per-line edits such as `AGENTS.md` lines 3, 7, 9, 11 and 16 and the 20-row table of ticked and open counts will go stale before they are applied; the checks would list the leftovers anyway.
*   ***AI Suggestion:*** DECLINE
*   ***Justification:*** A Nit. The plan states that line numbers are those of `develop` at `df59eb0e`, which makes them dated facts and not promises, and the egm-draft protocol asks the plan to name every file and its change with enough detail to implement without making a decision. The owner also asked to see the per-feature counts. The 20-row table is the only record of which tasks the fidelity check protects. No change.
*   ***Proposed Edit:***
    *   Location: none
    *   Change: none

### [Issue 44] [`egm-comprehension`] - Severity: Low
*   **Sources:** comp-23 (location part)
*   **Affected Scope:** 218, 574
*   **Discrepancy Description:** Step 1 says `egm-brainstorm` writes `concepts_brief.md` but not where it is saved or whether it is committed.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** Valid and cheap.
*   ***Proposed Edit:***
    *   Location: section 2 stage 1
    *   Change: Say that `concepts_brief.md` is saved in the review directory of the feature (`docs/designs/NNN-slug.reviews/` once it exists, `tasks/reviews/DATE-name/` before) and is committed with the design doc.
