# Package dependencies for TypeScript and Python scripts

*   **Author:** Claude Code, for the owner
*   **Status:** In Review
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

From release 0.3.0 a TypeScript or Python script can use packages from npm and PyPI, and private packages its tenant
uploads to the platform. The author declares what the script needs. In Python that is a PEP 723 block at the top of the
source. In TypeScript it is a `package.json` of dependencies kept beside the source. When the author publishes:

1. The platform resolves the declaration into a lock: every package with an exact version and a content hash.
2. It checks the lock against the namespace's supply-chain policy and the platform's advisory snapshot.
3. It unpacks the packages into a dependency layer, one per native target, named by a digest.
4. It signs a bundle that names the lock and the layer digests.

A run never downloads anything. The runner builds the layer once from the verified package files, checks that it has
the signed digest, and mounts it read-only into the run container, which still has no network.

One component reaches a package registry: the **package gateway**, a small platform service declared in this
repository's compose. Resolution runs in a sandbox container that has no network at all: its HTTP requests travel over
the sandbox's supervisor channel to the runner, which forwards them to the gateway. Installation runs no installer and no
package code.

Windmill, the reference product for the 0.3.0 parity scope (Owner decision 2026-11-24-1), resolves imports at deployment
and installs packages on its workers from a registry. This design gives the same convenience with guarantees Windmill
does not give: no network in a run, no package code before the run, a policy at publication, a stop for malware found
later, a software bill of materials (SBOM) and signed provenance for every build, offline builds, and protection against
dependency confusion.

The architecture is
[Architecture decision 0120](../adr/ADR-0120-script-packages-are-resolved-at-publication-through-the-package-gateway-and-run-from-a-signed-read-only-layer.md);
the policy and its effect on published versions is
[Architecture decision 0121](../adr/ADR-0121-a-namespace-supply-chain-policy-gates-publication-and-a-malicious-package-advisory-stops-leases.md).
Both are Proposed. The registers (decisions, gaps) are in
[research.md](../../specs/046-script-dependencies/research.md); the contracts, the data model and the review resolution
are under [specs/046-script-dependencies/](../../specs/046-script-dependencies/). Principle numbers refer to
[the constitution](../constitution.md).

This design closes Gap 97 of feature 005 for TypeScript and Python
([feature 005 research](../../specs/005-inline-code-sandbox/research.md)). It replaces Decision 105 of feature 005 for
those two languages only. C# keeps its closed catalog
([Architecture decision 0021](../adr/ADR-0021-csharp-builds-use-a-curated-offline-toolchain.md)); Rust and PHP keep
theirs ([feature 023](../../specs/023-script-languages/spec.md),
[Architecture decision 0057](../adr/ADR-0057-additional-script-languages-reuse-the-sandbox-and-signed-artifacts.md)).

### Terms

Terms of this design:

- **Manifest.** What the author declares: package names and version ranges. Python: the PEP 723 inline metadata block.
  TypeScript: a `package.json` document with only `dependencies` and, optionally, `overrides`.
- **Lock.** The resolved closure: every package, its exact version, its gateway path and its content hash. Python: a
  PEP 751 `pylock.toml` with wheels only. TypeScript: an npm `package-lock.json` with `lockfileVersion: 3`.
- **Package gateway (the gateway).** `apps/package-gateway`: a read-only proxy of the npm registry protocol and of the
  Python simple repository API (PEP 691), with its own package store. It is the only component with registry egress.
- **Package store.** The gateway's content-addressed file store, a blob store separate from the tenant payload store.
- **Resolve job, install job.** Two runner jobs that precede the build of a version with packages. A resolve job runs the
  pinned resolver and returns the lock. An install job, one per required target, unpacks the locked files and returns
  the layer's digest.
- **Dependency layer (the layer).** The unpacked packages for one native target (`linux-amd64` or `linux-arm64`), named
  by the digest of a canonical serialization (`contracts/bundle-v5.md`).
- **Materialization.** A runner building a layer from the package store into a local engine volume, and checking the
  digest, before its first run of that version.
- **Required targets.** The native targets a deployment serves (`SAGAT_SCRIPT_DEPENDENCY_TARGETS`, default
  `linux-amd64`). Every version with packages has a layer for each.
- **Supply-chain policy (the policy).** The `packages` section of the namespace's script policy document (Technical
  Plan 6).
- **Advisory snapshot.** The platform's copy of the OSV (Open Source Vulnerabilities) records of the npm and PyPI
  ecosystems, vulnerabilities and malicious packages alike (Technical Plan 7).
- **Finding.** A policy result on one package of a lock: an advisory, a licence, an age, a skipped install script.
- **Block.** A row that stops the engine from leasing the script steps of an artifact, for one namespace or for all.
- **Hostile battery.** The real-container test suite of the Verification Plan that tries every escape this design
  closes.

Terms of the platform this design uses (defined by feature 005 unless said otherwise):

- **Build key.** The digest of everything a build depends on; a ready version with the same key is reused instead of
  built again (feature 005, Decision 99).
- **Bundle.** The canonical JSON artifact the control plane signs. Versions 1 to 3 exist
  (`crates/infra-schema/src/bundle.rs`): 1 is text files for TypeScript and Python, 2 is C#, 3 is Go and Java. Version 4
  is planned by feature 023 for PHP.
- **Framework-only version.** A TypeScript or Python version without packages, as feature 005 builds today.
- **Sandbox profile.** The container settings of feature 005, Decision 95: no network, read-only root, a small
  `noexec` scratch area, no capabilities, `no-new-privileges`, user 65534, resource limits.
- **Supervisor channel.** The JSON-line channel between a sandbox container and its runner, on standard input and output
  (feature 005, Decision 96).
- **Lease.** The engine's time-bounded grant of a step to a worker; a runner serves script steps as a worker.
- **Instance registry and capability.** The record of what each control plane and engine can do, and the operator's
  activation of a requirement (Architecture decision 0082, Requirement 018-052).

## The Problem

Windmill's main convenience for scripts is that `import pandas` works. On deployment it infers the requirements from the
imports and resolves them with `uv` or `bun`. It stores a lockfile with the script. Its workers install the packages
from a public or private registry, with network access, and cache them on the worker's disk. It applies no
supply-chain policy: no vulnerability check, no quarantine of new releases, no protection against a private name being
taken on the public registry, no SBOM.

This platform does the opposite today. Feature 005 refuses every package for TypeScript and Python (Decision 105). A
script can use only the standard library and the platform's script shim. Gap 97 of feature 005 asks for pinned
dependencies, an SBOM and a supply-chain policy; it is closed only for one C# package. An author who needs an HTTP
client, a date library or pandas writes a worker in an SDK instead. That takes the code away from the platform's
sandbox, signing and versioning.

Any answer must keep these constraints:

- The run container has no network, a read-only root and a `noexec` scratch area. A run must not download a package.
- Untrusted code must not run in the control plane, the scheduler or the runner process (Principle V). Installing a
  package in either ecosystem can run its code: npm lifecycle scripts, a Python source distribution's build.
- A bundle is at most 2 MiB and lives in PostgreSQL (feature 005, Gap 108). A dependency tree is far larger.
- Infrastructure is self-contained in this repository (AGENTS.md rule 16). The proxy must be in the compose, pinned,
  on loopback, with a healthcheck.
- A published version is immutable (Principle IV). What ran yesterday runs today, even if the registry deletes the
  package.

## Requirements

### Functional

**Declaration and resolution**

- **Requirement 046-001** A TypeScript or Python draft MAY carry a manifest: Python in a PEP 723 block of its source,
  TypeScript in the draft's `manifest` field. A draft without one MUST build and run exactly as today, with the same build
  key. *Acceptance:* `framework_only_versions_keep_their_build_key` (unit, `crates/application`) and a publication and
  run with each manifest form (`integration_script_dependencies::publish_and_run_*`).
- **Requirement 046-002** Publication of a version with a manifest MUST resolve it into a lock through a resolve job
  (Technical Plan 4), every time, or accept a lock the author supplies. An author lock MUST satisfy the manifest, and
  every entry MUST be a file the gateway serves with the same hash. *Acceptance:* both paths produce a version whose
  lock satisfies the manifest; an author lock with a changed hash, an extra root, a missing dependency or a source outside
  the gateway is refused with `422 LOCKFILE_INVALID` (`lock_rules_*` fixtures).
- **Requirement 046-003** The platform MUST NOT infer a manifest from the imports. *Acceptance:* a publication with an
  import that no manifest entry covers fails when the module is imported, like any missing module, and nothing is added
  to the manifest.
- **Requirement 046-004** A resolve job MUST run in a container with the sandbox profile and no network. Its HTTP
  requests MUST reach only the gateway, forwarded by the runner over the supervisor channel; the container MUST receive
  no secret and no token. Its only output is the lock. *Acceptance:* the hostile battery shows a resolve container
  reaching nothing but the gateway's package routes (not the internet, the control plane, the engine, the runner host,
  another container, the cloud metadata address, any name through the resolver).
