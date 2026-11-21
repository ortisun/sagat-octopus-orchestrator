package main

// The schedule steps of the runner (feature 018, FR-018-016): a scenario's `write.action` may carry the three
// business-id fields and the typed write sends them; the observation of a schedule prints them only when the
// server's action has them, so every other observation is what it was.

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
)

const (
	testNS       = "0190aaaa-0000-7000-8000-00000000aaaa"
	testSchedule = "0190aaaa-0000-7000-8000-0000000000a1"
	testWorkflow = "0190aaaa-0000-7000-8000-0000000000c1"
)

// scheduleServer answers every request with the schedule whose action is `action`, and keeps what it was sent.
func scheduleServer(t *testing.T, action map[string]any) (c *client.Client, bodies func() []string) {
	t.Helper()
	var mu sync.Mutex
	var seen []string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		mu.Lock()
		seen = append(seen, string(body))
		mu.Unlock()
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(200)
		_ = json.NewEncoder(w).Encode(map[string]any{
			"scheduleId": testSchedule, "name": "nightly", "origin": "API", "state": "ACTIVE", "revision": 2, "createdAt": "2026-10-10T10:00:00Z",
			"action": action, "nextActionAt": "2099-01-01T09:00:00Z", "updatedAt": "2026-10-10T10:00:00Z",
			"nextActions": []any{}, "recentActions": []any{}, "openRuns": []any{}, "bufferedCount": 0, "counters": map[string]any{},
		})
	}))
	t.Cleanup(server.Close)
	c, err := client.New(server.URL+"/v1", "payments", sagat.StaticToken("tok-1"))
	if err != nil {
		t.Fatal(err)
	}
	return c, func() []string {
		mu.Lock()
		defer mu.Unlock()
		return append([]string(nil), seen...)
	}
}

func stepOf(t *testing.T, text string) object {
	t.Helper()
	value, err := decode([]byte(text))
	if err != nil {
		t.Fatal(err)
	}
	return asObject(value)
}

func play(t *testing.T, c *client.Client, step object, op string) object {
	t.Helper()
	ops := &operations{client: c, config: object{"namespaceId": testNS}, results: map[string]object{"create": {"scheduleId": testSchedule}}}
	result, handled, err := ops.play(context.Background(), step, op, []client.CallOption{client.WithIdempotencyKey("conformance-key-1")})
	if err != nil || !handled {
		t.Fatalf("handled %v: %v", handled, err)
	}
	return result
}

func actionText(t *testing.T, result object) string {
	t.Helper()
	text, err := encode(asObject(result["action"]))
	if err != nil {
		t.Fatal(err)
	}
	return string(text)
}

func sameJSON(t *testing.T, got, want string) {
	t.Helper()
	var a, b any
	if err := json.Unmarshal([]byte(got), &a); err != nil {
		t.Fatalf("not JSON: %s", got)
	}
	if err := json.Unmarshal([]byte(want), &b); err != nil {
		t.Fatal(err)
	}
	ga, _ := json.Marshal(a)
	gb, _ := json.Marshal(b)
	if string(ga) != string(gb) {
		t.Errorf("\n got  %s\n want %s", ga, gb)
	}
}

const writeWith = `{"id":"create","op":"%OP%","schedule":"create","expectedRevision":2,"write":{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":%ACTION%}}`

func scheduleStep(t *testing.T, op, action string) object {
	t.Helper()
	return stepOf(t, strings.NewReplacer("%OP%", op, "%ACTION%", action).Replace(writeWith))
}

