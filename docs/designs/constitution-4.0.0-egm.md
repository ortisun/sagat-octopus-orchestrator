# Constitution 4.0.0: EGM as the only specification workflow

*   **Author:** Marcelo Ortiz de Santana (owner)
*   **Status:** Draft
*   **Date:** 2026-11-02

--------------------------------------------------------------------------------

## Overview

The constitution of Sagat Octopus Orchestrator (version 3.0.0, ratified 2026-10-11, last amended 2026-11-01)
does two jobs. It states eight principles that every feature must meet (durable state, modular domain, at-least-once
effects, pinned compatibility, isolation, observability, redeliverable events, honest tests). It also names Spec
Kit as the way features are specified: the Development Workflow section lists the commands from `/speckit-specify`
to `/speckit-converge`, and the Governance section requires a Constitution Check in every `plan.md`.

The owner has decided to stop using Spec Kit and to use only the Elephant-Goldfish Model (EGM). In EGM, a design
doc is drafted section by section with the owner and then reviewed by reviewers that have no conversation history.

This design replaces Spec Kit with EGM across the repository:

1.  The constitution moves to `docs/constitution.md` as version 4.0.0. All eight principles stay in force. Only the
    process parts of Development Workflow and Governance are rewritten.
2.  A rule states when a design doc counts as approved.
3.  The Spec Kit tooling is removed, after its replacements exist.
4.  The 20 features that have open tasks are converted into EGM design docs.
5.  New checks enforce the rules that the Spec Kit file layout supported without anyone writing them down.

## The Problem

Deleting Spec Kit is easy. The risk is in what the deletion takes with it. The constitution and the repository rely
on Spec Kit's file layout for several rules, and EGM supplies none of them.

What the repository relies on today (sources are named; "reported" means a reviewer found it and the main context
did not re-read the source):

*   **A compliance gate.** Governance requires each `plan.md` to carry a Constitution Check and a Complexity
    Tracking table (`.specify/memory/constitution.md`, Governance). Reviewers counted 32 plans with a Check
    (reported). Feature 035's Check is a live gate: principle V reads "Pass with conditions" and the
    technology-baseline row reads "Requires ADR".
*   **Task state with evidence.** `AGENTS.md` says a task is ticked only with the evidence that closes it. The
    ids `FR-NNN-xxx`, `SC-NNN-xxx` and `TNNN-xxx` are feature-scoped and are cited in commit footers. EGM's design
    template has no place for ids, ticks or evidence.
*   **Append-only registers.** `research.md` holds decisions (`D`), gaps (`G`) and owner questions (`O`) that are
    never renumbered. The EGM template tells the author to remove answered questions.
*   **Contracts that tests and CI read.** CI lints `specs/001-platform-foundation/contracts/openapi.yaml`,
    `worker.proto`, `events.asyncapi.yaml` and `workflow.schema.json` (`.github/workflows/ci.yml`, lines 320-340).
    `.cargo/audit.toml` requires each advisory ignore to be justified in
    `specs/001-platform-foundation/research.md`.
*   **Pointers that nothing checks.** `scripts/check-agent-config.py` checks relative markdown links and that the
    `AGENTS.md` rule numbers run without gaps. It does not check backticked paths, so `AGENTS.md`, `CLAUDE.md`,
    `README.md` and the reviewer agents can point at a deleted file and the check still passes.
*   **A duplicate-work warning.** The session-start hook reads `.specify/feature.json` and warns when other
    branches mention the same feature. It was added after feature 016 was built twice (reported; the hook was not
    re-read).

EGM also has no verdict. Its review skill allows one re-audit per stage, then records the remaining findings as
"accepted trade-offs" and moves on (`skills/egm-review/SKILL.md`, the "1-Audit Convergence Cap"). The one EGM review
run in this repository ended with 0 Blocker and 6 Major findings and every action still open. A rule that says
"approved when the review is clean" is only as strict as its definition of clean.

The scale of the change: the repository has 35 feature directories under `specs/`. Twenty have open tasks, with
1,268 open boxes; 678 boxes are ticked overall.

## Goals

Each goal can be checked.

1.  **The constitution keeps every obligation.** `docs/constitution.md` exists as version 4.0.0. The Sync Impact
    Report lists every unit of version 3.0.0 as KEEP, MOVE, REWRITE or DROP. All eight principles are KEEP. A REWRITE
    or DROP may appear only in the process parts of Development Workflow and Governance, and each one says where its
    obligation went.
2.  **Approval is one rule, and a script checks its record.** A design doc is approved when `egm-review` and the three
    repository reviewers (architecture, security, correctness) each end with no Blocker or Major finding open. A
    finding that is contested or left unfixed counts as settled only when the owner accepts it in writing. An
    escalation, or a reviewer that did not run, keeps the doc not approved. The review record states the reviewed
    commit, the EGM hash and the report paths.
3.  **Every rule the layout supplied has a named carrier.** For each item in "The Problem", the design says whether
    a script checks it or a written rule covers it, and why. The Constitution Check and Complexity Tracking become
    required sections of a design doc.
4.  **Removal leaves no dangling pointer.** The `.specify/` directory and the ten `speckit-*` skills are deleted
    only after the hook, the checks and the re-pointed documents are in place. After deletion, `check-agent-config.py`
    and a new backticked-path check pass, and a search for `speckit` finds nothing outside the historical records:
    `specs/`, `tasks/reviews/`, `docs/adr/`, `docs/governance/` and `docs/owner-decisions.md`, which cite
    `/speckit-clarify` and `/speckit-plan` as dated facts.
5.  **The conversion loses no task.** Each of the 20 features is converted in its own pull request. The converted
    doc keeps every task with its original id, and a script confirms that the counts of ticked and open tasks equal
    those of the source. The old `tasks.md` is marked superseded, and a check fails if its body changes afterwards.
6.  **Nothing that tests or CI read moves.** The contract files, the evidence directories and the research
    registers stay where they are and stay editable.
7.  **The architecture check still passes unchanged.** `scripts/check-architecture.py` and its negative tests pass
    without edits to their rules, and no principle text loses an obligation. This is the owner's measure of success:
    keep and evolve the architecture.

## Non-Goals

*   Changing the substance of any of the eight principles.
*   Deleting or moving `specs/001-platform-foundation/contracts/` or any file that a test or CI job reads.
*   Converting the 15 features that have no open task. They stay as history.
*   Letting an agent decide product, scope, release or security-exception questions. These stay with the owner.
*   Running `egm-review` in CI. The EGM harnesses do not include CI.
*   Renumbering the rules of `AGENTS.md`. About 24 files cite them by number (reported).
*   Building a replacement for Spec Kit's task generator. A design doc's tasks are written by the author.
*   Changing any runtime code, schema, contract or ADR of the platform.

## Open Questions

*   **Q1: Where is the "broader reviewed Constitution candidate"?** The 3.0.0 Sync Impact Report says it was not
    adopted and must be rebased before later use. No file or branch with that name was found. This design leaves it
    out and does not change it. The owner says where it lives or drops it. Its "plugin rules" overlap with section 6
    of the Technical Plan.
