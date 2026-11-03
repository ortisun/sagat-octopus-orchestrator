# Constitution 4.0.0: EGM as the only specification workflow

*   **Author:** Marcelo Ortiz de Santana (owner)
*   **Status:** Draft
*   **Date:** 2026-11-02

--------------------------------------------------------------------------------

## Overview

Sagat Octopus Orchestrator is a durable workflow platform written in Rust. Its constitution (version 3.0.0, ratified
2026-10-11, last amended 2026-11-01) does two jobs. It states eight principles that every feature must meet (durable
state, modular domain, at-least-once effects, pinned compatibility, isolation, observability, redeliverable events,
honest tests). It also names Spec Kit as the way features are specified: the Development Workflow section lists the
commands from `/speckit-specify` to `/speckit-converge`, and the Governance section requires a Constitution Check in
every `plan.md`.

The owner has decided to stop using Spec Kit and to use only the Elephant-Goldfish Model (EGM). In EGM, a design doc is
drafted section by section with the owner and then reviewed by agents that have no conversation history.

This design replaces Spec Kit with EGM across the repository. It does eight things:

1.  The constitution moves to `docs/constitution.md` as version 4.0.0. All eight principles stay in force. Only the
    process parts of Development Workflow and Governance are rewritten.
2.  A rule states when a design doc counts as approved, and a script checks the record of that approval.
3.  A task that depends on an owner decision carries a prerequisite that a script checks before the task is ticked.
4.  The EGM plugin is pinned by a hash of the files that define its behavior.
5.  A script allocates feature numbers, so two branches cannot take the same one.
6.  The 20 features that have open tasks are converted into EGM design docs.
7.  New checks enforce the rules that the Spec Kit file layout supported without anyone writing them down.
8.  The Spec Kit tooling is removed, after its replacements exist.

Nothing described here exists yet unless it is marked "today". The Status line reads `Draft` because nothing is built.

## Terms

*   **EGM**: the Elephant-Goldfish Model, a Google open-source plugin for design documents. The **Elephant** is the
    agent that holds project context and drafts. A **Goldfish** is a reviewer agent started with no conversation
    history.
*   **EGM skills**: `egm-brainstorm` (writes a concepts brief), `egm-draft` (writes a design doc section by section),
    `egm-review` (runs five review stages), `egm-review-loop` (repeats a review round until it is clean),
    `egm-resolve` (classifies findings and applies approved fixes) and `egm-drift-report` (compares code with a doc).
*   **Lens agents**: EGM's seven reviewer agents (zoom-out, comprehension, design, structural, style, implementation
    readiness, codebase details). **Repository reviewers**: this repository's three read-only agents,
    `architecture-reviewer`, `security-reviewer` and `correctness-reviewer`. **Brainstorm panel**: the six lens agents
    that `egm-brainstorm` starts.
*   **Harness**: the coding-agent environment that runs a model (Claude Code, Codex, Antigravity, Claude Cowork,
    ChatGPT Work).
*   **Readiness standard**: EGM's rule that an implementation plan fails if the implementer must still make a design
    choice. **ESCALATE**: the `egm-resolve` category for a finding whose evidence is outside every file the reviewer
    could read; it goes to the owner.
*   **Spec Kit**: the specification toolkit used so far. It generates, per feature, `specs/NNN-slug/spec.md` (what and
    why), `plan.md` (how, with the Constitution Check), `tasks.md` (the task list) and supporting files, through the
    `/speckit-*` skills.
*   **FR, SC, T ids**: `FR-NNN-xxx` is a functional requirement, `SC-NNN-xxx` a success criterion and `TNNN-xxx` a
    task, where `NNN` is the feature number. **D, G, O**: decision, gap and owner-question rows in a feature's
    `research.md`. **ADR**: an architecture decision record under `docs/adr/`.
*   **Constitution Check**: a table in a plan or design doc that states, for each principle, whether the design meets
    it. **Complexity Tracking**: a table that justifies each violation or added moving part with the simpler
    alternative that was rejected. **Delivery Constraints**: the constitution section on technology baseline,
    out-of-scope items, targets and security defaults. **Sync Impact Report**: the document that lists, for an
    amendment, what each clause became.
*   **Status block**: the header lines of a design doc that record its approval. **Approval record**: the data in that
    block. **Evidence block**: the text that proves a task is done (defined in Technical Plan section 4).
    **Owner gate**: only the `Owner-gate:` prerequisite of a task (section 5). **Section approval**: the owner's checkpoint while a
    doc is drafted. **CI check**: a step of the CI workflow. **The check**: `scripts/check-design-docs.py`.
*   **Task**: one Markdown checkbox line whose text begins with an id. **Author**: the person or agent that writes or
    edits a doc.
*   **Claude Code plugin settings**: `extraKnownMarketplaces` registers a plugin source, `enabledPlugins` turns a plugin
    on, `ref` names a branch or tag, `sha` a commit, and `autoUpdate` controls updates.

## The Problem

Removing the Spec Kit files is mechanical. The rules that depend on its layout are the risk. The constitution and the
repository rely on Spec Kit's file layout for several rules, and EGM supplies none of them.

What the repository relies on today. Sources are named; "reported" means a reviewer found it and the author did not
re-read the source:

*   **A compliance gate.** Governance requires each `plan.md` to carry a Constitution Check and a Complexity
    Tracking table (`.specify/memory/constitution.md`, Governance). A reviewer counted 32 plans with a Check.
    Feature 035's Check is a gate that is still open: in `specs/035-iceberg-write-steps/plan.md` line 115 the
    technology-baseline row reads "Requires ADR" before design and "Pass pending ADR-0071 acceptance" after it.
*   **Task state with evidence.** `AGENTS.md` says a task is ticked only with the evidence that closes it. The ids
    `FR-NNN-xxx`, `SC-NNN-xxx` and `TNNN-xxx` are feature-scoped and are cited in commit footers. EGM's design template
    has no place for ids, ticks or evidence.
*   **Append-only registers.** `research.md` holds decisions (`D`), gaps (`G`) and owner questions (`O`) that are never
    renumbered. The EGM template tells the author to remove answered questions.
*   **Contracts that tests and CI read.** CI lints `specs/001-platform-foundation/contracts/openapi.yaml`,
    `worker.proto`, `events.asyncapi.yaml` and `workflow.schema.json` ([`ci.yml`](../../.github/workflows/ci.yml), lines
    320-341). [`.cargo/audit.toml`](../../.cargo/audit.toml) requires each advisory ignore to be justified in
    `specs/001-platform-foundation/research.md`.
*   **Pointers that nothing checks.** [`scripts/check-agent-config.py`](../../scripts/check-agent-config.py) enforces
    line budgets, the `CLAUDE.md` import, skill and agent front matter, reviewer tool lists, hook existence and the
    ADR format, and it checks relative Markdown links only in `AGENTS.md`, `CLAUDE.md`, `docs/engineering/*.md`,
    `docs/adr/README.md` and nested `AGENTS.md` files. It does not check backticked paths, and it does not scan
    `README.md`, `SPECIFICATION-INDEX.md`, `specs/README.md` or `.claude/`, so those files can point at a deleted file
    and the check still passes.
*   **A duplicate-work warning.** The session-start hook reads `.specify/feature.json` and warns when other branches
    mention the same feature. It was added after feature 016 was built twice (reported; the hook was not re-read).
*   **Feature numbers.** Spec Kit's `create-new-feature.sh` takes the highest number in the local `specs/` directory
    and adds one, so it cannot see numbers taken on other branches. `AGENTS.md` ("Start here") says to look at other
    open branches before taking a number, "a clash happened", and numbers 013 and 015 live on other branches.
*   **A pinned toolchain.** Spec Kit was pinned: `.specify/init-options.json` names its version and its integration
    manifest hashes the ten skills (reported). EGM is a per-user plugin, at version 0.1.0 today, that is not declared
    anywhere in the repository, and `docs/engineering/ai-engineering.md` says plugins are supply chain.

EGM also has no verdict. Its review skill allows one re-audit per stage, then records the remaining findings as
"accepted trade-offs" and moves on (`skills/egm-review/SKILL.md`, the "1-Audit Convergence Cap"). One EGM review of a
prior document in this repository (`tasks/reviews/2026-10-30-open-tasks/`) ended with 0 Blocker and 6 Major findings and
every action still open. A rule that says "approved when the review is clean" is only as strict as its definition of
clean.

Scale: the repository has 35 feature directories under `specs/`. Twenty have open tasks, with 1,268 open tasks; 678
tasks are ticked overall (counted on 2026-11-02 at commit `df59eb0e`).

## Goals

Each goal can be checked.

1.  **The constitution keeps every obligation.** `docs/constitution.md` exists as version 4.0.0. A **unit** of version
    3.0.0 is one heading or one bullet of its text, and an **obligation** is a statement in it that uses MUST or
    MUST NOT. The Sync Impact Report gives every unit one disposition from KEEP, MOVE, REWRITE, DROP or ADD. All eight
    principles are KEEP, so no principle text loses an obligation. A REWRITE or DROP appears only in the process
    parts of Development Workflow and Governance, and each one says where its obligation went.
2.  **Approval is one rule, and a script checks its record.** A design doc is approved when `egm-review` and the three
    repository reviewers each end with no Blocker or Major finding open. A finding that is contested or left unfixed
    counts as settled only when the owner accepts it in writing. An escalation, or a reviewer that did not run,
    keeps the doc not approved. The approval record states the reviewed commit, the EGM hash and the report paths.
3.  **Every rule the layout supplied has a named carrier.** Technical Plan section 10 gives, for each item in "The
    Problem" and for each Spec Kit skill, the file, script or written rule that holds it, or says that none does. The
    Constitution Check and Complexity Tracking become required sections of a design doc.
4.  **Removal leaves no dangling pointer.** The `.specify/` directory and the ten `speckit-*` skills are deleted only
    after the hook, the checks and the re-pointed documents are in place. After deletion, `check-agent-config.py` and
    its new backticked-path check pass, and a case-insensitive search for `speckit` and `Spec Kit` finds nothing on
    the live instruction surfaces: `AGENTS.md`, `CLAUDE.md`, `README.md`, `.claude/`, `.github/`, `scripts/`,
    `docs/engineering/` and `docs/constitution.md`. Historical records (`specs/`, `tasks/reviews/`, `docs/adr/`,
    `docs/governance/`, `docs/designs/`, `docs/owner-decisions.md`) may name Spec Kit as a dated fact.
