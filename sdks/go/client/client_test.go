package client_test

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"regexp"
	"strings"
	"sync"
	"testing"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
)

const workflowID = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee"

// seen is a request as the fake server saw it.
type seen struct {
	method  string
	url     string
	headers http.Header
	body    []byte
}

func (s seen) json(t *testing.T) map[string]any {
	t.Helper()
	var m map[string]any
	if err := json.Unmarshal(s.body, &m); err != nil {
		t.Fatalf("the body is not JSON: %v: %s", err, s.body)
	}
	return m
}

// fake is a scripted HTTP server: it records every request and lets the test answer it.
type fake struct {
	t       *testing.T
	server  *httptest.Server
	mu      sync.Mutex
	seen    []seen
	handler func(seen, http.ResponseWriter)
}

func newFake(t *testing.T) *fake {
	f := &fake{t: t}
	f.server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		entry := seen{method: r.Method, url: r.URL.RequestURI(), headers: r.Header.Clone(), body: body}
		f.mu.Lock()
		f.seen = append(f.seen, entry)
		handler := f.handler
		f.mu.Unlock()
		handler(entry, w)
	}))
	t.Cleanup(f.server.Close)
	return f
}

func (f *fake) base() string { return f.server.URL + "/v1" }

func (f *fake) requests() []seen {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]seen(nil), f.seen...)
}

// answer replaces the handler and forgets the requests seen so far.
func (f *fake) answer(handler func(seen, http.ResponseWriter)) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.seen = nil
	f.handler = handler
}

func reply(w http.ResponseWriter, status int, body any, headers ...string) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("X-Trace-Id", strings.Repeat("f", 32))
	for i := 0; i+1 < len(headers); i += 2 {
		w.Header().Set(headers[i], headers[i+1])
	}
	w.WriteHeader(status)
	switch b := body.(type) {
	case nil:
	case string:
		_, _ = io.WriteString(w, b)
	default:
		_ = json.NewEncoder(w).Encode(b)
	}
}

func run(overrides map[string]any) map[string]any {
	m := map[string]any{
		"runId": "0190aaaa-0000-7000-8000-000000000001", "workflowId": workflowID, "versionId": "0190aaaa-0000-7000-8000-000000000002",
		"status": "RUNNING", "version": 3, "paused": false, "startedBy": "alice", "createdAt": "2026-10-10T10:00:00Z",
		"updatedAt": "2026-10-10T10:00:01Z", "finishedAt": nil, "deadline": nil, "traceId": "abcdefabcdefabcdefabcdefabcdefab",
		"correlationId": nil, "depth": 0, "parent": nil, "futureField": 7,
	}
	for k, v := range overrides {
		m[k] = v
	}
	return m
}

func fast() client.Option {
	return client.WithRetry(sagat.RetryPolicy{InitialDelay: 5 * time.Millisecond, MaxDelay: 20 * time.Millisecond})
}

