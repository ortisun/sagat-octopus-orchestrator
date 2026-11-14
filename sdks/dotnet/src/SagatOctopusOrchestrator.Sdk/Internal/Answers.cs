using System.Text.Json;

namespace SagatOctopusOrchestrator.Sdk.Internal;

/// <summary>
/// The typed reads of an answer. An answer that does not have the shape the contract promises is a <c>PROTOCOL</c> error, never a guess;
/// a field that is optional in the contract is <c>null</c> when it is absent, <c>null</c> or of another type.
/// </summary>
internal static class Answers
{
    internal static JsonElement ObjectOf(JsonElement? value, string what) =>
        value is { ValueKind: JsonValueKind.Object } element
            ? element
            : throw new SagatException(SagatErrorCode.Protocol, $"{what} is not an object");

    internal static string Str(JsonElement raw, string key) =>
        raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()!
            : throw new SagatException(SagatErrorCode.Protocol, $"{key} is missing from the answer");

    internal static string? OptStr(JsonElement raw, string key) =>
        raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.String ? value.GetString() : null;

    internal static DateTimeOffset? OptDate(JsonElement raw, string key) =>
        Wire.TryParseInstant(OptStr(raw, key), out var value) ? value : null;

    /// <summary>An instant the contract says is always there.</summary>
    internal static DateTimeOffset Date(JsonElement raw, string key) =>
        OptDate(raw, key) ?? throw new SagatException(SagatErrorCode.Protocol, $"{key} is missing from the answer");

    /// <summary>A number, or <paramref name="fallback"/> when the field is absent or is not a number.</summary>
    internal static long Num(JsonElement raw, string key, long fallback = 0) =>
        raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.Number
            ? (value.TryGetInt64(out var integer) ? integer : (long)value.GetDouble())
            : fallback;

    /// <summary>A number, or <c>null</c> when the field is absent, <c>null</c> or is not a number.</summary>
    internal static long? OptNum(JsonElement raw, string key) =>
        raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.Number
            ? (value.TryGetInt64(out var integer) ? integer : (long)value.GetDouble())
            : null;

    /// <summary>An integer that is at least <paramref name="minimum"/>; anything else is <c>PROTOCOL</c>.</summary>
    internal static long Revision(JsonElement raw, string key, long minimum) =>
        raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.Number
            && value.TryGetInt64(out var revision) && revision >= minimum
                ? revision
                : throw new SagatException(SagatErrorCode.Protocol, $"{key} is invalid in the answer");

    internal static bool Flag(JsonElement raw, string key) =>
        raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.True;

    /// <summary>The members of an array, read one by one; an absent field or another type is an empty list.</summary>
    internal static List<T> ListOf<T>(JsonElement raw, string key, Func<JsonElement, T> read) =>
        raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.Array
            ? [.. value.EnumerateArray().Select(read)]
            : [];

    /// <summary>The texts of an array (any other member is kept as its JSON text).</summary>
    internal static List<string> Strings(JsonElement raw, string key) =>
        ListOf(raw, key, item => item.ValueKind == JsonValueKind.String ? item.GetString()! : item.GetRawText());

    /// <summary>The members of an array as the JSON they came as.</summary>
    internal static List<JsonElement> Elements(JsonElement raw, string key) =>
        ListOf(raw, key, item => item.Clone());

    /// <summary>An object of numbers (a counter by name); a member that is not a number is left out.</summary>
    internal static Dictionary<string, long> Counters(JsonElement raw, string key)
    {
        var counters = new Dictionary<string, long>(StringComparer.Ordinal);
        if (raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.Object)
        {
            foreach (var member in value.EnumerateObject())
            {
                if (member.Value.ValueKind == JsonValueKind.Number)
                {
                    counters[member.Name] = member.Value.TryGetInt64(out var integer) ? integer : (long)member.Value.GetDouble();
                }
            }
        }

        return counters;
    }

    /// <summary>A page: <c>items</c> read one by one, and the opaque <c>nextCursor</c> when there is one.</summary>
    internal static Page<T> PageOf<T>(JsonElement? body, string what, Func<JsonElement, T> read)
    {
        var raw = ObjectOf(body, what);
        if (!raw.TryGetProperty("items", out var items) || items.ValueKind != JsonValueKind.Array)
        {
            throw new SagatException(SagatErrorCode.Protocol, "items is missing from the page");
        }

        return new Page<T>([.. items.EnumerateArray().Select(read)], OptStr(raw, "nextCursor"));
    }
}
