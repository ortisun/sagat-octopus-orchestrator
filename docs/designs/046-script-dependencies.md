# Package dependencies for TypeScript and Python scripts

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

From release 0.3.0 a TypeScript or Python script can use packages from npm and PyPI, and private packages its tenant
uploads to the platform. The author declares what the script needs: a PEP 723 block at the top of a Python script, a
`package.json` of dependencies beside a TypeScript one. When the author publishes, the platform resolves the
declaration into a lock in which every package has an exact version and a content hash, checks the lock against the
namespace's supply-chain policy, unpacks the packages into a dependency layer named by its SHA-256, and signs a bundle
that names that layer. A run never downloads anything: the runner mounts the layer read-only into the run container,
which still has no network.

Only one component reaches a package registry: the **package gateway**, a small platform service declared in this
repository's compose. Resolution runs in a sandbox container whose only network is the gateway; installation runs no
installer and no package code. The architecture is
[ADR-0120](../adr/ADR-0120-script-packages-are-resolved-at-publication-through-the-package-gateway-and-run-from-a-signed-read-only-layer.md);
the policy and its effect on runs that are already published is
[ADR-0121](../adr/ADR-0121-a-namespace-supply-chain-policy-gates-publication-and-a-malicious-package-advisory-stops-leases.md).
Both are Proposed. The registers (decisions, gaps) are in `specs/046-script-dependencies/research.md`; the contracts in
`specs/046-script-dependencies/contracts/`. The principles cited by numeral are those of
[the constitution](../constitution.md).

This design closes Gap 97 of feature 005 for TypeScript and Python. It replaces Decision 105 of feature 005 for those
two languages only: C# keeps its closed catalog (ADR-0021), and Rust and PHP keep theirs (feature 023, ADR-0057).

### Terms

- **Manifest.** What the author declares: package names and version ranges. Python: the PEP 723 inline metadata block
  (`# /// script`, `dependencies = [...]`, `requires-python`). TypeScript: a `package.json` document that holds only
  `dependencies` and, optionally, `overrides`.
- **Lock.** The resolved closure: every package, its exact version, where the gateway serves it and its content hash.
  Python: a PEP 751 `pylock.toml` restricted to wheels. TypeScript: an npm `package-lock.json` with `lockfileVersion: 3`.
- **Package gateway (the gateway).** `apps/package-gateway`. A read-only proxy of the npm registry protocol and of the
  Python simple repository API in its JSON form (PEP 691), with a content-addressed store in the platform's blob store.
  It serves only what the namespace's policy allows, to callers that present a job token.
- **Resolve job.** A runner job that runs the pinned resolver in a sandbox container attached to the gateway's
  network, and returns the lock.
- **Install job.** A runner job that fetches the locked files from the gateway, checks each hash, and has the
  platform's unpacker write the dependency layer in a sandbox container with no network.
- **Dependency layer (the layer).** A canonical archive of the installed packages for one native target
  (`linux-amd64` or `linux-arm64`), named by its SHA-256. A version has one layer per target it supports.
- **Supply-chain policy (the policy).** The namespace's rules for packages (Technical Plan 6).
- **Advisory snapshot.** The platform's copy of OSV vulnerability and malicious-package records for npm and PyPI,
  imported by the control plane or by an operator (Technical Plan 7).
- **Finding.** A policy result on one package of a lock: an advisory match, a licence, an age, an install script that
  was not run.

## The Problem

Windmill's main convenience for scripts is that `import pandas` works: on deployment it infers the requirements from
the imports, resolves them with `uv` or `bun`, stores a lockfile, and its workers install the packages from a public or
private registry, with network access, caching them on the worker's disk. It applies no supply-chain policy: no
vulnerability check, no quarantine of new releases, no protection against a private name being taken on the public
registry, no SBOM.

This platform does the opposite today. Feature 005 refuses every package for TypeScript and Python (Decision 105): a
script can use only the standard library and the platform's SDK shim. Gap 97 of feature 005 asks for pinned
dependencies, an SBOM and a supply-chain policy, and is closed only for one C# package. An author who needs an HTTP
client, a date library or pandas writes an SDK worker instead, which takes the script away from the platform's sandbox,
signing and versioning.

The constraints any answer must keep:

- The run container has no network, a read-only root and a noexec scratch area (feature 005, Decision 95). A run must
  not download a package.
