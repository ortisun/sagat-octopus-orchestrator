# Validation quickstart: Platform foundation and contracts

> Feature 001 is implemented in part. The commands below are real and were run; items marked
> **pending** are not available yet. The authoritative status per requirement is in
> [`docs/traceability.md`](../../docs/traceability.md).

## Prerequisites

- Rust toolchain: `rustup` reads `rust-toolchain.toml` and installs the pinned compiler.
- Docker (the integration tests start PostgreSQL 16 with Testcontainers; the local environment uses
  Docker Compose).
- Node.js 22 (only for the contract checks: OpenAPI, AsyncAPI and JSON Schema validation).
- `protoc` for the standalone Protobuf compilation check. The Rust build uses `protox` and does not
  need `protoc`.
- An OIDC provider for real tokens. For the local stack, feature 012 ships a development identity
  provider (gap G10, closed): `sagat dev token` asks it (see `docs/quickstarts/README.md`). A shared
  environment uses its own provider.

## 1. Run the tests

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked                      # unit, contract, integration, e2e, fault
python3 scripts/check-architecture.py                # layering and domain-purity rules
python3 -m unittest discover -s scripts/tests        # tests of that checker
npx --yes @redocly/cli@1 lint specs/001-platform-foundation/contracts/openapi.yaml
protoc --proto_path=specs/001-platform-foundation/contracts --descriptor_set_out=/tmp/worker.pb \
  specs/001-platform-foundation/contracts/worker.proto
npx --yes --ignore-scripts @asyncapi/cli@3.6.0 validate \
  specs/001-platform-foundation/contracts/events.asyncapi.yaml
cargo install cargo-audit --locked && cargo audit --deny warnings   # dependency advisories
```

Each user story has its own file (`tests/e2e/platform-foundation/us01.rs` … `us04.rs`); the
token matrix is `tests/contract/platform-foundation/auth.rs`; failure scenarios are in
`tests/fault/platform-foundation/`.

The baseline benchmark is ignored by default (it is slow and machine-specific):

```sh
SAGAT_PERF_REPORT=tests/performance/reports/platform-foundation.md \
  cargo test -p sagat-tests --test perf_platform_foundation --release --locked -- --ignored
```

To reuse a PostgreSQL you already run instead of a container, set
`SAGAT_TEST_DATABASE_URL=postgres://user:pass@127.0.0.1:PORT/postgres`.

## 2. Bring up the local environment (US03)

```sh
cp infra/docker/.env.example infra/docker/.env       # edit the values
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env --profile seed run --rm seed
curl -fsS http://127.0.0.1:18080/v1/health           # {"status":"ok"}
curl -fsS http://127.0.0.1:18080/v1/ready            # checks PostgreSQL
curl -fsS http://127.0.0.1:18222/healthz             # NATS JetStream
```

The database role used by the API (`sagat_app`) is created the first time the PostgreSQL volume is
initialised. If you already had a volume from an earlier version, recreate it once with
`docker compose -f infra/docker/compose.yaml down -v`.

Everything is published on `127.0.0.1` with non-default host ports (`15432` PostgreSQL, `14222`
NATS, `18222` NATS monitor, `18080` API) so it does not collide with services already running.

The whole procedure, including building the image, running the migrations a second time and
seeding twice, is automated and is what CI runs:

```sh
scripts/smoke-compose.sh
```

Not part of the environment yet: an S3-compatible object store (optional in this phase), and the
NATS broker is not used by the control plane until feature 002.

## 3. Run the control plane without Docker

```sh
export DATABASE_URL=postgres://user:pass@127.0.0.1:5432/sagat
export SAGAT_ENV=dev
export SAGAT_OIDC_ISSUER=https://your-idp/ SAGAT_OIDC_AUDIENCE=sagat-octopus-orchestrator
export SAGAT_OIDC_JWKS_URI=https://your-idp/.well-known/jwks.json
cargo run -p sagat-control-plane --bin control-plane -- migrate
cargo run -p sagat-control-plane --bin control-plane -- seed-dev      # tenant "dev", admin "dev-admin"
# outside dev, create a real tenant explicitly (owner role; no defaults):
SAGAT_PROVISION_TENANT=acme SAGAT_PROVISION_ADMIN=alice cargo run -p sagat-control-plane --bin control-plane -- provision-tenant
cargo run -p sagat-control-plane --bin control-plane -- serve
cargo run -p sagat-control-plane --bin control-plane -- --help        # every setting
```

