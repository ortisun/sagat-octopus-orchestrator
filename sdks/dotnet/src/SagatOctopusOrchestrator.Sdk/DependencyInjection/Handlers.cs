using SagatOctopusOrchestrator.Sdk.Workers;

namespace SagatOctopusOrchestrator.Sdk.DependencyInjection;

/// <summary>
/// An activity (or a compensation) as a class: it is resolved from the dependency injection container in a new scope for every task,
/// so it may use scoped services. Register it with <c>AddActivity&lt;THandler&gt;(name)</c> or <c>AddCompensation&lt;THandler&gt;(name)</c>.
/// </summary>
/// <typeparam name="TInput">The type the run input is read as.</typeparam>
/// <typeparam name="TOutput">The type of the result, written as JSON.</typeparam>
public interface IActivity<in TInput, TOutput>
{
    /// <summary>Runs one attempt. <paramref name="cancellationToken"/> is the one of <paramref name="context"/>.</summary>
    Task<TOutput> ExecuteAsync(TInput input, ActivityExecutionContext context, CancellationToken cancellationToken);
}

/// <summary>A reconciler as a class: it answers a <see cref="Verdict"/>. Register it with <c>AddReconciler&lt;THandler&gt;(name)</c>.</summary>
/// <typeparam name="TInput">The type the run input is read as.</typeparam>
public interface IReconciler<in TInput>
{
    /// <summary>Finds out whether the effect of the step <see cref="ActivityExecutionContext.TargetNode"/> happened.</summary>
    Task<Verdict> ReconcileAsync(TInput input, ActivityExecutionContext context, CancellationToken cancellationToken);
}

/// <summary>What the dependency injection integration is building: the services behind <c>AddSagatOctopusOrchestrator</c>.</summary>
public interface ISagatBuilder
{
    /// <summary>The service collection the SDK registers into.</summary>
    Microsoft.Extensions.DependencyInjection.IServiceCollection Services { get; }
}
