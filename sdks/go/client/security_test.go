package client_test

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
	"strings"
	"testing"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
)

const runUUID = "0190aaaa-0000-7000-8000-000000000001"

// ---- a credential is never printed

func TestAClientNeverPrintsACredentialInAnyFormat(t *testing.T) {
	const canary = "CANARY-BEARER-TOKEN-8f3a"
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	keyMaterial := []string{key.D.String(), fmt.Sprintf("%x", key.D)}
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	c, err := client.New(f.base(), "payments", sagat.StaticToken(canary),
		client.WithTLSConfig(&tls.Config{Certificates: []tls.Certificate{{Certificate: [][]byte{{1, 2, 3}}, PrivateKey: key}}}),
		client.WithUserAgent("my-app"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := c.Get(context.Background(), runUUID); err != nil {
		t.Fatal(err)
	}
	var logged bytes.Buffer
	text := slog.New(slog.NewTextHandler(&logged, nil))
	jsonLog := slog.New(slog.NewJSONHandler(&logged, nil))
	text.Info("client", "client", c, "token", sagat.StaticToken(canary))
	jsonLog.Info("client", "client", c, "token", sagat.StaticToken(canary))
	marshalled, err := json.Marshal(map[string]any{"client": c, "token": sagat.StaticToken(canary)})
	if err != nil {
		t.Fatal(err)
	}
	outputs := []string{logged.String(), string(marshalled)}
	for _, verb := range []string{"%v", "%+v", "%#v", "%s", "%q", "%d", "%x", "%T"} {
		outputs = append(outputs, fmt.Sprintf(verb, c), fmt.Sprintf(verb, sagat.StaticToken(canary)), fmt.Sprintf(verb, []any{c, sagat.StaticToken(canary)}))
	}
	outputs = append(outputs, fmt.Sprint(c), fmt.Sprintln(c), fmt.Sprintf("%v", struct{ C *client.Client }{c}), fmt.Sprintf("%+v", struct{ C *client.Client }{c}))
	for _, out := range outputs {
		for _, secret := range append([]string{canary}, keyMaterial...) {
			if strings.Contains(out, secret) {
				t.Errorf("%q is in %q", secret, out)
			}
		}
	}
}

// ---- ids and names in a path

func TestARunIDThatIsNotAUUIDIsRefusedBeforeAnyRequest(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	c := newClient(t, f)
	ctx := context.Background()
	calls := map[string]func(id string) error{
		"get":    func(id string) error { _, err := c.Get(ctx, id); return err },
		"events": func(id string) error { _, err := c.Events(ctx, id, client.EventsOptions{}); return err },
		"cancel": func(id string) error { _, err := c.Cancel(ctx, id, "no longer needed"); return err },
		"operate": func(id string) error {
			_, err := c.Operate(ctx, id, client.ActionPause, "an operator paused it")
			return err
		},
		"signal": func(id string) error { _, err := c.Signal(ctx, id, "go", nil); return err },
		"wait": func(id string) error {
			_, err := c.WaitForCompletion(ctx, id, client.WaitTimeout(time.Second))
			return err
		},
	}
	bad := []string{"", ".", "..", "../operations", "r-1", "../../v1/workflows", runUUID + "/operations", runUUID + "?x=1", runUUID + "#", "%2e%2e",
		"0190aaaa-0000-7000-8000-00000000000g", " " + runUUID, runUUID + "\n"}
	for name, call := range calls {
		for _, id := range bad {
			if err := call(id); !sagat.IsCode(err, sagat.CodeValidationFailed) {
				t.Errorf("%s(%q): %v", name, id, err)
			}
		}
	}
	if n := len(f.requests()); n != 0 {
		t.Fatalf("%d requests were sent for ids that are not UUIDs: %s", n, f.requests()[0].url)
	}
	// A UUID in either case is a run id.
	for _, id := range []string{runUUID, strings.ToUpper(runUUID)} {
		if err := calls["get"](id); err != nil {
			t.Errorf("%s: %v", id, err)
		}
	}
}

func TestASignalNameOfDotDotDotOrNothingIsRefusedAndAnyOtherNameStaysPercentEncoded(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, map[string]any{"deliveredTo": []any{}}) })
	c := newClient(t, f)
	for _, name := range []string{"", ".", ".."} {
		if _, err := c.Signal(context.Background(), runUUID, name, nil); !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("%q: %v", name, err)
		}
	}
	if n := len(f.requests()); n != 0 {
		t.Fatalf("%d requests were sent: %s", n, f.requests()[0].url)
	}
	want := map[string]string{
		"payment.confirmed": "payment.confirmed", "a/b": "a%2Fb", "../x": "..%2Fx", "a b": "a%20b", "q?x=1#f": "q%3Fx=1%23f",
		"100%": "100%25", "é": "%C3%A9", "...": "...", "a\\b": "a%5Cb",
	}
	for name, encoded := range want {
		f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, map[string]any{"deliveredTo": []any{}}) })
		if _, err := c.Signal(context.Background(), runUUID, name, nil); err != nil {
			t.Errorf("%q: %v", name, err)
			continue
		}
		if got := f.requests()[0].url; got != "/v1/runs/"+runUUID+"/signals/"+encoded {
			t.Errorf("%q was sent as %s, want .../signals/%s", name, got, encoded)
		}
	}
}

