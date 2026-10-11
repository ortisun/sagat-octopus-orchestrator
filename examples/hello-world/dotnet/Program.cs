// The hello-world workflow served and started with the .NET SDK: a worker that answers the two
// activities of the workflow (templates/hello-world), a run of it, and the end of the run.
//
//	SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 \
//	SAGAT_NAMESPACE=payments SAGAT_TOKEN=... dotnet run [-- NAME]
using SagatOctopusOrchestrator.Sdk;
using SagatOctopusOrchestrator.Sdk.Workers;

var apiUrl = Setting("SAGAT_SERVER");
var grpcUrl = Setting("SAGAT_GRPC");
var ns = Setting("SAGAT_NAMESPACE");
var token = Setting("SAGAT_TOKEN");
var name = args.Length > 0 ? args[0] : "Ada";

using var interrupt = new CancellationTokenSource();
Console.CancelKeyPress += (_, e) =>
{
    e.Cancel = true;
    interrupt.Cancel();
};

try
{
    return await RunAsync();
}
catch (SagatException failure)
{
    // The code, the trace and what to do about it; never the payload or the token.
    Console.Error.WriteLine($"{failure.Code.ToWireName()} (trace {failure.TraceId}): {failure.Remediation}");
    if (failure.Reason == "WAIT_TIMEOUT")
    {
        Console.Error.WriteLine("the run did not finish in time: is it waiting for a worker? Let this identity serve its activities (sagat worker bind)");
    }
    return 1;
}
catch (OperationCanceledException)
{
    Console.Error.WriteLine("interrupted");
    return 1;
}

async Task<int> RunAsync()
{
    // One set of options for the client and for the worker.
    var options = new SagatOptions { ApiUrl = apiUrl, GrpcUrl = grpcUrl, Namespace = ns, Token = token };
    options.Worker.Name = "hello-world-dotnet";

    // The worker: it takes the tasks of the two activities and answers them. What a step receives is
    // the input of the run (the first argument) and the outputs of the steps before it (context.Steps).
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
    try
    {
        Console.WriteLine($"serving greeting.compose and greeting.deliver at protocol {host.ProtocolVersion}");

        // The client: start the run and wait for it.
        using var api = new SagatClient(options);
        var started = await api.StartAsync("hello-world", new Who(name), cancellationToken: interrupt.Token);
        Console.WriteLine($"run {started.RunId} started");
        var done = await api.WaitForCompletionAsync(
            started.RunId,
            new WaitOptions { Timeout = TimeSpan.FromSeconds(60) },
            interrupt.Token);
        Console.WriteLine($"run {done.RunId} {done.Status}");
        if (done.Status != RunStatuses.Succeeded)
        {
            Console.Error.WriteLine($"the run ended {done.Status}");
            return 1;
        }

        return 0;
    }
    finally
    {
        try
        {
            await host.StopAsync(TimeSpan.FromSeconds(10));
        }
        catch (SagatException)
        {
            // What the program says is how the run ended; a host that does not stop cleanly has nothing to add.
        }
    }
}

static string Setting(string name)
{
    var value = Environment.GetEnvironmentVariable(name);
    if (string.IsNullOrEmpty(value))
    {
        Console.Error.WriteLine($"{name} is not set (see the README)");
        Environment.Exit(2);
    }

    return value;
}

// The SDK reads and writes JSON in camel case: Who is {"name": ...}, Composed is {"greeting": ...} and
// Delivery is {"delivered": ..., "greeting": ...}.
internal sealed record Who(string Name);

internal sealed record Composed(string Greeting);

internal sealed record Delivery(bool Delivered, string Greeting);
