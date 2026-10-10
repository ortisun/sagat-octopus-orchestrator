using System.Globalization;
using System.Security.Authentication;
using System.Text.RegularExpressions;
using Grpc.Core;

namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>The mapping of a gRPC failure onto the canonical errors (SPEC section 2). The trailer <c>x-sagat-error</c> wins when present.</summary>
internal static partial class GrpcErrors
{
    [GeneratedRegex("unsupported protocol", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
    private static partial Regex UnsupportedProtocolText();

    internal static SagatException Map(string method, RpcException error)
    {
        var reason = error.Trailers.GetValue("x-sagat-error");
        TimeSpan? retryAfter = long.TryParse(error.Trailers.GetValue("retry-after-ms"), NumberStyles.None, CultureInfo.InvariantCulture, out var ms)
            ? TimeSpan.FromMilliseconds(ms)
            : null;
        var statusName = SagatErrorCodes.SnakeCase(error.StatusCode.ToString());

        SagatException Safe(SagatErrorCode code, string message) =>
            new(code, message, grpcStatus: statusName, reason: reason, retryAfter: retryAfter);

        // The transport failed before any answer (a refused connection, a reset, a TLS handshake that did not verify): the status
        // the library gives it varies, the fact does not.
        if (reason is null && error.Status.DebugException is HttpRequestException or IOException or AuthenticationException)
        {
            return new SagatException(SagatErrorCode.Transport, "no answer from the worker service", grpcStatus: statusName);
        }

        switch (error.StatusCode)
        {
            case StatusCode.Unauthenticated:
                return Safe(SagatErrorCode.Unauthenticated, "the worker service refused the credentials or the session");
            case StatusCode.PermissionDenied:
                return reason switch
                {
                    "NOT_BOUND" => Safe(SagatErrorCode.NotBound, "the identity is not bound to the namespace and activity"),
                    "SECRET_NOT_USABLE" => Safe(SagatErrorCode.SecretNotUsable, "a secret of the step cannot be used"),
                    _ => Safe(SagatErrorCode.Forbidden, "the identity may not use the worker service"),
                };
            case StatusCode.InvalidArgument:
                if (reason == "UNSUPPORTED_PROTOCOL" || (method == "RegisterWorker" && UnsupportedProtocolText().IsMatch(error.Status.Detail)))
                {
                    return Safe(SagatErrorCode.UnsupportedProtocol, "the server does not speak this protocol version");
                }

                return Safe(SagatErrorCode.ValidationFailed, "the worker service refused a field");
            case StatusCode.Aborted:
                if (reason == "IDEMPOTENCY_KEY_REUSED")
                {
                    return Safe(SagatErrorCode.IdempotencyKeyReused, "the idempotency key was used for a different report");
                }

                if (reason is null or "STALE_ATTEMPT")
                {
                    return Safe(SagatErrorCode.StaleAttempt, "the lease is no longer current");
                }

                return Safe(SagatErrorCode.Conflict, "the request conflicts with the state of the run");
            case StatusCode.FailedPrecondition:
                return Safe(SagatErrorCode.StaleAttempt, "the lease is no longer current");
            case StatusCode.NotFound:
                return Safe(SagatErrorCode.NotFound, "the worker service does not know it");
            case StatusCode.ResourceExhausted:
                return reason == "QUOTA_EXCEEDED"
                    ? Safe(SagatErrorCode.QuotaExceeded, "the worker service asks for a slower pace")
                    : Safe(SagatErrorCode.RateLimited, "the worker service asks for a slower pace");
            case StatusCode.Unavailable:
            case StatusCode.DeadlineExceeded:
                // The server's own UNAVAILABLE (its storage) is told from the lack of an answer by its trailer.
                return reason == "UNAVAILABLE"
                    ? Safe(SagatErrorCode.Unavailable, "the worker service could not serve the call now")
                    : Safe(SagatErrorCode.Transport, "no answer from the worker service");
            case StatusCode.Cancelled:
                return new SagatException(SagatErrorCode.Cancelled, "the call was cancelled", grpcStatus: statusName);
            default:
                return Safe(SagatErrorCode.Internal, "the worker service failed");
        }
    }
}
