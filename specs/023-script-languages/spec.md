# Feature Specification: Additional script languages

**Feature Branch**: `feature/023-script-languages`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: Add Bash/POSIX shell, PowerShell, PHP and Rust script runtimes, and evaluate Deno and Bun as
TypeScript runtime variants, in the feature-005 model: isolated per-language builders, pinned toolchain
images, lockfiles (`composer.lock`, `Cargo.lock`), SBOM and provenance, additive artifact formats,
runner startup probes and advertised capabilities, the unchanged sandbox profile (no network, read-only
root, `noexec` scratch; compiled Rust follows the Go sealed-memfd execution of FR-005-010), the
supervisor channel for HTTP and secret requests (a shim per language), typed input/output schemas and
forms (FR-005-006), dependency policy, limits, Monaco language support and test runs, with
shell-specific risks stated honestly.

## Context and delivery boundary

Feature 005 lets an author write TypeScript, Python, C#, Go or Java in the platform, try it, publish an
immutable signed artifact and pin it from a workflow; a separate runner (`apps/sandbox-runner`) builds
and runs each artifact in its own container and never touches the database (ADR-0006, ADR-0013,
ADR-0021, ADR-0024). Operators and integrators also write shell, PowerShell, PHP and Rust. This feature
adds those four languages **without a second execution model**: same registry, same versions and build
jobs, same signing, same lease and fencing, same container profile, same host-call channel. What is new
per language is a pinned builder and runtime image, a trusted in-container shim, a startup probe, a
validator branch for its artifact and, for Rust and PHP, a closed offline package catalog. Two
mechanisms are new for every language: a signed build provenance statement and a namespace policy that
narrows which languages and packages may be used.

Deno and Bun are evaluated as TypeScript runtime variants through a bounded spike; this feature enables
neither (research D1201).

**Ownership boundaries.** Warm or dedicated script workers, worker deployment versions and
target-aware capability routing belong to feature 019. Test-mode pinned outputs and partial execution
of drafts belong to 022. Generic resources and variables belong to 021. Encrypted payload codecs belong
to 031; this feature only states how an encrypted field behaves in a script form and step. The template
hub (028) may later import starters; it is not required here.

The [proposed ADR-0057](../../docs/adr/ADR-0057-additional-script-languages-reuse-the-sandbox-and-signed-artifacts.md)
records the hard-to-reverse choices: artifact families per language, bundle v4, closed offline catalogs,
non-PIE static Rust through the sealed-memory loader, signed provenance and the decision not to adopt
Deno or Bun.

## User Scenarios & Testing *(mandatory)*

### US01 — Write, test and publish a Bash script (Priority: P1)

An author (role `developer`) writes a Bash script for an operations task in the Studio, tries it against
an input, publishes it and uses it as a workflow step, without building or deploying anything.

**Why this priority**: it is the smallest vertical slice that crosses every new shared mechanism (a new
language id in the contracts and the database, a v1 artifact of a new language, a shim that is not a
library import, the startup probe, the runtime policy, provenance and Monaco) with the least toolchain.

**Independent Test**: on a runner whose `SAGAT_RUNNER_LANGUAGES` includes `bash`, create, test, publish
and run a Bash script through the public API, the worker protocol and real containers.

**Acceptance Scenarios**:

1. **Given** an author with `script:write` in namespace A and a runner that advertises `bash`, **When**
   they create a `bash` script from the starter and start a test run with `{"n": 21}`, **Then** the test
   run returns the JSON the script recorded with `octopus result`, the redacted log of its stdout and
   stderr, and schemas inferred from the input and the result; the test run had no secret.
2. **Given** that draft, **When** the author publishes it, **Then** the version becomes `READY` with a
   signed v1 artifact of language `bash`, a CycloneDX SBOM that lists the runtime image's toolset, and a
   signed provenance statement whose subject is the artifact digest; a workflow that pins the digest runs
   it as a step and the result is validated against the declared output schema.
3. **Given** a published script with egress `namespace`, **When** it runs `octopus fetch` against a host
   the namespace's egress policy allows and then against one it does not, **Then** the first request is
   made by the supervisor and returned, the second is refused, the step sees a non-zero exit of the
   helper, and the refusal `fetch https://<host>` is audited once.

**Failure scenarios**: a syntax error fails the build as `SCRIPT_BUILD_REJECTED` with the first message
of `bash -n`; a script that exits 0 without `octopus result` fails as `SCRIPT_NO_RESULT`; a failing
command under the default `errexit` fails as `SCRIPT_ERROR`; a non-empty lockfile fails the build with
kind `lockfile`; a runner without `bash` in its allowlist never advertises it.

---

### US02 — Compile and run a Rust script natively (Priority: P1)

