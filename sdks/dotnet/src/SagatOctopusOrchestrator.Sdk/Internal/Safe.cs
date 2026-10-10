using System.Diagnostics;
using System.Diagnostics.Metrics;
using Microsoft.Extensions.Logging;

namespace SagatOctopusOrchestrator.Sdk.Internal;

/// <summary>
/// The hooks of the host application (a logger, a metrics listener, a trace listener) run inside the SDK's loops: whatever they raise is
/// theirs, and it never kills the heartbeat, the poll loop or a task in flight (SPEC section 10).
/// </summary>
internal static class Safe
{
    internal static void Run(Action action)
    {
#pragma warning disable CA1031 // A hook of the application may raise anything; the SDK goes on.
        try
        {
            action();
        }
        catch (Exception error) when (error is not OutOfMemoryException)
        {
            // Not ours to handle, and nowhere safe to report it.
        }
#pragma warning restore CA1031
    }

    internal static void Add(Counter<long> counter, in TagList tags)
    {
        var copy = tags;
        Run(() => counter.Add(1, copy));
    }

    internal static void Record(Histogram<double> histogram, double value, in TagList tags)
    {
        var copy = tags;
        Run(() => histogram.Record(value, copy));
    }

    /// <summary>Starts a span; null when nobody listens or when the listener raised.</summary>
    internal static Activity? Start(string name, ActivityKind kind, ActivityContext? parent = null)
    {
        Activity? span = null;
        Run(() => span = parent is { } context ? Telemetry.Source.StartActivity(name, kind, context) : Telemetry.Source.StartActivity(name, kind));
        return span;
    }

    internal static void Tag(Activity? span, string key, object? value)
    {
        if (span is not null)
        {
            Run(() => span.SetTag(key, value));
        }
    }

    internal static void Fail(Activity? span, string? description = null)
    {
        if (span is not null)
        {
            Run(() => span.SetStatus(ActivityStatusCode.Error, description));
        }
    }

    internal static void Stop(Activity? span)
    {
        if (span is not null)
        {
            Run(span.Dispose);
        }
    }
}

/// <summary>A logger that cannot raise: the logger of the application is not allowed to take the SDK down.</summary>
internal sealed class SafeLogger(ILogger inner) : ILogger
{
    public IDisposable? BeginScope<TState>(TState state)
        where TState : notnull
    {
#pragma warning disable CA1031 // see Safe
        try
        {
            return inner.BeginScope(state);
        }
        catch (Exception error) when (error is not OutOfMemoryException)
        {
            return null;
        }
#pragma warning restore CA1031
    }

    public bool IsEnabled(LogLevel logLevel)
    {
#pragma warning disable CA1031 // see Safe
        try
        {
            return inner.IsEnabled(logLevel);
        }
        catch (Exception error) when (error is not OutOfMemoryException)
        {
            return false;
        }
#pragma warning restore CA1031
    }

    public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter) =>
        Safe.Run(() => inner.Log(logLevel, eventId, state, exception, formatter));

    internal static ILogger Of(ILogger logger) => logger as SafeLogger ?? new SafeLogger(logger);
}
