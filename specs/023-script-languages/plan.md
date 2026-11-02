# Implementation Plan: Additional script languages

**Branch**: `feature/023-script-languages` | **Date**: 2026-10-26 | **Spec**: [spec.md](./spec.md)

Status: Proposed — not started.

## Summary

Add `bash`, `powershell`, `php` and `rust` to the feature-005 script registry and sandbox runner without a
second execution model. Each language gets a pinned builder and runtime image under `runners/`, a trusted
in-container shim speaking the existing JSON-line channel, a bundled startup probe and a validator branch:
Bash and PowerShell reuse bundle v1, Rust reuses bundle v3 and the sealed-memory loader, PHP introduces
bundle v4 (a bounded text tree). Rust and PHP dependencies come only from closed, image-baked catalogs
whose logic is shared by the control plane and the runner; the id `python-data` is reserved for feature
024's image-catalog runtime, whose mechanism this feature defines. Every build gains a signed
DSSE/in-toto provenance statement stored per build key; every namespace gains a narrowing runtime
policy, enforced also when a script step is leased; Studio gains Monaco
highlighting, starters and an availability-driven language list. Deno and Bun are measured in a spike
outside product code. Hard-to-reverse choices are in
[ADR-0057](../../docs/adr/ADR-0057-additional-script-languages-reuse-the-sandbox-and-signed-artifacts.md).

## Technical Context

**Language/Version**: Rust 1.98.0 (`rust-toolchain.toml`) for the platform; TypeScript/React for Studio;
in-container hosts in Rust (`runners/shell/host`, `runners/rust/sagat-inline`), PowerShell and PHP.
Script toolchains: GNU Bash 5.x, PowerShell 7.6 LTS (on .NET 10), the current stable PHP minor (8.5 at the time of
writing) with a pinned Composer 2, rustc pinned in the builder image (initially 1.98.0, then independent of the
workspace) with `*-unknown-linux-musl` targets. Exact versions and image digests are fixed at
implementation and recorded in the toolchain identity.

**Primary Dependencies**: existing Tokio, Axum, tonic, SQLx, bollard (`crates/infra-sandbox`), flate2 and
base64 (`crates/infra-schema`), ed25519 signing (`crates/application/src/script_signing.rs`); new in
adapters only: a TOML parser for Cargo.lock in `crates/infra-schema`. Monaco 0.57.0 (already pinned).

**Storage**: PostgreSQL (one additive migration); artifacts, SBOM and provenance stay in
`script_artifacts` (2 MiB ceiling, G108).

**Testing**: `cargo test` unit suites; real PostgreSQL integration; real-container suites (ignored by
default, run in the `native-runners` CI job on `ubuntu-24.04` and `ubuntu-24.04-arm`); REST contract
suite against the merged OpenAPI; fault suites; Playwright real-browser tests; five-SDK conformance.

**Target Platform**: Linux AMD64 and ARM64 runners with a container engine; Kubernetes via Helm and
Docker Compose `sandbox` profile for development.

**Project Type**: modular Rust monorepo + React Studio + five SDKs.

**Performance Goals**: none promised. Cold/warm start, build time, peak memory and image size per language
are benchmark criteria measured on a declared environment (SC-023-013).

**Constraints**: the 005 container profile is not relaxed for any language; the control plane never
starts a container; the runner holds no database URL or signing key; no network in builds or runs.

**Scale/Scope**: four languages, two native targets, two catalogs, four new REST operations plus one additive response field (`ScriptVersion.provenance`), one reserved runtime id (`python-data`), one migration.

## Constitution Check

These are design checks, not implementation approvals or test results.