5.  **The conversion keeps every task id and the counts of ticked and open tasks.** Each of the 20 features is
    converted in its own pull request. The converted doc keeps every task with its original id, and a script confirms
    that the ids and the counts of ticked and open tasks equal those of the frozen source. The old `spec.md`, `plan.md`
    and `tasks.md` are marked superseded, and a check fails if their bodies change afterwards.
6.  **Nothing that tests or CI read moves.** The contract files, the evidence directories and the research registers
    stay where they are and stay append-only.
7.  **The architecture check still passes unchanged.** `scripts/check-architecture.py` checks crate layering and
    dependency direction. This change touches no crate dependency, so the script and its negative tests pass without
    edits. This is the owner's measure of success: the project's architecture is kept.
8.  **The EGM plugin is pinned by content.** A review record that carries a hash not on the accepted list fails the
    check.
9.  **Feature numbers do not clash.** `scripts/next-feature-id.py` prints the next free feature number, and the check
    fails if two docs or a doc and a `specs/` directory share a number.
10. **A task that needs an owner decision cannot be ticked without it.** The check fails a ticked task whose
    `Owner-gate:` entry does not exist.

## Non-Goals

*   Changing the substance of any of the eight principles.
*   Deleting or moving `specs/001-platform-foundation/contracts/` or any file that a test or CI job reads.
*   Converting the 15 features that have no open task. They stay as history.
*   Letting an agent decide product, scope, release or security-exception questions. These stay with the owner.
*   Running `egm-review` in CI. The harnesses EGM supports do not include CI.
*   Renumbering the rules of `AGENTS.md`. About 24 files cite them by number (reported).
*   Building a replacement for Spec Kit's task generator. A design doc's tasks are written by the author.
*   Changing any runtime code, schema, contract or ADR of the platform.

## Open Questions

Each question below is open. The owner's decisions on them are not applied in this design.

*   **Q1: Where is the "broader reviewed Constitution candidate"?** The 3.0.0 Sync Impact Report
    (`docs/governance/constitution-3.0.0-sync-impact.md`, "The broader reviewed Constitution candidate is not adopted")
    says it was not adopted and must be rebased before later use, and lists "authority hierarchy and plugin rules"
    among its parts. No file or branch with that name was found. This design leaves it out. Where does it live, or is
    it dropped? Its plugin rules overlap with Technical Plan section 6.
*   **Q2: Should `CODEOWNERS` and a branch-protection rule protect the files that carry authority?** Those files are
    `docs/constitution.md`, `docs/owner-decisions.md`, `docs/engineering/egm-accepted-hashes.json`,
    `.claude/settings.json` and `.claude/agents/`. The repository has no `CODEOWNERS` file. If not, the design records
    "no technical protection; review by the owner is by convention" as an assumption (review round 1, Issue 5).
*   **Q3: Should the status block carry a document-level `Scope-approval: <owner-decisions ref> | none` field** that
    fails any tick while it is `none`? It differs from Alternative E because the review status does not change, and
    it was not weighed as an alternative (review round 1, Issue 9).
*   **Q4: When is the Spec Kit tooling removed?** The design removes it after the last conversion (PR 23). It could be
    removed right after the constitution pull request and the sibling-branch rebases, which drops the transitional
    clause in Technical Plan section 9 (review round 1, Issue 14).
*   **Q5: Is the version 4.0.0 or 3.1.0?** By the versioning clause as written, adding sections is MINOR (3.1.0). A
    MAJOR bump needs an amendment to the clause so that a change to the mandated process is MAJOR (review round 1,
    Issue 36).

### Owner confirmations

These record decisions already taken in the text. The owner confirms them.

*   **Security exceptions need no register of their own.** This design uses the homes that exist today: entries in
    `docs/owner-decisions.md`, and the gap register in `specs/001-platform-foundation/research.md` that
    `.cargo/audit.toml` already points to.
*   **Recommendations.** Outside an EGM session, the repository practice stays: ask the owner once, with the options
    and a recommendation (`docs/engineering/sdd-workflow.md`, item 4). Inside an EGM brainstorm or draft, the skill
    forbids recommendations and defaults, and that rule applies there.
*   **Review reports live next to the doc,** in `docs/designs/NNN-slug.reviews/YYYY-MM-DD/`, because the approval
    record points at them. The earlier EGM runs in this repository used `tasks/reviews/`.

### Contested findings awaiting the owner's written acceptance

The first review round contested four findings by citing decisions recorded in this document. A contested finding
counts as settled only when the owner accepts the contest in writing (`goldfish_review_report.md`, Issues 31, 32, 34
and 35):

*   the conversion wave is larger than the root problem needs (Alternatives C and D);
*   "Approved" should stay `Proposed` until the owner approves the scope (Alternative E);
*   the id allocator goes beyond what Spec Kit supplied (Alternative N);
*   the reviewed-commit diff should give way to a hash of the normalized body (Alternative H).

## Critical User Journeys (CUJs)

### CUJ 1: Design a new feature

*   **Given**: An idea for a feature that does not exist yet, on a branch cut from `develop`, and the number printed by
    `scripts/next-feature-id.py`.
*   **When**: The author runs `egm-brainstorm`, then `egm-draft`, saves the draft as `docs/designs/NNN-slug.md`, and
    runs `egm-review` and the architecture, security and correctness reviewers until none has a Blocker or Major
    finding open.
*   **Then**: The status block says `Approved` and holds the reviewed commit, the EGM hash, the report paths and the
    finding counts. `scripts/check-design-docs.py` passes. No separate approval step happened.

### CUJ 2: Tick a task after approval

*   **Given**: An approved design doc with an unticked task `T018-012`.
*   **When**: The author ticks the box and adds an evidence block (Technical Plan section 4) with the test name and
    its result.
*   **Then**: `check-design-docs.py` passes, because only a checkbox and an evidence block changed since the reviewed
    commit. If the author also edits a sentence of the design, the check fails until the changed lines are reviewed
    again.

### CUJ 3: A task that needs an owner decision

*   **Given**: A task that depends on a product, scope, release or security-exception decision and carries an
    `Owner-gate:` line that names the entry it needs.
*   **When**: An agent ticks the task before the owner has recorded that entry in `docs/owner-decisions.md`.
*   **Then**: `check-design-docs.py` fails. After the owner records the entry, the same change passes. The check does
    not detect that a task was started.

### CUJ 4: Upgrade the EGM plugin

*   **Given**: A list of accepted EGM hashes in the repository and a new EGM release installed on a developer machine.
*   **When**: A review is run with the new release and its record is committed.
*   **Then**: `check-design-docs.py` fails because the recorded hash is not on the list. A pull request adds the hash
    after a person reads the diff of the plugin files, and amends the procedure in the constitution if a step changed.

### CUJ 5: Convert an existing feature

*   **Given**: A feature in `specs/NNN-slug/` with open tasks, and no open branch that still ticks its `tasks.md`.
*   **When**: A conversion pull request creates `docs/designs/NNN-slug.md`, moves the tasks with their ids and ticks
    into it, and marks the old `spec.md`, `plan.md` and `tasks.md` as superseded.
*   **Then**: `check-design-docs.py --conversion NNN` confirms that the task ids, the counts of ticked and open tasks,
    and the normalized text of each task equal those of the frozen source. A later edit to a superseded file fails the
    check.

## Technical Plan (Architecture)

The plan applies these decisions, all taken by the owner on 2026-11-02 and recorded in
`tasks/reviews/2026-11-02-egm-constitution/concepts_brief.md`: EGM is the only workflow; only the tooling is removed
and `specs/` stays; all eight principles stay whole; the constitution lives at `docs/constitution.md`; approval is
automatic and has one state; "blocking" means Blocker or Major and only the owner accepts a contested finding; the 20
features with open tasks are converted; the record is a status block in the doc; ticks are checked by a diff against
the reviewed commit; the whole EGM procedure lives in the constitution; the pin is a settings entry plus a list of
accepted hashes; owner decisions are gates on tasks; conversions are mapped by an agent; the three repository
reviewers are part of approval; the id allocator ships with the tooling.

### Shape of the change

```text
BEFORE
  .specify/memory/constitution.md --> Spec Kit skills (10) --> specs/NNN-slug/{spec,plan,tasks}.md --> code
        v3.0.0                          /speckit-*               Check in plan.md, ticks in tasks.md

AFTER
  docs/constitution.md --> EGM (brainstorm, draft, review) --> docs/designs/NNN-slug.md --> code
        v4.0.0                                                  status block + Check + plan with tasks
           |                                                          |
           |  scripts/check-design-docs.py (new)                      +-- docs/designs/NNN-slug.reviews/DATE/
           |  scripts/check-agent-config.py (extended)
           |  scripts/next-feature-id.py (new)                    specs/NNN-slug/ keeps contracts/, evidence/,
           |  docs/engineering/egm-accepted-hashes.json (new)     research.md, quickstart.md, checklists/
           +-- .claude/settings.json declares the EGM plugin

PROCEDURE AND APPROVAL CHECK
  idea --> egm-brainstorm --> concepts_brief.md
       --> egm-draft (section approval by the owner) --> docs/designs/NNN-slug.md      Status: Draft
       --> egm-review (stages 5 to 9) + architecture, security and correctness reviewers
                 |  every report ends with a footer: id, reviewer, severity, status
                 v
           Blocker or Major open? --yes--> author fixes, or owner accepts the finding in
                 | no                       docs/owner-decisions.md; run the reviews again
                 v
           author writes the status block                                              Status: Approved
                 v
           CI: check-design-docs.py verifies the block (commit reachable, hashes, report hashes, counts)
                 v
           build: tasks ticked with evidence blocks; only checkbox and evidence changes pass the diff check
```

### 1. The constitution, version 4.0.0

The file moves with `git mv .specify/memory/constitution.md docs/constitution.md` in a commit of its own, so history
follows it. A second commit edits it. Squash merging is not used (section 4 explains why).

The Sync Impact Report (format of `docs/governance/constitution-3.0.0-sync-impact.md`) gives every unit of 3.0.0 one
disposition from KEEP, MOVE, REWRITE, DROP or ADD. "Disposition" is used here so that it is not confused with the
verdict of a review.

