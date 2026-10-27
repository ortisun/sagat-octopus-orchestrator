# ADR-0057: Additional script languages reuse the container-per-run sandbox, the signed artifact families and closed offline catalogs

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26)
**Refines / supersedes:** refines ADR-0006, ADR-0013, ADR-0021 and ADR-0024; supersedes none of them
**Enforced by:** `crates/infra-schema` bundle and catalog unit tests over the fixtures of T023-003; the
real-container suites `integration_script_languages_containers` (probes, hostile battery per language)
in the `native-runners` CI job on AMD64 and ARM64; `contract_rest_script_languages`; the provenance
contract tests (T023-047) and per-build provenance tests (T023-009); the lease-time policy tests
(T023-061); the `python-data` binding fixtures (T023-062); the reserved host-call gate, budget and secret-net
tests (T023-064, T023-065); `scripts/check-architecture.py` for crate placement. The rule "no product
configuration selects Deno or Bun" is checked by T023-049.

## Context

Feature 023 adds Bash, PowerShell, PHP and Rust scripts and evaluates Deno and Bun for TypeScript
(`specs/023-script-languages/spec.md`). Feature 005 already runs five languages in one model: a draft is
published as an immutable version, a separate runner builds it in a container with no network into a
canonical bundle, the control plane signs `tenant ‖ digest ‖ language` with a key it alone holds, and the
runner verifies and runs the bundle in a fresh container under a fixed profile (ADR-0013). C# added a
binary bundle and a closed package catalog (ADR-0021); Go and Java added target-aware v3 bundles and a
sealed-memory loader that keeps `/tmp` `noexec` (ADR-0024).

The forces:

1. Each new language comes with a language-level "sandbox" that its own documentation says is not one:
   PHP's `disable_functions` "can be circumvented" and `open_basedir` "can not be relied upon"; PowerShell's
   ConstrainedLanguage mode needs Windows application control to resist bypass; Deno's permissions do not
   cover subprocesses or FFI; the Bash manual calls its restricted mode "only one component" and points to
   jails, zones or containers. Constitution V: no language is a
   sandbox.
2. Each new ecosystem expects live package installation (crates.io, Packagist, the PowerShell Gallery,
   npm), and two of them run package code at install or build time (Cargo build scripts and proc-macros,
   Composer plugins and scripts).
3. A Composer vendor tree does not fit bundle v1 (32 flat files); a Rust executable fits v3 only if it is
   static and non-PIE, because the validator accepts only `ET_EXEC` without an interpreter.
4. Provenance today is a toolchain string, a key id and a build key on the version; nothing signed states
   how an artifact was built.
5. Worker claims name a language, not a native target (005, G1201).

## Decision

1. **One sandbox for every language.** Builds and runs of `bash`, `powershell`, `php` and `rust` MUST use
   the ADR-0013 container profile unchanged, started by the runner, never by the control plane, engine or
   scheduler. A language that needs network, an executable scratch mount, a capability, a seccomp change
   or a writable root MUST fail its probe instead of receiving it. Language-level restrictions
   (`disable_functions`, `open_basedir`, ConstrainedLanguage, restricted shells, runtime permissions) MUST
   NOT be described or relied upon as isolation.
2. **Existing artifact families where they fit; one new family where they do not.** Bash and PowerShell
   MUST use bundle v1 text. Rust MUST use bundle v3 (`linux-amd64|linux-arm64`, ABI `sagat-inline-v1`,
   entry `script.elf`, `base64+gzip`) with the unchanged ELF rules, built with `-C relocation-model=static`
   and `-C target-feature=+crt-static`, and MUST execute through the shared four-seal memory-file loader
   with input on sealed fd 3 (FR-005-010). PHP MUST use the new bundle v4: a canonical tree of at most
   1,024 UTF-8 text files, relative paths of at most 10 segments and 255 bytes, 1,400 KiB decoded, 2 MiB
   stored, entry `main.php`, `lockDigest` binding the lockfile, `runtimeCatalog` null. The reserved
   `python-data` runtime (decision 4) uses the same v4 shape with entry `main.py` and a non-null
   `runtimeCatalog`. Bundles v1, v2 and v3 and their stored artifacts MUST NOT change.
