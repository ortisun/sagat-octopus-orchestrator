package workflow

import (
	"regexp"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
)

// An option that is not given is absent from the document, never null: every option below writes its
// key when it is applied, even with a zero value (Irreversible(false) writes false).

var secretName = regexp.MustCompile(`^[a-z][a-z0-9._-]{0,63}$`)

func millis(d time.Duration) int64 { return d.Milliseconds() }

// ---- retry

// Strategy is how the delay between attempts grows.
type Strategy string

// The retry strategies of the IR.
const (
	Fixed       Strategy = "fixed"
	Exponential Strategy = "exponential"
)

// RetryOption sets one member of a retry policy.
type RetryOption func(map[string]any)

// MaxAttempts is the number of attempts in all, the first included.
func MaxAttempts(n int) RetryOption { return func(m map[string]any) { m["maxAttempts"] = int64(n) } }

// InitialDelay is the wait before the second attempt.
func InitialDelay(d time.Duration) RetryOption {
	return func(m map[string]any) { m["initialDelayMs"] = millis(d) }
}

// MaxDelay is the longest wait between two attempts.
func MaxDelay(d time.Duration) RetryOption {
	return func(m map[string]any) { m["maxDelayMs"] = millis(d) }
}

// WithStrategy is Fixed or Exponential.
func WithStrategy(s Strategy) RetryOption {
	return func(m map[string]any) { m["strategy"] = string(s) }
}

// Jitter turns the random spread of the delays on or off.
func Jitter(on bool) RetryOption { return func(m map[string]any) { m["jitter"] = on } }

func retryOf(opts []RetryOption) map[string]any {
	m := map[string]any{}
	for _, opt := range opts {
		opt(m)
	}
	return m
}

// ---- step

// StepOption sets one option of a step.
type StepOption func(*stepSpec)

type stepSpec struct {
	fields map[string]any
	err    error
}

// Retry is the retry policy of the step; the server's retry policy, not the SDK's.
func Retry(opts ...RetryOption) StepOption {
	return func(s *stepSpec) { s.fields["retry"] = retryOf(opts) }
}

// Timeout bounds one attempt of the step.
func Timeout(d time.Duration) StepOption {
	return func(s *stepSpec) { s.fields["timeoutMs"] = millis(d) }
}

// CompensateWith names the activity that undoes the step. It must be idempotent and tolerate an
// effect that never happened.
func CompensateWith(activity string) StepOption {
	return func(s *stepSpec) { s.fields["compensateWith"] = activity }
}

// ReconcileWith names the activity that finds out whether the step happened, after an attempt with an
// unknown outcome.
func ReconcileWith(activity string) StepOption {
	return func(s *stepSpec) { s.fields["reconcileWith"] = activity }
}

// IdempotencyTemplate is the template of the idempotency key the step's attempts carry.
func IdempotencyTemplate(template string) StepOption {
	return func(s *stepSpec) { s.fields["idempotencyTemplate"] = template }
}

// Secrets names the secrets of the namespace the worker may ask for while it holds a lease on the
// step. Names only: a value is never part of a definition.
func Secrets(names ...string) StepOption {
	return func(s *stepSpec) {
		list := make([]any, 0, len(names))
		for _, name := range names {
			if !secretName.MatchString(name) {
				if s.err == nil {
					s.err = sagat.Invalid("a step names a secret that is not a name: %q", name)
				}
				return
			}
			list = append(list, name)
		}
		s.fields["secrets"] = list
	}
}

// CompensationRetry is the retry policy of the compensation.
func CompensationRetry(opts ...RetryOption) StepOption {
	return func(s *stepSpec) { s.fields["compensationRetry"] = retryOf(opts) }
}

// CompensationTimeout bounds one attempt of the compensation.
func CompensationTimeout(d time.Duration) StepOption {
	return func(s *stepSpec) { s.fields["compensationTimeoutMs"] = millis(d) }
}

// Irreversible marks an effect that cannot be undone. It cannot be combined with CompensateWith.
func Irreversible(on bool) StepOption { return func(s *stepSpec) { s.fields["irreversible"] = on } }

// HighRisk marks a step whose attempt with an unknown outcome is never retried or compensated
// blindly: it is reconciled when ReconcileWith says how, and otherwise waits for an operator.
func HighRisk(on bool) StepOption { return func(s *stepSpec) { s.fields["highRisk"] = on } }

// OnError names the node the flow goes to when the step fails for good.
func OnError(node string) StepOption { return func(s *stepSpec) { s.fields["onError"] = node } }

// OutputSchema sets the JSON Schema of the node result carried by journal completion events.
// It is valid for activity steps.
func OutputSchema(schema map[string]any) StepOption {
	return func(s *stepSpec) { s.fields["outputSchema"] = schema }
}

// ScriptOption sets one optional field of a pinned script node.
type ScriptOption func(map[string]any)

func ScriptLanguage(value string) ScriptOption {
	return func(m map[string]any) { m["language"] = value }
}
func ScriptRetry(opts ...RetryOption) ScriptOption {
	return func(m map[string]any) { m["retry"] = retryOf(opts) }
}
func ScriptCompensateWith(value string) ScriptOption {
	return func(m map[string]any) { m["compensateWith"] = value }
}
func ScriptOnError(value string) ScriptOption { return func(m map[string]any) { m["onError"] = value } }
func ScriptOutputSchema(schema map[string]any) ScriptOption {
	return func(m map[string]any) { m["outputSchema"] = schema }
}

func ScriptSecrets(names ...string) ScriptOption {
	return func(m map[string]any) {
		list := make([]any, 0, len(names))
		for _, name := range names {
			list = append(list, name)
		}
		m["secrets"] = list
	}
}

// ---- wait for an event

