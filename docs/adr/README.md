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

Reset of a run that is still open: [ADR-0081](ADR-0081-cancel-source-is-switched-on-per-process-until-the-capability-registry-exists.md) — accepted (OD-2026-11-13-4); `CANCEL_SOURCE` is switched on per process, engine first, until the instance-capability registry of ADR-0050 decision 9 exists, and records the order of locks of its request (feature 018, T018-057).

The instance-capability registry: [ADR-0082](ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md) — accepted (OD-2026-11-13-4); requirements are recorded only by the operator, a refused process stays up with a closed gate, a requirement is learned within one heartbeat (refines ADR-0050 decision 9).

The handover of the cron triggers: [ADR-0083](ADR-0083-the-cron-triggers-are-handed-to-the-schedule-pass-once-and-only-when-they-have-converged.md) — accepted (OD-2026-11-13-4); no data migration, the quota exemption, the engine's authority without a membership, a one-way switch that requires convergence and opens its control after a few seconds (refines ADR-0082).

Lake sandbox API: [ADR-0079](ADR-0079-the-lake-runner-asks-the-sandbox-runner-for-an-attempt-never-for-a-container.md) — proposed; the lake runner asks a lake-mode sandbox runner for an attempt over a node-local Unix socket (attempt id, slot, budget class, deadline; never an image, mount, command or limit), with all lifecycle state in engine labels (refines ADR-0071 and ADR-0013; the Kubernetes shape stays open under T035-029).

Capacity locks: [ADR-0078](ADR-0078-capacity-group-lock-order-and-possession-protocol.md) — proposed; the ChildRoot capacity group header is a lock nobody waits for and possession moves only through fenced commands (refines ADR-0075; leaves the creator, the sweeper's coverage and the worker's complete lock sequence open).

Private platform nodes: [ADR-0080](ADR-0080-a-node-only-the-platform-may-write-is-a-private-document.md) — proposed; a node that only the platform may write (today the Iceberg probe) is a private document: the IR codec has a private twin of its two parse entry points over the public schema plus the private node types, the loader chooses it from `workflows.internal`, and the public schema never learns the node (refines the way ADR-0071 and feature 017 keep the platform's own runs private).

Step logs: [ADR-0084](ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md) — accepted (OD-2026-11-13-17), revised after review before any code; a step log is a diagnostic record beside the journal, never execution state: fenced as a report is short of the run lock, never holding the tenant usage row before its commit, stored at most once per (attempt, generation, sequence), bounded by ceilings that drop and count but never fail a step, in ordinary PostgreSQL rows with no foreign key to `runs`, read in stored order (applies ADR-0027 and ADR-0015; feature 041).

Step-log disclosure: [ADR-0085](ADR-0085-step-log-text-is-read-under-its-own-permission-and-redacted-by-value-in-the-worker.md) — accepted (OD-2026-11-13-17), revised after review before any code; the SDK, in the per-attempt logger, removes the secret values of every attempt in flight in the process before a line leaves the worker, the server applies the existing shape and field-name rules and never resolves a secret to redact, lines are read only with `run:logs:read` (and the PII reveal grant in a classified namespace, OD-2026-11-13-18), as is a script execution's log, and reads and deletions are audited; it states the limit that a transformed secret is stored as written (a condition on principle V; feature 041).

Operator chains on triggers: [ADR-0086](ADR-0086-operator-chains-on-triggers-are-declared-data-and-a-fenced-pass-seals-a-window-that-starts-one-run.md) — accepted (OD-2026-11-13-17), revised after review the same day; a trigger's operator chain (design 042, later release) is declared data applied by the engine once per admitted event, its state is rows per chain and key on the database clock, a fenced pass seals a window and a sealed window starts one run by its identity through the one admission port; no operator paces a start (that is ADR-0088).

Collection operators: [ADR-0087](ADR-0087-collection-operators-are-applied-once-at-scope-entry-and-the-snapshot-is-the-record.md) — accepted (OD-2026-11-13-17), revised after review the same day; a `forEach` operator chain (design 042, release 0.3.0) is a pure function of the evaluated `items`, applied once by one evaluator call under running ceilings on elements and bytes before the membership snapshot, which is the only record a replay reads.

Rate limits: [ADR-0088](ADR-0088-rate-limits-are-a-durable-window-of-grants-on-the-database-clock-and-the-scheduler-grants-never-the-claim.md) — accepted (OD-2026-11-13-17); a per-key rate limit is a strict sliding window of durable grant rows on the database clock, buffered work is granted by its shard owner's scheduler pass and never by the claim, a step's slot counts from its lease, and a run that declares a concurrency key too gets permit and slot in one transaction (refines ADR-0056, ADR-0050 decision 7 and ADR-0020; the owner chose the strict window in OD-2026-11-13-16, the ADR itself is not yet accepted). ADR-0081 to ADR-0083 are taken on the business-ids branch of feature 018.

Gated steps: [ADR-0089](ADR-0089-a-gated-step-waits-outside-ready-and-one-fenced-grant-routine-serves-every-waiter-by-key.md) — accepted (OD-2026-11-13-19); a step with node-level `concurrency` or `rateLimit` waits outside `READY` for every readiness source, and one fenced grant routine driven by key, with a reservation rule, serves every waiter whatever its gates (supersedes ADR-0056 decisions 3 and 5, refines ADR-0088 decisions 3 and 7).

Run input validation: [ADR-0090](ADR-0090-every-run-input-is-checked-against-its-pinned-version-schema-before-the-run-is-created.md) — accepted (OD-2026-11-14-15; design 043; OD-2026-11-14-2, -8 to -11 and -14); every path that creates a run checks its input against the pinned version's `inputSchema` before the creation transaction, through one gate, with a documented subset of JSON Schema validated by the platform's own validator with a work counter, no added statement in `enforce`, a value-free final refusal per creator, and the `enforce` mode required through the instance registry.

Run input validation, refined in the build: [ADR-0092](ADR-0092-the-application-names-a-runtime-only-for-the-blob-source-parse-and-the-run-input-rules-refined-in-the-build.md) — accepted (OD-2026-11-16-4; design 043, D043-154); refines ADR-0090 and ADR-0050 decision 2: `sagat-application` names `tokio` (`rt` alone) only in `collection_source.rs` for design 042's blocking parse, every composition root wires the gate into each creator, the 1 KiB name limit and the cache share of decision 5, `/` for every repeat and trigger-now (decision 8), and the audit of a `report`-mode start an open holder answers (decision 10); a stricter subset ships `run-input-validation-v2` (decision 6); a refused run input is an `INPUT_INVALID` action whose time is acted on, not owed (decision 7).
