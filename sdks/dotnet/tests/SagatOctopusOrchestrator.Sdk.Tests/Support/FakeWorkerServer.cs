using System.Net;
using Grpc.Core;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Hosting.Server;
using Microsoft.AspNetCore.Hosting.Server.Features;
using Microsoft.AspNetCore.Server.Kestrel.Core;
using Microsoft.AspNetCore.Server.Kestrel.Https;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Protocol.V1;

namespace SagatOctopusOrchestrator.Sdk.Tests.Support;

internal sealed record Scripted(StatusCode Code, string? Reason = null, long? RetryAfterMs = null);

internal sealed record RecordedCall(string Method, object Request, Metadata Headers, string? ClientCertificate = null)
{
    /// <summary>When the call arrived, on the monotonic clock.</summary>
    internal long At { get; } = System.Diagnostics.Stopwatch.GetTimestamp();
}

/// <summary>The state of the fake worker service: what it was called with and what it answers. Like the real service, it never leases more tasks than were asked for.</summary>
internal sealed class FakeWorkerState
{
    private readonly object gate = new();
    private readonly List<RecordedCall> calls = [];
    private readonly List<TaskLease> leases = [];
    private readonly Dictionary<string, Queue<Scripted>> failures = [];

    private int polls;

    internal string AcceptedVersion { get; set; } = "1.2";

    /// <summary>A server that hands out every lease it holds, whatever <c>max_tasks</c> says.</summary>
    internal bool IgnoreMax { get; set; }

    /// <summary>Called when a <c>CompleteStep</c> arrives, to play a platform that releases the next task at once.</summary>
    internal Action? OnComplete { get; set; }

    /// <summary>Takes the leases when a poll arrives and answers after <see cref="PollDelay"/>: a task released meanwhile is for the next poll.</summary>
    internal bool TakeBeforeDelay { get; set; }

    internal int Polls => Volatile.Read(ref polls);

    internal void CountPoll() => Interlocked.Increment(ref polls);

    /// <summary>The same error for the next <paramref name="times"/> calls of a method.</summary>
    internal void FailAlways(string method, Scripted error, int times = 1_000_000) => Fail(method, Enumerable.Repeat(error, times).ToArray());

    internal bool CancelRequested { get; set; }

    internal long HeartbeatExpiresMs { get; set; } = DateTimeOffset.UtcNow.AddSeconds(60).ToUnixTimeMilliseconds();

    internal List<ResolvedSecret> Secrets { get; } = [];

    /// <summary>How long a poll takes to answer, and the wait the answer asks for when it brings no task.</summary>
    internal TimeSpan PollDelay { get; set; }

    internal int PollRetryAfterMs { get; set; } = 50;

    internal int Registrations => Of("RegisterWorker").Count;

    internal void Lease(params TaskLease[] more)
    {
        lock (gate)
        {
            leases.AddRange(more);
        }
    }

    /// <summary>Errors to answer a method with before it succeeds, consumed in order.</summary>
    internal void Fail(string method, params Scripted[] errors)
    {
        lock (gate)
        {
            if (!failures.TryGetValue(method, out var queue))
            {
                failures[method] = queue = new Queue<Scripted>();
            }

            foreach (var error in errors)
            {
                queue.Enqueue(error);
            }
        }
    }

    internal List<RecordedCall> Of(string method)
    {
        lock (gate)
        {
            return [.. calls.Where(call => call.Method == method)];
        }
    }

    internal void Record(string method, object request, Metadata headers, string? clientCertificate = null)
    {
        lock (gate)
        {
            calls.Add(new RecordedCall(method, request, headers, clientCertificate));
        }
    }

    internal Scripted? NextFailure(string method)
    {
        lock (gate)
        {
            return failures.TryGetValue(method, out var queue) && queue.Count > 0 ? queue.Dequeue() : null;
        }
    }

    internal List<TaskLease> Take(int max)
    {
        lock (gate)
        {
            var taken = leases.Take(max).ToList();
            leases.RemoveRange(0, taken.Count);
            return taken;
        }
    }
}

internal sealed class FakeWorkerService(FakeWorkerState state) : WorkerService.WorkerServiceBase
{
    public override Task<RegisterWorkerResponse> RegisterWorker(RegisterWorkerRequest request, ServerCallContext context) =>
        Answer("RegisterWorker", request, context, () => new RegisterWorkerResponse { WorkerId = "w-1", SessionToken = "session-1", AcceptedProtocolVersion = state.AcceptedVersion });

