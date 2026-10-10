package worker

import (
	"context"
	"errors"
	"testing"
	"time"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
)

// The mapping of a gRPC failure onto the canonical errors (sdks/SPEC.md, section 2).
func TestAGRPCFailureIsMappedOntoTheCanonicalErrors(t *testing.T) {
	cases := []struct {
		name      string
		method    string
		code      codes.Code
		message   string
		reason    string
		want      sagat.Code
		retryable bool
	}{
		{"unauthenticated", methodPoll, codes.Unauthenticated, "", "", sagat.CodeUnauthenticated, false},
		{"not bound", methodRegister, codes.PermissionDenied, "", "NOT_BOUND", sagat.CodeNotBound, false},
		{"secret not usable", methodResolveSecrets, codes.PermissionDenied, "", "SECRET_NOT_USABLE", sagat.CodeSecretNotUsable, false},
		{"not a worker", methodPoll, codes.PermissionDenied, "", "", sagat.CodeForbidden, false},
		{"unsupported protocol by trailer", methodRegister, codes.InvalidArgument, "", "UNSUPPORTED_PROTOCOL", sagat.CodeUnsupportedProtocol, false},
		{"unsupported protocol by message", methodRegister, codes.InvalidArgument, "Unsupported Protocol version 2", "", sagat.CodeUnsupportedProtocol, false},
		{"the same message elsewhere is only a bad field", methodPoll, codes.InvalidArgument, "unsupported protocol", "", sagat.CodeValidationFailed, false},
		{"invalid argument", methodCompleteStep, codes.InvalidArgument, "", "", sagat.CodeValidationFailed, false},
		{"stale by trailer", methodCompleteStep, codes.Aborted, "", "STALE_ATTEMPT", sagat.CodeStaleAttempt, false},
		{"stale with no trailer", methodHeartbeat, codes.Aborted, "", "", sagat.CodeStaleAttempt, false},
		{"stale by precondition", methodHeartbeat, codes.FailedPrecondition, "", "", sagat.CodeStaleAttempt, false},
		{"key reused", methodFailStep, codes.Aborted, "", "IDEMPOTENCY_KEY_REUSED", sagat.CodeIdempotencyKeyReused, false},
		{"another conflict", methodFailStep, codes.Aborted, "", "SOMETHING_NEW", sagat.CodeConflict, false},
		{"not found", methodHeartbeat, codes.NotFound, "", "", sagat.CodeNotFound, false},
		{"rate limited", methodPoll, codes.ResourceExhausted, "", "", sagat.CodeRateLimited, true},
		{"quota", methodPoll, codes.ResourceExhausted, "", "QUOTA_EXCEEDED", sagat.CodeQuotaExceeded, false},
		{"storage unavailable", methodCompleteStep, codes.Unavailable, "", "UNAVAILABLE", sagat.CodeUnavailable, true},
		{"no answer", methodCompleteStep, codes.Unavailable, "connection refused", "", sagat.CodeTransport, true},
		{"deadline", methodPoll, codes.DeadlineExceeded, "", "", sagat.CodeTransport, true},
		{"cancelled by the server", methodPoll, codes.Canceled, "", "", sagat.CodeCancelled, false},
		{"anything else", methodPoll, codes.DataLoss, "", "", sagat.CodeInternal, false},
		{"unknown", methodPoll, codes.Unknown, "", "", sagat.CodeInternal, false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			trailer := metadata.MD{}
			if c.reason != "" {
				trailer.Set("x-sagat-error", c.reason)
			}
			err := errorOfStatus(context.Background(), c.method, status.Error(c.code, c.message), nil, trailer)
			var se *sagat.Error
			if !errors.As(err, &se) {
				t.Fatalf("%T", err)
			}
			if se.Code != c.want || se.Retryable != c.retryable {
				t.Errorf("%s retryable=%v, want %s retryable=%v", se.Code, se.Retryable, c.want, c.retryable)
			}
			if c.want != sagat.CodeCancelled && se.GRPCStatus != statusName(c.code) {
				t.Errorf("status %q", se.GRPCStatus)
			}
			if se.Reason != c.reason {
				t.Errorf("reason %q", se.Reason)
			}
			if se.Remediation != sagat.Remediation(c.want) || se.Message == "" {
				t.Errorf("%q / %q", se.Remediation, se.Message)
			}
		})
	}
}

func TestTheMachineCodeAndTheRetryAfterAreReadFromTheTrailersAndFromTheHeadersAsAFallback(t *testing.T) {
	header := metadata.Pairs("x-sagat-error", "NOT_BOUND", "retry-after-ms", "250")
	err := errorOfStatus(context.Background(), methodRegister, status.Error(codes.PermissionDenied, ""), header, nil)
	var se *sagat.Error
	if !errors.As(err, &se) || se.Code != sagat.CodeNotBound || se.RetryAfter == nil || *se.RetryAfter != 250*time.Millisecond {
		t.Errorf("%+v", err)
	}
	trailer := metadata.Pairs("x-sagat-error", "STALE_ATTEMPT")
	header = metadata.Pairs("x-sagat-error", "IDEMPOTENCY_KEY_REUSED")
	err = errorOfStatus(context.Background(), methodCompleteStep, status.Error(codes.Aborted, ""), header, trailer)
	if !errors.As(err, &se) || se.Code != sagat.CodeStaleAttempt {
		t.Errorf("the trailer wins: %+v", err)
	}
	bad := metadata.Pairs("retry-after-ms", "soon")
	err = errorOfStatus(context.Background(), methodPoll, status.Error(codes.ResourceExhausted, ""), nil, bad)
	if !errors.As(err, &se) || se.RetryAfter != nil {
		t.Errorf("a retry-after that is not a number is ignored: %+v", se)
	}
}

func TestACallerWhoseContextEndedGetsCancelledAndAnErrorOfAnInterceptorIsKept(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	err := errorOfStatus(ctx, methodPoll, status.Error(codes.Canceled, "context canceled"), nil, nil)
	if !sagat.IsCode(err, sagat.CodeCancelled) || !errors.Is(err, context.Canceled) {
		t.Errorf("%v", err)
	}
	own := sagat.NewError(sagat.CodeUnauthenticated, "the token source failed")
	if got := errorOfStatus(context.Background(), methodPoll, own, nil, nil); got != error(own) {
		t.Errorf("%v", got)
	}
}

func TestMinorVersions(t *testing.T) {
	for version, want := range map[string]int{"1": 0, "1.0": 0, "1.1": 1, "1.2": 2, "1.12": 12, "": 0, "one": 0, "2": 0, "1.x": 0, "1.": 0} {
		if got := minorOf(version); got != want {
			t.Errorf("%q: %d, want %d", version, got, want)
		}
	}
}

func TestTheSafeMessageIsValidUTF8AndAtMost512Characters(t *testing.T) {
	if got := safeMessage("a\xffb"); got != "a�b" {
		t.Errorf("%q", got)
	}
	long := make([]rune, 600)
	for i := range long {
		long[i] = 'é'
	}
	if got := safeMessage(string(long)); len([]rune(got)) != 512 {
		t.Errorf("%d", len([]rune(got)))
	}
}
