using SagatOctopusOrchestrator.Sdk.Internal;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>
/// The one error an SDK call raises (<c>sdks/SPEC.md</c> section 2). <see cref="Exception.Message"/> is safe to log: it
/// never holds a payload, a token or a secret; <see cref="Remediation"/> says what to do about <see cref="Code"/>.
/// </summary>
/// <remarks>
/// A call cancelled by the caller's own <see cref="CancellationToken"/> raises <see cref="OperationCanceledException"/>,
/// as every .NET API does, and not a <see cref="SagatException"/> (the catalog's <c>CANCELLED</c> is for a call the server
/// or the transport cancelled).
/// </remarks>
#pragma warning disable CA1032 // Not the standard exception constructors: an error always has a code.
public sealed class SagatException : Exception
#pragma warning restore CA1032
{
    /// <summary>Creates the error. <paramref name="retryable"/> defaults to the catalog's value for the code.</summary>
    public SagatException(
        SagatErrorCode code,
        string message,
        int? httpStatus = null,
        string? grpcStatus = null,
        bool? retryable = null,
        string? traceId = null,
        string? reason = null,
        TimeSpan? retryAfter = null,
        Exception? innerException = null)
        : base(Limits.Cut(message, Limits.MaxMessage), innerException)
    {
        Code = code;
        HttpStatus = httpStatus;
        GrpcStatus = grpcStatus;
        Reason = reason is null ? null : Limits.Cut(reason, Limits.MaxReason);

        // The rule is by code (SPEC section 5): only these four are ever repeated, and only when they say they may be. A body that claims
        // `retryable: true` on any other code is not believed. (A token provider that failed is the one exception: it is asked again.)
        var eligible = code is SagatErrorCode.Transport or SagatErrorCode.Unavailable or SagatErrorCode.RateLimited or SagatErrorCode.Internal
            || Reason == TokenProviderFailed;
        Retryable = eligible && (Reason == TokenProviderFailed || (retryable ?? code.RetryableByDefault()));
        TraceId = Limits.TraceId(traceId);
        RetryAfter = retryAfter is { } wait ? Limits.Capped(wait) : null;
        Remediation = code.Remediation();
    }

    /// <summary>The reason of the error raised when the token provider of the caller fails.</summary>
    internal const string TokenProviderFailed = "TOKEN_PROVIDER_FAILED";

    /// <summary>A stable machine code from the catalog.</summary>
    public SagatErrorCode Code { get; }

    /// <summary>The HTTP status; null when the failure was not an HTTP answer.</summary>
    public int? HttpStatus { get; }

    /// <summary>The gRPC status name (<c>PERMISSION_DENIED</c>); null when the failure was not a gRPC answer.</summary>
    public string? GrpcStatus { get; }

    /// <summary>
    /// Whether the same call may be made again as it is: only <c>TRANSPORT</c>, <c>UNAVAILABLE</c>, <c>RATE_LIMITED</c> and <c>INTERNAL</c> ever are, and
    /// <c>INTERNAL</c> only when the server said so.
    /// </summary>
    public bool Retryable { get; }

    /// <summary>32 lowercase hex characters from <c>X-Trace-Id</c> / the error envelope; empty when the server did not answer.</summary>
    public string TraceId { get; }

    /// <summary>The server's stable sub-reason (<c>details.reason</c>, or the gRPC <c>x-sagat-error</c> value), if any.</summary>
    public string? Reason { get; }

    /// <summary>From <c>Retry-After</c> / <c>retry-after-ms</c>, if the server sent it; never more than 60 s, whatever it said.</summary>
    public TimeSpan? RetryAfter { get; }

    /// <summary>What to do about it: the fixed sentence of the catalog for <see cref="Code"/>.</summary>
    public string Remediation { get; }

    internal static SagatException Invalid(string message) => new(SagatErrorCode.ValidationFailed, message);
}
