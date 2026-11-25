# ADR-0120: TypeScript and Python packages are resolved at publication through the platform's package gateway and run from a signed, read-only, content-addressed layer

**Status:** Proposed
**Decided by:** agent proposal (design 046, 2026-11-24), to be accepted or rejected under Owner decision 2026-11-24-4 when design 046 completes its review
**Refines / supersedes:** refines ADR-0013 (one exception to "nothing mounted": a read-only dependency layer), ADR-0021 and ADR-0057 (the closed-catalog rule stays for C#, Rust and PHP; TypeScript and Python get a policy-governed open catalog); supersedes Decision 105 of feature 005 for TypeScript and Python only
**Enforced by:** planned tests of design 046 (`docs/designs/046-script-dependencies.md`, Verification Plan): the lock validator fixtures in `crates/infra-packages`, the hostile battery `integration_script_dependencies_containers` (no network but the gateway during a resolve, no network during install and run, read-only layer, refused archives), the layer-digest tests, the signature test that a layer not named by the signed bundle never mounts, and `scripts/check-architecture.py` for crate placement. Until they exist: review only.

## Context

Windmill resolves a script's imports when it is deployed, stores a lockfile and installs packages on the worker from a
public or private registry, with network access on the worker. Feature 005 refuses every package for TypeScript and
Python (Decision 105, Gap 97); C# accepts one closed catalog baked into its image (ADR-0021), and feature 023 does the same
for Rust and PHP (ADR-0057). A closed catalog cannot serve the npm and PyPI ecosystems, which are what authors expect.

The forces:

1. Installing a package runs its code at install time in both ecosystems unless told not to: npm lifecycle scripts,
   and a Python source distribution's build backend. ADR-0057 rejected "live package installation through a registry
   proxy in the builder" for this reason and because it gives the builder a network path.
2. The sandbox (ADR-0013) has no network, a read-only root and nothing mounted. A dependency tree is far larger than the
   2 MiB artifact ceiling (feature 005, Gap 108), and a run must not download it.
3. Resolution needs registry metadata, so something must have network access to a registry.
4. Package registries are attacked: typosquatting, dependency confusion, account takeover publishing a malicious
   version, and known vulnerabilities disclosed after a version was chosen.

## Decision

1. **Resolution happens at publication, never at run time.** A published version carries a lock in which every
   package has an exact version and a content hash. A run uses exactly that lock. A new resolution is a new version.
2. **One platform component reaches registries: the package gateway** (`apps/package-gateway`, declared in this
   repository's compose). It is a read-only proxy of the npm registry protocol and of the Python simple repository API
   (JSON form), stores what it fetches by SHA-256 in the platform's blob store, filters what it serves by the
   namespace's supply-chain policy, and can run with no upstream at all (offline). No other component MUST reach a
   package registry.
3. **A resolve job runs in a sandbox container whose only network is the gateway.** It uses the 005 profile with one
   change: it is attached to an internal network on which only the gateway listens. It runs the pinned resolver with
   install scripts and source builds disabled. Its only output is the lock. It receives no secret.
4. **Installation runs no installer and no package code.** The runner fetches each locked file from the gateway,
   checks its hash against the lock, and streams the files into an install container (005 profile, no network) that
   runs the platform's own unpacker. The unpacker writes a canonical archive (sorted, fixed times and modes, no links,
   no special files) whose SHA-256 is the layer's digest. One layer per native target.
5. **The layer is named by the signed bundle.** A new additive bundle version (5) carries the source files and, per
   target, the digest and size of the layer, the digests of the lock, the manifest, the SBOM and the policy. The
   control plane signs the bundle as today; the layer is bound by that signature. A runner MUST refuse a layer whose
   digest differs from the signed one.
6. **A run mounts the layer read-only.** The runner keeps one engine volume per (tenant, layer digest), populated once
   by a container of the 005 profile, and mounts it read-only, `nosuid` and `nodev`, into the run container. This is
   the only mount a run container has. A volume is never shared between tenants.
7. **Supply-chain policy is evaluated by the control plane** on the lock, with the gateway's metadata and the
   platform's advisory snapshot (ADR-0121), before a build is signed.

## Considered options

1. **Gateway, sandboxed resolve, unpack-only install, signed read-only layer (chosen).** Package code never runs
   before the run; the only network path is a platform component that applies policy; the run stays without network;
   the layer is content-addressed and bound by the existing signature.
2. **Closed curated catalog in the image, as ADR-0021 and ADR-0057 do.** Strongest, but a catalog of a few packages
   does not serve npm and PyPI, and every addition is an image release. Kept for C#, Rust and PHP.
3. **Install at run time on the worker (Windmill).** Gives every run container a network path, runs install scripts,
   and makes a run depend on a registry being up. Rejected.
4. **A third-party repository manager (Nexus, Artifactory, Verdaccio, devpi) as the proxy.** Adds a large component
   whose policy model is not the namespace's, a licence question for some, and an image to pin and patch. Rejected for a
   small proxy that the platform builds, tests and controls; an operator may still point the gateway's upstream at such
   a manager.
5. **Package the layer as an image (one image per lock) or an image volume.** Clean on Kubernetes (image volumes) but
   needs an image builder or a registry on the runner host. Deferred: the volume mount is behind a port so an image
   volume can replace it.
6. **Stream the dependencies into the scratch area at every run.** No mount, but a cost proportional to the tree at
   every run and a writable copy. Rejected.

## Consequences

- One more service to operate (the gateway) and one more network (internal, gateway only). The runner needs disk for
  layers (an LRU bound, configurable).
- The sandbox gains one read-only mount; residue analysis of ADR-0052 is unchanged because the mount is read-only.
- A version whose package becomes unavailable upstream still builds and runs: its files are in the gateway's store.
- Packages that need an install script to work, or that publish only a source distribution, are not served
  (Gap 046-001); this is stated at publication with a typed reason.
- Compatibility: additive bundle version, additive tables and endpoints; framework-only TypeScript and Python versions,
  and bundles 1 to 4, do not change. Rolling back the binaries leaves version-5 rows that an old runner refuses to run
  (it does not know bundle 5), so rollback is forward-only for data, as in ADR-0057.

## Evidence

None yet: this is a proposal. Required before acceptance: the tests named under "Enforced by", the probe that the pinned
resolvers produce the lock formats of design 046 against the gateway with no other network, and the benchmark of design
046 (resolve time, layer population time, first-run and warm-run start with a layer).
