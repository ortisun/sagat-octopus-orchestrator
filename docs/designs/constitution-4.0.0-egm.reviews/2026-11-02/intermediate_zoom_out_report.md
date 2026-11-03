# Goldfish Review Report (Stage 8, Intermediate Zoom Out)

**Target:** `docs/designs/constitution-4.0.0-egm.md` (`1..1216`, the full file; the header `## Detailed Implementation Plan` is at line 908, bounds confirmed by the synthesizer and the reviewer)

**Standard:** `~/.claude/plugins/cache/egm/egm/0.1.0/resources/document_standards/zoom-out-standard.md`

**Mode:** REVIEW

**Earlier report read by the reviewer:** `docs/designs/constitution-4.0.0-egm.reviews/2026-11-02/zoom_out_report.md` (stage 5, 878-line version of the document)

## Reviewer Status

| Reviewer | Status |
| --- | --- |
| `egm-zoom-out` | completed |

One instance ran (no second independent instance). The reviewer had Read, Grep and Glob only.

## Severity Breakdown

| Severity | Reported by `egm-zoom-out` | Added by the synthesizer's spot-check | Total |
| --- | ---: | ---: | ---: |
| Blocker | 0 | 0 | 0 |
| Major | 5 | 1 | 6 |
| Nit | 11 | 0 | 11 |

The synthesizer-added finding (finding 6) is marked as such. It was not raised by the reviewer.

## Verdict

The document does not pass the Zoom-Out Standard as written. The context check (standard section 0) passes: the Overview, Terms and "The Problem" say what is being done and why. The document solves the root problem, because the carrier table (Technical Plan section 10), the constitution amendment, the pointer fixes and the deletion all address losing obligations when Spec Kit goes.

Of the six stage 5 Majors, none is fully resolved. Two are partly resolved (1 and 6). Four are contested by a recorded owner decision (2, 3, 4 and 5), and the document states their consequences plainly, but by its own rule (lines 202-211) a contest counts as settled only after the owner accepts it in writing, and no such acceptance exists yet. The additions introduced six new Majors, three of which are defects in rules that the check cannot enforce as written or that contradict other sections (findings 1, 2 and 6).

**Are the additions larger than the root problem needs? Yes, in part.** The document grew from 878 to 1,216 lines (+338, +38.5%). Some additions earn their place; others do not (see "Size and Proportion").

## Disposition of the Stage 5 Findings

