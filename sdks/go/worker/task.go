package worker

import (
	"context"
	"encoding/json"
	"errors"
	"regexp"
	"strings"
	"sync/atomic"
	"time"
	"unicode/utf8"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	boff "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/backoff"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
)

var errorCode = regexp.MustCompile(`^[A-Z0-9_]{1,64}$`)

// minorOf is the minor version of an accepted protocol: "1" is 1.0.
func minorOf(version string) int {
	if version == "1" {
		return 0
	}
	rest, ok := strings.CutPrefix(version, "1.")
	if !ok || rest == "" {
		return 0
	}
	minor := 0
	for _, c := range rest {
		if c < '0' || c > '9' {
			return 0
		}
		minor = minor*10 + int(c-'0')
		if minor > 1_000_000 {
			return minor
		}
	}
	return minor
}

func (h *Host) dispatch(l *link, lease *pb.TaskLease) {
	ctx, cancel := context.WithCancelCause(context.Background())
	task := &running{cancel: cancel}
	h.mu.Lock()
	h.tasks[lease.GetTaskId()] = task
	h.wg.Add(1)
	h.mu.Unlock()
	h.emit(Event{Type: EventTaskTaken, TaskID: lease.GetTaskId(), Activity: lease.GetActivityName(), Attempt: int(lease.GetAttempt()), Traceparent: lease.GetTraceparent()})
	go func() {
		defer func() {
			cancel(nil)
			h.mu.Lock()
			delete(h.tasks, lease.GetTaskId())
			h.mu.Unlock()
			h.wg.Done()
			h.signal()
		}()
		h.execute(ctx, l, cancel, lease)
	}()
}

// invoke runs the handler; a panic is reported as a failure of the activity, never as the value that
// was panicked with.
func (h *Host) invoke(ctx context.Context, reg *registered, input json.RawMessage) (result any, verdict Verdict, err error) {
	defer func() {
		if recovered := recover(); recovered != nil {
			err = panicError{}
			if task := TaskFrom(ctx); task != nil {
				task.Logger.Error("the activity panicked")
			}
		}
	}()
	if reg.reconciler != nil {
		verdict, err = reg.reconciler(ctx, input)
		return
	}
	result, err = reg.handler(ctx, input)
	return
}

