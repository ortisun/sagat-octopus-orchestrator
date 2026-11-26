# Package dependencies for TypeScript and Python scripts

*   **Author:** Claude Code, for the owner
*   **Status:** In Review
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

From release 0.3.0 a TypeScript or Python script can use packages from npm and PyPI, and private packages its tenant
uploads to the platform. The author declares what the script needs: in Python, a PEP 723 block at the top of the source;
in TypeScript, a `package.json` of dependencies kept beside the source. When the author publishes:

1. The platform resolves the declaration into a lock: every package with an exact version and a content hash.
2. It checks the lock against the namespace's supply-chain policy and the platform's advisory snapshot.
3. It signs a bundle that names the lock, and, for each native target, a layer record that names the digest of the
   unpacked packages.

A run never downloads anything. Before a runner offers to run such a version, it builds the layer from the verified
package files, checks that it has the signed digest, and keeps it. The run container gets the layer read-only and still
has no network.

One component reaches the public package registries: the **package gateway**, a platform service declared in this
repository's compose. It holds only public files. Resolution runs in a sandbox container with no network at all: the
resolver's requests travel over the sandbox's supervisor channel to the runner, which forwards them. Installation runs no
installer and no package code. Private packages never touch the gateway: they live in the tenant's payload store and are
served by the control plane.

Windmill, the reference product for the 0.3.0 parity scope (Owner decision 2026-11-24-1), resolves imports at deployment
and installs packages on its workers from a registry. This design gives the same convenience with guarantees Windmill does
not give: no network in a resolution or a run, no package code before the run, a policy at publication, a stop for
malware found later, a software bill of materials (SBOM) and signed provenance for every build, offline builds, and
protection against dependency confusion.

The architecture is
[Architecture decision 0120](../adr/ADR-0120-script-packages-are-resolved-at-publication-through-the-package-gateway-and-run-from-a-signed-read-only-layer.md);
the policy and its effect on published versions is
[Architecture decision 0121](../adr/ADR-0121-a-namespace-supply-chain-policy-gates-publication-and-a-malicious-package-advisory-stops-leases.md).
Both are Proposed. The registers are in [research.md](../../specs/046-script-dependencies/research.md); the contracts
and the data model are under [specs/046-script-dependencies/](../../specs/046-script-dependencies/). Principle numbers
refer to [the constitution](../constitution.md).

This design closes Gap 97 of feature 005 for TypeScript and Python
([feature 005 research](../../specs/005-inline-code-sandbox/research.md)) and replaces its Decision 105 for those two
languages only. C# keeps its closed catalog
([Architecture decision 0021](../adr/ADR-0021-csharp-builds-use-a-curated-offline-toolchain.md)); Rust and PHP keep theirs
([feature 023](../../specs/023-script-languages/spec.md),
[Architecture decision 0057](../adr/ADR-0057-additional-script-languages-reuse-the-sandbox-and-signed-artifacts.md)).

### Terms

Terms of this design:

- **Manifest.** What the author declares: package names and version ranges (a PEP 723 block; a `package.json` with only
  `dependencies` and `overrides`).
- **Lock.** The resolved closure: every package, its exact version, where it is served and its content hash (a PEP 751
  `pylock.toml` with wheels only; an npm `package-lock.json` with `lockfileVersion: 3`). Its rules are in
  [lock-formats.md](../../specs/046-script-dependencies/contracts/lock-formats.md).
- **Package gateway (the gateway).** `apps/package-gateway`: a read-only proxy of the public npm registry protocol and of
  the Python simple repository API (PEP 691), with a content-addressed store of public files
  ([package-gateway.md](../../specs/046-script-dependencies/contracts/package-gateway.md)).
- **Job grant.** A short token the control plane signs for one attempt of one job, which the runner adds to the requests
  it forwards. It carries the job's quarantine cut, an absolute time on the control plane's clock.
- **Resolve job, install job.** Runner jobs of a version with packages. A resolve job runs the pinned resolver and returns
  the lock. An install job, one per required target, unpacks the locked files and returns the layer's digest.
- **Layer, layer record.** The unpacked packages for one native target (`linux-amd64` or `linux-arm64`), named by the
  digest of a canonical serialization; and the control plane's signed statement that a given artifact's layer for a
  target has that digest ([bundle-v5.md](../../specs/046-script-dependencies/contracts/bundle-v5.md)).
- **Materialization.** A runner building a layer into a local engine volume and checking its digest, before it offers to
  run the version.
- **Required targets.** The native targets a deployment serves (`SAGAT_SCRIPT_DEPENDENCY_TARGETS`, default
  `linux-amd64`).
- **Supply-chain policy (the policy).** The `packages` section of the namespace's script policy document (Technical
  Plan 6).
- **Advisory snapshot.** The platform's copy of the OSV (Open Source Vulnerabilities) records of npm and PyPI, for
  vulnerabilities and malicious packages (Technical Plan 7).
- **Finding, block.** A policy result on one locked package; a row that stops the engine from leasing an artifact's
  script steps, for one namespace or for all.
- **Reserved name.** A package name or npm scope a tenant reserved; it resolves only from the tenant's uploads.
- **Hostile battery.** The real-container test suite of the Verification Plan that tries every escape this design
  closes.

Terms of the platform (defined by feature 005 unless said otherwise):

