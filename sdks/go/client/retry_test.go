package client_test

import (
	"context"
	"fmt"
	"net/http"
	"strings"
	"testing"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
)

// The rule is by code (SPEC section 5): a call is repeated when the error's code is TRANSPORT, UNAVAILABLE,
// RATE_LIMITED or INTERNAL and it says it may be. The server's 408 is UNAVAILABLE (retryable); a 429 that is a
// quota is never repeated; a body that claims retryable on any other 4xx is not believed.
func TestTheRetryRuleIsByCodeAndNotByStatus(t *testing.T) {
	envelope := func(code string, retryable any) any {
		body := map[string]any{"code": code, "message": "m", "traceId": strings.Repeat("a", 32)}
		if retryable != nil {
			body["retryable"] = retryable
		}
		return body
	}
	cases := []struct {
		name   string
		status int
		body   any
		repeat bool
	}{
		{"408 with no envelope", 408, "<html>timeout</html>", true},
		{"408 empty", 408, nil, true},
		{"408 UNAVAILABLE retryable", 408, envelope("UNAVAILABLE", true), true},
		{"429 RATE_LIMITED", 429, envelope("RATE_LIMITED", nil), true},
		{"429 RATE_LIMITED retryable", 429, envelope("RATE_LIMITED", true), true},
		{"429 RATE_LIMITED not retryable", 429, envelope("RATE_LIMITED", false), false},
		{"429 QUOTA_EXCEEDED", 429, envelope("QUOTA_EXCEEDED", nil), false},
		{"429 QUOTA_EXCEEDED claims retryable", 429, envelope("QUOTA_EXCEEDED", true), false},
		{"429 with no envelope", 429, "slow down", true},
		{"502 with no envelope", 502, "bad gateway", true},
		{"503 UNAVAILABLE", 503, envelope("UNAVAILABLE", nil), true},
		{"503 UNAVAILABLE retryable", 503, envelope("UNAVAILABLE", true), true},
		{"503 UNAVAILABLE says it may not be", 503, envelope("UNAVAILABLE", false), false},
		{"504 with no envelope", 504, nil, true},
		{"500 INTERNAL", 500, envelope("INTERNAL", nil), false},
		{"500 INTERNAL retryable", 500, envelope("INTERNAL", true), true},
		{"500 with no envelope", 500, "oops", false},
		{"400 BAD_REQUEST claims retryable", 400, envelope("BAD_REQUEST", true), false},
		{"401 UNAUTHENTICATED claims retryable", 401, envelope("UNAUTHENTICATED", true), false},
		{"403 FORBIDDEN claims retryable", 403, envelope("FORBIDDEN", true), false},
		{"404 NOT_FOUND claims retryable", 404, envelope("NOT_FOUND", true), false},
		{"409 CONFLICT claims retryable", 409, envelope("CONFLICT", true), false},
		{"422 VALIDATION_FAILED claims retryable", 422, envelope("VALIDATION_FAILED", true), false},
		{"418 with no envelope", 418, "teapot", false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			f := newFake(t)
			f.answer(func(_ seen, w http.ResponseWriter) { reply(w, c.status, c.body) })
			cl := newClient(t, f, client.WithRetry(sagat.RetryPolicy{MaxAttempts: 3, InitialDelay: 1, MaxDelay: 2}))
			for name, call := range map[string]func() error{
				"get": func() error { _, err := cl.Get(context.Background(), runUUID); return err },
				"start": func() error {
					_, err := cl.Start(context.Background(), workflowID, nil, client.WithIdempotencyKey("key-12345678"))
					return err
				},
			} {
				f.answer(f.handler)
				if err := call(); err == nil {
					t.Fatalf("%s: no error", name)
				}
				want := 1
				if c.repeat {
					want = 3
				}
				if got := len(f.requests()); got != want {
					t.Errorf("%s: %d requests, want %d (repeat=%v)", name, got, want, c.repeat)
				}
			}
		})
	}
}

func TestAServerThatDoesNotHaveTheCallIsNotFoundWithTheReasonUnsupportedByServer(t *testing.T) {
	envelope := func(code, reason string) any {
		body := map[string]any{"code": code, "message": "m"}
		if reason != "" {
			body["details"] = map[string]any{"reason": reason}
		}
		return body
	}
	cases := []struct {
		name        string
		status      int
		body        any
		unsupported bool
		wantCode    sagat.Code
	}{
		{"unknown route", 404, envelope("NOT_FOUND", "ROUTE_NOT_FOUND"), true, sagat.CodeNotFound},
		{"wrong method", 405, envelope("BAD_REQUEST", "METHOD_NOT_ALLOWED"), true, sagat.CodeNotFound},
		{"not implemented", 501, nil, true, sagat.CodeNotFound},
		{"not implemented with an envelope", 501, envelope("INTERNAL", ""), true, sagat.CodeNotFound},
		{"404 that is not the platform's envelope", 404, "<html>nope</html>", true, sagat.CodeNotFound},
		{"404 with an empty body", 404, nil, true, sagat.CodeNotFound},
		{"a run that does not exist", 404, envelope("NOT_FOUND", ""), false, sagat.CodeNotFound},
		{"a 404 with another reason", 404, envelope("NOT_FOUND", "RUN_NOT_FOUND"), false, sagat.CodeNotFound},
		{"405 with no reason", 405, envelope("BAD_REQUEST", ""), false, sagat.CodeBadRequest},
		{"405 with another reason", 405, envelope("BAD_REQUEST", "SOMETHING_ELSE"), false, sagat.CodeBadRequest},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			f := newFake(t)
			f.answer(func(_ seen, w http.ResponseWriter) { reply(w, c.status, c.body) })
			_, err := newClient(t, f).Get(context.Background(), runUUID)
			se := asSagat(t, err)
			if se.Code != c.wantCode || (se.Reason == "UNSUPPORTED_BY_SERVER") != c.unsupported || se.Retryable {
				t.Errorf("%+v", se)
			}
			if c.unsupported && se.HTTPStatus != c.status {
				t.Errorf("the status is kept: %d", se.HTTPStatus)
			}
			if !c.unsupported && c.wantCode == sagat.CodeNotFound && se.Reason == fmt.Sprint("UNSUPPORTED_BY_SERVER") {
				t.Error("a missing resource is not an unsupported call")
			}
		})
	}
}
