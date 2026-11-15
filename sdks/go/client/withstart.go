package client

import (
	"context"
	"encoding/json"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// BusinessIDConflictPolicy says what a start does when an open execution already holds the business
// identifier.
type BusinessIDConflictPolicy string

// The conflict policies. The server may add others; one this SDK does not know is sent as given.
const (
	// BusinessIDFail refuses the start with CONFLICT (reason BUSINESS_ID_IN_USE). It is the default.
	BusinessIDFail BusinessIDConflictPolicy = "FAIL"
	// BusinessIDUseExisting creates nothing and answers the run that holds the identifier
	// (StartedRun.Started is false).
	BusinessIDUseExisting BusinessIDConflictPolicy = "USE_EXISTING"
)

// BusinessIDReusePolicy says what a start does when a closed execution still remembers the business
// identifier.
type BusinessIDReusePolicy string

// The reuse policies. The server may add others; one this SDK does not know is sent as given.
const (
	// BusinessIDAllowDuplicate starts the run whatever the closed execution ended as. It is the default.
	BusinessIDAllowDuplicate BusinessIDReusePolicy = "ALLOW_DUPLICATE"
	// BusinessIDAllowDuplicateFailedOnly starts the run only if the closed execution did not succeed.
	BusinessIDAllowDuplicateFailedOnly BusinessIDReusePolicy = "ALLOW_DUPLICATE_FAILED_ONLY"
	// BusinessIDRejectDuplicate refuses the start while the closed execution is remembered.
	BusinessIDRejectDuplicate BusinessIDReusePolicy = "REJECT_DUPLICATE"
)

// WithBusinessID gives the run the caller's own identifier (1 to 200 visible ASCII characters), sent as
// it is: nothing is trimmed, case-folded or checked here, the server judges it. At most one open
// execution of a namespace holds an identifier (Start, SignalWithStart, UpdateWithStart).
func WithBusinessID(id string) CallOption {
	return func(o *callOptions) { o.businessID = &id }
}

// WithBusinessIDConflictPolicy says what to do when an open execution holds the identifier (Start,
// UpdateWithStart). Without it the server's default, BusinessIDFail, applies.
func WithBusinessIDConflictPolicy(p BusinessIDConflictPolicy) CallOption {
	return func(o *callOptions) { o.businessIDConflictPolicy = p }
}

// WithBusinessIDReusePolicy says what to do when a closed execution still remembers the identifier
// (Start, SignalWithStart, UpdateWithStart).
func WithBusinessIDReusePolicy(p BusinessIDReusePolicy) CallOption {
	return func(o *callOptions) { o.businessIDReusePolicy = p }
}

// WithStartDelay holds the entry of the run for d (Start, UpdateWithStart). The duration is turned into
// a whole number of milliseconds once, before the first attempt, by truncation toward zero, and sent
// as startDelayMs; a zero duration is sent as 0. The range (1 ms to 366 days) is the server's to
// judge: it answers VALIDATION_FAILED with the reason START_DELAY_INVALID outside it.
func WithStartDelay(d time.Duration) CallOption {
	return func(o *callOptions) { o.startDelay, o.hasStartDelay = d, true }
}

// startDelayMs is the integer number of milliseconds sent for a start delay.
func startDelayMs(d time.Duration) int64 { return int64(d / time.Millisecond) }

// SignalWithStartResult is the answer to SignalWithStart.
type SignalWithStartResult struct {
	RunID string
	// Started is true when this request created the run.
	Started bool
	// DeliveredTo are the nodes the signal woke.
	DeliveredTo []string
	Raw         map[string]json.RawMessage
}

// UpdateOutcome is what became of the Update of an UpdateWithStart.
type UpdateOutcome struct {
	// Outcome is APPLIED or REJECTED; a value this SDK does not know is kept as it came.
	Outcome string
	// Receipt is the durable receipt of an applied Update, or nil.
	Receipt *UpdateReceipt
	// Reason says why an Update was rejected (UPDATE_REJECTED by the validator, REVISION_CONFLICT for a
	// stale expected revision), or is empty.
	Reason string
}

// UpdateWithStartResult is the answer to UpdateWithStart. A rejection by the validator or a stale
// revision is an answer (Update.Outcome is REJECTED), not an error.
type UpdateWithStartResult struct {
	RunID string
	// Started is true when this request created the run.
	Started bool
	Update  UpdateOutcome
	Raw     map[string]json.RawMessage
}

// SignalWithStart signals the open run of a business identifier, or starts one and delivers the signal
// at its creation. An open run (or a new run) that no node of which waits for the signal refuses it
// with CONFLICT and nothing is created or changed. input and payload are encoded as JSON; nil is an
// empty object.
//
// Reads WithIdempotencyKey, WithTraceparent, WithVersionID, WithCorrelationID, WithDeadline,
// WithDeadlineIn and WithBusinessIDReusePolicy. The identifier is sent as given.
func (c *Client) SignalWithStart(ctx context.Context, workflow, businessID string, input any, signalName string, payload any, opts ...CallOption) (*SignalWithStartResult, error) {
	o := callOptionsOf(opts)
	idempotencyKey, traceparent, err := c.mutation(ctx, o)
	if err != nil {
		return nil, err
	}
	workflowID, err := c.ResolveWorkflow(ctx, workflow)
	if err != nil {
		return nil, err
	}
	if input == nil {
		input = map[string]any{}
	}
	if payload == nil {
		payload = map[string]any{}
	}
	body := map[string]any{
		"workflowId": workflowID, "businessId": businessID, "input": input,
		"signal": map[string]any{"name": signalName, "payload": payload},
	}
	withStartFields(body, o)
	resp, err := c.do(ctx, request{
		method: "POST", path: "/runs/signal-with-start", body: body, hasBody: true,
		idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true,
	})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a signal-with-start result")
	if err != nil {
		return nil, err
	}
	result := &SignalWithStartResult{Raw: raw}
	if result.RunID, err = requiredString(raw, "runId"); err != nil {
		return nil, err
	}
	result.Started, _ = jsonx.Bool(raw, "started")
	result.DeliveredTo = stringList(raw["deliveredTo"])
	return result, nil
}

// UpdateWithStart applies a declared Update to the open run of a business identifier, starting the run
// first when none is open. The caller's update.UpdateKey is required (1 to 128 visible ASCII
// characters) and kept by every retry; update.ExpectedStateRevision, if given, is not negative; both
// are checked before anything is sent. input is encoded as JSON; nil is an empty object.
//
// Reads WithIdempotencyKey, WithTraceparent, WithVersionID, WithCorrelationID, WithDeadline,
// WithDeadlineIn, WithBusinessIDConflictPolicy, WithBusinessIDReusePolicy and WithStartDelay.
func (c *Client) UpdateWithStart(ctx context.Context, workflow, businessID string, input any, updateName string, update UpdateRequest, opts ...CallOption) (*UpdateWithStartResult, error) {
	if !updateKeyPattern.MatchString(update.UpdateKey) {
		return nil, sagat.Invalid("an update key is 1 to 128 visible ASCII characters")
	}
	if update.ExpectedStateRevision != nil && *update.ExpectedStateRevision < 0 {
		return nil, sagat.Invalid("expected state revision is non-negative")
	}
	if !declaredOperationNamePattern.MatchString(updateName) {
		return nil, sagat.Invalid("an operation name is 1 to 128 permitted characters and starts with a letter or digit")
	}
	o := callOptionsOf(opts)
	idempotencyKey, traceparent, err := c.mutation(ctx, o)
	if err != nil {
		return nil, err
	}
	workflowID, err := c.ResolveWorkflow(ctx, workflow)
	if err != nil {
		return nil, err
	}
	if input == nil {
		input = map[string]any{}
	}
	change := map[string]any{"name": updateName, "updateKey": update.UpdateKey, "args": update.Args}
	if update.ExpectedStateRevision != nil {
		change["expectedStateRevision"] = *update.ExpectedStateRevision
	}
	body := map[string]any{"workflowId": workflowID, "businessId": businessID, "input": input, "update": change}
	withStartFields(body, o)
	if o.businessIDConflictPolicy != "" {
		body["businessIdConflictPolicy"] = string(o.businessIDConflictPolicy)
	}
	if o.hasStartDelay {
		body["startDelayMs"] = startDelayMs(o.startDelay)
	}
	resp, err := c.do(ctx, request{
		method: "POST", path: "/runs/update-with-start", body: body, hasBody: true,
		idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true,
	})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "an update-with-start result")
	if err != nil {
		return nil, err
	}
	result := &UpdateWithStartResult{Raw: raw}
	if result.RunID, err = requiredString(raw, "runId"); err != nil {
		return nil, err
	}
	result.Started, _ = jsonx.Bool(raw, "started")
	answer, err := object(raw["update"], "the update of the result")
	if err != nil {
		return nil, err
	}
	if result.Update.Outcome, err = requiredString(answer, "outcome"); err != nil {
		return nil, err
	}
	result.Update.Reason, _ = jsonx.String(answer, "reason")
	if receipt, present := answer["receipt"]; present && !jsonx.IsNull(receipt) {
		fields, err := object(receipt, "an update receipt")
		if err != nil {
			return nil, err
		}
		if result.Update.Receipt, err = updateWithStartReceipt(fields); err != nil {
			return nil, err
		}
	}
	return result, nil
}

