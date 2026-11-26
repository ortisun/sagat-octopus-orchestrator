# ADR-0120: TypeScript and Python packages are resolved at publication through the platform's package gateway and run from a signed, read-only, content-addressed layer

**Status:** Proposed
**Decided by:** agent proposal (design 046, 2026-11-24, revised after review rounds 1 to 3 and the re-reads of 2026-11-26), to be accepted or rejected under Owner decision 2026-11-24-4 when design 046 completes its review
**Refines / supersedes:** supersedes ADR-0013 **in part**: a run container of a bundle version 5 has one added mount, a read-only dependency layer that is not `noexec` (every other rule of ADR-0013 stays, including "no network" for every sandbox container). Narrows ADR-0057 decision 4 to C#, Rust and PHP: for TypeScript and Python a resolve container's requests reach, through its runner, only the package gateway's public routes and the control plane's private document, metadata and file routes for the tenant's reserved names. Refines ADR-0023: private packages use its tenant payload store; the gateway's public files use a store of their own. Supersedes Decision 105 of feature 005 for TypeScript and Python only.
**Enforced by:** planned tests of design 046 (`docs/designs/046-script-dependencies.md`, Verification Plan): the lock validator fixtures in `crates/infra-packages` (including `a_relabelled_file_is_refused`); the hostile battery `integration_script_dependencies_containers` (`a_resolve_container_reaches_only_the_forwarded_routes`, `a_reserved_name_never_reaches_the_public_registry`, `a_script_cannot_write_the_layer`, `two_tenants_with_one_lock_get_two_volumes`, `an_unmarked_volume_is_never_mounted`, `unpacker_refuses_*`); `the_digest_is_recomputed_by_a_fresh_container`; `layer_digest_is_identical_on_two_hosts`; the tamper tests; the Helm test `only_the_gateway_leaves_the_cluster`; and four rules of `scripts/check-architecture.py` with negative tests: no network mode other than `none` in `crates/infra-sandbox` (`docker.rs`, `lake.rs`); exactly one added read-only mount on a run container; no token or secret in a resolve or install container; only `apps/package-gateway` names the public store's settings, and its dependency allow-list keeps it away from the database, application and sandbox crates. Until they exist: review only.

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
   carries a lock in which every package has an exact version and a content hash; a run uses exactly that lock; a new
   resolution is a new version.
2. **One platform component reaches public registries: the package gateway** (`apps/package-gateway`, in this
   repository's compose). It proxies the npm registry protocol and the Python simple repository API (PEP 691) read-only,
   keeps public files by SHA-256 in a store of its own (not the tenant payload store of ADR-0023), holds no tenant data,
   applies only fixed rules (the job's quarantine cut, wheels only), and can run with no upstream. It never calls the
   control plane. Other components MUST NOT reach a public registry.
3. **A resolve container has no network.** It uses the ADR-0013 profile unchanged. Its resolver's requests become
   `fetch` ops on the supervisor channel; the runner forwards only the gateway's public package routes, or the control
   plane's private document, metadata and file routes for a tenant's reserved names, adding a job grant. No token or secret enters the
   container.
4. **Private packages stay with the tenant.** They are stored in the tenant's payload store and served to runners by the
   control plane, for that tenant only.
5. **Installation runs no installer and no package code.** The runner checks each file against the lock's own hash and
   streams the files into an unpack container (ADR-0013 profile, no network). The layer's digest is computed by a fresh
   container on a read-only mount over the whole tree, with no bytecode compiled into the layer; modes are
   fixed and the completion marker written last.
6. **The signature binds the lock and each layer.** Bundle version 5 names the lock, the unpacker and the runtime; a
   layer record per (artifact, target), signed with the same key, names the layer digest. Every runner materializes its
   own layer before it offers to run the version, and MUST refuse one whose digest differs from its record.
7. **A run mounts the layer read-only**, not `noexec`, as the only added mount, from a volume of its tenant whose marker
   names the recorded digest.
8. **Supply-chain policy is evaluated by the control plane** on every version, with the gateway's metadata and the
   advisory snapshot (ADR-0121), before anything is signed.
## Considered options

1. **A gateway of public files with fixed rules, channel-forwarded resolution, unpack-only install, a signed digest each
   runner materializes and checks (chosen).**
   Package code never runs before the run; every sandbox container keeps no network; the gateway is the only egress and
   holds no tenant data, while the control plane applies the policy; layers are content-addressed and bound by the existing signature.
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
8. **Policy filtering and private files in the gateway.** The internet-facing process would need tenants' rules and
   would hold their private packages. Rejected in review.

## Consequences

- One more service (the gateway) and one more store (its public store). Runners need disk for files and layers (bounded,
  least recently used), on a filesystem of its own for the volumes.
- Runners keep every image digest that a ready version names; a target added later gets layer records by a backfill pass.
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
