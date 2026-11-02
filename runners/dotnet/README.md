# C# sandbox hosts

The builder and runtime are separate immutable images. The builder uses the Roslyn assemblies from
the pinned .NET 10.0.401 SDK and a closed package catalog. A build container has no network; it
compiles source directly and never runs MSBuild or a customer project. The runtime image has no SDK
or package catalog and loads only the signed bundle's managed assemblies from memory.

A script defines exactly one public, parameterless `IInlineScript`:

```csharp
using System.Text.Json;
using Sagat.ScriptContract;

public sealed class Script : IInlineScript
{
    public ValueTask<JsonElement> RunAsync(
        JsonElement input,
        IOctopus octopus,
        CancellationToken cancellationToken) =>
        ValueTask.FromResult(JsonSerializer.SerializeToElement(new { ok = true }));
}
```

`IOctopus` is the only route to an HTTP request or a leased secret. The runtime container itself has
no network. `IdempotencyKey` is stable for every attempt of one workflow step.

The image build verifies the selected archive's SHA-512 and managed DLL's SHA-256. Catalog restore
uses NuGet audit in `all` mode, treats `NU1901` through `NU1904` as errors, and prints the direct and
transitive vulnerability report in the image build log.

## Interface

- [Sagat.DotnetBuild](Sagat.DotnetBuild/README.md) — Trusted Roslyn builder compiling inline C# against an approved immutable package catalog.
- [Sagat.DotnetRuntime](Sagat.DotnetRuntime/README.md) — Trusted .NET runtime loading submitted assemblies and mediating host calls through JSON standard I/O.
- [Sagat.ScriptContract](Sagat.ScriptContract/README.md) — Public .NET script entry and mediated host-call contract assembly.

## Invariants

- `Sagat.DotnetBuild`: Requires nonblank source and exactly one concrete `IInlineScript` with a public parameterless constructor; uses C# 14, deterministic non-concurrent compilation, and no unsafe code. [runners/dotnet/Sagat.DotnetBuild/Program.cs:Program.Compile]
- `Sagat.DotnetRuntime`: Requires `op=run`, `encoding=base64`, `script.dll`, and exactly one concrete instantiable `IInlineScript` type. [runners/dotnet/Sagat.DotnetRuntime/Program.cs:Program.Main]
- `Sagat.ScriptContract`: The explicit assembly identity is `Sagat.ScriptContract`. [runners/dotnet/Sagat.ScriptContract/Sagat.ScriptContract.csproj:AssemblyName]

## Side Effects

- `Sagat.DotnetBuild`: Reads approved catalog files and assemblies, compiles in memory, and emits built/rejected JSON. [runners/dotnet/Sagat.DotnetBuild/Program.cs:Program.Main]
- `Sagat.DotnetRuntime`: Redirects script stdout to stderr, loads assemblies from memory, executes the script, and emits ready/result/error JSON. [runners/dotnet/Sagat.DotnetRuntime/Program.cs:Program.Main]
- `Sagat.ScriptContract`: Implementations mediate fetch, secret access, and logging; this assembly defines the contract without implementing those effects. [runners/dotnet/Sagat.ScriptContract/Contract.cs:IOctopus]

## Verification

From the repository root, with Docker available for the container integration suite and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-infra-sandbox --locked
cargo test -p sagat-tests --test integration_inline_code_sandbox_containers --locked
```

The crate checks and container integration target are declared in `crates/infra-sandbox/Cargo.toml` and `tests/Cargo.toml`.
