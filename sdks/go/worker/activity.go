package worker

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"sort"
	"sync"
	"sync/atomic"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
)

// TaskKind tells the three kinds of task apart (worker protocol 1.2).
type TaskKind string

// The kinds of task. A server at protocol 1.0 or 1.1 only sends forward tasks.
const (
	KindForward        TaskKind = "FORWARD"
	KindCompensation   TaskKind = "COMPENSATION"
	KindReconciliation TaskKind = "RECONCILIATION"
)

// The reasons an activity's context is cancelled; context.Cause(ctx) returns one of them.
var (
	// ErrRunCancelled: the run was cancelled. The handler decides what to do; when it ends, the host
	// reports whatever it returned, and a handler that returns its cancellation error is reported as
	// CANCELLED, not retryable.
	ErrRunCancelled = errors.New("run-cancelled")
	// ErrLeaseLost: the lease on the task is gone (it expired or was taken); whatever the handler
	// returns is dropped.
	ErrLeaseLost = errors.New("lease-lost")
	// ErrShutdown: the host is stopping and the grace period passed. A handler that then returns its
	// cancellation error (ctx.Err(), context.Cause(ctx)) is not reported: the lease lapses and the
	// platform leases the step again, because failing a step for good for a deploy would be wrong. A
	// handler that returns a value, or fails with another error, is reported as usual.
	ErrShutdown = errors.New("shutdown")
	// ErrInputInvalid: the input of the run does not have the type of the handler. Return it to fail
	// the step for good with the code INPUT_INVALID; Typed does.
	ErrInputInvalid = errors.New("input-invalid")
)

// ActivityFailure is how an activity fails on purpose. Code is a stable machine code
// ([A-Z0-9_]{1,64}; any other is reported as WORKER_ERROR); Message is recorded with the step, so it
// must not carry a payload or a secret.
type ActivityFailure struct {
	Code      string
	Message   string
	Retryable bool
}

// Error implements error.
func (f *ActivityFailure) Error() string { return f.Code + ": " + f.Message }

// Retryable is a failure after which the step may be tried again as its node's retry policy says.
func Retryable(code, message string) *ActivityFailure {
	return &ActivityFailure{Code: code, Message: message, Retryable: true}
}

// Permanent is a failure for good: the policy goes to its error branch or its saga compensates.
func Permanent(code, message string) *ActivityFailure {
	return &ActivityFailure{Code: code, Message: message, Retryable: false}
}

// OutcomeUnknown is the failure of an activity that sent a request and cannot tell whether it was
// applied (a timeout after connecting, an answer that never arrived). The platform then reconciles
// the step, when it has a ReconcileWith, instead of retrying it blindly.
func OutcomeUnknown(message string) *ActivityFailure {
	return &ActivityFailure{Code: "OUTCOME_UNKNOWN", Message: message, Retryable: false}
}

// Effect is what a reconciler found out about the effect of a step.
type Effect string

// The three answers of a reconciler.
const (
	EffectApplied    Effect = "applied"
	EffectNotApplied Effect = "not_applied"
	EffectUnknown    Effect = "unknown"
)

// Verdict is the answer of a reconciler. The host completes the task with exactly
// {"effect": ..., "output": ...} (output only for applied) and nothing else.
type Verdict struct {
	Effect Effect
	// Output is what the step would have returned; only for EffectApplied, and nil for none.
	Output any
}

// Applied: the effect happened. output is what the step would have returned, or nil.
func Applied(output any) Verdict { return Verdict{Effect: EffectApplied, Output: output} }

// NotApplied: the effect did not happen; the platform may try it again, with the same key.
func NotApplied() Verdict { return Verdict{Effect: EffectNotApplied} }

// Unknown: the reconciler cannot tell; the decision goes to a person.
func Unknown() Verdict { return Verdict{Effect: EffectUnknown} }

// Void is the output of an activity that has none: the host completes the task with no value, which
// is not the same as the JSON null that a nil output is.
type Void struct{}

