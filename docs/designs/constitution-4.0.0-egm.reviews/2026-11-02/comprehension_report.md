## What this proposes

The design replaces Spec Kit with the Elephant-Goldfish Model (EGM) as the only way features are specified in this repository. The constitution moves from `.specify/memory/constitution.md` to `docs/constitution.md` as version 4.0.0. The eight principles stay as they are, and only the process parts (Development Workflow and Governance) are rewritten around EGM's brainstorm, draft and review stages.

A design doc counts as "Approved" automatically. This happens when `egm-review` and three repository reviewers (architecture, security, correctness) leave no Blocker or Major finding open. A status block in the doc records the approval, and a new script, `scripts/check-design-docs.py`, checks that record. The plan also adds:
- task-level "Owner-gate" citations, which block a task until the owner has recorded a decision;
- a pin on the EGM plugin, using a content hash and a list of accepted hashes;
- an id allocator.

The 20 features with open tasks are converted into design docs, one pull request each. `.specify/` and the ten `speckit-*` skills are deleted last.

## How the existing system works, per this document

Version 3.0.0 of the constitution lives in `.specify/memory/`. It states the eight principles and names Spec Kit's commands as the specification workflow. Each feature is a directory `specs/NNN-slug/` holding `spec.md`, `plan.md` and `tasks.md`, plus contracts, evidence and append-only registers (`research.md` with D, G and O rows).

`plan.md` carries a Constitution Check and a Complexity Tracking table. `tasks.md` carries ticks and evidence, with ids such as `FR-NNN-xxx`, `SC-NNN-xxx` and `TNNN-xxx`. CI lints the contract files. `.cargo/audit.toml` points into `research.md`. A session-start hook reads `.specify/feature.json` to warn about duplicate work. Several documents point at Spec Kit files, and only relative markdown links are checked. EGM is a per-user plugin with no approval verdict. Its review skill allows one re-audit and then records what remains as "accepted trade-offs".

I could paraphrase every section, so there are no Blockers. Two paraphrases needed a guess, and both are Major findings: the conversion check (finding 6) and the reviewed-commit diff (finding 4).

## Findings

**1. Severity: Major. EGM vocabulary is used as if known.**
Affected Scope: lines 17-19, 58, 101, 224-226, 242, 388, 424, 513, 586, 697.
- Lines 17-19 give the only definition of EGM, one sentence. The doc then relies on much more, none of it explained:
  - the six skills `egm-brainstorm`, `egm-draft`, `egm-review`, `egm-review-loop`, `egm-resolve` and `egm-drift-report`;
  - "EGM's seven lenses" (226, 388, 697);
  - "EGM's readiness standard/rule" (424, 513);
  - "ESCALATE" (242);
  - "stage" in the convergence cap (58);
  - the "harness list" that "does not include CI" (101, 586).
- The "seven lenses" claim is the entire justification for the three extra reviewers in the approval rule and for Alternative M. A reader cannot judge it without knowing what the lenses are.
- Define each in one line, or link the EGM doc that does (`docs/getting_started.md` is cited only at the bottom).

**2. Severity: Major. The evidence base for the Problem is described inconsistently.**
Affected Scope: lines 57-61, 120, 313, 326, 430, 455, 470, 575.
- "The one EGM review run in this repository" (59) and "the one real EGM run here" (430) conflict with "the earlier EGM runs in this repository" (120), "the reviews so far" (313) and "the reviews of this design" (326).
- The counts are "six reviewers" (470) and a "six-lens review" (575), against "seven lenses" (226).
- The doc never says what was reviewed, when, or by which lenses. The only pointer is a resource entry at line 574.
- The key claim "0 Blocker and 6 Major findings and every action still open" rests on that run. A new reader cannot tell which run it was or whether it ran on this design or on the concepts brief.

**3. Severity: Major. The format of a task, a requirement citation, an evidence line and an owner gate is never shown.**
Affected Scope: lines 138, 277-280, 289-290, 304-305, 417; CUJ 2 and CUJ 3.
- The check parses these, but no example task line exists. The reader cannot tell where the id, the checkbox, the requirement citation, `Owner-gate:` and the evidence line sit.
- The evidence line is spelled three ways: "a dated `Evidence:` line" (138), "a date and an `Evidence:` line" (280) and "a line that begins with `*Evidence`" (290).
- "A date and the words `partly done`" (290) and "partly done lines" (417) are never explained.
- "Inside a task" (290) is not defined either. Add one worked task block, and keep a single spelling.