A developer writes a Rust function that uses `serde_json`, publishes it and runs it as a native step.

**Why this priority**: Rust is the only new compiled language. It proves the closed crate catalog, the
non-PIE static output, the reuse of the sealed-memory loader and the per-target capability gate, which
are the riskiest parts of the feature.

**Independent Test**: on a native runner of one configured target, build a Rust script in the real
isolated builder, validate and sign its v3 artifact, then execute it through the production loader with
mediated host calls; repeat on the other architecture's native CI runner.

**Acceptance Scenarios**:

1. **Given** a runner configured with `SAGAT_RUNNER_RUST_TARGET=linux-arm64` whose Rust probe passed,
   **When** the author publishes a `rust` script whose `Cargo.lock` was produced by the catalog API for
   `serde_json`, **Then** `rustc` runs offline with fixed arguments in the builder container, the
   output is a static `ET_EXEC` ELF for `linux-arm64` that passes the existing validator, the v3 artifact
   is signed, and the runtime executes it from a four-seal memory file with the input on sealed fd 3.
2. **Given** that version, **When** a step of a workflow runs it on a runner of the same target, **Then**
   the function receives the step document, its `octopus.fetch`, `octopus.secret` and
   `octopus.idempotency_key()` calls cross the supervisor channel, and its JSON result completes the step.
3. **Given** a lock naming a crate outside the catalog, a changed checksum, a missing dependency of the
   closure, an extra root package, or source containing an `unsafe` block, **When** the author publishes,
   **Then** the lock problems are refused at publication with `422` reason `LOCKFILE_INVALID` or
   `PACKAGE_NOT_ALLOWED`, and anything that reaches a builder (the `unsafe` block, or a lock checked only
   against the image catalog) fails before a binary is signed, with kind `rejected` or `lockfile` and no
   network access.

**Failure scenarios**: an artifact whose signed target differs from the runner's fails the step as
`SCRIPT_SANDBOX_UNAVAILABLE` and is retried by the engine; a failed optional Rust probe omits Rust from
the advertisement; a deployment that requires Rust refuses to start; a configured Rust target that
differs from the Go target refuses to start.

---

### US03 — Run a PHP script with a locked Composer package (Priority: P2)

An integrator writes a PHP function that uses an approved Composer package.

**Why this priority**: PHP needs the only new artifact format (the v4 tree) and the Composer catalog,
but its isolation is the same as Python's.

**Independent Test**: build a PHP script whose `composer.lock` names one catalog package, verify the
signed v4 bundle carries the package tree and the generated class-map autoloader, and run it.

**Acceptance Scenarios**:

1. **Given** a `php` script defining `run(array $input, Octopus $octopus): mixed` and a `composer.lock`
   whose single package matches the catalog on name, version, dist type and dist reference, **When** the
   author publishes it, **Then** the builder copies the package tree, runs only `composer dump-autoload
   --classmap-authoritative --no-dev --no-scripts --no-plugins`, and the signed v4 artifact holds
   `main.php`, `vendor/autoload.php` and the package files; the SBOM lists the package with its purl.
2. **Given** that version, **When** it runs, **Then** `echo` output is in the log, the return value is
   the result, `file_get_contents('https://…')` fails because `allow_url_fopen` is off and there is no
   network, and `$octopus->fetch(...)` is mediated.
3. **Given** a lock with `packages-dev` entries, a `composer-plugin` package, an unknown package or a
   changed dist reference, **When** the author publishes, **Then** it is refused at publication with
   `LOCKFILE_INVALID` or `PACKAGE_NOT_ALLOWED`, and the builder's own check refuses it before signing if
   the control-plane check is bypassed.

**Failure scenarios**: a parse error fails as `SCRIPT_BUILD_REJECTED` with the first line of `php -l`;
a tree above 1,024 files or 1,400 KiB fails with kind `limit`; memory exhaustion is
`SCRIPT_OUT_OF_MEMORY` reported by the cgroup.

---

### US04 — Run a PowerShell script (Priority: P2)

An author who administers Windows-adjacent systems writes a PowerShell script (run on Linux) that
transforms an input and calls an allowed API.

**Why this priority**: PowerShell is framework-only and interpreted; its value is real but it needs no
new artifact format or catalog.

**Independent Test**: test, publish and run a PowerShell script through the real runtime image.

**Acceptance Scenarios**:

1. **Given** a `powershell` script defining `function Run { param($InputObject, $Octopus) … }`, **When**
   the author starts a test run, **Then** `pwsh` starts with `-NoProfile -NonInteractive -NoLogo` and
   telemetry opted out, `Run`'s single returned object is the result, and `Write-Host`, warning, verbose
   and error streams are the log.