- **Requirement 046-005** Python locks MUST contain wheels only. A package with only a source distribution is refused
  with `PACKAGE_SOURCE_ONLY`. Every wheel MUST match the runtime's interpreter and one required target, and every
  required target MUST be served (`PACKAGE_PLATFORM_UNSUPPORTED` otherwise). *Acceptance:* one fixture per refusal; a
  lock with wheels for two targets gives a layer per target.
- **Requirement 046-006** npm lifecycle scripts (`preinstall`, `install`, `postinstall`, `prepare`) of dependencies MUST
  NOT run. A package that declares one is admitted with a finding `INSTALL_SCRIPT_SKIPPED`, unless the policy refuses such
  packages. *Acceptance:* a fixture whose `postinstall` writes a file; the file is absent from the layer and the finding
  is in the version's report.
- **Requirement 046-007** The control plane MUST validate every lock it stores against the closed subsets of
  [lock-formats.md](../../specs/046-script-dependencies/contracts/lock-formats.md): hashes on every entry, gateway paths
  only, no `git`, `file`, `link`, directory or URL dependency, no alias, names normalised, the closure complete against
  the dependencies the gateway reports, the manifest satisfied. *Acceptance:* one refused fixture per rule.

**Policy and advisories**

- **Requirement 046-008** Before a build is signed, the control plane MUST evaluate the lock against the namespace's
  policy, the gateway's metadata and the advisory snapshot (Technical Plans 6 and 7). It MUST refuse with the first
  failing reason in the order of
  [errors.md](../../specs/046-script-dependencies/contracts/errors.md) and list every finding. The evaluation runs on
  every publication, also when a build with the same key exists. *Acceptance:* one fixture per reason; the cache-hit
  test `a_cached_build_is_evaluated_again`.
- **Requirement 046-009** A package matched by a malicious-package advisory MUST be refused at publication and MUST NOT
  be leased afterwards, whatever the policy or a waiver says (Architecture decision 0121). *Acceptance:*
  `a_snapshot_marking_a_locked_package_malicious_blocks_its_next_lease` and
  `a_snapshot_imported_during_a_build_is_seen_before_signing`.
- **Requirement 046-010** A change of the policy, an expired waiver, a new snapshot and a new private-name reservation
  MUST each re-evaluate the stored locks they affect, through a durable job, and write or remove blocks accordingly.
  *Acceptance:* `policy_change_reaches_the_lease`, `waiver_expiry_reaches_the_lease`,
  `reevaluation_resumes_after_a_crash`.
- **Requirement 046-011** A namespace administrator (`script:policy:manage`) MUST be able to read and change the policy,
  add a waiver (one advisory, one package version, an expiry of at most 90 days, a reason), and list findings; every
  change is audited. A waiver MUST NOT cover a malicious-package advisory. *Acceptance:* REST contract tests;
  `a_waiver_cannot_cover_malware`; `a_waiver_longer_than_90_days_is_refused`.

**Installation, bundle and run**

- **Requirement 046-012** Installation MUST NOT run an installer or package code. The runner MUST check each fetched
  file against the lock's hash before use. The platform's unpacker, in a container with the sandbox profile and no
  network, MUST write the layer and its digest (Technical Plan 8). *Acceptance:* `layer_digest_is_identical_on_two_hosts`
  (the native-runner CI job); an archive with a link, an absolute path, a `..` segment, a device or special file, a
  set-user-id bit, a path written by two packages, a path over the bound or a size over the bound is refused.
- **Requirement 046-013** A version with packages MUST be signed as bundle version 5
  ([bundle-v5.md](../../specs/046-script-dependencies/contracts/bundle-v5.md)), which names the lock digest and, per
  required target, the layer digest. A runner MUST refuse to mount a layer whose digest differs from the signed one.
  *Acceptance:* a tampered lock, a tampered bundle and a layer built from another tenant's files each fail before the run
  container starts, with `SCRIPT_UNTRUSTED_ARTIFACT`.
- **Requirement 046-014** A run of a bundle version 5 MUST mount its layer read-only as the only added mount of the run
  container, from a runner volume of that tenant that carries the completion marker of the signed digest. Volumes are
  never shared between tenants. *Acceptance:* a script that writes into the layer fails;
  `two_tenants_with_one_lock_get_two_volumes`; `an_unmarked_volume_is_never_mounted`.
- **Requirement 046-015** Every build of a version with packages MUST have a CycloneDX SBOM, built by the control plane
  from the lock and the gateway's metadata, listing every locked package with its package URL, version, hashes and
  declared licence, and the provenance statement of Architecture decision 0057 decision 6, whose resolved dependencies are
  the lock's entries and whose runner-reported fields are marked as such. *Acceptance:* the SBOM package set equals the
  lock's; the provenance verifies with the platform key.
- **Requirement 046-016** A test run of a draft with a manifest MUST resolve, evaluate and materialize on its runner as
  a publication would, MUST NOT be signed, and MUST NOT receive a secret (feature 005, Decision 104). *Acceptance:*
  `a_test_run_with_packages_runs_and_is_refused_for_malware`.

**Private packages and offline operation**

- **Requirement 046-017** A tenant administrator (`package:manage`) MUST be able to reserve package names and npm scopes
  for the tenant. Only a reserved name can be uploaded, by a holder of `package:upload`. A reserved name MUST resolve
  only from the tenant's uploads, never from a public registry, for every namespace of the tenant; names are compared
  after normalisation. *Acceptance:* `a_reserved_name_never_resolves_from_the_public_registry` with case and separator
  variants; `an_identical_upload_is_accepted_again`, `a_different_file_for_an_uploaded_version_is_a_conflict`.
- **Requirement 046-018** The gateway MUST run `online` (with upstream registries) or `offline` (only its store), and
  MUST export and import signed store bundles so that an air-gapped deployment can build. A version whose files are
  already stored MUST build and run without any upstream. *Acceptance:* `offline_build_from_an_imported_bundle`.

**Contracts, rollout and surfaces**

- **Requirement 046-019** Failures MUST be typed and value-free (errors.md): publication refusals are `422` with a closed
  reason and the package name and version, never a registry response; build failures carry the kind `resolve` or
  `install`; an unreachable gateway is `PACKAGE_GATEWAY_UNAVAILABLE`, retryable, whether it refused, timed out or answered
  `503`; a run of a blocked artifact is `SCRIPT_DENIED`, reason `PACKAGE_BLOCKED`, not retryable. *Acceptance:* the error
  contract tests.
- **Requirement 046-020** The feature MUST be switched on by the capability `script-dependencies-v1` of the instance
  registry for control planes and engines, activated by the operator (Requirement 018-052), and runners MUST declare it
  in their claims. Without it a manifest or a package-bearing lock is refused as today; a runner that does not declare it
  is never given a dependency job or a step of a bundle version 5. *Acceptance:* `without_the_capability_a_manifest_is_refused`,
  `an_old_runner_claim_gets_no_dependency_work`.
- **Requirement 046-021** The Studio MUST show the manifest, the lock summary, the findings, the blocks and the policy;
  the CLI MUST show a version's packages and upload private packages. *Acceptance:* the Studio journey test and the CLI
  contract tests.

### Non-functional

- **Non-functional requirement 046-001 (isolation)** No component other than the gateway reaches a registry; no package
  code runs outside a sandbox container; the gateway holds no tenant secret and only the package store's credential.
- **Non-functional requirement 046-002 (reproducibility)** The same lock and unpacker give the same layer digest on
  every runner of a target (bytecode files are outside the digest).
- **Non-functional requirement 046-003 (performance targets, to be measured)** On the development machine: a warm run
  with a materialized layer adds at most 50 ms to the start of the same script without packages; resolving a lock of 50
  packages with a warm gateway takes at most 30 s; materializing a 100 MiB layer takes at most 20 s. Benchmark criteria,
  not promises (Delivery Constraints).
- **Non-functional requirement 046-004 (bounds)** Manifest 64 KiB; lock 256 KiB (the existing lockfile ceiling) and
  2,000 entries (a typical npm entry takes about 120 bytes, a wheel entry about 250, so the byte bound is reached first
  for large locks; the probe of Task 046-12 measures real locks); a layer of 1 GiB by default and 4 GiB at the platform
  ceiling; 200,000 files per layer; a path of 1,024 bytes; a resolve job of 300 s with 1 GiB of forwarded responses; an
  install job of 600 s; a runner volume cache of 50 GiB and a file cache of 10 GiB by default; 200,000 advisory records
  per ecosystem per snapshot.
- **Non-functional requirement 046-005 (telemetry)** Metrics with closed labels only; tenant, namespace, script and
  package names in spans and logs, never in metric labels (Principle V).

### Success criteria

- **Success criterion 046-001** The packages pinned in
  [acceptance-packages.md](../../specs/046-script-dependencies/contracts/acceptance-packages.md) (ten Python and ten npm
  packages, chosen for coverage of native wheels, prebuilt npm binaries and deep trees) each publish and run in a script
  on every required target in the acceptance run.
- **Success criterion 046-002** The hostile battery passes on both targets.
- **Success criterion 046-003** A deployment with the gateway `offline` and an imported store bundle publishes and runs a
  script whose lock is in the bundle.
- **Success criterion 046-004** The benchmark report exists and states its environment and failures.

