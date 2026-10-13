package worker_test

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"runtime"
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

func ok(any) (any, error) { return 1, nil }

func rawOK(context.Context, json.RawMessage) (any, error) { return 1, nil }

func TestAWorkerRegistersItsActivitiesAndTheNewestProtocolItSpeaksWithTheTokenAsMetadata(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f, worker.WithMaxInflight(7))
	must(t, h.RegisterActivity("b.second", rawOK))
	must(t, h.RegisterCompensation("a.undo", rawOK))
	must(t, h.RegisterReconciler("c.lookup", func(context.Context, json.RawMessage) (worker.Verdict, error) { return worker.Unknown(), nil }))
	start(t, h)
	registers := f.of("RegisterWorker")
	if len(registers) != 1 {
		t.Fatalf("%d registrations", len(registers))
	}
	request := registers[0].request.(*pb.RegisterWorkerRequest)
	capabilities := request.GetCapabilities()
	if request.GetWorkerName() != "test-worker" || request.GetNamespace() != "payments" ||
		fmt.Sprint(capabilities.GetLanguages()) != "[go]" || fmt.Sprint(capabilities.GetActivities()) != "[a.undo b.second c.lookup]" ||
		len(capabilities.GetPools()) != 0 || capabilities.GetMaxInflight() != 7 || capabilities.GetProtocolVersion() != "1.4" {
		t.Errorf("%v", request)
	}
	if got := registers[0].md.Get("authorization"); len(got) != 1 || got[0] != "Bearer tok" {
		t.Errorf("authorization %v", got)
	}
	if h.ProtocolVersion() != "1.2" {
		t.Errorf("protocol %q", h.ProtocolVersion())
	}
}

func TestATaskIsRunWithItsContextAndCompletedOnceWithAKeyThatIsTheSameOnEveryRetry(t *testing.T) {
	f := newFakeService(t)
	var events []worker.Event
	var mu sync.Mutex
	h := newHost(t, f, worker.WithEventHandler(func(e worker.Event) { mu.Lock(); events = append(events, e); mu.Unlock() }))
	type chargeInput struct {
		Amount int `json:"amount"`
	}
	var seen *worker.Task
	var received chargeInput
	must(t, h.RegisterActivity("charge", worker.Typed(func(ctx context.Context, in chargeInput) (map[string]int, error) {
		seen = worker.TaskFrom(ctx)
		received = in
		return map[string]int{"charged": in.Amount * 2}, nil
	})))
	f.fail("CompleteStep", failure{code: codes.Unavailable}, failure{code: codes.ResourceExhausted, reason: "RATE_LIMITED", retryAfter: "1"})
	f.lease(lease())
	start(t, h)
	until(t, "three attempts to report", func() bool { return len(f.of("CompleteStep")) == 3 })
	if received.Amount != 5 {
		t.Errorf("input %+v", received)
	}
	if seen == nil || seen.RunID != "run-1" || seen.Node != "charge" || seen.StepID != "charge" || seen.Attempt != 1 || seen.IdempotencyKey != "run-1:charge" ||
		seen.Kind != worker.KindForward || seen.Traceparent != traceparent || string(seen.Steps["earlier"]) != `{"ok":true}` || seen.ActivityName != "charge" {
		t.Errorf("%+v", seen)
	}
	keys := map[string]bool{}
	for _, c := range f.of("CompleteStep") {
		keys[c.request.(*pb.CompleteStepRequest).GetIdempotencyKey()] = true
	}
	if len(keys) != 1 || !keys["done-run-1/charge/1"] {
		t.Errorf("keys %v", keys)
	}
	last := f.of("CompleteStep")[2].request.(*pb.CompleteStepRequest)
	if last.GetOutputJson() != `{"charged":10}` || last.GetLeaseToken() != "lt" || last.GetGeneration() != 1 || last.GetTraceparent() != traceparent || last.GetAttempt() != 1 {
		t.Errorf("%v", last)
	}
	until(t, "the completed event", func() bool {
		mu.Lock()
		defer mu.Unlock()
		for _, e := range events {
			if e.Type == worker.EventCompleted && e.TaskID == "run-1/charge/1" {
				return true
			}
		}
		return false
	})
}

func TestAFailureOnPurposeCarriesItsCodeAndWhetherItMayBeRetriedAndAnyOtherErrorSaysOnlyItsType(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	must(t, h.RegisterActivity("permanent", func(context.Context, json.RawMessage) (any, error) {
		return nil, worker.Permanent("INSUFFICIENT_FUNDS", "not enough")
	}))
	must(t, h.RegisterActivity("unknown", func(context.Context, json.RawMessage) (any, error) {
		return nil, worker.OutcomeUnknown("the provider timed out")
	}))
	must(t, h.RegisterActivity("retryable", func(context.Context, json.RawMessage) (any, error) {
		return nil, fmt.Errorf("wrapped: %w", worker.Retryable("PROVIDER_BUSY", "try again"))
	}))
	must(t, h.RegisterActivity("boom", func(context.Context, json.RawMessage) (any, error) {
		return nil, errors.New("secret card 4111 leaked in the message")
	}))
	must(t, h.RegisterActivity("badcode", func(context.Context, json.RawMessage) (any, error) {
		return nil, &worker.ActivityFailure{Code: "not a code!", Message: strings.Repeat("é", 600)}
	}))
	for i, name := range []string{"permanent", "unknown", "retryable", "boom", "badcode"} {
		f.lease(lease(forActivity(name, "1"), func(l *pb.TaskLease) { l.RunId = fmt.Sprintf("run-%d", i) }))
	}
	start(t, h)
	until(t, "five failures", func() bool { return len(f.of("FailStep")) == 5 })
	by := map[string]*pb.FailStepRequest{}
	for _, c := range f.of("FailStep") {
		r := c.request.(*pb.FailStepRequest)
		by[r.GetStepId()] = r
	}
	check := func(step, code string, retryable bool) *pb.FailStepRequest {
		r := by[step]
		if r == nil || r.GetErrorCode() != code || r.GetRetryable() != retryable {
			t.Errorf("%s: %v", step, r)
		}
		return r
	}
	if r := check("permanent", "INSUFFICIENT_FUNDS", false); r != nil && r.GetSafeMessage() != "not enough" {
		t.Errorf("%q", r.GetSafeMessage())
	}
	check("unknown", "OUTCOME_UNKNOWN", false)
	check("retryable", "PROVIDER_BUSY", true)
	if r := check("boom", "ACTIVITY_ERROR", true); r != nil && (r.GetSafeMessage() != "*errors.errorString" || strings.Contains(r.GetSafeMessage(), "4111")) {
		t.Errorf("the message must be the type, not the text: %q", r.GetSafeMessage())
	}
	if r := check("badcode", "WORKER_ERROR", false); r != nil && len([]rune(r.GetSafeMessage())) != 512 {
		t.Errorf("cut to 512 characters: %d", len([]rune(r.GetSafeMessage())))
	}
	if r := by["permanent"]; r.GetIdempotencyKey() != "failed-run-1/permanent/1" || r.GetTraceparent() != traceparent {
		t.Errorf("%v", r)
	}
}

