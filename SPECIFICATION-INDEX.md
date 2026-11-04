# Specification index — Sagat Octopus Orchestrator

This index covers the feature specifications present in this checkout. A checked task is a
record of claimed completion with its linked evidence; it is not a new test run or release approval.
For acceptance limits, read the feature's spec, research register and evidence together with
[requirement traceability](docs/traceability.md). Features on sibling branches are not included.

## Feature records

| Feature | Specification | Delivery record |
|---|---|---|
| 001-platform-foundation | [Platform foundation and contracts](specs/001-platform-foundation/spec.md) | [Plan](specs/001-platform-foundation/plan.md), [tasks](specs/001-platform-foundation/tasks.md), [research](specs/001-platform-foundation/research.md) |
| 002-durable-execution | [Durable execution engine and scheduler](specs/002-durable-execution/spec.md) | [Plan](specs/002-durable-execution/plan.md), [tasks](specs/002-durable-execution/tasks.md), [research](specs/002-durable-execution/research.md) |
| 003-sagas-compensations | [Native sagas and compensations](specs/003-sagas-compensations/spec.md) | [Plan](specs/003-sagas-compensations/plan.md), [tasks](specs/003-sagas-compensations/tasks.md), [research](specs/003-sagas-compensations/research.md) |
| 004-events-webhooks-messaging | [Triggers, webhooks, messaging and correlation](specs/004-events-webhooks-messaging/spec.md) | [Plan](specs/004-events-webhooks-messaging/plan.md), [tasks](specs/004-events-webhooks-messaging/tasks.md), [research](specs/004-events-webhooks-messaging/research.md) |
| 005-inline-code-sandbox | [Script registry, code editor and inline execution](specs/005-inline-code-sandbox/spec.md) | [Plan](specs/005-inline-code-sandbox/plan.md), [tasks](specs/005-inline-code-sandbox/tasks.md), [research](specs/005-inline-code-sandbox/research.md) |
| 006-sdk-multilanguage | [TypeScript, Go, Python, C# and Java SDKs](specs/006-sdk-multilanguage/spec.md) | [Plan](specs/006-sdk-multilanguage/plan.md), [tasks](specs/006-sdk-multilanguage/tasks.md), [research](specs/006-sdk-multilanguage/research.md) |
| 007-workflow-studio | [Visual Studio, Monaco and runtime graph](specs/007-workflow-studio/spec.md) | [Plan](specs/007-workflow-studio/plan.md), [tasks](specs/007-workflow-studio/tasks.md), [research](specs/007-workflow-studio/research.md) |
| 008-observability-operations | [Distributed observability and operations](specs/008-observability-operations/spec.md) | [Plan](specs/008-observability-operations/plan.md), [tasks](specs/008-observability-operations/tasks.md), [research](specs/008-observability-operations/research.md) |
| 009-security-multitenancy | [Security, isolation and multi-tenant governance](specs/009-security-multitenancy/spec.md) | [Plan](specs/009-security-multitenancy/plan.md), [tasks](specs/009-security-multitenancy/tasks.md), [research](specs/009-security-multitenancy/research.md) |
| 010-scalability-ha | [Performance, scalability and high availability](specs/010-scalability-ha/spec.md) | [Plan](specs/010-scalability-ha/plan.md), [tasks](specs/010-scalability-ha/tasks.md), [research](specs/010-scalability-ha/research.md) |
| 011-gitops-versioning | [GitOps, versioning and definition promotion](specs/011-gitops-versioning/spec.md) | [Plan](specs/011-gitops-versioning/plan.md), [tasks](specs/011-gitops-versioning/tasks.md), [research](specs/011-gitops-versioning/research.md) |
| 012-developer-experience | [CLI, quickstarts, local testing and documentation](specs/012-developer-experience/spec.md) | [Plan](specs/012-developer-experience/plan.md), [tasks](specs/012-developer-experience/tasks.md), [research](specs/012-developer-experience/research.md) |
| 014-workflow-primitives | [Durable workflow primitives and practical SDK examples](specs/014-workflow-primitives/spec.md) | [Plan](specs/014-workflow-primitives/plan.md), [tasks](specs/014-workflow-primitives/tasks.md), [research](specs/014-workflow-primitives/research.md) |
| 016-payload-classification | [Per-field payload classification](specs/016-payload-classification/spec.md) | [Plan](specs/016-payload-classification/plan.md), [tasks](specs/016-payload-classification/tasks.md), [research](specs/016-payload-classification/research.md) |
| 017-database-connections | [Database connections, OpenBao secrets and shared execution](specs/017-database-connections/spec.md) | [Plan](specs/017-database-connections/plan.md), [tasks](specs/017-database-connections/tasks.md), [research](specs/017-database-connections/research.md) |
| 033-redis-streams | [Redis Streams for internal notifications and outbound callbacks](specs/033-redis-streams/spec.md) | [Plan](specs/033-redis-streams/plan.md), [tasks](specs/033-redis-streams/tasks.md), [research](specs/033-redis-streams/research.md) |
| 034-rust-sdk | [Official Rust SDK — release0.3](specs/034-rust-sdk/spec.md) | [Plan](specs/034-rust-sdk/plan.md), [tasks](specs/034-rust-sdk/tasks.md), [research](specs/034-rust-sdk/research.md) |

## Approved delivery in verification

[037 — Compensation safety](specs/037-compensation-safety/spec.md) records B02 S1-B/S2-B
and delivery 1 approved on 2026-11-01. Implementation is in verification, not release acceptance.
[Research](specs/037-compensation-safety/research.md) and
[validation limits](specs/037-compensation-safety/validation-report.md) separate the baseline
characterization tests from new behavior. The approved [plan](specs/037-compensation-safety/plan.md),
[tasks](specs/037-compensation-safety/tasks.md) and [coverage](specs/037-compensation-safety/analysis.md)
separate conservative delivery 1 from the still-open provider-qualification gate. Delivery-1
acceptance remains pending; US04 has no selected or qualified provider.

## Current scope and limits

The checkout contains the durable engine, sagas, events and broker adapters, isolated script
execution, six SDKs, Studio, tenant/security controls, telemetry, GitOps and developer tooling.
Later records cover workflow primitives (014), payload classification (016), managed PostgreSQL
connections (017), Redis transport/callbacks (033) and the Rust SDK (034).

Feature 014 explicitly retains partial implementation and production-activation gates.
Feature 033 retains a final integrated validation task. Read their task records for current
acceptance; source presence and previously recorded results do not prove release readiness.
Performance and disaster-recovery reports describe their stated environments, not universal SLOs.

## Previously recorded product and validation questions

These questions are preserved from the previous index pending explicit disposition. Their inclusion
does not establish that every implementation gap is still present; consult the linked feature
evidence and owner decisions before treating an item as a new requirement.

- [017 Database Connections](specs/017-database-connections/spec.md): PostgreSQL connections configured in the UI, OpenBao secrets in the first delivery, and shared SDK/visual/ad hoc execution. User approved implementation on 2026-10-16; [ADR-0035](docs/adr/ADR-0035-database-connections-and-managed-execution.md) is accepted and the feature plan/tasks track implementation. Product completion remains pending runtime evidence.

- Choose registry ownership, signing and release policy for the SDK packages and the `sagat` binary.
- Choose production identity/KMS/object-storage providers and the retention/classification tiers.
- Define a distributed performance testbed that separates workers, API and PostgreSQL; the checked-in reports are single-machine evidence and do not prove 5 000 transitions/s.
- Run a full product/KEDA deployment, node-drain exercise and real dependency-protocol checks; current cluster evidence covers NetworkPolicy canaries and restricted Pod admission only.
- Conduct the newcomer timing and human accessibility studies that automation cannot replace.
- Approve financial and compensation policies per application domain.

## Authoritative navigation

- [Product and source-checkout usage](README.md).
- [Approved principles](docs/constitution.md).
- New features are described in [`docs/designs/`](docs/designs/README.md); existing features keep `specs/NNN-slug/` until they are converted.
- [Current architecture](docs/architecture/overview.md) and [decision records](docs/adr/README.md).
- [Canonical contracts](specs/001-platform-foundation/contracts/).
- [SDK semantic contract](sdks/SPEC.md) and [compatibility evidence](docs/sdk-compatibility.md).
- [Owner decisions](docs/owner-decisions.md), [roadmap](docs/roadmap.md) and [release notes](docs/releases/unreleased.md).
- [Engineering workflow](docs/engineering/sdd-workflow.md) and [test strategy](docs/test-strategy.md).

Feature numbering is shared across branches and worktrees. Missing numbers in this checkout are
not available numbers by default. Preserve historical requirement IDs and dated validation claims.

## Proposed parity features 018–032 (drafts, 2026-10-26/27)

Fifteen draft features bring Temporal and Windmill capabilities into the platform's architecture. Each has a
spec, plan, research, data model, contract fragments, quickstart and tasks; none is approved or implemented,
and every ADR they propose is `Proposed`. They were written in parallel, reviewed independently (architecture,
security and cross-feature consistency) and revised. Suggested order and gates: [docs/roadmap.md](docs/roadmap.md).

| Feature | Scope | Proposed ADRs |
|---|---|---|
| [018](specs/018-schedules-bulk-operations/spec.md) | schedules, batch operations, reset to a point, Signal/Update-with-Start, business ids | ADR-0050 |
| [019](specs/019-worker-deployments-dispatch/spec.md) | worker deployment versions, priority and fairness keys, checkpoints, task tokens, push dispatch, warm script containers | ADR-0051, ADR-0052 |
| [020](specs/020-visibility-analytics-archival/spec.md) | search attributes and list queries, journal-derived analytics sinks, archival and retention | ADR-0053, ADR-0054 |
| [021](specs/021-integration-resources-triggers/spec.md) | resources, managed operations, variables, new trigger kinds | ADR-0055 |
| [022](specs/022-flow-control-step-policies/spec.md) | step cache, early return, failure handlers, concurrency keys, standalone steps, draft test mode | ADR-0056 |
| [023](specs/023-script-languages/spec.md) | Bash, PowerShell, PHP, Rust scripts; `python-data` runtime mechanism | ADR-0057 |
| [024](specs/024-data-engines-pipelines/spec.md) | MySQL/MariaDB, SQL Server, BigQuery, Snowflake, DuckDB; data assets and pipelines | ADR-0058 |
| [025](specs/025-ai-agents-mcp/spec.md) | durable agent step, MCP server and client | ADR-0059, ADR-0060 |
| [026](specs/026-invocation-human-surfaces/spec.md) | forms and public links, run-and-wait, per-workflow OpenAPI, Workflow Streams, channel approvals and notifications | ADR-0061, ADR-0068 |
| [027](specs/027-internal-apps/spec.md) | bounded internal app builder | ADR-0062 |
| [028](specs/028-developer-tooling/spec.md) | time-skipping tests, interceptors, VS Code, Git sync, curated template hub | ADR-0063 |
| [029](specs/029-custom-roles-folders/spec.md) | custom roles, groups, folders, operation lists | ADR-0064 |
| [030](specs/030-cross-namespace-services/spec.md) | cross-namespace service operations within a tenant | ADR-0065 |
| [031](specs/031-client-side-encryption/spec.md) | client-side payload encryption and codec server | ADR-0066 |
| [032](specs/032-multi-region-failover/spec.md) | active-passive multi-region cells | ADR-0067 |

**Owner decisions that gate the drafts** (each recorded as an `O…` question in the feature's `research.md`):
the amendment of constitution principle I for retention (020, ADR-0054) and of the Delivery Constraints for
multi-region (032, ADR-0067); the scope line of `docs/architecture/overview.md` that defers a low-code designer
(027, O1360); one rule for `pii` leaving the platform (020, O1081, applied by 020/025/026/031); the placement
rule for customer-credentialed calls (021, ADR-0055); and acceptance of every proposed ADR.

**Cross-feature registries and their owners.** Each concern has one owner; the others cite it.

| Concern | Owner |
|---|---|
| Reset rule of every journal event type | 018 (`contracts/journal-and-events.md`) |
| Merged admission-transaction lock order | 018 (ADR-0050 decision 7) |
| Instance-capability registry and fleet activations (`schedules-v1`, `access-narrowing-v1`, `payload-encoding-v1`) | 018 (FR-018-052, ADR-0050 decision 9) |
| Where customer-credentialed calls run | 021 (ADR-0055) |
| Content stores and their purge/erasure rules; `pii` egress rule | 020 |
| Encoded-field positions, carriers and write paths | 031 |
| Time-class catalog (workflow vs infrastructure time) | 028 (ADR-0063) |
| Studio origins and CSP (refines ADR-0016) | 027 (ADR-0062) |
| Keys a cell holds | 032 |
| Script host calls (`checkpoint`, `stream.append`) | 023 |
| The only payload-transforming SDK stage | 031 (028 interceptors only observe and decorate) |

**Amendments to accepted work, stated as proposals:** FR-016-002's closed carrier list and classification
rules version 2 (one shared amendment, opened by the first of these features to be implemented, for every
carrier the drafts add); 014's approval owner kinds (025 owns, 026 cites); ADR-0009, ADR-0013, ADR-0016,
ADR-0023, ADR-0027, ADR-0033, ADR-0035 and ADR-0036 where each draft's ADR says "supersedes in part" or
"refines"; constitution principle I and the Delivery Constraints.

**Provisional at implementation time:** worker-protocol minor numbers (1.6 is current; drafts that write "1.7"
take the next free minor in merge order), migration timestamps, and the permission catalog order. Numbers
reserved for these drafts: ADR-0050..ADR-0068; decisions, gaps and owner questions D/G/O 1000–1599, with
overflow 1600–1674.

## Commitment terminology

`MUST` = design requirement. `V1` = first full implementation of the specified scope. `P2/later` = explicit roadmap. A checked task names implementation evidence, but it does not erase an explicit partial/open requirement in `docs/traceability.md` or a gap in the feature's `research.md`.
