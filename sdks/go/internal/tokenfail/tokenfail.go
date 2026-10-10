// Package tokenfail marks, privately, that an error came from the token source of the SDK's user and that
// nothing was sent: repeating the call cannot do harm (sdks/SPEC.md, section 5). The marker is a Go type in an
// internal package, which no caller can build and no server can send; a reason string in a server's answer
// (even "TOKEN_PROVIDER_FAILED") never makes a call repeatable.
package tokenfail

import "errors"

// Failure is the cause the SDK attaches to the UNAUTHENTICATED error of a token source that failed. Cause is
// what the source returned, if anything (a panic leaves none), kept for errors.Is and errors.As; it is never
// part of the error's message.
type Failure struct{ Cause error }

// Error implements error.
func (Failure) Error() string { return "the token source failed" }

// Unwrap returns what the token source returned.
func (f Failure) Unwrap() error { return f.Cause }

// Is reports whether err is, or wraps, a Failure.
func Is(err error) bool {
	var failure Failure
	return errors.As(err, &failure)
}
