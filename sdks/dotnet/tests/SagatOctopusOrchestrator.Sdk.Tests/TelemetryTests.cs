using System.Diagnostics;
using System.Text.Json.Nodes;
using OpenTelemetry.Metrics;
using OpenTelemetry.Trace;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workers;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>The <see cref="ActivitySource"/> <c>SagatOctopusOrchestrator.Sdk</c>: its spans carry the W3C trace to the server and from a lease into the handler.</summary>
[Collection("telemetry")]
public sealed class TelemetryTests
{
    private const string Token = "tok-for-telemetry";
    private const string Run = """{"runId":"r","workflowId":"w","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z"}""";

    private static SagatClient ClientOf(FakeRestServer server, string space) =>
        new(new SagatOptions { ApiUrl = server.BaseUrl, Namespace = space, Token = Token, Retry = { MaxAttempts = 1 } });

    private static string Space() => "ns-" + Guid.NewGuid().ToString("N");

    private static bool Of(Activity span, string space) => (string?)span.GetTagItem("sagat.namespace") == space;

    [Fact]
    public async Task ARequestCarriesTheTraceOfTheCallersActivityAsAChildSpanOfTheSdk()
    {
        await using var server = await FakeRestServer.StartAsync();
        server.Handler = _ => new FakeAnswer(200, Run);
        var space = Space();
        var exported = new List<Activity>();
        using var provider = global::OpenTelemetry.Sdk.CreateTracerProviderBuilder()
            .AddSource(SdkInfo.ActivitySourceName, "Tests.Caller")
            .AddInMemoryExporter(exported)
            .Build();
        using var source = new ActivitySource("Tests.Caller");
        using var callers = new SpanCapture(_ => true, "Tests.Caller");
        using var client = ClientOf(server, space);

        using var caller = source.StartActivity("caller")!;
        await client.GetAsync(Ids.Run);

        var sent = server.Requests[0].Header("traceparent");
        Assert.True(Traceparent.IsValid(sent));
        Assert.Equal(caller.TraceId.ToString(), Traceparent.TraceIdOf(sent));
        var span = Assert.Single(exported, span => Of(span, space));
        Assert.Equal(span.Id, sent);
        Assert.Equal(caller.SpanId, span.ParentSpanId);
        Assert.NotEqual(caller.SpanId.ToString(), sent.Split('-')[2]);
        Assert.Equal(ActivityKind.Client, span.Kind);
        Assert.Equal("get", span.GetTagItem("sagat.operation"));
        Assert.Equal("/runs/{runId}", span.GetTagItem("url.template"));
        Assert.Equal(200, span.GetTagItem("http.response.status_code"));
        Assert.Equal(space, span.GetTagItem("sagat.namespace"));
        Assert.DoesNotContain(span.TagObjects, tag => Convert.ToString(tag.Value, System.Globalization.CultureInfo.InvariantCulture)!.Contains(Token, StringComparison.Ordinal));
        Assert.DoesNotContain(Ids.Run, string.Concat(span.TagObjects.Select(t => t.Value)), StringComparison.Ordinal);
    }

    [Fact]
    public async Task AnExplicitTraceparentIsSentAsItIsAndTheSpanIsItsChild()
    {
        await using var server = await FakeRestServer.StartAsync();
        server.Handler = _ => new FakeAnswer(202, Run);
        var space = Space();
        using var spans = new SpanCapture(span => Of(span, space), SdkInfo.ActivitySourceName);
        using var client = ClientOf(server, space);
        await client.StartAsync("0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee", new { }, new StartOptions { Traceparent = Leases.Traceparent });
        Assert.Equal(Leases.Traceparent, server.Requests[0].Header("traceparent"));
        var span = Assert.Single(spans.Spans);
        Assert.Equal("0af7651916cd43dd8448eb211c80319c", span.TraceId.ToString());
        Assert.Equal("b7ad6b7169203331", span.ParentSpanId.ToString());
    }

    [Fact]
    public async Task AFailedCallMarksItsSpanWithTheCanonicalCodeAndNothingElse()
    {
        await using var server = await FakeRestServer.StartAsync();
        server.Handler = _ => new FakeAnswer(422, """{"code":"VALIDATION_FAILED","message":"card 4111 is wrong","retryable":false,"traceId":""}""");
        var space = Space();
        using var spans = new SpanCapture(span => Of(span, space), SdkInfo.ActivitySourceName);
        using var client = ClientOf(server, space);
        await Assert.ThrowsAsync<SagatException>(() => client.StartAsync("0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee", new { card = "canary-card-number" }));
        var span = Assert.Single(spans.Spans);
        Assert.Equal(ActivityStatusCode.Error, span.Status);
        Assert.Equal("VALIDATION_FAILED", span.GetTagItem("error.type"));
        Assert.DoesNotContain("canary-card-number", string.Concat(span.TagObjects.Select(t => t.Value)) + span.StatusDescription, StringComparison.Ordinal);
    }

