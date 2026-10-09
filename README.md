# Sagat Octopus Orchestrator — Durable Workflow Platform (Spec Kit)

> **Status:** specification with implementation in progress: features 001 (platform foundation) and 002 (durable execution) are built and tested (see [`docs/traceability.md`](docs/traceability.md)); features 003–012 are still only specified. **Name:** provisional. **Language:** English (en). **Date:** 2026-10-07.

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

## Build, test and run (features 001 and 002)

```sh
cargo test --workspace --locked          # needs Docker: integration tests start PostgreSQL 16
scripts/smoke-compose.sh                 # builds the images, starts PostgreSQL + NATS + API + engine, checks health
# baseline benchmark of feature 002 (10k suspended runs), on purpose:
SAGAT_PERF_REPORT=tests/performance/reports/002-durable-execution.md \
  cargo test -p sagat-tests --test perf_002 --release -- --ignored --nocapture
```

The full procedure, the environment variables and what is not built yet are in
[`specs/001-platform-foundation/quickstart.md`](specs/001-platform-foundation/quickstart.md);
operating the control plane is in [`docs/runbooks/control-plane.md`](docs/runbooks/control-plane.md) and the
execution engine (worker gRPC API, scheduler, NATS outbox) in
[`docs/runbooks/execution-engine.md`](docs/runbooks/execution-engine.md); the validation flow of feature 002 is
in [`specs/002-durable-execution/quickstart.md`](specs/002-durable-execution/quickstart.md).
Workflow: Gitflow branches and Conventional Commits (see `AGENTS.md`).

## Folders

```text
.specify/memory/constitution.md   Architecture and product constitution
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

1. Install and initialize Spec Kit in the **real code repository** following https://github.com/github/spec-kit (do not overwrite `.specify/memory/constitution.md` from this package).
2. Copy `.specify/memory`, `specs/`, `docs/` and `AGENTS.md` to the repository root. If `specify init` creates files that already exist, merge deliberately.
3. Treat `specs/001-platform-foundation` as the starting point; evolve one feature per branch with `/speckit-clarify`, `/speckit-plan`, `/speckit-checklist`, `/speckit-tasks`, `/speckit-analyze`, `/speckit-implement` and `/speckit-converge` according to the agent integration.
4. The plan and task files are already **initial proposals**, subject to review by the agent; they do not represent existing code.
5. Before implementing, validate the ADRs, test the protocol against a real worker and run the indicated quality gates.

**Spec Kit note:** there are 12 independent features ordered by dependency, not a single monolithic spec. Cross-cutting architecture artifacts live in `docs/`; initial canonical contracts in `specs/001-platform-foundation/contracts/`.

## Guarantees and limits

- The database confirms state transitions atomically; workers receive tasks at-least-once. External side effects do **not** receive a universal exactly-once guarantee.
- Suspended workflows do not hold threads or execution containers.
- There is no arbitrary automatic migration of running workflows to a new version.
- The graphical editor is a faithful representation of the **declarative** DSL/IR. Arbitrary imperative code is not always convertible to a static graph; Studio merges the planned graph and the actual execution.
- Throughput and latency targets are **benchmark criteria to verify**, not commercial promises.

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
