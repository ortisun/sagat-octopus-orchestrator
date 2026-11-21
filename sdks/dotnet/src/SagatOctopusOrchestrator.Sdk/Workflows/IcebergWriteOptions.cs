using System.Diagnostics.CodeAnalysis;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace SagatOctopusOrchestrator.Sdk.Workflows;

/// <summary>A table of an Iceberg catalog: a namespace of 1 to 8 levels and a name.</summary>
public sealed record IcebergTable(IReadOnlyList<string> Namespace, string Name);

/// <summary>A field of a record (a JSON pointer, <c>/customer/name</c>) and the column it is written to.</summary>
public sealed record IcebergMapping(string Field, string Column);

/// <summary>The undo of a write: removing its commit.</summary>
public enum IcebergCompensation { Revert }

/// <summary>Optional tighter limits on one write; an empty set is kept as an empty object.</summary>
public sealed record IcebergLimits
{
    public long? MaxRecordBytes { get; init; }

    public int? MaxRecords { get; init; }

    public TimeSpan? Timeout { get; init; }
}

/// <summary>
/// Where the records of the write come from: exactly one of an expression that evaluates to a list of objects, one that
/// evaluates to 1 to 32 blob descriptors, or the id of an earlier root-level database step.
/// </summary>
public sealed class IcebergRecords
{
    private IcebergRecords(string source, string? value)
    {
        Source = source;
        Value = value;
    }

    internal string Source { get; }

    internal string? Value { get; }

    public static IcebergRecords Inline(string expression) => new("inline", expression);

    public static IcebergRecords Blobs(string expression) => new("blobs", expression);

    public static IcebergRecords DatabaseResult(string stepId) => new("databaseResult", stepId);

    public override string ToString() => "[iceberg records]";
}

/// <summary>
/// An append to an Iceberg table through the isolated Sagat lake runner. Only a scoped logical connection name and
/// revision appear in a workflow: no catalog, bucket, credential or secret. The mode is append; overwrite and upsert are
/// not built yet. Shape validation only: the catalog, access, modes and classification belong to the server.
/// </summary>
public sealed record IcebergWriteOptions
{
    public required ConnectionRef ConnectionRef { get; init; }

    public required IcebergTable Table { get; init; }

    public required IcebergRecords Records { get; init; }

    /// <summary>The branch or tag to commit to; <c>main</c> when absent.</summary>
    public string? Ref { get; init; }

    public IReadOnlyList<IcebergMapping>? Mapping { get; init; }

    public bool? IgnoreUnknownFields { get; init; }

    public IcebergLimits? Limits { get; init; }

    public IcebergCompensation? Compensation { get; init; }

    public string? OnError { get; init; }

    public override string ToString() => "[iceberg write]";

    internal static SagatException Invalid() => SagatException.Invalid("invalid iceberg write");

    /// <summary>1 to <paramref name="limit"/> bytes of UTF-8 without a control character: the platform's plain text.</summary>
    private static bool Plain([NotNullWhen(true)] string? text, int limit)
        => !string.IsNullOrEmpty(text) && Encoding.UTF8.GetByteCount(text) <= limit && !text.EnumerateRunes().Any(Rune.IsControl);

    /// <summary>A level or a name of a table: plain, and not a path component of its own.</summary>
    private static bool Part([NotNullWhen(true)] string? text) => Plain(text, 255) && text is not ("." or "..");

    /// <summary>A JSON pointer into a record whose escapes are <c>~0</c> and <c>~1</c>, of at most 16 segments.</summary>
    private static bool Pointer([NotNullWhen(true)] string? text)
    {
        if (!Plain(text, 512) || text[0] != '/')
        {
            return false;
        }

        var segments = text[1..].Split('/');
        if (segments.Length > 16 || segments.Any(segment => segment.Length == 0))
        {
            return false;
        }

        for (var i = 0; i < text.Length; i++)
        {
            if (text[i] == '~' && (i + 1 >= text.Length || text[i + 1] is not ('0' or '1')))
            {
                return false;
            }
        }

        return true;
    }

