# Validation quickstart: Durable execution engine and scheduler

Everything below was run against this repository. The pieces that are **not** built are listed at the end.

## Prerequisites

- The Rust toolchain pinned by `rust-toolchain.toml` and Docker (the suites start throw-away PostgreSQL 16 and NATS containers through Testcontainers; no port is published on the host).
- Node 22 (`npx`) only for the contract linters in CI.

## Validation flow

1. **The four user stories and the worker protocol**, against real PostgreSQL, a fake OIDC provider and real sockets:

   ```sh
   cargo test -p sagat-tests --test e2e_durable_execution           # US01..US04 and the running engine process
   cargo test -p sagat-tests --test contract_grpc_durable_execution # the worker protocol over the wire
   cargo test -p sagat-tests --test contract_rest_durable_execution # definitions, runs, operations, signals vs. the OpenAPI
   cargo test -p sagat-tests --test integration_durable_execution   # store, scheduler, outbox, NATS, faults, children
   cargo test -p sagat-domain -p sagat-infra-expr -p sagat-infra-ir -p sagat-execution-engine
   ```

2. **Failure injection.** `integration_durable_execution` (`faults.rs`) kills database connections around commits, `outbox.rs` loses the confirmation of a published message, `nats.rs` severs the broker connection through a proxy, `concurrency.rs` races workers, timers and cancellations, and `e2e_durable_execution` (`engine.rs`) lets the engine reclaim a dead worker's step by itself. The property tests (`crates/domain/tests/execution_properties.rs`, `PROPTEST_CASES=50000` for a long soak) apply random command sequences, including stale fences and out-of-order reports, and check the invariants after every step.

3. **The local environment.** `scripts/smoke-compose.sh` builds both images, starts PostgreSQL + NATS + control plane + execution engine, and checks health, idempotent migrations and seed, that every non-owner connection is the least-privilege role, and that the worker port accepts connections. To run the stack by hand:

   ```sh
   cp infra/docker/.env.example infra/docker/.env       # then edit the OIDC settings
   docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
   ```

   Publishing workflows and starting runs needs an access token from your OIDC provider (for the local stack, `sagat dev token` asks the development issuer of feature 012; gap G10 is closed); the workers' identity needs the `worker` role in the tenant. Anything that can mint such a token can drive the REST API (`openapi.yaml`) and the worker protocol (`worker.proto`, for example with `grpcurl`).

4. **The benchmark** (10k suspended runs, scheduling latency):

   ```sh
   SAGAT_PERF_REPORT=tests/performance/reports/durable-execution.md \
     cargo test -p sagat-tests --test perf_durable_execution --release -- --ignored --nocapture
   ```

5. **Operating it:** `docs/runbooks/execution-engine.md`.

## Security review (T002-035)

The review of the engine (abuse by a tenant, a worker or a run; isolation; secrets) found fifteen things. Each is fixed or recorded as a gap, and each fix has a test that fails without it: the guards were weakened one at a time (the mutation checks below) and the named test failed every time.