// ---- what the server says is bounded

func TestTheMessageOfAServerErrorIsCutTo512CharactersAndATraceIDThatIsNotHexIsNotRepeated(t *testing.T) {
	f := newFake(t)
	c := newClient(t, f, client.WithRetry(sagat.RetryPolicy{MaxAttempts: 1}))
	long := strings.Repeat("é", 5000)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 422, map[string]any{"code": "VALIDATION_FAILED", "message": long, "traceId": "../../etc/passwd Bearer abc"})
	})
	_, err := c.Get(context.Background(), runUUID)
	se := asSagat(t, err)
	if n := len([]rune(se.Message)); n != 512 {
		t.Errorf("the message has %d characters", n)
	}
	if se.TraceID != "" {
		t.Errorf("a trace id that is not 32 hex characters was repeated: %q", se.TraceID)
	}
	for name, id := range map[string]string{
		"short": strings.Repeat("a", 31), "long": strings.Repeat("a", 33), "not hex": strings.Repeat("g", 32), "newline": strings.Repeat("a", 31) + "\n",
	} {
		f.answer(func(_ seen, w http.ResponseWriter) {
			reply(w, 404, map[string]any{"code": "NOT_FOUND", "message": "no", "traceId": id}, "X-Trace-Id", id)
		})
		_, err := c.Get(context.Background(), runUUID)
		if se := asSagat(t, err); se.TraceID != "" {
			t.Errorf("%s: %q", name, se.TraceID)
		}
	}
	// A header that is not hex is not repeated either, and a good one is.
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 503, "down", "X-Trace-Id", "<script>") })
	if _, err := c.Get(context.Background(), runUUID); asSagat(t, err).TraceID != "" {
		t.Error("the header was repeated")
	}
	good := strings.Repeat("ab12", 8)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 503, "down", "X-Trace-Id", strings.ToUpper(good)) })
	if _, err := c.Get(context.Background(), runUUID); asSagat(t, err).TraceID != good {
		t.Errorf("a good trace id is kept, lower case")
	}
	// A message that is not valid UTF-8 is repaired, not repeated as it came.
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 422, "{\"code\":\"VALIDATION_FAILED\",\"message\":\"bad \xff byte\"}")
	})
	if _, err := c.Get(context.Background(), runUUID); !strings.Contains(asSagat(t, err).Message, "bad") {
		t.Error("the message is kept")
	}
}

func TestAWaitTheServerAsksForIsCappedAt60Seconds(t *testing.T) {
	f := newFake(t)
	c := newClient(t, f, client.WithRetry(sagat.RetryPolicy{MaxAttempts: 1}))
	for name, header := range map[string]string{"an hour": "3600", "the largest": "4294967295", "a minute": "60", "ten seconds": "10"} {
		f.answer(func(_ seen, w http.ResponseWriter) {
			reply(w, 429, map[string]any{"code": "RATE_LIMITED", "message": "slow", "retryable": true}, "Retry-After", header)
		})
		_, err := c.Get(context.Background(), runUUID)
		se := asSagat(t, err)
		want := 60 * time.Second
		if header == "10" {
			want = 10 * time.Second
		}
		if se.RetryAfter == nil || *se.RetryAfter != want {
			t.Errorf("%s: %v, want %v", name, se.RetryAfter, want)
		}
	}
}