| Principle | Before research | After design | Evidence / future gate |
|---|---|---|---|
| I — durable atomic transitions | Pass | Pass | No new state machine; build completion commits artifact, SBOM, per-build provenance, version, job and audit together; a step refused at lease fails with journal, next work, outbox and audit in the lease transaction (transaction table); T023-009, T023-061, T023-052 |
| II — pure domain, inward dependencies | Pass | Pass | `Language`, policy narrowing and package identities are domain values; TOML/JSON lock parsing and DSSE live in adapters; `scripts/check-architecture.py` (T023-059) |
| III — at-least-once effects | Pass | Pass | Scripts keep the step idempotency key; no exactly-once claim; G114 of 005 unchanged; T023-052 |
| IV — pinned versions and toolchains | Pass | Pass | Digest-pinned images, catalog digest in the build key, immutable artifacts; additive contracts; T023-004, T023-010, T023-051 |
| V — isolated code, tenant and secrets | Pass | Pass | Unchanged profile, no relaxation, runner without database; tenant-keyed rows; hostile battery per language; T023-029..033, T023-054 |
| VI — OTel, audit, typed errors | Pass | Pass | Spans, closed-label metrics, audit actions and `details.reason` values below; T023-012, T023-044, T023-055 |
| VII — inbox/outbox, redelivery | Pass | Pass | No new message path; script steps keep worker-protocol redelivery and reconciliation; T023-052 |
| VIII — failure evidence, verified contracts | Pass | Pass | Failure matrix below; contract merge and lint; honest pending items G1206/G1207/G1216; T023-052..059 |

**Governance gate**: ADR-0057 is proposed, pending owner acceptance. It refines ADR-0013, ADR-0021 and
ADR-0024 and supersedes none of them. Implementation starts only after acceptance.

## Project Structure

### Documentation (this feature)

```text
specs/023-script-languages/
├── spec.md  plan.md  research.md  data-model.md  quickstart.md  tasks.md
├── checklists/requirements.md
└── contracts/
    ├── openapi-script-languages.yaml       # REST fragment (enums, four operations, schemas)
    ├── workflow-script-language.schema.json # IR fragment (script.language enum)
    ├── artifact-bundle-v4.schema.json       # PHP tree bundle
    ├── shim-protocol.md                     # per-language host-call shims over the existing channel
    ├── catalog.schema.json                  # catalog file format (runners/*/catalog/catalog.json)
    ├── provenance.md                        # DSSE/in-toto/SLSA statement
    └── errors.md                            # details.reason values and failure mapping
```

### Source placement

