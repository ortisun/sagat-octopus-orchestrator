package sagat

import "context"

// TokenSource supplies the bearer token of a call. It is asked before every request, so that a
// rotated token is used at once. It must be safe for concurrent use. The SDK never writes a token to
// a log, an error message or a metric label.
//
// The tenant is never configured: it is the token's tenant claim.
type TokenSource interface {
	Token(ctx context.Context) (string, error)
}

// TokenFunc is a TokenSource made of a function: a provider that returns a fresh token.
type TokenFunc func(ctx context.Context) (string, error)

// Token implements TokenSource.
func (f TokenFunc) Token(ctx context.Context) (string, error) { return f(ctx) }

type staticToken string

func (s staticToken) Token(context.Context) (string, error) { return string(s), nil }

// StaticToken is a TokenSource for a token that does not change.
func StaticToken(token string) TokenSource { return staticToken(token) }

// CanRefresh reports whether asking the source again may return another token: false for a
// StaticToken. The worker host registers again, once, with a fresh token when it can.
func CanRefresh(source TokenSource) bool {
	_, static := source.(staticToken)
	return !static
}

// BearerToken asks the source for the token of one call. A provider that fails is an
// UNAUTHENTICATED error whose message does not repeat the provider's.
func BearerToken(ctx context.Context, source TokenSource) (string, error) {
	if source == nil {
		return "", NewError(CodeUnauthenticated, "no token source is configured")
	}
	token, err := source.Token(ctx)
	if err != nil {
		if ctx.Err() != nil {
			return "", NewError(CodeCancelled, "the call was cancelled").WithCause(ctx.Err())
		}
		e := NewError(CodeUnauthenticated, "the token source failed")
		e.Reason = "TOKEN_SOURCE_FAILED"
		return "", e.WithCause(err)
	}
	return token, nil
}