### User stories

- **User story 046-01** An author uses pandas and httpx in a Python script, and axios and zod in a TypeScript one,
  without writing a worker (Journey 1).
- **User story 046-02** A security owner sets the namespace's policy and sees, for every version, what it contains and
  what was found (Journeys 2 and 3).
- **User story 046-03** An operator runs the platform without internet access (Journey 4).
- **User story 046-04** A team uses its private packages without exposing their names to a public registry (Journey 5).

## Goals

- Scripts in TypeScript and Python get the packages of their ecosystems, as in Windmill.
- Guarantees Windmill does not give: no network in a run or a resolution, no package code before the run, a policy at
  publication, a stop for malware found later, an SBOM and signed provenance for every build, offline builds, protection
  against dependency confusion.
- Reuse feature 005's machinery: drafts, versions, the build key, signing, the runner, the sandbox profile, the
  execution record.

## Non-Goals

- Packages for C#, Go, Java, Rust or PHP (they keep closed catalogs).
- Source distributions and install scripts (Gap 046-001).
- Proxying a tenant's own external registry with its credentials (Gap 046-002): it needs a new disclosure path under
  Principle V. Tenants upload private packages instead.
- Changing the TypeScript runtime (Deno and Bun stay out, Architecture decision 0057 decision 7).
- Inferring manifests from imports (Decision 046-003).
- Promising that a package without advisories is safe: the snapshot is evidence at a date, not a guarantee.

## Open Questions

None blocks the design. Three defaults wait on measurements, each registered as a gap: the quarantine default (Gap
046-003), the cache bounds (Gap 046-004) and the resolver command lines proved by the probe (Gap 046-005).

## Owner decisions needed

None is open. The design follows Owner decision 2026-11-24-1 (the feature is in 0.3.0). Its product and design choices
were taken under the delegation of Owner decisions 2026-11-20-6 and 2026-11-24-4 and are recorded as Decisions 046-001
to 046-034 in research.md, with the register's literal marker "delegate (OD-2026-11-24-4)" and the reasoning. That
includes the one security exception the design needs: the read-only dependency mount, which relaxes Architecture
decision 0013's "nothing mounted" (Decision 046-009), recorded in Architecture decision 0120 as a partial supersession.
Architecture decisions 0120 and 0121 are accepted or rejected under Owner decision 2026-11-24-4 when this design
completes its review.

## Critical User Journeys (CUJs)

### Journey 1: An author uses pandas in a Python script

*   **Given**: a namespace with the default policy, the capability `script-dependencies-v1` activated, a runner and the
    gateway online.
*   **When**: the author writes a script whose first lines are a PEP 723 block naming `pandas>=2.2` and `httpx`, tries it
    (a test run) and publishes it.
*   **Then**: the test run resolves, is evaluated, materializes and runs on its runner, with no secret, as today.
    Publication returns the version with its lock summary, its findings (none) and its SBOM link. The first run on each
    runner materializes the layer; later runs start from the cached volume. The run container has no network.

### Journey 2: A new release of a dependency is malware

*   **Given**: a TypeScript script locked to `left-pad-ish 1.0.3`, and the default quarantine of 72 hours.
*   **When**: `1.0.4` is published upstream with a credential stealer, and the author publishes again an hour later.
*   **Then**: the gateway does not offer `1.0.4` (younger than the quarantine) and the new lock keeps `1.0.3`. Later, OSV
    marks `1.0.4` malicious and the advisory pass imports it. In that same transaction, every artifact of any tenant
    that locked `1.0.4` (a namespace with quarantine 0) is blocked. Its next script step fails `SCRIPT_DENIED`
    (`PACKAGE_BLOCKED`); an attempt already running finishes; the security owner sees the finding.

### Journey 3: A vulnerability is disclosed in a pinned package

*   **Given**: a version locked to `requests 2.31.0`.
*   **When**: an advisory of severity HIGH affecting it is imported.
*   **Then**: the version gets a finding, an audit record and a metric. It keeps running, because the namespace's policy
    does not block new findings at lease by default. A new publication of the same lock is refused with
    `PACKAGE_VULNERABLE` unless a waiver with an expiry is recorded.

### Journey 4: An air-gapped deployment

*   **Given**: a deployment with no internet and the gateway `offline`.
*   **When**: an operator exports a signed store bundle and an advisory snapshot on a connected machine, imports both, and
    an author publishes a script whose manifest those packages satisfy.
*   **Then**: the resolver sees only the store, the build succeeds and the run works. A package not in the store fails
    with `RESOLUTION_FAILED`, naming it.

### Journey 5: A private package name appears on the public registry

*   **Given**: a tenant that reserved and uploaded `acme-billing 1.4.0`.
*   **When**: an attacker publishes `acme_billing 99.0.0` on PyPI.
*   **Then**: `acme_billing` normalises to `acme-billing`, a reserved name, so every namespace of the tenant resolves only
    the uploaded files; the public project is never offered, and the lock validator would refuse it if it appeared.

## Technical Plan (Architecture)

### 1. The path of a version with packages

```mermaid
sequenceDiagram
    participant A as Author (Studio, CLI)
    participant CP as Control plane
    participant R as Sandbox runner
    participant RC as Resolve container
    participant G as Package gateway
    participant IC as Install container
    A->>CP: publish draft (source, manifest [, lock])
    CP->>CP: validate manifest; version BUILDING; queue resolve job
    R->>CP: claim dependency job (capability, target)
    R->>RC: start (sandbox profile, no network)
    RC->>R: HTTP request over the supervisor channel
    R->>G: forward with the job token
    G-->>R: response
    R-->>RC: response over the channel
    RC-->>R: lock (chunked on the channel)
    R->>CP: complete resolve (lock)
    CP->>G: metadata of the locked files (internal listener)
    CP->>CP: validate lock; evaluate policy; queue an install job per required target
    R->>CP: claim install job (its target)
    R->>G: fetch each locked file by hash; check it
    R->>IC: files on standard input (no network)
    IC-->>R: layer digest and size
    R->>CP: complete install (digest)
    CP->>CP: queue the source build (feature 005)
    R->>CP: complete build
    CP->>CP: finishing transaction: snapshot check, SBOM, sign, version READY
```

A version without a manifest takes feature 005's path unchanged.

Dependency jobs live in a table of their own, `script_dependency_jobs` (one row per version and phase and target,
[data-model.md](../../specs/046-script-dependencies/data-model.md)), with the same claim, heartbeat, retry, `not_before`
and `script_jobs` quota as builds (feature 005, Decisions 99 and 109). `script_build_jobs` keeps its one row per version.
Each transition is one transaction fenced by the job's lease token:

- **Resolve completes.** The control plane calls the gateway's metadata endpoint first, with no transaction open. Then,
  in one transaction under the resolve job's token, it stores the lock, the metadata and the report, and either refuses
  the version (`FAILED` with the reason) or inserts the install jobs. A failed metadata call fails the resolve job as
  retryable. A duplicate completion finds the job finished and changes nothing (`AlreadyDone`, as feature 005 answers
  today).
- **The last install completes.** In one transaction it records the layer digests and inserts the source build job.
- **The build finishes** (Technical Plan 9).

**Resolution is never cached.** Every publication with a manifest and no author lock runs a resolve job, so publishing
again picks up newer versions that the quarantine now allows. **The build key** of a bundle version 5 is computed when
the build job is inserted, after the lock is fixed: the feature 005 parts plus the manifest digest, the lock digest, the
unpacker identity, the dependency runtime image identity and the layer digests, under the new prefix `sagat/build/v2`.
Framework-only versions keep the `sagat/build/v1` bytes of `crates/application/src/sandbox.rs`. A cached build with the
same key is reused only after this version's own evaluation passed, and the version's report is its own (Technical Plan
6), so reuse never carries another version's verdict.

### 2. The manifest

Python: the control plane parses the PEP 723 block from the source. TypeScript: the draft's `manifest` field, a new
column of `scripts`, `script_versions` and `script_test_runs` beside `lockfile`, protected by `script_versions_guard`
like the other published columns. The closed rules are in lock-formats.md. The existing `Lockfile` type
(`crates/domain/src/scripts.rs`) carries the lock; its documentation changes from "`requirements.txt` with hashes" to
PEP 751 for Python. No stored Python version has a package-bearing lock today, because `lockfile.rs::accept` refuses
them, so nothing is migrated.

`crates/infra-sandbox/src/lockfile.rs::accept` stays the single gate on the runner side. For TypeScript and Python it
accepts a lock only on a dependency job or a bundle version 5, whose lock the control plane has already validated; for
every other build it refuses package-bearing locks as today.

### 3. The package gateway

`apps/package-gateway` is a Rust service with no database
([package-gateway.md](../../specs/046-script-dependencies/contracts/package-gateway.md)). It has two listeners: a
package listener for runners, and an internal listener for the control plane's metadata calls and store
administration. It serves npm package documents and tarballs and the PyPI simple API, with wheel core-metadata files
(PEP 658 and PEP 714), so that a resolver reads metadata without downloading wheels.

