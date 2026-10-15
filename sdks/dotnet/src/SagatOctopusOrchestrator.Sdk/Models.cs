using System.Text.Json;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>The statuses of a run, and which of them are final.</summary>
public static class RunStatuses
{
    /// <summary>The run is executing.</summary>
    public const string Running = "RUNNING";

    /// <summary>The run waits for an event, a timer or a signal.</summary>
    public const string Waiting = "WAITING";

    /// <summary>The run was paused by an operator.</summary>
    public const string Paused = "PAUSED";

    /// <summary>A cancellation was requested and is being carried out.</summary>
    public const string Cancelling = "CANCELLING";

    /// <summary>The run is compensating.</summary>
    public const string Compensating = "COMPENSATING";

    /// <summary>The run waits for a person.</summary>
    public const string ManualReview = "MANUAL_REVIEW";

    /// <summary>Final: every step succeeded.</summary>
    public const string Succeeded = "SUCCEEDED";

    /// <summary>Final: the run failed.</summary>
    public const string Failed = "FAILED";

    /// <summary>Final: the run was cancelled.</summary>
    public const string Cancelled = "CANCELLED";

    /// <summary>Final: the run reached its deadline.</summary>
    public const string TimedOut = "TIMED_OUT";

    /// <summary>Final: the saga compensated.</summary>
    public const string Compensated = "COMPENSATED";

    /// <summary>Final: the saga compensated what could be compensated.</summary>
    public const string Partial = "PARTIAL";

    /// <summary>Final: a compensation failed.</summary>
    public const string CompensationFailed = "COMPENSATION_FAILED";

    private static readonly HashSet<string> Terminal =
    [
        Succeeded, Failed, Cancelled, TimedOut, Compensated, Partial, CompensationFailed,
    ];

    /// <summary>True for a status in which the run will not change any more.</summary>
    public static bool IsTerminal(string? status) => status is not null && Terminal.Contains(status);
}

/// <summary>The parent of a child run.</summary>
public sealed record ParentRun(string RunId, string Node, string CancelPolicy);

/// <summary>A run of a workflow. <see cref="Raw"/> keeps the answer as the server sent it, so a field of a newer server is reachable.</summary>
public sealed record Run
{
    /// <summary>The id of the run.</summary>
    public required string RunId { get; init; }

    /// <summary>The workflow the run belongs to.</summary>
    public required string WorkflowId { get; init; }

    /// <summary>The published version of the workflow the run executes.</summary>
    public required string VersionId { get; init; }

    /// <summary>The status; compare with <see cref="RunStatuses"/>. A newer server may send a status this SDK does not list.</summary>
    public required string Status { get; init; }

    /// <summary>The number of journal events applied; usable as <c>expectedRunVersion</c>.</summary>
    public long Version { get; init; }

    /// <summary>Whether the run is paused.</summary>
    public bool Paused { get; init; }

    /// <summary>The identity that started the run.</summary>
    public string? StartedBy { get; init; }

    /// <summary>When the run was created (UTC).</summary>
    public required DateTimeOffset CreatedAt { get; init; }

    /// <summary>When the run last changed (UTC).</summary>
    public DateTimeOffset? UpdatedAt { get; init; }

    /// <summary>When the run ended (UTC).</summary>
    public DateTimeOffset? FinishedAt { get; init; }

    /// <summary>The instant at which the run times out (UTC).</summary>
    public DateTimeOffset? Deadline { get; init; }

    /// <summary>The trace id of the run (32 hex characters).</summary>
    public string? TraceId { get; init; }

    /// <summary>The correlation id given at the start.</summary>
    public string? CorrelationId { get; init; }

    /// <summary>The depth of a child run; 0 for a run that has no parent.</summary>
    public int Depth { get; init; }

    /// <summary>The parent of a child run.</summary>
    public ParentRun? Parent { get; init; }

    /// <summary>The answer as the server sent it.</summary>
    public JsonElement Raw { get; init; }

    /// <summary>True when <see cref="Status"/> is final.</summary>
    public bool IsTerminal => RunStatuses.IsTerminal(Status);
}

/// <summary>A page of runs.</summary>
public sealed record RunPage(IReadOnlyList<Run> Items, string? NextCursor);

/// <summary>The operations an operator can do to a run (<c>POST /runs/{id}/operations</c>).</summary>
public enum RunAction
{
    /// <summary>Pause the run.</summary>
    Pause,

    /// <summary>Resume a paused run.</summary>
    Resume,

    /// <summary>Cancel the run.</summary>
    Cancel,

    /// <summary>Retry a failed step.</summary>
    RetryStep,

    /// <summary>Retry a failed compensation.</summary>
    RetryCompensation,

    /// <summary>Skip a compensation.</summary>
    SkipCompensation,

    /// <summary>Resolve a step by hand.</summary>
    ResolveStep,
}