*   **Q2: Do security exceptions need a register of their own?** This design uses the homes that exist today:
    entries in `docs/owner-decisions.md`, and the gap register in `specs/001-platform-foundation/research.md` that
    `.cargo/audit.toml` already points to. The owner confirms that no new register is wanted.
*   **Q3: Recommendations.** Outside an EGM session, the repository practice stays: ask the owner once, with the
    options and a recommendation (`docs/engineering/sdd-workflow.md`, item 4). Inside an EGM brainstorm or draft,
    the skill forbids recommendations and defaults, and that rule applies there. The owner confirms this split.
*   **Q4: Where do review reports live?** This design puts them next to the doc, in
    `docs/designs/NNN-slug.reviews/YYYY-MM-DD/`, because the approval record points at them. The earlier EGM runs
    in this repository used `tasks/reviews/`. The owner may prefer one place for both.

## Critical User Journeys (CUJs)

### CUJ 1: Design a new feature

*   **Given**: An idea for a feature that does not exist yet, on a branch `feature/NNN-slug` cut from `develop`.
*   **When**: The author runs `egm-brainstorm`, then `egm-draft`, saves the draft as `docs/designs/NNN-slug.md`, and
    runs `egm-review` and the architecture, security and correctness reviewers until none has a Blocker or Major
    finding open.
*   **Then**: The status block says `Approved` and holds the reviewed commit, the EGM hash, the report paths and the
    finding counts. `scripts/check-design-docs.py` passes. No person pressed an approve button.

### CUJ 2: Tick a task after approval

*   **Given**: An approved design doc with an unticked task `T018-012`.
*   **When**: The author ticks the box and adds a dated `Evidence:` line with the test name and its result.
*   **Then**: The check passes, because only a checkbox and an evidence line changed since the reviewed commit. If the
    author also edits a sentence of the design, the check fails until the doc is reviewed again.

### CUJ 3: A task that needs an owner decision

*   **Given**: A task that depends on a product, scope, release or security-exception decision and cites the entry
    it needs.
*   **When**: An agent tries to start or tick the task before the owner has recorded that entry in
    `docs/owner-decisions.md`.
*   **Then**: The check fails. After the owner records the entry, the same change passes.

### CUJ 4: Upgrade the EGM plugin

*   **Given**: A list of accepted EGM hashes in the repository and a new EGM release installed on a developer
    machine.
*   **When**: A review is run with the new release and its record is committed.
*   **Then**: The check fails because the recorded hash is not in the list. A pull request adds the hash after the
    owner reads the diff of the plugin files, and amends the procedure in the constitution if a step changed.

### CUJ 5: Convert an existing feature

*   **Given**: A feature in `specs/NNN-slug/` with open tasks, and no open branch that still ticks its `tasks.md`.
*   **When**: A conversion pull request creates `docs/designs/NNN-slug.md`, moves the tasks with their ids and ticks,
    and marks the old `spec.md`, `plan.md` and `tasks.md` as superseded.
*   **Then**: The check confirms that the set of task ids and the counts of ticked and open tasks are the same as in
    the source. A later edit to a superseded file fails the check.

## Technical Plan (Architecture)

### Shape of the change

```text
BEFORE
  .specify/memory/constitution.md  ->  Spec Kit skills (10)  ->  specs/NNN-slug/{spec,plan,tasks}.md  ->  code
        v3.0.0                          /speckit-*                  Check in plan.md, ticks in tasks.md

AFTER
  docs/constitution.md  ->  EGM (brainstorm, draft, review)  ->  docs/designs/NNN-slug.md  ->  code
        v4.0.0                                                     status block + Check + plan with tasks
        |                                                          |
        |  scripts/check-design-docs.py (new)                      +-- docs/designs/NNN-slug.reviews/DATE/
        |  scripts/check-agent-config.py (extended)
        |  docs/engineering/egm-accepted-hashes.json (new)         specs/NNN-slug/ keeps contracts/, evidence/,
        +-- .claude/settings.json declares the EGM plugin             research.md, quickstart.md, checklists/
```

### 1. The constitution, version 4.0.0

The file moves with `git mv .specify/memory/constitution.md docs/constitution.md` in a commit of its own, so history
follows it. A second commit edits it. Squash merging is not used (section 4 explains why).

Every unit of version 3.0.0 gets one verdict in the Sync Impact Report, in the format of the 3.0.0 report
(`docs/governance/constitution-3.0.0-sync-impact.md`) plus the verdicts MOVE and DROP:

| Unit of 3.0.0 | Verdict | Change |
|---|---|---|
| Principles I to VIII | KEEP | Text unchanged. Only a link or path that moved is corrected. |
| Delivery Constraints | KEEP | Unchanged. |
| Development Workflow: Specification first | KEEP | Unchanged. |
| Development Workflow: Spec Kit cycle | REWRITE | Replaced by the EGM procedure (section 2). |
| Development Workflow: Identifiers | REWRITE | The sentence about the Spec Kit template is removed. The id scheme is unchanged. |
| Development Workflow: Read before you change | REWRITE | Names the constitution and the design doc of the active feature. |
| Development Workflow: Same-delivery rule | REWRITE | "A checked task in `tasks.md`" becomes "a ticked task in the design doc's plan". |
| Development Workflow: CI gates | KEEP | Gains the new design-doc check. |
| Development Workflow: Branches and commits | KEEP | Unchanged. |
| Development Workflow: "Features 001-012 exist; a new feature continues from 013" | DROP | Stale. No obligation is lost. |
| Governance: supremacy, ADR divergence, amendments, versioning | KEEP | Unchanged. |
| Governance: Compliance review | REWRITE | "Every `plan.md` carries a Constitution Check" becomes "every design doc carries a Constitution Check and a Complexity Tracking section". The obligation is the same. |
| Approval rule | ADD | Section 3. |
| Owner decisions | ADD | Section 5. Today this sits in a bullet of `AGENTS.md`. |
| EGM pin | ADD | Section 6. |

The version is MAJOR (4.0.0) because the carrier of the Compliance review obligation changes, as in 2.0.0 and 3.0.0.
The Sync Impact Report lists the dependents (`AGENTS.md` rules 1 and 2 and "Start here", `CLAUDE.md`, the reviewer
agents, the skills, the docs in section 8), the risks and the rollback. The 1.1.0 report now in
`.specify/memory/sync-impact-rust-sdk-2026-10-27.md` moves to `docs/governance/` before `.specify/` is deleted.

### 2. The procedure inside the constitution

The Development Workflow section holds the whole procedure, with the skill names (owner decision). Its stages:

1.  **Idea**: `egm-brainstorm` writes `concepts_brief.md`.
2.  **Draft**: `egm-draft` writes the design doc, section by section, with the owner. It is saved as
    `docs/designs/NNN-slug.md`, where `NNN` is the number of the feature. Numbers are shared by all branches, so the
    author runs `scripts/next-feature-id.py` first. It reads `origin/*`, the sibling worktrees and the table of
    reserved ranges, and prints the next free feature number and the free ranges for `D`, `G`, `O` and ADR numbers
    (owner decision; section 9).
