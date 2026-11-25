# Implementation Plan: Additional script languages

**Branch**: `feature/023-script-languages` | **Date**: 2026-10-26 | **Spec**: [spec.md](./spec.md)

Status: Proposed — not started. Updated 2026-11-24: approved for implementation (Owner decision
2026-11-11-1) and part of release 0.3.0 (Owner decision 2026-11-24-1); re-read against `develop` at
`2c94064f` and prepared for build (research section 1a, Decisions 1234 to 1239 and 1625 to 1627);
Architecture decision 0057 accepted with an amendment, delegate (OD-2026-11-24-4). Not built.

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

**Language/Version**: Rust 1.98.0 (`rust-toolchain.toml`) for the platform (1.100.0 on 2026-11-24); TypeScript/React for Studio;
in-container hosts in Rust (`runners/shell/host`, `runners/rust/sagat-inline`), PowerShell and PHP.
Script toolchains: GNU Bash 5.x, PowerShell 7.6 LTS (on .NET 10), the current stable PHP minor (8.5 at the time of
writing) with a pinned Composer 2, rustc pinned in the builder image (initially 1.98.0, then independent of the
workspace) with `*-unknown-linux-musl` targets. Exact versions and image digests are fixed at
implementation and recorded in the toolchain identity.

**Primary Dependencies**: existing Tokio, Axum, tonic, SQLx, bollard (`crates/infra-sandbox`), flate2 and
base64 (`crates/infra-schema`), ed25519 signing (`crates/application/src/script_signing.rs`); new in
adapters only: a TOML parser for Cargo.lock in `crates/infra-schema`. Monaco 0.57.0 (already pinned).
Shared crate (2026-11-24, Decisions 1236 and 1627): design 046's new adapter `crates/infra-packages`
(`sagat-infra-packages`, Task T046-15) holds the provenance and SBOM builders; this feature depends on it and
adds its build type.

**Storage**: PostgreSQL (one additive migration); artifacts, SBOM and provenance stay in
`script_artifacts` (2 MiB ceiling, G108). Corrected 2026-11-24: artifacts and SBOMs stay in
`script_artifacts`; provenance statements are rows of `script_build_provenance`, one per build key
(Decision 1227). Two migrations, the second validating the widened registry CHECK (Decision 1234).

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

