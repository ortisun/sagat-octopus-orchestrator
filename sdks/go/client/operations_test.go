package client_test

// The client surface of feature 018 against a recording server: what each call sends, what it keeps across a
// retry, what it refuses before sending, and how it reads an answer that is newer than the SDK. Each test has a
// counterpart in the TypeScript SDK (sdks/typescript/test/operations.test.ts).

import (
	"context"
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
	"net/url"
	"reflect"
	"strings"
	"testing"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
)

const (
	nsID       = "0190aaaa-0000-7000-8000-00000000aaaa"
	scheduleID = "0190aaaa-0000-7000-8000-0000000000a1"
	batchID    = "0190aaaa-0000-7000-8000-0000000000b1"
)

func scheduleAnswer(overrides map[string]any) map[string]any {
	m := map[string]any{
		"scheduleId": scheduleID, "name": "nightly", "origin": "API", "state": "ACTIVE", "revision": 2, "createdAt": "2026-10-10T10:00:00Z",
		"action": map[string]any{"workflowId": workflowID, "versionPolicy": "ACTIVE", "inputDigest": strings.Repeat("a", 64)}, "note": nil,
		"remainingActions": nil, "nextActionAt": "2099-01-01T09:00:00Z", "updatedAt": "2026-10-10T10:00:00Z",
	}
	for k, v := range overrides {
		m[k] = v
	}
	return m
}

func actionAnswer(overrides map[string]any) map[string]any {
	m := map[string]any{
		"nominalTime": "2026-10-10T10:00:00Z", "actualTime": "2026-10-10T10:00:01Z", "kind": "TRIGGERED", "origin": "MANUAL", "revision": 2,
		"outcome": "STARTED", "reason": nil, "runId": runUUID, "backfillId": nil, "decidedAt": "2026-10-10T10:00:01Z",
	}
	for k, v := range overrides {
		m[k] = v
	}
	return m
}

func unavailable() map[string]any {
	return map[string]any{"code": "UNAVAILABLE", "message": "busy", "retryable": true, "traceId": strings.Repeat("e", 32)}
}

// body checks a request body against the JSON it must be, member by member: a member that is left out must
// be absent, not null.
func sameBody(t *testing.T, r seen, want string) {
	t.Helper()
	var got, expected any
	if err := json.Unmarshal(r.body, &got); err != nil {
		t.Fatalf("the body is not JSON: %v: %s", err, r.body)
	}
	if err := json.Unmarshal([]byte(want), &expected); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(got, expected) {
		t.Errorf("body\n got  %s\n want %s", r.body, want)
	}
}

func only(t *testing.T, f *fake) seen {
	t.Helper()
	requests := f.requests()
	if len(requests) != 1 {
		t.Fatalf("%d requests, want exactly one", len(requests))
	}
	return requests[0]
}

func queryOf(t *testing.T, r seen) url.Values {
	t.Helper()
	u, err := url.Parse(r.url)
	if err != nil {
		t.Fatal(err)
	}
	return u.Query()
}

func pathOf(t *testing.T, r seen) string {
	t.Helper()
	u, err := url.Parse(r.url)
	if err != nil {
		t.Fatal(err)
	}
	return u.EscapedPath()
}

func wantCode(t *testing.T, err error, code sagat.Code) *sagat.Error {
	t.Helper()
	se := asSagat(t, err)
	if se.Code != code {
		t.Errorf("code %s, want %s: %v", se.Code, code, se)
	}
	return se
}

// ---- start options

func TestAStartSendsTheBusinessIDItsPoliciesAndTheDelayOnceAndSaysWhetherItCreatedTheRun(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 202, run(map[string]any{"businessId": "order-1", "startDelayedUntil": "2026-10-10T10:00:05Z"}))
	})
	c := newClient(t, f)
	started, err := c.Start(context.Background(), workflowID, map[string]any{"a": 1},
		client.WithIdempotencyKey("key-12345678"), client.WithBusinessID("Order/1 "),
		client.WithBusinessIDConflictPolicy(client.BusinessIDUseExisting), client.WithBusinessIDReusePolicy(client.BusinessIDRejectDuplicate),
		client.WithStartDelay(5000900*time.Microsecond))
	if err != nil {
		t.Fatal(err)
	}
	sameBody(t, only(t, f), `{"workflowId":"`+workflowID+`","input":{"a":1},"businessId":"Order/1 ","businessIdConflictPolicy":"USE_EXISTING","businessIdReusePolicy":"REJECT_DUPLICATE","startDelayMs":5000}`)
	if !started.Started || started.BusinessID != "order-1" || !started.StartDelayedUntil.Equal(time.Date(2026, 10, 10, 10, 0, 5, 0, time.UTC)) {
		t.Errorf("%+v", started)
	}
	if started.RunID != runUUID {
		t.Errorf("the run is reachable through the answer: %q", started.RunID)
	}
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	plain, err := c.Start(context.Background(), workflowID, nil, client.WithIdempotencyKey("key-12345678"))
	if err != nil {
		t.Fatal(err)
	}
	if plain.BusinessID != "" || !plain.StartDelayedUntil.IsZero() {
		t.Errorf("a run with no business identifier and no delay has neither: %+v", plain.Run)
	}
}

// ---- where a run came from

func TestARunSaysWhichScheduleStartedItByWhichKindOfActionForWhichNominalTimeAndUnderWhichRevisionAndARunNoScheduleStartedHasNone(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, run(map[string]any{
			"startedBy": "system:schedule:" + scheduleID,
			"schedule":  map[string]any{"scheduleId": scheduleID, "nominalTime": "2026-11-04T02:00:00.123456Z", "kind": "BACKFILL", "revision": 3},
		}))
	})
	c := newClient(t, f)
	scheduled, err := c.Get(context.Background(), runUUID)
	if err != nil {
		t.Fatal(err)
	}
	if scheduled.Schedule == nil {
		t.Fatalf("the run has a schedule: %+v", scheduled)
	}
	// A fixed literal, so that an instant that was carried and not parsed shows: the microseconds are kept (a time.Time
	// holds nanoseconds) and the instant is in UTC.
	wantTime := time.Date(2026, 11, 4, 2, 0, 0, 123456000, time.UTC)
	if *scheduled.Schedule != (client.RunSchedule{ScheduleID: scheduleID, NominalTime: wantTime, Kind: "BACKFILL", Revision: 3}) {
		t.Errorf("%+v", *scheduled.Schedule)
	}
	if got := scheduled.Schedule.NominalTime; !got.Equal(wantTime) || got.Nanosecond() != 123456000 || got.Location() != time.UTC || got.Unix() != 1793757600 {
		t.Errorf("the nominal time is a parsed instant in UTC, to the microsecond: %v (unix %d)", got, got.Unix())
	}
	var text struct {
		NominalTime string `json:"nominalTime"`
	}
	if err := json.Unmarshal(scheduled.Raw["schedule"], &text); err != nil || text.NominalTime != "2026-11-04T02:00:00.123456Z" {
		t.Errorf("the exact text stays in Raw: %q (%v)", text.NominalTime, err)
	}
	if scheduled.ResetFrom != nil {
		t.Errorf("a run a schedule started was not made by a reset: %+v", *scheduled.ResetFrom)
	}

	// An offset is an instant too, and a kind this SDK does not know is kept as the server sent it.
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, run(map[string]any{"schedule": map[string]any{"scheduleId": scheduleID, "nominalTime": "2026-11-04T04:00:00+02:00", "kind": "SOMETHING_NEW", "revision": 1}}))
	})
	unknown, err := c.Get(context.Background(), runUUID)
	if err != nil {
		t.Fatal(err)
	}
	if unknown.Schedule == nil || unknown.Schedule.Kind != "SOMETHING_NEW" || !unknown.Schedule.NominalTime.Equal(time.Date(2026, 11, 4, 2, 0, 0, 0, time.UTC)) {
		t.Errorf("%+v", unknown.Schedule)
	}

	// A plain run, and a run whose members are null, have neither.
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	plain, err := c.Get(context.Background(), runUUID)
	if err != nil {
		t.Fatal(err)
	}
	if plain.Schedule != nil || plain.ResetFrom != nil {
		t.Errorf("a run that no schedule started and no reset made has neither: %+v %+v", plain.Schedule, plain.ResetFrom)
	}
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, run(map[string]any{"schedule": nil, "resetFrom": nil}))
	})
	nulls, err := c.Get(context.Background(), runUUID)
	if err != nil {
		t.Fatal(err)
	}
	if nulls.Schedule != nil || nulls.ResetFrom != nil {
		t.Errorf("a null is absent: %+v %+v", nulls.Schedule, nulls.ResetFrom)
	}
}