3.  **Review**: `egm-review` (with `egm-review-loop` and `egm-resolve`) runs reviewers with no conversation history.
    The `architecture-reviewer`, `security-reviewer` and `correctness-reviewer` agents of this repository also read
    the doc, because EGM's lenses do not cover constitution compliance, security or correctness. The reports are
    committed in `docs/designs/NNN-slug.reviews/YYYY-MM-DD/`.
4.  **Approve**: by the rule in section 3. No person is asked.
5.  **Build**: code, tests and documents in the same delivery. A task is ticked only with the evidence that closes it.
6.  **Reconcile**: `egm-drift-report` compares the code with the doc after delivery. It reports; the owner decides
    what to change. It replaces `/speckit-converge` in part only, and the design says so.

The cost of keeping the whole procedure in the constitution is that every change to a step is an amendment with a
version and a Sync Impact Report. Section 6 ties the text to the EGM files it describes, so a plugin upgrade that
changes a step is caught by the check and routed to an amendment.

### 3. Approval: record, rule and checks

**Rule.** A design doc is `Approved` when `egm-review` and each of the three repository reviewers
(`architecture-reviewer`, `security-reviewer`, `correctness-reviewer`) end with no Blocker and no Major finding open
(owner decision). A finding that the author contests, or that is recorded without a fix, counts as settled only when
the owner accepts it in writing in `docs/owner-decisions.md`. An escalation (ESCALATE) or a reviewer that did not run
keeps the doc `Draft`. Minor findings and below are in the reports and do not block.

The three repository reviewers are Claude Code agents defined in `.claude/agents/`. A session in another harness
cannot run them, so it cannot produce an `Approved` doc. They are pinned by the reviewed commit: their definitions
are files of the repository, so the commit in the record fixes the version that ran.

**Template.** EGM's own design template has no status block, no Constitution Check, no Complexity Tracking, no
requirement ids and no owner gates, and `egm-draft` documents no way to replace it. The repository therefore keeps
`docs/designs/TEMPLATE.md`, which extends EGM's template with those sections, and `docs/designs/README.md`, which
explains the layout, says that a doc whose name has no leading number is a cross-cutting design (like this one) and
carries no `FR/SC/T` ids, and maps each Spec Kit artifact to a section of a design doc. The author adds the extra
sections after the draft is written, and the check requires them.

**Record.** The record is a status block at the top of the doc (owner decision):

```text
*   **Status:** Approved
*   **Reviewed-commit:** <40 hex characters>
*   **EGM-hash:** <hash of the EGM files that were read>
*   **Review-report:** docs/designs/NNN-slug.reviews/YYYY-MM-DD/consolidated_review_report.md
*   **Repo-reviewer-reports:** docs/designs/NNN-slug.reviews/YYYY-MM-DD/{architecture,security,correctness}.md
*   **Open findings:** Blocker 0, Major 0 (across all four reports)
*   **Owner-accepted:** REV-003 (docs/owner-decisions.md, 2026-MM-DD, item N)
```

The author agent writes the block after the review. `scripts/check-design-docs.py` does not trust it. It checks:

*   the required headings exist, including one Constitution Check row for each of principles I to VIII and for the
    Delivery Constraints, and a Complexity Tracking section;
*   for an `Approved` doc, every field is present, the reviewed commit exists in the history (the check fails when it
    does not; it never skips), the EGM hash is in the accepted list (section 6), the EGM report and the three
    repository-reviewer reports exist and their counts match the block, and each owner-accepted finding has a matching entry in `docs/owner-decisions.md`;
*   task ids: the prefix equals the doc number, ids are unique, and an id is defined in one place across all design
    docs and non-superseded specs;
*   coverage: every `FR` and `SC` is cited by at least one task, and every task cites at least one requirement. This
    replaces `/speckit-analyze`'s coverage check;
*   the forward-only evidence rule: a line that is newly ticked in the diff against the base branch carries a date and
    an `Evidence:` line.

The check does not prove that a review ran in a fresh context or that the evidence is true. Those stay with the
people and agents who run them (section 10).

### 4. Task state after approval

Tasks are a section of the doc's Detailed Implementation Plan. Ticking a task changes the doc after it was reviewed.
The record handles this by the reviewed commit: the script runs a diff from `Reviewed-commit` to the working tree and
accepts only two kinds of change in the body of the doc: a checkbox going from `[ ]` to `[x]`, and a line that begins
with `*Evidence` or with a date and the words `partly done` inside a task. Any other change sets the doc back to
`Draft` and the check fails until the review is run again.

This works only if the reviewed commit stays reachable. So: pull requests that touch design docs are merged with a
merge commit and never squashed or rebased away (the Gitflow rule in `AGENTS.md` already merges with `--no-ff`), and
the CI job checks out the full history. The same reason applies to the evidence manifests of features 014 and 037,
which record a hash at a base commit.

### 5. Owner decisions: gates on tasks

The constitution gains a clause: product, scope, budget, release and security-exception decisions belong to the owner
and are recorded in `docs/owner-decisions.md`. A design doc's `Approved` status is a statement about the review, not
one of those decisions.

The gate is on the task. A task that depends on such a decision cites the entry it needs, in the form
`Owner-gate: docs/owner-decisions.md#<entry>`. The task cannot be started or ticked until the entry exists, and the
check enforces that for every ticked task that carries a gate. This is how feature 035's `T035-001` works today,
written as a rule. The check sees only gates that the author wrote; an unmarked task is a risk the reviewers must
catch (section 10).

### 6. Pinning the EGM

EGM is a per-user plugin today (version 0.1.0, commit `f0ba151b`). The upstream tag `v0.1.0` points to commit
`b3a35956`, which is 14 commits and 105 files behind the commit that ran the reviews so far, and both say "0.1.0". A
version string therefore does not identify the content, and `ref` in a settings file could only name that older tag.

*   `.claude/settings.json` registers the marketplace `google/elephant-goldfish-model` with `autoUpdate` false and
    enables `egm@egm`. Claude Code documents no way to require a version or a commit there. Each person runs
    `claude plugin install egm@egm --scope project` once, and `AGENTS.md` says so.
*   `scripts/egm-tree-hash.py` computes one hash from `plugin.json` and the files under the plugin's `agents/`, `docs/`,
    `resources/` and `skills/` (sorted relative paths and the hash of each file). `docs/` is included because the
    skills tell the agent to read `docs/claude.md` before they run. Review records carry this hash, produced by the
    script, not typed by hand.
*   `docs/engineering/egm-accepted-hashes.json` lists the accepted hashes. Its first entry is the commit that ran the
    reviews of this design. The check refuses a record whose hash is not in the list.
*   Upgrading EGM is a pull request that adds a hash after the owner has read the diff of the plugin files, and that
    amends the procedure in the constitution if a step changed.
*   The settings apply to Claude Code only. Codex, Cursor and other harnesses are not covered by these settings; a
    review from another harness is accepted only if the hash it records is in the list. `AGENTS.md` names the steps
    that need a harness able to start subagents.

