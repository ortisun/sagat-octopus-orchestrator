package worker_test

// Step-log capture through the host (design 041, sdks/SPEC.md section 6.9): what is captured and when, the batches
// and the answers of AppendStepLogs, the carry and the wait before a report, against the in-memory worker service.
// Races are ordered by held answers and by the test's own tick, never by a sleep.

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"google.golang.org/grpc/codes"
	"google.golang.org/protobuf/proto"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

// stepLogService is a fake service that accepts 1.7 with the step-log policy on.
func stepLogService(t *testing.T) *fakeService {
	f := newFakeService(t)
	f.accepted = "1.7"
	f.stepLogs = &pb.StepLogPolicy{Enabled: true}
	return f
}

// stepLogHost is a host that offers 1.7 and runs one task at a time; its senders tick only when the test sends on the
// channel it returns.
func stepLogHost(t *testing.T, f *fakeService, opts ...worker.Option) (*worker.Host, chan time.Time) {
	t.Helper()
	ticks := make(chan time.Time)
	h := newHost(t, f, append([]worker.Option{worker.WithProtocolVersion("1.7"), worker.WithMaxInflight(1)}, opts...)...)
	worker.SetStepLogTicker(h, ticks)
	return h, ticks
}

// drops gathers the step-log drops the host reports, by reason.
type drops struct {
	mu   sync.Mutex
	seen map[string]int
}

func (d *drops) option() worker.Option {
	return worker.WithEventHandler(func(e worker.Event) {
		if e.Type != worker.EventStepLogsDropped {
			return
		}
		d.mu.Lock()
		defer d.mu.Unlock()
		if d.seen == nil {
			d.seen = map[string]int{}
		}
		d.seen[e.Code] += e.Records
	})
}

func (d *drops) String() string {
	d.mu.Lock()
	defer d.mu.Unlock()
	return fmt.Sprint(d.seen)
}

// logLines is a handler that writes count lines named "line n" and returns {"ok": true}, or what end returns.
func logLines(count int, end func(ctx context.Context) (any, error)) worker.Handler {
	return func(ctx context.Context, _ json.RawMessage) (any, error) {
		logger := worker.TaskFrom(ctx).Logger
		for n := 1; n <= count; n++ {
			logger.Info(fmt.Sprintf("line %d", n))
		}
		if end != nil {
			return end(ctx)
		}
		return map[string]bool{"ok": true}, nil
	}
}

// reportOf waits for the first report and returns its carry.
func reportOf(t *testing.T, f *fakeService) *pb.StepLogCarry {
	t.Helper()
	until(t, "the report", func() bool { return len(f.of("CompleteStep"))+len(f.of("FailStep")) > 0 })
	if completed := f.of("CompleteStep"); len(completed) > 0 {
		return completed[0].request.(*pb.CompleteStepRequest).GetStepLogs()
	}
	return f.of("FailStep")[0].request.(*pb.FailStepRequest).GetStepLogs()
}

// runOne runs one activity that logs count lines and returns.
func runOne(t *testing.T, f *fakeService, count int, opts ...worker.Option) *pb.StepLogCarry {
	t.Helper()
	h, _ := stepLogHost(t, f, opts...)
	must(t, h.RegisterActivity("charge", logLines(count, nil)))
	start(t, h)
	f.lease(lease())
	return reportOf(t, f)
}

// tick sends the tick of a second to the sender of the attempt that runs; a host with no sender fails the test.
func tick(t *testing.T, ticks chan time.Time) {
	t.Helper()
	select {
	case ticks <- time.Now():
	case <-time.After(10 * time.Second):
		t.Fatal("no step-log sender took the tick")
	}
}

func sequences(records []*pb.StepLogRecord) []uint64 {
	out := make([]uint64, len(records))
	for i, record := range records {
		out[i] = record.GetSequence()
	}
	return out
}

func messages(records []*pb.StepLogRecord) []string {
	out := make([]string, len(records))
	for i, record := range records {
		out[i] = record.GetMessage()
	}
	return out
}

func span(from, to uint64) []uint64 {
	var out []uint64
	for n := from; n <= to; n++ {
		out = append(out, n)
	}
	return out
}

func appends(f *fakeService) []*pb.AppendStepLogsRequest {
	var out []*pb.AppendStepLogsRequest
	for _, c := range f.of("AppendStepLogs") {
		out = append(out, c.request.(*pb.AppendStepLogsRequest))
	}
	return out
}