func TestAStaleReportIsDroppedAndNeverRetried(t *testing.T) {
	f := newFakeService(t)
	events := make(chan worker.Event, 100)
	h := newHost(t, f, worker.WithEventHandler(func(e worker.Event) { events <- e }))
	must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) { return map[string]bool{"ok": true}, nil }))
	f.fail("CompleteStep", failure{code: codes.Aborted, reason: "STALE_ATTEMPT"})
	f.lease(lease())
	start(t, h)
	waitForEvent(t, events, worker.EventStale)
	time.Sleep(100 * time.Millisecond)
	if n := len(f.of("CompleteStep")); n != 1 {
		t.Errorf("%d reports", n)
	}
}

func waitForEvent(t *testing.T, events chan worker.Event, kind worker.EventType) worker.Event {
	t.Helper()
	timeout := time.After(10 * time.Second)
	for {
		select {
		case e := <-events:
			if e.Type == kind {
				return e
			}
		case <-timeout:
			t.Fatalf("timed out waiting for the event %s", kind)
		}
	}
}

func TestAnInputTheHandlerCannotTakeAnActivityNobodyRegisteredAndAnOutputTooLargeAreFailuresForGood(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	type order struct {
		Amount int `json:"amount"`
	}
	must(t, h.RegisterActivity("typed", worker.Typed(func(context.Context, order) (int, error) { return 1, nil })))
	must(t, h.RegisterActivity("huge", func(context.Context, json.RawMessage) (any, error) { return strings.Repeat("x", 300*1024), nil }))
	must(t, h.RegisterActivity("bad-number", func(context.Context, json.RawMessage) (any, error) { return map[string]float64{"n": nan()}, nil }))
	must(t, h.RegisterActivity("broken-utf8", func(context.Context, json.RawMessage) (any, error) { return "\xff", nil }))
	must(t, h.RegisterActivity("junk", rawOK))
	f.lease(
		lease(forActivity("typed", "1"), func(l *pb.TaskLease) { l.InputJson = `{"input":{"amount":"not a number"}}` }),
		lease(forActivity("huge", "1")),
		lease(forActivity("nobody", "1")),
		lease(forActivity("bad-number", "1")),
		lease(forActivity("broken-utf8", "1")),
		lease(forActivity("junk", "1"), func(l *pb.TaskLease) { l.InputJson = "{nope" }),
	)
	start(t, h)
	until(t, "six failures", func() bool { return len(f.of("FailStep")) == 6 })
	codeOf := func(step string) string {
		for _, c := range f.of("FailStep") {
			if r := c.request.(*pb.FailStepRequest); r.GetStepId() == step {
				if r.GetRetryable() {
					t.Errorf("%s is retryable", step)
				}
				return r.GetErrorCode()
			}
		}
		return ""
	}
	for step, want := range map[string]string{
		"typed": "INPUT_INVALID", "huge": "PAYLOAD_TOO_LARGE", "nobody": "NO_HANDLER", "bad-number": "OUTPUT_INVALID",
		"broken-utf8": "OUTPUT_INVALID", "junk": "INPUT_INVALID",
	} {
		if got := codeOf(step); got != want {
			t.Errorf("%s: %q, want %q", step, got, want)
		}
	}
}

func nan() float64 {
	zero := 0.0
	return zero / zero
}

func TestAnOutputOf256KiBIsSentAndNoResultIsNoValue(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	must(t, h.RegisterActivity("edge", func(context.Context, json.RawMessage) (any, error) { return strings.Repeat("x", 256*1024-2), nil }))
	must(t, h.RegisterActivity("void", func(context.Context, json.RawMessage) (any, error) { return worker.Void{}, nil }))
	must(t, h.RegisterActivity("null", func(context.Context, json.RawMessage) (any, error) { return nil, nil }))
	f.lease(lease(forActivity("edge", "1")), lease(forActivity("void", "1")), lease(forActivity("null", "1")))
	start(t, h)
	until(t, "three completions", func() bool { return len(f.of("CompleteStep")) == 3 })
	out := map[string]string{}
	for _, c := range f.of("CompleteStep") {
		r := c.request.(*pb.CompleteStepRequest)
		out[r.GetStepId()] = r.GetOutputJson()
	}
	if len(out["edge"]) != 256*1024 || out["void"] != "" || out["null"] != "null" {
		t.Errorf("edge %d, void %q, null %q", len(out["edge"]), out["void"], out["null"])
	}
}

func TestTheHeartbeatStartsBeforeTheHandlerEndsCarriesTheStepIdentityAndMovesTheDeadline(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f, worker.WithHeartbeatInterval(20*time.Millisecond))
	var deadlines []time.Time
	must(t, h.RegisterActivity("slow", func(ctx context.Context, _ json.RawMessage) (any, error) {
		task := worker.TaskFrom(ctx)
		deadlines = append(deadlines, task.Deadline())
		time.Sleep(150 * time.Millisecond)
		deadlines = append(deadlines, task.Deadline())
		return map[string]bool{"ok": true}, nil
	}))
	later := time.Now().Add(2 * time.Minute).UnixMilli()
	f.set(func(f *fakeService) { f.heartbeatExpires = later })
	f.lease(lease(forActivity("slow", "1")))
	start(t, h)
	until(t, "the completion", func() bool { return len(f.of("CompleteStep")) == 1 })
	beats := f.of("Heartbeat")
	if len(beats) < 3 {
		t.Fatalf("%d beats", len(beats))
	}
	first := beats[0].request.(*pb.HeartbeatRequest)
	if first.GetRunId() != "run-1" || first.GetStepId() != "slow" || first.GetAttempt() != 1 || first.GetLeaseToken() != "lt" || first.GetWorkerId() != "w-1" || first.GetGeneration() != 1 {
		t.Errorf("%v", first)
	}
	if got := beats[0].md.Get("traceparent"); len(got) != 1 || got[0] != traceparent {
		t.Errorf("the heartbeat carries the run's trace: %v", got)
	}
	if !deadlines[1].After(deadlines[0]) || deadlines[1].UnixMilli() != later {
		t.Errorf("the lease was extended: %v", deadlines)
	}
	count := len(f.of("Heartbeat"))
	time.Sleep(100 * time.Millisecond)
	if len(f.of("Heartbeat")) != count {
		t.Error("the heartbeat stops with the task")
	}
}