func TestARunAResetMadeSaysTheRunItCameFromAndTheJournalSequenceItKeeps(t *testing.T) {
	const source = "0190aaaa-0000-7000-8000-0000000000aa"
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, run(map[string]any{"rerunOf": source, "resetFrom": map[string]any{"runId": source, "sequence": 4}}))
	})
	c := newClient(t, f)
	reset, err := c.Get(context.Background(), runUUID)
	if err != nil {
		t.Fatal(err)
	}
	if reset.ResetFrom == nil || *reset.ResetFrom != (client.RunResetFrom{RunID: source, Sequence: 4}) {
		t.Errorf("%+v", reset.ResetFrom)
	}
	if reset.RerunOf != source || reset.Schedule != nil {
		t.Errorf("%+v", reset)
	}
	// started again, not reset: RerunOf and nothing else
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(map[string]any{"rerunOf": source})) })
	again, err := c.Get(context.Background(), runUUID)
	if err != nil {
		t.Fatal(err)
	}
	if again.RerunOf != source || again.ResetFrom != nil {
		t.Errorf("a rerun is not a reset: %+v", again.ResetFrom)
	}
}

func TestAnAnswerWhoseScheduleOrResetIsNotWhatTheContractSaysIsAProtocolErrorNotARunWithHalfOfIt(t *testing.T) {
	f := newFake(t)
	c := newClient(t, f)
	for _, broken := range []map[string]any{
		{"schedule": map[string]any{"scheduleId": scheduleID, "nominalTime": "yesterday", "kind": "SCHEDULED", "revision": 1}},
		{"schedule": map[string]any{"scheduleId": scheduleID, "kind": "SCHEDULED", "revision": 1}},
		{"schedule": map[string]any{"scheduleId": scheduleID, "nominalTime": "2026-11-04T02:00:00Z", "kind": "SCHEDULED", "revision": 0}},
		{"schedule": map[string]any{"scheduleId": scheduleID, "nominalTime": "2026-11-04T02:00:00Z", "kind": "SCHEDULED"}},
		{"schedule": map[string]any{"nominalTime": "2026-11-04T02:00:00Z", "kind": "SCHEDULED", "revision": 1}},
		{"schedule": map[string]any{"scheduleId": scheduleID, "nominalTime": "2026-11-04T02:00:00Z", "revision": 1}},
		{"schedule": "nightly"},
		{"schedule": []any{}},
		{"resetFrom": map[string]any{"runId": runUUID}},
		{"resetFrom": map[string]any{"runId": runUUID, "sequence": 0}},
		{"resetFrom": map[string]any{"sequence": 4}},
		{"resetFrom": "0190aaaa-0000-7000-8000-0000000000aa"},
	} {
		f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(broken)) })
		got, err := c.Get(context.Background(), runUUID)
		if err == nil {
			t.Errorf("%v: a run with half of it: %+v", broken, got)
			continue
		}
		if se := asSagat(t, err); se.Code != sagat.CodeProtocol {
			t.Errorf("%v: code %s, want %s", broken, se.Code, sagat.CodeProtocol)
		}
	}
}

func TestThe200OfUseExistingIsARunThatWasNotStartedHereAndAStartWithoutTheNewOptionsSendsNoneOfThem(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, run(nil)) })
	c := newClient(t, f)
	existing, err := c.Start(context.Background(), workflowID, nil, client.WithIdempotencyKey("key-12345678"),
		client.WithBusinessID("o-1"), client.WithBusinessIDConflictPolicy(client.BusinessIDUseExisting))
	if err != nil {
		t.Fatal(err)
	}
	if existing.Started {
		t.Error("an open run held the identifier: nothing was started")
	}
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	if _, err := c.Start(context.Background(), workflowID, nil, client.WithIdempotencyKey("key-12345678")); err != nil {
		t.Fatal(err)
	}
	if got := only(t, f).body; string(got) != `{"input":{},"workflowId":"`+workflowID+`"}` {
		t.Errorf("an old call is byte for byte what it was: %s", got)
	}
}

func TestAStartDelayIsWholeMillisecondsTruncatedTowardZeroAGivenZeroIsSentAndARetryKeepsTheOneItComputed(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
	c := newClient(t, f)
	for delay, want := range map[time.Duration]int64{
		1500 * time.Microsecond: 1, 2999999 * time.Nanosecond: 2, -1500 * time.Microsecond: -1, -999 * time.Microsecond: 0, 0: 0,
		366 * 24 * time.Hour: 31622400000,
	} {
		f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, run(nil)) })
		if _, err := c.Start(context.Background(), workflowID, nil, client.WithIdempotencyKey("key-12345678"), client.WithStartDelay(delay)); err != nil {
			t.Fatal(err)
		}
		sent, present := only(t, f).json(t)["startDelayMs"].(float64)
		if !present || int64(sent) != want {
			t.Errorf("%v is sent as %v, want %d", delay, sent, want)
		}
	}
	attempts := 0
	f.answer(func(_ seen, w http.ResponseWriter) {
		attempts++
		if attempts < 2 {
			reply(w, 503, unavailable())
			return
		}
		reply(w, 202, run(nil))
	})
	if _, err := c.Start(context.Background(), workflowID, nil, client.WithBusinessID("o-2"), client.WithStartDelay(1500*time.Millisecond)); err != nil {
		t.Fatal(err)
	}
	requests := f.requests()
	if len(requests) != 2 {
		t.Fatalf("%d attempts", len(requests))
	}
	if string(requests[0].body) != string(requests[1].body) || requests[0].headers.Get("Idempotency-Key") != requests[1].headers.Get("Idempotency-Key") {
		t.Error("the retry is the same body under the same key")
	}
}

// ---- schedules

func TestCreateSendsTheScheduleWithItsInstantsInUTCAndOneKeyAndReadsTheScheduleItGetsBack(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 201, scheduleAnswer(map[string]any{"futureField": 1})) })
	c := newClient(t, f)
	created, err := c.Schedules.Create(context.Background(), nsID, client.ScheduleWrite{
		Name: "nightly",
		Spec: client.ScheduleSpec{
			Calendars: []client.ScheduleCalendarSpec{{Hour: []client.ScheduleRange{{Start: 9}}, Year: []client.ScheduleRange{{Start: 2099}}}},
			StartAt:   time.Date(2099, 1, 1, 1, 0, 0, 0, time.FixedZone("CET", 3600)), Timezone: "UTC",
		},
		Policies: &client.SchedulePolicies{Overlap: client.OverlapSkip},
		Action:   client.ScheduleAction{WorkflowID: workflowID, Input: map[string]any{"n": 1}},
	}, client.WithIdempotencyKey("sched-key-1"))
	if err != nil {
		t.Fatal(err)
	}
	r := only(t, f)
	if r.method != "POST" || pathOf(t, r) != "/v1/namespaces/"+nsID+"/schedules" || r.headers.Get("Idempotency-Key") != "sched-key-1" {
		t.Errorf("%s %s %v", r.method, r.url, r.headers)
	}
	sameBody(t, r, `{"name":"nightly","spec":{"calendars":[{"hour":[{"start":9}],"year":[{"start":2099}]}],"startAt":"2099-01-01T00:00:00.000Z","timezone":"UTC"},`+
		`"policies":{"overlap":"SKIP"},"action":{"workflowId":"`+workflowID+`","input":{"n":1}}}`)
	if created.Revision != 2 || !created.NextActionAt.Equal(time.Date(2099, 1, 1, 9, 0, 0, 0, time.UTC)) || created.NextActionAt.Location() != time.UTC {
		t.Errorf("%+v", created)
	}
	if created.Action.InputDigest != strings.Repeat("a", 64) {
		t.Error("the input is never returned, only its digest")
	}
	if string(created.Raw["futureField"]) != "1" {
		t.Error("a field of a newer server is reachable")
	}
	if created.Note != "" || created.RemainingActions != nil {
		t.Errorf("null is the zero value: %+v", created)
	}
}

