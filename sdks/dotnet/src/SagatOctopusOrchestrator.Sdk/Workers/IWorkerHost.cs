namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>
/// The worker host (SPEC section 6): registers activities with the worker service, takes leases, heartbeats, runs the handlers and
/// reports their results. Register every handler before <see cref="StartAsync"/>.
/// </summary>
public interface IWorkerHost : IAsyncDisposable
{
    /// <summary>The protocol version the server agreed to (<c>1.2</c>); empty before <see cref="StartAsync"/>.</summary>
    string ProtocolVersion { get; }

    /// <summary>True between a successful <see cref="StartAsync"/> and the end of <see cref="StopAsync"/>.</summary>
    bool IsRunning { get; }

    /// <summary>Registers the handler of an activity. The input is read from the run's input as <typeparamref name="TInput"/>; the result is written as JSON.</summary>
    IWorkerHost RegisterActivity<TInput, TOutput>(string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task<TOutput>> handler);

    /// <summary>Registers the handler of an activity that has no result.</summary>
    IWorkerHost RegisterActivity<TInput>(string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task> handler);

    /// <summary>Registers the handler of the activity a node names in <c>compensateWith</c>. It must be idempotent and tolerate an effect that never happened.</summary>
    IWorkerHost RegisterCompensation<TInput, TOutput>(string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task<TOutput>> handler);

    /// <summary>Registers the handler of a compensation that has no result.</summary>
    IWorkerHost RegisterCompensation<TInput>(string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task> handler);

    /// <summary>Registers the handler of the activity a node names in <c>reconcileWith</c>; it answers a <see cref="Verdict"/>.</summary>
    IWorkerHost RegisterReconciler<TInput>(string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task<Verdict>> handler);

    /// <summary>Registers with the server and starts the loop; completes once registered.</summary>
    Task StartAsync(CancellationToken cancellationToken = default);

    /// <summary>
    /// Stops polling, waits up to <paramref name="timeout"/> (<see cref="WorkerOptions.ShutdownTimeout"/> by default) for the tasks in flight,
    /// then cancels the ones left with the reason <see cref="CancellationReasons.Shutdown"/>. A task that did not finish keeps its lease until it
    /// lapses. Cancelling <paramref name="cancellationToken"/> stops waiting at once.
    /// </summary>
    Task StopAsync(TimeSpan? timeout = null, CancellationToken cancellationToken = default);
}
