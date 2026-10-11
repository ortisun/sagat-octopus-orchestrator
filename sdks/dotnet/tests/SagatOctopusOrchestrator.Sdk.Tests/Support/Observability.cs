using System.Collections.Concurrent;
using System.Diagnostics;
using System.Diagnostics.Metrics;
using Microsoft.Extensions.Logging;

namespace SagatOctopusOrchestrator.Sdk.Tests.Support;

/// <summary>Captures what the SDK's meter records for one worker (the tag <c>sagat.worker</c> is the worker's name, unique per test).</summary>
internal sealed class MeterCapture : IDisposable
{
    private readonly MeterListener listener = new();
    private readonly ConcurrentDictionary<string, long> totals = new();
    private readonly string worker;

    internal MeterCapture(string worker)
    {
        this.worker = worker;
        listener.InstrumentPublished = (instrument, subscribed) =>
        {
            if (instrument.Meter.Name == SdkInfo.MeterName)
            {
                subscribed.EnableMeasurementEvents(instrument);
            }
        };
        listener.SetMeasurementEventCallback<long>(OnMeasurement);
        listener.SetMeasurementEventCallback<double>(OnMeasurement);
        listener.Start();
    }

    internal long Total(string instrument) => totals.GetValueOrDefault(instrument);

    public void Dispose() => listener.Dispose();

    private void OnMeasurement<T>(Instrument instrument, T measurement, ReadOnlySpan<KeyValuePair<string, object?>> tags, object? state)
        where T : struct
    {
        foreach (var tag in tags)
        {
            if (tag.Key == "sagat.worker" && (string?)tag.Value == worker)
            {
                totals.AddOrUpdate(instrument.Name, 1, (_, count) => count + 1);
                return;
            }
        }
    }
}

/// <summary>Captures every line a logger of the SDK writes, with its scopes and exception, to prove none of them holds a token or a payload.</summary>
internal sealed class CapturingLoggerProvider : ILoggerProvider, ISupportExternalScope
{
    private readonly ConcurrentQueue<string> lines = new();
    private IExternalScopeProvider scopes = new LoggerExternalScopeProvider();

    internal IReadOnlyCollection<string> Lines => [.. lines];

    public ILogger CreateLogger(string categoryName) => new Capture(this, categoryName);

    public void SetScopeProvider(IExternalScopeProvider scopeProvider) => scopes = scopeProvider;

    public void Dispose()
    {
    }

    private sealed class Capture(CapturingLoggerProvider owner, string category) : ILogger
    {
        public IDisposable? BeginScope<TState>(TState state)
            where TState : notnull => owner.scopes.Push(state);

        public bool IsEnabled(LogLevel logLevel) => true;

        public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
        {
            var text = $"{category} {logLevel} {formatter(state, exception)} {exception}";
            owner.scopes.ForEachScope((scope, builder) => builder.Append(' ').Append(scope), new System.Text.StringBuilder());
            var scoped = new System.Text.StringBuilder(text);
            owner.scopes.ForEachScope((scope, builder) => builder.Append(' ').Append(scope), scoped);
            if (state is IEnumerable<KeyValuePair<string, object?>> values)
            {
                foreach (var (key, value) in values)
                {
                    scoped.Append(' ').Append(key).Append('=').Append(value);
                }
            }

            owner.lines.Enqueue(scoped.ToString());
        }
    }
}

/// <summary>
/// Collects the spans of the given sources that pass the filter. A listener is process-wide and the tests run in
/// parallel, so each test says which spans are its own (its trace, its namespace or its worker name).
/// </summary>
internal sealed class SpanCapture : IDisposable
{
    private readonly ActivityListener listener;
    private readonly ConcurrentQueue<Activity> stopped = new();

    internal SpanCapture(Func<Activity, bool> accept, params string[] sources)
    {
        listener = new ActivityListener
        {
            ShouldListenTo = source => sources.Contains(source.Name),
            Sample = (ref ActivityCreationOptions<ActivityContext> _) => ActivitySamplingResult.AllDataAndRecorded,
            ActivityStopped = activity =>
            {
                if (accept(activity))
                {
                    stopped.Enqueue(activity);
                }
            },
        };
        ActivitySource.AddActivityListener(listener);
    }

    internal IReadOnlyCollection<Activity> Spans => [.. stopped];

    public void Dispose() => listener.Dispose();
}
