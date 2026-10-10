using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>
/// Canonical JSON text (SPEC section 7, RFC 8785): UTF-8, keys sorted at every depth by UTF-16 code unit (ordinal comparison, never
/// culture), no insignificant whitespace, strings escaped as <c>JSON.stringify</c> does and numbers written as ECMAScript writes them
/// (<c>1.0</c> is <c>1</c>, <c>1e3</c> is <c>1000</c>, <c>600000</c> stays <c>600000</c>), so that the five SDKs write byte-identical text.
/// </summary>
public static class CanonicalJson
{
    /// <summary>
    /// The canonical text of a node. A number outside the range of an IEEE 754 double is
    /// <see cref="SagatErrorCode.ValidationFailed"/> (RFC 8785 has no spelling for it).
    /// </summary>
    public static string Write(JsonNode? node)
    {
        var builder = new StringBuilder();
        Append(builder, node);
        return builder.ToString();
    }

    private static void Append(StringBuilder builder, JsonNode? node)
    {
        switch (node)
        {
            case null:
                builder.Append("null");
                break;
            case JsonObject obj:
                builder.Append('{');
                var first = true;
                foreach (var key in obj.Select(pair => pair.Key).Order(StringComparer.Ordinal))
                {
                    if (!first)
                    {
                        builder.Append(',');
                    }

                    first = false;
                    AppendString(builder, key);
                    builder.Append(':');
                    Append(builder, obj[key]);
                }

                builder.Append('}');
                break;
            case JsonArray array:
                builder.Append('[');
                for (var i = 0; i < array.Count; i++)
                {
                    if (i > 0)
                    {
                        builder.Append(',');
                    }

                    Append(builder, array[i]);
                }

                builder.Append(']');
                break;
            case JsonValue value:
                AppendValue(builder, value);
                break;
            default:
                throw new ArgumentException("unknown JSON node", nameof(node));
        }
    }

    private static void AppendValue(StringBuilder builder, JsonValue value)
    {
        if (value.GetValueKind() == JsonValueKind.String)
        {
            AppendString(builder, value.GetValue<string>());
            return;
        }

        if (value.GetValueKind() == JsonValueKind.Number)
        {
            var number = double.Parse(value.ToJsonString(), NumberStyles.Float, CultureInfo.InvariantCulture);
            var text = EcmaScriptNumber(number);

            // An integer beyond 2^53 - 1 that ECMAScript prints in digits was rounded on the way in, and only TypeScript could print the nearest
            // double and call it exact: it is refused, so that the text is the same bytes in every language. One printed with an exponent
            // (1e21) is a number, not a rounded integer.
            if (Math.Abs(number) > 9007199254740991d && !text.Contains('e', StringComparison.Ordinal))
            {
                throw SagatException.Invalid("the document holds an integer beyond 2^53 - 1: write it as a string");
            }

            builder.Append(text);
            return;
        }

        builder.Append(value.ToJsonString());
    }

    /// <summary><c>Number.prototype.toString</c> of ECMAScript for a finite double: the shortest digits that read back, fixed notation for 1e-6 up to 1e21.</summary>
    internal static string EcmaScriptNumber(double value)
    {
        if (!double.IsFinite(value))
        {
            throw SagatException.Invalid("a number is outside the range of a double");
        }

        if (value == 0)
        {
            return "0";
        }

        var text = Math.Abs(value).ToString("R", CultureInfo.InvariantCulture);
        var exponent = 0;
        var e = text.IndexOf('E', StringComparison.Ordinal);
        if (e >= 0)
        {
            exponent = int.Parse(text[(e + 1)..], NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture);
            text = text[..e];
        }

        var dot = text.IndexOf('.', StringComparison.Ordinal);
        var integer = dot < 0 ? text : text[..dot];
        var digits = integer + (dot < 0 ? string.Empty : text[(dot + 1)..]);

        // value = 0.<digits> * 10^n, with no leading and no trailing zeros in digits.
        var n = integer.Length + exponent;
        var leading = digits.Length - digits.TrimStart('0').Length;
        digits = digits.TrimStart('0').TrimEnd('0');
        n -= leading;
        var k = digits.Length;

        var builder = new StringBuilder(value < 0 ? "-" : string.Empty);
        if (k <= n && n <= 21)
        {
            builder.Append(digits).Append('0', n - k);
        }
        else if (0 < n && n <= 21)
        {
            builder.Append(digits, 0, n).Append('.').Append(digits, n, k - n);
        }
        else if (-6 < n && n <= 0)
        {
            builder.Append("0.").Append('0', -n).Append(digits);
        }
        else
        {
            var power = n - 1;
            builder.Append(digits[0]);
            if (k > 1)
            {
                builder.Append('.').Append(digits, 1, k - 1);
            }

            builder.Append('e').Append(power < 0 ? '-' : '+').Append(Math.Abs(power).ToString(CultureInfo.InvariantCulture));
        }

        return builder.ToString();
    }

    private static void AppendString(StringBuilder builder, string text)
    {
        builder.Append('"');
        for (var i = 0; i < text.Length; i++)
        {
            var c = text[i];
            switch (c)
            {
                case '"':
                    builder.Append("\\\"");
                    break;
                case '\\':
                    builder.Append("\\\\");
                    break;
                case '\b':
                    builder.Append("\\b");
                    break;
                case '\f':
                    builder.Append("\\f");
                    break;
                case '\n':
                    builder.Append("\\n");
                    break;
                case '\r':
                    builder.Append("\\r");
                    break;
                case '\t':
                    builder.Append("\\t");
                    break;
                default:
                    if (c < 0x20)
                    {
                        builder.Append("\\u").Append(((int)c).ToString("x4", CultureInfo.InvariantCulture));
                    }
                    else if (char.IsHighSurrogate(c) && i + 1 < text.Length && char.IsLowSurrogate(text[i + 1]))
                    {
                        builder.Append(c).Append(text[++i]);
                    }
                    else if (char.IsSurrogate(c))
                    {
                        // A lone surrogate is not text (SPEC section 8): it is refused, not written escaped.
                        throw SagatException.Invalid("the document holds text that is not valid Unicode (a lone surrogate)");
                    }
                    else
                    {
                        builder.Append(c);
                    }

                    break;
            }
        }

        builder.Append('"');
    }
}