3. **The builder owns the project.** Authors supply source and an optional lockfile only. The Rust
   builder MUST invoke `rustc` directly with fixed arguments and `#![forbid(unsafe_code)]` on the author
   crate, never Cargo. The PHP builder MUST NOT run `composer install`, `update`, `require`, a script or a
   plugin; it copies catalog trees and runs only `composer dump-autoload --classmap-authoritative --no-dev
   --no-scripts --no-plugins`. PowerShell and Bash MUST refuse non-empty lockfiles.
4. **Dependencies come only from closed offline catalogs baked into images.** A Rust or PHP package MUST
   be a reviewed catalog entry with upstream checksum or reference, an independent archive SHA-256, licence,
   build-time-code flags and image-build advisory audit evidence. Cargo build scripts run only when the
   image is built. Reviewed proc-macro code is loaded by `rustc` from the read-only image and runs inside
   the sandboxed builder container on every customer build that uses it; the builder profile, not the
   review flag, contains it. No customer build or run MAY reach a registry. The catalog digest MUST be part
   of the toolchain identity and so of the build key. The control plane MUST sign a build only if its SBOM
   package set equals its lockfile package set. Dependencies travel inside the signed bundle (PHP) or the
   static binary (Rust), and their runtime images MUST NOT carry the catalog. **One exception:** an
   image-catalog runtime, of which `python-data` is the only one, may keep its catalog in its runtime image
   because its packages (native wheels such as Polars and PyArrow) exceed the 2 MiB artifact ceiling. Its
   artifacts MUST bind the catalog digest in `runtimeCatalog`, its runner MUST refuse to run an artifact
   whose `runtimeCatalog` differs from its configured runtime's catalog digest, and only the locked
   packages are placed on the import path. This decision defines that mechanism; feature 024 defines the
   image, the packages and their evidence, and `python-data` stays unbuilt until it does.
5. **A language is served only after its production-path probe and only if the deployment asks for it.**
   The runner MUST NOT advertise a new language that is absent from its allowlist (default: the five
   005 languages) or whose bundled probe did not build and run through the production images, profile and
   protocol. Outside development, a tag-only image reference MUST fail the probe. A runner has one native
   target; Rust and Go targets MUST be equal; each target is enabled only by its own native proof.
6. **Every new build is attested.** The control plane MUST attach to every artifact it signs after this
   decision, in any language, a DSSE envelope (`application/vnd.in-toto+json`) over an in-toto Statement v1
   with an SLSA Provenance v1 predicate whose subject is the artifact digest, signed with the existing key,
   separating fields it verified from fields the runner reported, stored once per build (tenant and build
   key) and not on the content-addressed artifact row. It MUST NOT claim an SLSA build level.
7. **Deno and Bun are not adopted.** No product configuration, contract or runner setting MAY select them.
   Adopting either later requires a new ADR, conformance parity and binding the runtime into the toolchain
   identity so a published version never changes engine silently.
8. **A namespace's language policy holds at run time.** Besides draft creation, publication, test runs and
   workflow publication, the engine MUST check the policy when it leases a script step: a step whose
   artifact language is disabled MUST NOT be leased; it is failed in the lease transaction as
   non-retryable `SCRIPT_DENIED` with its journal event, next work, outbox rows and a `script.policy_denied`
   audit record. An attempt already leased finishes.
9. **The shim protocol has two reserved, capability-gated host calls** (coordinator decision C17), for
   every script runtime including the five of feature 005 and `python-data`: `checkpoint` (heartbeat
   details, meaning owned by feature 019) and `stream.append` (Workflow Streams, meaning owned by feature
   026 and ADR-0068). The supervisor MUST refuse them unless the current lease grants the capability
   (never in a test run), MUST bound sizes and per-run budgets, MUST assign sequences itself, MUST refuse
   a payload carrying a secret handed to the run, and MUST NOT log, record or audit either payload.