- **Build key.** The digest of everything a build depends on, computed when a runner claims the build (it includes the
  runner's toolchain); a ready version with the same key is reused (Decision 99).
- **Bundle.** The canonical JSON artifact the control plane signs. Versions 1 (TypeScript and Python text), 2 (C#) and 3
  (Go and Java) exist in `crates/infra-schema/src/bundle.rs`; version 4 is planned by feature 023 for PHP.
- **Framework-only version.** A TypeScript or Python version without packages, as feature 005 builds today.
- **Sandbox profile.** Decision 95's container settings: no network, read-only root, a small `noexec` scratch area, no
  capabilities, `no-new-privileges`, user 65534, resource limits. Feature 035's lake profile
  (`crates/infra-sandbox/src/lake.rs`) already adds one volume mount to it.
- **Supervisor channel.** The JSON-line channel between a sandbox container and its runner (Decision 96), whose messages
  have an `op` field; a script's mediated request is the `fetch` op.
- **Lease and admission.** The engine leases a script step to a runner; at lease it calls
  `RunTx::script_execution_admitted_instance` (`crates/application/src/execution.rs`).
- **Instance registry and capability.** The record of what each control plane and engine can do, and the operator's
  audited activation of a requirement (Architecture decision 0082, Requirement 018-052).
- **Design 038's budget test.** The test that counts the statements of a lease and a completion
  ([design 038](038-claim-transition-efficiency.md)).

## The Problem

Windmill's main convenience for scripts is that `import pandas` works. On deployment it infers requirements from the
imports and resolves them with `uv` or `bun`, stores a lockfile with the script, and its workers install the packages from
a public or private registry, with network access, caching them on disk. It applies no supply-chain policy: no
vulnerability check, no quarantine of new releases, no protection against a private name taken on the public registry,
no SBOM.

This platform does the opposite today. Feature 005 refuses every package for TypeScript and Python (Decision 105). A
script can use only the standard library and the platform's shim. Gap 97 asks for pinned dependencies, an SBOM and a
supply-chain policy; it is closed only for one C# package. An author who needs an HTTP client, a date library or pandas
writes a worker in an SDK instead, which takes the code away from the sandbox, signing and versioning.

Any answer must keep these constraints:

- The run container has no network, a read-only root and a `noexec` scratch area. A run must not download a package.
- Untrusted code must not run in the control plane, the scheduler or the runner process (Principle V). Installing a
  package can run its code: npm lifecycle scripts, a Python source distribution's build.
- A bundle is at most 2 MiB and lives in PostgreSQL (feature 005, Gap 108). A dependency tree is far larger.
- Infrastructure is self-contained in this repository (AGENTS.md rule 16): the proxy is in the compose, pinned, on
  loopback, with a healthcheck.
- A published version is immutable (Principle IV): what ran yesterday runs today, even if the registry deletes the
  package.

## Requirements

### Functional

#### Declaration and resolution

- **Requirement 046-001** A TypeScript or Python draft MAY carry a manifest. A draft without one MUST build and run as
  today, with the same build key. *Acceptance:* `framework_only_versions_keep_their_build_key`;
  `publish_and_run_with_a_pep_723_block`, `publish_and_run_with_a_package_manifest`.
- **Requirement 046-002** Publication of a version with a manifest MUST resolve it into a lock through a resolve job, or
  accept an author lock that satisfies the manifest and whose every entry is a file the gateway (or the tenant's uploads)
  serves with the same hash. Publishing a draft equal to the latest version returns that version, as today; a new
  resolution of an unchanged draft is an explicit request (`:resolve-again`). *Acceptance:*
  `an_author_lock_is_validated_synchronously`, `resolve_again_makes_a_new_version`, the `lock_rules_*` refusals.
- **Requirement 046-003** The platform MUST NOT infer a manifest from the imports. *Acceptance:*
  `an_undeclared_import_fails_at_import_time_and_changes_nothing`.
- **Requirement 046-004** A resolve job MUST run in a container with the sandbox profile and no network. Its requests
  MUST reach only the gateway's public package routes and the control plane's private index routes, forwarded by the
  runner over the supervisor channel. No token or secret enters the container. Its only output is the lock.
  *Acceptance:* `a_resolve_container_reaches_only_the_forwarded_routes` (the hostile battery).
- **Requirement 046-005** Python locks MUST contain wheels only (`PACKAGE_SOURCE_ONLY` otherwise). Every wheel MUST match
  the runtime's interpreter and a required target, and every required target MUST be served
  (`PACKAGE_PLATFORM_UNSUPPORTED` otherwise). *Acceptance:* one fixture per refusal; a lock for two targets gives a layer
  per target.
- **Requirement 046-006** npm lifecycle scripts of dependencies MUST NOT run. A package that declares one is admitted with
  the finding `INSTALL_SCRIPT_SKIPPED`, unless the policy refuses such packages. *Acceptance:*
  `a_postinstall_never_runs_and_is_reported`.
- **Requirement 046-007** The control plane MUST validate every lock against the closed subsets of lock-formats.md.
  *Acceptance:* one refused fixture per rule (`lock_rules_npm_*`, `lock_rules_pypi_*`).

#### Policy and advisories

- **Requirement 046-008** Before a build is signed, the control plane MUST evaluate the lock against the policy, the
  gateway's metadata and the advisory snapshot, refuse with the first failing reason in the order of
  [errors.md](../../specs/046-script-dependencies/contracts/errors.md), and list every finding. The evaluation runs for
  every version, also when its build is a cache hit. *Acceptance:* one fixture per reason;
  `a_cached_build_is_evaluated_and_finished_like_a_new_one`.
- **Requirement 046-009** A package matched by a malicious-package advisory MUST be refused at publication and MUST NOT
  start afterwards: not at lease, not at materialization, not just before the run container starts. No policy or waiver
  allows it. *Acceptance:* `a_malware_import_blocks_the_next_lease`, `a_malware_import_during_materialization_stops_the_start`,
  `a_snapshot_imported_during_a_build_is_seen_before_signing`.
- **Requirement 046-010** A change of the policy, an added, removed or expired waiver, a new snapshot, a new reservation
  and a deleted upload MUST each re-evaluate the stored locks they affect, through a durable job, and write or remove
  blocks; a malware block is removed only by its release route. *Acceptance:* `policy_change_reaches_the_lease`,
  `waiver_expiry_reaches_the_lease`, `reevaluation_resumes_after_a_crash`, `reevaluation_never_removes_a_malware_block`.
- **Requirement 046-011** A namespace administrator (`script:policy:manage`) MUST be able to read and change the policy
  and its rules, add and remove a waiver (one advisory, one package version, an expiry of at most 90 days, a reason), and
  list findings and blocks. A waiver MUST NOT cover a malicious-package advisory. *Acceptance:* the REST contract tests;
  `a_waiver_cannot_cover_malware`; `a_waiver_longer_than_90_days_is_refused`.

#### Installation, bundle and run

- **Requirement 046-012** Installation MUST NOT run an installer or package code. The runner MUST check every fetched file
  against the lock's own hash (SHA-512 integrity for npm, SHA-256 for wheels). The layer's digest MUST be computed by a
  fresh container that mounts the layer read-only, not by the container that unpacked it. *Acceptance:*
  `unpacker_refuses_*`, `the_digest_is_recomputed_by_a_fresh_container`, `layer_digest_is_identical_on_two_hosts`.
- **Requirement 046-013** A version with packages MUST be signed as bundle version 5, which names the lock digest, the
  unpacker and the runtime, and each of its layers MUST have a signed layer record. A runner MUST refuse to use a layer
  whose digest differs from its record. *Acceptance:* a tampered lock, bundle, record and layer each fail before the run
  container starts.
- **Requirement 046-014** A run of a bundle version 5 MUST mount its layer read-only as the only added mount, from a
  runner volume of that tenant whose completion marker names the recorded digest. Volumes are never shared between
  tenants. *Acceptance:* `a_script_cannot_write_the_layer`, `two_tenants_with_one_lock_get_two_volumes`,
  `an_unmarked_volume_is_never_mounted`.
- **Requirement 046-015** Every version with packages MUST have a CycloneDX SBOM built by the control plane from the lock
  and the gateway's metadata. Its build MUST have the provenance statement of decision 6 of Architecture decision 0057,
  naming the lock and SBOM digests and marking the runner-reported fields. *Acceptance:* `sbom_package_set_equals_the_lock`,
  `provenance_verifies_and_marks_reported_fields`.
- **Requirement 046-016** A test run of a draft with a manifest MUST resolve, be evaluated, install and run on its runner
  as a publication would, MUST NOT be signed and MUST NOT receive a secret. *Acceptance:*
  `a_test_run_with_packages_runs_and_is_refused_for_malware`.

#### Private packages and offline operation

- **Requirement 046-017** A tenant administrator (`package:manage`) MUST be able to reserve package names and npm scopes
  for the tenant. Only a reserved name can be uploaded, by a holder of `package:upload`. A reserved name MUST resolve
  only from the tenant's uploads, for every namespace of the tenant, and no request for it MUST reach a public registry;
  names are compared after normalisation. *Acceptance:* `a_reserved_name_never_reaches_the_public_registry` with case and
  separator variants; `an_identical_upload_is_accepted_again`, `a_different_file_for_an_uploaded_version_is_a_conflict`.
- **Requirement 046-018** The gateway MUST run `online` or `offline`, and MUST export and import signed store bundles.
  A version whose files are stored MUST build and run with no upstream. *Acceptance:*
  `offline_build_from_an_imported_bundle`.

#### Contracts, rollout and surfaces

- **Requirement 046-019** Failures MUST be typed and value-free, per path (errors.md). *Acceptance:* the error contract
  tests.
- **Requirement 046-020** The capability `script-dependencies-v1` of control planes and engines, activated by the
  operator, MUST switch the feature on; runners MUST declare it in their claims. Without it a manifest is refused as
  today; a runner that does not declare it is never given dependency work or a step of a bundle version 5. Activation is
  refused until the lease check of Task 046-17 is in the binaries. *Acceptance:*
  `without_the_capability_a_manifest_is_refused`, `an_old_runner_claim_gets_no_dependency_work`.
- **Requirement 046-021** The Studio MUST show the manifest, the lock summary, findings, blocks and the policy; the CLI
  MUST show a version's packages and upload private packages. *Acceptance:* the Studio journey test and the CLI
  contract tests.

### Non-functional

- **Non-functional requirement 046-001 (isolation)** No component other than the gateway reaches a public registry; no
  package code runs outside a sandbox container; the gateway holds no tenant data, no tenant secret and only the public
  store's credential. *Acceptance:* the hostile battery; `only_the_gateway_leaves_the_cluster` (Helm).
- **Non-functional requirement 046-002 (reproducibility)** The same lock and unpacker give the same layer digest on every
  runner of a target. *Acceptance:* `layer_digest_is_identical_on_two_hosts`.
- **Non-functional requirement 046-003 (performance targets, to be measured)** On the development machine: a warm run
  with a materialized layer adds at most 50 ms to the start of the same script without packages; resolving 50 packages
  with a warm gateway takes at most 30 s; materializing a 100 MiB layer takes at most 20 s. Benchmark criteria, not
  promises. *Acceptance:* the benchmark of Task 046-23.
- **Non-functional requirement 046-004 (bounds)** The bounds are in the table below. *Acceptance:* the bound fixtures of
  Tasks 046-3, 046-4 and 046-13.

| Quantity | Bound |
|---|---|
| Manifest | 64 KiB |
| Lock | 256 KiB (the existing lockfile ceiling) and 2,000 entries; the probe of Task 046-12 measures real locks for two targets |
| Layer | 1 GiB by default, 4 GiB at the platform ceiling; 200,000 files; paths of 1,024 bytes |
| Resolve job | 300 s; 1 GiB forwarded; 4,000 forwarded requests |
| Install job | 600 s |
| Runner caches | 50 GiB of volumes and 10 GiB of files by default |
| Advisory snapshot | 200,000 records per ecosystem, 64 KiB per record, 2 GiB uncompressed per archive |

- **Non-functional requirement 046-005 (telemetry)** Metrics with closed labels only; names of tenants, namespaces,
  scripts and packages in spans and logs, never in labels (Principle V). *Acceptance:* `metric_labels_are_closed`.

### Success criteria

- **Success criterion 046-001** The packages pinned in
  [acceptance-packages.md](../../specs/046-script-dependencies/contracts/acceptance-packages.md) each publish and run on
  every required target in the acceptance run.
- **Success criterion 046-002** The hostile battery passes on both targets.
- **Success criterion 046-003** A deployment with the gateway `offline` and an imported store bundle publishes and runs a
  script whose lock is in the bundle.
- **Success criterion 046-004** The benchmark report exists and states its environment and failures.

### User stories

- **User story 046-01** An author uses pandas and httpx in Python, and axios and zod in TypeScript, without a worker
  (Journey 1).
- **User story 046-02** A security owner sets the policy and sees what every version contains and what was found
  (Journeys 2 and 3).
- **User story 046-03** An operator runs the platform without internet access (Journey 4).
- **User story 046-04** A team uses its private packages without exposing their names to a public registry (Journey 5).

## Goals

- TypeScript and Python scripts get the packages of their ecosystems, as in Windmill.
- The guarantees listed in the Overview.
- Reuse feature 005's machinery: drafts, versions, the build key, signing, the runner, the sandbox profile.

## Non-Goals

- Packages for C#, Go, Java, Rust or PHP (closed catalogs).
- Source distributions and install scripts (Gap 046-001).
- Proxying a tenant's external registry with its credentials (Gap 046-002): it needs a new disclosure path under
  Principle V; tenants upload instead.
- Changing the TypeScript runtime (Deno and Bun stay out, decision 7 of Architecture decision 0057).
- Inferring manifests from imports (Decision 046-003).
- Promising that a package without advisories is safe.

## Open Questions

None blocks the design. Three defaults wait on measurements (Gaps 046-003 to 046-005).

## Owner decisions needed

None is open. Architecture decisions 0120 and 0121 are accepted with the approval of this design, under Owner decision
2026-11-24-4 (Owner decision 2026-11-24-312), on the approval rule of Owner decision 2026-11-25-2. Every other choice,
including the read-only mount that relaxes Architecture decision 0013 (Decision 046-009, a partial supersession), is
recorded in research.md as Decisions 046-001 to 046-072 with the register's literal marker "delegate (OD-2026-11-24-4)".

## Critical User Journeys (CUJs)

The heading is the template's; the journeys are numbered in words.

### Journey 1: An author uses pandas in a Python script

*   **Given**: a namespace with the default policy, `script-dependencies-v1` activated, a runner and the gateway online.
*   **When**: the author writes a PEP 723 block naming `pandas>=2.2` and `httpx`, tries the script and publishes it.
*   **Then**: the test run resolves, is evaluated, installs and runs on its runner, with no secret. Publication answers
    `202` with the version `BUILDING`; when it is `READY` it shows its lock summary, findings and SBOM. Runners that will
    run it materialize the layer first, then offer the step. The run container has no network.

### Journey 2: A new release of a dependency is malware

*   **Given**: a TypeScript script locked to `left-pad-ish 1.0.3`, and the default quarantine of 72 hours.
*   **When**: `1.0.4` is published upstream with a credential stealer, and the author asks to resolve again an hour later.
*   **Then**: the gateway does not offer `1.0.4` (younger than the job's quarantine cut) and the new lock keeps `1.0.3`.
    Later OSV marks `1.0.4` malicious. In the import's switch transaction every artifact that locked `1.0.4` is blocked;
    its next lease, its materialization and its start are refused; an attempt already running finishes.

### Journey 3: A vulnerability is disclosed in a pinned package

*   **Given**: a version locked to `requests 2.31.0`.
*   **When**: an advisory of severity HIGH affecting it is imported.
*   **Then**: the version gets a finding, an audit record and a metric, and keeps running (the default policy does not
    block new findings at lease). A new version with the same lock is refused with `PACKAGE_VULNERABLE` unless a waiver
    is recorded.

### Journey 4: An air-gapped deployment

*   **Given**: no internet; the gateway `offline`.
*   **When**: an operator exports a signed store bundle and an advisory archive on a connected machine and imports both;
    an author publishes a script whose manifest those packages satisfy.
*   **Then**: the resolver sees only the store; the build succeeds; the run works. A package not in the store fails with
    `RESOLUTION_FAILED`.

### Journey 5: A private package name appears on the public registry

*   **Given**: a tenant that reserved and uploaded `acme-billing 1.4.0`.
*   **When**: an attacker publishes `acme_billing 99.0.0` on PyPI.
*   **Then**: `acme_billing` normalises to `acme-billing`, a reserved name; the runner forwards every request for it to
    the control plane's private index, never to the gateway; no request for it reaches PyPI; the lock validator would
    refuse a public source for it.

## Technical Plan (Architecture)

### 1. The path of a version with packages

```mermaid
sequenceDiagram
    participant A as Author
    participant CP as Control plane
    participant R as Sandbox runner
    participant RC as Resolve container
    participant G as Package gateway
    A->>CP: publish (source, manifest)
    CP->>CP: version BUILDING; resolve job (namespace, quota)
    R->>CP: claim (capability, target): job grant, quarantine cut, reserved names
    R->>RC: start (sandbox profile, no network)
    RC->>R: fetch over the channel
    R->>G: public routes, with the grant
    R->>CP: private index routes, for reserved names
    RC-->>R: lock
    R->>CP: complete resolve
    CP->>G: metadata (internal listener), no transaction open
    CP->>CP: one transaction: lock, metadata, report; refuse, or insert install jobs and the build job
    R->>CP: claim install (its target)
    R->>R: fetch files, check hashes, unpack, verify the digest in a fresh container, seal
    R->>CP: complete install (digest)
    R->>CP: claim and complete the build (feature 005; bundle version 5)
    CP->>CP: finishing transaction: checks, SBOM, provenance, sign bundle and layer records, READY
```

A version without a manifest takes feature 005's path unchanged.

**Jobs.** Dependency jobs are rows of `script_dependency_jobs` with the version's namespace, the states of builds
(`QUEUED`, `RUNNING`, `SUCCEEDED`, `FAILED`), and the same claim, heartbeat, retry and `not_before`. The `script_jobs`
quota counts them with builds and test runs (`crates/infra-postgres/src/quotas.rs`). `script_build_jobs` keeps one row per
version. Every transition is one transaction fenced by the job's lease token; a duplicate completion answers
`AlreadyDone` and changes nothing, as feature 005 answers today.

- **Resolve completes.** The control plane calls the gateway's metadata endpoint with no transaction open. Then one
  transaction: lock the version row and require `BUILDING`; read the ecosystem's snapshot row, the namespace's policy row
  and the tenant's reservation revision `FOR SHARE`; evaluate; store the lock, metadata, report and those three
  revisions; charge the fetch budget; then refuse the version (`FAILED` with its reason, an event and an audit record) or
  insert one install job per required target and the build job. A failed metadata call fails the resolve job as
  retryable.
- **An install completes.** One transaction: the job row, then the version row (`BUILDING` required); record the target's
  digest; if this install refused the version (size, metadata or identity mismatch, or its last attempt failed), mark the
  version `FAILED` and nothing else. No transaction writes a sibling's row: a sibling's completion finds the version no
  longer `BUILDING` and finishes its own job as cancelled. The unique key of dependency jobs is
  `(tenant, version, phase, target)` with `NULLS NOT DISTINCT`.
- **The build** waits until every install job of the version succeeded. The build claim, the dependency-job claim and the
  `script_jobs` quota count only jobs of versions that are `BUILDING`, so the jobs of a refused version stop counting at
  once.
- **The finish** (Technical Plan 9) runs on both the new-build path and the cache-hit path.

**Publication and the build key.** `content_digest` (`crates/application/src/scripts.rs`) gains the manifest under a new
prefix for drafts that carry one, so an unchanged draft deduplicates as today and a changed manifest makes a new version.
`POST .../scripts/{script}:resolve-again` makes a new version of the latest draft with a fresh resolution. The build key
of a bundle version 5 is computed at the build's claim, as feature 005 computes it (with the claiming runner's
toolchain), under the prefix `sagat/build/v2`, adding the manifest digest, the lock digest, the unpacker identity and the
dependency runtime image identity. Framework-only versions keep `sagat/build/v1` (`crates/application/src/sandbox.rs`).

**An author lock** skips the resolve job: publication calls the gateway's metadata synchronously (an outage answers
`503`, retryable), validates and evaluates in the publication transaction, answers `422` on refusal, and inserts the
install jobs and the build job directly.

**Where the evaluation runs.** The evaluation is pure code in the application. Each store call that commits a decision
(resolve completion, an author-lock publication, the finish) takes the snapshot, policy and reservation revisions and the
earliest waiver expiry the application evaluated against, locks their rows `FOR SHARE` itself, compares, and answers
`StaleInputs` when any differs or the waiver time has passed; the application then evaluates again and retries
(`crates/infra-postgres/src/scripts.rs`, `finish_build` and `finish_build_cached` gain these parameters).

### 2. The manifest and the lockfile gate

Python's PEP 723 block is parsed from the source by the control plane. TypeScript's manifest is a new `manifest` column of
`scripts`, `script_versions` and `script_test_runs`, protected by `script_versions_guard`. The domain's `Lockfile` type
documents PEP 751 for Python.

The runner's gate is `crates/infra-sandbox/src/lockfile.rs::approved`, reached from `docker.rs`. It gains a context
parameter (`Build`, `DependencyJob`, `TestRun`, `Bundle5Run`): for TypeScript and Python it accepts a package-bearing lock
only in the last three, whose locks the control plane has validated; in a `Build` it refuses them as today.

### 3. The package gateway

`apps/package-gateway` has no database and holds no tenant data. Its routes (package-gateway.md):

- npm package documents, which the gateway builds in the abbreviated form from the full document it reads upstream, and
  tarballs; the PyPI JSON simple API, wheel files and their core-metadata files (PEP 658 and PEP 714), all public;
- on a separate internal listener, for the control plane's service token: file metadata, the advisory archives, store
  administration (pins, marks, deletions);
- a `package-gateway healthcheck` subcommand for the compose healthcheck, as the other images have.

The gateway applies three fixed rules to what it serves, none of them policy. Versions published after the job grant's
quarantine cut are left out: for npm it reads each version's time from the full document with a streaming parse capped at
64 MiB and drops a version without a time. Because `versions` usually comes before `time` in that document, the parse
keeps only the extracted per-version entries of the abbreviated form with each version's time, never the raw document;
the entries are cached per name for 5 minutes in a cache bounded to 256 MiB with least-recently-used eviction, so the
filter for any cut is a lookup; at most 8 parses run at once per gateway, at most 4 per tenant (the grant's `tenant` claim) and at most 2 per job grant
(others wait, then answer `503` after 10 s, which the runner treats as `PACKAGE_GATEWAY_UNAVAILABLE`, retryable), so one
tenant's resolves cannot hold every slot; a
document over 64 MiB is refused with `502` and the job fails `RESOLUTION_FAILED`, never served partly filtered; for PyPI it drops a file without `upload-time`. For Python, source
distributions and wheels that none of the grant's targets and interpreter can use are left out. Yanked versions are marked
as the registries mark them. Allow and deny rules, waivers and advisories are not the gateway's: the control plane's evaluation
applies them.

Tokens are EdDSA web tokens signed with the package token key (`SAGAT_PACKAGE_TOKEN_KEY`, not the script signing key): a
job grant bound to one job attempt, valid until the job's deadline (at most 10 minutes), and a service token for the
control plane. The gateway never calls the control plane.

The metadata endpoint also answers, for each (ecosystem, name, version, file name), the hash the registry's own document
lists for that file, so the control plane can refuse a lock entry whose hash belongs to another file (Technical Plan 5).

Upstream fetches use a streaming GET-only send that `crates/infra-egress` gains: addresses checked once, private ranges
refused unless the operator allows an upstream's, redirects not followed, only https hosts that are configured upstreams
or file hosts (`registry.npmjs.org`, `pypi.org`, `files.pythonhosted.org` by default). Names are validated and normalised
before an upstream path is built. A file is checked against the registry's declared hash before it is stored once under
its SHA-256 in the public store, a blob store of its own (`SAGAT_PACKAGE_STORE_*`, read by `BlobSettings` with a prefix
parameter that `crates/infra-blobstore` gains). The timing of a public cache hit can tell a tenant that some tenant
fetched a public file: an accepted residual (Gap 046-007).

**Collection.** A leased control-plane pass (daily) marks public objects that no stored lock references and no pin holds,
and deletes, after 7 days, those still unreferenced and unpinned at deletion time. Pins are rows of `package_store_pins`:
a running resolve, install, test run or materialization pins its files until it ends; an imported offline bundle pins its
files until an operator removes it.

**Fetch budget.** The claim of a resolve job charges `package_fetch_usage` the job's 1 GiB forwarding ceiling in the
claim transaction, under the usage row's lock, and is refused when that would pass the daily budget (20 GiB per tenant
by default; the job stays queued until the next day, and its version shows why); so concurrent claims cannot overshoot.
The job row records the day and the amount charged (`charged_day`, `charged_bytes`); every completion of that attempt,
a failed one included, settles the charge on that day's row only, to the bytes the runner reports it forwarded, never
below zero, so a claim before midnight completed after it cannot credit the new day; a lease that expires without a
completion keeps the ceiling. A successful completion also charges the sizes
of the locked files the metadata marks as not yet stored; over the budget the version is refused with
`PACKAGE_FETCH_BUDGET_EXCEEDED`.

### 4. The resolve job

The claim answer carries the source, the manifest, the runtime line, the required targets, the job grant, the quarantine
cut and the tenant's reserved names for the ecosystem (runner-protocol.md). The runner starts the pinned resolver image
(`runners/resolver-python`, `runners/resolver-node`, by digest) with the sandbox profile, no network, 1 GiB of memory and
512 MiB of scratch.

The resolver shim serves HTTP on the container's loopback interface and turns each request into a `fetch` op on the
supervisor channel, the op scripts use today. For a resolve job the runner's host calls allow only the routes that
runner-protocol.md lists, each with how its package name is read: the package document (the name is the path; an npm
scope's `%2f` is decoded once into the canonical `@scope/name`, and nothing else may be encoded), the core-metadata file
and the file (for npm the name is the path's `{name}` segment, scope included and decoded as for the document, and the file
name must equal `{unscoped name}-{version}.tgz`; for PyPI the name is read from the wheel file name; either must match
the name its index answer gave), with no `..`, no `//` and no query. A request whose name, normalised, is reserved goes to the control plane's private routes
for the same three kinds; every other goes to the gateway; a path that does not parse is refused. The runner adds the
grant and forwards. Responses come back in chunks of at most 160 KiB decoded per line. A resolve job has its own
budgets: 4,000 requests, 30,000 channel lines, 32 MiB per response, 1 GiB in all.

The shim runs the pinned `uv` (0.8 or later; PEP 751 output, binary wheels only, the quarantine cut as `exclude-newer`,
the interpreter version and every required target as environments) or the pinned `npm` (the one of the Node 24 image;
lock-only, scripts ignored). The probe of Task 046-12 fixes the command lines, measures real locks for two targets and
chooses the Python lock path before Task 046-4 starts: PEP 751 directly, or `uv`'s own lock converted to PEP 751 by the
shim. The base images are `python:3.13-slim-bookworm` and `node:24-bookworm-slim`, by digest.

The lock returns as `lockChunk` messages (256 KiB at most). A resolver failure keeps its first 8 KiB of error output,
redacted, in the build log and fails with `RESOLUTION_FAILED`. Any transport failure of a forwarded request during the
job (refused, timed out, `502`, `503`, `504`) fails the job as `PACKAGE_GATEWAY_UNAVAILABLE`, retryable, whatever the
resolver then returns, so a missed optional dependency can never become a lock.

### 5. Lock validation

The control plane parses the lock in its closed subset with bounded nesting and sizes. Beyond lock-formats.md: every
manifest entry is satisfied by a root entry; the closure is complete against the dependencies the gateway reports for
each file; Python may hold several entries for one name only when their markers select different targets; each required
target selects a full set (wheel tags; npm `os`, `cpu`, `libc`); a reserved name has no public source. Version orders and
ranges sit behind the application port `PackageRules`; the SBOM and provenance builders behind `SupplyChainDocuments`;
the OSV parser behind `AdvisoryArchives`. Every lock entry's hash must be the one the registry's own document lists for
its name, version and file name (from the metadata endpoint; for a private entry, from `private_packages`), so a hash
cannot be relabelled with another package's name. The ports are implemented in `crates/infra-packages` with published
crates (`nodejs-semver`, `pep440_rs`, `pep508_rs`); the domain holds the policy types and the combination of results.

### 6. The supply-chain policy

The policy is the `packages` section of the namespace's script policy document in `script_runtime_policies` (this
design creates the table; feature 023 adds the `languages` section, its Decisions 1627 and 1628), changed through
`PUT /v1/namespaces/{namespace}/script-policy` with an expected revision. Allow and deny rules are rows of
`script_package_rules`; waivers are rows of `supply_chain_waivers`
([supply-chain-policy.schema.json](../../specs/046-script-dependencies/contracts/supply-chain-policy.schema.json),
[rest-api.md](../../specs/046-script-dependencies/contracts/rest-api.md)).

| Field | Default | Bounds |
|---|---|---|
| `npm.enabled`, `pypi.enabled` | true | |
| `allow` rules | none (everything not denied) | 500 rows |
| `deny` rules | none | 500 rows |
| `quarantineHours` | 72 | 0 to 720 |
| `private.quarantineHours` | 24 | 0 to 720; the hold of a private upload, Technical Plan 10 |
| `vulnerabilities.blockAtOrAbove` | `HIGH` | `CRITICAL`, `HIGH`, `MODERATE`, `LOW`, `NONE` |
| `vulnerabilities.blockUnrated` | false | |
| `vulnerabilities.blockNewAtLease` | false | |
| `licences.deny` | empty | 100 SPDX (Software Package Data Exchange) identifiers |
| `installScripts` | `skip` | `skip` or `refuse` |
| `maxLayerBytes` | 1 GiB | up to 4 GiB |

The verdict's rules and their order are in errors.md: the lock's own rules first, then malware, reserved-name conflicts,
deny and allow, quarantine, vulnerabilities, licences, yanked versions, install scripts, metadata mismatch, layer size.
The report (verdict, findings, the snapshot, policy and reservation revisions) is stored per version in
`script_version_dependencies`.

**Blocks and re-evaluation.** A block is a row of `script_artifact_blocks`, keyed by tenant: tenant, artifact digest,
namespace (null for all namespaces, with `UNIQUE NULLS NOT DISTINCT`), reason, source (`ADVISORY_IMPORT`, `REEVALUATION`,
`RESERVATION`, `OPERATOR`), and the policy and reservation revisions it was evaluated at; every predicate that reads a
block names the tenant. Artifact rows stay immutable. A durable row of `script_policy_reevaluations` is inserted in the transaction
that changes the policy, adds or removes a waiver, imports a snapshot, creates a reservation or deletes an upload; the
hourly waiver pass inserts one for each expired waiver. A leased pass works a tenant's jobs in creation order (a
tenant-wide job before the namespace jobs created after it), in pages of 200 versions, one transaction per page that holds
the policy, snapshot and reservation-revision rows `FOR SHARE` and advances the cursor in its commit. A page writes and
removes only `REEVALUATION` and `RESERVATION` blocks, removes one only when it evaluated at revisions at least those the
block records, and never removes an `ADVISORY_IMPORT` or `OPERATOR` block.

**Steps of a blocked artifact.** A step whose artifact becomes blocked may have no runner holding its volume, so no runner
would advertise it and the lease check would never see it. So the transaction that inserts a block also inserts a row the
block pass reads; that pass fails the ready script steps of the blocked artifact in the affected namespaces, in batches,
through `reject_input` with `SCRIPT_DENIED` and `PACKAGE_BLOCKED`. With `blockNewAtLease`, a finding at or above the threshold without a waiver
blocks that namespace.

### 7. The advisory snapshot

The gateway fetches each ecosystem's OSV archive from the operator-configured source (default the public OSV bucket),
as one more upstream, and serves it on its internal listener; offline, an operator imports the archive with
`control-plane advisories import <file>`. The control plane parses it with streaming decompression and the bounds of
Non-functional requirement 046-004. A record is malicious when its identifier or an alias starts with `MAL-`, or its
database-specific type says malware; severity is the GitHub advisory label, else the CVSS (Common Vulnerability Scoring
System) base score mapped to the same labels, else unrated.

An import begins with a barrier: one short transaction takes the snapshot row `FOR UPDATE`, which waits for every
writer of `script_locked_packages` (each takes that row `FOR SHARE` before its first insert and holds it to its commit),
records the highest `locked_seq` of the table (an identity column declared with `CACHE 1`, so values are handed out in
order across sessions) as the import's mark, and commits. The migration seeds one snapshot row per ecosystem, so the row
every writer and the barrier lock always exists. Every row at or
below the mark was committed before the barrier; every row inserted later gets a higher value. The import then stages
its records under a new snapshot identifier in paged transactions, outside any lock, and computes, for each new
malicious record, the exact (name, version) pairs it matches among the rows at or below the mark. The switch is one
transaction that takes the snapshot row `FOR UPDATE` and then: refuses the import if it is older than the current
snapshot of that ecosystem or holds at least 1 percent fewer records (checked again under the lock); makes it current;
inserts a block for every artifact that locks a precomputed pair, whatever its `locked_seq`; evaluates the ranges,
through `PackageRules`, for every row above the mark; and inserts one re-evaluation row per tenant that either of the
two found. An import that would newly block more than 50 artifacts is held for an
operator's confirmation (`advisory:import`) and an alert fires; while it is held, finishes and materializations refuse
artifacts that lock a package its malicious records match, so a wide compromise is not delayed. Refusals and holds are
audited. The old snapshot's rows are deleted afterwards. A malware block is released only through
`DELETE /v1/admin/script-blocks/{id}` (`script:block:release`, the platform operator), which a database trigger allows only
when the advisory is withdrawn.

### 8. Installation and the layer

The runner claims an install job for its own target (claims carry the target; the only target setting today is
`SAGAT_RUNNER_GO_TARGET`, so a general `SAGAT_RUNNER_TARGET` is added). It fetches each locked file through the route that
carries its name (`/npm/{name}/-/{file}`, `/pypi/files/{sha256}/{filename}`, so the gateway can fetch it again upstream
if its store no longer holds it), or from the control plane's private file route for a reserved name, with a URL it builds
from its configured base, checks the hash and, for npm, the lock's SHA-512 integrity, and keeps the file in its file
cache. A transport failure or a hash mismatch on these fetches fails the job as `PACKAGE_FILE_UNAVAILABLE`, retryable.
Then three containers, each with the sandbox profile, no network, and the volume mount builder of feature 035's lake
profile:

1. **Unpack** (`runners/layer-tool`, a static Rust binary; volume read-write; files on standard input): wheels into
   `site-packages/` following the wheel format, tarballs into their `node_modules` paths. It refuses links, absolute
   paths, `..`, special files, set-user-id and set-group-id bits, `__pycache__` in a wheel, a path written by two
   packages, and the path, file and size bounds, and stops writing at `maxLayerBytes`. It checks each wheel's
   `.dist-info` name and version and each tarball's `package.json` name and version against the lock entry, and reports
   the lifecycle scripts it sees in each `package.json`.
2. **Verify** (the layer tool; volume read-only; no input): computes the layer digest from the whole tree.
3. **Seal and mark** (the layer tool; volume read-write), only after the runner has compared the verify step's digest
   with the expected one: sets modes (0444 files, 0555 directories; the owner stays the sandbox user), then writes the
   completion marker `.sagat-layer` with the digest, last.

No bytecode is compiled into the layer: a bytecode file would be code that runs without being covered by the digest, so
the run's Python never writes or reads cached bytecode for the layer (`PYTHONDONTWRITEBYTECODE=1`, as today, and no
`__pycache__` in the tree, which the verify step refuses). The cold-import cost this adds is measured by the benchmark
(Gap 046-010).

