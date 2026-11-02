# Validation quickstart: Additional script languages (feature 023)

Status: proposed demo for the Definition of Done. Nothing below has been run: the images, targets and
suites named here do not exist yet. When the feature is implemented, each command is executed and its
real result recorded at the end of this file, failures and skips included.

## Prerequisites

- Everything of the [005 quickstart](../005-inline-code-sandbox/quickstart.md): the pinned Rust toolchain,
  Docker for the user running tests and the runner, PostgreSQL through Testcontainers, the signing key.
- Build the new images natively for the host architecture (`TARGETARCH` is `amd64` or `arm64`). Emulated
  builds are not evidence for native Rust execution:

  ```sh
  docker build --platform linux/$TARGETARCH -f runners/shell/Dockerfile -t sagat/shell:dev .
  docker build --platform linux/$TARGETARCH -f runners/powershell/Dockerfile -t sagat/powershell:dev .
  docker build --platform linux/$TARGETARCH -f runners/php/Dockerfile.builder -t sagat/php-builder:dev .
  docker build --platform linux/$TARGETARCH -f runners/php/Dockerfile.runtime -t sagat/php-runtime:dev .
  docker build --platform linux/$TARGETARCH --build-arg TARGETARCH=$TARGETARCH \
    -f runners/rust/Dockerfile.builder -t sagat/rust-builder:dev .
  docker build --platform linux/$TARGETARCH --build-arg TARGETARCH=$TARGETARCH \
    -f runners/rust/Dockerfile.runtime -t sagat/rust-runtime:dev .
  ```

  Production profiles reference these images by digest; `SAGAT_ENV=dev` is the only setting in which a
  tag-only reference passes a new language's probe (D1214).

## Automated evidence (planned targets)

```sh
# Units: language ids, policy narrowing, bundle v1/v3/v4 validators, catalogs, provenance, shims' protocol
cargo test -p sagat-domain -p sagat-application -p sagat-infra-schema -p sagat-infra-sandbox -p sagat-sandbox-runner

# Registry, policy, presence, provenance and migration against real PostgreSQL
cargo test -p sagat-tests --test integration_script_languages

# Real containers: probes, builds, runs and the hostile battery per language (native host only)
cargo test -p sagat-tests --test integration_script_languages_containers -- --ignored --test-threads=1

# User stories through the API, the worker protocol and the runner
cargo test -p sagat-tests --test e2e_script_languages

# Faults: runner death, duplicate completion, lost hint, cancellation race, restart
cargo test -p sagat-tests --test fault_script_languages

# Contract: the merged OpenAPI operations and fields, normal and error paths
cargo test -p sagat-tests --test contract_rest_script_languages

# Five SDKs: canonical IR of script nodes in the four new languages
cargo test -p sagat-tests --test sdk_conformance

# Benchmark and the Deno/Bun spike (release mode, on purpose)
SAGAT_PERF_REPORT=tests/performance/reports/script-languages.md \
  cargo test -p sagat-tests --test perf_script_languages --release -- --ignored --nocapture
```

## By hand: the happy path (Bash, then Rust)

1. Start the stack and a runner as in the 005 quickstart, adding the new languages explicitly:

   ```sh
   SAGAT_ENV=dev SAGAT_RUNNER_API_URL=http://127.0.0.1:18080/v1 SAGAT_RUNNER_GRPC_URL=http://127.0.0.1:19000 \
   SAGAT_RUNNER_TOKEN_FILE=./runner.token SAGAT_RUNNER_NAMESPACES=ops=$NS \
   SAGAT_RUNNER_LANGUAGES=typescript,python,bash,rust \
   SAGAT_RUNNER_REQUIRED_LANGUAGES=bash \
   SAGAT_RUNNER_RUST_TARGET=linux-$TARGETARCH \
   SAGAT_RUNNER_IMAGE_BASH=sagat/shell:dev \
   SAGAT_RUNNER_IMAGE_RUST_BUILD=sagat/rust-builder:dev SAGAT_RUNNER_IMAGE_RUST_RUNTIME=sagat/rust-runtime:dev \
   cargo run -p sagat-sandbox-runner --bin sandbox-runner
   ```

   Expected: the log reports `script.probe` passed for `bash` and `rust`; a failed Bash probe stops the
   runner (required); a failed Rust probe only removes Rust.

