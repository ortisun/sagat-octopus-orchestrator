# Specification-driven development with Spec Kit

The specification is the source of truth and the code is its implementation. Spec Kit (`.specify/`, version in `.specify/init-options.json`) supplies the cycle and the Claude Code skills; this page records how the project uses it and what it adds.

## The cycle

| Step | Skill | Produces | Gate it must pass |
|---|---|---|---|
| Principles | `/speckit-constitution` | `.specify/memory/constitution.md` | amended by pull request, version bump, Sync Impact Report |
| What and why | `/speckit-specify` | `specs/NNN-slug/spec.md` | user stories independently testable; requirements testable and technology-free; measurable success criteria |
| Resolve doubt | `/speckit-clarify` | answers in `spec.md` and `research.md` | no `[NEEDS CLARIFICATION]` left before planning; product questions go to the owner |
| How | `/speckit-plan` | `plan.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md` | Constitution Check passes, or the violation is justified in Complexity Tracking |
| Quality of the requirements | `/speckit-checklist` | `checklists/` | "unit tests for English": complete, unambiguous, consistent, measurable |
| Work | `/speckit-tasks` | `tasks.md` | tasks traced to requirements, tests before implementation, parallel markers `[P]` only for disjoint files |
| Consistency | `/speckit-analyze` | `analysis.md` | no uncovered requirement, no orphan task, no contradiction across artifacts |
| Build | `/speckit-implement` | code, tests, docs | red first, evidence per task, Definition of Done |
| Reconcile | `/speckit-converge` | updated spec/plan/tasks | the documents describe what was built, including what was not |

Feature numbers are sequential and shared by every branch (a draft on another branch may already hold the next one), so look at `origin/*` and the sibling worktrees before creating a feature (`git branch -a`, `git worktree list`). The directories under `specs/` are the record of what exists.

## What the repository adds to the stock cycle

1. **Feature-scoped ids** (`FR-014-012`, `T014-038`) so an id is unique across features; commit footers cite them (`Refs:`).
2. **Evidence per task.** A task is ticked with the named test and its result, or the command and its output, written under the task. A partly done task stays open with a dated note of what is done and what is not. Never tick from a report you did not verify.
3. **Registers in `research.md`.** Decisions `D`, gaps `G`, owner questions `O`: append-only, numbers never reused or renumbered. A gap is a promise to tell the truth about a limit; it is closed by evidence, not by deletion.
4. **Owner decisions.** What is product, scope, budget or release belongs to the owner. Ask once, with a recommendation and the options; record the answer in `docs/owner-decisions.md` and where the item lived.
5. **Amending a task.** If a task names something that cannot be (a file never created, a range that contradicts a contract) the fix is a decision that says so and an edit of the task text that cites it, not a silent reinterpretation.
6. **Validation report.** Measured evidence (commands, counts, environment, failures, what was skipped) goes in `validation-report.md`, dated; a later run adds a section, it does not rewrite history.
7. **Traceability.** `docs/traceability.md` maps requirement → tests → contract → metric/log → demo. It changes in the same delivery as the code.
8. **Same-delivery rule.** Spec, contracts, tests and documentation move together with the code (constitution, workflow).

## Writing a spec an agent can implement

- Behaviour and outcomes, not technology. If you must name a technology, it is in the plan.
- Every requirement has an observable acceptance condition. "Fast" becomes a measured scenario on a stated environment; targets are not promises (constitution).
- State the failure behaviour next to the happy path: duplicate, timeout, restart, tenant boundary, quota.
- Say what is out of scope and which feature owns it.
- Prefer examples (concrete inputs and outputs) to adjectives; they become the first tests.

## Changing something that exists

- **Bug**: a `bugfix/<slug>` branch from `develop`, a failing test that reproduces it, the fix, `Refs:` to the requirement it violates. If the spec was wrong, amend the spec in the same pull request.
- **Behaviour change**: change the spec first (or `/speckit-clarify`), then plan and tasks, then code. A contract change is additive unless an ADR says otherwise.
- **Divergence from an ADR**: a new ADR (`/adr`), then the documents that depend on it.

## Enforcement, not convention

The usual criticism of spec-driven tooling is that the documents govern by convention. Here they are tied to checks: `scripts/check-architecture.py` (layering), the runtime-privilege audit, the storage-classification manifest, contract baselines and compatibility tests, the five-language conformance suite, the cross-tenant battery, `scripts/check-agent-config.py`, and CI gates on all of them. When you write a rule, write the check.

## Anti-patterns seen here

- Starting a feature that a sibling session had already built (016). Check branches and worktrees first.
- Two branches taking the same decision number or migration timestamp. Grep the other branches.
- Ticking a task from an agent's summary. Run the gate on the merged tree.
- Editing generated Spec Kit skills by hand. Customise `.specify/templates` and the constitution.
- Leaving a placeholder that looks like a guarantee. Mark it as a pending decision or test.
