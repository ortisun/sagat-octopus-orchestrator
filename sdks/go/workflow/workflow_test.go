package workflow_test

import (
	"encoding/json"
	"fmt"
	"reflect"
	"regexp"
	"strings"
	"testing"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/workflow"
)

type doc = map[string]any

func nodes(t *testing.T, d doc) []any {
	t.Helper()
	list, ok := d["nodes"].([]any)
	if !ok {
		t.Fatalf("nodes: %T", d["nodes"])
	}
	return list
}

// same compares two values as the JSON they write: the document holds int64 and the expectations ints.
func same(t *testing.T, got, want any) {
	t.Helper()
	g, _ := json.Marshal(got)
	w, _ := json.Marshal(want)
	if string(g) != string(w) {
		t.Errorf("\n got %s\nwant %s", g, w)
	}
}

func TestAChainOfStepsIsLinkedInOrderAndStartsAtTheFirst(t *testing.T) {
	d, err := workflow.New("order-flow").
		Description("charges and ships").
		Step("charge", "payments.charge", workflow.Retry(workflow.MaxAttempts(3), workflow.WithStrategy(workflow.Exponential),
			workflow.InitialDelay(100*time.Millisecond), workflow.MaxDelay(time.Second), workflow.Jitter(true))).
		Sleep("pause", 1500*time.Millisecond).
		Step("ship", "shipping.ship").
		Build()
	if err != nil {
		t.Fatal(err)
	}
	if d["entrypoint"] != "charge" {
		t.Errorf("entrypoint %v", d["entrypoint"])
	}
	same(t, d["metadata"], doc{"name": "order-flow", "description": "charges and ships"})
	same(t, d["nodes"], []doc{
		{"id": "charge", "type": "activity", "activity": "payments.charge", "retry": doc{"maxAttempts": 3, "strategy": "exponential", "initialDelayMs": 100, "maxDelayMs": 1000, "jitter": true}, "next": "pause"},
		{"id": "pause", "type": "timer", "durationMs": 1500, "next": "ship"},
		{"id": "ship", "type": "activity", "activity": "shipping.ship"},
	})
}

func TestAbsentOptionsAreLeftOutAndNeverNullAndAGivenFalseIsKept(t *testing.T) {
	d, err := workflow.New("plain-flow").Step("a", "act.a").Step("b", "act.b", workflow.Irreversible(false), workflow.Retry(workflow.Jitter(false))).Build()
	if err != nil {
		t.Fatal(err)
	}
	same(t, d["nodes"], []doc{
		{"id": "a", "type": "activity", "activity": "act.a", "next": "b"},
		{"id": "b", "type": "activity", "activity": "act.b", "irreversible": false, "retry": doc{"jitter": false}},
	})
	text, err := workflow.New("plain-flow").Step("a", "act.a").JSON()
	if err != nil || strings.Contains(text, "null") {
		t.Errorf("%s %v", text, err)
	}
}

func TestCompensationReconciliationAndTheOtherStepOptionsAreWrittenAsTheSchemaNamesThem(t *testing.T) {
	d, err := workflow.New("saga-flow").Step("reserve", "limit.reserve",
		workflow.CompensateWith("limit.release"), workflow.ReconcileWith("limit.lookup"),
		workflow.IdempotencyTemplate("reserve:${run.id}"), workflow.Secrets("payments-key"),
		workflow.CompensationRetry(workflow.MaxAttempts(5)), workflow.CompensationTimeout(10*time.Second),
		workflow.HighRisk(true), workflow.Timeout(5*time.Second), workflow.OnError("reserve")).Build()
	if err != nil {
		t.Fatal(err)
	}
	same(t, nodes(t, d)[0], doc{
		"id": "reserve", "type": "activity", "activity": "limit.reserve", "compensateWith": "limit.release", "reconcileWith": "limit.lookup",
		"idempotencyTemplate": "reserve:${run.id}", "secrets": []string{"payments-key"}, "compensationRetry": doc{"maxAttempts": 5},
		"compensationTimeoutMs": 10000, "highRisk": true, "timeoutMs": 5000, "onError": "reserve",
	})
}

