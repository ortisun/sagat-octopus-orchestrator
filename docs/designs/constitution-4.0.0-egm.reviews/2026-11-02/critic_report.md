# Goldfish Review Report

**Target:** `docs/designs/constitution-4.0.0-egm.md` (`lines 1-590`)

**Round:** 1 (2026-11-02). **Mode:** REVIEW.

**Bounds check.** The caller gave "1..588 of 878". The file now has 880 lines. The header `## Detailed Implementation Plan` is at
line 591, line 589 is the horizontal rule above it and line 590 is blank. All reviewers were told to audit lines 1-590 and to
read line 591 onward only to resolve cross-references. Line numbers below are TargetFile line numbers. Where a reviewer cited a
line past 590, the citation is marked "(outside the range)".

## Reviewer Status

| Reviewer | Status |
| --- | --- |
| `egm-design` | completed |
| `egm-structural` | completed |
| `egm-style` | completed |

No reviewer needed a retry. Each ran once, as a single instance (no A/B pair).

## Severity Breakdown

After deduplication, 69 raw findings (design 27, structural 12, style 30) became 52 report entries.

| Severity | Entries |
| --- | --- |
| Blocker | 1 |
| Major | 24 |
| Nit | 27 |
| Total | 52 |

By the lens of the primary id (the id each entry carries):

| Lens | Blocker | Major | Nit | Total |
| --- | --- | --- | --- | --- |
| `egm-design` | 0 | 17 | 5 | 22 |
| `egm-structural` | 0 | 3 | 7 | 10 |
| `egm-style` | 1 | 4 | 15 | 20 |
| Total | 1 | 24 | 27 | 52 |

The "Traceability" table at the end maps every raw finding to its entry.

## How this report was built

* **Ids.** Each entry carries the id of its primary raw finding, in the form `<reviewer>-1-<n>`, which is the reviewer's own
  numbering. Merged entries list the other raw ids under "Also reported as". Every raw id appears in the Traceability table.
* **Merging rule.** Findings were merged when they describe the same defect in the same place. A merged entry takes the highest
  severity of its parts, unless a "Reviewer severity" line says otherwise.
* **Triage rubric.** Blocker means a requirement can be implemented two different ways and the choice decides whether the
  central control works. Major means the finding changes what must be built, checked or approved, or leaves a stated goal or
  rule unverifiable or self-contradictory in a way a builder must resolve. Nit means a wording, layout or clarity fix that does
  not change what is built. Where the rubric moved a reviewer's severity, the entry says so under "Reviewer severity". The only
  downgrades are nine `egm-style` findings (`style-1-2`, `-3`, `-4`, `-5`, `-8`, `-9`, `-10`, `-16`, `-19`), which the style lens
  rated Major. Three merges raised a Nit to Major because another part of the merge was Major (`style-1-7`, `style-1-12`,
  `style-1-14`), and one merge raised a Major to Blocker (`style-1-1`).
* **Synthesizer spot checks.** These claims were confirmed against the repository before they were used to set severity:
  * `.github/workflows/ci.yml` lines 320-340 lint the four contract files (as the Problem section says).
  * Real task evidence in `specs/018-schedules-bulk-operations/tasks.md` (lines 17, 28, 38, 47, 53) has the shape
    `*Done 2026-10-29.* Evidence: ...`, a multi-line paragraph.
  * The repository has no `CODEOWNERS` file (Glob over the whole worktree).
  * EGM's Claude Code adapters are in `.claude-plugin/agents/` and carry `tools: [Read, Grep, Glob]` and `omitClaudeMd: true`
    (`~/.claude/plugins/cache/egm/egm/0.1.0/docs/claude.md`, lines 112-138). The hash set at lines 319-321 does
    not include `.claude-plugin/`.
  * `specs/035-iceberg-write-steps/plan.md` line 115 reads "Requires ADR" in the Pre-design column and "Pass pending ADR-0071
    acceptance" in the Post-design column.
  * The word "Minor" appears once in the range (line 243). "Nit" never appears.
  * Four non-table lines in the range are longer than 120 characters: 274, 323, 355, 564.
  * All 1,946 checkbox lines in `specs/*/tasks.md` begin with a task id, so "boxes" and "tasks" are the same count today
    (1,946 = 1,268 open + 678 ticked).
  * `.specify/memory/constitution.md` line 184 reads "Features 001-012 already exist; a new feature continues the sequence from
    013", which is not the wording quoted in the verdict table at line 202.
* **Not re-verified by the synthesizer.** The EGM run-count models, the "MAJOR as in 2.0.0 and 3.0.0" history, the match counts for
  `speckit` beyond the 22 lines of this file, the claims about `autoUpdate` semantics, and the contents of the reviewer agents'
  output formats rest on the reviewers' reading.
* **Harness notice.** The harness flagged the `egm-design` output as instruction-shaped (patterns `settings-json` and
  `permissions-allow-deny`), because the findings quote the settings file and the `permissions.allow` list. The content is
  findings only. It was treated as data, and no directive in it was acted on.

---

## Prioritized Findings (Blockers -> Major -> Nits)

## Blocker

### [style-1-1] The evidence and partial-progress line format is defined three ways and matches no real task - Severity: Blocker
* **Reviewer Lens:** `egm-style` (primary), `egm-design`
* **Also reported as:** `design-1-6` (Major)
* **Reviewer severity:** style Blocker, design Major. Merged at Blocker (see the rubric: two implementations are possible and the
  choice decides whether the post-approval control works).
* **Affected Scope:** lines 135-138 (CUJ 2), 277-280 (section 3), 288-291 (section 4), 417, 496
* **Discrepancy Description**: The grammar of an allowed post-approval edit appears in three forms.
  * CUJ 2 (136): "a dated `Evidence:` line".
  * Section 3 (279-280): "carries a date and an `Evidence:` line".
  * Section 4 (289-290): "a line that begins with `*Evidence` or with a date and the words `partly done` inside a task".
  Line 289-290 also parses two ways, because "inside a task" can bind to either alternative. Lines 417 and 496 add a fourth name,
  "evidence lines". The real format in the repository does not match any of these. In
  `specs/018-schedules-bulk-operations/tasks.md` (lines 17-31) evidence is a wrapped paragraph that begins `*Done 2026-10-29.*
  Evidence: ...` and runs over up to eight lines. Its continuation lines match none of the three descriptions.
* **Rationale & Recommendation**: This rule is the only barrier between ticking a task and silently rewriting an approved design.
  A line-based matcher written from this text either rejects real evidence or accepts any line that starts with `*Evidence:`
  followed by design text. Define one grammar once, in the template and in the constitution (for example, an evidence block
  delimited by markers, with a size cap), and have CUJ 2, section 3, section 4 and section 10 cite it by name. Cover the
  `*Done DATE.* Evidence:` form that exists today. Require tests for multi-line evidence, smuggled design text inside an evidence
  line, and a tick with no evidence.

## Major

### [design-1-1] The "counts match the block" check cannot be built from the reviewers' real output - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Also reported as:** parts of `structural-1-8` (the undefined `REV-003` id and "Minor") and `style-1-11` (the "Minor" scale and "counts match")
* **Affected Scope:** lines 239-247, 256-274; also 243 ("Minor findings and below") and 265 (`REV-003`)
* **Discrepancy Description**: The record needs per-report counts of open Blocker and Major findings and a stable id for each
  owner-accepted finding. Per the design lens, the inputs do not exist.
  * `security-reviewer.md` reports high, medium and low. `correctness-reviewer.md` has confidence and no severity.
    `architecture-reviewer.md` only says to "say which findings are blocking". EGM's synthesizer receives "unconstrained natural
    language" and normalizes it.
  * Finding ids are review-local. The prior consolidated report calls `REV-001..008` "review-local aliases", and the repository
    reviewers produce no ids. `Owner-accepted: REV-003` can therefore point at several findings, and `item N` (line 265) is undefined.
  * Line 243 says "Minor findings and below", but EGM's scale is Blocker, Major and Nit. The word "Nit" never appears in the range.
  * "Their counts match the block" (273-274) does not say which counts. Line 264 says `Blocker 0, Major 0` while line 265 lists
    an owner-accepted finding.
