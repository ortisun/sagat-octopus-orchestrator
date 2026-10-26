# Architecture Decision Records

One file per decision, `ADR-NNNN-<slug>.md`, numbered in the order they are taken. Start from [TEMPLATE.md](TEMPLATE.md) or run `/adr`.

**Write an ADR when a decision is hard to reverse, surprising without context, and the result of a real trade-off.** A smaller choice is a rule in the nearest `AGENTS.md` or a `D…` entry in the feature's `research.md`. Product, scope and release decisions are the owner's and are also recorded in [../owner-decisions.md](../owner-decisions.md).

Rules:

1. Numbers are shared by every branch. Before taking one, list this directory, `origin/*` and the sibling worktrees. Never renumber a published ADR.
2. An accepted ADR is not edited in substance. A change of mind is a new ADR that supersedes it and updates the documents that depend on it, with benchmark data, failure tests, operational cost and compatibility with runs in flight (constitution, Governance). Mark the old one `Superseded by`.
3. State the decision as MUST / MUST NOT and name what enforces it. A rule that nothing checks is added to [../engineering/architecture-principles.md](../engineering/architecture-principles.md) with the check that should exist, or the ADR says "review only" and why.
4. Say who decided. An agent may draft and propose; a person accepts.
5. `python3 scripts/check-agent-config.py` checks that each file starts with `# ADR-NNNN`, has a Status line near the top, and that numbers are unique. Older ADRs use a dash after the number; new ones use the colon of the template.

Release checkpoints: [ADR-0040](ADR-0040-release-checkpoints-preserve-incomplete-feature-boundaries.md) preserves incomplete-feature boundaries and all technical release gates.
