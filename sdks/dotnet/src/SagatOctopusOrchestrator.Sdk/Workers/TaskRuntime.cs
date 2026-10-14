namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>The mutable state of one task in flight: its cancellation, the lease it holds and whether it was lost.</summary>
internal sealed class TaskRuntime
{
    private string? reason;
    private long deadlineEpochMs;
    private int lost;
    private int finished;

    internal TaskRuntime(long expiresAtEpochMs)
    {
        Abort = new CancellationTokenSource();
        Interlocked.Exchange(ref deadlineEpochMs, expiresAtEpochMs);

        // A lease that says anything (the past, the far future) gives an interval, not an exception.
        var left = Math.Clamp(expiresAtEpochMs - DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(), 0, 3_600_000);
        HeartbeatInterval = WorkerHost.HeartbeatIntervalFor(TimeSpan.FromMilliseconds(left));
    }

    /// <summary>
    /// One third of the time left on the lease when the task was taken, clamped (SPEC section 6.4). It is counted by the monotonic
    /// timer of the platform from then on, so a clock that is set while the task runs cannot stretch it.
    /// </summary>
    internal TimeSpan HeartbeatInterval { get; }

    internal CancellationTokenSource Abort { get; }

    internal string? Reason => Volatile.Read(ref reason);

    internal long DeadlineEpochMs => Interlocked.Read(ref deadlineEpochMs);

    internal bool Lost => Volatile.Read(ref lost) == 1;

    internal bool Finished => Volatile.Read(ref finished) == 1;

    internal void MarkFinished() => Volatile.Write(ref finished, 1);

    internal void MarkLost() => Volatile.Write(ref lost, 1);

    /// <summary>Cancels the activity. The first reason wins; it is set before the token fires so the handler can read it.</summary>
    internal void Cancel(string why)
    {
        Interlocked.CompareExchange(ref reason, why, null);
        try
        {
            Abort.Cancel();
        }
        catch (ObjectDisposedException)
        {
            // The task is over: there is nobody to tell.
        }
    }

    /// <summary>Records the end of the lease as the server said it; it becomes the deadline of the activity.</summary>
    internal void SetLease(long expiresAtEpochMs) => Interlocked.Exchange(ref deadlineEpochMs, expiresAtEpochMs);
}