func TestAParallelListsTheFirstNodeOfEachBranchAndContinuesWithTheNextNodeOfItsFlow(t *testing.T) {
	d, err := workflow.New("fan-out").
		Parallel("split", workflow.JoinAll,
			workflow.Flow().Step("card", "pay.card").Step("card-2", "pay.card2"),
			workflow.Flow().Step("stock", "stock.hold")).
		Step("done", "order.done").
		Build()
	if err != nil {
		t.Fatal(err)
	}
	same(t, d["nodes"], []doc{
		{"id": "split", "type": "parallel", "branches": []string{"card", "stock"}, "join": "all", "next": "done"},
		{"id": "done", "type": "activity", "activity": "order.done"},
		{"id": "card", "type": "activity", "activity": "pay.card", "next": "card-2"},
		{"id": "card-2", "type": "activity", "activity": "pay.card2"},
		{"id": "stock", "type": "activity", "activity": "stock.hold"},
	})
}

func TestASagaBeginsAtItsBodyHoldsNoEndAndContinuesWithTheNextNodeOfItsFlow(t *testing.T) {
	d, err := workflow.New("saga-flow").
		Saga("tx", workflow.Flow().Step("reserve", "a.reserve", workflow.CompensateWith("a.release")).Step("submit", "a.submit"),
			workflow.OnFailure(workflow.FailRun), workflow.OnCancel(workflow.AbortNoCompensation)).
		Step("notify", "a.notify").
		Build()
	if err != nil {
		t.Fatal(err)
	}
	same(t, nodes(t, d)[0], doc{"id": "tx", "type": "saga", "begin": "reserve", "onCompensationFailure": "fail", "cancelPolicy": "abort_no_compensation", "next": "notify"})
	var ids []string
	for _, n := range nodes(t, d) {
		ids = append(ids, n.(doc)["id"].(string))
	}
	if fmt.Sprint(ids) != "[tx notify reserve submit]" {
		t.Errorf("%v", ids)
	}
}

func TestWaitForAnEventInvokeAChildDecideAndEnd(t *testing.T) {
	d, err := workflow.New("mixed-flow").
		WaitForEvent("paid", "payment.confirmed", "input.orderId", time.Minute, workflow.OnTimeout("cancel"), workflow.Lookback(time.Second)).
		InvokeChild("ship", "shipping-flow", workflow.ChildVersion(2), workflow.ChildCancelPolicy(workflow.Detach), workflow.ChildInput("input.order")).
		Condition("big", "input.amount > 100", "review", "cancel").
		Step("review", "order.review", workflow.OnError("cancel")).
		Step("cancel", "order.cancel").
		End("done", workflow.OutputExpr("steps.ship.output")).
		Build()
	if err != nil {
		t.Fatal(err)
	}
	list := nodes(t, d)
	if _, has := list[2].(doc)["next"]; has {
		t.Error("a condition names both branches and has no next")
	}
	same(t, list[0], doc{"id": "paid", "type": "waitEvent", "eventType": "payment.confirmed", "correlationExpr": "input.orderId", "timeoutMs": 60000, "onTimeout": "cancel", "lookbackMs": 1000, "next": "ship"})
	same(t, list[1], doc{"id": "ship", "type": "child", "workflowId": "shipping-flow", "version": 2, "cancelPolicy": "detach", "input": "input.order", "next": "big"})
	same(t, list[5], doc{"id": "done", "type": "end", "outputExpr": "steps.ship.output"})
	if _, has := list[4].(doc)["next"]; has == false {
		t.Error("the step before an end links to it")
	}
}

func TestTriggersLabelsAndSchemasGoWhereTheDocumentKeepsThem(t *testing.T) {
	d, err := workflow.New("with-meta").
		Label("team", "payments").
		Trigger(workflow.Trigger{Kind: workflow.TriggerEvent, EventType: "order.created", Source: "api"}).
		Trigger(workflow.Trigger{Kind: workflow.TriggerManual}).
		InputSchema(doc{"type": "object"}).
		OutputSchema(doc{"type": "object"}).
		Step("a", "act.a").Build()
	if err != nil {
		t.Fatal(err)
	}
	same(t, d["metadata"], doc{"name": "with-meta", "labels": doc{"team": "payments"}})
	same(t, d["triggers"], []doc{{"kind": "event", "eventType": "order.created", "source": "api"}, {"kind": "manual"}})
	same(t, d["inputSchema"], doc{"type": "object"})
	same(t, d["outputSchema"], doc{"type": "object"})
}