| # | Stage 5 finding | Status now | Evidence |
| --- | --- | --- | --- |
| 1 (Major) | EGM hash pin does not cover what Claude Code loads; CI cannot recompute it | PARTLY RESOLVED | The hash set now includes `.claude-plugin/` and its adapters (lines 516-524). Section 6 (line 533) and the Risks table (line 682) say CI compares strings only. Remaining: `egm-tree-hash.py`, its tests and the file-set decision stay. Alternative I rejects "record only the version", not "record the plugin commit", which Claude Code already stores (`~/.claude/plugins/installed_plugins.json:46`, `gitCommitSha`, per user and outside the repository). See finding 7. |
| 2 (Major) | Reviewed-commit diff carries the cost Alternative H rejected; grammar does not match the data | CONTESTED BY A RECORDED DECISION (grammar half: PARTLY RESOLVED) | Recorded in Alternative H (lines 787-795) and "Contested findings" item 4 (line 211); the drafting decision is in `tasks/reviews/2026-11-02-egm-constitution/concepts_brief.md` ("Decisions taken while drafting", line 277). The consequence is stated. The rejection reasoning is not sound at this altitude: the weakness is said to be "handled" by one convention, one CI line and a `Reviewed-blob` fallback that cannot work (finding 1). The `Reviewed-blob` fallback was added by the Critic round, not decided by the owner, so it can be deleted without reversing the owner's choice of a diff. The evidence grammar now matches only part of the real data (finding 8). |
| 3 (Major) | 20-feature wave and 23-PR plan disproportionate; check does not fit the population | CONTESTED BY A RECORDED DECISION | Recorded in Alternatives C and D (lines 739-754) and "Contested findings" item 1. The cost model and a stop condition (lines 592-600) now state the consequence, but the figures are understated (finding 3). The 033 heading problem is handled by a "no Check in the source plan" rule (lines 560-562) that the check itself then rejects (finding 2). The deletion of `.specify/` still waits behind the wave only through the transitional clause (lines 637-639), which breaks the Spec Kit skills it protects (finding 6). Q4 (lines 182-184) leaves early removal open. |
| 4 (Major) | "Approved" is overloaded; the owner gate is a leaky patch | CONTESTED BY A RECORDED DECISION | Recorded in Alternative E (lines 758-765), "Contested findings" item 2, Section 5 (lines 487-489), Section 7 (lines 583-586) and a Risks row (line 677: "That gate is the only barrier"). The consequence is stated plainly. The understatement about the template's Status field is fixed (line 394). The "adds a step" rejection does not address the no-extra-state variant that leaves review-clean docs at `Proposed`, a status the EGM template already defines (`design_doc.md:4`). Q3 reopens a related variant. |
| 5 (Major) | The id allocator is scope creep | CONTESTED BY A RECORDED DECISION | Recorded in Alternative N (lines 857-865) and "Contested findings" item 3; the accepted cost is stated. One sub-point is untouched: the "last fetch older than a day" warning stays (Step 7, lines 981-982; Risks line 679) although `.claude/settings.json:21` already allows `git fetch`, so the script could fetch itself. `SPECIFICATION-INDEX.md` is edited twice (Steps 7 and 18). |
| 6 (Major) | `check-design-docs.py` bundles too many rule families | PARTLY RESOLVED | Resolved: coverage is forward-only and measured on the first conversion (lines 433-435); the id-and-count comparison runs once per conversion (lines 568-570); the constitution checks moved into the design-doc checker (lines 438-439, 620-621); findings footers replace free-text count parsing (but see finding 4). Remaining: the superseded-body check stays permanent; the standing `speckit` search stays (off until PR 23); cross-doc id uniqueness stays. New families were added (append-only `research.md` check, blob fallback, minimum Claude Code version, owner-gate parser). Step 1 now lists about 15 families against "more than ten" at stage 5, and Section 3 lists only six, so the total is stated nowhere. |
| 7 (Nit) | Factual errors about `check-agent-config.py` | RESOLVED | Lines 93-98 and 617-621 match `scripts/check-agent-config.py:66-199` and `:188-189` (synthesizer re-read). |
| 8 (Nit) | Hook change larger than needed | RESOLVED | Step 5 (lines 959-965) reads the number from the branch name and keeps the warning; the lookup and fallback are gone. Matches `.claude/hooks/session-start.py:28-35`. |
| 9 (Nit) | Permissions and command list inconsistent | RESOLVED | Step 4 adds one allow entry; the Non-Functional Requirements state the widening (lines 874-876); Step 10 adds the script to the `AGENTS.md` Commands block. |
| 10 (Nit) | Step 0 tag not needed for reachability | PARTLY RESOLVED | The tag is now optional and the text says the manifests' base commits stay reachable without it (lines 917-920). It is still justified as a "restore point for the rollback", yet rollback is a forward amendment (line 641). Drop it. |
| 11 (Nit) | Altitude drops: per-line edit lists and a snapshot table | STILL OPEN | Steps 15-27 still list line numbers; the 20-row snapshot table remains (lines 1139-1160). Only a rule sentence was added (lines 604-605). |
| 12 (Nit) | Landing order puts normative text before its machinery | RESOLVED | Tooling is PR 1 and the constitution PR 2 (lines 625-635). Residuals: findings 4 and 9. PR 1 also enables the plugin in settings before PR 2's pin rule exists. |

## Size and Proportion

