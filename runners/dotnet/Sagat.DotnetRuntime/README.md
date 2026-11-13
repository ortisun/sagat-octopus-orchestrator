# Sagat.DotnetRuntime

## Overview

Trusted .NET runtime loading submitted assemblies and mediating host calls through JSON standard I/O.

## Interface

- `Program.Main` is the runtime executable entry point; internal envelope, load-context, and `Octopus` types implement its protocol. [runners/dotnet/Sagat.DotnetRuntime/Program.cs:Program.Main]

## Invariants

- Requires `op=run`, `encoding=base64`, `script.dll`, and exactly one concrete instantiable `IInlineScript` type. [runners/dotnet/Sagat.DotnetRuntime/Program.cs:Program.Main]
- The collectible load context shares the contract assembly and resolves other submitted assemblies by name. [runners/dotnet/Sagat.DotnetRuntime/Program.cs:ScriptLoadContext.Load]
- Host calls are guarded by a semaphore and increasing IDs; unmatched replies are ignored, and refused or closed-channel replies fail. [runners/dotnet/Sagat.DotnetRuntime/Program.cs:Octopus.Call]

## Side Effects

- Redirects script stdout to stderr, loads assemblies from memory, executes the script, and emits ready/result/error JSON. [runners/dotnet/Sagat.DotnetRuntime/Program.cs:Program.Main]
- Mediates fetch/secret requests over standard I/O and writes script logs to stderr. [runners/dotnet/Sagat.DotnetRuntime/Program.cs:Octopus]

## Verification

From the repository root, with Docker available for the container integration suite and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-infra-sandbox --locked
cargo test -p sagat-tests --test integration_inline_code_sandbox_containers --locked
```

The crate checks and container integration target are declared in `crates/infra-sandbox/Cargo.toml` and `tests/Cargo.toml`.