| Path | Change |
|---|---|
| `crates/domain/src/scripts.rs` | `Language::{Bash, PowerShell, Php, Rust}` and reserved `PythonData` (`is_built` false), extensions, `is_built`; per-language default policy and build profile; `RuntimePolicy` narrowing rules; `PackageRef` value |
| `crates/domain/src/identity.rs` | `Permission::ScriptPolicyManage` (`script:policy:manage`), admin only |
| `crates/domain/src/audit.rs` | `script.policy_changed`, `script.policy_denied` |
| `crates/application/src/scripts.rs` | policy get/put with CAS, admission checks, discovery, catalog listing, provenance read, presence upsert; ports on `ScriptStore` |
| `crates/application/src/sandbox.rs` | `probe_language` fails closed for every new language unless the adapter implements it; `BuildRequest` carries the validated package set |
| `crates/application/src/script_signing.rs` | DSSE signing and verification of provenance with the existing key |
| `crates/infra-schema/src/bundle.rs` | v1 accepts `bash`/`powershell`; v3 accepts `rust` with the unchanged ELF rules; new v4 for `php` and `python-data` (`runtimeCatalog`) |
| `crates/infra-schema/src/catalog.rs` (new) | embedded catalogs, Cargo.lock, composer.lock and hash-pinned requirements parsing, closure and policy checks, lock text generation |
| `crates/infra-schema/src/provenance.rs` (new) | in-toto statement construction and shape validation |
| `crates/infra-sandbox/src/docker.rs`, `lockfile.rs` | images, commands, probes, toolchain identity, native build semaphore; lockfile policy delegates to `infra-schema` catalog |
| `crates/infra-sandbox/src/protocol.rs`, `crates/infra-sandbox/shims/runtime.{mjs,py}`, `runners/{dotnet,go,java}` trusted APIs | reserved host calls `checkpoint` and `stream.append` (FR-023-028): message parsing, per-run budgets, the previous checkpoint in the first message, and the author API of the five 005 languages |
| `runners/shell/` | `Dockerfile` (one image for build and run), toolset manifest, `host/` (in-container host and `octopus` helper, Rust, own `Cargo.lock`) |
| `runners/powershell/` | `Dockerfile`, `build.ps1` (parser check), `runtime.ps1`, `Sagat.Octopus/` module |
| `runners/php/` | `Dockerfile.builder` (catalog trees, pinned Composer), `Dockerfile.runtime` (no catalog), `build.php`, `runtime.php`, `php.ini`, `catalog/catalog.json` |
| `runners/rust/` | `Dockerfile.builder` (rustc, musl targets, prebuilt read-only catalog libraries), `Dockerfile.runtime`, `buildhost/`, `sagat-inline/` trusted crate, `catalog/catalog.json` |
| `runners/native/runtime.py` | the sealed-memory loader moved from `runners/go/runtime.py`, shared by Go and Rust (G1218) |
| `runners/ts-variants/` | spike-only Deno and Bun images; not referenced by any product configuration |
| `apps/sandbox-runner/src/host.rs` | gate on the lease's capabilities; secret net; forward checkpoints with the next heartbeat (019) and appends as `AppendStreamEvents` (026); supervisor-assigned sequences |
| `apps/sandbox-runner/src/{config,lib,api}.rs` | `SAGAT_RUNNER_LANGUAGES`, `SAGAT_RUNNER_REQUIRED_LANGUAGES`, `SAGAT_RUNNER_RUST_TARGET`, `SAGAT_RUNNER_MAX_NATIVE_BUILDS`, image variables, probes, presence fields on claims |
| `crates/infra-postgres/src/scripts.rs`, `migrations/<next>_script_languages.{up,down}.sql` | constraint, policy table, presence table, provenance column, grants, classification manifest entries |
| `crates/api-http/src/routes/{scripts,runner}.rs` | four operations and `ScriptVersion.provenance`; policy and lock checks at admission; presence on claims |
| `crates/application/src/definitions.rs`, `ScriptLookup` in `crates/application/src/scripts.rs` | workflow publication rejects a script node whose artifact language is disabled by policy (beside `missing_artifacts`) |
| `crates/application/src/worker.rs` (`lease_one`), `crates/infra-postgres/src/workers.rs` | lease-time language check of `script.sha256-*` steps: refuse, fail the step non-retryable `SCRIPT_DENIED`, audit (D1226) |
| `frontend/apps/studio/src/features/scripts/` | `languages.ts`, `monaco.ts`, starters, catalog picker, provenance/SBOM panel |
| `sdks/typescript/src/workflow.ts`, `sdks/python/sagat_octopus_orchestrator/workflow.py` | language unions; Go/C#/Java already accept strings |
| `tests/{integration,e2e,fault,contract,performance}/script-languages/`, `tests/contract/sdk-conformance/scenarios/` | suites below |
| `docs/runbooks/scripts.md`, `docs/architecture/security-and-sandbox.md`, `tests/performance/reports/script-languages.md` | runbook, architecture and report |

## Design

### 1. Builders and runtimes per language

| Language | Builder image | Build command (fixed) | Artifact | Runtime image | Run command (fixed) | Shim |
|---|---|---|---|---|---|---|
| `bash` | `sagat/shell` | `sagat-shell-host build` → `bash -n main.sh` | v1 `main.sh` | same image | `sagat-shell-host run` → `bash --noprofile --norc -o errexit -o nounset -o pipefail main.sh` | `octopus` helper over a Unix socket |
| `powershell` | `sagat/powershell` | `pwsh -NoProfile -NonInteractive -NoLogo -File /opt/sagat/build.ps1` (AST parse; `Run` defined) | v1 `main.ps1` | same image | `pwsh … -File /opt/sagat/runtime.ps1` | `Sagat.Octopus` module |
| `php` | `sagat/php-builder` | `php -n -c /opt/sagat/php.ini /opt/sagat/build.php` (`php -l`, catalog copy, `composer dump-autoload --classmap-authoritative --no-dev --no-scripts --no-plugins`) | v4 tree | `sagat/php-runtime` | `php -n -c /opt/sagat/php.ini /opt/sagat/runtime.php` | `Octopus` class |
| `rust` | `sagat/rust-builder` | `/opt/sagat/buildhost` → one `rustc` (D1204, D1205) | v3 `script.elf` | `sagat/rust-runtime` | `python -I -B /opt/sagat/runtime.py` (shared loader) | `sagat_inline` crate |

