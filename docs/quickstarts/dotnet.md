# C# quickstart

The hello-world workflow ([`templates/hello-world`](../../templates/hello-world/workflow.json): `greeting.compose`, then
`greeting.deliver`) served and started with the C# SDK, in [one file](../../examples/hello-world/dotnet/Program.cs). You need the .NET SDK 10 or newer, the
local stack and `sagat`; the [main quickstart](README.md) gets you there.

## Run it

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)

sagat new hello-world
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json
```

then, in [`examples/hello-world/dotnet`](../../examples/hello-world/dotnet), follow its [README](../../examples/hello-world/dotnet/README.md). It prints

```text
serving greeting.compose and greeting.deliver at protocol 1.2
run 01a12b51-158b-7147-b9b3-847882a5c939 started
delivering Hello, Ada!
run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

and `sagat workflow inspect <run>` shows the journal. The same program in each of the five languages makes the same run,
the same journal and the same outputs, byte for byte, which the end-to-end test of the examples checks.

`HelloWorld.csproj` references the SDK project in this checkout (`ProjectReference`); once the package is published, replace that line with
`<PackageReference Include="SagatOctopusOrchestrator.Sdk" Version="0.2.0" />`. In an ASP.NET application you would register the worker with
`AddSagatOctopusOrchestrator(...).AddActivity<...>(...)` instead (see the SDK README); the example uses `WorkerHost` directly so that it is one file.

## The worker

An activity receives the **input of the run** and, in the context, the **outputs of the steps that ran before it**:
`context.Steps["compose"]` is the JSON output of the step with that id. It returns its output. It fails on purpose with a retryable or a permanent failure; anything else it
raises is reported as `ACTIVITY_ERROR`, retryable, with only the type of the error, never its text.

```csharp excerpt=examples/hello-world/dotnet/Program.cs
await using var host = new WorkerHost(options);
host.RegisterActivity<Who, Composed>("greeting.compose", (input, context, ct) =>
    Task.FromResult(new Composed($"Hello, {input.Name}!")));
host.RegisterActivity<Who, Delivery>("greeting.deliver", (input, context, ct) =>
{
    // A real delivery is an external effect: send context.IdempotencyKey to the provider, which is
    // the same for every attempt of this step.
    var composed = context.Steps["compose"].GetProperty("greeting").GetString() ?? string.Empty;
    Console.WriteLine($"delivering {composed}");
    return Task.FromResult(new Delivery(Delivered: true, Greeting: composed));
});
await host.StartAsync(interrupt.Token);
```

The platform delivers at least once, so an activity that makes an effect in the world (a payment, an e-mail) sends
`context.IdempotencyKey` to its provider: it is the same for every attempt of the step, so a repeat does not do it twice. The
platform cannot make the provider exactly-once; the key is how the provider can.

## The client