The unpacker's bound is the only limit on what a compromised unpacker could write to the runner's disk; the runbook
places the engine's volume root on a filesystem of its own, so that a full layer disk cannot stop other work (Gap
046-009).

### 9. Finishing, signing, the SBOM and the provenance

Bundle version 5 is version 1's text bundle plus `dependencies`: `ecosystem`, `runtime` (the dependency runtime image
digest), `manifestDigest`, `lockDigest`, `unpacker` (the layer tool's digest). The runner assembles it; the control plane
checks it with `Expected` (`crates/application/src/scripts.rs`) and `Bundles::check` (`crates/infra-schema/src/bundle.rs`),
both extended to compare the dependencies object with `script_version_dependencies`; `BuildAssignment` carries that
object to the runner.

A layer record is a separate signed statement per (artifact digest, target): `sagat/script-layer/v1 ‖ tenant ‖ artifact
digest ‖ target ‖ layer digest ‖ unpacker`, Ed25519 with the script signing key, kept in `script_layer_records`, which is
the one place per-target digests live once a version is `READY`. Because records are separate from the bundle, a
deployment that adds a target later runs a backfill pass (leased, the platform operator starts it, audited): it inserts
install jobs of the phase `BACKFILL` for `READY` versions; a backfill completion signs and inserts the new record in its own
transaction and does not touch the version's state; nothing is republished.

The finishing transaction of a bundle version 5, on the new-build path and on the cache-hit path
(`finish_build_cached` gains these steps):

1. takes the version row, then the ecosystem's snapshot row, the namespace's policy row and the tenant's reservation
   revision `FOR SHARE`;
2. if any differs from the revisions stored with the report, re-runs the evaluation on the stored metadata (no network)
   and replaces the findings; a changed verdict refuses the version;
3. writes the SBOM to `script_version_sboms` (CycloneDX 1.5, at most 4 MiB);
4. writes, for a new build, the provenance to `script_build_provenance` (this design's migration creates the table with
   feature 023's columns; its envelope names the lock and SBOM digests and marks the runner-reported fields, so it stays
   within 65,536 bytes);
5. signs the bundle (feature 005, Decision 98) and the layer records, and makes the version `READY`.

A snapshot import's switch takes the snapshot row `FOR UPDATE`, and a policy write and a reservation take their rows
`FOR UPDATE`, so each is ordered against every finish.

### 10. Private packages

A tenant administrator reserves names and scopes (`POST /v1/package-reservations`, `package:manage`); each reservation
increases the tenant's reservation revision. Only a reserved name can be uploaded
(`PUT /v1/namespaces/{namespace}/packages/{ecosystem}`, `package:upload`, at most 64 MiB, on a router with that body
limit). The control plane reads the archive's own name and version (at most 1 MiB of metadata, at most 64 MiB inspected) and
checks them against the reservation, stores it in the tenant's
payload store (Architecture decision 0023's blob keys, under the uploading namespace), then inserts the `private_packages`
row; uploads are visible to every namespace of the tenant. The same file again answers `200`; another file for an
uploaded version answers `409 PACKAGE_VERSION_EXISTS`. A private version is held out of the private index for the
resolving namespace's `private.quarantineHours` (default 24, 0 to 720) after its `uploaded_at`, unless a holder of
`package:manage` other than the uploader releases it earlier (`POST /v1/packages/{ecosystem}/{name}/{version}/release`,
audited; the uploader is refused with `PACKAGE_RELEASE_BY_UPLOADER`). So a stolen `package:upload` credential cannot
put a version into a build at once: someone else sees it first, or it waits out the hold, during which an administrator
can delete it. The hold applies everywhere a private version enters a build: the private index leaves it out, an author
lock that names it is refused with `PACKAGE_QUARANTINED`, and the private file route answers the unknown `404` for its
file. The hold that applies is the one of the namespace named by the request's grant (the job's, the test run's or the
materialization's namespace), never another namespace of the tenant. Uploads count against the tenant's content bytes (Architecture
decision 0027).

The control plane serves the private index and files to runners (`/script-runner/private-index/...`,
`/script-runner/private-files/{sha256}`), for the runner's tenant only; another tenant's name or file answers the same
`404` and the attempt is audited. A new reservation re-evaluates the tenant: a stored lock that took that name from a
public source is blocked for every namespace, because it may be the dependency confusion itself.

### 11. Materialization and the run

A version with packages runs on a dependency runtime line: a pinned glibc image of the same interpreter minor as the
framework-only line, because native wheels and prebuilt npm binaries are most often published for glibc. Framework-only
versions keep their Alpine images. The runner's configuration gains the resolver, layer-tool and dependency runtime
images and the target.

Materialization comes before the lease. The pending-activities route tells a runner which ready activities need a layer
it lacks; the runner materializes those (the three containers of Technical Plan 8, with the files of the version's lock,
which must hash to the bundle's lock digest), and advertises an activity only once its volume is marked. So the engine's
attempt timeout never includes a materialization. Materializations run at most two at a time per runner and one at a
time per (tenant, digest), within a deadline of 120 s plus 20 s per 100 MiB of layer (about 16 minutes at the 4 GiB
ceiling; the install job and the grant use the same deadline). The materialization route takes the ready activity, not a namespace: the control plane reads the tenant, namespace and
artifact from it, checks the blocks for that namespace, and answers an `id`. A runner reports a failed materialization
(`POST /script-runner/materializations/{id}/failed`, accepted only from the principal that created that row, in its
tenant; any other caller gets the unknown `404`); when three distinct runner identities (the authenticated runner
principals of the route, never the self-configured `SAGAT_RUNNER_INSTANCE_ID`) have failed one, or no registered target
has a layer record for 10 minutes, the control plane fails the ready steps of that artifact through `reject_input` with
`SCRIPT_SANDBOX_UNAVAILABLE` and `LAYER_UNAVAILABLE`, so no step waits silently, and the gauge
`script_steps_waiting_for_layer` shows the rest.

Before starting a run container the runner asks once whether the attempt may start (`GET /script-runner/attempts/{attempt}/runnable`
for a step, `GET /script-runner/test-runs/{run}/runnable` with the lock digest for a test run); the tenant and namespace come
from the attempt or the test run, never from the runner, and a test run's lock digest must equal the one stored for it.
A block found there, at materialization or at lease refuses the start with `SCRIPT_DENIED` and `PACKAGE_BLOCKED`; an
unknown attempt and another tenant's attempt or test run answer the same `404`.

Volumes are named by tenant, layer digest and the runner's configured instance identifier (`SAGAT_RUNNER_INSTANCE_ID`,
required with this feature; at start the runner refuses to run if another live runner on the engine carries the same
identifier). A runner keeps a volume held from lookup until its run container is removed, so eviction never removes one
in use. After a restart it checks each volume's marker with a verify container once before trusting it.

The run container gets the volume read-only at `/opt/sagat/deps`, not `noexec`, because native extensions are mapped
executable. The probe of Task 046-12 records from `/proc/self/mountinfo` whether the engine applies `nosuid` and `nodev`;
the unpacker's refusal of privileged bits and special files and `no-new-privileges` hold either way. The Python shim adds
the layer to `sys.path`; the Node shim resolves bare specifiers from the layer's `node_modules` through a link in the
scratch area, for both module systems.

Images: a runner keeps every resolver, layer-tool and runtime image digest that a `READY` version names (the control
plane lists them at registration), and the release process never deletes such a digest from its registry. A patched
layer tool is a new unpacker identity: existing versions keep their recorded unpacker; the backfill pass may add records
for the new one.

If feature 019 ships warm containers (Architecture decision 0052, Proposed), a new block on an artifact evicts its warm
containers.

### 12. Blocking at lease

Today `RunTx::script_execution_admitted_instance` (`crates/application/src/execution.rs`) is the retention-quota
admission and returns a `bool` whose `false` leaves the step ready. It becomes a typed admission, `Admit`, `Wait` (the
quota, as today) or `Deny(reason)`, implemented in `crates/infra-postgres/src/execution.rs` and in
`crates/infra-postgres/src/execution/path.rs`, and called on both lease paths: `lease_one` (`crates/application/src/worker.rs`)
and the scoped claims (`crates/application/src/worker/scoped_claims.rs`). Its statement reads, with the tenant, whether a
block matches the artifact digest and the step's namespace or all namespaces; feature 023 joins its language section into
the same statement (its Decision 1628). `Deny` fails the step permanently through `reject_input` in the lease
transaction, with `Failure::Denied` carrying the closed reason `PACKAGE_BLOCKED` (`crates/domain/src/scripts.rs`), its
journal event, next work, outbox rows and a `script.supply_chain_blocked` audit record. An attempt already started
finishes. Design 038's budget test gains a script-step shape.

### 13. Where the code goes

| Part | Place | Action |
|---|---|---|
| Policy, finding, verdict, block types; `Failure::Denied` reason; the `Lockfile` documentation | `crates/domain/src/scripts/packages.rs`, `crates/domain/src/scripts.rs` | Create, Modify |
| Permissions | `crates/domain/src/identity.rs` | Modify |
| Ports `PackageRules`, `PackageMetadata`, `SupplyChainDocuments`, `AdvisoryArchives`; publication, dependency jobs, finishing, passes, waivers, reservations, uploads | `crates/application/src/scripts/packages.rs`, `crates/application/src/scripts.rs` (`Draft`, `Expected`, `content_digest`, the stores), `crates/application/src/sandbox.rs` (requests, build key version 2) | Create, Modify |
| Lease check | `crates/application/src/execution.rs`, `crates/application/src/worker.rs`, `crates/application/src/worker/scoped_claims.rs`, `crates/infra-postgres/src/execution.rs`, `crates/infra-postgres/src/execution/path.rs` | Modify |
| Lock parsers, version orders, advisory parsing and matching, SBOM and provenance builders | `crates/infra-packages` (new adapter crate) | Create |
| Bundle version 5 | `crates/infra-schema/src/bundle.rs` | Modify |
| Tables and statements | `crates/infra-postgres` (migrations after `20270119000000`, `scripts.rs`, `quotas.rs`, `storage-classification.json`) | Create, Modify |
| REST and runner routes | `crates/api-http/src/routes/{scripts,script_packages,runner}.rs`, `crates/api-http/src/dto_scripts.rs`, `crates/api-http/src/lib.rs` | Create, Modify |
| Streaming GET send; store settings prefix | `crates/infra-egress/src/lib.rs`, `crates/infra-blobstore/src/config.rs` | Modify |
| Container driver: resolve, unpack, verify, seal containers; volumes; channel forwarding; the lockfile gate | `crates/infra-sandbox/src/{docker,lake,protocol,lockfile}.rs` | Modify |
| Runner: dependency jobs, materialization, caches, configuration | `apps/sandbox-runner/src/{lib,config,api,host}.rs` | Modify |
| The gateway | `apps/package-gateway` | Create |
| Images | `runners/resolver-python`, `runners/resolver-node`, `runners/layer-tool`, `runners/python-glibc`, `runners/node-glibc` | Create |
| Compose and Helm | `infra/docker/compose.yaml`, `infra/docker/Dockerfile`, `infra/helm` | Modify |
| Studio and CLI | `frontend/apps/studio/src/features/scripts/`, `apps/cli` | Modify |

`scripts/check-architecture.py` gains checks with negative tests: no network mode other than `none` in any container
configuration of `crates/infra-sandbox` (`docker.rs` and `lake.rs`); a run container's configuration has exactly one added
mount, read-only; no token or secret in a resolve or install container's environment or input (a configuration-builder
unit test); only `apps/package-gateway` names the public store's settings; and a dependency allow-list for `apps/package-gateway` (as the
existing lake-runner rule does), which fails if it links `sagat-infra-postgres`, `sagat-application`, `sagat-infra-sandbox`
or any crate that reads database or payload-store settings. A test checks that activating `script-dependencies-v1` is
refused when the admission statement does not read `script_artifact_blocks`. A Helm render test checks that only the gateway
may leave the cluster. The rows go into `docs/engineering/architecture-principles.md`.