func TestADescriptionAsksForTheCountsItWasGivenAndKeepsAnOutcomeItDoesNotKnowAsTheStringItCameAs(t *testing.T) {
	f := newFake(t)
	answer := scheduleAnswer(map[string]any{
		"nextActions":   []any{map[string]any{"nominalTime": "2099-01-01T09:00:00Z", "actualTime": "2099-01-01T09:00:00Z"}},
		"recentActions": []any{actionAnswer(nil), actionAnswer(map[string]any{"outcome": "SOMETHING_NEWER"})},
		"openRuns":      []any{runUUID}, "bufferedCount": 1, "counters": map[string]any{"STARTED": 1, "SOMETHING_NEWER": 1},
	})
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, answer) })
	c := newClient(t, f)
	described, err := c.Schedules.Get(context.Background(), nsID, scheduleID, client.WithNextCount(3), client.WithRecentCount(5))
	if err != nil {
		t.Fatal(err)
	}
	r := only(t, f)
	if r.method != "GET" || pathOf(t, r) != "/v1/namespaces/"+nsID+"/schedules/"+scheduleID || queryOf(t, r).Encode() != "nextCount=3&recentCount=5" {
		t.Errorf("%s %s", r.method, r.url)
	}
	if got := described.NextActions[0].NominalTime.UnixMilli(); got != time.Date(2099, 1, 1, 9, 0, 0, 0, time.UTC).UnixMilli() {
		t.Errorf("next time %d", got)
	}
	if described.RecentActions[1].Outcome != "SOMETHING_NEWER" || described.OpenRuns[0] != runUUID || described.BufferedCount != 1 ||
		!reflect.DeepEqual(described.Counters, map[string]int64{"STARTED": 1, "SOMETHING_NEWER": 1}) {
		t.Errorf("%+v", described)
	}
	// An asked-for zero is sent, and a count not asked for is not.
	if _, err := c.Schedules.Get(context.Background(), nsID, scheduleID, client.WithNextCount(0), client.WithRecentCount(0)); err != nil {
		t.Fatal(err)
	}
	if got := queryOf(t, f.requests()[1]).Encode(); got != "nextCount=0&recentCount=0" {
		t.Errorf("query %q", got)
	}
	if _, err := c.Schedules.Get(context.Background(), nsID, scheduleID); err != nil {
		t.Fatal(err)
	}
	if got := f.requests()[2].url; strings.Contains(got, "?") {
		t.Errorf("no counts asked: %s", got)
	}
}

func TestListSendsTheFiltersItWasGivenAndListAllFollowsTheOpaqueCursorsToTheEnd(t *testing.T) {
	f := newFake(t)
	page := 0
	f.answer(func(_ seen, w http.ResponseWriter) {
		page++
		if page == 1 {
			reply(w, 200, map[string]any{"items": []any{scheduleAnswer(map[string]any{"name": "a-1"})}, "nextCursor": "c1"})
			return
		}
		reply(w, 200, map[string]any{"items": []any{scheduleAnswer(map[string]any{"name": "a-2"})}, "nextCursor": nil})
	})
	c := newClient(t, f)
	var names []string
	for s, err := range c.Schedules.ListAll(context.Background(), nsID, client.ScheduleListFilter{State: "PAUSED", Origin: "API", Name: "a-", Limit: 1}) {
		if err != nil {
			t.Fatal(err)
		}
		names = append(names, s.Name)
	}
	if !reflect.DeepEqual(names, []string{"a-1", "a-2"}) {
		t.Errorf("%v", names)
	}
	requests := f.requests()
	if len(requests) != 2 {
		t.Fatalf("%d requests", len(requests))
	}
	if got := queryOf(t, requests[0]).Encode(); got != "limit=1&name=a-&origin=API&state=PAUSED" {
		t.Errorf("first page: %s", got)
	}
	if got := queryOf(t, requests[1]).Encode(); got != "cursor=c1&limit=1&name=a-&origin=API&state=PAUSED" {
		t.Errorf("second page: %s", got)
	}
	// A filter by workflow takes a UUID, and a page whose answer has no items is refused.
	if _, err := c.Schedules.List(context.Background(), nsID, client.ScheduleListFilter{WorkflowID: "not-a-uuid"}); err != nil {
		wantCode(t, err, sagat.CodeValidationFailed)
	} else {
		t.Error("a workflow id that is not a UUID is refused")
	}
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, map[string]any{"nextCursor": nil}) })
	_, err := c.Schedules.List(context.Background(), nsID, client.ScheduleListFilter{})
	wantCode(t, err, sagat.CodeProtocol)
}

func TestUpdateCarriesTheRevisionItExpectsInTheBodyDeleteInTheQueryAndBothCarryAKey(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, scheduleAnswer(map[string]any{"revision": 3})) })
	c := newClient(t, f)
	write := client.ScheduleWrite{Name: "nightly", Spec: client.ScheduleSpec{Cron: []string{"0 9 * * *"}}, Action: client.ScheduleAction{WorkflowID: workflowID}}
	updated, err := c.Schedules.Update(context.Background(), nsID, scheduleID, write, 2, client.WithIdempotencyKey("upd-key-0001"))
	if err != nil {
		t.Fatal(err)
	}
	if updated.Revision != 3 {
		t.Errorf("%+v", updated)
	}
	r := only(t, f)
	if r.method != "PUT" || r.headers.Get("Idempotency-Key") != "upd-key-0001" {
		t.Errorf("%s %v", r.method, r.headers)
	}
	sameBody(t, r, `{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":{"workflowId":"`+workflowID+`","input":{}},"expectedRevision":2}`)

	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 204, nil) })
	if err := c.Schedules.Delete(context.Background(), nsID, scheduleID, 3, client.WithIdempotencyKey("del-key-0001")); err != nil {
		t.Fatal(err)
	}
	r = only(t, f)
	if r.method != "DELETE" || r.url != "/v1/namespaces/"+nsID+"/schedules/"+scheduleID+"?expectedRevision=3" || r.headers.Get("Idempotency-Key") != "del-key-0001" || len(r.body) != 0 {
		t.Errorf("%s %s %v %q", r.method, r.url, r.headers, r.body)
	}
}

// ---- the business-id prefix of a schedule's action (FR-018-016)

func TestAScheduleActionWithABusinessIDPrefixSendsTheThreeFieldsAsGivenOnCreateAndOnUpdateAndReadsThemBack(t *testing.T) {
	prefix := " Order/{x}-é+%20 " // customer text: no trimming, no case folding, no local pattern or length check
	f := newFake(t)
	answer := scheduleAnswer(map[string]any{"action": map[string]any{
		"workflowId": workflowID, "versionPolicy": "ACTIVE", "inputDigest": strings.Repeat("a", 64),
		"businessIdPrefix": prefix, "businessIdConflictPolicy": "USE_EXISTING", "businessIdReusePolicy": "REJECT_DUPLICATE",
	}})
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 201, answer) })
	c := newClient(t, f)
	write := client.ScheduleWrite{
		Name: "nightly", Spec: client.ScheduleSpec{Cron: []string{"0 9 * * *"}},
		Action: client.ScheduleAction{
			WorkflowID: workflowID, BusinessIDPrefix: &prefix,
			BusinessIDConflictPolicy: client.BusinessIDUseExisting, BusinessIDReusePolicy: client.BusinessIDRejectDuplicate,
		},
	}
	created, err := c.Schedules.Create(context.Background(), nsID, write, client.WithIdempotencyKey("sched-key-2"))
	if err != nil {
		t.Fatal(err)
	}
	wantAction := `{"workflowId":"` + workflowID + `","input":{},"businessIdPrefix":` + mustJSON(t, prefix) +
		`,"businessIdConflictPolicy":"USE_EXISTING","businessIdReusePolicy":"REJECT_DUPLICATE"}`
	sameBody(t, only(t, f), `{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":`+wantAction+`}`)
	if a := created.Action; a.BusinessIDPrefix != prefix || a.BusinessIDConflictPolicy != "USE_EXISTING" || a.BusinessIDReusePolicy != "REJECT_DUPLICATE" {
		t.Errorf("%+v", a)
	}
	// An update takes the same action document.
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, answer) })
	updated, err := c.Schedules.Update(context.Background(), nsID, scheduleID, write, 2, client.WithIdempotencyKey("upd-key-0002"))
	if err != nil {
		t.Fatal(err)
	}
	sameBody(t, only(t, f), `{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":`+wantAction+`,"expectedRevision":2}`)
	if updated.Action.BusinessIDPrefix != prefix {
		t.Errorf("%+v", updated.Action)
	}
}

