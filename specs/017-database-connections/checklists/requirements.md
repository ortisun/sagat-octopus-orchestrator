# Feature 017 requirements checklist

## Specification quality

- [x] User-confirmed PostgreSQL-first scope and OpenBao-in-this-delivery choice recorded.
- [x] UI registration, inline/standalone secret management, SDK reuse and visual/ad hoc execution covered.
- [x] Existing capability distinguished from proposed schema/driver/executor behavior.
- [x] Functional requirements have unique FR-017 identifiers and future automated evidence mapping.
- [x] Objective, commands, repository structure, coding conventions, testing and boundaries documented.
- [x] Immutable connection revisions distinguished from runtime credential versions and authorization.
- [x] Cross-system provider save, unknown database outcome, partial UI save and migration covered.
- [x] Tenant boundary, private networking, typed errors, audit, OTel and resource limits explicit.
- [x] All five SDKs use common semantics and conformance scenarios.
- [x] Scope excludes provisioning, arbitrary SQL scripts and unproven dynamic-credential guarantees.
- [x] Related unmerged OpenBao/security drafts identified without claiming implementation.

## Gates before implementation

- [x] Human review of the specification and proposed ADR completed.
- [x] Operation representation, executor capabilities and result disclosure accepted.
- [x] Approved dependencies recorded in affected feature specifications/plans.
- [x] Exact OpenBao lifecycle/reconciliation/migration, deployment identity and limits planned.
- [x] `plan.md`, canonical contract changes and traced `tasks.md` reviewed.

## Delivery gates — see verification.md

- [x] Real PostgreSQL/OpenBao happy/failure and recovery suites pass.
- [x] Tenant/namespace, private-network, secret-canary, audit-failure and revocation suites pass.
- [x] Crash/duplicate/lost-broker/timeout-signal/failed-compensation/restart suites pass.
- [x] Five SDKs pass shared contract and runtime fixtures.
- [x] Browser E2E proves registration, partial-save recovery, workflows and ad hoc execution.
- [x] Measured resource/latency evidence, runbooks and backup/restore demo recorded.