2. **Given** a published version, **When** it calls `Invoke-OctopusFetch` and `Get-OctopusSecret` for a
   declared secret, **Then** both are mediated by the supervisor and the secret value is redacted from the
   log and the result.
3. **Given** a script that returns an object nested deeper than 64 levels, **When** it runs, **Then** the
   step fails as `SCRIPT_ERROR` with a fixed message instead of a silently truncated result.

**Failure scenarios**: a non-empty lockfile (a Gallery module request) is refused; `Invoke-WebRequest`
fails because there is no network; a parse error is `SCRIPT_BUILD_REJECTED` with the parser's first error.

---

### US05 — Contain hostile scripts in every new language (Priority: P1)

A security administrator needs proof that a hostile script in any of the four languages is held to the
same limits as in feature 005.

**Why this priority**: a sandbox escape, unauthorized execution, secret exposure or quota bypass is
release-blocking (constitution V).

**Independent Test**: run the hostile-script battery in real containers for each language on each
enabled native target.

**Acceptance Scenarios**:

1. **Given** scripts in each language that reach for the cloud metadata address, the container engine's
   socket, host files, a direct socket (including Bash `/dev/tcp`), an undeclared secret or the runner's
   environment, **When** they run, **Then** each attempt fails, refused host calls are audited once per
   actor, resource and reason, and no container is left behind.
2. **Given** a fork bomb, a memory bomb, a CPU burner, a disk flood, a log flood, a channel flood and a
   script that never reads its replies, **When** they run, **Then** each ends at its limit with the
   005 failure code and the runner slot is released.
3. **Given** a Bash script started with hostile environment names in its input (`BASH_ENV`, `ENV`,
   `BASH_FUNC_x%%`, `LD_PRELOAD`), **When** it runs, **Then** none reaches the interpreter, because the
   input is a JSON file and the environment is a fixed allowlist.

---

### US06 — Govern languages and packages per namespace and inspect provenance (Priority: P2)

A namespace administrator decides which languages and catalog packages a namespace may use, and an
auditor checks how an artifact was built.

**Why this priority**: production namespaces need to narrow what authors may publish; without it every
built language is available everywhere a runner serves it.

**Independent Test**: change the policy, observe refusals at each admission point, and verify a
provenance statement offline with the control plane's public key.

**Acceptance Scenarios**:

1. **Given** an admin with `script:policy:manage`, **When** they disable `bash` in namespace `prod` with
   the current policy revision, **Then** creating a Bash script (a draft's language is fixed at
   creation), publishing a Bash version and starting a Bash test run are refused with
   `VALIDATION_FAILED` reason `LANGUAGE_DISABLED`, publishing a workflow whose script node pins a Bash
   artifact is refused with the publication diagnostic `SCRIPT_LANGUAGE_DISABLED`, each refusal is
   audited, and a run admitted before the change fails its next Bash step attempt at lease with
   non-retryable `SCRIPT_DENIED` and a `script.policy_denied` audit record (point `lease`), while an
   attempt already leased completes.
2. **Given** a policy restricting `rust` to a subset of the catalog, **When** a lock names a crate outside
   it, **Then** publication is refused with reason `PACKAGE_NOT_ALLOWED`; a policy that names a package
   absent from the deployment catalog is itself refused.
3. **Given** a `READY` version built after this feature, **When** an auditor with `script:read` fetches
   its provenance, **Then** the DSSE envelope verifies with the published key, its subject is the
   artifact digest, and the predicate separates fields the control plane verified from fields the runner
   reported.

**Failure scenarios**: a policy update with a stale revision is `409 CONFLICT`; another tenant's
namespace is an opaque `404`; an author without `script:policy:manage` gets `403`.

---

### US07 — Evaluate Deno and Bun as TypeScript runtime variants (Priority: P3)

The platform owner wants evidence before deciding whether TypeScript should run on another engine.

**Why this priority**: it produces a decision record, not a capability.

**Independent Test**: run the TypeScript conformance fixtures and the start/memory benchmark on Node,
Deno and Bun images under the unchanged container profile, outside product code.

**Acceptance Scenarios**:

1. **Given** spike images built from `runners/ts-variants/`, **When** the harness runs the existing
   TypeScript fixtures through `DockerSandbox` with those images, **Then** the report records, per
   runtime, whether each fixture produced the identical result and failure code, and cold/warm start and
   peak memory on a stated environment.
2. **Given** the report, **When** it is reviewed, **Then** no product configuration, contract or runner
   setting can select Deno or Bun, and adoption is possible only through a new ADR.

### Edge Cases

- A runner upgraded to this release with no change to `SAGAT_RUNNER_LANGUAGES` serves exactly the five
  005 languages.
