namespace SagatOctopusOrchestrator.Sdk;

/// <summary>
/// The client of the REST API (SPEC section 3). Every call takes a <see cref="CancellationToken"/>; a call cancelled with it raises
/// <see cref="OperationCanceledException"/>. Every other failure is a <see cref="SagatException"/>.
/// </summary>
public interface ISagatClient
{
    /// <summary>Streams a staged run input exactly once and verifies the returned descriptor.</summary>
    Task<Workers.BlobDescriptor> UploadRunInputAsync(Stream source, RunInputBlobOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>The id of a workflow given by id or by name (the active version of the workflow of that name).</summary>
    Task<string> ResolveWorkflowAsync(string workflow, CancellationToken cancellationToken = default);

    /// <summary>
    /// Starts a run of a published workflow, given by id or by name. The call carries an idempotency key (the caller's, or one
    /// generated for this call and reused by every retry of it), so that it is repeated only when that cannot start a second run.
    /// </summary>
    Task<Run> StartAsync<TInput>(string workflow, TInput input, StartOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Atomically starts a run with a staged input descriptor.</summary>
    Task<Run> StartWithBlobAsync(string workflow, Workers.BlobDescriptor descriptor, StartOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Reads a run.</summary>
    Task<Run> GetAsync(string runId, CancellationToken cancellationToken = default);

    /// <summary>One page of runs, newest first.</summary>
    Task<RunPage> ListAsync(ListOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Every run that matches, page after page, in the order the server gave.</summary>
    IAsyncEnumerable<Run> ListAllAsync(ListOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Pauses, resumes, cancels or decides about a run.</summary>
    Task<OperationResult> OperateAsync(string runId, RunAction action, string reason, OperateOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Cancels a run (the server's reason has 8 to 2000 characters).</summary>
    Task<OperationResult> CancelAsync(string runId, string reason, OperateOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Records an eligible actor's durable decision for an approval request.</summary>
    Task<ApprovalDecisionReceipt> DecideApprovalAsync(string approvalId, ApprovalDecision decision, ApprovalDecisionOptions options, CancellationToken cancellationToken = default);

    /// <summary>Reads one bounded page of approval requests in immutable opening order.</summary>
    Task<ApprovalPage> ListApprovalsAsync(string runId, ApprovalListOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Streams every approval request for a run, following opaque cursors.</summary>
    IAsyncEnumerable<ApprovalListItem> ListAllApprovalsAsync(string runId, ApprovalListOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Evaluates a declared side-effect-free query over one committed state snapshot.</summary>
    Task<QueryResult> QueryAsync<TArgs>(string runId, string name, TArgs args, RequestOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Evaluates a declared query against the chain head selected under the server's chain lock.</summary>
    Task<QueryResult> QueryChainAsync<TArgs>(string namespaceId, string chainId, string name, TArgs args, ChainQueryOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Applies or replays a declared durable state update.</summary>
    Task<UpdateReceipt> UpdateAsync<TArgs>(string runId, string name, TArgs args, UpdateOptions options, CancellationToken cancellationToken = default);

    /// <summary>Sends a signal to the steps of a run that wait for it; a signal nobody waits for is a <see cref="SagatErrorCode.Conflict"/>.</summary>
    Task<SignalResult> SignalAsync<TPayload>(string runId, string name, TPayload payload, RequestOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Reads one committed chain head and lineage snapshot.</summary>
    Task<ChainInspection> InspectChainAsync(string namespaceId, string chainId, RequestOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>Signals a root wait on the expected current chain head.</summary>
    Task<SignalResult> SignalChainAsync<TPayload>(string namespaceId, string chainId, string name, TPayload payload, ChainCommandOptions options, CancellationToken cancellationToken = default);

    /// <summary>Latches cancellation and cancels the expected current chain head.</summary>
    Task CancelChainAsync(string namespaceId, string chainId, string reason, ChainCommandOptions options, CancellationToken cancellationToken = default);

    /// <summary>Publishes an event to the inbox of the namespace, once per event id for the deduplication window.</summary>
    Task<EventAccepted> PublishEventAsync(PublishEventRequest request, RequestOptions? options = null, CancellationToken cancellationToken = default);

    /// <summary>The journal of a run, a page at a time.</summary>
    Task<JournalPage> EventsAsync(string runId, string? cursor = null, int? limit = null, CancellationToken cancellationToken = default);

    /// <summary>Polls <see cref="GetAsync"/> until the run is in a terminal status or the timeout of <paramref name="options"/> passes.</summary>
    Task<Run> WaitForCompletionAsync(string runId, WaitOptions? options = null, CancellationToken cancellationToken = default);
}