func TestStepLogsAShortStepCarriesItsLinesAndMakesNoCall(t *testing.T) {
	f := stepLogService(t)
	carry := runOne(t, f, 3)
	if n := len(f.of("AppendStepLogs")); n != 0 {
		t.Errorf("%d AppendStepLogs calls", n)
	}
	if !slices.Equal(messages(carry.GetRecords()), []string{"line 1", "line 2", "line 3"}) ||
		!slices.Equal(sequences(carry.GetRecords()), []uint64{1, 2, 3}) || carry.GetHighestSequence() != 3 ||
		carry.GetRecords()[0].GetLevel() != pb.StepLogLevel_STEP_LOG_LEVEL_INFO {
		t.Errorf("%v", carry)
	}
	if offered := f.of("RegisterWorker")[0].request.(*pb.RegisterWorkerRequest).GetCapabilities().GetProtocolVersion(); offered != "1.7" {
		t.Errorf("offered %s", offered)
	}
}

func TestStepLogsNothingIsCapturedBelow17WithThePolicyOffWithoutAPolicyOrWithTheOptionOff(t *testing.T) {
	for _, test := range []struct {
		name  string
		setup func(*fakeService)
		opts  []worker.Option
	}{
		{"accepted 1.5", func(f *fakeService) { f.accepted = "1.5" }, nil},
		{"policy off", func(f *fakeService) { f.stepLogs = &pb.StepLogPolicy{} }, nil},
		{"no policy", func(f *fakeService) { f.stepLogs = nil }, nil},
		{"option off", func(*fakeService) {}, []worker.Option{worker.WithStepLogs(false, slog.LevelInfo)}},
	} {
		t.Run(test.name, func(t *testing.T) {
			f := stepLogService(t)
			test.setup(f)
			if carry := runOne(t, f, 300, test.opts...); carry != nil {
				t.Errorf("carry %v", carry)
			}
			if n := len(f.of("AppendStepLogs")); n != 0 {
				t.Errorf("%d AppendStepLogs calls", n)
			}
		})
	}
}

func TestStepLogsALevelThatIsNotOneOfTheFiveIsRefused(t *testing.T) {
	_, err := worker.New("http://127.0.0.1:1", "payments", "w", sagat.StaticToken("tok"), worker.WithStepLogs(true, slog.Level(2)))
	if !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Fatalf("%v", err)
	}
	if _, err := worker.New("http://127.0.0.1:1", "payments", "w", sagat.StaticToken("tok"), worker.WithStepLogs(true, worker.LevelTrace)); err != nil {
		t.Fatal(err)
	}
}

func TestStepLogsALongStepSendsTheBatchIts256thCallCutWaitsForItsAnswerAndCarriesTheRest(t *testing.T) {
	f := stepLogService(t)
	handlerDone := make(chan struct{})
	f.appendAnswers = []appendAnswer{{verdict: pb.StepLogVerdict_STEP_LOG_VERDICT_CONTINUE, hold: func() bool {
		select {
		case <-handlerDone:
			return true
		default:
			return false
		}
	}}}
	h, _ := stepLogHost(t, f)
	must(t, h.RegisterActivity("charge", logLines(260, func(context.Context) (any, error) {
		close(handlerDone)
		return 1, nil
	})))
	start(t, h)
	f.lease(lease())
	carry := reportOf(t, f)
	batches := appends(f)
	if len(batches) != 1 || !slices.Equal(sequences(batches[0].GetRecords()), span(1, 256)) {
		t.Fatalf("%d batches", len(batches))
	}
	if batches[0].GetSessionToken() != "session-1" || batches[0].GetStepId() != "charge" || batches[0].GetLeaseToken() != "lt" ||
		batches[0].GetWorkerId() != "w-1" || batches[0].GetAttempt() != 1 || batches[0].GetGeneration() != 1 {
		t.Errorf("the batch names %v", batches[0].GetTaskId())
	}
	if !slices.Equal(sequences(carry.GetRecords()), span(257, 260)) || carry.GetHighestSequence() != 260 {
		t.Errorf("carry %v highest %d", sequences(carry.GetRecords()), carry.GetHighestSequence())
	}
	// The report waited for the answer of the batch, which was held until the handler had returned.
	order := []string{}
	for _, c := range f.sequence() {
		if c.method == "answered AppendStepLogs" || c.method == "CompleteStep" {
			order = append(order, c.method)
		}
	}
	if !slices.Equal(order, []string{"answered AppendStepLogs", "CompleteStep"}) {
		t.Errorf("%v", order)
	}
}