func TestTheRunnerPassesTheThreeBusinessIDFieldsOfAScenarioActionThroughTheTypedWriteAndPrintsThemWhenTheServerHasThem(t *testing.T) {
	prefix := " Order/{x}-é "
	prefixJSON, _ := json.Marshal(prefix)
	action := `{"workflowId":"` + testWorkflow + `","input":{"n":1},"businessIdPrefix":` + string(prefixJSON) +
		`,"businessIdConflictPolicy":"USE_EXISTING","businessIdReusePolicy":"ALLOW_DUPLICATE_FAILED_ONLY"}`
	for _, op := range []string{"scheduleCreate", "scheduleUpdate"} {
		c, bodies := scheduleServer(t, map[string]any{
			"workflowId": testWorkflow, "versionPolicy": "ACTIVE", "inputDigest": strings.Repeat("a", 64),
			"businessIdPrefix": prefix, "businessIdConflictPolicy": "USE_EXISTING", "businessIdReusePolicy": "ALLOW_DUPLICATE_FAILED_ONLY",
		})
		result := play(t, c, scheduleStep(t, op, action), op)
		sent := bodies()
		if len(sent) != 1 {
			t.Fatalf("%s: %d requests", op, len(sent))
		}
		want := `{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":` + action + `}`
		if op == "scheduleUpdate" {
			want = `{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":` + action + `,"expectedRevision":2}`
		}
		sameJSON(t, sent[0], want)
		sameJSON(t, actionText(t, result), `{"workflowId":"`+testWorkflow+`","versionPolicy":"ACTIVE","versionId":null,"inputDigest":"`+strings.Repeat("a", 64)+
			`","nominalTimeField":null,"runTimeoutMs":null,"businessIdPrefix":`+string(prefixJSON)+
			`,"businessIdConflictPolicy":"USE_EXISTING","businessIdReusePolicy":"ALLOW_DUPLICATE_FAILED_ONLY"}`)
	}
}

func TestTheRunnerSendsAndPrintsNoneOfTheThreeFieldsForAnActionThatHasNone(t *testing.T) {
	action := `{"workflowId":"` + testWorkflow + `","input":{"n":1}}`
	for _, op := range []string{"scheduleCreate", "scheduleUpdate"} {
		c, bodies := scheduleServer(t, map[string]any{"workflowId": testWorkflow, "versionPolicy": "ACTIVE", "inputDigest": strings.Repeat("a", 64)})
		result := play(t, c, scheduleStep(t, op, action), op)
		sent := bodies()[0]
		for _, key := range []string{"businessIdPrefix", "businessIdConflictPolicy", "businessIdReusePolicy"} {
			if strings.Contains(sent, key) {
				t.Errorf("%s sent %s: %s", op, key, sent)
			}
		}
		// Byte for byte what the golden files already hold.
		sameJSON(t, actionText(t, result), `{"workflowId":"`+testWorkflow+`","versionPolicy":"ACTIVE","versionId":null,"inputDigest":"`+strings.Repeat("a", 64)+
			`","nominalTimeField":null,"runTimeoutMs":null}`)
	}
	// An action the server answered with the three keys as null has none either.
	c, _ := scheduleServer(t, map[string]any{
		"workflowId": testWorkflow, "inputDigest": strings.Repeat("a", 64), "businessIdPrefix": nil, "businessIdConflictPolicy": nil, "businessIdReusePolicy": nil,
	})
	result := play(t, c, stepOf(t, `{"id":"describe","op":"scheduleGet","schedule":"create","nextCount":5,"recentCount":5}`), "scheduleGet")
	if text := actionText(t, result); strings.Contains(text, "businessId") {
		t.Errorf("null is absent: %s", text)
	}
}

func TestTheDescriptionOfAScheduleAndTheEmptyPrefixOfAScenarioFollowTheSameRules(t *testing.T) {
	c, _ := scheduleServer(t, map[string]any{
		"workflowId": testWorkflow, "versionPolicy": "ACTIVE", "inputDigest": strings.Repeat("a", 64),
		"businessIdPrefix": "order", "businessIdConflictPolicy": "FAIL", "businessIdReusePolicy": "ALLOW_DUPLICATE",
	})
	result := play(t, c, stepOf(t, `{"id":"describe","op":"scheduleGet","schedule":"create","nextCount":5,"recentCount":5}`), "scheduleGet")
	sameJSON(t, actionText(t, result), `{"workflowId":"`+testWorkflow+`","versionPolicy":"ACTIVE","versionId":null,"inputDigest":"`+strings.Repeat("a", 64)+
		`","nominalTimeField":null,"runTimeoutMs":null,"businessIdPrefix":"order","businessIdConflictPolicy":"FAIL","businessIdReusePolicy":"ALLOW_DUPLICATE"}`)
	// The empty prefix of a scenario is sent as the empty string, for the server to refuse.
	c, bodies := scheduleServer(t, map[string]any{"workflowId": testWorkflow, "inputDigest": strings.Repeat("a", 64)})
	play(t, c, scheduleStep(t, "scheduleCreate", `{"workflowId":"`+testWorkflow+`","input":{},"businessIdPrefix":""}`), "scheduleCreate")
	sameJSON(t, bodies()[0], `{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":{"workflowId":"`+testWorkflow+`","input":{},"businessIdPrefix":""}}`)
}
