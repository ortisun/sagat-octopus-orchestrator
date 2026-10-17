# Implementation Plan: Security, isolation and multi-tenant governance

**Branch:** `009-security-multitenancy` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Protect tenant workloads, secrets and untrusted code execution before offering inline scripts.

**Technical approach:** RBAC policy port in the application layer, verified claims, query filters + defenses-in-depth, sandbox executor without privileges and egress proxy.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** Quota approval on the critical path must not depend on an eventually consistent counter for a security limit.
- **Constraints:** single-region HA; state loss after ACK forbidden; side effects at-least-once; security-by-default.
- **Scale/Scope:** scale horizontally without keeping live processes for waits.

## Constitution Check

- [x] State and retry semantics documented (`I`).
- [x] Modularity/ports and adapters planned (`II`).
- [x] Saga and reversibility applied where relevant (`III`).
- [x] Compatibility and version pinning (`IV`).
- [x] Code/tenant boundary and secrets (`V`).
- [x] OTel and auditable journal (`VI`).
- [x] Redeliverable events and inbox/outbox (`VII`).
- [x] Fault injection/contract plan (`VIII`).

These checks indicate conformance **of the design**, not approval of the implementation.

## Project Structure

```text
crates/domain/src/identity.rs            new permissions and the role matrix (the plan's permissions.rs)
crates/domain/src/{quota,binding,secrets,egress}.rs   pure rules: limits, activity patterns, secret types and cipher port, egress policy
crates/application/src/{authz,quotas,bindings,secrets,egress}.rs   use cases and storage ports (authz.rs: the cross-tenant guard)
crates/infra-postgres/src/{quotas,bindings,secrets,egress}.rs      adapters (the plan's tenant_scopes.rs); migrations 20261010000000..300
crates/infra-secrets/                    AES-256-GCM key ring (SecretCipher), SecretsConfig
crates/infra-egress/                     guarded outbound client
crates/api-http/src/{governance,dto_gov}.rs, routes/governance.rs   REST routes for quotas, bindings, secrets, egress
crates/api-grpc/src/{limits,server}.rs   connection and call caps, TLS / mTLS; ResolveSecrets in service.rs
apps/control-plane, apps/execution-engine  SAGAT_SECRETS_KEYS, SAGAT_GRPC_TLS_*; `control-plane reseal-secrets`
docs/runbooks/security-governance.md     operations
tests/integration/security-multitenancy/   quotas, bindings, secrets, egress against PostgreSQL
tests/contract/security-multitenancy/      rest/ (conformance, cross-tenant battery), grpc/ (TLS, mTLS)
tests/e2e/security-multitenancy/           us01, us02, us04 through REST and gRPC
tests/fault/security-multitenancy/         connection kills around quota and secret transactions
tests/performance/security-multitenancy/   benchmark, report in tests/performance/reports/
infra/helm/templates/{networkpolicy,networkpolicy-egress}.yaml   integrated 010 chart plus 009 isolation
tests/security/sandbox/kubernetes-network-policy.sh              real Cilium/admission verifier
tests/contract/scalability-ha/                                   exact Helm role/peer/port contracts
```

Published dependency implementations are reused rather than duplicated: Feature 005 supplies the
production Docker sandbox used by the egress E2E/fault tests, and Feature 010 supplies the deployment
chart extended by the Feature 009 NetworkPolicies. The chart proof applies policies, service accounts
and controlled canaries; Feature 010 subsequently verified the full product/KEDA installation in a single-node kind/Cilium cluster (T010-038; `tests/performance/reports/kubernetes-validation.md`). Multi-node drain and zone failure remain unproven.

## Implementation phases

1. Domain contracts and types, invariant validation + tests.
2. Persistence and port adapters + migration and integration tests.
3. APIs/command processing + versioning + authorization and observability.
4. Happy path flow with e2e tests.
5. Fault injection, redelivery, timeouts, concurrency and regression.
6. Security/performance review, documentation and release gate.

## Data Model / Contracts

- [`data-model.md`](./data-model.md)
- Canonical contracts in `specs/001-platform-foundation/contracts/` with reviewed migrations.
- Applicable ADRs in `docs/adr/`.

## Risks & Mitigations

- **Functional risk:** tenant token spoofing. Mitigate via adversarial scenario with fixture.
- **Operational risk:** throughput below target; perf profiling and scaling driven by metrics, never a promise without a benchmark.
- **Compatibility risk:** schema/SDK diverges; schema evolution and golden fixtures.
- **Security risk:** tenant/sandbox boundary; adversarial tests and fail closed.

## Complexity Tracking

| Decision | Cost | Justification |
|---|---|---|
| Rust modular monolith | Learning curve | Centralized invariants and gradual evolution |
| DB transactional outbox | Extra reads/writes | Avoid losing a wakeup after commit |
| Multi-SDK via proto | More CI and tests | Real parity and low coupling |

## Delivery Gate

Battery of cross-tenant, SSRF, sandbox escape, secret leak, webhook replay and quota racing; fail the release if any critical finding exists.

## Durable quota completion

Follow [ADR-0027](../../docs/adr/ADR-0027-durable-content-rate-and-retention-quotas.md) and
T009-037..T009-040. Implement the classified byte ledger and shared policy fences first, then
authoritative event rate/retention admission, every content/log persistence and purge path, and
canonical namespace API admission. Each slice retains pending status until its real multi-replica,
rollback, tenant-boundary and terminal-state tests pass. A partial inbox/script counter cannot be
advertised as complete retained-content accounting.


## Approved workflow-primitives dependency (2026-10-14)

[ADR-0030](../../docs/adr/ADR-0030-structured-instances-and-continuation-chains.md) and
[ADR-0031](../../docs/adr/ADR-0031-declared-operation-result-disclosure.md) accept the feature-014
architecture and separate declared-result disclosure policy.
[ADR-0032](../../docs/adr/ADR-0032-additive-workflow-ir.md) keeps the workflow IR at v1 and
supersedes the earlier separate-revision allocation. Add the accepted built-in approval/read/decision and run/query/update grants, current authorization on receipt replay, tenant-scoped instance/chain references and atomic quotas/audit. ADR-0031 permits only explicitly declared bounded business results; ordinary structural inspection retains ADR-0028 and no custom/per-operation roles are invented.

This approves the described dependency for implementation, not a runtime guarantee. Canonical
contracts, migration/replay, tenant/fault tests and applicable five-SDK evidence remain the
feature-014 release gates; existing feature behavior stays supported.
