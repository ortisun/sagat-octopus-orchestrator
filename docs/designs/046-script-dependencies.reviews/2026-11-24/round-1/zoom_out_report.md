Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cfb22df54df5b084f748531dbdb9b3546c01f60d
Reviewer: egm-zoom-out (egm-review stage 5)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 5, Nit 4

Context check passed: the goal (npm and PyPI packages for scripts, matching Windmill, keeping feature 005's sandbox) is clear.

1. **Major. Scope.** One design carries several features: the core (manifest, gateway, resolve, unpack, layer, bundle 5, SBOM) plus an advisory snapshot with re-scan and lease block, a full policy language, private uploads, offline bundles and a proposal endpoint, behind one acceptance. Fix: phases with their own acceptance, each still in 0.3.0.
2. **Major. `vulnerabilities.blockNewAtLease`** is relied on by ADR-0121 decision 4 but no flow defines it; `blockUnrated`, licence expressions, 500-rule lists and 200 waivers are knobs no journey uses. Fix: specify or drop; cut bounds to need.
3. **Major. Four "from 023, or built here" fallbacks** (policy storage, provenance module, lease check, `runnable` column). None exists in `crates/` today, so each has no owner and two possible build orders. Fix: pick one owner now.
4. **Major. The layer's data path is heavy and its trade-off not argued:** the install job uploads up to 4 GiB through the control plane to the blob store, then every runner downloads it through the control plane and runs a second container to populate a volume. Fix: compare with each runner unpacking locally from gateway files and checking the signed digest; if the upload stays, stream directly to and from the blob store.
5. **Major. The existing lockfile gate** (`crates/infra-sandbox/src/lockfile.rs`, `accept`) is not mentioned; two validators would result. Fix: name it the single entry point that delegates to `infra-packages` when the capability is present.
6. **Nit.** Gateway filtering and control-plane evaluation duplicate rules; say the gateway reuses the domain functions or narrow its filter.
7. **Nit.** A bespoke Ed25519 job token where the workspace already uses `jsonwebtoken`; use an EdDSA JWT with its own key.
8. **Nit.** Low-level details (forbidden specifiers, file modes, wheel `.data`, volume names, severity mapping, metric names) belong in contracts.
9. **Nit.** The proposal endpoint is speculative and needs a maintained mapping; defer it or make it a Studio-only hint.