func TestStepLogsAnAnswerHeldPast2SecondsIsGivenUp(t *testing.T) {
	f := stepLogService(t)
	f.appendAnswers = []appendAnswer{{hold: func() bool { return len(f.of("CompleteStep")) > 0 }}}
	var ended atomic.Int64
	h, _ := stepLogHost(t, f)
	must(t, h.RegisterActivity("charge", logLines(260, func(context.Context) (any, error) {
		ended.Store(time.Now().UnixNano())
		return 1, nil
	})))
	start(t, h)
	f.lease(lease())
	carry := reportOf(t, f)
	if waited := f.of("CompleteStep")[0].at.Sub(time.Unix(0, ended.Load())); waited < 2*time.Second {
		t.Errorf("reported %v after the handler's end", waited)
	}
	if !slices.Equal(sequences(carry.GetRecords()), span(257, 260)) || carry.GetHighestSequence() != 260 {
		t.Errorf("carry %v", sequences(carry.GetRecords()))
	}
}

func TestStepLogsFinalOnlyDropsWhatIsBufferedAndTheReportCarriesTheMarkWithNoRecord(t *testing.T) {
	f := stepLogService(t)
	f.appendAnswers = []appendAnswer{{verdict: pb.StepLogVerdict_STEP_LOG_VERDICT_FINAL_ONLY}}
	var seen drops
	carry := runOne(t, f, 300, seen.option())
	if n := len(f.of("AppendStepLogs")); n != 1 {
		t.Errorf("%d calls", n)
	}
	if carry == nil || len(carry.GetRecords()) != 0 || carry.GetHighestSequence() != 300 {
		t.Errorf("%v", carry)
	}
}

func TestStepLogsStopAttemptSendsNoCarryAndStopCaptureStopsTheSession(t *testing.T) {
	t.Run("stop attempt", func(t *testing.T) {
		f := stepLogService(t)
		f.appendAnswers = []appendAnswer{{verdict: pb.StepLogVerdict_STEP_LOG_VERDICT_STOP_ATTEMPT, stopReason: pb.StepLogStopReason_STEP_LOG_STOP_REASON_LEASE}}
		if carry := runOne(t, f, 300); carry != nil {
			t.Errorf("%v", carry)
		}
	})
	t.Run("stop capture", func(t *testing.T) {
		f := stepLogService(t)
		f.appendAnswers = []appendAnswer{{verdict: pb.StepLogVerdict_STEP_LOG_VERDICT_STOP_CAPTURE, stopReason: pb.StepLogStopReason_STEP_LOG_STOP_REASON_CAPTURE_OFF}}
		h, _ := stepLogHost(t, f)
		must(t, h.RegisterActivity("charge", logLines(300, nil)))
		start(t, h)
		f.lease(lease())
		until(t, "the first report", func() bool { return len(f.of("CompleteStep")) == 1 })
		f.lease(lease(func(l *pb.TaskLease) { l.TaskId, l.Attempt = "run-1/charge/2", 2 }))
		until(t, "both reports", func() bool { return len(f.of("CompleteStep")) == 2 })
		if n := len(f.of("AppendStepLogs")); n != 1 {
			t.Errorf("the second attempt of the session sent: %d calls", n)
		}
		for _, report := range f.of("CompleteStep") {
			if carry := report.request.(*pb.CompleteStepRequest).GetStepLogs(); carry != nil {
				t.Errorf("%v", carry)
			}
		}
	})
}

