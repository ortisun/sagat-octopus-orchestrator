using System.Globalization;
using System.Text.RegularExpressions;

namespace SagatOctopusOrchestrator.Sdk.Internal;

/// <summary>What the client modules check before they send anything, and how they spell a path and a query.</summary>
internal static partial class Requests
{
    [GeneratedRegex("^[\\x21-\\x7e]{1,200}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex BusinessIdPattern();

    /// <summary>An id that becomes a path segment (a namespace, a schedule, a backfill, a batch job, a reset): a UUID, else <c>VALIDATION_FAILED</c> and nothing is sent.</summary>
    internal static string Uuid(string? value, string what) =>
        value is not null && SagatClient.UuidPattern().IsMatch(value)
            ? value.ToLowerInvariant()
            : throw SagatException.Invalid($"{what} is a UUID");

    internal static string Run(string? runId) => Uuid(runId, "a run id");

    /// <summary>
    /// A business identifier as a path segment, percent-encoded and otherwise as given: no trimming, no case folding. Only what cannot be a
    /// path segment at all (not 1 to 200 visible ASCII characters, or a dot segment that a URL would resolve to another route) is refused here.
    /// </summary>
    internal static string BusinessIdSegment(string? businessId) =>
        businessId is not null && BusinessIdPattern().IsMatch(businessId) && businessId is not ("." or "..")
            ? Uri.EscapeDataString(businessId)
            : throw SagatException.Invalid("a business identifier is 1 to 200 visible ASCII characters");

    /// <summary>An integer expected revision: at least 1, else <c>VALIDATION_FAILED</c>.</summary>
    internal static long ExpectedRevision(long value) =>
        value >= 1 ? value : throw SagatException.Invalid("expectedRevision is a positive integer");
}

/// <summary>The query string of a call, in the order the parts were added; a part without a value is left out.</summary>
internal sealed class QueryBuilder
{
    private readonly List<KeyValuePair<string, string>> parts = [];

    internal QueryBuilder Add(string name, string? value)
    {
        if (value is not null)
        {
            parts.Add(new(name, value));
        }

        return this;
    }

    internal QueryBuilder Add(string name, long? value) =>
        Add(name, value?.ToString(CultureInfo.InvariantCulture));

    internal IReadOnlyList<KeyValuePair<string, string>> Build() => parts;
}
