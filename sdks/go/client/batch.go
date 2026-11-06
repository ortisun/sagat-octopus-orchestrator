package client

import (
	"context"
	"encoding/json"
	"iter"
	"net/url"
	"time"

	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// BatchFilter selects the runs of a batch job by what they are. The zero value of a field selects on
// nothing.
type BatchFilter struct {
	WorkflowID string   `json:"workflowId,omitempty"`
	VersionID  string   `json:"versionId,omitempty"`
	Status     []string `json:"status,omitempty"`
	Origin     string   `json:"origin,omitempty"`
	ScheduleID string   `json:"scheduleId,omitempty"`
	// CreatedAfter and CreatedBefore bound the creation of the runs; the zero time.Time is no bound. They
	// are sent in UTC, as instants with a Z, whatever the zone of the value.
	CreatedAfter     time.Time `json:"-"`
	CreatedBefore    time.Time `json:"-"`
	BusinessIDPrefix string    `json:"businessIdPrefix,omitempty"`
}

// MarshalJSON writes CreatedAfter and CreatedBefore as UTC instants with a Z.
func (f BatchFilter) MarshalJSON() ([]byte, error) {
	type fields BatchFilter
	return jsonx.Encode(struct {
		fields
		CreatedAfter  string `json:"createdAfter,omitempty"`
		CreatedBefore string `json:"createdBefore,omitempty"`
	}{fields(f), instantText(f.CreatedAfter), instantText(f.CreatedBefore)})
}

// UnmarshalJSON reads CreatedAfter and CreatedBefore from RFC 3339 text with any offset; text that is not an
// instant is an error.
func (f *BatchFilter) UnmarshalJSON(data []byte) error {
	type fields BatchFilter
	wire := struct {
		*fields
		CreatedAfter  *string `json:"createdAfter"`
		CreatedBefore *string `json:"createdBefore"`
	}{fields: (*fields)(f)}
	if err := json.Unmarshal(data, &wire); err != nil {
		return err
	}
	var err error
	if f.CreatedAfter, err = instantOf(wire.CreatedAfter, "createdAfter"); err != nil {
		return err
	}
	f.CreatedBefore, err = instantOf(wire.CreatedBefore, "createdBefore")
	return err
}

// BatchSelector says which runs a batch job is about: by a filter, by the ids of the runs (an empty but
// non-nil RunIDs is sent as an empty list, which the server refuses), or by a query. Normally one of them is
// set; the server judges any other combination.
type BatchSelector struct {
	Filter *BatchFilter `json:"filter,omitempty"`
	RunIDs []string     `json:"runIds,omitempty"`
	Query  string       `json:"query,omitempty"`
}

// MarshalJSON writes the members that are set, an empty RunIDs among them when it is not nil.
func (s BatchSelector) MarshalJSON() ([]byte, error) {
	members := map[string]any{}
	if s.Filter != nil {
		members["filter"] = s.Filter
	}
	if s.RunIDs != nil {
		members["runIds"] = s.RunIDs
	}
	if s.Query != "" {
		members["query"] = s.Query
	}
	return jsonx.Encode(members)
}

// BatchSignal is the signal a SIGNAL job sends to every run.
type BatchSignal struct {
	Name string `json:"name"`
	// Payload is encoded as JSON; nil is an empty object.
	Payload any `json:"payload"`
}

// MarshalJSON sends an empty object for a nil Payload.
func (s BatchSignal) MarshalJSON() ([]byte, error) {
	type fields BatchSignal
	if s.Payload == nil {
		s.Payload = map[string]any{}
	}
	return jsonx.Encode(fields(s))
}

// BatchUpdate is the declared Update an UPDATE job applies to every run.
type BatchUpdate struct {
	Name string `json:"name"`
	Args any    `json:"args,omitempty"`
}

// BatchRequest is a batch job to create. An optional field left at its zero value is left out of the
// request.
type BatchRequest struct {
	Selector BatchSelector `json:"selector"`
	// Action is CANCEL, SIGNAL, UPDATE, RERUN, RETRY_STEP, PAUSE, RESUME, MARK_MANUAL_REVIEW, REPIN, ...:
	// the set only grows, and one this build does not offer is refused by the server.
	Action string       `json:"action"`
	Signal *BatchSignal `json:"signal,omitempty"`
	Update *BatchUpdate `json:"update,omitempty"`
	StepID string       `json:"stepId,omitempty"`
	// BusinessID is KEEP or DROP (RERUN only).
	BusinessID string `json:"businessId,omitempty"`
	// Reason is 8 to 2000 characters, kept with the job.
	Reason         string  `json:"reason"`
	MaxItems       int64   `json:"maxItems,omitempty"`
	ItemsPerSecond float64 `json:"itemsPerSecond,omitempty"`
}

// BatchPreview is what a selector matches, up to maxItems; nothing was changed.
type BatchPreview struct {
	Matched int64
	// Capped is true when more than maxItems matched.
	Capped bool
	// Sample is up to 20 of the matched run ids, in the order the server chose.
	Sample []string
	Raw    map[string]json.RawMessage
}

// BatchCounts are the counters of a batch job.
type BatchCounts struct {
	Selected  int64
	Succeeded int64
	Skipped   int64
	Failed    int64
	Cancelled int64
	Deferred  int64
}

// BatchJob is a batch job as the server answers it. State and FailureReason are the strings the server
// sent; an instant the server did not send is the zero time.Time.
type BatchJob struct {
	BatchID       string
	Action        string
	State         string
	FailureReason string
	CreatedBy     string
	HasPayload    bool
	MaxItems      *int64
	// ItemsPerSecond is a number of the contract that need not be whole.
	ItemsPerSecond *float64
	Counts         BatchCounts
	CreatedAt      time.Time
	StartedAt      time.Time
	FinishedAt     time.Time
	ExpiresAt      time.Time
	Raw            map[string]json.RawMessage
}

// BatchItem is what a batch job did to one run. State and Reason are the strings the server sent; one this
// SDK does not know is kept as it came.
type BatchItem struct {
	Ordinal   int64
	RunID     string
	State     string
	Reason    string
	ErrorCode string
	Attempts  int64
	NewRunID  string
	Raw       map[string]json.RawMessage
}

// BatchListOptions filters and pages Batch.List and Batch.ListItems.
type BatchListOptions struct {
	// State keeps the jobs (or the items) in that state.
	State string
	// Limit is the page size; the server's default when zero.
	Limit int
	// Cursor continues a previous page.
	Cursor string
}

// BatchOperations is the group of batch job calls, reached from Client.Batch. A job is a resource of a
// namespace given by its id (a UUID).
type BatchOperations struct{ c *Client }

func (b *BatchOperations) path(namespaceID, batchID, tail string) (string, error) {
	base, err := namespacePath(namespaceID)
	if err != nil {
		return "", err
	}
	base += "/batch-operations"
	if batchID == "" {
		return base + tail, nil
	}
	id, err := idSegment(batchID, "a batch id")
	if err != nil {
		return "", err
	}
	return base + "/" + id + tail, nil
}

// Preview counts what a selector matches, up to maxItems (the server's default when 0); it changes
// nothing. Reads WithTraceparent.
func (b *BatchOperations) Preview(ctx context.Context, namespaceID string, selector BatchSelector, maxItems int64, opts ...CallOption) (*BatchPreview, error) {
	path, err := b.path(namespaceID, "", "/preview")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := b.c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	body := map[string]any{"selector": selector}
	if maxItems != 0 {
		body["maxItems"] = maxItems
	}
	resp, err := b.c.do(ctx, request{method: "POST", path: path, body: body, hasBody: true, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a batch preview")
	if err != nil {
		return nil, err
	}
	preview := &BatchPreview{Raw: raw}
	if preview.Matched, err = requiredRevision(raw, "matched", 0); err != nil {
		return nil, err
	}
	preview.Capped, _ = jsonx.Bool(raw, "capped")
	preview.Sample = stringList(raw["sample"])
	return preview, nil
}

// Create creates a durable job. A request sent again with the same key and body is answered with the first
// job. Reads WithIdempotencyKey and WithTraceparent.
func (b *BatchOperations) Create(ctx context.Context, namespaceID string, batch BatchRequest, opts ...CallOption) (*BatchJob, error) {
	path, err := b.path(namespaceID, "", "")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	idempotencyKey, traceparent, err := b.c.mutation(ctx, o)
	if err != nil {
		return nil, err
	}
	return b.job(ctx, requestOf("POST", path, batch, idempotencyKey, traceparent))
}

// Get reads a job. Reads WithTraceparent.
func (b *BatchOperations) Get(ctx context.Context, namespaceID, batchID string, opts ...CallOption) (*BatchJob, error) {
	path, err := b.path(namespaceID, batchID, "")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := b.c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	return b.job(ctx, request{method: "GET", path: path, repeatable: true, traceparent: traceparent})
}

// Cancel stops admitting items; what was applied stays applied. A job that already ended is CONFLICT.
// Reads WithIdempotencyKey and WithTraceparent.
func (b *BatchOperations) Cancel(ctx context.Context, namespaceID, batchID, reason string, opts ...CallOption) (*BatchJob, error) {
	path, err := b.path(namespaceID, batchID, "/cancel")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	idempotencyKey, traceparent, err := b.c.mutation(ctx, o)
	if err != nil {
		return nil, err
	}
	return b.job(ctx, requestOf("POST", path, map[string]any{"reason": reason}, idempotencyKey, traceparent))
}

func requestOf(method, path string, body any, idempotencyKey, traceparent string) request {
	return request{
		method: method, path: path, body: body, hasBody: true,
		idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true,
	}
}

func (b *BatchOperations) job(ctx context.Context, req request) (*BatchJob, error) {
	resp, err := b.c.do(ctx, req)
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a batch job")
	if err != nil {
		return nil, err
	}
	return batchJobOf(raw)
}

// List reads one page of jobs. Reads WithTraceparent.
func (b *BatchOperations) List(ctx context.Context, namespaceID string, options BatchListOptions, opts ...CallOption) (*Page[BatchJob], error) {
	path, err := b.path(namespaceID, "", "")
	if err != nil {
		return nil, err
	}
	return listOf(ctx, b.c, path, options, opts, "a page of batch jobs", func(m map[string]json.RawMessage) (BatchJob, error) {
		job, err := batchJobOf(m)
		if err != nil {
			return BatchJob{}, err
		}
		return *job, nil
	})
}

// ListAll yields every job the options select, following the opaque cursors.
func (b *BatchOperations) ListAll(ctx context.Context, namespaceID string, options BatchListOptions, opts ...CallOption) iter.Seq2[*BatchJob, error] {
	return allPages(options.Cursor, func(cursor string) (*Page[BatchJob], error) {
		options.Cursor = cursor
		return b.List(ctx, namespaceID, options, opts...)
	})
}

// ListItems reads one page of what a job did to each run. Reads WithTraceparent.
func (b *BatchOperations) ListItems(ctx context.Context, namespaceID, batchID string, options BatchListOptions, opts ...CallOption) (*Page[BatchItem], error) {
	path, err := b.path(namespaceID, batchID, "/items")
	if err != nil {
		return nil, err
	}
	return listOf(ctx, b.c, path, options, opts, "a page of batch items", batchItemOf)
}

// ListAllItems yields every item the options select, following the opaque cursors.
func (b *BatchOperations) ListAllItems(ctx context.Context, namespaceID, batchID string, options BatchListOptions, opts ...CallOption) iter.Seq2[*BatchItem, error] {
	return allPages(options.Cursor, func(cursor string) (*Page[BatchItem], error) {
		options.Cursor = cursor
		return b.ListItems(ctx, namespaceID, batchID, options, opts...)
	})
}

func listOf[T any](ctx context.Context, c *Client, path string, options BatchListOptions, opts []CallOption, what string, read func(map[string]json.RawMessage) (T, error)) (*Page[T], error) {
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
	if options.State != "" {
		query.Set("state", options.State)
	}
	resp, err := c.do(ctx, request{method: "GET", path: path, query: query, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	return parsePage(resp.body, what, read)
}

func batchJobOf(raw map[string]json.RawMessage) (*BatchJob, error) {
	var err error
	job := &BatchJob{Raw: raw}
	if job.BatchID, err = requiredString(raw, "batchId"); err != nil {
		return nil, err
	}
	if job.Action, err = requiredString(raw, "action"); err != nil {
		return nil, err
	}
	if job.State, err = requiredString(raw, "state"); err != nil {
		return nil, err
	}
	job.FailureReason, _ = jsonx.String(raw, "failureReason")
	if job.CreatedBy, err = requiredString(raw, "createdBy"); err != nil {
		return nil, err
	}
	job.HasPayload, _ = jsonx.Bool(raw, "hasPayload")
	job.MaxItems = optionalInt(raw, "maxItems")
	job.ItemsPerSecond = optionalFloat(raw, "itemsPerSecond")
	if counts, err := jsonx.Object(raw["counts"]); err == nil {
		job.Counts = BatchCounts{
			Selected: intOr(counts, "selected", 0), Succeeded: intOr(counts, "succeeded", 0), Skipped: intOr(counts, "skipped", 0),
			Failed: intOr(counts, "failed", 0), Cancelled: intOr(counts, "cancelled", 0), Deferred: intOr(counts, "deferred", 0),
		}
	}
	if job.CreatedAt, err = requiredTime(raw, "createdAt"); err != nil {
		return nil, err
	}
	job.StartedAt, _ = jsonx.Time(raw, "startedAt")
	job.FinishedAt, _ = jsonx.Time(raw, "finishedAt")
	job.ExpiresAt, _ = jsonx.Time(raw, "expiresAt")
	return job, nil
}

func batchItemOf(raw map[string]json.RawMessage) (BatchItem, error) {
	var err error
	item := BatchItem{Raw: raw}
	if item.Ordinal, err = requiredRevision(raw, "ordinal", 0); err != nil {
		return BatchItem{}, err
	}
	if item.RunID, err = requiredString(raw, "runId"); err != nil {
		return BatchItem{}, err
	}
	if item.State, err = requiredString(raw, "state"); err != nil {
		return BatchItem{}, err
	}
	item.Reason, _ = jsonx.String(raw, "reason")
	item.ErrorCode, _ = jsonx.String(raw, "errorCode")
	item.Attempts = intOr(raw, "attempts", 0)
	item.NewRunID, _ = jsonx.String(raw, "newRunId")
	return item, nil
}
