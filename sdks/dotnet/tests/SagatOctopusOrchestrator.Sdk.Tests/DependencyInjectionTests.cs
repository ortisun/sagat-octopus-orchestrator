using System.Collections.Concurrent;
using System.Text.Json.Nodes;
using Grpc.Core;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.DependencyInjection;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workers;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

public sealed record Order(string OrderId, int Amount);

public sealed record Placed(string Number);

/// <summary>A scoped service: every task gets its own, and it is disposed with the task's scope.</summary>
public sealed class Numbering : IDisposable
{
    internal static readonly ConcurrentBag<Numbering> Created = [];

    public Numbering() => Created.Add(this);

    public Guid Id { get; } = Guid.NewGuid();

    public bool Disposed { get; private set; }

    public void Dispose() => Disposed = true;
}

public sealed class PlaceOrder(Numbering numbering) : IActivity<Order, Placed>
{
    public Task<Placed> ExecuteAsync(Order input, ActivityExecutionContext context, CancellationToken cancellationToken) =>
        Task.FromResult(new Placed($"{input.OrderId}-{numbering.Id:N}"));
}

public sealed class Unplace : IActivity<Order, Placed>
{
    public Task<Placed> ExecuteAsync(Order input, ActivityExecutionContext context, CancellationToken cancellationToken) =>
        Task.FromResult(new Placed("undone-" + context.TargetNode));
}

public sealed class LookUp : IReconciler<Order>
{
    public Task<Verdict> ReconcileAsync(Order input, ActivityExecutionContext context, CancellationToken cancellationToken) =>
        Task.FromResult(Verdict.Applied(new Placed("found-" + input.OrderId)));
}

public sealed class NotAHandler;

/// <summary>FR-006-007: <c>Microsoft.Extensions.DependencyInjection</c>, <c>IHostedService</c>, <c>CancellationToken</c>, <c>ILogger</c> and <c>ActivitySource</c>.</summary>
public sealed class DependencyInjectionTests
{
    private static HostApplicationBuilder Builder()
    {
        var builder = Host.CreateApplicationBuilder();
        builder.Logging.ClearProviders();
        return builder;
    }

    [Fact]
    public async Task TheClientIsRegisteredAsOneSingletonUnderBothItsInterfaceAndItsClass()
    {
        await using var rest = await FakeRestServer.StartAsync();
        var services = new ServiceCollection();
        services.AddLogging();
        services.AddSagatOctopusOrchestrator(o =>
        {
            o.ApiUrl = rest.BaseUrl;
            o.Namespace = "payments";
            o.Token = "tok";
        });
        await using var provider = services.BuildServiceProvider();
        var viaInterface = provider.GetRequiredService<ISagatClient>();
        Assert.Same(viaInterface, provider.GetRequiredService<ISagatClient>());
        Assert.Same(viaInterface, provider.GetRequiredService<SagatClient>());
        Assert.Null(provider.GetService<IWorkerHost>());
        Assert.Empty(provider.GetServices<IHostedService>());

        rest.Handler = _ => new FakeAnswer(200, """{"runId":"r","workflowId":"w","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z"}""");
        Assert.Equal("r", (await viaInterface.GetAsync("r")).RunId);
    }

    [Fact]
    public async Task TheOptionsBindFromConfigurationAndTheDelegateIsAppliedOnTop()
    {
        var configuration = new ConfigurationBuilder()
            .AddInMemoryCollection(new Dictionary<string, string?>
            {
                ["Sagat:ApiUrl"] = "http://127.0.0.1:9/v1",
                ["Sagat:GrpcUrl"] = "http://127.0.0.1:9",
                ["Sagat:Namespace"] = "from-config",
                ["Sagat:Token"] = "tok-config",
                ["Sagat:Timeout"] = "00:00:05",
                ["Sagat:Retry:MaxAttempts"] = "2",
                ["Sagat:Retry:InitialDelay"] = "00:00:00.050",
                ["Sagat:UserAgent"] = "my-service/1",
                ["Sagat:Worker:Name"] = "worker-from-config",
                ["Sagat:Worker:MaxInflight"] = "3",
                ["Sagat:Worker:ShutdownTimeout"] = "00:00:10",
            })
            .Build();
        var services = new ServiceCollection();
        services.AddLogging();
        services.Configure<SagatOptions>(configuration.GetSection("Sagat"));
        services.AddSagatOctopusOrchestrator(o => o.Namespace = "overridden");
        await using var provider = services.BuildServiceProvider();
        var options = provider.GetRequiredService<IOptions<SagatOptions>>().Value;
        Assert.Equal("overridden", options.Namespace);
        Assert.Equal("http://127.0.0.1:9/v1", options.ApiUrl);
        Assert.Equal(TimeSpan.FromSeconds(5), options.Timeout);
        Assert.Equal(2, options.Retry.MaxAttempts);
        Assert.Equal(TimeSpan.FromMilliseconds(50), options.Retry.InitialDelay);
        Assert.Equal("worker-from-config", options.Worker.Name);
        Assert.Equal(3, options.Worker.MaxInflight);
        Assert.Equal(TimeSpan.FromSeconds(10), options.Worker.ShutdownTimeout);
        Assert.NotNull(provider.GetRequiredService<ISagatClient>());
    }

