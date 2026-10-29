# Tasks: Additional script languages (feature 023)

**Input**: `spec.md`, `plan.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md` of
`specs/023-script-languages/`.
**Status**: Proposed — not started. T023-061..063 were added after the architecture review and
T023-064..066 after the cross-feature review (coordinator decision C17), both on 2026-10-27; each sits in
the phase where it runs. No task is checked; a task becomes `[x]` only with the evidence it
names (test name and real result), never because this document exists.
**Format**: `[ID] [P?] [Story?] Description — files — evidence`. `[P]` means parallel: different files,
no dependency on an unfinished task. Tests are written first and must fail before the code they prove.

## Phase 1: Setup

- [ ] T023-001 Obtain owner acceptance of ADR-0057 and answers to O1200–O1202; record them in
  `specs/023-script-languages/research.md` §6 and the ADR status line — files
  `docs/adr/ADR-0057-additional-script-languages-reuse-the-sandbox-and-signed-artifacts.md`, `research.md`
  — evidence: dated owner decision. Blocks every later task.
- [ ] T023-002 [P] Add test targets `integration_script_languages`, `integration_script_languages_containers`,
  `e2e_script_languages`, `fault_script_languages`, `contract_rest_script_languages`, `perf_script_languages`
  and their directories under `tests/{integration,e2e,fault,contract,performance}/script-languages/` in
  `tests/Cargo.toml` — evidence: `cargo test -p sagat-tests --no-run --locked` builds them.
- [ ] T023-003 [P] Create fixtures in `tests/contract/script-languages/fixtures/`: malformed v4 bundles
  (dot segment, `..`, 11 segments, 256-byte path, case collision, 1,025 files, 1,400 KiB + 1 byte, NUL,
  missing `vendor/autoload.php` with `lockDigest`, wrong `entry`, language other than `php`); Rust ELF
  fixtures (static-PIE `ET_DYN`, `PT_INTERP`, wrong machine, writable-executable segment, executable
  stack); Cargo.lock and composer.lock fixtures (unknown package, checksum/reference mismatch, missing
  closure member, extra root, `packages-dev`, `composer-plugin`, lock version 3) — evidence: fixture list
  reviewed against FR-023-004/008.

## Phase 2: Foundational (blocks all stories)

