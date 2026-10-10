package worker_test

import (
	"context"
	"encoding/json"
	"errors"
	"sync/atomic"
	"testing"
	"time"

	"google.golang.org/grpc/codes"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

// SPEC section 5: a result that was computed is not lost to a hiccup of the identity provider: the report is
// repeated, because nothing was sent.
func TestAResultIsNotLostWhenTheTokenSourceFailsOnceAtReportTime(t *testing.T) {
	for name, fail := range map[string]func() (string, error){
		"an error": func() (string, error) { return "", errors.New("the vault is restarting") },
		"a panic":  func() (string, error) { panic("the vault exploded") },
	} {
		t.Run(name, func(t *testing.T) {
			f := newFakeService(t)
			var failNext atomic.Bool
			var failures atomic.Int32
			source := sagat.TokenFunc(func(context.Context) (string, error) {
				if failNext.CompareAndSwap(true, false) {
					failures.Add(1)
					return fail()
				}
				return "tok", nil
			})
			h, err := worker.New(f.url(), "payments", "w", source, worker.WithMaxInflight(1), worker.WithHeartbeatInterval(time.Minute),
				worker.WithRetry(sagat.RetryPolicy{InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}))
			must(t, err)
			t.Cleanup(func() {
				ctx, cancel := context.WithTimeout(context.Background(), time.Second)
				defer cancel()
				_ = h.Stop(ctx)
			})
			must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) {
				failNext.Store(true) // the very next token the host asks for is the one of the report
				return map[string]bool{"ok": true}, nil
			}))
			f.lease(lease())
			must(t, h.Start(context.Background()))
			until(t, "the report", func() bool { return len(f.of("CompleteStep")) == 1 })
			if failures.Load() != 1 || len(f.of("FailStep")) != 0 {
				t.Errorf("the source failed %d times, %d failures reported", failures.Load(), len(f.of("FailStep")))
			}
			if r := f.of("CompleteStep")[0].request.(*pb.CompleteStepRequest); r.GetIdempotencyKey() != "done-run-1/charge/1" || r.GetOutputJson() != `{"ok":true}` {
				t.Errorf("%v", r)
			}
		})
	}
}

func TestAFailureIsNotLostToATokenSourceThatFailsOnceEither(t *testing.T) {
	f := newFakeService(t)
	var failNext atomic.Bool
	source := sagat.TokenFunc(func(context.Context) (string, error) {
		if failNext.CompareAndSwap(true, false) {
			return "", errors.New("the vault is restarting")
		}
		return "tok", nil
	})
	h, err := worker.New(f.url(), "payments", "w", source, worker.WithMaxInflight(1), worker.WithHeartbeatInterval(time.Minute),
		worker.WithRetry(sagat.RetryPolicy{InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}))
	must(t, err)
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) {
		failNext.Store(true)
		return nil, worker.Permanent("NO_FUNDS", "nothing to charge")
	}))
	f.lease(lease())
	must(t, h.Start(context.Background()))
	until(t, "the failure", func() bool { return len(f.of("FailStep")) == 1 })
	if r := f.of("FailStep")[0].request.(*pb.FailStepRequest); r.GetErrorCode() != "NO_FUNDS" || r.GetIdempotencyKey() != "failed-run-1/charge/1" {
		t.Errorf("%v", r)
	}
}

func TestARegistrationWhoseTokenSourceFailsOnceIsRepeated(t *testing.T) {
	f := newFakeService(t)
	var asked atomic.Int32
	source := sagat.TokenFunc(func(context.Context) (string, error) {
		if asked.Add(1) == 1 {
			return "", errors.New("the vault is restarting")
		}
		return "tok", nil
	})
	h, err := worker.New(f.url(), "payments", "w", source, worker.WithRetry(sagat.RetryPolicy{InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}))
	must(t, err)
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	must(t, h.RegisterActivity("charge", rawOK))
	must(t, h.Start(context.Background()))
	if n := len(f.of("RegisterWorker")); n != 1 {
		t.Errorf("%d registrations reached the service: nothing was sent by the attempt that failed", n)
	}
}

