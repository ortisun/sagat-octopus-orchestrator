package client

import (
	"context"
	"encoding/json"
	"iter"
	"net/url"
	"strconv"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// OverlapPolicy says what a schedule does when a run it started is still open at its next action.
type OverlapPolicy string

// The overlap policies. The server may add others; one this SDK does not know is sent as given.
const (
	OverlapSkip        OverlapPolicy = "SKIP"
	OverlapBufferOne   OverlapPolicy = "BUFFER_ONE"
	OverlapBufferAll   OverlapPolicy = "BUFFER_ALL"
	OverlapCancelOther OverlapPolicy = "CANCEL_OTHER"
	OverlapAllowAll    OverlapPolicy = "ALLOW_ALL"
)

// WithOverlap sets the overlap policy of one action (Schedules.Trigger, Schedules.Backfill); without it
// the schedule's own policy applies.
func WithOverlap(p OverlapPolicy) CallOption { return func(o *callOptions) { o.overlap = p } }

// WithNextCount asks Schedules.Get for that many upcoming times; 0 asks for none. Without it the
// server's default applies.
func WithNextCount(n int) CallOption { return func(o *callOptions) { o.nextCount = &n } }

// WithRecentCount asks Schedules.Get for that many latest decisions; 0 asks for none. Without it the
// server's default applies.
func WithRecentCount(n int) CallOption { return func(o *callOptions) { o.recentCount = &n } }

// ScheduleRange is a range of a calendar field: Start, up to End (Start alone when End is nil), every
// Step (1 when nil).
type ScheduleRange struct {
	Start int  `json:"start"`
	End   *int `json:"end,omitempty"`
	Step  *int `json:"step,omitempty"`
}

// ScheduleCalendarSpec is the calendar form of a schedule: a time matches when every field it names matches.
type ScheduleCalendarSpec struct {
	Second     []ScheduleRange `json:"second,omitempty"`
	Minute     []ScheduleRange `json:"minute,omitempty"`
	Hour       []ScheduleRange `json:"hour,omitempty"`
	DayOfMonth []ScheduleRange `json:"dayOfMonth,omitempty"`
	Month      []ScheduleRange `json:"month,omitempty"`
	Year       []ScheduleRange `json:"year,omitempty"`
	DayOfWeek  []ScheduleRange `json:"dayOfWeek,omitempty"`
	Comment    string          `json:"comment,omitempty"`
}

// ScheduleInterval is a fixed interval, optionally shifted by an offset from the epoch.
type ScheduleInterval struct {
	EveryMs  int64  `json:"everyMs"`
	OffsetMs *int64 `json:"offsetMs,omitempty"`
}

// ScheduleDST says what a local time that the zone skips or repeats does: Gap is SKIP or SHIFT_FORWARD,
// Ambiguous is ONCE_EARLIER or TWICE.
type ScheduleDST struct {
	Gap       string `json:"gap,omitempty"`
	Ambiguous string `json:"ambiguous,omitempty"`
}

// ScheduleSpec says when a schedule acts: any of calendars, cron strings and intervals, less the
// exclusions, between StartAt and EndAt, in the zone Timezone.
type ScheduleSpec struct {
	Calendars  []ScheduleCalendarSpec `json:"calendars,omitempty"`
	Cron       []string               `json:"cron,omitempty"`
	Intervals  []ScheduleInterval     `json:"intervals,omitempty"`
	Exclusions []ScheduleCalendarSpec `json:"exclusions,omitempty"`
	// StartAt and EndAt bound the schedule; the zero time.Time is no bound. They are sent in UTC, as
	// instants with a Z, whatever the zone of the value.
	StartAt  time.Time    `json:"-"`
	EndAt    time.Time    `json:"-"`
	Timezone string       `json:"timezone,omitempty"`
	JitterMs int64        `json:"jitterMs,omitempty"`
	DST      *ScheduleDST `json:"dst,omitempty"`
}

// MarshalJSON writes StartAt and EndAt as UTC instants with a Z.
func (s ScheduleSpec) MarshalJSON() ([]byte, error) {
	type fields ScheduleSpec
	return jsonx.Encode(struct {
		fields
		StartAt string `json:"startAt,omitempty"`
		EndAt   string `json:"endAt,omitempty"`
	}{fields(s), instantText(s.StartAt), instantText(s.EndAt)})
}

// UnmarshalJSON reads StartAt and EndAt from RFC 3339 text with any offset; text that is not an instant is
// an error.
func (s *ScheduleSpec) UnmarshalJSON(data []byte) error {
	type fields ScheduleSpec
	wire := struct {
		*fields
		StartAt *string `json:"startAt"`
		EndAt   *string `json:"endAt"`
	}{fields: (*fields)(s)}
	if err := json.Unmarshal(data, &wire); err != nil {
		return err
	}
	var err error
	if s.StartAt, err = instantOf(wire.StartAt, "startAt"); err != nil {
		return err
	}
	s.EndAt, err = instantOf(wire.EndAt, "endAt")
	return err
}

// instantText is an instant as it is sent: UTC with a Z; the zero time.Time is left out.
func instantText(t time.Time) string {
	if t.IsZero() {
		return ""
	}
	return jsonx.Instant(t)
}

// instantOf reads optional RFC 3339 text; absent or null is the zero time.Time.
func instantOf(text *string, what string) (time.Time, error) {
	if text == nil {
		return time.Time{}, nil
	}
	t, ok := jsonx.ParseInstant(*text)
	if !ok {
		return time.Time{}, sagat.Invalid("%s is not an instant", what)
	}
	return t, nil
}

// SchedulePolicies say how a schedule behaves around its actions.
type SchedulePolicies struct {
	Overlap OverlapPolicy `json:"overlap,omitempty"`
	// CatchupWindowMs is how far back a missed time is still acted on (0: none). Nil is the server's default.
	CatchupWindowMs *int64 `json:"catchupWindowMs,omitempty"`
	// CatchupMode is ONE or ALL.
	CatchupMode    string `json:"catchupMode,omitempty"`
	PauseOnFailure *bool  `json:"pauseOnFailure,omitempty"`
	BufferLimit    *int   `json:"bufferLimit,omitempty"`
}

// ScheduleAction is what a schedule does at each time: start a run of a workflow.
type ScheduleAction struct {
	WorkflowID string `json:"workflowId"`
	// VersionPolicy is ACTIVE or PINNED (with VersionID).
	VersionPolicy string `json:"versionPolicy,omitempty"`
	VersionID     string `json:"versionId,omitempty"`
	// Input is the input of every run, at most 64 KiB, encoded as JSON (a json.RawMessage is sent as it
	// is); nil is an empty object. It is never returned: only its digest is.
	Input            any    `json:"input"`
	NominalTimeField string `json:"nominalTimeField,omitempty"`
	RunTimeoutMs     int64  `json:"runTimeoutMs,omitempty"`
	// BusinessIDPrefix makes every run the schedule starts carry the business id "<prefix>-<nominal time>"
	// (RFC 3339 UTC to the second). Nil sends none of the three business-id fields; a pointer to the empty
	// string sends the empty string, which the server refuses. It is customer text, sent as given: no trimming,
	// no case folding and no local check of its pattern or length (the server answers VALIDATION_FAILED with
	// the reason SCHEDULE_SPEC_INVALID); the SDK never logs it.
	BusinessIDPrefix *string `json:"businessIdPrefix,omitempty"`
	// BusinessIDConflictPolicy is FAIL (the server's default) or USE_EXISTING, and only means something with a
	// BusinessIDPrefix: the server judges any other combination. "" sends none.
	BusinessIDConflictPolicy BusinessIDConflictPolicy `json:"businessIdConflictPolicy,omitempty"`
	// BusinessIDReusePolicy is ALLOW_DUPLICATE (the server's default), ALLOW_DUPLICATE_FAILED_ONLY or
	// REJECT_DUPLICATE, and only means something with a BusinessIDPrefix. "" sends none.
	BusinessIDReusePolicy BusinessIDReusePolicy `json:"businessIdReusePolicy,omitempty"`
}

// MarshalJSON sends an empty object for a nil Input.
func (a ScheduleAction) MarshalJSON() ([]byte, error) {
	type fields ScheduleAction
	if a.Input == nil {
		a.Input = map[string]any{}
	}
	return jsonx.Encode(fields(a))
}

// ScheduleWrite is what a schedule is written as (Schedules.Create, Schedules.Update). An optional field
// left at its zero value is left out of the request, never sent as null.
type ScheduleWrite struct {
	// Name is 1 to 128 characters of a-z, 0-9 and "-", starting with a letter or digit.
	Name     string            `json:"name"`
	Spec     ScheduleSpec      `json:"spec"`
	Policies *SchedulePolicies `json:"policies,omitempty"`
	Action   ScheduleAction    `json:"action"`
	// Paused creates the schedule paused.
	Paused bool   `json:"paused,omitempty"`
	Note   string `json:"note,omitempty"`
	// RemainingActions limits how many times the schedule acts; nil is no limit.
	RemainingActions *int64 `json:"remainingActions,omitempty"`
}

// ScheduleActionInfo is the action as the server keeps it: without the input, with its digest.
type ScheduleActionInfo struct {
	WorkflowID       string
	VersionPolicy    string
	VersionID        string
	InputDigest      string
	NominalTimeField string
	RunTimeoutMs     *int64
	// BusinessIDPrefix, BusinessIDConflictPolicy and BusinessIDReusePolicy are all empty when the action names
	// no prefix; with a prefix the server writes the three, the policies with their defaults. The policies are
	// the strings the server sent: one this SDK does not know is never an error. The prefix is customer text.
	BusinessIDPrefix         string
	BusinessIDConflictPolicy string
	BusinessIDReusePolicy    string
}

// Schedule is a schedule as the server answers it. An instant or a string the server did not send, or sent
// as null, is the zero value; every instant is in UTC.
type Schedule struct {
	ScheduleID string
	Name       string
	Origin     string
	State      string
	Revision   int64
	Exhausted  bool
	Action     ScheduleActionInfo
	Note       string
	NoteBy     string
	// RemainingActions is nil when the schedule has no limit.
	RemainingActions *int64
	NextActionAt     time.Time
	CreatedAt        time.Time
	UpdatedAt        time.Time
	Raw              map[string]json.RawMessage
}

// ScheduleActionRecord is one decision a schedule made. Outcome is one of the outcomes this SDK knows or
// another string: the set only grows.
type ScheduleActionRecord struct {
	NominalTime            time.Time
	ActualTime             time.Time
	Kind                   string
	Origin                 string
	Revision               int64
	Outcome                string
	Reason                 string
	RunID                  string
	BackfillID             string
	SummaryCount           *int64
	SummaryLastNominalTime time.Time
	DecidedAt              time.Time
	Raw                    map[string]json.RawMessage
}

// ScheduleNextAction is an upcoming time of a schedule.
type ScheduleNextAction struct {
	NominalTime time.Time
	ActualTime  time.Time
}

// ScheduleDescription is a schedule with its next times, its latest decisions, the runs it has open and
// its counters.
type ScheduleDescription struct {
	Schedule
	NextActions   []ScheduleNextAction
	RecentActions []ScheduleActionRecord
	OpenRuns      []string
	BufferedCount int64
	Counters      map[string]int64
}

// ScheduleBackfill is a backfill: the past range [Start, End) it acts on, and how far it got.
type ScheduleBackfill struct {
	BackfillID string
	Start      time.Time
	End        time.Time
	Cursor     time.Time
	State      string
	Overlap    string
	Counts     map[string]int64
	Raw        map[string]json.RawMessage
}

// ScheduleListFilter filters and pages Schedules.List and Schedules.ListAll.
type ScheduleListFilter struct {
	// State is ACTIVE or PAUSED.
	State string
	// Origin is API or TRIGGER.
	Origin     string
	WorkflowID string
	// Name selects the names that start with this text.
	Name string
	// Limit is the page size; the server's default when zero.
	Limit int
	// Cursor continues a previous page.
	Cursor string
}

// ScheduleActionFilter filters and pages Schedules.ListActions.
type ScheduleActionFilter struct {
	Outcome string
	Limit   int
	Cursor  string
}

// Schedules is the group of schedule calls, reached from Client.Schedules. A schedule is a resource of a
// namespace given by its id (a UUID).
type Schedules struct{ c *Client }

func (s *Schedules) path(namespaceID, scheduleID, tail string) (string, error) {
	base, err := namespacePath(namespaceID)
	if err != nil {
		return "", err
	}
	base += "/schedules"
	if scheduleID == "" {
		return base + tail, nil
	}
	id, err := idSegment(scheduleID, "a schedule id")
	if err != nil {
		return "", err
	}
	return base + "/" + id + tail, nil
}

// Create creates a schedule. A name in use is CONFLICT; the same key and body answers what the first call
// did. Reads WithIdempotencyKey and WithTraceparent.
func (s *Schedules) Create(ctx context.Context, namespaceID string, write ScheduleWrite, opts ...CallOption) (*Schedule, error) {
	path, err := s.path(namespaceID, "", "")
	if err != nil {
		return nil, err
	}
	return s.write(ctx, "POST", path, write, opts)
}

// Update replaces the spec, the policies and the action as a new revision; a stale expectedRevision is
// CONFLICT with the reason REVISION_CONFLICT. expectedRevision is at least 1. Reads WithIdempotencyKey and
// WithTraceparent.
func (s *Schedules) Update(ctx context.Context, namespaceID, scheduleID string, write ScheduleWrite, expectedRevision int64, opts ...CallOption) (*Schedule, error) {
	if expectedRevision < 1 {
		return nil, sagat.Invalid("expectedRevision is a positive integer")
	}
	path, err := s.path(namespaceID, scheduleID, "")
	if err != nil {
		return nil, err
	}
	body := struct {
		ScheduleWrite
		ExpectedRevision int64 `json:"expectedRevision"`
	}{write, expectedRevision}
	return s.write(ctx, "PUT", path, body, opts)
}

func (s *Schedules) write(ctx context.Context, method, path string, body any, opts []CallOption) (*Schedule, error) {
	o := callOptionsOf(opts)
	idempotencyKey, traceparent, err := s.c.mutation(ctx, o)
	if err != nil {
		return nil, err
	}
	resp, err := s.c.do(ctx, request{
		method: method, path: path, body: body, hasBody: true,
		idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true,
	})
	if err != nil {
		return nil, err
	}
	return parseSchedule(resp.body)
}

// Get reads a schedule with its next times, its latest decisions, the runs it has open and its counters.
// Reads WithNextCount, WithRecentCount and WithTraceparent.
func (s *Schedules) Get(ctx context.Context, namespaceID, scheduleID string, opts ...CallOption) (*ScheduleDescription, error) {
	path, err := s.path(namespaceID, scheduleID, "")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := s.c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	query := url.Values{}
	if o.nextCount != nil {
		query.Set("nextCount", strconv.Itoa(*o.nextCount))
	}
	if o.recentCount != nil {
		query.Set("recentCount", strconv.Itoa(*o.recentCount))
	}
	resp, err := s.c.do(ctx, request{method: "GET", path: path, query: query, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a schedule description")
	if err != nil {
		return nil, err
	}
	schedule, err := scheduleOf(raw)
	if err != nil {
		return nil, err
	}
	description := &ScheduleDescription{Schedule: *schedule}
	description.NextActions = []ScheduleNextAction{}
	for _, member := range rawList(raw["nextActions"]) {
		entry, err := object(member, "a next action")
		if err != nil {
			return nil, err
		}
		var next ScheduleNextAction
		if next.NominalTime, err = requiredTime(entry, "nominalTime"); err != nil {
			return nil, err
		}
		if next.ActualTime, err = requiredTime(entry, "actualTime"); err != nil {
			return nil, err
		}
		description.NextActions = append(description.NextActions, next)
	}
	description.RecentActions = []ScheduleActionRecord{}
	for _, member := range rawList(raw["recentActions"]) {
		fields, err := object(member, "a schedule action")
		if err != nil {
			return nil, err
		}
		record, err := actionRecordOf(fields)
		if err != nil {
			return nil, err
		}
		description.RecentActions = append(description.RecentActions, *record)
	}
	description.OpenRuns = stringList(raw["openRuns"])
	description.BufferedCount = intOr(raw, "bufferedCount", 0)
	description.Counters = counters(raw["counters"])
	return description, nil
}

// List reads one page of schedules. Reads WithTraceparent.
func (s *Schedules) List(ctx context.Context, namespaceID string, filter ScheduleListFilter, opts ...CallOption) (*Page[Schedule], error) {
	path, err := s.path(namespaceID, "", "")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := s.c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	query := url.Values{}
	if filter.Cursor != "" {
		query.Set("cursor", filter.Cursor)
	}
	positive(query, "limit", filter.Limit)
	if filter.State != "" {
		query.Set("state", filter.State)
	}
	if filter.Origin != "" {
		query.Set("origin", filter.Origin)
	}
	if filter.Name != "" {
		query.Set("name", filter.Name)
	}
	if filter.WorkflowID != "" {
		id, err := idSegment(filter.WorkflowID, "a workflow id")
		if err != nil {
			return nil, err
		}
		query.Set("workflowId", id)
	}
	resp, err := s.c.do(ctx, request{method: "GET", path: path, query: query, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	return parsePage(resp.body, "a page of schedules", func(m map[string]json.RawMessage) (Schedule, error) {
		schedule, err := scheduleOf(m)
		if err != nil {
			return Schedule{}, err
		}
		return *schedule, nil
	})
}

// ListAll yields every schedule the filter selects, following the opaque cursors from filter.Cursor. The
// iteration ends with the first error, which is yielded last.
func (s *Schedules) ListAll(ctx context.Context, namespaceID string, filter ScheduleListFilter, opts ...CallOption) iter.Seq2[*Schedule, error] {
	return allPages(filter.Cursor, func(cursor string) (*Page[Schedule], error) {
		filter.Cursor = cursor
		return s.List(ctx, namespaceID, filter, opts...)
	})
}

// Delete deletes a schedule; the revision it expects is at least 1. Reads WithIdempotencyKey and
// WithTraceparent.
func (s *Schedules) Delete(ctx context.Context, namespaceID, scheduleID string, expectedRevision int64, opts ...CallOption) error {
	if expectedRevision < 1 {
		return sagat.Invalid("expectedRevision is a positive integer")
	}
	path, err := s.path(namespaceID, scheduleID, "")
	if err != nil {
		return err
	}
	o := callOptionsOf(opts)
	idempotencyKey, traceparent, err := s.c.mutation(ctx, o)
	if err != nil {
		return err
	}
	_, err = s.c.do(ctx, request{
		method: "DELETE", path: path, query: url.Values{"expectedRevision": {strconv.FormatInt(expectedRevision, 10)}},
		idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true,
	})
	return err
}

// Pause pauses a schedule with a note (not empty). Reads WithIdempotencyKey and WithTraceparent.
func (s *Schedules) Pause(ctx context.Context, namespaceID, scheduleID, note string, opts ...CallOption) (*Schedule, error) {
	return s.noted(ctx, namespaceID, scheduleID, "/pause", note, opts)
}

// Resume resumes a paused schedule with a note (not empty). Reads WithIdempotencyKey and WithTraceparent.
func (s *Schedules) Resume(ctx context.Context, namespaceID, scheduleID, note string, opts ...CallOption) (*Schedule, error) {
	return s.noted(ctx, namespaceID, scheduleID, "/resume", note, opts)
}

func (s *Schedules) noted(ctx context.Context, namespaceID, scheduleID, tail, note string, opts []CallOption) (*Schedule, error) {
	if note == "" {
		return nil, sagat.Invalid("a note is a non-empty text")
	}
	path, err := s.path(namespaceID, scheduleID, tail)
	if err != nil {
		return nil, err
	}
	return s.write(ctx, "POST", path, map[string]any{"note": note}, opts)
}

// Trigger acts on the schedule now, also while it is paused. The answer says what became of the action.
// Reads WithOverlap (for this action only), WithIdempotencyKey and WithTraceparent.
func (s *Schedules) Trigger(ctx context.Context, namespaceID, scheduleID string, opts ...CallOption) (*ScheduleActionRecord, error) {
	path, err := s.path(namespaceID, scheduleID, "/trigger")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	idempotencyKey, traceparent, err := s.c.mutation(ctx, o)
	if err != nil {
		return nil, err
	}
	body := map[string]any{}
	if o.overlap != "" {
		body["overlap"] = string(o.overlap)
	}
	resp, err := s.c.do(ctx, request{
		method: "POST", path: path, body: body, hasBody: true,
		idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true,
	})
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a schedule action")
	if err != nil {
		return nil, err
	}
	return actionRecordOf(raw)
}

// Backfill acts on every nominal time of the past range [start, end) that has no action yet. Both
// instants are sent in UTC. Reads WithOverlap, WithIdempotencyKey and WithTraceparent.
func (s *Schedules) Backfill(ctx context.Context, namespaceID, scheduleID string, start, end time.Time, opts ...CallOption) (*ScheduleBackfill, error) {
	if start.IsZero() {
		return nil, sagat.Invalid("start is not an instant")
	}
	if end.IsZero() {
		return nil, sagat.Invalid("end is not an instant")
	}
	path, err := s.path(namespaceID, scheduleID, "/backfills")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	idempotencyKey, traceparent, err := s.c.mutation(ctx, o)
	if err != nil {
		return nil, err
	}
	body := map[string]any{"start": jsonx.Instant(start), "end": jsonx.Instant(end)}
	if o.overlap != "" {
		body["overlap"] = string(o.overlap)
	}
	return s.backfill(ctx, request{
		method: "POST", path: path, body: body, hasBody: true,
		idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true,
	})
}

// CancelBackfill stops a backfill; one that already ended is CONFLICT. Reads WithIdempotencyKey and
// WithTraceparent.
func (s *Schedules) CancelBackfill(ctx context.Context, namespaceID, scheduleID, backfillID string, opts ...CallOption) (*ScheduleBackfill, error) {
	id, err := idSegment(backfillID, "a backfill id")
	if err != nil {
		return nil, err
	}
	path, err := s.path(namespaceID, scheduleID, "/backfills/"+id+"/cancel")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	idempotencyKey, traceparent, err := s.c.mutation(ctx, o)
	if err != nil {
		return nil, err
	}
	return s.backfill(ctx, request{
		method: "POST", path: path, idempotencyKey: idempotencyKey, traceparent: traceparent, repeatable: true,
	})
}

func (s *Schedules) backfill(ctx context.Context, req request) (*ScheduleBackfill, error) {
	resp, err := s.c.do(ctx, req)
	if err != nil {
		return nil, err
	}
	raw, err := object(resp.body, "a backfill")
	if err != nil {
		return nil, err
	}
	backfill, err := backfillOf(raw)
	if err != nil {
		return nil, err
	}
	return &backfill, nil
}

// ListBackfills reads at most the latest 100 backfills of the schedule. Reads WithTraceparent.
func (s *Schedules) ListBackfills(ctx context.Context, namespaceID, scheduleID string, opts ...CallOption) ([]ScheduleBackfill, error) {
	path, err := s.path(namespaceID, scheduleID, "/backfills")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := s.c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	resp, err := s.c.do(ctx, request{method: "GET", path: path, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	return parseItems(resp.body, "a list of backfills", backfillOf)
}

// ListActions reads one page of the decisions the schedule has made, newest first. Reads WithTraceparent.
func (s *Schedules) ListActions(ctx context.Context, namespaceID, scheduleID string, filter ScheduleActionFilter, opts ...CallOption) (*Page[ScheduleActionRecord], error) {
	path, err := s.path(namespaceID, scheduleID, "/actions")
	if err != nil {
		return nil, err
	}
	o := callOptionsOf(opts)
	traceparent, err := s.c.trace(ctx, o.traceparent)
	if err != nil {
		return nil, err
	}
	query := url.Values{}
	if filter.Cursor != "" {
		query.Set("cursor", filter.Cursor)
	}
	positive(query, "limit", filter.Limit)
	if filter.Outcome != "" {
		query.Set("outcome", filter.Outcome)
	}
	resp, err := s.c.do(ctx, request{method: "GET", path: path, query: query, repeatable: true, traceparent: traceparent})
	if err != nil {
		return nil, err
	}
	return parsePage(resp.body, "a page of schedule actions", func(m map[string]json.RawMessage) (ScheduleActionRecord, error) {
		record, err := actionRecordOf(m)
		if err != nil {
			return ScheduleActionRecord{}, err
		}
		return *record, nil
	})
}

// ListAllActions yields every decision the filter selects, following the opaque cursors.
func (s *Schedules) ListAllActions(ctx context.Context, namespaceID, scheduleID string, filter ScheduleActionFilter, opts ...CallOption) iter.Seq2[*ScheduleActionRecord, error] {
	return allPages(filter.Cursor, func(cursor string) (*Page[ScheduleActionRecord], error) {
		filter.Cursor = cursor
		return s.ListActions(ctx, namespaceID, scheduleID, filter, opts...)
	})
}

func parseSchedule(body []byte) (*Schedule, error) {
	raw, err := object(body, "a schedule")
	if err != nil {
		return nil, err
	}
	return scheduleOf(raw)
}

func scheduleOf(raw map[string]json.RawMessage) (*Schedule, error) {
	var err error
	schedule := &Schedule{Raw: raw}
	if schedule.ScheduleID, err = requiredString(raw, "scheduleId"); err != nil {
		return nil, err
	}
	if schedule.Name, err = requiredString(raw, "name"); err != nil {
		return nil, err
	}
	if schedule.Origin, err = requiredString(raw, "origin"); err != nil {
		return nil, err
	}
	if schedule.State, err = requiredString(raw, "state"); err != nil {
		return nil, err
	}
	if schedule.Revision, err = requiredRevision(raw, "revision", 1); err != nil {
		return nil, err
	}
	schedule.Exhausted, _ = jsonx.Bool(raw, "exhausted")
	if action, err := jsonx.Object(raw["action"]); err == nil {
		schedule.Action.WorkflowID, _ = jsonx.String(action, "workflowId")
		schedule.Action.VersionPolicy, _ = jsonx.String(action, "versionPolicy")
		schedule.Action.VersionID, _ = jsonx.String(action, "versionId")
		schedule.Action.InputDigest, _ = jsonx.String(action, "inputDigest")
		schedule.Action.NominalTimeField, _ = jsonx.String(action, "nominalTimeField")
		schedule.Action.RunTimeoutMs = optionalInt(action, "runTimeoutMs")
		schedule.Action.BusinessIDPrefix, _ = jsonx.String(action, "businessIdPrefix")
		schedule.Action.BusinessIDConflictPolicy, _ = jsonx.String(action, "businessIdConflictPolicy")
		schedule.Action.BusinessIDReusePolicy, _ = jsonx.String(action, "businessIdReusePolicy")
	}
	schedule.Note, _ = jsonx.String(raw, "note")
	schedule.NoteBy, _ = jsonx.String(raw, "noteBy")
	schedule.RemainingActions = optionalInt(raw, "remainingActions")
	schedule.NextActionAt, _ = jsonx.Time(raw, "nextActionAt")
	if schedule.CreatedAt, err = requiredTime(raw, "createdAt"); err != nil {
		return nil, err
	}
	schedule.UpdatedAt, _ = jsonx.Time(raw, "updatedAt")
	return schedule, nil
}

func actionRecordOf(raw map[string]json.RawMessage) (*ScheduleActionRecord, error) {
	var err error
	record := &ScheduleActionRecord{Raw: raw}
	if record.NominalTime, err = requiredTime(raw, "nominalTime"); err != nil {
		return nil, err
	}
	record.ActualTime, _ = jsonx.Time(raw, "actualTime")
	if record.Kind, err = requiredString(raw, "kind"); err != nil {
		return nil, err
	}
	record.Origin, _ = jsonx.String(raw, "origin")
	if record.Revision, err = requiredRevision(raw, "revision", 1); err != nil {
		return nil, err
	}
	if record.Outcome, err = requiredString(raw, "outcome"); err != nil {
		return nil, err
	}
	record.Reason, _ = jsonx.String(raw, "reason")
	record.RunID, _ = jsonx.String(raw, "runId")
	record.BackfillID, _ = jsonx.String(raw, "backfillId")
	record.SummaryCount = optionalInt(raw, "summaryCount")
	record.SummaryLastNominalTime, _ = jsonx.Time(raw, "summaryLastNominalTime")
	if record.DecidedAt, err = requiredTime(raw, "decidedAt"); err != nil {
		return nil, err
	}
	return record, nil
}

func backfillOf(raw map[string]json.RawMessage) (ScheduleBackfill, error) {
	var err error
	backfill := ScheduleBackfill{Raw: raw}
	if backfill.BackfillID, err = requiredString(raw, "backfillId"); err != nil {
		return ScheduleBackfill{}, err
	}
	if backfill.Start, err = requiredTime(raw, "start"); err != nil {
		return ScheduleBackfill{}, err
	}
	if backfill.End, err = requiredTime(raw, "end"); err != nil {
		return ScheduleBackfill{}, err
	}
	backfill.Cursor, _ = jsonx.Time(raw, "cursor")
	if backfill.State, err = requiredString(raw, "state"); err != nil {
		return ScheduleBackfill{}, err
	}
	if backfill.Overlap, err = requiredString(raw, "overlap"); err != nil {
		return ScheduleBackfill{}, err
	}
	backfill.Counts = counters(raw["counts"])
	return backfill, nil
}