func newClient(t *testing.T, f *fake, opts ...client.Option) *client.Client {
	t.Helper()
	c, err := client.New(f.base(), "payments", sagat.StaticToken("tok-1"), append([]client.Option{fast()}, opts...)...)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func asSagat(t *testing.T, err error) *sagat.Error {
	t.Helper()
	var se *sagat.Error
	if !errors.As(err, &se) {
		t.Fatalf("want a *sagat.Error, got %T: %v", err, err)
	}
	return se
}

var uuidText = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`)

func TestAStartSendsTheHeadersAndTheBodyTheAPIExpectsAndReturnsATypedRunThatKeepsTheRawAnswer(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	c := newClient(t, f)
	result, err := c.Start(context.Background(), workflowID, map[string]any{"orderId": "o-1"},
		client.WithIdempotencyKey("key-12345678"), client.WithCorrelationID("corr-1"),
		client.WithTraceparent("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"),
		client.WithDeadline(time.Date(2030, 1, 1, 5, 30, 0, 0, time.FixedZone("IST", 5*3600+1800))))
	if err != nil {
		t.Fatal(err)
	}
	requests := f.requests()
	if len(requests) != 1 {
		t.Fatalf("%d requests", len(requests))
	}
	r := requests[0]
	if r.method != "POST" || r.url != "/v1/runs" {
		t.Errorf("%s %s", r.method, r.url)
	}
	for header, want := range map[string]string{
		"Authorization": "Bearer tok-1", "X-Octopus-Namespace": "payments", "Idempotency-Key": "key-12345678",
		"Traceparent": "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01", "Content-Type": "application/json", "Accept": "application/json",
	} {
		if got := r.headers.Get(header); got != want {
			t.Errorf("%s: %q, want %q", header, got, want)
		}
	}
	if ua := r.headers.Get("User-Agent"); !regexp.MustCompile(`^sagat-sdk-go/\d+\.\d+\.\d+`).MatchString(ua) {
		t.Errorf("user agent %q", ua)
	}
	body := r.json(t)
	want := map[string]any{"workflowId": workflowID, "input": map[string]any{"orderId": "o-1"}, "correlationId": "corr-1", "deadline": "2030-01-01T00:00:00.000Z"}
	if fmt.Sprint(body) != fmt.Sprint(want) {
		t.Errorf("body %v, want %v", body, want)
	}
	if result.Status != client.StatusRunning || result.CreatedAt.Format(time.RFC3339) != "2026-10-10T10:00:00Z" || result.CreatedAt.Location() != time.UTC {
		t.Errorf("%+v", result)
	}
	if string(result.Raw["futureField"]) != "7" {
		t.Error("a field of a newer server is reachable")
	}
	if result.Version != 3 || result.StartedBy != "alice" || result.CorrelationID != "" || !result.FinishedAt.IsZero() || result.Parent != nil {
		t.Errorf("%+v", result)
	}
}

func TestACallWithoutAKeyGetsAFreshOnePerCallAndKeepsItAcrossItsRetries(t *testing.T) {
	f := newFake(t)
	attempts := 0
	f.answer(func(_ seen, w http.ResponseWriter) {
		attempts++
		if attempts < 3 {
			reply(w, 503, map[string]any{"code": "UNAVAILABLE", "message": "busy", "retryable": true, "traceId": strings.Repeat("e", 32)})
			return
		}
		reply(w, 202, run(nil))
	})
	c := newClient(t, f)
	if _, err := c.Start(context.Background(), workflowID, nil); err != nil {
		t.Fatal(err)
	}
	requests := f.requests()
	if len(requests) != 3 {
		t.Fatalf("%d attempts", len(requests))
	}
	keys := map[string]bool{}
	for _, r := range requests {
		keys[r.headers.Get("Idempotency-Key")] = true
	}
	if len(keys) != 1 {
		t.Fatalf("the same key on every attempt: %v", keys)
	}
	first := requests[0].headers.Get("Idempotency-Key")
	if !uuidText.MatchString(first) {
		t.Errorf("the key is a UUID: %q", first)
	}
	attempts = 0
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	if _, err := c.Start(context.Background(), workflowID, nil); err != nil {
		t.Fatal(err)
	}
	if second := f.requests()[0].headers.Get("Idempotency-Key"); second == first {
		t.Error("another call, another key")
	}
}

func TestAnEventWithoutAnIDGetsOnePerCallKeptAcrossRetriesSoARepetitionIsADuplicateAndNotASecondEvent(t *testing.T) {
	f := newFake(t)
	attempts := 0
	f.answer(func(_ seen, w http.ResponseWriter) {
		attempts++
		if attempts < 2 {
			reply(w, 503, "upstream down")
			return
		}
		reply(w, 202, map[string]any{"eventId": "e", "source": "api", "status": "ACCEPTED"})
	})
	c := newClient(t, f)
	receipt, err := c.PublishEvent(context.Background(), client.Event{Type: "order.created", Data: map[string]any{"id": 1}})
	if err != nil {
		t.Fatal(err)
	}
	if receipt.Status != client.EventAccepted {
		t.Errorf("%+v", receipt)
	}
	requests := f.requests()
	if len(requests) != 2 {
		t.Fatalf("%d requests", len(requests))
	}
	first, second := requests[0].json(t)["eventId"], requests[1].json(t)["eventId"]
	if first != second || !uuidText.MatchString(fmt.Sprint(first)) {
		t.Errorf("event ids %v %v", first, second)
	}
}

func TestGetIsRetriedOnUnavailabilityAndOnRateLimitingHonouringRetryAfterAndGivesUpAfterTheAttempts(t *testing.T) {
	f := newFake(t)
	attempts := 0
	f.answer(func(_ seen, w http.ResponseWriter) {
		attempts++
		if attempts == 1 {
			reply(w, 429, map[string]any{"code": "RATE_LIMITED", "message": "slow", "retryable": true, "traceId": strings.Repeat("a", 32)}, "Retry-After", "0")
			return
		}
		reply(w, 200, run(nil))
	})
	c := newClient(t, f)
	got, err := c.Get(context.Background(), runUUID)
	if err != nil {
		t.Fatal(err)
	}
	if got.RunID != "0190aaaa-0000-7000-8000-000000000001" || len(f.requests()) != 2 {
		t.Errorf("%s after %d requests", got.RunID, len(f.requests()))
	}

	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 503, "<html>bad gateway</html>") })
	_, err = c.Get(context.Background(), runUUID)
	se := asSagat(t, err)
	if se.Code != sagat.CodeUnavailable || se.HTTPStatus != 503 || !se.Retryable {
		t.Errorf("%+v", se)
	}
	if n := len(f.requests()); n != 4 {
		t.Errorf("four attempts in all, got %d", n)
	}
}

func TestRetryAfterLongerThanTheBackoffIsWaitedFor(t *testing.T) {
	f := newFake(t)
	attempts := 0
	f.answer(func(_ seen, w http.ResponseWriter) {
		attempts++
		if attempts == 1 {
			reply(w, 429, map[string]any{"code": "RATE_LIMITED", "message": "slow", "retryable": true}, "Retry-After", "1")
			return
		}
		reply(w, 200, run(nil))
	})
	began := time.Now()
	if _, err := newClient(t, f).Get(context.Background(), runUUID); err != nil {
		t.Fatal(err)
	}
	if waited := time.Since(began); waited < 900*time.Millisecond {
		t.Errorf("waited only %v for a Retry-After of 1 s", waited)
	}
}

func TestA4xxOtherThan429IsNeverRetriedAndKeepsWhatTheServerSaid(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 409, map[string]any{
			"code": "CONFLICT", "message": "the idempotency key was used for another request", "retryable": false,
			"traceId": strings.Repeat("c", 32), "details": map[string]any{"reason": "IDEMPOTENCY_KEY_REUSED"},
		})
	})
	_, err := newClient(t, f).Start(context.Background(), workflowID, nil, client.WithIdempotencyKey("key-12345678"))
	se := asSagat(t, err)
	if se.Code != sagat.CodeConflict || se.Reason != "IDEMPOTENCY_KEY_REUSED" || se.TraceID != strings.Repeat("c", 32) || se.Retryable || se.Remediation == "" {
		t.Errorf("%+v", se)
	}
	if n := len(f.requests()); n != 1 {
		t.Errorf("%d requests", n)
	}

	// Even when the envelope says it may be repeated: a 4xx other than 429 is not.
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 409, map[string]any{"code": "CONFLICT", "message": "x", "retryable": true})
	})
	_, err = newClient(t, f).Start(context.Background(), workflowID, nil)
	asSagat(t, err)
	if n := len(f.requests()); n != 1 {
		t.Errorf("a 409 marked retryable was repeated: %d requests", n)
	}
}

func TestARefusedTokenAMissingResourceAndAValidationErrorAreToldApart(t *testing.T) {
	f := newFake(t)
	c := newClient(t, f)
	for status, code := range map[int]sagat.Code{401: sagat.CodeUnauthenticated, 403: sagat.CodeForbidden, 404: sagat.CodeNotFound, 422: sagat.CodeValidationFailed} {
		f.answer(func(_ seen, w http.ResponseWriter) {
			reply(w, status, map[string]any{"code": string(code), "message": "no", "retryable": false, "traceId": strings.Repeat("b", 32)})
		})
		_, err := c.Get(context.Background(), runUUID)
		se := asSagat(t, err)
		if se.Code != code || se.HTTPStatus != status {
			t.Errorf("%d: %+v", status, se)
		}
		if n := len(f.requests()); n != 1 {
			t.Errorf("%d: %d requests", status, n)
		}
	}
}

func TestTheTokenIsAskedForBeforeEveryRequestSoARotatedOneIsUsedAtOnce(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	n := 0
	c, err := client.New(f.base(), "payments", sagat.TokenFunc(func(context.Context) (string, error) {
		n++
		return fmt.Sprintf("tok-%d", n), nil
	}))
	if err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{runUUID, runUUID} {
		if _, err := c.Get(context.Background(), id); err != nil {
			t.Fatal(err)
		}
	}
	requests := f.requests()
	if requests[0].headers.Get("Authorization") != "Bearer tok-1" || requests[1].headers.Get("Authorization") != "Bearer tok-2" {
		t.Errorf("%v %v", requests[0].headers, requests[1].headers)
	}
}

func TestANameIsResolvedToTheIDOfItsWorkflowThroughEveryPageAndRemembered(t *testing.T) {
	f := newFake(t)
	f.answer(func(r seen, w http.ResponseWriter) {
		if strings.HasPrefix(r.url, "/v1/workflows") {
			if strings.Contains(r.url, "cursor=c2") {
				reply(w, 200, map[string]any{"items": []any{map[string]any{"id": workflowID, "name": "order-flow"}}, "nextCursor": nil})
			} else {
				reply(w, 200, map[string]any{"items": []any{map[string]any{"id": "0190aaaa-0000-7000-8000-00000000ffff", "name": "other"}}, "nextCursor": "c2"})
			}
			return
		}
		reply(w, 202, run(nil))
	})
	c := newClient(t, f)
	for i := 0; i < 2; i++ {
		if _, err := c.Start(context.Background(), "order-flow", nil); err != nil {
			t.Fatal(err)
		}
	}
	lookups, posts := 0, []seen{}
	for _, r := range f.requests() {
		if strings.HasPrefix(r.url, "/v1/workflows") {
			lookups++
		} else {
			posts = append(posts, r)
		}
	}
	if lookups != 2 {
		t.Errorf("two pages, once: %d lookups", lookups)
	}
	if posts[0].json(t)["workflowId"] != workflowID {
		t.Errorf("%s", posts[0].body)
	}

	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"items": []any{}, "nextCursor": nil})
	})
	_, err := newClient(t, f).Start(context.Background(), "nothing-here", nil)
	if se := asSagat(t, err); se.Code != sagat.CodeNotFound || se.Reason != "WORKFLOW_NOT_FOUND" {
		t.Errorf("%+v", se)
	}
}

func TestAMalformedTraceparentOrKeyIsRefusedBeforeAnythingIsSent(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	c := newClient(t, f)
	_, err := c.Start(context.Background(), workflowID, nil, client.WithTraceparent("not-a-trace"))
	if se := asSagat(t, err); se.Code != sagat.CodeBadRequest {
		t.Errorf("%+v", se)
	}
	_, err = c.Start(context.Background(), workflowID, nil, client.WithIdempotencyKey("short"))
	if se := asSagat(t, err); se.Code != sagat.CodeValidationFailed {
		t.Errorf("%+v", se)
	}
	for _, key := range []string{"has a space in it", "tab\tin-the-key-12", "del\x7fin-the-key-12", "non-ascii-é-in-key", strings.Repeat("k", 161)} {
		_, err = c.Start(context.Background(), workflowID, nil, client.WithIdempotencyKey(key))
		if se := asSagat(t, err); se.Code != sagat.CodeValidationFailed {
			t.Errorf("%q: %+v", key, se)
		}
	}
	_, err = c.Start(context.Background(), "a-name-not-an-id", nil, client.WithIdempotencyKey("short"))
	asSagat(t, err)
	if n := len(f.requests()); n != 0 {
		t.Errorf("%d requests were sent (not even the lookup of a name)", n)
	}
}

func TestAKeyOfVisibleASCIIFrom8To160CharactersIsAccepted(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	c := newClient(t, f)
	for _, key := range []string{"12345678", "!~!~!~!~", strings.Repeat("k", 160), "order/o-1:attempt#1"} {
		if _, err := c.Start(context.Background(), workflowID, nil, client.WithIdempotencyKey(key)); err != nil {
			t.Errorf("%q: %v", key, err)
		}
	}
}

func TestTheAmbientTraceContextIsUsedWhenTheCallHasNone(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	ambient := "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
	c := newClient(t, f, client.WithTraceContext(func(context.Context) string { return ambient }))
	if _, err := c.Get(context.Background(), runUUID); err != nil {
		t.Fatal(err)
	}
	if got := f.requests()[0].headers.Get("Traceparent"); got != ambient {
		t.Errorf("%q", got)
	}
	// An explicit one wins, an empty ambient one sends nothing, and a malformed one is refused.
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	explicit := "00-11111111111111111111111111111111-2222222222222222-01"
	if _, err := c.Start(context.Background(), workflowID, nil, client.WithTraceparent(explicit)); err != nil {
		t.Fatal(err)
	}
	if got := f.requests()[0].headers.Get("Traceparent"); got != explicit {
		t.Errorf("%q", got)
	}
	none := newClient(t, f, client.WithTraceContext(func(context.Context) string { return "" }))
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	if _, err := none.Get(context.Background(), runUUID); err != nil {
		t.Fatal(err)
	}
	if got := f.requests()[0].headers.Get("Traceparent"); got != "" {
		t.Errorf("%q", got)
	}
	bad := newClient(t, f, client.WithTraceContext(func(context.Context) string { return "garbage" }))
	if _, err := bad.Get(context.Background(), runUUID); !sagat.IsCode(err, sagat.CodeBadRequest) {
		t.Errorf("%v", err)
	}
}

func TestAListFollowsItsCursorADeadlineGivenAsADurationIsComputedOnceAndWhatIsNotJSONIsRefused(t *testing.T) {
	f := newFake(t)
	f.answer(func(r seen, w http.ResponseWriter) {
		if strings.Contains(r.url, "cursor=n2") {
			reply(w, 200, map[string]any{"items": []any{run(map[string]any{"runId": "r-2"})}, "nextCursor": nil})
		} else {
			reply(w, 200, map[string]any{"items": []any{run(map[string]any{"runId": "r-1"})}, "nextCursor": "n2"})
		}
	})
	c := newClient(t, f)
	var ids []string
	for item, err := range c.ListAll(context.Background(), client.ListOptions{Status: client.StatusRunning, Limit: 1}) {
		if err != nil {
			t.Fatal(err)
		}
		ids = append(ids, item.RunID)
	}
	if fmt.Sprint(ids) != "[r-1 r-2]" {
		t.Errorf("%v (the order the server gave)", ids)
	}
	if first := f.requests()[0].url; !strings.Contains(first, "status=RUNNING") || !strings.Contains(first, "limit=1") {
		t.Errorf("%s", first)
	}
	// A page, a cursor given by the caller, and stopping early.
	page, err := c.List(context.Background(), client.ListOptions{Limit: 1})
	if err != nil || page.NextCursor != "n2" || len(page.Items) != 1 {
		t.Errorf("%+v %v", page, err)
	}
	var fromCursor []string
	for item, err := range c.ListAll(context.Background(), client.ListOptions{Cursor: "n2"}) {
		if err != nil {
			t.Fatal(err)
		}
		fromCursor = append(fromCursor, item.RunID)
	}
	if fmt.Sprint(fromCursor) != "[r-2]" {
		t.Errorf("a list from a cursor starts there: %v", fromCursor)
	}
	count := 0
	for _, err := range c.ListAll(context.Background(), client.ListOptions{}) {
		if err != nil {
			t.Fatal(err)
		}
		count++
		break
	}
	if count != 1 {
		t.Error("the loop can stop early")
	}

	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	before := time.Now()
	if _, err := c.Start(context.Background(), workflowID, nil, client.WithDeadlineIn(time.Minute)); err != nil {
		t.Fatal(err)
	}
	sent, err := time.Parse(time.RFC3339Nano, f.requests()[0].json(t)["deadline"].(string))
	if err != nil || sent.Before(before.Add(time.Minute-time.Second)) || sent.After(before.Add(time.Minute+time.Second)) {
		t.Errorf("deadline %v (%v)", sent, err)
	}

	// An integer past 2^53 is sent exactly (Go holds it), and what is not JSON is refused.
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	if _, err := c.Start(context.Background(), workflowID, json.RawMessage(`{"big":9007199254740993,"ratio":0.1,"money":"19.99"}`)); err != nil {
		t.Fatal(err)
	}
	if body := string(f.requests()[0].body); !strings.Contains(body, `"big":9007199254740993`) || !strings.Contains(body, `"ratio":0.1`) || !strings.Contains(body, `"money":"19.99"`) {
		t.Errorf("%s", body)
	}
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	for name, input := range map[string]any{"nan": map[string]any{"x": mathNaN()}, "invalid utf-8": map[string]any{"x": "\xff"}} {
		if _, err := c.Start(context.Background(), workflowID, input); !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("%s: %v", name, err)
		}
	}
	if n := len(f.requests()); n != 0 {
		t.Errorf("%d requests were sent", n)
	}
}

func mathNaN() float64 {
	zero := 0.0
	return zero / zero
}

// lossy is a transport that makes the call, lets the server apply it, and loses the answer.
type lossy struct {
	inner http.RoundTripper
	mu    sync.Mutex
	calls int
}

func (l *lossy) RoundTrip(r *http.Request) (*http.Response, error) {
	l.mu.Lock()
	l.calls++
	n := l.calls
	l.mu.Unlock()
	resp, err := l.inner.RoundTrip(r)
	if err == nil && n == 1 {
		_, _ = io.Copy(io.Discard, resp.Body)
		_ = resp.Body.Close()
		return nil, errors.New("connection reset")
	}
	return resp, err
}

func TestNoAnswerAtAllIsARetryableTransportErrorAndALostAnswerIsRetriedOnlyBecauseTheCallCarriesAKey(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	c := newClient(t, f, client.WithHTTPClient(&http.Client{Transport: &lossy{inner: http.DefaultTransport}}))
	result, err := c.Start(context.Background(), workflowID, nil, client.WithIdempotencyKey("key-12345678"))
	if err != nil || result.Status != client.StatusRunning {
		t.Fatalf("%v %v", result, err)
	}
	requests := f.requests()
	if len(requests) != 2 || requests[0].headers.Get("Idempotency-Key") != requests[1].headers.Get("Idempotency-Key") {
		t.Errorf("the lost answer was repeated with the same key: %d requests", len(requests))
	}

	dead, err := client.New("http://127.0.0.1:1/v1", "p", sagat.StaticToken("t"),
		client.WithRetry(sagat.RetryPolicy{MaxAttempts: 2, InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}))
	if err != nil {
		t.Fatal(err)
	}
	_, err = dead.Get(context.Background(), runUUID)
	if se := asSagat(t, err); se.Code != sagat.CodeTransport || !se.Retryable || se.HTTPStatus != 0 || se.GRPCStatus != "" {
		t.Errorf("%+v", se)
	}
}

func TestCancelAndSignalAreOperationsWithAReasonAndAPayloadAndTheResultIsTyped(t *testing.T) {
	f := newFake(t)
	f.answer(func(r seen, w http.ResponseWriter) {
		if strings.HasSuffix(r.url, "/operations") {
			reply(w, 202, map[string]any{"operationId": "op-1", "accepted": true, "runStatus": "CANCELLING", "runVersion": 9})
			return
		}
		reply(w, 202, map[string]any{"deliveredTo": []any{"wait"}})
	})
	c := newClient(t, f)
	cancelled, err := c.Cancel(context.Background(), runUUID, "no longer needed", client.WithExpectedRunVersion(3))
	if err != nil {
		t.Fatal(err)
	}
	if cancelled.OperationID != "op-1" || !cancelled.Accepted || cancelled.RunStatus != client.StatusCancelling || cancelled.RunVersion != 9 {
		t.Errorf("%+v", cancelled)
	}
	if body := f.requests()[0].json(t); fmt.Sprint(body) != "map[action:CANCEL expectedRunVersion:3 reason:no longer needed]" {
		t.Errorf("%v", body)
	}
	signalled, err := c.Signal(context.Background(), runUUID, "payment-confirmed", map[string]any{"paymentId": "p-1"})
	if err != nil {
		t.Fatal(err)
	}
	if fmt.Sprint(signalled.DeliveredTo) != "[wait]" {
		t.Errorf("%v", signalled.DeliveredTo)
	}
	second := f.requests()[1]
	if second.url != "/v1/runs/"+runUUID+"/signals/payment-confirmed" || string(second.body) != `{"paymentId":"p-1"}` {
		t.Errorf("%s %s", second.url, second.body)
	}
	if second.headers.Get("Idempotency-Key") == "" {
		t.Error("a signal carries a key")
	}
	// Operate: the other actions and their options.
	if _, err := c.Operate(context.Background(), runUUID, client.ActionResolveStep, "an operator decided", client.WithStepID("s"), client.WithOutcome("FAILED"), client.WithErrorCode("E_X")); err != nil {
		t.Fatal(err)
	}
	if body := f.requests()[2].json(t); fmt.Sprint(body) != "map[action:RESOLVE_STEP errorCode:E_X outcome:FAILED reason:an operator decided stepId:s]" {
		t.Errorf("%v", body)
	}
}

func TestWaitingForCompletionPollsUntilTheRunIsTerminal(t *testing.T) {
	f := newFake(t)
	polls := 0
	f.answer(func(_ seen, w http.ResponseWriter) {
		polls++
		status := "RUNNING"
		if polls >= 3 {
			status = "SUCCEEDED"
		}
		reply(w, 200, run(map[string]any{"status": status}))
	})
	c := newClient(t, f)
	done, err := c.WaitForCompletion(context.Background(), runUUID, client.PollInterval(5*time.Millisecond), client.WaitTimeout(5*time.Second))
	if err != nil || done.Status != client.StatusSucceeded || polls != 3 {
		t.Fatalf("%v %v after %d polls", done, err, polls)
	}
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	_, err = c.WaitForCompletion(context.Background(), runUUID, client.PollInterval(5*time.Millisecond), client.WaitTimeout(30*time.Millisecond))
	if se := asSagat(t, err); se.Reason != "WAIT_TIMEOUT" {
		t.Errorf("%+v", se)
	}
}

func TestEveryTerminalStatusEndsTheWaitAndTheOthersDoNot(t *testing.T) {
	for _, s := range []client.RunStatus{client.StatusSucceeded, client.StatusFailed, client.StatusCancelled, client.StatusTimedOut, client.StatusCompensated, client.StatusPartial, client.StatusCompensationFailed} {
		if !s.Terminal() {
			t.Errorf("%s is terminal", s)
		}
	}
	for _, s := range []client.RunStatus{client.StatusRunning, client.StatusWaiting, client.StatusPaused, client.StatusCancelling, client.StatusCompensating, client.StatusManualReview, "SOMETHING_NEW"} {
		if s.Terminal() {
			t.Errorf("%s is not terminal", s)
		}
	}
}

func TestACancelledContextEndsTheCallWithCancelledThatWrapsTheContextsError(t *testing.T) {
	f := newFake(t)
	started := make(chan struct{})
	f.answer(func(_ seen, w http.ResponseWriter) {
		close(started)
		time.Sleep(300 * time.Millisecond)
		reply(w, 200, run(nil))
	})
	ctx, cancel := context.WithCancel(context.Background())
	go func() { <-started; cancel() }()
	began := time.Now()
	_, err := newClient(t, f).Get(ctx, runUUID)
	se := asSagat(t, err)
	if se.Code != sagat.CodeCancelled || !errors.Is(err, context.Canceled) || se.Retryable {
		t.Errorf("%+v", se)
	}
	if time.Since(began) > 250*time.Millisecond {
		t.Errorf("the call did not end with its context: %v", time.Since(began))
	}
	if n := len(f.requests()); n != 1 {
		t.Errorf("a cancelled call is not retried: %d", n)
	}

	// Cancelled while it waits to retry.
	g := newFake(t)
	g.answer(func(_ seen, w http.ResponseWriter) { reply(w, 503, "down") })
	slow, err := client.New(g.base(), "p", sagat.StaticToken("t"), client.WithRetry(sagat.RetryPolicy{InitialDelay: time.Minute, MaxDelay: time.Minute}))
	if err != nil {
		t.Fatal(err)
	}
	ctx2, cancel2 := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel2()
	_, err = slow.Get(ctx2, runUUID)
	if !sagat.IsCode(err, sagat.CodeCancelled) || !errors.Is(err, context.DeadlineExceeded) {
		t.Errorf("%v", err)
	}
}

func TestACallThatRunsOutOfTimeIsATransportErrorNotACancellation(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { time.Sleep(300 * time.Millisecond); reply(w, 200, run(nil)) })
	c := newClient(t, f, client.WithTimeout(50*time.Millisecond), client.WithRetry(sagat.RetryPolicy{MaxAttempts: 1}))
	_, err := c.Get(context.Background(), runUUID)
	if se := asSagat(t, err); se.Code != sagat.CodeTransport || !strings.Contains(se.Message, "timed out") {
		t.Errorf("%+v", se)
	}
}

func TestAServerThatDoesNotHaveTheCallIsToldFromAResourceThatIsMissing(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 404, "<html>no such route</html>") })
	_, err := newClient(t, f).Get(context.Background(), runUUID)
	if se := asSagat(t, err); se.Code != sagat.CodeNotFound || se.Reason != "UNSUPPORTED_BY_SERVER" || se.Retryable {
		t.Errorf("%+v", se)
	}
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 501, nil) })
	_, err = newClient(t, f).Get(context.Background(), runUUID)
	if se := asSagat(t, err); se.Code != sagat.CodeNotFound || se.Reason != "UNSUPPORTED_BY_SERVER" {
		t.Errorf("%+v", se)
	}
	// A run that does not exist is a 404 with the platform's own envelope.
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 404, map[string]any{"code": "NOT_FOUND", "message": "no such run", "retryable": false})
	})
	_, err = newClient(t, f).Get(context.Background(), runUUID)
	if se := asSagat(t, err); se.Code != sagat.CodeNotFound || se.Reason != "" {
		t.Errorf("%+v", se)
	}
}

func TestAnAnswerThatIsNotWhatTheContractSaysIsAProtocolError(t *testing.T) {
	f := newFake(t)
	c := newClient(t, f)
	for name, body := range map[string]string{
		"not json":         "this is not json",
		"not an object":    "[1,2]",
		"no run id":        `{"workflowId":"w","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z"}`,
		"null run id":      `{"runId":null,"workflowId":"w","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z"}`,
		"no creation time": `{"runId":"r","workflowId":"w","versionId":"v","status":"RUNNING"}`,
	} {
		f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, body) })
		if _, err := c.Get(context.Background(), runUUID); !sagat.IsCode(err, sagat.CodeProtocol) {
			t.Errorf("%s: %v", name, err)
		}
	}
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, `{"items": 5}`) })
	if _, err := c.List(context.Background(), client.ListOptions{}); !sagat.IsCode(err, sagat.CodeProtocol) {
		t.Errorf("a page without items: %v", err)
	}
}

func TestTheJournalOfARunIsReadAPageAtATime(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"items": []any{
			map[string]any{"eventType": "run.started"},
			map[string]any{"eventType": "step.leased", "stepId": "work", "attempt": 2},
		}, "nextCursor": "c9"})
	})
	page, err := newClient(t, f).Events(context.Background(), runUUID, client.EventsOptions{Limit: 200, Cursor: "c1"})
	if err != nil {
		t.Fatal(err)
	}
	if url := f.requests()[0].url; !strings.HasPrefix(url, "/v1/runs/"+runUUID+"/events?") || !strings.Contains(url, "limit=200") || !strings.Contains(url, "cursor=c1") {
		t.Errorf("%s", url)
	}
	if len(page.Items) != 2 || page.Items[0].Type != "run.started" || page.Items[0].Attempt != nil || page.Items[1].StepID != "work" || *page.Items[1].Attempt != 2 || page.NextCursor != "c9" {
		t.Errorf("%+v", page)
	}
}

func TestTheConfigurationIsCheckedAndAWarningIsLoggedForPlainHTTPOutsideLoopback(t *testing.T) {
	for name, args := range map[string][3]string{
		"no api url": {"", "ns", "t"}, "no namespace": {"http://127.0.0.1/v1", "", "t"}, "not a url": {"ftp://x/v1", "ns", "t"},
	} {
		if _, err := client.New(args[0], args[1], sagat.StaticToken(args[2])); !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("%s: %v", name, err)
		}
	}
	if _, err := client.New("http://127.0.0.1/v1", "ns", nil); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("no token source: %v", err)
	}
	var logged bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&logged, nil))
	if _, err := client.New("http://127.0.0.1:1/v1", "ns", sagat.StaticToken("super-secret-token"), client.WithLogger(logger)); err != nil {
		t.Fatal(err)
	}
	if logged.Len() != 0 {
		t.Errorf("loopback is fine: %s", logged.String())
	}
	if _, err := client.New("http://api.example.com/v1", "ns", sagat.StaticToken("super-secret-token"), client.WithLogger(logger)); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(logged.String(), "without TLS") || strings.Contains(logged.String(), "super-secret-token") {
		t.Errorf("%s", logged.String())
	}
	// The user agent takes a suffix.
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	if _, err := newClient(t, f, client.WithUserAgent("my-app/2.0")).Get(context.Background(), runUUID); err != nil {
		t.Fatal(err)
	}
	if ua := f.requests()[0].headers.Get("User-Agent"); !strings.HasSuffix(ua, " my-app/2.0") || !strings.HasPrefix(ua, "sagat-sdk-go/") {
		t.Errorf("%q", ua)
	}
}

func TestTheInstantsOfARunAreUTCWhateverTheZoneOfTheMachineAndTheServersOffset(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, run(map[string]any{"createdAt": "2026-10-10T15:30:00+05:30", "finishedAt": "2026-10-10T10:00:05.123456Z"}))
	})
	got, err := newClient(t, f).Get(context.Background(), runUUID)
	if err != nil {
		t.Fatal(err)
	}
	if got.CreatedAt.Format(time.RFC3339) != "2026-10-10T10:00:00Z" || got.CreatedAt.Location() != time.UTC || got.FinishedAt.UTC() != got.FinishedAt {
		t.Errorf("%v %v", got.CreatedAt, got.FinishedAt)
	}
}