**4. Severity: Major. The reviewed-commit diff and the finding counts are underspecified.**
Affected Scope: lines 242, 256-266, 273-274, 287-291.
- Section 4 diffs from `Reviewed-commit` to the working tree and allows changes only "in the body of the doc". "Body" is not defined. The status block is written after the review (268), so it is itself a change since the reviewed commit.
- It is unstated which commit is "reviewed" when there are several review rounds, with fixes between them.
- "Their counts match the block" (274) does not say how the script reads Blocker and Major counts out of four report files with different formats. It also does not say how it tells an open finding from a fixed one.
- The block says `Open findings: Blocker 0, Major 0` while also listing an owner-accepted finding (`REV-003`). It is unclear whether that finding counts as open.
- "REV-003" and "item N" are never defined: where finding ids come from, and how a finding is matched to an entry in `docs/owner-decisions.md`.
- The severity scale is implied ("Minor findings and below", 243), but it is never stated and may differ from the Blocker/Major/Nit set used elsewhere.

**5. Severity: Major. CUJ 3 promises more than Section 5 delivers.**
Affected Scope: lines 140-146 against 304-308.
- CUJ 3 says the check fails when an agent tries to "start or tick" a gated task before the owner entry exists.
- Section 5 says the task "cannot be started or ticked", but the check "enforces that for every ticked task that carries a gate". Nothing can detect a start.
- The format of an owner-decisions entry (the `#<entry>` anchor) is also not described.
- It is not said how the check knows the owner wrote the entry rather than an agent. The same question applies to "accepts it in writing" in the approval rule (242).

**6. Severity: Major. The conversion mechanism contradicts itself.**
Affected Scope: lines 86-88, 156-162, 348-360.
- Tasks are "moved, not copied" (349). The old `tasks.md` is bannered, and its body must not change after commit X (353-354).
- The script then compares the design doc's ids and counts with "the superseded `tasks.md` at X" (359-360).
- If tasks were moved out, `tasks.md` at X has none, so there is nothing to compare. If its body is unchanged, the tasks were copied, and the id-uniqueness rule has to exclude superseded files to avoid duplicates (276).
- My guess is that "moved" means "the design doc becomes the live copy" and the comparison is against the parent of X. The doc should say so, because this check is the whole basis of Goal 5, "the conversion loses no task".

**7. Severity: Major. Splitting a large feature "by increment" is undefined.**
Affected Scope: lines 355, 821, and Goal 5 (86).
- "Increment" is not defined.
- It is not said whether a split produces several files, how they are named under the `NNN-slug.md` scheme, or whether they stay in one pull request.
- It is not said how the id-uniqueness and conversion-fidelity checks handle one `tasks.md` mapped to several docs, or how the 15-27 reviewer runs apply to each.
- Feature 014 is the largest case and is explicitly in scope.

**8. Severity: Major. Section 8 and Section 9 disagree on what lands in PR 1.**
Affected Scope: lines 378-380, 384, 395; 402-406.
- Section 8 says pointers are fixed "in the same change as the constitution", and its list includes the session-start hook and `check-agent-config.py`.
- Section 9 says PR 1 contains "every pointer fix of section 8 except the deletions", and then puts the hook change and the extended `check-agent-config.py` in PR 2.
- The Detailed Plan agrees with Section 9 (items 23 and 24).
- Between PR 1 and PR 2 the constitution also names a CI gate that does not exist yet (line 200, "CI gates gains the new design-doc check"). The doc does not say whether that interim state is acceptable.

**9. Severity: Major. Goal 3 ("every rule has a named carrier") has no visible mapping, and several carriers are missing.**
Affected Scope: lines 78-80, 36-55, 44-45, 103, 231, 351-352.
- The Problem lists six items. The design never says, item by item, "script" or "written rule" and why, which is what Goal 3 promises.
- For "Append-only registers" the only statement is that they stay and are editable (Goal 6). No check or rule is named that keeps them append-only (351-352 says "must stay append-only").
- Line 231 says `egm-drift-report` replaces `/speckit-converge` "in part only, and the design says so". The design never says which part is not replaced, and never says what converge, clarify, checklist or taskstoissues did.
- Only analyze (277-278) and the task generator (103) are addressed. A reader cannot tell whether a capability is being dropped silently, in a document whose thesis is that nothing is lost silently.

