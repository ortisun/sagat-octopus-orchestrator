package worker_test

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"google.golang.org/grpc/codes"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

// ---- a server that refuses every session is not hammered

func TestAServerThatRegistersAWorkerAndRefusesItAtOnceIsNotHammered(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.always["PollTasks"] = failure{code: codes.Unauthenticated} })
	// The default retry policy: this is the backoff a user gets.
	h, err := worker.New(f.url(), "payments", "w", sagat.StaticToken("tok"))
	must(t, err)
	must(t, h.RegisterActivity("charge", rawOK))
	must(t, h.Start(context.Background()))
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	time.Sleep(2 * time.Second)
	registers, polls := len(f.of("RegisterWorker")), len(f.of("PollTasks"))
	t.Logf("%d registrations and %d polls in 2 s", registers, polls)
	if registers > 10 || registers < 2 {
		t.Errorf("%d registrations and %d polls in 2 s: the server was hammered (or the host gave up)", registers, polls)
	}
}

func TestTheBackoffAfterARefusedSessionIsResetByASuccessfulPoll(t *testing.T) {
	f := newFakeService(t)
	h, err := worker.New(f.url(), "payments", "w", sagat.StaticToken("tok"), worker.WithRetry(sagat.RetryPolicy{InitialDelay: 400 * time.Millisecond, MaxDelay: 400 * time.Millisecond}))
	must(t, err)
	must(t, h.RegisterActivity("charge", rawOK))
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	f.set(func(f *fakeService) { f.pollRetryAfterMs = 20 })
	// Two refusals in a row: the second registration waits for the backoff.
	f.fail("PollTasks", failure{code: codes.Unauthenticated}, failure{code: codes.Unauthenticated})
	must(t, h.Start(context.Background()))
	until(t, "three registrations", func() bool { return len(f.of("RegisterWorker")) == 3 })
	polls := f.of("PollTasks")
	registers := f.of("RegisterWorker")
	if gap := registers[2].at.Sub(polls[1].at); gap < 150*time.Millisecond {
		t.Errorf("the second refusal in a row was answered after %v: it should have waited for the backoff", gap)
	}
	if gap := registers[1].at.Sub(polls[0].at); gap > 150*time.Millisecond {
		t.Errorf("the first refusal waits for nothing, and waited %v", gap)
	}
	// A poll that succeeded resets it: the next refusal is answered at once.
	until(t, "a poll that succeeded", func() bool { return len(f.of("PollTasks")) >= 5 })
	f.fail("PollTasks", failure{code: codes.Unauthenticated})
	before := len(f.of("PollTasks"))
	until(t, "the fourth registration", func() bool { return len(f.of("RegisterWorker")) == 4 })
	polls = f.of("PollTasks")
	registers = f.of("RegisterWorker")
	refused := polls[before]
	if gap := registers[3].at.Sub(refused.at); gap > 150*time.Millisecond {
		t.Errorf("after a good poll the first refusal waited %v", gap)
	}
}

// ---- a server that sends more than was asked for

func TestAPollAnswerWithMoreTasksThanTheHostAskedForRunsOnlyWhatItAskedFor(t *testing.T) {
	f := newFakeService(t)
	var logged bytes.Buffer
	var mu sync.Mutex
	h := newHost(t, f, worker.WithMaxInflight(2), worker.WithLogger(slog.New(slog.NewTextHandler(&lockedWriter{&mu, &logged}, nil))))
	var running, peak, total atomic.Int32
	must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) {
		total.Add(1)
		now := running.Add(1)
		for {
			old := peak.Load()
			if now <= old || peak.CompareAndSwap(old, now) {
				break
			}
		}
		time.Sleep(150 * time.Millisecond)
		running.Add(-1)
		return 1, nil
	}))
	f.set(func(f *fakeService) { f.overfill = true })
	for i := 1; i <= 6; i++ {
		f.lease(lease(func(l *pb.TaskLease) {
			l.TaskId, l.LeaseToken = fmt.Sprintf("run-%d/charge/1", i), fmt.Sprintf("LEASE-TOKEN-%d", i)
		}))
	}
	start(t, h)
	until(t, "the tasks that were kept", func() bool { return len(f.of("CompleteStep")) == 2 })
	time.Sleep(300 * time.Millisecond)
	if peak.Load() > 2 || total.Load() != 2 || len(f.of("CompleteStep")) != 2 {
		t.Errorf("peak %d, %d handlers, %d reports: only the first 2 of 6 may run", peak.Load(), total.Load(), len(f.of("CompleteStep")))
	}
	mu.Lock()
	defer mu.Unlock()
	text := logged.String()
	if !strings.Contains(text, "4") || strings.Contains(text, "LEASE-TOKEN") || strings.Contains(text, "run-3/charge") {
		t.Errorf("the log says how many were left, and nothing more: %s", text)
	}
}

// ---- a secret that could not be resolved is asked for again

