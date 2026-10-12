using System.Reflection;
using System.Text.Json;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>What the SDK shares with the repository: the error catalog and the worker protocol. Each is a copy made by scripts/sync-shared.sh.</summary>
public class SharedContractsTests
{
    private const string Hint = "run `bash sdks/dotnet/scripts/sync-shared.sh`";

    [Fact]
    public void TheEmbeddedErrorCatalogIsTheSharedOne()
    {
        var shared = File.ReadAllBytes(Repository.Path("sdks", "error-catalog.json"));
        using var stream = typeof(SagatClient).Assembly.GetManifestResourceStream("error-catalog.json");
        Assert.NotNull(stream);
        using var copy = new MemoryStream();
        stream.CopyTo(copy);
        Assert.True(shared.AsSpan().SequenceEqual(copy.ToArray()), $"the embedded catalog is behind the file: {Hint}");
    }

    [Fact]
    public void TheSourceCopyOfTheCatalogIsTheSharedOne()
    {
        var shared = File.ReadAllBytes(Repository.Path("sdks", "error-catalog.json"));
        var copy = File.ReadAllBytes(Repository.Path("sdks", "dotnet", "src", "SagatOctopusOrchestrator.Sdk", "error-catalog.json"));
        Assert.True(shared.AsSpan().SequenceEqual(copy), $"the catalog copy is behind the file: {Hint}");
    }

    [Fact]
    public void TheWorkerProtocolTheSdkIsBuiltFromIsTheOneOfTheContracts()
    {
        var contract = File.ReadAllBytes(Repository.Path("specs", "001-platform-foundation", "contracts", "worker.proto"));
        var shipped = File.ReadAllBytes(Repository.Path("sdks", "dotnet", "proto", "worker.proto"));
        Assert.True(contract.AsSpan().SequenceEqual(shipped), $"the proto copy is behind the contract: {Hint}");
    }

    [Fact]
    public void EveryCatalogEntryHasACodeAndEveryCodeAnEntryWithItsRemediationAndDefault()
    {
        using var document = JsonDocument.Parse(File.ReadAllBytes(Repository.Path("sdks", "error-catalog.json")));
        var entries = document.RootElement.GetProperty("errors").EnumerateArray().ToList();
        var codes = entries.Select(e => e.GetProperty("code").GetString()!).ToList();
        Assert.Equal(codes.Count, codes.Distinct().Count());

        foreach (var entry in entries)
        {
            var wire = entry.GetProperty("code").GetString()!;
            Assert.True(SagatErrorCodes.TryParse(wire, out var code), $"{wire} is not a SagatErrorCode");
            Assert.Equal(wire, code.ToWireName());
            Assert.True(entry.GetProperty("remediation").GetString()!.Length > 20, wire);
            Assert.Equal(entry.GetProperty("remediation").GetString(), code.Remediation());
            Assert.Equal(entry.GetProperty("retryable").GetBoolean(), code.RetryableByDefault());
        }

        var enumCodes = Enum.GetValues<SagatErrorCode>().Select(c => c.ToWireName()).Order().ToList();
        Assert.Equal(codes.Order().ToList(), enumCodes);
    }

    [Fact]
    public void AStatusWithoutAnEnvelopeIsClassifiedByTheCatalog()
    {
        Assert.Equal(SagatErrorCode.Unauthenticated, SagatErrorCodes.FromHttpStatus(401));
        Assert.Equal(SagatErrorCode.NotFound, SagatErrorCodes.FromHttpStatus(404));
        Assert.Equal(SagatErrorCode.Conflict, SagatErrorCodes.FromHttpStatus(409));
        Assert.Equal(SagatErrorCode.ValidationFailed, SagatErrorCodes.FromHttpStatus(422));
        Assert.Equal(SagatErrorCode.RateLimited, SagatErrorCodes.FromHttpStatus(429));
        Assert.Equal(SagatErrorCode.Unavailable, SagatErrorCodes.FromHttpStatus(503));
        Assert.Equal(SagatErrorCode.Internal, SagatErrorCodes.FromHttpStatus(500));
        Assert.Equal(SagatErrorCode.Internal, SagatErrorCodes.FromHttpStatus(501));
        Assert.Equal(SagatErrorCode.BadRequest, SagatErrorCodes.FromHttpStatus(418));
        foreach (var status in new[] { 400, 413, 415 })
        {
            Assert.Equal(SagatErrorCode.BadRequest, SagatErrorCodes.FromHttpStatus(status));
        }
    }

    [Fact]
    public void TheSdkVersionIsTheOneOfThePackage()
    {
        var informational = typeof(SagatClient).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()!.InformationalVersion;
        Assert.StartsWith(SdkInfo.Version, informational, StringComparison.Ordinal);
        Assert.Equal("1.3", SdkInfo.ProtocolVersion);
        Assert.Equal("SagatOctopusOrchestrator.Sdk", SdkInfo.ActivitySourceName);
    }
}