func TestAScheduleActionThatNamesNoPrefixSendsNoneOfTheThreeFieldsAndReadsNoneBack(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 201, scheduleAnswer(nil)) })
	c := newClient(t, f)
	write := client.ScheduleWrite{Name: "nightly", Spec: client.ScheduleSpec{Cron: []string{"0 9 * * *"}}, Action: client.ScheduleAction{WorkflowID: workflowID}}
	created, err := c.Schedules.Create(context.Background(), nsID, write, client.WithIdempotencyKey("sched-key-3"))
	if err != nil {
		t.Fatal(err)
	}
	// sameBody compares member by member: a key that is left out must be absent, never null and never empty.
	sameBody(t, only(t, f), `{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":{"workflowId":"`+workflowID+`","input":{}}}`)
	if a := created.Action; a.BusinessIDPrefix != "" || a.BusinessIDConflictPolicy != "" || a.BusinessIDReusePolicy != "" {
		t.Errorf("an action with no prefix has none of the three: %+v", a)
	}
	// The server writes null for what it has not: the same as absent.
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, scheduleAnswer(map[string]any{"action": map[string]any{
			"workflowId": workflowID, "inputDigest": strings.Repeat("a", 64), "businessIdPrefix": nil, "businessIdConflictPolicy": nil, "businessIdReusePolicy": nil,
		}}))
	})
	updated, err := c.Schedules.Update(context.Background(), nsID, scheduleID, write, 2, client.WithIdempotencyKey("upd-key-0003"))
	if err != nil {
		t.Fatal(err)
	}
	sameBody(t, only(t, f), `{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":{"workflowId":"`+workflowID+`","input":{}},"expectedRevision":2}`)
	if a := updated.Action; a.BusinessIDPrefix != "" || a.BusinessIDConflictPolicy != "" || a.BusinessIDReusePolicy != "" {
		t.Errorf("null is absent: %+v", a)
	}
}

func TestAnEmptyPrefixIsSentAsTheEmptyStringAndAPolicyWithoutAPrefixIsSentAsGivenForTheServerToJudge(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 400, map[string]any{
			"code": "VALIDATION_FAILED", "message": "the schedule is not valid", "retryable": false, "traceId": strings.Repeat("e", 32),
			"details": map[string]any{"reason": "SCHEDULE_SPEC_INVALID", "field": "/action/businessIdPrefix"},
		})
	})
	c := newClient(t, f)
	empty := ""
	write := client.ScheduleWrite{
		Name: "nightly", Spec: client.ScheduleSpec{Cron: []string{"0 9 * * *"}},
		Action: client.ScheduleAction{WorkflowID: workflowID, BusinessIDPrefix: &empty},
	}
	_, err := c.Schedules.Create(context.Background(), nsID, write, client.WithIdempotencyKey("sched-key-4"))
	if se := wantCode(t, err, sagat.CodeValidationFailed); se.Reason != "SCHEDULE_SPEC_INVALID" || se.HTTPStatus != 400 {
		t.Errorf("the server's refusal is read as it is: %+v", se)
	}
	sameBody(t, only(t, f), `{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":{"workflowId":"`+workflowID+`","input":{},"businessIdPrefix":""}}`)
	// A policy with no prefix is the server's to refuse, not the SDK's.
	write.Action = client.ScheduleAction{WorkflowID: workflowID, BusinessIDReusePolicy: client.BusinessIDAllowDuplicateFailedOnly}
	_, _ = c.Schedules.Create(context.Background(), nsID, write, client.WithIdempotencyKey("sched-key-5"))
	sameBody(t, f.requests()[1], `{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":{"workflowId":"`+workflowID+`","input":{},"businessIdReusePolicy":"ALLOW_DUPLICATE_FAILED_ONLY"}}`)
}

func TestAPolicyOfTheActionThatThisSDKDoesNotKnowIsKeptAsTheStringTheServerSentNotAnError(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, scheduleAnswer(map[string]any{"action": map[string]any{
			"workflowId": workflowID, "inputDigest": strings.Repeat("a", 64),
			"businessIdPrefix": "p", "businessIdConflictPolicy": "REPLACE", "businessIdReusePolicy": "SOMETHING_NEWER",
		}}))
	})
	c := newClient(t, f)
	described, err := c.Schedules.Get(context.Background(), nsID, scheduleID)
	if err != nil {
		t.Fatal(err)
	}
	if a := described.Action; a.BusinessIDPrefix != "p" || a.BusinessIDConflictPolicy != "REPLACE" || a.BusinessIDReusePolicy != "SOMETHING_NEWER" {
		t.Errorf("%+v", a)
	}
}

func TestTheOutcomesAndReasonsOfABusinessIDConflictAreKeptAsTheStringsTheServerSent(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, scheduleAnswer(map[string]any{"recentActions": []any{
			actionAnswer(map[string]any{"outcome": "BUSINESS_ID_CONFLICT", "reason": "BUSINESS_ID_IN_USE", "runId": nil}),
			actionAnswer(map[string]any{"outcome": "BUSINESS_ID_CONFLICT", "reason": "BUSINESS_ID_REUSE_REJECTED", "runId": nil}),
			actionAnswer(map[string]any{"outcome": "ALREADY_ACTED", "reason": "BUSINESS_ID_IN_USE"}),
		}}))
	})
	c := newClient(t, f)
	described, err := c.Schedules.Get(context.Background(), nsID, scheduleID)
	if err != nil {
		t.Fatal(err)
	}
	type pair struct{ outcome, reason, runID string }
	var got []pair
	for _, a := range described.RecentActions {
		got = append(got, pair{a.Outcome, a.Reason, a.RunID})
	}
	want := []pair{{"BUSINESS_ID_CONFLICT", "BUSINESS_ID_IN_USE", ""}, {"BUSINESS_ID_CONFLICT", "BUSINESS_ID_REUSE_REJECTED", ""}, {"ALREADY_ACTED", "BUSINESS_ID_IN_USE", runUUID}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got %+v want %+v", got, want)
	}
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"items": []any{actionAnswer(map[string]any{"outcome": "BUSINESS_ID_CONFLICT", "reason": "BUSINESS_ID_IN_USE", "runId": nil})}, "nextCursor": nil})
	})
	page, err := c.Schedules.ListActions(context.Background(), nsID, scheduleID, client.ScheduleActionFilter{Outcome: "BUSINESS_ID_CONFLICT"})
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Items) != 1 || page.Items[0].Outcome != "BUSINESS_ID_CONFLICT" || page.Items[0].Reason != "BUSINESS_ID_IN_USE" {
		t.Errorf("%+v", page.Items)
	}
	if got := queryOf(t, only(t, f)).Get("outcome"); got != "BUSINESS_ID_CONFLICT" {
		t.Errorf("an outcome the SDK has no constant for is still a filter: %q", got)
	}
}

