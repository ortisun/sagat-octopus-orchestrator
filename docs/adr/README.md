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

Step logs: [ADR-0084](ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md) — accepted (OD-2026-11-13-17), revised after review before any code; a step log is a diagnostic record beside the journal, never execution state: fenced as a report is short of the run lock, never holding the tenant usage row before its commit, stored at most once per (attempt, generation, sequence), bounded by ceilings that drop and count but never fail a step, in ordinary PostgreSQL rows with no foreign key to `runs`, read in stored order (applies ADR-0027 and ADR-0015; partly superseded by ADR-0095 and ADR-0097, 2026-11-20; feature 041).

Step-log disclosure: [ADR-0085](ADR-0085-step-log-text-is-read-under-its-own-permission-and-redacted-by-value-in-the-worker.md) — accepted (OD-2026-11-13-17), revised after review before any code; the SDK, in the per-attempt logger, removes the secret values of every attempt in flight in the process before a line leaves the worker, the server applies the existing shape and field-name rules and never resolves a secret to redact, lines are read only with `run:logs:read` (and the PII reveal grant in a classified namespace, OD-2026-11-13-18), as is a script execution's log, and reads and deletions are audited; it states the limit that a transformed secret is stored as written (a condition on principle V; feature 041).

Step-log carry: [ADR-0095](ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md) — accepted 2026-11-20 (OD-2026-11-20-15; OD-2026-11-19-1 chose the direction); an attempt's last step-log records ride on its `CompleteStep` or `FailStep` and are written by the report's own transaction, at most 64 records in one call of a database function whose step-log locks are only tried, `AppendStepLogs` stays for a long step, a quota refusal or a busy usage row is answered in the same call by a mark-only final mark, and a carry call that fails is dropped by a downgrade re-run of the report (supersedes ADR-0084 decisions 2, 6 and 11; decisions 2 to 5 restated with OD-2026-11-20-18; feature 041).

Step-log quota leases: [ADR-0097](ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md) — accepted 2026-11-20 (OD-2026-11-20-15; D041-108, delegate OD-2026-11-20-6); a step-log charge goes to its writer's own lease (one per tenant and server backend), whose room the store tops up after a carrying report commits, outside the run lock (OD-2026-11-20-18); the usage row is locked only by a refill (a batch's, a marker's, or a carry's fallback bounded at 200 ms a lock and by wait slots), the top-up (which never waits), a release, an idle reclaim or a limit change, and the usage never exceeds the limit in force when the charge was admitted (supersedes the step-log charge of ADR-0027's Amendment item 3 and, for an ingest, ADR-0084's decision 3 clause on the usage row; a release never waits for a lease; feature 041).

Operator chains on triggers: [ADR-0086](ADR-0086-operator-chains-on-triggers-are-declared-data-and-a-fenced-pass-seals-a-window-that-starts-one-run.md) — accepted (OD-2026-11-13-17), revised after review the same day; a trigger's operator chain (design 042, later release) is declared data applied by the engine once per admitted event, its state is rows per chain and key on the database clock, a fenced pass seals a window and a sealed window starts one run by its identity through the one admission port; no operator paces a start (that is ADR-0088).

Collection operators: [ADR-0087](ADR-0087-collection-operators-are-applied-once-at-scope-entry-and-the-snapshot-is-the-record.md) — accepted (OD-2026-11-13-17), revised after review the same day; a `forEach` operator chain (design 042, release 0.3.0) is a pure function of the evaluated `items`, applied once by one evaluator call under running ceilings on elements and bytes before the membership snapshot, which is the only record a replay reads.