| Unit of 3.0.0 (a heading or a bullet) | Disposition | Change |
|---|---|---|
| The file `.specify/memory/constitution.md` | MOVE | To `docs/constitution.md`, by `git mv` in its own commit. |
| Core Principles I to VIII (one heading each) | KEEP | Text unchanged. Only a link or path that moved is corrected. |
| Delivery Constraints | KEEP | Unchanged. |
| Development Workflow: Specification first | KEEP | Unchanged. |
| Development Workflow: Spec Kit cycle | REWRITE | Replaced by the EGM procedure (section 2). Its last sentence, "Features 001–012 already exist; a new feature continues the sequence from 013.", is DROPped: stale, and no obligation is lost. |
| Development Workflow: Identifiers | REWRITE | The sentence about the Spec Kit template is removed. The id scheme is unchanged. |
| Development Workflow: Read before you change | REWRITE | Names the constitution and the design doc of the active feature. |
| Development Workflow: Same-delivery rule | REWRITE | "A checked task in `tasks.md`" becomes "a ticked task in the design doc's Detailed Implementation Plan". |
| Development Workflow: CI gates | KEEP | Gains the design-doc CI check. |
| Development Workflow: Branches and commits | KEEP | Unchanged. |
| Governance: supremacy, ADR divergence, amendments, versioning (one bullet each) | KEEP | Unchanged. |
| Governance: Compliance review | REWRITE | "Every `plan.md` carries a Constitution Check" becomes "every design doc carries a Constitution Check and a Complexity Tracking section". The obligation is the same. |
| Footer: Version line and closing link paragraph | REWRITE | `Version: 4.0.0`, last-amended date, and the link to the Sync Impact Reports. |

Added units, not in 3.0.0:

*   **Approval rule**: ADD (section 3).
*   **Owner decisions clause**: ADD, moved from a bullet of `AGENTS.md` (section 5).
*   **EGM pin rule**: ADD (section 6).

The version number is MAJOR (4.0.0) in this draft because the carrier of the Compliance review obligation changes, as
it did in 2.0.0 and 3.0.0. Open question Q5 asks whether the versioning clause as written supports that. The Sync
Impact Report also lists the dependents (`AGENTS.md` rules 1 and 2 and "Start here", `CLAUDE.md`, the reviewer agents,
the skills, the documents in section 8), the risks and the rollback. The 1.1.0 report now in
`.specify/memory/sync-impact-rust-sdk-2026-10-27.md` moves to `docs/governance/` before `.specify/` is deleted.

### 2. The procedure inside the constitution

The Development Workflow section holds the whole procedure, with the skill names. Its stages:

1.  **Idea**: `egm-brainstorm` writes `concepts_brief.md`. The brief is saved in the review directory of the feature
    (`tasks/reviews/DATE-name/` until the feature has a number, `docs/designs/NNN-slug.reviews/` after) and is
    committed with the design doc.
2.  **Draft**: `egm-draft` writes the design doc, section by section, with the owner's section approval. It is saved
    as `docs/designs/NNN-slug.md`, where `NNN` is the number of the feature. Numbers are shared by all branches, so
    the author runs `scripts/next-feature-id.py` first. It reads `origin/*`, the sibling worktrees and the table of
    reserved ranges (the `D`, `G`, `O` and ADR number blocks that `SPECIFICATION-INDEX.md` assigns to features), and
    prints the next free feature number and the free ranges.
3.  **Review**: `egm-review` (with `egm-review-loop` and `egm-resolve`) runs the lens agents with no conversation
    history, through all five stages (preset 0 or 1). The three repository reviewers also read the doc, because EGM's
    lens agents do not cover constitution compliance, security or correctness. The reports are committed in
    `docs/designs/NNN-slug.reviews/YYYY-MM-DD/`.
4.  **Approve**: by the rule in section 3. No separate approval step exists.
5.  **Build**: code, tests and documents in the same delivery. A task is ticked only with the evidence that closes it.
6.  **Reconcile**: `egm-drift-report` compares the code with the doc after delivery. It reports; the owner decides
    what to change. It covers the assessment half of `/speckit-converge` and not its second half, which appended the
    unbuilt work as new tasks; the author does that by hand (section 10).

The cost of keeping the whole procedure in the constitution is that every change to a step is an amendment with a
version and a Sync Impact Report. Section 6 ties the text to the EGM files it describes. A new EGM release that
changes a step makes the recorded hash unlisted, which fails the check; a person then reads the diff and decides
whether an amendment is needed.

### 3. Approval: rule, record and checks

**Rule.** A design doc is `Approved` when `egm-review` and each of the three repository reviewers
(`architecture-reviewer`, `security-reviewer`, `correctness-reviewer`) end with no Blocker and no Major finding open.
The severity scale is Blocker, Major and Nit. A Nit is in the reports and does not block. A finding that the author
contests, or that is recorded without a fix, counts as settled only when the owner accepts it in writing in
`docs/owner-decisions.md`. An escalation (ESCALATE) or a reviewer that did not run keeps the doc `Draft`. "Open" means
raised in the last complete run of all four reviewers and not owner-accepted, so a final run follows the last fix. A
complete run uses all five stages of `egm-review` (preset 0 or 1), because preset 2 skips the readiness stage and
preset 3 runs only that stage.

No separate approval step exists. The owner acts only to accept a contested or unfixed Blocker or Major finding.
Merging a design-doc pull request is the normal merge under `AGENTS.md` rule 14 and is not an approval act.

The three repository reviewers are Claude Code agents defined in `.claude/agents/`. A session in another harness
cannot run them, so it cannot produce an `Approved` doc. They are pinned by the reviewed commit: their definitions
are files of the repository, so the commit in the record fixes the version that ran.

**Findings footer.** The check needs per-finding data that today's reports do not give: `security-reviewer` reports
high, medium and low, `correctness-reviewer` reports confidence and no severity, `architecture-reviewer` says which
findings block, and EGM's synthesizer normalizes free text. Every report therefore ends with a machine-readable footer
of `id, reviewer, severity, status` per finding. The three repository reviewers' output contract is amended in the
constitution pull request (PR 2), with this map: high to Blocker, medium to Major, low to Nit, and a correctness
finding with stated confidence to Major. Counts in the record use the highest severity any reviewer assigned. A
synthesizer or an author may not lower a Blocker or Major to Nit; EGM's synthesizer rubric did that for nine findings
in the first review round of this design, which is why the rule is needed. An owner acceptance cites
`report-path#finding-id`, and an owner-accepted finding is listed separately and is not counted as open.

**Template.** EGM's own design template has a header with `Status: [Draft | Proposed | In Review | Approved]`, and no
Constitution Check, no Complexity Tracking, no requirement ids and no section for owner decisions. `egm-draft` documents
no way to replace its template. The repository therefore keeps `docs/designs/TEMPLATE.md`, which extends EGM's
template with those sections, and `docs/designs/README.md`, which explains the layout, says that a doc whose name has
no leading number is a cross-cutting design (like this one), maps each Spec Kit artifact to a section of a design doc,
and names the "Owner decisions needed" section. An unnumbered doc is exempt only from `FR/SC/T` ids and the
Requirements section: it carries the status block, a Constitution Check and a Complexity Tracking section. The author
adds the extra sections after the draft is written, and the check requires them.

**Record.** The record is a status block: the header lines at the top of the doc, which extend the template header.
The `Status:` line is the only status line of the doc. The check treats `Proposed` and `In Review` like `Draft`.

```text
*   **Author:** ...
*   **Status:** Approved
*   **Date:** YYYY-MM-DD
*   **Reviewed-commit:** <40 hex characters>
*   **Reviewed-blob:** <git blob id of this doc at the reviewed commit>
*   **Review-preset:** 0 (interactive) or 1 (consolidated); rounds: <n>; stages: 5,6,7,8,9
*   **EGM-hash:** <hash computed by scripts/egm-tree-hash.py>
*   **Claude-Code:** <version>, model <id>
*   **Review-reports:** docs/designs/NNN-slug.reviews/YYYY-MM-DD/consolidated_review_report.md (sha256 <hex>)
*   **Repo-reviewer-reports:** .../{architecture,security,correctness}.md (sha256 <hex> each)
*   **Open findings:** Blocker 0, Major 0 (across all four reports, highest severity assigned by any reviewer)
*   **Owner-accepted:** <report-path>#<finding-id> (docs/owner-decisions.md#YYYY-MM-DD/<n>)
```

The author agent writes the block after the review, so every field can be written by the author (section 10).
Reports are immutable after the pull request that introduces them, and their hashes are in the block.
`scripts/check-design-docs.py` does not trust the block. It checks:

*   the required sections exist, including one Constitution Check row for each of principles I to VIII and for the
    Delivery Constraints, and a Complexity Tracking section;
*   for an `Approved` doc, every field is present, the reviewed commit exists in the history (the check fails when it
    does not; it never skips) or the blob at this path equals `Reviewed-blob`, the EGM hash is on the accepted list
    (section 6), the four reports exist and match their hashes, the counts in the block equal the footers, and each
    owner-accepted finding has a matching entry in `docs/owner-decisions.md`;
*   task ids: the prefix equals the doc number, ids are unique, and an id is defined in one place across all design
    docs and the non-superseded specs;
*   coverage, forward-only: every `FR` and `SC` of a doc converted after the first conversion is cited by at least one
    task, and every task cites at least one requirement. This replaces `/speckit-analyze`'s coverage check. It is
    measured on the first conversion before it applies to the others;
*   the forward-only evidence rule: a task that is newly ticked in the diff against the merge base with the target
    branch carries an evidence block;
*   the constitution checks, once `docs/constitution.md` exists: the version line equals the newest Sync Impact
    Report, the eight principle headings exist, and each "Enforced by" path exists.

The check does not prove that a review ran in a fresh context or that the evidence is true. Those stay with the
people and agents who run them (Risks and Mitigations).

### 4. Task state after approval

Tasks are in the doc's Detailed Implementation Plan. Ticking a task changes the doc after it was reviewed. The record
handles this by the reviewed commit.

**The diff rule.** The script runs a diff from `Reviewed-commit` to the working tree, on pull requests and on pushes
alike. The **body** of a doc is everything below its status block; the status block is excluded from the diff. The
script accepts only two kinds of change in the body: a checkbox going from `[ ]` to `[x]`, and an evidence block. Any
other change makes the doc count as `Draft`. The script exits non-zero and never edits the doc: until the changed
lines are reviewed again, the doc is treated as `Draft`.

