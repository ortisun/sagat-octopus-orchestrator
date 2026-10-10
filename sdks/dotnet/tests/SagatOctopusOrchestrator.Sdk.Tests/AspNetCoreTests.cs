using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.TestHost;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>
/// US01-AC01: given an ASP.NET Core service, when it registers a client via dependency injection and calls <c>StartAsync</c>, then a run is
/// created with the idempotency key and the trace context propagated.
/// </summary>
[Collection("telemetry")]
public sealed class AspNetCoreTests
{
    private const string Workflow = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    private const string Incoming = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    [Fact]
    public async Task AnEndpointThatInjectsTheClientStartsARunWithTheKeyAndTheTraceOfTheRequest()
    {
        await using var server = await FakeRestServer.StartAsync();
        server.Handler = request => request.Method == "GET" && request.PathAndQuery.StartsWith("/v1/workflows", StringComparison.Ordinal)
            ? new FakeAnswer(200, $$"""{"items":[{"id":"{{Workflow}}","name":"order-flow"}],"nextCursor":null}""")
            : new FakeAnswer(202, $$"""{"runId":"0190aaaa-0000-7000-8000-000000000001","workflowId":"{{Workflow}}","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z","traceId":"4bf92f3577b34da6a3ce929d0e0e4736","correlationId":"o-1"}""");

        // The hosting layer starts the request's activity only when somebody listens: an application with OpenTelemetry does.
        using var spans = new SpanCapture(span => span.TraceId.ToString() == "4bf92f3577b34da6a3ce929d0e0e4736", "Microsoft.AspNetCore", SdkInfo.ActivitySourceName);

        var builder = WebApplication.CreateBuilder();
        builder.WebHost.UseTestServer();
        builder.Logging.ClearProviders();
        builder.Services.AddSagatOctopusOrchestrator(options =>
        {
            options.ApiUrl = server.BaseUrl;
            options.Namespace = "payments";
            options.Token = "tok-aspnet";
            options.UserAgent = "orders-api/1.0";
        });
        await using var app = builder.Build();
        app.MapPost("/orders/{id}", async (string id, ISagatClient sagat, CancellationToken cancellationToken) =>
        {
            var run = await sagat.StartAsync("order-flow", new { orderId = id }, new StartOptions { IdempotencyKey = $"order-{id}", CorrelationId = id }, cancellationToken);
            return Results.Accepted($"/runs/{run.RunId}", new { run.RunId, run.Status, run.TraceId });
        });
        await app.StartAsync();
        using var http = app.GetTestClient();

        using var request = new HttpRequestMessage(HttpMethod.Post, "/orders/o-1");
        request.Headers.TryAddWithoutValidation("traceparent", Incoming);
        using var response = await http.SendAsync(request);

        Assert.Equal(HttpStatusCode.Accepted, response.StatusCode);
        using var body = JsonDocument.Parse(await response.Content.ReadAsStringAsync());
        Assert.Equal("0190aaaa-0000-7000-8000-000000000001", body.RootElement.GetProperty("runId").GetString());
        Assert.Equal("RUNNING", body.RootElement.GetProperty("status").GetString());

        var start = Assert.Single(server.Requests, r => r.Method == "POST");
        Assert.Equal("/v1/runs", start.PathAndQuery);
        Assert.Equal("order-o-1", start.Header("idempotency-key"));
        Assert.Equal("Bearer tok-aspnet", start.Header("authorization"));
        Assert.Equal("payments", start.Header("x-octopus-namespace"));
        Assert.EndsWith(" orders-api/1.0", start.Header("user-agent"), StringComparison.Ordinal);
        Assert.Equal($$"""{"workflowId":"{{Workflow}}","input":{"orderId":"o-1"},"correlationId":"o-1"}""", start.BodyText);

        // The trace of the incoming request is the trace of the run: same trace id, and a span of the SDK in between.
        var sent = start.Header("traceparent");
        Assert.True(Traceparent.IsValid(sent), sent);
        Assert.Equal("4bf92f3577b34da6a3ce929d0e0e4736", Traceparent.TraceIdOf(sent));
        Assert.NotEqual("00f067aa0ba902b7", sent.Split('-')[2]);
        var sdkSpan = Assert.Single(spans.Spans, s => s.Source.Name == SdkInfo.ActivitySourceName && (string?)s.GetTagItem("sagat.operation") == "start");
        Assert.Equal(sdkSpan.Id, sent);
        var hosting = Assert.Single(spans.Spans, s => s.Source.Name == "Microsoft.AspNetCore" && s.ParentSpanId.ToString() == "00f067aa0ba902b7");
        Assert.Equal(hosting.SpanId, sdkSpan.ParentSpanId);

        // The server (here, the fake one, which is instrumented too) continues the same trace from the span of the SDK.
        Assert.Contains(spans.Spans, s => s.Source.Name == "Microsoft.AspNetCore" && s.ParentSpanId == sdkSpan.SpanId);

        // The same client serves the next request, and a request with no trace at all starts a run without one.
        var second = await http.PostAsync("/orders/o-2", null);
        Assert.Equal(HttpStatusCode.Accepted, second.StatusCode);
        Assert.Same(app.Services.GetRequiredService<ISagatClient>(), app.Services.GetRequiredService<ISagatClient>());
        Assert.Equal("order-o-2", server.Requests.Last(r => r.Method == "POST").Header("idempotency-key"));
        Assert.Equal(1, server.Requests.Count(r => r.PathAndQuery.StartsWith("/v1/workflows", StringComparison.Ordinal)));
    }

