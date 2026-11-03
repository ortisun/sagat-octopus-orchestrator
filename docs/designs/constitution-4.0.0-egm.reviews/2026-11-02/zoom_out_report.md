# Goldfish Review Report

**Target:** `/Users/marcelo/Documents/Projects/sagat-octopus-egm/docs/designs/constitution-4.0.0-egm.md` (`1..878`, the full file)

**Standard:** `/Users/marcelo/.claude/plugins/cache/egm/egm/0.1.0/resources/document_standards/zoom-out-standard.md`

**Mode:** REVIEW

## Reviewer Status

| Reviewer | Status |
| --- | --- |
| `egm-zoom-out` | completed |

## Severity Breakdown

| Severity | Count |
| --- | ---: |
| Blocker | 0 |
| Major | 6 |
| Nit | 6 |

**Verdict of the zoom-out lens:** the design does not pass the Zoom-Out Standard as written. The context check (standard section 0) passes: the Overview and "The Problem" state what is being done and why. Six open Major findings keep the verdict at "not passing".

## Prioritized Findings (Blockers -> Major -> Nits)

No Blocker was reported.

### 1. The EGM hash pin does not cover what Claude Code loads, and CI cannot recompute it - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** "6. Pinning the EGM" (lines 310-333); Step 20 (743-748); Step 21 (750-754); Non-Functional Requirements, "Security and supply chain" (555-560); CUJ 4 (148-154).
* **Discrepancy Description**: The hash covers `plugin.json` plus the files under `agents/`, `docs/`, `resources/` and `skills/`. Claude Code loads the adapters listed in `.claude-plugin/plugin.json`, and those live in `.claude-plugin/agents/*.md`. `docs/claude.md:99-121` of the plugin says the root `agents/` folder is ignored when the manifest lists adapters. The adapters carry the read-only `tools: [Read, Grep, Glob]` and `omitClaudeMd: true` settings (for example `.claude-plugin/agents/zoom_out.md:5,8`). An edit that widens a reviewer's tools would leave the hash unchanged, so the statement that the pin "limits the plugin to files whose hash the owner accepted" is false as specified. Separately, CI has no plugin installed, so `check-design-docs.py` can only compare a string that the author agent typed against the list. That detects accidents and nothing more, which Section 10's first bullet already admits.
* **Rationale & Recommendation**: The design already knows the plugin commit (`f0ba151b`). Record the plugin commit or the installed version record in the status block and keep the accepted list as those values. That removes `egm-tree-hash.py`, its tests and the file-set decision. If the hash is kept, it must include `.claude-plugin/` (adapters and manifest), and the design must say that CI only compares strings.

### 2. The reviewed-commit diff mechanism carries the cost that Alternative H rejected, and its grammar does not match the repository's data - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** "4. Task state after approval" (285-296); "3. Approval: record, rule and checks" (268-283); CUJ 2 (133-138); Alternative H (492-499); Step 26 (785-789); Section 10 (416-417).
* **Discrepancy Description**: Alternative H is rejected because a normalizing rule "can hide text in evidence lines". The chosen design accepts any line starting with `*Evidence` or a dated "partly done" line, and Section 10 concedes that those lines are unreviewed text. The weakness is the same. The chosen design adds costs that H does not have: `fetch-depth: 0` in CI, a merge-commit-only policy, reachability failures when a branch is rebased (the conversion expects six worktrees to rebase, lines 369-370 and 408), and a self-reference, because the status block that holds `Reviewed-commit` is written after the review. The evidence syntax is described three ways: a line beginning `*Evidence` (line 290), a dated `Evidence:` line (138, 280) and a date plus "partly done" (290). Repository check: no `specs/*/tasks.md` has a line that starts with `*Evidence` (zero matches). Ticked tasks carry an inline `**Evidence (2026-11-01)**:` appended to the task line (20 occurrences in `specs/037-compensation-safety/tasks.md`; the reviewer counted 22 in all, for example `specs/037-compensation-safety/tasks.md:59`). Unticked tasks already contain the word "Evidence:" as the required evidence (`specs/019-worker-deployments-dispatch/tasks.md:15-19`). An inline append changes more than the checkbox, so the rule "only a checkbox and an evidence line changed" would reject the existing practice.
* **Rationale & Recommendation**: Store a hash of the normalized body (ticks reset, evidence stripped) in the status block. That removes git history from the check and removes the merge-commit and full-history requirements. If the diff approach stays, state one evidence grammar taken from the real data and use a single diff base.

