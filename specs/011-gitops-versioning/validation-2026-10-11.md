# Feature 011 validation — 2026-10-11

Environment: macOS arm64, repository-pinned Rust 1.98.0, isolated local PostgreSQL 18.6,
`SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:55439/postgres`. Each database test creates its own database.
The temporary PostgreSQL server was used only for this validation. These are local results, not CI results.

## Results

| Command / suite | Result |
|---|---|
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Pass |
| `python3 -m unittest discover -s scripts/tests -v` | 8 passed |
| `python3 scripts/check-architecture.py` | Pass: 20 crates, correct layer direction, no cycles |
| `cargo audit --deny warnings` | Pass |
| GitOps focused suites plus REST security battery | 74 passed, 0 failed, 0 ignored |
| `cargo test --workspace --locked --no-fail-fast` | Exit 101: 1,626 passed, 63 failed, 26 ignored |
| Studio `npm run check:api` and `npm run typecheck` | Pass after regenerating the API types |
| OpenAPI lint | Pass with 9 existing warnings |
| Protobuf compile, AsyncAPI validation, workflow IR example validation | Pass; AsyncAPI emits one informational version notice |

The 26 ignored workspace tests retain their existing opt-in requirements, including benchmarks,
SDK toolchains and browser journeys. No test was disabled for this run. Performance figures remain the
earlier recorded benchmark; no new performance result is claimed.

## Focused command

```sh
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:55439/postgres
cargo test -p sagat-tests --locked --no-fail-fast \
  --test contract_manifest_gitops_versioning --test contract_rest_gitops_versioning \
  --test contract_rest_security_multitenancy --test e2e_gitops_versioning \
  --test fault_gitops_versioning --test integration_gitops_versioning
```

Use the URL of a disposable PostgreSQL server you control. Counts by target: manifest contract 3,
GitOps REST contract 1, REST security 5, GitOps e2e 15, faults 15, integration 35.

## Regression proved before and after the fix

`requests::a_retry_storm_with_one_key_gets_one_answer_and_later_the_same_one` first failed because
retrying a committed publication after saving a new invalid draft returned `SUPERSEDED`. It passed
after recovery was changed to read the committed publication using the immutable request fingerprint.
A final focused run also verified that only one publication audit entry exists. The test checks one
version, one activation, the original version on replay, final request state, and refusal of a different key.
REST conformance covers replay with `created=false` and continued permission and tenant enforcement.

## Container failures and remaining gate

Docker 29.8.2 responds, but container creation fails with HTTP 500:

```text
write /var/lib/desktop-containerd/daemon/io.containerd.metadata.v1.bolt/meta.db: read-only file system
```

The initial Docker-based focused run failed during PostgreSQL setup. Switching only the test database
to the local server allowed the complete feature 011 suite to pass. The workspace still exercises NATS,
Kafka, SQS and sandbox containers, so it cannot pass in this environment:

| Failed target | Failed tests |
|---|---:|
| `e2e_durable_execution` | 1 |
| `e2e_events_webhooks_messaging` | 5 |
| `e2e_inline_code_sandbox` | 24 |
| `fault_inline_code_sandbox` | 7 |
| `integration_durable_execution` | 3 |
| `integration_inline_code_sandbox_containers` | 23 |

Broker tests report failed container creation; sandbox paths report unavailable execution/build
infrastructure. These failures remain failures, not passes or exclusions. PostgreSQL 16 in containers
and `scripts/smoke-compose.sh` still need verification on a working Docker installation. SC-011-003
therefore remains partial.
