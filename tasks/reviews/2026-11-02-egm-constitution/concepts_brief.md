# Replace Spec Kit with EGM and rebuild the constitution

* **Author:** Marcelo Ortiz de Santana (owner); working notes by Claude Code
* **Date:** 2026-11-02
* **Status:** Brainstorm in progress. Goldfish round 1 is done. Nothing in the repository has been deleted or changed yet.

## Overview

The owner wants the project to stop using Spec Kit and use only the Elephant-Goldfish Model (EGM). The
project constitution (`.specify/memory/constitution.md`, version 3.0.0, ratified 2026-10-11, last amended
2026-11-01) is to be rebuilt through EGM and moved to `docs/constitution.md`.

What EGM is: a design workflow. `egm-brainstorm` writes `concepts_brief.md`, `egm-draft` writes a design doc
(saved under `docs/designs/`), and `egm-review` has reviewers with no conversation history check it. It has no
constitution, no plan, no task list and no implement step. EGM is already used in this repository for
directory README contracts and for two review runs (`tasks/reviews/`).

Decisions the owner has taken (2026-11-02):

1. The owner prefers EGM. No defect of Spec Kit was named as the reason.
2. All eight principles of the constitution stay whole, together with the architectural rules.
3. Requirements live in EGM artifacts. Tasks become a section of the design doc's implementation plan.
4. Live features are converted into EGM design docs.
5. A design doc is approved automatically.
6. Success means keeping and evolving the project's architecture.
7. Removal covers the Spec Kit tooling only: `.specify/`, the ten `.claude/skills/speckit-*` skills and the Spec
   Kit passages in docs and hooks. Every `specs/NNN-*/` directory stays.
8. The new constitution is `docs/constitution.md`. The work is on branch `feature/egm-replaces-speckit` in its
   own pull request, with a version bump and a Sync Impact Report.

## Analysis

Six reviewers with no context (design, codebase-details, zoom-out, comprehension, implementation-readiness,
structural) read the concept and the repository. None failed. They found no threat to the architecture itself:
the principles are untouched, and `scripts/check-architecture.py` and CI enforce the layering. The risk is in
the process half. Claims below come from the reviewers unless marked "re-checked", which means the main context
read the source line itself.

### Blockers

**1. "Approved automatically" is not a control, and it collides with owner approval.** All six lenses raised it.
- EGM gives no verdict, and "blocking" is not defined. The review skill caps each stage at one re-audit, then
  records what is left as "accepted trade-offs" and advances (re-checked, `skills/egm-review/SKILL.md`, the
  "1-Audit Convergence Cap"). In the review-loop skill a finding counts as settled when the author agent
  contests it or records it without a fix.
- The one real EGM run in this repository (`tasks/reviews/2026-10-30-open-tasks/`) ended with 0 Blocker, 6 Major
  and every action open. Under "no open blocking finding" it would have been approved.
- In this repository "approved" already means that the owner approved the scope. Specs 019 to 032 are drafts
  pending owner review, and `docs/owner-decisions.md` says only 018 was approved. Feature 035 puts six owner gates
  (ADR-0071 and the O035 questions) ahead of any code. Converting and auto-approving would mark about 16
  unapproved drafts "Approved", and an implementation agent would act on it.
- Approvals are partial today (feature 037 was approved for T037-001..031 and T037-034, not for US04). One flag
  cannot say that.
- The reviewers' fix, not yet accepted by the owner: two states. "Review-clean" is automatic. "Approved for
  implementation" is an owner entry in `docs/owner-decisions.md`. A doc with an open owner question or gate cannot
  be approved. Record the document hash, the plugin version and the committed report path.

**2. The compliance control and the non-process parts of Governance have no EGM carrier.** The Governance section
of the constitution holds more than process: the supremacy clause, the ADR-divergence rule (cited by the `/adr`
skill, the ADR README and the constitution itself), the amendment and versioning rules, the Constitution Check
and Complexity Tracking. Thirty-two `plan.md` files carry a Check. Feature 035's Check is a live gate: principle
V is "Pass with conditions" and the technology-baseline row is "Requires ADR". EGM's seven review lenses do not
cover constitution compliance, security or correctness, and reviewers read the constitution only if the design
doc links it.

