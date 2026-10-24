package main

import (
	"encoding/json"
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

// asCount reads a small non-negative integer the scenario wrote (a limit, a position); absent is zero.
func asCount(value any) int {
	n, ok := value.(interface{ Int64() (int64, error) })
	if !ok {
		return 0
	}
	count, err := n.Int64()
	if err != nil || count < 0 {
		return 0
	}
	return int(count)
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
		case "outputSchema":
			opts = append(opts, workflow.OutputSchema(asObject(value)))
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
		case "database":
			var database workflow.DatabaseOptions
			raw, err := json.Marshal(options)
			if err != nil {
				panic(sagat.Invalid("invalid managed database operation"))
			}
			if err = json.Unmarshal(raw, &database); err != nil {
				panic(sagat.Invalid("invalid managed database operation"))
			}
			b.Database(id, database)
		case "step":
			b.Step(id, asString(op["activity"]), stepOptions(options)...)
		case "script":
			var opts []workflow.ScriptOption
			for _, key := range sortedKeys(options) {
				switch key {
				case "language":
					opts = append(opts, workflow.ScriptLanguage(asString(options[key])))
				case "retry":
					opts = append(opts, workflow.ScriptRetry(retryOptions(asObject(options[key]))...))
				case "compensateWith":
					opts = append(opts, workflow.ScriptCompensateWith(asString(options[key])))
				case "secrets":
					var names []string
					for _, value := range asList(options[key]) {
						names = append(names, asString(value))
					}
					opts = append(opts, workflow.ScriptSecrets(names...))
				case "onError":
					opts = append(opts, workflow.ScriptOnError(asString(options[key])))
				case "outputSchema":
					opts = append(opts, workflow.ScriptOutputSchema(asObject(options[key])))
				default:
					panic(fmt.Sprintf("unknown script option %q", key))
				}
			}
			b.Script(id, asString(op["artifactDigest"]), opts...)
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
				case "outputSchema":
					opts = append(opts, workflow.WaitOutputSchema(asObject(options[key])))
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
				case "outputSchema":
					opts = append(opts, workflow.ChildOutputSchema(asObject(options[key])))
				default:
					panic(fmt.Sprintf("unknown invokeChild option %q", key))
				}
			}
			b.InvokeChild(id, asString(op["workflow"]), opts...)
		case "condition":
			b.Condition(id, asString(op["expression"]), asString(op["ifTrue"]), asString(op["ifFalse"]))
		case "switch":
			items := asList(op["cases"])
			cases := make([]workflow.SwitchCase, 0, len(items))
			for _, item := range items {
				caseObject := asObject(item)
				cases = append(cases, workflow.SwitchCase{ID: asString(caseObject["id"]), When: asString(caseObject["when"]), Target: asString(caseObject["target"])})
			}
			b.Switch(id, cases, asString(op["default"]))
		case "forEach":
			b.ForEach(id, asString(op["items"]), asString(op["body"]), asString(op["result"]), asInt(op["maxItems"]), asInt(op["maxConcurrency"]))
		case "map":
			b.Map(id, asString(op["items"]), asString(op["body"]), asString(op["result"]), asInt(op["maxItems"]), asInt(op["maxConcurrency"]))
		case "while", "repeatUntil":
			var opts []workflow.LoopOption
			if op["delayMs"] != nil {
				opts = append(opts, workflow.LoopDelay(asMillis(op["delayMs"])))
			}
			if op["onError"] != nil {
				opts = append(opts, workflow.LoopOnError(asString(op["onError"])))
			}
			if asString(op["op"]) == "while" {
				b.While(id, asString(op["condition"]), asString(op["body"]), asString(op["initial"]), asString(op["carry"]), asInt(op["maxIterations"]), asMillis(op["timeoutMs"]), opts...)
			} else {
				b.RepeatUntil(id, asString(op["condition"]), asString(op["body"]), asString(op["initial"]), asString(op["carry"]), asInt(op["maxIterations"]), asMillis(op["timeoutMs"]), opts...)
			}
		case "approval":
			policyObject := asObject(op["policy"])
			policy := workflow.ApprovalPolicy{}
			if value, present := policyObject["subjects"]; present {
				items := asList(value)
				policy.Subjects = make([]string, 0, len(items))
				for _, subject := range items {
					policy.Subjects = append(policy.Subjects, asString(subject))
				}
			}
			if value, present := policyObject["roles"]; present {
				items := asList(value)
				policy.Roles = make([]workflow.ApprovalRole, 0, len(items))
				for _, role := range items {
					policy.Roles = append(policy.Roles, workflow.ApprovalRole(asString(role)))
				}
			}
			if value, present := policyObject["excludeInitiator"]; present {
				exclude := asBool(value)
				policy.ExcludeInitiator = &exclude
			}
			b.Approval(id, policy, asMillis(op["timeoutMs"]), asString(op["onApprove"]), asString(op["onReject"]), asString(op["onExpire"]))
		case "continueAsNew":
			var opts []workflow.ContinuationOption
			if state, present := op["state"]; present {
				opts = append(opts, workflow.ContinuationState(asString(state)))
			}
			b.ContinueAsNew(id, asString(op["input"]), opts...)
		case "return":
			b.Return(id, asString(op["result"]))
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
	if limits := asObject(definition["limits"]); limits != nil {
		var opts []workflow.LimitOption
		if value, present := limits["maxInstances"]; present {
			opts = append(opts, workflow.MaxInstances(asInt(value)))
		}
		if value, present := limits["maxDepth"]; present {
			opts = append(opts, workflow.MaxDepth(asInt(value)))
		}
		b.Limits(opts...)
	}
	if state := asObject(definition["state"]); state != nil {
		queries := make(map[string]workflow.QueryDefinition)
		for name, value := range asObject(state["queries"]) {
			query := asObject(value)
			queries[name] = workflow.QueryDefinition{
				ArgsSchema: asObject(query["argsSchema"]), ResultSchema: asObject(query["resultSchema"]),
				Expression: asString(query["expression"]), Disclosure: workflow.DisclosurePolicy(asString(query["disclosure"])),
			}
		}
		updates := make(map[string]workflow.UpdateDefinition)
		for name, value := range asObject(state["updates"]) {
			update := asObject(value)
			updates[name] = workflow.UpdateDefinition{
				ArgsSchema: asObject(update["argsSchema"]), Validate: asString(update["validate"]),
				State: asString(update["state"]), Result: asString(update["result"]),
				ResultSchema: asObject(update["resultSchema"]), Disclosure: workflow.DisclosurePolicy(asString(update["disclosure"])),
			}
		}
		b.State(workflow.StateDefinition{
			Schema: asObject(state["schema"]), Initial: asString(state["initial"]), Queries: queries, Updates: updates,
		})
	}
	if policy := asObject(definition["chainPolicy"]); policy != nil {
		b.ChainPolicy(asInt(policy["maxSegments"]), asMillis(policy["timeoutMs"]))
	}
	for _, name := range sortedKeys(asObject(definition["bodies"])) {
		bodyDefinition := asObject(asObject(definition["bodies"])[name])
		body := workflow.Flow()
		apply(body, asList(bodyDefinition["program"]))
		b.Body(name, body)
	}
	apply(b, asList(definition["program"]))
	return b
}