// WaitOption sets an option of WaitForEvent.
type WaitOption func(map[string]any)

// OnTimeout names the node the flow goes to when the wait times out.
func OnTimeout(node string) WaitOption { return func(m map[string]any) { m["onTimeout"] = node } }

// Lookback lets an event that arrived up to d before the wait started wake it (once).
func Lookback(d time.Duration) WaitOption {
	return func(m map[string]any) { m["lookbackMs"] = millis(d) }
}

// WaitOutputSchema sets the JSON Schema of the event payload carried by a satisfied wait.
func WaitOutputSchema(schema map[string]any) WaitOption {
	return func(m map[string]any) { m["outputSchema"] = schema }
}

// ---- sequential loops

// LoopOption sets an optional member of While or RepeatUntil.
type LoopOption func(map[string]any)

// LoopDelay waits on a durable timer between iterations that are still needed.
func LoopDelay(d time.Duration) LoopOption {
	return func(m map[string]any) { m["delayMs"] = millis(d) }
}

// LoopOnError names the node the flow follows when a loop fails.
func LoopOnError(node string) LoopOption {
	return func(m map[string]any) { m["onError"] = node }
}

// ---- continuation

// ContinuationOption sets an optional member of ContinueAsNew.
type ContinuationOption func(map[string]any)

// ContinuationState explicitly selects the declared state transferred to the successor.
func ContinuationState(expression string) ContinuationOption {
	return func(m map[string]any) { m["state"] = expression }
}

// ---- child

// CancelPolicy is what happens to a child run when its parent is cancelled.
type CancelPolicy string

// The cancel policies of a child.
const (
	Propagate     CancelPolicy = "propagate"
	Detach        CancelPolicy = "detach"
	RequestCancel CancelPolicy = "request_cancel"
)

// ChildOption sets an option of InvokeChild.
type ChildOption func(map[string]any)

// ChildVersion pins a published version number of the child workflow.
func ChildVersion(version int) ChildOption {
	return func(m map[string]any) { m["version"] = int64(version) }
}

// ChildInput is the expression that yields the input of the child.
func ChildInput(expression string) ChildOption {
	return func(m map[string]any) { m["input"] = expression }
}

// ChildCancelPolicy sets what happens to the child when the parent is cancelled.
func ChildCancelPolicy(p CancelPolicy) ChildOption {
	return func(m map[string]any) { m["cancelPolicy"] = string(p) }
}

// ChildOutputSchema sets the JSON Schema of the child result carried by the completed node.
func ChildOutputSchema(schema map[string]any) ChildOption {
	return func(m map[string]any) { m["outputSchema"] = schema }
}

// LimitOption sets one optional workflow-wide execution ceiling.
type LimitOption func(map[string]any)

// MaxInstances lowers the maximum number of node instances admitted during one run.
func MaxInstances(value int) LimitOption {
	return func(m map[string]any) { m["maxInstances"] = int64(value) }
}

// MaxDepth lowers the maximum nesting depth of structured workflow nodes.
func MaxDepth(value int) LimitOption {
	return func(m map[string]any) { m["maxDepth"] = int64(value) }
}

// ---- saga

// SagaOption sets an option of Saga.
type SagaOption func(map[string]any)

// OnCompensationFailure is what a saga does when a compensation fails for good.
type OnCompensationFailure string

// The values of OnCompensationFailure.
const (
	ManualReview OnCompensationFailure = "manual_review"
	FailRun      OnCompensationFailure = "fail"
)

// SagaCancelPolicy is what a saga does when the run is cancelled.
type SagaCancelPolicy string

// The cancel policies of a saga.
const (
	CompensateCompleted        SagaCancelPolicy = "compensate_completed"
	AbortNoCompensation        SagaCancelPolicy = "abort_no_compensation"
	WaitInFlightThenCompensate SagaCancelPolicy = "wait_in_flight_then_compensate"
)

// OnFailure sets what the saga does when a compensation fails for good.
func OnFailure(v OnCompensationFailure) SagaOption {
	return func(m map[string]any) { m["onCompensationFailure"] = string(v) }
}

// OnCancel sets what the saga does when the run is cancelled.
func OnCancel(p SagaCancelPolicy) SagaOption {
	return func(m map[string]any) { m["cancelPolicy"] = string(p) }
}

// ---- end

// EndOption sets an option of End.
type EndOption func(map[string]any)

// OutputExpr is the expression that yields the output of the run.
func OutputExpr(expression string) EndOption {
	return func(m map[string]any) { m["outputExpr"] = expression }
}

// ---- trigger

// TriggerKind is what starts a run.
type TriggerKind string

// The kinds of trigger.
const (
	TriggerManual  TriggerKind = "manual"
	TriggerEvent   TriggerKind = "event"
	TriggerWebhook TriggerKind = "webhook"
	TriggerMessage TriggerKind = "message"
	TriggerCron    TriggerKind = "cron"
)

// Trigger says what starts a run of the workflow. Only the members that are set are written.
type Trigger struct {
	Kind      TriggerKind
	EventType string
	Source    string
	Endpoint  string
	Connector string
	Filter    string
	Cron      string
	Timezone  string
}

func (t Trigger) document() map[string]any {
	m := map[string]any{"kind": string(t.Kind)}
	for key, value := range map[string]string{
		"eventType": t.EventType, "source": t.Source, "endpoint": t.Endpoint, "connector": t.Connector,
		"filter": t.Filter, "cron": t.Cron, "timezone": t.Timezone,
	} {
		if value != "" {
			m[key] = value
		}
	}
	return m
}

// Join says how a parallel node waits for its branches.
type Join string

// The joins of a parallel node.
const (
	JoinAll Join = "all"
	JoinAny Join = "any"
)
