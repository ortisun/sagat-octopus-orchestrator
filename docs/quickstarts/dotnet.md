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
`<PackageReference Include="SagatOctopusOrchestrator.Sdk" Version="0.1.0" />`. In an ASP.NET application you would register the worker with
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

## Where to go next

- [The SDK README](../../sdks/dotnet/README.md): the client, the worker host, the builders of the workflow document, the errors.
- [`sdks/SPEC.md`](../../sdks/SPEC.md): what the five SDKs do the same way, and the conformance suite that holds them to it.
- [saga.md](saga.md): a payment saga, with compensation and reconciliation.