// Handler runs an activity, a compensation or a reconciler. input is the input of the run as JSON;
// the result is encoded as JSON. Return an *ActivityFailure to fail on purpose; any other error is
// reported as ACTIVITY_ERROR, retryable, with only its type name as the message, never its text.
//
// ctx is cancelled with a cause (context.Cause): ErrRunCancelled, ErrLeaseLost or ErrShutdown. Use
// TaskFrom(ctx) for what the host knows about the task.
type Handler func(ctx context.Context, input json.RawMessage) (any, error)

// ReconcilerHandler answers a Verdict about the step named by TaskFrom(ctx).TargetNode.
type ReconcilerHandler func(ctx context.Context, input json.RawMessage) (Verdict, error)

// Typed adapts a handler of a declared input type: the input is decoded into I before the handler
// runs, and a document that cannot be decoded fails the step for good as INPUT_INVALID. Decoding
// into any turns numbers into float64; declare int64 fields, or take json.RawMessage, to keep
// integers exact.
func Typed[I, O any](fn func(ctx context.Context, input I) (O, error)) Handler {
	return func(ctx context.Context, raw json.RawMessage) (any, error) {
		var input I
		if err := decode(raw, &input); err != nil {
			return nil, ErrInputInvalid
		}
		return fn(ctx, input)
	}
}

// TypedReconciler is Typed for a reconciler.
func TypedReconciler[I any](fn func(ctx context.Context, input I) (Verdict, error)) ReconcilerHandler {
	return func(ctx context.Context, raw json.RawMessage) (Verdict, error) {
		var input I
		if err := decode(raw, &input); err != nil {
			return Verdict{}, ErrInputInvalid
		}
		return fn(ctx, input)
	}
}

func decode(raw json.RawMessage, into any) error {
	if len(bytes.TrimSpace(raw)) == 0 {
		return nil
	}
	return json.Unmarshal(raw, into)
}

// Task is what the host knows about the attempt a handler is running (sdks/SPEC.md, section 6.3).
// Everything but Deadline and Secrets is fixed when the task is taken.
type Task struct {
	ActivityName string
	TaskID       string
	RunID        string
	// Node is the id of the step in the workflow definition ("charge"), from the step document.
	Node string
	// StepID is the platform's id of the step (step_id of the lease), which the reports carry.
	StepID     string
	Attempt    int
	Generation int64
	// IdempotencyKey is stable across the attempts of the step: send it to the provider of an
	// external effect. For a compensation it is the key of this compensation.
	IdempotencyKey string
	Traceparent    string
	Kind           TaskKind
	// TargetNode is, for a compensation or a reconciliation, the forward step it is about.
	TargetNode string
	// TargetIdempotencyKey identifies the effect of that step at the provider: send it to the
	// provider, not IdempotencyKey.
	TargetIdempotencyKey string
	// Steps are the outputs of the steps that completed before this one, by node, as JSON.
	Steps map[string]json.RawMessage
	// Logger is the host's logger with the identity of the task. Never log an input, an output or a
	// secret through it.
	Logger *slog.Logger

	deadline atomic.Int64 // epoch milliseconds

	secretsMu    sync.Mutex
	secretsDone  bool
	secrets      *Secrets
	resolve      func(ctx context.Context) (*Secrets, error)
	writeBlob    func(context.Context, io.Reader, WriteBlobOptions) (BlobDescriptor, error)
	readBlob     func(context.Context, BlobDescriptor) (io.ReadCloser, error)
	droppedAfter atomic.Bool
}

// Deadline is the end of the lease; it moves forward with every heartbeat.
func (t *Task) Deadline() time.Time { return time.UnixMilli(t.deadline.Load()) }