The reviewers listed the "plugin rules" of the unadopted 3.0.0 candidate as related. See Q1.

### 7. Conversion of the 20 features

The 20 features with open tasks are: 014, 018, 019 to 032, 033, 035, 036 and 037. Each is converted in its own pull
request, after the checks of section 3 exist.

An agent writes each converted doc by mapping `spec.md`, `plan.md` and `tasks.md` onto the EGM template (owner
decision). The conversion is not an interactive `egm-draft` session, and the owner is not asked to approve sections.
`egm-review` judges the result, and the owner sees only the findings that need the owner's acceptance. Interactive
drafting stays for new features, where there is a design to discuss.

A conversion PR:

*   creates `docs/designs/NNN-slug.md` from `spec.md` (Overview, Problem, Goals, Non-Goals, user stories as CUJs,
    requirements with their ids), `plan.md` (Technical Plan, Constitution Check, Complexity Tracking) and `tasks.md`
    (the plan section with every task, its id, tick and evidence, **moved, not copied**);
*   leaves `contracts/`, `data-model.md`, `quickstart.md`, `checklists/`, `evidence/`, `research.md` and
    `validation-report.md` in `specs/NNN-slug/`. Tests and CI read some of these, and the registers must stay
    append-only. The design doc links to them;
*   puts a banner on the old `spec.md`, `plan.md` and `tasks.md`: "Superseded by `docs/designs/NNN-slug.md` at commit
    X". The check fails if their bodies change after X;
*   splits a large doc by increment. The markdown of feature 014 is 14,004 lines in 42 files and cannot be reviewed in one pass;
*   starts with a feature that has not started, to learn what the conversion costs before it touches a feature in
    flight.

The script `scripts/check-design-docs.py` confirms for each converted doc that the set of task ids and the counts of
ticked and open tasks equal those of the superseded `tasks.md` at X. It compares ids and counts, not meaning.

A converted draft is reviewed like any other doc. If the review ends clean, the rule of section 3 makes it
`Approved`, even for a draft such as 019 to 032 on which the owner has made no scope decision. The owner accepted
this consequence. The gate of section 5 is what keeps unapproved scope from being built: a task that depends on a scope
decision cites its entry.

Sibling branches that still tick an old `tasks.md` after a conversion fail the check on the superseded file. They are
told to move the tick to the design doc. This is a visible failure, not a silent one. Because the conversion is
done one feature at a time, each PR is coordinated with the branches and worktrees of that feature (today: 014, 016,
018, 036, 037 and 035 each have a worktree).

The cost is large. The reviewers estimated 15 to 27 reviewer runs per feature, so 300 to 540 in total. That is an
estimate and has not been measured. The first conversion measures it. The three repository reviewers add three
more runs per doc, 60 in total.

### 8. Removal and pointers

Deleted last: the `.specify/` directory (22 files on disk), the ten `.claude/skills/speckit-*` skills, and the Spec
Kit passages of the files below. Pointers are fixed in the same change as the constitution, because an amendment must
update its dependents (constitution, Governance).

*   `AGENTS.md` (rules 1 and 2 and "Start here"; the rule numbers stay, which keeps the 24 files that cite them valid),
    `CLAUDE.md`, `specs/AGENTS.md`, `specs/README.md`, `README.md`, `SPECIFICATION-INDEX.md`.
*   `.claude/hooks/session-start.py`: it reads `.specify/feature.json` to find the active feature. It will read the
    branch name `feature/NNN-slug` instead, look for `docs/designs/NNN-slug.md` (or `specs/NNN-slug/` before the
    conversion), and keep the warning about other branches that mention the same number.
*   `.claude/agents/architecture-reviewer.md`, `security-reviewer.md`, `correctness-reviewer.md`: they point at the
    Spec Kit artifacts. They are part of the approval rule of section 3, because EGM's seven lenses do not
    cover constitution compliance, security or correctness.
*   `.github/pull_request_template.md`, the skills `/verify`, `/parallel-agents`, `/adr` and `/migration`,
    `docs/engineering/{README,ai-engineering,sdd-workflow}.md` (the eight repository additions in `sdd-workflow.md`
    are kept; the nine-step Spec Kit table is replaced by a pointer to the constitution), `docs/traceability.md`,
    `docs/owner-decisions.md`, `tests/Cargo.toml` (the description names `plan.md`), and the quickstarts of 014 and
    016, which call a `.specify` script.
*   `scripts/check-agent-config.py` gains a check of backticked paths, a check that the version line equals the newest
    Sync Impact Report, a check that the eight principle headings exist, and a check that `speckit` appears nowhere
    outside the historical records listed in Goal 4. Today it checks relative links and rule numbering only.

### 9. Landing order and rollback

1.  Tag `develop` as the last Spec Kit state.
2.  **PR 1**: the constitution (`git mv`, then the edit), the Sync Impact Report, and every pointer fix of section 8
    except the deletions.
3.  **PR 2**: `scripts/check-design-docs.py`, the extended `check-agent-config.py`, `scripts/egm-tree-hash.py`, the
    accepted-hash list, the settings entry, the hook change, the id allocator `scripts/next-feature-id.py` with the
    table of reserved ranges, and the CI step with a full-history checkout.
4.  **PR 3 to PR 22**: one conversion per feature, starting with one that has not started.
5.  **Last PR**: delete `.specify/` and the ten skills, after the sibling branches have rebased.

Rollback is a forward amendment, as for 3.0.0, not `git revert`: once ids and ticks live in design docs, reverting the
move would restore stale task state.

### 10. What this design does not guarantee

*   The check proves that an approval record is consistent. It does not prove that the review ran with reviewers in a
    fresh context. `egm-review` is not run in CI.
*   Evidence and `partly done` lines are not reviewed text. They may carry words that look like design.
*   The conversion check compares ids and counts. A task whose meaning changed in the move passes it.
*   The owner gate protects only tasks that the author marked. An unmarked task that needed a decision passes the
    check.
*   An `Approved` converted draft can be read as permission to build. The gate of section 5 is the only barrier.
*   A converted doc is mapped by an agent and was never discussed in a draft session. The agent can change the
    meaning of a requirement or a task while mapping, and the check sees only ids and counts. EGM's readiness
    standard may also fail a doc that came from a spec and not from a design discussion.
*   The id allocator sees only what the last `git fetch` brought, and its table of reserved ranges is current only if
    someone updates it when a range is taken.
*   The three repository reviewers run only in Claude Code, so only a Claude Code session can produce an `Approved`
    doc. They are pinned by the reviewed commit and not by a hash, so an edit to their definitions in the same commit
    changes what "reviewed" meant.
*   Reviewers read long documents selectively. The one real EGM run here said so.

## Alternatives Considered

The first four alternatives were weighed in the brainstorm (`concepts_brief.md`, "Alternatives Weighed"). The rest
were put to the owner as choices while this document was drafted. In every case the owner's decision is the reason
for rejection; the trade-off that was accepted is stated next to it.

### Alternative A: Keep Spec Kit

*   **Pros**: Nothing to migrate. About 374 source files cite `FR/SC/T` ids, and about 2,036 register rows keep their
    place.