- A version of a language no runner serves stays `BUILDING`; Studio shows that no runner advertises it
  (G1212). A test run of such a language fails `SCRIPT_SANDBOX_UNAVAILABLE` at its two-minute deadline.
- Two runners of one tenant on different architectures both configured for Rust: misconfiguration;
  artifacts built on one fail as unavailable on the other (G1201); the runbook forbids it.
- A Rust image tag that moved between the claim and the run: the toolchain identity is recomputed at
  every claim (005 G116); a tag-only image fails the probe outside development.
- A catalog image rebuilt with a new catalog: its digest changes the toolchain identity, so the same
  source builds again; old artifacts keep running because they carry their dependencies.
- A Rust source that uses `include_str!` on a builder file: it reads the builder image, which holds no
  secret, token or platform state by construction (FR-023-002).
- A PHP script that writes a PHP file to `/tmp` and includes it: allowed, its own code in its own sandbox.
- A PowerShell `Add-Type` compiling C#: allowed inside the runtime container (G1215).
- A Bash script that starts a background process and exits: the run ends when the host returns; the
  container is removed with the process.
- A lockfile valid for Rust submitted on a PHP script: refused with reason `LOCKFILE_INVALID`.
- The language is disabled while a step of that language is ready: the next lease fails it; while an
  attempt is already running: that attempt finishes, and a retry is refused at its lease.
- The namespace policy is changed while a publication is in flight: the publication sees the policy row
  it read under its share lock; the edit waits or the publication waits, never a mixed view.
- An encrypted field (feature 031) in a test-run form: entered as an opaque value; never decrypted.
- A script calls `checkpoint` or `stream.append` in a test run, or on a lease that does not grant it:
  `NOT_GRANTED`, the script decides what to do, and the step is not failed (G1222).
- A script appends an event containing a secret it was handed: refused before it leaves the runner, and the
  server's own scan would refuse it too; nothing is stored.
- A field classified `secret` in the input schema: never prefilled from a previous run nor kept in
  browser storage.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-023-001 — Language ids**: The platform MUST add the script languages `bash`, `powershell`, `php` and
  `rust` additively to the domain language set, the database constraint, the OpenAPI script schemas and
  the workflow schema's informational `script.language` enum (additive IR v1, ADR-0032). Existing
  values, artifacts and versions MUST keep their meaning. A language the platform has no builder and
  runtime for MUST be refused with `details.reason` `LANGUAGE_NOT_BUILT`. POSIX `sh` scripts are served
  by `bash`; there is no `sh` id. The id `python-data` is reserved for the image-catalog runtime of
  FR-023-027 and is refused `LANGUAGE_NOT_BUILT` until feature 024 ships it.
- **FR-023-002 — Isolated per-language builders**: Each new language MUST build in its own pinned builder
  container started by a runner, never in the control plane, scheduler or engine. The builder MUST own
  the project: the author supplies source and an optional lockfile only; no manifest, build script,
  compiler or linker option, environment variable, Composer script, plugin or Gallery module of the
  author is executed or read. Builder images MUST hold no token, key, secret or platform state.
- **FR-023-003 — Pinned toolchains**: Builder and runtime images MUST be referenced by digest in
  production profiles; the toolchain identity MUST name the build and runtime image ids, the interpreter
  or compiler version, the shim or host hash, the native target and ABI where applicable, and the
  catalog digest where applicable. The identity MUST participate in the build key and be recomputed at
  every claim, so a changed image or catalog builds again.
- **FR-023-004 — Lockfiles**: Bash and PowerShell MUST refuse a non-empty lockfile. Rust MUST accept only a
  Cargo.lock (format version 4) whose root is the fixed `sagat-script 0.0.0` and whose packages equal
  catalog entries on name, version, source and checksum with a complete closure. PHP MUST accept only a
  `composer.lock` whose `packages` equal catalog entries on name, version, dist type and dist reference,
  with empty `packages-dev`. A customer build or run MUST NOT access a package registry.
- **FR-023-005 — Closed offline catalogs**: Each catalog entry MUST record its identity, version,
  upstream checksum or reference, an independently verified archive SHA-256, its SPDX licence, whether it
  runs code at build time (Rust build script or proc-macro), and the advisory audit evidence of the image
  build. Composer plugins and packages needing extensions outside the runtime's closed set MUST NOT be
  admitted. Adding an entry MUST require review and a new image digest; the audit is evidence at image
  build time, not a permanent claim.
- **FR-023-006 — SBOM**: Every artifact of a new language MUST carry a CycloneDX 1.5 SBOM listing the
  platform component (language, toolchain identity, target and ABI), every locked package with its purl
  and hashes, and, for interpreted runtimes, the runtime image's toolset or extension inventory. The
  control plane MUST refuse to sign a build whose SBOM package set differs from the version's lockfile
  (`BUNDLE_MISMATCH`).