Every request carries a token signed by the control plane with its package token key, which is not the script signing
key: a job token for runners, a service token for the control plane. The runner adds the job token to the requests it
forwards; no token enters a container.

What the gateway serves to a job is filtered by the job's policy, using the same pure rules as the control plane
(`crates/domain`, Technical Plan 13): versions younger than the quarantine, denied names, yanked versions, for Python
every source distribution and every wheel no required target can use, and every public version of a name the tenant
reserved. The filter is a second check. The control plane's evaluation is the authority.

The gateway is a trusted platform component: the control plane relies on the upload times, licences, install-script
flags and declared dependencies it reports. Two checks keep it honest. The unpacker reports the install scripts it
actually sees in each `package.json`, and a mismatch with the metadata refuses the version (`PACKAGE_METADATA_MISMATCH`).
Every file's hash is checked against the lock by the runner, and against the registry's declared hash by the gateway
when it is first stored.

Files are fetched on first request through a streaming send that `crates/infra-egress` gains (Task 046-6): addresses
checked once, private ranges refused unless the operator allows an upstream's, redirects not followed. The gateway
fetches only https URLs whose host is a configured upstream or file host (`registry.npmjs.org`,
`files.pythonhosted.org` by default). Names are validated and normalised before an upstream path is built.

The package store is a blob store of its own (`SAGAT_PACKAGE_STORE_*`, the settings of `crates/infra-blobstore`), not the
tenant payload store of Architecture decision 0023, so the internet-facing gateway never holds a credential that reads
tenants' payloads. Keys are `public/sha256/<hex>` and `private/<tenant>/sha256/<hex>`. Public files are shared by all
tenants, because they are public; the timing of a cache hit can tell a tenant that some tenant fetched a public file,
which is accepted as a residual risk (Gap 046-007). A private file, a name reserved by another tenant and a missing file
all answer the same `404`.

Collection: a leased control-plane pass (daily) lists store objects older than 30 days through the internal listener
and deletes those that no stored lock or upload references (`script_locked_packages`, `private_packages`). Private
uploads count against the tenant's content bytes (Architecture decision 0027). New public bytes that a tenant's locks
bring into the store count against a daily fetch budget per tenant (20 GiB by default); over it, publication fails
retryable with `PACKAGE_FETCH_BUDGET_EXCEEDED`.

Upstreams are configured by the operator (any compatible mirror an organization runs can be the upstream). In
`offline` mode there is none. `package-gateway export` writes a store bundle signed with the operator's export key;
`package-gateway import` refuses a bundle whose signature or any hash does not verify.

### 4. The resolve job

The runner claims a resolve job with its declared capability and target. The claim returns the source, the manifest,
the runtime line and the required targets. The runner starts a container from the pinned resolver image of the language
(`runners/resolver-python`, `runners/resolver-node`, by digest) with the sandbox profile and no network; its memory is
1 GiB and its scratch area 512 MiB for the resolver's cache.

The resolver shim starts a small HTTP server on the container's loopback interface and points the resolver at it. Each
request it receives becomes an `http` message on the supervisor channel
([runner-protocol.md](../../specs/046-script-dependencies/contracts/runner-protocol.md)). The runner checks that the
path is a gateway package route for the job's ecosystem, adds the job token, forwards it to the gateway, and returns the
response in chunks of at most 192 KiB per line. A response is at most 32 MiB; a job forwards at most 1 GiB. Nothing else
leaves the container: it has no network interface but loopback, no name resolution, and no token. This keeps feature
005's "no network" for every sandbox container, and works whatever host the runner is on.

The shim then runs:

- Python: the pinned `uv`, with the loopback server as its only index, binary wheels only, the quarantine as its
  `exclude-newer` cut, the runtime's Python version and every required target as the environments, writing a PEP 751
  lock.
- TypeScript: the pinned `npm`, in lock-only mode, scripts ignored, the loopback server as its registry, the quarantine
  as its `before` date, writing `package-lock.json` version 3.

The probe of Task 046-12 fixes the command lines against the pinned versions (`uv` 0.8 or later; the `npm` 11 of the
Node 24 image) and the base images (`python:3.13-slim-bookworm` and `node:24-bookworm-slim`, by digest). Its acceptance:
the twenty packages of Success criterion 046-001 resolve to locks that the validator accepts, for both targets, with no
request outside the gateway routes. If `uv` cannot write PEP 751 directly for a script, the shim converts `uv`'s own lock
to PEP 751; if neither works, the release falls back to a `requirements.txt` with one `--hash` per file (pip's
hash-checking format), which the validator then accepts in place of PEP 751 (Gap 046-005).

The lock returns as `lockChunk` messages (at most 256 KiB in total). A resolver that fails returns its first 8 KiB of
error output, redacted, kept in the build log; the build fails with kind `resolve` and `RESOLUTION_FAILED`, not
retryable. A transport failure toward the gateway (refused, timed out, `502`, `503`, `504`) is reported by the runner as
`PACKAGE_GATEWAY_UNAVAILABLE`, retryable.

An author-supplied lock skips the resolve job and goes straight to validation and evaluation.

### 5. Lock validation

The control plane parses the lock in the closed subset of its format (lock-formats.md) with bounded nesting and sizes,
and refuses anything outside it. Beyond the rules of Requirement 046-007: every manifest entry is satisfied by a root
entry; the closure is complete when checked against the dependencies the gateway reports for each exact file; Python
has one entry per normalised name; npm paths are canonical; each required target selects a full set of entries (Python
by wheel tags, npm by the `os`, `cpu` and `libc` fields). The lock digest is the SHA-256 of its stored bytes.

Version orders and range matching (node semantic versions; PEP 440 and PEP 508) sit behind the application port
`PackageRules`, implemented by `crates/infra-packages` with published crates (`nodejs-semver`, `pep440_rs`, `pep508_rs`),
because the domain's dependency allowlist admits none of them. The domain holds the policy types and the pure
combination of per-package results into a verdict.

### 6. The supply-chain policy

The policy is the `packages` section of the namespace's script policy document, in the table `script_runtime_policies`
that this design creates (feature 023 adds its `languages` section, Decision 1627 of feature 023), changed through
`PUT /v1/namespaces/{namespace_id}/script-policy` with an expected revision. Allow and deny rules are rows of
`script_package_rules`, changed through `/v1/namespaces/{namespace_id}/script-policy/package-rules`, so that the document
stays within its 64 KiB bound. The schema is
[supply-chain-policy.schema.json](../../specs/046-script-dependencies/contracts/supply-chain-policy.schema.json). Its
fields and defaults:

| Field | Default | Bounds |
|---|---|---|
| `npm.enabled`, `pypi.enabled` | true | |
| `allow` rules | none (everything not denied) | 500 rows: a name glob, an optional version range |
| `deny` rules | none | 500 rows |
| `quarantineHours` | 72 | 0 to 720 |
| `vulnerabilities.blockAtOrAbove` | `HIGH` | `CRITICAL`, `HIGH`, `MODERATE`, `LOW`, `NONE` |
| `vulnerabilities.blockUnrated` | false | |
| `vulnerabilities.blockNewAtLease` | false | |
| `licences.deny` | empty | 100 SPDX identifiers |
| `installScripts` | `skip` | `skip` (admit with a finding) or `refuse` |
| `maxLayerBytes` | 1 GiB | up to the platform ceiling, 4 GiB |

Waivers are rows of `supply_chain_waivers`: one advisory, one package version, an expiry of at most 90 days, a reason, the
actor. Any `allow` rule makes the policy an allow-list. Malicious-package advisories are not a field: they are always
refused, and a waiver cannot name one.

Evaluation takes each locked entry, the gateway's metadata and the advisory matches, and returns findings and a verdict.
The verdict refuses on these rules (their codes and their order, with the lock's own rules first, are in errors.md):

1. a malicious-package advisory;
2. a reserved name taken from a public source;
3. a deny rule, or not allowed by a non-empty allow list;
4. a version younger than the quarantine at evaluation time;
5. an advisory at or above the threshold without a live waiver, or an unrated one if `blockUnrated`;
6. a denied licence (an SPDX expression is denied when every alternative contains a denied identifier; an unparseable
   or absent licence is a finding, not a refusal);
7. a yanked or deprecated version without a live waiver;
8. an install script with `installScripts: refuse`;
9. a layer over `maxLayerBytes` (known after install, checked again before signing).

The report (verdict, findings, snapshot identities, policy revision and digest) is stored with the version in
`script_version_dependencies`, not in the bundle.

**Re-evaluation and blocks.** A block is a row of `script_artifact_blocks` keyed by tenant, artifact digest and
namespace (null for every namespace), with a reason and a source. Artifact rows stay immutable (the trigger
`script_artifacts_immutable` refuses every update). A durable row of `script_policy_reevaluations` (tenant, namespace or
null, reason, cursor) is inserted in the same transaction as a policy change, a waiver's expiry (by the hourly waiver
pass), a snapshot import or a name reservation. A leased pass works through it in pages of 200 versions, one transaction
per page, advancing the cursor in that page's commit. For each version it recomputes findings and writes or removes the
namespace's blocks: a malicious package or a reserved-name conflict blocks for every namespace; with
`blockNewAtLease`, a finding at or above the threshold without a waiver blocks that namespace.