### 3. The 20-feature conversion wave and the 23-PR plan are disproportionate to the root problem, and the check does not fit the population it converts - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Goals 5 and 6 (86-90); "7. Conversion of the 20 features" (335-374); "9. Landing order and rollback" (399-411); the PR 3-22 table (796-825); Alternatives C and D (453-466).
* **Discrepancy Description**: The root problem is to stop using Spec Kit without losing obligations. Fifteen of the 20 features have 0 ticked tasks and have not started (019-032 and 036), so nothing is lost by leaving them in `specs/` until they are built, which is Alternative D. The fidelity check protects ticked tasks in only five features: 014, 018, 033, 035 and 037. The cost is 300-540 reviewer runs plus about 60 repository-reviewer runs, all unmeasured (372-374), and the Verification Plan (874-875) already plans to revisit the design after PR 3. The deletion of `.specify/` and the skills is the owner's actual request; it is parked at PR 23, behind the wave, though nothing in the design makes the deletion depend on the conversions. The required-headings check also fails a faithful conversion of 033: `specs/033-redis-streams/plan.md:19` has only a one-line "Constitution:" note, with no Check table and no Complexity Tracking. The other 19 plans have Complexity Tracking, and 033 is the one of the 20 without a Check. Converting a feature with one open task would force the agent to author content, which contradicts "mapping, not authoring" (340-343).
* **Rationale & Recommendation**: Scope this design to PR 1, PR 2 and the removal. Record the conversion as a follow-up decision taken after the first conversion measures the cost, with just-in-time conversion for the unstarted drafts. The owner decisions on this point (Alternatives C and D) stay on record, but the design should state them as an explicit choice and not as a precondition for removing Spec Kit. If the wave stays, give the heading check an exemption path or accept that 033 is not a mechanical mapping.

### 4. "Approved" is overloaded, and the owner gate is a leaky patch for it - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** "3. Approval" (237-243 and 249-254); "5. Owner decisions: gates on tasks" (298-308); "7. Conversion", last paragraphs (362-366); Section 10 (421-422); Alternative E (468-475).
* **Discrepancy Description**: EGM's template already defines `Status: [Draft | Proposed | In Review | Approved]` (`/Users/marcelo/.claude/plugins/cache/egm/egm/0.1.0/skills/egm-draft/references/design_doc.md:4`), so line 249's claim that the template has "no status block" understates what exists. Under the design, a review-clean but unscoped draft becomes `Approved`, while `SPECIFICATION-INDEX.md:83-86` says the drafts are "none approved". Section 10 admits that the owner gate "is the only barrier" and covers only tasks the author marked.
* **Rationale & Recommendation**: A cost-free alternative is unused: leave review-clean docs at `Proposed` and reserve `Approved` for the owner. That needs no extra state beyond the one the EGM template already defines. The design records this as an owner decision, so the owner should re-decide it knowingly, with the consequence stated plainly: "Approved" would no longer mean "the scope is accepted".

### 5. The id allocator is scope creep beyond what Spec Kit supplied - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** "2. The procedure inside the constitution", stage 2 (219-223); Step 25 (775-783); Alternative N (543-551); Section 10 (425-426).
* **Discrepancy Description**: The only Spec Kit allocator is `.specify/scripts/bash/create-new-feature.sh:111-134`, which takes the highest number in the local `specs/` directory and adds one. Scanning `origin/*`, worktrees, a reserved-range table and the D/G/O/ADR number blocks is a new capability. The D/G/O/ADR numbering lives in `research.md` and `docs/adr/`, which the design leaves alone. The duplicate-work incident (feature 016) is already covered by the session-start hook, which the design keeps, and `specs/AGENTS.md:12` already states the manual rule. The "warn if the last fetch is older than a day" logic, its test and its Section 10 caveat can go if the script runs `git fetch` itself, which `.claude/settings.json:21` already allows.
* **Rationale & Recommendation**: Ship only the duplicate-number check (the first option of Alternative N). Defer the allocator, the range table and the `SPECIFICATION-INDEX.md` rewrite from prose to table. This removes one script, one test file and a row of Section 10.