func TestStepLogsStopCaptureForTheSessionSendsOnceMoreWithTheTokenNowHeld(t *testing.T) {
	f := stepLogService(t)
	reregistered := make(chan struct{})
	f.appendAnswers = []appendAnswer{{
		verdict: pb.StepLogVerdict_STEP_LOG_VERDICT_STOP_CAPTURE, stopReason: pb.StepLogStopReason_STEP_LOG_STOP_REASON_SESSION,
		// Answered once the host holds the session of its second registration: not the one the batch sent.
		hold: func() bool {
			select {
			case <-reregistered:
				return true
			default:
				return false
			}
		},
	}}
	h, _ := stepLogHost(t, f)
	must(t, h.RegisterActivity("charge", logLines(260, nil)))
	start(t, h)
	f.lease(lease())
	until(t, "the batch", func() bool { return len(f.of("AppendStepLogs")) == 1 })
	must(t, worker.Reregister(h))
	close(reregistered)
	carry := reportOf(t, f)
	batches := appends(f)
	if len(batches) != 2 || batches[0].GetSessionToken() != "session-1" || batches[1].GetSessionToken() != "session-2" ||
		!slices.Equal(sequences(batches[1].GetRecords()), span(1, 256)) {
		t.Fatalf("%d batches", len(batches))
	}
	if !slices.Equal(sequences(carry.GetRecords()), span(257, 260)) {
		t.Errorf("%v", sequences(carry.GetRecords()))
	}
}

func TestStepLogsStopCaptureForTheSessionWithTheTokenStillHeldStopsAndSendsNoCarry(t *testing.T) {
	f := stepLogService(t)
	f.appendAnswers = []appendAnswer{{verdict: pb.StepLogVerdict_STEP_LOG_VERDICT_STOP_CAPTURE, stopReason: pb.StepLogStopReason_STEP_LOG_STOP_REASON_SESSION}}
	if carry := runOne(t, f, 260); carry != nil {
		t.Errorf("%v", carry)
	}
	if n := len(f.of("AppendStepLogs")); n != 1 {
		t.Errorf("%d calls", n)
	}
}

func TestStepLogsRetryLaterAndResourceExhaustedSendTheSameBatchWithoutCountingARetry(t *testing.T) {
	f := stepLogService(t)
	f.appendAnswers = []appendAnswer{
		{verdict: pb.StepLogVerdict_STEP_LOG_VERDICT_RETRY_LATER, retryMs: 60},
		{code: codes.ResourceExhausted, retryAfter: "60"},
		{code: codes.Unavailable},
		{code: codes.Unavailable},
	}
	carry := runOne(t, f, 260)
	calls := f.of("AppendStepLogs")
	if len(calls) != 5 {
		t.Fatalf("%d calls: two waits, two retries, then the answer", len(calls))
	}
	for _, c := range calls {
		if !proto.Equal(&pb.AppendStepLogsRequest{Records: c.request.(*pb.AppendStepLogsRequest).GetRecords()},
			&pb.AppendStepLogsRequest{Records: calls[0].request.(*pb.AppendStepLogsRequest).GetRecords()}) {
			t.Error("a resend holds other records")
		}
	}
	if gap := calls[1].at.Sub(calls[0].at); gap < 55*time.Millisecond {
		t.Errorf("RETRY_LATER waited %v", gap)
	}
	if gap := calls[2].at.Sub(calls[1].at); gap < 55*time.Millisecond {
		t.Errorf("RESOURCE_EXHAUSTED waited %v", gap)
	}
	if !slices.Equal(sequences(carry.GetRecords()), span(257, 260)) {
		t.Errorf("%v", sequences(carry.GetRecords()))
	}
}

func TestStepLogsUnavailableThriceDropsTheBatchAndInvalidArgumentAtOnce(t *testing.T) {
	t.Run("unavailable", func(t *testing.T) {
		f := stepLogService(t)
		f.appendAnswers = []appendAnswer{{code: codes.Unavailable}, {code: codes.Unavailable}, {code: codes.Unavailable}}
		var seen drops
		carry := runOne(t, f, 260, seen.option())
		if n := len(f.of("AppendStepLogs")); n != 3 {
			t.Errorf("%d calls", n)
		}
		if !slices.Equal(sequences(carry.GetRecords()), span(257, 260)) {
			t.Errorf("%v", sequences(carry.GetRecords()))
		}
		until(t, "the drops", func() bool { return seen.String() == "map[send_failed:256]" })
	})
	t.Run("invalid argument", func(t *testing.T) {
		f := stepLogService(t)
		f.appendAnswers = []appendAnswer{{code: codes.InvalidArgument}}
		carry := runOne(t, f, 260)
		if n := len(f.of("AppendStepLogs")); n != 1 {
			t.Errorf("%d calls", n)
		}
		if !slices.Equal(sequences(carry.GetRecords()), span(257, 260)) {
			t.Errorf("%v", sequences(carry.GetRecords()))
		}
	})
}

