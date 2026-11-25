# Research and registers: package dependencies for TypeScript and Python scripts (design 046)

The design is [docs/designs/046-script-dependencies.md](../../docs/designs/046-script-dependencies.md). This file holds
its registers, append-only: decisions, gaps and owner questions. Every product or design choice below was taken under
the delegation of Owner decisions 2026-11-20-6 and 2026-11-24-4, weighing every option for the most architectural and
business advantage; each says "delegate (OD-2026-11-24-4)" and why.

## Sources read

- Feature 005: Decisions 95 to 118 and Gaps 96 to 120 (`specs/005-inline-code-sandbox/research.md`), in particular
  Decision 99 (the build key), Decision 105 (lockfiles refused for TypeScript and Python) and Gap 97.
- ADR-0013, ADR-0021, ADR-0023, ADR-0024, ADR-0027, ADR-0052, ADR-0057, ADR-0082.
- The code: `crates/infra-sandbox/src/{docker,lockfile,protocol}.rs`, `crates/infra-schema/src/bundle.rs` (bundles 1 to 3),
  `crates/domain/src/scripts.rs` (`Language`, `Lockfile`, 256 KiB), `apps/sandbox-runner`, `crates/infra-blobstore`,
  `infra/docker/compose.yaml`.
- Standards and tools named in the design (to be proved by the probe, Gap 046-005): PEP 723 (inline script metadata),
  PEP 751 (`pylock.toml`), PEP 691 (simple repository API in JSON), PEP 440, PEP 508, the wheel format (`RECORD`,
  `.data`), npm `package-lock.json` version 3 and the registry's package document, the OSV schema and its per-ecosystem
  archives, CycloneDX 1.5, in-toto Statement v1 and SLSA Provenance v1.
- Windmill's documented behaviour for dependencies (deployment-time resolution with `uv` and `bun`, lockfiles stored
  with the script, worker-side installation from public or private registries, a worker cache).

## Decisions

