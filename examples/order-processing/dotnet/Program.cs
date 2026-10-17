using SagatOctopusOrchestrator.Sdk;
using SagatOctopusOrchestrator.Sdk.Workers;

var apiUrl = RequiredSetting("SAGAT_SERVER");
var grpcUrl = RequiredSetting("SAGAT_GRPC");
var ns = RequiredSetting("SAGAT_NAMESPACE");
var token = RequiredSetting("SAGAT_TOKEN");
var (scenario, workflow) = SelectedScenario();
var input = new OrderInput(
    OrderId: "demo-order-001",
    Amount: Environment.GetEnvironmentVariable("SAGAT_REJECT_ORDER") == "1" ? -1 : 100,
    FailNotify: Environment.GetEnvironmentVariable("SAGAT_FAIL_NOTIFY") == "1");

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
    Console.Error.WriteLine($"{failure.Code.ToWireName()} (trace {failure.TraceId}): {failure.Remediation}");
    return 1;
}
catch (OperationCanceledException)
{
    Console.Error.WriteLine("interrupted");
    return 1;
}

async Task<int> RunAsync()
{
    var options = new SagatOptions { ApiUrl = apiUrl, GrpcUrl = grpcUrl, Namespace = ns, Token = token };
    options.Worker.Name = "order-processing-dotnet";

    await using var host = new WorkerHost(options);
    host.RegisterActivity<OrderInput, Validation>("order.validate", (order, _, _) =>
    {
        Console.WriteLine("activity order.validate");
        return Task.FromResult(new Validation(Accepted: order.Amount > 0));
    });
    host.RegisterActivity<OrderInput, Reservation>("order.reserve", (order, _, _) =>
    {
        Console.WriteLine("activity order.reserve");
        return Task.FromResult(new Reservation(Reserved: true, OrderId: order.OrderId));
    });
    host.RegisterActivity<OrderInput, Quote>("order.quote", (_, _, _) =>
    {
        Console.WriteLine("activity order.quote");
        return Task.FromResult(new Quote(ShippingCents: 500));
    });
    host.RegisterActivity<OrderInput, Notification>("order.notify", (order, _, _) =>
    {
        Console.WriteLine("activity order.notify");
        // Predecessors differ by scenario. If a real notification needs one, use
        // ActivityExecutionContext.Steps.TryGetValue instead of assuming that output exists.
        if (order.FailNotify)
        {
            throw ActivityFailure.Permanent("NOTIFICATION_REJECTED", "the simulated notification was rejected");
        }

        // A real provider call must use context.IdempotencyKey for deduplication and reconcile an
        // unknown outcome before retrying. This example has no external effect.
        return Task.FromResult(new Notification(Notified: true, OrderId: order.OrderId));
    });

    await host.StartAsync(interrupt.Token);
    try
    {
        using var api = new SagatClient(options);
        var started = await api.StartAsync(workflow, input, cancellationToken: interrupt.Token);
        var done = await api.WaitForCompletionAsync(
            started.RunId,
            new WaitOptions { Timeout = TimeSpan.FromSeconds(60) },
            interrupt.Token);
        Console.WriteLine($"scenario {scenario} run {done.RunId} {done.Status}");
        return done.Status == RunStatuses.Succeeded ? 0 : 1;
    }
    finally
    {
        try
        {
            await host.StopAsync(TimeSpan.FromSeconds(10));
        }
        catch (SagatException)
        {
            // The terminal line reports how the run ended; shutdown has no additional payload to print.
        }
    }
}

static (string Scenario, string Workflow) SelectedScenario()
{
    var scenario = Environment.GetEnvironmentVariable("SAGAT_SCENARIO");
    if (string.IsNullOrEmpty(scenario))
    {
        scenario = "routing";
    }
    var workflow = scenario switch
    {
        "routing" => "order-routing",
        "fulfillment" => "order-fulfillment",
        "reminder" => "order-reminder",
        _ => string.Empty,
    };
    if (workflow.Length == 0)
    {
        Console.Error.WriteLine("SAGAT_SCENARIO must be routing, fulfillment, or reminder");
        Environment.Exit(2);
    }

    return (scenario, workflow);
}

static string RequiredSetting(string name)
{
    var value = Environment.GetEnvironmentVariable(name);
    if (string.IsNullOrEmpty(value))
    {
        Console.Error.WriteLine($"{name} is not set (see the README)");
        Environment.Exit(2);
    }

    return value;
}

internal sealed record OrderInput(string OrderId, int Amount, bool FailNotify);

internal sealed record Validation(bool Accepted);

internal sealed record Reservation(bool Reserved, string OrderId);

internal sealed record Quote(int ShippingCents);

internal sealed record Notification(bool Notified, string OrderId);