// Numeric fixture values that cannot inhabit a typed public option are validation errors.
func tryBuilderOf(definition object) (builder *workflow.Builder, err error) {
	defer func() {
		if problem := recover(); problem != nil {
			if validation, ok := problem.(*sagat.Error); ok {
				err = validation
			} else {
				panic(problem)
			}
		}
	}()
	builder = builderOf(definition)
	return
}

// build builds every workflow of the scenario with the SDK's builders, and records what the
// builders raise for the invalid ones.
func build(scenario object) (object, error) {
	workflows, canonical := object{}, object{}
	for key, definition := range asObject(scenario["workflows"]) {
		builder, err := tryBuilderOf(asObject(definition))
		if err != nil {
			return nil, err
		}
		document, err := builder.Build()
		if err != nil {
			return nil, fmt.Errorf("workflow %s: %w", key, err)
		}
		workflows[key] = document
		// The canonical text (toJson, RFC 8785): the harness compares these bytes across the SDKs.
		text, err := builder.JSON()
		if err != nil {
			return nil, fmt.Errorf("workflow %s: canonical text: %w", key, err)
		}
		canonical[key] = text
	}
	out := object{"workflows": workflows, "canonical": canonical}
	problems := object{}
	for key, item := range asObject(scenario["invalidWorkflows"]) {
		invalid := asObject(item)
		// Built with the builders and written as canonical text: the refusal may come from either.
		builder, err := tryBuilderOf(asObject(invalid["workflow"]))
		if err == nil {
			_, err = builder.Build()
		}
		if err == nil {
			_, err = builder.JSON()
		}
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
