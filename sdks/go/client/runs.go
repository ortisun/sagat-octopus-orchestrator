package client

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"iter"
	"math/rand/v2"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"time"
	"unicode/utf8"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/backoff"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/ids"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// BlobDescriptor is the verified identity of a staged run input.
type BlobDescriptor struct {
	ID          string `json:"id"`
	Size        int64  `json:"size"`
	SHA256      string `json:"sha256"`
	ContentType string `json:"contentType"`
}

// UploadRunInputOptions adds optional server-side integrity preconditions.
type UploadRunInputOptions struct {
	ContentType    string
	ExpectedSize   *int64
	ExpectedSHA256 string
}

// CallOption adjusts one call. An option that a call has no use for is ignored; the doc of each
// option says which calls read it.
type CallOption func(*callOptions)

type callOptions struct {
	idempotencyKey     string
	traceparent        string
	versionID          string
	correlationID      string
	deadline           time.Time
	deadlineIn         time.Duration
	hasDeadlineIn      bool
	stepID             string
	outcome            string
	errorCode          string
	expectedRunVersion *int64
}

func callOptionsOf(opts []CallOption) callOptions {
	var o callOptions
	for _, opt := range opts {
		opt(&o)
	}
	return o
}

// WithIdempotencyKey sets the Idempotency-Key of a Start, Operate, Cancel or Signal: 8 to 160
// visible ASCII characters (0x21 to 0x7e: no space, no control character). Without it the SDK generates a random one per call and reuses it for
// every retry of that call, so two separate calls with no key are two runs.
func WithIdempotencyKey(key string) CallOption {
	return func(o *callOptions) { o.idempotencyKey = key }
}

// WithTraceparent sets the W3C traceparent of the call. It wins over the ambient trace context. A
// malformed one is a BAD_REQUEST raised by the SDK and nothing is sent.
func WithTraceparent(traceparent string) CallOption {
	return func(o *callOptions) { o.traceparent = traceparent }
}

// WithVersionID starts a specific version of the workflow (Start).
func WithVersionID(id string) CallOption { return func(o *callOptions) { o.versionID = id } }

// WithCorrelationID sets the correlation id of the run (Start).
func WithCorrelationID(id string) CallOption { return func(o *callOptions) { o.correlationID = id } }

// WithDeadline sets the instant at which the run times out (Start); it must be in the future and
// within a year. It is sent as an RFC 3339 instant in UTC.
func WithDeadline(t time.Time) CallOption { return func(o *callOptions) { o.deadline = t } }

// WithDeadlineIn is WithDeadline as a duration from now, computed once, before the first attempt
// (Start).
func WithDeadlineIn(d time.Duration) CallOption {
	return func(o *callOptions) { o.deadlineIn, o.hasDeadlineIn = d, true }
}

// WithExpectedRunVersion makes the operation conditional on the version of the run (Cancel,
// Operate).
func WithExpectedRunVersion(version int64) CallOption {
	return func(o *callOptions) { o.expectedRunVersion = &version }
}

// WithStepID names the step an operation is about (Operate).
func WithStepID(id string) CallOption { return func(o *callOptions) { o.stepID = id } }

// WithOutcome is the outcome a step is resolved with, SUCCEEDED or FAILED (Operate).
func WithOutcome(outcome string) CallOption { return func(o *callOptions) { o.outcome = outcome } }

// WithErrorCode is the error code a step is resolved as failed with (Operate).
func WithErrorCode(code string) CallOption { return func(o *callOptions) { o.errorCode = code } }

// trace returns the traceparent of a call: the explicit one, else the ambient one, validated. A trace
// hook that panics sends no trace.
func (c *Client) trace(ctx context.Context, explicit string) (string, error) {
	value := explicit
	if value == "" && c.traceContext != nil {
		value = c.ambient(ctx)
	}
	if value == "" {
		return "", nil
	}
	if err := sagat.CheckTraceparent(value); err != nil {
		return "", err
	}
	return value, nil
}

func (c *Client) ambient(ctx context.Context) (value string) {
	defer func() {
		if recover() != nil {
			c.logger.Warn("the trace context hook panicked: no trace is sent")
			value = ""
		}
	}()
	return c.traceContext(ctx)
}

// runPath is the path of a run: the id is a UUID, so that it can only name one segment of the path (a
// ".." or a "/" in it would send the request somewhere else).
func runPath(runID string) (string, error) {
	if !uuidPattern.MatchString(runID) {
		return "", sagat.Invalid("a run id is a UUID")
	}
	return "/runs/" + runID, nil
}

func chainPath(namespaceID, chainID string) (string, error) {
	if !uuidPattern.MatchString(namespaceID) {
		return "", sagat.Invalid("a namespace id is a UUID")
	}
	if !uuidPattern.MatchString(chainID) {
		return "", sagat.Invalid("a chain id is a UUID")
	}
	return "/namespaces/" + lower(namespaceID) + "/chains/" + lower(chainID), nil
}

