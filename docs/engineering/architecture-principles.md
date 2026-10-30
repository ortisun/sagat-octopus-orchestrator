# Architecture principles and how they are kept

Source of the principles: the constitution. This page says how a principle becomes a structure, a record and a check.

## Shape

A Rust **modular monolith**, a crate per bounded context, ports and adapters, dependencies pointing inwards: `domain ← application ← adapters ← apps`. A process is split out only after measurement. PostgreSQL is the authority; the transactional outbox and the inbox carry messages; NATS is a hint. Overview: `docs/architecture/overview.md`.

## Principles to checks (fitness functions)

An architectural rule that nothing checks is a wish. Each row is a rule and what fails when it is broken.

| Rule | Check that fails |
|---|---|
| Domain has no framework, SQL driver or broker dependency; layers point inwards; no cycles | `scripts/check-architecture.py` (CI `architecture`), tests in `scripts/tests/` |
| Every store statement is within the runtime role's privileges | `tests/integration/platform-foundation/runtime_privileges`; [supported static SQL forms](sql-privilege-audit.md) |
| Every service works as the least-privilege role | `SAGAT_TEST_AS_RUNTIME_ROLE=1` over the suites |
| Every variable-size column is classified | `storage-classification.json` + `integration_security_multitenancy` |
| Journal is the authority; projections equal a replay | `verify_run` / `assert_consistent` at the end of the fault tests |
| One transaction per transition (state, scheduling, timers, outbox) | fault tests with an injected failure per statement |
| Contracts only grow | `*.baseline.json` + compatibility tests; OpenAPI/AsyncAPI/proto/IR schema lint in CI `contracts` |
| Five SDKs, one semantics | `tests/contract/sdk-conformance` (every scenario, five languages) |
| Tenant boundary: another tenant's resource is a missing one | `tests/contract/security-multitenancy/rest/battery.rs` over every operation |
| Observability vocabulary is closed and low-cardinality | telemetry tests (`observability.rs`, `approval_telemetry.rs`), metrics enums |
| Docs and generated references match the code | `contract_developer_experience`, `npm run check:api` |
| Agent guidance is bounded and consistent | `scripts/check-agent-config.py` |
| Incomplete capacity coordination stays dormant; new lease producers need inventory review | `scripts/check-capacity-activation.py` and negative fixtures (source-text check, not a call-graph proof); runtime lock/race tests remain required |
| The temporary Kafka dependency snapshot matches its registry source except for the reviewed zstd version | `scripts/check-vendor.py` and its negative controls (CI `architecture`) |

When you add a rule worth keeping, add its row and its check in the same change.

## Architecture Decision Records

`docs/adr/`, one per decision, numbered, immutable once accepted (superseded, never edited). Write one when a decision is **hard to reverse, surprising without context, and the result of a real trade-off**. Otherwise it is a rule in a nested `AGENTS.md` or a `D…` entry.

An ADR here has: status, context (forces, not history), the decision stated as MUST / MUST NOT, at least two considered options with why they lost, consequences (good, bad, operational), **Enforced by** (the check above, or "review only"), and who decided and when. Decisions taken by an agent are reviewed by a person and say so; product and scope decisions are the owner's and also go to `docs/owner-decisions.md`. Template: `docs/adr/TEMPLATE.md`; index and rules: `docs/adr/README.md`.

## Quality attributes are scenarios

State each as stimulus, environment, response and measure (`docs/architecture/observability-slo.md`, `capacity-and-topology.md`, the plan's performance section). Targets are not promises: a benchmark report names the hardware, the load and the failures (`tests/performance/reports/`). A ceiling is accepted only when a measured run supports it; when it does not, the limit is published lower and the gap stays open.

## Evolving safely

- Additive migrations and contracts; readers tolerate what they do not know; a rolled-back reader keeps working or refuses with a typed reason.
- Published workflow versions are immutable; a run references exactly one.
- Small batches: a pull request closes one coherent increment with its evidence; long-lived branches merge `develop` often.
- Degrade instead of fail: the platform works by polling when the broker is down.
- Threat model per trust boundary (`docs/architecture/security-and-sandbox.md`); untrusted code only ever runs isolated.

## Records and where they live

| Record | Place |
|---|---|
| Principle | constitution |
| Structural decision | ADR |
| Feature decision, gap, question | `specs/NNN-slug/research.md` (`D`, `G`, `O`) |
| Product/scope/release decision | `docs/owner-decisions.md` |
| Operational procedure | `docs/runbooks/` |
| Evidence of a measured claim | `validation-report.md`, `tests/performance/reports/` |