    private static void Bounded(long? value, long low, long high)
    {
        if (value is { } number && (number < low || number > high))
        {
            throw Invalid();
        }
    }

    internal JsonObject ToNode(string id)
    {
        try
        {
            if (!DatabaseOptions.ValidConnectionRef(ConnectionRef) || Table is null || Records is null)
            {
                throw Invalid();
            }

            if (Table.Namespace is not { Count: >= 1 and <= 8 } levels || !Part(Table.Name) || !levels.All(Part))
            {
                throw Invalid();
            }

            if (Ref is not null && !Plain(Ref, 255))
            {
                throw Invalid();
            }

            if (Records.Source == "databaseResult")
            {
                if (!Plain(Records.Value, 64))
                {
                    throw Invalid();
                }
            }
            else if (Records.Value is null || Encoding.UTF8.GetByteCount(Records.Value) is < 1 or > 1024)
            {
                // an expression of 1 to 1024 UTF-8 bytes, as everywhere else: the schema says 4096, the engine takes 1024
                throw Invalid();
            }

            if (Mapping?.Count > 1024 || OnError is { Length: 0 })
            {
                throw Invalid();
            }

            if (Limits is { } limits)
            {
                Bounded(limits.MaxRecordBytes, 1024, 2147483648L);
                Bounded(limits.MaxRecords, 1, 100000000L);
                if (limits.Timeout is { } timeout && (timeout.Ticks % TimeSpan.TicksPerMillisecond != 0 || timeout.TotalMilliseconds is < 1000 or > 3600000))
                {
                    throw Invalid();
                }
            }

            var reference = new JsonObject { ["name"] = ConnectionRef.Name };
            if (ConnectionRef.Revision is { } revision)
            {
                reference["revision"] = revision;
            }

            var node = new JsonObject
            {
                ["id"] = id,
                ["type"] = "icebergWrite",
                ["mode"] = "append",
                ["connectionRef"] = reference,
                ["table"] = new JsonObject { ["namespace"] = new JsonArray([.. levels.Select(level => (JsonNode)level)]), ["name"] = Table.Name },
                ["records"] = new JsonObject { [Records.Source] = Records.Value },
            };
            if (Ref is not null)
            {
                node["ref"] = Ref;
            }

            if (Mapping is { } mapping)
            {
                // a field or a column twice is two rows of the mapping that disagree about one thing
                var fields = new HashSet<string>(StringComparer.Ordinal);
                var columns = new HashSet<string>(StringComparer.Ordinal);
                var rows = new JsonArray();
                foreach (var pair in mapping)
                {
                    if (pair is null || !Pointer(pair.Field) || !Plain(pair.Column, 255) || !fields.Add(pair.Field) || !columns.Add(pair.Column))
                    {
                        throw Invalid();
                    }

                    rows.Add(new JsonObject { ["field"] = pair.Field, ["column"] = pair.Column });
                }

                node["mapping"] = rows;
            }

            if (IgnoreUnknownFields is { } ignore)
            {
                node["ignoreUnknownFields"] = ignore;
            }

            if (Limits is { } bounds)
            {
                var bounded = new JsonObject();
                if (bounds.MaxRecordBytes is { } maxRecordBytes)
                {
                    bounded["maxRecordBytes"] = maxRecordBytes;
                }

                if (bounds.MaxRecords is { } maxRecords)
                {
                    bounded["maxRecords"] = maxRecords;
                }

                if (bounds.Timeout is { } timeout)
                {
                    bounded["timeoutMs"] = timeout.Ticks / TimeSpan.TicksPerMillisecond;
                }

                node["limits"] = bounded;
            }

            if (Compensation is not null)
            {
                node["compensation"] = "revert";
            }

            if (OnError is { } onError)
            {
                node["onError"] = onError;
            }

            // Snapshot all caller-owned values, and reject non-canonical ones consistently across SDKs.
            return JsonNode.Parse(CanonicalJson.Write(node))!.AsObject();
        }
        catch (Exception error) when (error is SagatException or JsonException or ArgumentException or InvalidOperationException or NotSupportedException)
        {
            // Do not retain an inner exception that may quote a name or an expression.
            throw Invalid();
        }
    }
}
