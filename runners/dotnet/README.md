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
