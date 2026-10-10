package client_test

import (
	"context"
	"net/http"
	"strings"
	"testing"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
)

// An origin that answers every call with a redirect to a second server; the second server must never
// see a request, whatever the status and the method: the request carries a bearer token, the namespace
// and an idempotency key.
func TestARedirectIsNeverFollowedAndIsAProtocolError(t *testing.T) {
	for _, status := range []int{301, 302, 303, 307, 308} {
		origin, target := newFake(t), newFake(t)
		target.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
		origin.answer(func(_ seen, w http.ResponseWriter) {
			w.Header().Set("Location", target.server.URL+"/v1/runs/stolen")
			w.Header().Set("X-Trace-Id", strings.Repeat("d", 32))
			w.WriteHeader(status)
		})
		for name, call := range map[string]func(*client.Client) error{
			"get": func(c *client.Client) error { _, err := c.Get(context.Background(), "r"); return err },
			"start": func(c *client.Client) error {
				_, err := c.Start(context.Background(), workflowID, map[string]any{"a": 1}, client.WithIdempotencyKey("key-12345678"))
				return err
			},
			"signal": func(c *client.Client) error {
				_, err := c.Signal(context.Background(), "r", "go", map[string]any{"a": 1})
				return err
			},
			"publish": func(c *client.Client) error {
				_, err := c.PublishEvent(context.Background(), client.Event{Type: "t", Data: map[string]any{"a": 1}})
				return err
			},
		} {
			origin.answer(origin.handler)
			err := call(newClient(t, origin))
			se := asSagat(t, err)
			if se.Code != sagat.CodeProtocol || se.HTTPStatus != status || se.Retryable || se.TraceID != strings.Repeat("d", 32) {
				t.Errorf("%d %s: %+v", status, name, se)
			}
			if strings.Contains(se.Error(), target.server.URL) || strings.Contains(se.Error(), "stolen") || !strings.Contains(se.Message, "redirected") {
				t.Errorf("%d %s: the message is %q", status, name, se.Message)
			}
			if n := len(origin.requests()); n != 1 {
				t.Errorf("%d %s: a redirect is not retried: the origin saw %d requests", status, name, n)
			}
			if n := len(target.requests()); n != 0 {
				t.Fatalf("%d %s: the redirect target saw %d requests: the redirect was followed", status, name, n)
			}
		}
	}
}

func TestAClientTheCallerSuppliesDoesNotFollowRedirectsEitherAndIsLeftAsItWas(t *testing.T) {
	origin, target := newFake(t), newFake(t)
	target.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	origin.answer(func(_ seen, w http.ResponseWriter) {
		w.Header().Set("Location", target.server.URL+"/v1/runs/stolen")
		w.WriteHeader(http.StatusFound)
	})
	// A client that would follow ten redirects, and says so with a CheckRedirect of its own.
	followed := 0
	theirs := &http.Client{Timeout: time.Minute, CheckRedirect: func(*http.Request, []*http.Request) error { followed++; return nil }}
	c := newClient(t, origin, client.WithHTTPClient(theirs))
	_, err := c.Get(context.Background(), "r")
	if se := asSagat(t, err); se.Code != sagat.CodeProtocol || se.HTTPStatus != 302 {
		t.Errorf("%+v", se)
	}
	if n := len(target.requests()); n != 0 || followed != 0 {
		t.Errorf("the target saw %d requests and their CheckRedirect ran %d times", n, followed)
	}
	// The caller's client is not changed: it is copied.
	if theirs.CheckRedirect == nil || theirs.Timeout != time.Minute {
		t.Errorf("the client of the caller was changed: %+v", theirs)
	}
	req, _ := http.NewRequest("GET", origin.base(), nil)
	if err := theirs.CheckRedirect(req, nil); err != nil || followed != 1 {
		t.Errorf("their own CheckRedirect is still theirs: %v %d", err, followed)
	}
}

func TestAnAnswerThatIsNotARedirectButHasALocationIsNotTakenForOne(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		w.Header().Set("Location", "/v1/runs/r")
		reply(w, 202, run(nil))
	})
	if _, err := newClient(t, f).Start(context.Background(), workflowID, nil); err != nil {
		t.Errorf("a 202 with a Location is a start: %v", err)
	}
}