With `SAGAT_ENV=dev` the commands above all run as the database owner. Outside dev, `migrate` and
`provision-tenant` use the owner role, while `serve` must use the least-privilege role
(`sagat_app`, created by `infra/docker/postgres-init/01-runtime-role.sh` or by your DBA before the
first migration): `serve` checks its own database role at start and exits with code 2 if it is a
superuser, owns `audit_records` or may `UPDATE`/`DELETE`/`TRUNCATE` it. `SAGAT_MIGRATE_ON_START` is
accepted only in dev. See the runbook, sections 1 and 2.

A caller needs a token whose `sub` is a member of the tenant named in its `tenant` claim (the seed
creates tenant `dev` with administrator `dev-admin`). Memberships are read from the database on
every request, so removing one takes effect immediately.

## Validation flow of the scenarios

| Story | Scenario | Automated by |
|---|---|---|
| US01 | An administrator creates a workspace and a `dev` namespace; both are persisted and visible | `tests/e2e/platform-foundation/us01.rs` |
| US02 | A client with a valid tenant token lists namespaces and gets only its own, plus a trace id | `…/us02.rs` |
| US03 | Migrate (twice), seed, start; API and database are healthy | `…/us03.rs`, `scripts/smoke-compose.sh` |
| US04 | A tenant A user asks for tenant B's resource: nothing is revealed, the attempt is audited | `…/us04.rs` |

## Contract completion evidence (T001-002, T001-030, T001-031)

Run on 2026-10-11 with the pinned Rust toolchain and a local PostgreSQL 16 server; the Cargo
commands used `CARGO_INCREMENTAL=0` and an isolated target directory:

| Gate | Command | Result |
|---|---|---|
| Static fixtures and authentication | `cargo test -p sagat-tests --test contract_platform_foundation --locked` | 27 passed |
| Live REST/OpenAPI conformance | `cargo test -p sagat-tests --test contract_rest_platform_foundation --locked` | 3 passed |
| Protobuf compatibility | `cargo test -p sagat-api-grpc --test compatibility --locked` | 2 passed; published baseline unchanged |
| AsyncAPI compatibility and live conformance | `cargo test -p sagat-tests --test contract_asyncapi_events_webhooks_messaging --locked` | 4 passed; published baseline unchanged |
| Migration idempotency | `cargo test -p sagat-tests --test integration_platform_foundation --locked migrations::applying_migrations_twice_is_a_no_op` | 1 passed against PostgreSQL |
| Rust lint for the new fixture suite | `cargo clippy -p sagat-tests --test contract_platform_foundation --locked -- -D warnings` | passed |
| Contract syntax | Redocly 1, `protoc`, AsyncAPI CLI 3.6.0 | 0 errors; Redocly reported 9 non-blocking style warnings and AsyncAPI reported one version information item |

The static fixture test binds each REST example to its exact operation request or response schema,
and the event header/payload examples to the `TaskReady` message. It also proves the validators are
active by rejecting a workspace request that supplies `tenantId` and a task-ready payload without
`tenantId`. CI runs the same Rust gates through its workspace test job and the three standalone
contract syntax checks through its contracts job.

## Gates

- Contract/API + tenant isolation suite green, and a reproducible local environment with migrations
  executed twice (`scripts/smoke-compose.sh`).
- Errors are reproducible and annotated; no confirmed transition is lost.
- A task in `tasks.md` becomes `[x]` only when real tests have passed.

## Known limits

Recorded in [`research.md`](./research.md): no token revocation list (G7), create operations are not
idempotent through `Idempotency-Key` (G8), `403`s are not audited (G9), no OTLP span export (G6;
logs, trace ids and Prometheus metrics exist), and rate
limits that are per process (G13; a shared limiter is feature 010). Access-token `typ` enforcement
is available but off by default (G14). Operations: [`docs/runbooks/control-plane.md`](../../docs/runbooks/control-plane.md).

## Security review evidence (T001-034)

An independent, read-only review of the 001 code (authentication, tenant isolation, SQL, error
handling, the process, the container and CI) found no authentication bypass and no cross-tenant read
or write. It did find the issues below; each fix was written test-first or verified by removing it.

