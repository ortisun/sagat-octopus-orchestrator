using System.Text.Json;
using SagatOctopusOrchestrator.Sdk.Internal;
using static SagatOctopusOrchestrator.Sdk.Internal.Answers;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>One bounded page, and the opaque cursor of the next one (<c>null</c> on the last).</summary>
public sealed record Page<T>(IReadOnlyList<T> Items, string? NextCursor);

/// <summary>What to do with the business identifier of a run that is started again (<c>KEEP</c> or <c>DROP</c>).</summary>
public static class BusinessIdHandling
{
    /// <summary>The new run keeps the identifier.</summary>
    public const string Keep = "KEEP";

    /// <summary>The new run has no identifier.</summary>
    public const string Drop = "DROP";
}

/// <summary>The filter of a batch selector. A field left out selects on nothing.</summary>
public sealed record BatchFilter
{
    /// <summary>Only the runs of this workflow.</summary>
    public string? WorkflowId { get; init; }

    /// <summary>Only the runs of this version.</summary>
    public string? VersionId { get; init; }

    /// <summary>Only the runs in one of these statuses.</summary>
    public IReadOnlyList<string>? Status { get; init; }

    /// <summary>Only the runs with this origin.</summary>
    public string? Origin { get; init; }

    /// <summary>Only the runs a schedule started.</summary>
    public string? ScheduleId { get; init; }

    /// <summary>Only the runs created at or after this instant. Sent in UTC.</summary>
    public Instant? CreatedAfter { get; init; }

    /// <summary>Only the runs created before this instant. Sent in UTC.</summary>
    public Instant? CreatedBefore { get; init; }

    /// <summary>Only the runs whose business identifier starts with this text.</summary>
    public string? BusinessIdPrefix { get; init; }
}

/// <summary>Which runs a batch job is about: a filter, a list of run ids or a query. Build one with <see cref="ByFilter"/>, <see cref="ByRunIds"/> or <see cref="ByQuery"/>.</summary>
public abstract record BatchSelector
{
    private protected BatchSelector()
    {
    }

    /// <summary>The runs a filter selects.</summary>
    public static BatchSelector ByFilter(BatchFilter filter) => new BatchFilterSelector(filter);

    /// <summary>The runs with these ids (sent as given: the server judges them).</summary>
    public static BatchSelector ByRunIds(IEnumerable<string> runIds) => new BatchRunIdsSelector([.. runIds]);

    /// <summary>The runs a query selects.</summary>
    public static BatchSelector ByQuery(string query) => new BatchQuerySelector(query);
}

/// <summary>A selector made of a filter.</summary>
public sealed record BatchFilterSelector(BatchFilter Filter) : BatchSelector;

/// <summary>A selector made of run ids.</summary>
public sealed record BatchRunIdsSelector(IReadOnlyList<string> RunIds) : BatchSelector;

/// <summary>A selector made of a query.</summary>
public sealed record BatchQuerySelector(string Query) : BatchSelector;

/// <summary>The signal a <c>SIGNAL</c> batch job sends to each run.</summary>
public sealed record BatchSignal(string Name, object? Payload);

/// <summary>The update an <c>UPDATE</c> batch job applies to each run.</summary>
public sealed record BatchUpdate(string Name, object? Args = null);

/// <summary>A batch job to create. A field left <c>null</c> is left out of the request.</summary>
public sealed record BatchRequest
{
    /// <summary>Which runs.</summary>
    public required BatchSelector Selector { get; init; }

    /// <summary><c>CANCEL</c>, <c>SIGNAL</c>, <c>RERUN</c>, <c>RETRY_STEP</c>, <c>PAUSE</c>, <c>RESUME</c>, <c>MARK_MANUAL_REVIEW</c>, ... The set only grows.</summary>
    public required string Action { get; init; }

    /// <summary>For <c>SIGNAL</c>.</summary>
    public BatchSignal? Signal { get; init; }

    /// <summary>For an update action.</summary>
    public BatchUpdate? Update { get; init; }

    /// <summary>For <c>RETRY_STEP</c>.</summary>
    public string? StepId { get; init; }

    /// <summary>For <c>RERUN</c>: see <see cref="BusinessIdHandling"/>.</summary>
    public string? BusinessId { get; init; }

    /// <summary>Why (the server asks for a reason of some length).</summary>
    public required string Reason { get; init; }

    /// <summary>At most this many runs.</summary>
    public long? MaxItems { get; init; }