func TestStepLogsUnimplementedStopsTheBatchesAndTheReportCarriesTheNewest64(t *testing.T) {
	f := stepLogService(t)
	f.appendAnswers = []appendAnswer{{code: codes.Unimplemented}}
	var seen drops
	carry := runOne(t, f, 400, seen.option())
	if n := len(f.of("AppendStepLogs")); n != 1 {
		t.Errorf("%d calls", n)
	}
	if !slices.Equal(sequences(carry.GetRecords()), span(337, 400)) || carry.GetHighestSequence() != 400 {
		t.Errorf("%v", sequences(carry.GetRecords()))
	}
	until(t, "the drops", func() bool { return seen.String() == "map[report_deadline:80 send_failed:256]" })
}

func TestStepLogsALostAnswerResendsTheSameCarryAndAFailureCarriesToo(t *testing.T) {
	t.Run("resend", func(t *testing.T) {
		f := stepLogService(t)
		f.fail("CompleteStep", failure{code: codes.Unavailable})
		runOne(t, f, 3)
		until(t, "the report sent again", func() bool { return len(f.of("CompleteStep")) == 2 })
		reports := f.of("CompleteStep")
		first := reports[0].request.(*pb.CompleteStepRequest).GetStepLogs()
		if first == nil || !proto.Equal(first, reports[1].request.(*pb.CompleteStepRequest).GetStepLogs()) {
			t.Errorf("%v", first)
		}
	})
	t.Run("failure", func(t *testing.T) {
		f := stepLogService(t)
		h, _ := stepLogHost(t, f)
		must(t, h.RegisterActivity("charge", logLines(2, func(context.Context) (any, error) {
			return nil, worker.Permanent("BROKEN", "broken")
		})))
		start(t, h)
		f.lease(lease())
		until(t, "the failure", func() bool { return len(f.of("FailStep")) == 1 })
		carry := f.of("FailStep")[0].request.(*pb.FailStepRequest).GetStepLogs()
		if !slices.Equal(messages(carry.GetRecords()), []string{"line 1", "line 2"}) {
			t.Errorf("%v", carry)
		}
	})
}

func TestStepLogsTheCaptureLevelIsTheOptionAndTheHostLogsWhatItLogged(t *testing.T) {
	f := stepLogService(t)
	var hostOutput bytes.Buffer
	var mu sync.Mutex
	hostLogger := slog.New(slog.NewTextHandler(writerFunc(func(p []byte) (int, error) {
		mu.Lock()
		defer mu.Unlock()
		return hostOutput.Write(p)
	}), &slog.HandlerOptions{Level: slog.LevelError}))
	h, _ := stepLogHost(t, f, worker.WithStepLogs(true, slog.LevelDebug), worker.WithLogger(hostLogger))
	must(t, h.RegisterActivity("charge", func(ctx context.Context, _ json.RawMessage) (any, error) {
		logger := worker.TaskFrom(ctx).Logger
		logger.Log(ctx, worker.LevelTrace, "trace line")
		logger.Debug("debug line")
		logger.With("order", "o-1").WithGroup("g").Info("info line", "k", 1, slog.Group("h", "x", true))
		logger.Warn("warn line")
		logger.Error("error line", "err", errors.New("the payment provider refused"))
		return 1, nil
	}))
	start(t, h)
	f.lease(lease())
	records := reportOf(t, f).GetRecords()
	if !slices.Equal(messages(records), []string{"debug line", "info line", "warn line", "error line"}) ||
		!slices.Equal(sequences(records), []uint64{1, 2, 3, 4}) {
		t.Fatalf("%v %v", messages(records), sequences(records))
	}
	// The attributes of With and of the call are the fields, groups flattened; the five the SDK adds are not.
	var keys []string
	for _, field := range records[1].GetFields() {
		keys = append(keys, field.GetKey())
	}
	if !slices.Equal(keys, []string{"g.h.x", "g.k", "order"}) {
		t.Errorf("fields %v", keys)
	}
	if records[3].GetException() != "the payment provider refused" || records[3].GetFields()[0].GetStringValue() != "the payment provider refused" {
		t.Errorf("%q", records[3].GetException())
	}
	mu.Lock()
	defer mu.Unlock()
	var lines []string
	for _, line := range strings.Split(strings.TrimSpace(hostOutput.String()), "\n") {
		if strings.Contains(line, " line") {
			lines = append(lines, line)
		}
	}
	if len(lines) != 1 || !strings.Contains(lines[0], "error line") || !strings.Contains(lines[0], "task=run-1/charge/1") {
		t.Errorf("the host logged %q", lines)
	}
}