* **Rationale & Recommendation**: Define one machine-readable findings footer or sidecar (id, reviewer, severity, status) that all
  four reports must carry. Amend the three repository reviewers' output contract in PR 1, with a map from high/medium/low to
  Blocker/Major/Nit. Make owner-acceptance entries cite `report-path#finding-id`. State the severity scale once and name the
  counts the script compares.

### [design-1-3] "Review ends clean" is not tied to which review ran, and "open" is undefined - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Also reported as:** `style-1-11` (open, settled, accepted, contested, clean)
* **Affected Scope:** lines 60, 73-78, 239-243, 256-266, 272-274, 362
* **Discrepancy Description**: `egm-review` has four presets. Preset 2 skips Stage 9, Preset 3 runs only Stage 9, and only
  Preset 1 writes `consolidated_review_report.md`, the file the block cites. Preset 1 has no resolve or re-audit loop. In
  `egm-review-loop`, "settled" means contested or recorded without a fix, and a round is judged clean before that round's fixes are
  applied. A fixed finding still appears in the raw reports. "Open", "settled", "accepted", "contested", "left unfixed",
  "recorded without a fix" and "clean" name overlapping states without a definition. "Open findings: Major 0" could mean the last
  full run found none, or that the author marked findings resolved. The block records neither the stages run nor the round.
* **Rationale & Recommendation**: Require all five stages and record the preset and round in the block. Define "open" as raised in
  the last complete run of all four reviewers and not owner-accepted, which forces a final re-run after the last fix. Add a
  failing test for a Preset 2 record.

### [design-1-2] Approval is self-attested end to end, and nothing protects the files that carry authority - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Also reported as:** `style-1-6` (Major; "No person is asked" against the owner-acceptance step)
* **Affected Scope:** lines 74-75, 131, 228, 241-243, 268-283, 298-308, 325-328, 341-343
* **Discrepancy Description**: The author agent writes the status block. Under Preset 1 the main agent also writes the
  consolidated report and the synthesizer assigns severities. The EGM hash is pasted by the agent, and an owner acceptance is a
  row in `docs/owner-decisions.md` that any writer can add. The repository has no `CODEOWNERS` file (confirmed), and the design
  names no branch protection or merge method. The text says "No person pressed an approve button" (131) and "No person is asked"
  (228) as absolutes, then requires the owner's written acceptance of a contested or unfixed finding (74-75, 241-243) and has the
  owner see "the findings that need the owner's acceptance" (341-343). The 3.0.0 Sync Impact Report
  (`docs/governance/constitution-3.0.0-sync-impact.md`, lines 6-7) shows the owner's merge was the only human act. The design
  never says whether merging a design-doc PR is an owner act. Section 10 admits that fresh context is unproven. It does not say
  that every field of the record can be forged by the author.
* **Rationale & Recommendation**: State who must review PRs that touch `docs/constitution.md`, `docs/owner-decisions.md`,
  `docs/engineering/egm-accepted-hashes.json`, `.claude/settings.json` and `.claude/agents/`. Back it with CODEOWNERS and branch
  protection, or record it as an assumption and a risk. Make review reports immutable after they are introduced, and put each
  report's SHA-256 in the status block. Replace the two absolutes with a statement of what the owner does, for example: "No
  separate approval step exists. The owner acts only to accept a contested or unfixed Blocker or Major finding." Say separately
  whether merging a design-doc PR is an owner act.

### [design-1-5] The reviewed-commit identity breaks on ordinary git operations - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 287-296, 408, 498-499
* **Discrepancy Description**: Section 9 step 5 requires sibling branches to rebase. A rebase, an amend, a force-push or a
  squash or rebase merge makes `Reviewed-commit` unreachable or no longer the same commit. The check then fails although the
  content is unchanged, and the only fix is a full re-review. The merge-commit rule cannot be enforced from the repository:
  `AGENTS.md` rule 14 says `--no-ff`, but no setting is checked in. The unit-test model, `scripts/tests/test_check_architecture.py`,
  uses synthetic JSON, while these rules need temporary git repositories for rebase, amend and shallow-clone cases, and the design
  does not say so. Alternative H's weakness (hiding text in evidence lines) applies equally to the chosen design, so it does not
  separate the two.
* **Rationale & Recommendation**: Also record a content-bound identity (the blob SHA of the path, or a hash of the normalized body).
  Accept a reviewed commit that is no longer an ancestor if the blob at that path matches. Otherwise forbid rebasing branches that
  hold approved docs, and document the required GitHub merge settings. State that tests need disposable git repositories.

### [design-1-7] There is no amendment path after approval - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 137-138, 230-232, 287-291
* **Discrepancy Description**: Any edit other than a tick sets the doc back to `Draft` and fails CI until the full approval (the
  EGM review plus three repository reviewers) runs again. That includes a typo and anything `egm-drift-report` finds during
  Reconcile. Converted docs are up to 4,000 to 14,000 lines. The cost is not in the estimate, and it pushes authors to hide edits in
  evidence lines (which section 10 concedes) or to leave the design stale.
* **Rationale & Recommendation**: Define change classes. Accept editorial changes by a normalized diff. Re-review substantive changes
  by line range only, since `egm-review` takes ranges. Add an "Amendments" section that cites an owner entry, and cost it.

### [style-1-14] The "reverts to Draft" rule does not say what the script does, what "body" is, or which diff base applies - Severity: Major
* **Reviewer Lens:** `egm-style`
* **Also reported as:** `design-1-23` (Nit; the diff base on push events)
* **Reviewer severity:** style Major, design Nit. Merged at Major.
* **Affected Scope:** lines 268-269, 279-280, 288-291, 353-354
* **Discrepancy Description**: "Any other change sets the doc back to `Draft`" (290-291) does not say whether the script rewrites
  the status or only fails, while lines 268-269 say the script "does not trust" the block. "Body of the doc" (289, 354) is
  undefined, and the status block, written after review, must be excluded from the diff. Line 279 compares against "the base
  branch" and line 288 against `Reviewed-commit`, which are two diff bases. "Base branch" has no meaning on push events: CI runs
  on pushes to `main`, `develop`, `release/**` and `hotfix/**` as well as on pull requests (`ci.yml` lines 3-6).
* **Rationale & Recommendation**: Write "The script exits non-zero. It does not edit the doc. Until the doc is re-reviewed it is
  treated as `Draft`." Define "body" as everything below the status block. Choose one diff base per event type and state it.

### [design-1-8] The guard against building unapproved scope is fail-open - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 298-308, 362-366, 419-421, 460-466
* **Discrepancy Description**: About 1,190 of the 1,268 open boxes are in drafts or features not started (line 462). A clean review
  marks them `Approved`. The only barrier is an `Owner-gate:` that the mapping agent must judge for each task, and a missing gate
  passes the check (419).