* **Line count.** 1,216 lines now against 878 at stage 5: +338 lines, +38.5%.
* **Where the lines sit.** Technical Plan 410 lines, Detailed Implementation Plan 305, Alternatives 151, Open Questions plus owner confirmations plus contested findings 46, Terms 36, Risks 17, Constitution Check 16, Complexity Tracking 10, carrier table 24.
* **Machinery.** New scripts: 3, unchanged since stage 5 (`check-design-docs.py`, `egm-tree-hash.py`, `next-feature-id.py`). New test files: 4, plus 1 modified. Two existing scripts modified. About 15 rule families in `check-design-docs.py`. A 12-line status block. 15 alternatives, 13 risk rows, 29 numbered steps plus Step 0, 23 PRs plus the optional tag. Q1 to Q5 open, plus 4 contested findings.
* **Net effect.** No stage 5 area got smaller, and the rule-family count went up.
* **Additions that earn their place.** The carrier table (serves Goal 3 and the root problem directly; it covers all 10 Spec Kit skills and all 8 items of "The Problem"). The stop condition. The Risks table. The Constitution Check and Complexity Tracking sections, which the constitution's own compliance rule requires. Terms, which is partly a comprehension fix.
* **Additions that do not.** The reachability and blob-fallback block, the amendment path with its formatting exception, the findings-footer contract, the append-only `research.md` check, and 46 lines of review-round process state in the body (Open Questions and "Contested findings", which belong in a review log or in `docs/owner-decisions.md` once the owner answers).
* **One cheap structural change.** Replace `Reviewed-commit`, `Reviewed-blob` and the reachability measures with a single hash of the normalized body (status block excluded, ticks reset, evidence stripped). It removes the largest piece of machinery added since stage 5 and fixes finding 1 at the same time. The owner decided on a diff (Alternative H), so the owner has to choose. The narrower fix, deleting only the fallback and stating that a rebased branch needs a re-review, stays inside the owner's decision.

## Prioritized Findings (Blockers -> Major -> Nits)

No Blocker was reported.

### 1. The `Reviewed-blob` fallback cannot work as written, and the reachability measures do not cover the failure they name - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 3 "Record" (line 411) and the check list (lines 427-430); Section 4 "Reachability" (lines 477-483); Alternative H (lines 793-795); Step 1 (lines 927-930); Risks (line 681).
* **Discrepancy Description:** The author writes the status block after the review (line 421). At the reviewed commit the doc held a Draft block with no `Reviewed-blob` field. Adding the Approved block changes the blob, and every later tick changes it again. So "the blob at this path equals `Reviewed-blob`" (line 428) holds only for a doc unchanged since review, which is the case that needs no diff. A blob id also cannot sit inside the file it identifies, so the self-reference that stage 5 Major 2 named returns through the fallback. Of the three reachability measures, the merge-commit rule is a convention (the document admits no setting enforces it), full-history checkout is one CI line, and the fallback does not function. Alternative H nevertheless says the weakness "is handled by" all three. Alternative H also conflates two options (a hash that ignores ticks, and a sibling ledger); its remaining objection, that the normalizer can hide text in evidence lines, applies equally to the chosen design.
* **Rationale & Recommendation:** Either record one hash of the normalized body (the evidence grammar at lines 455-460 already defines the normalization), which removes `Reviewed-blob`, `fetch-depth: 0` (Step 9), the merge-commit convention and the disposable-git-repository tests (line 936, Verification Plan line 1201); or, if the owner keeps the diff, delete the fallback and state that a rebased branch needs a re-review. Keep `Reviewed-commit` as information. *Synthesizer check: confirmed by reading lines 411, 421 and 427-430.*

### 2. The required-sections rule cannot be met by "mapping" for most of the 20 conversions - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 3 (lines 425-426); Section 7 (lines 560-562); Sync Impact row (line 324); Constitution Check table (lines 687-702).
* **Discrepancy Description:** The check demands one Constitution Check row for each of principles I to VIII and one for the Delivery Constraints, plus a Complexity Tracking section. The real plans do not carry that. Only 4 of the 20 plans have any Delivery Constraints row or item (`specs/032-multi-region-failover/plan.md:73`, `specs/036-elastic-scaling/plan.md:46`, `specs/028-developer-tooling/plan.md:347`, `specs/035-iceberg-write-steps/plan.md:115`). `specs/033-redis-streams/plan.md` has a one-line note at lines 19-22, no table and no Complexity Tracking. Section 7 tells the agent to write "no Check in the source plan" and not to author one, so the check then fails. For the 16 plans without a Delivery Constraints row, passing means authoring a row, which contradicts "mapping, not authoring" (lines 548-552). Line 324 also says the Compliance-review obligation "is the same" as in 3.0.0, which required a Check and never a Delivery Constraints row.
* **Rationale & Recommendation:** For converted docs, check only that the two sections exist and are non-empty, and make the row-per-principle rule forward-only (as the coverage rule already is). Or state that 16 conversions need an authored row and drop the "mapping" claim for them. *Synthesizer check: the Delivery-row counts and the 033 contents were confirmed; plan 019's table has rows I to VIII (lines 361-368) and no Delivery Constraints row. The other plans' principle rows were not individually checked.*