### 6. `check-design-docs.py` bundles too many rule families, and some are redundant or misplaced - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Step 19 (733-741); "3. Approval: record, rule and checks" (268-283); Step 24 (769-773); Section 8, last bullet (395-397); Verification Plan (866-867).
* **Discrepancy Description**: The script bundles more than ten rule families. Cross-doc id uniqueness (line 276) is already implied by "prefix equals doc number" plus unique doc numbers. The FR/SC-to-task coverage rule will be applied to converted drafts whose sources are unmeasured, so the first run may fail many of them. The superseded-body check and the conversion-fidelity check are one-time migration scaffolding made permanent. Report-count parsing reads free-text LLM reports, which is brittle, and the author agent writes both the block and the reports. The version-line, eight-headings and no-`speckit` checks are constitution checks, so they do not belong in `check-agent-config.py`. The no-`speckit` check duplicates the one-time grep in the Verification Plan; once the paths are deleted, the backticked-path check already catches dangling `.specify/...` pointers, and the no-`speckit` check needs a growing exception list.
* **Rationale & Recommendation**: Measure the coverage rule or make it forward-only. Reduce the superseded files to a stub that points at the design doc and run the id-and-count comparison once in each conversion PR, not as a permanent CI rule. Move the constitution checks to the design-doc checker (or a small constitution checker). Drop the standing no-`speckit` check in favour of the one-time search plus the backticked-path check.

### 7. Factual errors about the current checker that affect the justification - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** "The Problem", "Pointers that nothing checks" (50-52); Section 8, last bullet (395-397); Step 24 (769-773).
* **Discrepancy Description**: The design says `check-agent-config.py` "checks relative links and rule numbering only" (50-52, 397). It also enforces line budgets, the `CLAUDE.md` import, skill and agent front matter, reviewer allowlists, hook existence, the `.env` deny rule and the ADR format (`scripts/check-agent-config.py:66-199`). Its link check scans only `AGENTS.md`, `CLAUDE.md`, `docs/engineering/*.md`, `docs/adr/README.md` and nested `AGENTS.md` (lines 188-189). It does not scan `README.md`, `SPECIFICATION-INDEX.md`, `specs/README.md`, `.claude/agents` or `.claude/skills`, which are the files that "The Problem" says are exposed. The extension must therefore widen the scanned set, and the design does not say so. The backticked-path check has no rule for what counts as a path: backticked non-paths and gitignored files are common (`CLAUDE.local.md` at `CLAUDE.md:11`, `.specify/feature.json`, `sdks/python/.venv` in `AGENTS.md`). Step 24 also lists an "Enforced by" path check that Section 8 omits.
* **Rationale & Recommendation**: Correct the description of the current checker, name the files the new check scans, and add one sentence on what counts as a path. The backticked-path check itself is the justified root-cause fix and should stay.

### 8. The hook change is larger than needed - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Step 23 (762-767); Section 8, `session-start.py` bullet (384-386).
* **Discrepancy Description**: The hook's valuable behaviour is the duplicate-work warning (`.claude/hooks/session-start.py:28-35`), and it needs only `NNN` from the branch name. The design-doc lookup and the `specs/NNN-slug/` fallback add nothing to that. The hook is a top-level script, so its first test needs subprocess execution or a refactor.
* **Rationale & Recommendation**: Read the number from the branch name and keep the warning. Drop the lookup and the fallback, or justify them with a behaviour someone relies on.

### 9. Permissions and the command list are inconsistent with the new check - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Step 22 (756-760); Non-Functional Requirements (557-559); Steps 14 and 27 (the `verify` skill).
* **Discrepancy Description**: `.claude/settings.json:13-15` allow-lists the exact check commands. The new `check-design-docs.py` has no entry, so agents are prompted on every run, and the design says it "does not add to the `permissions.allow` list". The `AGENTS.md` Commands line (43) also omits the new script.
* **Rationale & Recommendation**: Either add one allow entry (a small, explicit widening the design should state) or say that the prompt is accepted. Add the script to the `AGENTS.md` Commands block.