    [Fact]
    public async Task TheTraceOfALeaseReachesTheHandlerAndFromThereTheRequestsItMakes()
    {
        await using var fake = await FakeWorkerServer.StartAsync();
        await using var rest = await FakeRestServer.StartAsync();
        rest.Handler = _ => new FakeAnswer(200, Run);
        var space = Space();
        var worker = "telemetry-" + Guid.NewGuid().ToString("N");
        var exportedMetrics = new List<Metric>();
        using var meterProvider = global::OpenTelemetry.Sdk.CreateMeterProviderBuilder()
            .AddMeter(SdkInfo.MeterName)
            .AddInMemoryExporter(exportedMetrics)
            .Build();
        using var spans = new SpanCapture(span => (string?)span.GetTagItem("sagat.worker") == worker, SdkInfo.ActivitySourceName);
        using var client = ClientOf(rest, space);
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "payments", Token = Token };
        options.Worker.Name = worker;
        await using var host = new WorkerHost(options);
        Activity? inside = null;
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, ct) =>
        {
            inside = Activity.Current;
            await client.GetAsync(Ids.Run, ct);
            Assert.Equal(Leases.Traceparent, context.Traceparent);
            return 1;
        });
        fake.State.Lease(Leases.Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => fake.State.Of("CompleteStep").Count == 1, "the completion");
        // Receiving the RPC precedes the response, outcome metrics and span disposal.
        await Eventually.UntilAsync(() => spans.Spans.Any(s => s.OperationName == "sagat.worker.task"), "the completed task span");

        Assert.NotNull(inside);
        Assert.Equal(SdkInfo.ActivitySourceName, inside.Source.Name);
        Assert.Equal("0af7651916cd43dd8448eb211c80319c", inside.TraceId.ToString());
        Assert.Equal("b7ad6b7169203331", inside.ParentSpanId.ToString());
        Assert.Equal(ActivityKind.Consumer, inside.Kind);
        Assert.Equal("0af7651916cd43dd8448eb211c80319c", Traceparent.TraceIdOf(rest.Requests[0].Header("traceparent")));

        var task = spans.Spans.Single(s => s.OperationName == "sagat.worker.task");
        Assert.Equal("run-1/charge/1", task.GetTagItem("sagat.task.id"));
        Assert.Equal("run-1", task.GetTagItem("sagat.run.id"));
        Assert.Equal("completed", task.GetTagItem("sagat.outcome"));
        Assert.DoesNotContain("lt-secret-lease-token", string.Concat(task.TagObjects.Select(t => t.Value)), StringComparison.Ordinal);
        Assert.DoesNotContain(Token, string.Concat(task.TagObjects.Select(t => t.Value)), StringComparison.Ordinal);
        Assert.True(meterProvider.ForceFlush());
        Assert.Contains(exportedMetrics, metric => metric.Name == "sagat.worker.tasks.taken");
        Assert.Contains(exportedMetrics, metric => metric.Name == "sagat.worker.tasks.completed");
        Assert.Contains(exportedMetrics, metric => metric.Name == "sagat.worker.activity.duration");

        // What the host reports back is the trace context of the run, as the lease gave it.
        Assert.Equal(Leases.Traceparent, fake.State.Of("CompleteStep")[0].Headers.GetValue("traceparent"));
    }

    [Fact]
    public async Task ALeaseWithoutATraceStartsATraceOfItsOwnAndAFailedTaskMarksItsSpan()
    {
        await using var fake = await FakeWorkerServer.StartAsync();
        var worker = "telemetry-" + Guid.NewGuid().ToString("N");
        using var spans = new SpanCapture(span => (string?)span.GetTagItem("sagat.worker") == worker, SdkInfo.ActivitySourceName);
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "payments", Token = Token };
        options.Worker.Name = worker;
        await using var host = new WorkerHost(options);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => throw ActivityFailure.Permanent("NO_FUNDS", "none"));
        fake.State.Lease(Leases.Lease(l => l.Traceparent = string.Empty));
        await host.StartAsync();
        await Eventually.UntilAsync(() => fake.State.Of("FailStep").Count == 1, "the failure");
        await Eventually.UntilAsync(() => spans.Spans.Any(s => s.OperationName == "sagat.worker.task"), "the span");
        var task = spans.Spans.Single(s => s.OperationName == "sagat.worker.task");
        Assert.Equal(default, task.ParentSpanId);
        Assert.Equal(ActivityStatusCode.Error, task.Status);
        Assert.Equal("NO_FUNDS", task.GetTagItem("error.type"));

        // Nothing was given to send back, so the span of the task is what the transport propagates.
        Assert.Equal(task.TraceId.ToString(), Traceparent.TraceIdOf(fake.State.Of("FailStep")[0].Headers.GetValue("traceparent")));
        Assert.Equal(string.Empty, ((Protocol.V1.FailStepRequest)fake.State.Of("FailStep")[0].Request).Traceparent);
    }
}