func (h *Host) execute(ctx context.Context, l *link, cancel context.CancelCauseFunc, lease *pb.TaskLease) {
	started := time.Now()
	sess := h.session.Load()
	minor := minorOf(sess.accepted)
	reg := h.lookup(lease.GetActivityName())
	if reg == nil {
		h.fail(l, lease, "NO_HANDLER", "no handler is registered for the activity", false, started)
		return
	}

	// The step document: {run: {id}, node, input, steps: {<node>: {output}}}.
	document := map[string]json.RawMessage{}
	if text := lease.GetInputJson(); text != "" {
		if !json.Valid([]byte(text)) {
			h.fail(l, lease, "INPUT_INVALID", "the step document is not JSON", false, started)
			return
		}
		if parsed, err := jsonx.Object([]byte(text)); err == nil {
			document = parsed
		}
	}
	input := document["input"]
	if len(input) == 0 {
		input = json.RawMessage("null")
	}
	steps := map[string]json.RawMessage{}
	if stepsDocument, err := jsonx.Object(document["steps"]); err == nil {
		for node, value := range stepsDocument {
			if member, err := jsonx.Object(value); err == nil {
				if output, ok := member["output"]; ok {
					steps[node] = output
				}
			}
		}
	}
	node, _ := jsonx.String(document, "node")
	if node == "" {
		node = lease.GetStepId()
	}
	kind := KindForward
	if minor >= 2 {
		switch lease.GetTaskKind() {
		case string(KindCompensation):
			kind = KindCompensation
		case string(KindReconciliation):
			kind = KindReconciliation
		}
	}

	task := &Task{
		ActivityName: lease.GetActivityName(), TaskID: lease.GetTaskId(), RunID: lease.GetRunId(), Node: node,
		StepID: lease.GetStepId(), Attempt: int(lease.GetAttempt()), Generation: lease.GetGeneration(),
		IdempotencyKey: lease.GetIdempotencyKey(), Traceparent: lease.GetTraceparent(), Kind: kind,
		TargetNode: lease.GetTargetNode(), TargetIdempotencyKey: lease.GetTargetIdempotencyKey(), Steps: steps,
		Logger: h.log.With("task", lease.GetTaskId(), "activity", lease.GetActivityName(), "run", lease.GetRunId(), "node", node, "attempt", lease.GetAttempt()),
	}
	task.deadline.Store(lease.GetExpiresAtEpochMs())
	task.resolve = func(ctx context.Context) (*Secrets, error) { return h.resolveSecrets(ctx, l, lease, minor) }
	handlerCtx := context.WithValue(ctx, taskKey{}, task)

	// The heartbeat starts before the handler and runs until the report is made.
	var lost atomic.Bool
	// ...and keeps going until the report is made: the retries of a report may outlast the lease.
	stopBeat := h.beat(l, lease, minor, task, cancel, &lost)
	defer stopBeat()

	result, verdict, err := h.invoke(handlerCtx, reg, input)
	task.dropSecrets()

	if lost.Load() {
		h.emit(Event{Type: EventStale, TaskID: lease.GetTaskId()})
		task.Logger.Warn("the lease was lost: the result is dropped")
		return
	}
	if err != nil {
		if endedByShutdown(ctx, err) {
			// A deploy stopped this worker: failing the step for good would be wrong. Nothing is reported,
			// the lease lapses and the platform leases the step again.
			h.emit(Event{Type: EventDropped, TaskID: lease.GetTaskId(), Code: "SHUTDOWN"})
			task.Logger.Warn("the host is stopping and the activity gave up: nothing is reported, the lease will lapse")
			return
		}
		code, message, retryable := failureOf(ctx, err)
		h.fail(l, lease, code, message, retryable, started)
		return
	}

	var output string
	if reg.reconciler != nil {
		if verdict.Effect != EffectApplied && verdict.Effect != EffectNotApplied && verdict.Effect != EffectUnknown {
			h.fail(l, lease, "RECONCILIATION_ANSWER_INVALID", "a reconciler answers applied, not_applied or unknown", false, started)
			return
		}
		answer := map[string]any{"effect": string(verdict.Effect)}
		if verdict.Effect == EffectApplied && verdict.Output != nil {
			answer["output"] = verdict.Output
		}
		result = answer
	}
	if _, none := result.(Void); !none {
		encoded, err := jsonx.Encode(result)
		if err != nil {
			message := "the output is not JSON"
			var se *sagat.Error
			if errors.As(err, &se) {
				message = se.Message
			}
			h.fail(l, lease, "OUTPUT_INVALID", message, false, started)
			return
		}
		output = string(encoded)
	}
	if len(output) > maxOutputBytes {
		h.fail(l, lease, "PAYLOAD_TOO_LARGE", "the output is over 256 KiB", false, started)
		return
	}
	h.complete(l, lease, output, started)
}

func (h *Host) lookup(name string) *registered {
	h.mu.Lock()
	defer h.mu.Unlock()
	return h.handlers[name]
}

// failureOf turns what a handler returned into what the host reports. An ActivityFailure is
// reported as it is; the cancellation of the activity's own context is CANCELLED for good; any other
// error says only its type, because its text may hold the data the handler was given.
func failureOf(ctx context.Context, err error) (code, message string, retryable bool) {
	var failure *ActivityFailure
	switch {
	case errors.As(err, &failure):
		return failure.Code, failure.Message, failure.Retryable
	case errors.Is(err, ErrInputInvalid):
		return "INPUT_INVALID", "the input does not have the type of the handler", false
	case ctx.Err() != nil && isCancellation(err):
		return "CANCELLED", "the activity was cancelled", false
	default:
		return "ACTIVITY_ERROR", typeName(err), true
	}
}

