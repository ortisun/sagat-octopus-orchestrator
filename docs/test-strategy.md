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

## What CI runs

The rule is in the constitution (Development Workflow, CI gates) and is checked by `scripts/tests/test_ci_workflow.py`.

**A pull request, and a push to `develop`,** runs only the tests of what it changes:

- the unit tests of the workspace packages that contain a changed file (`cargo test -p <package>`), and every package when
  `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/`, `clippy.toml`, `rustfmt.toml` or `vendor/` changes;
- the unit tests of an SDK when a file under `sdks/<language>/` changes, of the Studio when a file under `frontend/` changes,
  and of the scripts when a file under `scripts/` changes; and of the lake runtime (Python, `runners/lake/`) with the
  audit of its lock when a file other than Markdown under it changes;
- the integration suites (the `sagat-tests` targets: integration, contract, fault, e2e and performance, which start
  PostgreSQL through Testcontainers) that touch the change: the suite whose directory holds a changed file (the deepest
  one, so `tests/contract/<feature>/rest/` does not select its parent) or whose `[[test]]` entry is new or changed; the
  suites whose own sources name an affected package, that is a changed package or one that depends on it; the browser
  suites when `frontend/e2e` or `examples/` changes; the SDK suites when `sdks/<language>/conformance` changes; the lake
  suites (`integration`, `fault`, `e2e` and `performance` of `iceberg-writes`) when `runners/lake/` changes; and every
  suite when `tests/support`, `tests/fixtures`, any other `runners/` directory, a contract file under `specs/` or a
  dependency of `tests/Cargo.toml` changes.

Nothing else runs: no formatting, no `clippy`, no architecture, agent-configuration or design-doc check, no contract
validation, no advisory audit, no upgrade, Compose smoke-test, scale or native-runner job. A change that touches only
Markdown, `docs/`, `tasks/`, `.claude/` or `infra/` runs no test at all.

**A release** runs everything: every unit test and integration suite, formatting, `clippy`, the checks above, the contract
and observability validation, the advisory audit, and the upgrade, Compose smoke-test, scale and native-runner jobs. A
release is a push to `main`, `release/**` or `hotfix/**`, a tag `v*`, a pull request into one of those branches, a manual
run (`workflow_dispatch`), a change of `.github/workflows`, or any run with no usable base commit. For the lake runtime
that adds the types and lint, the image build, the smoke test of the image and its SBOM; its lock is also audited every
week by `.github/workflows/lake-audit.yml`.

What this costs, so it is not forgotten: a violation of a lint or of an architecture rule reaches `develop` and is found when
a release is closed, and a suite that reaches the code only over HTTP, without naming its package, is not selected for a
change to that package. Run the lints and the suites you affect before the pull request (`/verify`). Measured on the
repository: a change to a leaf package (`infra-egress`) selects 12 of the 83 suites, a change to `domain` selects 76,
because almost everything depends on it. A coverage map from test to file would select exactly; it is not built.

Measured on GitHub's runners on 2026-11-05, with two throwaway pull requests against `develop`: a change to one Markdown
file ran the selection job (8 s) and skipped every other job; a comment added to `infra-egress` ran only the `rust` job,
which took 28 min 51 s (the unit tests of that package and the 12 suites above), against 1 h 30 min for the same job in a
release, with every other job skipped. The first run of the release rule, the merge of the change that introduced it, ran
all 13 jobs green because it changed `.github/workflows`.

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

The loop-drain recovery test performs its expected-error downgrade probes on dedicated
PostgreSQL sessions. SQLx 0.9 returns early on a refused migration before its session advisory
unlock; returning that connection to a pool can block a later probe. The fixture acknowledges
unlock and closes the session before reporting its outcome, with bounded waits and database-scoped
lock assertions. This regression runs first in the capacity/migration CI preflight.

Scoped event quota tests separate a claimed event's durable retry from a mixed lookback/claim
tick. A controlled inbox SHARE permits native lookback while making UPDATE SKIP LOCKED defer
the claim; that tick need not increment a retry counter. The held-source case requires a
positive quota-refusal diagnostic, unchanged execution/receipt/accounting snapshots, and cold
lookback recovery after the explicit unlock. The independent dispatch case requires the stored
quota retry class and zero consumed attempts. Neither proof assumes an unobserved rollback
cleanup timing or relies on sleeping until a lock disappears.

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
