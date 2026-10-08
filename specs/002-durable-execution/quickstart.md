# Validation quickstart: Durable execution engine and scheduler

> This quickstart describes the target experience; the commands are **proposed contracts**, not executable until `001` and this feature are implemented.

## Prerequisites

- Rust stable toolchain pinned by the repository, Docker, PostgreSQL and NATS JetStream.
- Dev auth token, test tenant and feature fixtures.

## Validation flow

1. Start `docker compose -f infra/docker/compose.yaml up -d`.
2. Run the API/engine migrations.
3. Start the workspace's APIs, scheduler and worker simulator.
4. Run the scenarios from `spec.md` US01..US04 via the `tests/e2e/002-durable-execution/` suite.
5. Confirm outcomes, journal, outbox, traces, logs and that the isolation policies were applied.
6. Take down the worker/scheduler at a controlled moment and confirm recovery.

## Gates

- Deterministic crash test at each point of claim→effect→commit→publish, with an intact journal and no double transition.
- Reproducible, annotated errors; no confirmed transition lost.
- Update tasks `[x]` only when real tests have passed.
