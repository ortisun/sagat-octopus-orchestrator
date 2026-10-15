// Package workflow builds the workflow IR (workflow.schema.json): pure, deterministic constructors
// with no side effect. Build returns the document the server validates and versions, and JSON its
// canonical text (UTF-8, keys sorted, no insignificant whitespace), byte-identical to the one the
// other SDKs write for the same program (sdks/SPEC.md, section 7).
//
// A Builder is a chain of nodes: the body of a workflow, of a parallel branch or of a saga. The
// methods return the builder so that calls chain; the first mistake the builder can see (a bad name,
// a duplicate id, a parallel with one branch...) is remembered, the calls after it do nothing, and
// Build, JSON and Err return it as a *sagat.Error with the code VALIDATION_FAILED. A mistake is
// therefore raised by the call that adds the node, and reported where the document is asked for.
//
//	doc, err := workflow.New("order-flow").
//		Description("charges and ships").
//		Step("charge", "payments.charge", workflow.Retry(workflow.MaxAttempts(3))).
//		Sleep("pause", 2*time.Second).
//		Step("ship", "shipping.ship").
//		Build()
package workflow

import (
	"regexp"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

type node = map[string]any

// SwitchCase is one ordered branch of a switch decision.
type SwitchCase struct {
	ID     string
	When   string
	Target string
}

var namePattern = regexp.MustCompile(`^[a-z][a-z0-9-]{2,63}$`)

// Builder builds a workflow, or a nested flow (see Flow).
type Builder struct {
	name        string
	description *string
	labels      map[string]string
	triggers    []Trigger
	input       map[string]any
	output      map[string]any
	bodies      map[string]*Builder

	nodes    []node
	embedded []*Builder
	ids      map[string]struct{}
	err      error

	// sagaID is the id of the saga whose body this flow is, once it was joined as one.
	sagaID string
}

// New starts a workflow definition. The name is lower case letters, digits and dashes, 3 to 64,
// starting with a letter.
func New(name string) *Builder {
	b := &Builder{name: name, labels: map[string]string{}, bodies: map[string]*Builder{}, ids: map[string]struct{}{}}
	if !namePattern.MatchString(name) {
		b.fail(sagat.Invalid("a workflow name is lower case letters, digits and dashes, 3 to 64: %q", name))
	}
	return b
}

// Flow starts a nested flow: the body of a Saga or a branch of Parallel. It has no name and cannot
// be built on its own.
func Flow() *Builder { return &Builder{labels: map[string]string{}, ids: map[string]struct{}{}} }

func (b *Builder) fail(err error) *Builder {
	if b.err == nil {
		b.err = err
	}
	return b
}

// Err returns the first mistake the builder saw, or nil.
func (b *Builder) Err() error { return b.err }

func (b *Builder) push(n node) *Builder {
	if b.err != nil {
		return b
	}
	id, _ := n["id"].(string)
	if id == "" {
		return b.fail(sagat.Invalid("a node needs an id"))
	}
	// An id is checked when its node is added to its flow, and again by Build over the whole document.
	if _, twice := b.ids[id]; twice {
		return b.fail(sagat.Invalid("the node id %q is used twice", id))
	}
	if b.sagaID != "" && n["type"] == "end" {
		return b.fail(sagat.Invalid("saga %s cannot hold an end node", b.sagaID))
	}
	b.ids[id] = struct{}{}
	b.nodes = append(b.nodes, n)
	return b
}

// ---- the metadata of the workflow

// Description sets the description of the workflow.
func (b *Builder) Description(text string) *Builder {
	b.description = &text
	return b
}

// Label sets a label of the workflow.
func (b *Builder) Label(key, value string) *Builder {
	b.labels[key] = value
	return b
}

// Trigger adds a trigger.
func (b *Builder) Trigger(t Trigger) *Builder {
	b.triggers = append(b.triggers, t)
	return b
}

// InputSchema sets the JSON schema of the run input.
func (b *Builder) InputSchema(schema map[string]any) *Builder {
	b.input = schema
	return b
}

// OutputSchema sets the JSON schema of the run output.
func (b *Builder) OutputSchema(schema map[string]any) *Builder {
	b.output = schema
	return b
}

// ---- the nodes

// Step adds an activity executed by a worker.
func (b *Builder) Step(id, activity string, opts ...StepOption) *Builder {
	if b.err != nil {
		return b
	}
	if activity == "" {
		return b.fail(sagat.Invalid("step %s needs an activity name", id))
	}
	spec := &stepSpec{fields: map[string]any{}}
	for _, opt := range opts {
		opt(spec)
	}
	if spec.err != nil {
		return b.fail(spec.err)
	}
	if irreversible, _ := spec.fields["irreversible"].(bool); irreversible && spec.fields["compensateWith"] != nil && spec.fields["compensateWith"] != "" {
		return b.fail(sagat.Invalid("step %s cannot be irreversible and have a compensation", id))
	}
	n := node{"id": id, "type": "activity", "activity": activity}
	for key, value := range spec.fields {
		n[key] = value
	}
	return b.push(n)
}

// Sleep adds a timer: waits for a duration of at least a millisecond. A timer, not a process:
// nothing is kept alive.
func (b *Builder) Sleep(id string, d time.Duration) *Builder {
	if b.err != nil {
		return b
	}
	if millis(d) < 1 {
		return b.fail(sagat.Invalid("sleep %s needs a duration of at least 1 ms", id))
	}
	return b.push(node{"id": id, "type": "timer", "durationMs": millis(d)})
}

// WaitForEvent adds a wait for an event of a type whose correlation key is the value of
// correlationExpr (an expression over the run's data), or for the timeout.
func (b *Builder) WaitForEvent(id, eventType, correlationExpr string, timeout time.Duration, opts ...WaitOption) *Builder {
	if b.err != nil {
		return b
	}
	if eventType == "" || correlationExpr == "" {
		return b.fail(sagat.Invalid("waitForEvent %s needs an eventType and a correlationExpr", id))
	}
	if millis(timeout) < 1 {
		return b.fail(sagat.Invalid("waitForEvent %s needs a timeout of at least 1 ms", id))
	}
	n := node{"id": id, "type": "waitEvent", "eventType": eventType, "correlationExpr": correlationExpr, "timeoutMs": millis(timeout)}
	for _, opt := range opts {
		opt(n)
	}
	return b.push(n)
}

// InvokeChild adds the start of another workflow of the namespace, by name, and waits for it.
func (b *Builder) InvokeChild(id, workflow string, opts ...ChildOption) *Builder {
	if b.err != nil {
		return b
	}
	if workflow == "" {
		return b.fail(sagat.Invalid("invokeChild %s needs the name of a workflow", id))
	}
	n := node{"id": id, "type": "child", "workflowId": workflow}
	for _, opt := range opts {
		opt(n)
	}
	return b.push(n)
}

// Condition adds a decision over the run's data. It names both of its branches and has no next
// node.
func (b *Builder) Condition(id, expression, ifTrue, ifFalse string) *Builder {
	if b.err != nil {
		return b
	}
	if expression == "" {
		return b.fail(sagat.Invalid("condition %s needs an expression", id))
	}
	return b.push(node{"id": id, "type": "condition", "expression": expression, "ifTrue": ifTrue, "ifFalse": ifFalse})
}

// Switch adds an ordered decision that follows the first matching case, or its default target. It
// has no next node.
func (b *Builder) Switch(id string, cases []SwitchCase, defaultTarget string) *Builder {
	if b.err != nil {
		return b
	}
	if len(cases) < 1 || len(cases) > 32 {
		return b.fail(sagat.Invalid("switch %s needs between 1 and 32 cases", id))
	}
	if defaultTarget == "" {
		return b.fail(sagat.Invalid("switch %s needs a default target", id))
	}
	caseIDs := make(map[string]struct{}, len(cases))
	copied := make([]any, 0, len(cases))
	for _, item := range cases {
		if item.ID == "" {
			return b.fail(sagat.Invalid("switch %s has a case without an id", id))
		}
		if _, exists := caseIDs[item.ID]; exists {
			return b.fail(sagat.Invalid("switch %s uses case id %q twice", id, item.ID))
		}
		if item.When == "" {
			return b.fail(sagat.Invalid("switch %s case %q needs a when expression", id, item.ID))
		}
		if item.Target == "" {
			return b.fail(sagat.Invalid("switch %s case %q needs a target", id, item.ID))
		}
		caseIDs[item.ID] = struct{}{}
		copied = append(copied, map[string]any{"id": item.ID, "when": item.When, "target": item.Target})
	}
	return b.push(node{"id": id, "type": "switch", "cases": copied, "default": defaultTarget})
}

// ForEach invokes a named body once for every item captured from items.
func (b *Builder) ForEach(id, items, body, result string, maxItems, maxConcurrency int) *Builder {
	if b.err != nil {
		return b
	}
	if items == "" || body == "" || result == "" {
		return b.fail(sagat.Invalid("forEach %s needs items, body and result expressions", id))
	}
	if maxItems < 1 || maxItems > 10000 {
		return b.fail(sagat.Invalid("forEach %s maxItems must be between 1 and 10000", id))
	}
	if maxConcurrency < 1 || maxConcurrency > 256 {
		return b.fail(sagat.Invalid("forEach %s maxConcurrency must be between 1 and 256", id))
	}
	return b.push(node{"id": id, "type": "forEach", "items": items, "body": body, "result": result, "maxItems": maxItems, "maxConcurrency": maxConcurrency})
}

// Map is value-returning collection authoring; it emits the same durable forEach node.
func (b *Builder) Map(id, items, body, result string, maxItems, maxConcurrency int) *Builder {
	return b.ForEach(id, items, body, result, maxItems, maxConcurrency)
}

// Parallel adds branches that run at once: JoinAll waits for every one, JoinAny for the first. There
// are at least two, none empty.
func (b *Builder) Parallel(id string, join Join, branches ...*Builder) *Builder {
	if b.err != nil {
		return b
	}
	if len(branches) < 2 {
		return b.fail(sagat.Invalid("parallel %s needs at least two branches", id))
	}
	heads := make([]any, 0, len(branches))
	for _, branch := range branches {
		if branch == nil || len(branch.nodes) == 0 {
			if branch != nil && branch.err != nil {
				return b.fail(branch.err)
			}
			return b.fail(sagat.Invalid("parallel %s has an empty branch", id))
		}
		if branch.err != nil {
			return b.fail(branch.err)
		}
		heads = append(heads, branch.nodes[0]["id"])
	}
	b.push(node{"id": id, "type": "parallel", "branches": heads, "join": string(join)})
	if b.err == nil {
		b.embedded = append(b.embedded, branches...)
	}
	return b
}

// Saga adds a body that is compensated, in reverse order of completion, when it fails or the run is
// cancelled; a step in it names its undo with CompensateWith. The body holds no End.
func (b *Builder) Saga(id string, body *Builder, opts ...SagaOption) *Builder {
	if b.err != nil {
		return b
	}
	if body == nil || len(body.nodes) == 0 {
		if body != nil && body.err != nil {
			return b.fail(body.err)
		}
		return b.fail(sagat.Invalid("saga %s has an empty body", id))
	}
	if body.err != nil {
		return b.fail(body.err)
	}
	for _, n := range body.nodes {
		if n["type"] == "end" {
			return b.fail(sagat.Invalid("saga %s cannot hold an end node", id))
		}
	}
	n := node{"id": id, "type": "saga", "begin": body.nodes[0]["id"]}
	for _, opt := range opts {
		opt(n)
	}
	b.push(n)
	if b.err == nil {
		body.sagaID = id
		b.embedded = append(b.embedded, body)
	}
	return b
}

// End adds an explicit end of the flow, optionally with the expression of the run's output.
func (b *Builder) End(id string, opts ...EndOption) *Builder {
	if b.err != nil {
		return b
	}
	n := node{"id": id, "type": "end"}
	for _, opt := range opts {
		opt(n)
	}
	return b.push(n)
}

// Return closes a named body with its result. It is not a workflow terminal.
func (b *Builder) Return(id, result string) *Builder {
	if result == "" {
		return b.fail(sagat.Invalid("return %s needs a result expression", id))
	}
	return b.push(node{"id": id, "type": "return", "result": result})
}

// Body registers a closed body by name. The body is read when Build runs.
func (b *Builder) Body(name string, body *Builder) *Builder {
	if b.err != nil {
		return b
	}
	if name == "" {
		return b.fail(sagat.Invalid("a body needs a name"))
	}
	if body == nil {
		return b.fail(sagat.Invalid("body %q is empty", name))
	}
	if b.bodies == nil {
		b.bodies = map[string]*Builder{}
	}
	if _, exists := b.bodies[name]; exists {
		return b.fail(sagat.Invalid("the body %q is defined twice", name))
	}
	b.bodies[name] = body
	return b
}

// ---- the document

// flatten returns every node of the flow and of the flows inside it, in order, with next set: node i
// gets next = id of node i+1, except a condition or switch (it names its branches) and an end (it ends);
// the last node of a flow has none.
func (b *Builder) flatten() []node {
	out := make([]node, 0, len(b.nodes))
	for i, n := range b.nodes {
		linked := node{}
		for key, value := range n {
			linked[key] = value
		}
		if i+1 < len(b.nodes) && linked["type"] != "condition" && linked["type"] != "switch" && linked["type"] != "end" && linked["type"] != "return" {
			if _, set := linked["next"]; !set {
				linked["next"] = b.nodes[i+1]["id"]
			}
		}
		out = append(out, linked)
	}
	for _, inner := range b.embedded {
		out = append(out, inner.flatten()...)
	}
	return out
}

// Build returns the document. It fails with VALIDATION_FAILED for what the server would refuse and
// the builder can see. The document is a fresh copy: changing it does not change the builder.
func (b *Builder) Build() (map[string]any, error) {
	if b.err != nil {
		return nil, b.err
	}
	// The flows inside are read now, not when they were joined: a step added to a branch or to a saga body
	// after the join is part of the document, and a mistake in it is raised here.
	if err := b.joinedErr(); err != nil {
		return nil, err
	}
	if len(b.nodes) == 0 {
		return nil, sagat.Invalid("a workflow needs at least one node")
	}
	nodes := b.flatten()
	for _, n := range nodes {
		if n["type"] == "return" {
			return nil, sagat.Invalid("return is only valid in a named body")
		}
	}
	bodyNodes := make(map[string][]node, len(b.bodies))
	for name, body := range b.bodies {
		if body.err != nil {
			return nil, body.err
		}
		nested := body.flatten()
		if len(nested) == 0 {
			return nil, sagat.Invalid("body %q is empty", name)
		}
		returns := 0
		for _, n := range nested {
			if n["type"] == "end" {
				return nil, sagat.Invalid("body %q cannot hold an end node", name)
			}
			if n["type"] == "return" {
				returns++
			}
		}
		if returns != 1 {
			return nil, sagat.Invalid("body %q needs exactly one return node", name)
		}
		bodyNodes[name] = nested
	}
	validateScope := func(scope []node) error {
		local := make(map[string]struct{}, len(scope))
		for _, n := range scope {
			id := n["id"].(string)
			if _, seen := local[id]; seen {
				return sagat.Invalid("the node id %q is used twice", id)
			}
			local[id] = struct{}{}
		}
		reference := func(from, field string, target any) error {
			if name, ok := target.(string); ok {
				if _, found := local[name]; !found {
					return sagat.Invalid("%s.%s points at %q, which is not a node", from, field, name)
				}
			}
			return nil
		}
		for _, n := range scope {
			id := n["id"].(string)
			for _, field := range []string{"next", "onError", "onTimeout", "ifTrue", "ifFalse", "begin"} {
				if err := reference(id, field, n[field]); err != nil {
					return err
				}
			}
			if branches, ok := n["branches"].([]any); ok {
				for _, branch := range branches {
					if err := reference(id, "branches", branch); err != nil {
						return err
					}
				}
			}
			if cases, ok := n["cases"].([]any); ok {
				for _, item := range cases {
					if err := reference(id, "cases", item.(map[string]any)["target"]); err != nil {
						return err
					}
				}
			}
			if err := reference(id, "default", n["default"]); err != nil {
				return err
			}
			if n["type"] == "forEach" {
				name, _ := n["body"].(string)
				if _, found := b.bodies[name]; !found {
					return sagat.Invalid("forEach %s points at %q, which is not a body", id, name)
				}
				if _, found := n["next"].(string); !found {
					return sagat.Invalid("forEach %s needs a next node", id)
				}
			}
		}
		return nil
	}
	if err := validateScope(nodes); err != nil {
		return nil, err
	}
	for _, nested := range bodyNodes {
		if err := validateScope(nested); err != nil {
			return nil, err
		}
	}
	metadata := map[string]any{"name": b.name}
	if b.description != nil {
		metadata["description"] = *b.description
	}
	if len(b.labels) > 0 {
		labels := make(map[string]any, len(b.labels))
		for key, value := range b.labels {
			labels[key] = value
		}
		metadata["labels"] = labels
	}
	list := make([]any, len(nodes))
	for i, n := range nodes {
		list[i] = n
	}
	document := map[string]any{
		"apiVersion": "sagat-octopus-orchestrator.io/v1",
		"kind":       "Workflow",
		"metadata":   metadata,
		"entrypoint": b.nodes[0]["id"],
		"nodes":      list,
	}
	if len(b.triggers) > 0 {
		triggers := make([]any, len(b.triggers))
		for i, t := range b.triggers {
			triggers[i] = t.document()
		}
		document["triggers"] = triggers
	}
	if b.input != nil {
		document["inputSchema"] = b.input
	}
	if b.output != nil {
		document["outputSchema"] = b.output
	}
	if len(bodyNodes) > 0 {
		bodies := make(map[string]any, len(bodyNodes))
		for name, nested := range bodyNodes {
			list := make([]any, len(nested))
			for i, n := range nested {
				list[i] = n
			}
			bodies[name] = map[string]any{"entrypoint": nested[0]["id"], "nodes": list}
		}
		document["bodies"] = bodies
	}
	// What cannot be written as JSON (a NaN in a schema, say) is refused here, not by the server.
	if _, err := jsonx.Encode(document); err != nil {
		return nil, err
	}
	return clone(document).(map[string]any), nil
}

// joinedErr is the first mistake in the flows joined to this one, at any depth, as they are now.
func (b *Builder) joinedErr() error {
	for _, inner := range b.embedded {
		if inner.err != nil {
			return inner.err
		}
		if inner.sagaID != "" {
			for _, n := range inner.nodes {
				if n["type"] == "end" {
					return sagat.Invalid("saga %s cannot hold an end node", inner.sagaID)
				}
			}
		}
		if err := inner.joinedErr(); err != nil {
			return err
		}
	}
	for _, body := range b.bodies {
		if body.err != nil {
			return body.err
		}
		if err := body.joinedErr(); err != nil {
			return err
		}
	}
	return nil
}

// clone copies the maps and slices of a document, so that the caller owns them. Values of other
// types (numbers, strings, a caller's own types) are immutable or the caller's.
func clone(value any) any {
	switch v := value.(type) {
	case map[string]any:
		out := make(map[string]any, len(v))
		for key, item := range v {
			out[key] = clone(item)
		}
		return out
	case []any:
		out := make([]any, len(v))
		for i, item := range v {
			out[i] = clone(item)
		}
		return out
	case []string:
		return append([]string(nil), v...)
	default:
		return value
	}
}

// JSON returns the canonical text of Build, RFC 8785: UTF-8, no insignificant whitespace, the keys of
// every object sorted by their UTF-16 code units, numbers as ECMAScript prints them. The same program
// writes byte-identical text in every SDK.
func (b *Builder) JSON() (string, error) {
	document, err := b.Build()
	if err != nil {
		return "", err
	}
	text, err := jsonx.Canonical(document)
	if err != nil {
		return "", err
	}
	return string(text), nil
}
