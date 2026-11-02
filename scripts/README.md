# Repository verification and packaging scripts

## Overview

Architecture/configuration/vendor fitness checks, version consistency, SDK packaging and disposable Compose smoke verification. Test-only scripts are outside this contract inventory.

## Interface

- `check-agent-config.py [repo-root]` validates agent configuration and documentation links; `check-architecture.py [--metadata FILE]` checks crate layers/dependencies/cycles. [check-agent-config.py:check] [check-architecture.py:main]
- `check-vendor.py` verifies the exact upstream snapshot and reviewed manifest override. `release-version.sh` prints the product version after checking workspace, SDK packaging and Studio declarations. [check-vendor.py:verify] [release-version.sh:workspace]
- `package-sdks.sh [output-directory]` builds six-language artifacts and consumers; `package-rust-sdk.sh <output-directory>` packages committed Rust SDK source and runs an extracted-artifact consumer. [package-sdks.sh:SDK_VERSION] [package-rust-sdk.sh:output_dir]
- `smoke-compose.sh` supports `ENV_FILE`, optional `SAGAT_BIN` and `SAGAT_SMOKE_BROWSER` for isolated stack and user-journey checks. [smoke-compose.sh:ENV_FILE] [smoke-compose.sh:SAGAT_BIN] [smoke-compose.sh:SAGAT_SMOKE_BROWSER]

## Invariants

- Architecture checks enforce domain dependency allow/deny lists, inward layer dependencies and no workspace cycles. Agent checks enforce instruction/skill budgets, read-only reviewer allowlists, hooks, ADR identity and links. [check-architecture.py:DOMAIN_ALLOWED] [check-architecture.py:ALLOWED_LAYER_DEPS] [check-agent-config.py:check]
- Vendor validation requires exactly 84 upstream files with recorded hashes, forbids symlinks/special files and permits only the reviewed zstd manifest substitution. [check-vendor.py:FILE_COUNT] [check-vendor.py:verify]
- Packaging refuses conflicting output/working directories; Rust packaging requires clean committed SDK source and an external absolute Cargo target directory. Cleanup removes only directories created by that invocation. [package-sdks.sh:cleanup] [package-rust-sdk.sh:CARGO_TARGET_DIR]
- Compose smoke reads environment data without sourcing it and owns a PID-scoped project. Version consistency requires matching semver declarations. [smoke-compose.sh:env_value] [smoke-compose.sh:PROJECT] [release-version.sh:workspace]

## Side Effects

- Fitness checks read files, hash contents and print results; architecture checking can invoke Cargo metadata. [check-agent-config.py:main] [check-vendor.py:main] [check-architecture.py:load]
- Packaging invokes language build/package/install tools, writes distribution artifacts, provenance/checksums and temporary consumer/cache directories. Compose smoke builds images, starts services, applies migrations/seeds, optionally runs workflows/browser journeys, then removes its own project and volumes. [package-sdks.sh:SHA256SUMS] [package-rust-sdk.sh:RUST_SDK_PROVENANCE] [smoke-compose.sh:cleanup]

## Verification

Run from the repository root:

```sh
python3 scripts/check-architecture.py
python3 scripts/check-agent-config.py
python3 scripts/check-vendor.py
bash scripts/release-version.sh
```

Commands are verification entry points, not results from this documentation pass. Use the repository resource profile in [AGENTS.md](../AGENTS.md) for Cargo builds; integration/drill commands need their declared services and tools.
