# Validation quickstart: Script registry, code editor and inline execution

Everything below runs against the real stack. Nothing is simulated except time, where a test says so.

## Prerequisites

- Rust toolchain pinned by the repository, **Docker** (the sandbox is containers, and the tests use Testcontainers for PostgreSQL), and the images `node:24-alpine` and `python:3.13-alpine` (pulled on first use).
- Build the pinned C# images from the repository root before starting the runner or the container/E2E suites:

  ```sh
  docker build -f runners/dotnet/Dockerfile.builder -t sagat/dotnet-builder:10.0.401 .
  docker build -f runners/dotnet/Dockerfile.runtime -t sagat/dotnet-runtime:10.0.12 .
  ```
- Build Go for the host's native Linux architecture (`arm64` or `amd64`) and the Java 17 images.
  Do not use an emulated Go execution as native capability evidence:

  ```sh
  docker build --platform linux/$TARGETARCH --build-arg TARGETARCH=$TARGETARCH \
    -f runners/go/Dockerfile.builder -t sagat/go-builder:1.25.3 .
  docker build --platform linux/$TARGETARCH --build-arg TARGETARCH=$TARGETARCH \
    -f runners/go/Dockerfile.runtime -t sagat/go-runtime:1.25.3 .
  docker build --platform linux/$TARGETARCH --build-arg TARGETARCH=$TARGETARCH \
    -f runners/java/Dockerfile.builder -t sagat/java-builder:17.0.16 .
  docker build --platform linux/$TARGETARCH --build-arg TARGETARCH=$TARGETARCH \
    -f runners/java/Dockerfile.runtime -t sagat/java-runtime:17.0.16 .
  ```
- `docker info` must work for the user running the tests and the runner.

## The automated evidence

```sh
# Domain, application and adapter units (no Docker)
cargo test -p sagat-domain -p sagat-application -p sagat-infra-schema -p sagat-infra-sandbox -p sagat-sandbox-runner

# The registry against a real PostgreSQL: versions, signing, leases, quotas, retention, migration
cargo test -p sagat-tests --test integration_inline_code_sandbox

# The sandbox against real containers: what a hostile script cannot do (T005-031)
cargo test -p sagat-tests --test integration_inline_code_sandbox_containers

# The user stories through the API, the worker protocol and the runner
cargo test -p sagat-tests --test e2e_inline_code_sandbox          # US01, US02, US03 (real C#), US04

# Failure: runner dies mid-build and mid-step, restarts, engine down, cancelled run (the stop is reported), room for one, scripts that become ready at different times
cargo test -p sagat-tests --test fault_inline_code_sandbox

# The contract: every one of the 25 operations, normal and error paths
cargo test -p sagat-tests --test contract_rest_inline_code_sandbox

# The benchmark (on purpose; release mode)
SAGAT_PERF_REPORT=tests/performance/reports/inline-code-sandbox.md \
  cargo test -p sagat-tests --test perf_inline_code_sandbox --release -- --ignored --nocapture

# The focused C# build/runtime benchmark writes its own report
SAGAT_CSHARP_PERF_REPORT=tests/performance/reports/inline-code-sandbox-csharp.md \
  cargo test -p sagat-tests --test perf_inline_code_sandbox --release \
  csharp::csharp_build_and_execution_benchmark -- --ignored --nocapture

# Native Go/Java production-path capability, mediation, isolation, timeout and cleanup
cargo test -p sagat-infra-sandbox production_path -- --ignored --nocapture
cargo test -p sagat-infra-sandbox a_go_cpu_burn_is_stopped_by_the_production_quota \
  -- --ignored --nocapture

# Focused native Go/Java measurements (six isolated build+run samples per language)
cargo test -p sagat-infra-sandbox go_and_java_native_container_measurements \
  -- --ignored --nocapture --test-threads=1
```

