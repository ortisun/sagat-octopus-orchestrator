# SDK packaging and clean-install verification

This runbook covers FR-006-008 for the TypeScript, Python, Go, C#, Java and Rust SDKs. It builds local release artifacts at the pinned SDK version, installs each artifact into a temporary consumer, exercises a public SDK symbol, checks that the package carries the MIT license and writes SHA-256 checksums. It never publishes to an external registry.

## Build and verify all packages

Use the repository toolchain versions from `.github/workflows/sdk-packages.yml`, then run:

```console
$ bash scripts/package-sdks.sh
```

The default output is `target/sdk-packages`. The directory must be absent or empty so a stale artifact cannot pass the smoke tests. To choose another empty directory:

```console
$ bash scripts/package-sdks.sh /tmp/sagat-sdk-packages
```

The script records `SAGAT_SOURCE_REVISION` (default: the commit at startup) in the NuGet package. It uses `SOURCE_DATE_EPOCH` when set and otherwise uses that commit's timestamp. It stages the TypeScript, Python and Java package sources in a temporary directory, redirects .NET build output to the dedicated `target/.sdk-package-dotnet-build` directory, builds with fixed SDK and build-tool versions, normalizes source, NuGet and Maven classifier archive metadata, uses isolated caches for each clean consumer, and removes its workspace on exit. An atomic directory creation prevents concurrent local packaging runs from sharing .NET generated sources. It does not clean or write another test run's SDK build directories. Network access is required to restore each package manager's pinned dependencies.

Set `PYTHON_BIN` when the system `python3` is outside the tested Python 3.10–3.14 build range, for example `PYTHON_BIN=python3.14 bash scripts/package-sdks.sh`. CI uses Python 3.12.

The expected version is `0.2.0`. The script rejects a different version or license before building. A release that changes the version must update the constant at the top of `scripts/package-sdks.sh` together with every SDK manifest and public version constant.

## Artifacts

The output contains:

- `sagat-octopus-orchestrator-sdk-0.2.0.tgz`: npm package, including `LICENSE`.
- `sagat_octopus_orchestrator-0.2.0-py3-none-any.whl` and `sagat_octopus_orchestrator-0.2.0.tar.gz`: Python wheel and source distribution, including `LICENSE`.
- `sagat-octopus-orchestrator-go-0.2.0.tar.gz`: deterministic Go module source archive, including `go.mod`, `go.sum` and `LICENSE`.
- `SagatOctopusOrchestrator.Sdk.0.2.0.nupkg`: NuGet package, including `LICENSE.txt`.
- `sagat-octopus-orchestrator-sdk-0.2.0.jar`, `-sources.jar`, `-javadoc.jar`, its POM and the parent POM: Maven release candidate, with `META-INF/LICENSE` in every JAR.
- `SDK_RELEASE_NOTES-0.2.0.md`: reviewed notes for the five SDK packages at this version.
- `SHA256SUMS`: checksums of every distributable file.

The npm consumer imports `SDK_VERSION` and builds a workflow document. The Python consumer installs locked runtime dependencies, imports `__version__` and builds the same minimal document. The Go, .NET and Java consumers compile and run against the extracted or locally installed artifact and assert the public SDK version. The Java smoke repository installs the candidate source and Javadoc classifiers with the main JAR and verifies their bytes before compiling the consumer. The .NET smoke test prefers .NET 8 and uses .NET 10 when only that supported runtime is installed.

The `sdk packages` GitHub Actions workflow runs the same command for SDK changes, manual dispatches and `sdk-v*` tags. It uploads one short-lived workflow artifact. The workflow has read-only repository permissions and needs no publishing credentials.

## Releases go to GitHub Releases

On 2026-10-16 the owner chose GitHub Releases as the only channel: the `release` workflow attaches these packages, `SHA256SUMS` and provenance attestations to the release `v<version>` ([release.md](release.md)). The registry steps below are kept for a later decision and are not run.

## Registry publication remains a release-owner action

Before external publication, the release owner must review the checksums and release notes, then configure the registry-specific identity and provenance:

- npm requires an organization publishing token or trusted-publisher configuration for `@sagat-octopus-orchestrator/sdk`.
- PyPI requires a trusted publisher or scoped API token for `sagat-octopus-orchestrator`.
- NuGet.org requires an API key scoped to `SagatOctopusOrchestrator.Sdk`.
- Maven Central requires namespace ownership, Central Portal credentials and artifact signing. The local candidate already includes the required source and Javadoc artifacts.
- The Go module is released by signing and pushing the module tag `sdks/go/v0.2.0`; the public Go proxy fetches the module from Git rather than accepting this tarball.

Registry uploads, release signing and long-term artifact retention are intentionally outside this credential-free workflow. The repository and all five package formats use the MIT license.

## Rust crate candidate

`bash scripts/package-rust-sdk.sh /tmp/sagat-rust-artifact` builds the committed Rust SDK source
with the pinned Rust toolchain and Python 3.12 or newer. The helper refuses uncommitted SDK files,
packages with `cargo package --locked`, checks the MIT license, extracts the actual `.crate`,
and compiles/runs an independent consumer using only that extracted artifact. A fresh consumer
lock is resolved once, then the run is repeated with `--locked`. It records the source commit,
crate version and toolchain in `RUST_SDK_PROVENANCE.txt`. Its temporary workspace is removed; an
explicit caller-owned external `CARGO_TARGET_DIR` is reused and left to its owner.

The common packaging command also invokes this helper before checksums. The first Rust crate is
a 0.3.0 candidate while the existing released packages remain at 0.2.0 until coordinated release
preparation. GitHub Release assets include the `.crate`; crates.io publication is not enabled.