| # | Finding | Severity | Outcome |
|---|---|---|---|
| 1 | The key-cache refresh held its lock across network I/O, so an unauthenticated token with an unknown `kid` could stall every request | Medium | Fixed: lock-free lookups, stale-while-revalidate, one detached bounded fetch (`fault/provider.rs`) |
| 2 | Each cross-tenant 404 committed an unbounded audit row and leaked existence through latency | Medium | Fixed: off the response path, deduplicated atomically, bounded (`integration/store.rs`, `e2e/us04.rs`) |
| 3 | No header-read timeout or connection cap (Slowloris); one stuck connection blocked shutdown | Medium | Fixed: own HTTP/1 loop with timer, cap and bounded grace (`fault/server.rs`) |
| 4 | "Append-only audit" was a convention; the API ran as the owner | Medium | Fixed: least-privilege `sagat_app`, `ENABLE ALWAYS` triggers (`integration/runtime_role.rs`, `scripts/smoke-compose.sh`) |
| 5 | The insecure-JWKS flag was honoured in production | Low-Medium | Fixed: refused unless `SAGAT_ENV=dev` |
| 6 | No rate limiting; `/v1/ready` unauthenticated and DB-backed | Low | Fixed for one instance: per-tenant token bucket, in-flight cap, 1 s readiness cache (`fault/limits.rs`); per-process limits remain: G13 |
| 7 | PostgreSQL TLS not enforced | Low | Fixed: required outside dev |
| 8 | Role grants and provisioning not audited; compose pinned `SAGAT_ENV=dev` | Low | Fixed: audited atomically (D14); `SAGAT_ENV` configurable |
| 9 | `crit` header, array `iss`, empty key set replacing a good cache | Low | Fixed; `typ` enforcement open: G14 |
| 10 | Logs could carry URL credentials; unbounded echo of rejected input | Low | Fixed: redaction and clipping; spoofable trace id accepted: G15 |
| 11 | Smoke test could wipe real volumes; mutable Action tags; no advisory scanning | Low | Fixed: throw-away project name, SHA-pinned actions, `cargo audit` (one justified ignore: G16) |

The operational runbook is [`docs/runbooks/control-plane.md`](../../docs/runbooks/control-plane.md).

### Second pass (rate limiting, metrics, readiness, `provision-tenant`, start-up)

A second independent review of the code added after the first pass found no authentication bypass and
no cross-tenant access. It found the issues below. Every fix was written test-first: the new test
was run against the old code and failed for the stated reason before the change (the commit message
of each fix records it). "Report #" is the numbering of the review.