func TestATokenSourceThatKeepsFailingGivesUpAfterTheReportAttemptsWithTheReason(t *testing.T) {
	f := newFakeService(t)
	var failing atomic.Bool
	source := sagat.TokenFunc(func(context.Context) (string, error) {
		if failing.Load() {
			return "", errors.New("the vault is down")
		}
		return "tok", nil
	})
	events := make(chan worker.Event, 100)
	h, err := worker.New(f.url(), "payments", "w", source, worker.WithMaxInflight(1), worker.WithHeartbeatInterval(time.Minute),
		worker.WithRetry(sagat.RetryPolicy{InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}),
		worker.WithEventHandler(func(e worker.Event) { events <- e }))
	must(t, err)
	t.Cleanup(func() {
		failing.Store(false)
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) { failing.Store(true); return 1, nil }))
	f.lease(lease())
	must(t, h.Start(context.Background()))
	dropped := waitForEvent(t, events, worker.EventDropped)
	if dropped.Code != string(sagat.CodeUnauthenticated) || len(f.of("CompleteStep")) != 0 {
		t.Errorf("%+v after %d reports", dropped, len(f.of("CompleteStep")))
	}
}

// A reason in a server's answer is the server's: it never makes a call repeatable.
func TestAReasonOfTheTokenProviderInAServersAnswerNeverMakesAReportOrARegistrationRepeatable(t *testing.T) {
	for _, code := range []codes.Code{codes.InvalidArgument, codes.Unauthenticated, codes.PermissionDenied, codes.Aborted, codes.FailedPrecondition, codes.NotFound} {
		for _, reason := range []string{"TOKEN_PROVIDER_FAILED", "TOKEN_SOURCE_FAILED"} {
			f := newFakeService(t)
			f.set(func(f *fakeService) {
				f.always["CompleteStep"] = failure{code: code, reason: reason}
				f.always["RegisterWorker"] = failure{code: code, reason: reason}
			})
			// A static token cannot be refreshed, so the registration is not repeated for UNAUTHENTICATED either.
			h, err := worker.New(f.url(), "payments", "w", sagat.StaticToken("tok"), worker.WithRetry(sagat.RetryPolicy{InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}))
			must(t, err)
			must(t, h.RegisterActivity("charge", rawOK))
			if err := h.Start(context.Background()); err == nil {
				t.Fatalf("%v: the registration was accepted", code)
			}
			if n := len(f.of("RegisterWorker")); n != 1 {
				t.Errorf("%v with the reason %s: the registration was repeated, %d calls", code, reason, n)
			}

			g := newFakeService(t)
			g.set(func(g *fakeService) { g.always["CompleteStep"] = failure{code: code, reason: reason} })
			events := make(chan worker.Event, 100)
			h2, err := worker.New(g.url(), "payments", "w", sagat.StaticToken("tok"), worker.WithEventHandler(func(e worker.Event) { events <- e }),
				worker.WithRetry(sagat.RetryPolicy{InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}))
			must(t, err)
			must(t, h2.RegisterActivity("charge", rawOK))
			g.lease(lease())
			must(t, h2.Start(context.Background()))
			until(t, "the report", func() bool { return len(g.of("CompleteStep")) >= 1 })
			time.Sleep(60 * time.Millisecond)
			if n := len(g.of("CompleteStep")); n != 1 {
				t.Errorf("%v with the reason %s: the report was repeated, %d calls", code, reason, n)
			}
			ctx, cancel := context.WithTimeout(context.Background(), time.Second)
			_ = h2.Stop(ctx)
			cancel()
		}
	}
}

// A token source that fails is not a session that is gone: the host does not register again for it.
func TestAPollWhoseTokenSourceFailsOnceDoesNotRegisterAgainAndTheHostGoesOn(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.pollRetryAfterMs = 20 })
	var failNext atomic.Bool
	source := sagat.TokenFunc(func(context.Context) (string, error) {
		if failNext.CompareAndSwap(true, false) {
			return "", errors.New("the vault is restarting")
		}
		return "tok", nil
	})
	h, err := worker.New(f.url(), "payments", "w", source, worker.WithRetry(sagat.RetryPolicy{InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}))
	must(t, err)
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	must(t, h.RegisterActivity("charge", rawOK))
	must(t, h.Start(context.Background()))
	until(t, "a poll", func() bool { return len(f.of("PollTasks")) >= 1 })
	polls := len(f.of("PollTasks"))
	failNext.Store(true)
	until(t, "the host polls on after the failure", func() bool { return len(f.of("PollTasks")) >= polls+3 && !failNext.Load() })
	if n := len(f.of("RegisterWorker")); n != 1 {
		t.Errorf("%d registrations: a token source that failed was taken for a session that is gone", n)
	}
	f.lease(lease())
	until(t, "a task after it", func() bool { return len(f.of("CompleteStep")) == 1 })
}