| ID | Decision | Reasoning |
|---|---|---|
| D046-001 | Resolve at publication, never at run time; a new resolution is a new version. delegate (OD-2026-11-24-4) | Principle IV pins what runs; a run that resolves depends on a registry being up and on what it serves that day. Windmill also resolves at deployment, so nothing is lost in convenience |
| D046-002 | Python declares packages in a PEP 723 block of the source; TypeScript in a `package.json` of only `dependencies` and `overrides`, stored with the draft. delegate (OD-2026-11-24-4) | PEP 723 is the standard for single-file scripts and runs locally with the same tools, so the author's laptop and the platform agree. TypeScript has no inline standard; a `package.json` subset is what every Node developer and tool reads. Alternatives (Deno `npm:` specifiers, version comments in imports) are non-standard and tie the source to this platform |
| D046-003 | No manifest inference at publication; a proposal endpoint suggests entries that the author saves. delegate (OD-2026-11-24-4) | An import name is not a package name (`import yaml` is PyYAML); a guessed name is how a typosquatted package enters. Windmill infers silently. The proposal keeps most of the convenience and leaves the choice with a person |
| D046-004 | One platform-built gateway is the only component that reaches registries; it has no database. delegate (OD-2026-11-24-4) | One audited path, the namespace's policy applied as a filter before a resolver sees anything, offline mode, self-contained in the compose. A third-party manager (Nexus, Artifactory, Verdaccio, devpi) has its own policy model and image to patch; an operator can still chain one as upstream. No database keeps the gateway stateless and horizontally scalable; the blob store holds files |
| D046-005 | Resolution runs in a sandbox container whose only network is the gateway's internal network, with no secret. delegate (OD-2026-11-24-4) | Resolvers are large third-party programs reading untrusted metadata; they belong in the sandbox (Principle V), not in the control plane or the runner. The internal network gives them exactly one destination |
| D046-006 | Wheels only for Python; npm lifecycle scripts never run, the package admitted with a finding unless the policy says `refuse`. delegate (OD-2026-11-24-4) | A source build and an install script are arbitrary code at install time. Refusing every npm package that declares a script would refuse many that work without it (pnpm 10 stopped running dependency scripts by default for the same reason); the finding makes the skip visible. Source builds in the sandbox are possible later (Gap 046-001) |
| D046-007 | Installation is unpacking by the platform's own tool in a sandbox container with no network; no installer runs. delegate (OD-2026-11-24-4) | Once scripts and source builds are excluded, installing is unpacking; a small deterministic tool gives a reproducible layer digest, and running it in the sandbox keeps a crafted archive away from the process that holds the engine socket |
| D046-008 | The layer is a canonical archive per native target, stored in the blob store, named by the signed bundle 5. delegate (OD-2026-11-24-4) | The tree does not fit the 2 MiB bundle (Gap 108 of feature 005); naming it from the bundle reuses the existing signature instead of adding a second one |
| D046-009 | A run mounts the layer read-only, `nosuid`, `nodev`, from a runner volume populated once; not `noexec`. delegate (OD-2026-11-24-4) | Streaming the tree per run costs time in proportion to its size; an image per lock needs an image builder on the runner host. Native extensions must be mapped executable; the files are fixed by digest and read-only, so allowing execution there gives a script nothing it does not already have (it can already run arbitrary code in its interpreter) |
| D046-010 | A name a namespace uploads is private for it: public versions are hidden and refused for that namespace; a claimed npm scope is private entirely. delegate (OD-2026-11-24-4) | Dependency confusion is removed by construction, not by ordering of indexes, which is what most tools get wrong |
| D046-011 | Layer volumes and private files are per tenant, even when content is identical. delegate (OD-2026-11-24-4) | A shared cache would let a tenant learn by timing what another tenant uses; feature 005's rule is that the cache belongs to the tenant. Runners serve one tenant today (Gap 99 of feature 005), so the duplication costs nothing yet |
| D046-012 | Default policy: both ecosystems enabled, quarantine 72 hours, block at HIGH and above, unrated advisories reported, new findings after publication reported, malicious packages always refused and blocked. delegate (OD-2026-11-24-4) | The defaults must make the platform safer than Windmill on day one without refusing ordinary work. 72 hours lets most malicious releases be detected and removed before they can be chosen while a fix of a real vulnerability is a short wait (a waiver or a quarantine of 0 for one namespace covers urgent cases). Unrated advisories are many in PyPI and blocking them would refuse much ordinary work; blocking every new finding at lease would stop production on a disclosure that may not apply. Malware is never acceptable |
| D046-013 | Advisories come from an imported OSV snapshot, refreshed by a leased pass or imported by an operator. delegate (OD-2026-11-24-4) | No third party sees which packages a tenant uses; works offline; one format for both ecosystems with malicious-package records (`MAL-`) included |
| D046-014 | A malicious-package match blocks leases of the versions that contain it, in the lease transaction (ADR-0121). delegate (OD-2026-11-24-4) | Known malware must stop at its next step, not at the next publication. Reusing feature 023's lease-time check costs no new statement |
| D046-015 | Dependency-bearing versions run on glibc runtime images of the same interpreter minor; framework-only versions keep their images. delegate (OD-2026-11-24-4) | Native wheels (`manylinux`) and prebuilt npm binaries are most often published for glibc; keeping Alpine would refuse many common packages. Not touching framework-only versions keeps their build keys and caches |
| D046-016 | Private packages are uploaded to the platform; proxying a tenant's external registry with its credentials is not in 0.3.0. delegate (OD-2026-11-24-4) | Giving the gateway a tenant credential is a third secret-disclosure path that Principle V does not list; uploads give the same result for a tenant's CI with no credential leaving the tenant. Recorded as Gap 046-002 with the amendment it needs |
| D046-017 | The feature is switched on by the capability `script-dependencies` in the instance registry. delegate (OD-2026-11-24-4) | Every replica must agree before a bundle 5 exists, or an old runner would be asked to run what it cannot (ADR-0082) |
| D046-018 | Bundle version 5; version 4 stays feature 023's (PHP and `python-data`). delegate (OD-2026-11-24-4) | Bundles 1 to 3 exist in `crates/infra-schema/src/bundle.rs`; 023 reserves 4; a separate version keeps the two features independent |
| D046-019 | The policy lives beside feature 023's language policy and is changed through the same endpoint. delegate (OD-2026-11-24-4) | One place where a namespace administrator decides what code may run; one lease-time check |
| D046-020 | The SBOM and the provenance use the module feature 023 defines for ADR-0057 decision 6; whichever feature is built first builds it. delegate (OD-2026-11-24-4) | One attestation format for every language; no fork |

## Gaps

| ID | Gap | State |
|---|---|---|
| G046-001 | Packages that need an install script, and Python packages with only a source distribution, are not served. A reviewed exception (a build of the source distribution in the sandbox with its own resolved backend) is possible later | Open (later release) |
| G046-002 | A tenant's external private registry cannot be proxied with its credentials: it needs a disclosure path that Principle V does not list. Recommended to the coordinating session as a candidate for a later amendment | Open |
| G046-003 | The quarantine default (72 hours) is a judgement; revisit with data on detection delays | Open (measure) |
| G046-004 | The runner layer cache bound (50 GiB) and the layer ceiling (1 GiB default, 4 GiB maximum) are estimates; set after the benchmark | Open (measure) |
| G046-005 | The resolver command lines (pinned `uv` writing PEP 751 for two targets with an `exclude-newer` cut; pinned `npm` lock-only with `before`) are proved by the probe of Task T046-12 before any other runner task; if `uv` cannot write PEP 751 for the script form, the fallback is its own lock exported to a PEP 751 file by the resolver shim | Open (probe) |
| G046-006 | Layers are reproducible, so a second runner could rebuild and compare (as Gap 118 of feature 005 proposes for builds); not built in 0.3.0 | Open |

## Owner questions

None open. ADR-0120 and ADR-0121 are accepted or rejected under Owner decision 2026-11-24-4 when the design completes
its review.
