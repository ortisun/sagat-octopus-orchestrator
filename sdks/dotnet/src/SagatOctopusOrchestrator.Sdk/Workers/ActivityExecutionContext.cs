using System.Text.Json;
using Microsoft.Extensions.Logging;

namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>The kind of task a lease is for (protocol 1.2; an older server only sends <see cref="Forward"/>).</summary>
public enum TaskKind
{
    /// <summary>An ordinary step.</summary>
    Forward,

    /// <summary>The undo of a forward step (<c>compensateWith</c>).</summary>
    Compensation,

    /// <summary>The lookup of whether a forward step happened (<c>reconcileWith</c>).</summary>
    Reconciliation,
}

/// <summary>Why the cancellation token of an activity was cancelled (<see cref="ActivityExecutionContext.CancellationReason"/>).</summary>
public static class CancellationReasons
{
    /// <summary>The run was cancelled; seen at the next heartbeat.</summary>
    public const string RunCancelled = "run-cancelled";

    /// <summary>The lease was lost: the result will be dropped.</summary>
    public const string LeaseLost = "lease-lost";

    /// <summary>The host is stopping and the task did not finish in time.</summary>
    public const string Shutdown = "shutdown";
}

/// <summary>What an activity knows about the attempt it is running (SPEC section 6.3).</summary>
public sealed class ActivityExecutionContext
{
    private readonly TaskRuntime runtime;
    private readonly Func<CancellationToken, Task<ActivitySecrets>> secrets;

    internal ActivityExecutionContext(
        TaskRuntime runtime,
        string activityName,
        string taskId,
        string runId,
        string node,
        string stepId,
        int attempt,
        long generation,
        string idempotencyKey,
        string traceparent,
        TaskKind taskKind,
        string targetNode,
        string targetIdempotencyKey,
        IReadOnlyDictionary<string, JsonElement> steps,
        ILogger logger,
        Func<CancellationToken, Task<ActivitySecrets>> secrets)
    {
        this.runtime = runtime;
        this.secrets = secrets;
        ActivityName = activityName;
        TaskId = taskId;
        RunId = runId;
        Node = node;
        StepId = stepId;
        Attempt = attempt;
        Generation = generation;
        IdempotencyKey = idempotencyKey;
        Traceparent = traceparent;
        TaskKind = taskKind;
        TargetNode = targetNode;
        TargetIdempotencyKey = targetIdempotencyKey;
        Steps = steps;
        Logger = logger;
    }

    /// <summary>The name the handler was registered under.</summary>
    public string ActivityName { get; }

    /// <summary>The id of the task (the lease).</summary>
    public string TaskId { get; }

    /// <summary>The id of the run.</summary>
    public string RunId { get; }

    /// <summary>The node id of the step in the workflow definition (<c>charge</c>), from the step document.</summary>
    public string Node { get; }

    /// <summary>The platform's id of the step (<c>step_id</c> of the lease, a UUID), which the reports carry.</summary>
    public string StepId { get; }

    /// <summary>The attempt, from 1.</summary>
    public int Attempt { get; }

    /// <summary>The generation of the lease; it grows each time the step is leased again.</summary>
    public long Generation { get; }

    /// <summary>Stable across the attempts of the step: send it to the provider of an external effect.</summary>
    public string IdempotencyKey { get; }

    /// <summary>The W3C trace context of the run (may be empty); <see cref="System.Diagnostics.Activity.Current"/> is a child of it inside the handler.</summary>
    public string Traceparent { get; }

    /// <summary>What kind of task this is.</summary>
    public TaskKind TaskKind { get; }

    /// <summary>For a compensation or a reconciliation: the forward step it is about.</summary>
    public string TargetNode { get; }

    /// <summary>For a compensation or a reconciliation: the key that identifies the effect of that step at the provider.</summary>
    public string TargetIdempotencyKey { get; }

    /// <summary>The outputs of the steps that completed before this one, by node.</summary>
    public IReadOnlyDictionary<string, JsonElement> Steps { get; }

    /// <summary>The end of the lease; moves forward with every heartbeat.</summary>
    public DateTimeOffset Deadline => DateTimeOffset.FromUnixTimeMilliseconds(Math.Clamp(runtime.DeadlineEpochMs, -62_135_596_800_000L, 253_402_300_799_999L));

    /// <summary>Cancelled when the run is cancelled, the lease is lost or the host stops; <see cref="CancellationReason"/> says which.</summary>
    public CancellationToken CancellationToken => runtime.Abort.Token;

    /// <summary>One of <see cref="CancellationReasons"/> once <see cref="CancellationToken"/> is cancelled; null before.</summary>
    public string? CancellationReason => runtime.Reason;

    /// <summary>A logger for the handler; the log scope carries the task, run, node and attempt. Never log a payload or a secret.</summary>
    public ILogger Logger { get; }

    /// <summary>
    /// The secrets the step declares; resolved once per attempt, on first use. Against protocol 1 this raises
    /// <see cref="SagatErrorCode.UnsupportedProtocol"/> instead of calling the server.
    /// </summary>
    public Task<ActivitySecrets> GetSecretsAsync(CancellationToken cancellationToken = default) => secrets(cancellationToken);
}
