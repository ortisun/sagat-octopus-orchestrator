using System.Diagnostics;
using System.Net;
using System.Net.Http.Json;
using System.Text.Json.Nodes;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Hosting.Server;
using Microsoft.AspNetCore.Hosting.Server.Features;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;

namespace SagatOctopusOrchestrator.Sdk.Conformance;

/// <summary>
/// `run.sh aspnet`: US01-AC01 against the real platform. A minimal ASP.NET Core service registers the SDK through dependency
/// injection and starts a run from a request handler with the request's <c>Idempotency-Key</c> and no explicit trace: the trace
/// reaches the platform through the ambient <see cref="Activity"/> that ASP.NET Core made from the request's <c>traceparent</c>.
/// The runner calls its own service twice with the same key and one trace.
/// </summary>
internal static class AspNetMode
{
    internal static async Task<JsonObject> PlayAsync(JsonObject config, CancellationToken cancellationToken)
    {
        // ASP.NET Core makes the Activity of a request only when something observes it: an OpenTelemetry SDK does, and so does this
        // listener, which stands in for it (the sources are the ones OpenTelemetry's ASP.NET Core instrumentation and
        // `AddSource("SagatOctopusOrchestrator.Sdk")` would listen to).
        using var listener = new ActivityListener
        {
            ShouldListenTo = source => source.Name is "Microsoft.AspNetCore" or SdkInfo.ActivitySourceName,
            Sample = (ref ActivityCreationOptions<ActivityContext> _) => ActivitySamplingResult.AllDataAndRecorded,
        };
        ActivitySource.AddActivityListener(listener);

        var workflowId = (string)config["workflows"]!["order"]!["workflowId"]!;
        var builder = WebApplication.CreateSlimBuilder();
        builder.Logging.ClearProviders();
        builder.WebHost.ConfigureKestrel(options => options.Listen(IPAddress.Loopback, 0));
        builder.Services.AddSagatOctopusOrchestrator(options =>
        {
            options.ApiUrl = (string)config["apiUrl"]!;
            options.Namespace = (string)config["namespace"]!;
            options.Token = (string)config["clientToken"]!;
        });

        await using var app = builder.Build();
        app.MapPost("/orders", async (HttpRequest request, JsonObject order, ISagatClient sagat, CancellationToken requestAborted) =>
        {
            // No Traceparent option: the SDK takes the trace from Activity.Current.
            var run = await sagat.StartAsync(workflowId, order, new StartOptions { IdempotencyKey = request.Headers["Idempotency-Key"].ToString() }, requestAborted);
            return Results.Text(RunMode.RunJson(run).ToJsonString(), "application/json", statusCode: StatusCodes.Status202Accepted);
        });
        await app.StartAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var address = app.Services.GetRequiredService<IServer>().Features.Get<IServerAddressesFeature>()!.Addresses.First();
            var unique = (string)config["unique"]!;
            var key = $"aspnet-{unique}-1";
            var traceparent = Traceparent.New();
            var traceId = Traceparent.TraceIdOf(traceparent)!;

            using var http = new HttpClient { BaseAddress = new Uri(address) };
            var first = await CallAsync(http, key, traceparent, unique, cancellationToken).ConfigureAwait(false);
            var second = await CallAsync(http, key, traceparent, unique, cancellationToken).ConfigureAwait(false);
            return new JsonObject
            {
                ["sent"] = new JsonObject { ["idempotencyKey"] = key, ["traceparent"] = traceparent, ["traceId"] = traceId },
                ["first"] = first,
                ["second"] = second,
            };
        }
        finally
        {
            await app.StopAsync(CancellationToken.None).ConfigureAwait(false);
        }
    }

    private static async Task<JsonObject> CallAsync(HttpClient http, string key, string traceparent, string unique, CancellationToken cancellationToken)
    {
        using var request = new HttpRequestMessage(HttpMethod.Post, "/orders") { Content = JsonContent.Create(new JsonObject { ["orderId"] = $"order-{unique}" }) };
        request.Headers.TryAddWithoutValidation("Idempotency-Key", key);
        request.Headers.TryAddWithoutValidation("traceparent", traceparent);
        using var response = await http.SendAsync(request, cancellationToken).ConfigureAwait(false);
        var status = (int)response.StatusCode;
        if (status != StatusCodes.Status202Accepted)
        {
            throw new InvalidOperationException($"the service answered {status} to POST /orders");
        }

        var run = JsonNode.Parse(await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false))!.AsObject();
        return new JsonObject { ["status"] = status, ["run"] = run };
    }
}
