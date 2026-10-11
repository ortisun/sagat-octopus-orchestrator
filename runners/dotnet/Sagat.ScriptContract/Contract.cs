using System.Text.Json;

namespace Sagat.ScriptContract;

public interface IInlineScript
{
    ValueTask<JsonElement> RunAsync(
        JsonElement input,
        IOctopus octopus,
        CancellationToken cancellationToken);
}

public interface IOctopus
{
    string? IdempotencyKey { get; }

    ValueTask<ScriptResponse> FetchAsync(
        string url,
        string method = "GET",
        IReadOnlyDictionary<string, string>? headers = null,
        string? body = null,
        CancellationToken cancellationToken = default);

    ValueTask<string> SecretAsync(string name, CancellationToken cancellationToken = default);

    void Log(params object?[] values);
}

public sealed record ScriptResponse(
    int Status,
    IReadOnlyDictionary<string, string> Headers,
    string Body);