func TestACancellationReachesTheActivityThroughItsContextWithTheCauseRunCancelledAndItEndsHoweverItLikes(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f, worker.WithHeartbeatInterval(20*time.Millisecond))
	var cause error
	must(t, h.RegisterActivity("long", func(ctx context.Context, _ json.RawMessage) (any, error) {
		<-ctx.Done()
		cause = context.Cause(ctx)
		return map[string]bool{"stopped": true}, nil
	}))
	f.set(func(f *fakeService) { f.cancelRequested = true })
	f.lease(lease(forActivity("long", "1")))
	start(t, h)
	until(t, "the completion", func() bool { return len(f.of("CompleteStep")) == 1 })
	if !errors.Is(cause, worker.ErrRunCancelled) || cause.Error() != "run-cancelled" {
		t.Errorf("cause %v", cause)
	}
	if out := f.of("CompleteStep")[0].request.(*pb.CompleteStepRequest).GetOutputJson(); out != `{"stopped":true}` {
		t.Errorf("%s", out)
	}
}

func TestAnActivityThatReturnsItsCancellationIsFailedForGoodAsCancelled(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f, worker.WithHeartbeatInterval(20*time.Millisecond))
	for _, name := range []string{"ctx-err", "cause", "sagat", "wrapped"} {
		name := name
		must(t, h.RegisterActivity(name, func(ctx context.Context, _ json.RawMessage) (any, error) {
			<-ctx.Done()
			switch name {
			case "ctx-err":
				return nil, ctx.Err()
			case "cause":
				return nil, context.Cause(ctx)
			case "sagat":
				return nil, sagat.NewError(sagat.CodeCancelled, "stopped")
			default:
				return nil, fmt.Errorf("giving up: %w", ctx.Err())
			}
		}))
		f.lease(lease(forActivity(name, "1")))
	}
	f.set(func(f *fakeService) { f.cancelRequested = true })
	start(t, h)
	until(t, "four failures", func() bool { return len(f.of("FailStep")) == 4 })
	for _, c := range f.of("FailStep") {
		r := c.request.(*pb.FailStepRequest)
		if r.GetErrorCode() != "CANCELLED" || r.GetRetryable() {
			t.Errorf("%s: %v", r.GetStepId(), r)
		}
	}
}

func TestALeaseLostDuringTheActivityStopsItAndItsResultIsNeverReported(t *testing.T) {
	f := newFakeService(t)
	events := make(chan worker.Event, 100)
	h := newHost(t, f, worker.WithHeartbeatInterval(20*time.Millisecond), worker.WithEventHandler(func(e worker.Event) { events <- e }))
	var cause error
	returned := make(chan struct{})
	must(t, h.RegisterActivity("slow", func(ctx context.Context, _ json.RawMessage) (any, error) {
		<-ctx.Done()
		cause = context.Cause(ctx)
		close(returned)
		return map[string]bool{"late": true}, nil
	}))
	f.fail("Heartbeat", failure{code: codes.Aborted, reason: "STALE_ATTEMPT"})
	f.lease(lease(forActivity("slow", "1")))
	start(t, h)
	waitForEvent(t, events, worker.EventStale)
	<-returned
	if !errors.Is(cause, worker.ErrLeaseLost) || cause.Error() != "lease-lost" {
		t.Errorf("cause %v", cause)
	}
	time.Sleep(100 * time.Millisecond)
	if len(f.of("CompleteStep")) != 0 || len(f.of("FailStep")) != 0 {
		t.Error("nothing is reported for a lease that is gone")
	}
}

func TestACompensationSeesItsKindAndTheEffectItUndoesAndAReconcilerAnswersExactlyAVerdict(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	var mu sync.Mutex
	var seen []string
	must(t, h.RegisterCompensation("limit.release", func(ctx context.Context, _ json.RawMessage) (any, error) {
		task := worker.TaskFrom(ctx)
		mu.Lock()
		seen = append(seen, fmt.Sprint(task.Kind, " ", task.TargetNode, " ", task.TargetIdempotencyKey, " ", task.IdempotencyKey))
		mu.Unlock()
		return map[string]bool{"released": true}, nil
	}))
	verdicts := map[string]worker.Verdict{
		"limit.lookup": worker.Applied(map[string]string{"id": "r-9"}), "limit.none": worker.NotApplied(),
		"limit.unknown": worker.Unknown(), "limit.bad": {Effect: "yes"}, "limit.nooutput": worker.Applied(nil),
	}
	for name, verdict := range verdicts {
		verdict := verdict
		must(t, h.RegisterReconciler(name, func(context.Context, json.RawMessage) (worker.Verdict, error) { return verdict, nil }))
	}
	kind := func(k string, target string) func(*pb.TaskLease) {
		return func(l *pb.TaskLease) { l.TaskKind, l.TargetNode = k, target }
	}
	f.lease(
		lease(forActivity("limit.release", "2"), kind("COMPENSATION", "reserve"), func(l *pb.TaskLease) {
			l.TargetIdempotencyKey, l.IdempotencyKey = "run-1:reserve", "compensation:c1:run-1"
		}),
		lease(forActivity("limit.lookup", "1"), kind("RECONCILIATION", "a")),
		lease(forActivity("limit.none", "1"), kind("RECONCILIATION", "b")),
		lease(forActivity("limit.unknown", "1"), kind("RECONCILIATION", "c")),
		lease(forActivity("limit.nooutput", "1"), kind("RECONCILIATION", "d")),
		lease(forActivity("limit.bad", "1"), kind("RECONCILIATION", "e")),
	)
	start(t, h)
	until(t, "every report", func() bool { return len(f.of("CompleteStep")) == 5 && len(f.of("FailStep")) == 1 })
	if fmt.Sprint(seen) != "[COMPENSATION reserve run-1:reserve compensation:c1:run-1]" {
		t.Errorf("%v", seen)
	}
	out := map[string]string{}
	for _, c := range f.of("CompleteStep") {
		r := c.request.(*pb.CompleteStepRequest)
		out[r.GetStepId()] = r.GetOutputJson()
	}
	want := map[string]string{
		"limit.lookup": `{"effect":"applied","output":{"id":"r-9"}}`, "limit.none": `{"effect":"not_applied"}`, "limit.unknown": `{"effect":"unknown"}`,
		"limit.nooutput": `{"effect":"applied"}`, "limit.release": `{"released":true}`,
	}
	for step, text := range want {
		if out[step] != text {
			t.Errorf("%s: %s, want %s", step, out[step], text)
		}
	}
	if r := f.of("FailStep")[0].request.(*pb.FailStepRequest); r.GetErrorCode() != "RECONCILIATION_ANSWER_INVALID" || r.GetRetryable() {
		t.Errorf("%v", r)
	}
}