**Scale/Scope**: four languages, two native targets, two catalogs, four new REST operations plus one additive response field (`ScriptVersion.provenance`), one reserved runtime id (`python-data`), one migration (two since 2026-11-24: the second validates the widened registry CHECK), one registry capability (`script-languages-v1`), no new crate (it extends design 046's `sagat-infra-packages`).

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

Updated 2026-11-24: Architecture decision 0057 is accepted, with an amendment for the stale items,
delegate (OD-2026-11-24-4); the gate is open. The checks above stand against constitution 4.7.0; the
5.0.0 amendment in progress (Owner decision 2026-11-24-2) does not change principle V. Principle I gains
one row (the lease-time refusal stays inside the lease transaction, Decision 1235) and principle IV one
(fleet activation through the registry, Decision 1234).

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
| `crates/infra-schema/src/provenance.rs` (new) | in-toto statement construction and shape validation. Superseded 2026-11-24 by the row below (Decision 1236) |
| `crates/infra-packages` (design 046's crate, Task T046-15; this feature's additions, 2026-11-24) | the build type `ScriptBuild` of the in-toto statement and SLSA predicate, and the SBOM components of the new languages (toolset inventory, `pkg:cargo`, `pkg:composer`); the DSSE envelope, the CycloneDX 1.5 builder and the package-set comparison are design 046's (Decision 1627) |
| `crates/application/src/attestation.rs` (2026-11-24) | the port the build completion calls, implemented by `sagat-infra-packages`; if design 046 defines that port first, this feature uses design 046's |
| `crates/application/src/instances.rs`, registry CHECK in the migration (2026-11-24) | capability `script-languages-v1` (kinds `CONTROL_PLANE`, `EXECUTION_ENGINE`), the refusals before it is recorded and the refusal to remove it (Decision 1234) |
| `tests/integration/claim-transition-efficiency/work_budget.rs` (2026-11-24) | a script-step shape whose lease costs one more statement; the existing shapes unchanged (Decision 1235) |
| `apps/sandbox-runner/src/lake/config.rs` tests (2026-11-24) | lake mode refuses each new `SAGAT_RUNNER_` variable by name (Decision 1238) |
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
| `crates/infra-postgres/src/scripts.rs`, `migrations/<next>_script_languages.{up,down}.sql` | constraint, policy table, presence table, provenance column, grants, classification manifest entries. Corrected 2026-11-24: provenance **table** `script_build_provenance`, not a column; the registry capability CHECK widened `NOT VALID`; proposed versions `20270323000000_script_languages` and `20270323000100_script_languages_validate` (Gap 1213) |
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

Updated 2026-11-24 (Decisions 1236 and 1627): the statement, the envelope and the SBOM are built and
checked by design 046's crate `sagat-infra-packages` (Task T046-15) through the port
`crates/application/src/attestation.rs`; the DSSE signature is made by `Signer::sign_dsse` in
`crates/application/src/script_signing.rs` over the pre-authentication encoding, so the key never leaves
the application layer. The runner writes its SBOM with the same crate. This feature adds the build type
`ScriptBuild` and the components of its languages, and stores its statements in `script_build_provenance`;
design 046 stores its layer attestations with its layers.

### 8. Runtime policy and discovery

`script_runtime_policies` holds one revisioned document per namespace (data-model.md). Admission reads it
under `FOR SHARE` in the admission transaction; a missing row means the deployment default (O1201: all
built languages enabled, whole catalog). Discovery reads the policy, the embedded catalogs and the
presence rows; presence is upserted by claim handlers at most once per 60 s per runner subject.

Updated 2026-11-24. Fleet activation (Decision 1234): until `script-languages-v1` is recorded in the
instance registry, `createScript` of a new language and a `putScriptRuntimePolicy` that disables a
language or narrows packages are refused `SCRIPT_LANGUAGES_CAPABILITY_DISABLED`; the registry's
activation refuses while a live control plane or engine lacks the capability, and the removal of the
requirement is refused while a new-language script or a stored policy row exists. Extension by design
046 (Decision 1237): the per-language object is closed per release and grows additively; `packages` and
any rule for `typescript` and `python` are reserved for design 046 and refused until it ships; design
046's large rule sets live in rows of its own, so the document stays under 64 KiB. Owner of the document
(Decision 1627): design 046 builds the namespace script policy document, its table and its endpoint (Task
T046-2 and its REST task); this section's rules are what the language section of that document holds, and
`getScriptRuntimePolicy` and `putScriptRuntimePolicy` become that section's shape in design 046's endpoint.
This feature's migration therefore does not create `script_runtime_policies` once design 046's document
exists; it adds what the language section needs to it.

### 8a. Lease-time language check

Superseded in part on 2026-11-24 by Decision 1235: the policy row is read with no row lock, in one
statement with the artifact, for script activities only; the rest of this section stands.

The engine's lease path (`lease_one`) recognizes `script.sha256-<hex>` activities. Inside the lease
transaction, after the existing run and step locks, it reads the artifact language by
`(tenant_id, digest)` (primary key of `script_artifacts`) and the namespace policy row `FOR SHARE`. A
disabled language produces no lease: the step takes the same non-retryable failure transition a worker
report would cause (`SCRIPT_DENIED`, fixed safe message), with its journal event, next scheduled work
and outbox rows, plus a `script.policy_denied` audit row (point `lease`), all in that transaction. Other
activities pay nothing; script activities pay one indexed read (G1221).

As built from 2026-11-24 (Decision 1235): in `lease_one`, once the step to lease is chosen and before the
lease transition runs, a step whose activity starts with `sagat_domain::scripts::ACTIVITY_PREFIX` costs one
statement, `SELECT a.language, p.document -> 'languages' -> a.language FROM script_artifacts a LEFT JOIN
script_runtime_policies p ON (p.tenant_id, p.namespace_id) = (a.tenant_id, $namespace) WHERE (a.tenant_id,
a.digest) = ($tenant, $digest)`, with no row lock; a disabled language runs the step's non-retryable
failure transition (`SCRIPT_DENIED`) in place of the lease, which also releases a flow-control grant or
permit as any failure does (Architecture decisions 0088 and 0089). A probed candidate with nothing to lease
and every non-script lease cost nothing new (Requirements 038-001 and 038-002); the budget test gains a
script-step shape. Design 046's malicious-package block extends this statement (Architecture decision 0121
decision 5) rather than adding one. Owner of the statement (Decision 1627): design 046 builds it first
(Task T046-17), reading the block rows of `script_artifact_blocks` keyed by tenant, artifact digest and namespace
(artifact rows are immutable, so nothing is stored on them; Decision 1628); Task T023-061 adds the language reason to
that same statement by joining the namespace's language section, still one statement with no row lock.