Every container keeps the 005 profile. Interpreted languages use one image for build and run, as
TypeScript and Python do; PHP and Rust separate them so the runtime has neither the catalog nor the
compiler. Images are built from `runners/`, run as user 65534, have setuid/setgid bits stripped, and
contain no token, key or platform state. The toolchain identity is
`build=<image id>;runtime=<image id>;protocol=artifact-v<n>;lang=<id>;version=<interpreter or compiler>;shim=<sha256>[;target=…;abi=sagat-inline-v1][;catalog=<sha256>]`,
at most 512 bytes (the existing `ClaimBuildRequest` bound).

### 2. Catalogs and lockfiles

`runners/rust/catalog/catalog.json` and `runners/php/catalog/catalog.json` follow
[catalog.schema.json](contracts/catalog.schema.json). The image build downloads each entry once, verifies
the upstream checksum (crates.io `checksum`; Composer dist reference) and an independent SHA-256, runs the
advisory audit with fatal findings, and for Rust compiles every crate with the exact flags of D1204 into
`/opt/sagat/rust/deps` (read-only). `crates/infra-schema/src/catalog.rs` embeds the same files, so the
control plane validates a lock at admission (early, typed `LOCKFILE_INVALID` / `PACKAGE_NOT_ALLOWED`)
and the runner validates it again against its image (authoritative, build kind `lockfile`). A catalog
digest mismatch between control plane and image fails the build as `lockfile` with a fixed message. The
discovery API generates the canonical lock text for a selection, so a lock is never hand-written.

### 3. Bundle v4

See [artifact-bundle-v4.schema.json](contracts/artifact-bundle-v4.schema.json). The validator in
`bundle.rs` rejects unknown or duplicate fields, non-canonical JSON, paths with `..`, empty or dot-led
segments, more than 10 segments, more than 255 path bytes, case-insensitive collisions, more than 1,024
files, non-UTF-8 or NUL content, more than 1,400 KiB decoded content, an `entry` other than `main.php`,
a missing `vendor/autoload.php` when `lockDigest` is set, and any language other than `php`. The runtime
host writes the tree under `/tmp/app` (interpreted, so `noexec` is irrelevant) after validating again.

### 4. Rust

The builder accepts `{"op":"build","source":…,"lockfile":…,"packages":[…]}`, writes `/tmp/src/script.rs`
and the fixed `main.rs`, and runs `rustc` with an empty environment plus fixed variables. Proc-macro
libraries are mapped from the read-only image path, never from `/tmp`. The output is gzip-encoded
canonically (shared with Go, `compression=gzip-canonical-v1`), validated by `static_elf()` for the
runner's target, and reported with the SBOM. The runtime image is Go's loader image pattern with
`runners/native/runtime.py`. Target mismatch at load is `SCRIPT_SANDBOX_UNAVAILABLE`.

### 5. Shims

[shim-protocol.md](contracts/shim-protocol.md) fixes each language's author API and the mapping to the
existing messages. The runner-side supervisor in `docker.rs` is unchanged except for per-language command
lines, first messages and line bounds (Rust uses the native `MAX_BUILD_LINE`; others the ordinary line).