*   **Cons**: The owner no longer wants it. The reviewers noted that no defect of Spec Kit was named as the reason.
*   **Reason for Rejection**: Owner decision of 2026-11-02: use only EGM.

### Alternative B: Delete `specs/` and move everything to `docs/designs/`

*   **Pros**: One layout for the whole repository.
*   **Cons**: CI reads `specs/001-platform-foundation/contracts/` (OpenAPI, protobuf, AsyncAPI, workflow schema), and
    tests read files of other features. Evidence of closed tasks would be lost.
*   **Reason for Rejection**: Owner decision of 2026-11-02 (remove the tool only, keep `specs/`). It would break the
    contract tests.

### Alternative C: Overlay, EGM for new features only

*   **Pros**: The cheapest option, and the one every review lens proposed. No id moves, no conversion cost, and
    `tasks.md` stays canonical for work in flight.
*   **Cons**: Two formats coexist for a long time. The owner wants one format.
*   **Reason for Rejection**: The owner chose to convert the features with open tasks.

### Alternative D: Convert each feature on a trigger

*   **Pros**: Spreads the cost. About 1,190 of the 1,268 open boxes are in drafts or features not started, and a
    draft that is never built is never converted. 014, 033 and 037 finish where they are.
*   **Cons**: Two formats coexist until the last trigger, and the index drifts.
*   **Reason for Rejection**: The owner chose to convert all 20 features with open tasks in one wave. The accepted
    cost is the 300 to 540 reviewer runs the reviewers estimated (not measured).

### Alternative E: Approval with a second state, or locked while an owner question is open

*   **Pros**: Keeps "approved" meaning that the owner approved the scope. All six reviewers advised it.
*   **Cons**: Adds a step the owner did not want. A lock on open owner questions also stops docs such as 035, which
    carry owner gates by design.
*   **Reason for Rejection**: Owner decision: pure automatic approval, with "blocking" defined as Blocker or Major and
    only the owner accepting a contested finding. The per-task owner gate (section 5) keeps owner decisions without
    a second state.

### Alternative F: Procedure outside the constitution, or only its stages inside

*   **Pros**: A change to a skill name or a command needs no amendment. `docs/engineering/sdd-workflow.md` is already
    link-checked.
*   **Cons**: An obligation can lose its step without anyone amending anything.
*   **Reason for Rejection**: Owner decision: the whole procedure lives in the constitution. The accepted cost is an
    amendment for every step change, which section 6 routes through the accepted-hash check.

### Alternative G: Approval record in a central ledger or in a file per doc

*   **Pros**: The doc does not change when it is approved. A ledger is append-only, like the D/G/O registers.
*   **Cons**: A shared ledger is touched by all 20 conversion pull requests and conflicts on merge. A per-doc JSON
    file puts the owner's acceptance in a second file.
*   **Reason for Rejection**: Owner decision: the status block lives in the doc.

### Alternative H: Hash that ignores ticks, or tasks in a sibling ledger file

*   **Pros**: A hash that ignores ticks does not depend on git history. A sibling ledger leaves the approved doc
    untouched.
*   **Cons**: The normalizing rule can hide text in evidence lines. A sibling ledger splits the tasks from the plan,
    against the owner's decision that tasks are a section of the plan.
*   **Reason for Rejection**: Owner decision: a diff against the reviewed commit. Its weakness (a squash, a rebase or a
    shallow clone hides the commit) is handled by the merge-commit rule and the full-history checkout of section 4.

### Alternative I: Other ways to pin EGM

*   **Pros**: Recording only the version is the cheapest. A `ref` on the upstream tag uses a documented field. A fork
    with a `sha` makes Claude Code enforce an exact commit. A vendored copy is reproducible and reviewable.
*   **Cons**: The version string `0.1.0` does not tell two commits apart. The tag `v0.1.0` is 14 commits and 105 files
    behind the commit in use. A fork must be maintained and its creation is an external action. A vendored copy
    ages. Project settings cannot require a version or a commit.
*   **Reason for Rejection**: Owner decision: settings declaration plus a list of accepted hashes in the repository.

### Alternative J: Owner decisions by "stop and ask", or by a review finding

*   **Pros**: "Stop and ask" costs nothing and matches today's practice. A Major finding under EGM's readiness rule
    uses a gate that already exists.
*   **Cons**: "Stop and ask" depends on the agent noticing the question, which is how feature 016 was built twice. The
    readiness rule depends on how a reviewer grades severity, and it also fails docs with a deliberate spike.
*   **Reason for Rejection**: Owner decision: a gate on the task, checked by the script.

### Alternative K: Interactive drafting for every converted feature

*   **Pros**: Full EGM discipline, and the owner knows each doc.
*   **Cons**: Four section gates for each of 20 features is more than 80 owner gates, plus the reviews.
*   **Reason for Rejection**: Owner decision: an agent maps the sources, and `egm-review` judges. Drafting stays
    interactive for new features.

### Alternative L: No new checks, rules kept as prose

*   **Pros**: No new code. No script reads `tasks.md` today, so this keeps the status quo.
*   **Cons**: The rules that Spec Kit's layout half-supplied (the Check, the evidence rule, id uniqueness, coverage)
    would be unenforced, and `AGENTS.md` rule 21 says architecture is enforced by fitness functions, not by prose.
*   **Reason for Rejection**: The goals of this design require that each rule has a named carrier, and the
    approval record needs a check that can fail.

### Alternative M: Conformance, security and correctness review outside the approval of the doc

*   **Pros**: Approving a doc stays cheap, with `egm-review` alone. The three repository reviewers run on the diff of
    the implementation, as `CLAUDE.md` already says before a feature is declared done. A variant passes the
    constitution to every `egm-review` and requires the Constitution Check section, with no extra agent.
*   **Cons**: A design that violates the constitution can be `Approved` and found only after it is built, because the
    Check section is the author's own claim. Reading the constitution does not give EGM a lens that verifies it.
*   **Reason for Rejection**: Owner decision: the three repository reviewers are part of the approval. The accepted
    cost is about 60 more reviewer runs, and that only Claude Code can produce an `Approved` doc.

### Alternative N: No id allocator, or only a duplicate check in CI

*   **Pros**: No new code if the manual look at `origin/*` stays. A duplicate check in `check-design-docs.py` is cheap
    and catches a clash before the merge.
*   **Cons**: The manual look already failed once: two branches took the same number and it was found afterwards. A
    CI check finds the clash only when the second pull request opens, after both authors have worked, and cannot
    see branches that were never pushed.
*   **Reason for Rejection**: Owner decision: the allocator ships in PR 2. The accepted cost is a table of reserved
    ranges that someone must keep current, and an allocator that sees only what the last `git fetch` brought.

## Non-Functional Requirements

*   **Security and supply chain.** EGM is a third-party plugin that runs with the permissions of the person using it,
    and its reviewers read files in the repository. The pin (section 6) limits the plugin to files whose hash the
    owner accepted. The design adds no secret and widens no permission: the `.claude/settings.json` change adds a
    marketplace and a plugin entry and does not add to the `permissions.allow` list. Design docs must hold no secret,
    because reviewers read them. Output of a subagent is data: the harness flagged two of this design's own review
    outputs as instruction-shaped because they quoted a settings path, and the content held findings only.
