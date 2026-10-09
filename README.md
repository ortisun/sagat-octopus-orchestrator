# Sagat Octopus Orchestrator — Durable Workflow Platform (Spec Kit)

> **Status:** specification with implementation in progress: features 001 (platform foundation), 002 (durable execution), 003 (sagas and compensations, apart from the SDK DSL, Studio screens and dashboards, which belong to other features) and 009 (security and multi-tenant governance, apart from the sandbox, inbound webhooks and a Helm chart, which belong to other features) are built and tested (see [`docs/traceability.md`](docs/traceability.md)); the other features are still only specified. **Name:** provisional. **Language:** English (en). **Date:** 2026-10-07.

## Mission

Self-hosted and potentially SaaS platform to orchestrate durable workflows and Sagas, with development via SDKs, inline scripts without traditional deployment, visual Studio, webhooks, messaging, end-to-end observability and high availability.

Inspired by **Temporal** (history and recovery), **Restate** (journal and durable invocations), **Inngest** (event-driven primitives) and **Windmill** (script IDE/execution and visual flows). It does not assume reuse of code from those projects.

## Fixed decisions

- **Backend:** Rust (Tokio, Axum, tonic, SQLx), domain modules independent of libraries. Modular monolith with separately deployable processes when justified.
- **V1 durability:** PostgreSQL as source of truth; transactional log and outbox. NATS JetStream is transport/notification, not the source of truth for execution state.
- **Studio:** React/TypeScript, React Flow, Monaco, with the definition graph and the runtime graph kept separate.
- **Official SDKs:** TypeScript, Go, Python, C#/.NET and Java, common semantic contract via gRPC/Protobuf and REST/JSON.
- **Inline script:** compilation/build of immutable artifacts, process/container isolation; progressive support for TypeScript, Python, C#, Go and Java.
- **Integrations:** inbound and outbound HTTP/webhooks, Kafka, NATS and AWS SQS; inbox/outbox, backpressure and DLQ.
- **Security:** tenant/namespace isolation, RBAC, auditing and explicit egress/secrets policy.
- **Observability:** OTel + auditable journal + execution timeline and DAG.

## Build, test and run (features 001, 002, 003 and 009)

```sh
cargo test --workspace --locked          # needs Docker: integration tests start PostgreSQL 16
scripts/smoke-compose.sh                 # builds the images, starts PostgreSQL + NATS + API + engine, checks health
# baseline benchmark of feature 002 (10k suspended runs), on purpose:
SAGAT_PERF_REPORT=tests/performance/reports/002-durable-execution.md \
  cargo test -p sagat-tests --test perf_002 --release -- --ignored --nocapture
# the security battery of feature 009 (cross-tenant, secret leak, quota racing, SSRF on the egress guard):
cargo test -p sagat-tests --test contract_rest_009 --test e2e_009 --test integration_009 --test fault_009
# sagas, compensations and reconciliation (feature 003): stories, PostgreSQL, contracts, failure injection:
cargo test -p sagat-tests --test e2e_003 --test integration_003 --test contract_rest_003 --test contract_grpc_003 --test fault_003
```

The full procedure, the environment variables and what is not built yet are in
[`specs/001-platform-foundation/quickstart.md`](specs/001-platform-foundation/quickstart.md);
operating the control plane is in [`docs/runbooks/control-plane.md`](docs/runbooks/control-plane.md) and the
execution engine (worker gRPC API, scheduler, NATS outbox) in
[`docs/runbooks/execution-engine.md`](docs/runbooks/execution-engine.md); the validation flow of feature 002 is
in [`specs/002-durable-execution/quickstart.md`](specs/002-durable-execution/quickstart.md). Operating the
security controls (quotas and their ceilings, worker bindings, the secret store, the egress policy, TLS
on the worker API) is in [`docs/runbooks/security-governance.md`](docs/runbooks/security-governance.md);
the validation flow and the security review of feature 009 are in
[`specs/009-security-multitenancy/quickstart.md`](specs/009-security-multitenancy/quickstart.md). Operating sagas (what happens when a step fails, the runs that wait for a person, the circuit breaker) is in
[`docs/runbooks/sagas.md`](docs/runbooks/sagas.md); the validation flow and review of feature 003 are in
[`specs/003-sagas-compensations/quickstart.md`](specs/003-sagas-compensations/quickstart.md); `examples/payment-saga.workflow.json` is a saga you can publish.
Workflow: Gitflow branches and Conventional Commits (see `AGENTS.md`).

## Folders

