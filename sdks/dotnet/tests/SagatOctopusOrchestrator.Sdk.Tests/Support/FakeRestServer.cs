using System.Net;
using System.Text;
using System.Text.Json.Nodes;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Hosting.Server;
using Microsoft.AspNetCore.Hosting.Server.Features;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Server.Kestrel.Https;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;

namespace SagatOctopusOrchestrator.Sdk.Tests.Support;

/// <summary>What the fake server saw of one request.</summary>
internal sealed record SeenRequest(string Method, string PathAndQuery, IReadOnlyDictionary<string, string> Headers, string BodyText, string? ClientCertificate = null)
{
    internal JsonNode? Body => BodyText.Length == 0 ? null : JsonNode.Parse(BodyText);

    internal string Header(string name) => Headers.TryGetValue(name.ToLowerInvariant(), out var value) ? value : string.Empty;
}

/// <summary>What the fake server answers.</summary>
internal sealed record FakeAnswer(int Status, string? Body = null, IReadOnlyDictionary<string, string>? Headers = null)
{
    /// <summary>Writes the body as a stream (for an answer too large to hold in a string).</summary>
    internal Func<Stream, CancellationToken, Task>? Stream { get; init; }
}

/// <summary>An in-process HTTP server on a loopback port that records every request and answers as the test scripts it.</summary>
internal sealed class FakeRestServer : IAsyncDisposable
{
    private readonly WebApplication app;
    private readonly List<SeenRequest> seen = [];

    private FakeRestServer(WebApplication app, string baseUrl)
    {
        this.app = app;
        BaseUrl = baseUrl;
    }

    /// <summary>The API URL including <c>/v1</c>.</summary>
    internal string BaseUrl { get; }

    internal Func<SeenRequest, FakeAnswer> Handler { get; set; } = _ => new FakeAnswer(200, "{}");

    /// <summary>A delay before every answer, to play a server that is slow.</summary>
    internal TimeSpan Delay { get; set; }

    internal IReadOnlyList<SeenRequest> Requests
    {
        get
        {
            lock (seen)
            {
                return [.. seen];
            }
        }
    }

    internal static async Task<FakeRestServer> StartAsync(TestCertificate? https = null, bool requireClientCertificate = false)
    {
        var builder = WebApplication.CreateSlimBuilder();
        builder.Logging.ClearProviders();
        builder.WebHost.ConfigureKestrel(options => options.Listen(IPAddress.Loopback, 0, listen =>
        {
            if (https is not null)
            {
                listen.UseHttps(https.Server, https2 =>
                {
                    https2.ClientCertificateMode = requireClientCertificate ? ClientCertificateMode.RequireCertificate : ClientCertificateMode.NoCertificate;
                    https2.AllowAnyClientCertificate();
                });
            }
        }));
        var app = builder.Build();
        FakeRestServer? server = null;
        app.Run(async context =>
        {
            using var reader = new StreamReader(context.Request.Body, Encoding.UTF8);
            var text = await reader.ReadToEndAsync();
            var headers = context.Request.Headers.ToDictionary(pair => pair.Key.ToLowerInvariant(), pair => pair.Value.ToString());
            var request = new SeenRequest(context.Request.Method, context.Request.Path + context.Request.QueryString, headers, text, context.Connection.ClientCertificate?.Thumbprint);
            lock (server!.seen)
            {
                server.seen.Add(request);
            }

            if (server.Delay > TimeSpan.Zero)
            {
                await Task.Delay(server.Delay, context.RequestAborted);
            }

            var answer = server.Handler(request);
            context.Response.StatusCode = answer.Status;
            context.Response.Headers["x-trace-id"] = new string('f', 32);
            foreach (var (name, value) in answer.Headers ?? new Dictionary<string, string>())
            {
                context.Response.Headers[name] = value;
            }

            if (answer.Stream is not null)
            {
                context.Response.ContentType = "application/json";
                try
                {
                    await answer.Stream(context.Response.Body, context.RequestAborted);
                }
                catch (Exception error) when (error is IOException or OperationCanceledException or InvalidOperationException)
                {
                    // The client gave up on a body it will not read to the end.
                }
            }
            else if (answer.Body is not null)
            {
                context.Response.ContentType = "application/json";
                await context.Response.WriteAsync(answer.Body);
            }
        });
        await app.StartAsync();
        var address = app.Services.GetRequiredService<IServer>().Features.Get<IServerAddressesFeature>()!.Addresses.First();
        server = new FakeRestServer(app, address.TrimEnd('/') + "/v1");
        return server;
    }

    public async ValueTask DisposeAsync()
    {
        using var grace = new CancellationTokenSource(TimeSpan.FromSeconds(1));
        try
        {
            await app.StopAsync(grace.Token);
        }
        catch (OperationCanceledException)
        {
            // Connections that outlive the grace period are closed with the server.
        }

        await app.DisposeAsync();
    }
}