* **Rationale & Recommendation**: Invert the default. Add `Scope-approval: <owner-decisions ref> | none` to the block and fail any
  tick while it is `none`. This differs from Alternative E because the review status is unchanged. If per-task gates stay, state who
  verifies the gating in the conversion PRs (PR 3 to PR 22).

### [design-1-9] The `Owner-gate:` reference has no resolvable target, and "start" is not enforced - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Also reported as:** `style-1-13` (Major; "start" cannot be checked)
* **Affected Scope:** lines 140-146, 265, 304-308
* **Discrepancy Description**: In `docs/owner-decisions.md` the entries are table rows under dated headings with no per-entry anchors.
  A gate can need several answers: `T035-001` (`specs/035-iceberg-write-steps/tasks.md` lines 44-50) needs the ADR-0071 acceptance
  plus O035-003, -004 and -005. "The entry exists" does not show that it answers the task's question: an older, unrelated entry
  would satisfy the check, and an agent could write the entry itself. CUJ 3 says the check fails when an agent "tries to start or
  tick" a gated task, but line 306 limits enforcement to "every ticked task that carries a gate", and nothing in the design detects
  a start.
* **Rationale & Recommendation**: Fix a grammar such as `docs/owner-decisions.md#YYYY-MM-DD/<n>`. Have the script parse the row,
  require its Refs column to cite the task or doc, and require a decided status. Use one entry per answer. Align CUJ 3 to "tries to
  tick", or define how a start is detected (for example, a commit footer citing the task id). Add negative tests.

### [design-1-10] The conversion check is weaker than it needs to be, and a deterministic alternative is not weighed - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 340-361, 421-424, 518-523
* **Discrepancy Description**: Tasks are "moved, not copied", so a per-id hash of the normalized task text (and of FR/SC rows against
  `spec.md`) is cheap. It would close the residual risk in section 10, where an agent changes meaning while mapping. It covers
  1,356 tasks across the 20 features. Alternative K compares only interactive drafting against agent mapping and does not consider
  a converter script with a golden test. "Moved, not copied" is also inaccurate: the superseded `tasks.md` keeps its full body
  (353-354, 359-360), so every id exists in two files.
* **Rationale & Recommendation**: Compare per-id text hashes, with only listed amendments exempt. Add the scripted converter as an
  alternative. Reword to "moved into the live doc; the superseded copy stays frozen".

### [design-1-11] Splitting large docs is required but not specified - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Also reported as:** `style-1-21` (Major; "splits a large doc by increment" is undefined)
* **Affected Scope:** lines 86, 259-260, 273-278, 347, 355-361, 430-431
* **Discrepancy Description**: Feature 014 is 14,004 lines in 42 files, and the design says docs are split "by increment". It gives
  no threshold for "large", no definition of "increment", and no naming scheme for the parts. It does not say whether the approval
  record is per part, or how "prefix equals doc number" (275), "every FR cited by a task" and the count comparison work across
  parts. The naming rule `NNN-slug.md` (219-220) and the one-PR-per-feature rule (86, 347) have no wording for a split doc. The
  last bullet at 356-357 ("starts with a feature that has not started") describes the order of work, not what a conversion PR
  does, and "not started" is undefined. Section 10 says reviewers read long documents selectively, and without a size ceiling "no
  Major open" can come from a sampled read.
* **Rationale & Recommendation**: Define part naming, a shared id prefix, per-part status blocks, a feature-level index,
  per-feature coverage, and a line ceiling for an approvable doc. Move the ordering bullet to section 9. Define "not started"
  (for example, no ticked task).

### [design-1-12] The repository state between PR 1 and the last PR is not defined - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 183-231, 399-411, 561-563
* **Discrepancy Description**: PR 1 lands a constitution whose rules name the check, hash list and template that arrive in PR 2.
  Section 9 omits `TEMPLATE.md` and `README.md` from both PR lists, though line 631 of the plan (outside the range) puts them in
  PR 1. `AGENTS.md` rule 1 will point at a design doc that does not exist for any unconverted feature, and the Spec Kit skills stay
  until the last PR. Line 199 says ticks live in the design doc's plan, but `tasks.md` stays canonical for unconverted features.
  Section 8 and the NFR mention only the hook fallback.
* **Rationale & Recommendation**: Add a transitional clause: until a feature's conversion PR merges, its `tasks.md` is canonical and
  `/speckit-*` is allowed for it. Give the clause a sunset. Mark the approval rule effective from PR 2, or land the checks first.

### [structural-1-3] Sections 8 and 9 duplicate the Detailed Implementation Plan, the copies disagree, and the sequencing statements contradict each other - Severity: Major
* **Reviewer Lens:** `egm-structural`
* **Also reported as:** `style-1-17` (Major; sequencing contradictions)
* **Affected Scope:** lines 268-281, 368-370, 376-411, 465-466, 566-567; plan items 14 and 24 (outside the range, lines 700-704 and 769-773)
* **Discrepancy Description**: The draft protocol separates a high-level range from a detailed range so that different reviewers
  audit each. Sections 8 and 9 list file-level edits and PR contents inside the high-level range, and the two copies already
  disagree.
  * Section 8 names four new checks in `check-agent-config.py` (395-397). Plan item 24 lists five, adding the "Enforced by" path check.
  * Section 8 lists `/adr`, `/migration` and `docs/traceability.md` among the pointer fixes (390-393), and section 9 says PR 1
    contains "every pointer fix of section 8" (402-403). The plan has no PR 1 item for any of the three: item 14 covers only
    `verify` and `parallel-agents`, and `traceability.md` appears only in the conversion PRs.
  * Line 378 lists "the Spec Kit passages of the files below" under "Deleted last", while lines 402-403 do those pointer fixes in
    PR 1 and Goal 4 (81-83) requires the re-pointed documents before deletion.
  * Line 465 says "convert all 20 features ... in one wave", while lines 368-369 and 566-567 describe one feature at a time with a
    review of the plan before the second. "Wave" is not defined.
* **Rationale & Recommendation**: Keep the intent and constraints in sections 8 and 9, and leave file lists to the plan with a
  pointer, or make the two copies match. State in section 8 which items go in PR 1 and which in the last PR. Replace "one wave"
  with "one pull request per feature, all 20 committed to".

### [design-1-4] The review and owner-attention costs are understated, and the estimate does not match the reason Alternative K was rejected - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 372-377, 468-475, 518-523, 564-568
* **Discrepancy Description**: From the skill text, one pass costs 1+1+3+1+3 = 9 reviewer runs. "Converged" needs two consecutive clean
  rounds per looped stage, a minimum of 15. The 5-round cap gives 33 before synthesizer and resolve runs. The design's 15-27 band is
  the reviewers' estimate, and its top is below the cap-derived worst case. The loop skill says "a round with zero findings is
  rare". The 60 repository-reviewer runs count one run per doc, though "each end with no Blocker or Major open" implies re-runs
  after fixes. Alternative K rejects interactive drafting for "more than 80 owner gates", yet the design never estimates the
  number of owner-acceptance entries its own rule will produce, though the only real EGM run left 6 Major open.
* **Rationale & Recommendation**: Model the run count as rounds times reviewers per stage. Count owner acceptances as a cost line.
  Set a stop condition after PR 3, such as a maximum number of runs and acceptances, before PR 4 starts.

### [design-1-13] Goal 4's `speckit` search fails on this design and on its own artifacts - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Also reported as:** `style-1-20` (Major)
* **Affected Scope:** lines 81-85, 395-398
* **Discrepancy Description**: Goal 4 requires that a search for `speckit` finds nothing outside `specs/`, `tasks/reviews/`,
  `docs/adr/`, `docs/governance/` and `docs/owner-decisions.md`. This design sits in `docs/designs/`, which is not on the list, and
  names `/speckit-*` and "Spec Kit" throughout (22 lines of the file contain `speckit`). Its `.reviews/` reports repeat the term, the
  README mapping table will name Spec Kit artifacts, and converted 035 and 036 docs inherit their mentions. The new check would be
  red on its own PR, or the exception list would grow ad hoc.
