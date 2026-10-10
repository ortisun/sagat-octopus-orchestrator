package dev.sagatoctopusorchestrator.sdk.worker;

import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import io.grpc.Metadata;
import io.grpc.Status;
import io.grpc.StatusRuntimeException;
import java.util.Locale;
import java.util.regex.Pattern;

/** The mapping of a gRPC failure onto the canonical errors (SPEC section 2). */
final class GrpcErrors {
    static final Metadata.Key<String> SAGAT_ERROR = Metadata.Key.of("x-sagat-error", Metadata.ASCII_STRING_MARSHALLER);
    static final Metadata.Key<String> RETRY_AFTER_MS = Metadata.Key.of("retry-after-ms", Metadata.ASCII_STRING_MARSHALLER);
    private static final Pattern DIGITS = Pattern.compile("^\\d+$");
    private static final Pattern UNSUPPORTED = Pattern.compile("unsupported protocol", Pattern.CASE_INSENSITIVE);

    private GrpcErrors() {}

    static SagatException of(String method, StatusRuntimeException error) {
        Status status = error.getStatus();
        Metadata trailers = error.getTrailers();
        String reason = trailers == null ? null : trailers.get(SAGAT_ERROR);
        String retryAfter = trailers == null ? null : trailers.get(RETRY_AFTER_MS);
        Long retryAfterMs = retryAfter != null && DIGITS.matcher(retryAfter).matches() ? parse(retryAfter) : null;
        String name = status.getCode().name();

        switch (status.getCode()) {
            case UNAUTHENTICATED:
                return safe(ErrorCode.UNAUTHENTICATED, "the worker service refused the credentials or the session", name, reason, retryAfterMs);
            case PERMISSION_DENIED:
                if ("NOT_BOUND".equals(reason)) {
                    return safe(ErrorCode.NOT_BOUND, "the identity is not bound to the namespace and activity", name, reason, retryAfterMs);
                }
                if ("SECRET_NOT_USABLE".equals(reason)) {
                    return safe(ErrorCode.SECRET_NOT_USABLE, "a secret of the step cannot be used", name, reason, retryAfterMs);
                }
                return safe(ErrorCode.FORBIDDEN, "the identity may not use the worker service", name, reason, retryAfterMs);
            case INVALID_ARGUMENT:
                if ("UNSUPPORTED_PROTOCOL".equals(reason)
                        || ("RegisterWorker".equals(method) && status.getDescription() != null && UNSUPPORTED.matcher(status.getDescription()).find())) {
                    return safe(ErrorCode.UNSUPPORTED_PROTOCOL, "the server does not speak this protocol version", name, reason, retryAfterMs);
                }
                return safe(ErrorCode.VALIDATION_FAILED, "the worker service refused a field", name, reason, retryAfterMs);
            case ABORTED:
                if ("IDEMPOTENCY_KEY_REUSED".equals(reason)) {
                    return safe(ErrorCode.IDEMPOTENCY_KEY_REUSED, "the idempotency key was used for a different report", name, reason, retryAfterMs);
                }
                if (reason == null || "STALE_ATTEMPT".equals(reason)) {
                    return safe(ErrorCode.STALE_ATTEMPT, "the lease is no longer current", name, reason, retryAfterMs);
                }
                return safe(ErrorCode.CONFLICT, "the request conflicts with the state of the run", name, reason, retryAfterMs);
            case FAILED_PRECONDITION:
                return safe(ErrorCode.STALE_ATTEMPT, "the lease is no longer current", name, reason, retryAfterMs);
            case NOT_FOUND:
                return safe(ErrorCode.NOT_FOUND, "the worker service does not know it", name, reason, retryAfterMs);
            case RESOURCE_EXHAUSTED:
                return safe(
                        "QUOTA_EXCEEDED".equals(reason) ? ErrorCode.QUOTA_EXCEEDED : ErrorCode.RATE_LIMITED,
                        "the worker service asks for a slower pace",
                        name,
                        reason,
                        retryAfterMs);
            case UNAVAILABLE:
            case DEADLINE_EXCEEDED:
                // The server's own UNAVAILABLE (its storage) is told from the lack of an answer by its trailer.
                if ("UNAVAILABLE".equals(reason)) {
                    return safe(ErrorCode.UNAVAILABLE, "the worker service could not serve the call now", name, reason, retryAfterMs);
                }
                return SagatException.builder(ErrorCode.TRANSPORT, "no answer from the worker service")
                        .grpcStatus(name)
                        .reason(reason)
                        .retryAfterMs(retryAfterMs)
                        .cause(error)
                        .build();
            case CANCELLED:
                return SagatException.builder(ErrorCode.CANCELLED, "the call was cancelled").grpcStatus(name).cause(error).build();
            default:
                return safe(ErrorCode.INTERNAL, "the worker service failed", name, reason, retryAfterMs);
        }
    }

    private static Long parse(String digits) {
        try {
            return Long.parseLong(digits.toLowerCase(Locale.ROOT));
        } catch (NumberFormatException e) {
            return null;
        }
    }

    private static SagatException safe(ErrorCode code, String message, String status, String reason, Long retryAfterMs) {
        return SagatException.builder(code, message).grpcStatus(status).reason(reason).retryAfterMs(retryAfterMs).build();
    }
}
