# Constitution 4.0.0: EGM as the only specification workflow

*   **Author:** Marcelo Ortiz de Santana (owner)
*   **Status:** Draft
*   **Date:** 2026-11-03

--------------------------------------------------------------------------------

## Overview

Sagat Octopus Orchestrator is a durable workflow platform written in Rust. Its constitution (version 3.0.0, ratified
2026-10-11, last amended 2026-11-01) does two jobs. It states eight principles that every feature must meet (durable
state, modular domain, at-least-once effects, pinned compatibility, isolation, observability, redeliverable events,
honest tests). It also names Spec Kit as the way features are specified: the Development Workflow section lists the
commands from `/speckit-specify` to `/speckit-converge`, and the Governance section requires a Constitution Check in
every `plan.md`.

The owner decided to stop using Spec Kit and to use only the Elephant-Goldfish Model (EGM). In EGM, a design doc is
drafted section by section with the owner and then reviewed by agents that have no conversation history.

This design replaces Spec Kit with EGM. It does six things:

1.  The constitution moves to `docs/constitution.md` as version 4.0.0. All eight principles stay in force. Only the
    process parts of Development Workflow and Governance are rewritten.
2.  A rule states when a design doc counts as approved, and a script checks the record of that approval.
3.  A task that depends on an owner decision carries a prerequisite that the script checks before the task is ticked.
4.  The EGM plugin is pinned by a hash of the files that define its behavior.
5.  A small script prints the next free feature number.
6.  The Spec Kit tooling is removed in the same pull request that adopts the new constitution.

The conversion of the 20 features that have open tasks is **not** part of this design. The owner's decision to convert
them stands. It is moved to a follow-up design (`docs/designs/spec-conversion.md`), drafted after this design is built
and after a first conversion has been measured (Decisions, A).

Nothing described here exists yet unless it is marked "today". The Status line reads `Draft` because nothing is built.
An earlier, larger version of this design (880 and 1,216 lines) is in git history at commits `87dbb02e` and `95098f4a`,
with its review reports in `docs/designs/constitution-4.0.0-egm.reviews/2026-11-02/`.

## Terms

*   **EGM**: the Elephant-Goldfish Model, a Google open-source plugin for design documents. The **Elephant** is the agent
    that holds project context and drafts. A **Goldfish** is a reviewer agent started with no conversation history.
*   **EGM skills**: `egm-brainstorm` (writes a concepts brief), `egm-draft` (writes a design doc section by section),
    `egm-review` (runs five review stages), `egm-review-loop` (repeats a review round until it is clean), `egm-resolve`
    (classifies findings and applies approved fixes) and `egm-drift-report` (compares code with a doc).
*   **Lens agents**: EGM's seven reviewer agents. **Repository reviewers**: this repository's three read-only agents,
    `architecture-reviewer`, `security-reviewer` and `correctness-reviewer`.
*   **Harness**: the coding-agent environment that runs a model (Claude Code, Codex, Antigravity, Claude Cowork).
*   **ESCALATE**: the `egm-resolve` category for a finding whose evidence is outside every file the reviewer could read;
    it goes to the owner.
*   **Spec Kit**: the specification toolkit used so far. Per feature it generates `specs/NNN-slug/spec.md` (what and
    why), `plan.md` (how, with the Constitution Check) and `tasks.md` (the task list), through the `/speckit-*` skills.
*   **FR, SC, T ids**: `FR-NNN-xxx` is a functional requirement, `SC-NNN-xxx` a success criterion and `TNNN-xxx` a task,
    where `NNN` is the feature number. **D, G, O**: decision, gap and owner-question rows in a feature's `research.md`.
    **ADR**: an architecture decision record under `docs/adr/`.
*   **Constitution Check**: a table that states, for each principle, whether a design meets it. **Complexity Tracking**: a
    table that justifies each added moving part with the simpler alternative that was rejected. **Delivery
    Constraints**: the constitution section on technology baseline, out-of-scope items and security defaults. **Sync
    Impact Report**: the document that lists, for an amendment, what each clause became.
*   **Status block**: the header lines of a design doc that record its approval. **Task**: one Markdown checkbox line
    whose text begins with a task id, together with the indented lines that follow it. **Evidence block**: the part of a
    task that proves it is done (Technical Plan section 4). **Owner gate**: the `Owner-gate:` prerequisite of a task
    (section 5). **Author**: the person or agent that writes or edits a doc. **The check**: `scripts/check-design-docs.py`.

## Decisions

### Owner decisions of 2026-11-02

Taken in the gates of `/egm-brainstorm` and `/egm-draft` and recorded in
`tasks/reviews/2026-11-02-egm-constitution/concepts_brief.md`:

*   EGM is the only specification workflow. Only the tooling is removed; every `specs/NNN-*/` directory stays.
*   All eight principles stay whole. The new constitution is `docs/constitution.md`.
*   Approval is automatic and has one state. "Blocking" means Blocker or Major. A contested or unfixed finding counts as
    settled only when the owner accepts it in writing.
*   The record is a status block in the doc. Ticks after approval are checked by a diff against the reviewed commit.
*   The whole EGM procedure lives in the constitution.
*   The pin is a settings entry plus a list of accepted hashes in the repository.
*   An owner decision gates a task, not the doc. The three repository reviewers are part of approval.
*   The 20 features with open tasks are converted into design docs by an agent that maps the sources, judged by
    `egm-review`. The id allocator ships with the tooling.

### Decisions taken under delegation, 2026-11-03

The owner asked the assistant to decide the open points: "Tome as decisões por mim." Each decision is reversible. The
owner may overrule any of them. They are recorded in `docs/owner-decisions.md` by PR 2.

*   **A. Scope and sequencing.** The conversion of the 20 features leaves this design and becomes a follow-up design,
    drafted after PR 2 and after the first conversion is measured. *Reason:* the first review round found that the
    conversion machinery (fidelity checks, evidence rules for moved tasks, splitting, a 20-row plan) caused most of the
    8 Blockers and 31 Major findings of the readiness stage. The owner's decision to convert all 20 is not reversed.
*   **B. Removal timing.** The Spec Kit tooling is removed in PR 2, with the constitution. There is no transitional
    clause. Features not yet converted keep their `tasks.md` and tick it by hand. *Reason:* all ten `speckit-*` skills
    read `.specify/memory/constitution.md`, which PR 2 moves, so a transitional clause would break the skills it
    protects; and removing the tool is the owner's request. At the time of this decision no sibling worktree of
    another session exists and no pull request is open.
*   **C. Version.** 4.0.0. The Versioning clause is amended in the same change: a change of the mandated specification
    process is MAJOR. *Reason:* the clause as written would make this MINOR, although it replaces the whole mandated
    workflow and the file location that other documents cite.
*   **D. No `CODEOWNERS` and no branch-protection change.** *Reason:* both are repository settings that affect every
    contributor and need the owner's GitHub identity. Recorded as a risk: review by the owner is by convention.
*   **E. No document-level `Scope-approval` field.** *Reason:* it is close to the locked-approval option the owner
    rejected, and the per-task owner gate stays.
*   **F. No blob fallback for the reviewed commit.** Branches that hold approved docs merge `develop` into themselves
    and are not rebased. *Reason:* the fallback could not match (the status block changes the blob), and the repository
    already merges `develop` into feature branches.
*   **G. Counts are stated per report.** Each report carries one `Counts:` line. The check sums them. There is no
    per-finding parser. *Reason:* reports come from third-party agents and free text.
*   **H. Owner gates cite an id.** `Owner-gate: OD-YYYY-MM-DD-N` names a row of `docs/owner-decisions.md` whose first
    cell is that id. *Reason:* the file's dated headings repeat and its tables have no common key.
*   **I. The allocator prints the next number only.** No table of reserved ranges. *Reason:* the `D`, `G`, `O` and ADR
    blocks are assigned in prose in `SPECIFICATION-INDEX.md`; turning them into a table is a separate piece of work.
*   **J. Contested findings.** The contests on pure automatic approval, the allocator and the diff mechanism are
    accepted: the owner's decisions stand. The contest on the conversion wave is handled by decision A.
