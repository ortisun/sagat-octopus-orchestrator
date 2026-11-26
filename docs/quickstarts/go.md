# Go quickstart

The hello-world workflow ([`templates/hello-world`](../../templates/hello-world/workflow.json): `greeting.compose`, then
`greeting.deliver`) served and started with the Go SDK, in [one file](../../examples/hello-world/go/main.go). You need Go 1.25 or newer, the
local stack and `sagat`; the [main quickstart](README.md) gets you there.

## Run it

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)

sagat new hello-world
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json
```

then, in [`examples/hello-world/go`](../../examples/hello-world/go), follow its [README](../../examples/hello-world/go/README.md). It prints

```text
serving greeting.compose and greeting.deliver at protocol 1.2
run 01a12b51-158b-7147-b9b3-847882a5c939 started
delivering Hello, Ada!
run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

and `sagat workflow inspect <run>` shows the journal. The same program in each of the five languages makes the same run,
the same journal and the same outputs, byte for byte, which the end-to-end test of the examples checks.

`go.mod` points the SDK at this checkout with a `replace` directive (`../../../sdks/go`) because the module has no tag yet; remove
that line once it has one. The root package is called `sagat` although the path ends in `/go`: import it with a name
(`sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"`).

## The worker

An activity receives the **input of the run** and, in the context, the **outputs of the steps that ran before it**:
`worker.TaskFrom(ctx).Steps["compose"]` is the raw JSON output of the step with that id. It returns its output. It fails on purpose with a retryable or a permanent failure; anything else it
raises is reported as `ACTIVITY_ERROR`, retryable, with only the type of the error, never its text.

```go excerpt=examples/hello-world/go/main.go
host, err := worker.New(grpcURL, namespace, "hello-world-go", sagat.StaticToken(token))
if err != nil {
	return err
}
if err := host.RegisterActivity("greeting.compose", worker.Typed(func(ctx context.Context, in Person) (Greeting, error) {
	return Greeting{Greeting: "Hello, " + in.Name + "!"}, nil
})); err != nil {
	return err
}
if err := host.RegisterActivity("greeting.deliver", worker.Typed(func(ctx context.Context, in Person) (Delivery, error) {
	// The outputs of the steps that ran before this one, by step id.
	var composed Greeting
	if err := json.Unmarshal(worker.TaskFrom(ctx).Steps["compose"], &composed); err != nil {
		return Delivery{}, worker.Permanent("NO_GREETING", "the compose step gave no greeting")
	}
	// A real delivery is an external effect: send worker.TaskFrom(ctx).IdempotencyKey to the
	// provider, which is the same for every attempt of this step.
	fmt.Printf("delivering %s\n", composed.Greeting)
	return Delivery{Delivered: true, Greeting: composed.Greeting}, nil
})); err != nil {
	return err
}
if err := host.Start(ctx); err != nil {
	return err
}
```

The platform delivers at least once, so an activity that makes an effect in the world (a payment, an e-mail) sends
`worker.TaskFrom(ctx).IdempotencyKey` to its provider: it is the same for every attempt of the step, so a repeat does not do it twice. The
platform cannot make the provider exactly-once; the key is how the provider can.

## The client

```go excerpt=examples/hello-world/go/main.go
api, err := client.New(apiURL, namespace, sagat.StaticToken(token))
if err != nil {
	return err
}
started, err := api.Start(ctx, "hello-world", map[string]any{"name": name})
if err != nil {
	return err
}
fmt.Printf("run %s started\n", started.RunID)
done, err := api.WaitForCompletion(ctx, started.RunID, client.WaitTimeout(60*time.Second))
if err != nil {
	return err
}
fmt.Printf("run %s %s\n", done.RunID, done.Status)
```

Starting a run takes the name of a workflow of the namespace (or its id). Without an idempotency key the SDK makes one per
call and reuses it for every retry of that call, so a lost answer never starts a second run. The tenant is never
configured: it is the token's.

## Testing

- **A workflow, before your worker exists:** `sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind` serves its
  activities from a file of canned answers and failures, on the real engine. Make `greeting.compose` fail once
  (`sequence: [{fail: {code: X, retryable: true}}, {output: ...}]`) and see the retry; make it `lose` the task and see the lease
  expire.
- **Your activities, against the platform:** the example is the harness. It is what the end-to-end test of this
  repository runs, with the platform it starts, and what you can run against your own stack.
- **Your activities, alone:** A handler is a function of `(ctx, input)`: keep the effect behind an interface and unit test the logic by calling the function with a
`context.Context`. A handler that reads `worker.TaskFrom(ctx)` needs a task in its context, which only the host builds; the SDK ships no fake yet.

## Workflow as code

**Not offered yet.** This section shows the API that [design 044](../designs/044-workflow-as-code.md) specifies; the Go
SDK offers it once its runtime, Task 044-16 of that design, lands. None of it runs today. The calls are those of
[`sdks/SPEC.md`](../../sdks/SPEC.md) section 12 (package `workflows`, imported from
`github.com/ortisun/sagat-octopus-orchestrator/sdks/go/workflows`, each call taking `workflows.Context` first,
durations in the language's idiom; the package is plural because `sdks/go/workflow` is the builder of the workflow
document, Decision 044-239); where the design leaves a name or a shape to the runtime's task (the results, registering the
function on the host, the replayer's entry point), the code below is a sketch that Task 044-16 settles. An operator also
records the requirements `code-workflows-v1` and, because this workflow declares a signal, `signal-inbox-v1`
([instance-registry runbook](../runbooks/instance-registry.md)).

Instead of a graph, the workflow is a function. It reserves the stock, charges the card, sleeps one day and sends a
reminder unless a `paid` signal arrived meanwhile (journey 1 of the design):

