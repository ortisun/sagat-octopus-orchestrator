using System.Text.Json.Nodes;

namespace SagatOctopusOrchestrator.Sdk.Workflows;

/// <summary>How the wait between the attempts of a step grows.</summary>
public enum RetryStrategy
{
    /// <summary>The same wait every time.</summary>
    Fixed,

    /// <summary>The wait doubles up to the maximum.</summary>
    Exponential,
}

/// <summary>The retry budget of a step or of its compensation (the node's <c>retry</c>). The server applies it, not the SDK.</summary>
public sealed record StepRetry
{
    /// <summary>Attempts in all, 1 to 100.</summary>
    public int? MaxAttempts { get; init; }

    /// <summary>The wait after the first failed attempt.</summary>
    public TimeSpan? InitialDelay { get; init; }

    /// <summary>The ceiling of the wait.</summary>
    public TimeSpan? MaxDelay { get; init; }

    /// <summary>How the wait grows.</summary>
    public RetryStrategy? Strategy { get; init; }

    /// <summary>Whether the wait is randomized.</summary>
    public bool? Jitter { get; init; }
}

/// <summary>Options of <see cref="FlowBase{TSelf}.Step"/>. Absent options are left out of the document, never written as null.</summary>
public sealed record StepOptions
{
    /// <summary>The retry budget of the step.</summary>
    public StepRetry? Retry { get; init; }

    /// <summary>Longest one attempt may take.</summary>
    public TimeSpan? Timeout { get; init; }

    /// <summary>The activity that undoes this step; it must be idempotent.</summary>
    public string? CompensateWith { get; init; }

    /// <summary>The activity that finds out whether this step happened (a reconciler).</summary>
    public string? ReconcileWith { get; init; }

    /// <summary>The template of the idempotency key of the effect.</summary>
    public string? IdempotencyTemplate { get; init; }

    /// <summary>Names of the secrets of the namespace the worker may ask for while it holds a lease on this step.</summary>
    public IReadOnlyList<string>? Secrets { get; init; }

    /// <summary>The retry budget of the compensation.</summary>
    public StepRetry? CompensationRetry { get; init; }

    /// <summary>Longest one attempt of the compensation may take.</summary>
    public TimeSpan? CompensationTimeout { get; init; }

    /// <summary>The effect cannot be undone once it has happened. Cannot be combined with <see cref="CompensateWith"/>.</summary>
    public bool? Irreversible { get; init; }

    /// <summary>An attempt whose outcome is unknown is never retried or compensated blindly.</summary>
    public bool? HighRisk { get; init; }

    /// <summary>The node to go to when the step fails for good.</summary>
    public string? OnError { get; init; }
}

/// <summary>Options of <see cref="FlowBase{TSelf}.WaitForEvent"/>.</summary>
public sealed record WaitForEventOptions
{
    /// <summary>The type of event that wakes the wait.</summary>
    public required string EventType { get; init; }

    /// <summary>An expression over the run's data that yields the correlation key.</summary>
    public required string CorrelationExpr { get; init; }

    /// <summary>How long to wait, at least 1 ms.</summary>
    public required TimeSpan Timeout { get; init; }

    /// <summary>The node to go to when the wait times out.</summary>
    public string? OnTimeout { get; init; }

    /// <summary>An event that arrived up to this long before the wait started also wakes it (once).</summary>
    public TimeSpan? Lookback { get; init; }
}

/// <summary>What happens to a child run when its parent is stopped.</summary>
public enum ChildCancelPolicy
{
    /// <summary>The child is cancelled and the parent does not wait for it.</summary>
    Propagate,

    /// <summary>The child keeps running.</summary>
    Detach,

    /// <summary>The child is cancelled and the parent waits until it has settled.</summary>
    RequestCancel,
}

/// <summary>Options of <see cref="FlowBase{TSelf}.InvokeChild"/>.</summary>
public sealed record ChildOptions
{
    /// <summary>Pins a published version number; the workflow's active version when absent.</summary>
    public int? Version { get; init; }

