using System.Text.Json;

namespace SagatOctopusOrchestrator.Sdk.Internal;

/// <summary>
/// The canonical errors of the SDKs (<c>sdks/error-catalog.json</c>), embedded as a resource. A test proves the
/// embedded copy equals the file in the repository.
/// </summary>
internal static class ErrorCatalog
{
    internal sealed record Entry(string Code, string Origin, IReadOnlyList<int> Http, string Grpc, bool Retryable, string Remediation);

    internal const string ResourceName = "error-catalog.json";

    private static readonly Lazy<IReadOnlyList<Entry>> Loaded = new(Load);

    internal static IReadOnlyList<Entry> Entries => Loaded.Value;

    internal static byte[] RawBytes()
    {
        using var stream = SdkInfo.Assembly.GetManifestResourceStream(ResourceName)
            ?? throw new InvalidOperationException("The embedded error catalog is missing from the assembly.");
        using var copy = new MemoryStream();
        stream.CopyTo(copy);
        return copy.ToArray();
    }

    private static List<Entry> Load()
    {
        using var document = JsonDocument.Parse(RawBytes());
        var entries = new List<Entry>();
        foreach (var item in document.RootElement.GetProperty("errors").EnumerateArray())
        {
            var http = item.GetProperty("http").EnumerateArray().Select(v => v.GetInt32()).ToArray();
            entries.Add(new Entry(
                item.GetProperty("code").GetString()!,
                item.GetProperty("origin").GetString()!,
                http,
                item.GetProperty("grpc").GetString()!,
                item.GetProperty("retryable").GetBoolean(),
                item.GetProperty("remediation").GetString()!));
        }

        return entries;
    }
}