    [Fact]
    public async Task ARequestAbortedByTheCallerCancelsTheCallToTheOrchestrator()
    {
        await using var server = await FakeRestServer.StartAsync();
        server.Delay = TimeSpan.FromSeconds(10);
        var builder = WebApplication.CreateBuilder();
        builder.WebHost.UseTestServer();
        builder.Logging.ClearProviders();
        builder.Services.AddSagatOctopusOrchestrator(options =>
        {
            options.ApiUrl = server.BaseUrl;
            options.Namespace = "payments";
            options.Token = "tok";
        });
        await using var app = builder.Build();
        var cancelled = new TaskCompletionSource<bool>();
        app.MapGet("/runs/{id}", async (string id, ISagatClient sagat, CancellationToken cancellationToken) =>
        {
            try
            {
                return Results.Ok(await sagat.GetAsync(id, cancellationToken));
            }
            catch (OperationCanceledException)
            {
                cancelled.SetResult(true);
                throw;
            }
        });
        await app.StartAsync();
        using var http = app.GetTestClient();
        using var abort = new CancellationTokenSource(TimeSpan.FromMilliseconds(200));
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => http.GetAsync("/runs/0190aaaa-0000-7000-8000-000000000001", abort.Token));
        Assert.True(await cancelled.Task.WaitAsync(TimeSpan.FromSeconds(10)));
    }

    /// <summary>The flow of the runner's <c>aspnet</c> mode, on a loopback port, against the fake server: the key of the request header, no explicit trace.</summary>
    [Fact]
    public async Task ARealListeningServiceStartsOneRunForTwoCallsWithTheSameKeyAndTheTraceOfTheRequestWithoutAnExplicitTrace()
    {
        await using var platform = await FakeRestServer.StartAsync();
        var runs = new Dictionary<string, string>();
        platform.Handler = request =>
        {
            if (request.Method != "POST" || !request.PathAndQuery.EndsWith("/runs", StringComparison.Ordinal))
            {
                return new FakeAnswer(404, "{}");
            }

            var key = request.Header("idempotency-key");
            if (!runs.TryGetValue(key, out var run))
            {
                var traceId = Traceparent.TraceIdOf(request.Header("traceparent"));
                run = $$"""{"runId":"0190aaaa-0000-7000-8000-00000000000{{runs.Count + 1}}","workflowId":"{{Workflow}}","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z","traceId":{{(traceId is null ? "null" : $"\"{traceId}\"")}}}""";
                runs[key] = run;
            }

            return new FakeAnswer(202, run);
        };
        var sent = Traceparent.New();
        var traceId = Traceparent.TraceIdOf(sent)!;
        using var spans = new SpanCapture(span => span.TraceId.ToString() == traceId, "Microsoft.AspNetCore", SdkInfo.ActivitySourceName);

        var builder = WebApplication.CreateSlimBuilder();
        builder.Logging.ClearProviders();
        builder.WebHost.ConfigureKestrel(options => options.Listen(IPAddress.Loopback, 0));
        builder.Services.AddSagatOctopusOrchestrator(options =>
        {
            options.ApiUrl = platform.BaseUrl;
            options.Namespace = "payments";
            options.Token = "tok-aspnet";
        });
        await using var app = builder.Build();
        app.MapPost("/orders", async (HttpRequest request, System.Text.Json.Nodes.JsonObject order, ISagatClient sagat, CancellationToken aborted) =>
        {
            var run = await sagat.StartAsync(Workflow, order, new StartOptions { IdempotencyKey = request.Headers["Idempotency-Key"].ToString() }, aborted);
            return Results.Text(run.Raw.GetRawText(), "application/json", statusCode: StatusCodes.Status202Accepted);
        });
        await app.StartAsync();
        var address = app.Services.GetRequiredService<Microsoft.AspNetCore.Hosting.Server.IServer>().Features
            .Get<Microsoft.AspNetCore.Hosting.Server.Features.IServerAddressesFeature>()!.Addresses.First();

        using var http = new HttpClient { BaseAddress = new Uri(address) };
        var answers = new List<string>();
        for (var i = 0; i < 2; i++)
        {
            using var request = new HttpRequestMessage(HttpMethod.Post, "/orders") { Content = JsonContent.Create(new { orderId = "o-1" }) };
            request.Headers.TryAddWithoutValidation("Idempotency-Key", "aspnet-loopback-1");
            request.Headers.TryAddWithoutValidation("traceparent", sent);
            using var response = await http.SendAsync(request);
            Assert.Equal(HttpStatusCode.Accepted, response.StatusCode);
            answers.Add(await response.Content.ReadAsStringAsync());
        }

        Assert.Equal(answers[0], answers[1]);
        using var run = JsonDocument.Parse(answers[0]);
        Assert.Equal(traceId, run.RootElement.GetProperty("traceId").GetString());
        Assert.Equal(["aspnet-loopback-1", "aspnet-loopback-1"], platform.Requests.Select(r => r.Header("idempotency-key")));
        Assert.All(platform.Requests, r => Assert.Equal(traceId, Traceparent.TraceIdOf(r.Header("traceparent"))));
        Assert.All(platform.Requests, r => Assert.NotEqual(sent, r.Header("traceparent")));
        Assert.Equal(2, spans.Spans.Count(s => s.Source.Name == SdkInfo.ActivitySourceName));
    }
}
