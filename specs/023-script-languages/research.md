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
