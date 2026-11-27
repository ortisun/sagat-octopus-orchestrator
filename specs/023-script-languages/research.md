# Research: Additional script languages (feature 023)

Baseline investigation: 2026-10-26, worktree `feature/018-032-parity-specs` cut from `origin/develop`
at `70f09435` (release 0.2.0). Nothing in this feature is implemented. Every path below was opened
or listed in that tree; where the code and a document disagree, the code is quoted.

## 1. What exists today (feature 005 and its ADRs)

| Area | Evidence | What 023 reuses or changes |
|---|---|---|
| Language set | `crates/domain/src/scripts.rs` `enum Language { TypeScript, Python, CSharp, Go, Java }`; `is_built()` true for all five; `FromStr` accepts exactly `typescript, python, csharp, go, java` | Additive variants `bash`, `powershell`, `php`, `rust` |
| Database guard | `crates/infra-postgres/migrations/20261014000000_scripts.up.sql` line 36: `CONSTRAINT scripts_language_valid CHECK (language IN ('typescript', 'python', 'csharp', 'go', 'java'))` | Replaced by an additive migration; down refuses while rows of new languages exist |
| Contracts | `specs/001-platform-foundation/contracts/openapi.yaml` (`Script`, `ScriptSummary`, `ScriptVersion` enums of five; `CreateScriptRequest.language` description still says C#/Go/Java are refused) and `workflow.schema.json` (`script.language` enum of five, "Informational") | Additive enum values; the stale description is G1200 |
| Container profile | `crates/infra-sandbox/src/docker.rs` `config()`: `network_mode: none`, `readonly_rootfs`, `cap_drop: ALL`, `no-new-privileges`, user `65534:65534`, `/tmp` tmpfs `rw,noexec,nosuid,nodev,size=<scratch>`, `pids_limit`, ulimits `nofile 256`, `core 0`, `fsize`, `cpu`, `ipc_mode: private`, log driver `none` | Unchanged for every new language (FR-023-010) |
| Images | `docker.rs` `struct Images` (typescript, python, csharp_build/runtime, go_build/runtime, java_build/runtime); defaults `node:24-alpine`, `python:3.13-alpine`, `sagat/*` tags; runner variables `SAGAT_RUNNER_IMAGE_*` in `apps/sandbox-runner/src/config.rs` | New fields and variables per language |
| Toolchain identity | `docker.rs` `toolchain()`: `build=<id>;runtime=<id>;protocol=artifact-v3;target=…;abi=sagat-inline-v1;compiler=go1.25.3;…` for native, `<image id>;shim=<hash>` for TS/Python | Same shape, plus catalog digest for languages with packages |
| Build key | `crates/application/src/sandbox.rs` `build_key` (length-prefixed language, source, lockfile, toolchain, policy, schemas) | Unchanged; catalog identity enters through the toolchain string |
| Build limits | `docker.rs` build job: `limits: request.policy.within(&Policy::default()).limits` ("a build gets at most the default limits") | A bounded per-language build profile for Rust (D1213) |
| Bundles | `crates/infra-schema/src/bundle.rs`: v1 text (TS/Python), v2 base64 managed DLLs (C#), v3 target-aware native/class files (Go/Java); `MAX_FILES = 32`, `MAX_BUNDLE_BYTES = 2 MiB`, `MAX_DECODED_BYTES = 1400 KiB`, `MAX_EXPANDED_BYTES = 8 MiB`; `static_elf()` requires `ET_EXEC`, the signed machine, no `PT_INTERP`, safe `PT_GNU_STACK`, no writable-executable segment | v1 for Bash/PowerShell, v3 for Rust, new v4 for PHP (D1202, D1203) |
| Native execution | `runners/go/runtime.py`: canonical gzip check, four seals `F_SEAL_WRITE|GROW|SHRINK|SEAL`, input on sealed fd 3, `os.execve` of the memfd; ADR-0024 decision 4 | Reused for Rust (D1206) |
| Startup probes | `crates/application/src/sandbox.rs` `Sandbox::probe_language` (default `Ok` except Go, which fails closed); `apps/sandbox-runner/src/lib.rs` `Runner::start` probes then collects toolchains; `SAGAT_RUNNER_GO_TARGET`, `SAGAT_RUNNER_GO_REQUIRED` | Every new language requires a production-path probe (D1214) |
| Stored language decoding | `crates/infra-postgres/src/scripts.rs` `language_of`: an unknown stored language is `corrupt("script language")`, an internal store error, so a 0.2.0 binary fails a listing that contains a row of a new language | Rollback is forward-only once new-language rows exist (D1230, G1220) |
| Step leasing | `crates/application/src/worker.rs` `claim` / `lease_one` with SQL in `crates/infra-postgres/src/workers.rs`; workflow publication checks pinned digests through `ScriptLookup::missing_artifacts` (`crates/application/src/scripts.rs`, used by `crates/application/src/definitions.rs`) | Lease-time policy check (D1226); publication check beside `missing_artifacts` |
| Reserved host calls (drafts) | 019 FR-019-018 asks the runner to expose checkpoint recording and the previous checkpoint through the supervisor channel; 019 `contracts/worker-protocol.md` puts `details_json`/`details_sequence` on `HeartbeatRequest` and a `DetailsStatus` enum on the response; 026 FR-026-020 has scripts publish by asking their supervisor, and 026 `contracts/worker-protocol.md` defines `AppendStreamEvents` (16 KiB event, 64 events / 64 KiB batch defaults) and names the script request `stream.publish` | Coordinator decision C17: this feature defines the shim side, named `checkpoint` and `stream.append` (D1231–D1233, G1224) |
| Capability advertisement | `lib.rs` registers `WorkerCapabilities { languages, activities, … protocol_version: "1" }`; build claims send `toolchains` (`ClaimBuildRequest`, maxLength 512); test claims send languages; `worker_sessions.languages text[]` (migration `20261009000100_execution.up.sql`); engine refuses an empty activity list (`crates/application/src/worker.rs` line 607) | Runner presence row for advisory availability (D1219) |
| Host channel | `crates/infra-sandbox/src/protocol.rs` messages `ready, fetch, secret, result, error, built, rejected`; `docker.rs` `MAX_HOST_CALLS = 100`, `MAX_CHANNEL_LINES = 400`, `MAX_DENIED = 32`, `MAX_DENIED_LEN = 200`, `ENGINE_CALL = 20 s`, `SETUP_GRACE = 60 s`; `HostCalls` port in `sandbox.rs` | Same protocol for every shim (FR-023-011) |
| Lockfiles | `crates/infra-sandbox/src/lockfile.rs`: C# exact Newtonsoft.Json 13.0.4 catalog (content hash, archive SHA-512, assembly SHA-256); others refuse package-bearing locks | Generalized closed catalogs for Rust and PHP (D1207, D1208, D1216) |
| SBOM | `docker.rs` build completion emits CycloneDX 1.5 with a `platform` component and package components; schemas in `tests/contract/inline-code-sandbox/cyclonedx-bom-1.5.schema.json` | Extended with purls and toolset inventory |
| Provenance | No in-toto/SLSA statement exists anywhere in `crates/` or `apps/`; provenance today is the version's `toolchain`, `key_id` and `build_key` | New signed statement (D1217) |
| Failure codes | `crates/domain/src/scripts.rs` `Failure::code()`: `SCRIPT_ERROR, _TIMEOUT, _OUT_OF_MEMORY, _OUTPUT_LIMIT, _DENIED, _SCHEMA_VIOLATION, _NO_RESULT, _SANDBOX_UNAVAILABLE, _BUILD_REJECTED, _UNTRUSTED_ARTIFACT`, `STORAGE_QUOTA_EXCEEDED`; `crates/api-http/src/error.rs` maps `LanguageNotBuilt` to `LANGUAGE_NOT_BUILT` | No new runtime code (D1221) |
| Permissions and audit | `crates/domain/src/identity.rs` `script:read|write|publish|build|execute`; `crates/domain/src/audit.rs` `script.created|changed|deleted|published|built|tested|violation` | One new permission and two audit actions |
| Metrics | runner: `script_runs_total{language,outcome}`, `script_start_seconds{language,temperature}`, `script_run_seconds{language}`, `script_builds_total{outcome}`, `script_build_seconds`, `script_denied_total`, `script_memory_peak_bytes` | Closed `language` label grows by four values |
| Studio | `frontend/apps/studio/src/features/scripts/languages.ts` (five entries; Go and Java still `built: false`, G1200); `monaco.ts` registers only `csharp`, `python`, `typescript`; `package.json` pins `monaco-editor` 0.57.0, whose package ships `esm/vs/languages/definitions/{shell,powershell,php,rust}` (checked in an installed copy of 0.57.0) | Syntax highlighting for four new languages |
| SDKs | `sdks/typescript/src/workflow.ts` `language?: 'typescript' | 'python' | 'csharp' | 'go' | 'java'`; `sdks/python/sagat_octopus_orchestrator/workflow.py` `Literal[...]` of five; Go/C#/Java accept any string (`sdks/go/workflow/options.go` `ScriptLanguage`, C# `ScriptOptions.Language`, Java `ScriptOptions.language`) | Unions extended; conformance scenario |
| CI | `.github/workflows/ci.yml` job `native-runners` on `ubuntu-24.04` and `ubuntu-24.04-arm` builds Go/Java images and runs `cargo test -p sagat-infra-sandbox --lib --locked -- --ignored` | Rust images and tests join this job |

Feature 005 gaps that stay open and apply to every new language: G97 (catalog evidence is image-build
time evidence), G98 (runc strength; gVisor/Kata not shipped), G99 (one tenant per runner), G101
(no signing-key rotation), G104 (test runs never get secrets), G106 (node `language` informational),
G108 (artifacts in PostgreSQL, 2 MiB), G116 (tags move; digests required), G118 (a stolen runner
token can get code signed for its own tenant), G119 (wedged engine not exercised).

Update of 2026-11-24: Gap 97 of feature 005 is now answered in two parts. This feature answers it for
Rust and PHP with closed offline catalogs (Bash and PowerShell take no packages); design 046 answers it
for TypeScript and Python with resolved, policy-governed dependency layers (Architecture decision 0120,
which supersedes Decision 105 of feature 005 for those two languages only). The point-in-time nature of
catalog evidence stays open for the catalogs of this feature (Gap 1205).

### 1a. Currency check against `develop` (2026-11-24)

The drafts were written against `70f09435` (release 0.2.0). On 2026-11-24 they were read again against
`origin/develop` at `2c94064f`, by opening the files below; nothing was built or run. What changed, and
what this feature does about it:

| Area | Evidence on `develop` (2026-11-24) | Effect on this feature |
|---|---|---|
| Bundle versions | `crates/infra-schema/src/bundle.rs` still reads only `v` 1, 2 and 3 (`decoded_files`, `target`); no other bundle version exists in `crates/infra-schema`, `crates/infra-sandbox` or the lake modules | This feature takes version 4 and only version 4; design 046 takes version 5 (Decision 1236) |
| Runner modes | `apps/sandbox-runner/src/cli.rs` has two commands, `serve` (scripts) and `lake` (feature 035, Architecture decision 0079); `apps/sandbox-runner/src/lake/config.rs` refuses to start lake mode when any variable starting with `SAGAT_RUNNER_` is set (`SCRIPT_MODE_PREFIX`, `script_mode_variable`) | New languages, images and variables belong to `serve` mode only; because every new variable starts with `SAGAT_RUNNER_`, lake mode already refuses them; a test keeps it so (Decision 1238) |
| Runner configuration | `apps/sandbox-runner/src/config.rs` `RunnerConfig` has `images`, `go_target`, `go_required`, `max_concurrent`, `ceiling`, `trusted_keys`, no language allowlist; `Runner::start` (`apps/sandbox-runner/src/lib.rs`) probes every language it knows and keeps those whose probe passes | The allowlist, required set, Rust target and native build limit of Decision 1214 and Decision 1225 are still new; nothing to undo |
| Worker protocol | `crates/application/src/worker.rs` `PROTOCOL_MINOR = 6`; the sandbox runner registers `protocol_version: "1"` (`apps/sandbox-runner/src/lib.rs`), that is minor 0 | This feature adds no minor. The reserved host calls stay `NOT_GRANTED` until the runner negotiates the minors of features 019 and 026, which it offers only once it implements them (Decision 1238, Gap 1229) |
| Instance registry | `crates/application/src/instances.rs` `Capability` (closed list, kinds `CONTROL_PLANE` and `EXECUTION_ENGINE`); the CHECK in `20270113000000_instance_registry.up.sql`, widened by `20270118000000_run_input_validation.up.sql` (`NOT VALID`, then validated by `20270118000100`); Architecture decision 0082 accepted | The policy checks and the new language ids need every control-plane and engine process to honour them during a rolling upgrade: capability `script-languages-v1` (Decision 1234). Runners are not registry instances; what a runner serves stays advertised on its sessions and claims |
| Claim path | Design 038 (`docs/designs/038-claim-transition-efficiency.md`, Requirements 038-001, 038-002 and 038-005), its budget test `tests/integration/claim-transition-efficiency/work_budget.rs`; contention on `LWLock:MultiXactGen` measured there; `lease_one` in `crates/application/src/worker.rs` | The lease-time check becomes one plain read with no row lock, paid only by script activities, recorded in the budget (Decision 1235) |
| Flow control | Architecture decisions 0088 and 0089 (accepted): a claim never reads a rate window; a gated step waits outside `READY` and its permit is released by its terminal transition | A step refused at lease takes the ordinary non-retryable failure transition, so its grant and permit are released as for any failed step; the check reads no window (Decision 1235) |
| Step logs | Design 041, Architecture decisions 0084 and 0085 (accepted): a script step's log stays in `script_executions.log` (Requirement 041-016), the run's Logs tab shows a marker, and the `log` of a script execution with a run is read under the step-log rule, not `script:read` alone (Requirement 041-022) | New languages inherit both unchanged; the plan's authorization table and Requirement 023-022 say so (Decision 1238) |
| Run input validation | Design 043, Architecture decisions 0090 and 0092 (accepted): the run-input validator lives in `crates/infra-schema/src/run_input/` and checks a workflow run's input before the run is created. Script input and output schemas are still validated by the runner with `sagat_infra_schema::schema` over the `jsonschema` crate (`crates/infra-sandbox/src/docker.rs`, `SchemaViolation`) | Script schemas of new languages use the feature 005 validator, unchanged; a workflow's run input is checked by design 043 whatever languages its scripts use (Decision 1238) |
| Warm containers | Architecture decision 0052 (feature 019) is still Proposed | New languages are not eligible for warm containers until the warm residue suite covers them (Decision 1239) |
| SDKs | Six SDKs exist: `sdks/rust` joined on 2026-10-27; `sdks/rust/src/workflow/primitives.rs` takes the script language as `Option<String>`, like Go, C# and Java; TypeScript (`sdks/typescript/src/workflow.ts`) and Python (`sdks/python/sagat_octopus_orchestrator/workflow.py`) still list the five 005 languages | Requirement 023-024 and the conformance scenario cover six SDKs |
| Toolchain | `rust-toolchain.toml` pins `1.100.0` (the drafts said 1.98.0) | The Rust builder image starts from the workspace's pinned compiler and is then pinned on its own |
| Contracts | `specs/001-platform-foundation/contracts/openapi.yaml`: `CreateScriptRequest.language` now names all five built languages; `Script`, `ScriptSummary`, `ScriptVersion` and `ScriptVersionSummary` still carry inline five-value enums; Studio `languages.ts` still marks Go and Java `built: false` | Gap 1200 is half closed (Studio remains) |
| Migrations | Newest on `develop`: `20270119000000_batch_purge`; newest on any open branch: `20270209000100` | Proposed reservation `20270323000000` and `20270323000100` (Gap 1213) |
| Design 046 | `docs/adr/ADR-0120-…` and `docs/adr/ADR-0121-…` (being written beside this update) take bundle version 5, a policy that is "part of the script language policy of feature 023", and a block "materialized in the row the lease path already reads for the language check of ADR-0057" | The boundary is Decision 1237; the shared attestation code is Decision 1236 |

## 2. External semantics consulted

- Windmill Bash/PowerShell/Nu quickstart, https://www.windmill.dev/docs/getting_started/scripts_quickstart/bash :
  arguments come from positional parameters (`$1`, `$2`), the last line of stdout is the return value,
  and writing `./result.json` makes a JSON result. PowerShell declares input in a `param` block.
- Windmill Rust quickstart, https://www.windmill.dev/docs/getting_started/scripts_quickstart/rust :
  dependencies in a partial Cargo manifest inside a doc comment, a lockfile per script version, a
  `main` entrypoint whose typed arguments drive the generated form, release builds for deployed scripts.
- Windmill PHP quickstart, https://www.windmill.dev/docs/getting_started/scripts_quickstart/php :
  Composer `require` lines in a header comment, a generated lockfile, a `main` function with typed
  parameters, the return value is the result.
- Cargo, https://doc.rust-lang.org/cargo/commands/cargo-build.html : `--locked` fails when the lock
  would change, `--offline` forbids network, `--frozen` is both.
- rustc codegen options, https://doc.rust-lang.org/rustc/codegen-options/index.html :
  `relocation-model=static` is non-relocatable code; with `pic` the linker is told `-pie`; when a
  target cannot be both static and PIE, `+crt-static` wins and yields a static, non-PIE executable.
- Composer CLI, https://getcomposer.org/doc/03-cli.md : `--no-scripts` skips scripts, `--no-plugins`
  disables plugins, `dump-autoload --classmap-authoritative` loads classes from the classmap only.
  Composer scripts docs, https://getcomposer.org/doc/articles/scripts.md : install events run scripts.
- PHP core ini, https://www.php.net/manual/en/ini.core.php : `disable_functions` "can be circumvented
  and should not be considered a sufficient security measure"; `open_basedir` "is just an extra safety
  net … can therefore not be relied upon when security is needed" and does not restrict Unix sockets.
- PowerShell language modes, https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_language_modes :
  `ConstrainedLanguage` can be bypassed unless it runs "in System Lockdown mode with App Control for
  Business"; the detected policies (AppLocker, WDAC) are Windows mechanisms.
- PowerShell support lifecycle, https://learn.microsoft.com/en-us/powershell/scripting/install/powershell-support-lifecycle ,
  and https://devblogs.microsoft.com/powershell/announcing-powershell-7-6/ : 7.6 is the current LTS line,
  built on .NET 10, generally available since 2026-03-18; LTS updates are security and servicing fixes.
- PHP release archive, https://www.php.net/archive/2025 and https://www.php.net/archive/2026 : PHP 8.5.0
  was released on 2025-11-20 and receives security releases; 8.6 was in pre-release during 2026.
- PowerShell telemetry, https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_telemetry :
  `POWERSHELL_TELEMETRY_OPTOUT` set to `1` before the process starts disables startup telemetry.
- GNU Bash manual, Redirections, https://www.gnu.org/software/bash/manual/html_node/Redirections.html :
  for `/dev/tcp/host/port` and `/dev/udp/host/port` Bash "attempts to open the corresponding TCP [UDP]
  socket". The Restricted Shell, https://www.gnu.org/software/bash/manual/html_node/The-Restricted-Shell.html :
  restricted mode "is only one component of a useful restricted environment"; "jails, zones, or
  containers" are the more secure ways.
- Node.js TypeScript, https://nodejs.org/api/typescript.html : type stripping enabled by default since
  v23.6.0/v22.18.0, stable since v25.2.0/v24.12.0.
- Deno security, https://docs.deno.com/runtime/fundamentals/security/ : permissions are secure by
  default, but spawned subprocesses and FFI act outside them; untrusted code calls for OS-level
  sandboxing or a VM in addition.
- Bun lockfile, https://bun.sh/docs/install/lockfile : text `bun.lock` is the default since Bun 1.2;
  `bun install --frozen-lockfile` (and `bun ci`) refuses to change it.
- in-toto Statement v1, https://github.com/in-toto/attestation/blob/main/spec/v1/statement.md ;
  SLSA Provenance v1, https://slsa.dev/spec/v1.0/provenance ; DSSE, https://github.com/secure-systems-lab/dsse .
- CycloneDX 1.5 (schema already vendored in `tests/contract/inline-code-sandbox/`); package URLs
  `pkg:cargo/…`, `pkg:composer/…`, https://github.com/package-url/purl-spec .

None of these sources is evidence that the platform behaves as described; they bound the design.

## 3. Decisions

- **D1200 — Four language ids, one shell.** `bash`, `powershell`, `php`, `rust`. POSIX `sh` scripts run
  under the pinned GNU Bash; there is no separate `sh` language. One interpreter is pinned, probed and
  hardened instead of two, and Bash accepts POSIX syntax. Scripts that depend on dash- or
  BusyBox-specific behaviour are not supported, and the starter says so.
- **D1201 — Deno and Bun are evaluated, not adopted.** TypeScript scripts are framework-only (005 D105)
  and have no network, so the main benefits of either runtime (npm compatibility, built-in permissions,
  fetch) do not apply inside the sandbox; Deno's permissions are not the boundary (its own docs say
  subprocesses and FFI escape them). A third JavaScript engine would add an image to pin, probe and
  patch and could change script semantics by variant, which violates one-semantics. The feature runs a
  bounded spike outside product code (US07) that measures start time, memory and conformance under the
  unchanged profile. Adoption requires a new ADR and would bind the runtime into the toolchain identity
  so a published version can never change runtime silently.
- **D1202 — Each language uses an existing artifact family where one fits.** Bash and PowerShell are
  single-file interpreted scripts: bundle v1 text, entries `main.sh` and `main.ps1`. Rust is native:
  bundle v3 with `target` `linux-amd64|linux-arm64`, ABI `sagat-inline-v1`, entry `script.elf`,
  encoding `base64+gzip`, unchanged ELF rules. PHP always uses the new bundle v4 (D1203), with or
  without packages, so PHP has one loader path.
- **D1203 — Bundle v4 is a bounded text tree.** v1's flat 32-file namespace cannot hold a Composer
  vendor tree. v4 has exactly `v`, `language`, `policy`, `inputSchema`, `outputSchema`, `entry`,
  `lockDigest` and `files`; file keys are relative paths of 1–10 segments, each `[A-Za-z0-9._-]{1,64}`
  not starting with a dot, total path at most 255 bytes, no duplicates after normalization, at most
  1,024 files, UTF-8 text without NUL, decoded content at most 1,400 KiB, stored bundle at most 2 MiB.
  Only `php` and the reserved `python-data` (D1229) may use v4. For `php`, dependencies travel inside
  the signed bundle, as C# assemblies do (ADR-0021 decision 4), so what runs is exactly what was signed;
  `runtimeCatalog` is then `null`. For `python-data`, `runtimeCatalog` names the image catalog the
  artifact was built against (D1229).
- **D1204 — The Rust builder owns the project and calls `rustc`, never Cargo.** The author supplies one
  module `script.rs` with `pub fn run(input: serde_json::Value, octopus: &mut sagat_inline::Octopus) ->
  Result<serde_json::Value, sagat_inline::Error>`. The builder writes a fixed `main.rs` (`#![forbid(unsafe_code)]`,
  `mod script;`, a call into the trusted `sagat_inline` crate) and runs one `rustc` with fixed
  arguments: edition 2024, the runner's musl target, fixed opt-level and codegen units, no debug info,
  stripped symbols, `--remap-path-prefix`, `--extern` only for the trusted crate and the catalog crates
  named by the validated lock, `-L dependency=` the image's read-only prebuilt directory, and an empty
  environment except fixed variables. No `build.rs`, Cargo manifest, feature flag, `RUSTFLAGS`,
  profile or linker option of the author is read. Catalog proc-macro libraries are loaded by `rustc`
  from the read-only image (a `noexec` `/tmp` would refuse their mapping, so they cannot live there).
  `forbid(unsafe_code)` is defence in depth for authors, not a security boundary.
- **D1205 — The Rust executable is static and non-PIE.** The builder passes `-C relocation-model=static`
  and `-C target-feature=+crt-static` so the output is an `ET_EXEC` image with no interpreter that the
  existing `static_elf()` validator accepts unchanged. This gives up ASLR of the main image inside a
  container that already has no network or secrets; accepting static-PIE (`ET_DYN`) would require
  parsing the dynamic section and is left for a later additive validator change. The startup probe, not
  a reading of toolchain defaults, proves the produced image passes the validator.
- **D1206 — Rust runs through the Go sealed-memory loader.** The trusted loader moves from
  `runners/go/runtime.py` to `runners/native/runtime.py`, is parameterized only by the bundle language,
  and both runtime images copy the same file. The Go native CI proofs (ADR-0024, T005-043) must pass
  again before the move merges (G1218).
- **D1207 — A Rust lock is a Cargo.lock that names only catalog crates.** Accepted lockfiles are Cargo.lock
  format version 4 with exactly one root package `sagat-script 0.0.0` (no source) and packages whose
  `name`, `version`, `source` (`registry+https://github.com/rust-lang/crates.io-index`) and `checksum`
  equal a catalog entry; the dependency closure must be complete and contain nothing else. The catalog
  listing API returns the exact lock text for a selection, so authors do not hand-write it. The
  catalog records, per crate, the crates.io checksum, an independently verified archive SHA-256,
  SPDX licence, whether it has a build script or is a proc-macro, and the advisory audit evidence of the
  image build. Build scripts run only when the image is built, by the platform, never per customer build.
- **D1208 — PHP packages come from a catalog and Composer never installs.** The PHP builder image holds
  the catalog's extracted package trees. A build copies the trees of the locked packages and runs the
  pinned Composer only as `composer dump-autoload --classmap-authoritative --no-dev` with
  `--no-scripts --no-plugins`, over a generated manifest, with a fixed `autoloader-suffix`. It never runs
  `install`, `update` or `require`. A lock entry matches the catalog on `name`, `version`, `dist.type` and
  `dist.reference`; `packages-dev` must be empty; packages of type `composer-plugin` and packages that
  need a native extension outside the runtime's closed set are not admitted to the catalog. The
  runtime image carries no catalog, so a script cannot `require` an undeclared package from the image.
- **D1209 — PHP runs with a platform ini only.** The pinned current stable minor (8.5 at the time of
  writing) runs as `php -n -c /opt/sagat/php/php.ini` with
  `allow_url_fopen=0`, `allow_url_include=0`, `phar.readonly=1`, `display_errors=stderr`,
  `log_errors=0`, `max_execution_time=0` (the supervisor owns wall time), `memory_limit=-1` (the cgroup
  owns memory, so exhaustion is reported as `SCRIPT_OUT_OF_MEMORY` consistently), and a closed extension
  set named in the toolchain identity. `disable_functions` and `open_basedir` are not used as
  boundaries (PHP's own warnings). The author defines `function run(array $input, Octopus $octopus): mixed`;
  `echo` and `print` go to the log through an output buffer.
- **D1210 — A shell script talks to its supervisor through a helper.** The trusted in-container host
  `sagat-shell-host` (Rust, `runners/shell/host/`, sharing the protocol crate of D1204) reads the first
  message, writes `main.sh` and the input file into `/tmp`, starts `bash --noprofile --norc -o errexit
  -o nounset -o pipefail main.sh` with a cleared environment (`PATH`, `HOME=/tmp`, `LANG=C.UTF-8`,
  `TZ=UTC`, `OCTOPUS_INPUT`, `OCTOPUS_SOCKET`, `OCTOPUS_IDEMPOTENCY_KEY` only; no `BASH_ENV`, `ENV`,
  `SHELLOPTS` or exported functions), and serves the `octopus` helper over a Unix socket in `/tmp`.
  `octopus input`, `octopus secret NAME`, `octopus fetch …`, `octopus idempotency-key` and
  `octopus result [JSON|-]` are the commands; stdout and stderr are both the log; the result is only
  what `octopus result` recorded (first call wins, later calls fail). Windmill's "last stdout line is the
  result" is rejected: stdout is the log and may hold what a script printed by mistake. The host and
  helper share the container with untrusted code and are not a security boundary; every bound is the
  runner-side supervisor's (005 D96).
- **D1211 — The shell toolset is curated and listed.** The Bash image contains Bash, coreutils, `sed`,
  `awk`, `grep`, `jq`, `base64`, `sha256sum`, `date`, `tar` and `gzip`, and nothing that opens a network
  connection or installs packages (no `curl`, `wget`, `nc`, `ssh`, package manager). Setuid and setgid
  bits are removed at image build. The image's package inventory is a file in the image, copied into the
  SBOM, and part of the toolchain identity through the image digest.
- **D1212 — PowerShell is the framework only.** The pinned PowerShell LTS line on Linux (7.6, on .NET 10,
  GA 2026-03-18; exact patch and digest recorded at implementation; LTS only, whose updates are
  security and servicing fixes; each update is a new image digest and so a rebuild), started with
  `-NoProfile -NonInteractive -NoLogo`, `POWERSHELL_TELEMETRY_OPTOUT=1`, `POWERSHELL_UPDATECHECK=Off`,
  `DOTNET_CLI_TELEMETRY_OPTOUT=1` and `PSModulePath` set to the image's read-only module directories.
  The author defines `function Run { param($InputObject, $Octopus) … }`; the trusted `Sagat.Octopus`
  module provides `Invoke-OctopusFetch`, `Get-OctopusSecret`, `Write-OctopusLog` and
  `Get-OctopusIdempotencyKey`. The result is `Run`'s output: none is `null`, one object is that object,
  several are an array; serialization is bounded to depth 64 and fails rather than truncating.
  Gallery modules are refused (a non-empty lockfile is refused); ConstrainedLanguage mode is not used.
- **D1213 — Builds and runs have per-language profiles inside the existing ceilings.** Interpreted
  builds keep the existing "at most the default limits". The Rust build profile defaults to 1 GiB,
  2,000 millicores, 120 s, 128 processes, 512 MiB scratch, with a deployment ceiling no higher than the
  domain maxima (2 GiB, 4,000 millicores, 600 s here, 256 processes, 512 MiB). The PowerShell runtime
  default memory is 512 MiB, pending the benchmark (G1207); every other runtime default is the 005
  default. These are limits, not performance claims.
- **D1214 — A new language is advertised only after its production-path probe, and only if the
  deployment asks for it.** `SAGAT_RUNNER_LANGUAGES` lists the languages a runner may serve (default:
  the five of 005, so upgrading a runner never starts serving a new language by itself);
  `SAGAT_RUNNER_REQUIRED_LANGUAGES` turns a failed probe into a startup failure. Each new language has a
  bundled probe that builds and runs through the configured images, profile and protocol, exercises the
  idempotency-key host call and checks the result. Outside `SAGAT_ENV=dev`, a new language whose image
  reference is not pinned by digest fails its probe.
- **D1215 — One native target per runner.** `SAGAT_RUNNER_RUST_TARGET` takes exactly `linux-amd64` or
  `linux-arm64`; if Go is also configured the two targets must be equal or startup fails. Rust pools are
  homogeneous until claims become target-aware (G1201).
- **D1216 — Catalog logic lives in `crates/infra-schema`.** Catalog files `runners/rust/catalog/catalog.json`,
  `runners/php/catalog/catalog.json` and, when feature 024 supplies it, the `python-data` catalog are embedded in a new `crates/infra-schema/src/catalog.rs`,
  used by the control plane for early lock validation and catalog listing, and by the runner (through
  `crates/infra-sandbox/src/lockfile.rs`) as the authoritative check; the builder images carry the same
  file, and its digest is in the toolchain identity. The control plane signs a build only if the SBOM's
  package set equals the version's lockfile package set (`BUNDLE_MISMATCH` otherwise).
- **D1217 — Every new build gets a signed provenance statement.** An in-toto Statement v1 whose subject
  is the artifact digest and whose predicate is SLSA Provenance v1 (`buildType`
  `https://sagat-octopus-orchestrator.io/script-build/v1`), wrapped in DSSE (payload type
  `application/vnd.in-toto+json`) and signed with the existing control-plane Ed25519 key. The predicate
  separates what the control plane verified (version, content digest, lock digest, policy, schemas,
  build key, toolchain handed out at the claim) from what the runner reported (runner identity, image
  ids, timings). It is stored **per build**, keyed by tenant and build key, not on the content-addressed
  artifact row: an artifact digest produced again by a build with another toolchain gets its own
  statement, and a version finished from an identical build key shares that build's statement (D1227).
  It is produced for builds of every language after this feature; older builds have none (G1217). No SLSA build level is claimed: G118 means a runner token holder can still get code
  signed for its own tenant.
- **D1218 — A namespace may narrow languages and packages, never widen them.** A script runtime policy
  per namespace enables or disables each built language and may restrict a language to a subset of the
  deployment catalog. It is checked at five points: draft creation (a draft's language is fixed),
  publication, test-run admission, workflow publication, and the lease of every script step attempt
  (D1226). New permission `script:policy:manage`, granted to `admin` only. Package narrowing is checked
  at the first four points only: a published artifact already carries its dependencies, so the lease
  check is about the language.
- **D1219 — Availability is advisory evidence.** The control plane upserts at most one presence row per
  tenant and runner subject per 60 s when a runner claims a build or a test run, holding its languages,
  native target and toolchain identities. The availability endpoint reports a language as served when a
  presence row is younger than 120 s. Admission never depends on it.
- **D1220 — Schemas and forms are language-neutral.** FR-005-006 applies unchanged: declared input and
  output schemas are validated by the runner-side supervisor before and after execution, schemas are
  inferred from test runs, and Studio's test form is generated from the input schema. Signature-derived
  schemas (Windmill-style) are not built (G1209). Fields classified `secret` (feature 016) are never
  prefilled or kept in browser storage; a field declared encrypted by feature 031 is entered and passed
  as an opaque value and the platform never decrypts it for a script.
- **D1221 — No new runtime failure code.** New languages map onto the existing `SCRIPT_*` codes; a Rust
  target mismatch is `SCRIPT_SANDBOX_UNAVAILABLE` like Go; a step refused at lease by the language
  policy is `SCRIPT_DENIED` (non-retryable) with a fixed safe message. Admission refusals use `VALIDATION_FAILED`
  with new `details.reason` values `LANGUAGE_DISABLED`, `PACKAGE_NOT_ALLOWED` and `LOCKFILE_INVALID`.
  Build failure kinds stay `rejected, lockfile, limit, unsupported, unavailable`.
- **D1222 — Studio highlights, the platform checks.** Monaco 0.57.0's `shell`, `powershell`, `php` and
  `rust` definitions give syntax highlighting only; no browser language service is added. The language
  list is driven by the availability endpoint, so a language is enabled when the platform builds it and
  a runner serves it, and disabled with the reason otherwise.
- **D1223 — Tests live under `tests/*/script-languages/`** with their own targets in `tests/Cargo.toml`;
  real-container suites ask their own sandbox for leftovers (005 D117).
- **D1224 — Shell-specific risk is accepted as stated, not hidden.** See section 5.
- **D1225 — Native builds are bounded per runner.** `SAGAT_RUNNER_MAX_NATIVE_BUILDS` (default 1,
  ceiling 4) bounds concurrent Rust builds on one runner in addition to `SAGAT_RUNNER_MAX_CONCURRENT`.
- **D1226 — The language policy is enforced when a script step is leased (architecture review S3,
  coordinator decision).** In the engine's lease transaction (`crates/application/src/worker.rs`
  `lease_one`), a step whose activity is `script.sha256-<hex>` is looked up by tenant and digest for its
  artifact language and checked against the namespace policy row read `FOR SHARE`. A disabled language
  is not leased: the step is failed in that same transaction as non-retryable `SCRIPT_DENIED`, with its
  journal event, the next scheduled work (`onError`, saga drain) and outbox records, and a
  `script.policy_denied` audit row (`point: lease`, once per actor, resource and reason; the actor is the
  run's initiator or publisher). An attempt already leased finishes. The lock order is the existing run
  and step locks, then the policy row; a policy update takes only the policy row, so the two never wait
  in a cycle. This changes D1218's earlier "admitted runs keep running": disabling a language now stops
  existing runs at their next script step attempt, which is the operator's intent; re-enabling lets new
  attempts lease. Cost: one indexed lookup per script-activity claim, measured in T023-056 (G1221).
- **D1227 — Provenance is a record of a build, not of an artifact (review S2).** Table
  `script_build_provenance` keyed `(tenant_id, build_key)`, subject the artifact digest, written in the
  build-completion transaction. `getScriptVersion` returns the statement of the version's build key.
- **D1228 — Proc-macros run in the builder on every build that uses them (review S1).** The earlier
  wording "run only at image build" was wrong for proc-macros: `rustc` loads the reviewed proc-macro
  library from the read-only image and runs it inside the sandboxed builder container during each
  customer build that uses it. Only Cargo build scripts run solely at image build. The catalog flag
  `procMacro` is review evidence, and the builder container's profile is what contains that code.
- **D1229 — `python-data` is a reserved image-catalog runtime whose packages feature 024 owns (024
  review S3, coordinator decision).** Feature 024 needs Polars and PyArrow, which are native wheels far
  above the 2 MiB artifact ceiling, so they cannot travel in the bundle. Under ADR-0057 this feature
  defines the mechanism; 024 defines the image, the packages, the execution environment and their tests:
  - Language id `python-data`, distinct from `python` so 005's framework-only Python (D105) is unchanged;
    `is_built` is false and requests are refused `LANGUAGE_NOT_BUILT` until 024 ships it.
  - Artifact: bundle v4 with `entry` `main.py`, the author's files, `lockDigest` of the lock, and
    `runtimeCatalog` set to the digest of the runtime image's catalog. The runner MUST refuse to run it
    (`SCRIPT_SANDBOX_UNAVAILABLE`) unless its configured `python-data` runtime has exactly that catalog
    digest, so a version never runs against other packages; only the locked packages are put on the
    import path (an SBOM-accuracy measure, not a boundary).
  - Lockfile: a hash-pinned requirements file, one `name==version --hash=sha256:<wheel hash>` line per
    package and target, matching catalog entries exactly (complete closure, nothing else).
  - Catalog: a file conforming to `contracts/catalog.schema.json` with `language: python-data` and
    upstream kind `pypi`, embedded by `crates/infra-schema/src/catalog.rs`; location and content are 024's.
  - Toolchain identity `build=…;runtime=…;protocol=artifact-v4;lang=python-data;version=python-3.x;
    shim=…;catalog=sha256:…`, hence in the build key; probe, allowlist, digest pinning, SBOM, provenance
    and namespace policy apply exactly as to the other new languages.
  This is the one exception to "dependencies travel inside the artifact", recorded in ADR-0057 decision 4.
- **D1230 — Rollback is binary-only and forward-only for data (review minor).** Every build after this
  feature writes provenance and every new-language script writes a row a 0.2.0 binary cannot decode
  (G1220). The down migration therefore refuses once any such row exists; the supported rollback is to
  keep the migrated schema and either roll forward, or roll the binaries back only while no
  new-language row exists. T023-063 tests a 0.2.0 control plane against the migrated schema with and
  without new-language rows and records the observed behaviour.
- **D1231 — The shim protocol reserves `checkpoint` and `stream.append` for every script language
  (coordinator decision C17).** This feature owns the shim protocol of all ten script runtimes (the five
  of 005 through their existing shims, the four new ones and `python-data`), so it defines the two calls'
  message shapes, gate, bounds and redaction net in `contracts/shim-protocol.md`, and nothing of their
  meaning: checkpoint ordering, coalescing, storage, delivery and classification are feature 019's;
  channels, schemas, budgets, offsets and the feed are feature 026's (ADR-0068). The gate is the step's
  current lease: a call is refused `NOT_GRANTED` unless the runner's session negotiated the owning
  feature's protocol minor and (for streams) the lease carries an `emits` channel; test runs have no
  workflow lease. The name `stream.append` follows C17 (026's draft contract says `stream.publish`, G1224).
- **D1232 — The supervisor owns sequencing and pacing.** It assigns 019's `details_sequence` and 026's
  `publisher_seq` itself, so a script cannot reuse or forge one; it keeps only the latest valid pending
  checkpoint and sends it with the next heartbeat or the final failure report (checkpoints ride on
  heartbeats in 019's plan), answering the script `QUEUED`; it forwards each `stream.append` request as
  one `AppendStreamEvents` call and returns the offsets or the refusal, retrying `UNAVAILABLE` with the
  same sequences until the lease deadline. Per run, 600 `checkpoint` and 1,000 `stream.append` calls are
  allowed, separately from the 100 fetch/secret calls; the channel line ceiling becomes 2,000. A request
  stays under the 240 KiB channel line, and each payload under the owning feature's size ceiling.
- **D1233 — The same redaction net, and nothing kept.** Each payload is searched for every secret value
  handed to the run before it leaves the runner (005 D111 net; the server scans again) and refused on a
  match. Neither payload is ever written to the script log, the execution record, a span, a metric label
  or audit; refusals are recorded as `checkpoint` or `stream.append <channel>` like other refusals. The
  feature stores neither: 019 and 026 own storage, retention and erasure (C8).

The decisions below were taken on 2026-11-24 by the agent preparing the feature for build, under the
owner's delegation for release 0.3.0: delegate (OD-2026-11-24-4), which widens OD-2026-11-20-6. Each
weighs the options and chooses the most architectural and business advantage, never simplicity alone.

- **D1234 — New language ids and the namespace policy are activated through the instance registry.**
  Release 0.3.0 is upgraded by rolling N and N-1 (OD-2026-11-24-1), so for a while control planes and
  engines of two releases serve together. A control plane of the previous release cannot decode a
  `bash` row (`language_of` in `crates/infra-postgres/src/scripts.rs`, Gap 1220) and an engine of the
  previous release never checks the language policy at lease. Options weighed: (a) document the upgrade
  order and hope; (b) a feature flag per process (Architecture decision 0081, which the registry
  replaces for fleet-wide behaviour); (c) a capability of the instance registry (Architecture decision
  0082), as features 029, 031 and design 043 do. Chosen (c): capability `script-languages-v1`, required
  of kinds `CONTROL_PLANE` and `EXECUTION_ENGINE`, recorded by an operator through the registry's
  audited activation, which is refused while a live process lacks it. Until it is recorded, a request
  that would create the first row of a new language (`createScript` with `bash`, `powershell`, `php` or
  `rust`) and a `putScriptRuntimePolicy` whose document disables a language or narrows packages are
  refused `422 VALIDATION_FAILED` with `details.reason` `SCRIPT_LANGUAGES_CAPABILITY_DISABLED`; the five
  languages of feature 005 and the default policy keep working. Removing the requirement is refused while
  a script of a new language or a stored policy row exists. After it is recorded, a process of a release
  that has the registry but not the capability is held by the registry gate (`503
  BINARY_CAPABILITY_MISSING`) instead of failing listings with an internal error, which turns Gap 1220
  from a fault into a stated refusal. Processes of release 0.2.x predate the registry and are invisible to
  it, so for the upgrade from 0.2.x the requirement is recorded only once no 0.2.x control plane or engine
  is left, which the runbook states and the activation cannot check (Gap 1230). Runners are
  not registry instances: whether a runner serves a language stays what it advertises on its worker
  sessions, build claims and presence row (Decision 1214, Decision 1219), because a language capability
  of one runner is a fact about one machine, not a rule every process must honour. The capability value
  joins the closed list in `crates/application/src/instances.rs` and in the registry CHECK, widened
  `NOT VALID` and validated in a second migration as `20270118000000` and `20270118000100` did.
- **D1235 — The lease-time check is one plain read, paid only by script activities (refines Decision
  1226).** Decision 1226 read the artifact row and then the policy row `FOR SHARE`. Design 038 measured
  `LWLock:MultiXactGen` among the waits of the claim path: many lessees taking a shared row lock on the
  same policy row of a busy namespace create exactly those multixacts. Options weighed: (a) keep `FOR
  SHARE`; (b) cache the policy in the engine (stale for an unbounded time, and a second source of
  truth); (c) one plain read with no row lock; (d) denormalize the language into
  `workflow_version_scripts` at publication (one more write path, and still one read at lease). Chosen
  (c): inside the lease transaction, after the run lock and only once the step to lease is chosen and its
  activity starts with `script.sha256-`, one statement reads the artifact's language by `(tenant_id,
  digest)` (primary key of `script_artifacts`) joined to the namespace's policy row, if any, by its
  primary key, with no row lock. A disabled language takes the non-retryable failure transition of
  Decision 1226 instead of the lease. What is given up: a lease whose transaction read the policy just
  before a disabling update committed may still lease once; that attempt finishes like an attempt
  already leased (Gap 1226), which is the documented behaviour of Requirement 023-017. What is gained: no
  row lock on the claim path, no wait between a policy update and the leases, and an exact cost for
  design 038: a non-script lease costs no new statement, a probed candidate with nothing to lease costs
  nothing new (Requirement 038-002), and a script-step lease costs exactly one more statement, recorded in
  `tests/integration/claim-transition-efficiency/work_budget.rs` with a script-step shape in the change
  that adds it (Requirement 038-001). It skips no lock, authority, quota or capacity read (Requirement
  038-005). The engine always runs the statement for a script activity, whether or not `script-languages-v1`
  is recorded, so no window exists in which a recorded requirement and a stale view in one process let a
  disabled language lease. The refusal is the same transition a worker's non-retryable report causes, so
  the step's flow-control grant and permit are released as for any failed step (Architecture decisions
  0088 and 0089) and no rate window is read. Design 046's lease-time refusal of a package marked
  malicious after publication (Architecture decision 0121) MUST extend this same statement, not add a
  second one; if it cannot, it moves the budget with its reason. Revised the same day by Decision 1627:
  design 046 builds the statement first (Task T046-17: a `runnable` and `blocked_reason` pair on the
  artifact row) and this feature extends it with the language reason; the rest of this decision (one
  statement, no row lock, script activities only, the budget) stands. Admission points (draft creation,
  publication, test run, workflow publication) keep the policy row `FOR SHARE`: they are rare, they write,
  and there an edit and a publication must see one serial order.
- **D1236 — Provenance and SBOM code is one shared module that design 046 also uses; this feature uses
  bundle versions 1, 3 and 4 only.** Design 046 attests every dependency-layer build with the same
  statement (DSSE envelope, in-toto Statement v1, SLSA Provenance v1, Architecture decision 0057 decision
  6, Task 023-047) and applies the same SBOM rule. Options weighed for where the code lives: (a) inside
  `crates/infra-schema` (the draft's `provenance.rs`), which already holds bundles, classification and the
  run-input validator and links `jsonschema`, `regex-automata` and Tokio; (b) inside the control plane
  only, which the runner also needs for the SBOM it writes; (c) an adapter crate outside
  `crates/infra-schema`. Chosen (c), for cohesion and so that design 046's code and the runner do not link
  a schema engine they do not use. As first written this was a new crate `crates/infra-attestation`;
  revised the same day by Decision 1627, the crate is design 046's new adapter `crates/infra-packages`
  (package `sagat-infra-packages`, Task T046-15), whose provenance and SBOM builders hold: the in-toto
  statement and SLSA predicate with a closed build type (this feature adds `ScriptBuild` with `buildType`
  `https://sagat-octopus-orchestrator.io/script-build/v1`, design 046 has its own), the DSSE envelope with
  its bounds and shape check, and the CycloneDX 1.5 document a runner writes with the control plane's
  comparison of its package set with a locked package set; their file layout is design 046's. The port is `crates/application/src/attestation.rs` (an `Attestations` trait, as `BundleChecker`
  is for bundles); signing stays with the key in `crates/application/src/script_signing.rs`, which gains
  the DSSE pre-authentication encoding and `Signer::sign_dsse` / `Verifier::verify_dsse`, so no adapter
  ever holds the key. `crates/infra-sandbox` moves its SBOM writing (`docker.rs`, build completion) onto
  `sbom.rs`. Storage is each feature's own: this feature's statements are rows of
  `script_build_provenance` with a foreign key to `script_artifacts`; design 046 keeps its layer
  attestations where its layers are recorded. Design 046 builds the crate first (Decision 1627); this
  feature depends on it and adds its build type. A check of `scripts/check-architecture.py` keeps the
  in-toto statement type and the DSSE payload type out of every crate but `sagat-infra-packages`
  (AGENTS.md rule 21). Bundle versions: Bash and PowerShell use version 1 and Rust version 3, both
  unchanged formats; PHP, and the reserved `python-data`, use version 4, the only number this feature
  adds. Design 046 takes version 5 for its additive bundle; neither feature takes another number.
- **D1237 — The boundary with design 046: closed catalogs here, open dependencies there, one policy
  surface.** Bash and PowerShell take no packages; Rust and PHP keep closed offline catalogs baked into
  builder images (Decision 1207, Decision 1208, Architecture decision 0057 decision 4); C# keeps its
  catalog (Architecture decision 0021). TypeScript and Python open dependencies (npm `package-lock.json`
  version 3; wheels only under a PEP 751 `pylock.toml`) are design 046's, resolved at publication
  through its package gateway (Architecture decision 0120). Options weighed for the policy: (a) two
  policy documents, one per feature; (b) one document per namespace that design 046 extends. Chosen (b),
  because an administrator governs "what code may run in this namespace" in one place, with one revision,
  one audit action and one lease-time read. Revised the same day by Decision 1627: the one document stays,
  but design 046 owns and builds it and its endpoint (Task T046-2), and this feature's language policy
  (User story 023-06) is a section of it; the rules below are what that section must hold. The per-language object of the policy document stays closed
  for each release (an unknown property is refused) and grows only additively: this feature defines
  `enabled` for every language and `packages` (`catalog`, `none`, `{"allow": [...]}`) for languages with a
  closed catalog (`csharp`, `rust`, `php`, `python-data`); for `typescript` and `python` the `packages`
  property and any rule property are reserved for design 046 and refused `PACKAGE_NOT_ALLOWED` until it
  ships. The draft's bounds were inconsistent (256 allowed names of up to 214 bytes cannot fit a 16 KiB
  document): the document bound becomes 64 KiB, the 256-name bound per language stays, and a request over
  either is refused `ABOVE_CEILING`. Design 046's rules that need more room (large deny lists, waivers)
  live in rows of its own that the document references, so the document and the lease read stay small.
- **D1238 — What changed beside the feature, and how it applies (currency, 2026-11-24).** (1) New
  languages, images and runner variables exist only in the runner's `serve` mode; lake mode (Architecture
  decision 0079) already refuses any `SAGAT_RUNNER_` variable, and a unit test in
  `apps/sandbox-runner/src/lake/config.rs` names each new variable so a renaming cannot slip past. The
  `python-data` runtime is not the lake image (`runners/lake`) and never runs under the lake profile. (2)
  A script step's log of any language stays in `script_executions.log`, with the marker row and read rule
  of design 041 (Requirements 041-016 and 041-022); a test run's log is unchanged. (3) Script input and
  output schemas of new languages are validated by the runner with the feature 005 validator
  (`sagat_infra_schema::schema`); the run-input validator of design 043 is not used for scripts and is not
  changed. (4) This feature adds no worker-protocol minor (develop speaks 1.6; the sandbox runner offers
  minor 0); the reserved host calls are gated on the minors that features 019 and 026 allocate in merge
  order, which the runner offers only when it implements them. (5) Six SDKs: the Rust SDK accepts any
  language string, like Go, C# and Java; the conformance scenario runs in all six.
- **D1239 — New languages are not eligible for warm containers until the warm residue suite covers
  them.** Architecture decision 0052 (feature 019, still Proposed) lets an opted-in artifact reuse a
  container. Bash background processes, PHP static and output-buffer state, a PowerShell runspace and
  native Rust process state are residue the suite of 0052 does not test yet. Options weighed: (a) let 019's
  policy apply to every language at once; (b) forbid warm containers for new languages for good; (c) make
  each language eligible only once that suite has a residue fixture for it and passes on each target.
  Chosen (c): it keeps the business value of warm starts for the languages that start slowest (PowerShell
  on .NET) while holding Constitution principle V. This feature supplies the per-language residue
  fixtures (Task 023-070); feature 019 owns the eligibility check and refuses a warm policy on an artifact
  whose language has no passing fixture.
- **D1625 — Build the `python-data` reservation now, its image-catalog mechanism only when feature 024
  confirms it.** Design 046 gives `python` a policy-governed dependency layer kept in the blob store and
  mounted read-only, which is not bounded by the 2 MiB artifact and is the very constraint that made
  Decision 1229 an exception. Building the reserved mechanism now would leave two Python packaging paths
  (an image catalog with a hash-pinned requirements lock, and design 046's layer with a PEP 751 lock).
  Options weighed: (a) build all of Task 023-062 now; (b) drop `python-data`; (c) reserve the id now
  (enum, database constraint, `LANGUAGE_NOT_BUILT` at every admission point, the bundle version 4 schema
  value) and build the image-catalog parts (requirements lock parser, `pypi` catalog kind,
  `runtimeCatalog` check at load) only when feature 024 confirms that it needs an image catalog rather than
  design 046's layer. Chosen (c): the reservation is cheap and additive and keeps feature 024 unblocked;
  the mechanism is built once, for a confirmed need, and the platform keeps one Python dependency path if
  024 can use 046's. Requirement 023-027 is amended accordingly; the choice between the two paths is
  feature 024's and design 046's, put to the coordinator (Gap 1227).
- **D1626 — Two builders, two lanes, joined per language.** The feature is cut into slices so that two
  builders never edit the same crate at the same time: after one foundation slice, a control-plane lane
  owns `sagat-domain`, `sagat-application`, `sagat-infra-schema`, its additions to design 046's
  `sagat-infra-packages` once that crate has merged, `sagat-infra-postgres`, `sagat-api-http`, the contracts,
  Studio and the SDKs; a runner lane owns
  `sagat-infra-sandbox`, `sagat-sandbox-runner`, `runners/` and the native CI job. Each language is closed by
  a join slice that flips `is_built` once both lanes have delivered its parts. The slices are in
  `plan.md`, "Build slices (2026-11-24)".
- **D1627 — Design 046 owns and builds first the three pieces both features share; this feature extends
  them (the coordinator's boundary decision of 2026-11-24, delegate (OD-2026-11-24-4)).** Decisions 1235,
  1236 and 1237 were first written with this feature owning the namespace policy document, owning the
  lease-time statement and building a crate of its own for attestation, with "whichever is built first"
  for the crate. The coordinating session decided the boundary the other way, and this feature applies
  it: (1) the namespace script policy document and its endpoint are design 046's (Task T046-2 for the
  document and its migration, its REST task for the endpoint); this feature's language policy is a section
  of that document, keeps the rules of Decision 1237 (closed per release, additive growth, `enabled`,
  `packages` for closed catalogs, 64 KiB and 256 names) and the operations `getScriptRuntimePolicy` and
  `putScriptRuntimePolicy` of `contracts/openapi-script-languages.yaml` become the shape of the language
  section in design 046's endpoint, named as design 046 names them; (2) the lease-time check is design
  046's statement over a `runnable` and `blocked_reason` pair on the per-tenant artifact row (Task
  T046-17), and Task T023-061 adds the language reason to the same statement, with no second read; (3) the
  provenance and SBOM builders live in design 046's new adapter `crates/infra-packages` (Task T046-15),
  and Task T023-047 and Task T023-068 reuse them. Why this is the better boundary, weighed against keeping
  the first text: design 046 is built in slice 1 of its own plan and needs all three pieces for its first
  user-visible result (a policy-checked, attested, lease-blockable dependency layer), while this feature
  needs them only from its slices A1, A2 and A4; one owner per shared piece removes the "whichever first"
  coordination, which would have left two builders free to start the same crate; and the crate that
  already parses locks and builds SBOMs for two ecosystems is the natural home of the SBOM rule. One
  variance is recorded: a namespace's language policy is per namespace while the artifact row is per
  tenant (a digest may serve scripts of several namespaces), so the language reason cannot be materialized
  in `runnable`; this feature's extension joins the namespace's language section to the same statement,
  still one statement, with no row lock (Decision 1235). Bundle numbers are unchanged: this feature keeps
  version 4, design 046 uses version 5. This feature's own pieces stay its own: `script_build_provenance`,
  the presence table, the language constraint, the registry capability `script-languages-v1` (Decision
  1234) and every refusal of the language section.

- **D1628 — Corrections after design 046's second review round (2026-11-24, delegate (OD-2026-11-24-4)).** Three
  statements above are corrected, without editing them: (a) the lease-time statement reads design 046's
  `script_artifact_blocks` rows (tenant, artifact digest, namespace or all), not a `runnable` pair on the artifact row,
  because `script_artifacts_immutable` refuses every update of that row (Decisions 1627 and 1235, plan 8a); (b) design
  046's policy section is the top-level `packages` key of the document beside `languages`, so the reservation of
  `languages.{typescript,python}.packages` in Decision 1237 is withdrawn (those keys stay refused); (c) design 046 keeps
  no layer in the blob store: each runner builds a layer from the package gateway's store and checks a signed digest
  (design 046, Decision 046-022), which Decision 1625 should read in place of "kept in the blob store". The table
  `script_build_provenance` is created by design 046's migration with the columns of this feature's data model, and
  its envelope names the lock and SBOM digests instead of listing every dependency, so the 65,536-byte bound holds.
  (d) The lease-time check is design 046's typed admission on both lease paths (`lease_one` and the scoped claims),
  not a statement in `crates/infra-postgres/src/workers.rs` alone; plan 8a and the code table say so (design 046's third
  review round).
## 4. Gaps

| # | Gap | State |
|---|---|---|
| G1200 | Baseline drift: OpenAPI `CreateScriptRequest.language` and the `createScript` description say C#/Go/Java are refused; Studio `languages.ts` marks Go and Java `built: false` | The contract merge (T023-051) and availability-driven list (T023-019) correct the shared text; noted for 005/007 owners |
| G1201 | Worker claims carry a language, not a native target; Rust inherits Go's homogeneous-pool rule | Open; target-aware capability routing depends on feature 019 |
| G1202 | G118 applies: the control plane cannot compare a built binary with its source | Open; provenance records reported fields as reported |
| G1203 | G98 applies; the Bash and PowerShell images have a larger userland than the 005 images | Open; mitigated by toolset curation and setuid removal |
| G1204 | Redaction is a net: `set -x`, transcripts, encodings and PHP error text can carry a secret in a transformed form | Accepted; documented in the runbook |
| G1205 | Catalog advisory audits are point-in-time; no re-audit or rebuild trigger for stored artifacts | Open |
| G1206 | Rust build resource needs are unmeasured; the D1213 profile is a proposal. Scratch on a tmpfs is charged to the container's memory cgroup, so compiler memory and written objects share the build memory limit | Pending benchmark T023-056 |
| G1207 | PowerShell start time and memory are unmeasured | Pending benchmark T023-056 |
| G1208 | Image size and runner disk use grow (pwsh, rustc with prebuilt crates); no image garbage-collection policy | Open (runbook guidance only) |
| G1209 | No signature-derived input schema inference | Out of scope |
| G1210 | G104 applies: no test secrets for any language | Open |
| G1211 | A script can listen on loopback inside its own network namespace and connect to itself with `/dev/tcp` | Accepted: nothing else is reachable |
| G1212 | Presence is up to 120 s stale; a version of a language no runner serves stays `BUILDING` (existing behaviour, no build deadline) | Accepted, shown in Studio |
| G1213 | Migration version numbers are shared by parallel features | Allocated at implementation |
| G1214 | PHP packages with `autoload.files` run code when the autoloader loads | Catalog review judgement |
| G1215 | PowerShell `Add-Type` compiles and loads C# in the runtime container | Accepted: the container is the boundary |
| G1216 | Deno/Bun results unknown until the spike runs | Pending T023-049 |
| G1217 | Artifacts built before this feature have no provenance statement | Accepted |
| G1218 | Moving the native loader requires rerunning 005's native Go proofs on both architectures | Gate of T023-014 |
| G1219 | A `python-data` artifact runs only on a runtime image with its exact catalog digest: re-pinning the 024 catalog makes older versions unrunnable until they are republished or a runner keeps the older image | Accepted cost of D1229; 024 states its re-pin procedure |
| G1220 | A 0.2.0 control plane fails any script listing that contains a new-language row (`corrupt("script language")`) | Rollback forward-only (D1230); behaviour measured by T023-063 |
| G1221 | The lease-time policy lookup adds an indexed read to every script-activity claim; its cost is unmeasured | Pending benchmark T023-056 |
| G1222 | A test run cannot exercise `checkpoint` or `stream.append` (always `NOT_GRANTED`), so an author tries them only in a real run | Accepted; 022's draft test runs may later grant them |
| G1223 | C#, Go and Java link their trusted API at build time, so versions built before the shim change cannot call the reserved host calls; TypeScript and Python receive them with the runner upgrade | Accepted; republish to use them |
| G1224 | Feature 026's draft contract names the script request `stream.publish`; C17 names it `stream.append` | 026 to cite this contract; noted for the coordinator | Resolved 2026-10-27: 026 now uses `stream.append`.
| G1225 | Until an operator records `script-languages-v1`, new languages and narrowing policies are refused (Decision 1234); a deployment that forgets the step sees `SCRIPT_LANGUAGES_CAPABILITY_DISABLED` | Accepted; the runbook's upgrade section names the step (Task 023-058) |
| G1226 | The lease reads the policy without a row lock (Decision 1235): a lease transaction that read the policy before a disabling update committed may lease once more; that attempt finishes | Accepted; bounded by the lease transactions in flight at the commit; stated in the runbook |
| G1227 | Whether feature 024 needs the `python-data` image catalog or can use design 046's dependency layer is not settled | Open; for the coordinator between 024 and 046 (Decision 1625); until then only the reservation is built |
| G1228 | Warm containers for new languages wait for per-language residue fixtures in feature 019's suite (Decision 1239) | Open; Task 023-070 after 019 ships the suite |
| G1229 | The sandbox runner registers worker protocol minor 0; the reserved host calls need it to negotiate the minors of 019 and 026 | Open; part of Task 023-064 and Task 023-066 |
| G1230 | Control planes and engines of release 0.2.x predate the instance registry, so recording `script-languages-v1` cannot see them; recorded too early, a 0.2.x control plane fails listings that contain a new-language row (Gap 1220) | Accepted; the upgrade runbook records the requirement only after the last 0.2.x process is gone, and Task 023-063 observes the case |

Updates of 2026-11-24 (states above are kept as history):

- Gap 1200: the OpenAPI `CreateScriptRequest.language` description on `develop` now names the five built
  languages; Studio `languages.ts` still marks Go and Java `built: false`. Half closed; Task 023-019 closes
  the rest.
- Gap 1201: feature 019 is approved and in release 0.3.0 (OD-2026-11-11-1, OD-2026-11-24-1); the rule stays
  until its target-aware routing is built.
- Gap 1213: proposed reservation `20270323000000_script_languages` and `20270323000100_script_languages_validate`
  (the second validates the widened registry CHECK, as `20270118000100` does). Newest on `develop` is
  `20270119000000`, newest on any open branch `20270209000100`; the coordinator confirmed the reservation on
  2026-11-26 (OD-2026-11-24-310), and the builder checks again before the slice merges.
- Gap 1220: once `script-languages-v1` is recorded, a control plane of a registry-aware release without the
  capability is held by the registry gate (`503 BINARY_CAPABILITY_MISSING`) instead of failing a listing
  (Decision 1234); a 0.2.x control plane is outside the registry (Gap 1230).
- Gap 1221: the lookup is now one plain statement for script activities only, counted in the budget test
  of design 038 (Decision 1235); its time cost stays to be measured by Task 023-056.

## 5. Shell-specific risks (stated honestly)

1. Shell is a language for running other programs. The image's binaries are the script's capabilities;
   `noexec` on `/tmp` stops executing written binaries but not `bash /tmp/x`, `awk -f` or `jq` programs:
   interpreters run text. The boundary is the container, not the mount option.
2. Input interpolated into commands (`eval`, unquoted expansions, `xargs`) lets a crafted input run
   commands inside the script's own sandbox and ask for `octopus fetch` to any host the egress policy
   allows. Input is delivered as a JSON file, never as environment variables or arguments, and the
   starter quotes; the platform cannot make an author's script injection-safe.
3. `set -x`, `declare -p`, `env` and error traces print expanded values, including secrets, to the log;
   the redaction net matches exact values and misses encodings (G1204).
4. A secret passed as a command argument is visible in `/proc/<pid>/cmdline` to every process of the same
   uid in the container, which is only the script's own processes.
5. Background jobs, `nohup`, `setsid` and `disown` outlive `bash`; they end when the container is
   removed at the end of the run, and the process limit bounds fork bombs.
6. Bash `/dev/tcp` and `/dev/udp` redirections open sockets; with network mode `none` only loopback
   exists (G1211).
7. Exit codes are not results: `errexit`, `nounset` and `pipefail` are on by default so a failed command
   fails the step; an author who turns them off owns partial results.
8. PowerShell and PHP carry analogous risks: `Invoke-Expression`, `Add-Type`, `eval`, `include` of
   written files, and `phar://` streams. None is a boundary problem; each is a correctness problem for
   the author, inside the same container.

## 6. Owner questions

- **O1200 — Initial catalogs.** Which packages ship in the first Rust and PHP catalogs? Recommended
  default: Rust ships exactly `serde_json` and its resolved closure (the trusted API needs it anyway);
  PHP ships the mechanism with one reviewed zero-dependency pure-PHP package chosen at T023-035 with
  security review, mirroring the single Newtonsoft.Json entry that proved the C# path; Bash and
  PowerShell ship no packages.
- **O1201 — Default namespace policy for the new languages.** Recommended default: enabled in every
  namespace, because deployments already opt in through `SAGAT_RUNNER_LANGUAGES`; namespace admins may
  disable a language, for example in production namespaces.
- **O1202 — Deno/Bun follow-up.** Recommended default: do not adopt either; revisit only if the US07
  report shows a measured warm-start or memory advantage on the reference environment together with
  identical TypeScript conformance results, through a new ADR.

### Answers (2026-11-24), delegate (OD-2026-11-24-4)

The three questions were open when the drafts were approved (OD-2026-11-11-1). Under the owner's
delegation for release 0.3.0 they are decided here; each answer is reported to the owner, who may
overrule it.

- **Owner question 1200 (initial catalogs): a small reviewed starter catalog per language, chosen by
  stated criteria, not a single proof package.** Options weighed: (a) the draft's minimum (Rust: the
  `serde_json` closure only; PHP: one zero-dependency package), which proves the mechanism and little
  else; (b) a starter catalog sized for the work scripts actually do (JSON, text and regular expressions,
  encodings, hashing, time, identifiers, CSV), admitted entry by entry under criteria; (c) no packages in
  the first release. Chosen (b): Windmill lets Rust and PHP scripts take any crates.io or Packagist
  package, and a catalog of one package would leave those two languages a demonstration rather than a
  tool, while the cost of each entry is bounded and paid once (a review, the archive checks, the advisory
  audit at image build, a new image digest), and the architecture is identical for one entry or thirty.
  Criteria, binding on every entry: a permissive SPDX licence; no network code that could only fail in
  the sandbox; for Rust, no Cargo build script that compiles foreign code and no feature selection by the
  author (each crate is compiled once with fixed features); a proc-macro only when a widely used derive
  needs it (`serde_derive`), because it runs in the builder container on every build that uses it
  (Decision 1228); for PHP, pure PHP, no extension outside the runtime's closed set, no
  `composer-plugin`, and `autoload.files` only after a reviewed reason (Gap 1214). Cap: at most 32 root
  packages per language in release 0.3.0, closures included in the 512-entry ceiling. Rust starts from
  `serde_json` with `serde` and `serde_derive`, then candidates such as `regex`, `base64`, `hex`, `sha2`,
  `time` (without its macros), `uuid` and `csv`; PHP starts from candidates such as `league/csv`,
  `ramsey/uuid` with `brick/math`, and a YAML parser if one meets the criteria. The final lists are
  fixed with the security review of Task 023-023 and Task 023-035 and written into each catalog's
  `review` fields; a candidate that fails a criterion is left out, not waived. Bash and PowerShell ship no
  packages. TypeScript and Python open dependencies are design 046's.
