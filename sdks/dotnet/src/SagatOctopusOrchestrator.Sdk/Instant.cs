using System.Globalization;
using System.Text.RegularExpressions;
using SagatOctopusOrchestrator.Sdk.Internal;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>
/// An instant the SDK sends (the <c>StartAt</c> of a schedule, the range of a backfill, the bounds of a batch filter). It is given as a
/// <see cref="DateTimeOffset"/> (converted implicitly) or as RFC 3339 text (<see cref="Parse"/>), and it is always sent in UTC with a <c>Z</c>: the
/// offset of the machine plays no part. Text that is not an instant (no date, no time, no offset) is refused by <see cref="Parse"/> with
/// <c>VALIDATION_FAILED</c>, before anything is sent.
/// </summary>
/// <param name="Value">The instant.</param>
public readonly partial record struct Instant(DateTimeOffset Value)
{
    [GeneratedRegex(@"^\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d{1,9})?([Zz]|[+-]\d{2}:\d{2})\z", RegexOptions.CultureInvariant)]
    private static partial Regex Rfc3339();

    /// <summary>Reads RFC 3339 text (<c>2026-10-10T10:00:00Z</c>, <c>2026-10-10T12:00:00+02:00</c>); anything else is <c>VALIDATION_FAILED</c>.</summary>
    public static Instant Parse(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        if (Rfc3339().IsMatch(text) && DateTimeOffset.TryParse(text, CultureInfo.InvariantCulture, DateTimeStyles.None, out var value))
        {
            return new Instant(value);
        }

        throw SagatException.Invalid("an instant is RFC 3339 text with a date, a time and an offset");
    }

    /// <summary>The text the SDK sends: UTC, with a <c>Z</c>, to the millisecond.</summary>
    public override string ToString() => Wire.Instant(Value);

    /// <summary>An instant given as a <see cref="DateTimeOffset"/>.</summary>
    public static implicit operator Instant(DateTimeOffset value) => new(value);

    /// <summary>An instant given as a <see cref="DateTimeOffset"/>.</summary>
    public static Instant FromDateTimeOffset(DateTimeOffset value) => new(value);
}
