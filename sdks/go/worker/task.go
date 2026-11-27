package worker

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"regexp"
	"strings"
	"sync/atomic"
	"time"
	"unicode/utf8"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	boff "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/backoff"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/safelog"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/tokenfail"
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

func instanceOf(lease *pb.TaskLease, minor int) *pb.ExecutionInstanceIdentity {
	if minor < 5 {
		return nil
	}
	return lease.GetInstance()
}

func activityInstanceOf(lease *pb.TaskLease, minor int) *ExecutionInstanceIdentity {
	instance := instanceOf(lease, minor)
	if instance == nil {
		return nil
	}
	path := make([]ExecutionScope, 0, len(instance.GetInstancePath()))
	for _, segment := range instance.GetInstancePath() {
		path = append(path, ExecutionScope{
			ScopeNodeID:    segment.GetScopeNodeId(),
			IterationIndex: segment.GetIterationIndex(),
		})
	}
	return &ExecutionInstanceIdentity{InstancePath: path, DefinitionNodeID: instance.GetDefinitionNodeId()}
}

// inflightKey is what identifies a task while it runs: the legacy task id is run/node/generation, which
// sibling iterations of a collection share, so from protocol 1.5 the instance path is part of it.
func inflightKey(lease *pb.TaskLease, minor int) string {
	key := lease.GetTaskId()
	for _, segment := range instanceOf(lease, minor).GetInstancePath() {
		key += fmt.Sprintf("/%s[%d]", segment.GetScopeNodeId(), segment.GetIterationIndex())
	}
	return key
}

// Why a lease the host received is not run (sdks/SPEC.md section 6.2).
const (
	handBackStopping  = "stopping"
	handBackCredit    = "credit"
	handBackMalformed = "malformed"
)

// handBackItem is a hand-back waiting its turn.
type handBackItem struct {
	l     *link
	lease *pb.TaskLease
	minor int
	key   string
}

// window is what one poll answer or one stream receive window has seen: the in-flight keys of the
// leases it dispatched or handed back.
type window struct {
	minor int
	seen  map[string]bool
}

func newWindow(sess *session) *window {
	return &window{minor: minorOf(sess.accepted), seen: map[string]bool{}}
}

// take dispatches a lease, or drops it when why is not empty (handing it back from protocol 1.8), and
// reports whether it dispatched it.
// A lease whose key is in flight, or was already seen in the window, is a duplicate: it is ignored and
// never reported, since its fence may be the one of the running handler.
func (h *Host) take(l *link, w *window, lease *pb.TaskLease, why string) bool {
	key := inflightKey(lease, w.minor)
	h.mu.Lock()
	_, inFlight := h.tasks[key]
	inFlight = inFlight || h.handingBack[key]
	h.mu.Unlock()
	if inFlight || w.seen[key] {
		h.log.Warn("a task that is in flight or was already received was leased again: ignored", "task", lease.GetTaskId())
		return false
	}
	w.seen[key] = true
	if why != "" {
		h.handBack(l, lease, w.minor, key, why)
		return false
	}
	h.dispatch(l, lease, key)
	return true
}

// handBackConcurrency is how many hand-backs are on their way at once: a server that floods one answer
// cannot make the host open a call per lease (sdks/SPEC.md section 6.2, decision D006-026).
const handBackConcurrency = 8

// handBackMinor is the first protocol minor on which the host hands a lease back: its server reserves
// TASK_NOT_STARTED, and an older one would record it as an ordinary failure that spends the attempt
// (decision D006-080).
const handBackMinor = 8