**3. Task state would live in two places, and ticking a task would edit an approved document.** EGM's design
template has no place for FR/SC/T ids, ticks or evidence (re-checked by the synthesizer in
`skills/egm-draft/references/design_doc.md`). Its implementation plan is a per-file list marked "no code", and it
tells the author to remove answered questions, which contradicts the append-only D/G/O registers. After
conversion, `specs/035-*/tasks.md` and the design doc would both carry `T035-NNN`. A sibling branch that ticks the
old file merges cleanly, and no check notices.

### Majors

4. **`specs/` is not history.** CI reads `specs/001-platform-foundation/contracts/` (re-checked: `ci.yml` lines
   320-340 lint `openapi.yaml`, `worker.proto`, `events.asyncapi.yaml`, `workflow.schema.json`). Tests read files
   of 011, 012, 014, 016 and 017, and one test rewrites a 014 contract file when an environment variable is set.
   `.cargo/audit.toml` requires each advisory ignore to be justified in `specs/001-platform-foundation/research.md`
   (re-checked), and several ignores expire in the next days. Freeze by file, not by directory.
5. **The conversion is large and mostly speculative.** About 1,190 of the 1,268 open boxes sit in drafts or
   not-started specs (035, 036, 019-032). Only 018 is being built; 014, 037 and 033 are closing out. The 035
   markdown files are 4,025 lines in 13 files, and the 014 markdown files are 14,004 lines in 42 files (the
   reviewers wrote 5,042 for 035, which counts its JSON schemas, and 14,004 for 014, which does not; the main
   context re-counted both with markdown only). The design lens estimates 15 to 27 reviewer runs per feature
   (an estimate, not a measurement). EGM's own FAQ says to use it for "what is important now".
6. **Silent breakage when `.specify` and the constitution path go.** `check-agent-config.py` checks relative
   markdown links and contiguous rule numbers only (re-checked), so backticked paths in `AGENTS.md`, `CLAUDE.md`,
   `README.md`, `SPECIFICATION-INDEX.md` and the reviewer agents stay green while broken. The session-start hook
   reads `.specify/feature.json` and swallows its absence, so the duplicate-work warning (added after feature 016
   was built twice) stops. The list of dependents is longer than 12: the PR template, `correctness-reviewer`, the
   skills `/verify`, `/parallel-agents`, `/adr` and `/migration`, `specs/AGENTS.md`, `specs/README.md`, the 014 and
   016 quickstarts (they run a `.specify` script), `docs/traceability.md` and `tests/Cargo.toml`.
   `.specify/memory/sync-impact-rust-sdk-2026-10-27.md` is the only record of the 1.0.0 to 1.1.0 amendment.
7. **EGM is not pinned or declared in the repository.** `.claude/settings.json` has no plugin entry. EGM is a
   per-user plugin at version 0.1.0, CI cannot run `egm-review`, and `docs/engineering/ai-engineering.md` says
   plugins are supply chain and should be pinned. Spec Kit was pinned.
8. **Version bump, Sync Impact Report and sequencing.** The constitution's rule gives MAJOR for a principle
   removed or redefined and MINOR for a section added or expanded. Replacing two sections fits neither. The 2.0.0
   and 3.0.0 reports treated relaxed or removed obligations as MAJOR, so 4.0.0 is likely. Amendments must update
   AGENTS.md and dependents in the same change, so a constitution-only PR is not allowed. The relative link at
   the end of the constitution breaks on the move. The 3.0.0 report leaves a "broader reviewed Constitution
   candidate" waiting to be rebased, and no reviewer found where it lives.
9. **The EGM template and standards do not fit.** EGM's readiness standard has a "Zero-Decision Rule", while 035
   has a deliberate spike and owner gates. Its zoom-out standard bans code-level detail that 45 FR rows need. The
   review splits the doc at "Detailed Implementation Plan", so tasks fall outside the range where their FRs sit.
   Contracts, data model, quickstart, checklists and evidence have no EGM home. Three gates exist only inside the
   Spec Kit skills (analyze, the checklist gate, converge). Feature 036's security checklist has 34 open items, 16
   release-blocking.