    [Fact]
    public async Task TheWorkerStartsWithTheHostRunsLambdaAndClassHandlersAndStopsGracefullyWithIt()
    {
        await using var fake = await FakeWorkerServer.StartAsync();
        var builder = Builder();
        var sawCancellation = new TaskCompletionSource<bool>();
        var inFlight = new TaskCompletionSource();
        builder.Services.AddScoped<Numbering>();
        builder.Services
            .AddSagatOctopusOrchestrator(o =>
            {
                o.ApiUrl = "http://127.0.0.1:1/v1";
                o.GrpcUrl = fake.Url;
                o.Namespace = "payments";
                o.Token = "tok";
                o.Worker.Name = "di-worker-" + Guid.NewGuid().ToString("N");
                o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20);
            })
            .AddActivity<PlaceOrder>("orders.place")
            .AddCompensation<Unplace>("orders.unplace")
            .AddReconciler<LookUp>("orders.lookup")
            .AddActivity<Order, int>("orders.double", (input, _, _) => Task.FromResult(input.Amount * 2))
            .AddActivity<JsonNode?>("orders.slow", async (_, context, ct) =>
            {
                inFlight.SetResult();
                await Task.Delay(300, CancellationToken.None);
                sawCancellation.SetResult(ct.IsCancellationRequested || context.CancellationReason is not null);
            });
        using var host = builder.Build();

        Assert.NotNull(host.Services.GetService<IWorkerHost>());
        Assert.Single(host.Services.GetServices<IHostedService>(), service => service.GetType().Name == "SagatWorkerService");
        Assert.Empty(fake.State.Of("RegisterWorker"));

        string Doc(string node, string input) => Leases.Document(JsonNode.Parse(input), node);
        fake.State.Lease(
            Leases.Lease(l => { l.ActivityName = "orders.place"; l.StepId = "place"; l.TaskId = "t1"; l.InputJson = Doc("place", """{"orderId":"o-1","amount":5}"""); }),
            Leases.Lease(l => { l.ActivityName = "orders.place"; l.StepId = "place"; l.TaskId = "t2"; l.InputJson = Doc("place", """{"orderId":"o-2","amount":6}"""); }),
            Leases.Lease(l => { l.ActivityName = "orders.unplace"; l.StepId = "place~c"; l.TaskId = "t3"; l.TaskKind = "COMPENSATION"; l.TargetNode = "place"; l.InputJson = Doc("place~c", """{"orderId":"o-1","amount":5}"""); }),
            Leases.Lease(l => { l.ActivityName = "orders.lookup"; l.StepId = "place~r"; l.TaskId = "t4"; l.TaskKind = "RECONCILIATION"; l.InputJson = Doc("place~r", """{"orderId":"o-3","amount":1}"""); }),
            Leases.Lease(l => { l.ActivityName = "orders.double"; l.StepId = "double"; l.TaskId = "t5"; l.InputJson = Doc("double", """{"orderId":"o-4","amount":21}"""); }));

        await host.StartAsync();
        var register = (RegisterWorkerRequest)Assert.Single(fake.State.Of("RegisterWorker")).Request;
        Assert.Equal(["orders.double", "orders.lookup", "orders.place", "orders.slow", "orders.unplace"], register.Capabilities.Activities);
        Assert.Equal("csharp", Assert.Single(register.Capabilities.Languages));
        Assert.True(host.Services.GetRequiredService<IWorkerHost>().IsRunning);

        await Eventually.UntilAsync(() => fake.State.Of("CompleteStep").Count == 5, "five completions");
        string Out(string task) => fake.State.Of("CompleteStep").Select(c => (CompleteStepRequest)c.Request).First(r => r.TaskId == task).OutputJson;
        var first = System.Text.Json.JsonDocument.Parse(Out("t1")).RootElement.GetProperty("number").GetString()!;
        var second = System.Text.Json.JsonDocument.Parse(Out("t2")).RootElement.GetProperty("number").GetString()!;
        Assert.StartsWith("o-1-", first, StringComparison.Ordinal);
        Assert.StartsWith("o-2-", second, StringComparison.Ordinal);
        Assert.NotEqual(first[4..], second[4..]);
        Assert.Equal("""{"number":"undone-place"}""", Out("t3"));
        Assert.Equal("""{"effect":"applied","output":{"number":"found-o-3"}}""", Out("t4"));
        Assert.Equal("42", Out("t5"));
        Assert.All(Numbering.Created, numbering => Assert.True(numbering.Disposed, "the scope of a task ends with the task"));