type writerFunc func([]byte) (int, error)

func (w writerFunc) Write(p []byte) (int, error) { return w(p) }

func TestStepLogsALineIsRedactedWithEverySecretTheProcessResolved(t *testing.T) {
	f := stepLogService(t)
	const canary = "canary-value-9Kx2"
	f.secrets = []*pb.ResolvedSecret{{Name: "key", Value: []byte(canary)}}
	h, _ := stepLogHost(t, f)
	must(t, h.RegisterActivity("charge", func(ctx context.Context, _ json.RawMessage) (any, error) {
		task := worker.TaskFrom(ctx)
		if task.Attempt == 1 {
			_, err := task.Secrets(ctx)
			return 1, err
		}
		task.Logger.Info(canary, canary, canary, "err", errors.New("failure "+canary))
		return 2, nil
	}))
	start(t, h)
	f.lease(lease())
	until(t, "the first report", func() bool { return len(f.of("CompleteStep")) == 1 })
	f.lease(lease(func(l *pb.TaskLease) { l.TaskId, l.Attempt = "run-1/charge/2", 2 }))
	until(t, "the second report", func() bool { return len(f.of("CompleteStep")) == 2 })
	report := f.of("CompleteStep")[1].request.(*pb.CompleteStepRequest)
	if len(report.GetStepLogs().GetRecords()) != 1 {
		t.Fatalf("carry %v", report.GetStepLogs())
	}
	record := report.GetStepLogs().GetRecords()[0]
	if record.GetMessage() != "[REDACTED]" || record.GetException() != "failure [REDACTED]" {
		t.Errorf("%q %q", record.GetMessage(), record.GetException())
	}
	if fields := record.GetFields(); len(fields) != 2 || fields[0].GetKey() != "[REDACTED]" || fields[0].GetStringValue() != "[REDACTED]" ||
		fields[1].GetKey() != "err" || fields[1].GetStringValue() != "failure [REDACTED]" {
		t.Errorf("%v", fields)
	}
	encoded, _ := proto.Marshal(report)
	if bytes.Contains(encoded, []byte(canary)) {
		t.Error("the report holds the secret")
	}
}

// hostile values for a logger call: a cycle, a marshaller and a String that panic, a LogValuer that panics.
type cyclic struct{ Self *cyclic }
type throwingJSON struct{}

func (throwingJSON) MarshalJSON() ([]byte, error) { panic("MarshalJSON") }

type throwingString struct{ X func() }

func (throwingString) String() string { panic("String") }

type throwingValuer struct{}

func (throwingValuer) LogValue() slog.Value { panic("LogValue") }

func TestStepLogsALoggerCallNeverPanicsIntoTheActivity(t *testing.T) {
	f := stepLogService(t)
	circular := map[string]any{}
	circular["self"] = circular
	loop := &cyclic{}
	loop.Self = loop
	// The host's own handler meets the same values: a JSON handler, which encodes them (a text handler would print the
	// cycle of maps with fmt, which never ends: the application's choice of handler, not the capture's).
	h, _ := stepLogHost(t, f, worker.WithLogger(slog.New(slog.NewJSONHandler(io.Discard, &slog.HandlerOptions{Level: slog.LevelDebug}))))
	must(t, h.RegisterActivity("charge", func(ctx context.Context, _ json.RawMessage) (any, error) {
		logger := worker.TaskFrom(ctx).Logger
		logger.Info("circular", "map", circular, "pointer", loop)
		logger.Info("marshaller", "json", throwingJSON{}, "string", throwingString{})
		logger.Info("valuer", "v", throwingValuer{}, "err", (*nilError)(nil))
		return 1, nil
	}))
	start(t, h)
	f.lease(lease())
	until(t, "the completion", func() bool { return len(f.of("CompleteStep")) == 1 })
	if n := len(f.of("FailStep")); n != 0 {
		t.Fatalf("the activity failed: %d", n)
	}
	carry := f.of("CompleteStep")[0].request.(*pb.CompleteStepRequest).GetStepLogs()
	if carry.GetHighestSequence() != 3 || len(carry.GetRecords()) != 3 {
		t.Errorf("%v", carry)
	}
}

type nilError struct{ message string }

func (e *nilError) Error() string { return e.message } // panics on a nil receiver

