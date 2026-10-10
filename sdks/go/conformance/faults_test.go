package main

import (
	"bytes"
	"io"
	"net/http"
	"strings"
	"testing"
)

func two() *int { n := 2; return &n }
func one() *int { n := 1; return &n }

// The counting rule of the README: a call is counted against every fault that matches it, whether or
// not another fault answered it; the first fault in file order that is due answers it.
func TestACallIsCountedAgainstEveryMatchingFaultAndTheFirstDueOneAnswers(t *testing.T) {
	p := newPlan([]fault{
		{Channel: "grpc", Match: "PollTasks", Action: "status", Code: "UNAVAILABLE", Times: two()},
		{Channel: "grpc", Match: "PollTasks", Action: "status", Code: "UNAUTHENTICATED", After: 2, Times: one()},
		{Channel: "grpc", Match: "Heartbeat", Action: "failBefore"},
		{Channel: "http", Match: "POST /runs", Action: "dropResponse", Times: one()},
	})
	var answers []string
	for i := 0; i < 5; i++ {
		if f := p.next("grpc", "PollTasks"); f != nil {
			answers = append(answers, f.Code)
		} else {
			answers = append(answers, "-")
		}
	}
	if got := strings.Join(answers, ","); got != "UNAVAILABLE,UNAVAILABLE,UNAUTHENTICATED,-,-" {
		t.Errorf("%s", got)
	}
	if p.next("grpc", "Heartbeat") == nil || p.next("grpc", "Heartbeat") == nil {
		t.Error("a fault with no times answers every call")
	}
	if p.next("http", "GET /runs") != nil {
		t.Error("another channel and method")
	}
	if p.next("http", "POST /runs/abc/operations") == nil {
		t.Error("the match is a prefix")
	}
	if p.next("http", "POST /runs") != nil {
		t.Error("times: 1 was used")
	}
	if got := p.applied(); len(got) != 4 || got[0] != 2 || got[1] != 1 || got[2] != 2 || got[3] != 1 {
		t.Errorf("%v", got)
	}
}

type recorder struct{ calls int }

func (r *recorder) RoundTrip(req *http.Request) (*http.Response, error) {
	r.calls++
	return &http.Response{StatusCode: 202, Body: io.NopCloser(strings.NewReader("{}")), Header: http.Header{}, Request: req}, nil
}

func TestTheRoundTripperAppliesWhatAFaultSays(t *testing.T) {
	retry := int64(1500)
	p := newPlan([]fault{
		{Channel: "http", Match: "POST /runs", Action: "failBefore", Times: one()},
		{Channel: "http", Match: "POST /runs", Action: "dropResponse", After: 1, Times: one()},
		{Channel: "http", Match: "GET /runs", Action: "status", Status: 429, RetryAfterMs: &retry, Body: []byte(`{"code":"RATE_LIMITED"}`), Times: one()},
	})
	inner := &recorder{}
	rt := &transport{plan: p, inner: inner, apiPath: "/v1"}
	post := func() (*http.Response, error) {
		req, _ := http.NewRequest("POST", "http://x/v1/runs", bytes.NewReader([]byte("{}")))
		return rt.RoundTrip(req)
	}
	if _, err := post(); err == nil || inner.calls != 0 {
		t.Errorf("failBefore: the call never leaves: %v after %d calls", err, inner.calls)
	}
	if _, err := post(); err == nil || inner.calls != 1 {
		t.Errorf("dropResponse: the call is made and the answer lost: %v after %d calls", err, inner.calls)
	}
	resp, err := post()
	if err != nil || resp.StatusCode != 202 || inner.calls != 2 {
		t.Errorf("no fault: the real thing: %v %v", resp, err)
	}
	get, _ := http.NewRequest("GET", "http://x/v1/runs", nil)
	resp, err = rt.RoundTrip(get)
	if err != nil || resp.StatusCode != 429 || resp.Header.Get("Retry-After") != "2" || inner.calls != 2 {
		t.Fatalf("status: %v %v", resp, err)
	}
	body, _ := io.ReadAll(resp.Body)
	if string(body) != `{"code":"RATE_LIMITED"}` {
		t.Errorf("%s", body)
	}
	if got := p.applied(); got[0] != 1 || got[1] != 1 || got[2] != 1 {
		t.Errorf("%v", got)
	}
}