// The prefix is customer text: the SDK's own log lines and its errors never carry it (sdks/SPEC.md section 10).
func TestThePrefixOfAnActionNeverReachesALogLineOrAnErrorTheSDKWrites(t *testing.T) {
	const prefix = "SECRET-PREFIX-4711"
	var logs strings.Builder
	logger := slog.New(slog.NewTextHandler(&logs, &slog.HandlerOptions{Level: slog.LevelDebug}))
	f := newFake(t)
	attempts := 0
	f.answer(func(_ seen, w http.ResponseWriter) {
		attempts++
		switch attempts {
		case 1:
			reply(w, 503, "down") // makes the client log that it retries
		case 2:
			reply(w, 400, map[string]any{"code": "VALIDATION_FAILED", "message": "the schedule is not valid", "retryable": false, "details": map[string]any{"reason": "SCHEDULE_SPEC_INVALID", "field": "/action/businessIdPrefix"}})
		default:
			reply(w, 201, scheduleAnswer(nil))
		}
	})
	c := newClient(t, f, client.WithLogger(logger))
	p := prefix
	write := client.ScheduleWrite{Name: "nightly", Spec: client.ScheduleSpec{Cron: []string{"0 9 * * *"}}, Action: client.ScheduleAction{WorkflowID: workflowID, BusinessIDPrefix: &p}}
	_, err := c.Schedules.Create(context.Background(), nsID, write, client.WithIdempotencyKey("sched-key-6"))
	if se := wantCode(t, err, sagat.CodeValidationFailed); strings.Contains(se.Error(), prefix) || strings.Contains(fmt.Sprintf("%+v", se), prefix) {
		t.Errorf("the error carries the prefix: %v", se)
	}
	if logs.Len() == 0 {
		t.Error("the retry was not logged, so the test proves nothing")
	}
	if strings.Contains(logs.String(), prefix) {
		t.Errorf("a log line carries the prefix:\n%s", logs.String())
	}
	// Printing the write does not show the text of the prefix (a pointer prints as an address).
	if strings.Contains(fmt.Sprintf("%v %+v", write, write.Action), prefix) {
		t.Error("printing the action shows the prefix")
	}
}

func mustJSON(t *testing.T, v any) string {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

func TestAStaleRevisionIsAConflictWithItsReasonAndARevisionThatCannotBeOneIsRefusedBeforeItIsSent(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 409, map[string]any{"code": "CONFLICT", "message": "stale", "retryable": false, "traceId": strings.Repeat("e", 32), "details": map[string]any{"reason": "REVISION_CONFLICT", "currentRevision": 4}})
	})
	c := newClient(t, f)
	write := client.ScheduleWrite{Name: "n", Spec: client.ScheduleSpec{Cron: []string{"0 9 * * *"}}, Action: client.ScheduleAction{WorkflowID: workflowID}}
	_, err := c.Schedules.Update(context.Background(), nsID, scheduleID, write, 2)
	if se := wantCode(t, err, sagat.CodeConflict); se.Reason != "REVISION_CONFLICT" {
		t.Errorf("reason %q", se.Reason)
	}
	if got := len(f.requests()); got != 1 {
		t.Errorf("a conflict is never retried: %d requests", got)
	}
	f.answer(f.handler)
	ctx := context.Background()
	wantCode(t, c.Schedules.Delete(ctx, nsID, scheduleID, 0), sagat.CodeValidationFailed)
	_, err = c.Schedules.Update(ctx, nsID, scheduleID, write, -1)
	wantCode(t, err, sagat.CodeValidationFailed)
	_, err = c.Schedules.Get(ctx, nsID, "../x")
	wantCode(t, err, sagat.CodeValidationFailed)
	_, err = c.Schedules.Get(ctx, "payments", scheduleID)
	wantCode(t, err, sagat.CodeValidationFailed)
	if got := len(f.requests()); got != 0 {
		t.Errorf("a local refusal sends nothing: %d requests", got)
	}
}

func TestPauseAndResumeSendTheNoteTriggerSendsTheOverlapForThisActionOnlyAndATriggerRetryKeepsItsKey(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, scheduleAnswer(map[string]any{"state": "PAUSED", "note": "maintenance"}))
	})
	c := newClient(t, f)
	ctx := context.Background()
	paused, err := c.Schedules.Pause(ctx, nsID, scheduleID, "maintenance", client.WithIdempotencyKey("pause-key-01"))
	if err != nil {
		t.Fatal(err)
	}
	r := only(t, f)
	if paused.State != "PAUSED" || paused.Note != "maintenance" || pathOf(t, r) != "/v1/namespaces/"+nsID+"/schedules/"+scheduleID+"/pause" {
		t.Errorf("%+v %s", paused, r.url)
	}
	sameBody(t, r, `{"note":"maintenance"}`)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 200, scheduleAnswer(nil)) })
	if _, err := c.Schedules.Resume(ctx, nsID, scheduleID, "done", client.WithIdempotencyKey("resume-key-1")); err != nil {
		t.Fatal(err)
	}
	if got := pathOf(t, only(t, f)); got != "/v1/namespaces/"+nsID+"/schedules/"+scheduleID+"/resume" {
		t.Errorf("%s", got)
	}
	f.answer(f.handler)
	_, err = c.Schedules.Pause(ctx, nsID, scheduleID, "")
	wantCode(t, err, sagat.CodeValidationFailed)
	if got := len(f.requests()); got != 0 {
		t.Errorf("an empty note sends nothing: %d requests", got)
	}

	attempts := 0
	f.answer(func(_ seen, w http.ResponseWriter) {
		attempts++
		if attempts < 2 {
			reply(w, 503, unavailable())
			return
		}
		reply(w, 202, actionAnswer(map[string]any{"outcome": "SKIPPED_OVERLAP"}))
	})
	triggered, err := c.Schedules.Trigger(ctx, nsID, scheduleID, client.WithOverlap(client.OverlapSkip))
	if err != nil {
		t.Fatal(err)
	}
	if triggered.Outcome != "SKIPPED_OVERLAP" {
		t.Errorf("%+v", triggered)
	}
	requests := f.requests()
	if len(requests) != 2 || requests[0].headers.Get("Idempotency-Key") == "" || requests[0].headers.Get("Idempotency-Key") != requests[1].headers.Get("Idempotency-Key") {
		t.Fatalf("the retry keeps the key: %d requests", len(requests))
	}
	sameBody(t, requests[1], `{"overlap":"SKIP"}`)
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, actionAnswer(nil)) })
	if _, err := c.Schedules.Trigger(ctx, nsID, scheduleID); err != nil {
		t.Fatal(err)
	}
	sameBody(t, only(t, f), `{}`)
}