**10. Severity: Major. The allocator, the pin and the hook have no motivating Problem or Goal.**
Affected Scope: Overview items 1-5 (22-27), Goals 1-7, lines 221-223, 404-406, 543-551, 775-784.
- `scripts/next-feature-id.py` is a PR 2 deliverable with its own alternative (N) and a detailed plan item. The Problem and Goals never state the numbering-collision problem.
- Alternative N is the only place the "two branches took the same number" incident appears.
- "Table of reserved ranges" and "reserved ranges for D, G, O and ADR numbers" are never explained: what is reserved, by whom, and why. The only clue is a parenthetical at line 781.
- The EGM pin (Section 6) and the owner gate (Section 5) are likewise absent from the Overview's five-item list and from the Goals.

**11. Severity: Major. The "no speckit outside historical records" rule is inconsistent with itself.**
Affected Scope: lines 83-85, 397, 852-855, 866.
- Goal 4 exempts whole directories (`specs/`, `tasks/reviews/`, `docs/adr/`, `docs/governance/`) plus `docs/owner-decisions.md`.
- The Detailed Plan exempts specific files (`ADR-0071-*.md`, `tasks/reviews/2026-10-30-*`, specific mentions in `owner-decisions.md`).
- `docs/designs/` is on neither list. This design doc contains `speckit-*`, `/speckit-analyze` and `speckit-last-state` many times (for example lines 14, 600, 840-846). `docs/designs/README.md` and the script that performs the check will too.
- It is unclear whether the check would fail on its own design doc, and which of the two exemption lists is authoritative.

**12. Severity: Major. "Owner gate" means three different things.**
Affected Scope: lines 250, 304-305, 521, 637.
- The EGM template is said to have no "owner gates" (250). The per-task `Owner-gate:` citation is another meaning (304). The "Owner gates" section the new template adds (637) and the "section gates" of interactive drafting (521) are further uses. Alternative K's "more than 80 owner gates" is probably the fourth.
- A reader cannot tell whether the template's "Owner gates" section is the same thing as the per-task citation. Give them distinct names.

**13. Severity: Major. The purpose of the Step 0 tag and the "reachable commit" argument is not explained.**
Affected Scope: lines 295-296, 600-601, 855-856.
- "Evidence manifests of features 014 and 037, which record a hash at a base commit" are never defined. The doc never says what they hash, only (at 855) "the constitution".
- The claim is that the tag "keeps that commit reachable". A commit on `develop` stays reachable under the merge-commit rule anyway, and the doc does not explain what would make it unreachable.
- It is also not explained why moving the constitution file affects a recorded hash. The tag's real purpose is unclear.

**14. Severity: Major. It is unclear whether this design satisfies its own required structure.**
Affected Scope: lines 249-255, 270-271; the document as a whole.
- Section 3 requires every design doc to have a Constitution Check (a row for each of principles I-VIII and the Delivery Constraints) and a Complexity Tracking section. It says a doc without a leading number is "cross-cutting" and "carries no FR/SC/T ids".
- It does not say whether cross-cutting docs are also exempt from the Check, the Complexity Tracking section, Requirements and the status block.
- This doc has none of those sections. A reader cannot tell whether it is exempt, or whether it would fail its own check once finished.

**15. Severity: Major. The sentence about the `Skill` tool cannot be understood.**
Affected Scope: lines 655-656; CUJ 1 (127).
- "EGM skills (which the `Skill` tool refuses, so they are read and followed in the same thread)". The reader is not told what is refused, who refuses it, or what "the same thread" excludes.
- CUJ 1 says the author "runs `egm-brainstorm`, then `egm-draft`". How these skills are actually run is therefore ambiguous.

**16. Severity: Major. The verdict vocabulary is inconsistent and undefined.**
Affected Scope: lines 69-72, 189-190, 617; table at 191-207.
- Goal 1 lists the verdicts "KEEP, MOVE, REWRITE or DROP". Section 1 says the format is the 3.0.0 report "plus the verdicts MOVE and DROP". The table and the Detailed Plan use ADD, which neither of those sentences lists.
- No row in the table is a MOVE, even though the constitution file itself moves.
- None of the verdicts is defined, for example REWRITE versus MOVE. The check in Goal 1 ("each REWRITE or DROP says where its obligation went") depends on them.