* **Rationale & Recommendation**: Scope the check to live instruction surfaces (`AGENTS.md`, `CLAUDE.md`, `README.md`, `.claude/`,
  `.github/`, `scripts/`, `docs/engineering/`, `docs/constitution.md`), or add `docs/designs/` to the allow-list. State the case rule
  for `speckit` against `Spec Kit`.

### [design-1-14] The backticked-path check is unspecified and would flag valid text - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 50-52, 81-85, 395-397
* **Discrepancy Description**: `README.md` has 14 backticked path-like tokens, and five do not exist in a clean checkout:
  `infra/docker/.env` and `infra/docker/runner.token` (gitignored), `frontend/apps/studio/dist` and `target/sdk-packages` (build
  output), and `registry.example.com/team/` (not a path). The design does not say which files are in scope, how placeholders such as
  `specs/NNN-slug/` are treated, or how exceptions are listed.
* **Rationale & Recommendation**: Specify the file scope, use `git ls-files` for existence, and keep an explicit allowlist file. Add
  tests for placeholders and build outputs.

### [design-1-15] The pin hashes the wrong files - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 249, 319-324
* **Discrepancy Description**: Claude Code ignores the root `agents/` folder and loads the adapters in `.claude-plugin/agents/`
  (`docs/claude.md` lines 112-121, confirmed). The adapters carry the Goldfish properties: `tools: [Read, Grep, Glob]`,
  `model: inherit` and `omitClaudeMd: true`, and the synthesizer and bootstrap adapters grant Write, Bash and Edit. None of
  `.claude-plugin/` is in the hashed set, and the root `plugin.json` is the OpenAI manifest, not the Claude one. Widening a reviewer's
  tools therefore leaves the hash unchanged. The hashed set does include non-behavioral files (`skills/*/TEST.md`,
  `test_scenarios.yaml`, `assets/icon.svg`, `docs/privacy.md`), so most upstream releases change the hash. `docs/claude.md` line 8
  says isolation needs Claude Code 2.1.271 or later, and the record keeps neither that version nor the model.
* **Rationale & Recommendation**: Hash `.claude-plugin/**` and the Claude manifest. Exclude `site/` and `tests/`. Record the Claude
  Code version and model in the block, and enforce a minimum version.

### [design-1-16] The pin detects drift but cannot reproduce the accepted version, and "caught by the check" overstates what the check sees - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Also reported as:** `style-1-15` (Major; claims exceed mechanism; this part covers lines 234-235, 483 and 556-557)
* **Affected Scope:** lines 148-154, 233-235, 310-331, 415-417, 483, 556-557
* **Discrepancy Description**: Settings cannot pin a version (318), so `claude plugin install egm@egm` installs whatever the
  marketplace holds today, and by the design's own account the tag is already 14 commits behind. A new contributor gets a hash that
  is not on the list and cannot produce an approvable review until the owner reads a new diff of 100+ files. Lines 233-235 say a
  step change is "caught by the check and routed to an amendment", and line 483 repeats it, but the check sees only a different
  hash. Whether a step changed is a human reading. Lines 556-557 say the pin "limits the plugin to files whose hash the owner
  accepted", while lines 316-318 and 415-417 say nothing enforces the version.
* **Rationale & Recommendation**: Document how to install the accepted commit (the list entry records it), or state that no such way
  exists. Name who reads the diff, with a checklist limited to behavioral files. Reword to: "A review record that carries an
  unlisted hash fails the check. The check does not stop a developer from running another EGM version."

### [design-1-17] "Widens no permission" overstates the security posture - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 316-318, 555-560
* **Discrepancy Description**: Enabling the plugin in the committed project `.claude/settings.json` loads EGM into every session of
  every contributor. It adds nine agent types, including a synthesizer with Write and a bootstrap orchestrator with Bash and Edit.
  `check-agent-config.py` forbids those tools for repository reviewers (`WRITE_TOOLS`), and `docs/engineering/ai-engineering.md`
  line 50 and `AGENTS.md` rule 20 treat plugins as supply chain. The claim holds only for `permissions.allow`. The design lens could
  not verify the `autoUpdate` semantics or the trust prompts from the repository, since the design cites URLs only.
* **Rationale & Recommendation**: Restate the NFR with the tool surface each agent adds. Consider user-scope enabling with only the
  marketplace entry committed, and say why project scope is accepted.

### [design-1-18] Goal 3 promises a named carrier for every rule, but the append-only registers have none - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 44-45, 78-80, 349-352, 487
* **Discrepancy Description**: The design says only that `research.md` stays in `specs/` and "must stay append-only". The checks at
  lines 268-281 and 395-398 include none for it, and the design itself notes that no script reads these files today. For a new
  feature, CUJ 1 creates only `docs/designs/NNN-slug.md`. Nothing says where its D/G/O registers, data model, quickstart and
  evidence live, yet the allocator hands out D/G/O ranges (222-223).
* **Rationale & Recommendation**: Add a cheap check that existing D/G/O ids in a changed `research.md` are not deleted or renumbered
  against the base, or state "written rule, unchecked, because ...". State the home of a new feature's artifacts.

### [structural-1-1] No Risks and Mitigations section; the standard's required risk content is spread across three places - Severity: Major
* **Reviewer Lens:** `egm-structural`
* **Affected Scope:** `### 10. What this design does not guarantee` (413-430) and `## Non-Functional Requirements` (553-568)
* **Discrepancy Description**: The standard requires "Risks & Mitigations" and says the spec must name failure modes, security
  implications and scalability limits "and how they will be mitigated". The document has no section with that role. The limits are
  in section 10, an H3 nested in the Technical Plan, and most of its bullets state a limit but no mitigation. The security, rollout
  and cost points are in the NFR section. Rollback appears in section 9 (410-411) and again in the NFR (561-563). The plan (line 618,
  outside the range) calls section 10 "the risks", but the heading does not say so. The design lens judged that the risk content
  exists; the gap reported here is its organization and the missing mitigations.
* **Rationale & Recommendation**: Promote it to an H2 "Risks and Mitigations", ideally a table of risk, mitigation and residual limit,
  and link the NFR items to it. This also makes the content findable by a reviewer who scans for the standard's headers.

### [structural-1-2] This document lacks the Constitution Check and Complexity Tracking sections that the planned check requires of every design doc - Severity: Major
* **Reviewer Lens:** `egm-structural`
* **Affected Scope:** lines 78-80 (Goal 3), 249-254, 270-271; exemption text at plan line 740 (outside the range)
* **Discrepancy Description**: `check-design-docs.py` is specified to require, for every design doc, one Constitution Check row for
  each of principles I to VIII and for the Delivery Constraints, plus a Complexity Tracking section. Section 3 exempts cross-cutting
  docs from the `FR/SC/T` ids only (252-253), and the plan exempts only `TEMPLATE.md` and `README.md` (line 740). This document is in
  `docs/designs/` and is itself a cross-cutting design. It has neither section, so it would fail its own check once PR 2 lands. It
  is also the document that amends the constitution, so a Constitution Check table fits it especially well.
* **Rationale & Recommendation**: Add both sections, or state the exemption for unnumbered docs in section 3 and in the check's
  specification.

