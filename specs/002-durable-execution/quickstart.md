# Validation quickstart: Durable execution engine and scheduler

Everything below was run against this repository. The pieces that are **not** built are listed at the end.

## Prerequisites

- The Rust toolchain pinned by `rust-toolchain.toml` and Docker (the suites start throw-away PostgreSQL 16 and NATS containers through Testcontainers; no port is published on the host).
- Node 22 (`npx`) only for the contract linters in CI.

## Validation flow

1. **The four user stories and the worker protocol**, against real PostgreSQL, a fake OIDC provider and real sockets:

   ```sh
   cargo test -p sagat-tests --test e2e_002           # US01..US04 and the running engine process
   cargo test -p sagat-tests --test contract_grpc_002 # the worker protocol over the wire
   cargo test -p sagat-tests --test contract_rest_002 # definitions, runs, operations, signals vs. the OpenAPI
   cargo test -p sagat-tests --test integration_002   # store, scheduler, outbox, NATS, faults, children
   cargo test -p sagat-domain -p sagat-infra-expr -p sagat-infra-ir -p sagat-execution-engine
   ```

2. **Failure injection.** `integration_002` (`faults.rs`) kills database connections around commits, `outbox.rs` loses the confirmation of a published message, `nats.rs` severs the broker connection through a proxy, `concurrency.rs` races workers, timers and cancellations, and `e2e_002` (`engine.rs`) lets the engine reclaim a dead worker's step by itself. The property tests (`crates/domain/tests/execution_properties.rs`, `PROPTEST_CASES=50000` for a long soak) apply random command sequences, including stale fences and out-of-order reports, and check the invariants after every step.

3. **The local environment.** `scripts/smoke-compose.sh` builds both images, starts PostgreSQL + NATS + control plane + execution engine, and checks health, idempotent migrations and seed, that every non-owner connection is the least-privilege role, and that the worker port accepts connections. To run the stack by hand:

   ```sh
   cp infra/docker/.env.example infra/docker/.env       # then edit the OIDC settings
   docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
   ```

   Publishing workflows and starting runs needs an access token from your OIDC provider (the repository ships no development issuer, gap G10); the workers' identity needs the `worker` role in the tenant. Anything that can mint such a token can drive the REST API (`openapi.yaml`) and the worker protocol (`worker.proto`, for example with `grpcurl`).

4. **The benchmark** (10k suspended runs, scheduling latency):

   ```sh
   SAGAT_PERF_REPORT=tests/performance/reports/002-durable-execution.md \
     cargo test -p sagat-tests --test perf_002 --release -- --ignored --nocapture
   ```

5. **Operating it:** `docs/runbooks/execution-engine.md`.

## Gates

- Crash at each point of claim, effect, commit, publish: journal intact and no double transition (`faults.rs`, `outbox.rs`, the property tests).
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `python3 scripts/check-architecture.py`, the OpenAPI lint and the protobuf compile (CI).
- Tasks are ticked only with a test or artifact named next to them (`tasks.md`).

## Not built (so nobody assumes it)

TLS and mutual TLS on the worker API (G27); a blob store for payloads over 256 KiB (G19); the result of a run or child (G21); `foreach` and `humanApproval` (G20); sagas (003); inbound events and webhooks (004); span export (008); partitioning and retention of the journal, bounded-concurrency timer firing and disjoint scheduler candidates across replicas (G22, G28; 010); a worker simulator binary and the SDKs (006).