- **FR-023-007 — Provenance**: Every artifact built after this feature, in any language, MUST have an
  in-toto Statement v1 with an SLSA Provenance v1 predicate in a DSSE envelope signed by the control-plane
  key, whose subject is the artifact digest and whose predicate separates verified from runner-reported
  fields. No SLSA build level MUST be claimed. The statement MUST be stored immutably **per build**,
  keyed by tenant and build key (not on the content-addressed artifact row), so the same artifact digest
  produced by builds with different toolchains has one statement per build and a version finished from
  an identical build key shares that build's statement; it MUST be retrievable with `script:read`
  through the version.
- **FR-023-008 — Additive artifact formats**: Bash and PowerShell MUST use bundle v1 text (entries
  `main.sh`, `main.ps1`); Rust MUST use bundle v3 (`target` `linux-amd64|linux-arm64`, ABI
  `sagat-inline-v1`, entry `script.elf`, encoding `base64+gzip`); PHP (and the reserved `python-data`, FR-023-027) MUST
  use the new bundle v4 tree with the bounds of FR-023-019. Validation MUST happen before signing and again at the runner's load; stored
  v1, v2 and v3 artifacts and their verification MUST be unchanged.
- **FR-023-009 — Native Rust execution**: A Rust artifact MUST be a static, non-position-independent
  executable for one signed Linux target with no interpreter, no writable-executable segment and a
  non-executable stack, executed from a four-seal memory file with input on sealed fd 3 and host
  replies on standard input, exactly as FR-005-010. The author crate MUST compile under
  `forbid(unsafe_code)` with fixed flags and only prebuilt read-only catalog libraries.
- **FR-023-010 — Unchanged sandbox profile**: Builds and runs of every new language MUST use the 005
  container profile without relaxation: network `none`, read-only root, `/tmp` mounted
  `noexec,nosuid,nodev` and bounded, all capabilities dropped, `no-new-privileges`, default seccomp,
  user 65534, private PID and IPC, memory with swap off, CPU quota and CPU-time limit, process, file
  size and open-file limits, no core dumps, no log driver, no mounts. A language that needs a relaxation
  MUST fail its probe instead.
- **FR-023-011 — Supervisor channel per language**: Each language MUST have a trusted shim (Bash helper and
  host, PowerShell module, PHP class, Rust crate) that speaks the existing JSON-line protocol (`ready`,
  `fetch`, `secret`, `result`, `error`) under the existing bounds: 240 KiB lines, 100 host calls, 400
  channel lines, refusals recorded once and cut to 200 characters. Each MUST expose the step's
  idempotency key and the two reserved host calls of FR-023-028 (whose separate budgets raise the channel
  line ceiling to 2,000). The shims MUST NOT be treated as a security boundary; every bound is enforced by the
  runner-side supervisor.
- **FR-023-012 — Bash runtime contract**: Bash MUST run with `--noprofile --norc`, `errexit`, `nounset`
  and `pipefail` on, a fixed environment allowlist (no `BASH_ENV`, `ENV`, `SHELLOPTS` or imported
  functions), the input as a JSON file, stdout and stderr as the log, and the result only from the first
  `octopus result`. The runtime image MUST contain only the curated toolset, no network or package tool,
  and no setuid or setgid file.
- **FR-023-013 — PowerShell runtime contract**: PowerShell MUST run the pinned LTS line with
  `-NoProfile -NonInteractive -NoLogo`, telemetry and update checks off, `PSModulePath` limited to the
  image, a `Run` function whose output is the result (none `null`, one the object, several an array),
  serialization bounded to depth 64 that fails instead of truncating, and no third-party module.
- **FR-023-014 — PHP runtime contract**: PHP MUST run with `-n` and the platform ini only
  (`allow_url_fopen=0`, `allow_url_include=0`, `phar.readonly=1`, errors to stderr, no execution-time or
  memory limit of its own), a closed extension set named in the toolchain identity, a `run(array $input,
  Octopus $octopus): mixed` entry, output buffering into the log, and a runtime image without the catalog.
- **FR-023-015 — Probes and advertised capabilities**: A runner MUST advertise a new language only if it is
  in `SAGAT_RUNNER_LANGUAGES` and its bundled probe built and ran through the production images,
  profile and protocol, including one host call. The default allowlist MUST be the five 005 languages.
  A failed probe of a language in `SAGAT_RUNNER_REQUIRED_LANGUAGES` MUST fail startup and readiness.
  Outside development, a tag-only image MUST fail the probe. Rust MUST have exactly one configured native
  target, equal to the Go target when both are set, and each target is enabled only by its own native
  proof.