func TestWhatTheServerWouldRefuseAndTheBuilderCanSeeIsRefusedHere(t *testing.T) {
	cases := []struct {
		name    string
		build   func() (doc, error)
		pattern string
	}{
		{"bad name", func() (doc, error) { return workflow.New("Bad Name").Step("a", "x").Build() }, "workflow name"},
		{"short name", func() (doc, error) { return workflow.New("ab").Step("a", "x").Build() }, "workflow name"},
		{"empty flow", func() (doc, error) { return workflow.New("empty-flow").Build() }, "at least one node"},
		{"duplicate ids", func() (doc, error) { return workflow.New("dup-ids").Step("a", "x").Step("a", "y").Build() }, "used twice"},
		{"duplicate in a nested flow", func() (doc, error) {
			return workflow.New("dup-nested").Step("a", "x").Parallel("p", workflow.JoinAll, workflow.Flow().Step("a", "y"), workflow.Flow().Step("b", "z")).Build()
		}, "used twice"},
		{"one branch", func() (doc, error) {
			return workflow.New("one-branch").Parallel("p", workflow.JoinAll, workflow.Flow().Step("a", "x")).Build()
		}, "two branches"},
		{"empty branch", func() (doc, error) {
			return workflow.New("empty-branch").Parallel("p", workflow.JoinAll, workflow.Flow().Step("a", "x"), workflow.Flow()).Build()
		}, "empty branch"},
		{"empty saga", func() (doc, error) { return workflow.New("empty-saga").Saga("s", workflow.Flow()).Build() }, "empty body"},
		{"end in a saga", func() (doc, error) {
			return workflow.New("saga-end").Saga("s", workflow.Flow().Step("a", "x").End("e")).Build()
		}, "end node"},
		{"irreversible with an undo", func() (doc, error) {
			return workflow.New("both").Step("a", "x", workflow.Irreversible(true), workflow.CompensateWith("undo")).Build()
		}, "irreversible"},
		{"bad secret", func() (doc, error) {
			return workflow.New("bad-secret").Step("a", "x", workflow.Secrets("Bad Secret")).Build()
		}, "secret"},
		{"bad timer", func() (doc, error) { return workflow.New("bad-timer").Sleep("t", 0).Build() }, "duration"},
		{"sub-millisecond timer", func() (doc, error) { return workflow.New("bad-timer").Sleep("t", time.Microsecond).Build() }, "duration"},
		{"no activity", func() (doc, error) { return workflow.New("no-activity").Step("a", "").Build() }, "activity"},
		{"no node id", func() (doc, error) { return workflow.New("no-id").Step("", "x").Build() }, "id"},
		{"wait without a timeout", func() (doc, error) {
			return workflow.New("no-timeout").WaitForEvent("w", "e", "x", 0).Build()
		}, "timeout"},
		{"child without a name", func() (doc, error) { return workflow.New("no-child").InvokeChild("c", "").Build() }, "name of a workflow"},
		{"condition without an expression", func() (doc, error) { return workflow.New("no-expr").Condition("c", "", "a", "b").Build() }, "expression"},
		{"dangling condition", func() (doc, error) {
			return workflow.New("dangling").Condition("c", "true", "nowhere", "a").Step("a", "x").Build()
		}, "not a node"},
		{"dangling timeout", func() (doc, error) {
			return workflow.New("dangling-timeout").WaitForEvent("w", "e", "x", time.Millisecond, workflow.OnTimeout("nowhere")).Build()
		}, "not a node"},
		{"dangling error branch", func() (doc, error) {
			return workflow.New("dangling-error").Step("a", "x", workflow.OnError("nowhere")).Build()
		}, "not a node"},
		{"a schema that is not JSON", func() (doc, error) {
			return workflow.New("bad-schema").InputSchema(doc{"x": nan()}).Step("a", "x").Build()
		}, "finite"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			_, err := c.build()
			if !sagat.IsCode(err, sagat.CodeValidationFailed) || !regexp.MustCompile(c.pattern).MatchString(err.Error()) {
				t.Errorf("%v, want VALIDATION_FAILED mentioning %q", err, c.pattern)
			}
		})
	}
}

func nan() float64 {
	zero := 0.0
	return zero / zero
}

