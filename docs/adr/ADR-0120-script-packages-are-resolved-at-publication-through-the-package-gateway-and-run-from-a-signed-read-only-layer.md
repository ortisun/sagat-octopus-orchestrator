# ADR-0120: TypeScript and Python packages are resolved at publication through the platform's package gateway and run from a signed, read-only, content-addressed layer

**Status:** Proposed
**Decided by:** agent proposal (design 046, 2026-11-24, revised after review round 1), to be accepted or rejected under Owner decision 2026-11-24-4 when design 046 completes its review
**Refines / supersedes:** supersedes ADR-0013 **in part**: a run container of a bundle version 5 has one added mount, a read-only dependency layer that is not `noexec` (every other rule of ADR-0013 stays, including "no network" for every sandbox container). Narrows ADR-0057 decision 4 ("no customer build or run may reach a registry"; "dependencies come only from closed offline catalogs") to C#, Rust and PHP: for TypeScript and Python, a resolve container reaches the package gateway's package routes, and nothing else, through its runner. Refines ADR-0021 and ADR-0023 (a separate package store, not the tenant payload store). Supersedes Decision 105 of feature 005 for TypeScript and Python only.
**Enforced by:** planned tests of design 046 (`docs/designs/046-script-dependencies.md`, Verification Plan): the lock validator fixtures in `crates/infra-packages`; the hostile battery `integration_script_dependencies_containers` (a resolve container reaches only the gateway's package routes, install and run containers reach nothing, the layer is read-only, refused archives, two tenants get two volumes, an unmarked volume is never mounted); `layer_digest_is_identical_on_two_hosts`; the tamper tests; and three rules in `scripts/check-architecture.py` (no network mode other than `none` in `crates/infra-sandbox`; exactly one added read-only mount on a run container; only the gateway and `crates/infra-egress` name the package store's settings). Until they exist: review only.

## Context

Windmill resolves a script's imports when it is deployed, stores a lockfile and installs packages on the worker from a
public or private registry, with network access on the worker. Feature 005 refuses every package for TypeScript and
Python (Decision 105, Gap 97); C# accepts one closed catalog baked into its image (ADR-0021), and feature 023 does the same
for Rust and PHP (ADR-0057). A closed catalog cannot serve the npm and PyPI ecosystems, which are what authors expect.

The forces:

1. Installing a package runs its code at install time in both ecosystems unless told not to: npm lifecycle scripts, and a
   Python source distribution's build. ADR-0057 rejected "live package installation through a registry proxy in the
   builder" for this reason and because it gives the builder a network path.
2. The sandbox (ADR-0013) has no network, a read-only root and nothing mounted. A dependency tree is far larger than the
   2 MiB bundle ceiling (feature 005, Gap 108), and a run must not download it.
3. Resolution needs registry metadata, so something must reach a registry.
4. Package registries are attacked: typosquatting, dependency confusion, account takeover publishing a malicious
   version, and vulnerabilities disclosed after a version was chosen.

## Decision

1. **Resolution happens at publication, never at run time, and is never served from a cache.** A published version
   carries a lock in which every package has an exact version and a content hash. A run uses exactly that lock. A new
   resolution is a new version.
2. **One platform component reaches registries: the package gateway** (`apps/package-gateway`, declared in this
   repository's compose). It is a read-only proxy of the npm registry protocol and of the Python simple repository API
   (PEP 691), keeps the files it fetches by SHA-256 in a package store of its own (not the tenant payload store of
   ADR-0023), filters what it serves by the namespace's supply-chain policy, and can run with no upstream at all. No other
   component MUST reach a package registry.
3. **A resolve container has no network.** It uses the ADR-0013 profile unchanged. Its resolver's HTTP requests go to a
   server on its loopback interface and travel over the supervisor channel to the runner, which forwards only the
   gateway's package routes for the job's ecosystem, adding the job token. No token or secret enters the container. Its
   only output is the lock.
4. **Installation runs no installer and no package code.** The runner fetches each locked file from the package store
   by hash, checks it, and streams the files into an install container (ADR-0013 profile, no network) that runs the
   platform's own unpacker into a fresh volume. The unpacker refuses links, special files, privileged bits and paths
   written twice, and reports the digest of a canonical serialization of the tree. Python bytecode is compiled in a second
   container with `python -I`, so no file of the layer is imported.
5. **The layer is named by the signed bundle.** Bundle version 5 carries the source files, the lock digest and, per
   required target, the layer digest. The control plane signs it as every bundle. The platform does not store layers:
   every runner materializes its own from the package store and MUST refuse a layer whose digest differs from the signed
   one.
6. **A run mounts the layer read-only.** The volume is per tenant and per runner, carries a completion marker written
   last, and is mounted read-only, not `noexec` (native extensions are mapped executable), as the only added mount of
   the run container.
7. **Supply-chain policy is evaluated by the control plane** on every publication, with the gateway's metadata and the
   platform's advisory snapshot (ADR-0121), before a build is signed.

## Considered options

1. **Gateway, channel-forwarded resolution, unpack-only install, signed digest materialized per runner (chosen).**
   Package code never runs before the run; every sandbox container keeps no network; the gateway is the only egress and
   applies policy; layers are content-addressed and bound by the existing signature.
2. **A closed curated catalog in the image** (ADR-0021, ADR-0057). Strongest, but a catalog of a few packages does not
   serve npm and PyPI. Kept for C#, Rust and PHP.
3. **Install at run time on the worker** (Windmill). A network path in every run, install scripts, a registry outage
   breaks runs. Rejected.
4. **A third-party repository manager** (Nexus, Artifactory, Verdaccio, devpi) as the proxy. A large component with its own
   policy model and image to patch, and no notion of namespaces. Rejected; an operator may chain one as the gateway's
   upstream.
5. **A layer as an image, or an image volume.** Clean on Kubernetes, but needs an image builder or a registry on the
   runner host. Deferred behind the volume port.
6. **A container network from the resolve container to the gateway** (the first draft). A shared bridge between tenants'
   containers, host-local, a token in the container. Rejected in review.
7. **Layers stored in the platform's blob store and downloaded by runners.** Gigabytes through the control plane, a second
   copy of the package store, a retention policy of its own. Rejected in review.

## Consequences

- One more service (the gateway) and one more store (the package store). Runners need disk for files and layers (bounded,
  least recently used).
- The sandbox gains one read-only mount for these versions; residue analysis of ADR-0052 is unchanged because the mount
  is read-only.
- A version whose package disappears upstream still builds and runs: its files are in the package store.
- Packages that need an install script, or that publish only a source distribution, are not served (Gap 046-001), with a
  typed reason at publication.
- Compatibility: additive bundle version, tables and endpoints; framework-only versions and bundles 1 to 4 do not change.
  Rolling back the binaries after bundle-5 versions exist leaves versions an old runner refuses, so rollback is
  forward-only for data, as in ADR-0057.

## Evidence

None yet: this is a proposal. Required before acceptance: the tests named under "Enforced by", the probe of design 046's
Task 046-12, and the benchmark of design 046.