*   **Rollout and rollback.** Section 9 gives the order. Until the first conversion merges, each of PR 1 and PR 2 can
    be reverted alone. After conversions, rollback is a forward amendment. The hook change keeps working for
    branches that have no design doc yet, by falling back to `specs/NNN-slug/`.
*   **Cost.** The conversion is estimated at 300 to 540 EGM reviewer runs (not measured), plus three runs of the repository
    reviewers per doc. The first conversion is the
    measurement, and the plan is revisited with the real number before the second one starts. The CI job adds one
    script that reads each design doc and runs one `git diff` per approved doc; its run time is not measured and is
    recorded with the first conversion.

### Additional Resources

*   `.specify/memory/constitution.md` (version 3.0.0) and `docs/governance/constitution-3.0.0-sync-impact.md`.
*   `docs/engineering/sdd-workflow.md`, `docs/engineering/ai-engineering.md`, `AGENTS.md`, `specs/AGENTS.md`.
*   `tasks/reviews/2026-11-02-egm-constitution/concepts_brief.md`: the brainstorm, the six-lens review and the
    owner's decisions of 2026-11-02.
*   EGM `skills/egm-review/SKILL.md`, `skills/egm-draft/references/design_doc.md`, `docs/getting_started.md`
    (plugin commit `f0ba151b`).
*   Claude Code documentation on plugins and `extraKnownMarketplaces`:
    `https://code.claude.com/docs/en/plugins/marketplace-reference.md` and
    `https://code.claude.com/docs/en/plugins/loading.md`.

### Future Improvements

*   **One generated index.** Three indexes exist today (`SPECIFICATION-INDEX.md`, `docs/roadmap.md`,
    `docs/traceability.md`), and they have drifted. The status blocks could generate one.
*   **A review in CI**, if EGM adds a way to run reviewers headless. Today the harness list does not include CI.
*   **Vendoring EGM**, if upstream changes faster than the owner can read the diffs.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Nothing below is built. The plan follows the landing order of section 9. Line numbers are those of `develop` at commit
`df59eb0e`. All paths are relative to the repository root.

### Target Files to Create/Modify/Delete

#### Step 0. Tag

*   `git tag speckit-last-state develop` and push the tag. This is an external action and the owner approves it when
    PR 1 is opened. It gives the evidence manifests of 014 and 037 and any rollback a fixed point.

#### PR 1. The constitution, the design-doc layout and the pointers

##### 1. `docs/constitution.md` (Move from `.specify/memory/constitution.md`, then Modify)

*   **Type / Scope**: Normative document.
*   **Proposed Changes**: Commit 1 is a pure `git mv`. Commit 2 applies the verdict table of section 1: principles I to VIII
    and Delivery Constraints unchanged; Development Workflow rewritten for the EGM procedure of section 2, with the
    approval rule (section 3) and the owner-decision clause (section 5); Governance's Compliance review renamed to the
    design doc; the EGM pin rule (section 6) added; the stale "features 001-012" sentence dropped. The footer becomes
    `Version: 4.0.0`, last amended on the merge date, and the link at its end is corrected for the new directory.

##### 2. `docs/governance/constitution-4.0.0-sync-impact.md` (Create)

*   **Type / Scope**: Sync Impact Report in the format of the 3.0.0 report.
*   **Proposed Changes**: A table with one row per unit of 3.0.0 and the verdict KEEP, MOVE, REWRITE, DROP or ADD;
    the dependents; the risks of section 10; and the rollback (a forward amendment).

##### 3. `docs/governance/sync-impact-rust-sdk-2026-10-27.md` (Move from `.specify/memory/sync-impact-rust-sdk-2026-10-27.md`)

*   **Type / Scope**: Historical record of the 1.0.0 to 1.1.0 amendment.
*   **Proposed Changes**: `git mv` only. It is the only record of that amendment and would be deleted with `.specify/`.

##### 4. `docs/governance/constitution-3.0.0-sync-impact.md` (Modify)

*   **Type / Scope**: Historical record.
*   **Proposed Changes**: One line near the top: "The constitution moved to `docs/constitution.md` in 4.0.0; paths
    below name its location at 3.0.0." The line 46 path stays as written.

##### 5. `docs/designs/README.md` (Create) and `docs/designs/TEMPLATE.md` (Create)

*   **Type / Scope**: Layout and template of design docs (section 3, "Template").
*   **Proposed Changes**: The README describes the naming (`NNN-slug.md`, the same `NNN` as `specs/`, and unnumbered names for
    cross-cutting designs), the status block, the
    reviews directory, the required sections, and a table that maps each Spec Kit artifact (`spec.md`, `plan.md`,
    `tasks.md`, `research.md`, `contracts/`, `quickstart.md`, `checklists/`) to its place. The template extends EGM's
    template with the status block, Requirements with ids, Constitution Check, Complexity Tracking and Owner gates.

##### 6. `AGENTS.md` (Modify)

*   **Type / Scope**: Agent rules, shared by all agents.
*   **Proposed Changes**: Line 3 drops "(Spec Kit)". Line 7 and rule 1 (line 16) name `docs/constitution.md` and the
    design doc of the active feature, and the branch name replaces `.specify/feature.json`. Line 9 replaces the
    Spec Kit cycle with the EGM procedure and says which steps need a harness that can start subagents and that approval needs Claude Code, which runs the
    three repository reviewers. Line 11 says
    "in the design doc's plan". A short "Install" line tells each person to run
    `claude plugin install egm@egm --scope project`. The "only the owner decides" bullet points at the constitution
    clause. Rule 2's "approved dependency" becomes "a dependency recorded in the design doc". The rule numbers do not
    change, and the file stays inside the line budget that `check-agent-config.py` enforces.

##### 7. `CLAUDE.md` (Modify)

*   **Type / Scope**: Claude Code specifics.
*   **Proposed Changes**: Line 7 replaces the `/speckit-*` description with the EGM skills (which the `Skill` tool refuses,
    so they are read and followed in the same thread) and keeps `/verify`, `/adr`, `/migration`, `/parallel-agents`.
    Line 10 says "re-read the active design doc" instead of `tasks.md`.

##### 8. `README.md` (Modify)

*   **Type / Scope**: Project overview.
*   **Proposed Changes**: Line 677 describes `specs/NNN-slug/` as contracts, evidence and registers, and adds
    `docs/designs/`. Line 681 names `docs/constitution.md`. Lines 721 and 722 drop the Spec Kit and `feature.json` text.

##### 9. `SPECIFICATION-INDEX.md` (Modify)

*   **Type / Scope**: Feature inventory.
*   **Proposed Changes**: Line 71 links `docs/constitution.md`. A short paragraph says that converted features are
    described in `docs/designs/`. Each conversion PR updates the status of its row.

##### 10. `specs/AGENTS.md` and `specs/README.md` (Modify)