func TestTheFirstMistakeIsKeptAndTheCallsAfterItDoNothing(t *testing.T) {
	b := workflow.New("Bad Name")
	b.Step("a", "x").Step("a", "y")
	if b.Err() == nil || !strings.Contains(b.Err().Error(), "workflow name") {
		t.Errorf("%v", b.Err())
	}
	if _, err := b.Build(); err != b.Err() {
		t.Errorf("Build returns the mistake: %v", err)
	}
	if _, err := b.JSON(); err == nil {
		t.Error("JSON returns the mistake")
	}
}

func TestTheCanonicalTextSortsKeysAtEveryDepthAndLeavesOutWhitespace(t *testing.T) {
	text, err := workflow.New("canon-flow").Step("a", "act.a", workflow.Retry(workflow.MaxAttempts(2))).
		InputSchema(doc{"z": 1, "a": doc{"d": []any{2, doc{"z": 1, "y": 2}}, "c": nil}}).JSON()
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(text, `{"apiVersion":`) || strings.Contains(text, " ") || strings.Contains(text, "\n") {
		t.Errorf("%s", text)
	}
	if !strings.Contains(text, `"inputSchema":{"a":{"c":null,"d":[2,{"y":2,"z":1}]},"z":1}`) {
		t.Errorf("%s", text)
	}
	again, _ := workflow.New("canon-flow").Step("a", "act.a", workflow.Retry(workflow.MaxAttempts(2))).
		InputSchema(doc{"z": 1, "a": doc{"d": []any{2, doc{"z": 1, "y": 2}}, "c": nil}}).JSON()
	if again != text {
		t.Error("the same program writes the same text")
	}
}

func TestTheDocumentIsACopyTheCallerOwnsAndAnIntegerStaysAnInteger(t *testing.T) {
	b := workflow.New("copy-flow").Step("a", "act.a", workflow.Timeout(10*time.Minute))
	first, err := b.Build()
	if err != nil {
		t.Fatal(err)
	}
	nodes(t, first)[0].(doc)["activity"] = "tampered"
	second, _ := b.Build()
	if nodes(t, second)[0].(doc)["activity"] != "act.a" {
		t.Error("changing the document changed the builder")
	}
	text, _ := b.JSON()
	if !strings.Contains(text, `"timeoutMs":600000`) || strings.Contains(text, "600000.0") {
		t.Errorf("%s", text)
	}
	if reflect.TypeOf(nodes(t, second)[0].(doc)["timeoutMs"]).Kind() != reflect.Int64 {
		t.Error("an integer is an int64")
	}
}

func TestTheCanonicalTextIsRFC8785KeysByUTF16CodeUnitsAndNumbersAsECMAScriptPrintsThem(t *testing.T) {
	text, err := workflow.New("rfc-flow").
		Label("\U0001F600", "emoji").Label("\uFFFD", "replacement").Label("b", "plain").
		InputSchema(doc{"minimum": json.Number("1.0"), "maximum": json.Number("6e5"), "ratio": json.Number("0.1")}).
		Step("a", "act.a", workflow.Timeout(10*time.Minute)).
		JSON()
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"\"labels\":{\"b\":\"plain\",\"\U0001F600\":\"emoji\",\"\uFFFD\":\"replacement\"}",
		`"inputSchema":{"maximum":600000,"minimum":1,"ratio":0.1}`,
		`"timeoutMs":600000`,
	} {
		if !strings.Contains(text, want) {
			t.Errorf("%s is missing from %s", want, text)
		}
	}
}

func TestTheCanonicalTextRefusesAnIntegerBeyond2To53MinusOneAndBuildDoesNot(t *testing.T) {
	b := workflow.New("big-flow").InputSchema(doc{"maximum": json.Number("9007199254740993")}).Step("a", "act.a")
	if _, err := b.Build(); err != nil {
		t.Errorf("Build keeps the document: %v", err)
	}
	if _, err := b.JSON(); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("JSON: %v", err)
	}
	if _, err := workflow.New("big-flow").InputSchema(doc{"maximum": int64(1) << 60}).Step("a", "act.a").JSON(); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("an int64 beyond 2^53-1: %v", err)
	}
	if _, err := workflow.New("fine-flow").InputSchema(doc{"maximum": json.Number("9007199254740991")}).Step("a", "act.a").JSON(); err != nil {
		t.Errorf("%v", err)
	}
}