- **Owner question 1201 (default namespace policy): every built language enabled by default.** Options
  weighed: (a) enabled everywhere, narrowed by namespace administrators; (b) disabled everywhere until a
  namespace administrator enables each; (c) a new deployment setting on the control plane that chooses
  the default. Chosen (a): a deployment already opts in twice before a language can run, once by
  releasing a runner whose `SAGAT_RUNNER_LANGUAGES` names it (default: the five 005 languages, so an
  upgrade serves nothing new by itself) and once by recording `script-languages-v1` (Decision 1234); a
  third switch (c) would be one more place that can disagree with the other two and with the namespace
  policy, and (b) makes every namespace administrator repeat what the operator already decided. A
  namespace that must not run a language, for example a production namespace without Bash, disables it
  with one audited policy update, which also stops admitted runs at their next lease (Decision 1226).
- **Owner question 1202 (Deno and Bun): not adopted; the spike stays in release 0.3.0 as input to the
  published benchmark.** Options weighed: (a) adopt Bun or Deno as a TypeScript variant now; (b) not
  adopt, and keep the bounded spike; (c) not adopt, and drop the spike. Chosen (b): Windmill runs
  TypeScript on Bun, and release 0.3.0 publishes a benchmark against Windmill (OD-2026-11-24-1), so
  measured start time and memory of Node, Deno and Bun under the same container profile are evidence the
  business needs whatever the answer; adopting either now would bind a third JavaScript engine into the
  toolchain identity and the conformance suite without that evidence (Decision 1201). Adoption later
  takes a new architecture decision, identical conformance results and a measured advantage on the
  reference environment.

Note (2026-11-20, from Architecture decision 0099): worker-protocol minor 1.8 is reserved for delivery acknowledgement (feature 006, Owner decision 2026-11-20-12; Owner question 006-002); the next free minor is 1.9, and minors are cumulative. An internal runner of this draft at 1.9 or later either implements the 1.8 host procedure (a flagged lease is acknowledged before it runs) or registers with `WorkerCapabilities.declines_acknowledgement = true` (field 10), and the server then flags none of its leases (Decision 006-063 of `specs/006-sdk-multilanguage/research.md`). `TaskLease` fields 23 and 24 and `WorkerCapabilities` field 10 are taken.