*   **K. Confirmations.** Security exceptions use the homes that exist today (`docs/owner-decisions.md` and the gap
    register in `specs/001-platform-foundation/research.md`). Outside an EGM session the repository practice of asking
    with a recommendation stays, and inside an EGM brainstorm or draft the skill's ban on recommendations applies.
    Review reports live in `docs/designs/NNN-slug.reviews/YYYY-MM-DD/`.
*   **L. The "broader reviewed Constitution candidate"** of the 3.0.0 report stays out of scope (Open Questions).

## The Problem

Removing the Spec Kit files is mechanical. The rules that depend on its layout are the risk. The constitution and the
repository rely on Spec Kit's file layout for several rules, and EGM supplies none of them. Sources are named;
"reported" means a reviewer found it and the author did not re-read the source:

*   **A compliance gate.** Governance requires each `plan.md` to carry a Constitution Check and a Complexity Tracking
    table. A reviewer counted 32 plans with a Check. In `specs/035-iceberg-write-steps/plan.md` line 115 the
    technology-baseline row reads "Requires ADR" before design and "Pass pending ADR-0071 acceptance" after it.
*   **Task state with evidence.** `AGENTS.md` says a task is ticked only with the evidence that closes it. The ids are
    feature-scoped and are cited in commit footers. EGM's design template has no place for ids, ticks or evidence.
*   **Append-only registers.** `research.md` holds `D`, `G` and `O` rows that are never renumbered. The EGM template
    tells the author to remove answered questions.
*   **Contracts that tests and CI read.** CI lints four files under `specs/001-platform-foundation/contracts/`
    ([`ci.yml`](../../.github/workflows/ci.yml), lines 320-341). [`.cargo/audit.toml`](../../.cargo/audit.toml) requires
    each advisory ignore to be justified in `specs/001-platform-foundation/research.md`.
*   **Pointers that nothing checks.** [`scripts/check-agent-config.py`](../../scripts/check-agent-config.py) enforces
    line budgets, the `CLAUDE.md` import, skill and agent front matter, reviewer tool lists, hook existence and the ADR
    format, and it checks relative Markdown links only in `AGENTS.md`, `CLAUDE.md`, `docs/engineering/*.md`,
    `docs/adr/README.md` and nested `AGENTS.md` files. It does not check backticked paths and does not scan `README.md`,
    `SPECIFICATION-INDEX.md`, `specs/README.md` or `.claude/`.
*   **A duplicate-work warning.** The session-start hook reads `.specify/feature.json` and warns when other branches
    mention the same feature. It was added after feature 016 was built twice (reported).
*   **Feature numbers.** Spec Kit's `create-new-feature.sh` takes the highest number in the local `specs/` directory and
    adds one, so it cannot see numbers taken on other branches. `AGENTS.md` ("Start here") says "a clash happened".
*   **A pinned toolchain.** Spec Kit was pinned (reported). EGM is a per-user plugin, at version 0.1.0 today, that is not
    declared anywhere in the repository, and `docs/engineering/ai-engineering.md` says plugins are supply chain.

EGM also has no verdict. Its review skill allows one re-audit per stage, then records the remaining findings as
"accepted trade-offs" and moves on (`skills/egm-review/SKILL.md`, the "1-Audit Convergence Cap"). One EGM review of a
prior document (`tasks/reviews/2026-10-30-open-tasks/`) ended with 0 Blocker and 6 Major findings and every action
open. A rule that says "approved when the review is clean" is only as strict as its definition of clean.

## Goals

Each goal can be checked.

1.  **The constitution keeps every obligation.** `docs/constitution.md` exists as version 4.0.0. A **unit** of 3.0.0 is
    one heading or one bullet of its text; an **obligation** is a statement in it that uses MUST or MUST NOT. The Sync
    Impact Report gives every unit one disposition from KEEP, MOVE, REWRITE, DROP or ADD. All eight principles are
    KEEP. A REWRITE or DROP appears only in the process parts of Development Workflow and Governance, and each one
    says where its obligation went.
2.  **Approval is one rule, and the check verifies its record.** A design doc is approved when `egm-review` and the three
    repository reviewers each end with no Blocker or Major finding open, where a finding accepted by the owner is not
    open. The check verifies that the record is consistent (Technical Plan section 3).
