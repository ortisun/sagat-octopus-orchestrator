# Implementation Plan: Script registry, code editor and inline execution

**Branch:** `005-inline-code-sandbox` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Allow writing and publishing code directly on the platform without manual application build/deploy.

**Technical approach:** ScriptRegistry controls src/digest and the build pipeline; the sandbox runner manages isolated containers. TypeScript/Python, compiled C#, native Go and Java 17 bytecode execute outside privileged processes. Artifact v3 adds exact target/ABI/encoding metadata without changing stored v1/v2 artifacts. Go is advertised only after a native production-path startup probe for one configured homogeneous pool target.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** Cold/warm start metrics per language; caches only if policy/digest are identical; builds separate from execution.
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

The plan proposed `crates/script-registry`, `crates/sandbox`, `apps/worker-supervisor` and a Studio editor. As built (ADR-0013, ADR-0021, G109, G110), by the same rule as feature 004: a crate boundary has to buy isolation, and these would have had to import the whole run state or the whole application layer for none.

```text
crates/domain/src/scripts.rs                       languages, source, lockfile, digest, limits and policy, versions, failures, redaction
crates/domain/src/{identity,audit,quota}.rs        script:* permissions, the runner role, script.* audit, scripts / script_jobs quotas
crates/application/src/scripts.rs                  ScriptService (authors), RunnerService (the runner), ports ScriptStore / RunnerStore / ScriptLookup
crates/application/src/sandbox.rs                  Sandbox and HostCalls ports, the build key
crates/application/src/script_signing.rs           Ed25519 signer and verifier of artifacts
crates/infra-sandbox/                              the container driver, JSON-line protocol, lockfile policy and Node/Python shims
crates/infra-schema/                               JSON Schema plus v1 text/v2 binary/v3 target-aware artifact validation before signing
runners/dotnet/                                    pinned Roslyn build host, closed NuGet catalog and separate .NET runtime host
runners/go/                                        fixed-module Go builder, prebuilt read-only std cache and sealed-memfd runtime loader
runners/java/                                      fixed javac 17 builder, trusted API/Jackson and parent-first in-memory runtime host
crates/infra-postgres/src/scripts.rs               the SQL; migration 20261014000000_scripts
crates/infra-ir/src/convert.rs                     a script node becomes the activity script.sha256-<hex>
crates/api-http/src/routes/{scripts,runner}.rs     25 operations
apps/control-plane                                 SAGAT_SCRIPT_SIGNING_KEY, SAGAT_SCRIPT_CEILING, `script-key`
apps/sandbox-runner/                               the runner: builds, test runs and script steps in containers
infra/docker/                                      the runner image and the `sandbox` compose profile
tests/integration/inline-code-sandbox/{registry,sandbox}/   PostgreSQL (versions, leases, quotas, retention, migration) and real containers (isolation)
tests/e2e/inline-code-sandbox/                 US01..US04 through the API, the worker protocol and the runner
tests/fault/inline-code-sandbox/               runner death, restart, engine down, cancellation, room
tests/contract/inline-code-sandbox/rest/       every operation against openapi.yaml
tests/performance/inline-code-sandbox/         the benchmark; report in tests/performance/reports/
```

## Implementation phases

1. Domain contracts and types, invariant validation + tests.
2. Persistence and port adapters + migration and integration tests.
3. APIs/command processing + versioning + authorization and observability.
4. Happy path flow with e2e tests.
5. Fault injection, redelivery, timeouts, concurrency and regression.
6. Security/performance review, documentation and release gate.
7. Enable each Go native target only after its own native compile-and-execute CI proof; keep runner pools homogeneous until target-aware claims exist.

## Data Model / Contracts

- [`data-model.md`](./data-model.md)
- Canonical contracts in `specs/001-platform-foundation/contracts/` with reviewed migrations.
- Applicable ADRs in `docs/adr/`.

## Risks & Mitigations

- **Functional risk:** dependency changed remotely. The C# catalog pins package identity, version, NuGet content hash, archive SHA-512 and selected assembly SHA-256 in the immutable builder image; customer builds have no registry access.
- **Operational risk:** throughput below target; perf profiling and scaling driven by metrics, never a promise without a benchmark.
- **Compatibility risk:** schema/SDK diverges; schema evolution and golden fixtures.
- **Native-target risk:** language-only claims cannot distinguish Go architecture; configure exactly one target per eligible runner pool, sign it in v3 and fail startup/readiness when required native capability is absent.
- **Artifact-parser risk:** compressed native code and class files are hostile input; validate canonical encoding, bounded expansion and structural invariants before signing and again at the runtime boundary.
- **Security risk:** tenant/sandbox boundary; adversarial tests and fail closed.

## Complexity Tracking

| Decision | Cost | Justification |
|---|---|---|
| Rust modular monolith | Learning curve | Centralized invariants and gradual evolution |
| DB transactional outbox | Extra reads/writes | Avoid losing a wakeup after commit |
| Multi-SDK via proto | More CI and tests | Real parity and low coupling |
| Native Go target per runner pool | Separate ARM64/AMD64 images and native proof | Current worker claims carry language but no target; homogeneity prevents incompatible scheduling |
| Go sealed-memory execution | Trusted loader and Linux-only path | Preserves the `noexec` scratch mount and default seccomp profile |
| Java class files instead of JAR/project | Narrower author surface | One inline class needs no manifests, plugins, dependency resolver or build tool |

## Delivery Gate

TS/Python/C#/Go/Java script goes from the editor to execution; malformed v3 and v1/v2 regression tests pass; escape and CPU/memory/network limits are verified. Every advertised Go target has a native startup-probe and CI proof. ARM64 proof does not enable AMD64.


## Approved workflow-primitives dependency (2026-10-14)

[ADR-0030](../../docs/adr/ADR-0030-structured-instances-and-continuation-chains.md) and
[ADR-0031](../../docs/adr/ADR-0031-declared-operation-result-disclosure.md) accept the feature-014
architecture and separate declared-result disclosure policy.
[ADR-0032](../../docs/adr/ADR-0032-additive-workflow-ir.md) keeps the workflow IR at v1 and
supersedes the earlier separate-revision allocation. Script attempt uniqueness, retention pins and capability addressing must distinguish scoped execution instances while preserving root identities. Structured bodies do not permit inline customer code, broader secrets/egress or a new sandbox execution path.

This approves the described dependency for implementation, not a runtime guarantee. Canonical
contracts, migration/replay, tenant/fault tests and applicable five-SDK evidence remain the
feature-014 release gates; existing feature behavior stays supported.