### 7. The advisory snapshot

The control plane keeps the OSV records of the `npm` and `PyPI` ecosystems in `package_advisories`: identifier,
aliases, kind, severity, affected ranges. A record is malicious when its identifier or any alias starts with `MAL-`, or
its database-specific type says malware. Severity is the GitHub advisory label when present, otherwise the base score of
its CVSS (Common Vulnerability Scoring System) version 3 or 4 vector mapped to the same four labels; otherwise the
record is unrated.

Imports come from a leased control-plane pass (every 6 hours by default), which downloads each ecosystem's OSV archive
from the operator-configured URL (`SAGAT_ADVISORY_SOURCE_*`, default the public OSV bucket) through the streaming send of
`crates/infra-egress` under the deployment's outbound allow-list; or from an operator command,
`control-plane advisories import <file>` (offline). An import is refused if it is older than the stored snapshot of that
ecosystem or holds 1 percent fewer records, unless the operator forces it (audited).

One import is one transaction, under the snapshot row's lock. It replaces the ecosystem's records and, through the index
`script_locked_packages` (tenant, version, ecosystem, normalised name, package version), inserts a block for every
artifact that locks a package a new malicious record matches. So malware is blocked in the import's own commit, not
after a paged re-scan. It also inserts the re-evaluation row for everything else. A malware block is removed only by an
administrator, audited, after the record is withdrawn.

### 8. Installation and the layer

The runner claims an install job for its own target. It fetches each locked file of that target from the gateway's
`/files/sha256/{hex}` route, with a URL it builds from its configured gateway base and the hash (never from a URL in the
lock), checks the hash, and keeps the file in its local file cache. It then starts an install container from the pinned
layer-tool image (`runners/layer-tool`, a static Rust binary) with the sandbox profile, no network, 1.5 GiB of memory, no
scratch area, 1,024 open files and a file-size limit equal to the layer limit, and a fresh engine volume mounted
read-write at `/out`. It streams the files in lock order on standard input. The unpacker:

- Python: unpacks each wheel into `site-packages/` following the wheel format (`purelib`, `platlib` and their `.data`
  directories; `scripts`, `headers` and `data` dropped), checks every file against the wheel's `RECORD`, keeps
  `.dist-info`.
- TypeScript: unpacks each tarball into the `node_modules` path its lock entry names, stripping the archive's top
  directory; `bin` links are not created; reports the lifecycle scripts it sees in each `package.json`.
- Both: refuses links, absolute paths, `..` segments, special files, set-user-id and set-group-id bits, a `__pycache__`
  directory in a wheel, a path written by two packages, a path over 1,024 bytes, more than 200,000 files and a total over
  `maxLayerBytes`; fixes modes (0444 files, 0555 directories), owners and times; computes the layer digest
  (bundle-v5.md) and reports it with the size and the file count.

For Python, a second container from the runtime image then compiles the layer to bytecode:
`python -I -m compileall` with the working directory `/`, the volume read-write, no network. `-I` keeps the layer and
the working directory off the import path, so no file of the layer is imported. Bytecode files are outside the digest.
Last, the layer tool writes the completion marker `.sagat-layer`.

An install job keeps nothing on the platform: the digest goes into the version, and the volume stays as the runner's
first cache entry. Doing the unpacking in a sandbox container means that an archive crafted to exploit the unpacker lands
in a container with no network and nothing to steal, not in the process that holds the engine's socket.

### 9. Finishing, signing, the SBOM and the provenance

Bundle version 5 is version 1's text bundle plus a `dependencies` object: `ecosystem`, `runtime`, `manifestDigest`,
`lockDigest`, `unpacker`, and `layers`, a map from required target to digest, size and file count. The control plane
parses and checks it in `crates/infra-schema/src/bundle.rs` (a closed key set added to `decoded_files` and
`Bundles::check`).

The finishing transaction of a bundle version 5:

1. reads the current snapshot identity of the ecosystem `FOR SHARE`;
2. if it differs from the one the version was evaluated against, re-runs the evaluation on the stored metadata (no
   input or output) and refuses the version if the verdict changed;
3. builds the SBOM from the lock and the stored metadata (CycloneDX 1.5, a component per locked package with package
   URL, version, hash and declared licence) into `script_version_sboms` (at most 4 MiB; the artifact's own `sbom` column
   keeps its 256 KiB bound and the platform component);
