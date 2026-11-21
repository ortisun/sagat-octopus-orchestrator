using System.Reflection;
using SagatOctopusOrchestrator.Sdk.Workers;

namespace SagatOctopusOrchestrator.Sdk.Tests.Support;

/// <summary>
/// A clock for a worker host that records every wait the host asks it for (the heartbeat interval, the pauses of the loop) and can pin the
/// time of day a lease is taken at. The timers still run on the system clock: the host behaves as it does in production, and a test reads
/// the wait the host chose instead of measuring it with the wall clock, which a loaded machine stretches (AGENTS.md rule 18).
/// </summary>
internal sealed class RecordingClock(DateTimeOffset? now = null) : TimeProvider
{
    private readonly object gate = new();
    private readonly List<Wait> waits = [];

    /// <summary>One wait the host asked for: how long, and what the fake service had seen when it was asked for.</summary>
    internal sealed record Wait(TimeSpan Due, int Registrations, int Polls);

    /// <summary>What the fake service has seen, read when a wait begins; the test sets it.</summary>
    internal Func<(int Registrations, int Polls)> Seen { get; set; } = () => (0, 0);

    internal List<Wait> Waits
    {
        get
        {
            lock (gate)
            {
                return [.. waits];
            }
        }
    }

    public override DateTimeOffset GetUtcNow() => now ?? TimeProvider.System.GetUtcNow();

    public override ITimer CreateTimer(TimerCallback callback, object? state, TimeSpan dueTime, TimeSpan period)
    {
        var (registrations, polls) = Seen();
        lock (gate)
        {
            waits.Add(new Wait(dueTime, registrations, polls));
        }

        return TimeProvider.System.CreateTimer(callback, state, dueTime, period);
    }

    /// <summary>Gives <paramref name="host"/> this clock; called before the host starts.</summary>
    internal RecordingClock On(WorkerHost host)
    {
        var field = typeof(WorkerHost).GetField("time", BindingFlags.Instance | BindingFlags.NonPublic)
            ?? throw new InvalidOperationException("the worker host no longer has a field named time for its clock");
        field.SetValue(host, this);
        return this;
    }
}