The protocol gains two reserved host calls for every script runtime, including the five of 005 and
`python-data` (coordinator decision C17, FR-023-028, D1231–D1233): `checkpoint` and `stream.append`. This
feature fixes only their shapes, the lease-capability gate (`NOT_GRANTED` otherwise, always in test runs),
the size bounds and per-run budgets, supervisor-assigned sequences, and the secret net; feature 019 owns
checkpoint meaning and storage (FR-019-015..018) and feature 026 owns streams (FR-026-013..020,
ADR-0068). Checkpoints are queued and sent with the next heartbeat or the final failure report; appends
are forwarded synchronously as `AppendStreamEvents`. Until 019 or 026 ships, the corresponding call is
`NOT_GRANTED`, so the shims and tests can land first.

### 6. Probes and advertisement

`Sandbox::probe_language` gains real implementations for the four languages in `DockerSandbox`: build the
bundled probe source through the production builder, validate, run it through the production runtime
with `NoHost` plus an idempotency key, and require `{"ok":true,"idempotencyKey":<key>}`. `Runner::start`
probes only languages in `SAGAT_RUNNER_LANGUAGES`; required languages fail startup; Rust and Go targets
must be equal. Outside `SAGAT_ENV=dev` an image reference without `@sha256:` fails the probe of a new
language. The `languages` field of `WorkerCapabilities` and build/test claims carry the probed set only.

### 7. Provenance and SBOM

At build completion the control plane builds the statement of [provenance.md](contracts/provenance.md)
from what it verified and what the runner reported, signs the DSSE PAE with the existing Ed25519 key
(`keyid` = the artifact `key_id`), and stores the envelope in `script_build_provenance` keyed by tenant
and build key (D1227). The artifact row stays content-addressed and insert-once; a second build of the
same digest under another toolchain has another build key and so its own statement; a version finished
from the build-key cache reads the statement of that key. `getScriptVersion` returns it. The SBOM is the
runner's, checked by the control plane against the lockfile package set before signing.

### 8. Runtime policy and discovery

`script_runtime_policies` holds one revisioned document per namespace (data-model.md). Admission reads it
under `FOR SHARE` in the admission transaction; a missing row means the deployment default (O1201: all
built languages enabled, whole catalog). Discovery reads the policy, the embedded catalogs and the
presence rows; presence is upserted by claim handlers at most once per 60 s per runner subject.

### 8a. Lease-time language check

The engine's lease path (`lease_one`) recognizes `script.sha256-<hex>` activities. Inside the lease
transaction, after the existing run and step locks, it reads the artifact language by
`(tenant_id, digest)` (primary key of `script_artifacts`) and the namespace policy row `FOR SHARE`. A
disabled language produces no lease: the step takes the same non-retryable failure transition a worker
report would cause (`SCRIPT_DENIED`, fixed safe message), with its journal event, next scheduled work
and outbox rows, plus a `script.policy_denied` audit row (point `lease`), all in that transaction. Other
activities pay nothing; script activities pay one indexed read (G1221).

### 8b. Reserved `python-data` runtime (mechanism for feature 024)

`Language::PythonData` (`python-data`) exists with `is_built` false. Its artifact is bundle v4 with
entry `main.py`, `lockDigest` and a non-null `runtimeCatalog`; its lock is a hash-pinned requirements file
validated by `catalog.rs` against a catalog of kind `pypi`; its toolchain identity names the runtime image,
the Python version, the shim and the catalog digest. The runner loads it only when its configured
`python-data` runtime image reports the same catalog digest, and builds an import path of the locked
packages only. Feature 024 supplies the image, the catalog file and the execution environment, and flips
`is_built` with its own evidence; this feature ships the validator branch, the lock parser, the catalog
kind and fixture tests (T023-062).

### 9. Studio, SDKs, spike

Studio registers `monaco-editor/languages/definitions/{shell,powershell,php,rust}/register`, adds
starters, reads `listScriptLanguages` for the enabled set, offers the catalog picker and a provenance/SBOM
panel. TypeScript and Python SDK unions gain the four ids; a conformance scenario fixes the canonical
bytes. The Deno/Bun spike lives in `runners/ts-variants/` and `tests/performance/script-languages/`
only.

## Transaction boundaries

