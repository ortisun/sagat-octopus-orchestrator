# Specification-driven development with EGM design docs

The specification is the source of truth and the code is its implementation. Features are described in design docs under `docs/designs/`, written and reviewed with the Elephant-Goldfish Model (EGM); this page records how the project uses them and what it adds. The procedure itself, the approval rule and the owner-decision rules are in the constitution (`docs/constitution.md`, Development Workflow); the layout, the status block and the task grammar are in [docs/designs/README.md](../designs/README.md).

## The cycle

`egm-brainstorm` → `egm-draft` → `egm-review` (all five stages, plus the `architecture-reviewer`, `security-reviewer` and `correctness-reviewer`) → build → `egm-drift-report`. A design doc is `Approved` when none of those reviewers has a Blocker or Major finding open; the owner may accept a finding in writing. `python3 scripts/check-design-docs.py` verifies the record. The EGM skills are manual-only: read the skill's `SKILL.md` and follow it, or start it with `/egm:<skill-name>`.

Feature numbers are shared by every branch (a draft on another branch may already hold the next one): take the next with `python3 scripts/next-feature-id.py`, and look at `git worktree list` before you start. A feature that has not been converted to a design doc keeps its `spec.md`, `plan.md` and `tasks.md` under `specs/NNN-slug/`; they are edited by hand under the constitution's transitional clause.

## What the repository adds to the stock cycle

1. **Feature-scoped ids** (`FR-014-012`, `T014-038`) so an id is unique across features; commit footers cite them (`Refs:`). A cross-cutting design without a number declares a `Task-prefix`.
2. **Evidence per task.** A task is ticked with the named test and its result, or the command and its output, written under the task. A partly done task stays open with a dated note of what is done and what is not. Never tick from a report you did not verify.
3. **Registers in `research.md`.** Decisions `D`, gaps `G`, owner questions `O`: append-only, numbers never reused or renumbered. A gap is a promise to tell the truth about a limit; it is closed by evidence, not by deletion. `research.md` stays in `specs/NNN-slug/`.
4. **Owner decisions.** What is product, scope, budget or release belongs to the owner. Ask once, with a recommendation and the options; record the answer in `docs/owner-decisions.md` and where the item lived. A task that depends on one carries an `Owner-gate:` line.
   **Who decides.** `scripts/check-design-docs.py` lets an `Owner-gate:` line, or the acceptance of a review finding in an approved design doc, cite only a row that counts as the owner's:
   - a row whose `Decided by` is `owner` and whose `Status` is `Decided`;
   - for release 0.3.0 only, a row whose `Decided by` is exactly `delegate (OD-2026-11-24-4)` and whose `Status` is `Decided`. This is the owner's delegation of 2026-11-24 ("Quero que os agents continuem por mim. Tomem a melhor decisão"), which lets the agents settle review findings, approve design docs, accept ADRs and checklist items, and merge into `develop`. The row OD-2026-11-24-4 must itself be an owner row whose `Status` is exactly `Decided`: a suffix such as "; reported" would stop every delegate row from counting.

   What stays the owner's, as the constitution says (5.0.0, Owner decisions): acts that need the owner's own hands or credentials, publishing the release, spending money, creating accounts, irreversible acts outside the repository, amending the constitution, and every security exception. The check enforces one of these: a delegate row may not accept a finding of the security report, the report whose `Reviewer` is `security`. Acceptances are counted per report, so spare acceptances of one report cannot cover the findings of another. The check does not read `checklists/*.md`, so keeping a security checklist item with the owner is a review rule, not a check.

   These still do not count: a plain `delegate` row (the delegated decisions of 2026-11-03, Owner decision 2026-11-03-22), and a delegate row that names any other decision.

   **When the delegation ends.** It ends when the tag `v0.3.0` exists. From then on a delegate row counts only if the identical row is in `docs/owner-decisions.md` as the tag holds it. So acts recorded during the release stay valid, while a row added or edited after the tag is a new act and is refused. In a pull request the check also requires the release tags to be in the clone (`v0.2.0` resolves), so that a clone without tags cannot keep the delegation open. The table `DELEGATIONS` in the script holds each delegation and the tag that ends it. A test (`DelegationsAreNamedByTheConstitution`) fails unless the constitution's Owner decisions clause names both, so a later delegation needs the owner's amendment first.
5. **Amending a task.** If a task names something that cannot be (a file never created, a range that contradicts a contract) the fix is a decision that says so and an edit of the task text that cites it, not a silent reinterpretation. After approval, only a checkbox, an evidence block and follow-up tasks may change.
6. **Validation report.** Measured evidence (commands, counts, environment, failures, what was skipped) goes in `validation-report.md`, dated; a later run adds a section, it does not rewrite history.
7. **Traceability.** `docs/traceability.md` maps requirement → tests → contract → metric/log → demo. It changes in the same delivery as the code.
8. **Same-delivery rule.** Spec, contracts, tests and documentation move together with the code (constitution, workflow).
9. **Documentation first.** The document leads and the code follows: write the change in its document first, commit it before the code or with it, and never let the code lead. When code and document disagree, fix the wrong one; never leave them apart. `scripts/check-docs-first.py` runs on every pull request (constitution 4.7.0; AGENTS.md rule 23).
10. **Words in full.** Write "Rule 8", "Blocker 1", "Task 022-064", not "R8", "B1", "T022-064". The old identifier forms and their new names are in `docs/governance/identifier-scheme.md`; `scripts/check-plain-words.py` runs on every pull request (AGENTS.md rule 24).

## Writing a design an agent can implement

- Behaviour and outcomes, not technology. If you must name a technology, it is in the Technical Plan.
- Every requirement has an observable acceptance condition. "Fast" becomes a measured scenario on a stated environment; targets are not promises (constitution).
- State the failure behaviour next to the happy path: duplicate, timeout, restart, tenant boundary, quota.
- Say what is out of scope and which feature owns it.
- Prefer examples (concrete inputs and outputs) to adjectives; they become the first tests.
- The Constitution Check and Complexity Tracking sections are required; a violation is justified there with the simpler alternative that was rejected.

## Changing something that exists

- **Bug**: a `bugfix/<slug>` branch from `develop`, a failing test that reproduces it, the fix, `Refs:` to the requirement it violates. If the design was wrong, amend it in the same pull request (an `Approved` doc becomes a draft until it is reviewed again).
- **Behaviour change**: change the design doc first (or the `spec.md` of a feature not yet converted), then the plan and tasks, then code. A contract change is additive unless an ADR says otherwise.
- **Divergence from an ADR**: a new ADR (`/adr`), then the documents that depend on it.

## Enforcement, not convention

The usual criticism of spec-driven tooling is that the documents govern by convention. Here they are tied to checks: `scripts/check-architecture.py` (layering), the runtime-privilege audit, the storage-classification manifest, contract baselines and compatibility tests, the six-language conformance suite, the cross-tenant battery, `scripts/check-agent-config.py`, `scripts/check-design-docs.py` (design docs and the approval record), `scripts/check-docs-first.py` (the document changes with the code) and `scripts/check-plain-words.py` (words in full), and CI gates on all of them. When you write a rule, write the check.

## Anti-patterns seen here

- Starting a feature that a sibling session had already built (016). Check branches and worktrees first.
- Two branches taking the same decision number or migration timestamp. Grep the other branches.
- Ticking a task from an agent's summary. Run the gate on the merged tree.
- Ticking a task of a superseded file, or in two places. A converted feature has one task list.
- Leaving a placeholder that looks like a guarantee. Mark it as a pending decision or test.