3.  **Every rule the layout supplied has a named carrier,** or the design says that none exists (section 9).
4.  **Removal leaves no dangling pointer.** After PR 2, `check-agent-config.py` passes with its new backticked-path
    check, and a case-insensitive search for `speckit` and `Spec Kit` finds nothing on the live instruction surfaces:
    `AGENTS.md`, `CLAUDE.md`, `README.md`, `.claude/`, `.github/`, `scripts/` (except the search's own files),
    `docs/engineering/` and `docs/constitution.md`.
5.  **Nothing that tests or CI read moves.** The contract files, the evidence directories and the research registers stay
    where they are.
6.  **The architecture check still passes unchanged.** `scripts/check-architecture.py` checks crate layering and
    dependency direction. This change touches no crate dependency.
7.  **The EGM plugin is pinned by content.** A review record whose hash is not on the accepted list fails the check.
8.  **Feature numbers do not clash.** `scripts/next-feature-id.py` prints the next free number, and the check fails if two
    docs share a number.
9.  **A task that needs an owner decision cannot be ticked without it.**

## Non-Goals

*   Changing the substance of any of the eight principles.
*   Deleting or moving `specs/001-platform-foundation/contracts/` or any file that a test or CI job reads.
*   Converting the 20 features with open tasks (the follow-up design) or the 15 features without open tasks (history).
*   Letting an agent decide product, scope, release or security-exception questions in the platform. These stay with the
    owner. (The decisions of 2026-11-03 above are about this repository's process, delegated by the owner.)
*   Running `egm-review` in CI. The harnesses EGM supports do not include CI.
*   Renumbering the rules of `AGENTS.md`.
*   Changing any runtime code, schema, contract or ADR of the platform.

## Open Questions

*   **Q1: Where is the "broader reviewed Constitution candidate"?** The 3.0.0 Sync Impact Report
    (`docs/governance/constitution-3.0.0-sync-impact.md`) says it was not adopted and must be rebased before later use,
    and lists "authority hierarchy and plugin rules" among its parts. No file or branch with that name was found. This
    design leaves it out. Its plugin rules overlap with Technical Plan section 6.

## Critical User Journeys (CUJs)

### CUJ 1: Design a new feature

*   **Given**: An idea for a feature, on a branch cut from `develop`, and the number printed by
    `scripts/next-feature-id.py`.
*   **When**: The author runs `egm-brainstorm`, then `egm-draft`, saves the draft as `docs/designs/NNN-slug.md`, and runs
    `egm-review` (all five stages) and the three repository reviewers until none has a Blocker or Major finding open.
*   **Then**: The status block says `Approved` and holds the reviewed commit, the EGM hash, the Claude Code version, the
    report paths and hashes, and the counts. `check-design-docs.py` passes. No separate approval step happened.

### CUJ 2: Tick a task after approval

*   **Given**: An approved design doc with an unticked task `T018-012`.
*   **When**: The author ticks the box and adds an evidence block (Technical Plan section 4).
*   **Then**: `check-design-docs.py` passes, because only a checkbox and an evidence block changed since the reviewed
    commit. If the author also edits a sentence of the design, the check fails until the doc is reviewed again.

### CUJ 3: A task that needs an owner decision

*   **Given**: A task that depends on an owner decision and carries `Owner-gate: OD-2026-12-01-1`.
*   **When**: An agent ticks the task before the row `OD-2026-12-01-1` exists in `docs/owner-decisions.md` with a status
    that begins with "Decided".
*   **Then**: `check-design-docs.py` fails. After the owner records the row, the same change passes. The check does not
    detect that a task was started.

### CUJ 4: Upgrade the EGM plugin

*   **Given**: A list of accepted EGM hashes in the repository and a new EGM release on a developer machine.
*   **When**: A review is run with the new release and its record is committed.
*   **Then**: `check-design-docs.py` fails because the recorded hash is not on the list. A pull request adds the hash after
    a person reads the diff of the plugin's behavioral files, and amends the procedure in the constitution if a step
    changed.

## Technical Plan (Architecture)

### Shape of the change

```text
BEFORE
  .specify/memory/constitution.md --> Spec Kit skills (10) --> specs/NNN-slug/{spec,plan,tasks}.md --> code
        v3.0.0                          /speckit-*               Check in plan.md, ticks in tasks.md

AFTER
  docs/constitution.md --> EGM (brainstorm, draft, review) --> docs/designs/NNN-slug.md --> code
        v4.0.0                                                  status block + Check + tasks in the plan
           |                                                          |
           |  scripts/check-design-docs.py (new)                      +-- docs/designs/NNN-slug.reviews/DATE/
           |  scripts/check-agent-config.py (extended)
           |  scripts/next-feature-id.py (new)                    specs/NNN-slug/ keeps contracts/, evidence/,
           |  docs/engineering/egm-accepted-hashes.json (new)     research.md; existing features keep their
           +-- .claude/settings.json declares the EGM plugin     spec.md, plan.md and tasks.md until converted

PROCEDURE AND APPROVAL CHECK
  idea --> egm-brainstorm --> concepts_brief.md
       --> egm-draft (section approval by the owner) --> docs/designs/NNN-slug.md      Status: Draft
       --> egm-review (all five stages) + architecture, security and correctness reviewers
                 |  every report carries one line:  Counts: Blocker n, Major n, Nit n
                 v
           Blocker or Major open? --yes--> fix, or the owner accepts the finding in
                 | no                       docs/owner-decisions.md (an OD- row); run the reviews again
                 v
           the author writes the status block                                          Status: Approved
                 v
           CI: check-design-docs.py verifies the block (commit exists, hashes, report hashes, counts)
                 v
           build: tasks ticked with evidence blocks; only checkbox and evidence changes pass the diff check
```

### 1. The constitution, version 4.0.0

The file moves with `git mv .specify/memory/constitution.md docs/constitution.md` in a commit of its own, so history
follows it. A second commit edits it.

| Unit of 3.0.0 (a heading or a bullet) | Disposition | Change |
|---|---|---|
| The file `.specify/memory/constitution.md` | MOVE | To `docs/constitution.md`, by `git mv` in its own commit. |
| Core Principles I to VIII (one heading each) | KEEP | Text unchanged. Only a link or path that moved is corrected. |
| Delivery Constraints | KEEP | Unchanged. |
| Development Workflow: Specification first | KEEP | Unchanged. |
| Development Workflow: Spec Kit cycle | REWRITE | Replaced by the EGM procedure (section 2). Its last sentence ("Features 001–012 already exist; a new feature continues the sequence from 013.") is DROPped: stale, no obligation lost. |
| Development Workflow: Identifiers | REWRITE | The sentence about the Spec Kit template is removed. The id scheme is unchanged. |
| Development Workflow: Read before you change | REWRITE | Names the constitution and the design doc of the active feature. |
| Development Workflow: Same-delivery rule | REWRITE | "A checked task in `tasks.md`" becomes "a ticked task in the design doc". |
| Development Workflow: CI gates | KEEP | Gains the design-doc CI check. |
| Development Workflow: Branches and commits | KEEP | Unchanged. |
| Governance: supremacy, ADR divergence, amendments (one bullet each) | KEEP | Unchanged. |
| Governance: Versioning | REWRITE | MAJOR also covers a change of the mandated specification process (decision C). |
| Governance: Compliance review | REWRITE | "Every `plan.md` carries a Constitution Check" becomes "every design doc carries a Constitution Check and a Complexity Tracking section". The obligation is the same. |
| Footer: Version line and closing link paragraph | REWRITE | `Version: 4.0.0`, the amendment date, and the link to the Sync Impact Reports. |

Added units, not in 3.0.0: the **approval rule** (ADD, section 3), the **owner decisions clause** (ADD, moved from a bullet
of `AGENTS.md`, section 5) and the **EGM pin rule** (ADD, section 6). The Sync Impact Report also lists the dependents,
the risks and the rollback (a forward amendment). The 1.1.0 report now in
`.specify/memory/sync-impact-rust-sdk-2026-10-27.md` moves to `docs/governance/` before `.specify/` is deleted.

### 2. The procedure inside the constitution

The Development Workflow section holds the whole procedure, with the skill names:

1.  **Idea**: `egm-brainstorm` writes `concepts_brief.md`, saved in the review directory of the feature and committed
    with the design doc.
2.  **Draft**: `egm-draft` writes the design doc section by section, with the owner's section approval, saved as
    `docs/designs/NNN-slug.md`. The author runs `scripts/next-feature-id.py` first for the number.
3.  **Review**: `egm-review` runs the lens agents with no conversation history through all five stages (preset 0 or 1).
    The three repository reviewers also read the doc, because EGM's lens agents do not cover constitution compliance,
    security or correctness. Reports are committed in `docs/designs/NNN-slug.reviews/YYYY-MM-DD/`.
4.  **Approve**: by the rule in section 3. No separate approval step exists.
5.  **Build**: code, tests and documents in the same delivery. A task is ticked only with the evidence that closes it.
6.  **Reconcile**: `egm-drift-report` compares the code with the doc after delivery. It reports; the owner decides what
    to change. It covers the assessment half of `/speckit-converge` and not its second half, which appended the unbuilt
    work as new tasks; the author does that by hand.

Keeping the whole procedure in the constitution means that every change to a step is an amendment with a version and a
Sync Impact Report. A new EGM release that changes a step makes the recorded hash unlisted, which fails the check; a
person then reads the diff and decides whether an amendment is needed.

### 3. Approval: rule, record and check

**Rule.** A design doc is `Approved` when `egm-review` and each of the three repository reviewers end with no Blocker and
no Major finding open. The scale is Blocker, Major and Nit; a Nit does not block. A finding that the author contests, or
that is recorded without a fix, counts as settled only when the owner accepts it in writing as an `OD-` row of
`docs/owner-decisions.md`. An escalation or a reviewer that did not run keeps the doc `Draft`. "Open" means raised in the
last complete run of all four reviewers and not owner-accepted. A complete run uses all five stages of `egm-review`
(preset 0 or 1) and follows the last fix. No separate approval step exists. The owner acts only to accept a finding.
Merging a design-doc pull request is the normal merge under `AGENTS.md` rule 14 and is not an approval act.

The repository reviewers are Claude Code agents defined in `.claude/agents/`, so only a Claude Code session can produce
an `Approved` doc. They are pinned by the reviewed commit, because their definitions are files of the repository.

**Counts line.** Every report in the record contains exactly one line `Counts: Blocker <n>, Major <n>, Nit <n>`. The
author appends it to the EGM reports; the repository reviewers cannot write files (`check-agent-config.py` forbids
`Write`, `Edit` and `Bash` on them), so the author also saves their reports and appends their line. The numbers are the
highest severity any reviewer assigned. A synthesizer or an author does not lower a Blocker or Major.

**Template.** EGM's design template has a header with `Status: [Draft | Proposed | In Review | Approved]` and no
Constitution Check, no Complexity Tracking, no requirement ids and no place for owner decisions. `egm-draft` documents
no way to replace its template. The repository therefore keeps `docs/designs/TEMPLATE.md`, which extends it, and
`docs/designs/README.md`, which explains the layout. A doc whose name has no leading number is a cross-cutting design
(like this one): it is exempt only from `FR/SC/T` ids. The author adds the extra sections after the draft is written.

**Record.** The status block is the header at the top of the doc. Its `Status:` line is the only status line. The check
treats `Proposed` and `In Review` like `Draft`.

```text
*   **Author:** ...
*   **Status:** Approved
*   **Date:** YYYY-MM-DD
*   **Reviewed-commit:** <40 hex characters>
*   **EGM-hash:** <hex, from scripts/egm-tree-hash.py>
*   **Claude-Code:** <version>
*   **Review-reports:** <path> sha256:<hex>; <path> sha256:<hex>; ...
*   **Repo-reviewer-reports:** <path> sha256:<hex>; <path> sha256:<hex>; <path> sha256:<hex>
*   **Open findings:** Blocker 0, Major 0
*   **Accepted:** <n> findings; OD-YYYY-MM-DD-N, ...
```

The header ends at the first line of 20 or more hyphens. The author writes the block after the review, so every field
can be written by the author (Risks and Mitigations).

**The check.** `scripts/check-design-docs.py` scans `docs/designs/*.md` except `README.md` and `TEMPLATE.md`, plus
`docs/owner-decisions.md`, `docs/constitution.md` and `docs/engineering/egm-accepted-hashes.json`. It exits non-zero and
prints one line per failure. It never edits a file and never skips a rule it cannot run: a missing git history is a
failure. The rules:

*   **R1 sections.** Every doc has non-empty `## Constitution Check` and `## Complexity Tracking` sections. The first has
    a table row beginning `| I.` through `| VIII.` and one beginning `| Delivery Constraints`.
*   **R2 record** (only when `Status` is `Approved`). Every field above is present; `Reviewed-commit` exists
    (`git cat-file -e`); `EGM-hash` is on the accepted list; `Claude-Code` is at least the list's minimum version; every
    listed report exists and its SHA-256 equals the recorded one; every report has exactly one `Counts:` line; the sum
    of Blocker plus Major over all reports is at most the `Accepted` number; and each `OD-` id named exists in
    `docs/owner-decisions.md` with a status that begins with "Decided".
*   **R3 diff** (only when `Approved`). Section 4.
*   **R4 ids.** In a numbered doc, every task id is `T<NNN>-<digits>` with the doc's own number and is unique, and no other
    design doc defines the same id.
*   **R5 owner gates.** A ticked task with `Owner-gate: OD-...` needs that row to exist and be decided (section 5).
*   **R6 numbers.** Two numbered docs with the same number but different slugs fail. A directory `specs/NNN-*` with the
    same number as a doc and a different slug fails.
*   **R7 forward-only evidence.** A task that is ticked in the working tree and was not ticked at the merge base with
    `origin/develop` (or whose doc is new) has an evidence block.
*   **R8 constitution** (when `docs/constitution.md` exists). The version line equals the version of the newest
    `docs/governance/constitution-*-sync-impact.md`; the eight principle headings exist; each "Enforced by" path exists.

The check does not prove that a review ran in a fresh context, that the counts are true, or that the evidence is true.
Those stay with the people and agents who run them.

### 4. Task state after approval

A **task item** starts at a line matching `^\s*[-*] \[[ xX]\] T\d{3}-\d+` and runs over the lines that follow while they
are blank or indented by at least two spaces. Tasks are in the doc's Detailed Implementation Plan.

**Evidence block.** An evidence block is the lines of a task item from the first line that contains a marker to the end of
the item. The markers are `*Done YYYY-MM-DD.*`, `*Partly done YYYY-MM-DD.*`, `**Evidence (YYYY-MM-DD)**:` and a line
beginning `_Evidence:`. These are the forms in use today: `*Done 2026-10-29.* Evidence: ...` in
`specs/018-schedules-bulk-operations/tasks.md`, `**Evidence (2026-11-01)**:` in
`specs/037-compensation-safety/tasks.md` and `_Evidence: ..._` in `specs/014-workflow-primitives/tasks.md`. A block has
at most 40 lines and no line, after its indentation is removed, begins with `#`, `|` or three backticks. A task with only
a `Partly done` marker stays unticked. Example (an illustration, not a real task):

```text
- [x] T018-012 Add the schedule pause endpoint. Refs: FR-018-007, SC-018-002.
  Owner-gate: OD-2026-12-01-1
  *Done 2026-12-03.* Evidence: `cargo test -p sagat-tests --test integration_schedules pause_endpoint` ->
  3 passed, 0 failed.
```

**The diff rule (R3).** The script takes the doc at `Reviewed-commit` and the doc in the working tree. The **header** is
every line before the first line of 20 or more hyphens, and the **body** is every line after it. It runs
`git diff --unified=0 <Reviewed-commit> -- <path>` and examines the body only. Every changed line must be one of:
a task line that differs from its old version only in the checkbox character, or a line that lies inside an evidence
block in the version it belongs to, or a blank line adjacent to such a block. Anything else fails with the line number.
The rule is the same on pull requests and on pushes. If `Reviewed-commit` is not an ancestor of `HEAD`, the check fails.
Until the changed lines are reviewed again, the doc counts as `Draft`.

**Rebase convention.** A rebase, an amend or a squash merge can make the reviewed commit unreachable. Branches that
hold approved docs therefore merge `develop` into themselves and are not rebased, which is the practice the repository
already follows. Pull requests that touch design docs are merged with a merge commit (`AGENTS.md` rule 14 already says
`--no-ff`; no repository setting enforces it, so this is a convention and a recorded risk), and the CI job checks out
the full history. A rebased branch needs a new review run.

**Amending an approved doc.** A change that leaves every word unchanged (whitespace and Markdown layout) keeps `Approved`.
Any other change, including a typo, makes the doc `Draft`; the re-review covers the changed line range (`egm-review`
accepts a range) and the three repository reviewers on the diff, and the status block records the new reviewed commit.

### 5. Owner decisions: gates on tasks

The constitution gains a clause: product, scope, budget, release and security-exception decisions belong to the owner and
are recorded in `docs/owner-decisions.md`. A design doc's `Approved` status is a statement about the review, not one of
those decisions.

The gate is on the task. A task that depends on such a decision carries a line `Owner-gate: OD-YYYY-MM-DD-N`. A row of
`docs/owner-decisions.md` that a gate or an acceptance can cite begins with that id in its first cell and has a status
cell that begins with "Decided" once the owner has answered. Rows written before this change have no id and cannot be
cited. A task with several dependencies lists several ids on one line, separated by commas. The check enforces gates
for ticked tasks only. It cannot tell that a task was started, and it cannot tell who wrote a row: an agent can write one,
and the owner's review of the pull request is what stops it. A missing gate on a task that needed one passes the check.

### 6. Pinning the EGM

EGM is a per-user plugin today (version 0.1.0, installed commit `f0ba151b`). The upstream tag `v0.1.0` points to commit
`b3a35956`, which is 14 commits and 105 files behind the installed commit, and both say "0.1.0" (reported by comparing the
two commits on GitHub on 2026-11-02). A version string therefore does not identify the content.

*   `.claude/settings.json` registers the marketplace `google/elephant-goldfish-model` with `autoUpdate` false and
    enables `egm@egm`. Claude Code documents no way to require a version or a commit there, and no way to install a
    chosen commit through project settings. Each person runs `claude plugin install egm@egm --scope project` once,
    and `AGENTS.md` says so.
*   **The hash.** `scripts/egm-tree-hash.py <plugin-dir>` prints the SHA-256 of the lines `<relative path>\0<sha256 of the
    file>\n`, sorted by path, over these files of the plugin directory: `.claude-plugin/**` (the Claude Code manifest and
    the adapters that carry the reviewers' tool limits), `agents/**`, `resources/**`, `skills/*/SKILL.md`,
    `skills/*/references/**`, `docs/claude.md` and `docs/harnesses.md`. Claude Code skips the root `agents/` folder when
    the manifest lists adapters, so the adapters are in the set. The script exits 2 if
    `.claude-plugin/plugin.json` is missing. It reads the install directory
    (`~/.claude/plugins/cache/egm/egm/<version>`) and never the marketplace clone, which differed from the installed copy
    in four files under `skills/` and four under `docs/` on 2026-11-02.
*   `docs/engineering/egm-accepted-hashes.json` has the shape `{"minimum_claude_code": "2.1.271", "entries": [{"hash":
    "<hex>", "commit": "<40 hex>", "accepted": "YYYY-MM-DD"}]}`. The first entry is the hash of the installed copy at
    commit `f0ba151b`, which ran the reviews of this design. The minimum Claude Code version is the one the plugin's
    `docs/claude.md` names for reviewer isolation.
*   **What the pin does and does not do.** A review record with an unlisted hash fails the check. The check does not stop
    a developer from running another EGM version, and it cannot tell whether a step of the procedure changed: a person
    reads the diff of the behavioral files and decides. CI has no plugin installed, so it compares strings only.
*   Upgrading EGM is a pull request that adds an entry after the owner has read that diff, and that amends the procedure
    in the constitution if a step changed.
*   The repository reviewers cannot run in another harness, so a review from another harness cannot make a doc `Approved`.

### 7. Feature numbers

`scripts/next-feature-id.py` prints the next free three-digit feature number: the highest `NNN` found, plus one, among the
directories `specs/NNN-*` and the files `docs/designs/NNN-*.md` of the working tree and of every `origin/*` branch
(`git ls-tree --name-only <ref> specs/ docs/designs/`). It prints a warning on standard error if `.git/FETCH_HEAD` is
older than one day, and it exits 0. It does not allocate `D`, `G`, `O` or ADR numbers; `SPECIFICATION-INDEX.md` assigns
those blocks in prose (decision I).

### 8. Removal and pointers

The file list is in the Detailed Implementation Plan. The rule is: remove Spec Kit wording from every live instruction
surface, keep the rule numbers of `AGENTS.md`, and let the checks list what is left. PR 2 fixes the pointers, changes the
session-start hook to read the number from the branch name `feature/NNN-slug` (a branch with no number gets no warning),
turns on the backticked-path check and the scoped search for `speckit` and `Spec Kit`, and deletes the `.specify/`
directory (21 tracked files) and the ten `.claude/skills/speckit-*` skills. The backticked-path check scans the files that
`check-agent-config.py` scans today plus `README.md`, `SPECIFICATION-INDEX.md`, `specs/README.md`, `.claude/agents` and
`.claude/skills`; it resolves a token against the repository root with `git ls-files` (a directory exists if any tracked
file lies under it); and it takes its exceptions from `scripts/paths-allowlist.txt`.

### 9. Landing order, carrier of each rule, and rollback

1.  **PR 1, the tooling** (additive, no normative text): `check-design-docs.py`, `egm-tree-hash.py`, the accepted-hash list,
    the settings entry, `next-feature-id.py`, `docs/designs/README.md` and `TEMPLATE.md`, and the CI step with a
    full-history checkout.
2.  **PR 2, the constitution and the removal**: the move and edit of the constitution, its Sync Impact Report, the
    `docs/owner-decisions.md` rows, every pointer fix, the repository reviewers' `Counts:` contract, the hook change,
    the path and search checks, and the deletion of the Spec Kit tooling.
3.  **Follow-up design**: the conversion of the 20 features (decision A).

Rollback is a forward amendment, as for 3.0.0, not `git revert`. Until the first design doc is approved under the new
rules, PR 1 and PR 2 can each be reverted alone.

| Rule or capability today | Carrier after the change | Checked by |
|---|---|---|
| Constitution Check, Complexity Tracking | Required sections of a design doc; approval by the three repository reviewers | R1 (sections exist); the reviewers judge content |
| Task state with evidence; ids | Tasks and evidence blocks in the doc | R4, R7; whether the evidence is true stays with people |
| Append-only D, G, O registers | `research.md` stays in `specs/NNN-slug/`; a written rule | **None.** A check is deferred to the conversion design |
| Contracts that tests and CI read | Unchanged in place | CI contract jobs |
| Pointers | Backticked-path check and a wider scan | `check-agent-config.py` |
| Duplicate-work warning | The hook, reading the number from the branch name | A test that runs the hook as a subprocess |
| Feature numbers | `next-feature-id.py` | R6 |
| Pinned toolchain | Settings entry and accepted hashes | R2 |
| `/speckit-constitution` | An amendment by pull request with a version bump and a Sync Impact Report | R8 |
| `/speckit-specify`, `/speckit-plan` | `egm-draft` | R1 |
| `/speckit-clarify` | `egm-brainstorm` questions and the Open Questions section | None |
| `/speckit-checklist` | **None.** Existing `checklists/` stay in `specs/`; the release-blocking items of feature 036's security checklist (34 open items, reported) are not enforced | None |
| `/speckit-tasks` | The author writes the tasks | R4 |
| `/speckit-analyze` | The approval review. The requirement-to-task coverage check is **deferred** to the conversion design | None for coverage |
| `/speckit-implement` | No skill; the agent builds from the doc and ticks with evidence | R7 |
| `/speckit-converge` | `egm-drift-report` (assessment); appending unbuilt work as tasks is done by the author | None for the appending |
| `/speckit-taskstoissues` | **None.** No replacement exists | None |

## Risks and Mitigations

| Risk | Mitigation | Residual limit |
|---|---|---|
| The check proves that a record is consistent, not that the review ran in a fresh context; `egm-review` is not run in CI | Report hashes in the record; the owner reviews the pull request | Every field of the record can be written by the author |
| Counts are typed by the author | The rule that counts are the highest severity any reviewer assigned; the owner's review | The check sums what the author wrote |
| Evidence blocks are not reviewed text | The grammar (marker, 40-line cap, no heading, table or fence) and a test for smuggled text | A short smuggled sentence inside a block passes |
| The owner gate protects only tasks that the author marked | The repository reviewers look for unmarked tasks | An unmarked task that needed a decision passes |
| An `Approved` doc can be read as permission to build | The owner gate of section 5 | That gate is the only barrier |
| A rebase or squash makes the reviewed commit unreachable | Merge `develop` into branches that hold approved docs; merge commits; full-history checkout | No repository setting enforces the merge method; a rebased branch needs a new review |
| The pin detects drift but cannot reproduce the accepted version | A person reads the diff; the entry records the commit | A developer can run another EGM version |
| Two of the three repository reviewers are written for code diffs | Each agent definition gets a section on what to check in a design doc (plan item 19) | On a design doc the security and correctness legs may find little |
| Enabling the plugin in project settings loads nine agent types into every session | Documented in the Non-Functional Requirements; the owner can move the enabling to user scope | Two of the agents have write tools |
| The id allocator sees only what the last `git fetch` brought | A warning when the fetch is older than a day | Numbers taken on branches never pushed are invisible |
| Removing the tooling breaks other sessions that rebase | No sibling worktree or open pull request existed on 2026-11-03; the pull request is not merged by the assistant | A session that rebases after the merge loses the skills |
| Rollback | A forward amendment | A revert restores stale task state |

## Constitution Check

The author's claim for this design, to be judged by the three repository reviewers. "n/a" means the principle's subject
is not touched.

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | n/a | No runtime state, transaction or scheduling change. |
| II. Modular domain, ports and adapters | Pass | No crate dependency changes; `check-architecture.py` is unchanged (Goal 6). |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect. |
| IV. Compatibility is versioned and pinned | Pass with conditions | The plugin is pinned by content (section 6), but the pin detects drift and does not reproduce a version. |
| V. Code, tenant boundary and secrets are isolated | Pass with conditions | No secret and no runtime path. The plugin adds agent types, two of them with write tools, to every session. |
| VI. Observability and audit are part of the feature | Pass | Approvals and owner acceptances are recorded in the repository (sections 3 and 5). |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | No event or message path. |
| VIII. Failure is tested and contracts are verified, honestly | Pass with conditions | Every new rule has a test seen failing first; unproven guarantees are listed in Risks and Mitigations. |
| Delivery Constraints | Pass | Scripts are Python 3 without dependencies, like the existing ones; EGM is development tooling and not part of the technology baseline. |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| `check-design-docs.py` with eight rules | The approval record and the rules the layout supplied need a check that can fail | Rules kept as prose (Alternative L) |
| `egm-tree-hash.py` and the accepted-hash list | The plugin is unpinned and its behavior lives in many files | Record only the version (Alternative I) |
| `next-feature-id.py` | Numbers clashed once and the manual look is unreliable | Manual look, or a duplicate check only (Alternative N) |
| The `Owner-gate:` line and its check | Owner decisions must gate work without a second approval state | "Stop and ask" or a review finding (Alternative J) |
| The three repository reviewers in the approval rule | EGM's lens agents do not cover compliance, security or correctness | Review outside approval (Alternative M) |
| A status block and a diff against the reviewed commit | The approval record and the ticks must coexist | A ledger, a file per doc, or a normalized hash (Alternatives G and H) |

## Alternatives Considered

The owner chose the option stated in each group; the accepted trade-off is in the rejection field.

#### Alternative A: Keep Spec Kit

*   **Pros**: Nothing to migrate; about 374 source files cite `FR/SC/T` ids (reported).
*   **Cons**: The owner no longer wants it, and no defect of Spec Kit was named.
*   **Reason for Rejection**: The owner decided on 2026-11-02 to use only EGM.

#### Alternative B: Delete `specs/` and move everything to `docs/designs/`

*   **Pros**: One layout.
*   **Cons**: CI and tests read `specs/001-platform-foundation/contracts/` and other files; evidence of closed tasks would be lost.
*   **Reason for Rejection**: It would break the contract tests. The owner chose to remove the tool only.

#### Alternative C: Overlay, EGM for new features only

*   **Pros**: The cheapest; no id moves.
*   **Cons**: Two formats coexist for a long time; a task could be ticked in two places.
*   **Reason for Rejection**: The owner chose to convert the features with open tasks, in the follow-up design (decision A).

#### Alternative D: Convert each feature on a trigger

*   **Pros**: Spreads the cost; a feature never built is never converted.
*   **Cons**: Two formats coexist until the last trigger.
*   **Reason for Rejection**: The owner chose all 20. The sequencing is decision A: after this design and a measured first case.

#### Alternative E: Approval with a second state, or locked while an owner question is open

*   **Pros**: "Approved" keeps meaning that the owner approved the scope.
*   **Cons**: Adds a step the owner did not want; a lock stops docs that carry owner gates by design.
*   **Reason for Rejection**: The owner chose pure automatic approval; the per-task owner gate keeps owner decisions.

#### Alternative F: Procedure outside the constitution

*   **Pros**: A skill rename needs no amendment.
*   **Cons**: An obligation can lose its step without an amendment.
*   **Reason for Rejection**: The owner chose the whole procedure in the constitution.

#### Alternative G: Approval record in a central ledger or a file per doc

*   **Pros**: The doc does not change when approved.
*   **Cons**: A shared ledger conflicts on merge; a per-doc file puts the owner's acceptance elsewhere.
*   **Reason for Rejection**: The owner chose a status block in the doc.

#### Alternative H: Hash that ignores ticks, or a sibling ledger of ticks

*   **Pros**: A normalized hash needs no git history; a ledger leaves the approved doc untouched.
*   **Cons**: The normalizer can hide text in evidence blocks, as the chosen design can; a ledger splits tasks from the plan.
*   **Reason for Rejection**: The owner chose the diff against the reviewed commit. Its weakness (a rebase hides the commit) is handled by the rebase convention of section 4, and a normalized-hash fallback was dropped because the status block makes it unable to match (decision F).

#### Alternative I: Other ways to pin EGM

*   **Pros**: Recording only the version is the cheapest; a fork with a `sha` makes Claude Code enforce a commit; a vendored copy is reproducible; user-scope enabling keeps the plugin's agents out of every session.
*   **Cons**: The version string does not tell two commits apart; the upstream tag is 14 commits behind; a fork must be maintained; project settings cannot require a commit.
*   **Reason for Rejection**: The owner chose the settings entry plus accepted hashes, accepting that project scope loads nine agent types into every session.

#### Alternative J: Owner decisions by "stop and ask", or by a review finding

*   **Pros**: Costs nothing; matches today's practice.
*   **Cons**: Depends on the agent noticing the question (how feature 016 was built twice); the readiness rule depends on a reviewer's grading.
*   **Reason for Rejection**: The owner chose a gate on the task, checked by the script.

#### Alternative L: No new checks, rules kept as prose

*   **Pros**: No new code.
*   **Cons**: The rules that Spec Kit's layout half-supplied would be unenforced; `AGENTS.md` rule 21 says architecture is enforced by checks, not prose.
*   **Reason for Rejection**: Goal 3 needs a carrier for each rule and the record needs a check that can fail.

#### Alternative M: Conformance, security and correctness review outside approval

*   **Pros**: Approval stays cheap; the agents run on the implementation diff.
*   **Cons**: A design that violates the constitution can be `Approved` and found only after it is built.
*   **Reason for Rejection**: The owner made the three repository reviewers part of approval, at about three more runs per doc.

#### Alternative N: No id allocator, or only a duplicate check

*   **Pros**: No new code; the check catches a clash before merge.
*   **Cons**: The manual look failed once; a CI check finds a clash only when the second pull request opens.
*   **Reason for Rejection**: The owner chose to ship the allocator; decision I limits it to the next number.

## Non-Functional Requirements

*   **Security and supply chain.** EGM is a third-party plugin that runs with the permissions of the person using it, and
    its reviewers read files in the repository. Enabling it in the committed `.claude/settings.json` loads nine agent
    types into every contributor's session: seven read-only lens agents (`Read`, `Grep`, `Glob`), the synthesizer
    (`Agent`, `Read`, `Grep`, `Glob`, `Write`) and the bootstrap orchestrator (`Agent`, `Bash`, `Read`, `Grep`, `Glob`,
    `Write`, `Edit`). `AGENTS.md` rule 20 treats plugins as supply chain. The design adds no secret. It adds one
    `permissions.allow` entry for `python3 scripts/check-design-docs.py`, in the style of the existing entries for the
    other check scripts; it is a small widening, and the owner may drop it and accept a prompt on each run. Design docs
    must hold no secret, because reviewers read them. Reviewer output is data and never instructions.
*   **Rollout and rollback.** Section 9. The hook change reads only the branch name.
*   **Cost.** A **reviewer run** is one reviewer agent started. One pass of the five `egm-review` stages starts
    1 + 1 + 3 + 1 + 3 = 9 reviewer runs. Stages 7 and 9 loop until two consecutive rounds are clean, so a converged review
    starts at least 15 and, at the cap of five rounds, up to 33, before synthesizer and resolve runs (derived from the
    skill text; none is measured). The approval rule adds a complete run after the last fix and three repository-reviewer
    runs per complete run. The first review round of the earlier version of this design produced 104 raw findings. The
    cost of the follow-up conversion is for that design to estimate.

### Additional Resources

*   `.specify/memory/constitution.md` (version 3.0.0) and
    [`docs/governance/constitution-3.0.0-sync-impact.md`](../governance/constitution-3.0.0-sync-impact.md).
*   [`docs/engineering/sdd-workflow.md`](../engineering/sdd-workflow.md), [`AGENTS.md`](../../AGENTS.md),
    [`specs/AGENTS.md`](../../specs/AGENTS.md).
*   `tasks/reviews/2026-11-02-egm-constitution/concepts_brief.md` and the review reports of the earlier version under
    `docs/designs/constitution-4.0.0-egm.reviews/2026-11-02/`.
*   EGM `skills/egm-review/SKILL.md` and `skills/egm-draft/references/design_doc.md` (plugin commit `f0ba151b`).

### Future Improvements

*   **The conversion design** (`docs/designs/spec-conversion.md`): requirement-to-task coverage, fidelity of moved tasks,
    splitting large docs, the append-only check on `research.md`, and a table of reserved `D`, `G`, `O` and ADR ranges.
*   **One generated index** from the status blocks, in place of `SPECIFICATION-INDEX.md`, `docs/roadmap.md` and
    `docs/traceability.md`, which have drifted.
*   **A review in CI**, if EGM adds a way to run reviewers headless.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Nothing below is built. Line numbers are those of `develop` at commit `df59eb0e`. All paths are relative to the
repository root. Every new script is Python 3 without dependencies, has a test file in `scripts/tests/` in the style of
`test_check_architecture.py`, and each of its rules has a test that is seen failing before the rule is written. Tests
that need git use a temporary directory on which the test runs `git init`, `git add` and `git commit`.

### Target Files to Create/Modify/Delete

#### PR 1. The tooling (additive, no normative text)

##### 1. `scripts/check-design-docs.py` (Create) and `scripts/tests/test_check_design_docs.py` (Create)

*   **Type / Scope**: Script with the rules R1 to R8 of Technical Plan section 3, run as `python3
    scripts/check-design-docs.py [--root DIR]`.
*   **Proposed Changes**: Implement R1 to R8 exactly as written there, with the task-item and evidence-block grammar of
    section 4. R8 is skipped only when `docs/constitution.md` does not exist. Tests: one passing and one failing case
    per rule; for R3, cases for a checkbox-only change, a change inside an evidence block, a changed sentence, a
    commit that is not an ancestor (after an amend), and a missing history (shallow clone). `docs/designs/README.md` and
    `TEMPLATE.md` are skipped by name.

##### 2. `scripts/egm-tree-hash.py` (Create) and `scripts/tests/test_egm_tree_hash.py` (Create)

*   **Type / Scope**: Script, run as `python3 scripts/egm-tree-hash.py <plugin-dir>`.
*   **Proposed Changes**: Compute the hash of section 6. Tests: the same tree gives the same hash; a changed byte in a
    hashed file changes it; a changed adapter in `.claude-plugin/agents/` changes it; a file outside the set (`site/`,
    `tests/`, `skills/*/TEST.md`) does not; a missing `.claude-plugin/plugin.json` exits 2.

##### 3. `docs/engineering/egm-accepted-hashes.json` (Create)

*   **Type / Scope**: Accepted entries, with the shape of section 6.
*   **Proposed Changes**: One entry: the output of `egm-tree-hash.py ~/.claude/plugins/cache/egm/egm/0.1.0`, the commit
    `f0ba151b252c9abf1318d9fe857a43265d06de89` and the date. The owner reads and accepts it in PR 1.

##### 4. `.claude/settings.json` (Modify)

*   **Type / Scope**: Project settings for Claude Code.
*   **Proposed Changes**: Add `extraKnownMarketplaces.egm` with `{"source": {"source": "github", "repo":
    "google/elephant-goldfish-model"}, "autoUpdate": false}`, add `enabledPlugins["egm@egm"] = true`, and add the string
    `Bash(python3 scripts/check-design-docs.py)` to `permissions.allow`.

##### 5. `scripts/next-feature-id.py` (Create) and `scripts/tests/test_next_feature_id.py` (Create)

*   **Type / Scope**: Script of section 7.
*   **Proposed Changes**: Implement section 7. Tests: a number taken only on another branch is counted; the result is
    zero-padded to three digits; a stale `FETCH_HEAD` prints the warning and still exits 0.

##### 6. `docs/designs/README.md` (Create) and `docs/designs/TEMPLATE.md` (Create)

*   **Type / Scope**: Layout and template of design docs.
*   **Proposed Changes**: The README states the naming (`NNN-slug.md`, the same `NNN` as `specs/`; unnumbered names for
    cross-cutting designs), the status block of section 3, the `Counts:` line, the reviews directory, the evidence-block
    grammar of section 4, the `Owner-gate:` and `OD-` formats of section 5, and a table that maps each Spec Kit artifact
    (`spec.md`, `plan.md`, `tasks.md`, `research.md`, `contracts/`, `quickstart.md`, `checklists/`) to its place. The
    template extends EGM's template with the status block, Requirements with ids, Constitution Check, Complexity
    Tracking and an "Owner decisions needed" section.

##### 7. `.github/workflows/ci.yml` (Modify)

*   **Type / Scope**: CI workflow, job `architecture` (lines 165 to 176).
*   **Proposed Changes**: Add `with: fetch-depth: 0` to the job's checkout step (the pattern exists at line 106 in job
    `released-upgrade`) and add the step `- run: python3 scripts/check-design-docs.py` after `check-agent-config.py`.
    The `unittest discover` step at line 171 already runs the new tests.

#### PR 2. The constitution and the removal

##### 8. `docs/constitution.md` (Move from `.specify/memory/constitution.md`, then Modify)

*   **Type / Scope**: Normative document.
*   **Proposed Changes**: Commit 1 is a pure `git mv`. Commit 2 applies the dispositions of section 1 and writes these
    clauses (the wording is the implementer's to keep; the obligations are fixed here):
    *   **Development Workflow, "EGM procedure"** (replaces the Spec Kit cycle): each feature is specified by a design doc
        `docs/designs/NNN-slug.md` produced by the six stages of section 2, with the skill names; the doc is `Approved` by
        the rule of section 3 and the repository enforces the record with `scripts/check-design-docs.py`.
    *   **Development Workflow, "Owner decisions"** (new): the clause of section 5.
    *   **Development Workflow, "EGM pin"** (new): reviews run only with an EGM whose hash is on
        `docs/engineering/egm-accepted-hashes.json`; upgrading is a pull request that adds an entry and amends the
        procedure if a step changed.
    *   **Development Workflow, "Read before you change", "Same-delivery rule", "CI gates"** rewritten as in section 1.
    *   **Governance, "Versioning"**: add "A change of the mandated specification process is also MAJOR."
    *   **Governance, "Compliance review"**: "every design doc carries a Constitution Check and a Complexity Tracking
        section", with the existing sentences otherwise unchanged.
    *   Footer: `Version: 4.0.0`, `Last Amended` the merge date, and the link to the Sync Impact Reports corrected for
        the new directory.

##### 9. `docs/governance/constitution-4.0.0-sync-impact.md` (Create)

*   **Type / Scope**: Sync Impact Report in the format of the 3.0.0 report.
*   **Proposed Changes**: The disposition table of section 1; the added units; the dependents (the files of this plan);
    the risks of Risks and Mitigations; the rollback.

##### 10. `docs/governance/sync-impact-rust-sdk-2026-10-27.md` (Move from `.specify/memory/sync-impact-rust-sdk-2026-10-27.md`)

*   **Type / Scope**: Historical record of the 1.0.0 to 1.1.0 amendment.
*   **Proposed Changes**: `git mv` only.

##### 11. `docs/governance/constitution-3.0.0-sync-impact.md` (Modify)

*   **Type / Scope**: Historical record.
*   **Proposed Changes**: Add one line near the top: "The constitution moved to `docs/constitution.md` in 4.0.0; paths below
    name its location at 3.0.0."

##### 12. `docs/owner-decisions.md` (Modify)

*   **Type / Scope**: Register of owner decisions.
*   **Proposed Changes**: Append at the end of the file (not at the top, to avoid a conflict with the open feature-035
    branch, which edits the top) a section `## 2026-11-03 — EGM replaces Spec Kit` with a table whose first column is the
    id. Rows `OD-2026-11-03-1` to `OD-2026-11-03-12` hold the owner decisions of 2026-11-02 and the decisions A to L of
    this design, each with a status that begins "Decided".

##### 13. `AGENTS.md` (Modify)

*   **Type / Scope**: Agent rules, shared by all agents.
*   **Proposed Changes**: Line 3: delete "(Spec Kit)". Lines 7 and 16 (rule 1): name `docs/constitution.md` and "the
    design doc `docs/designs/NNN-slug.md` of the active feature (the number is in the branch name)" in place of the
    `spec.md`, `plan.md`, `tasks.md` and `.specify/...` paths. Line 9: replace the cycle with "Design cycle:
    `egm-brainstorm` → `egm-draft` → `egm-review`; the steps need a harness that can start subagents, and approval needs
    Claude Code because it runs the three repository reviewers; run `claude plugin install egm@egm --scope project`
    once." Line 11: "A task in the design doc is ticked only with the evidence that closes it". Line 43 (Commands): add `python3
    scripts/check-design-docs.py`. The "only the owner decides" bullet ends with "(`docs/constitution.md`, Owner
    decisions)". Rule 2: "approved dependency" becomes "a dependency recorded in the design doc". The rule numbers do
    not change and the file stays within its line budget.

##### 14. `CLAUDE.md` (Modify)

*   **Type / Scope**: Claude Code specifics.
*   **Proposed Changes**: Line 7: replace the `/speckit-*` description with "EGM skills are marked manual-only, so Claude
    Code's `Skill` tool does not start them: read the skill's `SKILL.md` and follow it in the same session, or start it
    with `/egm:<skill-name>` (for example `/egm:egm-draft`)"; keep `/verify`, `/adr`, `/migration`, `/parallel-agents`.
    Line 10: "re-read the active design doc" in place of `tasks.md`.

##### 15. `README.md` (Modify)

*   **Type / Scope**: Project overview.
*   **Proposed Changes**: Line 677: describe `specs/NNN-slug/` as contracts, evidence and registers, and add a line for
    `docs/designs/`. Line 681: `docs/constitution.md`. Lines 721-722: delete the sentences that name Spec Kit and
    `.specify/feature.json`, and point to the design cycle in `AGENTS.md`.

##### 16. `SPECIFICATION-INDEX.md` (Modify)

*   **Type / Scope**: Feature inventory.
*   **Proposed Changes**: Line 71: link `docs/constitution.md`. Add one paragraph: new features are described in
    `docs/designs/`; existing features keep `specs/NNN-slug/` until the conversion design.

##### 17. `specs/AGENTS.md` and `specs/README.md` (Modify)

*   **Type / Scope**: Rules for the `specs/` directory.
*   **Proposed Changes**: `specs/AGENTS.md` lines 3, 5 and 9: delete "produced by Spec Kit" and the sentence on generated
    skills and templates, and state that `contracts/`, `evidence/` and the registers stay live and that existing
    `spec.md`, `plan.md` and `tasks.md` are ticked by hand until converted. `specs/README.md` line 5: delete "within the
    Spec Kit feature tree".

##### 18. `docs/engineering/sdd-workflow.md`, `docs/engineering/README.md`, `docs/engineering/ai-engineering.md` (Modify)

*   **Type / Scope**: Engineering guides.
*   **Proposed Changes**: `sdd-workflow.md`: retitle for design docs; replace the nine-step table with a pointer to the
    constitution's procedure; keep the eight "what the repository adds" items with `tasks.md` read as "the design doc's
    tasks"; adapt the sections on writing a spec and on changing something that exists; replace the anti-pattern about
    generated skills with "ticking a task of a superseded file"; name no Spec Kit. `README.md` lines 3 and 7: the new
    constitution path and the guide's new description. `ai-engineering.md`: add the EGM pin to its plugin paragraph and
    delete the Spec Kit reference on line 63.

##### 19. `.claude/agents/architecture-reviewer.md`, `security-reviewer.md`, `correctness-reviewer.md` (Modify)

*   **Type / Scope**: Read-only reviewers.
*   **Proposed Changes**: Line 10 of each: read `docs/constitution.md` and, when the target is a design doc, the doc
    instead of `.specify/memory/...`, `spec.md`, `plan.md` and `tasks.md`. Add a section "When the target is a design
    doc": architecture checks the Constitution Check rows and the layering claims; security checks trust boundaries,
    secrets and supply-chain claims in the doc; correctness checks that each rule is implementable as written and that
    the stated failure behavior is complete. Each report ends with the line `Counts: Blocker <n>, Major <n>, Nit <n>`,
    with the severity map high to Blocker, medium to Major, low to Nit, and a correctness finding of stated confidence
    "certain" or "likely" to Major and "plausible" to Nit.

##### 20. `.claude/skills/verify/SKILL.md` and `.claude/skills/parallel-agents/SKILL.md` (Modify)

*   **Type / Scope**: Project skills.
*   **Proposed Changes**: `verify` line 28 and `parallel-agents` lines 13 and 19: "the design doc's tasks" in place of
    `tasks.md`/`spec`/`plan`; `verify` also runs `python3 scripts/check-design-docs.py`.

##### 21. `.github/pull_request_template.md` (Modify)

*   **Type / Scope**: Pull request template.
*   **Proposed Changes**: Line 3: link the design doc `docs/designs/NNN-slug.md`. Line 19: "design doc (tasks ticked with
    evidence)". Line 25: "Constitution Check still holds, or the violation is justified in the design doc's Complexity
    Tracking".

##### 22. `tests/Cargo.toml` (Modify)

*   **Type / Scope**: Cargo manifest of the test crate.
*   **Proposed Changes**: Line 3: "paths follow `docs/designs/*.md`" in place of `specs/*/plan.md`.

##### 23. `specs/014-workflow-primitives/quickstart.md` and `specs/016-payload-classification/quickstart.md` (Modify)

*   **Type / Scope**: Quickstarts that call `.specify/scripts/bash/check-prerequisites.sh` on a command that spans two lines
    (lines 23 and 46), with prose around it that describes the call.
*   **Proposed Changes**: Replace the whole command and the sentence that describes it with a plain `ls` of the feature
    directory. The `analysis.md` files of 014 and 016 repeat the call as a dated record and are not edited. The 016
    quickstart is parsed by `tests/e2e/payload-classification/quickstart.rs`; that test runs after the edit.

##### 24. `.claude/hooks/session-start.py` (Modify) and `scripts/tests/test_session_start_hook.py` (Create)

*   **Type / Scope**: SessionStart hook and its first test.
*   **Proposed Changes**: Drop the `.specify/feature.json` read. When the branch name matches `^feature/(\d{3})-`, print the
    existing "Other branches mention NNN" warning for the number. Tests run the script as a subprocess in a temporary git
    repository: a numbered feature branch with a second branch that mentions the number, a branch with no number, and a
    detached HEAD.

##### 25. `scripts/check-agent-config.py` (Modify), `scripts/tests/test_check_agent_config.py` (Modify) and `scripts/paths-allowlist.txt` (Create)

*   **Type / Scope**: Existing checker, its tests, and the exceptions file.
*   **Proposed Changes**: Add the backticked-path check and the scoped search of section 8. A path token is a backticked
    string with no space that contains a `/` or ends in `.md`, `.py`, `.rs`, `.toml`, `.json`, `.yaml`, `.yml` or `.sh`,
    and contains none of `<`, `>`, `*`, `{`, `NNN`, `...`, and does not begin with `http`, `-`, `$`, `~` or `origin/`. The
    allowlist has one path or `fnmatch` pattern per line and `#` comments. Its first content is the list of tokens
    that still fail after the pointer fixes of this pull request, each with a reason on the line above; the implementer
    produces it by running the check. The search excludes `scripts/check-agent-config.py`,
    `scripts/tests/test_check_agent_config.py` and `scripts/paths-allowlist.txt`. Tests build a temporary git repository
    and cover: an existing path, a missing path, a placeholder, an allowlisted path, a directory prefix, and a file that
    names `speckit` on a live surface.

##### 26. `.specify/` (Delete, 21 tracked files) and `.claude/skills/speckit-*/SKILL.md` (Delete, 10 files)

*   **Type / Scope**: Spec Kit state, scripts, templates, workflows, integration manifests, and the generated skills.
*   **Proposed Changes**: Delete `.specify/.gitignore`, `init-options.json`, `integration.json`,
    `integrations/claude.manifest.json`, `integrations/speckit.manifest.json`, `memory/.constitution-template.json`,
    `scripts/bash/{check-prerequisites,common,create-new-feature,resolve-template,setup-plan,setup-tasks}.sh`,
    `templates/{checklist,constitution,plan,spec,tasks}-template.md`, `workflows/speckit/workflow.yml` and
    `workflows/workflow-registry.json` (`memory/constitution.md` and `memory/sync-impact-rust-sdk-2026-10-27.md` are already
    moved), and the `SKILL.md` of `speckit-analyze`, `-checklist`, `-clarify`, `-constitution`, `-converge`, `-implement`,
    `-plan`, `-specify`, `-tasks` and `-taskstoissues`. The ignored `.specify/feature.json` stays as an untracked file in
    checkouts that have it.

#### Files that are read and not changed

*   `specs/NNN-slug/{contracts,evidence}/`, `research.md`, `data-model.md`, `quickstart.md` (except the two above),
    `checklists/` and `validation-report.md`; and the existing `spec.md`, `plan.md` and `tasks.md` of every feature.
*   `docs/adr/ADR-0071-*.md`, the `/speckit-*` mentions in `docs/owner-decisions.md`, `docs/governance/` and
    `specs/001-platform-foundation/research.md`: dated records outside the scoped search.
*   `tasks/reviews/`: earlier review records.

### Verification Plan

*   **Unit tests**: `python3 -m unittest discover -s scripts/tests -v` passes, including the new tests; each rule's failing
    case is seen failing before the rule is written.
*   **Static checks**: `python3 scripts/check-architecture.py`, `python3 scripts/check-agent-config.py` and `python3
    scripts/check-design-docs.py` exit 0 on the merged tree of PR 2.
*   **Quickstart test**: `cargo test -p sagat-tests --test e2e_payload_classification` for the edited 016 quickstart, with
    a private `CARGO_TARGET_DIR`.
*   **Journeys**: CUJ 1 to 4 are run once each on a scratch branch. For CUJ 2, edit a sentence of an approved doc and see
    the check fail. For CUJ 4, record a review with an unlisted hash and see the check fail.
*   **Independent read**: the three repository reviewers read the diff of PR 1 and PR 2 together with this design.

--------------------------------------------------------------------------------

*Drafted with EGM*