/// <summary>The outcome an operator gives a step in <see cref="RunAction.ResolveStep"/>.</summary>
public enum StepOutcome
{
    /// <summary>The step is taken as succeeded.</summary>
    Succeeded,

    /// <summary>The step is taken as failed.</summary>
    Failed,
}

/// <summary>What the server answers to an operation.</summary>
public sealed record OperationResult(string OperationId, bool Accepted, string? RunStatus, long? RunVersion, JsonElement Raw);

/// <summary>A decision an eligible actor records for an approval request.</summary>
public enum ApprovalDecision
{
    /// <summary>Approve the request.</summary>
    Approve,

    /// <summary>Reject the request.</summary>
    Reject,
}

/// <summary>Options that identify one safely retryable approval decision.</summary>
public sealed record ApprovalDecisionOptions
{
    /// <summary>One to 128 printable ASCII characters, reused only for the same decision.</summary>
    public required string DecisionKey { get; init; }

    /// <summary>An optional machine-readable reason code.</summary>
    public string? ReasonCode { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>The durable receipt for an approval decision.</summary>
public sealed record ApprovalDecisionReceipt(
    string ReceiptId,
    string ApprovalId,
    string DecisionKey,
    ApprovalDecision Decision,
    string? ReasonCode,
    string Actor,
    DateTimeOffset DecidedAt,
    long OutcomeSequence,
    bool Replayed,
    JsonElement Raw);

/// <summary>The lifecycle state of a native approval request.</summary>
public enum ApprovalStatus
{
    Pending,
    Approved,
    Rejected,
    Expired,
    Cancelled,
}

/// <summary>One bounded collection or loop scope in an approval instance identity.</summary>
public sealed record ApprovalScope(string ScopeNodeId, int IterationIndex);

/// <summary>One approval request visible on a run, without eligibility or actor disclosure.</summary>
public sealed record ApprovalListItem(
    string ApprovalId,
    string RunId,
    string NodeId,
    IReadOnlyList<ApprovalScope> InstancePath,
    ApprovalStatus Status,
    DateTimeOffset Deadline,
    DateTimeOffset? SettledAt,
    JsonElement Raw);

/// <summary>One bounded approval page in immutable opening order.</summary>
public sealed record ApprovalPage(IReadOnlyList<ApprovalListItem> Items, string? NextCursor);

/// <summary>Options for one approval page.</summary>
public sealed record ApprovalListOptions
{
    public string? Cursor { get; init; }
    public int? Limit { get; init; }
    public string? Traceparent { get; init; }
}

/// <summary>The result of a declared query and the committed revisions it observed.</summary>
public sealed record QueryResult(string RunId, long RunVersion, long StateRevision, JsonElement Result, JsonElement Raw);

/// <summary>Options for resolving and querying a continuation chain head.</summary>
public sealed record ChainQueryOptions
{
    /// <summary>Require this run to remain the current head while the query is evaluated.</summary>
    public string? ExpectedRunId { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>Options that identify one durable state update.</summary>
public sealed record UpdateOptions
{
    /// <summary>One to 128 printable ASCII characters, preserved across retries.</summary>
    public required string UpdateKey { get; init; }

    /// <summary>Apply only at this committed state revision.</summary>
    public long? ExpectedStateRevision { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>The durable result of an applied or replayed declared update.</summary>
public sealed record UpdateReceipt(
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

/// <summary>What the server answers to a signal: the steps that were waiting for it.</summary>
public sealed record SignalResult(IReadOnlyList<string> DeliveredTo);

/// <summary>One immutable run link in an execution chain.</summary>
public sealed record ChainSegment(int Index, string RunId, string? PredecessorRunId, string? SuccessorRunId);

/// <summary>One committed snapshot of an execution chain.</summary>
public sealed record ChainInspection(
    string ChainId, string CurrentRunId, long Revision, bool CancelRequested, string? FinalOutcome,
    IReadOnlyList<ChainSegment> Segments, JsonElement Raw);

/// <summary>Options for a command against an observed chain head.</summary>
public sealed record ChainCommandOptions
{
    /// <summary>The current run id observed by the caller.</summary>
    public required string ExpectedRunId { get; init; }

    /// <summary>8 to 160 printable ASCII characters, preserved across retries.</summary>
    public string? IdempotencyKey { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>An event to publish to the inbox of the namespace.</summary>
public sealed record PublishEventRequest
{
    /// <summary>The type of the event; a wait for an event names it.</summary>
    public required string Type { get; init; }

    /// <summary>The data of the event; anything System.Text.Json can write as an object.</summary>
    public object? Data { get; init; }

    /// <summary>Unique within the source for the deduplication window; a fresh one per call when null, kept across the retries of that call.</summary>
    public string? EventId { get; init; }

    /// <summary>The subject of the event.</summary>
    public string? Subject { get; init; }

    /// <summary>When the event happened (written in UTC).</summary>
    public DateTimeOffset? OccurredAt { get; init; }

    /// <summary>The key a wait for an event matches against its correlation expression.</summary>
    public string? CorrelationKey { get; init; }

    /// <summary>How long the event id is remembered, in seconds.</summary>
    public int? DedupeTtlSeconds { get; init; }
}

/// <summary>What the server answers to a published event. <see cref="Status"/> is <c>ACCEPTED</c> or <c>DUPLICATE</c>.</summary>
public sealed record EventAccepted(string EventId, string Source, string Status);

/// <summary>An entry of the journal of a run; <see cref="Raw"/> is the event as the server sent it.</summary>
public sealed record JournalEvent(JsonElement Raw)
{
    /// <summary>The type of the event (<c>step.leased</c>).</summary>
    public string? Type => Raw.ValueKind == JsonValueKind.Object && Raw.TryGetProperty("eventType", out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    /// <summary>The step the event is about, if any.</summary>
    public string? StepId => Raw.ValueKind == JsonValueKind.Object && Raw.TryGetProperty("stepId", out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    /// <summary>The attempt the event is about, if any.</summary>
    public int? Attempt => Raw.ValueKind == JsonValueKind.Object && Raw.TryGetProperty("attempt", out var v) && v.ValueKind == JsonValueKind.Number && v.TryGetInt32(out var n) ? n : null;
}

/// <summary>A page of the journal.</summary>
public sealed record JournalPage(IReadOnlyList<JournalEvent> Items, string? NextCursor);

/// <summary>Options of a call that has no other.</summary>
public sealed record RequestOptions
{
    /// <summary>8 to 160 printable ASCII characters. A fresh one per call when null, reused by every retry of that call.</summary>
    public string? IdempotencyKey { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>; it wins over the ambient <see cref="System.Diagnostics.Activity"/>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>Options of <see cref="ISagatClient.StartAsync{TInput}"/>.</summary>
public sealed record StartOptions
{
    /// <summary>Pins a published version of the workflow; its active version when null.</summary>
    public string? VersionId { get; init; }

    /// <summary>8 to 160 printable ASCII characters. A fresh one per call when null, reused by every retry of that call.</summary>
    public string? IdempotencyKey { get; init; }

    /// <summary>Sent as <c>correlationId</c> in the body.</summary>
    public string? CorrelationId { get; init; }

    /// <summary>The instant at which the run times out; must be in the future and within a year. Written in UTC.</summary>
    public DateTimeOffset? Deadline { get; init; }

    /// <summary>The same, as a duration from now, computed once before the first attempt. Wins over <see cref="Deadline"/>.</summary>
    public TimeSpan? DeadlineIn { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>; it wins over the ambient <see cref="System.Diagnostics.Activity"/>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>Options of <see cref="ISagatClient.ListAsync"/>.</summary>
public sealed record ListOptions
{
    /// <summary>Only the runs of this workflow.</summary>
    public string? WorkflowId { get; init; }

    /// <summary>Only the runs in this status.</summary>
    public string? Status { get; init; }

    /// <summary>The page size.</summary>
    public int? Limit { get; init; }

    /// <summary>Where the previous page ended (<see cref="RunPage.NextCursor"/>).</summary>
    public string? Cursor { get; init; }
}

/// <summary>Options of <see cref="ISagatClient.OperateAsync"/> and <see cref="ISagatClient.CancelAsync"/>.</summary>
public sealed record OperateOptions
{
    /// <summary>8 to 160 printable ASCII characters. A fresh one per call when null, reused by every retry of that call.</summary>
    public string? IdempotencyKey { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }

    /// <summary>For <see cref="RunAction.RetryStep"/>, <see cref="RunAction.RetryCompensation"/>, <see cref="RunAction.SkipCompensation"/> and <see cref="RunAction.ResolveStep"/>: the step.</summary>
    public string? StepId { get; init; }

    /// <summary>For <see cref="RunAction.ResolveStep"/>.</summary>
    public StepOutcome? Outcome { get; init; }

    /// <summary>For <see cref="RunAction.ResolveStep"/> with <see cref="StepOutcome.Failed"/>.</summary>
    public string? ErrorCode { get; init; }

    /// <summary>The operation is applied only if the run is at this version.</summary>
    public long? ExpectedRunVersion { get; init; }
}

/// <summary>Options of <see cref="ISagatClient.WaitForCompletionAsync"/>.</summary>
public sealed record WaitOptions
{
    /// <summary>How long to wait for a terminal status. 60 s by default.</summary>
    public TimeSpan Timeout { get; init; } = TimeSpan.FromSeconds(60);

    /// <summary>The pause between two reads, +/- 20 % jitter. 500 ms by default.</summary>
    public TimeSpan PollInterval { get; init; } = TimeSpan.FromMilliseconds(500);
}
