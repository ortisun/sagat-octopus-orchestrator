# Validation quickstart: Platform foundation and contracts

> Feature 001 is implemented in part. The commands below are real and were run; items marked
> **pending** are not available yet. The authoritative status per requirement is in
> [`docs/traceability.md`](../../docs/traceability.md).

## Prerequisites

- Rust toolchain: `rustup` reads `rust-toolchain.toml` and installs the pinned compiler.
- Docker (the integration tests start PostgreSQL 16 with Testcontainers; the local environment uses
  Docker Compose).
- Node.js 22 (only for the contract checks: OpenAPI lint and JSON Schema validation).
- An OIDC provider for real tokens. **Pending:** a development identity provider shipped with the
  repository (gap G10). Until then use your own provider, or the fake provider the test suites use.

## 1. Run the tests

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked                      # unit, contract, integration, e2e, fault
python3 scripts/check-architecture.py                # layering and domain-purity rules
python3 -m unittest discover -s scripts/tests        # tests of that checker
npx --yes @redocly/cli@1 lint specs/001-platform-foundation/contracts/openapi.yaml
cargo install cargo-audit --locked && cargo audit --deny warnings   # dependency advisories
```

Each user story has its own file (`tests/e2e/001-platform-foundation/us01.rs` … `us04.rs`); the
token matrix is `tests/contract/001-platform-foundation/auth.rs`; failure scenarios are in
`tests/fault/001-platform-foundation/`.

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
cargo run -p sagat-control-plane --bin control-plane -- serve
cargo run -p sagat-control-plane --bin control-plane -- --help        # every setting
```

A caller needs a token whose `sub` is a member of the tenant named in its `tenant` claim (the seed
creates tenant `dev` with administrator `dev-admin`). Memberships are read from the database on
every request, so removing one takes effect immediately.

## Validation flow of the scenarios

| Story | Scenario | Automated by |
|---|---|---|
| US01 | An administrator creates a workspace and a `dev` namespace; both are persisted and visible | `tests/e2e/001-platform-foundation/us01.rs` |
| US02 | A client with a valid tenant token lists namespaces and gets only its own, plus a trace id | `…/us02.rs` |
| US03 | Migrate (twice), seed, start; API and database are healthy | `…/us03.rs`, `scripts/smoke-compose.sh` |
| US04 | A tenant A user asks for tenant B's resource: nothing is revealed, the attempt is audited | `…/us04.rs` |

## Gates

- Contract/API + tenant isolation suite green, and a reproducible local environment with migrations
  executed twice (`scripts/smoke-compose.sh`).
- Errors are reproducible and annotated; no confirmed transition is lost.
- A task in `tasks.md` becomes `[x]` only when real tests have passed.

## Known limits

Recorded in [`research.md`](./research.md): no token revocation list (G7), create operations are not
idempotent through `Idempotency-Key` (G8), `403`s are not audited (G9), no OTLP export or metrics
(G6), no development identity provider (G10), no rate limiting or per-tenant fairness (G13) and no
access-token `typ` enforcement (G14).

## Security review evidence (T001-034, partial)

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
| 6 | No rate limiting; `/v1/ready` unauthenticated and DB-backed | Low | Open: G13 |
| 7 | PostgreSQL TLS not enforced | Low | Fixed: required outside dev |
| 8 | Role grants and provisioning not audited; compose pinned `SAGAT_ENV=dev` | Low | Fixed: audited atomically (D14); `SAGAT_ENV` configurable |
| 9 | `crit` header, array `iss`, empty key set replacing a good cache | Low | Fixed; `typ` enforcement open: G14 |
| 10 | Logs could carry URL credentials; unbounded echo of rejected input | Low | Fixed: redaction and clipping; spoofable trace id accepted: G15 |
| 11 | Smoke test could wipe real volumes; mutable Action tags; no advisory scanning | Low | Fixed: throw-away project name, SHA-pinned actions, `cargo audit` (one justified ignore: G16) |

Still pending for T001-034: an operational runbook, and a second pass after rate limiting (G13).