The checked-in results are the [feature baseline](../../tests/performance/reports/inline-code-sandbox.md)
and the [focused C# measurements](../../tests/performance/reports/inline-code-sandbox-csharp.md).
The [focused native Go/Java measurements](../../tests/performance/reports/inline-code-sandbox-go-java.md)
record ARM64 separately and leave AMD64 gated on its native CI proof. The reports describe local
measurements and their environment rather than production SLOs.

### What the Go/Java ARM64 completion run said (2026-10-12, this machine, Colima/Docker 29)

| Suite | Result |
|---|---|
| `sagat-infra-sandbox` v3/schema/protocol units | 12 schema + 22 sandbox passed (5 intentional native-container/benchmark tests ignored by default) |
| Native production-path tests | Go startup probe, Go and Java mediated host calls, redaction, direct-network denial, target mismatch and cleanup passed |
| Native Go quota test | one-second CPU burn stopped as `Timeout`; cleanup passed |
| Existing `integration_inline_code_sandbox_containers` | 30/30 TS/Python/C# compatibility and isolation cases passed serially |
| Focused benchmark | 6/6 Go and 6/6 Java build/runs passed; zero owned containers remained |

This is Linux ARM64 evidence only. Linux AMD64 is not enabled by it; the native AMD64 CI matrix must
pass the same builder, v3 validator, startup probe and sealed-memory runtime before advertisement.

### What the C# completion run said (2026-10-11, this machine, Colima/Docker 29, PostgreSQL 16)

| Suite | Result |
|---|---|
| `integration_inline_code_sandbox_containers` (real containers) | 30 passed, including 7 C# build/runtime/supply-chain/isolation cases |
| `integration_inline_code_sandbox` (registry, PostgreSQL) | 41 passed |
| `e2e_inline_code_sandbox` | 28 passed, including real NuGet test/publish/workflow and stable Go/Java refusal |
| `fault_inline_code_sandbox` | 12 passed, including C# test-run lease loss/takeover |
| `contract_rest_inline_code_sandbox` | 1 passed (25 operations) |
| `cargo test --workspace` | 95 test binaries, 1328 passed, 0 failed, 6 ignored (the benchmarks) with `a_dispatcher_cut_off_at_any_await…` of feature 004 skipped, because it hung in most runs on `develop` too (6 of 8). Its cause was an sqlx 0.9.0 bug that leaves a connection in a transaction when a `begin` is cut off (ADR-0014); after the fix (`bugfix/dispatcher-cancel-hang`) the run with nothing skipped is 95 binaries, 1331 passed, 0 failed, 6 ignored |

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
   SAGAT_RUNNER_GO_TARGET=linux-arm64 SAGAT_RUNNER_GO_REQUIRED=true \
   cargo run -p sagat-sandbox-runner --bin sandbox-runner
   ```

   Use `linux-amd64` only in a deployment whose native AMD64 CI proof passed. The runner compiles and
   executes its bundled Go probe before advertising Go. If Go is optional, omit
   `SAGAT_RUNNER_GO_REQUIRED`; probe failure then removes Go from that runner's capabilities. One
   Go-enabled runner pool must use one target until worker claims carry a target.

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

- TS, Python, C#, Go and Java go from their dedicated builders to isolated execution with the limits, network and secrets held. C# uses an isolated Roslyn build container, a separate runtime container and an offline exact NuGet catalog (`integration_inline_code_sandbox_containers::csharp`, US03). Go uses a signed native v3 artifact and a four-seal memfd loader; Java uses validated Java 17 class files and a parent-first trusted host (ADR-0024, US05/US06).
- A production script has no direct network. When its signed policy permits namespace egress, the runner reloads the stored namespace policy before every mediated fetch and refuses the fetch if the policy read or durable category-only incident acknowledgement fails. `e2e_security_multitenancy::us03` and `fault_security_multitenancy::egress` cover allowed/denied calls, revocation during one run, duplicates, timeout, runner loss and tenant scope.
- Reproducible, annotated errors: every failure has a stable `SCRIPT_*` code, the envelope's `traceId`, and a reason that never repeats the script's words or a secret.
- No confirmed transition lost: a build and a test run are leased and fenced; a script step is the engine's own lease; a late report from a runner that lost its lease is refused.
- `[x]` in `tasks.md` only for what a test above proves.
