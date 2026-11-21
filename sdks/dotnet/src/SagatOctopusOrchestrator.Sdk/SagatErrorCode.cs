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

    /// <summary>The table or the target ref does not exist.</summary>
    IcebergTableNotFound,

    /// <summary>The step cannot write this table (format version, file format, partition transform or key).</summary>
    IcebergUnsupportedTable,

    /// <summary>A table, data or metadata location is outside the connection's warehouse locations.</summary>
    IcebergLocationNotAllowed,

    /// <summary>A record does not map losslessly to the table schema.</summary>
    IcebergSchemaMismatch,

    /// <summary>The batch holds the same key twice.</summary>
    IcebergDuplicateKey,

    /// <summary>A row is outside the overwrite filter, or the batch is empty without allowEmpty.</summary>
    IcebergScopeViolation,

    /// <summary>Concurrent commits kept the write from landing.</summary>
    IcebergCommitConflict,

    /// <summary>A later commit changed what this step wrote, so it cannot be reverted automatically.</summary>
    IcebergRevertBlocked,

    /// <summary>The write is over a records, bytes, files or columns limit.</summary>
    IcebergLimitExceeded,

    /// <summary>The catalog could not be reached.</summary>
    CatalogUnavailable,

    /// <summary>The object storage could not be reached or is throttling.</summary>
    StorageUnavailable,

    /// <summary>The destination key has no primary key or unique constraint.</summary>
    DestinationKeyConstraintMissing,

    /// <summary>A destination column is missing or cannot hold a value.</summary>
    DestinationSchemaMismatch,

    /// <summary>The retry is outside the destination's deduplication window, so a duplicate cannot be ruled out.</summary>
    DestinationDeduplicationNotConfirmed,

    /// <summary>An object with this name and different content already exists.</summary>
    DestinationObjectConflict,

    /// <summary>The delivery is over a message size, count or bytes limit.</summary>
    DestinationDeliveryBoundExceeded,

    /// <summary>The subscription also carries run events.</summary>
    DestinationSubscriptionShared,

    /// <summary>The destination refused the credentials.</summary>
    DestinationAuthFailed,

    /// <summary>The destination could not be reached or is overloaded.</summary>
    DestinationUnavailable,

    /// <summary>The subscription is disabled, deleted or below a field's classification level.</summary>
    DestinationSubscriptionUnusable,

    /// <summary>A destination whose policy is fail did not complete.</summary>
    DestinationFailed,

    /// <summary>The sandboxed lake host broke its protocol.</summary>
    LakeHostProtocol,

    /// <summary>The sandbox for the attempt could not be started.</summary>
    LakeSandboxUnavailable,

    /// <summary>A field is above the connection's classification ceiling, or is a secret.</summary>
    ClassificationEgressRefused,

    /// <summary>Whether the write happened is unknown.</summary>
    OutcomeUnknown,
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
