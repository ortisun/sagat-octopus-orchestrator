using System.Net;
using System.Text;
using System.Text.Json.Nodes;
using Grpc.Core;
using Grpc.Core.Interceptors;

namespace SagatOctopusOrchestrator.Sdk.Conformance;

/// <summary>A fault of the scenario file (tests/contract/sdk-conformance/README.md, "Faults").</summary>
internal sealed record Fault(string Channel, string Match, string Action, int Times, int After, int? Status, string? Code, string? Reason, int? RetryAfterMs, JsonNode? Body)
{
    internal static Fault Of(JsonObject json) => new(
        (string)json["channel"]!,
        (string)json["match"]!,
        (string)json["action"]!,
        json["times"] is { } times ? (int)times : int.MaxValue,
        json["after"] is { } after ? (int)after : 0,
        json["status"] is { } status ? (int)status : null,
        (string?)json["code"],
        (string?)json["reason"],
        json["retryAfterMs"] is { } retry ? (int)retry : null,
        json["body"]);
}

/// <summary>
/// The faults of a scenario, one plan for every client and worker host of the run. A call is counted against every fault that
/// matches it, and the first fault in file order that is due answers it; <see cref="Applied"/> says, per fault and in file order, how
/// many calls it answered.
/// </summary>
internal sealed class FaultPlan
{
    private readonly object gate = new();
    private readonly List<Fault> faults;
    private readonly int[] seen;
    private readonly int[] answered;

    internal FaultPlan(IEnumerable<Fault> faults)
    {
        this.faults = [.. faults];
        seen = new int[this.faults.Count];
        answered = new int[this.faults.Count];
    }

    internal bool HasGrpc => faults.Any(fault => fault.Channel == "grpc");

    internal int[] Applied()
    {
        lock (gate)
        {
            return [.. answered];
        }
    }

    internal Fault? Next(string channel, string key)
    {
        lock (gate)
        {
            int? due = null;
            for (var i = 0; i < faults.Count; i++)
            {
                var fault = faults[i];
                if (fault.Channel != channel || !key.StartsWith(fault.Match, StringComparison.Ordinal))
                {
                    continue;
                }

                var count = seen[i]++;
                if (due is null && count >= fault.After && count < (long)fault.After + fault.Times)
                {
                    due = i;
                }
            }

            if (due is null)
            {
                return null;
            }

            answered[due.Value]++;
            return faults[due.Value];
        }
    }
}

/// <summary>The REST transport of the runner: what a fault says happens to the call, otherwise the real thing.</summary>
internal sealed class FaultHandler(FaultPlan plan, string apiPath, HttpMessageHandler inner) : DelegatingHandler(inner)
{
    protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        var path = request.RequestUri!.AbsolutePath;
        path = path.StartsWith(apiPath, StringComparison.Ordinal) ? path[apiPath.Length..] : path;
        var key = $"{request.Method.Method.ToUpperInvariant()} {(path.Length == 0 ? "/" : path)}";
        var fault = plan.Next("http", key);
        if (fault is null)
        {
            return await base.SendAsync(request, cancellationToken).ConfigureAwait(false);
        }

        switch (fault.Action)
        {
            case "failBefore":
                throw new HttpRequestException("connection refused");
            case "status":
                var response = new HttpResponseMessage((HttpStatusCode)(fault.Status ?? 503))
                {
                    Content = new StringContent(fault.Body?.ToJsonString() ?? "{}", Encoding.UTF8, "application/json"),
                };
                if (fault.RetryAfterMs is { } ms)
                {
                    response.Headers.TryAddWithoutValidation("Retry-After", ((int)Math.Ceiling(ms / 1000.0)).ToString(System.Globalization.CultureInfo.InvariantCulture));
                }

                return response;
            default:
                // dropResponse: the call is made, the server applies it, the answer is lost.
                using (var delivered = await base.SendAsync(request, cancellationToken).ConfigureAwait(false))
                {
                    await delivered.Content.ReadAsByteArrayAsync(cancellationToken).ConfigureAwait(false);
                }

                throw new HttpRequestException("connection reset");
        }
    }
}

