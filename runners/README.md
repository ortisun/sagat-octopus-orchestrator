# runners

## Overview

Trusted builder and runtime hosts for isolated inline C#, Go, and Java scripts.

## Interface

- [dotnet](dotnet/README.md) — The C# script contract and separate trusted Roslyn builder and assembly runtime hosts.
- [go](go/README.md) — Trusted Go build/cache/runtime commands and the octopus host-call package embedded into inline programs.
- [java](java/README.md) — The Java inline-script API and separate trusted compilation and runtime hosts.

## Invariants

- C# compilation uses the approved package catalog and requires one concrete IInlineScript entry type. [runners/dotnet/Sagat.DotnetBuild/Program.cs:Program.Compile] [runners/dotnet/Sagat.DotnetBuild/Program.cs:Catalog.Approve]
- Java artifacts are restricted to Script.class and nested classes with explicit count/size bounds. [runners/java/host/sagat/host/BuildHost.java:build]
- Go runtime artifacts require the native target, canonical bounded gzip, and a single script.elf. [runners/go/runtime.py:decode_artifact]

## Side Effects

- Build hosts read or write compilation inputs and emit built/rejected JSON protocol results. [runners/dotnet/Sagat.DotnetBuild/Program.cs:Program.Main] [runners/go/buildhost.go:build] [runners/java/host/sagat/host/BuildHost.java:build]
- Runtime hosts execute submitted artifacts and mediate host operations; ordinary script diagnostics use stderr. [runners/dotnet/Sagat.DotnetRuntime/Program.cs:Program.Main] [runners/java/host/sagat/host/RuntimeHost.java:RuntimeHost.main] [runners/go/octopus.go:Channel]

## Verification

From the repository root, with Docker available for the container integration suite and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-infra-sandbox --locked
cargo test -p sagat-tests --test integration_inline_code_sandbox_containers --locked
```

The crate checks and container integration target are declared in `crates/infra-sandbox/Cargo.toml` and `tests/Cargo.toml`.