func TestAFailedResolutionOfSecretsIsNotCachedForTheRestOfTheAttempt(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	var first, second error
	var value string
	must(t, h.RegisterActivity("pay", func(ctx context.Context, _ json.RawMessage) (any, error) {
		task := worker.TaskFrom(ctx)
		_, first = task.Secrets(ctx)
		secrets, err := task.Secrets(ctx)
		second = err
		if err == nil {
			value, _ = secrets.Text("k")
		}
		return 1, nil
	}))
	f.set(func(f *fakeService) { f.secrets = []*pb.ResolvedSecret{{Name: "k", Value: []byte("v")}} })
	f.fail("ResolveSecrets", failure{code: codes.Unavailable})
	f.lease(lease(forActivity("pay", "1")))
	start(t, h)
	until(t, "the completion", func() bool { return len(f.of("CompleteStep")) == 1 })
	if first == nil || second != nil || value != "v" || len(f.of("ResolveSecrets")) != 2 {
		t.Errorf("first %v, second %v, value %q, %d resolutions", first, second, value, len(f.of("ResolveSecrets")))
	}
}

// ---- a credential is never printed

func TestAHostAndItsSecretsNeverPrintACredentialInAnyFormat(t *testing.T) {
	const token, secret = "CANARY-BEARER-TOKEN-77", "CANARY-SECRET-VALUE-91"
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	keyMaterial := []string{key.D.String(), fmt.Sprintf("%x", key.D)}
	f := newFakeService(t)
	h, err := worker.New(f.url(), "payments", "w", sagat.StaticToken(token),
		worker.WithTLSConfig(&tls.Config{Certificates: []tls.Certificate{{Certificate: [][]byte{{1}}, PrivateKey: key}}}),
		worker.WithDialOptions())
	must(t, err)
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	var secrets *worker.Secrets
	must(t, h.RegisterActivity("pay", func(ctx context.Context, _ json.RawMessage) (any, error) {
		s, err := worker.TaskFrom(ctx).Secrets(ctx)
		secrets = s
		if err != nil {
			return nil, err
		}
		// While the handler holds them: every way to print them.
		var out []string
		for _, verb := range []string{"%v", "%+v", "%#v", "%s", "%q", "%d", "%x"} {
			out = append(out, fmt.Sprintf(verb, s))
		}
		for _, o := range out {
			if strings.Contains(o, secret) {
				return nil, errors.New("a secret was printed: " + o)
			}
		}
		return 1, nil
	}))
	f.set(func(f *fakeService) { f.secrets = []*pb.ResolvedSecret{{Name: "k", Value: []byte(secret)}} })
	var logged bytes.Buffer
	var outputs []string
	for _, verb := range []string{"%v", "%+v", "%#v", "%s", "%q", "%d", "%x"} {
		outputs = append(outputs, fmt.Sprintf(verb, h), fmt.Sprintf(verb, struct{ H *worker.Host }{h}), fmt.Sprintf(verb, sagat.StaticToken(token)))
	}
	slog.New(slog.NewTextHandler(&logged, nil)).Info("host", "host", h, "token", sagat.StaticToken(token))
	slog.New(slog.NewJSONHandler(&logged, nil)).Info("host", "host", h, "token", sagat.StaticToken(token))
	marshalled, _ := json.Marshal(map[string]any{"host": h, "token": sagat.StaticToken(token)})
	outputs = append(outputs, logged.String(), string(marshalled))
	f.lease(lease(forActivity("pay", "1")))
	start(t, h)
	until(t, "the completion", func() bool { return len(f.of("CompleteStep")) == 1 })
	outputs = append(outputs, fmt.Sprintf("%v %+v %#v", h, h, h), fmt.Sprintf("%v %+v %#v", secrets, secrets, secrets))
	for _, out := range outputs {
		for _, canary := range append([]string{token, secret}, keyMaterial...) {
			if strings.Contains(out, canary) {
				t.Errorf("%q is in %q", canary, out)
			}
		}
	}
}

// ---- a hook that panics does not take the host with it

type panicHandler struct{ slog.Handler }

func (panicHandler) Enabled(context.Context, slog.Level) bool  { panic("the logger panicked") }
func (panicHandler) Handle(context.Context, slog.Record) error { panic("the logger panicked") }
func (p panicHandler) WithAttrs([]slog.Attr) slog.Handler      { panic("the logger panicked") }
func (p panicHandler) WithGroup(string) slog.Handler           { panic("the logger panicked") }

func TestHooksThatPanicDoNotTakeTheHostWithIt(t *testing.T) {
	f := newFakeService(t)
	var failing atomic.Bool
	provider := sagat.TokenFunc(func(context.Context) (string, error) {
		if failing.Load() {
			panic("the vault exploded: SECRET-IN-PANIC")
		}
		return "tok", nil
	})
	h, err := worker.New(f.url(), "payments", "w", provider,
		worker.WithRetry(sagat.RetryPolicy{InitialDelay: 5 * time.Millisecond, MaxDelay: 20 * time.Millisecond}),
		worker.WithHeartbeatInterval(10*time.Millisecond),
		worker.WithLogger(slog.New(panicHandler{})),
		worker.WithEventHandler(func(worker.Event) { panic("the metrics hook panicked") }))
	must(t, err)
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	must(t, h.RegisterActivity("slow", func(ctx context.Context, _ json.RawMessage) (any, error) {
		// The heartbeats ask the provider for a token while it panics.
		failing.Store(true)
		time.Sleep(100 * time.Millisecond)
		failing.Store(false)
		return map[string]bool{"ok": true}, nil
	}))
	f.lease(lease(forActivity("slow", "1")))
	must(t, h.Start(context.Background()))
	until(t, "the completion", func() bool { return len(f.of("CompleteStep")) == 1 })
}