### 8b. Reserved `python-data` runtime (mechanism for feature 024)

`Language::PythonData` (`python-data`) exists with `is_built` false. Its artifact is bundle v4 with
entry `main.py`, `lockDigest` and a non-null `runtimeCatalog`; its lock is a hash-pinned requirements file
validated by `catalog.rs` against a catalog of kind `pypi`; its toolchain identity names the runtime image,
the Python version, the shim and the catalog digest. The runner loads it only when its configured
`python-data` runtime image reports the same catalog digest, and builds an import path of the locked
packages only. Feature 024 supplies the image, the catalog file and the execution environment, and flips
`is_built` with its own evidence; this feature ships the validator branch, the lock parser, the catalog
kind and fixture tests (T023-062).

Amended 2026-11-24 (Decision 1625): this feature builds the reservation (the id, the constraint,
`LANGUAGE_NOT_BUILT` at every admission point, the version 4 schema value); the requirements-lock parser,
the `pypi` catalog kind and the `runtimeCatalog` check at load are built only once feature 024 confirms it
needs an image catalog rather than design 046's dependency layer (Gap 1227, Task 023-071).

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
| Lease of a script step | either the lease (as today) or, for a disabled language, the step failure transition with journal event, next work, outbox and `script.policy_denied` audit | existing run and step locks → artifact row read → policy row `FOR SHARE`. Changed 2026-11-24 (Decision 1235): one statement reading the artifact and the policy row with no row lock, after the run and step locks; the policy row is never locked on the claim path |
| Workflow publication | existing publication records plus the policy check of each script node's artifact language; a refusal commits only its `script.policy_denied` audit | policy row `FOR SHARE` read during validation, before any publication write; existing publication locks unchanged after it |
| Policy update | policy row (revision + 1), `script.policy_changed` audit | policy row `FOR UPDATE` with expected revision; never takes a script or run lock. Since 2026-11-24 a narrowing document first needs `script-languages-v1` recorded, read from the process's registry gate, not from a row (Decision 1234) |
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

Updated 2026-11-24 (Decision 1238): the `log` of a script execution that belongs to a run, of any
language, is read under the step-log rule of design 041 (Requirement 041-022), not `script:read` alone;
a test run's log is unchanged. Recording `script-languages-v1` is an operator act of the instance registry
(Architecture decision 0082), not a tenant permission.

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
| Runtime policy document, corrected 2026-11-24 (Decision 1237: 256 names of up to 214 bytes did not fit 16 KiB) | 64 KiB | 64 KiB, and at most 256 allowed names per language; over either, `ABOVE_CEILING` |
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
4. Fleet activation (added 2026-11-24, Decision 1234): control planes and engines are upgraded first, in
   any order and rolling; an operator then records `script-languages-v1`, which the registry refuses while
   a live process lacks it. For the upgrade from 0.2.x, whose processes predate the registry, the operator
   records it only once no 0.2.x control plane or engine is left (Gap 1230). Runners follow, each with an
   explicit `SAGAT_RUNNER_LANGUAGES`.