    /// <summary>The rate at which items are applied.</summary>
    public long? ItemsPerSecond { get; init; }
}

/// <summary>What a selector matches, up to the limit; nothing was changed.</summary>
public sealed record BatchPreview(long Matched, bool Capped, IReadOnlyList<string> Sample, JsonElement Raw);

/// <summary>The counters of a batch job: every selected item is in exactly one of the others once the job ends.</summary>
public sealed record BatchCounts(long Selected, long Succeeded, long Skipped, long Failed, long Cancelled, long Deferred);

/// <summary>A durable batch job. <see cref="State"/> and <see cref="Action"/> are strings: the set only grows.</summary>
public sealed record BatchJob
{
    /// <summary>The id.</summary>
    public required string BatchId { get; init; }

    /// <summary>The action.</summary>
    public required string Action { get; init; }

    /// <summary><c>PENDING</c>, <c>RUNNING</c>, <c>COMPLETED</c>, <c>CANCELLING</c>, <c>CANCELLED</c>, <c>FAILED</c>, ...</summary>
    public required string State { get; init; }

    /// <summary>Why the job failed.</summary>
    public string? FailureReason { get; init; }

    /// <summary>Who created it.</summary>
    public required string CreatedBy { get; init; }

    /// <summary>Whether the job carries a payload (a signal or an update).</summary>
    public bool HasPayload { get; init; }

    /// <summary>The limit it was created with.</summary>
    public long? MaxItems { get; init; }

    /// <summary>The rate it was created with.</summary>
    public long? ItemsPerSecond { get; init; }

    /// <summary>The counters.</summary>
    public required BatchCounts Counts { get; init; }

    /// <summary>When it was created (UTC).</summary>
    public required DateTimeOffset CreatedAt { get; init; }

    /// <summary>When it started (UTC).</summary>
    public DateTimeOffset? StartedAt { get; init; }

    /// <summary>When it ended (UTC).</summary>
    public DateTimeOffset? FinishedAt { get; init; }

    /// <summary>When its items are forgotten (UTC).</summary>
    public DateTimeOffset? ExpiresAt { get; init; }

    /// <summary>The answer as the server sent it.</summary>
    public JsonElement Raw { get; init; }
}

/// <summary>One run of a batch job. <see cref="State"/> and <see cref="Reason"/> are strings: the set only grows.</summary>
public sealed record BatchItem
{
    /// <summary>The position of the item in the job.</summary>
    public required long Ordinal { get; init; }

    /// <summary>The run.</summary>
    public required string RunId { get; init; }

    /// <summary><c>PENDING</c>, <c>SUCCEEDED</c>, <c>SKIPPED</c>, <c>FAILED</c>, ...</summary>
    public required string State { get; init; }

    /// <summary>Why the item was skipped or failed (<c>ALREADY_TERMINAL</c>, ...).</summary>
    public string? Reason { get; init; }

    /// <summary>The error code of a failed item.</summary>
    public string? ErrorCode { get; init; }

    /// <summary>How many times the item was tried.</summary>
    public long Attempts { get; init; }

    /// <summary>The run a <c>RERUN</c> created.</summary>
    public string? NewRunId { get; init; }

    /// <summary>The answer as the server sent it.</summary>
    public JsonElement Raw { get; init; }
}

/// <summary>Options of a listing of batch jobs or of their items.</summary>
public sealed record BatchListOptions
{
    /// <summary>The <see cref="Page{T}.NextCursor"/> of the previous page; opaque.</summary>
    public string? Cursor { get; init; }

    /// <summary>The page size.</summary>
    public int? Limit { get; init; }

