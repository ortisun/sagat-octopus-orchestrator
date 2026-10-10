package main

import (
	"fmt"
	"sort"
	"strings"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/workflow"
)

func asObject(value any) object {
	obj, _ := value.(object)
	return obj
}

func asList(value any) []any {
	list, _ := value.([]any)
	return list
}

func asString(value any) string {
	s, _ := value.(string)
	return s
}

func asMillis(value any) time.Duration {
	n, ok := value.(interface{ Int64() (int64, error) })
	if !ok {
		panic(fmt.Sprintf("a number was expected, got %v", value))
	}
	ms, err := n.Int64()
	if err != nil {
		panic(fmt.Sprintf("an integer was expected, got %v", value))
	}
	return time.Duration(ms) * time.Millisecond
}

func asInt(value any) int {
	return int(asMillis(value) / time.Millisecond)
}

func asBool(value any) bool {
	b, ok := value.(bool)
	if !ok {
		panic(fmt.Sprintf("a boolean was expected, got %v", value))
	}
	return b
}

func sortedKeys(m object) []string {
	keys := make([]string, 0, len(m))
	for key := range m {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}

func retryOptions(m object) []workflow.RetryOption {
	var opts []workflow.RetryOption
	for _, key := range sortedKeys(m) {
		value := m[key]
		switch key {
		case "maxAttempts":
			opts = append(opts, workflow.MaxAttempts(asInt(value)))
		case "initialDelayMs":
			opts = append(opts, workflow.InitialDelay(asMillis(value)))
		case "maxDelayMs":
			opts = append(opts, workflow.MaxDelay(asMillis(value)))
		case "strategy":
			opts = append(opts, workflow.WithStrategy(workflow.Strategy(asString(value))))
		case "jitter":
			opts = append(opts, workflow.Jitter(asBool(value)))
		default:
			panic(fmt.Sprintf("unknown retry option %q", key))
		}
	}
	return opts
}

func stepOptions(m object) []workflow.StepOption {
	var opts []workflow.StepOption
	for _, key := range sortedKeys(m) {
		value := m[key]
		switch key {
		case "retry":
			opts = append(opts, workflow.Retry(retryOptions(asObject(value))...))
		case "timeoutMs":
			opts = append(opts, workflow.Timeout(asMillis(value)))
		case "compensateWith":
			opts = append(opts, workflow.CompensateWith(asString(value)))
		case "reconcileWith":
			opts = append(opts, workflow.ReconcileWith(asString(value)))
		case "idempotencyTemplate":
			opts = append(opts, workflow.IdempotencyTemplate(asString(value)))
		case "secrets":
			var names []string
			for _, name := range asList(value) {
				names = append(names, asString(name))
			}
			opts = append(opts, workflow.Secrets(names...))
		case "compensationRetry":
			opts = append(opts, workflow.CompensationRetry(retryOptions(asObject(value))...))
		case "compensationTimeoutMs":
			opts = append(opts, workflow.CompensationTimeout(asMillis(value)))
		case "irreversible":
			opts = append(opts, workflow.Irreversible(asBool(value)))
		case "highRisk":
			opts = append(opts, workflow.HighRisk(asBool(value)))
		case "onError":
			opts = append(opts, workflow.OnError(asString(value)))
		default:
			panic(fmt.Sprintf("unknown step option %q", key))
		}
	}
	return opts
}

// apply plays a program of builder ops on a builder.
func apply(b *workflow.Builder, ops []any) {
	for _, item := range ops {
		op := asObject(item)
		id := asString(op["id"])
		options := asObject(op["options"])
		switch asString(op["op"]) {
		case "step":
			b.Step(id, asString(op["activity"]), stepOptions(options)...)
		case "sleep":
			b.Sleep(id, asMillis(op["durationMs"]))
		case "waitForEvent":
			var opts []workflow.WaitOption
			for _, key := range sortedKeys(options) {
				switch key {
				case "eventType", "correlationExpr", "timeoutMs":
				case "onTimeout":
					opts = append(opts, workflow.OnTimeout(asString(options[key])))
				case "lookbackMs":
					opts = append(opts, workflow.Lookback(asMillis(options[key])))
				default:
					panic(fmt.Sprintf("unknown waitForEvent option %q", key))
				}
			}
			b.WaitForEvent(id, asString(options["eventType"]), asString(options["correlationExpr"]), asMillis(options["timeoutMs"]), opts...)
		case "invokeChild":
			var opts []workflow.ChildOption
			for _, key := range sortedKeys(options) {
				switch key {
				case "version":
					opts = append(opts, workflow.ChildVersion(asInt(options[key])))
				case "input":
					opts = append(opts, workflow.ChildInput(asString(options[key])))
				case "cancelPolicy":
					opts = append(opts, workflow.ChildCancelPolicy(workflow.CancelPolicy(asString(options[key]))))
				default:
					panic(fmt.Sprintf("unknown invokeChild option %q", key))
				}
			}
			b.InvokeChild(id, asString(op["workflow"]), opts...)
		case "condition":
			b.Condition(id, asString(op["expression"]), asString(op["ifTrue"]), asString(op["ifFalse"]))
		case "parallel":
			var branches []*workflow.Builder
			for _, branch := range asList(op["branches"]) {
				flow := workflow.Flow()
				apply(flow, asList(branch))
				branches = append(branches, flow)
			}
			b.Parallel(id, workflow.Join(asString(op["join"])), branches...)
		case "saga":
			var opts []workflow.SagaOption
			for _, key := range sortedKeys(options) {
				switch key {
				case "onCompensationFailure":
					opts = append(opts, workflow.OnFailure(workflow.OnCompensationFailure(asString(options[key]))))
				case "cancelPolicy":
					opts = append(opts, workflow.OnCancel(workflow.SagaCancelPolicy(asString(options[key]))))
				default:
					panic(fmt.Sprintf("unknown saga option %q", key))
				}
			}
			body := workflow.Flow()
			apply(body, asList(op["body"]))
			b.Saga(id, body, opts...)
		case "end":
			var opts []workflow.EndOption
			for _, key := range sortedKeys(options) {
				if key != "outputExpr" {
					panic(fmt.Sprintf("unknown end option %q", key))
				}
				opts = append(opts, workflow.OutputExpr(asString(options[key])))
			}
			b.End(id, opts...)
		default:
			panic(fmt.Sprintf("unknown builder op %q", asString(op["op"])))
		}
	}
}

func builderOf(definition object) *workflow.Builder {
	b := workflow.New(asString(definition["name"]))
	if description := asString(definition["description"]); description != "" {
		b.Description(description)
	}
	labels := asObject(definition["labels"])
	for _, key := range sortedKeys(labels) {
		b.Label(key, asString(labels[key]))
	}
	for _, item := range asList(definition["triggers"]) {
		trigger := asObject(item)
		t := workflow.Trigger{}
		for key, value := range trigger {
			switch key {
			case "kind":
				t.Kind = workflow.TriggerKind(asString(value))
			case "eventType":
				t.EventType = asString(value)
			case "source":
				t.Source = asString(value)
			case "endpoint":
				t.Endpoint = asString(value)
			case "connector":
				t.Connector = asString(value)
			case "filter":
				t.Filter = asString(value)
			case "cron":
				t.Cron = asString(value)
			case "timezone":
				t.Timezone = asString(value)
			default:
				panic(fmt.Sprintf("unknown trigger member %q", key))
			}
		}
		b.Trigger(t)
	}
	if schema := asObject(definition["inputSchema"]); schema != nil {
		b.InputSchema(schema)
	}
	if schema := asObject(definition["outputSchema"]); schema != nil {
		b.OutputSchema(schema)
	}
	apply(b, asList(definition["program"]))
	return b
}

// build builds every workflow of the scenario with the SDK's builders, and records what the
// builders raise for the invalid ones.
func build(scenario object) (object, error) {
	workflows := object{}
	for key, definition := range asObject(scenario["workflows"]) {
		document, err := builderOf(asObject(definition)).Build()
		if err != nil {
			return nil, fmt.Errorf("workflow %s: %w", key, err)
		}
		workflows[key] = document
	}
	out := object{"workflows": workflows}
	problems := object{}
	for key, item := range asObject(scenario["invalidWorkflows"]) {
		invalid := asObject(item)
		_, err := builderOf(asObject(invalid["workflow"])).Build()
		contains := asString(invalid["contains"])
		var se *sagat.Error
		switch {
		case err == nil:
			problems[key] = object{"raised": false}
		case asError(err, &se):
			problems[key] = object{"raised": true, "code": string(se.Code), "mentions": contains == "" || strings.Contains(se.Message, contains)}
		default:
			problems[key] = object{"raised": true, "code": "UNEXPECTED", "mentions": false}
		}
	}
	if len(problems) > 0 {
		out["errors"] = problems
	}
	return out, nil
}