func TestABackfillSendsItsRangeAsUTCInstantsACancelNamesTheBackfillAndTheHistoryCanBeFilteredByOutcome(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 202, map[string]any{"backfillId": batchID, "start": "2026-10-10T08:00:00Z", "end": "2026-10-10T10:00:00Z", "cursor": nil, "state": "PENDING", "overlap": "ALLOW_ALL", "counts": map[string]any{"STARTED": 2}})
	})
	c := newClient(t, f)
	ctx := context.Background()
	ist := time.FixedZone("IST", 5*3600+1800)
	backfill, err := c.Schedules.Backfill(ctx, nsID, scheduleID, time.Date(2026, 10, 10, 13, 30, 0, 0, ist), time.Date(2026, 10, 10, 15, 30, 0, 0, ist),
		client.WithOverlap(client.OverlapAllowAll), client.WithIdempotencyKey("back-key-001"))
	if err != nil {
		t.Fatal(err)
	}
	r := only(t, f)
	sameBody(t, r, `{"start":"2026-10-10T08:00:00.000Z","end":"2026-10-10T10:00:00.000Z","overlap":"ALLOW_ALL"}`)
	if r.headers.Get("Idempotency-Key") != "back-key-001" || !backfill.Cursor.IsZero() || !reflect.DeepEqual(backfill.Counts, map[string]int64{"STARTED": 2}) {
		t.Errorf("%v %+v", r.headers, backfill)
	}
	f.answer(f.handler)
	_, err = c.Schedules.Backfill(ctx, nsID, scheduleID, time.Time{}, time.Now())
	wantCode(t, err, sagat.CodeValidationFailed)
	if got := len(f.requests()); got != 0 {
		t.Errorf("a range that has no start sends nothing: %d requests", got)
	}

	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"backfillId": batchID, "start": "2026-10-10T08:00:00Z", "end": "2026-10-10T10:00:00Z", "state": "CANCELLED", "overlap": "ALLOW_ALL", "counts": map[string]any{}})
	})
	cancelled, err := c.Schedules.CancelBackfill(ctx, nsID, scheduleID, batchID)
	if err != nil {
		t.Fatal(err)
	}
	r = only(t, f)
	if cancelled.State != "CANCELLED" || r.method != "POST" || pathOf(t, r) != "/v1/namespaces/"+nsID+"/schedules/"+scheduleID+"/backfills/"+batchID+"/cancel" || r.headers.Get("Idempotency-Key") == "" {
		t.Errorf("%+v %s %s", cancelled, r.method, r.url)
	}
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"items": []any{actionAnswer(map[string]any{"outcome": "ALREADY_ACTED"})}, "nextCursor": nil})
	})
	actions, err := c.Schedules.ListActions(ctx, nsID, scheduleID, client.ScheduleActionFilter{Outcome: "ALREADY_ACTED", Limit: 10})
	if err != nil {
		t.Fatal(err)
	}
	r = only(t, f)
	if got := queryOf(t, r).Encode(); got != "limit=10&outcome=ALREADY_ACTED" || actions.Items[0].Outcome != "ALREADY_ACTED" || actions.NextCursor != "" {
		t.Errorf("%s %+v", got, actions)
	}
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"items": []any{map[string]any{"backfillId": batchID, "start": "2026-10-10T08:00:00Z", "end": "2026-10-10T10:00:00Z", "state": "COMPLETED", "overlap": "SKIP"}}})
	})
	backfills, err := c.Schedules.ListBackfills(ctx, nsID, scheduleID)
	if err != nil || len(backfills) != 1 || backfills[0].Counts == nil {
		t.Errorf("%v %+v", err, backfills)
	}
}

// ---- business identifiers

func TestABusinessIDIsPercentEncodedAsGivenAndOneThatCannotBeAPathSegmentIsRefusedLocally(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{
			"businessId": "Ord/1#x?y", "executionId": runUUID, "currentRunId": runUUID, "open": false, "lastStatus": "SUCCEEDED", "workflowId": workflowID,
			"boundAt": "2026-10-10T10:00:00Z", "reusableAfter": "2026-11-09T10:00:00Z",
		})
	})
	c := newClient(t, f)
	ctx := context.Background()
	binding, err := c.BusinessIDs.Get(ctx, nsID, "Ord/1#x?y")
	if err != nil {
		t.Fatal(err)
	}
	if got := only(t, f).url; got != "/v1/namespaces/"+nsID+"/business-ids/Ord%2F1%23x%3Fy" {
		t.Errorf("no trimming and no case folding: %s", got)
	}
	if binding.Open || binding.LastStatus != "SUCCEEDED" || !binding.ReusableAfter.Equal(time.Date(2026, 11, 9, 10, 0, 0, 0, time.UTC)) {
		t.Errorf("%+v", binding)
	}
	// What URL syntax means stays text; dots that are not a whole segment are sent as given.
	f.answer(f.handler)
	for given, want := range map[string]string{"...": "...", "a.b": "a.b", ".x": ".x", "A:b@c+d": "A%3Ab%40c%2Bd", "~x_y-z": "~x_y-z", "100%": "100%25"} {
		f.answer(f.handler)
		if _, err := c.BusinessIDs.Get(ctx, nsID, given); err != nil {
			t.Fatal(err)
		}
		if got := only(t, f).url; got != "/v1/namespaces/"+nsID+"/business-ids/"+want {
			t.Errorf("%q is sent as %s, want %s", given, got, want)
		}
	}
	f.answer(f.handler)
	for _, bad := range []string{"", " padded", "with space", strings.Repeat("x", 201), "ünï", "tab\t", "del\x7f", ".", ".."} {
		_, err := c.BusinessIDs.Get(ctx, nsID, bad)
		wantCode(t, err, sagat.CodeValidationFailed)
	}
	if _, err := c.BusinessIDs.Get(ctx, nsID, strings.Repeat("x", 200)); err != nil {
		t.Errorf("200 characters are allowed: %v", err)
	}
	if got := len(f.requests()); got != 1 {
		t.Errorf("a local refusal sends nothing: %d requests", got)
	}
}

// ---- batch jobs

func TestABatchIsPreviewedCreatedWithOneKeyReadCancelledAndItsItemsFilteredByState(t *testing.T) {
	job := map[string]any{
		"batchId": batchID, "action": "CANCEL", "state": "RUNNING", "createdBy": "alice", "createdAt": "2026-10-10T10:00:00Z",
		"counts": map[string]any{"selected": 2, "succeeded": 1, "skipped": 0, "failed": 0, "cancelled": 0, "deferred": 1}, "itemsPerSecond": 0.5,
	}
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"matched": 2, "capped": false, "sample": []any{runUUID}})
	})
	c := newClient(t, f)
	ctx := context.Background()
	preview, err := c.Batch.Preview(ctx, nsID, client.BatchSelector{RunIDs: []string{runUUID}}, 50)
	if err != nil {
		t.Fatal(err)
	}
	r := only(t, f)
	if pathOf(t, r) != "/v1/namespaces/"+nsID+"/batch-operations/preview" || preview.Matched != 2 || preview.Capped || !reflect.DeepEqual(preview.Sample, []string{runUUID}) {
		t.Errorf("%s %+v", r.url, preview)
	}
	sameBody(t, r, `{"selector":{"runIds":["`+runUUID+`"]},"maxItems":50}`)
	// A selector with no runs is sent as an empty list, and a preview with no cap leaves maxItems out.
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"matched": 0, "capped": false, "sample": []any{}})
	})
	if _, err := c.Batch.Preview(ctx, nsID, client.BatchSelector{RunIDs: []string{}}, 0); err != nil {
		t.Fatal(err)
	}
	sameBody(t, only(t, f), `{"selector":{"runIds":[]}}`)

	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, job) })
	created, err := c.Batch.Create(ctx, nsID, client.BatchRequest{
		Selector: client.BatchSelector{Filter: &client.BatchFilter{Status: []string{"RUNNING"}, CreatedAfter: time.Date(2026, 10, 10, 2, 0, 0, 0, time.FixedZone("CEST", 7200))}},
		Action:   "CANCEL", Reason: "clean up the day", MaxItems: 10,
	}, client.WithIdempotencyKey("batch-key-01"))
	if err != nil {
		t.Fatal(err)
	}
	r = only(t, f)
	sameBody(t, r, `{"selector":{"filter":{"status":["RUNNING"],"createdAfter":"2026-10-10T00:00:00.000Z"}},"action":"CANCEL","reason":"clean up the day","maxItems":10}`)
	if r.headers.Get("Idempotency-Key") != "batch-key-01" || created.Counts != (client.BatchCounts{Selected: 2, Succeeded: 1, Deferred: 1}) || created.HasPayload {
		t.Errorf("%v %+v", r.headers, created)
	}
	if created.ItemsPerSecond == nil || *created.ItemsPerSecond != 0.5 || created.MaxItems != nil || !created.StartedAt.IsZero() {
		t.Errorf("a number that need not be whole, and an absent one: %+v", created)
	}

	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 202, map[string]any{"batchId": batchID, "action": "CANCEL", "state": "CANCELLING", "createdBy": "alice", "createdAt": "2026-10-10T10:00:00Z"})
	})
	cancelling, err := c.Batch.Cancel(ctx, nsID, batchID, "stop the clean up", client.WithIdempotencyKey("batch-cancel-1"))
	if err != nil {
		t.Fatal(err)
	}
	r = only(t, f)
	if pathOf(t, r) != "/v1/namespaces/"+nsID+"/batch-operations/"+batchID+"/cancel" || cancelling.State != "CANCELLING" || r.headers.Get("Idempotency-Key") != "batch-cancel-1" {
		t.Errorf("%s %+v", r.url, cancelling)
	}
	sameBody(t, r, `{"reason":"stop the clean up"}`)

	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"items": []any{
			map[string]any{"ordinal": 0, "runId": runUUID, "state": "SKIPPED", "reason": "ALREADY_TERMINAL"},
			map[string]any{"ordinal": 1, "runId": runUUID, "state": "NEWER_STATE"},
		}, "nextCursor": "n"})
	})
	items, err := c.Batch.ListItems(ctx, nsID, batchID, client.BatchListOptions{State: "SKIPPED", Limit: 2})
	if err != nil {
		t.Fatal(err)
	}
	r = only(t, f)
	if pathOf(t, r) != "/v1/namespaces/"+nsID+"/batch-operations/"+batchID+"/items" || queryOf(t, r).Encode() != "limit=2&state=SKIPPED" {
		t.Errorf("%s", r.url)
	}
	if items.Items[0].Reason != "ALREADY_TERMINAL" || items.Items[1].State != "NEWER_STATE" || items.NextCursor != "n" {
		t.Errorf("a state of a newer server is kept as a string: %+v", items)
	}
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"items": []any{job}, "nextCursor": nil})
	})
	listed, err := c.Batch.List(ctx, nsID, client.BatchListOptions{State: "COMPLETED", Limit: 10})
	if err != nil || len(listed.Items) != 1 || queryOf(t, only(t, f)).Encode() != "limit=10&state=COMPLETED" {
		t.Errorf("%v %+v", err, listed)
	}
	f.answer(f.handler)
	_, err = c.Batch.Get(ctx, nsID, "x/y")
	wantCode(t, err, sagat.CodeValidationFailed)
	if got := len(f.requests()); got != 0 {
		t.Errorf("a batch id that is not a UUID sends nothing: %d requests", got)
	}
}