func TestTheHostRunsAtTheVersionTheServerAgreedToAgainst10ItSendsNoStepIdentityAndCannotResolveSecrets(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.accepted = "1" })
	h := newHost(t, f, worker.WithHeartbeatInterval(20*time.Millisecond), worker.WithProtocolVersion("1.9"))
	var secretError error
	var kind worker.TaskKind
	must(t, h.RegisterActivity("slow", func(ctx context.Context, _ json.RawMessage) (any, error) {
		time.Sleep(80 * time.Millisecond)
		kind = worker.TaskFrom(ctx).Kind
		_, secretError = worker.TaskFrom(ctx).Secrets(ctx)
		return 1, nil
	}))
	// A server at 1.0 never sends the other kinds; if one came, it would not be relied on.
	f.lease(lease(forActivity("slow", "1"), func(l *pb.TaskLease) { l.TaskKind = "COMPENSATION" }))
	start(t, h)
	if offered := f.of("RegisterWorker")[0].request.(*pb.RegisterWorkerRequest).GetCapabilities().GetProtocolVersion(); offered != "1.9" {
		t.Errorf("offered %q", offered)
	}
	if h.ProtocolVersion() != "1" {
		t.Errorf("accepted %q", h.ProtocolVersion())
	}
	until(t, "the completion", func() bool { return len(f.of("CompleteStep")) == 1 })
	beat := f.of("Heartbeat")[0].request.(*pb.HeartbeatRequest)
	if beat.GetRunId() != "" || beat.GetStepId() != "" || beat.GetAttempt() != 0 {
		t.Errorf("%v", beat)
	}
	if !sagat.IsCode(secretError, sagat.CodeUnsupportedProtocol) || len(f.of("ResolveSecrets")) != 0 {
		t.Errorf("%v", secretError)
	}
	if kind != worker.KindForward {
		t.Errorf("kind %s", kind)
	}
}

func TestSecretsAreResolvedOncePerAttemptOnFirstUseAsAReadOnlySetAndWipedWhenTheHandlerReturns(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	var text string
	var names []string
	var kept *worker.Secrets
	var viaTask *worker.Task
	must(t, h.RegisterActivity("pay", func(ctx context.Context, _ json.RawMessage) (any, error) {
		task := worker.TaskFrom(ctx)
		viaTask = task
		first, err := task.Secrets(ctx)
		if err != nil {
			return nil, err
		}
		second, _ := task.Secrets(ctx)
		if first != second {
			return nil, errors.New("resolved twice")
		}
		text, _ = first.Text("stripe-key")
		bytes, _ := first.Bytes("stripe-key")
		bytes[0] = 'X' // a copy: this does not change the set
		again, _ := first.Text("stripe-key")
		if again != text {
			return nil, errors.New("the set can be changed from outside")
		}
		if _, err := first.Bytes("nope"); !sagat.IsCode(err, sagat.CodeNotFound) {
			return nil, errors.New("a secret the step does not declare is NOT_FOUND")
		}
		names = first.Names()
		kept = first
		return map[string]any{"names": names}, nil
	}))
	f.set(func(f *fakeService) {
		f.secrets = []*pb.ResolvedSecret{{Name: "stripe-key", Value: []byte("sk_live_x"), Version: 3}}
	})
	f.lease(lease(forActivity("pay", "1")))
	start(t, h)
	until(t, "the completion", func() bool { return len(f.of("CompleteStep")) == 1 })
	if text != "sk_live_x" || fmt.Sprint(names) != "[stripe-key]" {
		t.Errorf("%q %v", text, names)
	}
	if n := len(f.of("ResolveSecrets")); n != 1 {
		t.Errorf("%d resolutions", n)
	}
	request := f.of("ResolveSecrets")[0].request.(*pb.ResolveSecretsRequest)
	if request.GetLeaseToken() != "lt" || request.GetStepId() != "pay" || request.GetAttempt() != 1 || request.GetTraceparent() != traceparent {
		t.Errorf("%v", request)
	}
	if out := f.of("CompleteStep")[0].request.(*pb.CompleteStepRequest).GetOutputJson(); strings.Contains(out, "sk_live") {
		t.Errorf("a secret reached the output: %s", out)
	}
	if len(kept.Names()) != 0 {
		t.Error("the values are dropped when the handler returns")
	}
	if _, err := viaTask.Secrets(context.Background()); err == nil {
		t.Error("a task that is over has no secrets to give")
	}
}

func TestASecretTheStepCannotUseIsTheCanonicalError(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	var got error
	must(t, h.RegisterActivity("pay", func(ctx context.Context, _ json.RawMessage) (any, error) {
		_, got = worker.TaskFrom(ctx).Secrets(ctx)
		return 1, nil
	}))
	f.fail("ResolveSecrets", failure{code: codes.PermissionDenied, reason: "SECRET_NOT_USABLE"})
	f.lease(lease(forActivity("pay", "1")))
	start(t, h)
	until(t, "the completion", func() bool { return len(f.of("CompleteStep")) == 1 })
	var se *sagat.Error
	if !errors.As(got, &se) || se.Code != sagat.CodeSecretNotUsable || se.GRPCStatus != "PERMISSION_DENIED" || se.Reason != "SECRET_NOT_USABLE" {
		t.Errorf("%+v", got)
	}
}

func TestASessionThatIsGoneIsRegisteredAgainAndTheWorkerGoesOn(t *testing.T) {
	f := newFakeService(t)
	events := make(chan worker.Event, 100)
	h := newHost(t, f, worker.WithEventHandler(func(e worker.Event) { events <- e }))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	f.fail("PollTasks", failure{code: codes.Unauthenticated})
	until(t, "a second registration", func() bool { return len(f.of("RegisterWorker")) == 2 })
	f.lease(lease())
	until(t, "the task after it", func() bool { return len(f.of("CompleteStep")) == 1 })
	waitForEvent(t, events, worker.EventReconnect)
	// The poll after the new registration carries the new session.
	polls := f.of("PollTasks")
	if got := polls[len(polls)-1].request.(*pb.PollTasksRequest).GetSessionToken(); got != "session-2" {
		t.Errorf("session %q", got)
	}
}

func TestAnEmptyPollIsFollowedByAWaitNeverByAnotherPollAtOnce(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	time.Sleep(400 * time.Millisecond)
	if polls := len(f.of("PollTasks")); polls < 2 || polls > 10 {
		t.Errorf("%d polls in 400 ms", polls)
	}
}