## Considered options

1. **Reuse the 005 model with per-language images, existing families plus v4, closed catalogs and signed
   provenance (chosen).** One isolation story, one channel, one lease and signing path; new code is
   confined to images, shims, validators and catalogs.
2. **Rely on language-level restrictions inside a shared interpreter process** (PHP-FPM pool with
   `disable_functions`, a long-lived `pwsh` runspace in ConstrainedLanguage, Deno permissions). Rejected:
   each vendor documents its mechanism as bypassable or partial; a shared process lets one script leave
   state for the next (ADR-0013 rejected long-lived containers for the same reason).
3. **Live package installation through a registry proxy in the builder** (`cargo build` with a mirror,
   `composer install` with a proxy). Rejected: it gives the builder a network path, runs build scripts and
   plugins chosen by authors per build, and turns the platform into an unreviewed package manager; ADR-0021
   rejected the same for NuGet.
4. **WASM for Rust and PHP** (wasmtime, php-wasm). Rejected for this feature: a second sandbox model to
   prove and operate, a different host-call ABI, and the constitution excludes WASM as a mandatory runtime;
   the container remains the boundary either way.
5. **PHP packages in the runtime image instead of the signed bundle.** Rejected for PHP: a version would
   run against whatever the current image holds, the SBOM would not describe what ran, and undeclared
   packages would be loadable. Accepted only for `python-data`, where the packages cannot fit an artifact,
   and only with the exact `runtimeCatalog` binding and locked import path that remove both objections.
6. **Accept static-PIE Rust (`ET_DYN`).** Deferred: it needs a dynamic-section parser in the validator;
   non-PIE gives up main-image ASLR inside a container with no network or secrets. A later additive
   validator change can admit it.

## Consequences

- Operators maintain four more image lines (one or two images each), two catalogs and their review and
  audit cadence. The PowerShell and Rust builder images are large (G1208); runner disks need room.
- Rust builds need a larger, bounded build profile and a per-runner native build limit; values are
  proposals until the benchmark (G1206).
- Mixed-architecture Rust pools remain forbidden until claims are target-aware (feature 019).
- Provenance exists only for builds made after this decision; earlier artifacts stay valid and
  unattested.
- A `python-data` artifact runs only on a runtime image with its exact catalog digest; re-pinning that
  catalog makes older versions unrunnable until republished or until a runner keeps the older image.
- Disabling a language now stops existing runs at their next script step attempt; the lease path pays
  one indexed read per script activity claim.
- Compatibility: additive enums, an additive bundle version, an additive response field and an additive
  migration whose downgrade refuses while new-language rows or provenance exist, so rollback is
  binary-only and forward-only for data: a 0.2.0 control plane cannot decode a new-language row and fails
  a script listing that contains one. Runners without the new configuration behave exactly as before.
- Rollback: disable the languages (runner allowlists, namespace policy) and roll forward; roll binaries
  back to 0.2.0 only while no new-language row exists. Stored artifacts remain verifiable.

## Evidence

None yet: this is a proposal. Required before acceptance of the implementation: the hostile-script
battery per language and target, native AMD64 and ARM64 Rust probes in CI, malformed v4/ELF/lock
fixtures refused, v1/v2/v3 regressions unchanged, provenance verification and tamper tests, and the
benchmark report, the lease-time refusal tests, the `python-data` binding fixtures and the 0.2.0
rollback observation (`specs/023-script-languages/tasks.md`, T023-029..033, T023-025, T023-008, T023-047,
T023-056, T023-061, T023-062, T023-063). Sources: `specs/023-script-languages/research.md` §2 (Composer, PHP, PowerShell, Deno, Bun,
rustc, Cargo, in-toto, SLSA and DSSE documentation).
