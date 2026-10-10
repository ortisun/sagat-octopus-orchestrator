using System.Security.Cryptography;
using System.Text.RegularExpressions;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>Helpers for the W3C <c>traceparent</c> header (<c>00-&lt;32 hex&gt;-&lt;16 hex&gt;-&lt;2 hex&gt;</c>).</summary>
public static partial class Traceparent
{
    [GeneratedRegex("^00-([0-9a-f]{32})-([0-9a-f]{16})-[0-9a-f]{2}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex Pattern();

    /// <summary>True for a version 00 trace context with a non-zero trace id and parent id.</summary>
    public static bool IsValid(string? value)
    {
        if (value is null)
        {
            return false;
        }

        var match = Pattern().Match(value);
        return match.Success
            && match.Groups[1].Value != new string('0', 32)
            && match.Groups[2].Value != new string('0', 16);
    }

    /// <summary>Returns the value, or raises <see cref="SagatErrorCode.BadRequest"/> for a malformed one, before it is sent.</summary>
    public static string Check(string value) =>
        IsValid(value)
            ? value
            : throw new SagatException(SagatErrorCode.BadRequest, "traceparent is not a W3C trace context (00-<32 hex>-<16 hex>-<2 hex>)");

    /// <summary>The trace id of a valid <c>traceparent</c>; null otherwise.</summary>
    public static string? TraceIdOf(string? value)
    {
        return IsValid(value) ? Pattern().Match(value!).Groups[1].Value : null;
    }

    /// <summary>A fresh sampled trace context, for an application that has none.</summary>
    public static string New() =>
        $"00-{Convert.ToHexString(RandomNumberGenerator.GetBytes(16)).ToLowerInvariant()}-{Convert.ToHexString(RandomNumberGenerator.GetBytes(8)).ToLowerInvariant()}-01";
}
