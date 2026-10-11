using System.Globalization;
using System.Text.RegularExpressions;

namespace SagatOctopusOrchestrator.Sdk.Internal;

/// <summary>What the server says is bounded (SPEC section 10): a wait, a body, a message, a reason, a trace id.</summary>
internal static partial class Limits
{
    /// <summary>The longest wait the SDK honours, whatever the server asks for (<c>Retry-After</c>, <c>retry-after-ms</c>, the pause of an empty poll).</summary>
    internal static readonly TimeSpan MaxServerWait = TimeSpan.FromSeconds(60);

    /// <summary>A REST body is read up to this many bytes; beyond them the call fails as <c>PROTOCOL</c>.</summary>
    internal const int MaxBodyBytes = 32 * 1024 * 1024;

    internal const int MaxMessage = 512;
    internal const int MaxReason = 128;

    [GeneratedRegex("^[0-9a-f]{32}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex TraceIdPattern();

    [GeneratedRegex("^[0-9]+\\z", RegexOptions.CultureInvariant)]
    private static partial Regex DigitsPattern();

    /// <summary>A wait of <paramref name="milliseconds"/>, at most <see cref="MaxServerWait"/>; never negative.</summary>
    internal static TimeSpan Capped(long milliseconds) =>
        milliseconds <= 0 ? TimeSpan.Zero : milliseconds >= (long)MaxServerWait.TotalMilliseconds ? MaxServerWait : TimeSpan.FromMilliseconds(milliseconds);

    internal static TimeSpan Capped(TimeSpan wait) => wait <= TimeSpan.Zero ? TimeSpan.Zero : wait > MaxServerWait ? MaxServerWait : wait;

    /// <summary>A header or trailer made only of digits, in milliseconds: capped at <see cref="MaxServerWait"/> however many digits it has. Anything else is no wait.</summary>
    internal static TimeSpan? MillisecondsOf(string? digits)
    {
        if (digits is null || !DigitsPattern().IsMatch(digits))
        {
            return null;
        }

        return digits.Length > 15 || !long.TryParse(digits, NumberStyles.None, CultureInfo.InvariantCulture, out var milliseconds) ? MaxServerWait : Capped(milliseconds);
    }

    /// <summary>A <c>Retry-After</c> in seconds, and in seconds only (SPEC section 8: an HTTP date is ignored), capped at <see cref="MaxServerWait"/>.</summary>
    internal static TimeSpan? SecondsOf(string? digits)
    {
        if (digits is null || !DigitsPattern().IsMatch(digits))
        {
            return null;
        }

        return digits.Length > 9 || !long.TryParse(digits, NumberStyles.None, CultureInfo.InvariantCulture, out var seconds) ? MaxServerWait : Capped(seconds * 1000);
    }

    /// <summary>The trace id of a server's answer: 32 lowercase hexadecimal characters, or empty.</summary>
    internal static string TraceId(string? candidate) => candidate is not null && TraceIdPattern().IsMatch(candidate) ? candidate : string.Empty;

    /// <summary>At most <paramref name="max"/> UTF-16 units, never ending inside a surrogate pair.</summary>
    internal static string Cut(string text, int max)
    {
        if (text.Length <= max)
        {
            return text;
        }

        var cut = text[..max];
        return char.IsHighSurrogate(cut[^1]) ? cut[..^1] : cut;
    }
}
