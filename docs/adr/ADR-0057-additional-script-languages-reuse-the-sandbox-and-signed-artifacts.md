# ADR-0057: Additional script languages reuse the container-per-run sandbox, the signed artifact families and closed offline catalogs

**Status:** Accepted (2026-11-24, delegate (OD-2026-11-24-4)), with the amendment below; proposed 2026-10-26
**Decided by:** agent proposal of 2026-10-26; accepted on 2026-11-24 by the agent preparing feature 023 for
build, under the owner's delegation for release 0.3.0 (OD-2026-11-24-4, which lets the agents accept or reject
architecture decisions, each recorded with its reasoning and reported to the owner, who may overrule it).
Options weighed: (a) accept as written; (b) reject, for a registry proxy in the builder or WASM runtimes
(options 3 and 4 below); (c) accept with an amendment that brings the text up to `develop` of 2026-11-24.
Chosen (c). The core is still the strongest choice: one sandbox and one signing path for every language,
no network in any customer build or run, dependencies bound into what is signed. Design 046 (Architecture
decision 0120) reached the same conclusion for TypeScript and Python and builds on this decision rather
than beside it, which would be impossible if it were rejected. What had gone stale since 2026-10-26 (the
rolling N and N-1 upgrade of release 0.3.0, the instance registry, the claim-path budget of design 038,
the runner's lake mode, design 046) changes how decisions 2, 4, 5, 6 and 8 are carried out, not what they
decide, so an amendment is enough and no new decision record is needed
**Refines / supersedes:** refines ADR-0006, ADR-0013, ADR-0021 and ADR-0024; supersedes none of them.
Since 2026-11-24 it is itself refined by ADR-0121 (design 046 adds package rules to the lease check of
decision 8) and refined in its own amendment below; it relates to ADR-0082 (the registry that activates it),
ADR-0079 (lake mode serves no script language), ADR-0052 (warm containers, decision 11 below) and ADR-0120
(design 046's open dependencies for TypeScript and Python, which keeps decision 4 for every other language)
**Enforced by:** `crates/infra-schema` bundle and catalog unit tests over the fixtures of T023-003; the
real-container suites `integration_script_languages_containers` (probes, hostile battery per language)
in the `native-runners` CI job on AMD64 and ARM64; `contract_rest_script_languages`; the provenance
contract tests (T023-047) and per-build provenance tests (T023-009); the lease-time policy tests
(T023-061); the `python-data` binding fixtures (T023-062); the reserved host-call gate, budget and secret-net
tests (T023-064, T023-065); `scripts/check-architecture.py` for crate placement. The rule "no product
configuration selects Deno or Bun" is checked by T023-049. Added 2026-11-24: the fleet-activation tests
(T023-067), the budget test of design 038 with its script-step shape (T023-061), the rule of
`scripts/check-architecture.py` that keeps the in-toto and DSSE types inside `sagat-infra-packages`
(T023-068), and the lake-mode refusal test (T023-069). None exists yet; until they do, this decision is
review only.

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

### Amendment of 2026-11-24 (delegate, OD-2026-11-24-4)

The decisions above stand. The following refine them; each is argued in
`specs/023-script-languages/research.md` (Decisions 1234 to 1239 and 1625 to 1627).

- **Decision 2, bundle numbers.** Version 4 is the only bundle version this decision adds. Bash and
  PowerShell use version 1 and Rust version 3, unchanged. Design 046 takes version 5 for its additive
  bundle (ADR-0120 decision 5); no other number is taken by either (research Decision 1236).
- **Decision 4, scope.** The closed offline catalogs hold for Rust and PHP (and C#, ADR-0021); Bash and
  PowerShell take no packages. TypeScript and Python open dependencies are governed by ADR-0120 instead,
  resolved at publication through a platform package gateway and never during a customer build or run. The
  `python-data` exception is reserved: its id is built now, its image-catalog mechanism only once feature
  024 confirms it needs an image catalog rather than design 046's dependency layer (research Decision 1625).
- **Decision 5, where languages are served.** Only the runner's script mode (`serve`) serves a language.
  Lake mode (ADR-0079) refuses every `SAGAT_RUNNER_` variable, which covers the new ones.
- **Decision 6, one implementation.** The statement, the DSSE envelope and the SBOM package-set rule are
  one implementation, design 046's provenance and SBOM builders in `crates/infra-packages`, built first by
  design 046 and extended by feature 023 with its build type, behind an application port, with the DSSE
  signature made beside the key in `crates/application/src/script_signing.rs` (research Decisions 1236 and
  1627).
- **Decision 8 and the policy, ownership.** The namespace script policy document and its endpoint, and the
  lease-time statement (it reads design 046's `script_artifact_blocks`, keyed by tenant, artifact digest and
  namespace, because artifact rows are immutable; corrected on 2026-11-24 after design 046's review, feature 023
  Decision 1628), are design
  046's and built first by it (ADR-0121); feature 023's language policy is a section of that document and
  its language reason extends that statement, with no second read (research Decision 1627).
- **Decision 8, the lease read.** The check at lease is one statement with no row lock, made only for
  script activities once the step to lease is chosen; it costs a non-script lease nothing and a script-step
  lease one statement in design 038's budget. A lease transaction that read the policy before a disabling
  update committed may lease once more, and that attempt finishes like one already leased. Design 046
  extends this same statement (ADR-0121) instead of adding one (research Decision 1235).
- **Decision 10 (new), fleet activation.** The new language ids and every narrowing namespace policy are
  activated through the instance registry (ADR-0082) as capability `script-languages-v1`, required of
  `CONTROL_PLANE` and `EXECUTION_ENGINE`. Until an operator records it, a script of a new language and a
  narrowing policy are refused `SCRIPT_LANGUAGES_CAPABILITY_DISABLED`; removing it is refused while such a
  script or a policy row exists. For the upgrade from 0.2.x, whose processes predate the registry, it is
  recorded only once none is left (research Decision 1234).
- **Decision 11 (new), warm containers.** A warm-container policy (ADR-0052) does not apply to an artifact
  of a new language until the warm residue suite has a passing fixture for that language on each enabled
  target (research Decision 1239).

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
- Added 2026-11-24: an operator records `script-languages-v1` after the control planes and engines are
  upgraded; until then the four languages and narrowing policies are refused with a reason that says so.
  The claim path of a script step pays one statement and takes no row lock; non-script steps pay nothing.
  Feature 023 adds no crate: it extends design 046's `sagat-infra-packages`, its policy document and its
  lease statement, and so depends on design 046's Tasks T046-2, T046-15 and T046-17 for those parts.

## Evidence

None yet: this is a proposal. Required before acceptance of the implementation: the hostile-script
battery per language and target, native AMD64 and ARM64 Rust probes in CI, malformed v4/ELF/lock
fixtures refused, v1/v2/v3 regressions unchanged, provenance verification and tamper tests, and the
benchmark report, the lease-time refusal tests, the `python-data` binding fixtures and the 0.2.0
rollback observation (`specs/023-script-languages/tasks.md`, T023-029..033, T023-025, T023-008, T023-047,
T023-056, T023-061, T023-062, T023-063). Sources: `specs/023-script-languages/research.md` §2 (Composer, PHP, PowerShell, Deno, Bun,
rustc, Cargo, in-toto, SLSA and DSSE documentation).