### [style-1-7] The verdict set, "unit" and "obligation" are undefined or inconsistent, so Goal 1 cannot be checked as written - Severity: Major
* **Reviewer Lens:** `egm-style` (primary), `egm-structural`, `egm-design`
* **Also reported as:** `structural-1-5` (Nit), `design-1-21` (Nit)
* **Reviewer severity:** style Major, structural Nit, design Nit. Merged at Major because Goal 1 is the first checkable goal and
  "every unit" has no unit list.
* **Affected Scope:** lines 69-72, 92-93, 188-209; row at 202
* **Discrepancy Description**:
  * Goal 1 lists four verdicts (KEEP, MOVE, REWRITE, DROP). Line 188-189 says "the format of the 3.0.0 report plus the verdicts MOVE
    and DROP". The table uses a fifth, ADD (205-207), and has no MOVE row. "Owner decisions: ADD, today in an AGENTS.md bullet" is
    really a move.
  * "Unit" has no definition, and rows differ in size: eight principles in one row (193), four governance topics in another (203),
    one sentence in a third (202). The 3.0.0 report used finer units (V-3, VII-3, D-1, G-6) and RELAX and UPDATE verdicts.
  * "Obligation" is undefined, yet Goals 1 and 7 depend on it.
  * The first column is headed "Unit of 3.0.0", but the three ADD rows are not units of 3.0.0.
  * Row 202 quotes a sentence that is not verbatim: `.specify/memory/constitution.md` line 184 reads "Features 001-012 already exist;
    a new feature continues the sequence from 013" (confirmed). A sentence of the "Spec Kit cycle" bullet also gets its own DROP
    while the bullet has a REWRITE row, so one unit has two verdicts.
  * The footer (Version line and closing link paragraph), which plan item 1 changes, has no row.
  * "Verdict" also names a review outcome at line 57.
* **Rationale & Recommendation**: Define "unit" (for example, one heading or one bullet of the 3.0.0 text) and list the unit set once.
  List the full verdict set (KEEP, MOVE, REWRITE, DROP, ADD) in Goal 1 and at lines 188-189. Rename the first column, add MOVE and
  footer rows, and use "disposition" or "classification" for the table values to avoid the clash with line 57.

### [style-1-12] The `EGM-hash` field and the accepted-hash list are described inconsistently - Severity: Major
* **Reviewer Lens:** `egm-style`
* **Also reported as:** `design-1-25` (Nit; how another harness's install is hashed)
* **Reviewer severity:** style Major, design Nit. Merged at Major.
* **Affected Scope:** lines 258-261, 319-331
* **Discrepancy Description**: Line 261 defines the hash as "of the EGM files that were read", which implies the files a run used.
  Lines 319-321 hash a fixed set (`plugin.json` and four directories). Line 325 says the first list entry "is the commit that ran
  the reviews", but the list holds hashes (326-327), and a commit id is not a tree hash. "EGM hash" and "tree hash" are both used.
  `egm-tree-hash.py` reads only the Claude cache path, yet line 329 accepts other-harness reviews by listed hash, with no stated way
  to hash another install.
* **Rationale & Recommendation**: Write: "Hash computed by `scripts/egm-tree-hash.py` over ... (the set decided in [design-1-15])."
  And: "The first entry is the hash of the installed copy at commit `f0ba151b`." Define how another harness's install is hashed, or
  state that it is not accepted.

### [style-1-18] Two passages promise content that the document does not give - Severity: Major
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** lines 230-231, 108-111, 333
* **Discrepancy Description**: Line 230-231 says `egm-drift-report` "replaces `/speckit-converge` in part only, and the design says
  so." Nothing in lines 1-590 says which part is not replaced, so the reader cannot tell which obligation of `/speckit-converge`
  loses its carrier (Goal 3). Separately, Q1 says no file or branch named "broader reviewed Constitution candidate" was found, yet
  its "plugin rules" are said to overlap section 6 (110-111, 333), and the source of that claim is not named. The candidate also
  appears as "the unadopted 3.0.0 candidate" (333), which can be read as a candidate for the adopted 3.0.0 itself.
* **Rationale & Recommendation**: State the covered and uncovered parts of converge at 230-231. For Q1, name where the plugin rules
  were read and use one name for the candidate.

## Nit

### [design-1-20] "MAJOR as in 2.0.0 and 3.0.0" does not follow from the versioning rule as written - Severity: Nit
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 203, 209
* **Discrepancy Description**: The 2.0.0 and 3.0.0 bumps relaxed or redefined principle obligations
  (`compensation-safety-sync-impact.md` line 10; the "why MAJOR" section of the 3.0.0 report). Here all eight principles are KEEP
  and the versioning clause is KEEP unchanged. Under that clause, adding three sections reads as MINOR.
* **Rationale & Recommendation**: Give the argument for MAJOR, or amend the versioning clause in this change.

### [design-1-22] Factual precision of the Problem section's citations - Severity: Nit
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 37-40, 46-47
* **Discrepancy Description**: In `specs/035-iceberg-write-steps/plan.md` line 115, "Requires ADR" is the Pre-design column and the
  Post-design column reads "Pass pending ADR-0071 acceptance" (both confirmed). The ajv command in `ci.yml` ends at line 341, not
  340. "32 plans" is verifiable (the design lens counted 32), so the "(reported)" tag can go.
* **Rationale & Recommendation**: Quote both columns, correct the line range, and drop "(reported)" where the count was verified.

### [design-1-24] The session-start hook keys on `feature/NNN-slug`, which this design's own branch does not match - Severity: Nit
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** lines 384-386
* **Discrepancy Description**: This design's branch is `feature/egm-replaces-speckit` (no number), and `bugfix/` branches have none
  either. Cross-cutting work gets no duplicate-work warning.
* **Rationale & Recommendation**: State the fallback for branches without a feature number.

### [design-1-26] Alternatives rest on owner preference rather than a stated technical reason; the entries are ungrouped - Severity: Nit
* **Reviewer Lens:** `egm-design`
* **Also reported as:** `structural-1-9` (Nit)
* **Affected Scope:** lines 432-551 (Alternatives A to N); A, C and D at 438-466; H at 492-499
* **Discrepancy Description**: Every entry has Pros, Cons and Reason for Rejection, but 11 of the 14 "Reason for Rejection" fields cite
  an authority ("Owner decision") rather than a reason, and the trade-off sits in Cons. The text itself says no Spec Kit defect was
  named, and that C was the cheapest option and the one every review lens proposed. The standard asks for specific documented
  reasons. Alternatives A to D reuse the brief's alternatives under different letters (design C is brief A, design D is brief B, and
  the brief's "C" is the chosen plan). The 14 entries mix decisions about scope, approval, the record, the pin and the checks, with
  no grouping, so the reader cannot see which option was chosen for each decision without returning to the Technical Plan.
* **Rationale & Recommendation**: Group the entries by decision, state the option chosen for each, and put the technical trade-off in the
  rejection field. For A, C and D add the concrete cost of two formats (double ticking, id clashes). The two missing alternatives
  are covered by [design-1-5] and [design-1-10].

### [design-1-27] Goal 7's "keep and evolve" and Goal 5's title promise more than the mechanism checks - Severity: Nit
* **Reviewer Lens:** `egm-design`
* **Also reported as:** `style-1-15` (the Goal 5 and Goal 7 parts)
* **Affected Scope:** lines 86-88, 91-93
* **Discrepancy Description**: Only the "unchanged" half of Goal 7 is checkable, although line 66 says each goal can be checked. Goal 5
  is titled "The conversion loses no task" but is checked only by ids and counts (359-360, 418).
* **Rationale & Recommendation**: Drop "and evolve" from Goal 7 or give a measure. For Goal 5 write: "The conversion keeps every task id and
  the counts of ticked and open tasks."

### [structural-1-4] The status block is not related to the template header it extends - Severity: Nit
* **Reviewer Lens:** `egm-structural`
* **Also reported as:** `design-1-19` (Nit)
* **Affected Scope:** lines 4, 249-266
* **Discrepancy Description**: Line 249 says EGM's template "has no status block", but the template header already has
  `Status: [Draft | Proposed | In Review | Approved]`, and this document's own header uses it (line 4). The text does not say whether
  the new block replaces or extends the Author, Status and Date lines, or where Author and Date go. The example puts a second
  `**Status:**` line in the doc, so a parser could find two. It does not say what the check does with `Proposed` and `In Review`.
  The example is fenced as `text`, and the section title lists "record, rule and checks" while the body order is Rule, Template,
  Record, checks.
* **Rationale & Recommendation**: State that the block extends the template header, give the exact header layout the script parses,
  say how the check treats `Proposed` and `In Review`, and align the title with the body order.

### [structural-1-6] Two horizontal rules from the template are missing before the trailing sections - Severity: Nit
* **Reviewer Lens:** `egm-structural`
* **Affected Scope:** lines 551-553, 568-570, 582
* **Discrepancy Description**: The template has a rule before NFR, between NFR and Additional Resources, and before the Detailed Plan.
  The document has only the one before the Detailed Plan (line 589) and the one under the header. Without the second rule,
  "Additional Resources" and "Future Improvements" read as sub-sections of Non-Functional Requirements.
* **Rationale & Recommendation**: Restore the two rules.

### [structural-1-7] Cross-references are ambiguous: "section N" collides with other numeric sequences - Severity: Nit
* **Reviewer Lens:** `egm-structural`
* **Also reported as:** `style-1-26` (Nit; bare "section 3" and "Section 5", undated "(owner decision)" tags)
* **Affected Scope:** lines 111, 166, 181, 205-207, 213-214, 216, 223, 241, 256, 333, 341, 399-408; Alternatives C to N (473-550) against 443 and 450
* **Discrepancy Description**: "Section N" is used unqualified while the document has Goal N, Q1 to Q4, ordered-list indexes, and the
  plan's PR N and "##### N" items. In the verdict table, a capital "Section" can read as a section of the constitution.
  Section 9's list indexes are offset from its own PR labels (item 2 is PR 1, item 3 is PR 2, and "Tag develop" is item 1 here but
  "Step 0" in the plan). "Shape of the change" (166) is unnumbered among numbered siblings. Line 223 sends the reader to section 9
  for the id allocator, which that section only schedules. Line 618 (outside the range) calls section 10 "the risks", and line 633
  (outside the range) cites "section 3, 'Template'", which is a run-in bold label (249), not a heading. The tag "(owner decision)" is
  undated, and at line 216 it does not say what it modifies. Only Alternatives A and B carry a date.