5. Rollback is binary-only and forward-only for data (D1230): keep the migrated schema. Before any
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
| Reuse of design 046's crate `sagat-infra-packages` (2026-11-24) | a dependency on Task T046-15 | putting attestation in `sagat-infra-schema` would make design 046 and the runner link a schema engine and a runtime they do not use; a crate of this feature's own beside it would be a second implementation (Decision 1627) |
| Registry capability `script-languages-v1` (2026-11-24) | an operator step at upgrade | a rolling N and N-1 upgrade would otherwise let an older engine lease a disabled language and an older control plane fail listings |

## Delivery sequence

Superseded on 2026-11-24 by "Build slices (2026-11-24)" below, which keeps this order inside two lanes.

1. ADR-0057 acceptance; contracts drafted here merged into canonical files with implementation.
2. Foundation: domain ids, migration, catalog and bundle validators, provenance signing, probes framework.
3. US01 Bash (MVP) → US05 battery for Bash.
4. US02 Rust on ARM64 and AMD64 → US05 battery for Rust.
5. US03 PHP, US04 PowerShell, each with their battery.
6. US06 policy, discovery, provenance UI.
7. US07 spike and report.
8. Faults, performance report, runbook, quickstart, reviews and gates.

## Build slices (2026-11-24)

Decision 1626. Two builders work at the same time without editing the same crate. After one foundation
slice, each lane owns a fixed set of crates, directories and files; the other lane reads them (as a
dependency) but never edits them while the lane is open. A language is finished by a join slice once both
lanes have delivered its parts. Every slice is one pull request into `develop`, documentation first; each
names its tasks, and a task is ticked only with the evidence `tasks.md` names. Each builder uses a private
`CARGO_TARGET_DIR` outside the repository; at most two heavy builds run at once on the shared machine.

### Slice 0: foundation (one builder, blocks both lanes)

- Tasks: T023-002 (test targets), T023-003 (fixtures), T023-004 (language ids, `is_built` false for all
  four and for `python-data`), T023-005 (policy values, permission, audit actions), and the part of
  T023-008 that lets bundle version 1 carry `bash` and `powershell` and version 3 carry `rust` (no
  version 4 yet).
- Edits: `crates/domain/src/{scripts,identity,audit}.rs`; `crates/infra-schema/src/bundle.rs` (language
  acceptance only); `tests/Cargo.toml` and `tests/contract/script-languages/fixtures/`; and, only so the
  workspace compiles, the arms that every `match` over `Language` needs elsewhere, each refusing the new
  languages (`LANGUAGE_NOT_BUILT` at admission, build kind `unsupported` in the runner). No migration, no
  image, no contract change.
- Evidence: `cargo test -p sagat-domain -p sagat-infra-schema --locked`, the existing bundle suites
  unchanged.

### Lane A: control plane, data, contracts, Studio and SDKs

Owns `sagat-domain`, `sagat-application`, `sagat-infra-schema`, its additions to design 046's
`sagat-infra-packages` (after Task T046-15 has merged),
`sagat-infra-postgres` (with `crates/infra-postgres/migrations/` and `storage-classification.json`),
`sagat-api-http`, the composition roots `apps/control-plane` and `apps/execution-engine`,
`specs/001-platform-foundation/contracts/{openapi.yaml,workflow.schema.json}`, `frontend/apps/studio`,
`sdks/typescript`, `sdks/python`, `tests/contract/sdk-conformance/scenarios/`,
`tests/contract/script-languages/`, `tests/integration/claim-transition-efficiency/work_budget.rs` and
`tests/integration/script-languages/` except its `containers/` directory.

