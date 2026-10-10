package dev.sagatoctopusorchestrator.sdk;

import java.time.Duration;
import java.util.Objects;
import java.util.Optional;
import java.util.OptionalInt;
import java.util.OptionalLong;

/**
 * The one error an SDK raises (sdks/SPEC.md section 2). {@link #getMessage()} is safe to log: it never holds a
 * payload, a token or a secret. {@link #remediation()} says what to do about {@link #code()}.
 */
public class SagatException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    private final ErrorCode code;
    private final Integer httpStatus;
    private final String grpcStatus;
    private final boolean retryable;
    private final String traceId;
    private final String reason;
    private final Long retryAfterMs;

    /** The longest wait the SDK takes from what a server says ({@code Retry-After}, {@code retry-after-ms}). */
    public static final long MAX_SERVER_WAIT_MS = 60_000;

    private static final java.util.regex.Pattern TRACE_ID = java.util.regex.Pattern.compile("^[0-9a-f]{32}$");

    // What a server says is bounded (SPEC section 10): its words are cut (to 512 characters, a reason to 128), a trace
    // id that is not 32 lower case hexadecimal characters is not repeated, and no wait it asks for is longer than a minute.
    private SagatException(Builder builder) {
        super(Text.cut(builder.message, 512), builder.cause);
        this.code = builder.code;
        this.httpStatus = builder.httpStatus;
        this.grpcStatus = builder.grpcStatus;
        this.retryable = builder.retryable != null ? builder.retryable : builder.code.retryableByDefault();
        this.traceId = builder.traceId != null && TRACE_ID.matcher(builder.traceId).matches() ? builder.traceId : "";
        this.reason = builder.reason == null ? null : Text.cut(builder.reason, 128);
        this.retryAfterMs = builder.retryAfterMs == null ? null : Math.max(0, Math.min(builder.retryAfterMs, MAX_SERVER_WAIT_MS));
    }

    /** Starts an error with a code and a safe message. */
    public static Builder builder(ErrorCode code, String message) {
        return new Builder(code, message);
    }

    /** An error with only a code and a message. */
    public static SagatException of(ErrorCode code, String message) {
        return new Builder(code, message).build();
    }

    /** A failure the SDK raises about its own input, before anything is sent. */
    public static SagatException invalid(String message) {
        return of(ErrorCode.VALIDATION_FAILED, message);
    }

    /** A stable machine code from the catalog. */
    public ErrorCode code() {
        return code;
    }

    /** The HTTP status of the answer; empty for a failure before any answer and for a gRPC failure. */
    public OptionalInt httpStatus() {
        return httpStatus == null ? OptionalInt.empty() : OptionalInt.of(httpStatus);
    }

    /** The gRPC status name ({@code UNAVAILABLE}); empty for a REST failure and for a failure before any answer. */
    public Optional<String> grpcStatus() {
        return Optional.ofNullable(grpcStatus);
    }

    /** The HTTP status as text, or the gRPC status name; empty before any answer. */
    public Optional<String> status() {
        return httpStatus != null ? Optional.of(Integer.toString(httpStatus)) : Optional.ofNullable(grpcStatus);
    }

    /** Whether the same call may be made again as it is. From the server's own {@code retryable} when it sent one. */
    public boolean retryable() {
        return retryable;
    }

    /**
     * Whether the SDK may repeat the call that raised this error (SPEC section 5): the error says it may be repeated
     * <em>and</em> it is one of {@code TRANSPORT}, {@code UNAVAILABLE}, {@code RATE_LIMITED} or {@code INTERNAL}. A
     * {@code 4xx} other than {@code 429} is never repeated, whatever the body of the answer claims.
     */
    public boolean isRetriable() {
        return retryable && (code == ErrorCode.TRANSPORT || code == ErrorCode.UNAVAILABLE || code == ErrorCode.RATE_LIMITED || code == ErrorCode.INTERNAL);
    }

    /** 32 lowercase hex characters from {@code X-Trace-Id} or the error envelope; empty when the server did not answer. */
    public String traceId() {
        return traceId;
    }

    /** The server's stable sub-reason ({@code details.reason}, or the gRPC {@code x-sagat-error} value). */
    public Optional<String> reason() {
        return Optional.ofNullable(reason);
    }

    /** From {@code Retry-After} / {@code retry-after-ms}, in milliseconds, if the server sent one. */
    public OptionalLong retryAfterMs() {
        return retryAfterMs == null ? OptionalLong.empty() : OptionalLong.of(retryAfterMs);
    }

    /** {@link #retryAfterMs()} as a duration. */
    public Optional<Duration> retryAfter() {
        return retryAfterMs == null ? Optional.empty() : Optional.of(Duration.ofMillis(retryAfterMs));
    }

    /** What to do about it: the fixed sentence of the catalog for {@link #code()}. */
    public String remediation() {
        return code.remediation();
    }

    @Override
    public String toString() {
        return "SagatException[" + code + "]: " + getMessage();
    }

    /** Builds a {@link SagatException}. */
    public static final class Builder {
        private final ErrorCode code;
        private final String message;
        private Integer httpStatus;
        private String grpcStatus;
        private Boolean retryable;
        private String traceId;
        private String reason;
        private Long retryAfterMs;
        private Throwable cause;

        private Builder(ErrorCode code, String message) {
            this.code = Objects.requireNonNull(code, "code");
            this.message = Objects.requireNonNull(message, "message");
        }

        /** The HTTP status of the answer. */
        public Builder httpStatus(int status) {
            this.httpStatus = status;
            return this;
        }

        /** The gRPC status name of the answer. */
        public Builder grpcStatus(String status) {
            this.grpcStatus = status;
            return this;
        }

        /** Overrides the catalog's default for the code. */
        public Builder retryable(Boolean value) {
            this.retryable = value;
            return this;
        }

        /** The trace id of the server's answer. */
        public Builder traceId(String value) {
            this.traceId = value;
            return this;
        }

        /** The server's stable sub-reason. */
        public Builder reason(String value) {
            this.reason = value;
            return this;
        }

        /** The wait the server asked for. */
        public Builder retryAfterMs(Long value) {
            this.retryAfterMs = value;
            return this;
        }

        /** The underlying failure, for the stack trace only: it is never part of the message. */
        public Builder cause(Throwable value) {
            this.cause = value;
            return this;
        }

        public SagatException build() {
            return new SagatException(this);
        }
    }
}