*   **Type / Scope**: Rules for the `specs/` directory.
*   **Proposed Changes**: `specs/AGENTS.md` lines 3 and 9 stop saying the directories are produced and generated by Spec Kit,
    and state which files stay live (contracts, evidence, registers) and which are superseded after conversion.
    `specs/README.md` line 5 drops "within the Spec Kit feature tree".

##### 11. `docs/engineering/sdd-workflow.md` (Modify)

*   **Type / Scope**: Workflow guide.
*   **Proposed Changes**: Retitled for design docs. The nine-step Spec Kit table is replaced by a pointer to the constitution's
    procedure. The eight "what the repository adds" items stay, with `tasks.md` read as "the design doc's plan".
    The "Writing a spec an agent can implement" and "Changing something that exists" sections are adapted. The
    anti-pattern "editing generated Spec Kit skills" is replaced by "ticking a superseded `tasks.md`".

##### 12. `docs/engineering/README.md` and `docs/engineering/ai-engineering.md` (Modify)

*   **Type / Scope**: Engineering index and the AI-engineering guide.
*   **Proposed Changes**: `README.md` lines 3 and 7 name the new constitution path and describe the guide as the design-doc
    workflow. `ai-engineering.md` gains the EGM pin under its section on plugins as supply chain, and line 63 drops the
    Spec Kit reference.

##### 13. `.claude/agents/architecture-reviewer.md`, `security-reviewer.md`, `correctness-reviewer.md` (Modify)

*   **Type / Scope**: Read-only reviewers.
*   **Proposed Changes**: Line 10 of each points to `docs/constitution.md` and to the design doc instead of `.specify/memory/...`,
    `spec.md`, `plan.md` and `tasks.md`. They become the compliance, security and correctness gate that EGM's seven
    lenses do not cover.

##### 14. `.claude/skills/verify/SKILL.md` and `.claude/skills/parallel-agents/SKILL.md` (Modify)

*   **Type / Scope**: Project skills.
*   **Proposed Changes**: `verify` line 28 and `parallel-agents` lines 13 and 19 say "the design doc's plan" instead of
    `tasks.md`/`spec`/`plan`. `verify` adds `check-design-docs.py` to its checklist in PR 2.

##### 15. `.github/pull_request_template.md` (Modify)

*   **Type / Scope**: Pull request template.
*   **Proposed Changes**: Line 3 links the design doc, line 19 says "design doc (tasks ticked with evidence)", and line 25
    says the Constitution Check holds or the violation is justified in the design doc's Complexity Tracking.

##### 16. `docs/owner-decisions.md` (Modify)

*   **Type / Scope**: Register of owner decisions.
*   **Proposed Changes**: A new dated section records the decisions of 2026-11-02: EGM replaces Spec Kit; the tool only; the
    constitution at `docs/constitution.md`; all eight principles kept; automatic approval with the blocking
    definition; conversion of the 20 features; the five architecture decisions of this design.

##### 17. `tests/Cargo.toml` (Modify)

*   **Type / Scope**: Cargo manifest of the test crate.
*   **Proposed Changes**: The description on line 3 says "paths follow `docs/designs/*.md`" instead of `specs/*/plan.md`.

##### 18. `specs/014-workflow-primitives/quickstart.md` (Modify) and `specs/016-payload-classification/quickstart.md` (Modify)

*   **Type / Scope**: Quickstarts that call `.specify/scripts/bash/check-prerequisites.sh` (lines 23 and 46).
*   **Proposed Changes**: Replace the call with a plain `ls` of the files it checked. The 016 quickstart is parsed by
    `tests/e2e/payload-classification/quickstart.rs`, which compares the `sagat` commands it shows with the ones the
    walkthrough ran; that test runs after the edit.

#### PR 2. Checks, hook, pin and id allocator

##### 19. `scripts/check-design-docs.py` (Create) and `scripts/tests/test_check_design_docs.py` (Create)

*   **Type / Scope**: Python 3 script, no dependencies, and its unit tests in the style of `test_check_architecture.py`.
*   **Proposed Changes**: The checks of sections 3 to 5 and 7: required headings and Check rows; the status block of an
    approved doc; reachability of the reviewed commit (failure, never a skip); the tick-and-evidence-only diff; the
    accepted EGM hash; owner-accepted findings and owner gates against `docs/owner-decisions.md`; id prefix, uniqueness
    and one definition site; requirement-to-task coverage; the forward-only evidence rule; superseded files unchanged
    since their commit; conversion fidelity (same ids and counts). `docs/designs/TEMPLATE.md` and `README.md` are skipped
    by name. Tests: one passing case and one failing case for each rule.

##### 20. `scripts/egm-tree-hash.py` (Create) and `scripts/tests/test_egm_tree_hash.py` (Create)

*   **Type / Scope**: Python 3 script and tests.
*   **Proposed Changes**: Prints one hash of `plugin.json` and the files under a plugin directory's `agents/`, `docs/`,
    `resources/` and `skills/` (sorted relative paths and the hash of each file). Tests: the same tree gives the same
    hash, a changed byte changes it, and a file outside those folders (for example `site/` or `tests/`) does not.

##### 21. `docs/engineering/egm-accepted-hashes.json` (Create)

*   **Type / Scope**: List of accepted EGM hashes with the plugin commit and the date each was accepted.
*   **Proposed Changes**: One entry, produced by running `egm-tree-hash.py` on the installed plugin (commit `f0ba151b`).
    The owner reads and accepts it in PR 2.

##### 22. `.claude/settings.json` (Modify)

*   **Type / Scope**: Project settings for Claude Code.
*   **Proposed Changes**: Add `extraKnownMarketplaces.egm` (GitHub `google/elephant-goldfish-model`, `autoUpdate` false) and
    `enabledPlugins["egm@egm"]`. No permission is added.

##### 23. `.claude/hooks/session-start.py` (Modify) and `scripts/tests/test_session_start_hook.py` (Create)

*   **Type / Scope**: SessionStart hook and its first test.
*   **Proposed Changes**: Read the branch name instead of `.specify/feature.json`. From `feature/NNN-slug`, find
    `docs/designs/NNN-slug.md` or `specs/NNN-slug/`, and keep the warning about other branches that mention `NNN`.
    Tests: a feature branch, a branch that is not a feature, and a missing doc.

##### 24. `scripts/check-agent-config.py` (Modify) and `scripts/tests/test_check_agent_config.py` (Modify)

*   **Type / Scope**: Existing checker and its tests.
*   **Proposed Changes**: Add the backticked-path check, the version-line check against the newest Sync Impact Report, the
    eight-heading check, the "Enforced by" path check, and the no-`speckit`-outside-historical-records check.

##### 25. `scripts/next-feature-id.py` (Create), `scripts/tests/test_next_feature_id.py` (Create) and `SPECIFICATION-INDEX.md` (Modify, second edit)