    public override async Task<PollTasksResponse> PollTasks(PollTasksRequest request, ServerCallContext context)
    {
        state.Record("PollTasks", request, context.RequestHeaders, ClientCertificateOf(context));
        state.CountPoll();
        Throw("PollTasks");
        var max = state.IgnoreMax ? int.MaxValue : request.MaxTasks;
        var early = state.TakeBeforeDelay ? state.Take(max) : null;
        if (state.PollDelay > TimeSpan.Zero)
        {
            await Task.Delay(state.PollDelay);
        }

        var answer = new PollTasksResponse { RetryAfterMs = state.PollRetryAfterMs };
        answer.Tasks.AddRange(early ?? state.Take(max));
        return answer;
    }

    public override Task<HeartbeatResponse> Heartbeat(HeartbeatRequest request, ServerCallContext context) =>
        Answer("Heartbeat", request, context, () => new HeartbeatResponse { LeaseExpiresAtEpochMs = state.HeartbeatExpiresMs, CancelRequested = state.CancelRequested });

    public override Task<StepCommandResponse> CompleteStep(CompleteStepRequest request, ServerCallContext context) =>
        Answer("CompleteStep", request, context, () =>
        {
            state.OnComplete?.Invoke();
            return new StepCommandResponse { CommandId = "c", StepStatus = "SUCCEEDED", RunStatus = "RUNNING" };
        });

    public override Task<StepCommandResponse> FailStep(FailStepRequest request, ServerCallContext context) =>
        Answer("FailStep", request, context, () => new StepCommandResponse { CommandId = "c", StepStatus = "FAILED", RunStatus = "RUNNING" });

    public override Task<ResolveSecretsResponse> ResolveSecrets(ResolveSecretsRequest request, ServerCallContext context) =>
        Answer("ResolveSecrets", request, context, () =>
        {
            var answer = new ResolveSecretsResponse();
            answer.Secrets.AddRange(state.Secrets);
            return answer;
        });

    private Task<T> Answer<T>(string method, object request, ServerCallContext context, Func<T> answer)
    {
        state.Record(method, request, context.RequestHeaders, ClientCertificateOf(context));
        Throw(method);
        return Task.FromResult(answer());
    }

    private static string? ClientCertificateOf(ServerCallContext context) => context.GetHttpContext().Connection.ClientCertificate?.Thumbprint;

    private void Throw(string method)
    {
        if (state.NextFailure(method) is not { } scripted)
        {
            return;
        }

        var trailers = new Metadata();
        if (scripted.Reason is not null)
        {
            trailers.Add("x-sagat-error", scripted.Reason);
        }

        if (scripted.RetryAfterMs is { } ms)
        {
            trailers.Add("retry-after-ms", ms.ToString(System.Globalization.CultureInfo.InvariantCulture));
        }

        throw new RpcException(new Status(scripted.Code, "scripted"), trailers);
    }
}

/// <summary>An in-process gRPC server (HTTP/2 without TLS on a loopback port) that plays the worker service.</summary>
internal sealed class FakeWorkerServer : IAsyncDisposable
{
    private readonly WebApplication app;

    private FakeWorkerServer(WebApplication app, string url, FakeWorkerState state)
    {
        this.app = app;
        Url = url;
        State = state;
    }

    internal string Url { get; }

    internal FakeWorkerState State { get; }

    internal static async Task<FakeWorkerServer> StartAsync(TestCertificate? https = null, bool requireClientCertificate = false)
    {
        var state = new FakeWorkerState();
        var builder = WebApplication.CreateSlimBuilder();
        builder.Logging.ClearProviders();
        builder.WebHost.ConfigureKestrel(options => options.Listen(IPAddress.Loopback, 0, listen =>
        {
            listen.Protocols = HttpProtocols.Http2;
            if (https is not null)
            {
                listen.UseHttps(https.Server, options =>
                {
                    options.ClientCertificateMode = requireClientCertificate ? ClientCertificateMode.RequireCertificate : ClientCertificateMode.NoCertificate;
                    options.AllowAnyClientCertificate();
                });
            }
        }));
        builder.Services.AddGrpc();
        builder.Services.AddSingleton(state);
        var app = builder.Build();
        app.MapGrpcService<FakeWorkerService>();
        await app.StartAsync();
        var address = app.Services.GetRequiredService<IServer>().Features.Get<IServerAddressesFeature>()!.Addresses.First();
        return new FakeWorkerServer(app, address, state);
    }

    public async ValueTask DisposeAsync()
    {
        using var grace = new CancellationTokenSource(TimeSpan.FromSeconds(1));
        try
        {
            await app.StopAsync(grace.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            // Connections that outlive the grace period are closed with the server.
        }

        await app.DisposeAsync().ConfigureAwait(false);
    }
}