- [ ] T023-004 Add `Language::{Bash, PowerShell, Php, Rust}` (`bash`, `powershell`, `php`, `rust`;
  extensions `sh`, `ps1`, `php`, `rs`) and the reserved `Language::PythonData` (`python-data`, `is_built`
  always false in this feature), artifact family, `is_built` false until each story flips it,
  per-language default runtime policy ("PowerShell runtime memory default 512 MiB") and build profile
  ("Rust build 1 GiB / 2,000 m / 120 s / 128 pids / 512 MiB scratch; ceiling 2 GiB / 4,000 m / 600 s /
  256 / 512 MiB") in `crates/domain/src/scripts.rs`, with unit tests — FR-023-001, 019.
- [ ] T023-005 [P] Add `RuntimePolicy` narrowing ("`packages` is `catalog`, `none` or `allow` with at most
  256 names"), `PackageRef`, `Permission::ScriptPolicyManage` (`script:policy:manage`, admin only) and audit
  actions `script.policy_changed`, `script.policy_denied` in `crates/domain/src/{scripts,identity,audit}.rs`,
  with unit tests that no other role holds the permission; policy points `draft|publish|test|workflow|lease`
  — FR-023-017, 021, 022.
- [ ] T023-006 Write `crates/infra-postgres/migrations/<next>_script_languages.{up,down}.sql` per
  data-model.md (ten-id CHECK including `python-data`, `script_build_provenance` keyed `(tenant_id, build_key)`
  with "envelope ≤ 65,536 bytes" and an update/delete-refusing trigger, `script_runtime_policies`
  "document ≤ 16,384 bytes", `script_runner_presence` "≤ 256 rows per tenant", grants for `sagat_app`) and the
  `crates/infra-postgres/storage-classification.json` entries — evidence: forward/rollback with data present
  in `tests/integration/script-languages/migration.rs`, down refusal with new-language rows, the manifest
  test, the runtime-role privilege sweep — FR-023-001, 007, 017, 018, 021.
- [ ] T023-007 [P] Implement `crates/infra-schema/src/catalog.rs`: embedded `runners/{rust,php}/catalog/catalog.json`
  per `contracts/catalog.schema.json`, Cargo.lock v4 and composer.lock parsers, closure check, policy subset
  check, canonical lock generation, catalog digest — evidence: unit tests over every T023-003 lock fixture
  plus property tests that generated locks round-trip — FR-023-004, 005, SC-023-003.
- [ ] T023-008 [P] Extend `crates/infra-schema/src/bundle.rs`: v1 accepts `bash`/`powershell`; v3 accepts
  `rust` with the unchanged `static_elf()` rules; new v4 for `php` and `python-data` (`runtimeCatalog` null
  for `php`, required for `python-data`) per `contracts/artifact-bundle-v4.schema.json`
  — evidence: every T023-003 bundle/ELF fixture refused; the existing v1/v2/v3 fixtures and tests unchanged
  — FR-023-008, 009, SC-023-004.
- [ ] T023-009 Add provenance: `crates/infra-schema/src/provenance.rs` (statement per `contracts/provenance.md`),
  DSSE signing/verification in `crates/application/src/script_signing.rs`, and in the build-completion
  transaction of `crates/application/src/scripts.rs` / `crates/infra-postgres/src/scripts.rs` the SBOM
  package set == lockfile package set check (`BUNDLE_MISMATCH`) and the insert into
  `script_build_provenance` keyed by build key; `ScriptVersion.provenance` in `getScriptVersion` — evidence:
  integration tests that artifact, signature, SBOM, provenance, version, job and audit commit together or
  not at all (injected failure before commit); that the same interpreted artifact digest built under two
  toolchains has two statements; and that a cache-finished version shows its build key's statement —
  FR-023-006, 007, 020, SC-023-005, NFR-023-002.
- [ ] T023-010 Extend the runner: `SAGAT_RUNNER_LANGUAGES` (default the five 005 languages),
  `SAGAT_RUNNER_REQUIRED_LANGUAGES`, `SAGAT_RUNNER_RUST_TARGET` (equal to Go's when both set),
  `SAGAT_RUNNER_MAX_NATIVE_BUILDS` ("default 1, ceiling 4"), image variables, digest-pinning outside
  `SAGAT_ENV=dev`, and `Sandbox::probe_language` failing closed for new languages in
  `apps/sandbox-runner/src/{config,lib}.rs`, `crates/application/src/sandbox.rs` — evidence: config and
  startup unit tests (optional failure omits, required failure stops, target mismatch stops, tag-only refused)
  — FR-023-015.
- [ ] T023-011 Add store and admission: policy CRUD with CAS, policy `FOR SHARE` at create/publish/test,
  early lockfile shape check (`LOCKFILE_INVALID`) in `crates/infra-postgres/src/scripts.rs` and
  `crates/application/src/scripts.rs` — evidence: `tests/integration/script-languages/policy.rs` against
  real PostgreSQL, including two tenants and an interleaved policy edit/publication — FR-023-017, 021.
- [ ] T023-012 [P] Add spans `script.build`, `script.run`, `script.probe`, `script.lockfile.validate`,
  `script.provenance.sign`, `script.policy.check` and metrics of plan.md (closed `language`, `reason`,
  `point`, `outcome` labels) in `apps/sandbox-runner/src/lib.rs` and `crates/api-http/src/routes/scripts.rs`
  — evidence: unit tests of the label sets — FR-023-022.
- [ ] T023-013 Add `listScriptLanguages` and `getScriptPackageCatalog` routes and the presence upsert on build
  and test claims in `crates/api-http/src/routes/{scripts,runner}.rs` — evidence:
  `tests/integration/script-languages/discovery.rs` (served within 120 s, stale after, foreign namespace
  `404`, presence row cap) — FR-023-018.
- [ ] T023-014 Move the sealed-memory loader to `runners/native/runtime.py` (language parameter only) and make
  `runners/go/Dockerfile.runtime` copy it — evidence: the `native-runners` CI job passes the existing Go
  production-path tests on `ubuntu-24.04` and `ubuntu-24.04-arm` (G1218) — FR-023-009.

**Checkpoint**: foundation ready; US01 and US02 may proceed in parallel.

## Phase 3: US01 — Write, test and publish a Bash script (P1) — MVP

**Independent test**: create, test, publish and run a Bash script through the API, the worker protocol
and real containers.

- [ ] T023-015 [P] [US01] Write `tests/e2e/script-languages/us01_bash.rs` covering the three acceptance
  scenarios and the five failure scenarios of US01 — evidence: fails before T023-018 — SC-023-001.
- [ ] T023-016 [US01] Create `runners/shell/` (`Dockerfile` pinned by digest with Bash 5.x and the D1211
  toolset, setuid/setgid stripped, inventory file; `host/` Rust crate with its own `Cargo.lock` for
  `sagat-shell-host` and `octopus`; probe source; `README.md`) per `contracts/shim-protocol.md` — evidence:
  host unit tests (first result wins, exit codes, 8-connection cap, environment allowlist) —
  FR-023-002, 003, 011, 012.
- [ ] T023-017 [US01] Wire Bash in `crates/infra-sandbox/src/docker.rs` (image, commands, first message,
  toolchain identity, `probe_language`) and `lockfile.rs` (empty only) — evidence:
  `tests/integration/script-languages/containers/bash.rs` (build, run, `SCRIPT_NO_RESULT`, `errexit`,
  mediated fetch/secret, redaction, no leftovers) — FR-023-002, 003, 010, 012.
- [ ] T023-018 [US01] Flip `is_built` for `bash`, accept it at admission, emit the toolset SBOM — evidence:
  T023-015 passes.
- [ ] T023-019 [P] [US01] Studio: `bash` in `frontend/apps/studio/src/features/scripts/languages.ts`, register
  `monaco-editor/languages/definitions/shell/register` in `monaco.ts`, starter, language list driven by
  `listScriptLanguages` (also corrects Go/Java, G1200), classification-aware test form — evidence: Vitest
  units and `frontend/e2e/tests/script-languages.spec.ts` (real browser) — FR-023-016, 023, SC-023-010.
- [ ] T023-020 [US01] Bash faults and tenancy in `tests/fault/script-languages/bash.rs`: runner killed during
  build, test run and step; duplicate completion; a script speaking the socket directly 1,000 times;
  another tenant's Bash artifact `404` — FR-023-020, 021.

## Phase 4: US02 — Compile and run a Rust script natively (P1)

**Independent test**: real builder, v3 validation and signing, sealed-memory execution with mediated
host calls, on each native architecture separately.

- [ ] T023-021 [P] [US02] Write `tests/e2e/script-languages/us02_rust.rs` covering US02 scenarios and failures.
- [ ] T023-022 [US02] Create `runners/rust/sagat-inline/` (fd 3 input, channel, `Octopus` API, panic capture)
  and `runners/rust/buildhost/` (fixed `main.rs` with `#![forbid(unsafe_code)]`, one `rustc` with the D1204
  and D1205 flags, empty environment) — evidence: unit tests and a golden test of the exact argument vector — FR-023-002, 009, 011.
- [ ] T023-023 [US02] Create `runners/rust/catalog/catalog.json` (O1200 default: `serde_json` closure) and
  `runners/rust/Dockerfile.{builder,runtime}` (pinned by digest, musl targets, prebuilt read-only libraries,
  advisory audit fatal at image build, shared loader) — evidence: image build log and catalog review record
  — FR-023-003, 004, 005.
- [ ] T023-024 [US02] Wire Rust in `crates/infra-sandbox/src/docker.rs` (images, native build semaphore, build
  profile, probe, target check at load, catalog-digest check) — evidence:
  `tests/integration/script-languages/containers/rust.rs` (static `ET_EXEC` accepted, PIE refused, `unsafe`
  refused, foreign crate refused, catalog digest mismatch `lockfile`, target mismatch
  `SCRIPT_SANDBOX_UNAVAILABLE`, CPU burn stopped, no leftovers) — FR-023-009, 010.
- [ ] T023-025 [US02] Extend `.github/workflows/ci.yml` `native-runners` to build the Rust images and run the
  Rust container tests on `ubuntu-24.04` and `ubuntu-24.04-arm`; enable each target only after its own run —
  evidence: two CI job links recorded separately in `quickstart.md` — FR-023-015, 026, SC-023-006.
- [ ] T023-026 [US02] Flip `is_built` for `rust`; early lock validation; SBOM purls `pkg:cargo/…`; provenance
  `resolvedDependencies` — evidence: T023-021 passes on both targets.
- [ ] T023-027 [P] [US02] Studio: `rust` definition, starter, catalog picker that writes the canonical lock
  from `getScriptPackageCatalog?select=` — evidence: Vitest and real-browser test.
- [ ] T023-028 [US02] Rust faults and tenancy in `tests/fault/script-languages/rust.rs`: runner killed
  mid-build under the Rust profile, lease takeover builds once, target-mismatch retry never executes,
  native build concurrency held at the ceiling — FR-023-019, 020.

## Phase 5: US05 — Contain hostile scripts in every new language (P1)

**Independent test**: the battery in real containers for each language on each enabled target.

- [ ] T023-029 [P] [US05] Write the battery `tests/integration/script-languages/containers/hostile.rs`,
  parameterized by language: metadata address, engine socket, host files, direct sockets (Bash `/dev/tcp`,
  PHP `fsockopen`, PowerShell `Invoke-WebRequest`, Rust `std::net`), undeclared secret, fork/memory/CPU/
  disk/log/channel floods, unread replies — evidence: 100% contained, zero leftovers, refusals audited once
  — FR-023-010..014, SC-023-002.
- [ ] T023-030 [US05] Bash-specific attacks: hostile input names (`BASH_ENV`, `ENV`, `BASH_FUNC_x%%`,
  `LD_PRELOAD`), background processes after exit, no setuid file (`find / -perm /6000`), `set -x` with a
  secret (exact value redacted; encoded form documented, G1204) — evidence: tests in `hostile.rs`.
- [ ] T023-031 [US05] Rust-specific attacks: `std::process::Command` of image binaries, `include_str!` of
  builder files (no secret present), attempts to add a proc-macro or build script through the source —
  evidence: tests in `containers/rust.rs`.
- [ ] T023-032 [US05] Write `tests/e2e/script-languages/us05_hostile.rs` through the API: refusals appear in
  `script.violation` once per actor, resource and reason; step failure codes are the 005 codes.
- [ ] T023-033 [US05] Independent security review (read-only reviewer, given the diff and the spec) of
  `runners/*`, `crates/infra-sandbox`, `crates/infra-schema`; record findings and actions as D entries in
  `research.md` — evidence: findings table with a test per fix or an explicit gap.

## Phase 6: US03 — Run a PHP script with a locked Composer package (P2)

- [ ] T023-034 [P] [US03] Write `tests/e2e/script-languages/us03_php.rs` covering US03.
- [ ] T023-035 [US03] Create `runners/php/` (`Dockerfile.builder` with pinned PHP and Composer by digest and
  catalog trees; `Dockerfile.runtime` without catalog; `php.ini` per D1209; `build.php`; `runtime.php`;
  `catalog/catalog.json` with the O1200 package; probe) — evidence: image build log, catalog review record
  — FR-023-002, 003, 005, 014.
- [ ] T023-036 [US03] Wire PHP and v4 in `crates/infra-sandbox/src/docker.rs` — evidence:
  `tests/integration/script-languages/containers/php.rs` (`allow_url_fopen` off, `phar.readonly`,
  class-map autoload, `packages-dev`/plugin refused, tree limits `limit`, cgroup OOM
  `SCRIPT_OUT_OF_MEMORY`) and the battery for `php` — FR-023-008, 010, 014.
- [ ] T023-037 [US03] Flip `is_built` for `php`; SBOM purls `pkg:composer/…` — evidence: T023-034 passes.
- [ ] T023-038 [P] [US03] Studio: `php` definition, starter, catalog picker — evidence: Vitest and real browser.

## Phase 7: US04 — Run a PowerShell script (P2)

- [ ] T023-039 [P] [US04] Write `tests/e2e/script-languages/us04_powershell.rs` covering US04.
- [ ] T023-040 [US04] Create `runners/powershell/` (`Dockerfile` with pinned PowerShell LTS by digest,
  `build.ps1`, `runtime.ps1`, `Sagat.Octopus/` module, environment opt-outs, probe) — evidence: module tests
  run in the image — FR-023-002, 003, 011, 013.
- [ ] T023-041 [US04] Wire PowerShell in `crates/infra-sandbox/src/docker.rs` — evidence:
  `tests/integration/script-languages/containers/powershell.rs` (streams to log, result shaping, depth 65
  fails `SCRIPT_ERROR`, `Invoke-WebRequest` fails, `Add-Type` stays inside the container, lockfile refused)
  and the battery for `powershell` — FR-023-010, 013.
- [ ] T023-042 [US04] Flip `is_built` for `powershell`; Studio `powershell` definition and starter —
  evidence: T023-039 passes; real-browser test.

## Phase 8: US06 — Govern languages and packages, inspect provenance (P2)

- [ ] T023-043 [P] [US06] Write `tests/e2e/script-languages/us06_policy.rs`: the five points (draft,
  publication, test run, workflow publication, step lease) refused with typed reasons or `SCRIPT_DENIED`
  and audited, stale revision `409`, foreign namespace `404`, an attempt leased before the change
  completes — SC-023-007.
- [ ] T023-044 [US06] Add `getScriptRuntimePolicy` and `putScriptRuntimePolicy` with CAS and
  `script.policy_changed` in the same transaction; `script.policy_denied` once per actor/resource/reason —
  files `crates/api-http/src/routes/scripts.rs`, `crates/api-http/src/error.rs` — FR-023-017, 022.
- [ ] T023-045 [US06] Refuse workflow publication of a script node whose artifact language is disabled
  (`SCRIPT_LANGUAGE_DISABLED`) beside `ScriptLookup::missing_artifacts` in `crates/application/src/definitions.rs`
  and `crates/application/src/scripts.rs` — evidence: publication integration test, including a policy
  change racing a publication.
- [ ] T023-061 [US06] Enforce the language policy at lease (D1226) in `crates/application/src/worker.rs`
  (`lease_one`) and `crates/infra-postgres/src/workers.rs`: for `script.sha256-*` steps read the artifact
  language and the policy row `FOR SHARE` after the run/step locks; a disabled language fails the step
  non-retryable `SCRIPT_DENIED` with journal, next work, outbox and `script.policy_denied` (point `lease`)
  in the lease transaction — evidence: `tests/integration/script-languages/lease_policy.rs` against real
  PostgreSQL (ready step refused; running attempt finishes and its retry is refused; re-enable lets a new
  attempt lease; concurrent policy update and lease never deadlock; crash before commit leaves the step
  ready; non-script activities untouched; poll and streaming paths both checked) — FR-023-017, 020,
  SC-023-007.
- [ ] T023-046 [P] [US06] Studio: admin policy page and the SBOM/provenance panel in
  `frontend/apps/studio/src/features/scripts/VersionView.tsx` (verified/reported fields labelled) —
  evidence: Vitest, real browser, accessibility checks.
- [ ] T023-047 [US06] Contract tests that verify a provenance fixture offline and refuse a modified payload,
  a swapped subject and an artifact signature presented as DSSE — `tests/contract/script-languages/provenance.rs`.

## Phase 9: US07 — Evaluate Deno and Bun (P3)

- [ ] T023-048 [P] [US07] Create spike images `runners/ts-variants/{deno,bun}/Dockerfile` (pinned by digest)
  and the harness `tests/performance/script-languages/ts_variants.rs` that runs the existing TypeScript
  fixtures through `DockerSandbox` with those images under the unchanged profile — evidence: harness runs.
- [ ] T023-049 [US07] Publish `tests/performance/reports/script-languages-ts-variants.md` (per-fixture parity,
  cold/warm start, peak memory, environment, failures) and confirm or revise D1201 in `research.md`; add a
  check that no product configuration names the variant images — FR-023-025.

## Phase 10: Polish, cross-cutting gates

- [ ] T023-050 [P] Extend `sdks/typescript/src/workflow.ts` and `sdks/python/sagat_octopus_orchestrator/workflow.py`
  unions; add `tests/contract/sdk-conformance/scenarios/script-languages.json` with expected bytes; run all
  five SDKs — evidence: `cargo test -p sagat-tests --test sdk_conformance` — FR-023-024, SC-023-011.
- [ ] T023-051 Merge `contracts/openapi-script-languages.yaml` and `contracts/workflow-script-language.schema.json`
  into `specs/001-platform-foundation/contracts/{openapi.yaml,workflow.schema.json}` (correct the stale
  language descriptions, G1200; confirm no worker.proto or AsyncAPI change); Redocly lint; REST contract
  suite `tests/contract/script-languages/rest/` over every new operation and field, and the existing 005
  suites unchanged — FR-023-001, 018, NFR-023-003.
- [ ] T023-052 Cross-language fault suite `tests/fault/script-languages/`: crash at claim, before completion,
  after artifact insert before commit (injected), after commit before acknowledgement; duplicate completion;
  lost broker hint for a ready step; step timeout racing completion and cancellation; control-plane and
  runner restart — evidence: no duplicate version, artifact or transition; identical idempotency keys —
  FR-023-020, SC-023-008, NFR-023-002.
- [ ] T023-053 [P] Quota and ceiling exhaustion tests: `ABOVE_CEILING` for every new limit, `script_jobs` at
  8, native build semaphore, v4 bounds, policy and presence caps — `tests/integration/script-languages/limits.rs`
  — FR-023-019, SC-023-012.
- [ ] T023-054 [P] Add the new operations to the OpenAPI-driven cross-tenant battery of feature 009
  (`tests/contract/security-multitenancy/rest/battery.rs`) — evidence: every new operation returns opaque
  `404` across tenants and audits the probe — FR-023-021, NFR-023-001.
- [ ] T023-055 [P] Telemetry tests `tests/integration/script-languages/telemetry.rs`: label sets closed;
  source, input, lock and secret canaries absent from logs, spans, metrics and audit — SC-023-009,
  NFR-023-004.
- [ ] T023-056 Benchmark per language and target; publish `tests/performance/reports/script-languages.md`
  with environment and failures; revise the D1213 values within ceilings and close or keep G1206/G1207 —
  SC-023-013, NFR-023-005.
- [ ] T023-057 Deployment: new images by digest in `infra/docker/compose.yaml` (`sandbox` profile) and
  `infra/helm/values.yaml`; network policy unchanged (runner egress); `tests/security/sandbox/kubernetes-network-policy.sh`
  still passes — FR-023-026.
- [ ] T023-058 Update `docs/runbooks/scripts.md` (variables, images, catalog review procedure, shell risks,
  policy, provenance verification), `docs/architecture/security-and-sandbox.md` (what a script can and
  cannot do, per language) and the 005 gap cross-references — FR-023-026, NFR-023-006.
- [ ] T023-059 Final gates: independent correctness review, `scripts/check-architecture.py`, `cargo clippy
  --workspace --all-targets -- -D warnings`, workspace tests, frontend checks; fill "Recorded results" in
  `quickstart.md` with real outcomes, skips and failures.
- [ ] T023-060 Traceability: add FR-023/SC-023 evidence to `docs/traceability.md`, note the 019 dependency
  (G1201) in the 019 plan, the `python-data` mechanism in 024's plan, and the shared-loader change in
  005's research — evidence: links to the merged tests.
- [ ] T023-062 [P] Reserve `python-data` (D1229, FR-023-027): the hash-pinned requirements lock parser and
  catalog kind `pypi` in `crates/infra-schema/src/catalog.rs`, the `runtimeCatalog` check at load in
  `crates/infra-sandbox/src/docker.rs`, the import-path restriction contract for the 005 Python shim, and
  `LANGUAGE_NOT_BUILT` at every admission point — evidence: fixture catalog and lock tests (exact match,
  missing hash, extra package, wrong target), a v4 `python-data` fixture refused `SCRIPT_SANDBOX_UNAVAILABLE`
  on a runtime with another catalog digest, and `createScript` with `python-data` refused — FR-023-001,
  008, 027, SC-023-014. Feature 024 supplies the image, catalog content and the enabling evidence.
- [ ] T023-063 Rollback observation (D1230): run a 0.2.0 control plane and runner against the migrated
  schema, first with no new-language row, then with one `bash` script and one provenance row; record the
  observed behaviour (expected: works, then the namespace's script listing fails with an internal error)
  in `quickstart.md` and the runbook — evidence: `tests/integration/script-languages/rollback.rs` or a
  scripted run with its output — NFR-023-003.

- [ ] T023-064 [P] Reserved host calls in the supervisor (FR-023-028, D1231–D1233):
  `checkpoint` and `stream.append` messages in `crates/infra-sandbox/src/protocol.rs`; lease-capability gate
  (`NOT_GRANTED`, always in test runs), "600 `checkpoint` and 1,000 `stream.append` calls per run", channel
  line ceiling 2,000, payload bounds, supervisor-assigned `details_sequence`/`publisher_seq`, and the secret
  net in `apps/sandbox-runner/src/host.rs`; the previous checkpoint in the first message — evidence: unit
  tests (gate, budgets, bounds, sequences, secret refusal, no payload in log/record/metric/audit) —
  SC-023-015.
- [ ] T023-065 Shims for all ten runtimes: `crates/infra-sandbox/shims/runtime.{mjs,py}` (TypeScript,
  Python, `python-data`), `runners/dotnet/Sagat.ScriptContract/`, `runners/go/octopus.go`,
  `runners/java/api/`, and the four new shims of `contracts/shim-protocol.md` — evidence: per-language
  container tests in `tests/integration/script-languages/containers/host_calls.rs` and the 005 container
  suites unchanged otherwise; lease lost during an append, runner restart with a queued checkpoint —
  FR-023-011, 028, SC-023-015.
- [ ] T023-066 With features 019 and 026 present: e2e `tests/e2e/script-languages/reserved_host_calls.rs` —
  a script checkpoints, is killed, and its next attempt reads the previous checkpoint; a script appends to
  a declared channel and a subscriber receives the events with offsets; undeclared channel and `sagat.`
  channel refused — evidence: e2e results; until 019/026 ship, this task stays open and T023-064/065 prove
  `NOT_GRANTED` — FR-023-028, SC-023-015.

## Dependencies & execution order

- T023-001 gates everything. Phase 2 blocks all stories. T023-014 must pass before T023-023/024.
- US01 and US02 are independent after Phase 2. US05 tasks for a language follow that language's wiring
  task (Bash after T023-017, Rust after T023-024, PHP in T023-036, PowerShell in T023-041).
- US03, US04 and US06 depend only on Phase 2 (US06's workflow check also on T023-018 or T023-026 for a
  fixture language). US07 depends only on Phase 2 and the existing TypeScript fixtures.
- T023-061 needs T023-011 and a fixture language (T023-018 or T023-026). T023-062 needs T023-007 and
  T023-008. T023-063 needs T023-006.
- T023-064 needs Phase 2; T023-065 needs T023-064 and each language's wiring task; T023-066 needs 019's
  checkpoint minor and 026's `AppendStreamEvents`.
- Phase 10 runs after the stories it covers; T023-059 is last.

## Parallel examples

- After Phase 2: T023-015, T023-016 and T023-021, T023-022 in parallel (different directories).
- Studio tasks T023-019, T023-027, T023-038, T023-046 are parallel with their stories' runner work.
- T023-053, T023-054, T023-055 in parallel during Phase 10.

## Implementation strategy

MVP is Phase 2 + US01 + the Bash part of US05: one new interpreted language end to end, contained and
observable. Then Rust (the highest-risk language) with its battery, then PHP, PowerShell, policy and
provenance UI, and the spike. Each increment keeps the 005 suites green and ships its own runbook lines.
