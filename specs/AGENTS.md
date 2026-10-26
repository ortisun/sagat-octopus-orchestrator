# specs

One directory per feature, `NNN-slug/`, produced by Spec Kit (`spec.md`, `plan.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`, `tasks.md`, `analysis.md`, `checklists/`, and a `validation-report.md` when the feature has measured evidence).

- A spec says what and why, testable and technology-free; a plan says how and carries the Constitution Check; `research.md` holds the decision (`D`), gap (`G`) and owner-question (`O`) registers. Resolve every `[NEEDS CLARIFICATION]` before planning.
- Task text that turns out to name a file that was never created or a range that was wrong is amended with a decision that says so (as D300 did); it is not silently reinterpreted.
- Tick a task only with evidence (a named test and its result, a command and its output). A partly done task stays unticked with a dated note of what is done and what is not.
- Numbers (`D`, `G`, `O`, ADR, migration timestamps) are shared across branches: before taking the next one, grep `origin/*` and the sibling worktrees; never renumber one already published.
- `.claude/skills/speckit-*` and `.specify/integrations/*` are generated; customise through `.specify/templates` and the constitution. A constitution change is a pull request with a version bump and a Sync Impact Report (`/speckit-constitution`), not an edit in passing.
- Keep `docs/traceability.md`, `docs/roadmap.md` and `docs/owner-decisions.md` in step with the spec in the same delivery.