// withStartFields adds what the start calls share to a body: the version, the correlation id, the
// deadline and the reuse policy.
func withStartFields(body map[string]any, o callOptions) {
	if o.versionID != "" {
		body["versionId"] = o.versionID
	}
	if o.correlationID != "" {
		body["correlationId"] = o.correlationID
	}
	deadline := o.deadline
	if o.hasDeadlineIn {
		deadline = time.Now().Add(o.deadlineIn)
	}
	if !deadline.IsZero() {
		body["deadline"] = jsonx.Instant(deadline)
	}
	if o.businessIDReusePolicy != "" {
		body["businessIdReusePolicy"] = string(o.businessIDReusePolicy)
	}
}

// updateWithStartReceipt reads the receipt of an applied Update. It is as strict as the contract and no
// stricter: a status this SDK does not know is kept as it came.
func updateWithStartReceipt(raw map[string]json.RawMessage) (*UpdateReceipt, error) {
	var err error
	receipt := &UpdateReceipt{Raw: raw}
	if receipt.RunID, err = requiredString(raw, "runId"); err != nil {
		return nil, err
	}
	if receipt.Operation, err = requiredString(raw, "operation"); err != nil {
		return nil, err
	}
	if receipt.UpdateKey, err = requiredString(raw, "updateKey"); err != nil {
		return nil, err
	}
	if receipt.Status, err = requiredString(raw, "status"); err != nil {
		return nil, err
	}
	if value, ok := raw["result"]; ok {
		if err := json.Unmarshal(value, &receipt.Result); err != nil {
			return nil, protocolError("result is invalid in the answer")
		}
	}
	if receipt.BeforeStateRevision, err = requiredRevision(raw, "beforeStateRevision", 0); err != nil {
		return nil, err
	}
	if receipt.AfterStateRevision, err = requiredRevision(raw, "afterStateRevision", 0); err != nil {
		return nil, err
	}
	if receipt.RunVersion, err = requiredRevision(raw, "runVersion", 0); err != nil {
		return nil, err
	}
	receipt.Replayed, _ = jsonx.Bool(raw, "replayed")
	return receipt, nil
}