Rate limits: [ADR-0088](ADR-0088-rate-limits-are-a-durable-window-of-grants-on-the-database-clock-and-the-scheduler-grants-never-the-claim.md) — accepted (OD-2026-11-13-17); a per-key rate limit is a strict sliding window of durable grant rows on the database clock, buffered work is granted by its shard owner's scheduler pass and never by the claim, a step's slot counts from its lease, and a run that declares a concurrency key too gets permit and slot in one transaction (refines ADR-0056, ADR-0050 decision 7 and ADR-0020; the owner chose the strict window in OD-2026-11-13-16, the ADR itself is not yet accepted). ADR-0081 to ADR-0083 are taken on the business-ids branch of feature 018.

Gated steps: [ADR-0089](ADR-0089-a-gated-step-waits-outside-ready-and-one-fenced-grant-routine-serves-every-waiter-by-key.md) — accepted (OD-2026-11-13-19); a step with node-level `concurrency` or `rateLimit` waits outside `READY` for every readiness source, and one fenced grant routine driven by key, with a reservation rule, serves every waiter whatever its gates (supersedes ADR-0056 decisions 3 and 5, refines ADR-0088 decisions 3 and 7).

Run input validation: [ADR-0090](ADR-0090-every-run-input-is-checked-against-its-pinned-version-schema-before-the-run-is-created.md) — accepted (OD-2026-11-14-15; design 043; OD-2026-11-14-2, -8 to -11 and -14); every path that creates a run checks its input against the pinned version's `inputSchema` before the creation transaction, through one gate, with a documented subset of JSON Schema validated by the platform's own validator with a work counter, no added statement in `enforce`, a value-free final refusal per creator, and the `enforce` mode required through the instance registry.

Run input validation, refined in the build: [ADR-0092](ADR-0092-the-application-names-a-runtime-only-for-the-blob-source-parse-and-the-run-input-rules-refined-in-the-build.md) — accepted (OD-2026-11-16-4; design 043, D043-154); refines ADR-0090 and ADR-0050 decision 2: `sagat-application` names `tokio` (`rt` alone) only in `collection_source.rs` for design 042's blocking parse, every composition root wires the gate into each creator, the 1 KiB name limit and the cache share of decision 5, `/` for every repeat and trigger-now (decision 8), and the audit of a `report`-mode start an open holder answers (decision 10); a stricter subset ships `run-input-validation-v2` (decision 6, proposed, O043-008); a refused run input is an `INPUT_INVALID` action whose time is acted on, not owed (decision 7).

## Every decision

Each record, in number order, with the status written at its top (shortened to its first clause; the file has the
rest). Numbers 0011, 0034, 0041 to 0049 and 0091 have no file on `develop`; numbers are shared by every branch (rule 1
above), so check the other branches before taking one. The [template](TEMPLATE.md) is the starting point for a new record.