| Operation | Commits together | Lock order / fence |
|---|---|---|
| Create draft | script row, content-byte accounting, `scripts` quota, `script.created` audit | namespace policy row `FOR SHARE` → quota → script row insert |
| Update draft | as 005 (CAS on `revision`), plus early lockfile shape check before the write | script row; the language is fixed at creation, so no policy read |
| Publish version | version, build job, `script_jobs` quota, `script.published` audit | policy row `FOR SHARE` → script row lock (as 005) → quota |
| Test-run admission | test run, quota, `script.tested` audit | policy row `FOR SHARE` → script row → quota |
| Build completion | artifact (content-addressed, `ON CONFLICT DO NOTHING`), signature, SBOM, provenance row keyed by build key (insert-once; a repeat of the same build is a no-op), version `READY`, job `SUCCEEDED`, `script.built` audit | job lease token check → version row; no new lock |
| Lease of a script step | either the lease (as today) or, for a disabled language, the step failure transition with journal event, next work, outbox and `script.policy_denied` audit | existing run and step locks → artifact row read → policy row `FOR SHARE` |
| Workflow publication | existing publication records plus the policy check of each script node's artifact language; a refusal commits only its `script.policy_denied` audit | policy row `FOR SHARE` read during validation, before any publication write; existing publication locks unchanged after it |
| Policy update | policy row (revision + 1), `script.policy_changed` audit | policy row `FOR UPDATE` with expected revision; never takes a script or run lock |
| Policy denial | `script.policy_denied` audit only (once per actor, resource and reason), committed in the admission transaction itself: the policy is read first, so at the refusal that transaction holds no other write | policy row `FOR SHARE` → audit identity lock (as `script.violation`) |
| Presence upsert | one presence row | `ON CONFLICT … WHERE seen_at < now() - 60 s`; never in an admission transaction |

No transaction spans a container, the container engine or a provider call; the runner commits through
REST and the worker protocol as in 005.

## Authorization

| Permission | admin | developer | operator | viewer | worker | runner | database-runner |
|---|---|---|---|---|---|---|---|
| `script:read` (scripts, discovery, catalog, provenance, SBOM) | yes | yes | yes | yes | no | no | no |
| `script:write`, `script:publish` (any language) | yes | yes | no | no | no | no | no |
| `script:policy:manage` (new) | yes | no | no | no | no | no | no |
| `script:build`, `script:execute` (claims, presence) | yes | no | no | no | no | yes | no |

Foreign resources are opaque `NOT_FOUND`; denials are audited without becoming an existence oracle.
Policy revocation applies to the next admitted call. The runner's token is worth one tenant.

## Finite ceilings

| Resource | Default | Hard ceiling |
|---|---:|---:|
| Source bytes per draft (all languages) | 256 KiB | 256 KiB |
| Lockfile bytes | 256 KiB | 256 KiB |
| Locked packages per lockfile (Rust, PHP), fixed | 256 | 256 |
| Catalog entries per language per deployment | — | 512 |
| Bundle v4 files / path bytes / segments | 1,024 / 255 / 10 | same |
| Bundle v4 decoded content / stored bundle | 1,400 KiB / 2 MiB | same |
| Rust executable compressed / expanded | 1,400 KiB / 8 MiB | same (v3) |
| Interpreted build limits | 256 MiB, 1,000 m, 30 s, 64 pids, 32 MiB scratch | 005 defaults |
| PHP build scratch | 64 MiB | 128 MiB |
| Rust build memory / CPU / wall / pids / scratch | 1 GiB / 2,000 m / 120 s / 128 / 512 MiB | 2 GiB / 4,000 m / 600 s / 256 / 512 MiB |
| Concurrent Rust builds per runner | 1 | 4 |
| PowerShell runtime memory default | 512 MiB (pending G1207) | 2 GiB (domain maximum) |
| Other runtime limits | 005 defaults | 005 ceilings (`SAGAT_SCRIPT_CEILING`, `SAGAT_RUNNER_CEILING`) |
| Host calls / channel lines / result | 100 / 400 / 240 KiB | same |
| Reserved calls per run: `checkpoint` / `stream.append`; resulting channel lines | 600 / 1,000; 2,000 | same |
| Checkpoint details / stream event / append request | 019 `heartbeat_detail_bytes` (16 KiB) / 026 event (16 KiB) / 64 events, 64 KiB | 64 KiB / 64 KiB / 64 events, under the 240 KiB line |
| Concurrent helper connections per Bash run | 8 | 8 |
| Runtime policy document | 16 KiB | 16 KiB |
| Allowed package names per language in a policy | 256 | 256 |
| Presence rows per tenant | 256 | 256 |
| Provenance envelope | 64 KiB | 64 KiB |
| SBOM | 256 KiB | 256 KiB (existing check) |

