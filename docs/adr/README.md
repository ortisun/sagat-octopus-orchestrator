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

Recovery refinement: [ADR-0076](ADR-0076-compensation-failure-and-uncertain-effects.md) — accepted 2026-11-01 for delivery 1 (Constitution 2.0.0, principle III); provider qualification (US04) stays proposed.

Specification workflow: [ADR-0077](ADR-0077-egm-replaces-spec-kit.md) — proposed; EGM design docs replace Spec Kit (Constitution 4.0.0).

Reset of a run that is still open: [ADR-0081](ADR-0081-cancel-source-is-switched-on-per-process-until-the-capability-registry-exists.md) — proposed; `CANCEL_SOURCE` is switched on per process, engine first, until the instance-capability registry of ADR-0050 decision 9 exists, and records the order of locks of its request (feature 018, T018-057).

The instance-capability registry: [ADR-0082](ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md) — proposed; requirements are recorded only by the operator, a refused process stays up with a closed gate, a requirement is learned within one heartbeat (refines ADR-0050 decision 9).
The handover of the cron triggers: [ADR-0083](ADR-0083-the-cron-triggers-are-handed-to-the-schedule-pass-once-and-only-when-they-have-converged.md) — proposed; no data migration, the quota exemption, the engine's authority without a membership, a one-way switch that requires convergence and opens its control after a few seconds (refines ADR-0082).