        // A task in flight when the host stops is waited for: the application does not lose it.
        fake.State.Lease(Leases.Lease(l => { l.ActivityName = "orders.slow"; l.StepId = "slow"; l.TaskId = "t6"; }));
        await inFlight.Task.WaitAsync(TimeSpan.FromSeconds(10));
        await host.StopAsync();
        Assert.False(await sawCancellation.Task.WaitAsync(TimeSpan.FromSeconds(1)), "stop waits: it does not cancel what finishes in time");
        Assert.Contains(fake.State.Of("CompleteStep"), c => ((CompleteStepRequest)c.Request).TaskId == "t6");
        Assert.False(host.Services.GetRequiredService<IWorkerHost>().IsRunning);
    }

    [Fact]
    public async Task WhenTheHostStopsTooSoonTheTasksLeftAreCancelledWithTheReasonShutdown()
    {
        await using var fake = await FakeWorkerServer.StartAsync();
        var builder = Builder();
        var started = new TaskCompletionSource();
        string? reason = null;
        builder.Services.AddSagatOctopusOrchestrator(o =>
        {
            o.GrpcUrl = fake.Url;
            o.Namespace = "payments";
            o.Token = "tok";
            o.Worker.Name = "di-worker-" + Guid.NewGuid().ToString("N");
            o.Worker.ShutdownTimeout = TimeSpan.FromMilliseconds(100);
        }).AddActivity<JsonNode?, int>("orders.stuck", async (_, context, ct) =>
        {
            started.SetResult();
            await Task.Delay(Timeout.Infinite, ct).ContinueWith(_ => { }, TaskScheduler.Default);
            reason = context.CancellationReason;
            return 1;
        });
        using var host = builder.Build();
        fake.State.Lease(Leases.Lease(l => l.ActivityName = "orders.stuck"));
        await host.StartAsync();
        await started.Task.WaitAsync(TimeSpan.FromSeconds(10));
        await host.StopAsync();
        Assert.Equal(CancellationReasons.Shutdown, reason);
    }

    [Fact]
    public async Task ARegistrationTheServerRefusesStopsTheApplicationFromStarting()
    {
        await using var fake = await FakeWorkerServer.StartAsync();
        fake.State.Fail("RegisterWorker", new Scripted(StatusCode.PermissionDenied, "NOT_BOUND"));
        var builder = Builder();
        builder.Services.AddSagatOctopusOrchestrator(o =>
        {
            o.GrpcUrl = fake.Url;
            o.Namespace = "payments";
            o.Token = "tok";
        }).AddActivity<JsonNode?, int>("orders.any", (_, _, _) => Task.FromResult(1));
        using var host = builder.Build();
        var error = await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        Assert.Equal(SagatErrorCode.NotBound, error.Code);
    }

    [Fact]
    public async Task TheWorkerNameDefaultsToTheMachineNameAndTheLoggerFactoryOfTheContainerIsUsed()
    {
        await using var fake = await FakeWorkerServer.StartAsync();
        var provider = new CapturingLoggerProvider();
        var builder = Builder();
        builder.Logging.AddProvider(provider).SetMinimumLevel(LogLevel.Information);
        builder.Services.AddSagatOctopusOrchestrator(o =>
        {
            o.GrpcUrl = fake.Url;
            o.Namespace = "payments";
            o.Token = "tok-never-in-a-log";
        }).AddActivity<JsonNode?, int>("orders.any", (_, _, _) => Task.FromResult(1));
        using var host = builder.Build();
        await host.StartAsync();
        await host.StopAsync();
        var register = (RegisterWorkerRequest)fake.State.Of("RegisterWorker")[0].Request;
        Assert.Equal(Environment.MachineName, register.WorkerName);
        Assert.Contains(provider.Lines, line => line.Contains("registered at protocol 1.2", StringComparison.Ordinal));
        Assert.DoesNotContain(provider.Lines, line => line.Contains("tok-never-in-a-log", StringComparison.Ordinal));
    }

    [Fact]
    public void AClassThatIsNotAHandlerIsRefusedWhenItIsRegistered()
    {
        var services = new ServiceCollection();
        var builder = services.AddSagatOctopusOrchestrator();
        Assert.Throws<InvalidOperationException>(() => builder.AddActivity<NotAHandler>("x"));
        Assert.Throws<InvalidOperationException>(() => builder.AddReconciler<PlaceOrder>("x"));
        Assert.Throws<InvalidOperationException>(() => builder.AddCompensation<LookUp>("x"));
    }
}