* **Rationale & Recommendation**: Write "Technical Plan section N" at first use. Align the section 9 list with the PR numbers. Make the
  Template a heading or reference it by section number only. Move owner-decision tags to a dated list or add the date.

### [structural-1-8] Label styles are mixed, the Open Questions are not all questions, and provenance terms are never introduced - Severity: Nit
* **Reviewer Lens:** `egm-structural`
* **Also reported as:** `style-1-29` (Nit; the form of Open Questions)
* **Affected Scope:** lines 34-55, 69-93, 106-120, 216-231, 249-256, 555-568
* **Discrepancy Description**: Open Questions uses `**Q1: ...?**` with the colon inside the bold, the template says `**Question 1**:`,
  CUJs and Alternatives use `**Given**:` and `**Pros**:` with the colon outside, and Problem, Goals and NFR use `**Label.**` with a
  period. Q3 ("Recommendations.") is a noun label, not a question. Q2 and Q3 are owner confirmations of decisions already made in
  the text. "Owner decision", "reported" and "the reviewers" are used from line 34 and from line 216 on, but the brainstorm that
  produced them is introduced only at lines 434 and 574. (The undefined `REV-003`, "Minor" and "item N" are covered by
  [design-1-1].)
* **Rationale & Recommendation**: Pick one label style. Phrase each Open Question as a question, or move confirmations under an "Owner
  confirmations" heading. Add a one-line provenance note to the Overview.

### [structural-1-10] Four lines exceed 120 columns and several paragraphs are ragged - Severity: Nit
* **Reviewer Lens:** `egm-structural`
* **Affected Scope:** lines 274, 323, 355, 564 (over 120 characters, confirmed); ragged paragraphs at 272-274, 322-324, 564-566
* **Discrepancy Description**: Most paragraphs wrap near 120 columns, but these lines run over, and a short line such as 565 is followed
  by a long one, which is the sign of a hand edit that was not reflowed. The EGM template wraps at about 80 columns, the constitution
  at about 100, and `AGENTS.md` and `sdd-workflow.md` are not wrapped, so the repository has no single convention.
* **Rationale & Recommendation**: Reflow those paragraphs with one formatter setting.

### [structural-1-11] There is not one markdown link in the range - Severity: Nit
* **Reviewer Lens:** `egm-structural`
* **Affected Scope:** lines 1-590; references at 572-580
* **Discrepancy Description**: All internal references are backticked paths and the two URLs are in code spans, so none is clickable.
  `scripts/check-agent-config.py` checks relative links only in `AGENTS.md`, `CLAUDE.md`, `docs/engineering/*.md`,
  `docs/adr/README.md` and the nested `AGENTS.md` files, so a link in this document would not be checked either. The document
  criticizes unchecked pointers (50-52), yet its own pointers are unchecked. The two Claude Code documentation URLs end in `.md` and
  could not be verified offline.
* **Rationale & Recommendation**: Use relative links for existing repository files and confirm the two URLs.

### [structural-1-12] The architecture diagram is misaligned and covers only the artifact layout - Severity: Nit
* **Reviewer Lens:** `egm-structural`
* **Affected Scope:** lines 166-181
* **Discrepancy Description**: In line 179 `specs/NNN-slug/` starts at column 67 while its continuation on line 180 starts at column 70. The
  diagram shows only the artifacts before and after. The six-stage procedure, the approval check and the diff-on-tick rule have no
  diagram, although the template asks for ASCII or Mermaid diagrams to model data flow.
* **Rationale & Recommendation**: Align the columns and add a small flow diagram for the procedure and the approval check.

### [style-1-2] "The reviewers", "lens" and "reviewer" name different groups, and the counts disagree - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Reviewer severity:** Major. Triaged to Nit: the approval rule (224-226, 239-243) names its reviewers explicitly, so the ambiguity
  is in the narrative sections and does not change what is built.
* **Affected Scope:** lines 38, 224-226, 333, 372, 388, 442, 455, 470, 574-575
* **Discrepancy Description**: The phrases can mean EGM's lens reviewers, the three repository reviewers, or the brainstorm panel.
  The counts disagree: "seven lenses" (388), "six-lens review" (574), "All six reviewers" (470). "Every review lens proposed" (455) treats
  lens and reviewer as the same thing.
* **Rationale & Recommendation**: Pick one term for the EGM lens agents, one for the three repository agents and one for the brainstorm
  panel, and state the lens count once for each flow.

### [style-1-3] The number of EGM runs in the repository is stated inconsistently - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Reviewer severity:** Major. Triaged to Nit: a clarity defect in the evidence base, with no effect on the mechanism. The design
  lens confirmed the quoted result (0 Blocker, 6 Major, 2 Nit, all open).