// segment is a name in a URL path (a signal name): percent-encoded, and never ".", ".." or empty, which
// a server or a proxy would resolve as a path instead of a name.
func segment(name string) (string, error) {
	if name == "" || name == "." || name == ".." {
		return "", sagat.Invalid("a name in a URL path is not empty, \".\" or \"..\"")
	}
	return url.PathEscape(name), nil
}

func key(given string) (string, error) {
	if given == "" {
		return ids.UUID(), nil
	}
	if !keyPattern.MatchString(given) {
		return "", sagat.Invalid("an idempotency key is 8 to 160 visible ASCII characters (no space)")
	}
	return given, nil
}

// ResolveWorkflow returns the id of a workflow given by id or by name (the workflow of that name in
// the namespace). A name is remembered for a minute.
func (c *Client) ResolveWorkflow(ctx context.Context, workflow string) (string, error) {
	if uuidPattern.MatchString(workflow) {
		return lower(workflow), nil
	}
	c.mu.Lock()
	cached, ok := c.names[workflow]
	c.mu.Unlock()
	if ok && time.Since(cached.at) < time.Minute {
		return cached.id, nil
	}
	traceparent, err := c.trace(ctx, "")
	if err != nil {
		return "", err
	}
	cursor := ""
	for {
		query := url.Values{"limit": {"200"}}
		if cursor != "" {
			query.Set("cursor", cursor)
		}
		resp, err := c.do(ctx, request{method: "GET", path: "/workflows", query: query, repeatable: true, traceparent: traceparent})
		if err != nil {
			return "", err
		}
		page, err := object(resp.body, "a page")
		if err != nil {
			return "", err
		}
		var items []json.RawMessage
		_ = json.Unmarshal(page["items"], &items)
		for _, item := range items {
			draft, err := object(item, "a workflow")
			if err != nil {
				return "", err
			}
			if name, _ := jsonx.String(draft, "name"); name == workflow {
				id, err := requiredString(draft, "id")
				if err != nil {
					return "", err
				}
				c.mu.Lock()
				c.names[workflow] = nameEntry{id: id, at: time.Now()}
				c.mu.Unlock()
				return id, nil
			}
		}
		cursor, _ = jsonx.String(page, "nextCursor")
		if cursor == "" {
			break
		}
	}
	e := sagat.Errorf(sagat.CodeNotFound, "no workflow is named %q in this namespace", workflow)
	e.HTTPStatus, e.Reason = 404, "WORKFLOW_NOT_FOUND"
	return "", e
}

func lower(s string) string {
	b := []byte(s)
	for i, c := range b {
		if c >= 'A' && c <= 'F' {
			b[i] = c + 32
		}
	}
	return string(b)
}

// Start starts a run of a published workflow, given by id or by name. input is encoded as JSON (a
// json.RawMessage is sent as it is, so numbers keep their text); nil is an empty object.
func (c *Client) Start(ctx context.Context, workflow string, input any, opts ...CallOption) (*Run, error) {
	o := callOptionsOf(opts)
	idempotencyKey, err := key(o.idempotencyKey)
	if err != nil {
		return nil, err
	}
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	deadline := o.deadline
	if o.hasDeadlineIn {
		deadline = time.Now().Add(o.deadlineIn)
	}
	workflowID, err := c.ResolveWorkflow(ctx, workflow)
	if err != nil {
		return nil, err
	}
	if input == nil {
		input = map[string]any{}
	}
	body := map[string]any{"workflowId": workflowID, "input": input}
	if o.versionID != "" {
		body["versionId"] = o.versionID
	}
	if o.correlationID != "" {
		body["correlationId"] = o.correlationID
	}
	if !deadline.IsZero() {
		body["deadline"] = jsonx.Instant(deadline)
	}
	resp, err := c.do(ctx, request{
		method: "POST", path: "/runs", body: body, hasBody: true,
		idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true,
	})
	if err != nil {
		return nil, err
	}
	return parseRun(resp.body)
}

