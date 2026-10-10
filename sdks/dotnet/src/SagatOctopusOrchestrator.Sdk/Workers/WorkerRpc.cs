using Grpc.Core;
using Grpc.Core.Interceptors;
using Grpc.Net.Client;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Internal;

namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>The typed, authenticated, error-mapped worker RPCs.</summary>
internal sealed class WorkerRpc : IDisposable
{
    private readonly SagatOptions options;
    private readonly GrpcChannel channel;
    private readonly WorkerService.WorkerServiceClient client;

    internal WorkerRpc(SagatOptions options, ILogger logger)
    {
        this.options = options;
        var url = new Uri(options.GrpcUrl!, UriKind.Absolute);
        if (Tls.IsPlainTextOutsideLoopback(url))
        {
            Log.InsecureTransport(logger, url.Host);
        }

        var handler = Tls.CreateHandler(options.Tls, options.Timeout);
        handler.KeepAlivePingDelay = TimeSpan.FromSeconds(60);
        handler.KeepAlivePingTimeout = TimeSpan.FromSeconds(20);
        channel = GrpcChannel.ForAddress(url, new GrpcChannelOptions
        {
            HttpHandler = Tls.WithServerName(handler, options.Tls),
            DisposeHttpClient = true,
            MaxReceiveMessageSize = 16 * 1024 * 1024,
        });
        var invoker = channel.CreateCallInvoker();
        if (options.Worker.Interceptors.Count > 0)
        {
            invoker = invoker.Intercept([.. options.Worker.Interceptors]);
        }

        client = new WorkerService.WorkerServiceClient(invoker);
    }

    public void Dispose() => channel.Dispose();

    internal Task<RegisterWorkerResponse> RegisterAsync(RegisterWorkerRequest request, CancellationToken cancellationToken) =>
        CallAsync("RegisterWorker", null, (headers, call) => client.RegisterWorkerAsync(request, Options(headers, call, cancellationToken)), cancellationToken);

    internal Task<PollTasksResponse> PollAsync(PollTasksRequest request, CancellationToken cancellationToken) =>
        CallAsync("PollTasks", null, (headers, call) => client.PollTasksAsync(request, Options(headers, call, cancellationToken)), cancellationToken);

    internal Task<HeartbeatResponse> HeartbeatAsync(HeartbeatRequest request, string? traceparent, CancellationToken cancellationToken) =>
        CallAsync("Heartbeat", traceparent, (headers, call) => client.HeartbeatAsync(request, Options(headers, call, cancellationToken)), cancellationToken);

    internal Task<StepCommandResponse> CompleteStepAsync(CompleteStepRequest request, string? traceparent, CancellationToken cancellationToken) =>
        CallAsync("CompleteStep", traceparent, (headers, call) => client.CompleteStepAsync(request, Options(headers, call, cancellationToken)), cancellationToken);

    internal Task<StepCommandResponse> FailStepAsync(FailStepRequest request, string? traceparent, CancellationToken cancellationToken) =>
        CallAsync("FailStep", traceparent, (headers, call) => client.FailStepAsync(request, Options(headers, call, cancellationToken)), cancellationToken);

    internal Task<ResolveSecretsResponse> ResolveSecretsAsync(ResolveSecretsRequest request, string? traceparent, CancellationToken cancellationToken) =>
        CallAsync("ResolveSecrets", traceparent, (headers, call) => client.ResolveSecretsAsync(request, Options(headers, call, cancellationToken)), cancellationToken);

    private static Grpc.Core.CallOptions Options(Metadata headers, DateTime deadline, CancellationToken cancellationToken) =>
        new(headers: headers, deadline: deadline, cancellationToken: cancellationToken);

    private async Task<TResponse> CallAsync<TResponse>(
        string method,
        string? traceparent,
        Func<Metadata, DateTime, AsyncUnaryCall<TResponse>> start,
        CancellationToken cancellationToken)
    {
        var token = await options.ResolveTokenAsync(cancellationToken).ConfigureAwait(false);
        var headers = new Metadata { { "authorization", "Bearer " + token } };
        if (!string.IsNullOrEmpty(traceparent))
        {
            headers.Add("traceparent", Traceparent.Check(traceparent));
        }

        try
        {
            using var call = start(headers, DateTime.UtcNow + options.Timeout);
            return await call.ResponseAsync.ConfigureAwait(false);
        }
        catch (RpcException error)
        {
            cancellationToken.ThrowIfCancellationRequested();
            throw GrpcErrors.Map(method, error);
        }
        catch (Exception error) when (error is HttpRequestException or IOException or ObjectDisposedException)
        {
            cancellationToken.ThrowIfCancellationRequested();
            throw new SagatException(SagatErrorCode.Transport, "no answer from the worker service");
        }
    }
}