    /// <summary>An expression over the parent's data that evaluates to a JSON object; the parent's own input when absent.</summary>
    public string? Input { get; init; }

    /// <summary>What happens to the child when the parent is stopped.</summary>
    public ChildCancelPolicy? CancelPolicy { get; init; }
}

/// <summary>How a parallel node joins its branches.</summary>
public enum JoinMode
{
    /// <summary>Wait for every branch.</summary>
    All,

    /// <summary>Continue with the first branch that completes.</summary>
    Any,
}

/// <summary>What a saga does when a compensation fails after its retries.</summary>
public enum CompensationFailurePolicy
{
    /// <summary>The saga waits for an operator (the default).</summary>
    ManualReview,

    /// <summary>The saga ends <c>COMPENSATION_FAILED</c>.</summary>
    Fail,
}

/// <summary>What a cancellation or a deadline does to a saga.</summary>
public enum SagaCancelPolicy
{
    /// <summary>Stop what is in flight and compensate what completed (the default).</summary>
    CompensateCompleted,

    /// <summary>Stop and compensate nothing.</summary>
    AbortNoCompensation,

    /// <summary>Let the attempts in flight finish, then compensate everything that completed.</summary>
    WaitInFlightThenCompensate,
}

/// <summary>Options of <see cref="FlowBase{TSelf}.Saga"/>.</summary>
public sealed record SagaOptions
{
    /// <summary>What happens when a compensation fails.</summary>
    public CompensationFailurePolicy? OnCompensationFailure { get; init; }

    /// <summary>What a cancellation or a deadline does to the saga.</summary>
    public SagaCancelPolicy? CancelPolicy { get; init; }
}

/// <summary>Required bounds of a runtime-sized collection.</summary>
public sealed record CollectionOptions
{
    /// <summary>Largest captured collection, from 1 through 10,000.</summary>
    public required int MaxItems { get; init; }

    /// <summary>Largest number of admitted iterations, from 1 through 256.</summary>
    public required int MaxConcurrency { get; init; }
}

/// <summary>Required bounds and optional routing of a sequential workflow loop.</summary>
public sealed record LoopOptions
{
    /// <summary>Largest number of body executions, from 1 through 10,000.</summary>
    public required int MaxIterations { get; init; }

    /// <summary>Total time available to the loop, from 1 through 31,622,400,000 milliseconds.</summary>
    public required TimeSpan Timeout { get; init; }

    /// <summary>Optional durable delay between iterations that are still needed.</summary>
    public TimeSpan? Delay { get; init; }

    /// <summary>The node to follow when the loop fails.</summary>
    public string? OnError { get; init; }
}

/// <summary>A built-in namespace role that may decide an approval.</summary>
public enum ApprovalRole
{
    /// <summary>Namespace administrator.</summary>
    Admin,

    /// <summary>Workflow developer.</summary>
    Developer,

    /// <summary>Workflow operator.</summary>
    Operator,
}

/// <summary>Subjects and built-in roles eligible to decide an approval.</summary>
public sealed record ApprovalPolicy
{
    /// <summary>Eligible subject identifiers.</summary>
    public IReadOnlyList<string>? Subjects { get; init; }

    /// <summary>Eligible built-in namespace roles.</summary>
    public IReadOnlyList<ApprovalRole>? Roles { get; init; }

    /// <summary>Whether the subject that started the run is excluded. Omission uses the server default of true.</summary>
    public bool? ExcludeInitiator { get; init; }
}

/// <summary>Policy, deadline and routes of a native approval.</summary>
public sealed record ApprovalOptions
{
    /// <summary>The eligibility policy pinned when the request opens.</summary>
    public required ApprovalPolicy Policy { get; init; }

    /// <summary>Time available for a decision.</summary>
    public required TimeSpan Timeout { get; init; }

    /// <summary>The node selected by an approval.</summary>
    public required string OnApprove { get; init; }