### 14. Permissions

| Permission | Default holders | Grants |
|---|---|---|
| `script:read` | Viewer, Operator, Developer, Admin | lock summaries, findings, blocks, SBOMs of the namespace |
| `script:write`, `script:publish` | Developer, Admin | the manifest and lock of a draft; `:resolve-again` |
| `script:policy:manage` | Admin | the script policy, package rules and waivers |
| `package:manage` | Admin | reservations; deleting an upload |
| `package:upload` | Admin; grantable to a machine identity | uploading a reserved name |
| `advisory:import`, `script:block:release` | the platform operator | offline and forced imports, confirmation of held imports; releasing a malware block |

### 15. Telemetry and audit

The metric and audit names are in [telemetry.md](../../specs/046-script-dependencies/contracts/telemetry.md), split by
builder: the control plane's (jobs, evaluations, findings, lease refusals, re-evaluation backlog, advisory import
duration, snapshot age) and the gateway's and runner's (requests, upstream latency, cache hits and evictions,
materialization time). Every label is closed; names of tenants, namespaces, scripts and packages are span attributes.
Cross-tenant attempts at the control plane's private routes and materialization are audited.

### 16. Compose, deployment and operations

`infra/docker/compose.yaml` gains `package-gateway` (built from `infra/docker/Dockerfile`, target `package-gateway`; the
shared hardening settings of the `x-hardening` anchor; the package listener on
`127.0.0.1:${SAGAT_PACKAGE_GATEWAY_PORT:-18095}`; the internal listener unpublished; the healthcheck subcommand; a named
volume for a filesystem store in development). The sandbox runner gets the gateway URL, an instance identifier and a
named volume for its file cache. The Helm chart gains the gateway with a network policy: runners and the control plane may
reach it; only it may leave the cluster, to its upstreams. The runbook `docs/runbooks/script-dependencies.md` covers the
policy, waivers, reservations, imports, offline bundles, caches, the volume filesystem and a blocked version.