func TestASelectorAndAFilterAreWrittenAsTheyAreReadWithTheirInstantsInUTC(t *testing.T) {
	var selector client.BatchSelector
	text := `{"filter":{"workflowId":"` + workflowID + `","createdAfter":"2026-10-10T07:30:00+05:30","createdBefore":"2026-10-10T08:00:00Z","businessIdPrefix":"order-"}}`
	if err := json.Unmarshal([]byte(text), &selector); err != nil {
		t.Fatal(err)
	}
	if selector.Filter == nil || selector.Filter.CreatedAfter.UTC().Hour() != 2 || selector.Filter.CreatedAfter.Location() != time.UTC {
		t.Fatalf("%+v", selector.Filter)
	}
	written, err := json.Marshal(selector)
	if err != nil {
		t.Fatal(err)
	}
	if string(written) != `{"filter":{"workflowId":"`+workflowID+`","businessIdPrefix":"order-","createdAfter":"2026-10-10T02:00:00.000Z","createdBefore":"2026-10-10T08:00:00.000Z"}}` {
		t.Errorf("%s", written)
	}
	if err := json.Unmarshal([]byte(`{"filter":{"createdAfter":"yesterday"}}`), &selector); err == nil {
		t.Error("text that is not an instant is refused")
	}
	var spec client.ScheduleSpec
	if err := json.Unmarshal([]byte(`{"cron":["0 9 * * *"],"startAt":"2099-01-01T01:00:00+01:00","timezone":"UTC"}`), &spec); err != nil || spec.StartAt.UTC().Hour() != 0 || !spec.EndAt.IsZero() {
		t.Errorf("%v %+v", err, spec)
	}
	if err := json.Unmarshal([]byte(`{"endAt":"not a time"}`), &spec); err == nil {
		t.Error("text that is not an instant is refused")
	}
}

// ---- the with-start calls

func TestSignalWithStartNamesTheWorkflowByIDCarriesTheSignalAndOneKeyAndSaysWhatTheSignalWoke(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 202, map[string]any{"runId": runUUID, "started": true, "deliveredTo": []any{"wait"}})
	})
	c := newClient(t, f)
	result, err := c.SignalWithStart(context.Background(), workflowID, "order-9", map[string]any{"a": 1}, "go", map[string]any{"b": 2},
		client.WithIdempotencyKey("sws-key-0001"), client.WithBusinessIDReusePolicy(client.BusinessIDAllowDuplicateFailedOnly))
	if err != nil {
		t.Fatal(err)
	}
	r := only(t, f)
	if r.url != "/v1/runs/signal-with-start" || r.headers.Get("Idempotency-Key") != "sws-key-0001" {
		t.Errorf("%s %v", r.url, r.headers)
	}
	sameBody(t, r, `{"workflowId":"`+workflowID+`","businessId":"order-9","input":{"a":1},"signal":{"name":"go","payload":{"b":2}},"businessIdReusePolicy":"ALLOW_DUPLICATE_FAILED_ONLY"}`)
	if result.RunID != runUUID || !result.Started || !reflect.DeepEqual(result.DeliveredTo, []string{"wait"}) {
		t.Errorf("%+v", result)
	}
	// A business identifier is sent as given, empty or not, and an input or payload left nil is an empty object.
	f.answer(func(_ seen, w http.ResponseWriter) { reply(w, 202, map[string]any{"runId": runUUID, "started": false}) })
	if _, err := c.SignalWithStart(context.Background(), workflowID, " Padded ", nil, "go", nil); err != nil {
		t.Fatal(err)
	}
	sameBody(t, only(t, f), `{"workflowId":"`+workflowID+`","businessId":" Padded ","input":{},"signal":{"name":"go","payload":{}}}`)
}

func TestUpdateWithStartRequiresTheCallersKeyKeepsItAcrossARetryAndReadsARejectionAsAnAnswer(t *testing.T) {
	receipt := map[string]any{
		"runId": runUUID, "operation": "set", "updateKey": "u-1", "status": "APPLIED", "result": 7, "beforeStateRevision": 0, "afterStateRevision": 1,
		"runVersion": 4, "replayed": true,
	}
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"runId": runUUID, "started": true, "update": map[string]any{"outcome": "APPLIED", "receipt": receipt}})
	})
	c := newClient(t, f)
	ctx := context.Background()
	zero := int64(0)
	applied, err := c.UpdateWithStart(ctx, workflowID, "order-9", map[string]any{}, "set",
		client.UpdateRequest{UpdateKey: "u-1", Args: map[string]any{"value": 7}, ExpectedStateRevision: &zero},
		client.WithIdempotencyKey("uws-key-0001"), client.WithBusinessIDConflictPolicy(client.BusinessIDFail), client.WithStartDelay(2*time.Second))
	if err != nil {
		t.Fatal(err)
	}
	r := only(t, f)
	if r.url != "/v1/runs/update-with-start" || r.headers.Get("Idempotency-Key") != "uws-key-0001" {
		t.Errorf("%s %v", r.url, r.headers)
	}
	sameBody(t, r, `{"workflowId":"`+workflowID+`","businessId":"order-9","input":{},"update":{"name":"set","updateKey":"u-1","args":{"value":7},"expectedStateRevision":0},"businessIdConflictPolicy":"FAIL","startDelayMs":2000}`)
	if applied.Update.Outcome != "APPLIED" || applied.Update.Receipt == nil || applied.Update.Receipt.AfterStateRevision != 1 || applied.Update.Receipt.Result != float64(7) || !applied.Update.Receipt.Replayed {
		t.Errorf("a replay says so: %+v", applied.Update)
	}

	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"runId": runUUID, "started": false, "update": map[string]any{"outcome": "REJECTED", "reason": "REVISION_CONFLICT"}})
	})
	rejected, err := c.UpdateWithStart(ctx, workflowID, "order-9", nil, "set", client.UpdateRequest{UpdateKey: "u-2", Args: map[string]any{}}, client.WithIdempotencyKey("uws-key-0002"))
	if err != nil {
		t.Fatal(err)
	}
	if rejected.Started || rejected.Update.Outcome != "REJECTED" || rejected.Update.Reason != "REVISION_CONFLICT" || rejected.Update.Receipt != nil {
		t.Errorf("%+v", rejected)
	}

	f.answer(f.handler)
	negative := int64(-1)
	for _, bad := range []client.UpdateRequest{{UpdateKey: ""}, {UpdateKey: "has space"}, {UpdateKey: strings.Repeat("k", 129)}, {UpdateKey: "u-3", ExpectedStateRevision: &negative}} {
		_, err := c.UpdateWithStart(ctx, workflowID, "order-9", nil, "set", bad)
		wantCode(t, err, sagat.CodeValidationFailed)
	}
	_, err = c.UpdateWithStart(ctx, workflowID, "order-9", nil, "bad name!", client.UpdateRequest{UpdateKey: "u-3"})
	wantCode(t, err, sagat.CodeValidationFailed)
	if got := len(f.requests()); got != 0 {
		t.Errorf("a local refusal sends nothing: %d requests", got)
	}

	attempts := 0
	f.answer(func(_ seen, w http.ResponseWriter) {
		attempts++
		if attempts < 2 {
			reply(w, 503, unavailable())
			return
		}
		reply(w, 200, map[string]any{"runId": runUUID, "started": true, "update": map[string]any{"outcome": "APPLIED", "receipt": receipt}})
	})
	if _, err := c.UpdateWithStart(ctx, workflowID, "order-9", nil, "set", client.UpdateRequest{UpdateKey: "u-4"}); err != nil {
		t.Fatal(err)
	}
	requests := f.requests()
	if len(requests) != 2 || string(requests[0].body) != string(requests[1].body) || requests[0].headers.Get("Idempotency-Key") == "" ||
		requests[0].headers.Get("Idempotency-Key") != requests[1].headers.Get("Idempotency-Key") {
		t.Errorf("a retry is the same body under the same key: %d requests", len(requests))
	}
}