Effective values are the minimum of request, namespace and deployment; zero is not unlimited; admission
fails before work. `scripts` (500) and `script_jobs` (8) quotas apply unchanged.

## Failure and recovery matrix

| Failure | Detection | Outcome | Test |
|---|---|---|---|
| Runner dies during a build of any new language | build lease expiry | another runner takes it (three attempts), same build key; no second artifact | T023-052 |
| Runner dies during a step | worker lease expiry | engine retries; same idempotency key; container swept | T023-052 |
| Crash after artifact insert, before commit | transaction rollback | nothing visible; next completion inserts once | T023-052 |
| Duplicate build completion | job token and state | accepted once; repeat from holder idempotent; stale holder `409` | T023-052 |
| Lost broker hint for a ready script step | reconciliation loop | step found and leased; nothing lost | T023-052 |
| Cancellation racing completion | first commit wins | loser is an audited no-op; container removed | T023-052 |
| Probe fails at startup | runner start | language omitted, or startup fails if required | T023-010 |
| Catalog digest mismatch | builder check | build kind `lockfile`, fixed message | T023-007, T023-024 |
| Target mismatch at load | runtime check | `SCRIPT_SANDBOX_UNAVAILABLE`, retryable | T023-024, T023-028 |
| Policy changed during admission | share/update lock | one serial order; no mixed view | T023-011, T023-045 |
| Language disabled while steps are ready or running | lease-time check | ready steps fail `SCRIPT_DENIED` at their next lease; running attempts finish; a retry is refused at its lease | T023-061 |
| Lease lost while a script appends or checkpoints | 026 `STALE_ATTEMPT`; heartbeat fence | the append stores nothing and the script is told; a pending checkpoint is dropped with the lease; the step outcome is unchanged | T023-065 |
| Runner restarts with a queued checkpoint | lease expiry | only checkpoints already sent with a heartbeat exist; the next attempt gets the latest committed one (019) | T023-065 |
| Control plane restarted with an in-flight build | lease | runner completion after restart accepted with the token | T023-052 |
| Engine wedged on a container | 20 s engine-call bound (G119) | run fails unavailable; runner slot freed | inherited, not re-proven |

There is no compensation in this feature: a script's external effect is the author's, at least once.

## Observability

**Spans** (exported by the runner and control plane; ids as attributes, never labels):
`script.build` (`sagat.script.language`, `sagat.outcome`, `sagat.failure.code`), `script.run`,
`script.probe` (startup), `script.lockfile.validate` (control plane), `script.provenance.sign`,
`script.policy.check`. Step attempt spans stay journal-derived (ADR-0017).

**Metrics** (closed labels): existing `script_runs_total{language,outcome}`,
`script_start_seconds{language,temperature}`, `script_run_seconds{language}`; existing
`script_builds_total{outcome}` and `script_build_seconds` gain `language`; new
`script_probe_total{language,outcome}` (`passed|failed|skipped`),
`script_lockfile_refusals_total{language,reason}` (`malformed|unknown_package|version_mismatch|checksum_mismatch|incomplete_closure|policy_denied|catalog_mismatch`),
`script_policy_denials_total{language,point}` (`draft|publish|test|workflow|lease`),
`script_native_builds_waiting` (gauge),
`script_checkpoints_total{language,outcome}` (`queued|stored|coalesced|stale|refused_size|refused_secret|refused_encoding|not_granted|budget`),
`script_stream_appends_total{language,outcome}` (`appended|invalid|secret|limit|rate|stale|not_granted|budget`). `language` is one of the ten ids (nine languages and the reserved `python-data`).

