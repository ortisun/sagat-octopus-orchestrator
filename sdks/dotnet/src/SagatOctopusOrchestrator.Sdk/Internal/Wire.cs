using System.Globalization;
using System.Text;
using System.Text.Json;

namespace SagatOctopusOrchestrator.Sdk.Internal;

/// <summary>JSON on the wire: one place for how payloads are written, how instants are spelled and how numbers survive.</summary>
internal static class Wire
{
    internal static readonly JsonSerializerOptions DefaultOptions = new(JsonSerializerDefaults.Web)
    {
        Encoder = StrictJsonEncoder.Instance,
    };

    private static readonly JsonWriterOptions WriterOptions = new() { Encoder = StrictJsonEncoder.Instance };

    private const string UnwritableMessage =
        "the payload cannot be written as JSON (a number that is not finite, text that is not valid Unicode, or a type System.Text.Json cannot write)";

    internal static JsonSerializerOptions OptionsOf(SagatOptions options) => options.Json ?? DefaultOptions;

    /// <summary>Writes a JSON document with a <see cref="Utf8JsonWriter"/>; whatever cannot be written is <c>VALIDATION_FAILED</c>.</summary>
    internal static byte[] Write(Action<Utf8JsonWriter> write)
    {
        using var stream = new MemoryStream();
        try
        {
            using (var writer = new Utf8JsonWriter(stream, WriterOptions))
            {
                write(writer);
            }
        }
        catch (Exception error) when (error is JsonException or ArgumentException or InvalidOperationException or NotSupportedException)
        {
            throw new SagatException(SagatErrorCode.ValidationFailed, UnwritableMessage, innerException: null);
        }

        return stream.ToArray();
    }

    /// <summary>Writes a typed payload; a number is written as the shortest text that reads back as itself, a node as it was read.</summary>
    internal static void WritePayload<T>(Utf8JsonWriter writer, T value, JsonSerializerOptions options) =>
        JsonSerializer.Serialize(writer, value, options);

    /// <summary>The JSON text of a typed value, or <c>VALIDATION_FAILED</c> when it cannot be written.</summary>
    internal static string Serialize<T>(T value, JsonSerializerOptions options)
    {
        var bytes = Write(writer => WritePayload(writer, value, options));
        return Encoding.UTF8.GetString(bytes);
    }

    /// <summary>A string as a JSON literal, for messages (<c>"name"</c>).</summary>
    internal static string Quote(string? text)
    {
        try
        {
            return JsonSerializer.Serialize(text, DefaultOptions);
        }
        catch (ArgumentException)
        {
            // Text that is not valid Unicode cannot be quoted; the message must not fail because of it.
            return "\"<text that is not valid Unicode>\"";
        }
    }

    /// <summary>An RFC 3339 instant in UTC with a <c>Z</c>, as the SDK writes every instant.</summary>
    internal static string Instant(DateTimeOffset value) =>
        value.UtcDateTime.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'", CultureInfo.InvariantCulture);

    /// <summary>An instant the server wrote, with any offset, as an instant in UTC; false when it is not one.</summary>
    internal static bool TryParseInstant(string? text, out DateTimeOffset value)
    {
        value = default;
        return text is not null
            && DateTimeOffset.TryParse(text, CultureInfo.InvariantCulture, DateTimeStyles.AssumeUniversal | DateTimeStyles.AdjustToUniversal, out value);
    }
}
