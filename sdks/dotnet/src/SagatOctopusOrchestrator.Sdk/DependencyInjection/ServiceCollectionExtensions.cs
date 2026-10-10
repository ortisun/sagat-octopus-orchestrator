using System.Reflection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using SagatOctopusOrchestrator.Sdk;
using SagatOctopusOrchestrator.Sdk.DependencyInjection;
using SagatOctopusOrchestrator.Sdk.Workers;

namespace Microsoft.Extensions.DependencyInjection;

/// <summary>Registers the SDK with <c>Microsoft.Extensions.DependencyInjection</c>.</summary>
public static class SagatServiceCollectionExtensions
{
    /// <summary>
    /// Registers the client (<see cref="ISagatClient"/> and <see cref="SagatClient"/>, a singleton with its own pooled connections) and
    /// <see cref="SagatOptions"/> (also bindable from configuration). Add activities with <c>AddActivity</c>, <c>AddCompensation</c> and
    /// <c>AddReconciler</c> to get an <see cref="IWorkerHost"/> that runs as a hosted service.
    /// </summary>
    public static ISagatBuilder AddSagatOctopusOrchestrator(this IServiceCollection services, Action<SagatOptions>? configure = null)
    {
        ArgumentNullException.ThrowIfNull(services);
        var options = services.AddOptions<SagatOptions>();
        if (configure is not null)
        {
            options.Configure(configure);
        }

        services.TryAddSingleton(sp => new SagatClient(sp.GetRequiredService<IOptions<SagatOptions>>().Value, sp.GetRequiredService<ILogger<SagatClient>>()));
        services.TryAddSingleton<ISagatClient>(sp => sp.GetRequiredService<SagatClient>());
        return new SagatBuilder(services);
    }

    /// <summary>Registers the handler of an activity.</summary>
    public static ISagatBuilder AddActivity<TInput, TOutput>(this ISagatBuilder builder, string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task<TOutput>> handler) =>
        Add(builder, host => host.RegisterActivity(name, handler));

    /// <summary>Registers the handler of an activity that has no result.</summary>
    public static ISagatBuilder AddActivity<TInput>(this ISagatBuilder builder, string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task> handler) =>
        Add(builder, host => host.RegisterActivity(name, handler));

    /// <summary>Registers the handler of the activity a node names in <c>compensateWith</c>.</summary>
    public static ISagatBuilder AddCompensation<TInput, TOutput>(this ISagatBuilder builder, string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task<TOutput>> handler) =>
        Add(builder, host => host.RegisterCompensation(name, handler));

    /// <summary>Registers the handler of a compensation that has no result.</summary>
    public static ISagatBuilder AddCompensation<TInput>(this ISagatBuilder builder, string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task> handler) =>
        Add(builder, host => host.RegisterCompensation(name, handler));

    /// <summary>Registers the handler of the activity a node names in <c>reconcileWith</c>.</summary>
    public static ISagatBuilder AddReconciler<TInput>(this ISagatBuilder builder, string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task<Verdict>> handler) =>
        Add(builder, host => host.RegisterReconciler(name, handler));

    /// <summary>Registers an <see cref="IActivity{TInput, TOutput}"/> class as an activity; it is resolved in a new scope for every task.</summary>
    public static ISagatBuilder AddActivity<THandler>(this ISagatBuilder builder, string name)
        where THandler : class =>
        AddClass<THandler>(builder, name, compensation: false);

    /// <summary>Registers an <see cref="IActivity{TInput, TOutput}"/> class as a compensation; it is resolved in a new scope for every task.</summary>
    public static ISagatBuilder AddCompensation<THandler>(this ISagatBuilder builder, string name)
        where THandler : class =>
        AddClass<THandler>(builder, name, compensation: true);

    /// <summary>Registers an <see cref="IReconciler{TInput}"/> class as a reconciler; it is resolved in a new scope for every task.</summary>
    public static ISagatBuilder AddReconciler<THandler>(this ISagatBuilder builder, string name)
        where THandler : class
    {
        var contract = ContractOf(typeof(THandler), typeof(IReconciler<>));
        var method = typeof(SagatServiceCollectionExtensions).GetMethod(nameof(RegisterReconciler), BindingFlags.NonPublic | BindingFlags.Static)!;
        method.MakeGenericMethod(typeof(THandler), contract.GetGenericArguments()[0]).Invoke(null, [builder, name]);
        return builder;
    }