4. builds the provenance statement of Architecture decision 0057 decision 6 into `script_build_provenance` (the table
   feature 023 specifies; this design's migration creates it), with the lock entries as resolved dependencies and the
   runner-reported fields (layer digests, toolchain) marked as reported;
5. signs the bundle as every bundle is signed (feature 005, Decision 98) and makes the version `READY`.

A snapshot import takes the same snapshot row `FOR UPDATE`, so an import and a finishing transaction are ordered: either
the import sees the version's locked packages and blocks the artifact, or the finish sees the new snapshot.

### 10. Private packages

A tenant administrator reserves names and npm scopes (`POST /v1/package-reservations`, `package:manage`). Only a reserved
name can be uploaded: `PUT /v1/packages/{ecosystem}` with a wheel or tarball of at most 100 MiB, on a router of its own
with that body limit, by a holder of `package:upload` (an administrator, or a machine identity an administrator grants it
to, for a tenant's CI). The file goes to the package store first; then the row of `private_packages` (normalised name,
version, hash, uploader) is inserted. The same file again answers `200`; a different file for an uploaded version
answers `409`. Orphan files are collected by the store pass. Uploads are tenant-wide; a namespace's policy can still deny
or allow them.

A reserved name resolves only from the tenant's uploads, for every namespace of the tenant. Creating a reservation
inserts a re-evaluation of the tenant: a stored lock that took that name from a public source gets a finding and a block
for every namespace, because it may be the dependency confusion itself. Every reservation, upload and deletion is
audited with the name.

### 11. Materialization and the run

A version with packages runs on a dependency runtime line: a pinned glibc-based image of the same interpreter minor as
the framework-only line (`python-3.13-glibc`, `node-24-glibc`), because native wheels and prebuilt npm binaries are most
often published for glibc. Framework-only versions keep their Alpine images and their build keys. The runner's image
configuration gains the dependency runtime, resolver and layer-tool images (`SAGAT_RUNNER_IMAGE_*`).

The control plane offers a runner only the script activities whose artifact has a layer for the runner's target, and
only to runners that declare `script-dependencies-v1` (Requirement 046-020). Before it starts a run of a bundle version 5,
the runner:

1. looks for its volume for (tenant, layer digest) whose marker names the digest; the volume name carries the runner's
   instance id, so two runners on one engine never share a volume;
2. if none exists, asks the control plane for a materialization (`POST /script-runner/materializations`: the artifact
   digest and target; the answer is the lock bytes, which must hash to the signed lock digest, and a job token), fetches
   the files into its file cache, and runs the install and, for Python, the compile containers into a new volume. The
   reported digest must equal the signed one; otherwise the volume is removed and the attempt fails
   `SCRIPT_UNTRUSTED_ARTIFACT`. A materialization runs before the run's wall clock starts, under the lease's heartbeat,
   within 10 minutes, at most two at a time per runner, one at a time per (tenant, digest);
3. holds the volume against eviction from lookup until the run container is removed.

The run container gets the volume read-only at `/opt/sagat/deps`. The unpacker already refuses set-user-id bits and
special files, and `no-new-privileges` is set; the probe of Task 046-16 records from `/proc/self/mountinfo` whether the
engine also applies `nosuid` and `nodev` to the volume. The layer is not `noexec`, because native extensions are mapped
executable. Its files are fixed by digest and nobody can write them during a run.

The shims add the layer to the module path. Python inserts `/opt/sagat/deps/site-packages` in `sys.path`
(`PYTHONDONTWRITEBYTECODE=1` is already set). Node resolves the script's bare specifiers from the layer's
`node_modules`, through a link from the scratch area's script directory that the shim writes before the script loads,
for both ES modules and CommonJS. The probe proves both forms of import and a native extension in each language.

Volumes and files are kept under least-recently-used bounds (Non-functional requirement 046-004); a held volume is never
removed. A runner that restarts checks a volume's marker once, with a layer-tool container, before it trusts it. A runner
may materialize ahead when it sees a ready activity whose layer it lacks.

Warm containers (feature 019, Architecture decision 0052, Proposed) key a container by artifact digest, which names the
layer. If feature 019 ships them, a warm container keeps the read-only mount for its life, the reset between
invocations does not touch it, and a new block on its artifact evicts it. Without feature 019 nothing here changes.

### 12. Blocking at lease

The engine leases a script step only if no block row matches its artifact digest and its namespace (or null). The check
is part of the lease statement for script activities in `crates/infra-postgres/src/workers.rs`, reached from
`crates/application/src/worker.rs` (`lease_one`): one statement, no row lock. Feature 023's language check joins its
`languages` section into the same statement (Decision 1627 of feature 023). A step whose artifact is blocked is failed
in the lease transaction as non-retryable `SCRIPT_DENIED` with reason `PACKAGE_BLOCKED`, its journal event, next work,
outbox rows and a `script.supply_chain_blocked` audit record. An attempt already leased finishes. Design 038's budget
test gains a script-step shape.

### 13. Where the code goes

| Part | Place | Action |
|---|---|---|
| Policy, finding and verdict types; verdict combination | `crates/domain/src/scripts/packages.rs` | Create |
| `PackageRules`, `PackageLocks` and `PackageMetadata` ports; publication flow; dependency jobs; passes; waivers; reservations | `crates/application/src/scripts/packages.rs`, `crates/application/src/sandbox.rs` (`BuildRequest`, `RunRequest` gain the dependency fields; build key v2) | Create, Modify |
| Lock parsers, version orders, advisory matching, SBOM and provenance builders | `crates/infra-packages` (new adapter crate) | Create |
| Bundle version 5 | `crates/infra-schema/src/bundle.rs` | Modify |
| Tables, statements, the lease check | `crates/infra-postgres` (migrations after `20270119000000`, `workers.rs`, `scripts.rs`, `storage-classification.json`) | Create, Modify |
| REST: draft manifest, version packages, SBOM, policy, waivers, findings, reservations, uploads; runner routes | `crates/api-http/src/routes/{scripts,script_packages,runner}.rs`, `crates/api-http/src/lib.rs` | Create, Modify |
| Streaming send | `crates/infra-egress/src/lib.rs` | Modify |
| Container driver: resolve, install, compile and check containers; volumes; the read-only mount | `crates/infra-sandbox/src/docker.rs`, `crates/infra-sandbox/src/protocol.rs`, `crates/infra-sandbox/src/lockfile.rs` | Modify |
| Runner: dependency jobs, channel forwarding, materialization, caches, configuration | `apps/sandbox-runner/src/{lib,config,api}.rs` | Modify |
| The gateway | `apps/package-gateway` | Create |
| Images | `runners/resolver-python`, `runners/resolver-node`, `runners/layer-tool`, `runners/python-glibc`, `runners/node-glibc` | Create |
| Compose and Helm | `infra/docker/compose.yaml`, `infra/docker/Dockerfile`, `infra/helm` | Modify |
| Studio and CLI | `frontend/apps/studio` (script editor), `apps/cli` | Modify |

`scripts/check-architecture.py` gains three checks with negative tests: a network mode other than `none` appears in no
container configuration of `crates/infra-sandbox`; the run container's configuration has exactly one added mount, read-only
(a unit test on the configuration builder); only `apps/package-gateway` and `crates/infra-egress` name the package store's
settings. The rows go into `docs/engineering/architecture-principles.md`.

### 14. Permissions

| Permission | Holders by default | Grants |
|---|---|---|
| `script:read` | Viewer, Operator, Developer, Admin | the lock summary, findings, blocks, SBOM of the namespace's versions |
| `script:write`, `script:publish` | Developer, Admin | the manifest and the lock of a draft (feature 005) |
| `script:policy:manage` | Admin | the namespace's script policy and waivers |
| `package:manage` | Admin | the tenant's reservations; deleting an upload |
| `package:upload` | Admin; grantable to a machine identity | uploading a reserved name |
| `advisory:import` | the platform operator | the offline snapshot import and the forced import |

### 15. Telemetry and audit

Metrics (closed labels): `script_dependency_jobs_total{phase,ecosystem,outcome}` (`outcome`: `ok`, `refused`, `failed`,
`retried`), `script_dependency_job_seconds{phase,ecosystem}`, `script_layer_bytes{ecosystem}`,
`script_layer_cache_total{result}` (`hit`, `miss`, `evicted`), `script_layer_materialize_seconds{ecosystem}`,
`package_gateway_requests_total{ecosystem,kind,result}` (`kind`: `metadata`, `file`, `internal`; `result`: `ok`,
`not_found`, `refused`, `upstream_error`), `package_gateway_upstream_seconds{ecosystem}`,
`script_supply_chain_findings_total{kind,severity}`, `script_supply_chain_lease_refusals_total`,
`package_advisory_snapshot_age_seconds{ecosystem}`. Spans on each job, gateway request and evaluation carry tenant,
namespace, script, version and package names. Audit records: `script.dependencies_resolved`,
`script.supply_chain_refused`, `script.supply_chain_blocked`, `script.supply_chain_unblocked`,
`script_policy.changed`, `supply_chain_waiver.added`, `supply_chain_waiver.expired`, `supply_chain_waiver.removed`,
`package.reserved`, `package.uploaded`, `package.deleted`, `advisory_snapshot.imported`, `advisory_snapshot.forced`.

### 16. Compose, deployment and operations

`infra/docker/compose.yaml` gains the service `package-gateway` (built from `infra/docker/Dockerfile`, target
`package-gateway`; the hardening anchor; its package listener published on
`127.0.0.1:${SAGAT_PACKAGE_GATEWAY_PORT:-18095}`; its internal listener not published; a healthcheck on `/healthz`;
`SAGAT_PACKAGE_GATEWAY_MODE` `online` or `offline`; a named volume for a filesystem package store in development). The
sandbox runner gets the gateway URL and a named volume for its file cache. No extra network is needed: resolve
containers have none. The Helm chart gains the gateway with a network policy that lets runners and the control plane
reach it and lets only the gateway leave the cluster to its upstreams. The runbook
`docs/runbooks/script-dependencies.md` covers policy, waivers, reservations, advisory imports, offline bundles, the cache
bounds and a blocked version.

### 17. Compatibility and rollout

Everything is additive. With the capability not activated, nothing changes. Activation requires every control plane and
engine to list it (Architecture decision 0082); runners declare it per claim. Rolling back the binaries after bundle
version 5 rows exist leaves versions an old runner cannot run (it refuses an unknown bundle version with
`SCRIPT_UNTRUSTED_ARTIFACT`); the release notes say rollback is forward-only for data, as Architecture decision 0057 says.

### 18. Delivery phases

All phases are in 0.3.0; each has its own acceptance, so the feature can be reviewed and merged in parts.

- **Phase A, the core:** the capability gate first, manifests, the gateway for public packages, resolve, validation,
  install, materialization, bundle version 5, SBOM and provenance, quarantine, deny and allow rules, malware refusal at
  publication with the snapshot import. Acceptance: Success criteria 046-001 and 046-002 with the default policy.
- **Phase B, policy after publication:** vulnerabilities, licences, waivers, re-evaluation, blocks at lease.
  Acceptance: Requirements 046-009 to 046-011.
- **Phase C, private packages:** reservations, uploads, the private-name rule. Acceptance: Requirement 046-017.
- **Phase D, offline:** offline mode, signed store bundles, store collection. Acceptance: Success criterion 046-003.
- **Phase E, surfaces and proof:** Studio, CLI, runbook, benchmark. Acceptance: Success criterion 046-004.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A malicious package published upstream is resolved before anyone knows | Quarantine of 72 hours by default; malicious-package advisories refused and blocked in the import transaction; the gateway is the only path, so a policy change applies everywhere at once |
| The gateway is down | Publication fails retryable; versions already published keep running from their volumes, and runners materialize from the store only when they lack a volume; the gateway is stateless and can run several replicas |
| The gateway is compromised, or its public store poisoned | It holds no tenant secret and only the package store's credential; every file is checked against the lock's hash by the runner; the unpacker cross-checks install scripts; a poisoned store cannot change a locked file without changing its hash |
| A package needs its install script | Admitted with a finding; the author sees `INSTALL_SCRIPT_SKIPPED`; Gap 046-001 |
| The unpacker is exploited by a crafted archive | It runs in a sandbox container with no network; its output is treated as package content, which runs only in that tenant's sandbox |
| Disk on runners fills with layers | Least-recently-used bounds; volumes per tenant; cache metrics; Gap 046-004 |
| Re-evaluation cost grows with the locks stored | Paged, leased, resumable by cursor; malware blocks do not wait for it; its backlog is a metric |
| The resolver tools change behaviour | Pinned by image digest, part of the build key; the probe runs in CI on both targets |
| The advisory snapshot is stale or truncated | Age metric and warning; older and shrinking imports refused unless forced |
| Side channel through the public store | Accepted for public content (Gap 046-007); private files and volumes are per tenant |

## Constitution Check

Written against version 5.0.0 (Owner decision 2026-11-24-2, pull request #275). That amendment changes no principle this
design relies on; the rows below hold under 4.7.0 as well.

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Dependency jobs have the existing lease, heartbeat and retry; each phase change is one transaction fenced by the job token; a version becomes `READY` only in the finishing transaction, ordered against snapshot imports by the snapshot row; re-evaluation is a durable job with a cursor; a blocked step fails in the lease transaction with its journal event and outbox |
| II. Modular domain, ports and adapters | Pass | Policy types and verdicts in the domain; version math behind application ports in a new adapter crate; checks added to `scripts/check-architecture.py`. The gateway is a separate process before measurement because it is the only internet-facing component and must not share a process or a credential with anything that reads tenant data (Principle V); Complexity Tracking |
| III. External effects are at-least-once; sagas compensate | Not applicable | No external effect of a run is added; a script's own effects keep feature 005's idempotency key |
| IV. Compatibility is versioned and pinned | Pass | A version pins its lock and layer digests in the signed bundle; bundles 1 to 3 unchanged; resolvers, unpacker and runtimes pinned by digest and in the build key; framework-only build keys unchanged |
| V. Code, tenant boundary and secrets are isolated | Pass with conditions | No package code runs outside a sandbox container; every sandbox container keeps no network; the run container gains one read-only mount (Complexity Tracking); volumes, uploads and reservations are per tenant; identical `404`s across tenants; the gateway holds no tenant secret; Gap 046-002 keeps external private registries out |
| VI. Observability and audit are part of the feature | Pass | Technical Plan 15 |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | No new message path; a blocked step's failure writes its outbox rows in the lease transaction |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Hostile battery, crashes at each phase change and during materialization, the import and finish race, duplicate completions, gateway outage, offline build, re-evaluation resume (Verification Plan); gaps registered |
| Delivery Constraints | Pass | Self-contained compose service, pinned images, loopback port, healthcheck; no new region or transport; targets stated as benchmark criteria |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A new service, the package gateway, separate before measurement (Principle II) | One audited path to registries that applies the policy, works offline, and is the only process with internet egress and the package store's credential | Each runner talking to registries: many network paths, no shared policy, no offline mode |
| HTTP over the supervisor channel for the resolve container | A resolver needs registry metadata, and the sandbox keeps no network | A container network to the gateway: a shared bridge between tenants, host-local only, a token inside the container |
| One read-only mount in the run container (Principle V; Architecture decision 0013's "nothing mounted") | A dependency tree does not fit a 2 MiB bundle and must not be downloaded by a run | Streaming the tree into the scratch area at each run: a cost per run and a writable copy |
| Bundle version 5 | The bundle must name the layer so the existing signature binds it | Signing the layer separately: a second signature path |
| A dependency-job table and a build key version 2 | A version needs resolve and per-target install before its build, and the key must cover the lock | Reusing the build-job row: one row per version cannot hold three phases |
| A block table beside immutable artifacts | Blocks change; artifacts must not | An updatable column on the artifact row: refused by `script_artifacts_immutable` |
| An advisory snapshot and two version orders | The policy needs facts about vulnerabilities and malware without calling a third party per publication | An online scanning service: no offline mode, a third party sees every tenant's packages |
| Dependency runtime images on glibc | Native wheels and prebuilt binaries are most often published for glibc | Keeping Alpine: many common packages would be refused |

## Alternatives Considered

- **A closed catalog for TypeScript and Python** (Architecture decision 0021's model). Strongest isolation, but a
  catalog cannot serve the ecosystems; kept for the other languages.
- **Install at run time with network** (Windmill). A network path in every run, install scripts, and a registry outage
  breaks runs.
- **A third-party repository manager as the proxy** (Nexus, Artifactory, Verdaccio, devpi). A large component with its
  own policy model, an image to pin and patch, a licence question for some, and no knowledge of namespaces. An operator
  may still chain one as the gateway's upstream.
- **A container network from the resolve container to the gateway** (the first draft). Rejected after review: host-local
  networks, traffic between tenants' containers, a token in the container.
- **Store layers in the platform's blob store and have runners download them.** Gigabytes through the control plane per
  runner and a second copy of what the package store holds; materializing from the store with the signed digest as the
  check is cheaper and proves reproducibility on every runner.
- **A layer packaged as an image, or an image volume** (Architecture decision 0120 option 5). Clean on Kubernetes, but it
  needs an image builder or a registry on the runner host; deferred behind the volume port.
- **Infer the manifest from the imports** (Windmill). A guessed name is a typosquatting path (Decision 046-003).
- **Run the installer (`uv pip install`, `npm ci`) in the install container.** An installer is a large program whose
  behaviour changes by version; unpacking verified archives is small and deterministic once install scripts and source
  builds are excluded.
- **One shared volume per digest for all tenants.** Saves disk; rejected for the cache side channel and feature 005's
  rule that the cache belongs to the tenant.
- **Build source distributions in the sandbox.** Possible later (Gap 046-001); it needs its own resolved build backend.
- **Proxy tenants' private registries with their credentials.** Needs a new disclosure path; uploads instead (Gap
  046-002).

## Non-Functional Requirements

As stated under Requirements. Their tests: the hostile battery for Non-functional requirement 046-001;
`layer_digest_is_identical_on_two_hosts` for Non-functional requirement 046-002; the benchmark of Task 046-23 for
Non-functional requirement 046-003; the bound fixtures of Tasks 046-3, 046-4 and 046-13 for Non-functional requirement
046-004; the metric-label test of Task 046-21 for Non-functional requirement 046-005.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Documentation first: the contracts and the data model are written at this commit
([contracts](../../specs/046-script-dependencies/contracts/), [data-model.md](../../specs/046-script-dependencies/data-model.md));
Task 046-1 lands the rest of the documents before any code. Each task names the files it touches; its tests are written
first and seen failing. Migrations take timestamps after `20270119000000`, allocated by the coordinating session.

Two builders work at once on crates that do not overlap. Builder A owns `crates/domain`, `crates/application`,
`crates/infra-packages`, `crates/infra-postgres`, `crates/infra-schema` and `crates/api-http`. Builder B owns
`apps/package-gateway`, `crates/infra-egress`, `crates/infra-sandbox`, `apps/sandbox-runner`, `runners/` and `infra/`.
The contracts are the interface between them. Order and dependencies, by phase:

| Step | Builder A | Builder B | Depends on |
|---|---|---|---|
| A1 | Tasks 046-2 (capability, types), 046-3, 046-4 | Tasks 046-6, 046-12 (probe first) | Task 046-1 |
| A2 | Tasks 046-5, 046-10, 046-15 | Tasks 046-7, 046-13 | A1 |
| A3 | Task 046-14 (runner routes) | Task 046-16 | A2 |
| B | Tasks 046-11, 046-17 | Task 046-21 (gateway and runner metrics) | Phase A |
| C | Task 046-9 (control plane) | Task 046-8 (gateway side of private names) | Phase A |
| D | Task 046-18 (collection pass) | Task 046-20 | Phase A |
| E | Tasks 046-19, 046-22, 046-23, 046-24, either builder | | Phases A to D |

Feature 023 depends on Tasks 046-2 (the policy table), 046-15 (the provenance and SBOM module) and 046-17 (the lease
check), and extends them (feature 023, Decision 1627). Design 038's budget test is run by Task 046-17.

- [ ] T046-1 Land the documents not yet written: the OpenAPI additions (`contracts/openapi.yaml`), the runbook draft,
  the row of Gap 97 in `docs/architecture/security-and-sandbox.md`, the cross-references in feature 023 and feature 005.
  Test: `contract_script_dependencies` lints the OpenAPI.
  Refs: Requirement 046-007, Requirement 046-013, Requirement 046-019
- [ ] T046-2 Domain and application types (manifest, lock entries, policy, findings, verdict, blocks), the capability
  `script-dependencies-v1` (`Capability` variant and the registry CHECK), the migrations of data-model.md with grants
  and storage classification, `script_runtime_policies` and its endpoint. Files: `crates/domain/src/scripts/packages.rs`,
  `crates/application/src/instances.rs`, `crates/infra-postgres/migrations/`, `crates/api-http/src/routes/script_packages.rs`.
  Tests: `verdict_order_*`, `without_the_capability_a_manifest_is_refused`, the privilege audit of
  `integration_platform_foundation`.
  Refs: Requirement 046-001, Requirement 046-011, Requirement 046-020
- [ ] T046-3 npm lock parser and validator in `crates/infra-packages/src/npm.rs`: closed subset, hashes, canonical paths,
  closure against declared dependencies, target selection, manifest satisfaction. Tests: `lock_rules_npm_*`, one refused
  fixture per rule, the bounds.
  Refs: Requirement 046-002, Requirement 046-007, Non-functional requirement 046-004
- [ ] T046-4 PEP 751 parser and validator in `crates/infra-packages/src/pypi.rs`: wheels only, tags against targets,
  PEP 440 and PEP 508, normalised names. Tests: `lock_rules_pypi_*`, the bounds.
  Refs: Requirement 046-002, Requirement 046-005, Requirement 046-007, Non-functional requirement 046-004
- [ ] T046-5 Advisories: the OSV model, the import pass and command with the older and shrinking checks, the malware
  blocks in the import transaction, the matchers (property tests against both version orders), the severity mapping,
  the snapshot age metric. Files: `crates/infra-packages/src/advisories.rs`, `crates/application/src/scripts/packages.rs`.
  Tests: `matcher_agrees_with_the_version_order_*`, `a_snapshot_marking_a_locked_package_malicious_blocks_its_next_lease`,
  `an_older_snapshot_is_refused`.
  Refs: Requirement 046-008, Requirement 046-009
- [ ] T046-6 Gateway, npm side and its skeleton: two listeners, tokens (`jsonwebtoken`, EdDSA), package documents and
  tarballs, filtering with the domain rules, the package store, the streaming send in `crates/infra-egress`, the
  healthcheck. Files: `apps/package-gateway/`, `crates/infra-egress/src/lib.rs`. Tests against recorded registry
  responses (`apps/package-gateway/tests/fixtures/npm/`, refreshed by `scripts/record-registry-fixtures.sh`):
  `filters_versions_younger_than_the_quarantine`, `refuses_a_redirect_to_another_host`, `validates_names_before_upstream`.
  Refs: Requirement 046-004, Requirement 046-018
- [ ] T046-7 Gateway, PyPI side: the JSON simple API, core-metadata files, wheel-only and tag filtering, files by hash,
  the metadata endpoint. Tests: recorded PyPI fixtures; `never_serves_a_source_distribution`.
  Refs: Requirement 046-005, Requirement 046-007
- [ ] T046-8 Gateway side of private packages: tenant private keys, reserved names hidden from public results, identical
  `404`s. Tests: `a_reserved_name_never_resolves_from_the_public_registry` (gateway part), `another_tenants_file_is_not_found`.
  Refs: Requirement 046-017
- [ ] T046-9 Control-plane side of private packages: reservations, the upload router and its body limit, the store
  write then the row, normalised uniqueness, re-evaluation on reservation, audit; `sagat package push` in `apps/cli`.
  Tests: `an_identical_upload_is_accepted_again`, `a_different_file_for_an_uploaded_version_is_a_conflict`,
  `a_reservation_blocks_a_lock_that_took_the_name_from_public`.
  Refs: Requirement 046-017
- [ ] T046-10 Publication flow: `script_dependency_jobs`, the claim, heartbeat and completion routes, the fenced phase
  transactions, the build key version 2, the evaluation on every publication. Files:
  `crates/application/src/scripts/packages.rs`, `crates/application/src/sandbox.rs`, `crates/infra-postgres/src/scripts.rs`.
  Tests: `framework_only_versions_keep_their_build_key`, `a_cached_build_is_evaluated_again`,
  `a_crash_between_resolve_and_install_leaves_a_retryable_job`, `a_duplicate_completion_changes_nothing`.
  Refs: Requirement 046-002, Requirement 046-008, Requirement 046-019
- [ ] T046-11 Re-evaluation: `script_policy_reevaluations`, the leased pass with its cursor, the waiver pass, block
  writes and removals. Tests: `policy_change_reaches_the_lease`, `waiver_expiry_reaches_the_lease`,
  `reevaluation_resumes_after_a_crash`.
  Refs: Requirement 046-010
- [ ] T046-12 Resolver images and shims, the loopback server and the `http` channel messages, the runner's forwarding
  with its bounds; the probe that fixes the command lines, measures real locks and proves no request outside the
  gateway, on both targets. Files: `runners/resolver-python/`, `runners/resolver-node/`,
  `crates/infra-sandbox/src/protocol.rs`, `apps/sandbox-runner/src/lib.rs`. Tests: `probe_resolves_the_acceptance_packages`,
  `a_resolve_container_reaches_only_the_gateway_routes`.
  Refs: Requirement 046-004, Requirement 046-005, Requirement 046-006
- [ ] T046-13 The layer tool and the install job: the unpacker, its refusals and bounds, the digest, the install-script
  report, the compile container, the marker. Files: `runners/layer-tool/`, `crates/infra-sandbox/src/docker.rs`,
  `apps/sandbox-runner/src/lib.rs`. Tests: `unpacker_refuses_*`, `layer_digest_is_identical_on_two_hosts`,
  `compile_imports_nothing_from_the_layer`.
  Refs: Requirement 046-006, Requirement 046-012
- [ ] T046-14 Runner routes in the control plane: dependency-job claim with capability and target, materializations,
  test-run dependencies; target-aware pending activities. Files: `crates/api-http/src/routes/runner.rs`,
  `crates/application/src/scripts/packages.rs`. Tests: `an_old_runner_claim_gets_no_dependency_work`,
  `a_runner_of_another_target_is_not_offered_the_step`.
  Refs: Requirement 046-016, Requirement 046-020
- [ ] T046-15 Bundle version 5 in `crates/infra-schema/src/bundle.rs`, the finishing transaction with the snapshot
  check, the SBOM into `script_version_sboms`, the provenance into `script_build_provenance` (the module feature 023
  reuses). Tests: `a_snapshot_imported_during_a_build_is_seen_before_signing`, `sbom_package_set_equals_the_lock`,
  `provenance_verifies_and_marks_reported_fields`.
  Refs: Requirement 046-009, Requirement 046-013, Requirement 046-015
- [ ] T046-16 Materialization and the run: dependency runtime images, volumes per tenant and runner, single flight, the
  hold against eviction, the marker check, the read-only mount, the shims for both module systems and native
  extensions, the caches. Files: `runners/python-glibc/`, `runners/node-glibc/`, `crates/infra-sandbox/src/docker.rs`,
  `apps/sandbox-runner/src/{lib,config}.rs`. Tests: `two_tenants_with_one_lock_get_two_volumes`,
  `an_unmarked_volume_is_never_mounted`, `two_runs_that_miss_populate_once` (barrier), `a_script_cannot_write_the_layer`,
  `native_extension_imports_*`.
  Refs: Requirement 046-013, Requirement 046-014
- [ ] T046-17 The lease check: `script_artifact_blocks` in the lease statement of `crates/infra-postgres/src/workers.rs`,
  `SCRIPT_DENIED` with `PACKAGE_BLOCKED` in the lease transaction, design 038's budget test with a script-step shape.
  Tests: `a_blocked_artifact_fails_at_lease`, `an_attempt_already_leased_finishes`.
  Refs: Requirement 046-009, Requirement 046-019
- [ ] T046-18 Store collection pass and fetch budget. Tests: `an_unreferenced_file_is_collected_after_30_days`,
  `a_referenced_file_is_never_collected`, `the_fetch_budget_refuses_retryable`.
  Refs: Requirement 046-018
- [ ] T046-19 Studio panel (manifest, lock, findings, blocks, policy) and CLI (`sagat script packages`). Files:
  `frontend/apps/studio/src/features/scripts/`, `apps/cli`. Tests: the Studio journey test, the CLI contract tests.
  Refs: Requirement 046-021
- [ ] T046-20 Offline mode, signed export and import; compose service, Dockerfile target, Helm values and network
  policy; the compose smoke test. Tests: `offline_build_from_an_imported_bundle`, `an_unsigned_bundle_is_refused`,
  `scripts/smoke-compose.sh` with the gateway.
  Refs: Requirement 046-018
- [ ] T046-21 Telemetry and audit of Technical Plan 15, with the metric-label test.
  Refs: Non-functional requirement 046-005
- [ ] T046-22 The hostile battery and the fault tests of the Verification Plan, on both targets, in the native-runner
  CI job (`integration_script_dependencies_containers`).
  Refs: Requirement 046-004, Requirement 046-012, Requirement 046-014, Non-functional requirement 046-001
- [ ] T046-23 Benchmark report (`tests/performance/reports/script-dependencies.md`) with environment and failures.
  Refs: Non-functional requirement 046-003, Success criterion 046-004
- [ ] T046-24 Runbook, reference pages, quickstart, release notes; Gap 97 of feature 005 marked closed for TypeScript
  and Python with the evidence.
  Refs: Requirement 046-021

#### Follow-up tasks

None yet.

### Verification Plan

- **Unit and property tests:** lock validators (Tasks 046-3 and 046-4), matchers against both version orders (Task
  046-5), verdict order (Task 046-2), unpacker refusals (Task 046-13), the run configuration's single read-only mount.
- **Integration (real PostgreSQL, a real gateway with recorded upstream fixtures):** publication with each manifest form
  and an author lock; each refusal reason; `a_cached_build_is_evaluated_again`; the snapshot import that blocks and the
  lease that fails; `a_snapshot_imported_during_a_build_is_seen_before_signing` (an import held at a barrier between
  resolve and finish); policy change, waiver expiry and reservation reaching the lease; private names with case and
  separator variants; offline mode with an imported bundle; the test-run path.
- **Real containers (`integration_script_dependencies_containers`, native-runner CI on AMD64 and ARM64):** the hostile
  battery. A resolve container tries the internet, the control plane, the engine, the runner host, another container,
  the cloud metadata address and name resolution (all fail), and a channel request to a path outside the gateway's
  package routes (refused). A package with a `postinstall` and one with only a `setup.py` (neither runs). A script writes
  into the layer (refused). Two tenants with one lock (two volumes). A tampered lock, bundle, layer and another tenant's
  file (refused before start). A native extension imports in each language. `layer_digest_is_identical_on_two_hosts`
  compares the digests of the two CI hosts.
- **Faults:** the runner dies during resolve, install, compile and materialization (the job is retried; an unmarked
  volume is never mounted and is removed); the gateway refuses, times out and answers `503` (retryable, no version
  change); a duplicate completion (`AlreadyDone`, nothing changes); the control plane dies between the import commit and
  the first re-evaluation page, and during a page (the pass resumes at its cursor); two runs miss the same volume at a
  barrier (one materialization).
- **Contracts:** OpenAPI lint and conformance; the gateway's contract against recorded npm and PyPI responses.
- **Benchmark:** Task 046-23.