// handBack reports a lease the host received and will not run: FailStep TASK_NOT_STARTED, retryable,
// before any handler of it runs (sdks/SPEC.md section 6.2), on a session of protocol 1.8 or later. The
// server gives the step back without consuming the attempt when it can. Below 1.8 the lease is only
// logged and left to lapse. The report waits its turn behind at most handBackConcurrency others; until it
// ends its key is in flight, and Stop waits for it like a task in flight, so the connection is still open.
func (h *Host) handBack(l *link, lease *pb.TaskLease, minor int, key, why string) {
	if minor < handBackMinor {
		h.log.Warn("the host drops a task it did not start; its lease lapses", "task", lease.GetTaskId(), "reason", why)
		return
	}
	h.log.Warn("the host hands back a task it did not start", "task", lease.GetTaskId(), "reason", why)
	h.mu.Lock()
	h.wg.Add(1)
	h.handingBack[key] = true
	h.handBackQueue = append(h.handBackQueue, handBackItem{l: l, lease: lease, minor: minor, key: key})
	more := h.handBackWorkers < handBackConcurrency
	if more {
		h.handBackWorkers++
	}
	h.mu.Unlock()
	if more {
		go h.handBackInTurn()
	}
}

// handBackInTurn makes the hand-backs of the queue, one at a time, until the queue is empty.
func (h *Host) handBackInTurn() {
	for {
		h.mu.Lock()
		if len(h.handBackQueue) == 0 {
			h.handBackWorkers--
			h.mu.Unlock()
			return
		}
		item := h.handBackQueue[0]
		h.handBackQueue[0] = handBackItem{}
		h.handBackQueue = h.handBackQueue[1:]
		h.mu.Unlock()
		h.fail(item.l, item.lease, item.minor, "TASK_NOT_STARTED", "the host did not start this task", true, time.Now(), nil)
		h.mu.Lock()
		delete(h.handingBack, item.key)
		h.mu.Unlock()
		h.wg.Done()
	}
}