    /// <summary>The node selected by a rejection.</summary>
    public required string OnReject { get; init; }

    /// <summary>The node selected when the deadline wins.</summary>
    public required string OnExpire { get; init; }
}

/// <summary>Finite lifetime shared by every run in a continuation chain.</summary>
public sealed record ChainPolicyOptions
{
    /// <summary>Maximum number of segments, including the initial run.</summary>
    public required int MaxSegments { get; init; }

    /// <summary>Original deadline retained by every segment.</summary>
    public required TimeSpan Timeout { get; init; }
}

/// <summary>The explicit authorization boundary for declared operation results.</summary>
public static class OperationDisclosure
{
    /// <summary>An authorized caller may receive the declared, bounded business result.</summary>
    public const string DeclaredBusinessResultV1 = "declared-business-result-v1";
}

/// <summary>A bounded snapshot projection over declared workflow state.</summary>
public sealed record QueryDefinition
{
    public required JsonObject? ArgsSchema { get; init; }

    public required JsonObject? ResultSchema { get; init; }

    public required string? Expression { get; init; }

    public required string? Disclosure { get; init; }
}

/// <summary>An atomic declarative validator, state reducer and result projection.</summary>
public sealed record UpdateDefinition
{
    public required JsonObject? ArgsSchema { get; init; }

    public string? Validate { get; init; }

    public required string? State { get; init; }

    public required string? Result { get; init; }

    public required JsonObject? ResultSchema { get; init; }

    public required string? Disclosure { get; init; }
}

/// <summary>Bounded business state and its named side-effect-free operations.</summary>
public sealed record StateDefinition
{
    public required JsonObject? Schema { get; init; }

    public required string? Initial { get; init; }

    public IReadOnlyDictionary<string, QueryDefinition>? Queries { get; init; }

    public IReadOnlyDictionary<string, UpdateDefinition>? Updates { get; init; }
}

/// <summary>The kind of a workflow trigger.</summary>
public enum TriggerKind
{
    /// <summary>Somebody starts it.</summary>
    Manual,

    /// <summary>An event of a type, from any source.</summary>
    Event,

    /// <summary>An event received by a webhook endpoint of the namespace.</summary>
    Webhook,

    /// <summary>An event consumed from a topic or queue by a connector of the namespace.</summary>
    Message,

    /// <summary>A schedule.</summary>
    Cron,
}

/// <summary>A trigger of a workflow (<c>workflow.schema.json</c>, <c>triggers</c>).</summary>
public sealed record Trigger(TriggerKind Kind)
{
    /// <summary>Required for an event trigger; narrows the events of a webhook or message trigger.</summary>
    public string? EventType { get; init; }

    /// <summary>For an event trigger: only events published by this source.</summary>
    public string? Source { get; init; }

    /// <summary>Required for a webhook trigger: the slug of the endpoint that receives the events.</summary>
    public string? Endpoint { get; init; }

    /// <summary>Required for a message trigger: the slug of the connector that consumes the messages.</summary>
    public string? Connector { get; init; }

    /// <summary>A side-effect-free condition over the event; it starts a run only when it holds.</summary>
    public string? Filter { get; init; }

    /// <summary>Required for a cron trigger: five fields, minute hour day-of-month month day-of-week.</summary>
    public string? Cron { get; init; }

    /// <summary>IANA name of the time zone of a cron trigger (UTC when absent).</summary>
    public string? Timezone { get; init; }

    /// <summary>Somebody starts the workflow.</summary>
    public static Trigger Manual() => new(TriggerKind.Manual);

    /// <summary>An event of <paramref name="eventType"/>, optionally only from <paramref name="source"/>.</summary>
    public static Trigger OnEvent(string eventType, string? source = null) => new(TriggerKind.Event) { EventType = eventType, Source = source };

    /// <summary>A schedule.</summary>
    public static Trigger OnSchedule(string cron, string? timezone = null) => new(TriggerKind.Cron) { Cron = cron, Timezone = timezone };
}