func TestNoMoreTasksAreTakenThanTheHostHasRoomForAndStopWaitsForTheOnesInFlight(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f, worker.WithMaxInflight(2))
	var running, peak atomic.Int32
	var done atomic.Int32
	must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) {
		now := running.Add(1)
		for {
			old := peak.Load()
			if now <= old || peak.CompareAndSwap(old, now) {
				break
			}
		}
		time.Sleep(100 * time.Millisecond)
		running.Add(-1)
		done.Add(1)
		return 1, nil
	}))
	for i := 1; i <= 4; i++ {
		f.lease(lease(func(l *pb.TaskLease) { l.TaskId, l.RunId = fmt.Sprintf("run-%d/charge/1", i), fmt.Sprintf("run-%d", i) }))
	}
	start(t, h)
	until(t, "a poll", func() bool { return len(f.of("PollTasks")) > 0 })
	if got := f.of("PollTasks")[0].request.(*pb.PollTasksRequest).GetMaxTasks(); got > 2 {
		t.Errorf("asked for %d tasks with room for 2", got)
	}
	until(t, "a task taken", func() bool { return running.Load() > 0 })
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := h.Stop(ctx); err != nil {
		t.Errorf("the tasks in flight finished within the grace: %v", err)
	}
	if peak.Load() > 2 {
		t.Errorf("peak %d", peak.Load())
	}
	if running.Load() != 0 {
		t.Error("nothing was left running")
	}
	if int(done.Load()) != len(f.of("CompleteStep")) {
		t.Errorf("%d finished, %d reported: stop waits for the report too", done.Load(), len(f.of("CompleteStep")))
	}
}

func TestStopWhileAPollIsInFlightDoesNotWaitOutThePauseAnEmptyAnswerAsksFor(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.pollDelay = 150 * time.Millisecond; f.pollRetryAfterMs = 10_000 })
	h := newHost(t, f)
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "the first poll", func() bool { return len(f.of("PollTasks")) == 1 })
	began := time.Now()
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	_ = h.Stop(ctx)
	if took := time.Since(began); took > 1500*time.Millisecond {
		t.Errorf("stop took %v", took)
	}
}

func TestStopCancelsTheTasksThatAreStillRunningAfterTheGraceWithTheCauseShutdown(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	var cause error
	started := make(chan struct{})
	must(t, h.RegisterActivity("stuck", func(ctx context.Context, _ json.RawMessage) (any, error) {
		close(started)
		<-ctx.Done()
		cause = context.Cause(ctx)
		return map[string]bool{"stopped": true}, nil
	}))
	f.lease(lease(forActivity("stuck", "1")))
	start(t, h)
	<-started
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	err := h.Stop(ctx)
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Errorf("stop says the grace ran out: %v", err)
	}
	if !errors.Is(cause, worker.ErrShutdown) || cause.Error() != "shutdown" {
		t.Errorf("cause %v", cause)
	}
	// The handler returned within the two seconds more it is given, so its result was reported.
	if len(f.of("CompleteStep")) != 1 {
		t.Errorf("%d reports", len(f.of("CompleteStep")))
	}
}

func TestAHandlerThatRaisesItsCancellationAfterShutdownIsNotReportedAndOtherEndsAre(t *testing.T) {
	f := newFakeService(t)
	events := make(chan worker.Event, 100)
	h := newHost(t, f, worker.WithEventHandler(func(e worker.Event) { events <- e }))
	var started sync.WaitGroup
	for _, name := range []string{"ctx-err", "cause", "sagat", "wrapped"} {
		name := name
		started.Add(1)
		must(t, h.RegisterActivity(name, func(ctx context.Context, _ json.RawMessage) (any, error) {
			started.Done()
			<-ctx.Done()
			switch name {
			case "ctx-err":
				return nil, ctx.Err()
			case "cause":
				return nil, context.Cause(ctx)
			case "sagat":
				return nil, sagat.NewError(sagat.CodeCancelled, "stopped")
			default:
				return nil, fmt.Errorf("giving up: %w", ctx.Err())
			}
		}))
		f.lease(lease(forActivity(name, "1")))
	}
	// These end differently: a failure on purpose and another error are reported as usual after shutdown.
	must(t, h.RegisterActivity("on-purpose", func(ctx context.Context, _ json.RawMessage) (any, error) {
		started.Done()
		<-ctx.Done()
		return nil, worker.Permanent("GAVE_UP", "stopped on purpose")
	}))
	must(t, h.RegisterActivity("other", func(ctx context.Context, _ json.RawMessage) (any, error) {
		started.Done()
		<-ctx.Done()
		return nil, errors.New("something else broke")
	}))
	started.Add(2)
	f.lease(lease(forActivity("on-purpose", "1")), lease(forActivity("other", "1")))
	start(t, h)
	started.Wait()
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	_ = h.Stop(ctx)
	if n := len(f.of("CompleteStep")); n != 0 {
		t.Errorf("%d completions", n)
	}
	reported := map[string]string{}
	for _, c := range f.of("FailStep") {
		r := c.request.(*pb.FailStepRequest)
		reported[r.GetStepId()] = r.GetErrorCode()
	}
	if len(reported) != 2 || reported["on-purpose"] != "GAVE_UP" || reported["other"] != "ACTIVITY_ERROR" {
		t.Errorf("only the failure on purpose and the other error are reported: %v", reported)
	}
	dropped := 0
	for len(events) > 0 {
		if e := <-events; e.Type == worker.EventDropped && e.Code == "SHUTDOWN" {
			dropped++
		}
	}
	if dropped != 4 {
		t.Errorf("%d tasks dropped for the shutdown", dropped)
	}
}

func TestTheHeartbeatKeepsRunningUntilTheReportIsMadeNotOnlyUntilTheHandlerEnds(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f, worker.WithHeartbeatInterval(15*time.Millisecond))
	must(t, h.RegisterActivity("quick", rawOK))
	// The report is refused three times and each repetition waits 80 ms: it takes a quarter of a second.
	f.fail("CompleteStep", failure{code: codes.Unavailable, retryAfter: "80"}, failure{code: codes.Unavailable, retryAfter: "80"}, failure{code: codes.Unavailable, retryAfter: "80"})
	f.lease(lease(forActivity("quick", "1")))
	start(t, h)
	until(t, "the report", func() bool { return len(f.of("CompleteStep")) == 4 })
	time.Sleep(30 * time.Millisecond)
	beats := f.of("Heartbeat")
	if len(beats) < 8 {
		t.Errorf("%d heartbeats during a report that took 240 ms at 15 ms", len(beats))
	}
	count := len(beats)
	time.Sleep(100 * time.Millisecond)
	if len(f.of("Heartbeat")) != count {
		t.Error("the heartbeat stops once the report is made")
	}
}

