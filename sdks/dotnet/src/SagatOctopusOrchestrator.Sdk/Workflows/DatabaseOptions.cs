using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;

namespace SagatOctopusOrchestrator.Sdk.Workflows;

/// <summary>A scoped logical connection, never a DSN or provider path.</summary>
public sealed record ConnectionRef(string Name, int? Revision = null);

public enum DatabaseOperation { Query, Execute }

#pragma warning disable CA1720 // These names are the canonical SQL binding tags shared by all SDKs.
public enum DatabaseType { Null, Boolean, Integer, Decimal, Float, Text, Timestamp, Date, Binary, Json }
#pragma warning restore CA1720

/// <summary>An explicit SQL literal or expression binding. Integers and decimals are exact strings.</summary>
public sealed class DatabaseParameter
{
    private readonly DatabaseType type;
    private readonly object? value;
    private readonly bool expression;

    private DatabaseParameter(DatabaseType type, object? value, bool expression)
    {
        this.type = type;
        this.value = value;
        this.expression = expression;
    }

    public static DatabaseParameter Literal(DatabaseType type, object? value) => new(type, value, false);

    public static DatabaseParameter Expression(DatabaseType type, string expression) => new(type, expression, true);

    public override string ToString() => "[database parameter]";

    internal JsonObject ToNode()
    {
        if (!Enum.IsDefined(type))
        {
            throw DatabaseOptions.Invalid();
        }

        var name = type.ToString().ToLowerInvariant();
        if (expression)
        {
            return value is string { Length: > 0 } text
                ? new JsonObject { ["type"] = name, ["expression"] = text }
                : throw DatabaseOptions.Invalid();
        }

        var node = value is JsonNode json ? json.DeepClone() : JsonSerializer.SerializeToNode(value);
        var valid = type switch
        {
            DatabaseType.Null => node is null,
            DatabaseType.Boolean => node?.GetValueKind() is JsonValueKind.True or JsonValueKind.False,
            DatabaseType.Integer => Text(node) is { Length: <= 40 } integer && DatabaseOptions.IntegerPattern().IsMatch(integer),
            DatabaseType.Decimal => Text(node) is { Length: <= 16384 } number && DatabaseOptions.DecimalPattern().IsMatch(number),
            DatabaseType.Float => node?.GetValueKind() == JsonValueKind.Number && double.IsFinite(node.Deserialize<double>()),
            DatabaseType.Text => Text(node) is { } text && !text.Contains('\0', StringComparison.Ordinal),
            DatabaseType.Timestamp or DatabaseType.Date or DatabaseType.Binary => Text(node) is not null,
            DatabaseType.Json => true,
            _ => false,
        };
        if (!valid)
        {
            throw DatabaseOptions.Invalid();
        }

        return new JsonObject { ["literal"] = new JsonObject { ["type"] = name, ["value"] = node } };
    }

    private static string? Text(JsonNode? node) => node?.GetValueKind() == JsonValueKind.String ? node.GetValue<string>() : null;
}

/// <summary>Managed database authoring. SQL validation, authorization and retry semantics belong to the server.</summary>
public sealed partial record DatabaseOptions
{
    public required ConnectionRef ConnectionRef { get; init; }

    public required DatabaseOperation Operation { get; init; }

    public required string Statement { get; init; }

    public IReadOnlyList<DatabaseParameter>? Parameters { get; init; }

    /// <summary>Optional tighter limits, keyed by the canonical IR field names.</summary>
    public IReadOnlyDictionary<string, int>? Limits { get; init; }

    public TimeSpan? Timeout { get; init; }

    public string? OnError { get; init; }

    private static readonly Dictionary<string, int> Ceilings = new Dictionary<string, int>(StringComparer.Ordinal)
    {
        ["maxStatementBytes"] = 65536,
        ["maxParameters"] = 256,
        ["maxParameterBytes"] = 262144,
        ["maxRows"] = 10000,
        ["maxResultBytes"] = 4194304,
        ["timeoutMs"] = 30000,
        ["connectTimeoutMs"] = 10000,
        ["poolSize"] = 16,
        ["idleTimeoutMs"] = 60000,
    };

    [GeneratedRegex("^[a-z][a-z0-9-]{1,61}[a-z0-9]\\z", RegexOptions.CultureInvariant)]
    private static partial Regex NamePattern();

    [GeneratedRegex("^-?[0-9]+\\z", RegexOptions.CultureInvariant)]
    internal static partial Regex IntegerPattern();

    [GeneratedRegex("^-?[0-9]+(?:\\.[0-9]+)?\\z", RegexOptions.CultureInvariant)]
    internal static partial Regex DecimalPattern();

    public override string ToString() => "[managed database operation]";

    internal static SagatException Invalid() => SagatException.Invalid("invalid managed database operation");

    internal JsonObject ToNode(string id)
    {
        try
        {
            if (ConnectionRef is null || !NamePattern().IsMatch(ConnectionRef.Name ?? string.Empty)
                || ConnectionRef.Name!.Contains("--", StringComparison.Ordinal) || ConnectionRef.Revision is < 1
                || !Enum.IsDefined(Operation) || string.IsNullOrWhiteSpace(Statement) || Statement.Contains('\0', StringComparison.Ordinal)
                || Encoding.UTF8.GetByteCount(Statement) > 65536 || Parameters?.Count > 256 || OnError is { Length: 0 })
            {
                throw Invalid();
            }

            var reference = new JsonObject { ["name"] = ConnectionRef.Name };
            if (ConnectionRef.Revision is { } revision)
            {
                reference["revision"] = revision;
            }

            var node = new JsonObject
            {
                ["id"] = id,
                ["type"] = "database",
                ["connectionRef"] = reference,
                ["operation"] = Operation.ToString().ToLowerInvariant(),
                ["statement"] = Statement,
            };
            if (Parameters is { } parameters)
            {
                node["parameters"] = new JsonArray([.. parameters.Select(p => (JsonNode)(p?.ToNode() ?? throw Invalid()))]);
                if (Encoding.UTF8.GetByteCount(CanonicalJson.Write(node["parameters"])) > 262144)
                {
                    throw Invalid();
                }
            }

            if (Limits is { } limits)
            {
                var bounded = new JsonObject();
                foreach (var (key, value) in limits)
                {
                    if (!Ceilings.TryGetValue(key, out var maximum) || value < 1 || value > maximum)
                    {
                        throw Invalid();
                    }

                    bounded[key] = value;
                }

                node["limits"] = bounded;
            }

            if (Timeout is { } timeout)
            {
                if (timeout.Ticks % TimeSpan.TicksPerMillisecond != 0 || timeout.TotalMilliseconds is < 1 or > 60000)
                {
                    throw Invalid();
                }

                node["timeoutMs"] = timeout.Ticks / TimeSpan.TicksPerMillisecond;
            }

            if (OnError is { } onError)
            {
                node["onError"] = onError;
            }

            return JsonNode.Parse(CanonicalJson.Write(node))!.AsObject();
        }
        catch (Exception error) when (error is SagatException or JsonException or ArgumentException or InvalidOperationException or NotSupportedException)
        {
            // Do not retain an inner exception that may quote a SQL literal or a JSON property.
            throw Invalid();
        }
    }
}