**Logs**: structured, redacted; never source, input, result, lock content, package selection, secret or
a URL path/query.

**Audit**: `script.policy_changed` (actor, namespace, revision, changed languages), `script.policy_denied`
(actor, resource, point, reason), plus existing `script.*`.

## Migration and compatibility

1. Additive migration: replace `scripts_language_valid` with the list of ten ids (the nine languages and
   the reserved `python-data`), create `script_build_provenance`, `script_runtime_policies` and
   `script_runner_presence`, grants for `sagat_app`, storage-classification entries. Down refuses while
   any row has a new language, any provenance row or any policy exists, never deleting customer data.
   Because every build after the upgrade writes provenance, the down migration is in practice usable
   only before the first build (D1230).
2. Control plane first (accepts new ids, serves discovery, signs provenance), then runner images, then
   runners with an explicit `SAGAT_RUNNER_LANGUAGES`. A runner without the new variables behaves as today.
3. Contracts grow additively (ADR-0032). Capability is gated by publication, as FR-014-016 requires: a
   language is accepted only when `is_built` is true and, for workflows, only through a ready artifact of
   the tenant. Go, C# and Java SDKs already accept any language string; TypeScript and Python users
   upgrade to name the new ids.
4. Rollback is binary-only and forward-only for data (D1230): keep the migrated schema. Before any
   new-language script exists, the control plane and runners may be rolled back to 0.2.0 binaries, which
   ignore the new tables. After one exists, a 0.2.0 control plane fails any script listing containing it
   (`corrupt("script language")`, G1220), so the supported path is to disable the languages (runner
   allowlist, namespace policy) and roll forward. T023-063 runs a 0.2.0 control plane against the
   migrated schema with and without new-language rows and records what it observes.

## Complexity Tracking

No new process or app is added (coordinator decision C21 does not apply): builds and runs stay in
`apps/sandbox-runner`, and the reserved host calls of FR-023-028 add no transaction of this feature —
019 and 026 own the heartbeat and append transactions.

| Decision | Cost | Simpler alternative rejected because |
|---|---|---|
| Bundle v4 tree for PHP | a fourth validator branch | v1 caps 32 flat files; catalog in the runtime image would let the image drift from the signed version |
| `rustc` without Cargo | builder writes its own invocation | Cargo reads manifests and runs build scripts; `--frozen` still resolves and builds dependencies per build |
| Prebuilt catalog libraries in the image | larger builder image (G1208) | building dependencies per customer build exceeds bounded build profiles and runs build scripts |
| Shared native loader | Go proofs must be rerun | a copied loader would fork reviewed security code |
| Provenance for every language | 005 builds gain a record | language-specific provenance would leave older languages unprovable |
| Presence table | one bounded upsert per minute per runner | worker sessions need non-empty activities and exist only while steps are ready |

## Delivery sequence

1. ADR-0057 acceptance; contracts drafted here merged into canonical files with implementation.
2. Foundation: domain ids, migration, catalog and bundle validators, provenance signing, probes framework.
3. US01 Bash (MVP) → US05 battery for Bash.
4. US02 Rust on ARM64 and AMD64 → US05 battery for Rust.
5. US03 PHP, US04 PowerShell, each with their battery.
6. US06 policy, discovery, provenance UI.
7. US07 spike and report.
8. Faults, performance report, runbook, quickstart, reviews and gates.

## Verification plan

Every task names its evidence. Required before release: real-container batteries per language and per
target; native AMD64 and ARM64 CI jobs; v1/v2/v3 regression unchanged; REST contract suite over the
merged OpenAPI; five-SDK conformance; real-browser Studio flows; fault suite of the matrix above;
architecture check; independent security and correctness reviews; performance report with environment
and failures. A skipped or failing test is reported as such.
