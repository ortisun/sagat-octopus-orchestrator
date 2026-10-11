using System.Diagnostics;
using System.Diagnostics.Metrics;

namespace SagatOctopusOrchestrator.Sdk.Internal;

/// <summary>The spans and the metrics of the SDK. Nothing here ever carries a token, a lease token, a secret or a payload.</summary>
internal static class Telemetry
{
    internal static readonly ActivitySource Source = new(SdkInfo.ActivitySourceName, SdkInfo.Version);

    private static readonly Meter Meter = new(SdkInfo.MeterName, SdkInfo.Version);

    internal static readonly Counter<long> TasksTaken = Meter.CreateCounter<long>("sagat.worker.tasks.taken", "{task}", "Tasks leased by the worker host.");
    internal static readonly Counter<long> TasksCompleted = Meter.CreateCounter<long>("sagat.worker.tasks.completed", "{task}", "Tasks reported as completed.");
    internal static readonly Counter<long> TasksFailed = Meter.CreateCounter<long>("sagat.worker.tasks.failed", "{task}", "Tasks reported as failed.");
    internal static readonly Counter<long> TasksStale = Meter.CreateCounter<long>("sagat.worker.tasks.stale", "{task}", "Tasks whose lease was lost: the result was dropped.");
    internal static readonly Counter<long> TasksDropped = Meter.CreateCounter<long>("sagat.worker.tasks.dropped", "{task}", "Reports that could not be delivered.");
    internal static readonly Counter<long> Heartbeats = Meter.CreateCounter<long>("sagat.worker.heartbeats", "{heartbeat}", "Heartbeats answered.");
    internal static readonly Counter<long> Polls = Meter.CreateCounter<long>("sagat.worker.polls", "{poll}", "Polls answered.");
    internal static readonly Counter<long> Reconnects = Meter.CreateCounter<long>("sagat.worker.reconnects", "{reconnect}", "Times the host registered again or backed off after a failed call.");
    internal static readonly Histogram<double> ActivityDuration = Meter.CreateHistogram<double>("sagat.worker.activity.duration", "s", "How long an activity attempt took, from taking the lease to reporting it.");

    /// <summary>The W3C trace context of the current activity, when there is one and it is W3C.</summary>
    internal static string? CurrentTraceparent()
    {
        var current = Activity.Current;
        return current is { IdFormat: ActivityIdFormat.W3C } && current.Id is { } id ? id : null;
    }
}