    /// <summary>Only the jobs (or the items) in this state.</summary>
    public string? State { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>
/// The batch jobs of a namespace (feature 018): one action over many runs, durable, rate limited and cancellable. Every call that changes
/// something carries one <c>Idempotency-Key</c>, given by the caller or generated once per call and reused by every retry of it.
/// </summary>
[System.Diagnostics.DebuggerDisplay("{ToString(),nq}")]
public sealed class BatchClient
{
    [System.Diagnostics.DebuggerBrowsable(System.Diagnostics.DebuggerBrowsableState.Never)]
    private readonly HttpTransport transport;
    [System.Diagnostics.DebuggerBrowsable(System.Diagnostics.DebuggerBrowsableState.Never)]
    private readonly JsonSerializerOptions json;

    internal BatchClient(HttpTransport transport, JsonSerializerOptions json)
    {
        this.transport = transport;
        this.json = json;
    }

    /// <summary>The group; it shows nothing of the client that holds it.</summary>
    public override string ToString() => nameof(BatchClient);

    /// <summary>Counts what a selector matches, up to <paramref name="maxItems"/>; changes nothing.</summary>
    public async Task<BatchPreview> PreviewAsync(string namespaceId, BatchSelector selector, long? maxItems = null, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId, "/preview");
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            WriteSelector(writer, selector);
            if (maxItems is { } limit)
            {
                writer.WriteNumber("maxItems", limit);
            }

            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("preview batch", HttpMethod.Post, path, "/namespaces/{namespaceId}/batch-operations/preview", [], body, null, SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = ObjectOf(answer.Body, "a batch preview");
        return new BatchPreview(Revision(raw, "matched", 0), Flag(raw, "capped"), Strings(raw, "sample"), raw);
    }

    /// <summary>Creates a durable job. A request sent again with the same key and body is answered with the first job.</summary>
    public async Task<BatchJob> CreateAsync(string namespaceId, BatchRequest request, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(request);
        var path = PathOf(namespaceId);
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            WriteSelector(writer, request.Selector);
            writer.WriteString("action", request.Action);
            if (request.Signal is { } signal)
            {
                writer.WriteStartObject("signal");
                writer.WriteString("name", signal.Name);
                writer.WritePropertyName("payload");
                WritePayloadOrEmpty(writer, signal.Payload);
                writer.WriteEndObject();
            }

            if (request.Update is { } update)
            {
                writer.WriteStartObject("update");
                writer.WriteString("name", update.Name);
                writer.WritePropertyName("args");
                Wire.WritePayload(writer, update.Args, json);
                writer.WriteEndObject();
            }

            if (request.StepId is not null)
            {
                writer.WriteString("stepId", request.StepId);
            }

            if (request.BusinessId is not null)
            {
                writer.WriteString("businessId", request.BusinessId);
            }

            writer.WriteString("reason", request.Reason);
            if (request.MaxItems is { } maxItems)
            {
                writer.WriteNumber("maxItems", maxItems);
            }

            if (request.ItemsPerSecond is { } rate)
            {
                writer.WriteNumber("itemsPerSecond", rate);
            }

            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("create batch", HttpMethod.Post, path, "/namespaces/{namespaceId}/batch-operations", [], body, SagatClient.KeyOf(options?.IdempotencyKey), SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToJob(answer.Body);
    }

    /// <summary>One batch job.</summary>
    public async Task<BatchJob> GetAsync(string namespaceId, string batchId, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId, "/" + Requests.Uuid(batchId, "a batch id"));
        var answer = await transport.SendAsync(
            new HttpCall("get batch", HttpMethod.Get, path, "/namespaces/{namespaceId}/batch-operations/{batchId}", [], null, null, SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToJob(answer.Body);
    }

    /// <summary>One page of batch jobs.</summary>
    public async Task<Page<BatchJob>> ListAsync(string namespaceId, BatchListOptions? filter = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId);
        var query = new QueryBuilder().Add("cursor", filter?.Cursor).Add("limit", filter?.Limit).Add("state", filter?.State).Build();
        var answer = await transport.SendAsync(
            new HttpCall("list batches", HttpMethod.Get, path, "/namespaces/{namespaceId}/batch-operations", query, null, null, SagatClient.TraceOf(filter?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return PageOf(answer.Body, "a page of batch jobs", item => ToJob(item));
    }

    /// <summary>Stops admitting items; what was applied stays applied. A job that already ended is <c>CONFLICT</c>.</summary>
    public async Task<BatchJob> CancelAsync(string namespaceId, string batchId, string reason, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId, "/" + Requests.Uuid(batchId, "a batch id") + "/cancel");
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("reason", reason);
            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("cancel batch", HttpMethod.Post, path, "/namespaces/{namespaceId}/batch-operations/{batchId}/cancel", [], body, SagatClient.KeyOf(options?.IdempotencyKey), SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToJob(answer.Body);
    }

    /// <summary>One page of the items of a batch job.</summary>
    public async Task<Page<BatchItem>> ListItemsAsync(string namespaceId, string batchId, BatchListOptions? filter = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId, "/" + Requests.Uuid(batchId, "a batch id") + "/items");
        var query = new QueryBuilder().Add("cursor", filter?.Cursor).Add("limit", filter?.Limit).Add("state", filter?.State).Build();
        var answer = await transport.SendAsync(
            new HttpCall("list batch items", HttpMethod.Get, path, "/namespaces/{namespaceId}/batch-operations/{batchId}/items", query, null, null, SagatClient.TraceOf(filter?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return PageOf(answer.Body, "a page of batch items", item => ToItem(item));
    }

    // ------------------------------------------------------------------------------------------ requests

    private static string PathOf(string namespaceId, string tail = "") =>
        "/namespaces/" + Requests.Uuid(namespaceId, "a namespace id") + "/batch-operations" + tail;

    private void WritePayloadOrEmpty(Utf8JsonWriter writer, object? payload)
    {
        if (payload is null)
        {
            writer.WriteStartObject();
            writer.WriteEndObject();
            return;
        }

        Wire.WritePayload(writer, payload, json);
    }

    /// <summary>The selector as the wire has it; the instants of a filter are written as <c>Z</c> instants.</summary>
    private static void WriteSelector(Utf8JsonWriter writer, BatchSelector selector)
    {
        ArgumentNullException.ThrowIfNull(selector);
        writer.WriteStartObject("selector");
        switch (selector)
        {
            case BatchFilterSelector { Filter: var filter }:
                writer.WriteStartObject("filter");
                WriteString(writer, "workflowId", filter.WorkflowId);
                WriteString(writer, "versionId", filter.VersionId);
                if (filter.Status is { } statuses)
                {
                    writer.WriteStartArray("status");
                    foreach (var status in statuses)
                    {
                        writer.WriteStringValue(status);
                    }

                    writer.WriteEndArray();
                }

                WriteString(writer, "origin", filter.Origin);
                WriteString(writer, "scheduleId", filter.ScheduleId);
                WriteString(writer, "createdAfter", filter.CreatedAfter?.ToString());
                WriteString(writer, "createdBefore", filter.CreatedBefore?.ToString());
                WriteString(writer, "businessIdPrefix", filter.BusinessIdPrefix);
                writer.WriteEndObject();
                break;
            case BatchRunIdsSelector { RunIds: var runIds }:
                writer.WriteStartArray("runIds");
                foreach (var runId in runIds)
                {
                    writer.WriteStringValue(runId);
                }

                writer.WriteEndArray();
                break;
            case BatchQuerySelector { Query: var query }:
                writer.WriteString("query", query);
                break;
            default:
                throw SagatException.Invalid("a selector is a filter, a list of run ids or a query");
        }

        writer.WriteEndObject();
    }

    private static void WriteString(Utf8JsonWriter writer, string name, string? value)
    {
        if (value is not null)
        {
            writer.WriteString(name, value);
        }
    }

    // -------------------------------------------------------------------------------------------- answers

    private static BatchJob ToJob(JsonElement? value)
    {
        var raw = ObjectOf(value, "a batch job");
        var counts = raw.TryGetProperty("counts", out var found) && found.ValueKind == JsonValueKind.Object ? found : default;
        long Count(string name) => counts.ValueKind == JsonValueKind.Object ? Num(counts, name) : 0;
        return new BatchJob
        {
            BatchId = Str(raw, "batchId"),
            Action = Str(raw, "action"),
            State = Str(raw, "state"),
            FailureReason = OptStr(raw, "failureReason"),
            CreatedBy = Str(raw, "createdBy"),
            HasPayload = Flag(raw, "hasPayload"),
            MaxItems = OptNum(raw, "maxItems"),
            ItemsPerSecond = OptNum(raw, "itemsPerSecond"),
            Counts = new BatchCounts(Count("selected"), Count("succeeded"), Count("skipped"), Count("failed"), Count("cancelled"), Count("deferred")),
            CreatedAt = Date(raw, "createdAt"),
            StartedAt = OptDate(raw, "startedAt"),
            FinishedAt = OptDate(raw, "finishedAt"),
            ExpiresAt = OptDate(raw, "expiresAt"),
            Raw = raw,
        };
    }

    private static BatchItem ToItem(JsonElement? value)
    {
        var raw = ObjectOf(value, "a batch item");
        return new BatchItem
        {
            Ordinal = Revision(raw, "ordinal", 0),
            RunId = Str(raw, "runId"),
            State = Str(raw, "state"),
            Reason = OptStr(raw, "reason"),
            ErrorCode = OptStr(raw, "errorCode"),
            Attempts = Num(raw, "attempts"),
            NewRunId = OptStr(raw, "newRunId"),
            Raw = raw,
        };
    }
}
