package sagat

import (
	"context"
	"fmt"
	"io"
	"log/slog"

	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/tokenfail"
)

// TokenSource supplies the bearer token of a call. It is asked before every request, so that a rotated
// token is used at once. It must be safe for concurrent use. The SDK never writes a token to a log, an
// error message or a metric label, and a panic in the source is a failed source, not a crash.
//
// The tenant is never configured: it is the token's tenant claim.
type TokenSource interface {
	Token(ctx context.Context) (string, error)
}

// TokenFunc is a TokenSource made of a function: a provider that returns a fresh token.
type TokenFunc func(ctx context.Context) (string, error)

// Token implements TokenSource.
func (f TokenFunc) Token(ctx context.Context) (string, error) { return f(ctx) }

const redacted = "<redacted>"

// staticToken holds the secret in an unexported field behind a pointer, and prints as <redacted> in
// every way a value is printed (fmt with any verb, %#v, JSON, text, slog), so that a client, a host or
// a configuration that holds it can be logged with %+v without leaking the credential.
type staticToken struct{ secret string }

func (t *staticToken) Token(context.Context) (string, error) { return t.secret, nil }
func (*staticToken) String() string                          { return redacted }
func (*staticToken) GoString() string                        { return redacted }
func (*staticToken) Format(f fmt.State, _ rune)              { _, _ = io.WriteString(f, redacted) }
func (*staticToken) MarshalJSON() ([]byte, error)            { return []byte(`"` + redacted + `"`), nil }
func (*staticToken) MarshalText() ([]byte, error)            { return []byte(redacted), nil }
func (*staticToken) LogValue() slog.Value                    { return slog.StringValue(redacted) }

// StaticToken is a TokenSource for a token that does not change. It never prints the token.
func StaticToken(token string) TokenSource { return &staticToken{secret: token} }

// CanRefresh reports whether asking the source again may return another token: false for a
// StaticToken. The worker host registers again, once, with a fresh token when it can.
func CanRefresh(source TokenSource) bool {
	_, static := source.(*staticToken)
	return !static
}

// BearerToken asks the source for the token of one call. A provider that fails, or panics, is an
// UNAUTHENTICATED error with the reason TOKEN_PROVIDER_FAILED whose message does not repeat the provider's;
// the SDK repeats the call, because nothing was sent.
func BearerToken(ctx context.Context, source TokenSource) (string, error) {
	if source == nil {
		return "", NewError(CodeUnauthenticated, "no token source is configured")
	}
	token, err := ask(ctx, source)
	if err != nil {
		if ctx.Err() != nil {
			return "", NewError(CodeCancelled, "the call was cancelled").WithCause(ctx.Err())
		}
		// Nothing was sent: the SDK knows it, and keeps it privately (a reason a server sends never says so).
		e := NewError(CodeUnauthenticated, "the token source failed")
		e.Reason = "TOKEN_PROVIDER_FAILED"
		failure := tokenfail.Failure{}
		if _, panicked := err.(sourcePanic); !panicked {
			failure.Cause = err
		}
		return "", e.WithCause(failure)
	}
	return token, nil
}

// sourcePanic marks that the source panicked; what it panicked with is dropped, because it may hold
// the very credential that was being fetched.
type sourcePanic struct{}

func (sourcePanic) Error() string { return "the token source panicked" }

func ask(ctx context.Context, source TokenSource) (token string, err error) {
	defer func() {
		if recover() != nil {
			token, err = "", sourcePanic{}
		}
	}()
	return source.Token(ctx)
}