| Slice | Tasks | Crates and files | Needs |
|---|---|---|---|
| A1 Data and fleet activation | T023-006, T023-067, T023-011 | migrations `20270323000000_script_languages` and `20270323000100_script_languages_validate` (proposed, Gap 1213); `crates/infra-postgres/src/scripts.rs`; `crates/application/src/{scripts,instances}.rs`; `crates/api-http/src/error.rs` | slice 0; for the language section of the policy, Task T046-2 (design 046's policy document and its migration) |
| A2 Attestation and artifact formats | T023-068, T023-009, T023-047, T023-007, the version 4 part of T023-008 | additions to `crates/infra-packages/`; `crates/application/src/{attestation,script_signing,scripts}.rs`; `crates/infra-schema/src/{catalog,bundle}.rs`; `runners/{rust,php}/catalog/catalog.json` (catalog data only, files the runner lane does not edit) | A1 (the provenance table); Task T046-15 (the provenance and SBOM builders) |
| A3 Policy API, discovery, workflow publication | T023-044, T023-013, T023-045 | `crates/api-http/src/routes/{scripts,runner}.rs`; `crates/application/src/{scripts,definitions}.rs` | A1; for T023-044, design 046's policy endpoint (Task T046-2 and its REST task) |
| A4 Lease-time check | T023-061 (with the budget shape of Decision 1235) | `crates/application/src/worker.rs` (`lease_one`); `crates/infra-postgres/src/workers.rs`; `work_budget.rs` | A1; Task T046-17 (design 046's lease statement, which this slice extends). Its tests insert artifact rows of a new language directly, so it needs no flipped `is_built` |
| A5 Reservation of `python-data` | the reservation part of T023-062 | `crates/domain/src/scripts.rs`, admission refusals | A1 |
| A6 Contracts, SDKs, Studio | T023-051, T023-050, T023-019, the Studio parts of T023-027, T023-038, T023-042, and T023-046 | contracts of feature 001; `sdks/{typescript,python}`; the conformance scenario (all six SDKs run it); `frontend/apps/studio/src/features/scripts/` | A3 |
| A7 Control-plane cross-cutting | T023-053 (control-plane ceilings), T023-054, T023-055 (control-plane part), T023-063 | `tests/integration/script-languages/{limits,telemetry,rollback}.rs`; `tests/contract/security-multitenancy/rest/battery.rs` | A3, A4 |

### Lane B: runner, images and native CI

Owns `sagat-infra-sandbox` (with `crates/infra-sandbox/shims/`), `sagat-sandbox-runner`,
`runners/{shell,powershell,php,rust,native,go,ts-variants}` except the two catalog files of slice A2,
`runners/{dotnet,java}` trusted APIs for the reserved host calls, the `native-runners` job of
`.github/workflows/ci.yml`, the `sandbox` profile of `infra/docker/compose.yaml`, `infra/helm`,
`tests/integration/script-languages/containers/` and `tests/performance/script-languages/`.

| Slice | Tasks | Crates, images and files | Needs |
|---|---|---|---|
| B1 Runner framework | T023-010, T023-069, the runner part of T023-012, T023-014 | `apps/sandbox-runner/src/{config,lib}.rs`, `apps/sandbox-runner/src/lake/config.rs` (tests); `crates/infra-sandbox/src/docker.rs` (probe framework); `runners/native/runtime.py`, `runners/go/Dockerfile.runtime`; the `native-runners` job (Go proofs on both architectures) | slice 0 |
| B2 Bash | T023-016, T023-017 | `runners/shell/` (image, host, helper with its own `Cargo.lock`); `docker.rs`, `lockfile.rs` (empty lock only) | B1 |
| B3 PowerShell | T023-040, T023-041 | `runners/powershell/`; `docker.rs` | B1 |
| B4 Rust | T023-022, T023-023 (image side), T023-024, T023-025 | `runners/rust/{sagat-inline,buildhost,Dockerfile.builder,Dockerfile.runtime}`; `docker.rs`, `lockfile.rs` delegating to `sagat_infra_schema::catalog`; the `native-runners` job | B1 (T023-014), A2 (catalog) |
| B5 PHP | T023-035 (image side), T023-036 | `runners/php/` except `catalog/catalog.json`; `docker.rs` (version 4 at load) | B1, A2 (version 4, catalog) |
| B6 SBOM on the shared crate | the runner part of T023-068 | `docker.rs` build completion writes its SBOM through design 046's SBOM builder in `sagat-infra-packages` | A2; Task T046-15 |
| B7 Reserved host calls | T023-064, T023-065 | `crates/infra-sandbox/src/protocol.rs`, `crates/infra-sandbox/shims/runtime.{mjs,py}`, `apps/sandbox-runner/src/host.rs`, `runners/{dotnet,go,java}` trusted APIs, the four new shims | B1; each new shim after its language slice |
| B8 Hostile battery | T023-029, T023-030, T023-031 | `tests/integration/script-languages/containers/{hostile,rust}.rs` | per language: B2, B3, B4, B5 |
| B9 Spike and deployment | T023-048, T023-049, T023-057 | `runners/ts-variants/`, `tests/performance/script-languages/ts_variants.rs`; compose and Helm | B1 |

### Join slices (either builder, once both inputs have merged)

A join slice edits `sagat-domain` (it flips `is_built`) and adds end-to-end suites, so it is taken by the
lane A builder, or by the lane B builder only while no lane A slice that edits `sagat-domain` is open.

| Slice | Tasks | Needs |
|---|---|---|
| J1 Bash end to end | T023-015, T023-018, T023-020, the Bash part of T023-032 | A1, A3, B2 |
| J2 Rust end to end | T023-021, T023-026, T023-028 | A2, B4 on both architectures |
| J3 PHP end to end | T023-034, T023-037 | A2, B5 |
| J4 PowerShell end to end | T023-039, T023-042 (flip) | A1, B3 |
| J5 Policy end to end | T023-043 | A3, A4, J1 |
| J6 Close | T023-052, T023-056, T023-058, T023-033, T023-059, T023-060 | every slice above |
| Later | T023-066 (needs the checkpoint minor of feature 019 and `AppendStreamEvents` of feature 026), T023-070 (needs feature 019's warm residue suite), T023-071 (only if feature 024 confirms the image catalog, Decision 1625) | outside this feature |

### Order and dependencies outside this feature

1. Slice 0, then lanes A and B in parallel: A1 → {A2, A3, A4, A5} → A6 → A7, and B1 → {B2, B3} → {B4, B5
   once A2 has merged} → B6 → B7, B8, B9. The minimum viable product is slice 0 + A1 + A3 + B1 + B2 + J1.
2. Design 046 (Decision 1627, the coordinator's boundary): design 046 builds first, in its own slice 1,
   the namespace script policy document and its migration (Task T046-2), the provenance and SBOM builders
   in `sagat-infra-packages` (Task T046-15) and the lease statement on `script_artifact_blocks`
   (Task T046-17, Decision 1628). Slice A1 (the language section), A2, A3 (T023-044), A4 and B6 wait for the
   corresponding task to merge; slice 0, B1 to B5, B7 to B9 and the rest of A1 and A3 do not. Its bundle
   version 5 and this feature's version 4 both edit `bundle.rs`; the later of the two rebases, keeping both
   numbers.
3. Feature 019: target-aware routing would lift the one-target rule of Rust pools (Gap 1201); its
   checkpoint minor unblocks the checkpoint half of T023-066; its warm residue suite unblocks T023-070.
   Feature 026 unblocks the stream half of T023-066. None blocks slices 0 to J6.
4. Shared files: the registry CHECK is widened by other features too (feature 022's flow-control
   capabilities on its branch); whichever merges later restates the whole set in its `NOT VALID` CHECK and
   its validating migration. The budget test of design 038 is moved by A4 and by feature 022's claim
   shapes; the later of the two rebases the budget with its reason.

## Verification plan

Every task names its evidence. Required before release: real-container batteries per language and per
target; native AMD64 and ARM64 CI jobs; v1/v2/v3 regression unchanged; REST contract suite over the
merged OpenAPI; five-SDK conformance; real-browser Studio flows; fault suite of the matrix above;
architecture check; independent security and correctness reviews; performance report with environment
and failures. A skipped or failing test is reported as such.