```csharp excerpt=examples/hello-world/dotnet/Program.cs
using var api = new SagatClient(options);
var started = await api.StartAsync("hello-world", new Who(name), cancellationToken: interrupt.Token);
Console.WriteLine($"run {started.RunId} started");
var done = await api.WaitForCompletionAsync(
    started.RunId,
    new WaitOptions { Timeout = TimeSpan.FromSeconds(60) },
    interrupt.Token);
Console.WriteLine($"run {done.RunId} {done.Status}");
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
- **Your activities, alone:** A handler is a function of `(input, context, cancellationToken)`; with `AddActivity<TActivity, ...>` the handler is a class that takes its
dependencies from the container, which is the way to unit test it with a fake provider. The SDK ships no fake context yet.

## Workflow as code

**Not offered yet.** This section shows the API that [design 044](../designs/044-workflow-as-code.md) specifies; the C#
SDK offers it once its runtime, Task 044-17 of that design, lands. None of it runs today. The calls are those of
[`sdks/SPEC.md`](../../sdks/SPEC.md) section 12 (package `SagatOctopusOrchestrator.Workflows`, Pascal case, `Async` on
the calls that return a task, durations in the language's idiom); where the design leaves a name or a shape to the
runtime's task (the context type, registering the function on the host, the replayer's entry point), the code below is a
sketch that Task 044-17 settles. An operator also records the requirements `code-workflows-v1` and, because this
workflow declares a signal, `signal-inbox-v1` ([instance-registry runbook](../runbooks/instance-registry.md)).

Instead of a graph, the workflow is a method. It reserves the stock, charges the card, sleeps one day and sends a
reminder unless a `paid` signal arrived meanwhile (journey 1 of the design):

```csharp
public static async Task<OrderResult> Order(WorkflowContext ctx, OrderInput input)
{
    var paid = false;
    ctx.OnSignal("paid", _ => paid = true);
    await ctx.ActivityAsync<Reservation>("inventory.reserve", input);
    await ctx.ActivityAsync<Charge>("payments.charge", input);
    await ctx.SleepAsync(TimeSpan.FromDays(1));
    if (!paid)
    {
        await ctx.ActivityAsync<Reminder>("customers.remind", input);
    }
    return new OrderResult(input.OrderId, paid);
}
```

Each `ctx` call is a command the platform records and executes; the activities run in an ordinary worker as above. The
method is run again from the start against the run's journal whenever a worker does not hold it in memory, so it must be
deterministic. The SDK runs it on a single-threaded scheduler of its own with the invariant culture, and every SDK call
checks that it is still on that scheduler: `ConfigureAwait(false)`, `Task.Run`, `Task.Delay`, threads and `Parallel`
leave it and fail the workflow task (`NONDETERMINISM_RISK`). Use `ctx.Now()`, `ctx.Random()` and `ctx.Uuid()` instead of
`DateTime.Now`, `Random` and `Guid.NewGuid`; the analyzer package `SagatOctopusOrchestrator.Workflows.Analyzers` flags
those calls at build time. The precondition is .NET 8 and a local time zone of UTC for the workflow worker.

**One assembly for workflow code, another for activities.** The code digest of a C# workflow hashes the workflow
assembly and every assembly of its dependency closure, so a change of an activity in that closure makes a new digest,
and runs open on the old one move to it only by a verified move. Keep the workflow methods in their own worker
(`OrderWorkflows`) and the activities in another (`OrderActivities`) that the workflow worker does not reference.

**Write the manifest and publish the version.** The workflow worker prints its manifest:

```sh
dotnet publish OrderWorkflows -c Release -o out/workflows
dotnet out/workflows/OrderWorkflows.dll --sagat-manifest > manifest.json
```

```csharp
var manifest = await File.ReadAllTextAsync("manifest.json", cancellationToken);
var envelope = new CodeEnvelope(Activities: ["inventory.reserve", "payments.charge", "customers.remind"]);
await api.Definitions.PublishCodeAsync(manifest, envelope, cancellationToken);
```

`PublishCodeAsync` saves the code version as the workflow's draft and publishes it, making it active. The envelope lists
what the method's commands may use; a command outside it puts the run under review. The `paid` signal is a declaration
of the code document beside the envelope (`signals: [{name: paid, buffered: true}]`, Technical Plan 3.1 of the design).

**Serve it.** The identity of the workflow worker needs a binding that lists the workflow type `order` (`workflowTypes`
on the worker binding; setting it needs `definition:publish` as well as `worker:bind`). In `OrderWorkflows`, run with
`TZ=UTC`:

```csharp
await using var workflows = new WorkerHost(options);
workflows.RegisterWorkflow("order", OrderFlow.Order); // the registration's name is fixed by Task 044-17
await workflows.StartAsync(cancellationToken);
```

Deploy a new workflow worker beside the old one: runs stay on the digest they started with, so keep the old worker
running until the version list shows no run left on it, or move them.

**Test it with the replayer.** Export the history of a run (`api.Runs.ExportHistoryAsync(runId)`, or
`sagat workflow history export --workflow order --last 64 --dir histories/` once the command ships; both need
`history:export`), keep it with your tests, and replay it against the current code in xUnit:

```csharp
[Fact]
public async Task TheNewCodeReplaysTheRecordedOrderRuns()
{
    using var history = File.OpenText("histories/order-1.jsonl");
    var verdict = await Replayer.ReplayAsync(history, "order", OrderFlow.Order); // named by Task 044-17
    Assert.True(verdict.Matched);
}
```

A mismatch names the first command that differs; a history of an export format other than 1 is refused, and a replay
that reaches a value the export redacted stops with `INCONCLUSIVE_REDACTED` instead of a false mismatch.

## Where to go next

- [The SDK README](../../sdks/dotnet/README.md): the client, the worker host, the builders of the workflow document, the errors.
- [`sdks/SPEC.md`](../../sdks/SPEC.md): what the five SDKs do the same way, and the conformance suite that holds them to it.
- [saga.md](saga.md): a payment saga, with compensation and reconciliation.