// endedByShutdown reports whether a handler that was cancelled because the host is stopping ended by
// raising its cancellation error. Any other end (a value, or another failure) is reported as usual.
func endedByShutdown(ctx context.Context, err error) bool {
	var failure *ActivityFailure
	if errors.As(err, &failure) || ctx.Err() == nil || !errors.Is(context.Cause(ctx), ErrShutdown) {
		return false
	}
	return isCancellation(err)
}

func isCancellation(err error) bool {
	return errors.Is(err, context.Canceled) || errors.Is(err, ErrRunCancelled) || errors.Is(err, ErrLeaseLost) ||
		errors.Is(err, ErrShutdown) || sagat.IsCode(err, sagat.CodeCancelled)
}

// ---------------------------------------------------------------------------------------- heartbeat

// beat starts the heartbeat of a task and returns the function that stops it. The interval is a
// third of the time left on the lease, never under 500 ms and never over 10 s, unless it was fixed.
func (h *Host) beat(l *link, lease *pb.TaskLease, minor int, task *Task, cancel context.CancelCauseFunc, lost *atomic.Bool) (stop func()) {
	ctx, cancelBeat := context.WithCancel(l.life)
	finished := make(chan struct{})
	interval := func() time.Duration {
		if h.cfg.heartbeatInterval > 0 {
			return h.cfg.heartbeatInterval
		}
		left := time.Until(task.Deadline())
		return min(10*time.Second, max(500*time.Millisecond, left/3))
	}
	go func() {
		defer close(finished)
		timer := time.NewTimer(interval())
		defer timer.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-timer.C:
			}
			request := &pb.HeartbeatRequest{
				TaskId: lease.GetTaskId(), WorkerId: h.session.Load().workerID, Generation: lease.GetGeneration(), LeaseToken: lease.GetLeaseToken(),
			}
			// The step identity came with 1.1.
			if minor >= 1 {
				request.RunId, request.StepId, request.Attempt = lease.GetRunId(), lease.GetStepId(), lease.GetAttempt()
			}
			answer, err := l.rpc.heartbeat(ctx, request, lease.GetTraceparent())
			if err == nil {
				h.emit(Event{Type: EventHeartbeat, TaskID: lease.GetTaskId()})
				if expires := answer.GetLeaseExpiresAtEpochMs(); expires > 0 {
					task.deadline.Store(expires)
				}
				if answer.GetCancelRequested() {
					cancel(ErrRunCancelled)
				}
			} else {
				if ctx.Err() != nil {
					return
				}
				if sagat.IsCode(err, sagat.CodeStaleAttempt) || sagat.IsCode(err, sagat.CodeConflict) {
					lost.Store(true)
					cancel(ErrLeaseLost)
					return
				}
				// Anything else: the next beat tries again; the lease is ours until it runs out.
				task.Logger.Warn("a heartbeat failed", "code", codeOf(err))
			}
			timer.Reset(interval())
		}
	}()
	return func() {
		cancelBeat()
		<-finished
	}
}

func codeOf(err error) string {
	var se *sagat.Error
	if errors.As(err, &se) {
		return string(se.Code)
	}
	return "unknown"
}

// ---------------------------------------------------------------------------------------- reporting

// report makes a call that reports the result of a task, repeating it with the same body, and so the
// same idempotency key, while it is safe (at most reportAttempts times). A STALE_ATTEMPT is not an
// error to the caller: the lease is gone, so nothing is reported.
func (h *Host) report(l *link, call func(ctx context.Context) error) (stale bool, err error) {
	for n := 0; ; n++ {
		err = call(l.life)
		if err == nil {
			return false, nil
		}
		var se *sagat.Error
		if errors.As(err, &se) {
			if se.Code == sagat.CodeStaleAttempt {
				return true, nil
			}
			if boff.Retryable(se) && n+1 < reportAttempts {
				if sleepErr := boff.Sleep(l.life, boff.After(h.retry, n, se, nil)); sleepErr != nil {
					return false, err
				}
				continue
			}
		}
		return false, err
	}
}