- **FR-023-016 — Typed schemas and forms**: FR-005-006 MUST apply unchanged to the new languages: declared
  input and output JSON Schemas validated by the supervisor before and after execution, inferred schemas
  from test runs, and a Studio test form generated from the input schema. The `script` node's optional
  `outputSchema` remains the classification carrier of FR-016-002 (ADR-0033). Fields classified `secret`
  (feature 016) MUST NOT be prefilled or kept in browser storage; a field declared encrypted by feature
  031 MUST be handled as an opaque value and never decrypted by the platform.
- **FR-023-017 — Namespace script runtime policy**: An admin MUST be able to enable or disable each built
  language per namespace and restrict a language to a subset of the deployment catalog, with an
  optimistic revision. The language policy MUST be checked at five points: draft creation (a draft's
  language never changes), publication, test-run admission, workflow publication, and the lease of every
  script step attempt. At lease, the engine MUST NOT lease a step whose artifact language is disabled: it
  fails the step in the lease transaction as non-retryable `SCRIPT_DENIED` and writes a
  `script.policy_denied` audit record (point `lease`); an attempt already leased finishes. Package
  narrowing applies at the first four points. The policy MUST NOT widen the deployment catalog.
- **FR-023-018 — Discovery**: The API MUST list, per namespace, each language with whether the platform
  builds it, whether a runner of the tenant advertised it within 120 s (advisory), its toolchain summary
  and native targets, and MUST list the catalog of a language with the exact lock text for a selection.
- **FR-023-019 — Limits**: The finite ceilings of `plan.md` MUST apply: per-language build profiles, the
  PowerShell runtime default, per-runner native build concurrency, locked-package counts, v4 tree bounds,
  policy document size and presence rows. Effective values MUST be the minimum of request, namespace and
  deployment; a request above a ceiling MUST be refused `ABOVE_CEILING`; zero MUST NOT mean unlimited;
  existing `scripts` and `script_jobs` quotas and the ADR-0027 content, log and retention quotas MUST apply.
- **FR-023-020 — Durable semantics unchanged**: Builds, test runs and steps of the new languages MUST use
  the existing build and test leases, worker protocol leases, fencing, heartbeat, cancellation, retry
  and typed failures. Build completion MUST commit artifact, signature, SBOM, provenance, version state,
  job state and audit in one transaction (ADR-0002). A step MUST run at least once and receive the same
  idempotency key on every attempt; nothing is promised exactly once (ADR-0004).
- **FR-023-021 — Tenant boundary and authorization**: Every new row MUST be tenant- and namespace-keyed with
  composite foreign keys; another tenant's script, version, policy, provenance or presence MUST be an
  opaque `NOT_FOUND`, and the probe audited. New permission `script:policy:manage` MUST be granted to
  `admin` only; discovery and provenance need `script:read`. A runner serves one tenant.
- **FR-023-022 — Observability and audit**: The feature MUST emit the spans and metrics of `plan.md` (step
  attempt spans stay journal-derived, ADR-0017) with a
  closed `language` label and no tenant, script, digest, package or run identifier as a label; logs MUST
  carry no source, input, result, lock content or secret. Policy changes and policy denials MUST be
  audited in the transaction of the decision; refusals inside a run follow `script.violation`.
- **FR-023-023 — Studio**: The Studio MUST highlight Bash, PowerShell, PHP and Rust with Monaco's
  definitions, offer a starter per language, drive the enabled languages from the discovery API with the
  reason when disabled, offer a catalog picker that writes the canonical lockfile, and show SBOM and
  provenance of a version, with design tokens and WCAG AA. It MUST NOT draw a language or package as
  available without data (FR-007-010).
- **FR-023-024 — Five SDKs**: The `script` node builders of TypeScript, Go, Python, C# and Java MUST accept
  the four new language ids and emit byte-identical canonical IR in a shared conformance scenario.
- **FR-023-025 — Deno and Bun evaluation**: The feature MUST produce a report of Deno and Bun against the
  TypeScript fixtures and the start/memory benchmark under the unchanged profile, and MUST NOT add any
  product path that selects them.
- **FR-023-026 — Delivery**: Images MUST be reproducible from `runners/`, built and proven natively on
  AMD64 and ARM64 in CI; the scripts runbook, the performance report and the quickstart demo of a happy
  and a failure path MUST be updated in the same delivery.