2. Check availability: `curl $API/v1/namespaces/$NS/script-languages -H "Authorization: Bearer $DEV"`.
   Expected: `bash` and `rust` with `built: true, served: true`; `php` and `powershell` with
   `reason: NOT_SERVED`.

3. Create, try and publish the Bash starter (contract: [shim-protocol.md](contracts/shim-protocol.md)):

   ```sh
   cat > double.sh <<'EOF'
   n="$(octopus input | jq -r '.n // .input.n // 0')"
   octopus result "$(jq -cn --argjson n "$n" '{double: ($n * 2)}')"
   EOF
   jq -n --rawfile source double.sh '{name: "double-sh", language: "bash", source: $source}' |
     curl -X POST $API/v1/namespaces/$NS/scripts -H "Authorization: Bearer $DEV" \
          -H 'content-type: application/json' -d @-
   curl -X POST $API/v1/namespaces/$NS/scripts/$SCRIPT/test-runs -H "Authorization: Bearer $DEV" \
     -H 'content-type: application/json' -d '{"input":{"n":21}}'
   curl -X POST $API/v1/namespaces/$NS/scripts/$SCRIPT/versions -H "Authorization: Bearer $DEV"
   curl $API/v1/script-artifacts/$DIGEST -H "Authorization: Bearer $DEV"
   curl $API/v1/namespaces/$NS/scripts/$SCRIPT/versions/1 -H "Authorization: Bearer $DEV"
   ```

   A test run receives exactly its input; a workflow step receives the step document
   `{run, node, input, steps}` (005 D103), hence `.n // .input.n`.

   Expected: the test run succeeds with `{"double":42}` and inferred schemas; the version becomes `READY`;
   the artifact response has a CycloneDX `sbom` listing the toolset, and the version response has the
   `provenance` DSSE envelope of its build.
   Verify it offline with the public key (`getScriptSigningKey`) as [provenance.md](contracts/provenance.md)
   describes.

4. Rust: fetch the canonical lock, then publish:

   ```sh
   curl "$API/v1/namespaces/$NS/script-languages/rust/catalog?select=serde_json" -H "Authorization: Bearer $DEV"
   ```

   Create a `rust` script with the starter of `shim-protocol.md` and the returned `lockfile`, publish it
   and pin its digest from a workflow `script` node. Expected: the version is `READY` with a v3 artifact
   whose `target` equals the runner's; started with input `{"n":21}` and a starter adapted to read
   `input["input"]["n"]` from the step document, the workflow step completes with `{"double":42}`.

## By hand: the failure paths

1. **Hostile Bash**: publish `exec 3<>/dev/tcp/169.254.169.254/80` and a fork bomb `:(){ :|:& };:`.
   Expected: the first fails as `SCRIPT_ERROR` (no route, no network interface); the second ends at the
   process limit; `docker ps --filter label=sagat.sandbox=1` is empty afterwards.
2. **Undeclared secret**: `octopus secret not-declared` in a step. Expected: exit status 3, the step
   fails as the script decides, and `script.violation` holds `secret not-declared` once.
3. **Foreign crate**: publish a Rust script whose lock names a crate outside the catalog. Expected:
   `422 VALIDATION_FAILED` reason `PACKAGE_NOT_ALLOWED` at publication, or a `FAILED` version with kind
   `lockfile` when the control-plane check is bypassed in a test, with no network access attempted.
4. **Policy**: `PUT $API/v1/namespaces/$NS/script-runtime-policy` with `{"revision":0,"languages":{"bash":{"enabled":false}}}`
   as admin, then publish the Bash script again, and start a workflow run that pins the earlier Bash
   artifact. Expected: publication is `422` reason `LANGUAGE_DISABLED`; the run's Bash step is not leased
   and fails non-retryable `SCRIPT_DENIED`; each refusal has one `script.policy_denied` audit row (points
   `publish` and `lease`); a Bash attempt that was already running when the policy changed completes.
5. **Target mismatch**: run the Rust workflow on a runner configured for the other target. Expected:
   `SCRIPT_SANDBOX_UNAVAILABLE`, retried by the engine, never executed.
6. **Runner death**: kill the runner during a Rust build. Expected: after the lease expires another
   runner completes the same build key once; exactly one artifact row.

## Recorded results

None yet. This section is filled with the real outcome of every command above, the environment
(hardware, OS, engine, image digests) and every failure, when the feature is implemented (T023-059).