### 3. The cost model leaves out costs the design itself creates, so the cost the owner "accepted" is understated - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 7 "Cost model" (lines 592-600); Section 3 (lines 372-375); Section 4 (line 475); Alternatives C and D (lines 745, 753-754); Non-Functional Requirements (lines 880-882).
* **Discrepancy Description:** The arithmetic is right: 1+1+3+1+3 = 9 per pass; 15 for a converged review (two consecutive clean rounds in stages 7 and 9, `egm-review-loop/SKILL.md`); 33 at the five-round cap; 20 x 15 = 300, 20 x 33 = 660, 20 x 3 = 60. The model covers one converged pass only. (1) The approval rule says a final run follows the last fix and that a complete run uses all five stages (lines 372-375). If stage 5, 6 or 8 produces a fix after stage 7 has converged, a second complete pass is mandatory. Stage 5 raised six Majors on this very design, so the realistic range per doc is roughly 30 to 66 runs, or 600 to 1,320 in total (the reviewer's extrapolation, not a measured figure). (2) Line 475 says the amendment re-review "is part of the cost model (section 7)", but section 7 has no such figure. Task-text amendments are a documented practice (`docs/engineering/sdd-workflow.md:27`, `specs/AGENTS.md:7`), and under the diff rule each one turns an Approved doc into Draft and triggers a partial EGM review plus three repository reviewers. (3) The model counts runs, not size. Source text (spec, plan and tasks) runs from 1,281 lines for 019, the PR 3 measuring case, to 2,692 for 035, so PR 3 measures a cheap case.
* **Rationale & Recommendation:** Add the final-run clause and an amendment line to the model. Or relax the clause so that a fix found in stage 8 or 9 re-runs only the affected stage plus the repository reviewers. Re-state in Alternatives C and D what the owner accepts. *Synthesizer check: arithmetic, loop rules, the lines 372-375 clause, the missing figure at line 475 and the 019 and 035 line counts were confirmed.*

### 4. The findings footer has no grammar, no named author and a split across the PR boundary - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 3 "Findings footer" (lines 384-392); Step 1 (lines 929-930); Step 8 (lines 984-992); Step 22 (lines 1091-1097); Complexity Tracking (lines 704-713).
* **Discrepancy Description:** The footer format (`id, reviewer, severity, status`) appears only in prose. The `README.md` and `TEMPLATE.md` of PR 1 do not define it. EGM's reports come from a pinned third-party synthesizer and carry no footer. The three repository reviewers cannot write files, because `scripts/check-agent-config.py:27,144-145` forbids Write, Edit and Bash on reviewers, so the author (or the Elephant) appends every footer and saves every report. "Counts in the block equal the footers" then compares two things the author typed, which Risks row 1 already concedes. The PR 1/PR 2 swap removed the old forward reference but created a new one: PR 1 builds the parser (Step 1) while the producer contract lands in PR 2 (Step 22 and line 388). The footer and its three contract amendments are not in Complexity Tracking.
* **Rationale & Recommendation:** Define the grammar once, in PR 1. Say plainly that the author appends it. Drop count equality, or accept it as a consistency check only. *Synthesizer check: the write-tool ban and the absence of a grammar were confirmed.*

### 5. Two of the three repository reviewers are code reviewers, so the approval rule's security and correctness legs are largely vacuous on a design doc - Severity: Major
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 3 (lines 380-389); Step 22 (lines 1091-1097); Alternative M (lines 767-775); Complexity Tracking row (line 712).
* **Discrepancy Description:** `.claude/agents/security-reviewer.md:10` says "review the changed code for"; `.claude/agents/correctness-reviewer.md:10` says "then the diff" and its checklist asks for a failing scenario with concrete inputs. Only `architecture-reviewer.md:3` is written for "a design or diff". Step 22 edits line 10 of each and adds the footer; it does not tell the reviewers what to examine in a document. The severity map "a correctness finding with stated confidence maps to Major" (lines 387-389) turns every correctness finding, including "plausible", into a blocker that needs written owner acceptance. On a doc with no diff the correctness leg is likely empty or speculative, and the security leg is similar, yet both are counted in the 60 runs and in the approval rule.
* **Rationale & Recommendation:** The owner's decision (Alternative M) stands; this finding concerns how it is carried out. Add a design-review section to each agent definition, map "plausible" to Nit, or state plainly what each leg checks in a doc. *Synthesizer check: the lines quoted were confirmed in the agent files.*

### 6. The transitional clause promises the Spec Kit skills keep working, but PR 2 moves the file they read - Severity: Major
* **Reviewer Lens:** synthesizer spot-check (not raised by `egm-zoom-out`)
* **Affected Scope:** Section 9, "Transitional clause" (lines 637-639); Section 1 (line 304) and Step 11 (lines 1008-1016), which `git mv` the constitution in PR 2; Step 28 (lines 1170-1171); Q4 (lines 182-184).
* **Discrepancy Description:** The clause says that between PR 2 and PR 23 a feature whose conversion has not merged "may still use the `/speckit-*` skills". All ten `.claude/skills/speckit-*/SKILL.md` files name `.specify/memory/constitution.md` (15 occurrences). `speckit-plan` (line 62) reads it as part of "Load context", and `speckit-analyze` (lines 66 and 110) treats it as the non-negotiable authority. `speckit-tasks` (line 65) and `speckit-implement` (line 102) load it only "IF EXISTS". After PR 2 the file is gone from that path, so `speckit-plan` and `speckit-analyze` lose their constitution and the two "IF EXISTS" skills drop it silently. No step repoints those skills (they are generated; `CLAUDE.md` forbids hand edits), and Step 23 repoints only `verify` and `parallel-agents`. This is the only mechanism that makes the removal wait behind the 20 conversions (stage 5 Major 3), so the dependency that justifies the order does not hold.
* **Rationale & Recommendation:** Leave a pointer stub at `.specify/memory/constitution.md` until PR 23, or drop the clause and remove the tooling right after PR 2 (Q4), or say that unconverted features must not use the Spec Kit skills after PR 2. The owner should decide knowingly, because the answer changes whether the conversion wave is a precondition for removal at all. *Not verified by the reviewer; the synthesizer found it by search. The related stale sentence is finding 10.*

### 7. Complexity Tracking omits the largest moving parts, and its rejected alternative for the hash is a straw man - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Complexity Tracking (lines 704-713); Terms (lines 59-62); Section 6 (lines 525-527).
* **Discrepancy Description:** Terms defines the table as covering "each … added moving part", yet it does not list the conversion wave and `--conversion`, the reachability measures, the footer and its three contract amendments, the backticked-path check and its allowlist, the hook rewrite, `TEMPLATE.md` and `README.md`, or the append-only check. The hash row rejects "Record only the version (Alternative I)", but each accepted-list entry already records the plugin commit, and Claude Code stores the installed commit itself (`installed_plugins.json:46`). The real simpler alternative is acceptance by commit, with no `egm-tree-hash.py`. It is never weighed.
* **Rationale & Recommendation:** Add the missing rows, or define the table as covering only the items that violate a principle. Weigh "accept by commit" explicitly. One caveat the reviewer did not state: the installed-commit record is per user and outside the repository, so a repository-side check would still compare strings, and the design must say which side it relies on.

### 8. The evidence-block grammar is only partly taken from the real data - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 4 "Evidence block" (lines 455-460).
* **Discrepancy Description:** Line 457 says "These are the forms used today". Against `specs/*/tasks.md`: `*Done YYYY-MM-DD.*` matches 11 of the 13 ticked 018 tasks; two have a clause before the period (`specs/018-schedules-bulk-operations/tasks.md:300` and `:326`). `**Evidence (YYYY-MM-DD)**:` matches 20 of 32 ticked 037 tasks and sits on the checkbox line itself (`specs/037-compensation-safety/tasks.md:27-28`), which contradicts "after its checkbox line" (line 455) and requires an intra-line diff. `*Partly done YYYY-MM-DD.*` has no instance in any `tasks.md`; the phrase "partly done" appears only as header prose (`specs/019-worker-deployments-dispatch/tasks.md:7`, `specs/030-cross-namespace-services/tasks.md:4`, `specs/028-developer-tooling/tasks.md:6`). Features 014 (37 ticked) and 033 (6 ticked) use none of the three forms; 033 keeps its evidence in a paragraph after the list (`specs/033-redis-streams/tasks.md:19`). The reviewer counts the forms at 31 of the 88 ticked tasks in the four features that have ticks.
* **Rationale & Recommendation:** Label the markers as proposed. Allow a clause before the period. State the inline form explicitly. This matters only for ticks after approval, so existing evidence in 014 and 033 does not break the diff rule, but it affects the conversion comparison. *Synthesizer check: the "Partly done" absence, the header-prose lines, the 033 paragraph and the 14/18/37 marker counts were confirmed; the 11/13, 20/32 and 31/88 figures were not recomputed.*

### 9. The `Review-reports` field cannot be filled under preset 0 - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Status block (lines 412-416).
* **Discrepancy Description:** The block allows preset 0 and names only `consolidated_review_report.md`. `egm-review` writes that file only in preset 1 (`skills/egm-review/SKILL.md:149-152, 290-293`); preset 0 writes per-stage reports. This design's own review directory has no consolidated report.
* **Rationale & Recommendation:** Allow preset 1 only, or name the per-stage set.

### 10. The rollback sentence is stale after the PR 1/PR 2 swap - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 9 (lines 641-642).
* **Discrepancy Description:** "PR 1 and PR 2 can each be reverted alone" is no longer true. PR 2 now cites PR 1's check and hook, so reverting PR 1 alone strands the constitution.
* **Rationale & Recommendation:** Say that only PR 2 can be reverted alone before the first conversion merges.

### 11. Self-reference traps in the document's own text - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Example task (lines 462-467); Goal 4 (lines 133-138).
* **Discrepancy Description:** (1) The example task is a ticked task with `Owner-gate: docs/owner-decisions.md#2026-10-29/1` in a doc that the check scans. `docs/owner-decisions.md` has two sections dated 2026-10-29 (lines 33 and 174), and the first row there is the ChildRoot capacity row, which cites neither `T018-012` nor this doc, so the gate rule (lines 493-494) would fail the example. (2) The scoped search for `speckit` and `Spec Kit` covers `scripts/`, but the check script and its tests will contain those strings.
* **Rationale & Recommendation:** State that fenced blocks are skipped, and name the exclusion for the check's own files.

### 12. The amendment path adds machinery for a rare case - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 4 "Amending an approved doc" (lines 472-475).
* **Discrepancy Description:** "Formatting-only keeps `Approved`" needs a Markdown-aware word normalizer that nothing else in the design needs. The claim that `egm-review` accepts a range holds only through the ambiguous-boundary write-in (`SKILL.md:179-184`) and the loop's `SemanticTarget`. The text says the block records the range and the new reviewed commit, but the status block (lines 406-419) has no field for the range.
* **Rationale & Recommendation:** Delete the formatting clause; the plain rule is "any change outside a checkbox or an evidence block makes the doc `Draft`". Add the range field or stop saying the block records it.

### 13. The append-only `research.md` check is a new rule family that nothing in this change endangers - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Carrier table (line 653); Step 1 (line 932); Goal 6 (lines 143-144); the Section 3 check list (lines 423-439), which omits it.
* **Discrepancy Description:** `research.md` stays where it is, and no script reads it today (the reviewer searched `scripts/`, `.github/`, `tests/` and `crates/`). Register forms differ: 18 of the `research.md` files use `| Dnn |` rows and `specs/035-iceberg-write-steps/research.md` does not, so a parser needs several grammars.
* **Rationale & Recommendation:** Make the carrier a written rule plus the reviewers, or defer the check. At minimum list it in Section 3 so the rule-family count is stated once.

### 14. The 014 figure of 14,004 lines justifies the split machinery with the wrong population - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 7 "Splitting" (lines 573-578); table row (line 1158).
* **Discrepancy Description:** 14,004 lines in 42 files is correct, but only `spec.md` (701), `plan.md` (729) and `tasks.md` (295) are converted: 1,725 lines. The rest stays in `specs/`. The largest real sources are 035 (2,692) and 036 (1,543).
* **Rationale & Recommendation:** Defer the split machinery (parts, per-part blocks, per-feature coverage) to the PR 3 measurement, which already sets the ceiling. *Synthesizer check: the 014 and 035 sums were confirmed (1,725 and 2,692); the 036 figure was not re-counted.*

### 15. Alternative G's reasoning applies to files the design itself uses - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Alternative G (lines 779-785); Steps 18 and 25 and the PR 3-22 introduction (lines 1111-1117, 1133-1136).
* **Discrepancy Description:** A central ledger is rejected because "all 20 conversion pull requests … conflict on merge". Yet every conversion PR edits `docs/owner-decisions.md`, `docs/traceability.md` and `SPECIFICATION-INDEX.md`.
* **Rationale & Recommendation:** Say that the reason it works is that the PRs land one at a time.

### 16. The `Owner-gate` reference grammar does not match the existing register - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 5 (lines 491-498); Step 25 (lines 1111-1117).
* **Discrepancy Description:** The text says the script requires the row's "Refs column" to cite the task or the doc. `docs/owner-decisions.md` has no Refs column; its tables use `# | Item | Decision | Recorded in | Status` (lines 13, 35, 49 and others). Two sections share the date 2026-11-01 (lines 7 and 19) and two share 2026-10-29 (lines 33 and 174), and statuses are free text, so `#YYYY-MM-DD/<n>` is ambiguous.
* **Rationale & Recommendation:** Let Step 25 define the shape of new entries (heading, number, Refs column, status vocabulary) and let the check read only those. *Synthesizer check: the column headers were confirmed.*

### 17. The ordering sentence in Section 7 and the Terms are slightly off - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out`
* **Affected Scope:** Section 7 "Order" (lines 580-581); PR table (lines 1155-1160); Terms (lines 40-42); Non-Functional Requirements (lines 870-873).
* **Discrepancy Description:** "Features whose branches are open come last (section 9)", but Section 9 holds no order rule, and the table puts 033 and 037 at PR 17-18 ahead of 036 (0 ticked, with an open worktree) at PR 19; 037 also has a worktree. Terms lists six EGM skills, but the plugin ships seven (it also has `egm-bootstrap`), and the Non-Functional Requirements name the Bash- and Edit-capable agent of the skill that Terms omits.
* **Rationale & Recommendation:** Point the ordering sentence at the table, or state the rule. Add `egm-bootstrap` to Terms or say it is unused. *Synthesizer check: the seven skills were confirmed.*

## Verified Claims (confirmed by the reviewer, and re-checked by the synthesizer where marked)

* Counts: 35 feature directories, 20 with open tasks, 1,268 open and 678 ticked tasks. All 20 rows of the PR 3-22 table match the per-file counts and the open-task column sums to 1,268 (the synthesizer re-added it; the "about 1,190" of Alternative D is 966 + 94 + 129 = 1,189). The 15 features with no open task. 32 plans with a Constitution Check. 21 tracked `.specify` files (the synthesizer counted the 21 paths listed in Step 28) and 10 `speckit-*` skills (synthesizer confirmed). 14,004 lines in 42 files for 014. 34 open items in 036's security checklist. The 104 raw findings (12 + 23 + 69), 87 merged, and the nine lowered severities (`goldfish_review_report.md:9`).
* EGM plugin: nine agent types (synthesizer confirmed: seven lens adapters plus synthesizer plus bootstrap orchestrator), seven lens agents, the synthesizer and bootstrap tool lists, the brainstorm panel of six, presets 0 to 3, minimum Claude Code 2.1.271, manual-only skills, `/egm:<name>`, installed commit `f0ba151b` (synthesizer confirmed in `installed_plugins.json:46`), and the four-value Status at `design_doc.md:4`.
* Cost arithmetic: 9, 15, 33, 300, 660, 60, against the loop rules (synthesizer confirmed in `skills/egm-review-loop/SKILL.md`: "Converged: two consecutive clean rounds", "Cap: 5 rounds").
* PR numbering is consistent: PR 1 tooling, PR 2 constitution, PR 3 to PR 22 conversions, PR 23 removal, in Q4, Sections 3, 7, 8 and 9, the Step headings, the table and the Verification Plan. The hook is "PR 1" in Section 8 as well. Goals 5, 8, 9 and 10 agree with the body.
* Repository: the `check-agent-config.py` scope, the CI `architecture` job (`.github/workflows/ci.yml:165-176`) and the `contracts` job, the `session-start.py` reading of `.specify/feature.json`, the quickstart lines (014:23, 016:46), the README lines (677, 681, 721, 722), the PR template lines (3, 19, 25), line 10 of each reviewer agent, `.claude/settings.json:21` (`git fetch`), the three reviewers' severity vocabularies, no `CODEOWNERS` file, and the 3.0.0 Sync Impact section names.
* The owner decisions that contest findings 2 to 5 exist in `tasks/reviews/2026-11-02-egm-constitution/concepts_brief.md` (synthesizer confirmed the file and its "Decisions taken while drafting (2026-11-02)" heading). `docs/owner-decisions.md` has no entry for them yet: its only 2026-11-02 mention is the PR #75 row. Step 25 promises to record them in PR 2.

## Not Verifiable From the Repository

* The clone-versus-install file differences (lines 523-524). The sentence carries no "reported" label, unlike the tag claim.
* The upstream tag `v0.1.0` being 14 commits and 105 files behind.
* The 374 files that cite ids, the 2,036 register rows and the 24 files that cite rule numbers.
* The feature 016 duplicate-work history and the claim that 013 and 015 live on other branches.
* The worktree list (014, 016, 018, 035, 036, 037).
* That no repository merge-method setting exists.
* Claude Code documentation statements: no way to pin a version or commit in settings, and `--scope project` installation.
* That the "broader reviewed Constitution candidate" (Q1) does not exist anywhere.
* The estimate of 15 to 27 reviewer runs for a single converted feature: the lower bound of 15 follows from the skill text, but nothing in the repository measures the upper figure.

## Synthesis Notes

* The harness flagged the reviewer's output as instruction-shaped because it quotes `.claude/settings.json`. The content held findings only. It was treated as data, and nothing in it was acted on beyond recording the findings.
* One reviewer ran (instance A only). No second independent instance was launched, so recall is bounded by one pass.
* Spot-checks by the synthesizer before writing: file bounds (1,216 lines); cost arithmetic and loop rules; the nine plugin adapters and seven plugin skills; the Delivery Constraints and Complexity Tracking occurrence in all `plan.md` files and the 033 plan; the `Reviewed-blob` text at lines 411, 421, 427-430; the reviewer agent definitions and the write-tool ban in `check-agent-config.py`; the `owner-decisions.md` column headers and dated sections; the absence of "Partly done" in any `tasks.md`; the 019, 035 and 014 line counts; `installed_plugins.json:46`; preset 1 as the only writer of `consolidated_review_report.md`; and the ten Spec Kit skills' references to `.specify/memory/constitution.md` (finding 6). All held.
* The synthesizer did not independently recompute: the 11/13, 20/32 and 31/88 evidence-marker figures; the 18-of-N `research.md` register-form count; the principle rows of the plans other than 019; the 036 line count; the reviewer's "roughly 30 to 66 runs per doc" estimate, which is an extrapolation.
* The reviewer's disposition of stage 5 Majors 2 to 5 as "contested by a recorded decision" follows the document's own record. By the document's rule (lines 202-211) these remain unsettled until the owner accepts the contests in writing in `docs/owner-decisions.md`; that has not happened.