/// <summary>The gRPC transport of the runner.</summary>
internal sealed class FaultInterceptor(FaultPlan plan) : Interceptor
{
    public override AsyncServerStreamingCall<TResponse> AsyncServerStreamingCall<TRequest, TResponse>(
        TRequest request,
        ClientInterceptorContext<TRequest, TResponse> context,
        AsyncServerStreamingCallContinuation<TRequest, TResponse> continuation)
    {
        var fault = plan.Next("grpc", context.Method.Name);
        if (fault is null)
        {
            return continuation(request, context);
        }

        if (fault.Action == "dropStreamAfterFirst")
        {
            var real = continuation(request, context);
            return new AsyncServerStreamingCall<TResponse>(
                new DropAfterFirstReader<TResponse>(real.ResponseStream, real.Dispose),
                real.ResponseHeadersAsync,
                real.GetStatus,
                real.GetTrailers,
                real.Dispose);
        }

        var code = fault.Action == "failBefore" ? StatusCode.Unavailable : CodeOf(fault.Code);
        var trailers = TrailersOf(fault);
        var status = new Status(code, "injected");
        return new AsyncServerStreamingCall<TResponse>(
            new FailingReader<TResponse>(status, trailers),
            Task.FromResult(new Metadata()),
            () => status,
            () => trailers,
            () => { });
    }

    public override AsyncUnaryCall<TResponse> AsyncUnaryCall<TRequest, TResponse>(
        TRequest request,
        ClientInterceptorContext<TRequest, TResponse> context,
        AsyncUnaryCallContinuation<TRequest, TResponse> continuation)
    {
        var fault = plan.Next("grpc", context.Method.Name);
        if (fault is null)
        {
            return continuation(request, context);
        }

        if (fault.Action == "dropResponse")
        {
            // The server applies the call; its answer is replaced by a failure to deliver it.
            var real = continuation(request, context);
            return new AsyncUnaryCall<TResponse>(Drop(real), real.ResponseHeadersAsync, real.GetStatus, real.GetTrailers, real.Dispose);
        }

        var code = fault.Action == "failBefore" ? StatusCode.Unavailable : CodeOf(fault.Code);
        var trailers = TrailersOf(fault);

        var status = new Status(code, "injected");

        // The call never reaches the server: there is nothing underneath.
        return new AsyncUnaryCall<TResponse>(
            Task.FromException<TResponse>(new RpcException(status, trailers)),
            Task.FromResult(new Metadata()),
            () => status,
            () => trailers,
            () => { });
    }

    private static Metadata TrailersOf(Fault fault)
    {
        var trailers = new Metadata();
        if (fault.Reason is not null)
        {
            trailers.Add("x-sagat-error", fault.Reason);
        }
        if (fault.RetryAfterMs is { } ms)
        {
            trailers.Add("retry-after-ms", ms.ToString(System.Globalization.CultureInfo.InvariantCulture));
        }
        return trailers;
    }

    private static async Task<TResponse> Drop<TResponse>(AsyncUnaryCall<TResponse> real)
    {
        try
        {
            await real.ResponseAsync.ConfigureAwait(false);
        }
        catch (RpcException)
        {
            // Whatever the server answered is lost either way.
        }

        throw new RpcException(new Status(StatusCode.Unavailable, "injected"));
    }

    private static StatusCode CodeOf(string? name) =>
        Enum.TryParse<StatusCode>((name ?? "UNAVAILABLE").Replace("_", string.Empty, StringComparison.Ordinal), ignoreCase: true, out var code) ? code : StatusCode.Unavailable;

    private sealed class FailingReader<T>(Status status, Metadata trailers) : IAsyncStreamReader<T>
    {
        public T Current => default!;

        public Task<bool> MoveNext(CancellationToken cancellationToken) =>
            Task.FromException<bool>(new RpcException(status, trailers));
    }

    private sealed class DropAfterFirstReader<T>(IAsyncStreamReader<T> inner, Action cancel) : IAsyncStreamReader<T>
    {
        private bool forwarded;
        private bool failed;

        public T Current => inner.Current;

        public async Task<bool> MoveNext(CancellationToken cancellationToken)
        {
            if (forwarded && !failed)
            {
                failed = true;
                cancel();
                throw new RpcException(new Status(StatusCode.Unavailable, "injected"));
            }

            var more = await inner.MoveNext(cancellationToken).ConfigureAwait(false);
            if (more && IsNonemptyTaskFrame(inner.Current))
            {
                forwarded = true;
            }
            return more;
        }

        private static bool IsNonemptyTaskFrame(T frame)
        {
            if (frame is null || frame.GetType().Name != "PollTasksResponse")
            {
                return false;
            }

            var tasks = frame.GetType().GetProperty("Tasks")?.GetValue(frame);
            if (tasks is not System.Collections.IEnumerable enumerable)
            {
                return false;
            }

            var iterator = enumerable.GetEnumerator();
            try
            {
                return iterator.MoveNext();
            }
            finally
            {
                (iterator as IDisposable)?.Dispose();
            }
        }
    }
}