10. **No id allocator, no day-one answer, and "approved" means five things** (ADR accepted, owner approved for
    implementation, "approved dependency", the template's `Status: Approved`, review-clean). EGM writes
    `./design_draft.md` into the working directory, which collides across sibling worktrees.
11. **Without new scripts the rules fall back to prose.** Today no script reads `tasks.md`, so the evidence rule
    and the Check are already prose. The conversion should not keep them that way. A `check-design-docs.py` would
    check required headings, id prefix and uniqueness, requirement-to-task coverage, and a forward-only rule that a
    newly ticked line needs a date and an `Evidence:` line.
12. **Security exceptions and recommendations.** "Only the owner decides product, scope or release" is a bullet in
    "Start here", not a numbered rule, so a rewrite can lose it. "Security-exception" is new wording. EGM's draft
    skill forbids recommended options, while the repository asks the owner once, with a recommendation.

Nits: evidence manifests in 014 and 037 record a hash of the constitution at a base commit, so that commit must
stay reachable. Conversion would copy stale facts (035's plan says "five SDKs"; the constitution says six).
`tasks/todo.md` and `tasks/plan.md` are another planning family the concept does not mention.

### The owner's assumptions, tested

| Assumption | Verdict | Reason |
|---|---|---|
| (a) Approved = review ends with no open blocking finding, recorded in the repo | Fails as worded | EGM has no verdict field, "blocking" is undefined, the cap and "settled" close findings, reports are not committed by default. Works only as "review-clean". |
| (b) Ids and the evidence rule keep working in the plan section | Partly holds | Ids survive as text. Nothing in EGM enforces them, and the template has no slot. |
| (c) "Live" = features with open work; completed stay frozen | Fails | 033 is missing from the list. Tests and policy write into completed features (001, 014). Features 013 and 015 live on other branches. |
| (d) "Only the owner decides product, scope or release" stays | Holds as text, weak in practice | It is a bullet, not a rule, and auto-approval conflicts with it. |

### Facts that differ from what the reviewers were told

The reviewers were given figures from the main context that were wrong, and they corrected them:

- This checkout has **35** feature directories, not 37. Features 013 and 015 live on other branches.
- Feature 035 has **0** ticked tasks on `develop`. On its own branch (`feature/035-iceberg-write-steps`, pushed)
  T035-002 and T035-003 are ticked; "3 done" was wrong.
- `.specify` has 21 tracked files and 22 on disk (`feature.json` is ignored).
- The 12 dependents outside `specs/` and `tasks/` were incomplete (finding 6).

### Strongest counter-argument

No Spec Kit defect was named, and the cost is large: 374 source files cite FR/SC/T ids, about 2,036 register
rows, 32 Checks and 1,268 open tasks. EGM already works here as an overlay (feature 037's planning review; the
2026-10-30 review that found a task contradicting an owner decision). EGM adds no ids, tasks or enforcement, so
the repository must rebuild every check that Spec Kit's layout half-supplied.

### What would make the migration unsafe or irreversible

- Marking unreviewed drafts "Approved" and then cutting code, migrations or ADR baselines against them.
- Deleting `.specify`, the skills or the hook before the replacement checks, hook and re-pointed reviewers exist.
- Moving or editing the `specs/001` contracts or the test inputs of 011, 014, 016 and 017.
- Two constitutions, or none, between pull requests.
- Issuing ids from two places. Commit footers and code comments cannot be rewritten.
- Squash-merging away the base commits that the evidence manifests cite.
- Renumbering AGENTS.md rules, which about 24 files cite.
- An agent rewriting 14,004 lines of 014 in one pass.

### Not verified

No reviewer could run commands, git, CI or EGM. Cross-branch ids, worktree state, whether history survives
`git mv`, the review cost figures, how reviewers behave on documents of 5,000 lines or more, whether `egm-resolve`
can rename ids, and where the pending constitution candidate lives are all unchecked.

## Goals vs. Non-Goals

* **In-Scope Goals:**
  - A new `docs/constitution.md` that keeps all eight principles and every architectural rule in substance, with
    version, Sync Impact Report and a mapping of each old unit to its new place.
  - Process sections (Development Workflow, Governance) rewritten for EGM, keeping the non-process Governance
    clauses (supremacy, ADR divergence, amendment, versioning, Constitution Check, Complexity Tracking).
  - Removal of the Spec Kit tooling after its replacements (hook, checks, pointers) exist.
  - Conversion of features into EGM design docs, with ids and the evidence rule kept.
  - New checks so the rules are enforced, not only written.
* **Out-of-Scope Non-Goals:**
  - Changing the substance of any principle.
  - Deleting or moving `specs/001-platform-foundation/contracts/` or any file a test or CI job reads.
  - Letting an agent decide product, scope, release or security-exception questions.
  - Running `egm-review` in CI (the harness list does not support it).
  - Renumbering AGENTS.md rules.

## Alternatives Weighed

- **Keep Spec Kit.** Rejected by the owner, who prefers EGM. The reviewers' counter-argument (no defect named,
  large cost) is recorded above.
- **Delete `specs/` and migrate everything to `docs/designs/`.** Rejected by the owner on 2026-11-02: it breaks
  the contract tests and loses the evidence of closed tasks.
- **A. Overlay.** EGM for new features and for drafts the owner approves; `tasks.md` stays canonical for work in
  flight; the constitution changes narrowly. Fails when the owner wants one format across the repository soon.
  Not chosen; the owner chose to convert.
- **B. Convert on a trigger.** Each feature converts when the owner approves it or work starts. 014, 018, 033 and
  037 finish where they are. The new checks land first. Fails when two formats coexist for a long time and the
  index drifts. Not chosen: the owner chose C on 2026-11-02.
- **C. Convert everything in one wave.** About 20 features in per-feature pull requests after the guards exist.
  Fails when reviewers cannot cover the large docs, the owner becomes the bottleneck and sibling branches diverge
  on tick state and ids. Chosen by the owner on 2026-11-02.
- **Approval variants.** Two states (review-clean automatic, approved-for-implementation by the owner) and
  automatic approval locked while an owner gate is open were both offered and not chosen. Pure automatic approval
  was chosen (see Key Decisions).
- **Process rules inside the constitution** (owner's wording) versus **in `docs/engineering/sdd-workflow.md`**
  (the structural lens: process changes with the tool, and that file is already link-checked). Decided by the
  owner on 2026-11-02, while drafting: the whole procedure goes in the constitution.

## Key Decisions & Staging for Next Steps

Decided in round 1:

- **Approval is automatic and has one state** (owner, 2026-11-02, answer "Automática pura"). A design doc is
  approved when its `egm-review` ends clean, with no exception. All six reviewers advised a second, owner-entered
  state ("approved for implementation") and the owner chose against it after seeing their objection. Consequences
  the owner accepted: a converted draft such as 019-032, 035 or 036 shows as "Approved" without an owner decision
  on its scope, and an agent could read that status as permission to implement. The owner also confirmed, in the
  anchored hypothesis, that product, scope, release and security-exception decisions stay with the owner. How
  that rule is worded next to automatic approval is a drafting point for the workflow section of the new
  constitution; the owner reviews it there.

- **"Blocking" means Blocker or Major, and only the owner accepts a finding** (owner, 2026-11-02, answer "Blocker
  e Major, só você aceita"). A design doc is approved when `egm-review` ends with no Blocker or Major finding
  open. A finding that the author contests, or that is recorded without a fix, counts as settled only when the
  owner accepts it in writing. Any escalation (ESCALATE) or any reviewer that did not run keeps the doc not
  approved. Minor findings and below are recorded in the report and do not block. This closes the two loopholes
  the reviewers found: the one re-audit cap that turns leftovers into "accepted trade-offs", and settlement by the
  author agent.

- **All 20 features with open tasks are converted in one wave** (owner, 2026-11-02, answer "Todas as 20 de uma
  vez"): per-feature pull requests, after the new checks exist. This is the option the reviewers rated as failing
  when reviewers cannot cover large documents, the owner becomes the bottleneck and sibling branches diverge on
  tick state and ids. The cost estimate they gave (15 to 27 reviewer runs per feature, so roughly 300 to 540) is
  not measured. Mitigations to carry into the plan: move tasks instead of copying them and mark the old file as
  superseded with a check that fails if its body changes; split large documents by increment (014 is 14,004
  lines); start with a feature that has not started; keep `contracts/`, `evidence/`, `validation-report.md` and
  the research registers live in `specs/NNN-*/` (the owner already kept `specs/`).

Open decisions, in the order they block the next step:

2. Decided: see "Decided in round 1".
3. Decided: see "Decided in round 1".
4. Version bump: MAJOR (4.0.0) or MINOR?
5. Do the process rules live in the constitution or in `docs/engineering`?
6. How is EGM pinned, and which harnesses may run the review?
7. Where do security exceptions live, including the `.cargo/audit.toml` ignores?
8. What happens to the pending "broader reviewed Constitution candidate"?
9. Which rule wins on recommendations: EGM's "never recommend" or the repository's "ask with a recommendation"?

Proposed defaults for decisions 4 to 9. These are the main context's reading of the reviewers' findings, not
owner decisions. The owner confirms or changes each one when `/egm-draft` reaches the section it affects.

- **4. Version: 4.0.0 (MAJOR).** The 2.0.0 and 3.0.0 reports treated relaxed or removed obligations as MAJOR, and
  this change replaces two sections and moves the file. The owner may choose MINOR.
- **5. Where the process lives.** The constitution keeps the obligations (a design doc is reviewed and approved
  under the blocking rule; the Constitution Check and Complexity Tracking; ids and "ticked only with evidence";
  owner decisions; supremacy; ADR divergence; amendments and versioning). The step-by-step EGM procedure lives
  in `docs/engineering/sdd-workflow.md`, which is already link-checked and changes with the tool. This differs
  from the owner's wording ("process sections replaced by brainstorm, draft and egm-review") only in where the
  detail sits.
- **6. EGM pinning.** Each review record carries the plugin version (0.1.0 today) and the persona hashes.
  AGENTS.md states which steps need a harness that can spawn subagents. CI does not run `egm-review`.
- **7. Security exceptions.** They are owner decisions recorded in `docs/owner-decisions.md`. The live register
  for advisory ignores stays in `specs/001-platform-foundation/research.md`, which is not frozen.
- **8. The "broader reviewed Constitution candidate".** The 3.0.0 report says it is not adopted and must be
  rebased. No file or branch with that name was found, so its location is unknown. Default: out of scope for this
  rewrite, listed as an open item; its "plugin rules" are the part that overlaps with decision 6.
- **9. Recommendations.** The repository practice (ask the owner once, with a recommendation) applies to owner
  questions. EGM's ban on recommended options applies to the ideation and drafting interaction only.

Landing order proposed by the readiness lens (any option):

1. Tag `develop` as the last Spec Kit state.
2. PR 1: the constitution as a pure `git mv`, then an edit commit, with the Sync Impact Report, every pointer fix,
   the AGENTS.md rule 1 and 2 rewrite and a one-line stub at the old path.
3. PR 2: the branch-based hook, the new checks, the id allocator and the file-level freeze rules.
4. One feature conversion per PR, starting with a feature that has not started.
5. Delete `.specify` and the speckit skills after sibling branches have rebased.

## Decisions taken while drafting (2026-11-02)

These were taken by the owner in single-choice gates of `/egm-draft`, after the brief above was closed. Where they
differ from the brief's proposed defaults, the design draft follows them.

1. **Approval record**: a status block in the design doc itself (not a central ledger, not a file per doc).
2. **Ticks after approval**: a diff against the reviewed commit accepts only checkbox and evidence-line changes.
   Pull requests that touch design docs are merged with a merge commit, and CI checks out the full history.
3. **Where the procedure lives**: the whole EGM procedure is in the constitution (this replaces default 5).
4. **EGM pin**: `.claude/settings.json` registers the marketplace and enables the plugin; a list of accepted hashes
   of the plugin files is kept in the repository, and a review record whose hash is not on the list fails the check
   (this replaces default 6). Claude Code documents no way to require a version or a commit in project settings, and
   the upstream tag `v0.1.0` is 14 commits behind the commit that ran the reviews.
5. **Owner decisions**: a gate on the task, written as `Owner-gate:` and checked by the script, not a second approval
   state and not a "stop and ask" rule.
6. **Conversion method**: an agent maps `spec.md`, `plan.md` and `tasks.md` onto the design template and `egm-review`
   judges; the owner is not asked to approve sections.
7. **Repository reviewers**: `architecture-reviewer`, `security-reviewer` and `correctness-reviewer` are part of the
   approval rule, so only a Claude Code session can produce an `Approved` doc.
8. **Id allocator**: `scripts/next-feature-id.py` ships in PR 2, with a table of reserved ranges. The brief's landing
   order had it there; the first design draft had moved it to Future Improvements.