func TestACallThatIsAskedToWaitAnHourDoesNotWaitMoreThanAMinuteAndNotAtAllWhenItsContextEnds(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 429, map[string]any{"code": "RATE_LIMITED", "message": "slow", "retryable": true}, "Retry-After", "3600")
	})
	c := newClient(t, f)
	ctx, cancel := context.WithTimeout(context.Background(), 200*time.Millisecond)
	defer cancel()
	began := time.Now()
	_, err := c.Get(ctx, runUUID)
	if !sagat.IsCode(err, sagat.CodeCancelled) || time.Since(began) > 2*time.Second {
		t.Errorf("%v after %v", err, time.Since(began))
	}
}

func TestAnAnswerBeyondTheBoundIsAProtocolErrorNotATransportOne(t *testing.T) {
	restore := client.SetMaxAnswer(1000)
	defer restore()
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, `{"runId":"`+strings.Repeat("x", 5000)+`"}`)
	})
	_, err := newClient(t, f).Get(context.Background(), runUUID)
	if se := asSagat(t, err); se.Code != sagat.CodeProtocol || se.Retryable {
		t.Errorf("%+v", se)
	}
	if n := len(f.requests()); n != 1 {
		t.Errorf("an answer that is too large is not retried: %d requests", n)
	}
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 500, strings.Repeat("y", 5000)) })
	if _, err := newClient(t, f).Get(context.Background(), runUUID); !sagat.IsCode(err, sagat.CodeProtocol) {
		t.Errorf("an error body that is too large: %v", err)
	}
	// Just under the bound is read.
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	if _, err := newClient(t, f).Get(context.Background(), runUUID); err == nil {
		t.Log("a run fits in 1000 bytes")
	}
}

// ---- a hook that panics does not take the client with it

type panicHandler struct{ slog.Handler }

func (panicHandler) Enabled(context.Context, slog.Level) bool  { panic("the logger panicked") }
func (panicHandler) Handle(context.Context, slog.Record) error { panic("the logger panicked") }
func (p panicHandler) WithAttrs([]slog.Attr) slog.Handler      { panic("the logger panicked") }
func (p panicHandler) WithGroup(string) slog.Handler           { panic("the logger panicked") }

func TestAHookThatPanicsDoesNotTakeTheClientWithIt(t *testing.T) {
	f := newFake(t)
	attempts := 0
	f.answer(func(_ seen, w http.ResponseWriter) {
		attempts++
		if attempts == 1 {
			reply(w, 503, "down") // makes the client log that it retries
			return
		}
		reply(w, 200, run(nil))
	})
	// A logger that panics: the call still works, and is retried.
	loud := newClient(t, f, client.WithLogger(slog.New(panicHandler{})))
	if _, err := loud.Get(context.Background(), runUUID); err != nil {
		t.Errorf("a panicking logger: %v", err)
	}
	// A token provider that panics is a failed token source, not a crash.
	provider, err := client.New(f.base(), "payments", sagat.TokenFunc(func(context.Context) (string, error) { panic("the vault exploded: SECRET-IN-PANIC") }))
	if err != nil {
		t.Fatal(err)
	}
	_, err = provider.Get(context.Background(), runUUID)
	if se := asSagat(t, err); se.Code != sagat.CodeUnauthenticated || strings.Contains(se.Error(), "SECRET-IN-PANIC") {
		t.Errorf("%+v", se)
	}
	// A trace hook that panics sends no trace.
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	traced := newClient(t, f, client.WithTraceContext(func(context.Context) string { panic("the tracer panicked") }))
	if _, err := traced.Get(context.Background(), runUUID); err != nil {
		t.Errorf("a panicking trace hook: %v", err)
	}
	if got := f.requests()[0].headers.Get("Traceparent"); got != "" {
		t.Errorf("%q", got)
	}
}