### 17. Compatibility and rollout

Everything is additive. With the capability not activated nothing changes, and activation is refused until every control
plane and engine lists it. Rolling back binaries after bundle version 5 rows exist: an old runner answers an unknown
bundle version with `SCRIPT_SANDBOX_UNAVAILABLE` (retryable), so those steps retry until the binaries roll forward or
their retry policy ends them; the release notes say rollback is forward-only for data.

### 18. Delivery phases

All in 0.3.0, each with its own acceptance; the capability cannot be activated before Phase A is complete.

- **Phase A, the core:** the capability, manifests, the gateway, resolve, validation, install and materialization,
  bundle version 5 and layer records, SBOM and provenance, the policy's deny, allow and quarantine rules, the advisory
  import with malware blocks, and the lease, materialization and start checks. Acceptance: Requirements 046-001 to 046-009
  and 046-012 to 046-016, Success criteria 046-001 and 046-002 (the battery's Phase A part).
- **Phase B, policy after publication:** vulnerabilities, licences, waivers, re-evaluation. Acceptance: Requirements
  046-010 and 046-011.
- **Phase C, private packages.** Acceptance: Requirement 046-017.
- **Phase D, offline and collection.** Acceptance: Requirement 046-018, Success criterion 046-003.
- **Phase E, surfaces and proof:** Studio, CLI, the rest of the battery, benchmark, runbook. Acceptance: Requirement
  046-021, Success criteria 046-002 and 046-004.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A malicious release is resolved before anyone knows | Quarantine of 72 hours by default; malware blocked in the import's switch transaction, at lease, at materialization and at start |
| The gateway is down | Publication fails retryable; published versions keep running from their volumes; new runners cannot materialize until it is back (stateless, several replicas) |
| The gateway is compromised or its public store poisoned | It holds no tenant data or secret; every file is checked against the lock's own hash by the runner; private packages never pass through it |
| A poisoned or truncated advisory archive | Older and shrinking imports refused; imports that would block more than 50 artifacts held for an operator; bounded parsing |
| A package needs its install script | Admitted with a finding; Gap 046-001 |
| The unpacker is exploited | It runs in a sandbox container; the digest is recomputed by a fresh container; the volume root is on its own filesystem (Gap 046-009) |
| Runner disks fill | Least-recently-used bounds; per-tenant volumes; metrics |
| Re-evaluation cost grows | Paged, leased, resumable; malware does not wait for it; its backlog is a metric |
| Resolver tools change behaviour | Pinned by digest; in the build key; the probe runs in continuous integration on both targets |
| Images needed by published versions disappear | Runners keep every named digest; the release process never deletes one |
| A target is added later | Layer records are separate from the bundle; a backfill pass adds them |
| The public store's timing side channel | Accepted for public content (Gap 046-007) |

## Constitution Check

Written against version 4.7.0, in force at this commit; the 5.0.0 amendment (pull request #275) changes none of the
principles these rows rely on.

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Every job transition is one fenced transaction; the version row orders install completions; the finish holds the snapshot, policy and reservation rows `FOR SHARE` on both build paths; re-evaluation is a durable job with a cursor; a blocked step fails in the lease transaction with its journal event and outbox |
| II. Modular domain, ports and adapters | Pass with conditions | Policy types in the domain; version math behind ports in a new adapter; checks added. Condition: the gateway is a separate process from the start, without a measurement first, because it is the only process with internet egress and must share no process or credential with what reads tenant data (Complexity Tracking) |
| III. External effects are at-least-once; sagas compensate | Not applicable | No external effect of a run is added |
| IV. Compatibility is versioned and pinned | Pass | A version pins its lock, unpacker and runtime; layer records are signed per target; bundles 1 to 3 unchanged; images kept while named; framework-only build keys unchanged |
| V. Code, tenant boundary and secrets are isolated | Pass with conditions | No package code outside a sandbox container; no sandbox container has a network; private packages stay in the tenant's store; identical `404`s, audited. Condition: the read-only mount (Complexity Tracking) |
| VI. Observability and audit are part of the feature | Pass | Technical Plan 15 |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | No new message path; a blocked step's outbox rows are in the lease transaction |
| VIII. Failure is tested and contracts are verified, honestly | Pass | The battery, crashes at each transition and during materialization, the import and finish races, re-evaluation resume, gateway outage, offline build; gaps registered |
| Delivery Constraints | Pass | Self-contained compose service, pinned images, loopback port, healthcheck; targets stated as benchmark criteria |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A new service, separate from the start (Principle II) | The only process with internet egress; it holds only public files and one credential | Runners or the control plane talking to registries: many egress paths, no offline mode |
| Requests over the supervisor channel for the resolve container | A resolver needs metadata and the sandbox keeps no network | A container network: shared between tenants, host-local, a token in the container |
| One read-only mount in the run container (Principle V; Architecture decision 0013) | A tree does not fit a 2 MiB bundle and must not be downloaded | Streaming the tree into scratch at each run |
| Bundle version 5 and separate layer records | The signature must bind the lock and each layer, and targets can be added later | Layers inside the bundle: a target added later would need republishing |
| Dependency jobs, build key version 2, an extended cache-hit finish | A version needs resolve and install before its build; the key covers the lock | Reusing the build-job row: one row per version |
| A block table | Blocks change; artifacts must not | A column on the artifact row: refused by the immutability trigger |
| An advisory snapshot and two version orders | Facts about vulnerabilities and malware without a third party per publication | An online scanner |
| Three containers per materialization | The digest must not be reported by the container that parsed the archives, and modes must be fixed after the digest is checked | One container: the unpacker would vouch for itself |

## Alternatives Considered

- **A closed catalog for TypeScript and Python:** cannot serve the ecosystems; kept for the other languages.
- **Install at run time with network (Windmill):** a network path per run, install scripts, registry outages break runs.
- **A third-party repository manager as the proxy:** a large component with its own policy model, no namespaces; an
  operator may chain one as the gateway's upstream.
- **A container network to the gateway** (first draft): rejected in review (shared bridge, host-local, a token inside).
- **Layers in the platform's blob store** (first draft): gigabytes through the control plane and a second copy; rejected.
- **Policy filtering in the gateway** (second draft): the gateway would need the namespace's rules and reservations;
  rejected for fixed rules there and the policy in the control plane.
- **Private packages in the gateway's store** (second draft): the internet-facing process would hold tenant data;
  rejected.
- **A layer as an image, or an image volume:** needs an image builder on the runner host; deferred.
- **Inferring the manifest from imports:** a typosquatting path.
- **Running the installer:** a large program whose behaviour changes by version; unpacking verified archives is enough
  once scripts and source builds are excluded.
- **One volume per digest for all tenants:** a cache side channel; rejected.
- **Building source distributions in the sandbox:** later (Gap 046-001).

## Non-Functional Requirements

As stated under Requirements, each with its acceptance.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Documentation first: the contracts and the data model are written at this commit; Task 046-1 transcribes the REST
contract into the OpenAPI and lands the remaining documents. Each task names its files and tests; tests are written first
and seen failing. Migrations take timestamps after `20270119000000`, allocated by the coordinating session.

Builder A owns `crates/domain`, `crates/application`, `crates/infra-packages`, `crates/infra-postgres`,
`crates/infra-schema` and `crates/api-http`. Builder B owns `apps/package-gateway`, `crates/infra-egress`,
`crates/infra-blobstore`, `crates/infra-sandbox`, `apps/sandbox-runner`, `runners/`, `infra/`, `frontend/apps/studio` and
`apps/cli`. The contracts are their interface. Feature 023 extends Tasks 046-2, 046-15 and 046-17 (its Decisions 1627 and
1628).

| Phase and step | Builder A | Builder B |
|---|---|---|
| Phase A, step 1 | Tasks 046-2, 046-3, 046-4 | Tasks 046-12 (the probe first), 046-6 |
| Phase A, step 2 | Tasks 046-5, 046-10 | Tasks 046-7, 046-13 |
| Phase A, step 3 | Tasks 046-14, 046-15, 046-17 | Tasks 046-16, 046-22 (the Phase A part of the battery) |
| Phase B | Tasks 046-11, 046-25 | Task 046-21 |
| Phase C | Task 046-9 | Task 046-8 |
| Phase D | Task 046-18 | Task 046-20 |
| Phase E | Task 046-24 | Tasks 046-19, 046-22 (the rest), 046-23 |

- [ ] T046-1 Transcribe `contracts/rest-api.md` into the OpenAPI (`specs/046-script-dependencies/contracts/openapi.yaml`)
  and land the runbook draft, the row of Gap 97 in `docs/architecture/security-and-sandbox.md`, and the cross-references
  in features 005 and 023. Test: `contract_script_dependencies` lints the OpenAPI.
  Refs: Requirement 046-019
- [ ] T046-2 Domain types; permissions; the capability `script-dependencies-v1` in the three capability constraints
  (`serving_instances_capabilities_known`, `platform_requirements_capability_known`,
  `platform_requirement_history_capability_known`, restating the full set with `run-input-validation-v1`); the migrations
  of data-model.md with grants and storage classification; `script_runtime_policies` and the policy and package-rule
  routes. Files: `crates/domain/src/scripts/packages.rs`, `crates/domain/src/identity.rs`,
  `crates/application/src/instances.rs`, `crates/infra-postgres/migrations/`, `crates/api-http/src/routes/script_packages.rs`.
  Tests: `verdict_order_*`, `without_the_capability_a_manifest_is_refused`, the privilege audit of
  `integration_platform_foundation`.
  Refs: Requirement 046-001, Requirement 046-020, Requirement 046-011
- [ ] T046-3 npm lock validator in `crates/infra-packages/src/npm.rs`. Tests: `lock_rules_npm_*`, the bounds.
  Refs: Requirement 046-002, Requirement 046-007, Non-functional requirement 046-004
- [ ] T046-4 PEP 751 validator in `crates/infra-packages/src/pypi.rs`, after the probe chose the lock path. Tests:
  `lock_rules_pypi_*`, the bounds.
  Refs: Requirement 046-002, Requirement 046-005, Requirement 046-007, Non-functional requirement 046-004
- [ ] T046-5 Advisories: staged import, the older and shrinking refusals, the hold above 50 artifacts, the switch
  transaction with malware blocks, matching, severity, the release route and its trigger. Files:
  `crates/infra-packages/src/advisories.rs`, `crates/application/src/scripts/packages.rs`, migrations. Tests:
  `matcher_agrees_with_the_version_order_*`, `an_older_snapshot_is_refused`, `a_large_import_is_held`,
  `the_switch_rechecks_age_under_the_lock`, `a_finish_between_staging_and_switch_is_matched`,
  `a_lock_written_by_a_transaction_open_across_the_barrier_is_matched` (held-lock tests, the writer's transaction held
  open with its `FOR SHARE` while the import starts),
  `a_held_import_refuses_matching_finishes`.
  Refs: Requirement 046-008, Requirement 046-009
- [ ] T046-6 Gateway skeleton and npm side: listeners, tokens, abbreviated documents filtered by the cut, tarballs, the
  public store, the streaming GET send, the healthcheck subcommand. Files: `apps/package-gateway/`,
  `crates/infra-egress/src/lib.rs`, `crates/infra-blobstore/src/config.rs`. Tests against recorded responses
  (`apps/package-gateway/tests/fixtures/npm/`, refreshed by `scripts/record-registry-fixtures.sh`):
  `filters_versions_after_the_cut`, `drops_versions_without_a_time`, `refuses_a_redirect_to_another_host`,
  `a_document_over_the_cap_is_refused_not_partly_served`, `full_document_parses_are_bounded_and_cached`,
  `one_grant_cannot_hold_every_parse_slot`, `one_tenant_cannot_hold_every_parse_slot`,
  `validates_names_before_upstream`,
  `egress_is_get_only`.
  Refs: Requirement 046-004, Requirement 046-018
- [ ] T046-7 Gateway PyPI side: the JSON simple API, core-metadata files, wheel-only and tag filtering, files by hash, the
  metadata endpoint, the advisory archive route. Tests: recorded PyPI fixtures; `never_serves_a_source_distribution`.
  Refs: Requirement 046-005, Requirement 046-007
- [ ] T046-8 Runner side of private packages: rewriting reserved names to the private routes. Test:
  `a_reserved_name_never_reaches_the_public_registry` (runner part: index, metadata, file and scoped requests, a scoped
  tarball whose file name carries no scope among them, with the gateway receiving none).
  Refs: Requirement 046-017
- [ ] T046-9 Control-plane side of private packages: reservations and their revision, uploads into the tenant's payload
  store, the private index and file routes, re-evaluation on reservation and deletion, audit (`sagat package push` is
  Task 046-19's). Tests: `an_identical_upload_is_accepted_again`, `a_different_file_for_an_uploaded_version_is_a_conflict`,
  `a_reservation_blocks_a_lock_that_took_the_name_from_public`, `another_tenants_private_file_is_not_found`,
  `a_private_upload_is_held_for_the_private_quarantine`, `the_uploader_cannot_release_their_own_upload`,
  `a_held_upload_is_refused_in_an_author_lock_and_on_the_file_route`,
  `the_hold_is_the_grant_namespaces_with_two_namespaces_of_different_holds`.
  Refs: Requirement 046-017
- [ ] T046-10 Publication flow: `content_digest` with the manifest, `:resolve-again`, dependency jobs and their
  transitions, the build key version 2 at claim, the author-lock path, `Expected` and `BuildAssignment`. Files:
  `crates/application/src/scripts.rs`, `crates/application/src/scripts/packages.rs`, `crates/application/src/sandbox.rs`,
  `crates/infra-postgres/src/{scripts,quotas}.rs`. Tests: `framework_only_versions_keep_their_build_key`,
  `resolve_again_makes_a_new_version`, `an_author_lock_is_validated_synchronously`,
  `a_refused_install_and_a_sibling_completion_do_not_deadlock` (a barrier), `a_refused_install_stops_the_build_job_and_the_quota`,
  `a_duplicate_completion_changes_nothing`, `an_undeclared_import_fails_at_import_time_and_changes_nothing`.
  Refs: Requirement 046-002, Requirement 046-003, Requirement 046-008
- [ ] T046-11 Vulnerabilities, licences, waivers (routes and the hourly pass), findings and blocks listings,
  re-evaluation jobs and pass. Tests: `policy_change_reaches_the_lease`, `waiver_expiry_reaches_the_lease`,
  `reevaluation_resumes_after_a_crash`, `reevaluation_never_removes_a_malware_block`,
  `a_page_never_removes_a_fresher_reservation_block`, `a_waiver_cannot_cover_malware`,
  `a_waiver_longer_than_90_days_is_refused`, `block_new_at_lease_blocks_one_namespace`.
  Refs: Requirement 046-010, Requirement 046-011
- [ ] T046-12 Resolver images and shims; the `fetch` forwarding with its grammar and budgets; the probe that fixes the
  command lines, measures locks, chooses the Python lock path and records the mount options, on both targets. Files:
  `runners/resolver-python/`, `runners/resolver-node/`, `crates/infra-sandbox/src/protocol.rs`,
  `apps/sandbox-runner/src/host.rs`. Tests: `probe_resolves_the_acceptance_packages`,
  `a_resolve_container_reaches_only_the_forwarded_routes`, `a_forwarding_failure_fails_the_job_retryable`.
  Refs: Requirement 046-004, Requirement 046-005, Requirement 046-006
- [ ] T046-13 The layer tool and the install job: unpack (with the identity checks), verify, seal and mark. Files: `runners/layer-tool/`,
  `crates/infra-sandbox/src/{docker,lake}.rs`, `apps/sandbox-runner/src/lib.rs`. Tests: `unpacker_refuses_*`,
  `the_digest_is_recomputed_by_a_fresh_container`, `a_relabelled_file_is_refused`, `a_pycache_in_the_tree_is_refused`,
  `a_postinstall_never_runs_and_is_reported`, `layer_digest_is_identical_on_two_hosts`.
  Refs: Requirement 046-006, Requirement 046-012, Non-functional requirement 046-002
- [ ] T046-14 Runner routes in the control plane: claims with capability and target, materializations, the runnable
  check, test-run dependencies, target-aware pending activities. Files: `crates/api-http/src/routes/runner.rs`,
  `crates/application/src/scripts/packages.rs`. Tests: `an_old_runner_claim_gets_no_dependency_work`,
  `a_runner_of_another_target_is_not_offered_the_step`, `a_test_run_with_packages_runs_and_is_refused_for_malware`,
  `a_step_no_runner_can_materialize_fails_typed`, `the_runnable_check_takes_the_namespace_from_the_attempt`,
  `a_materialization_takes_the_namespace_from_the_activity`, `one_runner_failing_three_times_does_not_fail_the_step`,
  `another_tenants_attempt_answers_the_unknown_404`, `a_test_run_lock_digest_must_match`,
  `only_the_creating_runner_may_fail_a_materialization`.
  Refs: Requirement 046-016, Requirement 046-020
- [ ] T046-15 Bundle version 5, layer records, the finishing steps on both build paths, the SBOM, the provenance (the
  module feature 023 reuses). Files: `crates/infra-schema/src/bundle.rs`, `crates/infra-packages/src/{sbom,provenance}.rs`,
  `crates/application/src/scripts.rs`. Tests: `a_snapshot_imported_during_a_build_is_seen_before_signing`,
  `a_cached_build_is_evaluated_and_finished_like_a_new_one`, `sbom_package_set_equals_the_lock`,
  `provenance_verifies_and_marks_reported_fields`, `a_tampered_layer_record_is_refused`,
  `a_waiver_expiring_during_a_build_is_seen_at_finish`, `a_target_added_later_gets_records_without_republishing`.
  Refs: Requirement 046-008, Requirement 046-009, Requirement 046-013, Requirement 046-015
- [ ] T046-16 Materialization and the run: runtime images, volumes per tenant, digest and runner, the hold, the restart
  check, the runnable check before start, the mount, the shims, the caches, image retention. Files:
  `runners/python-glibc/`, `runners/node-glibc/`, `crates/infra-sandbox/src/docker.rs`,
  `apps/sandbox-runner/src/{lib,config}.rs`. Tests: `two_tenants_with_one_lock_get_two_volumes`,
  `an_unmarked_volume_is_never_mounted`, `two_materializations_of_one_layer_run_once` (a barrier),
  `a_script_cannot_write_the_layer`, `a_malware_import_during_materialization_stops_the_start`, `native_extension_imports_*`.
  Refs: Requirement 046-009, Requirement 046-013, Requirement 046-014
- [ ] T046-17 The typed admission on both lease paths, `reject_input`, `Failure::Denied` with its reason, the block pass
  for ready steps; design 038's budget test with a script-step shape. Tests: `a_malware_import_blocks_the_next_lease`,
  `a_blocked_child_script_step_fails_at_lease`, `a_blocked_step_with_no_materialized_runner_fails`,
  `an_attempt_already_started_finishes`, `activation_requires_the_block_check`.
  Refs: Requirement 046-009, Requirement 046-020
- [ ] T046-18 Store collection with marks, pins and the grace period; the fetch budget. Tests:
  `a_pinned_file_is_never_collected`, `a_reference_gained_after_the_mark_keeps_the_file`,
  `a_collected_file_is_fetched_again_by_name`, `the_fetch_budget_counts_forwarded_bytes`,
  `a_failed_resolve_is_charged_its_forwarded_bytes`, `an_over_budget_tenant_is_not_given_a_resolve`,
  `concurrent_claims_cannot_overshoot_the_budget` (a barrier), `a_claim_before_midnight_settles_on_its_own_day`.
  Refs: Requirement 046-018
- [ ] T046-19 Studio panel and CLI (`sagat script packages`, `sagat package push`). Files:
  `frontend/apps/studio/src/features/scripts/`, `apps/cli`. Tests: the Studio journey test, the CLI contract tests.
  Refs: Requirement 046-021
- [ ] T046-20 Offline mode and signed store bundles with their pins; compose service, Dockerfile target, Helm values and
  network policy; the compose smoke test. Tests: `offline_build_from_an_imported_bundle`, `an_unsigned_bundle_is_refused`,
  `only_the_gateway_leaves_the_cluster`, `scripts/smoke-compose.sh` with the gateway.
  Refs: Requirement 046-018, Success criterion 046-003
- [ ] T046-21 The gateway's and runner's metrics of telemetry.md. Test: `metric_labels_are_closed` (gateway and runner).
  Refs: Non-functional requirement 046-005
- [ ] T046-22 The hostile battery and the fault tests on both targets in the native-runner continuous-integration job
  (`integration_script_dependencies_containers`): the Phase A part with Task 046-16, the rest in Phase E.
  Refs: Requirement 046-004, Requirement 046-012, Requirement 046-014, Non-functional requirement 046-001, Success criterion 046-002
- [ ] T046-23 Benchmark report (`tests/performance/reports/script-dependencies.md`) with environment and failures.
  Refs: Non-functional requirement 046-003, Success criterion 046-004
- [ ] T046-24 Runbook, reference pages, quickstart, release notes; Gap 97 of feature 005 closed for TypeScript and Python
  with the evidence; the acceptance run of the pinned packages.
  Refs: Requirement 046-021, Success criterion 046-001
- [ ] T046-25 The control plane's metrics and audit records of telemetry.md. Test: `metric_labels_are_closed` (control
  plane).
  Refs: Requirement 046-019, Non-functional requirement 046-005

#### Follow-up tasks

None yet.

### Verification Plan

- **Unit and property:** lock validators; matchers against both version orders; verdict order; unpacker refusals; the
  run configuration's single read-only mount; no token in resolve or install containers.
- **Integration (real PostgreSQL, a real gateway with recorded fixtures):** each manifest form and an author lock; each
  refusal reason; `a_cached_build_is_evaluated_and_finished_like_a_new_one`;
  `a_snapshot_imported_during_a_build_is_seen_before_signing` (an import's switch held at a barrier between resolve and
  finish); a policy change, a reservation and a waiver during a build; malware blocks at lease, materialization and start;
  re-evaluation after a non-malware advisory with `blockNewAtLease`; private names with case and separator variants;
  offline mode; the test-run path.
- **Real containers (native-runner continuous integration, AMD64 and ARM64):** the hostile battery. A resolve container
  tries the internet, the control plane, the engine, the runner host, another container, the metadata address and name
  resolution, and a channel request outside the forwarded routes (all refused). A `postinstall` and a `setup.py`
  (neither runs). A write into the layer (refused). Two tenants, one lock (two volumes). A tampered lock, bundle, record,
  layer and another tenant's file (refused before start). A native extension in each language.
  `layer_digest_is_identical_on_two_hosts` compares two hosts of the same target, with a pure and a native lock.
- **Faults:** the runner dies during resolve, install, verify and materialization (a lapsed lease is reclaimed at once,
  as feature 005 reclaims it, and the job ends `FAILED` after its attempts; an unmarked volume is never mounted); the gateway refuses, times out
  and answers `503` (retryable); a duplicate completion (`AlreadyDone`); the control plane dies between an import's switch
  and the first re-evaluation page and during a page (the pass resumes); two materializations at a barrier.
- **Contracts:** OpenAPI lint and conformance; the gateway's contract against recorded npm and PyPI responses.
- **Benchmark:** Task 046-23.