```go
func Order(ctx workflows.Context, in OrderInput) (OrderResult, error) {
	paid := false
	workflows.OnSignal(ctx, "paid", func(json.RawMessage) { paid = true })
	if _, err := workflows.Activity[Reservation](ctx, "inventory.reserve", in); err != nil {
		return OrderResult{}, err
	}
	if _, err := workflows.Activity[Charge](ctx, "payments.charge", in); err != nil {
		return OrderResult{}, err
	}
	if err := workflows.Sleep(ctx, 24*time.Hour); err != nil {
		return OrderResult{}, err
	}
	if !paid {
		if _, err := workflows.Activity[Reminder](ctx, "customers.remind", in); err != nil {
			return OrderResult{}, err
		}
	}
	return OrderResult{OrderID: in.OrderID, Paid: paid}, nil
}
```

Each `workflows` call is a command the platform records and executes; the activities run in an ordinary worker as above.
The function is run again from the start against the run's journal whenever a worker does not hold it in memory, so it
must be deterministic, and Go cannot forbid a call at run time: no `go` statement, channel, `select`, `time.Now`,
`time.Sleep`, `math/rand`, `crypto/rand` or `range` over a map in workflow code. Use `workflows.Go`, `workflows.Channel`,
`workflows.Selector`, `workflows.Now`, `workflows.Sleep` and `workflows.SortedKeys` instead. The SDK's analyzer `sagatvet`
(`cmd/sagatvet`, a `go/analysis` analyzer) refuses the others in workflow functions, and the SDK's test helper runs it;
run it over your workflow package in your tests. A native channel that blocks stops at the activation deadline of 2 s.
The precondition is Go 1.25, the SDK's baseline.

An activity failure the function does not handle fails the workflow task, not the run: the task is retried after 1, 2,
4 and 8 s, and the fifth consecutive failure puts the run under review. To fail the run on purpose, return the SDK's
`workflows.WorkflowFailure` with a code (Requirement 044-009). How the runtime treats any other returned error, such as
the activity errors the function above returns, is decided by Task 044-16.

**One binary for workflow code, another for activities.** The code digest of a Go workflow is the SHA-256 of the whole
executable, so a change of an activity linked into the same binary makes a new digest, and runs open on the old one move
to it only by a verified move. Keep the workflow functions in their own worker binary (`cmd/order-workflows`) and the
activities in another (`cmd/order-activities`): a change of an activity then leaves the workflow digest as it was.

**Write the manifest and publish the version.** The workflow binary prints its manifest:

```sh
go build -o order-workflows ./cmd/order-workflows
./order-workflows --sagat-manifest > manifest.json
```

```go
manifest, err := os.ReadFile("manifest.json")
if err != nil {
	return err
}
envelope := workflows.Envelope{Activities: []workflows.ActivityEntry{
	{Name: "inventory.reserve"}, {Name: "payments.charge"}, {Name: "customers.remind"},
}}
declarations := workflows.Declarations{Signals: []workflows.Signal{{
	Name: "paid", Buffered: true, MaxPending: 10,
	Schema: json.RawMessage(`{"type": "object", "properties": {}, "additionalProperties": false}`),
}}}
if _, err := api.Definitions.PublishCode(ctx, manifest, envelope, declarations); err != nil {
	return err
}
```

`PublishCode` assembles the code document from the manifest, the envelope and the declarations, saves it as the
workflow's draft and publishes it, making it active (Decision 044-238). The envelope lists what the function's commands
may use; a command outside it puts the run under review. The declarations hold the rest of the code document of
Technical Plan 3.1, here the `paid` signal (`sagat workflow publish-code` takes them with `--declarations <file>`).

**Serve it.** The identity of the workflow worker needs a binding that lists the workflow type `order` (`workflowTypes`
on the worker binding; setting it needs `definition:publish` as well as `worker:bind`). In `cmd/order-workflows`:

```go
host, err := worker.New(grpcURL, namespace, "order-workflows", sagat.StaticToken(token))
if err != nil {
	return err
}
if err := host.RegisterWorkflow("order", Order); err != nil { // the registration's name is fixed by Task 044-16
	return err
}
return host.Start(ctx)
```

Deploy a new workflow binary beside the old one: runs stay on their current code version's digest (the start
version's until a move), so keep the old binary running until the version list shows no run left on it, or move them.

**Test it with the replayer.** Export the history of a run (`api.Runs.ExportHistory(ctx, runID)`, or
`sagat workflow history export --workflow order --last 64 --dir histories/` once the command ships; both need
`history:export`), keep it with your tests, and replay it against the current code in `go test`:

```go
func TestTheNewCodeReplaysTheRecordedOrderRuns(t *testing.T) {
	history, err := os.Open("testdata/order-1.jsonl")
	if err != nil {
		t.Fatal(err)
	}
	defer history.Close()
	verdict, err := workflows.Replay(t.Context(), history, "order", Order) // named by Task 044-16
	if err != nil {
		t.Fatal(err)
	}
	if !verdict.Matched {
		t.Fatalf("the first difference is at command %d", verdict.CommandNumber)
	}
}
```

A history of an export format other than 1 is refused, and a replay that reaches a value the export redacted stops
with `INCONCLUSIVE_REDACTED` instead of a false mismatch.

## Where to go next

- [The SDK README](../../sdks/go/README.md): the client, the worker host, the builders of the workflow document, the errors.
- [`sdks/SPEC.md`](../../sdks/SPEC.md): what the six SDKs do the same way, and the conformance suite that holds them to it.
- [saga.md](saga.md): a payment saga, with compensation and reconciliation.