func TestThePollWaitEndsEarlyWhenATaskFinishes(t *testing.T) {
	f := newFakeService(t)
	// An empty answer asks for ten seconds of quiet.
	f.set(func(f *fakeService) { f.pollRetryAfterMs = 10_000 })
	h := newHost(t, f, worker.WithMaxInflight(2))
	release := make(chan struct{})
	started := make(chan struct{})
	var taken atomic.Int32
	must(t, h.RegisterActivity("slow", func(ctx context.Context, _ json.RawMessage) (any, error) {
		if taken.Add(1) == 1 {
			close(started)
			<-release
		}
		return 1, nil
	}))
	f.lease(lease(forActivity("slow", "1")))
	start(t, h)
	<-started
	// The host has polled again, got nothing, and is now in its ten seconds of quiet.
	until(t, "a second poll", func() bool { return len(f.of("PollTasks")) >= 2 })
	time.Sleep(50 * time.Millisecond)
	polls := len(f.of("PollTasks"))
	f.lease(lease(forActivity("slow", "2")))
	began := time.Now()
	close(release) // the first task finishes: a slot is free
	until(t, "the second task", func() bool { return len(f.of("CompleteStep")) == 2 })
	if took := time.Since(began); took > 2*time.Second {
		t.Errorf("the second task waited %v: the quiet was not cut short", took)
	}
	if len(f.of("PollTasks")) <= polls {
		t.Error("a poll was made when the task finished")
	}
}

func TestPlainHTTPOutsideLoopbackLogsOneWarningAtStart(t *testing.T) {
	var logged bytes.Buffer
	var mu sync.Mutex
	logger := slog.New(slog.NewTextHandler(&lockedWriter{&mu, &logged}, nil))
	// 192.0.2.1 is TEST-NET-1: nothing answers there, so the start ends with the context.
	h, err := worker.New("http://192.0.2.1:50051", "payments", "w", sagat.StaticToken("super-secret-token"),
		worker.WithLogger(logger), worker.WithRetry(sagat.RetryPolicy{MaxAttempts: 5, InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}))
	must(t, err)
	must(t, h.RegisterActivity("a", rawOK))
	ctx, cancel := context.WithTimeout(context.Background(), 300*time.Millisecond)
	defer cancel()
	if err := h.Start(ctx); err == nil {
		t.Fatal("nothing listens there")
	}
	mu.Lock()
	text := logged.String()
	mu.Unlock()
	if n := strings.Count(text, "without TLS"); n != 1 || strings.Contains(text, "super-secret-token") {
		t.Errorf("%d warnings: %s", n, text)
	}
	// Loopback is development: no warning.
	f := newFakeService(t)
	logged.Reset()
	quiet := newHost(t, f, worker.WithLogger(logger))
	must(t, quiet.RegisterActivity("a", rawOK))
	start(t, quiet)
	mu.Lock()
	defer mu.Unlock()
	if strings.Contains(logged.String(), "without TLS") {
		t.Errorf("%s", logged.String())
	}
}

func TestRegistrationIsRefusedClearlyAnUnsupportedProtocolAnUnboundIdentity(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	must(t, h.RegisterActivity("charge", rawOK))
	f.fail("RegisterWorker", failure{code: codes.InvalidArgument, reason: "UNSUPPORTED_PROTOCOL"})
	err := h.Start(context.Background())
	var se *sagat.Error
	if !errors.As(err, &se) || se.Code != sagat.CodeUnsupportedProtocol || se.Retryable || se.GRPCStatus != "INVALID_ARGUMENT" || se.Reason != "UNSUPPORTED_PROTOCOL" {
		t.Errorf("%+v", err)
	}
	if n := len(f.of("RegisterWorker")); n != 1 {
		t.Errorf("not retried: %d", n)
	}
	f.fail("RegisterWorker", failure{code: codes.PermissionDenied, reason: "NOT_BOUND"})
	err = h.Start(context.Background())
	if !errors.As(err, &se) || se.Code != sagat.CodeNotBound || se.GRPCStatus != "PERMISSION_DENIED" || se.Retryable {
		t.Errorf("%+v", err)
	}
	if n := len(f.of("RegisterWorker")); n != 2 {
		t.Errorf("not retried: %d", n)
	}
	// After a refusal the host is not running, and it can be started again.
	start(t, h)
}

func TestARegistrationThatNeverLeftOrWasRefusedAsUnavailableIsRepeated(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	must(t, h.RegisterActivity("charge", rawOK))
	f.fail("RegisterWorker", failure{code: codes.Unavailable}, failure{code: codes.Unavailable, reason: "UNAVAILABLE"})
	start(t, h)
	if n := len(f.of("RegisterWorker")); n != 3 {
		t.Errorf("%d registrations", n)
	}
	// Out of attempts, the last error is returned.
	g := newFakeService(t)
	h2, err := worker.New(g.url(), "payments", "w", sagat.StaticToken("tok"), worker.WithRetry(sagat.RetryPolicy{MaxAttempts: 2, InitialDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond}))
	must(t, err)
	must(t, h2.RegisterActivity("charge", rawOK))
	g.fail("RegisterWorker", failure{code: codes.Unavailable}, failure{code: codes.Unavailable}, failure{code: codes.Unavailable})
	if err := h2.Start(context.Background()); !sagat.IsCode(err, sagat.CodeTransport) || len(g.of("RegisterWorker")) != 2 {
		t.Errorf("%v after %d", err, len(g.of("RegisterWorker")))
	}
}

func TestATokenTheProviderCanRefreshGetsOneMoreChanceAtRegistrationAndAStaticOneDoesNot(t *testing.T) {
	f := newFakeService(t)
	n := 0
	provider := sagat.TokenFunc(func(context.Context) (string, error) { n++; return fmt.Sprintf("tok-%d", n), nil })
	h, err := worker.New(f.url(), "payments", "w", provider)
	must(t, err)
	t.Cleanup(func() { _ = h.Stop(context.Background()) })
	must(t, h.RegisterActivity("charge", rawOK))
	f.fail("RegisterWorker", failure{code: codes.Unauthenticated})
	start(t, h)
	registers := f.of("RegisterWorker")
	if len(registers) != 2 || registers[0].md.Get("authorization")[0] != "Bearer tok-1" || registers[1].md.Get("authorization")[0] != "Bearer tok-2" {
		t.Errorf("%d registrations", len(registers))
	}

	g := newFakeService(t)
	h2, err := worker.New(g.url(), "payments", "w", sagat.StaticToken("tok"))
	must(t, err)
	must(t, h2.RegisterActivity("charge", rawOK))
	g.fail("RegisterWorker", failure{code: codes.Unauthenticated}, failure{code: codes.Unauthenticated})
	if err := h2.Start(context.Background()); !sagat.IsCode(err, sagat.CodeUnauthenticated) || len(g.of("RegisterWorker")) != 1 {
		t.Errorf("%v after %d", err, len(g.of("RegisterWorker")))
	}
}