    private static ISagatBuilder AddClass<THandler>(ISagatBuilder builder, string name, bool compensation)
        where THandler : class
    {
        var arguments = ContractOf(typeof(THandler), typeof(IActivity<,>)).GetGenericArguments();
        var method = typeof(SagatServiceCollectionExtensions).GetMethod(nameof(RegisterActivity), BindingFlags.NonPublic | BindingFlags.Static)!;
        method.MakeGenericMethod(typeof(THandler), arguments[0], arguments[1]).Invoke(null, [builder, name, compensation]);
        return builder;
    }

    private static Type ContractOf(Type handler, Type definition)
    {
        var matches = handler.GetInterfaces().Where(i => i.IsGenericType && i.GetGenericTypeDefinition() == definition).ToList();
        return matches.Count == 1
            ? matches[0]
            : throw new InvalidOperationException($"{handler.Name} must implement {definition.Name.Split('`')[0]} exactly once.");
    }

    private static void RegisterActivity<THandler, TInput, TOutput>(ISagatBuilder builder, string name, bool compensation)
        where THandler : class, IActivity<TInput, TOutput>
    {
        builder.Services.TryAddScoped<THandler>();
        Add(builder, (host, services) =>
        {
            async Task<TOutput> Run(TInput input, ActivityExecutionContext context, CancellationToken cancellationToken)
            {
                var scope = services.CreateAsyncScope();
                await using (scope.ConfigureAwait(false))
                {
                    return await scope.ServiceProvider.GetRequiredService<THandler>().ExecuteAsync(input, context, cancellationToken).ConfigureAwait(false);
                }
            }

            if (compensation)
            {
                host.RegisterCompensation<TInput, TOutput>(name, Run);
            }
            else
            {
                host.RegisterActivity<TInput, TOutput>(name, Run);
            }
        });
    }

    private static void RegisterReconciler<THandler, TInput>(ISagatBuilder builder, string name)
        where THandler : class, IReconciler<TInput>
    {
        builder.Services.TryAddScoped<THandler>();
        Add(builder, (host, services) =>
        {
            async Task<Verdict> Run(TInput input, ActivityExecutionContext context, CancellationToken cancellationToken)
            {
                var scope = services.CreateAsyncScope();
                await using (scope.ConfigureAwait(false))
                {
                    return await scope.ServiceProvider.GetRequiredService<THandler>().ReconcileAsync(input, context, cancellationToken).ConfigureAwait(false);
                }
            }

            host.RegisterReconciler<TInput>(name, Run);
        });
    }

    private static ISagatBuilder Add(ISagatBuilder builder, Action<IWorkerHost> register) => Add(builder, (host, _) => register(host));

    private static ISagatBuilder Add(ISagatBuilder builder, Action<IWorkerHost, IServiceProvider> register)
    {
        ArgumentNullException.ThrowIfNull(builder);
        RegistrationsOf(builder.Services).Items.Add(register);
        return builder;
    }

    /// <summary>The first handler registered adds the worker host and the hosted service that runs it; an application with no handlers has no worker.</summary>
    private static SagatWorkerRegistrations RegistrationsOf(IServiceCollection services)
    {
        if (services.FirstOrDefault(d => d.ServiceType == typeof(SagatWorkerRegistrations))?.ImplementationInstance is SagatWorkerRegistrations existing)
        {
            return existing;
        }

        var created = new SagatWorkerRegistrations();
        services.AddSingleton(created);
        services.TryAddSingleton<IWorkerHost>(sp =>
        {
            var options = sp.GetRequiredService<IOptions<SagatOptions>>().Value.Snapshot();
            if (string.IsNullOrWhiteSpace(options.Worker.Name))
            {
                // One session per name: the machine name is the pod of a replica.
                options.Worker.Name = Environment.MachineName;
            }

            var host = new WorkerHost(options, sp.GetRequiredService<ILoggerFactory>());
            foreach (var register in sp.GetRequiredService<SagatWorkerRegistrations>().Items)
            {
                register(host, sp);
            }

            return host;
        });
        services.AddSingleton<IHostedService, SagatWorkerService>();
        return created;
    }

    private sealed class SagatBuilder(IServiceCollection services) : ISagatBuilder
    {
        public IServiceCollection Services { get; } = services;
    }
}
