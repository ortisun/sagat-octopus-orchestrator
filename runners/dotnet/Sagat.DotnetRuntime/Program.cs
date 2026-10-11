using System.Reflection;
using System.Runtime.Loader;
using System.Text.Json;
using Sagat.ScriptContract;

internal static class Program
{
    private static readonly JsonSerializerOptions Json = new(JsonSerializerDefaults.Web);

    public static async Task<int> Main()
    {
        var channel = Console.Out;
        Console.SetOut(Console.Error);
        try
        {
            await Say(channel, new { op = "ready" });
            var line = await Console.In.ReadLineAsync();
            var request = JsonSerializer.Deserialize<RunEnvelope>(line ?? string.Empty, Json)
                ?? throw new InvalidOperationException("the run request is not JSON");
            if (request.Op != "run" || request.Encoding != "base64")
            {
                throw new InvalidOperationException("the run request has another shape");
            }
            var assemblies = request.Files.ToDictionary(
                pair => pair.Key,
                pair => Convert.FromBase64String(pair.Value),
                StringComparer.OrdinalIgnoreCase);
            if (!assemblies.TryGetValue("script.dll", out var scriptBytes))
            {
                throw new InvalidOperationException("the run request has no script.dll");
            }
            using var context = new ScriptLoadContext(assemblies);
            var assembly = context.LoadFromStream(new MemoryStream(scriptBytes, writable: false));
            var types = assembly.GetTypes()
                .Where(type => !type.IsAbstract && typeof(IInlineScript).IsAssignableFrom(type))
                .ToArray();
            if (types is not [var scriptType])
            {
                throw new InvalidOperationException("the artifact must contain one IInlineScript");
            }
            var script = Activator.CreateInstance(scriptType) as IInlineScript
                ?? throw new InvalidOperationException("the script cannot be constructed");
            var host = new Octopus(channel, request.IdempotencyKey);
            var result = await script.RunAsync(request.Input, host, CancellationToken.None);
            await Say(channel, new { op = "result", value = result });
            return 0;
        }
        catch (Exception error)
        {
            await Say(channel, new
            {
                op = "error",
                message = error.GetBaseException().Message,
                stack = error.ToString(),
            });
            return 1;
        }
    }

    private static async Task Say(TextWriter channel, object message)
    {
        await channel.WriteLineAsync(JsonSerializer.Serialize(message, Json));
        await channel.FlushAsync();
    }
}

internal sealed record RunEnvelope(
    string Op,
    JsonElement Input,
    string Encoding,
    Dictionary<string, string> Files,
    string? IdempotencyKey);

internal sealed class ScriptLoadContext(IReadOnlyDictionary<string, byte[]> files)
    : AssemblyLoadContext("sagat-inline-script", isCollectible: true), IDisposable
{
    protected override Assembly? Load(AssemblyName assemblyName)
    {
        if (assemblyName.Name == typeof(IInlineScript).Assembly.GetName().Name)
        {
            return typeof(IInlineScript).Assembly;
        }
        if (assemblyName.Name is { } name && files.TryGetValue($"{name}.dll", out var bytes))
        {
            return LoadFromStream(new MemoryStream(bytes, writable: false));
        }
        return null;
    }

    public void Dispose() => Unload();
}

internal sealed class Octopus(TextWriter channel, string? idempotencyKey) : IOctopus
{
    private readonly SemaphoreSlim _calls = new(1, 1);
    private long _nextId;

    public string? IdempotencyKey { get; } = idempotencyKey;

    public async ValueTask<ScriptResponse> FetchAsync(
        string url,
        string method = "GET",
        IReadOnlyDictionary<string, string>? headers = null,
        string? body = null,
        CancellationToken cancellationToken = default)
    {
        var reply = await Call(new
        {
            op = "fetch",
            url,
            method,
            headers = headers ?? new Dictionary<string, string>(),
            body,
        }, cancellationToken);
        return new ScriptResponse(
            reply.GetProperty("status").GetInt32(),
            reply.GetProperty("headers").EnumerateObject()
                .ToDictionary(property => property.Name, property => property.Value.GetString() ?? string.Empty),
            reply.GetProperty("body").GetString() ?? string.Empty);
    }

    public async ValueTask<string> SecretAsync(
        string name,
        CancellationToken cancellationToken = default)
    {
        var reply = await Call(new { op = "secret", name }, cancellationToken);
        return reply.GetProperty("value").GetString() ?? string.Empty;
    }

    public void Log(params object?[] values) => Console.Error.WriteLine(string.Join(" ", values));

    private async ValueTask<JsonElement> Call(object request, CancellationToken cancellationToken)
    {
        await _calls.WaitAsync(cancellationToken);
        try
        {
            var id = Interlocked.Increment(ref _nextId);
            var fields = JsonSerializer.SerializeToElement(request, new JsonSerializerOptions(JsonSerializerDefaults.Web));
            var message = new Dictionary<string, object?> { ["id"] = id };
            foreach (var property in fields.EnumerateObject())
            {
                message[property.Name] = property.Value.Clone();
            }
            await channel.WriteLineAsync(JsonSerializer.Serialize(message));
            await channel.FlushAsync(cancellationToken);
            while (true)
            {
                cancellationToken.ThrowIfCancellationRequested();
                var line = await Console.In.ReadLineAsync(cancellationToken)
                    ?? throw new InvalidOperationException("the platform closed the channel");
                using var document = JsonDocument.Parse(line);
                var reply = document.RootElement;
                if (!reply.TryGetProperty("id", out var replyId) || replyId.GetInt64() != id)
                {
                    continue;
                }
                if (!reply.TryGetProperty("ok", out var ok) || !ok.GetBoolean())
                {
                    throw new InvalidOperationException(
                        reply.TryGetProperty("error", out var error) ? error.GetString() : "refused");
                }
                return reply.Clone();
            }
        }
        finally
        {
            _calls.Release();
        }
    }
}
