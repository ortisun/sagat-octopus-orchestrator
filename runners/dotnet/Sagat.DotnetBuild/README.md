# Sagat.DotnetBuild

## Overview

Trusted Roslyn builder compiling inline C# against an approved immutable package catalog.

## Interface

- `Program.Main` implements the builder executable; `Catalog` handles optional dependency approval. [runners/dotnet/Sagat.DotnetBuild/Program.cs:Program.Main]

## Invariants

- Requires nonblank source and exactly one concrete `IInlineScript` with a public parameterless constructor; uses C# 14, deterministic non-concurrent compilation, and no unsafe code. [runners/dotnet/Sagat.DotnetBuild/Program.cs:Program.Compile]
- The optional package is pinned Newtonsoft.Json 13.0.4 with exact lockfile shape, content/archive hashes, assembly name, and assembly SHA-256. [runners/dotnet/Sagat.DotnetBuild/Program.cs:Catalog.Approve]
- Duplicate JSON members and unexpected properties are rejected; unapproved packages and changed catalog assemblies fail approval. [runners/dotnet/Sagat.DotnetBuild/Program.cs:Catalog.EnsureUnique]

## Side Effects

- Reads approved catalog files and assemblies, compiles in memory, and emits built/rejected JSON. [runners/dotnet/Sagat.DotnetBuild/Program.cs:Program.Main]
- The image build restores/publishes the trusted host and verifies package archive and assembly hashes before copying the catalog. [runners/dotnet/Dockerfile.builder:RUN]

## Verification

From the repository root, with Docker available for the container integration suite and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-infra-sandbox --locked
cargo test -p sagat-tests --test integration_inline_code_sandbox_containers --locked
```

The crate checks and container integration target are declared in `crates/infra-sandbox/Cargo.toml` and `tests/Cargo.toml`.