* **Affected Scope:** lines 58-59, 119-120, 326, 430, 559-560
* **Discrepancy Description**: "The one EGM review run in this repository" (58-59) and "The one real EGM run here" (430) conflict with "The
  earlier EGM runs" (119-120) and "the reviews of this design" (326). "Real" implies the other runs were not real. A new reader cannot
  tell how many runs exist or which one is cited.
* **Rationale & Recommendation**: Name each run once: "One EGM review of a prior document (`tasks/reviews/...`, date) ended with 0 Blocker and 6 Major findings."

### [style-1-4] "Draft" carries four meanings - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Reviewer severity:** Major. Triaged to Nit: each use is clear in context and the `Draft` status value is defined once.
* **Affected Scope:** lines 4, 116, 219-220, 243, 254, 362-363, 421-422, 462-463
* **Discrepancy Description**: The word names the status value `Draft` (4, 243), the EGM drafting stage (219), a written first version
  ("after the draft is written", 254) and a Spec Kit feature not yet built ("a draft such as 019 to 032", 363; "drafts or features not
  started", 462). "Converted draft" (362, 421) mixes the first and last senses, and the last sense is never defined.
* **Rationale & Recommendation**: Reserve `Draft` for the status value. Say "feature in specification only" (and define it) for 019-032, and "first version"
  for the text.

### [style-1-5] "Gate" and "owner gate" name different things - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Reviewer severity:** Major. Triaged to Nit: the `Owner-gate:` field is defined at line 305, and the other senses sit in narrative text.
* **Affected Scope:** lines 37, 39, 200, 250, 304-307, 364, 421, 473-474, 513, 521
* **Discrepancy Description**: "Owner gate" names a task-level prerequisite (305, `Owner-gate:`) and an approval checkpoint during interactive
  drafting ("no owner gates", 250; "section gates ... owner gates", 521). "Gate" also names a CI step (200), a review rule (513) and a
  compliance check (37).
* **Rationale & Recommendation**: Use "owner gate" only for the task prerequisite. Call the drafting checkpoint "section approval" and the CI step "CI check".

### [style-1-8] "The check" has several referents, and CUJ 2 and CUJ 3 name none - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Reviewer severity:** Major. Triaged to Nit: CUJ 1 names `scripts/check-design-docs.py`, and the script is the only candidate in context.
* **Affected Scope:** lines 137, 146, 161-163, 235, 306-307, 359, 396-397
* **Discrepancy Description**: The scripts involved are `check-design-docs.py`, `check-agent-config.py` and `check-architecture.py`. "The
  check" is also unrelated to "Constitution Check", a document section (37, 270). CUJ 2 and CUJ 3 give a reader no antecedent.
* **Rationale & Recommendation**: Write `check-design-docs.py` wherever that script is meant. Keep "Constitution Check" only for the section.

### [style-1-9] "Plan" has five meanings, and the location of the task list is stated loosely - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Reviewer severity:** Major. Triaged to Nit: line 287 states the location of the task list explicitly.
* **Affected Scope:** lines 111, 171, 199, 287, 348, 566
* **Discrepancy Description**: The word names `plan.md`, the "Technical Plan" section (111, 348), the "Detailed Implementation Plan" section
  where tasks live (287), "the design doc's plan" (199) and "the plan is revisited" (566). Line 348 ("the plan section with every task")
  does not say which section holds the task list.
* **Rationale & Recommendation**: Say "Detailed Implementation Plan" for the task section every time. Use "this design" or "the conversion schedule" for line 566.

### [style-1-10] The unit counted is "boxes", "tasks" or "checkboxes" - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Reviewer severity:** Major. Triaged to Nit: today every checkbox line in `specs/*/tasks.md` begins with a task id (1,946 = 1,268 open
  + 678 ticked, confirmed), so the counts coincide. The definition is still needed for future docs.
* **Affected Scope:** lines 62-63, 86-88, 279, 359-361, 462
* **Discrepancy Description**: The text gives "1,268 open boxes; 678 boxes are ticked" (62-63), then compares "counts of ticked and open tasks" (87,
  360). "Boxes" is informal, and a converted doc could contain sub-checkboxes.
* **Rationale & Recommendation**: Define "A task is one markdown checkbox line whose text begins with an id." Use only "task" afterwards.

