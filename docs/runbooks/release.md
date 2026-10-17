# Runbook: cutting a release (GitHub Releases)

The owner decided on 2026-10-16 that GitHub Releases is the only distribution channel (G123, G203,
G216): no npm, PyPI, NuGet or Maven publication for now. Go is consumed from the module path in this
repository (`sdks/go`), tagged `sdks/go/v<version>` as described in [sdk-packaging.md](sdk-packaging.md).

## What a release contains

| File | Built by |
|---|---|
| `sagat-<version>-x86_64-unknown-linux-gnu.tar.gz` | `cargo build --release --locked -p sagat-cli` on `ubuntu-24.04` |
| `sagat-<version>-aarch64-unknown-linux-gnu.tar.gz` | the same on `ubuntu-24.04-arm` |
| `sagat-<version>-aarch64-apple-darwin.tar.gz` | the same on `macos-14` |
| The SDK packages (npm tarball, Python wheel and sdist, Go module source archive, NuGet package, Maven JARs and POMs) and the SDK release notes | `scripts/package-sdks.sh`, which also installs each into a clean consumer; its own `SHA256SUMS` is replaced by the release-wide one |
| `SHA256SUMS` | the release job, over every file above |

Each archive holds the binary and `LICENSE`. Every file has a **build provenance attestation**
(Sigstore, through GitHub artifact attestations) made by the release job.

## Cutting it

1. Bump the version everywhere it is stated: `Cargo.toml` (`[workspace.package] version`) and
   `Cargo.lock` (`cargo update --workspace --offline`), `scripts/package-sdks.sh` (`SDK_VERSION`)
   with every SDK manifest and version constant, `frontend/apps/studio/package.json` and the
   lockfile entries, the OpenAPI `info.version`, and `infra/helm` `Chart.yaml`.
   `bash scripts/release-version.sh` prints the version, or fails naming the file that disagrees.
2. Write `docs/releases/<version>.md`: the release job refuses to publish without it.
3. Merge the release PR into `main` (Gitflow `release/<version>`), then tag the merge commit:
   `git tag v<version> && git push origin v<version>`.
4. The `release` workflow checks that the tag names the workspace version, builds the three CLI
   archives and the SDK packages, runs the disaster-recovery drill against the reference
   objectives (RPO 0, failover ≤ 5 min, restore ≤ 1 h; [disaster-recovery.md](disaster-recovery.md)),
   writes `SHA256SUMS`, attests every file and creates the release titled `v<version>` with the
   notes of step 2. Nothing is published when any job fails.

A pull request that changes `.github/workflows/release.yml`, `scripts/package-sdks.sh` or
`scripts/release-version.sh` runs every build job and stops before attesting and publishing.

## Verifying a download

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing
gh attestation verify sagat-<version>-aarch64-apple-darwin.tar.gz --repo ortisun/sagat-octopus-orchestrator
```

## Not covered

- Package registries (npm, PyPI, NuGet, Maven Central) and the Go proxy beyond the module tag: not
  part of the decision.
- Windows and macOS x86_64 binaries of `sagat`: not built.
- Hosted documentation (G216): still open.
- Long-term retention of the artifacts beyond what GitHub keeps for a release.