**17. Severity: Nit. "Spec Kit" is never described, and the id prefixes are not expanded.**
Affected Scope: lines 13-15, 41-42, 277.
- Spec Kit is described only by the commands it provides. The reader has to infer what `spec.md`, `plan.md` and `tasks.md` are.
- FR and SC are never expanded (probably functional requirement and success criterion), yet the coverage check depends on them.
- "Sagat Octopus Orchestrator" is also never described. A single sentence at the top would anchor the reader.

**18. Severity: Nit. Several named constructs are used before or without definition.**
Affected Scope: lines 37-40, 207, 270-271, 273, 326, 552 (Overview, Problem, Goal 2).
- "Complexity Tracking" and "Delivery Constraints" are never defined, and "technology-baseline row" (40) is not explained. "Constitution Check" is explained only by example.
- "Sync Impact Report" is defined only through the file it lives in.
- "EGM hash" is used in CUJ 1 and Goal 2 before Section 6 defines it.
- Line 325-326 says the first entry in the accepted list "is the commit that ran the reviews", but the list holds hashes, and Plan item 21 says each entry holds a hash plus a commit. Hash and commit are conflated.

**19. Severity: Nit. Claude Code plugin terms are used without inline definition.**
Affected Scope: lines 312-331, 503-507, 756-760.
- `extraKnownMarketplaces`, `enabledPlugins`, `egm@egm`, `ref`, `sha`, `autoUpdate` and `--scope project` appear without a definition. The links at 578-580 are in "Additional Resources" only.
- "Marketplace clone" versus "install directory" is explained, which helps. Line 333 ("The reviewers listed the 'plugin rules' of the unadopted 3.0.0 candidate as related. See Q1") names rules whose content is nowhere given, so the sentence does not stand alone.

**20. Severity: Nit. Provenance and units of measure are inconsistent.**
Affected Scope: lines 34-35, 102, 372-374, 440-441, 521, 564-565.
- Only some figures are tagged "(reported)". Others have no source: 374 source files, 2,036 register rows, 24 files citing rule numbers, 14,004 lines in 42 files.
- "The main context" (35) is EGM-internal jargon.
- "Reviewer run" is the cost unit for 300-540 and 60, and it is never defined. The 15-27 EGM runs presumably include re-audits, while the repository reviewers are counted as exactly three runs per doc.
- "More than 80 owner gates" for 4 sections times 20 features is exactly 80.

**21. Severity: Nit. "Plan" is overloaded, and some phrases do not read in a design doc.**
Affected Scope: lines 199, 287, 348, 566, 873-874.
- "The plan" means `plan.md`, the Technical Plan, the Detailed Implementation Plan, "the design doc's plan" and (566, 875) "the plan is revisited". Name which one each time.
- "Not my conclusions" (873) uses a first-person voice with no named speaker.
- "The tool only" (716) is a fragment that needs a gloss. It refers to the Alternative B decision to remove the tool but keep `specs/`.

**22. Severity: Nit. The architecture check's relation to this change is unexplained.**
Affected Scope: lines 91-93, 529.
- Goal 7 says `check-architecture.py` and its negative tests "pass without edits to their rules", but never says what that script checks or why Spec Kit removal could affect it.
- The goal also says "no principle text loses an obligation", which is not scriptable. It overlaps Goal 1 and is better stated there.
- "Fitness functions" (529) is general engineering vocabulary, but its tie to `AGENTS.md` rule 21 is left implicit.

**23. Severity: Nit. Minor ordering and location gaps.**
Affected Scope: lines 369-370, 818-825, 218, 574.
- Conversion order is meant to put "features that nobody is changing first and the ones with open branches last" (825). PR 17 (036) has an open worktree yet precedes PR 18 and PR 19 (033 and 037), which are "closing out".
- Step 1 says `egm-brainstorm` writes `concepts_brief.md`, but not where it is saved or whether it is committed. The only example path is `tasks/reviews/2026-11-02-egm-constitution/concepts_brief.md` (574).
- The sentence at line 559-561 about "two of this design's own review outputs" being flagged as instruction-shaped is an anecdote. It does not state the resulting requirement or tell the reader what happened to the review.
