# Testing and certification strategy

## Pyramid

- Unit: journal event reduction, DAG planner, backoff policies, IR validation, authz, serialization.
- Integration: real Postgres via Testcontainers, real NATS JetStream, S3-compatible object store, HTTP stub and MinIO when feasible.
- Contract: REST OpenAPI, gRPC Protobuf, AsyncAPI and `SDK compliance suite` per language.
- Property-based/model-based: state and ordering invariants; forbid terminal→running without creating a new instance.
- Fault injection: interrupt workers at claim/execute/ack, cut off the database/broker, duplicate messages, delay timers, diverging clocks, S3 outage.
- Security: cross tenant, secret exfiltration, code sandbox breakout, webhook replay, SSRF, RBAC, quotas.
- E2E: author→publish→trigger→run→saga compensation→visualize; inline code TypeScript and C#; human approval.
- Load/stress: partitioning, active steps, 10k/100k/1M waits, pools/quotas, high fanout, huge DAG and backpressure.

## Where the tests live and what they are called

`tests/<kind>/<feature>/` with `<kind>` one of `e2e`, `integration`, `contract`, `fault` and `performance`, and `<feature>` the
feature's name without its number (`tests/e2e/sdk-multilanguage/`). The Cargo targets of the `sagat-tests` crate are named the same
way, kind first (`e2e_sdk_multilanguage`, `contract_rest_security_multitenancy`), and a benchmark's report is
`tests/performance/reports/<feature>.md`. A number says nothing a name does not, and it makes a path hard to type and to read; the
number of a feature is kept where it is an identifier (`specs/NNN-slug/`, `FR-NNN-xxx`, `T0NN-xxx`).

## Local integration resource profile

When Docker runs in a small VM, bound the number of independent test fixtures while preserving
all concurrency inside each test:

```sh
RUST_TEST_THREADS=2 cargo test --workspace --locked --no-fail-fast
```

The completion validation uses a four-CPU, 6 GiB Colima VM, an explicit `DOCKER_HOST`,
`CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0` and `CARGO_PROFILE_DEV_DEBUG=0`. Leave
`SAGAT_TEST_DATABASE_URL` unset to give each test process its own Testcontainers server.
Unrestricted host-level test parallelism reproduced transient PostgreSQL pool timeouts and HTTP
503s in the developer-experience fixtures. With two test threads, all 32 runnable DX tests and
all 11 event E2E tests passed against fresh containers. This is a test-runner resource profile,
not a change to production pool sizes, admission limits or the races exercised within tests.
Keep performance benchmarks separate and report their own concurrency and hardware.

CI sets the Cargo dev and test profiles to `debug = "line-tables-only"`. This retains filename/line
backtraces while omitting type and variable debugging metadata; test commands are unchanged, and
`debug_assertions` and the build-script `DEBUG=true` value remain enabled for these profiles.

The Rust workspace job shares the SDK job's existing dependency-cache key (`sdks`) and enables
`cache-on-failure`. Both jobs use the same root workspace/profile; the action retains its compiler,
environment, platform and manifest hashes, and Cargo checks artifact freshness. Saving compiled
dependencies after a test failure lets later attempts reuse them. Every test gate still executes.
Workspace crates retain the action's default exclusion, so compiling/linking this repository's
test executables can still be substantial. Cache restoration alone does not establish a timing
improvement; inspect the cache-hit log and measured build stages before claiming one.

## Critical matrix

| Scenario | Expected invariant |
|---|---|
| Crash after transaction before NATS | Task reappears via outbox |
| NATS redelivery | Single valid lease and single step commit |
| Crash after side effect | Idempotency/reconciliation prevents duplicate effect when the provider cooperates |
| Simultaneous timeout and signal | One atomic resolution, the other is a no-op/audited |
| Compensation also fails | Retry, `COMPENSATION_FAILED` state, human action |
| Version change during Run | Run stays on the pinned version |
| Executor runs malicious script | Isolation, limits and network enforced |
| Tenant A tries to query tenant B | 403/404 without metadata leakage |

## Reproducible benchmarks

Always profile: CPU/cores, RAM, DB size, index, HA config, number of shards/workers, tenancy, payload, operation mix, replay mode and window. Publish p50/p95/p99, sustainable throughput, failures and regression vs baseline.

**Target scenarios to prove**: 10k, 100k and 1M runs waiting for an event/timer without dedicated workers; 100, 1000, 5000 transitions/s on properly sized hardware; shutdown/restart of 30% of the workers over 20 minutes. Numbers are experimental hypotheses/targets and may not be reached in V1.