- **FR-023-027 — Reserved `python-data` image-catalog runtime (mechanism only)**: The platform MUST reserve
  the language id `python-data` for a Python runtime whose packages are too large for the artifact and
  live in a pinned runtime image. Its artifact MUST be bundle v4 with entry `main.py`, `lockDigest`
  and a non-null `runtimeCatalog` naming the image catalog digest; its lock MUST be a hash-pinned
  requirements file matching catalog entries exactly; its catalog MUST follow
  `contracts/catalog.schema.json` and its digest MUST be in the toolchain identity and the build key; a
  runner MUST refuse to run an artifact whose `runtimeCatalog` differs from its configured runtime's
  catalog digest (`SCRIPT_SANDBOX_UNAVAILABLE`) and MUST put only locked packages on the import path.
  Probes, allowlist, digest pinning, SBOM, provenance and namespace policy apply as to every new language.
  Feature 024 owns the image, the package set, the execution environment and their evidence; until it
  ships them, `python-data` is not built.
- **FR-023-028 — Reserved host calls `checkpoint` and `stream.append`**: The shim protocol, owned by this
  feature for every script language (the five of feature 005 through their shims, the four of this
  feature and `python-data`), MUST reserve two host calls with the shapes, gate, bounds and redaction net
  of [contracts/shim-protocol.md](contracts/shim-protocol.md): `checkpoint` records heartbeat checkpoint
  details whose meaning is feature 019's (FR-019-015..018), and `stream.append` appends Workflow Stream
  events whose meaning is feature 026's (FR-026-013..020, ADR-0068). The supervisor MUST refuse either
  call (`NOT_GRANTED`) unless the step's current lease grants the capability (never in a test run), MUST
  bound sizes and per-run call budgets, MUST assign the sequence numbers itself, MUST refuse a payload
  containing a secret value handed to the run before forwarding it, and MUST NOT write either payload to
  the log, the execution record, telemetry or audit. A refusal never fails the step by itself. The
  previous checkpoint delivered with the lease MUST be readable by the script. Until 019 or 026 ships, the
  corresponding call is always `NOT_GRANTED`.

### Non-functional requirements

- **NFR-023-001 (security)**: Zero tested escapes, cross-tenant reads, secret canary leaks or ceiling
  bypasses across the four languages on every enabled target; any one blocks release.
- **NFR-023-002 (durability)**: A committed version, artifact, provenance, policy revision or step
  transition survives runner, engine and broker loss; a duplicate report creates no second effect.
- **NFR-023-003 (compatibility)**: Existing scripts, artifacts, workflows, runner deployments and SDK
  consumers behave unchanged; contracts grow additively; downgrade is refused while new-language rows exist.
- **NFR-023-004 (observability)**: Every build, test run, probe and step of a new language is visible in
  spans, closed-label metrics and redacted logs, and every policy decision in the audit trail.
- **NFR-023-005 (performance)**: Cold and warm start, build time, peak memory and image size per language
  are measured on a declared environment and published with failures; no figure is a commitment.
- **NFR-023-006 (supply-chain honesty)**: Documents and responses state that catalogs are reviewed at image
  build time, that provenance carries runner-reported fields, and that no SLSA level is claimed.

### Key Entities

- **Script language**: a closed id with its builder/runtime images, shim, artifact family and probe.
- **Package catalog**: a deployment-wide, image-baked, reviewed list of packages per language, with a digest.
- **Artifact bundle v4**: a signed, bounded tree of text files for PHP.
- **Build provenance**: a signed in-toto/SLSA statement bound to one artifact digest.
- **Script runtime policy**: a namespace-scoped, revisioned narrowing of languages and packages.
- **Runner presence**: a bounded, advisory record of what a runner of the tenant advertised recently.
- **Build provenance**: one signed statement per build key; the version reaches it through its build key.
- **Image-catalog runtime**: a reserved runtime (`python-data`) whose packages are in a pinned image and
  whose artifacts bind that image's catalog digest; content owned by feature 024.

## Success Criteria *(mandatory)*

All evidence below is required future evidence, not completed tests.