### [style-1-16] Tense and voice do not separate the current state from the proposed state - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Reviewer severity:** Major. Triaged to Nit: line 593 (outside the range) states that nothing is built, and the Status line reads `Draft`.
* **Affected Scope:** lines 316, 319, 384-386, 395, 563
* **Discrepancy Description**: New artifacts are described in the present tense ("`scripts/egm-tree-hash.py` computes", "settings.json
  registers", "gains a check"). Lines 384-386 mix "reads" with "will read". The only statement that nothing is built is at line 593,
  outside the audited range.
* **Rationale & Recommendation**: Add one sentence under the Overview ("Nothing described here exists yet unless marked 'today'") and use one tense for the target state.

### [style-1-19] Numeric claims lack a source or verification status, and one is miscounted - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Reviewer severity:** Major. Triaged to Nit: the figures are labeled estimates or "reported" in the places that drive cost decisions.
* **Affected Scope:** lines 38, 54, 102, 355, 374, 378, 382, 440-441, 463, 514, 521, 541
* **Discrepancy Description**: The "(reported)" label is defined at 34-35 and used at 40, 54 and 102, but not at 440-441 ("About 374", "about
  2,036"), 463 ("About 1,190"), 355 or 378. The same quantity changes precision: "About 24 files ... (reported)" at 102 becomes "the 24 files" at
  382, and "60 in total" (374) becomes "about 60" (541). Line 521 says "Four section gates for each of 20 features is more than 80 owner
  gates", but 4 x 20 = 80, and the four gates are never named. Line 514 states a causal claim ("which is how feature 016 was built twice")
  that line 54 marks only as reported.
* **Rationale & Recommendation**: Mark each figure "measured on <date> by <command>" or "reported", and use the same figure each time. Write "80
  section approvals" and name the four sections.

### [style-1-22] Figurative, subjective or drifting phrasing - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Also reported as:** `style-1-30` (Nit; drift in the verb for "the layout provides the rule")
* **Affected Scope:** lines 25, 31, 32, 35, 39, 58, 59-60, 78-79, 112, 131, 209, 441, 463, 507, 520, 528, 530-531, 547, 559, 585
* **Discrepancy Description**: Examples: "Deleting Spec Kit is easy" (31), which sits oddly beside a 22-PR plan and 1,268 open tasks; "The risk is in
  what the deletion takes with it" (31); "supported without anyone writing them down" (25); "live gate" (39); "moves on" (58); "only as
  strict as its definition of clean" (59-60); "homes" (112); "approve button" (131); "keep their place" (441); "finish where they are" (463);
  "ages" (507); "Full EGM discipline" (520); "half-supplied" (528); "manual look" (547); "instruction-shaped" (559); "drifted" (585). The
  verbs "supported", "supplied", "half-supplied", "relies on" and "carrier" (a coined term that is also the Goal 3 criterion, and at
  209 names the document that holds an obligation) alternate for one idea.
* **Rationale & Recommendation**: "Removing the Spec Kit files is mechanical. The rules that depend on its layout are the risk." Replace the rest with plain
  statements. Pick one verb, and replace "carrier" with "the file, script or written rule that holds it".

### [style-1-23] Unmeasured cost words and open-ended triggers - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** lines 355, 372, 455, 503, 535, 545, 566-567, 587
* **Discrepancy Description**: "The cost is large", "cheapest", "cheap" and "large doc" carry no number. "The plan is revisited" (566) has no
  criterion for what number changes the plan, and "faster than the owner can read" (587) has no threshold.
* **Rationale & Recommendation**: Give a number or drop the adjective. For 566 add a criterion such as "if the first conversion exceeds 27 runs". For 587
  write "if more than N hash additions are requested in a quarter".

### [style-1-24] Terms and acronyms are not defined at first use - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** lines 1, 17, 41, 84, 101, 223, 224, 242, 245, 277, 388, 423, 512, 586
* **Discrepancy Description**: EGM is used in the title (1) before it is expanded (17). Spec Kit is never described apart from its commands. ADR, FR,
  SC and ESCALATE are not expanded. "Harness", "lens" and "readiness standard/rule" are jargon with no definition. The skills
  `egm-review-loop`, `egm-resolve` and `egm-drift-report` are named without a description.
* **Rationale & Recommendation**: Add a short glossary after the Overview, or define each term once at first use.

### [style-1-25] Several names for one role and for one artifact - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** roles at 127, 151, 240, 268, 283, 340-341; artifact at 73, 77, 130, 150, 256, 289, 326, 415, 530
* **Discrepancy Description**: The roles appear as "owner", "author", "author agent", "agent", "person" and "developer", and "the author contests" does
  not say whether the author is a human or an agent. The one status block is called "approval record", "review record", "the record" and
  "status block".
* **Rationale & Recommendation**: Define "author" (human or agent) once. Use "status block" for the block and "approval record" for the data in it.

### [style-1-27] Loose modals and small wording defects - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** lines 70-71, 89-90, 305, 351-352, 428-429
* **Discrepancy Description**: "May appear only" (70-71) states a restriction and should be "appears only". Goal 6 says the registers "stay editable" (90)
  while 351-352 say "append-only". "Changes what 'reviewed' meant" (428-429) is in the past tense for a future condition.
* **Rationale & Recommendation**: "A REWRITE or DROP appears only in ..." and "the registers stay append-only".

### [style-1-28] Content is stated twice in nearly the same words - Severity: Nit
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** lines 282-283 against 415-417; lines 362-366 against 421-422
* **Discrepancy Description**: The caveat about review in a fresh context, and the one about an `Approved` converted draft being read as
  permission to build, are each stated twice.
* **Rationale & Recommendation**: Keep the full statement in section 10 and give a pointer elsewhere.

---

## Traceability: every raw finding and the entry that holds it

| Raw id | Raw severity | Report entry |
| --- | --- | --- |
| design-1-1 | Major | design-1-1 |
| design-1-2 | Major | design-1-2 |
| design-1-3 | Major | design-1-3 |
| design-1-4 | Major | design-1-4 |
| design-1-5 | Major | design-1-5 |
| design-1-6 | Major | style-1-1 (Blocker) |
| design-1-7 | Major | design-1-7 |
| design-1-8 | Major | design-1-8 |
| design-1-9 | Major | design-1-9 |
| design-1-10 | Major | design-1-10 |
| design-1-11 | Major | design-1-11 |
| design-1-12 | Major | design-1-12 |
| design-1-13 | Major | design-1-13 |
| design-1-14 | Major | design-1-14 |
| design-1-15 | Major | design-1-15 |
| design-1-16 | Major | design-1-16 |
| design-1-17 | Major | design-1-17 |
| design-1-18 | Major | design-1-18 |
| design-1-19 | Nit | structural-1-4 |
| design-1-20 | Nit | design-1-20 |
| design-1-21 | Nit | style-1-7 (Major) |
| design-1-22 | Nit | design-1-22 |
| design-1-23 | Nit | style-1-14 (Major) |
| design-1-24 | Nit | design-1-24 |
| design-1-25 | Nit | style-1-12 (Major) |
| design-1-26 | Nit | design-1-26 |
| design-1-27 | Nit | design-1-27 |
| structural-1-1 | Major | structural-1-1 |
| structural-1-2 | Major | structural-1-2 |
| structural-1-3 | Major | structural-1-3 |
| structural-1-4 | Nit | structural-1-4 |
| structural-1-5 | Nit | style-1-7 (Major) |
| structural-1-6 | Nit | structural-1-6 |
| structural-1-7 | Nit | structural-1-7 |
| structural-1-8 | Nit | structural-1-8 (the `REV-003` and "Minor" parts also in design-1-1) |
| structural-1-9 | Nit | design-1-26 |
| structural-1-10 | Nit | structural-1-10 |
| structural-1-11 | Nit | structural-1-11 |
| structural-1-12 | Nit | structural-1-12 |
| style-1-1 | Blocker | style-1-1 |
| style-1-2 | Major | style-1-2 (Nit) |
| style-1-3 | Major | style-1-3 (Nit) |
| style-1-4 | Major | style-1-4 (Nit) |
| style-1-5 | Major | style-1-5 (Nit) |
| style-1-6 | Major | design-1-2 |
| style-1-7 | Major | style-1-7 |
| style-1-8 | Major | style-1-8 (Nit) |
| style-1-9 | Major | style-1-9 (Nit) |
| style-1-10 | Major | style-1-10 (Nit) |
| style-1-11 | Major | design-1-3, with parts in design-1-1 |
| style-1-12 | Major | style-1-12 |
| style-1-13 | Major | design-1-9 |
| style-1-14 | Major | style-1-14 |
| style-1-15 | Major | design-1-16 (pin claims) and design-1-27 (Goal 5 and 7 claims) |
| style-1-16 | Major | style-1-16 (Nit) |
| style-1-17 | Major | structural-1-3 |
| style-1-18 | Major | style-1-18 |
| style-1-19 | Major | style-1-19 (Nit) |
| style-1-20 | Major | design-1-13 |
| style-1-21 | Major | design-1-11 |
| style-1-22 | Nit | style-1-22 |
| style-1-23 | Nit | style-1-23 |
| style-1-24 | Nit | style-1-24 |
| style-1-25 | Nit | style-1-25 |
| style-1-26 | Nit | structural-1-7 |
| style-1-27 | Nit | style-1-27 |
| style-1-28 | Nit | style-1-28 |
| style-1-29 | Nit | structural-1-8 |
| style-1-30 | Nit | style-1-22 |

## Claims the reviewers checked and found to hold

* **Design lens:**
  * CI lints the four contract files at `ci.yml` lines 320-341.
  * `check-agent-config.py` checks only relative links and rule numbering.
  * The session-start hook reads `.specify/feature.json`.
  * The counts are right: 35 feature directories, 20 features with open tasks, 1,268 open boxes and 678 ticked.
  * 32 plans carry a Constitution Check.
  * The constitution is at 3.0.0, ratified 2026-10-11, last amended 2026-11-01.
  * The prior EGM run ended with 6 Major, 2 Nit and 0 Blocker, all open.
* **Structural lens:**
  * The heading hierarchy is clean: no skipped level, no duplicate heading, sections 1 to 10 and Alternatives A to N continuous.
  * The only table (191-207) has 3 columns in every row. Both code fences are closed.
  * Bullet and numbered-list styles are consistent, and there is no trailing whitespace or tab in the range.
  * Every checkable "section N" reference resolves to the intended section, and every existing path cited exists.
  * The "1-Audit Convergence Cap" is at `egm-review/SKILL.md:252`.
  * `sdd-workflow.md` item 4 is the recommendation rule, and its "eight repository additions" and "nine-step table" counts are correct.