| # | Finding | Result | Evidence |
|---|---|---|---|
| 1 | A workflow could start itself in parallel and grow a tree of 8^8 runs | Fixed: 1024 runs per tree, `CHILD_LIMIT_EXCEEDED` (D37) | `children.rs::a_fan_out_bomb_stops_at_the_tree_budget_*` |
| 2 | Validating or caching a hostile definition cost unbounded CPU and memory | Fixed: 32 `parallel` nodes, cache weighed by footprint (D37) | `crates/domain/tests/workflow.rs` (`cost`), `crates/infra-ir/tests/cache.rs` |
| 3 | A run could retain unbounded output, and every lease carried it | Fixed: 4 MiB per run, `RUN_DATA_LIMIT`, signals refused with `QUOTA_EXCEEDED`; a poll stops at 8 MiB (D37, D38) | `hardening.rs::{a_run_cannot_retain_*, a_signal_that_would_exceed_*, a_poll_is_bounded_in_bytes_*}` |
| 4 | A worker could register unboundedly many sessions and arbitrary labels | Fixed: 32 sessions per identity, the same name replaces its session, labels validated (D38) | `hardening.rs::{what_a_worker_advertises_*, an_identity_holds_a_bounded_number_of_sessions_*}` |
| 5 | The claim query aggregated every ready step of the tenant | Fixed: a bounded probe of `steps_ready_idx` per activity (D38) | `hardening.rs::the_claim_query_walks_the_ready_index_*` (plan) and the benchmark |
| 6 | A poll that failed after leasing some steps lost their tokens, and one bad run blocked the tenant | Fixed: tasks join the response after their commit, a failing run is skipped (D38) | `hardening.rs::{a_task_leased_before_another_run_failed_*, a_run_that_cannot_be_leased_*}` |
| 7 | A missing consumer made the outbox grow without bound | Fixed: five re-announcements per ready step, purge until caught up (D39) | `hardening.rs::{a_task_nobody_takes_*, housekeeping_purges_a_backlog_*}` |
| 8 | Nothing limited the connections or the calls of the worker API as a whole | Fixed: connection cap and load shedding with a reserve for heartbeats and reports (D41). Per-tenant fairness is G32 | `grpc/limits.rs` (real sockets), `crates/api-grpc/src/limits.rs` (unit) |
| 9 | A worker's authority is the whole tenant | **Fixed by feature 009** (G31): worker bindings, fail closed, re-checked at use | `specs/009-security-multitenancy/research.md` D43, `tests/integration/security-multitenancy/bindings.rs` |
| 10 | An idempotency key was an oracle for a worker without the lease | Fixed: records scoped by the lease token (D40) | `hardening.rs::a_key_is_no_oracle_*` |
| 11 | An error echoed a map key computed from run data | Fixed: `<computed>`; literals of the expression are still named | `crates/infra-expr/tests/subset.rs` |
| 12 | The runtime role could delete an undelivered outbox message or rewrite journal tables it owned | Fixed: `BEFORE DELETE` trigger; the start-up guard refuses an owner of `run_events`, `payloads`, `workflow_versions` | `least_privilege.rs::{the_runtime_role_cannot_delete_*, each_append_only_table_*, a_role_that_owns_a_journal_table_*}`, `migrations.rs` |
| 13 | A late heartbeat could be overridden by a reclaim decided on stale data | Fixed: the lease is re-checked under the run's lock (D39) | `hardening.rs::a_lease_renewed_after_the_candidate_was_read_*` |
| 14 | Worker calls on another tenant's run were not audited | Fixed: audited against the caller's tenant (D40) | `hardening.rs::a_worker_of_another_tenant_is_refused_*` |
| 15 | A `TaskRef` could print its lease token | Fixed: redacted `Debug` | `hardening.rs::a_lease_token_is_never_printed` |

Mutation checks (a guard weakened, the suite must fail): 20 of 20 mutants killed: tree budget (2), run output budget, signal budget, poll byte cap, poison run, claimed-before-failure, key oracle, rehint cap, purge rounds, lease re-check, cross-tenant audit, session cap, label bounds, ready-index plan, shed threshold, reserve, ticket release, connection cap and heartbeat classification. One further mutant of the earlier work (`status = 'WAITING'`) was proven equivalent.

## Gates

- Crash at each point of claim, effect, commit, publish: journal intact and no double transition (`faults.rs`, `outbox.rs`, the property tests).
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `python3 scripts/check-architecture.py`, the OpenAPI lint and the protobuf compile (CI).
- Tasks are ticked only with a test or artifact named next to them (`tasks.md`).

## Not built (so nobody assumes it)

fairness between tenants beyond their call budgets (G32); a blob store for payloads over 256 KiB (G19); the result of a run or child (G21); `foreach` and `humanApproval` (G20); sagas (003); inbound events and webhooks (004); span export (008); partitioning and retention of the journal, bounded-concurrency timer firing and disjoint scheduler candidates across replicas (G22, G28; 010); a worker simulator binary and the SDKs (006).