| Criterion | Measurable outcome | Requirements |
|---|---|---|
| SC-023-001 | For each of the four languages, an automated e2e test takes a starter from draft to test run, publication, signed artifact and a workflow step on a real runner and real containers, 100% passing | FR-023-001..003, 008, 011..014, 020 |
| SC-023-002 | The hostile-script battery (metadata, engine socket, host files, direct sockets including `/dev/tcp`, undeclared secret, fork/memory/CPU/disk/log/channel floods, unread replies, environment injection) is contained in 100% of cases per language and target, with zero leftovers and audited refusals | FR-023-010..014, NFR-023-001 |
| SC-023-003 | 100% of malformed, unknown, substituted, incomplete, dev-only and plugin lock fixtures are refused before signing; accepted locks build with no registry access | FR-023-004, 005 |
| SC-023-004 | 100% of malformed v4 trees and Rust ELF fixtures (PIE, interpreter, wrong machine, writable-executable, executable stack) fail before signing and at load; the v1/v2/v3 regression suites pass unchanged | FR-023-008, 009 |
| SC-023-005 | Every new-language SBOM validates against CycloneDX 1.5; every provenance envelope verifies and names the artifact digest; a tampered envelope or SBOM/lock mismatch is refused | FR-023-006, 007 |
| SC-023-006 | A runner advertises a new language only after its probe; optional failures omit it, required failures stop startup, tag-only images fail outside development; Rust is enabled per target only with native AMD64 and ARM64 CI evidence recorded independently | FR-023-015 |
| SC-023-007 | Policy refusals occur at all five points (draft creation, publication, test-run admission, workflow publication, step lease) with the typed reason or `SCRIPT_DENIED` and one audit row each; stale revisions conflict; an attempt leased before the change completes; foreign namespaces are opaque | FR-023-017, 021 |
| SC-023-008 | Runner death during build, test run and step of each language, duplicate completion, lost broker hint, cancellation racing completion, and restart produce no duplicate version, artifact or step transition, and identical idempotency keys across attempts | FR-023-020, NFR-023-002 |
| SC-023-009 | Metric labels stay within the closed sets; source, input, lock and secret canaries are absent from logs, spans, metrics and audit | FR-023-022 |
| SC-023-010 | Real-browser tests highlight, start, test and publish each language from its starter, show disabled languages with their reason and pass the accessibility checks | FR-023-016, 018, 023 |
| SC-023-011 | All five SDKs emit byte-identical IR for script nodes of the four new languages | FR-023-024 |
| SC-023-012 | Requests above ceilings are refused `ABOVE_CEILING`; native build concurrency, v4 bounds and `script_jobs` hold under load | FR-023-019 |
| SC-023-013 | The performance report covers every language on each enabled target with environment and failures; the Deno/Bun report lists per-fixture parity and measurements | FR-023-025, 026, NFR-023-005 |
| SC-023-014 | `python-data` is refused `LANGUAGE_NOT_BUILT` until enabled; with a fixture catalog, a v4 `python-data` artifact runs only on a runtime whose catalog digest equals its `runtimeCatalog` and is refused `SCRIPT_SANDBOX_UNAVAILABLE` on any other, in 100% of the contract and container tests | FR-023-001, 008, 027 |
| SC-023-015 | In every language's container suite, `checkpoint` and `stream.append` are `NOT_GRANTED` in test runs and without the capability, oversized and secret-bearing payloads are refused before forwarding, budgets stop at 600 and 1,000 calls, and neither payload appears in logs, records, spans, metrics or audit (100% of cases); with 019 and 026 present, a checkpoint reaches the next attempt and events get offsets | FR-023-011, 028 |

## Assumptions

- Feature 005's registry, runner, signing and channel are the base; its gaps G97–G119 that remain open
  stay open (research section 1).
- Only Linux AMD64 and ARM64 runners exist; Windows containers are not a target, so PowerShell runs on Linux.
- Pinned versions (Bash 5.x, PowerShell 7.6 LTS, current stable PHP, Rust 1.98.0 for the builder) are
  chosen at implementation and recorded by digest; this document names lines, not digests.
- External effects stay at-least-once; scripts receive an idempotency key and nothing more.

## Dependencies

- Existing: 001 (identity, contracts), 002 (durable execution), 005 (script registry and runner), 007
  (Studio), 008 (telemetry), 009 (quotas, egress, secrets), 016 (classification of script output).
- New features: 019 owns target-aware capability routing and warm workers (G1201); 031 defines encrypted
  fields, which this feature only passes through; 028 may import starters later; 024 supplies the
  `python-data` image, catalog content and data execution environment on the FR-023-027 mechanism and
  depends on this feature for it; 019 (checkpoint semantics, FR-019-015..018) and 026 (Workflow Streams,
  FR-026-013..020, ADR-0068) own the meaning of the reserved host calls of FR-023-028 and cite them. If 019's proposed
  ADR-0052 (per-artifact warm containers) is accepted, it applies to artifacts of the new languages only
  on its own terms; this feature assumes one container per run, and Bash background processes and PHP or
  PowerShell session state are residue its warm-residue suite would have to cover.

## Out of Scope

- Live package installation from crates.io, Packagist, the PowerShell Gallery or npm; arbitrary
  dependency managers; Composer scripts and plugins; Cargo build scripts of the author.
- Windows containers, other shells (zsh, fish, Nushell), other languages, WASM runtimes.
- Enabling Deno or Bun; warm or shared script workers (019); test secrets (G104).
- Signature-derived schema inference (G1209); SLSA level certification; reproducible-build cross-checks.

## Owner Questions

See [research section 6](research.md#6-owner-questions): O1200 (initial catalogs), O1201 (default
namespace policy), O1202 (Deno/Bun follow-up). Each has a recommended default that this specification
already assumes.