### 10. The Step 0 tag is not needed for reachability - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Step 0 (598-601); "Files that are read and not changed", last bullet (855-856).
* **Discrepancy Description**: The tag is an external push. The base commits cited by the 014 and 037 manifests are on `develop`, and Section 4 forbids squashing, so those commits stay reachable without a tag.
* **Rationale & Recommendation**: Drop the tag, or state which commit would otherwise become unreachable.

### 11. Altitude drops: per-line edit lists and a snapshot table - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Steps 6-18 and 24 (640-729); the table at 802-823.
* **Discrepancy Description**: Per-line edits such as `AGENTS.md` lines 3, 7, 9, 11 and 16 and `README.md` lines 677, 681, 721 and 722 are accurate today but will rot before they are applied. The proposed backticked-path and no-`speckit` checks would enumerate them anyway. The per-feature ticked and open counts are a snapshot (018 and 035 are in flight), and the fidelity check recomputes them.
* **Rationale & Recommendation**: State the rule ("remove Spec Kit wording; keep rule numbers") and let the checks list the leftovers. Replace the 20-row table with the list of features and the rule that counts are compared at conversion time.

### 12. Landing order puts normative text before the machinery it cites - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** "9. Landing order and rollback" (399-411).
* **Discrepancy Description**: PR 1 rewrites the constitution and `AGENTS.md` to cite an approval record and checks that PR 2 builds. Between the two merges the normative text describes machinery that does not exist.
* **Rationale & Recommendation**: Land the additive tooling first (PR 2 before PR 1), or merge both in one change, so the constitution never cites a missing check.

## Verified Claims (spot-checked by the reviewer against the repository)

* The `contracts` job reads `specs/001-platform-foundation/contracts/` at `.github/workflows/ci.yml:320-341`. The `architecture` job is at lines 165-176 and runs unittest discover, `check-architecture.py`, `check-capacity-activation.py`, `check-upgrade-boundary.py`, `check-vendor.py` and `check-agent-config.py`.
* Open tasks total 1,268 across 20 files and ticked tasks total 678. Every row of the PR 3-22 table matches the per-file counts.
* 32 `plan.md` files contain a "Constitution Check". `.specify/` has 21 tracked files and there are 10 `speckit-*` skills. Five evidence JSON files cite `.specify/memory/constitution.md`.
* Nothing under `scripts/`, `tests/`, `crates/` or `.github/` reads `.specify` or `speckit`.
* The new `check-*.py` plus `scripts/tests/test_*.py` shape matches the existing pattern in the `architecture` job. Declaring the marketplace in `.claude/settings.json` is the platform-standard mechanism.

## Not Verifiable From the Repository

The document labels these "reported", and the reviewer could not confirm them:

* the clone-versus-install file differences (lines 322-324);
* the upstream tag `v0.1.0` being 14 commits and 105 files behind (312-315);
* the 374 files that cite ids, the 2,036 register rows and the 24 files that cite rule numbers;
* the estimate of 15 to 27 reviewer runs per feature.

## Synthesis Notes

* The harness flagged the reviewer's output as instruction-shaped because it quotes a `.claude/settings.json` path. The content held findings only. It was treated as data and nothing in it was acted on beyond recording the findings.
* The synthesizer spot-checked four reviewer claims before writing this report and all held: `.claude-plugin/agents/` and `.claude-plugin/plugin.json` exist in the installed plugin and the adapter carries `tools: [Read, Grep, Glob]` (Finding 1); no `tasks.md` has a line starting with `*Evidence` and the inline `**Evidence (date)**:` form exists (Finding 2); the EGM template defines a four-value `Status` field at `design_doc.md:4` (Finding 4); `git fetch` is allow-listed at `.claude/settings.json:21` (Finding 5). The count of inline evidence lines differs slightly from the reviewer's (20 in 037 on the spot-check, 22 reported across features), which does not change the finding.
* One reviewer ran (instance A only). No second independent instance was launched.
* The remaining findings were not independently re-verified by the synthesizer.
