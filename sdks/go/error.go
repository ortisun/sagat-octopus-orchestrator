package sagat

import (
	"errors"
	"fmt"
	"strings"
	"time"
)

// Code is a stable machine code of the error catalog (sdks/error-catalog.json).
type Code string

// The canonical error codes (sdks/SPEC.md, section 2).
const (
	// From the REST envelope.
	CodeBadRequest       Code = "BAD_REQUEST"
	CodeUnauthenticated  Code = "UNAUTHENTICATED"
	CodeForbidden        Code = "FORBIDDEN"
	CodeNotFound         Code = "NOT_FOUND"
	CodeConflict         Code = "CONFLICT"
	CodeValidationFailed Code = "VALIDATION_FAILED"
	CodeRateLimited      Code = "RATE_LIMITED"
	CodeQuotaExceeded    Code = "QUOTA_EXCEEDED"
	CodeUnavailable      Code = "UNAVAILABLE"
	CodeInternal         Code = "INTERNAL"

	// From the worker protocol.
	CodeStaleAttempt         Code = "STALE_ATTEMPT"
	CodeIdempotencyKeyReused Code = "IDEMPOTENCY_KEY_REUSED"
	CodeNotBound             Code = "NOT_BOUND"
	CodeSecretNotUsable      Code = "SECRET_NOT_USABLE"
	CodeUnsupportedProtocol  Code = "UNSUPPORTED_PROTOCOL"

	// Raised by the SDK itself.
	CodeTransport       Code = "TRANSPORT"
	CodeCancelled       Code = "CANCELLED"
	CodeProtocol        Code = "PROTOCOL"
	CodePayloadTooLarge Code = "PAYLOAD_TOO_LARGE"
)

// Error is the one error the SDK returns for a failure. Use errors.As to reach it:
//
//	var e *sagat.Error
//	if errors.As(err, &e) && e.Code == sagat.CodeNotFound { ... }
//
// Message is safe to log: it never holds a payload, a token, a lease token or a secret.
// Remediation says what to do about Code.
type Error struct {
	Code    Code
	Message string
	// HTTPStatus is the status of a REST answer; zero when the failure came before any answer or from
	// the worker protocol.
	HTTPStatus int
	// GRPCStatus is the name of the gRPC status ("PERMISSION_DENIED"); empty for a REST failure or a
	// failure before any answer.
	GRPCStatus string
	// Retryable says whether the same call may be made again as it is. It is the server's own flag
	// when it sent one, the catalog's default for Code otherwise.
	Retryable bool
	// TraceID is 32 lowercase hex characters from X-Trace-Id or the error envelope; empty when the
	// server did not answer.
	TraceID string
	// Reason is the server's stable sub-reason (details.reason, or the x-sagat-error value of a gRPC
	// answer); empty when there is none.
	Reason string
	// RetryAfter is what Retry-After or retry-after-ms asked for; nil when the server asked nothing.
	RetryAfter *time.Duration
	// Remediation is the fixed sentence of the catalog for Code.
	Remediation string

	cause error
}

// NewError returns an Error for code with the catalog's remediation and default Retryable. The
// caller sets the other fields it knows.
func NewError(code Code, message string) *Error {
	return &Error{
		Code:        code,
		Message:     message,
		Retryable:   RetryableByDefault(code),
		Remediation: Remediation(code),
	}
}

// Errorf is NewError with a formatted message. Never format a payload, a token or a secret into it.
func Errorf(code Code, format string, args ...any) *Error {
	return NewError(code, fmt.Sprintf(format, args...))
}

// WithCause attaches the error that led to this one, so that errors.Is and errors.As reach it
// (errors.Is(err, context.Canceled) for a cancelled call). The cause is never part of Error().
func (e *Error) WithCause(cause error) *Error {
	e.cause = cause
	return e
}

// Unwrap returns the cause, if any.
func (e *Error) Unwrap() error { return e.cause }

// Error implements error. It holds the code, the safe message and the trace id, nothing else.
func (e *Error) Error() string {
	var b strings.Builder
	b.WriteString("sagat: ")
	b.WriteString(string(e.Code))
	if e.Message != "" {
		b.WriteString(": ")
		b.WriteString(e.Message)
	}
	if e.TraceID != "" {
		b.WriteString(" (trace ")
		b.WriteString(e.TraceID)
		b.WriteString(")")
	}
	return b.String()
}

// IsCode reports whether err is, or wraps, a *Error with the code.
func IsCode(err error, code Code) bool {
	var e *Error
	return errors.As(err, &e) && e.Code == code
}

// Invalid is the error for something wrong in what the caller gave the SDK, raised before anything
// is sent (VALIDATION_FAILED).
func Invalid(format string, args ...any) *Error {
	return Errorf(CodeValidationFailed, format, args...)
}
