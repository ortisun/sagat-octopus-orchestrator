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

/// <summary>One repeated scope that addresses a runtime activation of a definition node.</summary>
public sealed record ExecutionScope(string ScopeNodeId, uint IterationIndex);

/// <summary>The run-local identity supplied for a task by worker protocol 1.5.</summary>
public sealed record ExecutionInstanceIdentity(IReadOnlyList<ExecutionScope> InstancePath, string DefinitionNodeId);

/// <summary>What an activity knows about the attempt it is running (SPEC section 6.3).</summary>
public sealed class ActivityExecutionContext
{
    private readonly TaskRuntime runtime;
    private readonly Func<CancellationToken, Task<ActivitySecrets>> secrets;
    private readonly Func<Stream, BlobWriteOptions, CancellationToken, Task<BlobDescriptor>> writeBlob;
    private readonly Func<BlobDescriptor, CancellationToken, IAsyncEnumerable<ReadOnlyMemory<byte>>> readBlob;

    internal ActivityExecutionContext(
        TaskRuntime runtime,
        string activityName,
        string taskId,
        string runId,
        string node,
        string stepId,
        int attempt,
        long generation,
        ExecutionInstanceIdentity? instance,
        JsonElement? item,
        uint? index,
        string idempotencyKey,
        string traceparent,
        TaskKind taskKind,
        string targetNode,
        string targetIdempotencyKey,
        IReadOnlyDictionary<string, JsonElement> steps,
        ILogger logger,
        Func<CancellationToken, Task<ActivitySecrets>> secrets,
        Func<Stream, BlobWriteOptions, CancellationToken, Task<BlobDescriptor>> writeBlob,
        Func<BlobDescriptor, CancellationToken, IAsyncEnumerable<ReadOnlyMemory<byte>>> readBlob)
    {
        this.runtime = runtime;
        this.secrets = secrets;
        this.writeBlob = writeBlob;
        this.readBlob = readBlob;
        ActivityName = activityName;
        TaskId = taskId;
        RunId = runId;
        Node = node;
        StepId = stepId;
        Attempt = attempt;
        Generation = generation;
        Instance = instance;
        Item = item;
        Index = index;
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

    /// <summary>Present only when protocol 1.5 was negotiated and the lease supplied an identity.</summary>
    public ExecutionInstanceIdentity? Instance { get; }

    /// <summary>The immutable captured collection item for this scoped task; null for root and non-collection work.</summary>
    public JsonElement? Item { get; }

    /// <summary>The captured collection index; null for root and non-collection work.</summary>
    public uint? Index { get; }

    /// <summary>Stable across the attempts of the step: send it to the provider of an external effect.</summary>
    public string IdempotencyKey { get; }

    /// <summary>
    /// The W3C trace context of the run (may be empty). When it is valid, <see cref="System.Diagnostics.Activity.Current"/> inside the handler is a
    /// child of it, whether or not anything listens to the SDK's <c>ActivitySource</c> (without a listener the SDK starts a plain, unsampled-by-source
    /// activity just so the requests of the handler carry the trace); when it is empty or invalid the SDK sets no activity.
    /// </summary>
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

    /// <summary>Streams a one-pass source and verifies the returned descriptor against the bytes sent.</summary>
    public Task<BlobDescriptor> WriteBlobAsync(Stream source, BlobWriteOptions? options = null, CancellationToken cancellationToken = default) =>
        writeBlob(source ?? throw new ArgumentNullException(nameof(source)), options ?? new(), cancellationToken);

    /// <summary>Streams bytes; successful exhaustion means size and SHA-256 matched the descriptor.</summary>
    public IAsyncEnumerable<ReadOnlyMemory<byte>> ReadBlobAsync(BlobDescriptor descriptor, CancellationToken cancellationToken = default) =>
        readBlob(descriptor ?? throw new ArgumentNullException(nameof(descriptor)), cancellationToken);
}