func (h *Host) complete(l *link, lease *pb.TaskLease, output string, started time.Time) {
	request := &pb.CompleteStepRequest{
		TaskId: lease.GetTaskId(), RunId: lease.GetRunId(), StepId: lease.GetStepId(), Attempt: lease.GetAttempt(),
		Generation: lease.GetGeneration(), LeaseToken: lease.GetLeaseToken(), IdempotencyKey: "done-" + lease.GetTaskId(),
		OutputJson: output, Traceparent: lease.GetTraceparent(),
	}
	stale, err := h.report(l, func(ctx context.Context) error {
		_, err := l.rpc.completeStep(ctx, request, lease.GetTraceparent())
		return err
	})
	switch {
	case err != nil:
		h.emit(Event{Type: EventDropped, TaskID: lease.GetTaskId(), Code: codeOf(err)})
		h.log.Error("the completion could not be reported", "task", lease.GetTaskId(), "code", codeOf(err))
	case stale:
		h.emit(Event{Type: EventStale, TaskID: lease.GetTaskId()})
	default:
		h.emit(Event{Type: EventCompleted, TaskID: lease.GetTaskId(), Duration: time.Since(started)})
	}
}

func (h *Host) fail(l *link, lease *pb.TaskLease, code, message string, retryable bool, started time.Time) {
	if !errorCode.MatchString(code) {
		code = "WORKER_ERROR"
	}
	request := &pb.FailStepRequest{
		TaskId: lease.GetTaskId(), RunId: lease.GetRunId(), StepId: lease.GetStepId(), Attempt: lease.GetAttempt(),
		Generation: lease.GetGeneration(), LeaseToken: lease.GetLeaseToken(), IdempotencyKey: "failed-" + lease.GetTaskId(),
		ErrorCode: code, SafeMessage: safeMessage(message), Retryable: retryable, Traceparent: lease.GetTraceparent(),
	}
	stale, err := h.report(l, func(ctx context.Context) error {
		_, err := l.rpc.failStep(ctx, request, lease.GetTraceparent())
		return err
	})
	switch {
	case err != nil:
		h.emit(Event{Type: EventDropped, TaskID: lease.GetTaskId(), Code: codeOf(err)})
		h.log.Error("the failure could not be reported", "task", lease.GetTaskId(), "code", codeOf(err))
	case stale:
		h.emit(Event{Type: EventStale, TaskID: lease.GetTaskId()})
	default:
		h.emit(Event{Type: EventFailed, TaskID: lease.GetTaskId(), Code: code, Retryable: retryable, Duration: time.Since(started)})
	}
}

// safeMessage is the message of a failure as the server records it: valid UTF-8, at most 512
// characters.
func safeMessage(message string) string {
	message = strings.ToValidUTF8(message, "�")
	if utf8.RuneCountInString(message) <= 512 {
		return message
	}
	return string([]rune(message)[:512])
}

// --------------------------------------------------------------------------------------------- secrets

func (h *Host) resolveSecrets(ctx context.Context, l *link, lease *pb.TaskLease, minor int) (*Secrets, error) {
	if minor < 2 {
		return nil, sagat.NewError(sagat.CodeUnsupportedProtocol, "the server speaks a protocol older than 1.2, which has no ResolveSecrets")
	}
	answer, err := l.rpc.resolveSecrets(ctx, &pb.ResolveSecretsRequest{
		TaskId: lease.GetTaskId(), RunId: lease.GetRunId(), StepId: lease.GetStepId(), Attempt: lease.GetAttempt(),
		Generation: lease.GetGeneration(), LeaseToken: lease.GetLeaseToken(), Traceparent: lease.GetTraceparent(),
	}, lease.GetTraceparent())
	if err != nil {
		return nil, err
	}
	values := make(map[string][]byte, len(answer.GetSecrets()))
	for _, secret := range answer.GetSecrets() {
		values[secret.GetName()] = append([]byte(nil), secret.GetValue()...)
	}
	return &Secrets{values: values}, nil
}