- Untrusted code must not run in the control plane, the scheduler or the runner process (Principle V); installing a
  package in either ecosystem can run its code (npm lifecycle scripts, a Python source distribution's build).
- An artifact is at most 2 MiB and lives in PostgreSQL (feature 005, Gap 108); a dependency tree is far larger.
- Infrastructure is self-contained in this repository (AGENTS.md rule 16): the proxy must be in the compose, pinned,
  loopback-only, with a healthcheck.
- A published version is immutable (Principle IV): what ran yesterday runs today, even if the registry deletes the
  package.

## Requirements

### Functional

- **Requirement 046-001** A TypeScript or Python draft MAY carry a manifest: Python in a PEP 723 block of its source,
  TypeScript in the draft's `manifest` field (a `package.json` with only `dependencies` and `overrides`). A draft
  without one behaves exactly as today. *Acceptance:* a draft with each manifest form publishes and runs; a draft
  without one is built as a framework-only version with the same build key as before this design.
- **Requirement 046-002** Publication of a version with a manifest MUST resolve it into a lock through a resolve job
  (Technical Plan 4), or accept a lock the author supplies if it satisfies the manifest and every entry is served by
  the gateway with the same hash. *Acceptance:* both paths produce a version whose lock satisfies the manifest; an
  author lock with a changed hash, an extra root, a missing dependency or a source outside the gateway is refused with
  `422 LOCKFILE_INVALID`.
- **Requirement 046-003** The platform MUST NOT infer the manifest at publication. It MAY propose manifest entries from
  the imports of a draft through a separate request, which changes nothing until the author saves the draft.
  *Acceptance:* a publication with an import that no manifest entry covers fails at build time like any missing module;
  the proposal endpoint returns entries and writes nothing.
- **Requirement 046-004** A resolve job MUST run in a container with the feature 005 profile whose only network is the
  gateway's internal network, MUST run the pinned resolver with install scripts and source builds disabled, MUST
  receive no secret, and MUST return only the lock. *Acceptance:* the hostile battery (Verification Plan) shows that a
  resolve container reaches the gateway and nothing else (not the internet, not the control plane, not the engine, not
  the runner's host), and that a package with an install script or only a source distribution runs no code.
- **Requirement 046-005** Python locks MUST contain wheels only; a package that publishes only a source distribution is
  refused with `PACKAGE_SOURCE_ONLY`. Every wheel MUST match the runtime's interpreter version and a supported native
  target. *Acceptance:* fixtures for each refusal; a lock with wheels for both targets builds a layer per target.
- **Requirement 046-006** npm lifecycle scripts (`preinstall`, `install`, `postinstall`, `prepare`) of dependencies MUST
  never run. A package whose lock entry declares one is admitted with a finding `INSTALL_SCRIPT_SKIPPED`, unless the
  policy refuses such packages. *Acceptance:* a fixture package whose `postinstall` writes a file shows the file absent
  from the layer and the finding in the version's report.
- **Requirement 046-007** The control plane MUST validate every lock it stores: the closed subset of each format
  (`specs/046-script-dependencies/contracts/lock-formats.md`), a hash on every entry (SHA-512 integrity for npm, SHA-256
  for wheels), every source a gateway path, no `git`, `file`, `link`, directory or URL dependency, no alias that hides
  a different package, the manifest satisfied. *Acceptance:* one refused fixture per rule.
- **Requirement 046-008** Before a build is signed, the control plane MUST evaluate the lock against the namespace's
  policy, the gateway's metadata and the advisory snapshot (Technical Plans 6 and 7) and refuse with the first failing
  reason, listing every finding. *Acceptance:* the reasons `PACKAGE_NOT_ALLOWED`, `PACKAGE_QUARANTINED`,
  `PACKAGE_VULNERABLE`, `PACKAGE_MALICIOUS`, `PACKAGE_LICENCE_DENIED`, `PACKAGE_SOURCE_ONLY`,
  `PACKAGE_PLATFORM_UNSUPPORTED`, `DEPENDENCY_LAYER_TOO_LARGE`, each with a fixture.
- **Requirement 046-009** A package matched by a malicious-package advisory MUST be refused at publication and MUST NOT
  be leased afterwards, whatever the policy or a waiver says (ADR-0121). *Acceptance:* the snapshot test of the
  Verification Plan.
- **Requirement 046-010** Installation MUST NOT run an installer or package code. The runner MUST check each fetched
  file against the lock's hash before use; the platform's unpacker, in a container with the feature 005 profile and no
  network, MUST write the canonical layer (Technical Plan 8). *Acceptance:* the layer of a lock is byte-identical across
  two runners and two runs; an archive with a link, an absolute path, a `..` segment, a device file, a set-user-id bit,
  a path over the bound or a decompressed size over the bound is refused.
- **Requirement 046-011** A version with packages MUST be signed as bundle version 5
  (`specs/046-script-dependencies/contracts/bundle-v5.md`), which names, per target, the layer's digest and size, and
  the digests of the manifest, the lock, the SBOM, the policy and the policy report. A runner MUST refuse to mount a
  layer whose digest differs from the signed one. *Acceptance:* a tampered layer, a tampered bundle and a layer of
  another tenant each fail before a container starts, with `SCRIPT_UNTRUSTED_ARTIFACT`.
- **Requirement 046-012** A run of a bundle-5 version MUST mount its layer read-only, `nosuid` and `nodev`, as the only
  mount of the run container, from a runner volume keyed by (tenant, layer digest), never shared between tenants.
  *Acceptance:* a script that tries to write into the layer fails; two tenants with the same lock get two volumes.
- **Requirement 046-013** Every build of a version with packages MUST produce a CycloneDX SBOM listing every locked
  package with its package URL, version, hashes and declared licence, and the provenance statement of ADR-0057
  decision 6 whose resolved dependencies are the lock's entries. *Acceptance:* the SBOM package set equals the lock's;
  the provenance verifies with the platform key.
- **Requirement 046-014** A tenant MAY upload private packages (npm tarballs, Python wheels) to a namespace with the
  permission `package:upload`. A name that a namespace has uploaded MUST resolve only from that namespace's uploads,
  never from a public registry, for that namespace. *Acceptance:* a public package with the same name and a higher
  version is never chosen; the dependency-confusion test of the Verification Plan.
- **Requirement 046-015** The gateway MUST run `online` (with upstream registries) or `offline` (only its store), and
  MUST import and export store bundles so that an air-gapped deployment can build. A version whose lock is already
  stored MUST build without any upstream. *Acceptance:* the offline build test.
- **Requirement 046-016** When an advisory snapshot is imported, the control plane MUST re-evaluate every stored lock of
  the tenants concerned, record new findings, and block the versions that now contain a malicious package (ADR-0121).
  *Acceptance:* the snapshot test of the Verification Plan, and the listing of findings by version.
- **Requirement 046-017** A namespace administrator MUST be able to read and change the policy, add a waiver (one
  advisory, one package, an expiry of at most 90 days, a reason), and list findings; every change is audited.
  *Acceptance:* REST contract tests; the waiver expiry test.
- **Requirement 046-018** Failures MUST be typed and value-free: publication refusals are `422` with a closed reason and
  the package name and version (never a registry response); build failures carry the kind `resolve` or `install`; run
  failures for a blocked version are `SCRIPT_DENIED` with reason `PACKAGE_BLOCKED`, not retryable; an unreachable
  gateway is `PACKAGE_GATEWAY_UNAVAILABLE`, retryable. *Acceptance:* the error contract tests.
- **Requirement 046-019** The feature MUST be switched on by the capability `script-dependencies` in the instance
  registry (ADR-0082): with it absent, a manifest or a package-bearing lock is refused as today (Decision 105 of feature
  005). *Acceptance:* the capability test.
- **Requirement 046-020** The Studio MUST show the manifest, the lock summary, the findings and the policy, and the CLI
  MUST resolve, show and upload. *Acceptance:* the Studio journey and the CLI contract tests.

### Non-functional

- **Non-functional requirement 046-001 (isolation)** No component other than the gateway reaches a registry; no
  package code runs outside a run container; the gateway holds no tenant secret.
- **Non-functional requirement 046-002 (reproducibility)** The same lock and the same unpacker give the same layer
  digest on every runner.
- **Non-functional requirement 046-003 (performance targets, to be measured)** On the development machine: a warm run
  with a cached layer adds at most 50 ms to the start of the same script without packages; resolving a lock of 50
  packages with a warm gateway takes at most 30 s; populating a 100 MiB layer takes at most 20 s. These are benchmark
  criteria, not promises (Delivery Constraints).
- **Non-functional requirement 046-004 (bounds)** Every quantity is bounded: manifest 64 KiB, lock 256 KiB (the existing
  lockfile ceiling), 2,000 packages per lock, a layer of 1 GiB by default and 4 GiB at the platform ceiling, 200,000
  files per layer, path of 1,024 bytes, a resolve job of 300 s, an install job of 600 s, a runner layer cache of 50 GiB
  by default.
- **Non-functional requirement 046-005 (telemetry)** Metrics with closed labels only; tenant, namespace, script and
  package names in spans and logs, never in metric labels (Principle V).

### Success criteria

- **Success criterion 046-001** The ten most downloaded Python packages that publish wheels for both targets, and the
  ten most downloaded npm packages without a required install script, each publish and run in a script on both targets
  in the acceptance run.
- **Success criterion 046-002** The hostile battery passes: no network but the gateway during a resolve, none during
  install and run, no install script executed, no write into a layer, no cross-tenant volume, every tampered input
  refused.
- **Success criterion 046-003** A deployment with the gateway `offline` and a store bundle imported publishes and runs
  a script whose lock is in the bundle.
- **Success criterion 046-004** The benchmark report exists and states its environment and failures.

### User stories

- **User story 046-01** An author uses pandas and httpx in a Python script, and axios and zod in a TypeScript one,
  without writing a worker (CUJ 1).
- **User story 046-02** A security owner sets the namespace's policy and sees, for every version, what it contains and
  what was found (CUJs 2 and 3).
- **User story 046-03** An operator runs the platform without internet access (CUJ 4).
- **User story 046-04** A team uses its private packages without exposing their names to a public registry (CUJ 5).

## Goals

- Scripts in the two most used languages get the packages of their ecosystems, as in Windmill.
- Stronger guarantees than Windmill: no network in a run, no package code before the run, a policy at publication, a
  stop for malware found later, an SBOM and signed provenance for every build, offline builds, protection against
  dependency confusion.
- Reuse the feature 005 machinery: drafts, versions, the build key, signing, the runner, the sandbox profile, the
  execution record.

## Non-Goals

- Packages for C#, Go, Java, Rust or PHP (they keep closed catalogs).
- Source distributions and install scripts (Gap 046-001).
- Proxying a tenant's own external registry with its credentials (Gap 046-002): it needs a new disclosure path under
  Principle V. Tenants upload private packages instead.
- Changing the TypeScript runtime (Deno and Bun stay out, ADR-0057 decision 7).
- Promising that a package without advisories is safe: the snapshot is evidence at a date, not a guarantee.

## Open Questions

None blocks the design. The measurements that may change a default are gaps: the quarantine default (Gap 046-003), the
layer cache bound (Gap 046-004), the resolver command lines proved by the probe (Gap 046-005).

## Owner decisions needed

None. The design follows Owner decision 2026-11-24-1 (the feature is in 0.3.0). Its product choices were taken under the
delegation of Owner decisions 2026-11-20-6 and 2026-11-24-4 and are recorded as Decisions 046-001 to 046-020 in
`specs/046-script-dependencies/research.md`, each marked "delegate (OD-2026-11-24-4)" with its reasoning. ADR-0120 and
ADR-0121 are accepted or rejected under Owner decision 2026-11-24-4 when this design completes its review.

## Critical User Journeys (CUJs)

### CUJ 1: An author uses pandas in a Python script

*   **Given**: a namespace with the default policy, the capability `script-dependencies` recorded, a runner and the
    gateway online.
*   **When**: the author writes a script whose first lines are a PEP 723 block naming `pandas>=2.2` and `httpx`, tries it
    (a test run) and publishes it.
*   **Then**: the test run resolves, installs and runs as a publication would (it never receives a secret, as today);
    publication returns the version with its lock summary (packages, versions), its findings (none), its SBOM link; the
    first run on a runner populates the layer and later runs start from the cached volume; the run container has no
    network.

### CUJ 2: A new release of a dependency is malware

*   **Given**: a TypeScript script locked to `left-pad-ish 1.0.3`; the default quarantine of 72 hours.
*   **When**: `1.0.4` is published upstream with a credential stealer and the author re-resolves an hour later.
*   **Then**: the gateway does not offer `1.0.4` (younger than the quarantine) and the lock keeps `1.0.3`. When OSV later
    marks `1.0.4` malicious and an operator's sync imports it, nothing changes for this version; a version of another
    namespace that had locked `1.0.4` (with quarantine 0) is blocked: its next script step fails `SCRIPT_DENIED`
    (`PACKAGE_BLOCKED`), the attempt already running finishes, and the security owner sees the finding.

### CUJ 3: A vulnerability is disclosed in a pinned package

*   **Given**: a version locked to `requests 2.31.0`.
*   **When**: an advisory of severity HIGH affecting it is imported.
*   **Then**: the version gets a finding, an audit record and a metric; it keeps running (the policy does not block at
    lease by default); a new publication of the same lock is refused with `PACKAGE_VULNERABLE` unless a waiver with an
    expiry is recorded.

### CUJ 4: An air-gapped deployment

*   **Given**: a deployment with no internet; the gateway `offline`.
*   **When**: an operator exports a store bundle and an advisory snapshot on a connected machine, imports both, and an
    author publishes a script whose manifest those packages satisfy.
*   **Then**: the resolve job resolves from the store only, the build succeeds, the run works; a package not in the store
    fails with `RESOLUTION_FAILED` and says which.

### CUJ 5: A private package name appears on the public registry

*   **Given**: a namespace that uploaded `acme-billing 1.4.0`.
*   **When**: an attacker publishes `acme-billing 99.0.0` on PyPI.
*   **Then**: resolution for that namespace sees only the uploaded `acme-billing`; the public one is never offered, and
    the attempt is visible as a gateway log line, not a metric label.

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
    participant B as Blob store
    A->>CP: publish draft (source + manifest [+ lock])
    CP->>CP: validate manifest; build key; queue resolve job
    R->>CP: claim resolve job (job token for the gateway)
    R->>RC: start (005 profile + gateway network only)
    RC->>G: metadata and files (token)
    G->>B: store files by SHA-256
    RC-->>R: lock (standard output)
    R->>CP: complete resolve (lock)
    CP->>G: metadata of the locked files (upload time, licence, flags)
    CP->>CP: validate lock; evaluate policy and advisories
    R->>CP: claim install job
    R->>G: fetch each locked file; check hash
    R->>IC: archives on standard input (no network)
    IC-->>R: canonical layer on standard output
    R->>CP: upload layer per target (streamed to the blob store)
    R->>CP: complete build (bundle 5, SBOM)
    CP->>CP: check SBOM = lock, layer digests; sign; version READY
```

A version without a manifest takes the path of feature 005 unchanged. The resolve and install jobs are kinds of the
existing build job (the same claim, heartbeat, retry, `not_before` and `script_jobs` quota of feature 005, Decisions 99
and 109), so a build that is already cached by its key is finished by the claim and never runs again.

The build key (feature 005, Decision 99) gains three parts: the manifest digest, the lock digest (empty when the
platform resolves; the resolved lock's digest from then on) and the unpacker's identity. The policy digest is not part
of the key: a policy change does not rebuild a layer, it re-evaluates findings.

### 2. The manifest

Python: the PEP 723 block is parsed from the source by the control plane (TOML, bounded, only the keys `dependencies`
and `requires-python`; `tool` tables refused). `requires-python` must admit the runtime's interpreter version. Entries
are PEP 508 requirements without URLs, without extras that name URLs, and with environment markers evaluated against the
runtime (Python version, `sys_platform == "linux"`, the two machines).

TypeScript: the draft's `manifest` is a JSON object with `dependencies` (name to npm range, no `git:`, `file:`,
`link:`, `http:`, `workspace:` or `npm:` alias specifiers) and optional `overrides` (same rules), nothing else. The
script imports packages by bare specifier.

The proposal endpoint `POST /v1/scripts/{id}/draft/dependencies:propose` reads the draft's imports and returns
candidate entries with the reason for each (`import yaml` proposes `PyYAML`, from a mapping the gateway serves for the
top import names of each ecosystem). It writes nothing. The platform never applies it on its own, because a guessed name
is how a typosquatted package gets in (Decision 046-003).

### 3. The package gateway

`apps/package-gateway` is a Rust service with no database. It serves, under `/npm/` and `/pypi/`:

- npm: the package document (`GET /npm/{name}`), filtered, and tarballs (`GET /npm/{name}/-/{file}`).
- PyPI: the project page of the simple API in its JSON form (`GET /pypi/simple/{project}/`, PEP 691), filtered, and
  files (`GET /pypi/files/{sha256}/{filename}`).
- `POST /internal/v1/metadata`: for a list of (ecosystem, name, version, hash), the upload time, the declared licence,
  the install-script flag, yanked, and whether the file is in the store. Called by the control plane only.
- `GET /healthz` for the compose healthcheck.

Every request carries a job token: a short-lived (15 minutes) Ed25519-signed token issued by the control plane with
the audience `package-gateway`, the tenant, the namespace, the job, the ecosystem and the policy digest. The gateway
verifies it with the control plane's public key, configured out of band as for runners (feature 005, Gap 102). The
control plane's own calls use a separate service token. The gateway fetches the policy of a digest from the control
plane once and caches it by digest.

What it serves is filtered by that policy, so a resolver never sees what the policy forbids: versions younger than the
quarantine (by the registry's upload time), deny rules, yanked versions, for Python every source distribution and every
wheel whose tags do not match a supported target, and, for a name the namespace has uploaded, every public version
(Technical Plan 10). Filtering is a convenience and a second wall; the control plane's evaluation (Technical Plan 6) is
the authority.

Files are fetched upstream on first request, checked against the registry's declared hash (npm `dist.integrity`, PyPI
`hashes.sha256`), and stored in the blob store under `packages/sha256/<hex>`, never replaced. Package documents and
project pages are cached for 10 minutes. Public files are shared by all tenants (they are public); uploaded files are
stored under the tenant's prefix and served only to that tenant's tokens.

Upstreams are configured by the operator (default `https://registry.npmjs.org` and `https://pypi.org`; any compatible
mirror an organization runs). The gateway's outbound connections use the platform's guarded client
(`crates/infra-egress`: resolve once, check every address, refuse private ranges unless the operator allows the
upstream's). In `offline` mode it has no upstream and serves only its store. Two commands move a store between
deployments: `package-gateway export --lock <file>... --out <bundle>` and `package-gateway import <bundle>`, a tar of
files named by hash plus their metadata; import checks every hash.

### 4. The resolve job

The runner claims a resolve job as it claims a build. The claim returns the source, the manifest, the runtime
(interpreter version, targets), the gateway URL and the job token. The runner starts a container from the pinned
resolver image of the language (by digest; `runners/resolver-python`, `runners/resolver-node`) with the feature 005
profile, except that it is attached to the engine network `sagat-packages` (internal: no route outside), on which only
the gateway listens. The first message on standard input carries the manifest, the targets and the token; the resolver
shim writes the tool's configuration in the scratch area and runs:

- Python: the pinned `uv` with the gateway as its only index, binary wheels only, the quarantine as its
  `exclude-newer` cut, the runtime's Python version and the two targets as the environments to resolve for, writing a
  PEP 751 lock.
- TypeScript: the pinned `npm` in lock-only mode, scripts ignored, the gateway as its registry, the quarantine as its
  `before` date, writing `package-lock.json` version 3.

The exact command lines are fixed by the probe of Task T046-12, which proves each against the pinned versions with no
network but the gateway (Gap 046-005). The lock comes back on standard output in the bounded result frame (256 KiB). A
resolver that fails returns its first 8 KiB of error output, redacted and kept in the build log; the build fails with
kind `resolve` and the reason `RESOLUTION_FAILED`, not retryable unless the gateway answered `503`.

An author-supplied lock skips the resolve job; the control plane checks it like a resolved one (Technical Plan 5).

### 5. Lock validation

The control plane (`crates/infra-packages`, Technical Plan 13) parses the lock in a closed subset of each format
(`contracts/lock-formats.md`) and refuses anything outside it. The rules of Requirement 046-007, plus: every manifest
entry is satisfied by a root entry (npm ranges with the node semantic-version rules; PEP 440 specifiers and PEP 508
markers for Python); the closure is complete (every dependency of every entry is present, nothing unreachable from a
root); no two entries for one name and target in Python; npm `node_modules` paths are canonical (no `..`, no absolute,
at most 1,024 bytes). The lock's digest is the SHA-256 of its bytes as stored.

### 6. The supply-chain policy

The policy is a versioned document of the namespace (`contracts/supply-chain-policy.schema.json`), stored beside the
language policy of feature 023 (User story 023-06) and changed through the same endpoint. Its fields and defaults:

| Field | Default | Bounds |
|---|---|---|
| `npm.enabled`, `pypi.enabled` | true | |
| `allow` | empty (everything not denied) | 500 rules: name pattern, optional version range |
| `deny` | empty | 500 rules |
| `quarantineHours` | 72 | 0 to 720 |
| `vulnerabilities.blockAtOrAbove` | `HIGH` | `CRITICAL`, `HIGH`, `MODERATE`, `LOW`, `NONE` |
| `vulnerabilities.blockUnrated` | false | |
| `vulnerabilities.blockNewAtLease` | false | |
| `licences.deny` | empty | 100 SPDX identifiers |
| `installScripts` | `skip` | `skip` (admit with a finding) or `refuse` |
| `waivers` | empty | 200; each one advisory, one package, an expiry of at most 90 days, a reason |
| `maxLayerBytes` | 1 GiB | up to the platform ceiling, 4 GiB |

A non-empty `allow` makes the policy an allow-list. Malicious-package advisories are not a field: they are always
refused (ADR-0121). A change takes effect for new publications and for lease checks; it never rebuilds a version. Each
change is audited with the old and new digests.

Evaluation takes, for each locked entry, the gateway's metadata (Technical Plan 3) and the advisory matches (Technical
Plan 7) and returns the findings and a verdict. The verdict refuses on: a deny rule; not allowed by a non-empty allow
list; younger than the quarantine at evaluation time; an advisory at or above the threshold without a live waiver; an
unrated advisory if `blockUnrated`; a denied licence (an SPDX expression is denied when every alternative of it contains
a denied identifier; an unparseable or absent licence is a finding, not a refusal); an install script with `refuse`; a
layer over `maxLayerBytes` (known after install, checked again before signing). The report (verdict, findings, the
snapshot's identity and age, the policy digest) is stored with the version and its digest is in the signed bundle.

### 7. The advisory snapshot

The control plane keeps the OSV records of the `npm` and `PyPI` ecosystems in `package_advisories`, with their
identifiers, aliases, kind (`MAL-` identifiers are the malicious kind), severity and affected ranges. Severity is the
GitHub advisory label when the record has one, otherwise the base score of its CVSS v3 or v4 vector mapped to the same
four labels; a record with neither is unrated.

The snapshot is imported in two ways: a leased control-plane pass (one replica at a time, every 6 hours by default)
downloads the OSV archive of each ecosystem through the gateway's upstream connection, or an operator imports the same
archives with `control-plane advisories import <file>` (offline). Each import is one transaction that replaces the
ecosystem's rows and records the snapshot's identity, time and count. Matching uses the ecosystem's version order (node
semantic versions; PEP 440) over the record's ranges and listed versions, and is property-tested against both orders.

After each import, a re-scan pass re-evaluates the stored locks of every tenant in pages, under the version rows'
locks taken one page at a time, records new and resolved findings, and sets or clears the block of a version that holds
a malicious package (ADR-0121). A snapshot older than 7 days is a warning on every publication and a metric.

### 8. Installation and the layer

The runner claims an install job, fetches each locked file from the gateway (job token), checks its hash, and keeps
it in a local cache named by hash. For each target the version supports, it starts an install container from the pinned
layer-tool image (`runners/layer-tool`, the platform's unpacker, a static Rust binary) with the feature 005 profile and no
network, and streams the files selected for that target on standard input. The unpacker:

- Python: unpacks each wheel into `site-packages/` following the wheel format (the `purelib` and `platlib` roots and
  their `.data` directories; `scripts`, `headers` and `data` are dropped), checks every file against the wheel's
  `RECORD`, and keeps the `.dist-info` directory for `importlib.metadata`.
- TypeScript: unpacks each tarball into the `node_modules` path its lock entry names, stripping the archive's top
  directory; `bin` links are not created.
- Both: refuses links, absolute paths, `..` segments, devices and other special files, set-user-id and set-group-id bits,
  a path over 1,024 bytes, more than 200,000 files, and a total over the policy's `maxLayerBytes`; writes a tar with
  entries sorted by path, time 0, owner 0, mode 0444 for files and 0555 for directories.

For Python the install container is the runtime image itself with the unpacker added, so that it then compiles the
layer's modules to bytecode with the checked-hash invalidation mode, which reads the files and runs none of their code.
The layer's SHA-256 is computed by the runner on the stream, which it uploads to the control plane
(`PUT /script-runner/layers/{job}/{target}`), which streams it to the blob store under
`script-layers/<tenant>/<sha256>` (ADR-0023 machinery) and records it in `script_dependency_layers`. Layers count against
the tenant's content bytes (ADR-0027). A layer is kept while any version references it, and deleted 30 days after the
last reference goes.

Doing the unpacking in a sandbox container and not in the runner means that an archive crafted to exploit the unpacker
lands in a container with no network and nothing to steal, not in the process that holds the container engine's socket.
Its output is untrusted for the same reason any package is: it runs only in that tenant's sandbox.

### 9. The bundle, the signature, the SBOM and the provenance

Bundle version 5 (`contracts/bundle-v5.md`) is the version 1 text bundle plus a `dependencies` object: `ecosystem`,
`runtime` (the runtime image line, Technical Plan 11), `manifestDigest`, `lockDigest`, `policyDigest`,
`policyReportDigest`, `sbomDigest`, `advisorySnapshot`, `unpacker`, and `layers`, a map from target to `{digest, size}`.
Bundles 1 to 4 do not change; feature 023 uses version 4 for PHP and the reserved `python-data` runtime.

The control plane signs a bundle 5 as it signs every bundle (feature 005, Decision 98: Ed25519 over
`sagat/script-artifact/v1 ‖ tenant ‖ digest ‖ language`), only after it has checked that the SBOM's package set equals the
lock's, that each layer digest names a stored layer of the tenant, and that the policy verdict admits the lock. The SBOM
is CycloneDX 1.5, as today, with a component per locked package (package URL, version, SHA-256 or SHA-512, declared
licence). The provenance is the in-toto statement of ADR-0057 decision 6 with the lock's entries as resolved
dependencies (gateway path and hash) and the gateway, resolver, unpacker and snapshot identities as builder parameters;
it is built by the module feature 023 defines for it, or by this design if it is built first (Technical Plan 13).

### 10. Private packages

`PUT /v1/namespaces/{namespace}/packages/{ecosystem}` with a wheel or an npm tarball (at most 100 MiB) stores it under
the tenant's prefix, records name, version, hash and uploader in `private_packages`, and audits it. A version, once
uploaded, cannot be replaced (a new version can). The CLI wraps it (`sagat package push`), so a tenant's CI can publish.
A name that a namespace has uploaded is a private name for that namespace: the gateway hides every public version of it
from that namespace's tokens, and the control plane refuses a lock that takes a private name from a public source. This
removes dependency confusion by construction (Decision 046-010). Private names do not need a scope, but npm names under
a scope the namespace claims (`@acme/`) are private for every version, uploaded or not.

### 11. The run

A version with packages runs on a dependency runtime line: a pinned glibc-based image of the same interpreter minor as
the framework-only line (`python-3.13-glibc`, `node-24-glibc`), because native wheels and prebuilt npm binaries are most
often published for glibc. Framework-only versions keep their images and their build keys.

When the runner verifies a bundle 5 for its native target, it looks up the engine volume `sagat-layer-<tenant
digest>-<layer digest>`. If it is absent, the runner downloads the layer from the control plane
(`GET /script-runner/layers/{digest}`, which streams from the blob store), checks its SHA-256 on the stream, and
populates a new volume with a container of the feature 005 profile from the layer-tool image, which has that volume
mounted read-write and the archive on standard input. The runner then labels the volume with the tenant, the digest and
the time, and never mounts it writable again. The run container gets the volume read-only, `nosuid` and `nodev` at
`/opt/sagat/deps`; the scratch area stays `noexec`. The layer is mounted without `noexec`, because native extensions are
mapped executable; its files are fixed by digest and nobody can write them during a run.

The shims add the layer to the module path: Python inserts `/opt/sagat/deps/site-packages` in `sys.path` and sets
`PYTHONDONTWRITEBYTECODE=1`; Node resolves bare specifiers of the script from the layer's `node_modules` (a link from the
scratch area's script directory to the read-only mount, written by the shim before the script is loaded), for both ES
modules and CommonJS. The probe of Task T046-16 proves both forms of import and a native extension in each language.

The runner keeps volumes under a least-recently-used bound (50 GiB by default); a volume in use is never removed. A
volume is never mounted for another tenant, even when the digest is the same, so that a cache hit cannot tell one
tenant what another uses (Decision 046-011).

Warm containers (feature 019, ADR-0052) key a warm container by artifact digest, which names the layer, so a warm
container has the same read-only mount for its whole life; the reset between invocations does not touch it, because
nothing can be written there. Lease, retry, timeout, cancellation, step logs (ADR-0084), host calls and result handling
are those of feature 005.

### 12. Blocking at lease

The block of ADR-0121 is a column of the per-tenant artifact row that the lease-time language check of feature 023 reads
(ADR-0057 decision 8, Task T023-061): `runnable = false` with a reason. If this design is built before 023's check,
Task T046-17 builds that check (one read of that row per script activity claim) and 023 extends it with the language. The
engine fails a step whose artifact is not runnable in the lease transaction, as non-retryable `SCRIPT_DENIED` with
reason `PACKAGE_BLOCKED`, its journal event, next work, outbox rows and a `script.supply_chain_blocked` audit record.
An attempt already leased finishes. The claim path gains no statement beyond the check of feature 023; design 038's
budget test covers it.

### 13. Where the code goes

| Part | Place |
|---|---|
| Manifest, lock and policy types; the evaluation rules | `crates/domain/src/scripts/packages.rs` (no I/O, no `serde_json` as a rule) |
| Lock parsers, version orders, advisory matching, SBOM and provenance builders | new crate `crates/infra-packages` (adapter) |
| Publication flow, resolve and install job orchestration, re-scan pass, waivers | `crates/application/src/scripts/packages.rs` |
| Tables, queries, migrations | `crates/infra-postgres` |
| REST (author, policy, findings, uploads; the runner's layer and resolve routes) | `crates/api-http/src/routes/{scripts,script_packages,runner}.rs` |
| Container driver: the gateway network, the volume, the read-only mount, the install and populate containers | `crates/infra-sandbox` |
| Runner jobs and the layer cache | `apps/sandbox-runner` |
| The gateway | new app `apps/package-gateway` |
| Resolver, layer-tool and dependency runtime images | `runners/resolver-python`, `runners/resolver-node`, `runners/layer-tool`, `runners/python-glibc`, `runners/node-glibc` |
| Studio panel | `frontend/apps/studio` (script editor) |
| CLI | `apps/cli` |

`scripts/check-architecture.py` gains the rule that only `apps/package-gateway` depends on an upstream registry client,
and that `crates/domain` does not depend on `crates/infra-packages`.

### 14. Data

Additive migrations (`specs/046-script-dependencies/data-model.md` is written with Task T046-2): `script_version_dependencies`
(tenant, version, ecosystem, manifest digest, lock bytes, lock digest, policy digest, report, snapshot id),
`script_dependency_layers` (tenant, digest, target, size, blob key, created, last referenced),
`script_supply_chain_findings` (tenant, version, package, version, kind, advisory id, severity, first seen, resolved),
`supply_chain_waivers`, `private_packages`, `package_advisories`, `package_advisory_snapshots`; a `runnable` and
`blocked_reason` pair on the artifact row of feature 023 (or added by this design). Grants for `sagat_app` follow the
migration checklist (`/migration`); the down migration refuses while bundle-5 rows exist.

### 15. Telemetry and audit

Metrics (closed labels): `script_dependency_resolve_seconds{ecosystem,outcome}`,
`script_dependency_install_seconds{ecosystem,outcome}`, `script_dependency_layer_bytes{ecosystem}`,
`script_layer_cache_total{result}` (`hit`, `miss`, `evicted`), `script_layer_populate_seconds`,
`package_gateway_requests_total{ecosystem,kind,result}`, `package_gateway_upstream_seconds{ecosystem}`,
`script_supply_chain_findings_total{kind,severity}`, `package_advisory_snapshot_age_seconds{ecosystem}`. Spans on each job,
each gateway request and each evaluation, with tenant, namespace, script, version and package names as attributes. Audit
records: `script.dependencies_resolved`, `script.supply_chain_refused`, `script.supply_chain_blocked`,
`script.supply_chain_unblocked`, `supply_chain_policy.changed`, `supply_chain_waiver.added|expired|removed`,
`package.uploaded`, `advisory_snapshot.imported`.

### 16. Compose, deployment and operations

`infra/docker/compose.yaml` gains the service `package-gateway` (built from `infra/docker/Dockerfile`, target
`package-gateway`; the hardening anchor; port `127.0.0.1:${SAGAT_PACKAGE_GATEWAY_PORT:-18090}:8090`; a healthcheck on
`/healthz`; `SAGAT_PACKAGE_GATEWAY_MODE` `online` or `offline`) and the network `sagat-packages` with `internal: true`,
which the gateway joins beside the default network. The sandbox runner learns the network name
(`SAGAT_RUNNER_PACKAGE_NETWORK`) and the gateway URL. The Helm chart gains the same, with a network policy that lets only
resolve pods and runners reach the gateway and lets only the gateway leave the cluster to its upstreams. The runbook
`docs/runbooks/script-dependencies.md` covers policy, waivers, advisory imports, offline bundles, the cache bound and a
blocked version.

### 17. Compatibility and rollout

Everything is additive. With the capability absent, nothing changes. Recording the capability requires every control
plane, engine and runner to list it (ADR-0082). Rolling back the binaries after bundle-5 versions exist leaves versions
an old runner cannot run (it refuses an unknown bundle version with `SCRIPT_UNTRUSTED_ARTIFACT`); the release notes say
rollback is forward-only for data, as ADR-0057 does.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A malicious package published upstream is resolved before anyone knows | Quarantine of 72 hours by default; malicious-package advisories refused and blocked at lease; the gateway is the only path, so a fix of the policy applies everywhere at once |
| A package needs its install script to work | Admitted with a finding; the author sees `INSTALL_SCRIPT_SKIPPED`; Gap 046-001 for a reviewed exception mechanism |
| The unpacker is exploited by a crafted archive | It runs in a sandbox container with no network; its output is treated as untrusted package content |
| Disk on runners fills with layers | LRU bound; layers are per tenant; metric of cache bytes; Gap 046-004 |
| The resolver tools change behaviour across versions | Pinned by image digest, part of the build key; the probe runs in CI on both targets |
| The advisory snapshot is stale | Age metric and warning; offline import command |
| Side channel through shared caches | Layer volumes are per tenant; gateway caches hold only public content or the tenant's own |

## Constitution Check

Written against version 5.0.0 as approved by Owner decision 2026-11-24-2 (Principles I to VIII unchanged in substance by
that amendment for this design).

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Resolve and install are build jobs with the existing lease, heartbeat and retry; a version becomes `READY` only in the build-completion transaction that stores lock, report and layer references; the lease-time block fails a step in the lease transaction with its journal event and outbox (Technical Plan 12) |
| II. Modular domain, ports and adapters | Pass | Rules in the domain without I/O; parsers and matchers in a new adapter crate; the gateway is an app; architecture rules added (Technical Plan 13) |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect of a run is added; a script's own effects keep the idempotency key of feature 005 |
| IV. Compatibility is versioned and pinned | Pass | A version pins its lock, layer digests, policy digest and snapshot identity in the signed bundle; bundles 1 to 4 unchanged; resolvers, unpacker and runtimes pinned by digest and part of the build key |
| V. Code, tenant boundary and secrets are isolated | Pass with conditions (Complexity Tracking) | No package code runs outside a run container; resolve has only the gateway as network and no secret; install has no network; the run container keeps no network and gains one read-only mount; volumes and uploads are per tenant; the gateway holds no tenant secret (Gap 046-002 keeps external private registries out) |
| VI. Observability and audit are part of the feature | Pass | Metrics with closed labels, spans, audit records (Technical Plan 15) |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | No new message path; the block's step failure writes its outbox rows in the lease transaction |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Hostile battery, crash of a runner during resolve, install and populate, duplicate completion, gateway outage, offline build, snapshot re-scan (Verification Plan); gaps registered |
| Delivery Constraints | Pass | Self-contained compose service, pinned images, loopback port, healthcheck; no new region or transport; targets stated as benchmark criteria |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A new service, the package gateway | One audited path to registries that applies the namespace's policy and works offline | Each runner talking to registries: many network paths, no shared policy, no offline mode |
| A sandbox container with a network (the resolve container) | A resolver needs registry metadata | Resolving in the control plane: third-party tools on untrusted input in a trusted process |
| One read-only mount in the run container (Principle V, ADR-0013's "nothing mounted") | A dependency tree does not fit the 2 MiB bundle and must not be downloaded by a run | Streaming the tree into the scratch area at each run: cost per run and a writable copy |
| Bundle version 5 | The bundle must name the layer so that the signature binds it | Signing the layer separately: a second signature path to verify and rotate |
| An advisory snapshot and two version orders | Policy needs facts about vulnerabilities and malware without calling a third party per publication | An online scanning service: no offline mode, a third party sees every tenant's packages |
| Dependency runtime images on glibc | Native wheels and prebuilt binaries are most often published for glibc | Keeping Alpine: many packages would be refused as unsupported |

## Alternatives Considered

- **A closed catalog for TypeScript and Python** (ADR-0021's model). Strongest isolation, but it cannot serve the
  ecosystems; rejected for these two languages, kept for the others.
- **Install at run time with network** (Windmill). Rejected: a network path in every run, install scripts, a registry
  outage breaks runs.
- **A third-party repository manager as the proxy.** Rejected for a small gateway the platform controls (ADR-0120,
  option 4); an operator can still chain one as the gateway's upstream.
- **Infer the manifest from the imports at publication** (Windmill). Rejected: a guessed name is a typosquatting path
  (Decision 046-003); offered as a proposal the author saves.
- **Run the installer (`uv pip install`, `npm ci`) in the install container.** Workable, but an installer is a large
  program whose behaviour differs by version; unpacking verified archives is small, deterministic and enough once
  install scripts and source builds are excluded.
- **One shared layer volume for every tenant with the same digest.** Saves disk; rejected for the cache side channel and
  feature 005's rule that the cache belongs to the tenant.
- **Allow source distributions, built in the sandbox.** Possible later (Gap 046-001); the build needs its own backend
  dependencies, which doubles the resolution.
- **Proxy tenants' private registries with their credentials.** Needs a new secret disclosure path; rejected for 0.3.0
  in favour of uploads (Gap 046-002).

## Non-Functional Requirements

As stated under Requirements (Non-functional requirements 046-001 to 046-005). Their tests: the hostile battery for
046-001; the cross-runner digest test for 046-002; the benchmark of Task T046-23 for 046-003; the bound fixtures of
Tasks T046-3, T046-4 and T046-13 for 046-004; the metric-label test of Task T046-21 for 046-005.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Documentation first: Task T046-1 lands before any code. Build slices for two builders that never touch the same crate at
the same time:

- **Slice 1 (builder A): rules and control plane.** Tasks T046-2 to T046-5, T046-10, T046-11, T046-15, T046-17,
  T046-18, T046-21. Crates `domain`, `infra-packages` (new), `application`, `infra-postgres`, `api-http`.
- **Slice 2 (builder B): gateway, runner and images.** Tasks T046-6 to T046-9 (gateway side), T046-12 to T046-14,
  T046-16, T046-20. Apps `package-gateway` (new), `sandbox-runner`; crate `infra-sandbox`; `runners/`;
  `infra/docker`, `infra/helm`.
- **Slice 3 (either, after both): surfaces and proof.** Tasks T046-19, T046-22, T046-23, T046-24. `frontend/apps/studio`,
  `apps/cli`, `tests/`.

The contract between slices 1 and 2 is the set of files in `specs/046-script-dependencies/contracts/` (Task T046-1):
lock subsets, bundle 5, the runner routes, the gateway's token and metadata API.

- [ ] T046-1 Write the contracts (`contracts/lock-formats.md`, `contracts/bundle-v5.md`, `contracts/package-gateway.md`,
  `contracts/supply-chain-policy.schema.json`, `contracts/errors.md`), `data-model.md`, the OpenAPI additions, the
  runbook draft, and update `docs/architecture/security-and-sandbox.md` (Gap 97 row) before code.
  Refs: Requirement 046-001, Requirement 046-007, Requirement 046-011, Requirement 046-018
- [ ] T046-2 Domain types: manifest (both forms), lock entries, policy document, findings, verdict; evaluation rules as
  pure functions with unit tests; the additive migrations and grants.
  Refs: Requirement 046-001, Requirement 046-008, Requirement 046-017
- [ ] T046-3 npm lock parser and validator (closed subset, hashes, canonical paths, closure, manifest satisfaction with
  node semantic-version rules), with one refused fixture per rule and the bounds.
  Refs: Requirement 046-002, Requirement 046-007
- [ ] T046-4 PEP 751 lock parser and validator (wheels only, tags against targets, PEP 440 and PEP 508), fixtures and
  bounds.
  Refs: Requirement 046-002, Requirement 046-005, Requirement 046-007
- [ ] T046-5 Advisory model, OSV import (pass and command), version-order matchers with property tests, severity
  mapping, snapshot age metric.
  Refs: Requirement 046-008, Requirement 046-009, Requirement 046-016
- [ ] T046-6 Gateway: npm package documents and tarballs, filtering by policy, hash check against upstream, blob store,
  job and service tokens, guarded egress, healthcheck.
  Refs: Requirement 046-004, Requirement 046-015
- [ ] T046-7 Gateway: PyPI simple API (PEP 691 JSON) and files, wheels-only filtering, tag filtering.
  Refs: Requirement 046-005, Requirement 046-015
- [ ] T046-8 Gateway: `offline` mode, `export` and `import`, the metadata endpoint.
  Refs: Requirement 046-015
- [ ] T046-9 Private packages: upload endpoint and CLI, `private_packages`, the private-name rule in the gateway and in
  the lock validator, scoped npm names.
  Refs: Requirement 046-014
- [ ] T046-10 Publication flow: resolve and install job kinds on the build job, build key parts, policy evaluation
  before signing, the stored report, typed refusals.
  Refs: Requirement 046-002, Requirement 046-008, Requirement 046-018, Requirement 046-019
- [ ] T046-11 Re-scan pass after each snapshot import, findings, the block and unblock of versions, audit.
  Refs: Requirement 046-009, Requirement 046-016
- [ ] T046-12 Resolver images (`runners/resolver-python`, `runners/resolver-node`), shims, the gateway network in the
  container driver; the probe that fixes the command lines and proves no network but the gateway, on both targets.
  Refs: Requirement 046-004, Requirement 046-005, Requirement 046-006
- [ ] T046-13 Layer tool (`runners/layer-tool`): the unpacker, its refusals and bounds, canonical output, bytecode
  compilation for Python; the install job in the runner with the hash checks.
  Refs: Requirement 046-010
- [ ] T046-14 Layer upload and download routes, blob store keys, `script_dependency_layers`, quota accounting,
  retention after the last reference.
  Refs: Requirement 046-010, Requirement 046-011
- [ ] T046-15 Bundle 5 validation and signing checks (SBOM equals lock, layer digests, verdict), SBOM components, the
  provenance statement (shared with Task T023-047).
  Refs: Requirement 046-011, Requirement 046-013
- [ ] T046-16 Runner: dependency runtime images, layer cache volumes per tenant and digest, populate container,
  read-only mount, shims for both module systems and native extensions, LRU eviction, warm-container compatibility.
  Refs: Requirement 046-012
- [ ] T046-17 Lease-time block: the `runnable` column and the check in the lease transaction (or the extension of Task
  T023-061), `SCRIPT_DENIED` with `PACKAGE_BLOCKED`, the budget test of design 038.
  Refs: Requirement 046-009, Requirement 046-018
- [ ] T046-18 REST: draft manifest, proposal endpoint, version lock and report, SBOM, policy, waivers, findings;
  OpenAPI and its contract tests; the capability `script-dependencies`.
  Refs: Requirement 046-003, Requirement 046-017, Requirement 046-019
- [ ] T046-19 Studio panel (manifest, lock, findings, policy) and CLI (`sagat script deps`, `sagat package push`).
  Refs: Requirement 046-020
- [ ] T046-20 Compose service and network, Dockerfile target, Helm values and network policy, smoke test.
  Refs: Requirement 046-015
- [ ] T046-21 Telemetry and audit of Technical Plan 15, with the metric-label test.
  Refs: Requirement 046-018
- [ ] T046-22 The hostile battery and the fault tests of the Verification Plan on both targets.
  Refs: Requirement 046-004, Requirement 046-010, Requirement 046-012
- [ ] T046-23 Benchmark report (`tests/performance/reports/script-dependencies.md`) with environment and failures.
  Refs: Requirement 046-001
- [ ] T046-24 Runbook, reference pages, quickstart, release notes; Gap 97 of feature 005 marked closed for TypeScript
  and Python with the evidence.
  Refs: Requirement 046-020

#### Follow-up tasks

None yet.

### Verification Plan

- **Unit and property tests:** the lock validators (Tasks T046-3 and T046-4), the matchers against both version orders
  (Task T046-5), the policy rules (Task T046-2), the unpacker's refusals (Task T046-13).
- **Integration (real PostgreSQL, a real gateway with a recorded upstream):** publication with each manifest form, an
  author lock, each refusal reason, the snapshot import that blocks a version and the lease that fails, the waiver
  expiry, private names, offline mode with an imported bundle.
- **Real containers (`integration_script_dependencies_containers`, the native-runner CI job on AMD64 and ARM64):** the
  hostile battery: a resolve container tries the internet, the control plane, the engine, the runner host and the cloud
  metadata address (all refused); a package with a `postinstall` that writes a file and one whose `setup.py` would run
  (neither runs); a script writes into the layer (refused); two tenants with one lock (two volumes); a tampered layer,
  bundle and cross-tenant layer (refused before start); a native extension imports in each language.
- **Faults:** the runner dies during resolve, install and populate (the job is retried; a half-populated volume without
  its label is removed and populated again); the gateway is down (retryable failure, no version change); duplicate
  completion of a build (refused as stale); the control plane restarts during a re-scan (the pass resumes by page).
- **Contracts:** OpenAPI lint and conformance; the gateway's contract against recorded npm and PyPI responses.
- **Benchmark:** Task T046-23.
