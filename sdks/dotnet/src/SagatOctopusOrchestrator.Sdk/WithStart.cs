using System.Text.Json;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>The options the with-start calls share.</summary>
public record WithStartOptions
{
    /// <summary>Pins a published version of the workflow; its active version when null.</summary>
    public string? VersionId { get; init; }

    /// <summary>8 to 160 printable ASCII characters. A fresh one per call when null, reused by every retry of that call.</summary>
    public string? IdempotencyKey { get; init; }

    /// <summary>Sent as <c>correlationId</c> in the body.</summary>
    public string? CorrelationId { get; init; }

    /// <summary>The instant at which the run times out. Written in UTC.</summary>
    public DateTimeOffset? Deadline { get; init; }

    /// <summary>The same, as a duration from now, computed once before the first attempt. Wins over <see cref="Deadline"/>.</summary>
    public TimeSpan? DeadlineIn { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>; it wins over the ambient <see cref="System.Diagnostics.Activity"/>.</summary>
    public string? Traceparent { get; init; }

    /// <summary>What to do when a closed execution still remembers the identifier: see <see cref="BusinessIdReusePolicies"/>.</summary>
    public string? BusinessIdReusePolicy { get; init; }
}

/// <summary>The options of an update that starts the run it applies to when none is open.</summary>
public sealed record UpdateWithStartOptions : WithStartOptions
{
    /// <summary>What to do when an open execution holds the identifier: see <see cref="BusinessIdConflictPolicies"/>.</summary>
    public string? BusinessIdConflictPolicy { get; init; }

    /// <summary>Holds the entry of the run this long when the call starts it. Converted once, before the first attempt, to whole milliseconds (a fraction is dropped).</summary>
    public TimeSpan? StartDelay { get; init; }

    /// <summary>Apply the update only at this committed state revision (not negative).</summary>
    public long? ExpectedStateRevision { get; init; }
}

/// <summary>What <c>SignalWithStartAsync</c> answers.</summary>
public sealed record SignalWithStartResult(string RunId, bool Started, IReadOnlyList<string> DeliveredTo, JsonElement Raw);

/// <summary>The durable receipt of an update that was applied (or replayed) by <c>UpdateWithStartAsync</c>.</summary>
public sealed record UpdateWithStartReceipt(
    string RunId,
    string Operation,
    string UpdateKey,
    string Status,
    JsonElement Result,
    long BeforeStateRevision,
    long AfterStateRevision,
    long RunVersion,
    bool Replayed,
    JsonElement Raw);

/// <summary>What became of the update of <c>UpdateWithStartAsync</c>: <c>APPLIED</c> with a receipt, or <c>REJECTED</c> with a reason (an answer, not an error).</summary>
public sealed record UpdateWithStartOutcome(string Outcome, UpdateWithStartReceipt? Receipt, string? Reason);

/// <summary>What <c>UpdateWithStartAsync</c> answers.</summary>
public sealed record UpdateWithStartResult(string RunId, bool Started, UpdateWithStartOutcome Update, JsonElement Raw);