func (h *Host) dispatch(l *link, lease *pb.TaskLease, key string) {
	ctx, cancel := context.WithCancelCause(context.Background())
	task := &running{cancel: cancel}
	h.mu.Lock()
	h.tasks[key] = task
	h.wg.Add(1)
	h.mu.Unlock()
	h.emit(Event{Type: EventTaskTaken, TaskID: lease.GetTaskId(), Activity: lease.GetActivityName(), Attempt: int(lease.GetAttempt()), Traceparent: lease.GetTraceparent()})
	go func() {
		defer func() {
			cancel(nil)
			h.mu.Lock()
			delete(h.tasks, key)
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
				task.log.Error("the activity panicked")
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
		h.fail(l, lease, minor, "NO_HANDLER", "no handler is registered for the activity", false, started, nil)
		return
	}

	// The step document: {run: {id}, node, input, steps: {<node>: {output}}}.
	document := map[string]json.RawMessage{}
	if text := lease.GetInputJson(); text != "" {
		if !json.Valid([]byte(text)) {
			h.fail(l, lease, minor, "INPUT_INVALID", "the step document is not JSON", false, started, nil)
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
	var item json.RawMessage
	var index *uint32
	if minor >= 5 && len(lease.GetInstance().GetInstancePath()) > 0 {
		rawItem, hasItem := document["item"]
		rawIndex, hasIndex := document["index"]
		if hasItem || hasIndex {
			var parsed uint32
			if !hasItem || !hasIndex || json.Unmarshal(rawIndex, &parsed) != nil {
				h.fail(l, lease, minor, "INPUT_INVALID", "the collection item context is malformed", false, started, nil)
				return
			}
			item = append(json.RawMessage(nil), rawItem...)
			index = &parsed
		}
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

	// The host's own lines about the task go to its logger with the identity of the task; the activity's logger is
	// the same logger behind the attempt's step-log capture (sdks/SPEC.md section 6.9), which captures nothing of the
	// SDK's own lines nor of the five attributes the SDK adds.
	hostLog := h.log.With("task", lease.GetTaskId(), "activity", lease.GetActivityName(), "run", lease.GetRunId(), "node", node, "attempt", lease.GetAttempt())
	capture := h.newCapture(l, lease, minor, sess)
	defer h.dropCapture(capture)
	task := &Task{
		ActivityName: lease.GetActivityName(), TaskID: lease.GetTaskId(), RunID: lease.GetRunId(), Node: node,
		StepID: lease.GetStepId(), Attempt: int(lease.GetAttempt()), Generation: lease.GetGeneration(),
		Instance: activityInstanceOf(lease, minor), Item: item, Index: index,
		IdempotencyKey: lease.GetIdempotencyKey(), Traceparent: lease.GetTraceparent(), Kind: kind,
		TargetNode: lease.GetTargetNode(), TargetIdempotencyKey: lease.GetTargetIdempotencyKey(), Steps: steps,
		Logger: slog.New(safelog.Handler(&stepLogHandler{host: hostLog.Handler(), capture: capture})),
		log:    hostLog,
	}
	task.deadline.Store(lease.GetExpiresAtEpochMs())
	task.resolve = func(ctx context.Context) (*Secrets, error) { return h.resolveSecrets(ctx, l, lease, minor) }
	task.writeBlob = func(ctx context.Context, source io.Reader, options WriteBlobOptions) (BlobDescriptor, error) {
		return h.writeBlob(ctx, l, lease, minor, source, options)
	}
	task.readBlob = func(ctx context.Context, descriptor BlobDescriptor) (io.ReadCloser, error) {
		return h.readBlob(ctx, l, lease, minor, descriptor)
	}
	handlerCtx := context.WithValue(ctx, taskKey{}, task)

	// The heartbeat starts before the handler and runs until the report is made.
	var lost atomic.Bool
	// ...and keeps going until the report is made: the retries of a report may outlast the lease.
	stopBeat := h.beat(l, lease, minor, task, cancel, &lost)
	defer stopBeat()

	result, verdict, err := h.invoke(handlerCtx, reg, input)
	task.dropSecrets()
	// The handler is done: its logger captures nothing more, and the report carries what it captured.
	capture.end()

	if lost.Load() {
		h.emit(Event{Type: EventStale, TaskID: lease.GetTaskId()})
		task.log.Warn("the lease was lost: the result is dropped")
		return
	}
	if err != nil {
		if endedByShutdown(ctx, err) {
			// A deploy stopped this worker: failing the step for good would be wrong. Nothing is reported,
			// the lease lapses and the platform leases the step again.
			h.emit(Event{Type: EventDropped, TaskID: lease.GetTaskId(), Code: "SHUTDOWN"})
			task.log.Warn("the host is stopping and the activity gave up: nothing is reported, the lease will lapse")
			return
		}
		code, message, retryable := failureOf(ctx, err)
		h.fail(l, lease, minor, code, message, retryable, started, capture.carry())
		return
	}

	var output string
	var blob *BlobDescriptor
	if reg.reconciler != nil {
		if verdict.Effect != EffectApplied && verdict.Effect != EffectNotApplied && verdict.Effect != EffectUnknown {
			h.fail(l, lease, minor, "RECONCILIATION_ANSWER_INVALID", "a reconciler answers applied, not_applied or unknown", false, started, capture.carry())
			return
		}
		answer := map[string]any{"effect": string(verdict.Effect)}
		if verdict.Effect == EffectApplied && verdict.Output != nil {
			answer["output"] = verdict.Output
		}
		result = answer
	}
	if blobOutput, ok := result.(BlobOutput); ok {
		if minor < 3 {
			h.fail(l, lease, minor, "UNSUPPORTED_PROTOCOL", "blob output requires worker protocol 1.3", false, started, capture.carry())
			return
		}
		if err := validateDescriptor(blobOutput.descriptor); err != nil {
			h.fail(l, lease, minor, "OUTPUT_INVALID", "the blob output descriptor is malformed", false, started, capture.carry())
			return
		}
		copy := blobOutput.descriptor
		blob = &copy
	} else if _, none := result.(Void); !none {
		encoded, err := jsonx.Encode(result)
		if err != nil {
			message := "the output is not JSON"
			var se *sagat.Error
			if errors.As(err, &se) {
				message = se.Message
			}
			h.fail(l, lease, minor, "OUTPUT_INVALID", message, false, started, capture.carry())
			return
		}
		output = string(encoded)
	}
	if len(output) > maxOutputBytes {
		h.fail(l, lease, minor, "PAYLOAD_TOO_LARGE", "the output is over 256 KiB", false, started, capture.carry())
		return
	}
	h.complete(l, lease, minor, output, blob, started, capture.carry())
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
		// TASK_NOT_STARTED is the host's word that no handler ran (sdks/SPEC.md section 6.2): a handler that
		// uses it reports an ordinary failure, with its own message and retryable.
		if failure.Code == "TASK_NOT_STARTED" {
			return "WORKER_ERROR", failure.Message, failure.Retryable
		}
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
			request.Instance = instanceOf(lease, minor)
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
				task.log.Warn("a heartbeat failed", "code", codeOf(err))
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

// complete reports the success of a task; carry is the attempt's step-log carry, sent from protocol 1.7 and the same
// on every resend of the report.
func (h *Host) complete(l *link, lease *pb.TaskLease, minor int, output string, blob *BlobDescriptor, started time.Time, carry *pb.StepLogCarry) {
	request := &pb.CompleteStepRequest{
		TaskId: lease.GetTaskId(), RunId: lease.GetRunId(), StepId: lease.GetStepId(), Attempt: lease.GetAttempt(),
		Generation: lease.GetGeneration(), LeaseToken: lease.GetLeaseToken(), IdempotencyKey: "done-" + lease.GetTaskId(),
		OutputJson: output, Traceparent: lease.GetTraceparent(), Instance: instanceOf(lease, minor),
	}
	if blob != nil {
		request.OutputBlobRef = blob.ID
		request.OutputSha256 = blob.SHA256
	}
	if minor >= stepLogMinor {
		request.StepLogs = carry
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

// fail reports the failure of a task; carry is as for complete, and nil for a report made before a handler ran or a
// hand-back.
func (h *Host) fail(l *link, lease *pb.TaskLease, minor int, code, message string, retryable bool, started time.Time, carry *pb.StepLogCarry) {
	if !errorCode.MatchString(code) {
		code = "WORKER_ERROR"
	}
	request := &pb.FailStepRequest{
		TaskId: lease.GetTaskId(), RunId: lease.GetRunId(), StepId: lease.GetStepId(), Attempt: lease.GetAttempt(),
		Generation: lease.GetGeneration(), LeaseToken: lease.GetLeaseToken(), IdempotencyKey: "failed-" + lease.GetTaskId(),
		ErrorCode: code, SafeMessage: safeMessage(message), Retryable: retryable, Traceparent: lease.GetTraceparent(),
		Instance: instanceOf(lease, minor),
	}
	if minor >= stepLogMinor {
		request.StepLogs = carry
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
		Instance: instanceOf(lease, minor),
	}, lease.GetTraceparent())
	if err != nil {
		return nil, err
	}
	// Every value the process resolves joins its secret set before any handler can see it (sdks/SPEC.md sections 6.7
	// and 6.9); the value that does not fit stops capture for the process, with one line.
	resolved := make([][]byte, 0, len(answer.GetSecrets()))
	for _, secret := range answer.GetSecrets() {
		resolved = append(resolved, secret.GetValue())
	}
	if processCapture.secrets.add(resolved) {
		h.warnOnce("the secret set of the process is full: step-log capture stops for the process")
	}
	values := make(map[string][]byte, len(answer.GetSecrets()))
	for _, secret := range answer.GetSecrets() {
		values[secret.GetName()] = append([]byte(nil), secret.GetValue()...)
	}
	// The received buffers are wiped: the handler gets the copies above (sdks/SPEC.md section 6.7).
	for _, secret := range answer.GetSecrets() {
		clear(secret.GetValue())
	}
	return &Secrets{values: values}, nil
}

// --------------------------------------------------------------------------------------------- step logs

// warnOnce writes a line at the warning level once per process (sdks/SPEC.md section 6.9).
func (h *Host) warnOnce(line string) {
	if processCapture.warnOnce(line) {
		h.log.Warn(line)
	}
}

// newCapture is the step-log capture of an attempt when its session captures (sdks/SPEC.md section 6.9), on the link
// the task was taken on, or nil: the session was accepted below 1.7, its policy is off or absent, the worker's option
// is off, the session's or the process's capture stopped, or the host began to stop.
func (h *Host) newCapture(l *link, lease *pb.TaskLease, minor int, sess *session) *attemptCapture {
	if sess == nil || sess.capture == nil || !sess.capture.enabled || sess.capture.stopped.Load() ||
		processCapture.secrets.full.Load() || h.stopping.Load() {
		return nil
	}
	ticker := h.stepLogTicker
	if ticker == nil {
		ticker = func() (<-chan time.Time, func()) {
			t := time.NewTicker(limitFlushEvery)
			return t.C, t.Stop
		}
	}
	taskID := lease.GetTaskId()
	capture := newAttemptCapture(captureHost{
		life: l.life,
		session: func() (string, string) {
			current := h.session.Load()
			return current.workerID, current.token
		},
		append: func(ctx context.Context, request *pb.AppendStepLogsRequest, timeout time.Duration) appendOutcome {
			return appendOutcomeOf(ctx, l, request, timeout)
		},
		timeout: h.timeout,
		dropped: func(reason string, records int) {
			h.emit(Event{Type: EventStepLogsDropped, TaskID: taskID, Code: reason, Records: records})
		},
		warnOnce: h.warnOnce,
		ticker:   ticker,
	}, sess.capture, attemptIdentity{
		taskID: taskID, runID: lease.GetRunId(), stepID: lease.GetStepId(), leaseToken: lease.GetLeaseToken(),
		attempt: lease.GetAttempt(), generation: lease.GetGeneration(), instance: instanceOf(lease, minor),
	}, h.cfg.stepLogs.level)
	h.mu.Lock()
	h.captures[capture] = struct{}{}
	h.mu.Unlock()
	return capture
}

// dropCapture lets an attempt's capture go when its task ends: an attempt that made no report (its lease was lost,
// it lapses at shutdown) sends nothing more.
func (h *Host) dropCapture(capture *attemptCapture) {
	if capture == nil {
		return
	}
	capture.close()
	h.mu.Lock()
	delete(h.captures, capture)
	h.mu.Unlock()
}

// appendOutcomeOf makes one AppendStepLogs and reads its answer, or the gRPC status of its failure, as the attempt
// does (contracts.md section 4 of design 041).
func appendOutcomeOf(ctx context.Context, l *link, request *pb.AppendStepLogsRequest, timeout time.Duration) appendOutcome {
	answer, err := l.rpc.appendStepLogs(ctx, request, timeout)
	if err == nil {
		return appendOutcome{answered: true, verdict: answer.GetVerdict(), stopReason: answer.GetStopReason(), retryMs: answer.GetRetryAfterMs()}
	}
	if ctx.Err() != nil {
		return appendOutcome{status: "CANCELLED"}
	}
	var se *sagat.Error
	if !errors.As(err, &se) {
		return appendOutcome{status: "UNAVAILABLE"}
	}
	status := se.GRPCStatus
	// A token source that failed is not a refusal of the session: the call is tried again as after an outage.
	if status == "" || tokenfail.Is(se) {
		status = "UNAVAILABLE"
	}
	return appendOutcome{status: status, retryAfter: se.RetryAfter}
}
