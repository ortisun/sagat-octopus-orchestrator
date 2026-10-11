using Microsoft.Extensions.Hosting;
using SagatOctopusOrchestrator.Sdk.Workers;

namespace SagatOctopusOrchestrator.Sdk.DependencyInjection;

/// <summary>
/// Runs the <see cref="IWorkerHost"/> with the application: it registers with the worker service when the host starts (a refusal
/// such as <see cref="SagatErrorCode.NotBound"/> stops the application from starting) and stops it gracefully when the host stops:
/// it waits for the tasks in flight, up to <see cref="WorkerOptions.ShutdownTimeout"/> or the host's own shutdown timeout, whichever
/// ends first, and cancels what is left with the reason <see cref="CancellationReasons.Shutdown"/>.
/// </summary>
internal sealed class SagatWorkerService(IWorkerHost worker) : IHostedService
{
    public Task StartAsync(CancellationToken cancellationToken) => worker.StartAsync(cancellationToken);

    public Task StopAsync(CancellationToken cancellationToken) => worker.StopAsync(null, cancellationToken);
}

/// <summary>The handlers registered through the builder, applied to the worker host when the container creates it.</summary>
internal sealed class SagatWorkerRegistrations
{
    internal List<Action<IWorkerHost, IServiceProvider>> Items { get; } = [];
}
