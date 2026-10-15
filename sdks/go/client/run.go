package client

import (
	"encoding/json"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// RunStatus is the status of a run as the server names it.
type RunStatus string

// The statuses of a run. The server may add others; a status the SDK does not know is returned as
// it came.
const (
	StatusRunning            RunStatus = "RUNNING"
	StatusWaiting            RunStatus = "WAITING"
	StatusPaused             RunStatus = "PAUSED"
	StatusCancelling         RunStatus = "CANCELLING"
	StatusCompensating       RunStatus = "COMPENSATING"
	StatusManualReview       RunStatus = "MANUAL_REVIEW"
	StatusSucceeded          RunStatus = "SUCCEEDED"
	StatusFailed             RunStatus = "FAILED"
	StatusCancelled          RunStatus = "CANCELLED"
	StatusTimedOut           RunStatus = "TIMED_OUT"
	StatusCompensated        RunStatus = "COMPENSATED"
	StatusPartial            RunStatus = "PARTIAL"
	StatusCompensationFailed RunStatus = "COMPENSATION_FAILED"
)

// Terminal reports whether no further transition of a run in this status is expected.
func (s RunStatus) Terminal() bool {
	switch s {
	case StatusSucceeded, StatusFailed, StatusCancelled, StatusTimedOut, StatusCompensated, StatusPartial, StatusCompensationFailed:
		return true
	}
	return false
}

// ParentRun says which node of which run started a child run.
type ParentRun struct {
	RunID        string
	Node         string
	CancelPolicy string
}

// Run is a workflow run. An instant that the server did not send is the zero time.Time; every
// instant is in UTC. A string that the server did not send, or sent as null, is empty.
type Run struct {
	RunID      string
	WorkflowID string
	VersionID  string
	Status     RunStatus
	// Version is the number of journal events applied; usable as the expected run version of an
	// operation.
	Version       int64
	Paused        bool
	StartedBy     string
	CreatedAt     time.Time
	UpdatedAt     time.Time
	FinishedAt    time.Time
	Deadline      time.Time
	TraceID       string
	CorrelationID string
	Depth         int
	Parent        *ParentRun
	// Raw is the answer as the server sent it, member by member, so that a field of a newer server
	// is reachable.
	Raw map[string]json.RawMessage
}

// RunPage is a page of runs, newest first as the server gave them.
type RunPage struct {
	Items []*Run
	// NextCursor is empty on the last page.
	NextCursor string
	Raw        map[string]json.RawMessage
}

// OperationResult is the answer to an operation on a run (cancel, pause, resume, ...).
type OperationResult struct {
	OperationID string
	Accepted    bool
	// RunStatus is empty when the server did not say.
	RunStatus  RunStatus
	RunVersion int64
	Raw        map[string]json.RawMessage
}

// ApprovalDecision is the outcome an eligible actor records for an approval request.
type ApprovalDecision string

const (
	ApprovalApprove ApprovalDecision = "APPROVE"
	ApprovalReject  ApprovalDecision = "REJECT"
)

// ApprovalDecisionRequest identifies one safely retryable decision.
type ApprovalDecisionRequest struct {
	DecisionKey string
	Decision    ApprovalDecision
	ReasonCode  string
}

// ApprovalDecisionReceipt proves which decision the server durably accepted.
type ApprovalDecisionReceipt struct {
	ReceiptID       string
	ApprovalID      string
	DecisionKey     string
	Decision        ApprovalDecision
	ReasonCode      string
	Actor           string
	DecidedAt       time.Time
	OutcomeSequence int64
	Replayed        bool
	Raw             map[string]json.RawMessage
}

// ApprovalStatus is the durable state of one approval request.
type ApprovalStatus string

const (
	ApprovalPending   ApprovalStatus = "PENDING"
	ApprovalApproved  ApprovalStatus = "APPROVED"
	ApprovalRejected  ApprovalStatus = "REJECTED"
	ApprovalExpired   ApprovalStatus = "EXPIRED"
	ApprovalCancelled ApprovalStatus = "CANCELLED"
)

// ApprovalScope is one ordered collection or loop segment owning an approval.
type ApprovalScope struct {
	ScopeNodeID    string
	IterationIndex int
}

// ApprovalListItem is the deliberately redacted public view of an approval request.
type ApprovalListItem struct {
	ApprovalID   string
	RunID        string
	NodeID       string
	InstancePath []ApprovalScope
	Status       ApprovalStatus
	Deadline     time.Time
	SettledAt    time.Time
	Raw          map[string]json.RawMessage
}

// ApprovalPage is one bounded page ordered by opening journal sequence.
type ApprovalPage struct {
	Items      []ApprovalListItem
	NextCursor string
	Raw        map[string]json.RawMessage
}

// ApprovalListOptions pages approval requests. Limit zero uses the server default.
type ApprovalListOptions struct {
	Cursor string
	Limit  int
}

// QueryResult is the declared result and exact committed revisions observed by a query.
type QueryResult struct {
	RunID         string
	RunVersion    int64
	StateRevision int64
	Result        any
	Raw           map[string]json.RawMessage
}

// UpdateRequest identifies one durable update. Retries keep UpdateKey unchanged.
type UpdateRequest struct {
	UpdateKey             string
	Args                  any
	ExpectedStateRevision *int64
}

// UpdateReceipt is the durable applied result of a declared update.
type UpdateReceipt struct {
	RunID               string
	Operation           string
	UpdateKey           string
	Status              string
	Result              any
	BeforeStateRevision int64
	AfterStateRevision  int64
	RunVersion          int64
	Replayed            bool
	Raw                 map[string]json.RawMessage
}

// SignalResult lists the steps a signal was delivered to.
type SignalResult struct {
	DeliveredTo []string
	Raw         map[string]json.RawMessage
}

// ChainSegment is one immutable run link in execution order.
type ChainSegment struct {
	Index            int
	RunID            string
	PredecessorRunID string
	SuccessorRunID   string
}

// ChainInspection is one committed snapshot of an execution chain.
type ChainInspection struct {
	ChainID         string
	CurrentRunID    string
	Revision        int64
	CancelRequested bool
	FinalOutcome    string
	Segments        []ChainSegment
	Raw             map[string]json.RawMessage
}

// ChainCommand identifies the head a safely retryable chain command targets.
type ChainCommand struct {
	ExpectedRunID string
}

// EventStatus says what the inbox did with a published event.
type EventStatus string

// The statuses of a published event.
const (
	EventAccepted  EventStatus = "ACCEPTED"
	EventDuplicate EventStatus = "DUPLICATE"
)

// EventReceipt is the answer to a published event.
type EventReceipt struct {
	EventID string
	Source  string
	Status  EventStatus
	Raw     map[string]json.RawMessage
}

// JournalEvent is an entry of the journal of a run. Raw keeps all of it.
type JournalEvent struct {
	Type    string
	StepID  string
	Attempt *int
	Raw     map[string]json.RawMessage
}

// JournalPage is a page of the journal of a run.
type JournalPage struct {
	Items      []JournalEvent
	NextCursor string
	Raw        map[string]json.RawMessage
}

func protocolError(format string, args ...any) *sagat.Error {
	return sagat.Errorf(sagat.CodeProtocol, format, args...)
}

func object(body []byte, what string) (map[string]json.RawMessage, error) {
	m, err := jsonx.Object(body)
	if err != nil {
		return nil, protocolError("%s is not an object", what)
	}
	return m, nil
}

func requiredString(m map[string]json.RawMessage, key string) (string, error) {
	s, ok := jsonx.String(m, key)
	if !ok {
		return "", protocolError("%s is missing from the answer", key)
	}
	return s, nil
}

func parseRun(body []byte) (*Run, error) {
	m, err := object(body, "a run")
	if err != nil {
		return nil, err
	}
	return runOf(m)
}

func runOf(m map[string]json.RawMessage) (*Run, error) {
	run := &Run{Raw: m}
	var err error
	if run.RunID, err = requiredString(m, "runId"); err != nil {
		return nil, err
	}
	if run.WorkflowID, err = requiredString(m, "workflowId"); err != nil {
		return nil, err
	}
	if run.VersionID, err = requiredString(m, "versionId"); err != nil {
		return nil, err
	}
	status, err := requiredString(m, "status")
	if err != nil {
		return nil, err
	}
	run.Status = RunStatus(status)
	created, ok := jsonx.Time(m, "createdAt")
	if !ok {
		return nil, protocolError("createdAt is missing from the answer")
	}
	run.CreatedAt = created
	run.Version, _ = jsonx.Int(m, "version")
	run.Paused, _ = jsonx.Bool(m, "paused")
	run.StartedBy, _ = jsonx.String(m, "startedBy")
	run.UpdatedAt, _ = jsonx.Time(m, "updatedAt")
	run.FinishedAt, _ = jsonx.Time(m, "finishedAt")
	run.Deadline, _ = jsonx.Time(m, "deadline")
	run.TraceID, _ = jsonx.String(m, "traceId")
	run.CorrelationID, _ = jsonx.String(m, "correlationId")
	depth, _ := jsonx.Int(m, "depth")
	run.Depth = int(depth)
	if raw, present := m["parent"]; present && !jsonx.IsNull(raw) {
		if parent, err := jsonx.Object(raw); err == nil {
			p := &ParentRun{}
			p.RunID, _ = jsonx.String(parent, "runId")
			p.Node, _ = jsonx.String(parent, "node")
			p.CancelPolicy, _ = jsonx.String(parent, "cancelPolicy")
			run.Parent = p
		}
	}
	return run, nil
}

func parseRunPage(body []byte) (*RunPage, error) {
	m, err := object(body, "a page")
	if err != nil {
		return nil, err
	}
	var items []json.RawMessage
	if err := json.Unmarshal(m["items"], &items); err != nil || m["items"] == nil {
		return nil, protocolError("items is missing from the page")
	}
	page := &RunPage{Items: make([]*Run, 0, len(items)), Raw: m}
	for _, item := range items {
		run, err := parseRun(item)
		if err != nil {
			return nil, err
		}
		page.Items = append(page.Items, run)
	}
	page.NextCursor, _ = jsonx.String(m, "nextCursor")
	return page, nil
}
