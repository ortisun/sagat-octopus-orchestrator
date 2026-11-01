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
- [Approved principles](.specify/memory/constitution.md).
- [Current architecture](docs/architecture/overview.md) and [decision records](docs/adr/README.md).
- [Canonical contracts](specs/001-platform-foundation/contracts/).
- [SDK semantic contract](sdks/SPEC.md) and [compatibility evidence](docs/sdk-compatibility.md).
- [Owner decisions](docs/owner-decisions.md), [roadmap](docs/roadmap.md) and [release notes](docs/releases/unreleased.md).
- [Engineering workflow](docs/engineering/sdd-workflow.md) and [test strategy](docs/test-strategy.md).

Feature numbering is shared across branches and worktrees. Missing numbers in this checkout are
not available numbers by default. Preserve historical requirement IDs and dated validation claims.

## Commitment terminology

`MUST` = design requirement. `V1` = first full implementation of the specified scope. `P2/later` = explicit roadmap. A checked task names implementation evidence, but it does not erase an explicit partial/open requirement in `docs/traceability.md` or a gap in the feature's `research.md`.