**Evidence block.** An evidence block is the text inside a task's list item, after its checkbox line, that starts at a
marker and runs to the end of that item. The markers are `*Done YYYY-MM-DD.*`, `*Partly done YYYY-MM-DD.*` and an
inline `**Evidence (YYYY-MM-DD)**:`. These are the forms used today (`*Done 2026-10-29.* Evidence: ...` in
`specs/018-schedules-bulk-operations/tasks.md`; `**Evidence (2026-11-01)**:` in
`specs/037-compensation-safety/tasks.md`). A block has at most 40 lines (a proposal) and contains no heading, table or code-fence marker. A task with the
marker `Partly done` stays unticked. Example task (an illustration, not a real task):

```text
- [x] T018-012 Add the schedule pause endpoint. Refs: FR-018-007, SC-018-002.
  Owner-gate: docs/owner-decisions.md#2026-10-29/1
  *Done 2026-11-03.* Evidence: `cargo test -p sagat-tests --test integration_schedules pause_endpoint` ->
  3 passed, 0 failed.
```

Tests of the evidence rule cover a multi-line block, design text hidden in a block, a tick with no block, and a
tick whose block is outside its task.

**Amending an approved doc.** A formatting-only change (whitespace and Markdown layout that leave every word
unchanged) keeps `Approved`. Any other change, including a typo, makes the doc `Draft`. The re-review then covers
only the changed line range (`egm-review` accepts a range) and the three repository reviewers on the diff; the block
records the range and the new reviewed commit. The cost of such a re-review is part of the cost model (section 7).

**Reachability.** The rule works only if the reviewed commit stays reachable. A rebase, an amend, a force-push or a
squash or rebase merge can make it unreachable or a different commit. Three measures cover this. Pull requests that
touch design docs are merged with a merge commit (the Gitflow rule in `AGENTS.md` already merges with `--no-ff`; no
repository setting enforces it today, so this is a convention and a recorded risk), and the CI job checks out the full
history. If the commit is no longer an ancestor, the check accepts the doc when the blob at its path equals
`Reviewed-blob`. The tests for these rules use disposable git repositories (rebase, amend and shallow-clone cases),
because `scripts/tests/test_check_architecture.py`, the model for the other tests, uses synthetic JSON.

### 5. Owner decisions: gates on tasks

The constitution gains a clause: product, scope, budget, release and security-exception decisions belong to the owner
and are recorded in `docs/owner-decisions.md`. A design doc's `Approved` status is a statement about the review, not
one of those decisions.

The gate is on the task. A task that depends on such a decision carries a line
`Owner-gate: docs/owner-decisions.md#YYYY-MM-DD/<n>`, where `<n>` is the number of the row under that dated heading.
The script parses the row, requires its Refs column to cite the task or the doc, and requires a decided status. One
entry answers one question; a task that needs several answers lists several gates (feature 035's `T035-001` needs the
ADR-0071 acceptance and three owner questions). A task with an `Owner-gate:` cannot be ticked until its entries
exist, and the check enforces that for every ticked task that carries a gate. The check cannot tell that a task was
started, and it cannot tell who wrote an entry: an agent can write one, and the owner's review of the pull request is
what stops it.

The check sees only gates that the author wrote. A missing gate on a task that needed one passes the check, and the
repository reviewers are asked to look for unmarked tasks (Risks and Mitigations). Open question Q3 asks whether to
invert this default.

### 6. Pinning the EGM

EGM is a per-user plugin today (version 0.1.0, installed commit `f0ba151b`). The upstream tag `v0.1.0` points to commit
`b3a35956`, which is 14 commits and 105 files behind the commit that ran the reviews so far, and both say "0.1.0". A
version string therefore does not identify the content, and `ref` in a settings file could only name that older tag
(reported by comparing the two commits on GitHub on 2026-11-02).

*   `.claude/settings.json` registers the marketplace `google/elephant-goldfish-model` with `autoUpdate` false and
    enables `egm@egm`. Claude Code documents no way to require a version or a commit there. Each person runs
    `claude plugin install egm@egm --scope project` once, and `AGENTS.md` says so. There is no documented way to
    install a chosen commit through project settings; the accepted-list entry records the commit so that a person can
    compare an installed copy with it.
*   **The hash.** `scripts/egm-tree-hash.py` computes one hash, over sorted relative paths and the hash of each file,
    of this set under the plugin install directory: `.claude-plugin/` (the Claude Code manifest and the adapters in
    `.claude-plugin/agents/`, which carry the reviewers' tool limits `tools: [Read, Grep, Glob]` and
    `omitClaudeMd: true`), `agents/`, `resources/`, the `SKILL.md` and `references/` files of every skill, and the
    `docs/` files the skills tell the agent to read (`docs/claude.md` and `docs/harnesses.md`). It excludes `TEST.md`,
    `test_scenarios.yaml`, `assets/`, `site/` and `tests/`. Claude Code skips the root `agents/` folder when the
    manifest lists adapters (`docs/claude.md` of the plugin), so the adapters must be in the set. The script reads the
    install directory (`~/.claude/plugins/cache/egm/egm/<version>`) and never the marketplace clone: on 2026-11-02
    the clone differed from the installed copy in four files under `skills/` and four under `docs/`.
