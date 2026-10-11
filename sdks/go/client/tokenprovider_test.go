package client_test

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
)

// SPEC section 5: a token provider that fails is repeated, because nothing was sent. That it is the provider is
// something the SDK knows and keeps privately; a reason in a server's answer never makes a call repeatable.
func TestACallWhoseTokenSourceFailsOnceIsRepeatedAndNothingWasSentTheFirstTime(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	var asked atomic.Int32
	source := sagat.TokenFunc(func(context.Context) (string, error) {
		if asked.Add(1) == 1 {
			return "", errors.New("the vault is restarting: SECRET-DETAIL")
		}
		return "tok-2", nil
	})
	c, err := client.New(f.base(), "payments", source, fast())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := c.Get(context.Background(), runUUID); err != nil {
		t.Fatalf("a source that failed once: %v", err)
	}
	requests := f.requests()
	if len(requests) != 1 || asked.Load() != 2 || requests[0].headers.Get("Authorization") != "Bearer tok-2" {
		t.Errorf("%d requests, the source asked %d times", len(requests), asked.Load())
	}

	// A call with a key too: the key of the call is the same on the attempt that went.
	asked.Store(0)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	if _, err := c.Start(context.Background(), workflowID, nil); err != nil {
		t.Fatalf("%v", err)
	}
	if n := len(f.requests()); n != 1 {
		t.Errorf("%d requests", n)
	}
}

func TestATokenSourceThatKeepsFailingEndsAsUnauthenticatedWithTheReasonAfterTheAttemptsAndSendsNothing(t *testing.T) {
	for name, fail := range map[string]func() (string, error){
		"an error": func() (string, error) { return "", errors.New("the vault said: SECRET-DETAIL") },
		"a panic":  func() (string, error) { panic("the vault exploded: SECRET-DETAIL") },
	} {
		f := newFake(t)
		f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
		var asked atomic.Int32
		c, err := client.New(f.base(), "payments", sagat.TokenFunc(func(context.Context) (string, error) { asked.Add(1); return fail() }),
			client.WithRetry(sagat.RetryPolicy{MaxAttempts: 3, InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}))
		if err != nil {
			t.Fatal(err)
		}
		_, err = c.Get(context.Background(), runUUID)
		se := asSagat(t, err)
		if se.Code != sagat.CodeUnauthenticated || se.Reason != "TOKEN_PROVIDER_FAILED" || strings.Contains(se.Error(), "SECRET-DETAIL") {
			t.Errorf("%s: %+v", name, se)
		}
		if asked.Load() != 3 || len(f.requests()) != 0 {
			t.Errorf("%s: the source was asked %d times and %d requests were sent", name, asked.Load(), len(f.requests()))
		}
	}
}

func TestAReasonOfTheTokenProviderInAServersAnswerNeverMakesACallRepeatable(t *testing.T) {
	for _, status := range []int{400, 401, 403, 409, 422} {
		for _, reason := range []string{"TOKEN_PROVIDER_FAILED", "TOKEN_SOURCE_FAILED"} {
			code := map[int]string{400: "BAD_REQUEST", 401: "UNAUTHENTICATED", 403: "FORBIDDEN", 409: "CONFLICT", 422: "VALIDATION_FAILED"}[status]
			f := newFake(t)
			f.answer(func(_ seen, w http.ResponseWriter) {
				reply(w, status, map[string]any{"code": code, "message": "m", "retryable": true, "details": map[string]any{"reason": reason}})
			})
			c := newClient(t, f, client.WithRetry(sagat.RetryPolicy{MaxAttempts: 4, InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}))
			_, err := c.Get(context.Background(), runUUID)
			if se := asSagat(t, err); se.Reason != reason {
				t.Errorf("%d: the reason is kept: %+v", status, se)
			}
			if n := len(f.requests()); n != 1 {
				t.Errorf("%d with the reason %s was repeated: %d requests", status, reason, n)
			}
		}
	}
}