*   **Type / Scope**: Python 3 script, its tests, and the reserved-ranges text of the index.
*   **Proposed Changes**: The script reads the branches of `origin`, `git worktree list`, the `specs/` and `docs/designs/`
    directories of each, and a table of reserved ranges, and prints the next free feature number and the free ranges
    for `D`, `G`, `O` and ADR numbers. `SPECIFICATION-INDEX.md` turns its prose about reserved ranges (for example
    ADR-0050 to ADR-0068 and the `D`/`G`/`O` 1000 to 1674 blocks) into a table the script can parse. The script warns
    when its last `git fetch` is older than a day. Tests: a number taken only on another branch is skipped, a reserved
    range is not offered, and a stale fetch is reported.

##### 26. `.github/workflows/ci.yml` (Modify)

*   **Type / Scope**: CI workflow, job `architecture` (lines 165 to 177).
*   **Proposed Changes**: Check out with `fetch-depth: 0` and add `python3 scripts/check-design-docs.py` after
    `check-agent-config.py`. The `unittest discover` step already picks up the new tests.

##### 27. `.claude/skills/verify/SKILL.md` (Modify, second edit)

*   **Type / Scope**: Project skill.
*   **Proposed Changes**: Add `python3 scripts/check-design-docs.py` to the list of gates it runs.

#### PR 3 to PR 22. One conversion per feature

Each PR creates `docs/designs/NNN-slug.md`, puts the superseded banner on `specs/NNN-slug/spec.md`, `plan.md` and
`tasks.md`, adds the review reports in `docs/designs/NNN-slug.reviews/`, and updates its row in `docs/traceability.md`
and `SPECIFICATION-INDEX.md`. Counts are those of `develop` at `df59eb0e`; the check compares them after the move.

| PR | Feature | Ticked | Open | Note |
|---|---|---:|---:|---|
| 3 | 019-worker-deployments-dispatch | 0 | 68 | Not started. Used to measure the cost. |
| 4 | 020-visibility-analytics-archival | 0 | 62 | Not started. |
| 5 | 021-integration-resources-triggers | 0 | 80 | Not started. |
| 6 | 022-flow-control-step-policies | 0 | 65 | Not started. |
| 7 | 023-script-languages | 0 | 66 | Not started. |
| 8 | 024-data-engines-pipelines | 0 | 69 | Not started. |
| 9 | 025-ai-agents-mcp | 0 | 81 | Not started. |
| 10 | 026-invocation-human-surfaces | 0 | 68 | Not started. |
| 11 | 027-internal-apps | 0 | 61 | Not started. |
| 12 | 028-developer-tooling | 0 | 74 | Not started. |
| 13 | 029-custom-roles-folders | 0 | 69 | Not started. |
| 14 | 030-cross-namespace-services | 0 | 70 | Not started. |
| 15 | 031-client-side-encryption | 0 | 69 | Not started. |
| 16 | 032-multi-region-failover | 0 | 64 | Not started. |
| 17 | 036-elastic-scaling | 0 | 94 | Spec only; has an open worktree. |
| 18 | 033-redis-streams | 6 | 1 | Closing out. |
| 19 | 037-compensation-safety | 32 | 2 | Closing out; evidence manifests cite the constitution at a base commit. |
| 20 | 014-workflow-primitives | 37 | 8 | 14,004 lines of markdown in 42 files; split by increment. |
| 21 | 018-schedules-bulk-operations | 13 | 68 | In construction; convert after its active branch merges. |
| 22 | 035-iceberg-write-steps | 0 on `develop`, 2 on its branch | 129 | Convert after `feature/035-iceberg-write-steps` merges, or its two ticks are lost. |

Total: 1,268 open tasks. The order puts features that nobody is changing first and the ones with open branches last.

#### PR 23. Removal

##### 28. `.specify/` (Delete, 21 tracked files)

*   **Type / Scope**: Spec Kit state, scripts, templates, workflows and integration manifests.
*   **Proposed Changes**: Delete every file listed here: `.specify/.gitignore`, `init-options.json`, `integration.json`,
    `integrations/claude.manifest.json`, `integrations/speckit.manifest.json`, `memory/.constitution-template.json`,
    `memory/constitution.md` (already moved), `memory/sync-impact-rust-sdk-2026-10-27.md` (already moved),
    `scripts/bash/{check-prerequisites,common,create-new-feature,resolve-template,setup-plan,setup-tasks}.sh`,
    `templates/{checklist,constitution,plan,spec,tasks}-template.md`, `workflows/speckit/workflow.yml` and
    `workflows/workflow-registry.json`. The ignored `.specify/feature.json` stays as an untracked file in checkouts
    that have it, and each person deletes it.

##### 29. `.claude/skills/speckit-*/SKILL.md` (Delete, 10 files)

*   **Type / Scope**: Generated Spec Kit skills.
*   **Proposed Changes**: Delete `speckit-analyze`, `speckit-checklist`, `speckit-clarify`, `speckit-constitution`,
    `speckit-converge`, `speckit-implement`, `speckit-plan`, `speckit-specify`, `speckit-tasks` and
    `speckit-taskstoissues`. This runs after sibling branches have rebased, so no session is left with a skill that
    points at a deleted script.

#### Files that are read and not changed

*   `specs/NNN-slug/{contracts,evidence}/`, `research.md`, `data-model.md`, `quickstart.md`, `checklists/` and
    `validation-report.md`: live, as the owner decided.
*   `docs/adr/ADR-0071-*.md`, the `/speckit-*` mentions in `docs/owner-decisions.md`, and
    `specs/001-platform-foundation/research.md`: dated records. The no-`speckit` check lists them as exceptions.
*   `tasks/reviews/2026-10-30-*`: earlier review records.
*   The evidence manifests of 014 and 037 that hash the constitution at a base commit: the tag of Step 0 keeps that
    commit reachable.

### Verification Plan

*   **Compilation/Build**: No Rust source changes. `cargo metadata --locked` still resolves after the `tests/Cargo.toml`
    description edit.
*   **Unit Tests**: `python3 -m unittest discover -s scripts/tests -v`, including the new tests for
    `check-design-docs.py`, `egm-tree-hash.py`, the session-start hook and the extended `check-agent-config.py`. Each
    rule has a case that must fail, seen failing before the rule is written.
*   **Static Analysis & Linting**: `python3 scripts/check-architecture.py`, `python3 scripts/check-agent-config.py` and
    `python3 scripts/check-design-docs.py` all exit 0 on the merged tree. The `speckit` search finds nothing outside
    the historical records.
*   **Quickstart test**: `cargo test -p sagat-tests --test e2e_payload_classification` for the edited 016 quickstart,
    run with a private `CARGO_TARGET_DIR`.
*   **Journeys**: CUJ 1 to 5 are run once each on a scratch branch. For CUJ 2, edit a sentence of an approved doc and
    see the check fail. For CUJ 4, record a review with an unlisted hash and see the check fail.
*   **Independent read**: the `architecture-reviewer`, `security-reviewer` and `correctness-reviewer` agents read the
    diff of PR 1 and PR 2 together with this design, not my conclusions.
*   **Cost**: PR 3 records the reviewer runs and the wall-clock time of one conversion, and the plan is revisited with
    that number before PR 4.


--------------------------------------------------------------------------------

*Drafted with EGM*