*   `docs/engineering/egm-accepted-hashes.json` lists the accepted entries. Each entry stores the hash, the plugin
    commit and the date accepted. The first entry is the hash of the installed copy at commit `f0ba151b`, which ran the
    reviews of this design. A review record is also required to state the Claude Code version and model, and the
    check enforces a minimum Claude Code version (2.1.271, from the plugin's `docs/claude.md`), because the reviewers'
    isolation depends on it.
*   **What the pin does and does not do.** A review record that carries an unlisted hash fails the check. The check
    does not stop a developer from running another EGM version, and it cannot tell whether a step of the procedure
    changed: a person reads the diff of the behavioral files (the adapters, the personas, the standards, the
    `SKILL.md` files) and decides. CI has no plugin installed, so it compares strings only.
*   Upgrading EGM is a pull request that adds an entry after the owner has read that diff, and that amends the
    procedure in the constitution if a step changed.
*   The settings apply to Claude Code only. The repository reviewers cannot run in another harness, so a review from
    another harness cannot make a doc `Approved`, and the hash of another harness's install is not accepted.
    `AGENTS.md` names the steps that need a harness able to start subagents.

The plugin rules of the unadopted 3.0.0 candidate overlap with this section (see Q1).

### 7. Conversion of the 20 features

The 20 features with open tasks are: 014, 018, 019 to 032, 033, 035, 036 and 037. Each is converted in its own pull
request, after the tooling pull request (PR 1) and the constitution pull request (PR 2) have merged. All 20 are
committed to.

An agent writes each converted doc by mapping `spec.md`, `plan.md` and `tasks.md` onto the design template. The
conversion is not an interactive `egm-draft` session, and the owner is not asked for section approval. `egm-review`
judges the result, and the owner sees only the findings that need the owner's acceptance. Interactive drafting stays
for new features, where there is a design to discuss. A scripted converter with a golden test was not chosen
(Alternative O).

A conversion PR:

*   creates `docs/designs/NNN-slug.md` from `spec.md` (Overview, Problem, Goals, Non-Goals, user stories as CUJs,
    requirements with their ids), `plan.md` (Technical Plan, Constitution Check, Complexity Tracking) and `tasks.md`
    (the Detailed Implementation Plan with every task, its id, tick and evidence block). The tasks are **moved into
    the live doc**; the superseded `tasks.md` stays frozen at commit X, so each id exists in two files and the
    id-uniqueness rule excludes superseded files. A plan with no Check (feature 033's plan has a one-line
    "Constitution:" note and no table) gets a Constitution Check section that says "no Check in the source plan";
    the agent does not author one;
*   leaves `contracts/`, `data-model.md`, `quickstart.md`, `checklists/`, `evidence/`, `research.md` and
    `validation-report.md` in `specs/NNN-slug/`. Tests and CI read some of these, and the registers must stay
    append-only. The design doc links to them;
*   puts a banner on the old `spec.md`, `plan.md` and `tasks.md`: "Superseded by `docs/designs/NNN-slug.md` at commit
    X". The check fails if their bodies change after X;
*   runs `check-design-docs.py --conversion NNN` once, which compares the frozen source with the new doc: the set of
    task ids, the counts of ticked and open tasks, and a hash of the normalized text of each task (only amendments
    listed in the PR are exempt). The comparison is not a permanent CI rule;
*   splits a large doc by increment, as below.

**Splitting.** An **increment** is a group of tasks that the source plan labels as one increment. A doc above the line
ceiling is split into `NNN-slug.md` (the index and the shared requirements) and `NNN-slug.part-K.md` files, one per
increment, in the same pull request. Each part has its own status block; the id prefix is the feature number for all
parts; coverage is computed per feature; the feature-level index lists the parts. The markdown of feature 014 is
14,004 lines in 42 files and cannot be reviewed in one pass. The ceiling is set after PR 3 measures a real conversion
and is then written in `docs/designs/README.md`.

**Order.** Features that have not started (no ticked task) come first, to learn what the conversion costs before it
touches a feature in flight. Features whose branches are open come last (section 9).

A converted doc is reviewed like any other doc. If the review ends clean, the rule of section 3 makes it `Approved`,
even for a feature in specification only (019 to 032 and 036) on which the owner has made no scope decision. The
owner accepted this consequence. The owner gate of section 5 is what keeps unapproved scope from being built: a task
that depends on a scope decision carries its entry.

Sibling branches that still tick an old `tasks.md` after a conversion fail the check on the superseded file. They are
told to move the tick to the design doc. This is a visible failure, not a silent one. Each PR is coordinated with the
branches and worktrees of its feature (today 014, 016, 018, 035, 036 and 037 each have a worktree).

**Cost model.** A **reviewer run** is one reviewer agent started. One pass of the five `egm-review` stages starts
1 + 1 + 3 + 1 + 3 = 9 reviewer runs. Stages 7 and 9 loop until two consecutive rounds are clean, so a converged review
starts at least 1 + 1 + 6 + 1 + 6 = 15, and at the cap of five rounds up to 1 + 1 + 15 + 1 + 15 = 33, before
synthesizer and resolve runs (a reviewer derived these from the skill text; none is measured). Across 20 docs that is
300 to 660 EGM reviewer runs. The three repository reviewers add three runs per full run of a doc, 60 for 20 docs, and
more after each fix. The owner's acceptances are a cost too: the first review round of this design produced 104 raw findings across three stages (87 entries after the Critic round merged duplicates).
PR 3 records the reviewer runs, the owner acceptances and the wall-clock time. **Stop condition:** if PR 3 needs more
than 27 EGM reviewer runs or more than 10 owner acceptances (both are proposals for the owner to set), PR 4 does not
start until the owner decides.

### 8. Removal and pointers

The file lists are in the Detailed Implementation Plan. The rule is: remove Spec Kit wording from every live
instruction surface, keep the rule numbers of `AGENTS.md`, and let the checks list what is left.

*   **Pointers** are fixed in the constitution pull request (PR 2), because an amendment must update its dependents
    (constitution, Governance): `AGENTS.md`, `CLAUDE.md`, `specs/AGENTS.md`, `specs/README.md`, `README.md`,
    `SPECIFICATION-INDEX.md`, the three repository reviewers, the skills, the PR template, the engineering guides,
    `docs/owner-decisions.md`, `tests/Cargo.toml` and two quickstarts.
*   **The session-start hook** (PR 1) reads `.specify/feature.json` today. It will read the number `NNN` from the
    branch name `feature/NNN-slug` and keep the warning about other branches that mention that number. A branch with
    no number (for example `feature/egm-replaces-speckit` or a `bugfix/` branch) gets no warning.
*   **Deleted last** (PR 23): the `.specify/` directory (21 tracked files) and the ten `.claude/skills/speckit-*`
    skills. The scoped search for `speckit` and `Spec Kit` (Goal 4) turns on in that PR, because the skills and the
    generated files name Spec Kit until then.
*   **`check-agent-config.py`** gains the backticked-path check (PR 1). It scans the files it scans today plus
    `README.md`, `SPECIFICATION-INDEX.md`, `specs/README.md`, `.claude/agents` and `.claude/skills`; it tests
    existence with `git ls-files`, keeps an explicit allowlist file for gitignored and build-output paths, and treats
    placeholders such as `specs/NNN-slug/` as non-paths. The constitution checks are in `check-design-docs.py`
    (section 3).

### 9. Landing order and rollback

1.  **Step 0 (optional)**: tag `develop` as the last Spec Kit state, as a named restore point for the rollback
    amendment. Pushing the tag is an external action that the owner authorizes.
2.  **PR 1, the tooling**: `check-design-docs.py`, `egm-tree-hash.py`, the accepted-hash list, the settings entry, the
    hook change, the extended `check-agent-config.py`, the id allocator with its table of reserved ranges,
    `docs/designs/README.md` and `TEMPLATE.md`, and the CI step with a full-history checkout. It is additive and
    changes no normative text.
3.  **PR 2, the constitution**: the move and edit of the constitution, its Sync Impact Report, every pointer fix of
    section 8 (including the repository reviewers' footer contract), and the approval rule. The normative text lands
    only when the machinery it cites exists. The rule takes effect when PR 2 merges.
4.  **PR 3 to PR 22**: one conversion per feature (section 7).
5.  **PR 23, the removal**: delete `.specify/` and the ten skills after the sibling branches have rebased.

**Transitional clause.** Between PR 2 and PR 23, a feature whose conversion has not merged keeps its `tasks.md` as the
canonical task list and may still use the `/speckit-*` skills. The clause ends when PR 23 merges. If the owner chooses
to remove the tooling earlier (Q4), the clause is dropped and unconverted features keep their `tasks.md` by hand.

Rollback is a forward amendment, as for 3.0.0, not `git revert`: once ids and ticks live in design docs, reverting the
move would restore stale task state. Until the first conversion merges, PR 1 and PR 2 can each be reverted alone.

### 10. Carrier of each rule

For each item of "The Problem" and each Spec Kit skill, the carrier after the change. "None" means no carrier exists
and the design says so.

| Rule or capability today | Carrier after the change | Checked by |
|---|---|---|
| Constitution Check, Complexity Tracking | Required sections of a design doc; approval by the three repository reviewers | `check-design-docs.py` checks that the sections exist; the reviewers judge their content |
| Task state with evidence; ids | Tasks and evidence blocks in the Detailed Implementation Plan | `check-design-docs.py` (forward-only rule, id rules); whether the evidence is true stays with people |
| Append-only D, G, O registers | `research.md` stays in `specs/NNN-slug/`; a written rule plus a check that existing ids in a changed `research.md` are not deleted or renumbered against the base | `check-design-docs.py` (new, small) |
| Contracts that tests and CI read | Unchanged in place | CI contract jobs |
| Pointers | Backticked-path check and a wider scan | `check-agent-config.py` |
| Duplicate-work warning | The hook, reading the number from the branch name | Hook (a test runs it as a subprocess) |
| Feature numbers | `next-feature-id.py` | The script; `check-design-docs.py` fails on a duplicate number |
| Pinned toolchain | Settings entry and accepted hashes | `check-design-docs.py` (unlisted hash fails) |
| `/speckit-constitution` | An amendment by pull request with a version bump and a Sync Impact Report (constitution, Governance) | Reviewers; the version-line check |
| `/speckit-specify`, `/speckit-plan` | `egm-draft` (Overview, Problem, Goals, Technical Plan, Constitution Check) | Required sections |
| `/speckit-clarify` | `egm-brainstorm` questions and the Open Questions section | None |
| `/speckit-checklist` | **None.** Existing `checklists/` stay in `specs/`; the release-blocking items of feature 036's security checklist (34 open items, reported) are not enforced by any check | None |
| `/speckit-tasks` | The author writes the tasks in the Detailed Implementation Plan | Coverage rule, forward-only |
| `/speckit-analyze` | The coverage rule and the approval review | `check-design-docs.py` |
| `/speckit-implement` | No skill; the agent builds from the doc and ticks with evidence | Evidence rule |
| `/speckit-converge` | `egm-drift-report` assesses code against the doc. Appending the unbuilt work as new tasks is done by the author | None for the appending step |
| `/speckit-taskstoissues` | **None.** No replacement exists | None |

## Risks and Mitigations

| Risk | Mitigation | Residual limit |
|---|---|---|
| The check proves that a record is consistent, not that the review ran with reviewers in a fresh context; `egm-review` is not run in CI | Report hashes in the block; reports immutable; the owner reviews the pull request | Every field of the record can be written by the author |
| Evidence blocks are not reviewed text and may carry words that look like design | The evidence-block grammar (size cap, no heading, table or fence); a test for smuggled text | A short smuggled sentence inside a block passes |
| The conversion check compares ids, counts and a hash of normalized task text; an agent can change a requirement's meaning in `spec.md` while mapping | A per-id task hash; `egm-review` of the converted doc | A requirement reworded in the Overview or CUJs passes |
| The owner gate protects only tasks that the author marked | Reviewers look for unmarked tasks; Q3 asks whether to invert the default | An unmarked task that needed a decision passes |
| An `Approved` converted draft can be read as permission to build | The owner gate of section 5 | That gate is the only barrier |
| A converted doc was never discussed in a draft session, and EGM's readiness standard may fail a doc that came from a spec | Findings go to the owner as ESCALATE or are owner-accepted | The standard fails docs with a deliberate spike, such as feature 035's G035-022 |
| The id allocator sees only what the last `git fetch` brought; its table of reserved ranges can go stale | The script warns when the last fetch is more than a day old | Someone must update the table when a range is taken |
| The repository reviewers run only in Claude Code, so only a Claude Code session can produce an `Approved` doc; they are pinned by the reviewed commit and not by a hash | The commit in the record fixes their definitions | An edit to their definitions in the same commit changes what "reviewed" meant |
| A rebase or squash makes the reviewed commit unreachable | Merge commits by convention; full-history checkout; blob fallback | No repository setting enforces the merge method |
| The pin detects drift but cannot reproduce the accepted version | A person reads the diff; the entry records the commit | A developer can run another EGM version |
| A new reader or contributor has an unlisted hash and cannot produce an approvable review | The upgrade pull request of CUJ 4 | A diff of 100 or more files must be read |
| Reviewers read long documents selectively; the prior EGM run said so | A line ceiling per doc; split by increment | The ceiling is set only after PR 3 |
| Rollback after conversions | A forward amendment (section 9) | A revert restores stale task state |

## Constitution Check

The author's claim for this design, to be judged by the three repository reviewers. "n/a" means the principle's subject
is not touched.

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | n/a | No runtime state, transaction or scheduling change. |
| II. Modular domain, ports and adapters | Pass | No crate dependency changes; `check-architecture.py` is unchanged (Goal 7). |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect. |
| IV. Compatibility is versioned and pinned | Pass with conditions | The plugin is pinned by content (section 6) but the pin detects drift and does not reproduce a version. |
| V. Code, tenant boundary and secrets are isolated | Pass with conditions | No secret and no runtime path. The plugin adds agent types, two of them with write tools, to every session (Non-Functional Requirements). |
| VI. Observability and audit are part of the feature | Pass | Approvals and owner acceptances are recorded in the repository (section 3, section 5). |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | No event or message path. |
| VIII. Failure is tested and contracts are verified, honestly | Pass with conditions | Every new rule has a test that must fail first; guarantees that are not proven are listed in Risks and Mitigations. |
| Delivery Constraints | Pass | Scripts are Python 3 without dependencies, like the existing ones; EGM is development tooling and not part of the technology baseline; cost figures are labeled estimates. |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| `check-design-docs.py` with several rule families | The approval record and the rules the layout supplied need a check that can fail | Rules kept as prose (Alternative L) |
| `egm-tree-hash.py` and the accepted-hash list | The plugin is unpinned and its behavior lives in many files | Record only the version (Alternative I) |
| `next-feature-id.py` and a table of reserved ranges | Numbers clashed once and the manual look is unreliable | Manual look, or a duplicate check only (Alternative N) |
| The `Owner-gate:` line and its check | Owner decisions must gate work without a second approval state | "Stop and ask" or a review finding (Alternative J) |
| The three repository reviewers in the approval rule | EGM's lenses do not cover compliance, security or correctness | Review outside approval (Alternative M) |
| A status block and a diff against the reviewed commit | The approval record and the ticks must coexist | A ledger, a file per doc, or a normalized hash (Alternatives G and H) |

## Alternatives Considered

The first four alternatives were weighed in the brainstorm (`concepts_brief.md`, "Alternatives Weighed"). The rest were
put to the owner as choices while this document was drafted. They are grouped by decision. In every group the owner
chose the option stated, and the accepted trade-off is written in the rejection field.

### Scope of the change

#### Alternative A: Keep Spec Kit

*   **Pros**: Nothing to migrate. About 374 source files cite `FR/SC/T` ids, and about 2,036 register rows keep their
    place (reported).
*   **Cons**: The owner no longer wants it, and no defect of Spec Kit was named as the reason.
*   **Reason for Rejection**: The owner's decision of 2026-11-02 is to use only EGM. Keeping Spec Kit would also keep
    two specification formats in the project's tooling.

#### Alternative B: Delete `specs/` and move everything to `docs/designs/`

*   **Pros**: One layout for the whole repository.
*   **Cons**: CI reads `specs/001-platform-foundation/contracts/` (OpenAPI, protobuf, AsyncAPI, workflow schema), and
    tests read files of other features. Evidence of closed tasks would be lost.
*   **Reason for Rejection**: It would break the contract tests. The owner chose to remove the tool only and keep
    `specs/`.

#### Alternative C: Overlay, EGM for new features only

*   **Pros**: The cheapest option, and the one every review lens proposed. No id moves, no conversion cost, and
    `tasks.md` stays canonical for work in flight.
*   **Cons**: Two formats coexist for a long time. A task could be ticked in two places, and the same id could be
    allocated in both formats. The owner wants one format.
*   **Reason for Rejection**: The owner chose to convert the features with open tasks.

#### Alternative D: Convert each feature on a trigger

*   **Pros**: Spreads the cost. About 1,190 of the 1,268 open tasks are in features in specification only or not
    started, and a feature that is never built is never converted. 014, 033 and 037 finish where they are.
*   **Cons**: Two formats coexist until the last trigger, so a task can be ticked in the wrong file and the index
    drifts.
*   **Reason for Rejection**: The owner chose to convert all 20 features with open tasks. The accepted cost is 300 to
    660 EGM reviewer runs (derived, not measured) and the owner acceptances they produce.

### Approval

#### Alternative E: Approval with a second state, or locked while an owner question is open

*   **Pros**: Keeps "approved" meaning that the owner approved the scope. All six brainstorm reviewers advised it.
*   **Cons**: Adds a step the owner did not want. A lock on open owner questions also stops docs such as 035, which
    carry owner gates by design.
*   **Reason for Rejection**: The owner chose pure automatic approval, with "blocking" defined as Blocker or Major and
    only the owner accepting a contested finding. The per-task owner gate (section 5) keeps owner decisions without a
    second state.

#### Alternative M: Conformance, security and correctness review outside the approval of the doc

*   **Pros**: Approving a doc stays cheap, with `egm-review` alone. The three repository reviewers run on the diff of
    the implementation, as `CLAUDE.md` already says before a feature is declared done. A variant passes the
    constitution to every `egm-review` and requires the Constitution Check section, with no extra agent.
*   **Cons**: A design that violates the constitution can be `Approved` and found only after it is built, because the
    Check section is the author's own claim. Reading the constitution does not give EGM a lens that verifies it.
*   **Reason for Rejection**: The owner made the three repository reviewers part of the approval. The accepted cost is
    about 60 more reviewer runs, and that only Claude Code can produce an `Approved` doc.

### Record and ticks

#### Alternative G: Approval record in a central ledger or in a file per doc

*   **Pros**: The doc does not change when it is approved. A ledger is append-only, like the D/G/O registers.
*   **Cons**: A shared ledger is touched by all 20 conversion pull requests and conflicts on merge. A per-doc JSON file
    puts the owner's acceptance in a second file.
*   **Reason for Rejection**: The owner chose a status block in the doc, accepting that the doc is edited when it is
    approved.

#### Alternative H: Hash that ignores ticks, or tasks in a sibling ledger file

*   **Pros**: A hash that ignores ticks does not depend on git history. A sibling ledger leaves the approved doc
    untouched.
*   **Cons**: The normalizing rule can hide text in evidence lines, which is also true of the chosen design. A sibling
    ledger splits the tasks from the plan, against the owner's decision that tasks are a section of the plan.
*   **Reason for Rejection**: The owner chose a diff against the reviewed commit. Its weakness (a squash, a rebase or a
    shallow clone hides the commit) is handled by the merge-commit convention, the full-history checkout and the
    `Reviewed-blob` fallback of section 4.

### Procedure and pin

#### Alternative F: Procedure outside the constitution, or only its stages inside

*   **Pros**: A change to a skill name or a command needs no amendment. `docs/engineering/sdd-workflow.md` is already
    link-checked.
*   **Cons**: An obligation can lose its step without anyone amending anything.
*   **Reason for Rejection**: The owner chose to keep the whole procedure in the constitution. The accepted cost is an
    amendment for every step change, which section 6 routes through the accepted-hash check.

#### Alternative I: Other ways to pin EGM

*   **Pros**: Recording only the version is the cheapest. A `ref` on the upstream tag uses a documented field. A fork
    with a `sha` makes Claude Code enforce an exact commit. A vendored copy is reproducible and reviewable. Enabling the
    plugin at user scope and committing only the marketplace entry keeps the plugin's agents out of every session.
*   **Cons**: The version string `0.1.0` does not tell two commits apart. The tag `v0.1.0` is 14 commits and 105 files
    behind the commit in use. A fork must be maintained and its creation is an external action. A vendored copy ages.
    Project settings cannot require a version or a commit. User-scope enabling leaves each person to install and
    enable the plugin without a repository declaration.
*   **Reason for Rejection**: The owner chose a settings declaration plus a list of accepted hashes in the repository,
    accepting that project scope loads the plugin's nine agent types into every contributor's session.

### Owner decisions

#### Alternative J: Owner decisions by "stop and ask", or by a review finding

*   **Pros**: "Stop and ask" costs nothing and matches today's practice. A Major finding under EGM's readiness rule
    uses a gate that already exists.
*   **Cons**: "Stop and ask" depends on the agent noticing the question, which is how feature 016 was built twice. The
    readiness rule depends on how a reviewer grades severity, and it also fails docs with a deliberate spike.
*   **Reason for Rejection**: The owner chose a gate on the task, checked by the script.

### Conversion method

#### Alternative K: Interactive drafting for every converted feature

*   **Pros**: Full EGM discipline, and the owner knows each doc.
*   **Cons**: Four section approvals for each of 20 features is 80 section approvals, plus the reviews.
*   **Reason for Rejection**: The owner chose agent mapping judged by `egm-review`. Drafting stays interactive for new
    features.

#### Alternative O: A scripted converter with a golden test

*   **Pros**: Deterministic. A per-id hash of task text and of requirement rows against `spec.md` can be checked against
    a golden output, which closes the risk that an agent changes meaning while mapping.
*   **Cons**: The sources differ across 20 features written over several months, so a converter needs many special
    cases (feature 033 has no Check table; feature 014 has a different task layout). It is a program to maintain.
*   **Reason for Rejection**: The owner chose agent mapping. The per-id task hash of section 7 captures part of the
    benefit without a converter.

### Checks and numbering

#### Alternative L: No new checks, rules kept as prose

*   **Pros**: No new code. No script reads `tasks.md` today, so this keeps the status quo.
*   **Cons**: The rules that Spec Kit's layout half-supplied (the Check, the evidence rule, id uniqueness, coverage)
    would be unenforced, and `AGENTS.md` rule 21 says architecture is enforced by fitness functions, not by prose.
*   **Reason for Rejection**: Goal 3 requires that each rule has a named carrier, and the approval record needs a check
    that can fail.

#### Alternative N: No id allocator, or only a duplicate check in CI

*   **Pros**: No new code if the manual look at `origin/*` stays. A duplicate check in `check-design-docs.py` is cheap
    and catches a clash before the merge.
*   **Cons**: The manual look already failed once: two branches took the same number and it was found afterwards. A CI
    check finds the clash only when the second pull request opens, after both authors have worked, and cannot see
    branches that were never pushed.
*   **Reason for Rejection**: The owner chose to ship the allocator. The accepted cost is a table of reserved ranges
    that someone must keep current, and an allocator that sees only what the last `git fetch` brought.

## Non-Functional Requirements

*   **Security and supply chain.** EGM is a third-party plugin that runs with the permissions of the person using it,
    and its reviewers read files in the repository. Enabling it in the committed `.claude/settings.json` loads nine
    agent types into every contributor's session: seven read-only lens agents (`Read`, `Grep`, `Glob`), the
    synthesizer (`Agent`, `Read`, `Grep`, `Glob`, `Write`) and the bootstrap orchestrator (`Agent`, `Bash`, `Read`,
    `Grep`, `Glob`, `Write`, `Edit`). `AGENTS.md` rule 20 treats plugins as supply chain. The design adds no secret.
    It does not add to the `permissions.allow` list except one entry for `python3 scripts/check-design-docs.py`, in the
    style of the existing entries for the other check scripts; it is a small widening, and the owner may skip it and
    accept a prompt on each run. Design docs must hold no secret, because reviewers read them. Reviewer output is data
    and never instructions (`AGENTS.md` rule 20).
*   **Rollout and rollback.** Section 9 gives the order. The hook change keeps working for branches that have no
    design doc yet, because it reads only the branch name. After conversions, rollback is a forward amendment.
*   **Cost.** Section 7 gives the model, the 300 to 660 range, the stop condition after PR 3, and the unmeasured CI
    time of `check-design-docs.py`, which reads each design doc and runs one `git diff` per approved doc and is
    recorded with the first conversion.

### Additional Resources

*   `.specify/memory/constitution.md` (version 3.0.0) and
    [`docs/governance/constitution-3.0.0-sync-impact.md`](../governance/constitution-3.0.0-sync-impact.md).
*   [`docs/engineering/sdd-workflow.md`](../engineering/sdd-workflow.md),
    [`docs/engineering/ai-engineering.md`](../engineering/ai-engineering.md), [`AGENTS.md`](../../AGENTS.md),
    [`specs/AGENTS.md`](../../specs/AGENTS.md).
*   `tasks/reviews/2026-11-02-egm-constitution/concepts_brief.md`: the brainstorm, the six-lens review and the owner's
    decisions of 2026-11-02.
*   EGM `skills/egm-review/SKILL.md`, `skills/egm-draft/references/design_doc.md`, `docs/getting_started.md` (plugin
    commit `f0ba151b`).
*   Claude Code documentation on plugins and `extraKnownMarketplaces`:
    `https://code.claude.com/docs/en/plugins/marketplace-reference.md` and
    `https://code.claude.com/docs/en/plugins/loading.md`.

### Future Improvements

*   **One generated index.** Three indexes exist today (`SPECIFICATION-INDEX.md`, `docs/roadmap.md`,
    `docs/traceability.md`), and they have drifted. The status blocks could generate one.
*   **A review in CI**, if EGM adds a way to run reviewers headless. Today the harness list does not include CI.
*   **Vendoring EGM**, if more than a few hash additions are requested in a quarter (the owner sets the number).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Nothing below is built. The plan follows the landing order of Technical Plan section 9. Line numbers are those of
`develop` at commit `df59eb0e`. All paths are relative to the repository root.

### Target Files to Create/Modify/Delete

#### Step 0 (optional). Tag

*   `git tag speckit-last-state develop` and push the tag. This is an external action and the owner authorizes it. The
    tag is a named restore point for the rollback amendment. The evidence manifests of features 014 and 037, which
    record a SHA-256 of `.specify/memory/constitution.md` at a base commit, name commits on `develop`, which stay
    reachable under the merge-commit convention without a tag.

#### PR 1. The tooling (additive, no normative text)

##### 1. `scripts/check-design-docs.py` (Create) and `scripts/tests/test_check_design_docs.py` (Create)

*   **Type / Scope**: Python 3 script, no dependencies, and its unit tests in the style of `test_check_architecture.py`.
*   **Proposed Changes**: The checks of Technical Plan sections 3 to 5 and 7: required sections and Check rows; the
    status block of an approved doc; reachability of the reviewed commit with the `Reviewed-blob` fallback (a failure,
    never a skip); the diff rule with the evidence-block grammar; the accepted EGM hash and the minimum Claude Code
    version; report hashes and footer counts; owner-accepted findings and owner gates against
    `docs/owner-decisions.md`; id prefix, uniqueness and one definition site; duplicate feature numbers; forward-only
    coverage; the forward-only evidence rule; the append-only check on `research.md` ids; the constitution checks once
    `docs/constitution.md` exists; and `--conversion NNN` (superseded files unchanged since X; same ids, counts and
    normalized task-text hashes). `docs/designs/TEMPLATE.md` and `README.md` are skipped by name. Tests: one passing
    case and one failing case for each rule. The reviewed-commit tests use disposable git repositories (rebase, amend,
    shallow clone). Each rule has a case that is seen failing before the rule is written.

##### 2. `scripts/egm-tree-hash.py` (Create) and `scripts/tests/test_egm_tree_hash.py` (Create)

*   **Type / Scope**: Python 3 script and tests.
*   **Proposed Changes**: Prints one hash of the set defined in Technical Plan section 6, computed over a plugin install
    directory given as an argument. Tests: the same tree gives the same hash, a changed byte in a hashed file changes
    it, a changed adapter in `.claude-plugin/agents/` changes it, and a file outside the set (`site/`, `tests/`,
    `TEST.md`) does not.

##### 3. `docs/engineering/egm-accepted-hashes.json` (Create)

*   **Type / Scope**: List of accepted EGM entries.
*   **Proposed Changes**: One entry with the hash (from `egm-tree-hash.py` run on the installed copy), the plugin commit
    `f0ba151b` and the date. The owner reads and accepts it in PR 1.

##### 4. `.claude/settings.json` (Modify)

*   **Type / Scope**: Project settings for Claude Code.
*   **Proposed Changes**: Add `extraKnownMarketplaces.egm` (GitHub `google/elephant-goldfish-model`, `autoUpdate`
    false) and `enabledPlugins["egm@egm"]`, and one `permissions.allow` entry for
    `python3 scripts/check-design-docs.py` (see Non-Functional Requirements).

##### 5. `.claude/hooks/session-start.py` (Modify) and `scripts/tests/test_session_start_hook.py` (Create)

*   **Type / Scope**: SessionStart hook and its first test.
*   **Proposed Changes**: Read the number `NNN` from the branch name `feature/NNN-slug` instead of
    `.specify/feature.json`, and keep the warning about other branches that mention `NNN`. A branch with no number gets
    no warning. Tests run the script as a subprocess: a numbered feature branch, a branch with no number, and a
    branch that other branches mention.

##### 6. `scripts/check-agent-config.py` (Modify) and `scripts/tests/test_check_agent_config.py` (Modify)

*   **Type / Scope**: Existing checker and its tests.
*   **Proposed Changes**: Add the backticked-path check of Technical Plan section 8 (wider scan, `git ls-files`
    existence, an allowlist file, placeholders as non-paths) and the scoped search for `speckit` and `Spec Kit`, which
    stays off until PR 23. Tests: placeholders, gitignored files and build outputs.

##### 7. `scripts/next-feature-id.py` (Create), `scripts/tests/test_next_feature_id.py` (Create) and `SPECIFICATION-INDEX.md` (Modify)

*   **Type / Scope**: Python 3 script, its tests, and the reserved-ranges text of the index.
*   **Proposed Changes**: The script reads the branches of `origin`, `git worktree list`, the `specs/` and
    `docs/designs/` directories of each, and a table of reserved ranges, and prints the next free feature number and
    the free ranges for `D`, `G`, `O` and ADR numbers. `SPECIFICATION-INDEX.md` turns its prose about reserved ranges
    (for example ADR-0050 to ADR-0068 and the `D`/`G`/`O` 1000 to 1674 blocks) into a table the script can parse. The
    script warns when its last `git fetch` is older than a day. Tests: a number taken only on another branch is
    skipped, a reserved range is not offered, and a stale fetch is reported.

##### 8. `docs/designs/README.md` (Create) and `docs/designs/TEMPLATE.md` (Create)

*   **Type / Scope**: Layout and template of design docs (Technical Plan section 3, "Template").
*   **Proposed Changes**: The README describes the naming (`NNN-slug.md`, the same `NNN` as `specs/`, `NNN-slug.part-K.md`
    for a split doc, and unnumbered names for cross-cutting designs), the status block, the reviews directory, the
    required sections, the line ceiling (set after PR 3), and a table that maps each Spec Kit artifact (`spec.md`,
    `plan.md`, `tasks.md`, `research.md`, `contracts/`, `quickstart.md`, `checklists/`) to its place. The template extends
    EGM's template with the status block, Requirements with ids, Constitution Check, Complexity Tracking and "Owner
    decisions needed".

##### 9. `.github/workflows/ci.yml` (Modify)

*   **Type / Scope**: CI workflow, job `architecture` (lines 165 to 177).
*   **Proposed Changes**: Check out with `fetch-depth: 0` and add `python3 scripts/check-design-docs.py` after
    `check-agent-config.py`. The `unittest discover` step already picks up the new tests.

##### 10. `.claude/skills/verify/SKILL.md` (Modify) and `AGENTS.md` (Modify, Commands block only)

*   **Type / Scope**: Project skill and the shared command list.
*   **Proposed Changes**: Add `python3 scripts/check-design-docs.py` to the gates `verify` runs and to the Commands block
    of `AGENTS.md` (line 43 today). No normative text changes in this pull request.

#### PR 2. The constitution and the pointers

##### 11. `docs/constitution.md` (Move from `.specify/memory/constitution.md`, then Modify)

*   **Type / Scope**: Normative document.
*   **Proposed Changes**: Commit 1 is a pure `git mv`. Commit 2 applies the disposition table of Technical Plan
    section 1: principles I to VIII and Delivery Constraints unchanged; Development Workflow rewritten for the EGM
    procedure of section 2, with the approval rule (section 3), the owner-decision clause (section 5) and the
    transitional clause (section 9); Governance's Compliance review renamed to the design doc; the EGM pin rule
    (section 6) added; the stale "features 001–012" sentence dropped. The footer becomes the version of Q5 and the
    merge date, and the link at its end is corrected for the new directory.

##### 12. `docs/governance/constitution-4.0.0-sync-impact.md` (Create)

*   **Type / Scope**: Sync Impact Report in the format of the 3.0.0 report.
*   **Proposed Changes**: A table with one row per unit of 3.0.0 and its disposition (KEEP, MOVE, REWRITE, DROP, ADD),
    the added units, the dependents, the risks of Risks and Mitigations, and the rollback (a forward amendment). The
    file name follows the version chosen in Q5.

##### 13. `docs/governance/sync-impact-rust-sdk-2026-10-27.md` (Move from `.specify/memory/sync-impact-rust-sdk-2026-10-27.md`)

*   **Type / Scope**: Historical record of the 1.0.0 to 1.1.0 amendment.
*   **Proposed Changes**: `git mv` only. It is the only record of that amendment and would be deleted with `.specify/`.

##### 14. `docs/governance/constitution-3.0.0-sync-impact.md` (Modify)

*   **Type / Scope**: Historical record.
*   **Proposed Changes**: One line near the top: "The constitution moved to `docs/constitution.md` in 4.0.0; paths
    below name its location at 3.0.0." The line 46 path stays as written.

##### 15. `AGENTS.md` (Modify)

*   **Type / Scope**: Agent rules, shared by all agents.
*   **Proposed Changes**: Line 3 drops "(Spec Kit)". Line 7 and rule 1 (line 16) name `docs/constitution.md` and the
    design doc of the active feature, and the branch name replaces `.specify/feature.json`. Line 9 replaces the
    Spec Kit cycle with the EGM procedure, says which steps need a harness that can start subagents, and says that
    approval needs Claude Code, which runs the three repository reviewers. Line 11 says "in the design doc's Detailed
    Implementation Plan". A short "Install" line tells each person to run `claude plugin install egm@egm --scope
    project`. The "only the owner decides" bullet points at the constitution clause. Rule 2's "approved dependency"
    becomes "a dependency recorded in the design doc". The rule numbers do not change, and the file stays inside the
    line budget that `check-agent-config.py` enforces.

##### 16. `CLAUDE.md` (Modify)

*   **Type / Scope**: Claude Code specifics.
*   **Proposed Changes**: Line 7 replaces the `/speckit-*` description with the EGM skills and keeps `/verify`, `/adr`,
    `/migration` and `/parallel-agents`. It says: "EGM skills are marked manual-only, so Claude Code's `Skill` tool does
    not start them. An agent reads the skill's `SKILL.md` and follows it in the same session, or the person starts the
    skill with its slash command `/egm:<name>`." Line 10 says "re-read the active design doc" instead of `tasks.md`.

##### 17. `README.md` (Modify)

*   **Type / Scope**: Project overview.
*   **Proposed Changes**: Line 677 describes `specs/NNN-slug/` as contracts, evidence and registers, and adds
    `docs/designs/`. Line 681 names `docs/constitution.md`. Lines 721 and 722 drop the Spec Kit and `feature.json` text.

##### 18. `SPECIFICATION-INDEX.md` (Modify, second edit)

*   **Type / Scope**: Feature inventory.
*   **Proposed Changes**: Line 71 links `docs/constitution.md`. A short paragraph says that converted features are
    described in `docs/designs/`. Each conversion PR updates the status of its row.

##### 19. `specs/AGENTS.md` and `specs/README.md` (Modify)

*   **Type / Scope**: Rules for the `specs/` directory.
*   **Proposed Changes**: `specs/AGENTS.md` lines 3 and 9 stop saying the directories are produced and generated by Spec Kit,
    and state which files stay live (contracts, evidence, registers) and which are superseded after conversion.
    `specs/README.md` line 5 drops "within the Spec Kit feature tree".

##### 20. `docs/engineering/sdd-workflow.md` (Modify)

*   **Type / Scope**: Workflow guide.
*   **Proposed Changes**: Retitled for design docs. The nine-step Spec Kit table is replaced by a pointer to the
    constitution's procedure. The eight "what the repository adds" items stay, with `tasks.md` read as "the design
    doc's Detailed Implementation Plan". The "Writing a spec an agent can implement" and "Changing something that
    exists" sections are adapted. The anti-pattern "editing generated Spec Kit skills" is replaced by "ticking a
    superseded `tasks.md`".

##### 21. `docs/engineering/README.md` and `docs/engineering/ai-engineering.md` (Modify)

*   **Type / Scope**: Engineering index and the AI-engineering guide.
*   **Proposed Changes**: `README.md` lines 3 and 7 name the new constitution path and describe the guide as the design-doc
    workflow. `ai-engineering.md` gains the EGM pin under its section on plugins as supply chain, and line 63 drops the
    Spec Kit reference.

##### 22. `.claude/agents/architecture-reviewer.md`, `security-reviewer.md`, `correctness-reviewer.md` (Modify)

*   **Type / Scope**: Read-only reviewers.
*   **Proposed Changes**: Line 10 of each points to `docs/constitution.md` and to the design doc instead of
    `.specify/memory/...`, `spec.md`, `plan.md` and `tasks.md`. Each report ends with the findings footer of Technical
    Plan section 3 (`id, reviewer, severity, status`), with the severity map high/medium/low to Blocker/Major/Nit.
    They are part of the approval rule.

##### 23. `.claude/skills/verify/SKILL.md` and `.claude/skills/parallel-agents/SKILL.md` (Modify, second edit of `verify`)

*   **Type / Scope**: Project skills.
*   **Proposed Changes**: `verify` line 28 and `parallel-agents` lines 13 and 19 say "the design doc's Detailed Implementation
    Plan" instead of `tasks.md`/`spec`/`plan`.

##### 24. `.github/pull_request_template.md` (Modify)

*   **Type / Scope**: Pull request template.
*   **Proposed Changes**: Line 3 links the design doc, line 19 says "design doc (tasks ticked with evidence)", and line 25
    says the Constitution Check holds or the violation is justified in the design doc's Complexity Tracking.

##### 25. `docs/owner-decisions.md` (Modify)

*   **Type / Scope**: Register of owner decisions.
*   **Proposed Changes**: A new dated section records the decisions of 2026-11-02: EGM replaces Spec Kit; the tool only; the
    constitution at `docs/constitution.md`; all eight principles kept; automatic approval with the blocking
    definition; conversion of the 20 features; the architecture decisions of this design; and the owner's answers to
    Q1 to Q5 and to the contested findings.

##### 26. `tests/Cargo.toml` (Modify)

*   **Type / Scope**: Cargo manifest of the test crate.
*   **Proposed Changes**: The description on line 3 says "paths follow `docs/designs/*.md`" instead of `specs/*/plan.md`.

##### 27. `specs/014-workflow-primitives/quickstart.md` (Modify) and `specs/016-payload-classification/quickstart.md` (Modify)

*   **Type / Scope**: Quickstarts that call `.specify/scripts/bash/check-prerequisites.sh` (lines 23 and 46).
*   **Proposed Changes**: Replace the call with a plain `ls` of the files it checked. The 016 quickstart is parsed by
    `tests/e2e/payload-classification/quickstart.rs`, which compares the `sagat` commands it shows with the ones the
    walkthrough ran; that test runs after the edit.

#### PR 3 to PR 22. One conversion per feature

Each PR creates `docs/designs/NNN-slug.md` (or its parts), puts the superseded banner on `specs/NNN-slug/spec.md`,
`plan.md` and `tasks.md`, adds the review reports in `docs/designs/NNN-slug.reviews/`, and updates its row in
`docs/traceability.md` and `SPECIFICATION-INDEX.md`. The counts below are those of `develop` at `df59eb0e`; the
conversion comparison recomputes them at conversion time. The order puts features that nobody is changing first and
the ones with open branches last.

| PR | Feature | Ticked | Open | Note |
|---|---|---:|---:|---|
| 3 | 019-worker-deployments-dispatch | 0 | 68 | In specification only. Used to measure the cost. |
| 4 | 020-visibility-analytics-archival | 0 | 62 | In specification only. |
| 5 | 021-integration-resources-triggers | 0 | 80 | In specification only. |
| 6 | 022-flow-control-step-policies | 0 | 65 | In specification only. |
| 7 | 023-script-languages | 0 | 66 | In specification only. |
| 8 | 024-data-engines-pipelines | 0 | 69 | In specification only. |
| 9 | 025-ai-agents-mcp | 0 | 81 | In specification only. |
| 10 | 026-invocation-human-surfaces | 0 | 68 | In specification only. |
| 11 | 027-internal-apps | 0 | 61 | In specification only. |
| 12 | 028-developer-tooling | 0 | 74 | In specification only. |
| 13 | 029-custom-roles-folders | 0 | 69 | In specification only. |
| 14 | 030-cross-namespace-services | 0 | 70 | In specification only. |
| 15 | 031-client-side-encryption | 0 | 69 | In specification only. |
| 16 | 032-multi-region-failover | 0 | 64 | In specification only. |
| 17 | 033-redis-streams | 6 | 1 | Closing out. Its plan has no Check table. |
| 18 | 037-compensation-safety | 32 | 2 | Closing out; evidence manifests cite the constitution at a base commit. |
| 19 | 036-elastic-scaling | 0 | 94 | In specification only; has an open worktree. |
| 20 | 014-workflow-primitives | 37 | 8 | 14,004 lines of markdown in 42 files; split by increment. |
| 21 | 018-schedules-bulk-operations | 13 | 68 | In construction; convert after its active branch merges. |
| 22 | 035-iceberg-write-steps | 0 on `develop`, 2 on its branch | 129 | Convert after `feature/035-iceberg-write-steps` merges, or its two ticks are lost. |

Total: 1,268 open tasks.

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

##### 29. `.claude/skills/speckit-*/SKILL.md` (Delete, 10 files) and the scoped search (Enable)

*   **Type / Scope**: Generated Spec Kit skills, and the check that no live surface names Spec Kit.
*   **Proposed Changes**: Delete `speckit-analyze`, `speckit-checklist`, `speckit-clarify`, `speckit-constitution`,
    `speckit-converge`, `speckit-implement`, `speckit-plan`, `speckit-specify`, `speckit-tasks` and
    `speckit-taskstoissues`. This runs after sibling branches have rebased, so no session is left with a skill that
    points at a deleted script. The scoped search of Goal 4 turns on in the same pull request.

#### Files that are read and not changed

*   `specs/NNN-slug/{contracts,evidence}/`, `research.md`, `data-model.md`, `quickstart.md`, `checklists/` and
    `validation-report.md`: live, as the owner decided.
*   `docs/adr/ADR-0071-*.md`, the `/speckit-*` mentions in `docs/owner-decisions.md`, and
    `specs/001-platform-foundation/research.md`: dated records outside the scoped search.
*   `tasks/reviews/2026-10-30-*`: earlier review records.
*   The evidence manifests of 014 and 037 that hash the constitution at a base commit.

### Verification Plan

*   **Compilation/Build**: No Rust source changes. `cargo metadata --locked` still resolves after the `tests/Cargo.toml`
    description edit.
*   **Unit Tests**: `python3 -m unittest discover -s scripts/tests -v`, including the new tests for
    `check-design-docs.py`, `egm-tree-hash.py`, `next-feature-id.py`, the session-start hook and the extended
    `check-agent-config.py`. Each rule has a case that must fail, seen failing before the rule is written. The
    reviewed-commit tests use disposable git repositories.
*   **Static Analysis & Linting**: `python3 scripts/check-architecture.py`, `python3 scripts/check-agent-config.py` and
    `python3 scripts/check-design-docs.py` all exit 0 on the merged tree. After PR 23 the scoped search for `speckit`
    and `Spec Kit` finds nothing on the live instruction surfaces.
*   **Quickstart test**: `cargo test -p sagat-tests --test e2e_payload_classification` for the edited 016 quickstart,
    run with a private `CARGO_TARGET_DIR`.
*   **Journeys**: CUJ 1 to 5 are run once each on a scratch branch. For CUJ 2, edit a sentence of an approved doc and see
    the check fail. For CUJ 4, record a review with an unlisted hash and see the check fail.
*   **Independent read**: the `architecture-reviewer`, `security-reviewer` and `correctness-reviewer` agents read the
    diff of PR 1 and PR 2 together with this design, and not the author's conclusions.
*   **Cost**: PR 3 records the reviewer runs, the owner acceptances and the wall-clock time of one conversion, and the
    stop condition of Technical Plan section 7 is applied before PR 4.

--------------------------------------------------------------------------------

*Drafted with EGM*
