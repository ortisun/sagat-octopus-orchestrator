using System.Collections.Concurrent;
using System.Reflection;
using Grpc.Core;
using Grpc.Core.Interceptors;
using SagatOctopusOrchestrator.Sdk.Workers;

namespace SagatOctopusOrchestrator.Sdk.Tests.Support;

/// <summary>
/// What a worker host sent, in the order it sent it ("sent Heartbeat"), with the marks a test leaves among the entries. The side of the
/// host: a beat sent before the report but received by the fake after it cannot blur what the test reads, as counting what reached the fake
/// does (AGENTS.md Rule 18).
/// </summary>
internal sealed class SendLog : Interceptor
{
    private readonly ConcurrentQueue<string> entries = new();

    internal IReadOnlyList<string> Entries => [.. entries];

    internal void Mark(string what) => entries.Enqueue(what);

    /// <summary>How many times <paramref name="entry"/> comes after the last <paramref name="mark"/>.</summary>
    internal int After(string mark, string entry)
    {
        var all = Entries.ToList();
        return all.Skip(all.LastIndexOf(mark) + 1).Count(e => e == entry);
    }

    public override AsyncUnaryCall<TResponse> AsyncUnaryCall<TRequest, TResponse>(
        TRequest request,
        ClientInterceptorContext<TRequest, TResponse> context,
        AsyncUnaryCallContinuation<TRequest, TResponse> continuation)
    {
        entries.Enqueue("sent " + context.Method.Name);
        return continuation(request, context);
    }

    /// <summary>
    /// The tasks <paramref name="host"/> has in flight. A task leaves only once its heartbeat has been cancelled and awaited, so from then on
    /// no beat of it can begin. Read by reflection, as <see cref="RecordingClock"/> sets the clock: the tests compile the protocol types
    /// themselves and cannot be given the internals.
    /// </summary>
    internal static int InFlight(WorkerHost host)
    {
        var field = typeof(WorkerHost).GetField("inflight", BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("the worker host no longer has a field named inflight");
        return ((System.Collections.ICollection)field.GetValue(host)!).Count;
    }
}
