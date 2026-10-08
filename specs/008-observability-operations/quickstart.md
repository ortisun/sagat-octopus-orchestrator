# Validation quickstart: Distributed observability and operations

> This quickstart describes the target experience; the commands are **proposed contracts**, not executable until `001` and this feature are implemented.

## Prerequisites

- Rust stable toolchain pinned by the repository, Docker, PostgreSQL and NATS JetStream.
- Dev auth token, test tenant and feature fixtures.

## Validation flow

1. Start `docker compose -f infra/docker/compose.yaml up -d`.
2. Run the API/engine migrations.
3. Start the workspace's APIs, scheduler and worker simulator.
4. Run the `spec.md` scenarios US01..US04 via the `tests/e2e/008-observability-operations/` suite.
5. Confirm outcomes, journal, outbox, traces, logs and that the isolation policies were applied.
6. Take down the worker/scheduler at a controlled moment and confirm recovery.

## Gates

- Operator diagnoses a failure in fewer than N interactions during a UX test, with run journal and trace correlated.
- Reproducible, annotated errors; no confirmed transition lost.
- Update tasks `[x]` only when real tests have passed.
