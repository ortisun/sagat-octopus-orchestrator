using System.Text.Json;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>What a reset runs again of the signals and updates the source run received after the point (<c>reapply</c>). The set only grows.</summary>
public static class ResetReapplyPolicies
{
    /// <summary>Nothing is applied again.</summary>
    public const string None = "NONE";

    /// <summary>The signals are applied again.</summary>
    public const string Signals = "SIGNALS";

    /// <summary>The updates are applied again.</summary>
    public const string Updates = "UPDATES";

    /// <summary>Both are applied again.</summary>
    public const string All = "ALL";
}

/// <summary>What a reset does with a source run that has not ended (<c>sourcePolicy</c>).</summary>
public static class ResetSourcePolicies
{
    /// <summary>The source must have ended (the default).</summary>
    public const string RequireTerminal = "REQUIRE_TERMINAL";

    /// <summary>The source is cancelled first.</summary>
    public const string CancelSource = "CANCEL_SOURCE";
}

/// <summary>A point of the journal of a run that ended that a reset can go back to. <see cref="Kind"/> is a string: the set only grows.</summary>
public sealed record ResetPoint
{
    /// <summary>The sequence of the point in the journal; what <c>PreviewResetAsync</c> and <c>ResetAsync</c> take.</summary>
    public required long Sequence { get; init; }

    /// <summary><c>RUN_STARTED</c>, <c>STEP_COMPLETED</c>, ...</summary>
    public required string Kind { get; init; }

    /// <summary>The node the point is about, if any.</summary>
    public string? Node { get; init; }

    /// <summary>When the point happened (UTC).</summary>
    public required DateTimeOffset OccurredAt { get; init; }

    /// <summary>The answer as the server sent it.</summary>
    public JsonElement Raw { get; init; }
}

/// <summary>A step the new run keeps as it was.</summary>
public sealed record ResetRestoredStep(string Node, string Kind);

/// <summary>A step the new run runs again; <see cref="Markers"/> tell why that deserves care (<c>irreversible</c>, ...).</summary>
public sealed record ResetReexecutedStep(string Node, bool CompletedInSource, IReadOnlyList<string> Markers);

/// <summary>What a reset to a sequence would restore and run again; nothing was changed.</summary>
public sealed record ResetPlan
{
    /// <summary>The sequence.</summary>
    public required long Sequence { get; init; }

    /// <summary>What the new run keeps.</summary>
    public IReadOnlyList<ResetRestoredStep> Restored { get; init; } = [];

    /// <summary>What the new run runs again.</summary>
    public IReadOnlyList<ResetReexecutedStep> Reexecuted { get; init; } = [];

    /// <summary>What would be applied again, as the server describes it.</summary>
    public IReadOnlyList<JsonElement> Reapply { get; init; } = [];

    /// <summary>Whether the reset must say it accepts that completed steps run again.</summary>
    public bool RequiresAcknowledgement { get; init; }

    /// <summary>How many journal events the new run starts with.</summary>
    public required long PrefixEvents { get; init; }

    /// <summary>The answer as the server sent it.</summary>
    public JsonElement Raw { get; init; }
}

/// <summary>A reset to ask for. A field left <c>null</c> is left out of the request.</summary>
public sealed record ResetRequest
{
    /// <summary>The point to go back to (at least 1).</summary>
    public required long Sequence { get; init; }

    /// <summary>Why (the server asks for a reason of some length).</summary>
    public required string Reason { get; init; }

    /// <summary>See <see cref="ResetReapplyPolicies"/>.</summary>
    public string? Reapply { get; init; }

    /// <summary>See <see cref="ResetSourcePolicies"/>.</summary>
    public string? SourcePolicy { get; init; }

    /// <summary>What the new run does with the business identifier of the source: see <see cref="BusinessIdHandling"/>.</summary>
    public string? BusinessId { get; init; }

    /// <summary>Whether the caller accepts that completed steps run again (needed when <see cref="ResetPlan.RequiresAcknowledgement"/>).</summary>
    public bool? AcknowledgeReexecution { get; init; }
}

/// <summary>A reset, as the register of the source run keeps it. <see cref="State"/> is a string: the set only grows.</summary>
public sealed record Reset
{
    /// <summary>The id of the reset.</summary>
    public required string ResetId { get; init; }

    /// <summary>The run that was reset.</summary>
    public required string SourceRunId { get; init; }

    /// <summary>The point it went back to.</summary>
    public required long Sequence { get; init; }

    /// <summary><c>CREATED</c>, <c>PENDING_SOURCE</c>, ...</summary>
    public required string State { get; init; }

    /// <summary>Why the reset failed.</summary>
    public string? FailureReason { get; init; }

    /// <summary>The run the reset created.</summary>
    public string? NewRunId { get; init; }

    /// <summary>What happened to each signal and update that was to be applied again, as the server describes it.</summary>
    public IReadOnlyList<JsonElement> ReapplyReport { get; init; } = [];

    /// <summary>When the reset was recorded (UTC).</summary>
    public DateTimeOffset? CreatedAt { get; init; }

    /// <summary>The answer as the server sent it.</summary>
    public JsonElement Raw { get; init; }
}

/// <summary>Options of a listing of the reset points of a run.</summary>
public sealed record ResetPointListOptions
{
    /// <summary>The <see cref="Page{T}.NextCursor"/> of the previous page; opaque.</summary>
    public string? Cursor { get; init; }

    /// <summary>The page size.</summary>
    public int? Limit { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }
}
