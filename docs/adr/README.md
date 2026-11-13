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

Lake sandbox API: [ADR-0079](ADR-0079-the-lake-runner-asks-the-sandbox-runner-for-an-attempt-never-for-a-container.md) — proposed; the lake runner asks a lake-mode sandbox runner for an attempt over a node-local Unix socket (attempt id, slot, budget class, deadline; never an image, mount, command or limit), with all lifecycle state in engine labels (refines ADR-0071 and ADR-0013; the Kubernetes shape stays open under T035-029).

Capacity locks: [ADR-0078](ADR-0078-capacity-group-lock-order-and-possession-protocol.md) — proposed; the ChildRoot capacity group header is a lock nobody waits for and possession moves only through fenced commands (refines ADR-0075; leaves the creator, the sweeper's coverage and the worker's complete lock sequence open).

Private platform nodes: [ADR-0080](ADR-0080-a-node-only-the-platform-may-write-is-a-private-document.md) — proposed; a node that only the platform may write (today the Iceberg probe) is a private document: the IR codec has a private twin of its two parse entry points over the public schema plus the private node types, the loader chooses it from `workflows.internal`, and the public schema never learns the node (refines the way ADR-0071 and feature 017 keep the platform's own runs private).

Step logs: [ADR-0084](ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md) — accepted (OD-2026-11-13-17); a step log is a diagnostic record beside the journal, never execution state: fenced by the lease, stored at most once per (attempt, generation, sequence), bounded by ceilings that drop and count but never fail a step, in ordinary PostgreSQL rows with no foreign key to `runs`, read in stored order (applies ADR-0027 and ADR-0015; feature 041).

Step-log disclosure: [ADR-0085](ADR-0085-step-log-text-is-read-under-its-own-permission-and-redacted-by-value-in-the-worker.md) — accepted (OD-2026-11-13-17); the SDK removes the attempt's resolved secret values before a line leaves the worker, the server applies the existing shape and field-name rules and never resolves a secret to redact, lines are read only with `run:logs:read` and reads are audited; it states the limit that a transformed secret is stored as written (a condition on principle V; feature 041).