func TestRegisteringTwiceOrAfterTheStartIsAMistakeAndSaysSo(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	must(t, h.RegisterActivity("a", rawOK))
	if err := h.RegisterActivity("a", rawOK); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("%v", err)
	}
	if err := h.RegisterCompensation("a", rawOK); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("%v", err)
	}
	if err := h.RegisterActivity("", rawOK); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("%v", err)
	}
	if err := h.RegisterActivity("nil-handler", nil); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("%v", err)
	}
	for _, maxInflight := range []int{-1, 1001} {
		if _, err := worker.New(f.url(), "payments", "w", sagat.StaticToken("t"), worker.WithMaxInflight(maxInflight)); !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("maxInflight %d: %v", maxInflight, err)
		}
	}
	empty := newHost(t, f)
	if err := empty.Start(context.Background()); err == nil || !strings.Contains(err.Error(), "at least one activity") {
		t.Errorf("%v", err)
	}
	start(t, h)
	if err := h.RegisterActivity("late", rawOK); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("a registration after the start: %v", err)
	}
	if err := h.Start(context.Background()); err != nil {
		t.Errorf("starting a host that runs does nothing: %v", err)
	}
	if n := len(f.of("RegisterWorker")); n != 1 {
		t.Errorf("%d registrations", n)
	}
	for name, args := range map[string][3]string{"url": {"", "ns", "w"}, "namespace": {f.url(), "", "w"}, "name": {f.url(), "ns", ""}, "scheme": {"ftp://x:1", "ns", "w"}} {
		if _, err := worker.New(args[0], args[1], args[2], sagat.StaticToken("t")); !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("%s: %v", name, err)
		}
	}
	if _, err := worker.New(f.url(), "ns", "w", nil); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("no token source: %v", err)
	}
}

func TestAHeartbeatThatFailsIsTriedAgainOnTheNextBeatAndTheLeaseIsKept(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f, worker.WithHeartbeatInterval(20*time.Millisecond))
	var cancelled bool
	must(t, h.RegisterActivity("slow", func(ctx context.Context, _ json.RawMessage) (any, error) {
		time.Sleep(200 * time.Millisecond)
		cancelled = ctx.Err() != nil
		return 1, nil
	}))
	f.fail("Heartbeat", failure{code: codes.Unavailable}, failure{code: codes.Unavailable}, failure{code: codes.Internal})
	f.lease(lease(forActivity("slow", "1")))
	start(t, h)
	until(t, "the completion", func() bool { return len(f.of("CompleteStep")) == 1 })
	if len(f.of("Heartbeat")) < 4 || cancelled {
		t.Errorf("%d beats, cancelled %v", len(f.of("Heartbeat")), cancelled)
	}
}

func TestAPollThatFailsIsFollowedByAWaitAndTheHostGoesOn(t *testing.T) {
	f := newFakeService(t)
	events := make(chan worker.Event, 100)
	h := newHost(t, f, worker.WithEventHandler(func(e worker.Event) { events <- e }))
	must(t, h.RegisterActivity("charge", rawOK))
	f.fail("PollTasks", failure{code: codes.Unavailable}, failure{code: codes.ResourceExhausted, reason: "RATE_LIMITED", retryAfter: "10"}, failure{code: codes.Internal})
	f.lease(lease())
	start(t, h)
	until(t, "the task", func() bool { return len(f.of("CompleteStep")) == 1 })
	waitForEvent(t, events, worker.EventReconnect)
	if len(f.of("RegisterWorker")) != 1 {
		t.Error("an outage is not a lost session: no new registration")
	}
}

func TestAPanickingActivityIsReportedAsAnErrorOfItsTypeAndTheHostSurvives(t *testing.T) {
	f := newFakeService(t)
	var logged bytes.Buffer
	var mu sync.Mutex
	h := newHost(t, f, worker.WithLogger(slog.New(slog.NewTextHandler(&lockedWriter{&mu, &logged}, &slog.HandlerOptions{Level: slog.LevelDebug}))))
	must(t, h.RegisterActivity("boom", func(context.Context, json.RawMessage) (any, error) { panic("card 4111111111111111 exploded") }))
	must(t, h.RegisterActivity("fine", rawOK))
	f.lease(lease(forActivity("boom", "1")), lease(forActivity("fine", "1")))
	start(t, h)
	until(t, "both reports", func() bool { return len(f.of("FailStep")) == 1 && len(f.of("CompleteStep")) == 1 })
	r := f.of("FailStep")[0].request.(*pb.FailStepRequest)
	if r.GetErrorCode() != "ACTIVITY_ERROR" || !r.GetRetryable() || strings.Contains(r.GetSafeMessage(), "4111") {
		t.Errorf("%v", r)
	}
	mu.Lock()
	defer mu.Unlock()
	if strings.Contains(logged.String(), "4111") {
		t.Errorf("the panic value reached the log: %s", logged.String())
	}
}

type lockedWriter struct {
	mu *sync.Mutex
	w  *bytes.Buffer
}

func (l *lockedWriter) Write(p []byte) (int, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.w.Write(p)
}

