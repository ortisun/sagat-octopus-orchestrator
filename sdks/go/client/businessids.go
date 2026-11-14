package client

import (
	"context"
	"encoding/json"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// BusinessIDBinding is how a business identifier stands in a namespace.
type BusinessIDBinding struct {
	BusinessID string
	// ExecutionID is the run that holds the identifier, or the initial run of the continuation chain that does.
	ExecutionID string
	// CurrentRunID is the run that carries the work now.
	CurrentRunID string
	Open         bool
	// LastStatus is the status the holder ended as, or empty while it is open.
	LastStatus string
	WorkflowID string
	BoundAt    time.Time
	// ReusableAfter is when a closed holder is forgotten, or the zero time.Time while it is open.
	ReusableAfter time.Time
	Raw           map[string]json.RawMessage
}

// BusinessIDs is the group of business identifier calls, reached from Client.BusinessIDs.
type BusinessIDs struct{ c *Client }

// Get reads how an identifier stands in a namespace given by its id. An identifier nobody holds is
// NOT_FOUND. The identifier is sent as given, percent-encoded in the path: nothing is trimmed or
// case-folded, and only a value that cannot be a path segment is refused here, with nothing sent: one that is
// not 1 to 200 visible ASCII characters (the characters from "!" to "~"), and "." and "..", which every URL
// resolver takes as a step in the path. "..." and a value that merely contains dots are sent as given.
// Reads WithTraceparent.
func (b *BusinessIDs) Get(ctx context.Context, namespaceID, businessID string, opts ...CallOption) (*BusinessIDBinding, error) {
	base, err := namespacePath(namespaceID)
	if err != nil {
		return nil, err
	}
	if !visibleASCII(businessID, 200) {
		return nil, sagat.Invalid("a business identifier is 1 to 200 visible ASCII characters")
	}
	if businessID == "." || businessID == ".." {
		return nil, sagat.Invalid("a business identifier cannot be \".\" or \"..\": it would be read as a step in the path")
	}
	o := callOptionsOf(opts)
	traceparent, err := b.c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	resp, err := b.c.do(ctx, request{method: "GET", path: base + "/business-ids/" + escapeSegment(businessID), repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a business id binding")
	if err != nil {
		return nil, err
	}
	binding := &BusinessIDBinding{Raw: raw}
	if binding.BusinessID, err = requiredString(raw, "businessId"); err != nil {
		return nil, err
	}
	if binding.ExecutionID, err = requiredString(raw, "executionId"); err != nil {
		return nil, err
	}
	if binding.CurrentRunID, err = requiredString(raw, "currentRunId"); err != nil {
		return nil, err
	}
	binding.Open, _ = jsonx.Bool(raw, "open")
	binding.LastStatus, _ = jsonx.String(raw, "lastStatus")
	if binding.WorkflowID, err = requiredString(raw, "workflowId"); err != nil {
		return nil, err
	}
	if binding.BoundAt, err = requiredTime(raw, "boundAt"); err != nil {
		return nil, err
	}
	binding.ReusableAfter, _ = jsonx.Time(raw, "reusableAfter")
	return binding, nil
}

// visibleASCII reports whether s is 1 to max characters from "!" to "~".
func visibleASCII(s string, max int) bool {
	if len(s) < 1 || len(s) > max {
		return false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < 0x21 || s[i] > 0x7e {
			return false
		}
	}
	return true
}