// UploadRunInput streams source exactly once and accepts only a descriptor matching the bytes read.
func (c *Client) UploadRunInput(ctx context.Context, source io.Reader, options UploadRunInputOptions) (BlobDescriptor, error) {
	if source == nil {
		return BlobDescriptor{}, sagat.Invalid("a run-input source is required")
	}
	if options.ExpectedSize != nil && *options.ExpectedSize < 0 {
		return BlobDescriptor{}, sagat.Invalid("blob expected size is non-negative")
	}
	if options.ExpectedSHA256 != "" && !regexp.MustCompile(`^[0-9a-f]{64}$`).MatchString(options.ExpectedSHA256) {
		return BlobDescriptor{}, sagat.Invalid("blob expected SHA-256 is lowercase hexadecimal")
	}
	digest := sha256.New()
	reader := &measuredReader{source: source, digest: digest}
	headers := make(http.Header)
	if options.ExpectedSize != nil {
		headers.Set("X-Octopus-Blob-Size", strconv.FormatInt(*options.ExpectedSize, 10))
	}
	if options.ExpectedSHA256 != "" {
		headers.Set("X-Octopus-Blob-Sha256", options.ExpectedSHA256)
	}
	contentType := options.ContentType
	if contentType == "" {
		contentType = "application/octet-stream"
	}
	resp, err := c.do(ctx, request{method: "POST", path: "/run-input-blobs", rawBody: reader, contentType: contentType, headers: headers})
	if err != nil {
		return BlobDescriptor{}, err
	}
	raw, err := jsonx.Object(resp.body)
	if err != nil {
		return BlobDescriptor{}, sagat.NewError(sagat.CodeProtocol, "the blob descriptor is not an object")
	}
	d := BlobDescriptor{}
	d.ID, _ = jsonx.String(raw, "id")
	d.Size, _ = jsonx.Int(raw, "size")
	d.SHA256, _ = jsonx.String(raw, "sha256")
	d.ContentType, _ = jsonx.String(raw, "contentType")
	if !reader.eof || !uuidPattern.MatchString(d.ID) || d.Size != reader.size || d.SHA256 != hex.EncodeToString(digest.Sum(nil)) || d.ContentType == "" {
		return BlobDescriptor{}, sagat.NewError(sagat.CodeProtocol, "the run-input descriptor does not match the uploaded bytes")
	}
	return d, nil
}

type measuredReader struct {
	source io.Reader
	digest io.Writer
	size   int64
	eof    bool
}

func (r *measuredReader) Read(p []byte) (int, error) {
	n, err := r.source.Read(p)
	if n > 0 {
		_, _ = r.digest.Write(p[:n])
		r.size += int64(n)
	}
	if err == io.EOF {
		r.eof = true
	}
	return n, err
}

// StartWithBlob atomically claims a descriptor previously returned by UploadRunInput.
func (c *Client) StartWithBlob(ctx context.Context, workflow string, descriptor BlobDescriptor, opts ...CallOption) (*Run, error) {
	if !uuidPattern.MatchString(descriptor.ID) || descriptor.Size < 0 || !regexp.MustCompile(`^[0-9a-f]{64}$`).MatchString(descriptor.SHA256) || descriptor.ContentType == "" {
		return nil, sagat.Invalid("a run-input blob descriptor is malformed")
	}
	o := callOptionsOf(opts)
	idempotencyKey, err := key(o.idempotencyKey)
	if err != nil {
		return nil, err
	}
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	deadline := o.deadline
	if o.hasDeadlineIn {
		deadline = time.Now().Add(o.deadlineIn)
	}
	workflowID, err := c.ResolveWorkflow(ctx, workflow)
	if err != nil {
		return nil, err
	}
	body := map[string]any{"workflowId": workflowID, "inputBlobRef": descriptor.ID}
	if o.versionID != "" {
		body["versionId"] = o.versionID
	}
	if o.correlationID != "" {
		body["correlationId"] = o.correlationID
	}
	if !deadline.IsZero() {
		body["deadline"] = jsonx.Instant(deadline)
	}
	resp, err := c.do(ctx, request{method: "POST", path: "/runs", body: body, hasBody: true, idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true})
	if err != nil {
		return nil, err
	}
	return parseRun(resp.body)
}