| Report # | Finding | Severity | Outcome | Proof |
|---|---|---|---|---|
| 1 | The HTTP `method` label was the raw request method; hyper accepts any token, the recorder never forgot a series (300 methods -> 300 series; 100 KB methods -> 41 MB RSS), even with `SAGAT_ADMIN_ADDR=off` | High | Fixed: closed label set (`GET`..`OPTIONS`, else `OTHER`), 10-minute idle expiry of every series, released every minute by the process even when nothing scrapes (D18) | `fault/metrics.rs::arbitrary_http_methods_cannot_create_unbounded_metric_series`, `crates/telemetry/tests/metrics.rs` |
| 2 | One authenticated member could hold every global slot with POSTs that announce a body and never send it (15 s each) | Medium | Fixed: body idle timeout (`SAGAT_BODY_IDLE_TIMEOUT_SECS`, 5 s, `408`) and a per-tenant concurrency cap (`SAGAT_MAX_IN_FLIGHT_PER_TENANT`, a quarter of the global cap, `429`) held by the `Caller` until the handler returns (D20). **Open:** a body trickled just under the idle timeout holds a slot until the 15 s request timeout, bounded by the per-tenant cap | `fault/limits.rs::stalled_uploads_of_one_tenant_cannot_starve_the_others`, `crates/api-http/tests/tenant_concurrency.rs`, `apps/control-plane/tests/config.rs` |
| 3 | A valid token without membership (or with an unknown tenant) cost two lookups per request and never reached the tenant limiter (300 requests -> 600 lookups, no `429`) | Medium | Fixed: per-subject bucket (`sub`; 20 req/s, burst 40; `SAGAT_SUBJECT_RATE_LIMIT_RPS`/`BURST`) after the JWT verifies and before `authenticate` (D19) | `fault/limits.rs::a_valid_token_without_membership_is_throttled_before_it_costs_database_lookups` (counts the lookups through a decorated store) |
| 7 | One viewer could spend the whole tenant bucket | Low | Fixed by the same per-subject bucket | `fault/limits.rs::one_member_cannot_exhaust_the_tenant_budget_on_their_own` |
| 4, 14 | `/v1/ready` was not single-flight (300 concurrent probes -> 300 pings), had no ping timeout, discarded the error, and the schema-missing branch logged nothing | Low | Fixed: one shared ping, 500 ms timeout (not ready), cause logged at `warn` and never returned (D21) | `fault/readiness.rs` (three tests) |
| 5 | `Subject::parse("system:provisioning")` succeeded although the doc said nobody can authenticate as it | Low | Fixed: the `system:` namespace is reserved (`SubjectViolation::Reserved`); `provision-tenant` and `seed-dev` refuse it with exit code 2 (D22) | `crates/domain/tests/identity.rs`, `crates/application/tests/identity_service.rs::a_token_cannot_authenticate_as_the_platform_actor_even_if_a_membership_exists`, `apps/control-plane/tests/cli.rs` |
| 6 | `SAGAT_RATE_LIMIT_RPS=1e-300` was accepted and the first over-budget request panicked (`200, 200, 500, 500`) | Low | Fixed: finite rate of at least 0.001 and finite burst of at least 1 are validated in the limiter (typed `RateLimitConfigError`) and in the configuration; the wait is capped at one hour | `crates/api-http/tests/rate_limiter.rs`, `apps/control-plane/tests/config.rs` |
| 8 | Above 10 000 tracked tenants every call swept the table under the global lock (217 us per call at 20 000) | Low | Fixed: the sweep runs at most once per refill window, the table holds at most 100 000 keys, keys beyond share an overflow bucket. The cost is bounded by construction and tested with a fake clock; the 217 us figure was **not** re-measured | `rate_limiter.rs::pruning_is_throttled_to_once_per_interval_not_once_per_call`, `keys_beyond_the_capacity_share_one_overflow_bucket` |
| 9 | `check_at` could move its clock backwards; the shed `Retry-After` was a constant 5 | Info | Fixed: monotonic update; the shed hint cycles through 3 to 8 s | `rate_limiter.rs::a_late_stale_timestamp_cannot_rewind_the_refill_clock`, `fault/limits.rs::when_saturated_the_api_sheds_load_at_once_and_probes_keep_answering` |
| 10 | Nothing stopped `serve` from running as the owner or a superuser | Low | Fixed: outside dev `serve` refuses a role that is a superuser, owns `audit_records` or may UPDATE/DELETE/TRUNCATE it, and an unmigrated database; `SAGAT_MIGRATE_ON_START` is dev-only (D24). Other role attributes are not checked, and the check runs once at start | `integration/least_privilege.rs`, `e2e/us03.rs` (five tests) |
| 11 | The unauthenticated `/metrics` port could be exposed without any hint | Low | Mitigated: a `WARN` at start when it is bound to a non-loopback address, and the runbook says to restrict it with a NetworkPolicy (D25). Not enforced | `apps/control-plane/tests/config.rs`, `e2e/us03.rs::a_metrics_listener_reachable_from_a_network_logs_a_warning` |
| 12 | `SAGAT_OIDC_REQUIRED_TYP` rejected `application/at+jwt` and, with a trailing newline, every token | Low | Fixed: media-type comparison and trimmed value (D23) | `contract/auth.rs` (two tests), `apps/control-plane/tests/config.rs` |
| 13 | CI ran an unpinned `npx @asyncapi/cli@3` with install scripts | Low | Fixed: pinned to 3.6.0 with `--ignore-scripts`; the command line was run locally and exits 0. **Not run in CI from this branch.** Transitive dependencies are still resolved at run time | `.github/workflows/ci.yml` |
| 9 (audit) | `provision-tenant` audit rows do not record the operator or the database session user | Low | **Not fixed**, recorded as G17 | research.md |
| - | The shed `Retry-After` does not match the pool size (512 in flight vs 10 connections) | Low | **Not fixed**, recorded as G18 | research.md |

Also changed: the two wall-clock assertions in `apps/control-plane/tests/cli.rs` (2 s) flaked under
CPU load; exit code 2 without `database error` already proves that no connection was attempted, so
the bound is now 10 s and only catches a hang.

Result on the branch that closed this pass (macOS, Docker Desktop, PostgreSQL 16 through
Testcontainers, 2026-10-08): `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
--locked -- -D warnings` and `python3 scripts/check-architecture.py` exit 0, and `cargo test
--workspace --locked --no-fail-fast` passes 268 tests with 0 failures and 1 ignored (the benchmark).
Three fault tests had timing-dependent results on a loaded machine (one of them, the tenant
boundary flood, also on the code before this pass: 600 simultaneous connects overflow the macOS
accept queue) and were made deterministic without weakening what they prove; the commit
`test(fault): remove three sources of flakiness` explains each. Not run from this branch: the CI
jobs themselves, `scripts/smoke-compose.sh` (the Docker image build) and `cargo audit`.
