using System.Diagnostics;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>
/// The tests that observe the spans of the SDK, or that need nobody to. A listener is process-wide: these tests share a collection, so they
/// do not run at the same time as one another.
/// </summary>
[Collection("telemetry")]
public sealed class TraceListenerTests
{
    private const string Run = """{"runId":"0190aaaa-0000-7000-8000-000000000001","workflowId":"w","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z"}""";

    [Fact]
    public async Task WithoutAnyTraceContextNoneIsSent()
    {
        await using var server = await FakeRestServer.StartAsync();
        server.Handler = _ => new FakeAnswer(200, Run);
        Activity.Current = null;
        using var client = new SagatClient(new SagatOptions { ApiUrl = server.BaseUrl, Namespace = "p", Token = "t" });
        await client.GetAsync(Ids.Run);
        Assert.Equal(string.Empty, server.Requests[0].Header("traceparent"));
    }

    [Fact]
    public async Task AnActivityListenerThatThrowsDoesNotFailAClientCall()
    {
        var trace = Traceparent.TraceIdOf(Traceparent.New())!;
        var parent = $"00-{trace}-b7ad6b7169203331-01";
        using var listener = new ActivityListener
        {
            ShouldListenTo = source => source.Name == SdkInfo.ActivitySourceName,
            Sample = (ref ActivityCreationOptions<ActivityContext> _) => ActivitySamplingResult.AllDataAndRecorded,
            ActivityStarted = activity => throw new InvalidOperationException("the tracing sink exploded (started)"),
            ActivityStopped = activity => throw new InvalidOperationException("the tracing sink exploded (stopped)"),
        };
        ActivitySource.AddActivityListener(listener);
        await using var server = await FakeRestServer.StartAsync();
        server.Handler = _ => new FakeAnswer(202, Run);
        using var client = new SagatClient(new SagatOptions { ApiUrl = server.BaseUrl, Namespace = "p", Token = "t" });
        var run = await client.StartAsync("0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee", new { }, new StartOptions { Traceparent = parent });
        Assert.Equal("RUNNING", run.Status);
        Assert.Equal(parent, server.Requests[0].Header("traceparent"));
        var got = await client.GetAsync(Ids.Run);
        Assert.Equal("RUNNING", got.Status);
    }

    [Fact]
    public async Task AnActivityListenerThatThrowsNeitherKillsTheHostNorDropsATask()
    {
        var trace = Traceparent.TraceIdOf(Traceparent.New())!;
        var leaseTrace = $"00-{trace}-b7ad6b7169203331-01";
        using var listener = new ActivityListener
        {
            ShouldListenTo = source => source.Name == SdkInfo.ActivitySourceName,
            Sample = (ref ActivityCreationOptions<ActivityContext> _) => ActivitySamplingResult.AllDataAndRecorded,
            ActivityStarted = activity => throw new InvalidOperationException("the tracing sink exploded (started)"),
            ActivityStopped = activity => throw new InvalidOperationException("the tracing sink exploded (stopped)"),
        };
        ActivitySource.AddActivityListener(listener);

        await using var fake = await FakeWorkerServer.StartAsync();
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "p", Token = "t" };
        options.Worker.Name = "listener-" + Guid.NewGuid().ToString("N");
        options.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(30);
        await using var host = new Workers.WorkerHost(options);
        var ran = 0;
        host.RegisterActivity<System.Text.Json.Nodes.JsonNode?, int>("charge", async (_, _, ct) =>
        {
            Interlocked.Increment(ref ran);
            await Task.Delay(400, ct);
            return 1;
        });
        fake.State.Lease(Leases.Lease(l => l.Traceparent = leaseTrace));
        await host.StartAsync();
        await Eventually.UntilAsync(() => fake.State.Of("CompleteStep").Count == 1, "the completion although the trace listener throws");
        Assert.Equal(1, ran);
        Assert.True(host.IsRunning);
        Assert.NotEmpty(fake.State.Of("Heartbeat"));
    }
}