// The workflow "canonical-edges" of the builders scenario, and the text the golden file holds for it
// (tests/contract/sdk-conformance/expected/builders.workflows.json): the same bytes in every language.
func TestTheCanonicalTextOfTheEdgesWorkflowIsTheGoldenText(t *testing.T) {
	schema := `{"type": "object", "properties": {"10": {"type": "integer", "maximum": 600000},
		"9": {"type": "number", "multipleOf": 0.1, "minimum": -1e-07, "maximum": 1e+21},
		"a": {"type": "number", "exclusiveMinimum": 1e-06, "exclusiveMaximum": 12345.678, "default": 4.35},
		"\ud83d\ude00": {"type": "string", "maxLength": 5}, "\uff5e": {"type": "string", "minLength": 1},
		"z": {"type": "array", "items": {"type": ["null", "boolean", "string"]}, "maxItems": 3}}}`
	decoder := json.NewDecoder(strings.NewReader(schema))
	decoder.UseNumber()
	var input map[string]any
	if err := decoder.Decode(&input); err != nil {
		t.Fatal(err)
	}
	text, err := workflow.New("canonical-edges").
		Description("The canonical text must be the same bytes in every language: keys that JavaScript would reorder (integer-like ones), keys above U+FFFF against keys at U+FF5E, and numbers as ECMAScript prints them.").
		InputSchema(input).Step("only", "edges.only").JSON()
	if err != nil {
		t.Fatal(err)
	}
	want := `{"apiVersion":"sagat-octopus-orchestrator.io/v1","entrypoint":"only","inputSchema":{"properties":{"10":{"maximum":600000,"type":"integer"},` +
		`"9":{"maximum":1e+21,"minimum":-1e-7,"multipleOf":0.1,"type":"number"},"a":{"default":4.35,"exclusiveMaximum":12345.678,"exclusiveMinimum":0.000001,"type":"number"},` +
		`"z":{"items":{"type":["null","boolean","string"]},"maxItems":3,"type":"array"},"😀":{"maxLength":5,"type":"string"},"～":{"minLength":1,"type":"string"}},"type":"object"},` +
		`"kind":"Workflow","metadata":{"description":"The canonical text must be the same bytes in every language: keys that JavaScript would reorder (integer-like ones), keys above U+FFFF against keys at U+FF5E, and numbers as ECMAScript prints them.","name":"canonical-edges"},` +
		`"nodes":[{"activity":"edges.only","id":"only","type":"activity"}]}`
	if text != want {
		t.Errorf("\n got %s\nwant %s", text, want)
	}
}

// SPEC section 7: a Flow is read when Build() is called, not when it is joined.
func TestAStepAddedToAJoinedBranchOrSagaBodyAfterTheJoinIsPartOfTheDocument(t *testing.T) {
	left := workflow.Flow().Step("card", "pay.card")
	right := workflow.Flow().Step("stock", "stock.hold")
	body := workflow.Flow().Step("reserve", "a.reserve")
	inner := workflow.Flow().Step("inner-1", "x.one")
	b := workflow.New("late-flow").
		Parallel("split", workflow.JoinAll, left, right).
		Saga("tx", body).
		Step("done", "order.done")
	// Added after the join, to a branch, to a body, and to a flow nested in a branch that was joined.
	left.Step("card-2", "pay.card2")
	body.Step("submit", "a.submit")
	left.Saga("nested", inner)
	inner.Step("inner-2", "x.two")
	right.Step("late-right", "stock.late")

	d, err := b.Build()
	if err != nil {
		t.Fatal(err)
	}
	next := map[string]any{}
	var ids []string
	for _, n := range nodes(t, d) {
		node := n.(doc)
		id := node["id"].(string)
		ids = append(ids, id)
		next[id] = node["next"]
	}
	if fmt.Sprint(ids) != "[split tx done card card-2 nested inner-1 inner-2 stock late-right reserve submit]" {
		t.Errorf("%v", ids)
	}
	for id, want := range map[string]any{"card": "card-2", "card-2": "nested", "nested": nil, "stock": "late-right", "late-right": nil,
		"reserve": "submit", "submit": nil, "inner-1": "inner-2", "inner-2": nil, "split": "tx", "tx": "done"} {
		if next[id] != want {
			t.Errorf("next of %s is %v, want %v", id, next[id], want)
		}
	}
	// The branches and the saga still begin where they began.
	list := nodes(t, d)
	same(t, list[0], doc{"id": "split", "type": "parallel", "branches": []string{"card", "stock"}, "join": "all", "next": "tx"})
	same(t, list[1], doc{"id": "tx", "type": "saga", "begin": "reserve", "next": "done"})
	// Built again after yet another step, it is read again.
	right.Step("later-still", "stock.later")
	again, err := b.Build()
	if err != nil || len(nodes(t, again)) != len(list)+1 {
		t.Errorf("%d nodes, %v", len(nodes(t, again)), err)
	}
}