func TestTheLogsAndEventsNeverHoldATokenALeaseTokenASessionTokenAPayloadOrASecret(t *testing.T) {
	f := newFakeService(t)
	var logged bytes.Buffer
	var mu sync.Mutex
	var events []worker.Event
	h := newHost(t, f, worker.WithHeartbeatInterval(10*time.Millisecond),
		worker.WithLogger(slog.New(slog.NewTextHandler(&lockedWriter{&mu, &logged}, &slog.HandlerOptions{Level: slog.LevelDebug}))),
		worker.WithEventHandler(func(e worker.Event) { mu.Lock(); events = append(events, e); mu.Unlock() }))
	must(t, h.RegisterActivity("pay", func(ctx context.Context, input json.RawMessage) (any, error) {
		task := worker.TaskFrom(ctx)
		task.Logger.Info("paying")
		secrets, err := task.Secrets(ctx)
		if err != nil {
			return nil, err
		}
		time.Sleep(50 * time.Millisecond)
		_, _ = secrets.Text("k")
		return nil, errors.New("the provider said: PAYLOAD-MARKER")
	}))
	f.set(func(f *fakeService) { f.secrets = []*pb.ResolvedSecret{{Name: "k", Value: []byte("SECRET-VALUE")}} })
	f.fail("CompleteStep", failure{code: codes.Unavailable})
	f.fail("FailStep", failure{code: codes.Unavailable}, failure{code: codes.Unavailable}, failure{code: codes.Unavailable}, failure{code: codes.Unavailable}, failure{code: codes.Unavailable})
	f.lease(lease(forActivity("pay", "1"), func(l *pb.TaskLease) {
		l.LeaseToken = "LEASE-TOKEN-MARKER"
		l.InputJson = `{"input":{"card":"PAYLOAD-MARKER"}}`
	}))
	start(t, h)
	until(t, "the failure to be given up on", func() bool { return len(f.of("FailStep")) == 5 })
	time.Sleep(50 * time.Millisecond)
	mu.Lock()
	defer mu.Unlock()
	text := logged.String() + fmt.Sprintf("%+v", events)
	for _, marker := range []string{"tok", "LEASE-TOKEN-MARKER", "session-1", "PAYLOAD-MARKER", "SECRET-VALUE"} {
		if strings.Contains(text, marker) {
			t.Errorf("%q is in the logs or the events:\n%s", marker, text)
		}
	}
	if !strings.Contains(text, "paying") {
		t.Error("the logger of the task works")
	}
}

// hostGoroutines counts the goroutines that run code of the host.
func hostGoroutines() int {
	buf := make([]byte, 1<<20)
	buf = buf[:runtime.Stack(buf, true)]
	n := 0
	for _, stack := range strings.Split(string(buf), "\n\n") {
		if strings.Contains(stack, "sdks/go/worker.(*Host)") {
			n++
		}
	}
	return n
}

func TestAStoppedHostLeavesNoGoroutineOrTimerOfItsOwnBehind(t *testing.T) {
	before := hostGoroutines() // the hosts of other tests may still be winding down
	f := newFakeService(t)
	h := newHost(t, f, worker.WithHeartbeatInterval(10*time.Millisecond))
	must(t, h.RegisterActivity("slow", func(ctx context.Context, _ json.RawMessage) (any, error) {
		time.Sleep(30 * time.Millisecond)
		return 1, nil
	}))
	for i := 1; i <= 3; i++ {
		f.lease(lease(forActivity("slow", fmt.Sprint(i))))
	}
	f.set(func(f *fakeService) { f.pollRetryAfterMs = 10_000 })
	start(t, h)
	until(t, "the tasks", func() bool { return len(f.of("CompleteStep")) == 3 })
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	must(t, h.Stop(ctx))
	until(t, "no goroutine of the host", func() bool { return hostGoroutines() == before })
}

func TestTheHostCanBeStartedAgainAfterItWasStopped(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f)
	must(t, h.RegisterActivity("charge", rawOK))
	for round := 1; round <= 2; round++ {
		f.lease(lease(func(l *pb.TaskLease) { l.TaskId = fmt.Sprintf("run-1/charge/%d", round) }))
		start(t, h)
		until(t, "the task", func() bool { return len(f.of("CompleteStep")) == round })
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		if err := h.Stop(ctx); err != nil {
			t.Errorf("round %d: %v", round, err)
		}
		cancel()
	}
	if n := len(f.of("RegisterWorker")); n != 2 {
		t.Errorf("%d registrations", n)
	}
	if err := h.Stop(context.Background()); err != nil {
		t.Errorf("stopping a host that is not running does nothing: %v", err)
	}
}

func TestTheTraceOfTheRunIsCarriedByEveryCallOfATaskButNotByTheRegistrationOrThePoll(t *testing.T) {
	f := newFakeService(t)
	h := newHost(t, f, worker.WithHeartbeatInterval(10*time.Millisecond))
	must(t, h.RegisterActivity("slow", func(ctx context.Context, _ json.RawMessage) (any, error) {
		time.Sleep(40 * time.Millisecond)
		return 1, nil
	}))
	must(t, h.RegisterActivity("bad", func(context.Context, json.RawMessage) (any, error) { return nil, worker.Permanent("X", "x") }))
	f.lease(lease(forActivity("slow", "1")), lease(forActivity("bad", "1"), func(l *pb.TaskLease) { l.Traceparent = "garbage" }))
	start(t, h)
	until(t, "both reports", func() bool { return len(f.of("CompleteStep")) == 1 && len(f.of("FailStep")) == 1 })
	for _, method := range []string{"Heartbeat", "CompleteStep"} {
		for _, c := range f.of(method) {
			if got := c.md.Get("traceparent"); len(got) != 1 || got[0] != traceparent {
				t.Errorf("%s: %v", method, got)
			}
		}
	}
	for _, method := range []string{"RegisterWorker", "PollTasks"} {
		for _, c := range f.of(method) {
			if got := c.md.Get("traceparent"); len(got) != 0 {
				t.Errorf("%s: %v", method, got)
			}
		}
	}
	if got := f.of("FailStep")[0].md.Get("traceparent"); len(got) != 0 {
		t.Errorf("a traceparent the server handed out that is malformed is not sent: %v", got)
	}
}

func TestTypedDecodesTheInputAndReconcilerHandlersToo(t *testing.T) {
	type in struct {
		ID int64 `json:"id"`
	}
	handler := worker.Typed(func(_ context.Context, i in) (int64, error) { return i.ID, nil })
	got, err := handler(context.Background(), json.RawMessage(`{"id": 9007199254740993}`))
	if err != nil || got != int64(9007199254740993) {
		t.Errorf("%v %v", got, err)
	}
	if _, err := handler(context.Background(), json.RawMessage(`{"id": "x"}`)); !errors.Is(err, worker.ErrInputInvalid) {
		t.Errorf("%v", err)
	}
	if got, err := handler(context.Background(), nil); err != nil || got != int64(0) {
		t.Errorf("an absent input is the zero value: %v %v", got, err)
	}
	reconciler := worker.TypedReconciler(func(_ context.Context, i in) (worker.Verdict, error) { return worker.Applied(i.ID), nil })
	if v, err := reconciler(context.Background(), json.RawMessage(`{"id": 4}`)); err != nil || v.Effect != worker.EffectApplied || v.Output != int64(4) {
		t.Errorf("%+v %v", v, err)
	}
	if _, err := reconciler(context.Background(), json.RawMessage(`[1]`)); !errors.Is(err, worker.ErrInputInvalid) {
		t.Errorf("%v", err)
	}
	if worker.TaskFrom(context.Background()) != nil {
		t.Error("outside a handler there is no task")
	}
}
