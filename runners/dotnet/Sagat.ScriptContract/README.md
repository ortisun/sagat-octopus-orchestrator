# Sagat.ScriptContract

## Overview

Public .NET script entry and mediated host-call contract assembly.

## Interface

- `IInlineScript.RunAsync` accepts JSON input, `IOctopus`, and cancellation. [runners/dotnet/Sagat.ScriptContract/Contract.cs:IInlineScript.RunAsync]
- `IOctopus` exposes `IdempotencyKey`, `FetchAsync`, `SecretAsync`, and `Log`; `ScriptResponse` carries status, headers, and body. [runners/dotnet/Sagat.ScriptContract/Contract.cs:IOctopus]

## Invariants

- The explicit assembly identity is `Sagat.ScriptContract`. [runners/dotnet/Sagat.ScriptContract/Sagat.ScriptContract.csproj:AssemblyName]

## Side Effects

- Implementations mediate fetch, secret access, and logging; this assembly defines the contract without implementing those effects. [runners/dotnet/Sagat.ScriptContract/Contract.cs:IOctopus]

## Verification

From the repository root, with Docker available for the container integration suite and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-infra-sandbox --locked
cargo test -p sagat-tests --test integration_inline_code_sandbox_containers --locked
```

The crate checks and container integration target are declared in `crates/infra-sandbox/Cargo.toml` and `tests/Cargo.toml`.