// Secrets resolves the secrets the step declares, once per attempt, on first use, and returns them
// as a read-only set. The values are credentials: never log them or put them in an error. The host
// wipes them when the handler returns. Against protocol 1 it returns UNSUPPORTED_PROTOCOL.
func (t *Task) Secrets(ctx context.Context) (*Secrets, error) {
	t.secretsMu.Lock()
	defer t.secretsMu.Unlock()
	if t.droppedAfter.Load() {
		return nil, errAttemptOver()
	}
	if t.secretsDone {
		return t.secrets, nil
	}
	secrets, err := t.resolve(ctx)
	if err != nil {
		// A failed resolution is not kept for the rest of the attempt: the next call asks again.
		return nil, err
	}
	t.secrets, t.secretsDone = secrets, true
	return t.secrets, nil
}

// WriteBlob streams one source to blob storage. It reads source once, in bounded chunks, and
// returns only after the service descriptor agrees with the bytes and SHA-256 observed locally.
func (t *Task) WriteBlob(ctx context.Context, source io.Reader, options WriteBlobOptions) (BlobDescriptor, error) {
	if t.droppedAfter.Load() {
		return BlobDescriptor{}, errAttemptOver()
	}
	if source == nil {
		return BlobDescriptor{}, sagat.Invalid("a blob source is required")
	}
	return t.writeBlob(ctx, source, options)
}

// ReadBlob returns a stream that verifies the descriptor while it is consumed. io.EOF is returned
// only after the gRPC stream ended cleanly and its byte count and SHA-256 matched the descriptor.
func (t *Task) ReadBlob(ctx context.Context, descriptor BlobDescriptor) (io.ReadCloser, error) {
	if t.droppedAfter.Load() {
		return nil, errAttemptOver()
	}
	return t.readBlob(ctx, descriptor)
}

// dropSecrets wipes the values; the handler has returned.
func (t *Task) dropSecrets() {
	t.secretsMu.Lock()
	defer t.secretsMu.Unlock()
	t.droppedAfter.Store(true)
	if t.secrets != nil {
		t.secrets.wipe()
	}
}

type taskKey struct{}

// TaskFrom returns the Task of the handler's context, or nil outside a handler.
func TaskFrom(ctx context.Context) *Task {
	t, _ := ctx.Value(taskKey{}).(*Task)
	return t
}

// Secrets is the read-only set of secrets of one attempt.
type Secrets struct {
	mu      sync.RWMutex
	values  map[string][]byte
	dropped bool
}

// errAttemptOver is what a later access to the secrets of an attempt raises: the handler has returned and the
// values are gone (SPEC section 6.7).
func errAttemptOver() error {
	return sagat.Invalid("the attempt is over: its secrets were dropped when the handler returned")
}

func (s *Secrets) lookup(name string) ([]byte, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	if s.dropped {
		return nil, errAttemptOver()
	}
	value, ok := s.values[name]
	if !ok {
		return nil, sagat.Errorf(sagat.CodeNotFound, "the step declares no secret named %s", name)
	}
	return append([]byte(nil), value...), nil
}

// Bytes returns a copy of the bytes of a secret the step declares.
func (s *Secrets) Bytes(name string) ([]byte, error) { return s.lookup(name) }

// Text returns a secret as UTF-8 text.
func (s *Secrets) Text(name string) (string, error) {
	value, err := s.lookup(name)
	return string(value), err
}

// Names lists the secrets the step declares, sorted.
func (s *Secrets) Names() []string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	names := make([]string, 0, len(s.values))
	for name := range s.values {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}

// Format prints <redacted> whatever the verb: no way of printing the set shows a value.
func (s *Secrets) Format(f fmt.State, _ rune) { _, _ = fmt.Fprint(f, "<redacted>") }

// String is <redacted>.
func (s *Secrets) String() string { return "<redacted>" }

// GoString is <redacted>.
func (s *Secrets) GoString() string { return "<redacted>" }

// LogValue is how log/slog shows the set.
func (s *Secrets) LogValue() slog.Value { return slog.StringValue("<redacted>") }

func (s *Secrets) wipe() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.dropped = true
	for name, value := range s.values {
		clear(value)
		delete(s.values, name)
	}
}

// panicError is what a panicking handler is reported as: only its existence, never the value.
type panicError struct{}

func (panicError) Error() string { return "the activity panicked" }

func typeName(err error) string { return fmt.Sprintf("%T", err) }
