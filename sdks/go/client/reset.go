package client

import (
	"context"
	"encoding/json"
	"net/url"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// ResetReapply says which of the commands that arrived after the point of a reset are applied again to the
// new run. The set only grows; a value this SDK does not know is sent as given and judged by the server.
type ResetReapply string

// The reapply policies.
const (
	ReapplyNone    ResetReapply = "NONE"
	ReapplySignals ResetReapply = "SIGNALS"
	ReapplyUpdates ResetReapply = "UPDATES"
	ReapplyAll     ResetReapply = "ALL"
)

// ResetPoint is a sequence of the journal of a run that ended that a reset can go back to. Kind is
// RUN_STARTED, STEP_COMPLETED, ...: the set only grows.
type ResetPoint struct {
	Sequence   int64
	Kind       string
	Node       string
	OccurredAt time.Time
	Raw        map[string]json.RawMessage
}

// ResetRestored is a step whose outcome the new run keeps.
type ResetRestored struct {
	Node string
	Kind string
}

// ResetReexecuted is a step the new run runs again.
type ResetReexecuted struct {
	Node              string
	CompletedInSource bool
	Markers           []string
}

// ResetPlan is what a reset to a sequence would restore and run again.
type ResetPlan struct {
	Sequence   int64
	Restored   []ResetRestored
	Reexecuted []ResetReexecuted
	// Reapply is the report of what would be applied again, each member as the JSON it was.
	Reapply                 []json.RawMessage
	RequiresAcknowledgement bool
	PrefixEvents            int64
	Raw                     map[string]json.RawMessage
}

// ResetRequest is a reset to make. An optional field left at its zero value is left out of the request.
type ResetRequest struct {
	// Sequence is a point of the run, at least 1.
	Sequence int64 `json:"sequence"`
	// Reason is 8 to 2000 characters.
	Reason  string       `json:"reason"`
	Reapply ResetReapply `json:"reapply,omitempty"`
	// SourcePolicy is REQUIRE_TERMINAL or CANCEL_SOURCE.
	SourcePolicy string `json:"sourcePolicy,omitempty"`
	// BusinessID is KEEP or DROP: whether the new run takes the business identifier of the source.
	BusinessID string `json:"businessId,omitempty"`
	// AcknowledgeReexecution is required when the plan runs completed steps again.
	AcknowledgeReexecution bool `json:"acknowledgeReexecution,omitempty"`
}

// RunReset is a reset as the register of the source run keeps it. State is the string the server sent.
type RunReset struct {
	ResetID     string
	SourceRunID string
	Sequence    int64
	State       string
	// FailureReason is empty unless the reset failed.
	FailureReason string
	// NewRunID is the run the reset created, or empty.
	NewRunID      string
	ReapplyReport []json.RawMessage
	CreatedAt     time.Time
	Raw           map[string]json.RawMessage
}

// ResetPointsOptions pages ResetPoints.
type ResetPointsOptions struct {
	// Limit is the page size; the server's default when zero.
	Limit int
	// Cursor continues a previous page.
	Cursor string
}

// ResetPoints lists where a run that ended can be reset to, by ascending sequence. Reads WithTraceparent.
func (c *Client) ResetPoints(ctx context.Context, runID string, options ResetPointsOptions, opts ...CallOption) (*Page[ResetPoint], error) {
	path, err := runPath(runID)
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	query := url.Values{}
	if options.Cursor != "" {
		query.Set("cursor", options.Cursor)
	}
	positive(query, "limit", options.Limit)
	resp, err := c.do(ctx, request{method: "GET", path: path + "/reset-points", query: query, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	return parsePage(resp.body, "a page of reset points", func(raw map[string]json.RawMessage) (ResetPoint, error) {
		var err error
		point := ResetPoint{Raw: raw}
		if point.Sequence, err = requiredRevision(raw, "sequence", 1); err != nil {
			return ResetPoint{}, err
		}
		if point.Kind, err = requiredString(raw, "kind"); err != nil {
			return ResetPoint{}, err
		}
		point.Node, _ = jsonx.String(raw, "node")
		if point.OccurredAt, err = requiredTime(raw, "occurredAt"); err != nil {
			return ResetPoint{}, err
		}
		return point, nil
	})
}

// PreviewReset says what a reset to sequence would restore and run again; it changes nothing. sequence is
// at least 1; reapply is left out of the request when empty. Reads WithTraceparent.
func (c *Client) PreviewReset(ctx context.Context, runID string, sequence int64, reapply ResetReapply, opts ...CallOption) (*ResetPlan, error) {
	if sequence < 1 {
		return nil, sagat.Invalid("sequence is a positive integer")
	}
	path, err := runPath(runID)
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	body := map[string]any{"sequence": sequence}
	if reapply != "" {
		body["reapply"] = string(reapply)
	}
	resp, err := c.do(ctx, request{method: "POST", path: path + "/reset-previews", body: body, hasBody: true, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a reset plan")
	if err != nil {
		return nil, err
	}
	plan := &ResetPlan{Raw: raw, Restored: []ResetRestored{}, Reexecuted: []ResetReexecuted{}}
	if plan.Sequence, err = requiredRevision(raw, "sequence", 1); err != nil {
		return nil, err
	}
	for _, member := range rawList(raw["restored"]) {
		fields, err := object(member, "a restored step")
		if err != nil {
			return nil, err
		}
		var step ResetRestored
		if step.Node, err = requiredString(fields, "node"); err != nil {
			return nil, err
		}
		if step.Kind, err = requiredString(fields, "kind"); err != nil {
			return nil, err
		}
		plan.Restored = append(plan.Restored, step)
	}
	for _, member := range rawList(raw["reexecuted"]) {
		fields, err := object(member, "a re-executed step")
		if err != nil {
			return nil, err
		}
		var step ResetReexecuted
		if step.Node, err = requiredString(fields, "node"); err != nil {
			return nil, err
		}
		step.CompletedInSource, _ = jsonx.Bool(fields, "completedInSource")
		step.Markers = stringList(fields["markers"])
		plan.Reexecuted = append(plan.Reexecuted, step)
	}
	plan.Reapply = rawList(raw["reapply"])
	plan.RequiresAcknowledgement, _ = jsonx.Bool(raw, "requiresAcknowledgement")
	if plan.PrefixEvents, err = requiredRevision(raw, "prefixEvents", 1); err != nil {
		return nil, err
	}
	return plan, nil
}

// Reset creates a new run from an eligible prefix of a run that ended. The same key and request answer the
// first reset. request.Sequence is at least 1. Reads WithIdempotencyKey and WithTraceparent.
func (c *Client) Reset(ctx context.Context, runID string, reset ResetRequest, opts ...CallOption) (*RunReset, error) {
	if reset.Sequence < 1 {
		return nil, sagat.Invalid("sequence is a positive integer")
	}
	path, err := runPath(runID)
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	idempotencyKey, traceparent, err := c.mutation(ctx, o)
	if err != nil {
		return nil, err
	}
	resp, err := c.do(ctx, requestOf("POST", path+"/resets", reset, idempotencyKey, traceparent))
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a reset")
	if err != nil {
		return nil, err
	}
	made, err := resetOf(raw)
	if err != nil {
		return nil, err
	}
	return &made, nil
}

// ListResets lists the resets made from a run, newest first (at most 200). Reads WithTraceparent.
func (c *Client) ListResets(ctx context.Context, runID string, opts ...CallOption) ([]RunReset, error) {
	path, err := runPath(runID)
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	resp, err := c.do(ctx, request{method: "GET", path: path + "/resets", repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	return parseItems(resp.body, "a list of resets", resetOf)
}

// GetReset reads one reset of a run by its id. Reads WithTraceparent.
func (c *Client) GetReset(ctx context.Context, runID, resetID string, opts ...CallOption) (*RunReset, error) {
	path, err := runPath(runID)
	if err != nil {
		return nil, err
	}
	id, err := idSegment(resetID, "a reset id")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	resp, err := c.do(ctx, request{method: "GET", path: path + "/resets/" + id, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a reset")
	if err != nil {
		return nil, err
	}
	got, err := resetOf(raw)
	if err != nil {
		return nil, err
	}
	return &got, nil
}

func resetOf(raw map[string]json.RawMessage) (RunReset, error) {
	var err error
	reset := RunReset{Raw: raw}
	if reset.ResetID, err = requiredString(raw, "resetId"); err != nil {
		return RunReset{}, err
	}
	if reset.SourceRunID, err = requiredString(raw, "sourceRunId"); err != nil {
		return RunReset{}, err
	}
	if reset.Sequence, err = requiredRevision(raw, "sequence", 1); err != nil {
		return RunReset{}, err
	}
	if reset.State, err = requiredString(raw, "state"); err != nil {
		return RunReset{}, err
	}
	reset.FailureReason, _ = jsonx.String(raw, "failureReason")
	reset.NewRunID, _ = jsonx.String(raw, "newRunId")
	reset.ReapplyReport = rawList(raw["reapplyReport"])
	reset.CreatedAt, _ = jsonx.Time(raw, "createdAt")
	return reset, nil
}