func TestStepLogsABatchFillsUpTo192KiBAndTheAttemptBufferStopsAtItsBound(t *testing.T) {
	f := stepLogService(t)
	var seen drops
	big := strings.Repeat("m", 8000)
	logged := make(chan struct{})
	h, ticks := stepLogHost(t, f, seen.option())
	must(t, h.RegisterActivity("charge", func(ctx context.Context, _ json.RawMessage) (any, error) {
		logger := worker.TaskFrom(ctx).Logger
		// 8 KB lines: the attempt buffer (256 KiB) is full long before 256 records, so the tick sends.
		for n := 1; n <= 100; n++ {
			logger.Info(fmt.Sprintf("%d %s", n, big))
		}
		close(logged)
		reached(func() bool { return len(f.of("AppendStepLogs")) == 1 })
		return 1, nil
	}))
	start(t, h)
	f.lease(lease())
	<-logged
	tick(t, ticks)
	reportOf(t, f)
	batches := appends(f)
	if len(batches) == 0 {
		t.Fatal("no batch")
	}
	size := proto.Size(batches[0])
	if size > 192*1024 || size <= 192*1024-8100 {
		t.Errorf("the batch is %d bytes", size)
	}
	// 33 records of 8,016 to 8,018 bytes reach 256 KiB: the 34th to the 100th are dropped.
	if len(batches[0].GetRecords()) != 24 {
		t.Errorf("%d records in the batch", len(batches[0].GetRecords()))
	}
	until(t, "the drops", func() bool { return strings.Contains(seen.String(), "buffer_full:67") })
}

func TestStepLogsOnceTheSecretSetIsFullNothingIsSent(t *testing.T) {
	t.Cleanup(worker.ResetStepLogSecrets)
	f := stepLogService(t)
	for n := 0; n <= 1024; n++ {
		f.secrets = append(f.secrets, &pb.ResolvedSecret{Name: fmt.Sprintf("s%d", n), Value: []byte(fmt.Sprintf("full-set-%06d", n))})
	}
	var seen drops
	h, ticks := stepLogHost(t, f, seen.option())
	resolved := make(chan struct{})
	ticked := make(chan struct{})
	must(t, h.RegisterActivity("charge", func(ctx context.Context, _ json.RawMessage) (any, error) {
		task := worker.TaskFrom(ctx)
		for n := 1; n <= 10; n++ {
			task.Logger.Info(fmt.Sprintf("line %d", n))
		}
		if _, err := task.Secrets(ctx); err != nil {
			return nil, err
		}
		close(resolved)
		<-ticked
		task.Logger.Info("after")
		return 1, nil
	}))
	start(t, h)
	f.lease(lease())
	<-resolved
	// The tick of the second, with ten records buffered and the set full, sends nothing.
	tick(t, ticks)
	close(ticked)
	carry := reportOf(t, f)
	if n := len(f.of("AppendStepLogs")); n != 0 {
		t.Errorf("%d calls", n)
	}
	if carry == nil || len(carry.GetRecords()) != 0 || carry.GetHighestSequence() != 11 {
		t.Errorf("%v", carry)
	}
	until(t, "the drops", func() bool { return seen.String() == "map[secret_set_full:11]" })
}

func TestStepLogsStopSendsWhatAnAttemptStillRunningBufferedOnceAsOneBatch(t *testing.T) {
	f := stepLogService(t)
	f.appendAnswers = []appendAnswer{{code: codes.Unavailable}}
	h, _ := stepLogHost(t, f)
	logged := make(chan struct{})
	must(t, h.RegisterActivity("charge", func(ctx context.Context, _ json.RawMessage) (any, error) {
		for n := 1; n <= 10; n++ {
			worker.TaskFrom(ctx).Logger.Info(fmt.Sprintf("line %d", n))
		}
		close(logged)
		<-ctx.Done()
		return nil, context.Cause(ctx)
	}))
	start(t, h)
	f.lease(lease())
	<-logged
	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()
	_ = h.Stop(ctx)
	batches := appends(f)
	if len(batches) != 1 || !slices.Equal(sequences(batches[0].GetRecords()), span(1, 10)) {
		t.Fatalf("%d batches", len(batches))
	}
	if n := len(f.of("CompleteStep")) + len(f.of("FailStep")); n != 0 {
		t.Errorf("%d reports", n)
	}
}