func TestAMistakeInAStepAddedToAJoinedFlowAfterTheJoinIsRaisedByBuild(t *testing.T) {
	cases := map[string]func(branch, other *workflow.Builder){
		"an id used in another flow":   func(branch, other *workflow.Builder) { branch.Step("stock", "x.dup") },
		"an id used twice in its flow": func(branch, other *workflow.Builder) { branch.Step("card-2", "x.a").Step("card-2", "x.b") },
		"a step with no activity":      func(branch, other *workflow.Builder) { branch.Step("card-2", "") },
		"an irreversible compensation": func(branch, other *workflow.Builder) {
			branch.Step("card-2", "x", workflow.Irreversible(true), workflow.CompensateWith("u"))
		},
		"a dangling reference": func(branch, other *workflow.Builder) { branch.Step("card-2", "x", workflow.OnError("nowhere")) },
	}
	for name, mistake := range cases {
		t.Run(name, func(t *testing.T) {
			branch := workflow.Flow().Step("card", "pay.card")
			other := workflow.Flow().Step("stock", "stock.hold")
			b := workflow.New("late-mistake").Parallel("split", workflow.JoinAll, branch, other)
			if b.Err() != nil {
				t.Fatalf("%v", b.Err())
			}
			mistake(branch, other)
			if _, err := b.Build(); !sagat.IsCode(err, sagat.CodeValidationFailed) {
				t.Errorf("Build did not raise it: %v", err)
			}
			if _, err := b.JSON(); !sagat.IsCode(err, sagat.CodeValidationFailed) {
				t.Errorf("JSON did not raise it: %v", err)
			}
		})
	}
	// An end node added to the body of a saga after the join.
	body := workflow.Flow().Step("a", "x")
	b := workflow.New("late-end").Saga("s", body)
	body.End("e")
	if _, err := b.Build(); !sagat.IsCode(err, sagat.CodeValidationFailed) || !strings.Contains(err.Error(), "end node") {
		t.Errorf("an end added to a saga body after the join: %v", err)
	}
}

func TestAnIDUsedTwiceInOneFlowIsRaisedWhenTheNodeIsAdded(t *testing.T) {
	b := workflow.New("dup-now").Step("a", "x").Step("a", "y")
	if b.Err() == nil || !strings.Contains(b.Err().Error(), "used twice") {
		t.Errorf("Err() is %v before Build", b.Err())
	}
	branch := workflow.Flow().Step("a", "x").Step("a", "y")
	if branch.Err() == nil {
		t.Error("the same in a nested flow")
	}
}

func TestTheCanonicalTextRefusesAWholeNumberBeyond2To53MinusOneWhateverWayItWasWrittenAndBuildKeepsIt(t *testing.T) {
	for name, value := range map[string]any{
		"an integer": json.Number("9007199254740993"), "an integer written as a float": json.Number("9007199254740992.0"), "1e20": json.Number("1e20"),
		"a float64 1e20": float64(1e20), "a float64 2^53": float64(9007199254740992), "an int64": int64(1) << 60,
	} {
		b := workflow.New("unsafe-flow").InputSchema(doc{"type": "object", "maximum": value}).Step("a", "x.y")
		if _, err := b.Build(); err != nil {
			t.Errorf("%s: Build keeps the document: %v", name, err)
		}
		if _, err := b.JSON(); !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("%s: JSON wrote it: %v", name, err)
		}
	}
	for name, value := range map[string]any{
		"1e21": json.Number("1e21"), "a float64 1e21": float64(1e21), "a fraction": json.Number("0.5"), "the largest exact": json.Number("9007199254740991"),
		"4.35": json.Number("4.35"), "a float64 4.35": 4.35,
	} {
		if _, err := workflow.New("safe-flow").InputSchema(doc{"type": "object", "maximum": value}).Step("a", "x.y").JSON(); err != nil {
			t.Errorf("%s: %v", name, err)
		}
	}
}
