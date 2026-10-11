# Research & Decisions: CLI, quickstarts, local testing and documentation

## Main decision

Rust CLI with clap or a similar tool; templates tested in CI; versioned docs with source generators.

## Rationale

Enable safe adoption by product, backend and operations teams without manually configuring complex infrastructure. The decision respects the recovery, auditability, security and versioning properties defined in the Constitution.

## Alternatives considered

1. **Generic ad hoc queue-based solution:** rejected as the sole durability mechanism, since message delivery is not equivalent to a transaction over workflow state.
2. **Independent services per feature from day 1:** postponed to avoid operational and consistency complexity without a benchmark.
3. **SDK-specific logic:** rejected for core invariants, to avoid behavior differences between languages.

## Internal references

- `docs/adr/` (architectural decisions)
- `docs/architecture/overview.md`
- `docs/test-strategy.md`
- `specs/001-platform-foundation/contracts/`

## Risks and mitigation

- Complexity/scope risk: deliver P1 user stories separately, running contract tests.
- Semantic ambiguity risk: encode reducers and golden fixtures, do not decide via UI alone.
- Crash/redelivery risk: test harness with real failures at checkpoints.
- Security risk: specific pentest and tenant isolation from the first milestone.

## Questions for spike validation

- Latency and overhead benchmarks on the critical path defined for this feature.
- Detail compliance and retention requirements according to the deployment environment.

## Foundation audit — 2026-10-11 (T012-001)

Baseline: `7c63100`. This audit covers the committed CLI, not concurrent working-tree changes.

| Requirement | Existing artifact | Remaining proof |
|---|---|---|
| FR-012-001 commands | `apps/cli/src/{workflow,script,worker,args}.rs` | End-to-end execution and packaging; the binary is `sagat`, whereas the spec names `df`. Naming remains unresolved; no silent rename or second implementation. |
| FR-012-002 five quickstarts | SDKs from 004 and existing examples | Execute each documented quickstart, particularly TS and C#, and inspect its trace. |
| FR-012-003 reproducible local stack | `infra/docker/compose.yaml`, real engine | Compose happy/failure scenarios; no independent emulator semantics. |
| FR-012-004 templates | `apps/cli/src/new.rs`, six templates | Unit validation passes; trigger/cron configuration still needs platform execution. |
| FR-012-005 generated docs | Canonical 001 contracts and `docs/quickstarts/errors.md` | Generated documentation and executable examples in CI. |
| FR-012-006 diagnostics | `apps/cli/src/doctor.rs`, API `/v1/version` | Credential, protocol and broker integration tests; onboarding telemetry and UX timing. |

SC-012-001..004 remain open. Unit tests do not prove a first run visible in Studio,
all four stories, durability, or trace IDs on every remote error. The <15 minute
criterion requires a new participant following the docs; compilation time is not UX evidence.

The implementation extends the CLI delivered by 011 through public REST/gRPC and
the shared IR codec. ADR-0018 remains authoritative for publishing: 012 does not
introduce a second persistence path or change transaction boundaries. Dependency
011 is now explicit in the plan because its CLI and publishing API are reused.
Existing 001–007 contracts remain the dependency boundary for runtime behavior.

The generic task paths `tools/cli` were stale; the implemented package is `apps/cli`.
T012-006 does not justify adding an otherwise unused migration: no new durable
entity is needed for the foundation. Its applicability must be reviewed with the
vertical slice before closure. Backend auditing, OTel and tenant isolation must
be verified through existing services in the integration gate, not simulated locally.

Validation: `cargo test -p sagat-cli --locked --quiet`: **47 passed** (including
one corpus test); `cargo fmt --all -- --check`. No database, broker, SDK or browser
journey was exercised by this command.