// Get reads a run.
func (c *Client) Get(ctx context.Context, runID string) (*Run, error) {
	path, err := runPath(runID)
	if err != nil {
		return nil, err
	}
	traceparent, err := c.trace(ctx, "")
	if err != nil {
		return nil, err
	}
	resp, err := c.do(ctx, request{method: "GET", path: path, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	return parseRun(resp.body)
}

// ListOptions filters and pages the runs of List and ListAll.
type ListOptions struct {
	WorkflowID string
	Status     RunStatus
	// Limit is the page size; the server's default when zero.
	Limit int
	// Cursor continues a previous page.
	Cursor string
}

// List reads one page of runs, newest first.
func (c *Client) List(ctx context.Context, options ListOptions) (*RunPage, error) {
	traceparent, err := c.trace(ctx, "")
	if err != nil {
		return nil, err
	}
	query := url.Values{}
	if options.WorkflowID != "" {
		query.Set("workflowId", options.WorkflowID)
	}
	if options.Status != "" {
		query.Set("status", string(options.Status))
	}
	if options.Limit > 0 {
		query.Set("limit", strconv.Itoa(options.Limit))
	}
	if options.Cursor != "" {
		query.Set("cursor", options.Cursor)
	}
	resp, err := c.do(ctx, request{method: "GET", path: "/runs", query: query, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	return parseRunPage(resp.body)
}

// ListAll yields every run that matches, page after page, in the order the server gave (from
// options.Cursor when one is given). The iteration ends with the first error, which is yielded last.
//
//	for run, err := range c.ListAll(ctx, client.ListOptions{Status: client.StatusRunning}) { ... }
func (c *Client) ListAll(ctx context.Context, options ListOptions) iter.Seq2[*Run, error] {
	return func(yield func(*Run, error) bool) {
		for {
			page, err := c.List(ctx, options)
			if err != nil {
				yield(nil, err)
				return
			}
			for _, run := range page.Items {
				if !yield(run, nil) {
					return
				}
			}
			if page.NextCursor == "" {
				return
			}
			options.Cursor = page.NextCursor
		}
	}
}

// Action is an operation on a run.
type Action string

// The actions of Operate.
const (
	ActionPause             Action = "PAUSE"
	ActionResume            Action = "RESUME"
	ActionCancel            Action = "CANCEL"
	ActionRetryStep         Action = "RETRY_STEP"
	ActionRetryCompensation Action = "RETRY_COMPENSATION"
	ActionSkipCompensation  Action = "SKIP_COMPENSATION"
	ActionResolveStep       Action = "RESOLVE_STEP"
)

// Operate applies an action to a run. reason has 8 to 2000 characters. Reads WithStepID,
// WithOutcome, WithErrorCode, WithExpectedRunVersion, WithIdempotencyKey and WithTraceparent.
func (c *Client) Operate(ctx context.Context, runID string, action Action, reason string, opts ...CallOption) (*OperationResult, error) {
	path, err := runPath(runID)
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	idempotencyKey, err := key(o.idempotencyKey)
	if err != nil {
		return nil, err
	}
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	body := map[string]any{"action": string(action), "reason": reason}
	if o.stepID != "" {
		body["stepId"] = o.stepID
	}
	if o.outcome != "" {
		body["outcome"] = o.outcome
	}
	if o.errorCode != "" {
		body["errorCode"] = o.errorCode
	}
	if o.expectedRunVersion != nil {
		body["expectedRunVersion"] = *o.expectedRunVersion
	}
	resp, err := c.do(ctx, request{
		method: "POST", path: path + "/operations", body: body, hasBody: true,
		idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true,
	})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "an operation result")
	if err != nil {
		return nil, err
	}
	result := &OperationResult{Raw: raw}
	result.OperationID, _ = jsonx.String(raw, "operationId")
	if result.OperationID == "" {
		return nil, protocolError("operationId is missing from the answer")
	}
	result.Accepted, _ = jsonx.Bool(raw, "accepted")
	status, _ := jsonx.String(raw, "runStatus")
	result.RunStatus = RunStatus(status)
	result.RunVersion, _ = jsonx.Int(raw, "runVersion")
	return result, nil
}

// Cancel cancels a run. reason has 8 to 2000 characters. It is Operate with ActionCancel.
func (c *Client) Cancel(ctx context.Context, runID, reason string, opts ...CallOption) (*OperationResult, error) {
	return c.Operate(ctx, runID, ActionCancel, reason, opts...)
}

var approvalDecisionKeyPattern = regexp.MustCompile(`^[\x21-\x7e]{1,128}$`)
var approvalReasonCodePattern = regexp.MustCompile(`^[A-Za-z0-9_.:-]{1,64}$`)
var declaredOperationNamePattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$`)
var updateKeyPattern = regexp.MustCompile(`^[\x21-\x7e]{1,128}$`)

// ListApprovals reads one bounded page in immutable opening order.
func (c *Client) ListApprovals(ctx context.Context, runID string, options ApprovalListOptions, opts ...CallOption) (*ApprovalPage, error) {
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
	if options.Limit > 0 {
		query.Set("limit", strconv.Itoa(options.Limit))
	}
	resp, err := c.do(ctx, request{method: "GET", path: path + "/approvals", query: query, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "an approval page")
	if err != nil {
		return nil, err
	}
	var values []map[string]json.RawMessage
	if encoded, ok := raw["items"]; !ok || json.Unmarshal(encoded, &values) != nil || len(values) > 200 {
		return nil, protocolError("items is invalid in the answer")
	}
	page := &ApprovalPage{Items: make([]ApprovalListItem, 0, len(values)), Raw: raw}
	page.NextCursor, _ = jsonx.String(raw, "nextCursor")
	for _, value := range values {
		item := ApprovalListItem{Raw: value}
		if item.ApprovalID, err = requiredString(value, "approvalId"); err != nil {
			return nil, err
		}
		if item.RunID, err = requiredString(value, "runId"); err != nil {
			return nil, err
		}
		if item.NodeID, err = requiredString(value, "nodeId"); err != nil {
			return nil, err
		}
		status, statusErr := requiredString(value, "status")
		if statusErr != nil || !validApprovalStatus(ApprovalStatus(status)) {
			return nil, protocolError("status is invalid in the answer")
		}
		item.Status = ApprovalStatus(status)
		if item.Deadline, _ = jsonx.Time(value, "deadline"); item.Deadline.IsZero() {
			return nil, protocolError("deadline is missing from the answer")
		}
		item.SettledAt, _ = jsonx.Time(value, "settledAt")
		var scopes []map[string]json.RawMessage
		if encoded, ok := value["instancePath"]; !ok || json.Unmarshal(encoded, &scopes) != nil || len(scopes) > 8 {
			return nil, protocolError("instancePath is invalid in the answer")
		}
		item.InstancePath = make([]ApprovalScope, 0, len(scopes))
		for _, scope := range scopes {
			node, nodeErr := requiredString(scope, "scopeNodeId")
			index, indexErr := requiredRevision(scope, "iterationIndex", 0)
			if nodeErr != nil || indexErr != nil || index > 9_999 {
				return nil, protocolError("approval scope is invalid in the answer")
			}
			item.InstancePath = append(item.InstancePath, ApprovalScope{ScopeNodeID: node, IterationIndex: int(index)})
		}
		page.Items = append(page.Items, item)
	}
	return page, nil
}

// ListAllApprovals yields every approval page until the opaque cursor ends.
func (c *Client) ListAllApprovals(ctx context.Context, runID string, options ApprovalListOptions, opts ...CallOption) iter.Seq2[*ApprovalListItem, error] {
	return func(yield func(*ApprovalListItem, error) bool) {
		for {
			page, err := c.ListApprovals(ctx, runID, options, opts...)
			if err != nil {
				yield(nil, err)
				return
			}
			for index := range page.Items {
				if !yield(&page.Items[index], nil) {
					return
				}
			}
			if page.NextCursor == "" {
				return
			}
			options.Cursor = page.NextCursor
		}
	}
}

func validApprovalStatus(status ApprovalStatus) bool {
	return status == ApprovalPending || status == ApprovalApproved || status == ApprovalRejected || status == ApprovalExpired || status == ApprovalCancelled
}

// DecideApproval records an eligible actor's durable decision. The body DecisionKey makes retries
// safe; WithTraceparent is the only call option this method reads.
func (c *Client) DecideApproval(ctx context.Context, approvalID string, decision ApprovalDecisionRequest, opts ...CallOption) (*ApprovalDecisionReceipt, error) {
	if !uuidPattern.MatchString(approvalID) {
		return nil, sagat.Invalid("an approval id is a UUID")
	}
	if decision.Decision != ApprovalApprove && decision.Decision != ApprovalReject {
		return nil, sagat.Invalid("an approval decision is APPROVE or REJECT")
	}
	if !approvalDecisionKeyPattern.MatchString(decision.DecisionKey) {
		return nil, sagat.Invalid("an approval decision key is 1 to 128 visible ASCII characters")
	}
	if decision.ReasonCode != "" && !approvalReasonCodePattern.MatchString(decision.ReasonCode) {
		return nil, sagat.Invalid("an approval reason code is 1 to 64 letters, digits, dots, underscores, colons or hyphens")
	}
	o := callOptionsOf(opts)
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	body := map[string]any{"decisionKey": decision.DecisionKey, "decision": string(decision.Decision)}
	if decision.ReasonCode != "" {
		body["reasonCode"] = decision.ReasonCode
	}
	resp, err := c.do(ctx, request{method: "POST", path: "/approvals/" + lower(approvalID) + "/decisions", body: body, hasBody: true, traceparent: traceparent, repeatable: true})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "an approval decision receipt")
	if err != nil {
		return nil, err
	}
	receipt := &ApprovalDecisionReceipt{Raw: raw}
	if receipt.ReceiptID, err = requiredString(raw, "receiptId"); err != nil {
		return nil, err
	}
	if receipt.ApprovalID, err = requiredString(raw, "approvalId"); err != nil {
		return nil, err
	}
	if receipt.DecisionKey, err = requiredString(raw, "decisionKey"); err != nil {
		return nil, err
	}
	value, err := requiredString(raw, "decision")
	if err != nil || (value != string(ApprovalApprove) && value != string(ApprovalReject)) {
		return nil, protocolError("decision is invalid in the answer")
	}
	receipt.Decision = ApprovalDecision(value)
	receipt.ReasonCode, _ = jsonx.String(raw, "reasonCode")
	if receipt.Actor, err = requiredString(raw, "actor"); err != nil {
		return nil, err
	}
	if receipt.DecidedAt, _ = jsonx.Time(raw, "decidedAt"); receipt.DecidedAt.IsZero() {
		return nil, protocolError("decidedAt is missing from the answer")
	}
	var hasOutcomeSequence bool
	if receipt.OutcomeSequence, hasOutcomeSequence = jsonx.Int(raw, "outcomeSequence"); !hasOutcomeSequence || receipt.OutcomeSequence < 1 {
		return nil, protocolError("outcomeSequence is invalid in the answer")
	}
	receipt.Replayed, _ = jsonx.Bool(raw, "replayed")
	return receipt, nil
}

// Query evaluates a declared side-effect-free query over one committed state snapshot.
func (c *Client) Query(ctx context.Context, runID, name string, args any, opts ...CallOption) (*QueryResult, error) {
	path, err := declaredOperationPath(runID, "queries", name)
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	resp, err := c.do(ctx, request{method: "POST", path: path, body: map[string]any{"args": args}, hasBody: true, traceparent: traceparent, repeatable: true})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a query result")
	if err != nil {
		return nil, err
	}
	result := &QueryResult{Raw: raw}
	if result.RunID, err = requiredString(raw, "runId"); err != nil {
		return nil, err
	}
	if result.RunVersion, err = requiredRevision(raw, "runVersion", 0); err != nil {
		return nil, err
	}
	if result.StateRevision, err = requiredRevision(raw, "stateRevision", 0); err != nil {
		return nil, err
	}
	if value, ok := raw["result"]; ok {
		if err := json.Unmarshal(value, &result.Result); err != nil {
			return nil, protocolError("result is invalid in the answer")
		}
	} else {
		return nil, protocolError("result is missing from the answer")
	}
	return result, nil
}

// Update applies or replays a declared durable state update.
func (c *Client) Update(ctx context.Context, runID, name string, update UpdateRequest, opts ...CallOption) (*UpdateReceipt, error) {
	path, err := declaredOperationPath(runID, "updates", name)
	if err != nil {
		return nil, err
	}
	if !updateKeyPattern.MatchString(update.UpdateKey) {
		return nil, sagat.Invalid("an update key is 1 to 128 visible ASCII characters")
	}
	if update.ExpectedStateRevision != nil && *update.ExpectedStateRevision < 0 {
		return nil, sagat.Invalid("expected state revision is non-negative")
	}
	o := callOptionsOf(opts)
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	body := map[string]any{"updateKey": update.UpdateKey, "args": update.Args}
	if update.ExpectedStateRevision != nil {
		body["expectedStateRevision"] = *update.ExpectedStateRevision
	}
	resp, err := c.do(ctx, request{method: "POST", path: path, body: body, hasBody: true, traceparent: traceparent, repeatable: true})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "an update receipt")
	if err != nil {
		return nil, err
	}
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
	if receipt.Status, err = requiredString(raw, "status"); err != nil || receipt.Status != "APPLIED" {
		return nil, protocolError("status is invalid in the answer")
	}
	if value, ok := raw["result"]; ok {
		if err := json.Unmarshal(value, &receipt.Result); err != nil {
			return nil, protocolError("result is invalid in the answer")
		}
	} else {
		return nil, protocolError("result is missing from the answer")
	}
	if receipt.BeforeStateRevision, err = requiredRevision(raw, "beforeStateRevision", 0); err != nil {
		return nil, err
	}
	if receipt.AfterStateRevision, err = requiredRevision(raw, "afterStateRevision", 1); err != nil {
		return nil, err
	}
	if receipt.RunVersion, err = requiredRevision(raw, "runVersion", 1); err != nil {
		return nil, err
	}
	receipt.Replayed, _ = jsonx.Bool(raw, "replayed")
	return receipt, nil
}

func declaredOperationPath(runID, kind, name string) (string, error) {
	path, err := runPath(runID)
	if err != nil {
		return "", err
	}
	if !declaredOperationNamePattern.MatchString(name) {
		return "", sagat.Invalid("an operation name is 1 to 128 permitted characters and starts with a letter or digit")
	}
	return path + "/" + kind + "/" + url.PathEscape(name), nil
}

func requiredRevision(raw map[string]json.RawMessage, key string, minimum int64) (int64, error) {
	value, ok := jsonx.Int(raw, key)
	if !ok || value < minimum {
		return 0, protocolError("%s is invalid in the answer", key)
	}
	return value, nil
}

// Signal sends a signal to the steps of a run that wait for it; a signal nobody waits for is a
// CONFLICT. payload is encoded as a JSON object; nil is an empty object.
func (c *Client) Signal(ctx context.Context, runID, name string, payload any, opts ...CallOption) (*SignalResult, error) {
	path, err := runPath(runID)
	if err != nil {
		return nil, err
	}
	encodedName, err := segment(name)
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	idempotencyKey, err := key(o.idempotencyKey)
	if err != nil {
		return nil, err
	}
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	if payload == nil {
		payload = map[string]any{}
	}
	resp, err := c.do(ctx, request{
		method: "POST", path: path + "/signals/" + encodedName, body: payload, hasBody: true,
		idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true,
	})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a signal result")
	if err != nil {
		return nil, err
	}
	result := &SignalResult{DeliveredTo: []string{}, Raw: raw}
	if delivered, ok := raw["deliveredTo"]; ok {
		var names []string
		if err := json.Unmarshal(delivered, &names); err == nil {
			result.DeliveredTo = names
		}
	}
	return result, nil
}

// InspectChain reads one committed chain head and lineage snapshot.
func (c *Client) InspectChain(ctx context.Context, namespaceID, chainID string, opts ...CallOption) (*ChainInspection, error) {
	path, err := chainPath(namespaceID, chainID)
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	resp, err := c.do(ctx, request{method: "GET", path: path, traceparent: traceparent, repeatable: true})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a chain inspection")
	if err != nil {
		return nil, err
	}
	result := &ChainInspection{Raw: raw}
	if result.ChainID, err = requiredString(raw, "chainId"); err != nil {
		return nil, err
	}
	if result.CurrentRunID, err = requiredString(raw, "currentRunId"); err != nil {
		return nil, err
	}
	if result.Revision, err = requiredRevision(raw, "revision", 0); err != nil {
		return nil, err
	}
	result.CancelRequested, _ = jsonx.Bool(raw, "cancelRequested")
	result.FinalOutcome, _ = jsonx.String(raw, "finalOutcome")
	var segments []map[string]json.RawMessage
	if value, ok := raw["segments"]; !ok || json.Unmarshal(value, &segments) != nil || len(segments) == 0 {
		return nil, protocolError("segments is invalid in the answer")
	}
	result.Segments = make([]ChainSegment, 0, len(segments))
	for _, item := range segments {
		index, err := requiredRevision(item, "index", 0)
		if err != nil || index > 65535 {
			return nil, protocolError("index is invalid in the answer")
		}
		runID, err := requiredString(item, "runId")
		if err != nil {
			return nil, err
		}
		predecessor, _ := jsonx.String(item, "predecessorRunId")
		successor, _ := jsonx.String(item, "successorRunId")
		result.Segments = append(result.Segments, ChainSegment{Index: int(index), RunID: runID, PredecessorRunID: predecessor, SuccessorRunID: successor})
	}
	return result, nil
}

// SignalChain signals a root wait on the expected current chain head.
func (c *Client) SignalChain(ctx context.Context, namespaceID, chainID, name string, payload any, command ChainCommand, opts ...CallOption) (*SignalResult, error) {
	path, err := chainPath(namespaceID, chainID)
	if err != nil {
		return nil, err
	}
	if !uuidPattern.MatchString(command.ExpectedRunID) {
		return nil, sagat.Invalid("an expected run id is a UUID")
	}
	encodedName, err := segment(name)
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	idempotencyKey, err := key(o.idempotencyKey)
	if err != nil {
		return nil, err
	}
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	resp, err := c.do(ctx, request{method: "POST", path: path + "/signals/" + encodedName, body: map[string]any{"expectedRunId": lower(command.ExpectedRunID), "payload": payload}, hasBody: true, idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a chain signal result")
	if err != nil {
		return nil, err
	}
	var delivered []string
	_ = json.Unmarshal(raw["deliveredTo"], &delivered)
	return &SignalResult{DeliveredTo: delivered, Raw: raw}, nil
}

// CancelChain latches cancellation and cancels the expected current chain head.
func (c *Client) CancelChain(ctx context.Context, namespaceID, chainID, reason string, command ChainCommand, opts ...CallOption) error {
	path, err := chainPath(namespaceID, chainID)
	if err != nil {
		return err
	}
	if !uuidPattern.MatchString(command.ExpectedRunID) {
		return sagat.Invalid("an expected run id is a UUID")
	}
	if utf8.RuneCountInString(reason) < 1 || utf8.RuneCountInString(reason) > 512 {
		return sagat.Invalid("a chain cancellation reason is 1 to 512 characters")
	}
	o := callOptionsOf(opts)
	idempotencyKey, err := key(o.idempotencyKey)
	if err != nil {
		return err
	}
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return err
	}
	_, err = c.do(ctx, request{method: "POST", path: path + "/cancel", body: map[string]any{"expectedRunId": lower(command.ExpectedRunID), "reason": reason}, hasBody: true, idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true})
	return err
}

// Event is an event published to the inbox of the namespace.
type Event struct {
	// ID is unique within the source for the deduplication window; a random one is generated per call
	// when empty and kept across its retries, so a retry is answered DUPLICATE and never admitted
	// twice.
	ID   string
	Type string
	// Data is encoded as a JSON object; nil is an empty object.
	Data           any
	Subject        string
	OccurredAt     time.Time
	CorrelationKey string
	// DedupeTTLSeconds overrides the deduplication window when positive.
	DedupeTTLSeconds int
}

// PublishEvent publishes an event, once per event id for the deduplication window. Reads
// WithTraceparent.
func (c *Client) PublishEvent(ctx context.Context, event Event, opts ...CallOption) (*EventReceipt, error) {
	o := callOptionsOf(opts)
	traceparent, err := c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	id := event.ID
	if id == "" {
		id = ids.UUID()
	}
	data := event.Data
	if data == nil {
		data = map[string]any{}
	}
	body := map[string]any{"eventId": id, "type": event.Type, "data": data}
	if event.Subject != "" {
		body["subject"] = event.Subject
	}
	if !event.OccurredAt.IsZero() {
		body["occurredAt"] = jsonx.Instant(event.OccurredAt)
	}
	if event.CorrelationKey != "" {
		body["correlationKey"] = event.CorrelationKey
	}
	if event.DedupeTTLSeconds > 0 {
		body["dedupeTtlSeconds"] = event.DedupeTTLSeconds
	}
	// The event id is the key: the server answers a repetition DUPLICATE.
	resp, err := c.do(ctx, request{method: "POST", path: "/events", body: body, hasBody: true, traceparent: traceparent, repeatable: true})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "an event answer")
	if err != nil {
		return nil, err
	}
	receipt := &EventReceipt{Raw: raw}
	if receipt.EventID, err = requiredString(raw, "eventId"); err != nil {
		return nil, err
	}
	if receipt.Source, err = requiredString(raw, "source"); err != nil {
		return nil, err
	}
	status, err := requiredString(raw, "status")
	if err != nil {
		return nil, err
	}
	receipt.Status = EventStatus(status)
	return receipt, nil
}

// EventsOptions pages the journal of a run.
type EventsOptions struct {
	Cursor string
	Limit  int
}

// Events reads a page of the journal of a run.
func (c *Client) Events(ctx context.Context, runID string, options EventsOptions) (*JournalPage, error) {
	path, err := runPath(runID)
	if err != nil {
		return nil, err
	}
	traceparent, err := c.trace(ctx, "")
	if err != nil {
		return nil, err
	}
	query := url.Values{}
	if options.Cursor != "" {
		query.Set("cursor", options.Cursor)
	}
	if options.Limit > 0 {
		query.Set("limit", strconv.Itoa(options.Limit))
	}
	resp, err := c.do(ctx, request{method: "GET", path: path + "/events", query: query, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a page")
	if err != nil {
		return nil, err
	}
	page := &JournalPage{Items: []JournalEvent{}, Raw: raw}
	var items []json.RawMessage
	_ = json.Unmarshal(raw["items"], &items)
	for _, item := range items {
		m, err := object(item, "an event")
		if err != nil {
			return nil, err
		}
		event := JournalEvent{Raw: m}
		event.Type, _ = jsonx.String(m, "eventType")
		event.StepID, _ = jsonx.String(m, "stepId")
		if attempt, ok := jsonx.Int(m, "attempt"); ok {
			a := int(attempt)
			event.Attempt = &a
		}
		page.Items = append(page.Items, event)
	}
	page.NextCursor, _ = jsonx.String(raw, "nextCursor")
	return page, nil
}

// WaitOption adjusts WaitForCompletion.
type WaitOption func(*waitOptions)

type waitOptions struct {
	timeout      time.Duration
	pollInterval time.Duration
}

// WaitTimeout bounds the wait; 60 s by default.
func WaitTimeout(d time.Duration) WaitOption { return func(o *waitOptions) { o.timeout = d } }

// PollInterval sets the time between two reads of the run, with 20 % jitter; 500 ms by default.
func PollInterval(d time.Duration) WaitOption { return func(o *waitOptions) { o.pollInterval = d } }

// WaitForCompletion reads the run until its status is terminal and returns it. When the wait
// timeout passes first it returns a TRANSPORT error with the reason WAIT_TIMEOUT; the run is
// untouched and may be waited for again.
func (c *Client) WaitForCompletion(ctx context.Context, runID string, opts ...WaitOption) (*Run, error) {
	o := waitOptions{timeout: 60 * time.Second, pollInterval: 500 * time.Millisecond}
	for _, opt := range opts {
		opt(&o)
	}
	end := time.Now().Add(o.timeout)
	for {
		run, err := c.Get(ctx, runID)
		if err != nil {
			return nil, err
		}
		if run.Status.Terminal() {
			return run, nil
		}
		if !time.Now().Before(end) {
			e := sagat.Errorf(sagat.CodeTransport, "the run did not finish within %d ms", o.timeout.Milliseconds())
			e.Reason = "WAIT_TIMEOUT"
			return nil, e
		}
		wait := time.Duration(float64(o.pollInterval) * (0.8 + rand.Float64()*0.4))
		if err := backoff.Sleep(ctx, wait); err != nil {
			return nil, sagat.NewError(sagat.CodeCancelled, "cancelled while waiting").WithCause(err)
		}
	}
}
