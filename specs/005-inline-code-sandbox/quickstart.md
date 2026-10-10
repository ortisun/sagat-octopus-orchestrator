# Validation quickstart: Script registry, code editor and inline execution

Everything below runs against the real stack. Nothing is simulated except time, where a test says so.

## Prerequisites

- Rust toolchain pinned by the repository, **Docker** (the sandbox is containers, and the tests use Testcontainers for PostgreSQL), and the images `node:24-alpine` and `python:3.13-alpine` (pulled on first use).
- `docker info` must work for the user running the tests and the runner.

## The automated evidence

```sh
# Domain, application and adapter units (no Docker)
cargo test -p sagat-domain -p sagat-application -p sagat-infra-schema -p sagat-infra-sandbox -p sagat-sandbox-runner

# The registry against a real PostgreSQL: versions, signing, leases, quotas, retention, migration
cargo test -p sagat-tests --test integration_005

# The sandbox against real containers: what a hostile script cannot do (T005-031)
cargo test -p sagat-tests --test integration_005_sandbox

# The user stories through the API, the worker protocol and the runner
cargo test -p sagat-tests --test e2e_005          # US01, US02, US03 (the refusal), US04

# Failure: runner dies mid-build and mid-step, restarts, engine down, cancelled run (the stop is reported), room for one, scripts that become ready at different times
cargo test -p sagat-tests --test fault_005

# The contract: every one of the 25 operations, normal and error paths
cargo test -p sagat-tests --test contract_rest_005

# The benchmark (on purpose; release mode)
SAGAT_PERF_REPORT=tests/performance/reports/005-inline-code-sandbox.md \
  cargo test -p sagat-tests --test perf_005 --release -- --ignored --nocapture
```

### What the last run of each said (2026-10-10, this machine, Docker Desktop, PostgreSQL 16)

| Suite | Result |
|---|---|
| `integration_005_sandbox` (real containers) | 23 passed |
| `integration_005` (registry, PostgreSQL) | 41 passed |
| `e2e_005` | 27 passed (three consecutive runs; one earlier run failed `us04::a_script_that_floods_its_output_is_cut_off` with `logTruncated` false and was not reproduced in the next five — the assertion now prints the stored record if it comes back) |
| `fault_005` | 12 passed (twice in a row) |
| `contract_rest_005` | 1 passed (25 operations) |
| `cargo test --workspace` | 95 test binaries, 1328 passed, 0 failed, 6 ignored (the benchmarks) — **with `a_dispatcher_cut_off_at_any_await…` of feature 004 skipped**: it hangs in a large share of runs on `develop` as well (6 of 8 there), which is not this feature's (`research.md`, memory of the session) |

### The reviews (T005-035)

Two independent read-only reviews (security, correctness) of the whole feature produced 35 findings. Each was read against the code before anything changed; the table in `research.md` (D116) says what was done about each and which test shows it. Three kinds of evidence were used and are named as such there: a test that failed on the old code (the supervisor that wedged when a script stopped reading; the old runner that got one task for two ready scripts; five guards of the registry switched off to see five tests fail), a test written with the fix, and a fix with no test of its own (G120).

## By hand

1. A signing key and the stack. The control plane serves scripts only if it has a key.

   ```sh
   cp infra/docker/.env.example infra/docker/.env      # then edit
   echo "SAGAT_SCRIPT_SIGNING_KEY=$(docker run --rm sagat-octopus-orchestrator/control-plane:dev script-key)" >> infra/docker/.env
   docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
   ```

2. A runner identity: a membership with the role `runner` (roles are granted in the database, as for workers, `docs/runbooks/control-plane.md` §8), bound to the script activities of the namespace:

   ```sh
   curl -X PUT $API/v1/namespaces/$NS/worker-bindings/runner-1 \
        -H "Authorization: Bearer $ADMIN" -H 'content-type: application/json' \
        -d '{"activities": ["script.*"]}'
   ```

3. The runner, on a host that holds nothing else (it holds the container engine: `docs/runbooks/scripts.md`):

   ```sh
   SAGAT_ENV=dev SAGAT_RUNNER_API_URL=http://127.0.0.1:18080/v1 SAGAT_RUNNER_GRPC_URL=http://127.0.0.1:19000 \
   SAGAT_RUNNER_TOKEN_FILE=./runner.token SAGAT_RUNNER_NAMESPACES=payments=$NS \
   cargo run -p sagat-sandbox-runner --bin sandbox-runner
   ```

4. Write, try and publish (US01, US02):

   ```sh
   # write
   curl -X POST $API/v1/namespaces/$NS/scripts -H "Authorization: Bearer $DEV" -H 'content-type: application/json' -d '{
     "name": "double-it", "language": "typescript",
     "source": "export default function (input: { n: number }) { console.log(\"n =\", input.n); return { double: input.n * 2 }; }"
   }'
   # try, without publishing: 202, then read it back
   curl -X POST $API/v1/namespaces/$NS/scripts/$SCRIPT/test-runs -H "Authorization: Bearer $DEV" \
        -H 'content-type: application/json' -d '{"input": {"n": 21}}'
   curl $API/v1/namespaces/$NS/scripts/$SCRIPT/test-runs/$TEST -H "Authorization: Bearer $DEV"
   #   -> "result": {"double": 42}, "log": "n = 21", "inferredOutputSchema": {...}, "startMs", "wallMs"
   # publish: 202 BUILDING, then READY with the artifactDigest
   curl -X POST $API/v1/namespaces/$NS/scripts/$SCRIPT/versions -H "Authorization: Bearer $DEV"
   curl $API/v1/namespaces/$NS/scripts/$SCRIPT/versions/1 -H "Authorization: Bearer $DEV"
   ```

5. Pin it in a workflow and run it:

   ```json
   {"apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow", "metadata": {"name": "uses-double-it"},
    "entrypoint": "s", "nodes": [{"id": "s", "type": "script", "artifactDigest": "sha256:…", "secrets": ["payments-api-key"]}]}
   ```

   Publishing refuses a digest that no ready version of the tenant built (`UNKNOWN_SCRIPT`). The runner is handed the step as the activity `script.sha256-<hex>`; `GET /v1/namespaces/$NS/script-executions?digest=…` shows what ran.

6. Hold a hostile script (US04): publish `export default async (_i, octopus) => octopus.fetch('http://169.254.169.254/latest/meta-data/')` and run it: the step fails `SCRIPT_DENIED`, the execution lists `fetch http://169.254.169.254` under `denied`, and `script.violation` is on the audit trail once.

## Gates

- TS and Python go from the editor's API to execution, with the limits, the network and the secrets held (US01, US02, US04; `integration_005_sandbox`). **C# does not** (`LANGUAGE_NOT_BUILT`; gap G96 in `research.md`).
- Reproducible, annotated errors: every failure has a stable `SCRIPT_*` code, the envelope's `traceId`, and a reason that never repeats the script's words or a secret.
- No confirmed transition lost: a build and a test run are leased and fenced; a script step is the engine's own lease; a late report from a runner that lost its lease is refused.
- `[x]` in `tasks.md` only for what a test above proves.
