using System.Text;
using SagatOctopusOrchestrator.Sdk.Internal;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>The canonical error codes of the SDKs (<c>sdks/SPEC.md</c> section 2).</summary>
public enum SagatErrorCode
{
    /// <summary>The request is malformed.</summary>
    BadRequest,

    /// <summary>The token was refused.</summary>
    Unauthenticated,

    /// <summary>The identity may not do this.</summary>
    Forbidden,

    /// <summary>The resource does not exist in this tenant and namespace.</summary>
    NotFound,

    /// <summary>The request conflicts with the current state.</summary>
    Conflict,

    /// <summary>A field is invalid.</summary>
    ValidationFailed,

    /// <summary>Too many requests.</summary>
    RateLimited,

    /// <summary>A quota is exhausted.</summary>
    QuotaExceeded,

    /// <summary>The service could not serve the request now.</summary>
    Unavailable,

    /// <summary>An unexpected error on the server.</summary>
    Internal,

    /// <summary>The lease is no longer current.</summary>
    StaleAttempt,

    /// <summary>The idempotency key was used for a different request.</summary>
    IdempotencyKeyReused,

    /// <summary>The worker identity is not bound to the namespace and activity.</summary>
    NotBound,

    /// <summary>A secret the step declares cannot be used.</summary>
    SecretNotUsable,

    /// <summary>The server does not speak this major version of the worker protocol.</summary>
    UnsupportedProtocol,

    /// <summary>No answer from the server (raised by the SDK).</summary>
    Transport,

    /// <summary>The call was cancelled (raised by the SDK).</summary>
    Cancelled,

    /// <summary>The server's answer did not match the contract (raised by the SDK).</summary>
    Protocol,

    /// <summary>The payload is over the limit (raised by the SDK, before it is sent).</summary>
    PayloadTooLarge,
}

/// <summary>The wire names and the catalog facts of <see cref="SagatErrorCode"/>.</summary>
public static class SagatErrorCodes
{
    private static readonly Dictionary<SagatErrorCode, ErrorCatalog.Entry> ByCode = BuildIndex();

    /// <summary>The stable machine name of a code, as the catalog and the conformance goldens spell it (<c>RATE_LIMITED</c>).</summary>
    public static string ToWireName(this SagatErrorCode code) => SnakeCase(code.ToString());

    /// <summary>Parses a wire name; false for a name this SDK does not know.</summary>
    public static bool TryParse(string? wireName, out SagatErrorCode code)
    {
        foreach (var candidate in Enum.GetValues<SagatErrorCode>())
        {
            if (string.Equals(candidate.ToWireName(), wireName, StringComparison.Ordinal))
            {
                code = candidate;
                return true;
            }
        }

        code = default;
        return false;
    }

    /// <summary>What to do about a code: the fixed sentence of the catalog.</summary>
    public static string Remediation(this SagatErrorCode code) =>
        ByCode.TryGetValue(code, out var entry) ? entry.Remediation : "Report the trace id to the operator.";

    /// <summary>The default <c>retryable</c> of a code, for an error the server did not classify itself.</summary>
    public static bool RetryableByDefault(this SagatErrorCode code) => ByCode.TryGetValue(code, out var entry) && entry.Retryable;

    /// <summary>The code the catalog gives an HTTP status when the body carried no envelope.</summary>
    public static SagatErrorCode FromHttpStatus(int status)
    {
        foreach (var entry in ErrorCatalog.Entries)
        {
            if (entry.Origin == "rest" && entry.Http.Contains(status) && TryParse(entry.Code, out var code))
            {
                return code;
            }
        }

        return status >= 500 ? SagatErrorCode.Internal : SagatErrorCode.BadRequest;
    }

    internal static string SnakeCase(string pascal)
    {
        var builder = new StringBuilder(pascal.Length + 4);
        for (var i = 0; i < pascal.Length; i++)
        {
            var c = pascal[i];
            if (i > 0 && char.IsUpper(c))
            {
                builder.Append('_');
            }

            builder.Append(char.ToUpperInvariant(c));
        }

        return builder.ToString();
    }

    private static Dictionary<SagatErrorCode, ErrorCatalog.Entry> BuildIndex()
    {
        var index = new Dictionary<SagatErrorCode, ErrorCatalog.Entry>();
        foreach (var entry in ErrorCatalog.Entries)
        {
            if (TryParse(entry.Code, out var code))
            {
                index[code] = entry;
            }
        }

        return index;
    }
}