// ---- reset

func TestTheResetCallsListThePointsPreviewThePlanResetWithAKeyAndReadTheRegister(t *testing.T) {
	f := newFake(t)
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"items": []any{
			map[string]any{"sequence": 1, "kind": "RUN_STARTED", "node": nil, "occurredAt": "2026-10-10T10:00:00Z"},
			map[string]any{"sequence": 4, "kind": "SOMETHING_NEWER", "node": "a", "occurredAt": "2026-10-10T10:00:01Z"},
		}, "nextCursor": nil})
	})
	c := newClient(t, f)
	ctx := context.Background()
	points, err := c.ResetPoints(ctx, runUUID, client.ResetPointsOptions{Limit: 5})
	if err != nil {
		t.Fatal(err)
	}
	if got := only(t, f).url; got != "/v1/runs/"+runUUID+"/reset-points?limit=5" {
		t.Errorf("%s", got)
	}
	if len(points.Items) != 2 || points.Items[1].Kind != "SOMETHING_NEWER" || points.Items[1].Node != "a" || points.Items[0].Node != "" {
		t.Errorf("%+v", points.Items)
	}

	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{
			"sequence": 4, "restored": []any{map[string]any{"node": "a", "kind": "STEP"}},
			"reexecuted": []any{map[string]any{"node": "b", "completedInSource": true, "markers": []any{"irreversible"}}},
			"reapply":    []any{}, "requiresAcknowledgement": true, "prefixEvents": 6,
		})
	})
	plan, err := c.PreviewReset(ctx, runUUID, 4, client.ReapplyNone)
	if err != nil {
		t.Fatal(err)
	}
	r := only(t, f)
	if r.url != "/v1/runs/"+runUUID+"/reset-previews" {
		t.Errorf("%s", r.url)
	}
	sameBody(t, r, `{"sequence":4,"reapply":"NONE"}`)
	if !plan.RequiresAcknowledgement || plan.PrefixEvents != 6 || !reflect.DeepEqual(plan.Reexecuted, []client.ResetReexecuted{{Node: "b", CompletedInSource: true, Markers: []string{"irreversible"}}}) {
		t.Errorf("%+v", plan)
	}
	if _, err := c.PreviewReset(ctx, runUUID, 4, ""); err != nil {
		t.Fatal(err)
	}
	sameBody(t, f.requests()[1], `{"sequence":4}`)
	f.answer(f.handler)
	_, err = c.PreviewReset(ctx, runUUID, 0, "")
	wantCode(t, err, sagat.CodeValidationFailed)
	_, err = c.PreviewReset(ctx, "../x", 1, "")
	wantCode(t, err, sagat.CodeValidationFailed)
	if got := len(f.requests()); got != 0 {
		t.Errorf("a local refusal sends nothing: %d requests", got)
	}

	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 202, map[string]any{"resetId": batchID, "sourceRunId": runUUID, "sequence": 4, "state": "CREATED", "newRunId": scheduleID, "reapplyReport": []any{}, "createdAt": "2026-10-10T10:00:02Z"})
	})
	made, err := c.Reset(ctx, runUUID, client.ResetRequest{Sequence: 4, Reason: "start over from a", AcknowledgeReexecution: true, BusinessID: "DROP"}, client.WithIdempotencyKey("reset-key-001"))
	if err != nil {
		t.Fatal(err)
	}
	r = only(t, f)
	if r.url != "/v1/runs/"+runUUID+"/resets" || r.headers.Get("Idempotency-Key") != "reset-key-001" {
		t.Errorf("%s %v", r.url, r.headers)
	}
	sameBody(t, r, `{"sequence":4,"reason":"start over from a","acknowledgeReexecution":true,"businessId":"DROP"}`)
	if made.State != "CREATED" || made.NewRunID != scheduleID || made.FailureReason != "" || made.ReapplyReport == nil {
		t.Errorf("%+v", made)
	}
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 202, map[string]any{"resetId": batchID, "sourceRunId": runUUID, "sequence": 1, "state": "PENDING_SOURCE"})
	})
	if _, err := c.Reset(ctx, runUUID, client.ResetRequest{Sequence: 1, Reason: "start over from the top"}); err != nil {
		t.Fatal(err)
	}
	sameBody(t, only(t, f), `{"sequence":1,"reason":"start over from the top"}`)
	f.answer(f.handler)
	_, err = c.Reset(ctx, runUUID, client.ResetRequest{Reason: "no sequence at all"})
	wantCode(t, err, sagat.CodeValidationFailed)

	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"items": []any{map[string]any{"resetId": batchID, "sourceRunId": runUUID, "sequence": 4, "state": "CREATED"}}})
	})
	register, err := c.ListResets(ctx, runUUID)
	if err != nil || len(register) != 1 || only(t, f).url != "/v1/runs/"+runUUID+"/resets" {
		t.Errorf("%v %+v", err, register)
	}
	f.answer(func(_ seen, w http.ResponseWriter) {
		reply(w, 200, map[string]any{"resetId": batchID, "sourceRunId": runUUID, "sequence": 4, "state": "CREATED"})
	})
	got, err := c.GetReset(ctx, runUUID, batchID)
	if err != nil || got.ResetID != batchID || only(t, f).url != "/v1/runs/"+runUUID+"/resets/"+batchID {
		t.Errorf("%v %+v", err, got)
	}
}

// ---- the token

func TestTheTokenDoesNotAppearInAnythingAGroupOfTheClientExposes(t *testing.T) {
	f := newFake(t)
	c := newClient(t, f)
	for name, group := range map[string]any{"schedules": c.Schedules, "batch": c.Batch, "businessIds": c.BusinessIDs} {
		encoded, err := json.Marshal(group)
		if err != nil {
			t.Fatal(err)
		}
		outputs := []string{string(encoded), fmt.Sprintf("%v", group), fmt.Sprintf("%+v", group), fmt.Sprintf("%#v", group), fmt.Sprintf("%v", struct{ G any }{group})}
		for _, out := range outputs {
			if strings.Contains(out, "tok-1") {
				t.Errorf("%s shows the token: %s", name, out)
			}
		}
	}
}