```text
.specify/memory/constitution.md   Architecture and product constitution (principles I–VIII)
.specify/                       Spec Kit 1.1.2 (Claude integration): templates, scripts, workflow
.claude/skills/speckit-*/       The /speckit-* commands
specs/001..012/                 Spec Kit features: spec.md, plan.md, tasks.md, ...
docs/architecture/              Cross-cutting architecture and semantic contracts
docs/adr/                       Architectural Decision Records
docs/roadmap.md                 Phases, dependencies and exit criteria
docs/traceability.md            Traceability of requirements and deliverables
docs/test-strategy.md           Failure, load, contract and security tests
docs/runbooks/                  Operational procedures
AGENTS.md                       Instructions for implementation agents
crates/ apps/ tests/            Rust workspace: domain, application, adapters, control plane, test suites
infra/docker/                   Local environment (Docker Compose) and the control-plane and execution-engine images
scripts/                        Architecture-rule checker and the compose smoke test
```

## Usage with GitHub Spec Kit

Spec Kit is initialized in this repository (`.specify/`, Claude integration). The constitution at
`.specify/memory/constitution.md` was derived from `AGENTS.md`, the ADRs and the Constitution Check that
every plan already carried; it is **version 1.0.0 with the ratification date still to be confirmed by the
maintainers** (see the comment at the top of the file).

1. Read `.specify/memory/constitution.md`, then the `spec.md`, `plan.md` and `tasks.md` of the feature you work on.
2. Evolve one feature per branch (`feature/<spec-id>-<slug>`, see `AGENTS.md`) with `/speckit-clarify`, `/speckit-plan`, `/speckit-checklist`, `/speckit-tasks`, `/speckit-analyze`, `/speckit-implement` and `/speckit-converge`. A new feature starts with `/speckit-specify` and continues the numbering from 013.
3. Spec Kit finds the active feature through `.specify/feature.json` (per checkout, git-ignored) or `SPECIFY_FEATURE_DIRECTORY`, not through the branch name, so the Gitflow branch names do not conflict with it.
4. For features 001, 002, 003 and 009 the checked tasks in `tasks.md` record what is built and its evidence; the other features' plans and tasks are still **initial proposals** and do not represent existing code.
5. Before implementing, validate the ADRs, test the protocol against a real worker and run the indicated quality gates.

**Spec Kit note:** there are 12 independent features ordered by dependency, not a single monolithic spec. Spec Kit has no place for cross-cutting material, so it stays in `docs/` (architecture, ADRs, runbooks, glossary, roadmap, traceability, test strategy), and the constitution points to it; initial canonical contracts are in `specs/001-platform-foundation/contracts/`.

## Guarantees and limits

- The database confirms state transitions atomically; workers receive tasks at-least-once. External side effects do **not** receive a universal exactly-once guarantee.
- Suspended workflows do not hold threads or execution containers.
- There is no arbitrary automatic migration of running workflows to a new version.
- The graphical editor is a faithful representation of the **declarative** DSL/IR. Arbitrary imperative code is not always convertible to a static graph; Studio merges the planned graph and the actual execution.
- Throughput and latency targets are **benchmark criteria to verify**, not commercial promises.
- A saga undoes what it did by asking the provider to (a compensation), which is not a rollback: an effect that cannot be undone ends the run `PARTIAL`, and a compensation that keeps failing waits for a person instead of being assumed done. An attempt whose outcome is unknown is looked up before it is repeated. The platform asks at least once with stable keys; a provider that does not deduplicate by them can still do an effect twice. What is not built (the SDK DSL, Studio screens, dashboards) is in [`specs/003-sagas-compensations/research.md`](specs/003-sagas-compensations/research.md).
- Tenant isolation, quotas, worker bindings, the secret store and the egress policy are enforced and tested (feature 009). **Not yet built:** running untrusted code in a sandbox, inbound webhook signature and replay controls, quotas for builds/storage/logs, a KMS behind the secret key ring, and a Helm chart with network policies. The list, with the reason for each, is in [`specs/009-security-multitenancy/research.md`](specs/009-security-multitenancy/research.md).

## Reading order

1. `.specify/memory/constitution.md`
2. `docs/architecture/overview.md` and `docs/architecture/execution-semantics.md`
3. `docs/architecture/sagas.md`, `storage-and-events.md`, `security-and-sandbox.md`
4. `specs/001-platform-foundation/spec.md`, `plan.md`, `tasks.md`
5. `docs/roadmap.md` and the other features

## Sources and references

- [GitHub Spec Kit](https://github.com/github/spec-kit) and [official templates](https://github.com/github/spec-kit/tree/main/templates)
- [Temporal Event History](https://docs.temporal.io/encyclopedia/event-history)
- [Inngest Durable Primitives](https://www.inngest.com/docs/durable-execution/primitives)
- [Restate GitHub](https://github.com/restatedev/restate)
- [Windmill GitHub](https://github.com/windmill-labs/windmill)