| Number | Decision | Status |
|---|---|---|
| 0001 | [Rust and modular monolith](ADR-0001-rust-modular-monolith.md) | Accepted for initial design |
| 0002 | [PostgreSQL as source of truth](ADR-0002-postgres-authority-outbox.md) | Accepted for initial design |
| 0003 | [Declarative IR as visual and execution contract](ADR-0003-declarative-ir.md) | Accepted for initial design |
| 0004 | [At-least-once semantics for external effects](ADR-0004-external-effects-at-least-once.md) | Accepted for initial design |
| 0005 | [Native and auditable compensation](ADR-0005-compensation-native.md) | Accepted for initial design |
| 0006 | [Scripts as untrusted execution](ADR-0006-untrusted-code-sandbox.md) | Accepted for initial design |
| 0007 | [Common SDK protocol](ADR-0007-sdk-protocol-parity.md) | Accepted for initial design |
| 0008 | [Runtime graph as a journal projection](ADR-0008-runtime-event-projection.md) | Accepted for initial design |
| 0009 | [Single region with HA before active-active](ADR-0009-single-region-first.md) | Accepted for initial design |
| 0010 | [Compensations and reconciliations are derived steps of the definition](ADR-0010-compensations-are-derived-steps.md) | Accepted |
| 0012 | [Events pass through a durable inbox and a durable delivery table; matching is asynchronous and idempotent](ADR-0012-events-pass-through-a-durable-inbox-and-delivery-table.md) | Accepted |
| 0013 | [Scripts run in a container of their own, started by a runner the control plane never lets near its database](ADR-0013-scripts-run-in-containers-the-control-plane-never-touches.md) | Accepted |
| 0014 | [Transactions begin in a task of their own, because sqlx 0.9.0 can be cut off halfway](ADR-0014-transactions-begin-where-nothing-can-cut-them-off.md) | Accepted |
| 0015 | [SDKs meet the server by minor version of the worker protocol, and are held to one conformance suite](ADR-0015-sdks-meet-the-server-by-minor-version-and-are-held-to-one-conformance-suite.md) | Accepted |
| 0016 | [The Studio is a same-origin app over the REST API; the server validates and compares, and live views follow the journal by sequence](ADR-0016-the-studio-is-a-same-origin-app-over-the-rest-api-that-follows-runs-by-journal-sequence.md) | Accepted |
| 0017 | [Telemetry is derived from the journal, never produced on the write path; conditions that need a person are found by looking](ADR-0017-telemetry-is-derived-from-the-journal-and-conditions-are-found-by-looking.md) | Accepted |
| 0018 | [Publishing is one transaction, the active pointer has a history that is only added to, and an approval is of a document](ADR-0018-publishing-is-one-transaction-the-pointer-has-a-history-and-an-approval-is-of-a-document.md) | Accepted |
| 0019 | [The first day runs on the real engine with canned answers, a development identity provider is a separate thing that refuses to run anywhere else, and the documentation is checked by the tool it describes](ADR-0019-the-first-day-runs-on-the-real-engine-with-canned-answers-and-a-development-identity-provider.md) | Accepted |
| 0020 | [The scheduler's work is sharded by run and owned by lease with a fence in the action's transaction, a pass shares its batch between tenants by weight, and what scales is measured before it is promised](ADR-0020-the-scheduler-is-sharded-by-run-and-owned-by-lease-with-a-fence.md) | Accepted |
| 0021 | [C# builds use a curated offline toolchain and signed binary artifacts](ADR-0021-csharp-builds-use-a-curated-offline-toolchain.md) | Accepted |
| 0022 | [Supervisor denials commit before acknowledgement](ADR-0022-supervisor-denials-commit-before-acknowledgement.md) | Accepted |
| 0023 | [Large payloads live in a provider-neutral blob store and travel as descriptors](ADR-0023-blob-store-claim-check.md) | Accepted |
| 0024 | [Go and Java use signed v3 artifacts and dedicated trusted hosts](ADR-0024-go-and-java-use-native-v3-artifacts.md) | Accepted |
| 0025 | [Outbound brokers share the durable delivery pipeline](ADR-0025-outbound-brokers-share-the-durable-delivery-pipeline.md) | Accepted |
| 0026 | [Authenticate webhook identities at the transport boundary](ADR-0026-webhook-identity-profiles.md) | Accepted and implemented |
| 0027 | [Durable content, log, rate and retention quotas](ADR-0027-durable-content-rate-and-retention-quotas.md) | Accepted |
| 0028 | [Authorized structural payload inspection](ADR-0028-authorized-structural-payload-inspection.md) | Accepted |
| 0029 | [Worker task streams spend finite receive credit](ADR-0029-worker-task-streams-spend-finite-receive-credit.md) | Accepted and implemented |
| 0030 | [Structured instances share the reducer and continuation creates an explicit chain](ADR-0030-structured-instances-and-continuation-chains.md) | Accepted architecture; its version decision is superseded by decision 0032 |
| 0031 | [Declared operations disclose only their authorized business results](ADR-0031-declared-operation-result-disclosure.md) | Accepted policy |
| 0032 | [Extend the existing workflow IR additively](ADR-0032-additive-workflow-ir.md) | Accepted architecture |
| 0033 | [Payload values are classified per field, and an unmarked field is personal data](ADR-0033-payload-values-are-classified-per-field.md) | Accepted direction |
| 0035 | [Reusable database connections and managed execution](ADR-0035-database-connections-and-managed-execution.md) | Accepted |
| 0036 | [Integrate scoped execution, managed databases and classified disclosure](ADR-0036-integrated-scoped-managed-and-classified-execution.md) | Accepted implementation reconciliation (2026-10-17) |
| 0037 | [Stable parent identity guards in child-chain handoff](ADR-0037-stable-parent-identity-guards-for-child-handoff.md) | Accepted scoped clarification |
| 0038 | [Production incidents follow execution lock entry](ADR-0038-production-incidents-follow-execution-lock-entry.md) | Accepted scoped correction |
| 0039 | [Stable rerun source identity guards](ADR-0039-stable-rerun-source-identity-guards.md) | Accepted scoped correction |
| 0040 | [Release checkpoints preserve incomplete feature boundaries](ADR-0040-release-checkpoints-preserve-incomplete-feature-boundaries.md) | Accepted |
| 0050 | [Many-run operations admit runs under stable keys in one lock-ordered transaction, and never buffer a signal or reopen a run](ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md) | Accepted |
| 0051 | [Deployment routing, task-token completion and push invocation are decided on the canonical attempt lease](ADR-0051-dispatch-modes-are-variations-of-the-attempt-lease.md) | Proposed |
| 0052 | [Warm script containers are an opt-in, per-artifact exception to one container per run](ADR-0052-warm-script-containers-are-an-opt-in-exception-to-one-container-per-run.md) | Proposed |
| 0053 | [Analytics is a journal-derived projection delivered to pluggable sinks through fenced per-sink cursors](ADR-0053-analytics-is-a-journal-derived-projection-delivered-through-per-sink-cursors.md) | Proposed |
| 0054 | [Run content leaves the hot store only through a verified archive under a namespace retention policy, and the run row stays as a stub](ADR-0054-run-content-leaves-the-hot-store-only-through-a-verified-archive-under-a-namespace-retention-policy.md) | Proposed |
| 0055 | [Integration resources reuse the managed-connection model, run in a separate managed executor, and new triggers are kinds of the existing inbox sources](ADR-0055-integration-resources-and-inbox-connector-kinds.md) | Accepted (OD-2026-11-24-4) |
| 0056 | [Concurrency keys are durable permits taken after the run lock, and a queued run is granted oldest first by its own shard owner](ADR-0056-concurrency-keys-are-durable-permits-granted-by-the-waiters-shard-owner.md) | Accepted |
| 0057 | [Additional script languages reuse the container-per-run sandbox, the signed artifact families and closed offline catalogs](ADR-0057-additional-script-languages-reuse-the-sandbox-and-signed-artifacts.md) | Accepted (OD-2026-11-24-4) |
| 0058 | [Engine-local data transforms run in per-attempt sandboxes over immutable snapshots, and assets are published by an atomic PostgreSQL pointer](ADR-0058-sandboxed-data-transforms-and-pointer-publication.md) | Proposed |
| 0059 | [An agent step is an engine-driven loop whose every model call and tool call is a durable derived step, run outside the control plane within reserved budgets and a declared data ceiling](ADR-0059-agent-steps-are-engine-driven-loops-of-durable-derived-steps.md) | Proposed |
| 0060 | [The MCP server exposes only declared tools through a classified result projection, and the MCP client calls only pinned tools of registered servers](ADR-0060-mcp-exposes-declared-tools-and-calls-pinned-tools.md) | Proposed |
| 0061 | [A link carries one narrow capability: an anonymous submitter may only submit its form, and a decision link never authorizes a decision](ADR-0061-link-bearers-hold-one-narrow-capability-and-never-decide.md) | Accepted (OD-2026-11-24-4) |
| 0062 | [An internal app narrows its viewer's own authority through a server-side session, and author code runs only in an opaque-origin frame](ADR-0062-internal-apps-narrow-the-viewers-authority-and-confine-custom-code.md) | Proposed |
| 0063 | [Workflow time and infrastructure time are separate classes, and only a test deployment may move workflow time](ADR-0063-only-a-test-deployment-moves-workflow-time.md) | Proposed |
| 0064 | [Access grants only add; only a restricted folder or an operation list narrows them; one evaluator decides every operation](ADR-0064-access-grants-add-and-only-restricted-folders-and-operation-lists-narrow.md) | Proposed |
| 0065 | [Cross-namespace service operations cross the boundary only through durable operation messages, under a pinned contract, an endpoint allow-list and a declared result disclosure](ADR-0065-cross-namespace-operations-cross-only-through-durable-operation-messages.md) | Proposed |
| 0066 | [Payload encryption is client-side and declared; the engine never holds a codec key and treats encoded values as opaque](ADR-0066-client-side-payload-encryption.md) | Proposed |
| 0067 | [A cell has one writable region at a time, fails over active-passive by an operator's activation, and fences everything granted before it with a region epoch](ADR-0067-a-cell-has-one-writable-region-and-fails-over-by-a-fenced-epoch.md) | Proposed (direction accepted, Owner decision 2026-11-24-2; acceptance waits for the region drill and the commit-latency benchmark) |
| 0068 | [Workflow Stream events are a fenced, budgeted, value-carrying log beside the journal, read only through a classified feed](ADR-0068-workflow-streams-are-a-fenced-classified-log-beside-the-journal.md) | Accepted (OD-2026-11-24-4) |
| 0069 | [Redis Streams share outbox and delivery authority](ADR-0069-redis-streams-share-outbox-and-delivery-authority.md) | Accepted for implementation |
| 0070 | [Rust shares the canonical SDK contract](ADR-0070-rust-sdk-shares-the-canonical-sdk-contract.md) | Accepted for implementation, at the owner's request of 2026-10-27 |
| 0071 | [Lake writes and step-output destinations are managed effects recovered at the destination](ADR-0071-lake-writes-and-destinations-are-managed-effects-recovered-at-the-destination.md) | Accepted |
| 0072 | [Scaling decisions are a deterministic evaluation, recorded with their inputs and published; the platform applies only what lives inside a replica](ADR-0072-scaling-decisions-are-deterministic-recorded-and-published-not-applied-by-the-platform.md) | Proposed |
| 0073 | [Connector partitions are shared by rank among the replicas that read them, and a connector's lag says why it is not draining](ADR-0073-connector-partitions-are-shared-by-rank-and-lag-says-why-a-consumer-is-not-draining.md) | Proposed |
| 0074 | [Platform operators act through their own listener and principal, and outside data comes from allowlisted, read-only sources](ADR-0074-platform-operators-act-through-their-own-listener-and-outside-data-comes-from-allowlisted-sources.md) | Proposed |
| 0075 | [Worker leases retain ChildRoot possession through commit](ADR-0075-worker-leases-retain-child-root-possession.md) | Accepted |
| 0076 | [Compensation failure and uncertain effects require explicit recovery authority](ADR-0076-compensation-failure-and-uncertain-effects.md) | Accepted on 2026-11-01 for its decisions 1 to 3, 5 and 7 |
| 0077 | [EGM design docs replace Spec Kit as the only specification workflow](ADR-0077-egm-replaces-spec-kit.md) | Proposed |
| 0078 | [The ChildRoot capacity group header is a lock nobody waits for, and possession moves only through fenced commands](ADR-0078-capacity-group-lock-order-and-possession-protocol.md) | Proposed |
| 0079 | [The lake runner asks the sandbox runner for an attempt, never for a container](ADR-0079-the-lake-runner-asks-the-sandbox-runner-for-an-attempt-never-for-a-container.md) | Proposed |
| 0080 | [A node that only the platform may write is a private document, not a member of the public schema](ADR-0080-a-node-only-the-platform-may-write-is-a-private-document.md) | Proposed |
| 0081 | [`CANCEL_SOURCE` is switched on per process, engine first, until the instance-capability registry exists](ADR-0081-cancel-source-is-switched-on-per-process-until-the-capability-registry-exists.md) | Accepted |
| 0082 | [The instance registry records requirements only for the operator, a refused process stays up with a closed gate, and it learns of a requirement within one heartbeat](ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md) | Accepted |
| 0083 | [The cron triggers are handed to the schedule pass once, and only when their schedules have converged](ADR-0083-the-cron-triggers-are-handed-to-the-schedule-pass-once-and-only-when-they-have-converged.md) | Accepted |
| 0084 | [Step logs are a bounded, lossy, per-attempt record beside the journal](ADR-0084-step-logs-are-a-bounded-lossy-per-attempt-record-beside-the-journal.md) | Accepted |
| 0085 | [Step-log text is read under its own permission and redacted by value in the worker, by shape on the server](ADR-0085-step-log-text-is-read-under-its-own-permission-and-redacted-by-value-in-the-worker.md) | Accepted |
| 0086 | [Operator chains on triggers are declared data, their state is rows on the database clock, and a fenced pass seals a window that starts one run by its identity](ADR-0086-operator-chains-on-triggers-are-declared-data-and-a-fenced-pass-seals-a-window-that-starts-one-run.md) | Accepted |
| 0087 | [Collection operators are applied once at scope entry and the snapshot is the record](ADR-0087-collection-operators-are-applied-once-at-scope-entry-and-the-snapshot-is-the-record.md) | Accepted |
| 0088 | [Rate limits are a durable window of grants on the database clock, buffered work is granted by its shard owner's scheduler pass, and a claim never reads the window](ADR-0088-rate-limits-are-a-durable-window-of-grants-on-the-database-clock-and-the-scheduler-grants-never-the-claim.md) | Accepted |
| 0089 | [A gated step waits outside READY, and one fenced grant routine, driven by key, serves every waiter whatever its gates](ADR-0089-a-gated-step-waits-outside-ready-and-one-fenced-grant-routine-serves-every-waiter-by-key.md) | Accepted |
| 0090 | [Every run input is checked against its pinned version's schema before the run is created](ADR-0090-every-run-input-is-checked-against-its-pinned-version-schema-before-the-run-is-created.md) | Accepted |
| 0092 | [The application names a runtime only for the blob-source parse, and the run-input rules refined in the build](ADR-0092-the-application-names-a-runtime-only-for-the-blob-source-parse-and-the-run-input-rules-refined-in-the-build.md) | Accepted |
| 0120 | [TypeScript and Python packages are resolved at publication through the platform's package gateway and run from a signed, read-only, content-addressed layer](ADR-0120-script-packages-are-resolved-at-publication-through-the-package-gateway-and-run-from-a-signed-read-only-layer.md) | Proposed |
| 0121 | [A namespace supply-chain policy gates publication, and a malicious-package advisory stops leases](ADR-0121-a-namespace-supply-chain-policy-gates-publication-and-a-malicious-package-advisory-stops-leases.md) | Proposed |
| 0122 | [OAuth tokens are held by a broker in the control plane, with values in the secret authority, and executors receive only a current access token](ADR-0122-oauth-tokens-are-held-by-a-broker-in-the-control-plane-and-executors-receive-only-a-current-access-token.md) | Proposed |
| 0123 | [Hub integrations are signed data that specialise closed operation kinds, and are imported as pinned versions](ADR-0123-hub-integrations-are-signed-data-that-specialise-closed-operation-kinds-and-are-imported-as-pinned-versions.md) | Proposed |
