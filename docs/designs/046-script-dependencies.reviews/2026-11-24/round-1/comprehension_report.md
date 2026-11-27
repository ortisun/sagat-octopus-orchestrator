Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cfb22df54df5b084f748531dbdb9b3546c01f60d
Reviewer: egm-comprehension (egm-review stage 6)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 7, Nit 5

**What this proposes**

From release 0.3.0, TypeScript and Python scripts can use npm and PyPI packages, plus private packages a tenant uploads. The author writes a manifest: a PEP 723 block for Python, or a `manifest` field holding a restricted `package.json` for TypeScript. At publication, a sandboxed resolve container turns the manifest into a hashed lock. Its only network is a new package gateway that filters what it serves by the namespace's policy. The control plane then validates the lock and checks it against the supply-chain policy and an imported OSV advisory snapshot. An install container with no network unpacks the verified archives into a canonical layer for each target, and the control plane signs a bundle 5 that binds the layer digests, the SBOM, the policy report and the provenance. Runs mount the layer read-only from a cache volume kept per tenant. A malicious-package advisory found later blocks the version when its next step is leased.

**How the existing system works, per this document**

Feature 005 refuses all TypeScript and Python packages (Decision 105). A script can use only the standard library and an SDK shim, so authors who need packages write SDK workers instead. Run containers have no network, a read-only root and a noexec scratch area. Artifacts are at most 2 MiB and live in PostgreSQL, and bundles are signed with Ed25519 and keyed by a build key. C#, Rust and PHP use closed package catalogs (ADR-0021, ADR-0057).

**Findings**

1. **Major.** Technical Plan 1 against the Risks table and Constitution Check IV. Technical Plan 1 says the build key gains three parts (manifest digest, lock digest, unpacker identity); the other two places say the resolver images and runtimes are also part of the key. It is unclear how the key can hold an "empty" lock digest that later becomes the resolved digest while a cached key "is finished by the claim", and whether a second publication of the same manifest an hour later re-resolves or reuses the old lock. Fix: list every key part once; say how the empty-then-filled lock digest interacts with the cache and with the time-dependent quarantine.
2. **Major.** Requirement 046-005, Technical Plans 8 and 11. Never says how a version's set of supported targets is chosen, what `PACKAGE_PLATFORM_UNSUPPORTED` triggers on, what a runner does when its native target has no layer, or how one runner builds a Python layer for the other architecture when the install container is the runtime image. Fix: define the target set, the refusal rule, runner matching and cross-target installs.
3. **Major.** `vulnerabilities.blockNewAtLease`, CUJ 3. Its behaviour is never described; Technical Plans 7 and 12 set `runnable = false` only for malicious packages; a policy change "takes effect for lease checks" but only snapshot imports trigger a re-scan. Fix: describe what the field blocks and which pass recomputes `runnable` after a policy change.
4. **Major.** Technical Plan 7. The control plane downloads OSV archives "through the gateway's upstream connection", but Technical Plan 3 lists no such route and OSV is not a configured upstream. Fix: add the route or describe the actual egress path.
5. **Major.** Technical Plan 10 against Technical Plan 3. How a namespace "claims" an npm scope is undefined; uploads are stored per tenant but the private-name rule is per namespace. Fix: define scope claims and whether uploads are shared within a tenant.
6. **Major.** Feature range. Project terms used but not defined (build key, lease, script activity claim, SDK shim, SDK worker, framework-only version, "feature 005 profile", hardening anchor, result frame, engine, Studio, "version 1 text bundle"); many references cite a number without a link. Fix: add the terms and link each reference.
7. **Major.** Detailed Implementation Plan. The builders "never touch the same crate", but slice 2 holds Task T046-14 (routes, a table, quota in `api-http` and `infra-postgres`) and Task T046-9 (upload endpoint, lock-validator rule, CLI); `sagat package push` is in both T046-9 and T046-19. Fix: split T046-9 and T046-14 by crate and give the CLI to one task.
8. **Nit.** Requirement 046-012: "the only mount" conflicts with the scratch area and the Node link. Write "the only added bind mount".
9. **Nit.** Tasks T046-21 and T046-23 cite Requirements 046-018 and 046-001; the intended targets are Non-functional requirements 046-005 and 046-003.
10. **Nit.** Acceptance criteria cite tests the Verification Plan does not name (snapshot, dependency confusion, capability, cross-runner digest).
11. **Nit.** "tenant digest" undefined; the Alpine base of framework-only images first appears in Complexity Tracking.
12. **Nit.** Versions 2 to 4 of the bundle are not explained; "retryable only on `503`" is not related to `PACKAGE_GATEWAY_UNAVAILABLE`.
