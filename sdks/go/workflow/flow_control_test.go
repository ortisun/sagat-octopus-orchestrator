package workflow

import (
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"strings"
	"testing"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// Feature 022 (FR-022-035, FR-022-050): concurrency and rateLimit at the workflow level and on activity and script
// steps. The builders assemble the policy with the IR's names; the server checks the bounds at publication.

func providerSync() *Builder {
	return New("provider-sync").
		Concurrency(ConcurrencyPolicy{Key: "input.accountId", Limit: 1}).
		RateLimit(RateLimitPolicy{Group: "provider-starts", Key: "input.providerId", Limit: 100, Per: PerMinute, QueueTimeout: 15 * time.Minute}).
		Step("fetch", "provider.fetch", RateLimit(RateLimitPolicy{Key: "input.providerId", Limit: 10, Per: PerSecond})).
		End("done")
}

func accountSerial() *Builder {
	return New("account-serial").
		Concurrency(ConcurrencyPolicy{Group: "orders-by-account", Key: "input.accountId", Limit: 1, QueueTimeout: 10 * time.Minute}).
		Step("price", "catalog.price").
		Step("notify", "crm.notify", Concurrency(ConcurrencyPolicy{Key: "steps.price.output.region", Limit: 2})).
		End("done")
}

func TestFlowControlBuildersWriteTheSharedFixturesByteForByte(t *testing.T) {
	for name, builder := range map[string]*Builder{"provider-sync": providerSync(), "account-serial": accountSerial()} {
		raw, err := os.ReadFile("../../../tests/contract/flow-control-step-policies/fixtures/" + name + ".workflow.json")
		if err != nil {
			t.Fatal(err)
		}
		var fixture any
		if err = json.Unmarshal(raw, &fixture); err != nil {
			t.Fatal(err)
		}
		expected, err := jsonx.Canonical(fixture)
		if err != nil {
			t.Fatal(err)
		}
		text, err := builder.JSON()
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if text != string(expected) {
			t.Fatalf("%s:\n got %s\nwant %s", name, text, expected)
		}
	}
}

func built(t *testing.T, b *Builder) map[string]any {
	t.Helper()
	text, err := b.JSON()
	if err != nil {
		t.Fatal(err)
	}
	var document map[string]any
	if err = json.Unmarshal([]byte(text), &document); err != nil {
		t.Fatal(err)
	}
	return document
}

func TestNoPolicyWritesNoFieldSoEarlierDocumentsAreUnchanged(t *testing.T) {
	document := built(t, New("plain-flow").Step("a", "act.a").Script("s", "sha256:"+strings.Repeat("0", 64)).End("done"))
	for _, key := range []string{"concurrency", "rateLimit"} {
		if _, present := document[key]; present {
			t.Fatalf("workflow carries %s", key)
		}
		for _, n := range document["nodes"].([]any) {
			if _, present := n.(map[string]any)[key]; present {
				t.Fatalf("node carries %s", key)
			}
		}
	}
}

func TestAScriptCarriesBothPoliciesWithEveryMember(t *testing.T) {
	document := built(t, New("gated-script").
		Script("s", "sha256:"+strings.Repeat("0", 64),
			ScriptConcurrency(ConcurrencyPolicy{Key: "input.tenant", Limit: 10000, Group: "scripts.a-b", QueueTimeout: 24 * time.Hour}),
			ScriptRateLimit(RateLimitPolicy{Key: "input.tenant", Limit: 3, Per: PerHour, Group: "scripts.a-b", QueueTimeout: time.Millisecond})).
		End("done"))
	n := document["nodes"].([]any)[0].(map[string]any)
	if want := map[string]any{"key": "input.tenant", "limit": 10000.0, "group": "scripts.a-b", "queueTimeoutMs": 86400000.0}; !reflect.DeepEqual(n["concurrency"], want) {
		t.Fatalf("concurrency %v", n["concurrency"])
	}
	if want := map[string]any{"key": "input.tenant", "limit": 3.0, "per": "hour", "group": "scripts.a-b", "queueTimeoutMs": 1.0}; !reflect.DeepEqual(n["rateLimit"], want) {
		t.Fatalf("rateLimit %v", n["rateLimit"])
	}
}

func TestTheLastWorkflowLevelCallWins(t *testing.T) {
	document := built(t, New("replaced-flow").
		Concurrency(ConcurrencyPolicy{Limit: 9}).Concurrency(ConcurrencyPolicy{Limit: 2}).
		RateLimit(RateLimitPolicy{Limit: 9, Per: PerHour}).RateLimit(RateLimitPolicy{Limit: 4, Per: PerSecond}).
		Step("a", "act.a").End("done"))
	if !reflect.DeepEqual(document["concurrency"], map[string]any{"limit": 2.0}) {
		t.Fatalf("concurrency %v", document["concurrency"])
	}
	if !reflect.DeepEqual(document["rateLimit"], map[string]any{"limit": 4.0, "per": "second"}) {
		t.Fatalf("rateLimit %v", document["rateLimit"])
	}
}

func TestTheBuildersOnlyAssembleTheBoundsAreTheServers(t *testing.T) {
	document := built(t, New("unchecked-flow").
		Concurrency(ConcurrencyPolicy{Limit: 0, Group: "Not A Group"}).
		RateLimit(RateLimitPolicy{Limit: 101, Per: PerSecond, QueueTimeout: 86400001 * time.Millisecond}).
		Step("a", "act.a", Concurrency(ConcurrencyPolicy{Limit: 10001}), RateLimit(RateLimitPolicy{Limit: 1, Per: "day"})).
		End("done"))
	if !reflect.DeepEqual(document["concurrency"], map[string]any{"limit": 0.0, "group": "Not A Group"}) {
		t.Fatalf("concurrency %v", document["concurrency"])
	}
	if !reflect.DeepEqual(document["rateLimit"], map[string]any{"limit": 101.0, "per": "second", "queueTimeoutMs": 86400001.0}) {
		t.Fatalf("rateLimit %v", document["rateLimit"])
	}
	n := document["nodes"].([]any)[0].(map[string]any)
	if !reflect.DeepEqual(n["concurrency"], map[string]any{"limit": 10001.0}) || !reflect.DeepEqual(n["rateLimit"], map[string]any{"limit": 1.0, "per": "day"}) {
		t.Fatalf("node %v", n)
	}
}

func TestAQueueTimeoutThatIsNotWholeMillisecondsIsRefused(t *testing.T) {
	for name, b := range map[string]*Builder{
		"workflow concurrency": New("odd-flow").Concurrency(ConcurrencyPolicy{Limit: 1, QueueTimeout: 1500 * time.Microsecond}).Step("a", "act.a"),
		"workflow rateLimit":   New("odd-flow").RateLimit(RateLimitPolicy{Limit: 1, Per: PerSecond, QueueTimeout: time.Microsecond}).Step("a", "act.a"),
		"step concurrency":     New("odd-flow").Step("a", "act.a", Concurrency(ConcurrencyPolicy{Limit: 1, QueueTimeout: time.Nanosecond})),
		"script rateLimit": New("odd-flow").Script("s", "sha256:"+strings.Repeat("0", 64),
			ScriptRateLimit(RateLimitPolicy{Limit: 1, Per: PerSecond, QueueTimeout: 1001 * time.Microsecond})),
	} {
		_, err := b.Build()
		var failure *sagat.Error
		if !errors.As(err, &failure) || failure.Code != sagat.CodeValidationFailed || !strings.Contains(failure.Message, "queueTimeoutMs") {
			t.Fatalf("%s: %v", name, err)
		}
	}
}
