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

## Where to go next

- [The SDK README](../../sdks/go/README.md): the client, the worker host, the builders of the workflow document, the errors.
- [`sdks/SPEC.md`](../../sdks/SPEC.md): what the five SDKs do the same way, and the conformance suite that holds them to it.
- [saga.md](saga.md): a payment saga, with compensation and reconciliation.
