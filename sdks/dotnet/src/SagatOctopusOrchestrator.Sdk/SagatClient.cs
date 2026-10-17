using System.Collections.Concurrent;
using System.Diagnostics;
using System.Globalization;
using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.RegularExpressions;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using SagatOctopusOrchestrator.Sdk.Internal;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>The client of the REST API (SPEC section 3). Thread-safe; create one and keep it (the dependency injection integration does).</summary>
[System.Diagnostics.DebuggerDisplay("{ToString(),nq}")]
public sealed partial class SagatClient : ISagatClient, IDisposable
{
    private static readonly TimeSpan NameCacheLifetime = TimeSpan.FromSeconds(60);

    [System.Diagnostics.DebuggerBrowsable(System.Diagnostics.DebuggerBrowsableState.Never)]
    private readonly SagatOptions options;
    private readonly HttpTransport transport;
    private readonly JsonSerializerOptions json;
    private readonly ConcurrentDictionary<string, (string Id, long At)> names = new(StringComparer.Ordinal);

    /// <summary>Creates a client. <see cref="SagatOptions.ApiUrl"/>, <see cref="SagatOptions.Namespace"/> and a token are required.</summary>
    public SagatClient(SagatOptions options, ILogger<SagatClient>? logger = null)
    {
        ArgumentNullException.ThrowIfNull(options);
        this.options = options.Snapshot();
        this.options.ValidateCommon();
        if (string.IsNullOrWhiteSpace(this.options.ApiUrl) || !Uri.TryCreate(this.options.ApiUrl, UriKind.Absolute, out var url) || url.Scheme is not ("http" or "https"))
        {
            throw SagatException.Invalid("ApiUrl is required and is an http or https URL");
        }

        Internal.Tls.RequireTlsForServerName(this.options.Tls, url);

        json = Wire.OptionsOf(this.options);
        transport = new HttpTransport(this.options, SafeLogger.Of(logger ?? (ILogger)NullLogger.Instance));
    }

    /// <summary>The settings of the client, without any credential.</summary>
    public override string ToString() => $"SagatClient {{ ApiUrl = {SagatOptions.HostOf(options.ApiUrl)}, Namespace = {options.Namespace} }}";

    /// <inheritdoc/>
    public void Dispose() => transport.Dispose();

    [GeneratedRegex("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\\z", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
    private static partial Regex UuidPattern();

    [GeneratedRegex("^[\\x21-\\x7e]{8,160}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex KeyPattern();

    [GeneratedRegex("^[\\x21-\\x7e]{1,128}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex ApprovalDecisionKeyPattern();

    [GeneratedRegex("^[A-Za-z0-9_.:-]{1,64}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex ApprovalReasonCodePattern();

    [GeneratedRegex("^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex DeclaredOperationNamePattern();

    [GeneratedRegex("^[\\x21-\\x7e]{1,128}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex UpdateKeyPattern();

    /// <inheritdoc/>
    public async Task<string> ResolveWorkflowAsync(string workflow, CancellationToken cancellationToken = default)
    {
        if (string.IsNullOrWhiteSpace(workflow))
        {
            throw SagatException.Invalid("a workflow is given by its id or by its name");
        }

        if (UuidPattern().IsMatch(workflow))
        {
            return workflow.ToLowerInvariant();
        }

        if (names.TryGetValue(workflow, out var cached) && Stopwatch.GetElapsedTime(cached.At) < NameCacheLifetime)
        {
            return cached.Id;
        }

        string? cursor = null;
        do
        {
            var query = new List<KeyValuePair<string, string>> { new("limit", "200") };
            if (cursor is not null)
            {
                query.Add(new("cursor", cursor));
            }

            var answer = await transport.SendAsync(
                new HttpCall("resolve-workflow", HttpMethod.Get, "/workflows", "/workflows", query, null, null, null, SafeToRepeat: true),
                cancellationToken).ConfigureAwait(false);
            var page = Object(answer.Body, "a page");
            if (page.TryGetProperty("items", out var items) && items.ValueKind == JsonValueKind.Array)
            {
                foreach (var item in items.EnumerateArray())
                {
                    var draft = Object(item, "a workflow");
                    if (draft.TryGetProperty("name", out var name) && name.ValueKind == JsonValueKind.String && name.GetString() == workflow)
                    {
                        var id = Str(draft, "id");
                        names[workflow] = (id, Stopwatch.GetTimestamp());
                        return id;
                    }
                }
            }

            cursor = OptStr(page, "nextCursor");
        }
        while (cursor is not null);

        throw new SagatException(
            SagatErrorCode.NotFound,
            $"no workflow is named {Wire.Quote(workflow)} in this namespace",
            httpStatus: 404,
            reason: "WORKFLOW_NOT_FOUND");
    }

    /// <inheritdoc/>
    public async Task<Run> StartAsync<TInput>(string workflow, TInput input, StartOptions? options = null, CancellationToken cancellationToken = default)
    {
        options ??= new StartOptions();
        var traceparent = TraceOf(options.Traceparent);
        var key = KeyOf(options.IdempotencyKey);
        var workflowId = await ResolveWorkflowAsync(workflow, cancellationToken).ConfigureAwait(false);
        DateTimeOffset? deadline = options.Deadline;
        if (options.DeadlineIn is { } span)
        {
            deadline = DateTimeOffset.UtcNow + span;
        }

        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("workflowId", workflowId);
            if (options.VersionId is { Length: > 0 } versionId)
            {
                writer.WriteString("versionId", versionId);
            }

            writer.WritePropertyName("input");
            WriteObjectOrEmpty(writer, input);
            if (options.CorrelationId is not null)
            {
                writer.WriteString("correlationId", options.CorrelationId);
            }

            if (deadline is { } at)
            {
                writer.WriteString("deadline", Wire.Instant(at));
            }

            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("start", HttpMethod.Post, "/runs", "/runs", [], body, key, traceparent, SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToRun(answer.Body);
    }

    /// <inheritdoc/>
    public async Task<Run> GetAsync(string runId, CancellationToken cancellationToken = default)
    {
        var answer = await transport.SendAsync(
            new HttpCall("get", HttpMethod.Get, RunPath(runId), "/runs/{runId}", [], null, null, null, SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToRun(answer.Body);
    }

    /// <inheritdoc/>
    public async Task<RunPage> ListAsync(ListOptions? options = null, CancellationToken cancellationToken = default)
    {
        options ??= new ListOptions();
        var query = new List<KeyValuePair<string, string>>();
        if (options.WorkflowId is not null)
        {
            query.Add(new("workflowId", options.WorkflowId));
        }

        if (options.Status is not null)
        {
            query.Add(new("status", options.Status));
        }

        if (options.Limit is { } limit)
        {
            query.Add(new("limit", limit.ToString(CultureInfo.InvariantCulture)));
        }

        if (options.Cursor is not null)
        {
            query.Add(new("cursor", options.Cursor));
        }

        var answer = await transport.SendAsync(
            new HttpCall("list", HttpMethod.Get, "/runs", "/runs", query, null, null, null, SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var page = Object(answer.Body, "a page");
        if (!page.TryGetProperty("items", out var items) || items.ValueKind != JsonValueKind.Array)
        {
            throw new SagatException(SagatErrorCode.Protocol, "items is missing from the page");
        }

        return new RunPage(items.EnumerateArray().Select(item => ToRun(item)).ToList(), OptStr(page, "nextCursor"));
    }

    /// <inheritdoc/>
    public async IAsyncEnumerable<Run> ListAllAsync(ListOptions? options = null, [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        var current = (options ?? new ListOptions()) with { Cursor = null };
        do
        {
            var page = await ListAsync(current, cancellationToken).ConfigureAwait(false);
            foreach (var run in page.Items)
            {
                yield return run;
            }

            current = current with { Cursor = page.NextCursor };
        }
        while (current.Cursor is not null);
    }

    /// <inheritdoc/>
    public async Task<OperationResult> OperateAsync(string runId, RunAction action, string reason, OperateOptions? options = null, CancellationToken cancellationToken = default)
    {
        options ??= new OperateOptions();
        var traceparent = TraceOf(options.Traceparent);
        var key = KeyOf(options.IdempotencyKey);
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("action", ActionName(action));
            writer.WriteString("reason", reason);
            if (options.StepId is not null)
            {
                writer.WriteString("stepId", options.StepId);
            }

            if (options.Outcome is { } outcome)
            {
                writer.WriteString("outcome", outcome == StepOutcome.Succeeded ? "SUCCEEDED" : "FAILED");
            }

            if (options.ErrorCode is not null)
            {
                writer.WriteString("errorCode", options.ErrorCode);
            }

            if (options.ExpectedRunVersion is { } version)
            {
                writer.WriteNumber("expectedRunVersion", version);
            }

            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("operate", HttpMethod.Post, RunPath(runId) + "/operations", "/runs/{runId}/operations", [], body, key, traceparent, SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "an operation result");
        return new OperationResult(
            Str(raw, "operationId"),
            raw.TryGetProperty("accepted", out var accepted) && accepted.ValueKind == JsonValueKind.True,
            OptStr(raw, "runStatus"),
            raw.TryGetProperty("runVersion", out var runVersion) && runVersion.ValueKind == JsonValueKind.Number && runVersion.TryGetInt64(out var v) ? v : null,
            raw)
        {
            NewRunId = OptStr(raw, "newRunId"),
        };
    }

    /// <inheritdoc/>
    public Task<OperationResult> RerunAsNewAsync(string runId, string reason, OperateOptions? options = null, CancellationToken cancellationToken = default) =>
        OperateAsync(runId, RunAction.RerunAsNew, reason, options, cancellationToken);

    /// <inheritdoc/>
    public Task<OperationResult> CancelAsync(string runId, string reason, OperateOptions? options = null, CancellationToken cancellationToken = default) =>
        OperateAsync(runId, RunAction.Cancel, reason, options, cancellationToken);

    /// <inheritdoc/>
    public async Task<ApprovalPage> ListApprovalsAsync(
        string runId,
        ApprovalListOptions? options = null,
        CancellationToken cancellationToken = default)
    {
        options ??= new ApprovalListOptions();
        var query = new List<KeyValuePair<string, string>>();
        if (options.Cursor is not null)
        {
            query.Add(new("cursor", options.Cursor));
        }

        if (options.Limit is { } limit)
        {
            query.Add(new("limit", limit.ToString(CultureInfo.InvariantCulture)));
        }

        var answer = await transport.SendAsync(
            new HttpCall("list approvals", HttpMethod.Get, RunPath(runId) + "/approvals", "/runs/{runId}/approvals", query, null, null, TraceOf(options.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "an approval page");
        if (!raw.TryGetProperty("items", out var values) || values.ValueKind != JsonValueKind.Array || values.GetArrayLength() > 200)
        {
            throw new SagatException(SagatErrorCode.Protocol, "items is invalid in the answer");
        }

        var items = new List<ApprovalListItem>(values.GetArrayLength());
        foreach (var value in values.EnumerateArray())
        {
            var item = Object(value, "an approval");
            var status = Str(item, "status") switch
            {
                "PENDING" => ApprovalStatus.Pending,
                "APPROVED" => ApprovalStatus.Approved,
                "REJECTED" => ApprovalStatus.Rejected,
                "EXPIRED" => ApprovalStatus.Expired,
                "CANCELLED" => ApprovalStatus.Cancelled,
                _ => throw new SagatException(SagatErrorCode.Protocol, "status is invalid in the answer"),
            };
            var deadline = OptDate(item, "deadline") ?? throw new SagatException(SagatErrorCode.Protocol, "deadline is missing from the answer");
            if (!item.TryGetProperty("instancePath", out var path) || path.ValueKind != JsonValueKind.Array || path.GetArrayLength() > 8)
            {
                throw new SagatException(SagatErrorCode.Protocol, "instancePath is invalid in the answer");
            }

            var scopes = new List<ApprovalScope>(path.GetArrayLength());
            foreach (var valueScope in path.EnumerateArray())
            {
                var scope = Object(valueScope, "an approval scope");
                if (!scope.TryGetProperty("iterationIndex", out var indexValue) || !indexValue.TryGetInt32(out var index) || index is < 0 or > 9_999)
                {
                    throw new SagatException(SagatErrorCode.Protocol, "iterationIndex is invalid in the answer");
                }

                scopes.Add(new ApprovalScope(Str(scope, "scopeNodeId"), index));
            }

            items.Add(new ApprovalListItem(
                Str(item, "approvalId"), Str(item, "runId"), Str(item, "nodeId"), scopes,
                status, deadline, OptDate(item, "settledAt"), item));
        }

        return new ApprovalPage(items, OptStr(raw, "nextCursor"));
    }

    /// <inheritdoc/>
    public async IAsyncEnumerable<ApprovalListItem> ListAllApprovalsAsync(
        string runId,
        ApprovalListOptions? options = null,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        options ??= new ApprovalListOptions();
        string? cursor = null;
        do
        {
            var page = await ListApprovalsAsync(runId, options with { Cursor = cursor }, cancellationToken).ConfigureAwait(false);
            foreach (var item in page.Items)
            {
                yield return item;
            }

            cursor = page.NextCursor;
        }
        while (cursor is not null);
    }

    /// <inheritdoc/>
    public async Task<ApprovalDecisionReceipt> DecideApprovalAsync(
        string approvalId,
        ApprovalDecision decision,
        ApprovalDecisionOptions options,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(options);
        if (approvalId is null || !UuidPattern().IsMatch(approvalId))
        {
            throw SagatException.Invalid("an approval id is a UUID");
        }

        if (decision is not (ApprovalDecision.Approve or ApprovalDecision.Reject))
        {
            throw SagatException.Invalid("an approval decision is APPROVE or REJECT");
        }

        if (string.IsNullOrEmpty(options.DecisionKey) || !ApprovalDecisionKeyPattern().IsMatch(options.DecisionKey))
        {
            throw SagatException.Invalid("an approval decision key is 1 to 128 printable ASCII characters");
        }

        if (options.ReasonCode is not null && !ApprovalReasonCodePattern().IsMatch(options.ReasonCode))
        {
            throw SagatException.Invalid("an approval reason code is 1 to 64 letters, digits, dots, underscores, colons or hyphens");
        }

        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("decisionKey", options.DecisionKey);
            writer.WriteString("decision", decision == ApprovalDecision.Approve ? "APPROVE" : "REJECT");
            if (options.ReasonCode is not null)
            {
                writer.WriteString("reasonCode", options.ReasonCode);
            }

            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("decide approval", HttpMethod.Post, "/approvals/" + approvalId.ToLowerInvariant() + "/decisions", "/approvals/{approvalId}/decisions", [], body, null, TraceOf(options.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "an approval decision receipt");
        var receivedDecision = Str(raw, "decision") switch
        {
            "APPROVE" => ApprovalDecision.Approve,
            "REJECT" => ApprovalDecision.Reject,
            _ => throw new SagatException(SagatErrorCode.Protocol, "decision is invalid in the answer"),
        };
        var decidedAt = OptDate(raw, "decidedAt") ?? throw new SagatException(SagatErrorCode.Protocol, "decidedAt is missing from the answer");
        if (!raw.TryGetProperty("outcomeSequence", out var sequence) || !sequence.TryGetInt64(out var outcomeSequence) || outcomeSequence < 1)
        {
            throw new SagatException(SagatErrorCode.Protocol, "outcomeSequence is invalid in the answer");
        }

        return new ApprovalDecisionReceipt(
            Str(raw, "receiptId"), Str(raw, "approvalId"), Str(raw, "decisionKey"), receivedDecision,
            OptStr(raw, "reasonCode"), Str(raw, "actor"), decidedAt, outcomeSequence,
            raw.TryGetProperty("replayed", out var replayed) && replayed.ValueKind == JsonValueKind.True, raw);
    }

    /// <inheritdoc/>
    public async Task<QueryResult> QueryAsync<TArgs>(
        string runId,
        string name,
        TArgs args,
        RequestOptions? options = null,
        CancellationToken cancellationToken = default)
    {
        var path = DeclaredOperationPath(runId, "queries", name);
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WritePropertyName("args");
            Wire.WritePayload(writer, args, json);
            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("query", HttpMethod.Post, path, "/runs/{runId}/queries/{name}", [], body, null, TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "a query result");
        if (!raw.TryGetProperty("result", out var result))
        {
            throw new SagatException(SagatErrorCode.Protocol, "result is missing from the answer");
        }

        return new QueryResult(Str(raw, "runId"), Revision(raw, "runVersion", 0), Revision(raw, "stateRevision", 0), result, raw);
    }

    /// <inheritdoc/>
    public async Task<QueryResult> QueryChainAsync<TArgs>(
        string namespaceId,
        string chainId,
        string name,
        TArgs args,
        ChainQueryOptions? options = null,
        CancellationToken cancellationToken = default)
    {
        var path = name is not null && DeclaredOperationNamePattern().IsMatch(name)
            ? ChainPath(namespaceId, chainId) + "/queries/" + Uri.EscapeDataString(name)
            : throw SagatException.Invalid("an operation name is 1 to 128 permitted characters and starts with a letter or digit");
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WritePropertyName("args");
            Wire.WritePayload(writer, args, json);
            if (options?.ExpectedRunId is { } expectedRunId)
            {
                writer.WriteString("expectedRunId", ExpectedRunId(expectedRunId));
            }
            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("query chain", HttpMethod.Post, path, "/namespaces/{namespaceId}/chains/{chainId}/queries/{name}", [], body, null, TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "a query result");
        if (!raw.TryGetProperty("result", out var result))
        {
            throw new SagatException(SagatErrorCode.Protocol, "result is missing from the answer");
        }
        return new QueryResult(Str(raw, "runId"), Revision(raw, "runVersion", 0), Revision(raw, "stateRevision", 0), result, raw);
    }

    /// <inheritdoc/>
    public async Task<UpdateReceipt> UpdateAsync<TArgs>(
        string runId,
        string name,
        TArgs args,
        UpdateOptions options,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(options);
        var path = DeclaredOperationPath(runId, "updates", name);
        if (string.IsNullOrEmpty(options.UpdateKey) || !UpdateKeyPattern().IsMatch(options.UpdateKey))
        {
            throw SagatException.Invalid("an update key is 1 to 128 printable ASCII characters");
        }

        if (options.ExpectedStateRevision is < 0)
        {
            throw SagatException.Invalid("expected state revision is non-negative");
        }

        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("updateKey", options.UpdateKey);
            writer.WritePropertyName("args");
            Wire.WritePayload(writer, args, json);
            if (options.ExpectedStateRevision is { } revision)
            {
                writer.WriteNumber("expectedStateRevision", revision);
            }

            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("update", HttpMethod.Post, path, "/runs/{runId}/updates/{name}", [], body, null, TraceOf(options.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "an update receipt");
        if (Str(raw, "status") != "APPLIED")
        {
            throw new SagatException(SagatErrorCode.Protocol, "status is invalid in the answer");
        }

        if (!raw.TryGetProperty("result", out var result))
        {
            throw new SagatException(SagatErrorCode.Protocol, "result is missing from the answer");
        }

        return new UpdateReceipt(
            Str(raw, "runId"), Str(raw, "operation"), Str(raw, "updateKey"), "APPLIED", result,
            Revision(raw, "beforeStateRevision", 0), Revision(raw, "afterStateRevision", 1),
            Revision(raw, "runVersion", 1),
            raw.TryGetProperty("replayed", out var replayed) && replayed.ValueKind == JsonValueKind.True, raw);
    }

    /// <inheritdoc/>
    public async Task<UpdateReceipt> UpdateChainAsync<TArgs>(
        string namespaceId,
        string chainId,
        string name,
        TArgs args,
        ChainUpdateOptions options,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(options);
        var path = name is not null && DeclaredOperationNamePattern().IsMatch(name)
            ? ChainPath(namespaceId, chainId) + "/updates/" + Uri.EscapeDataString(name)
            : throw SagatException.Invalid("an operation name is 1 to 128 permitted characters and starts with a letter or digit");
        if (string.IsNullOrEmpty(options.UpdateKey) || !UpdateKeyPattern().IsMatch(options.UpdateKey))
        {
            throw SagatException.Invalid("an update key is 1 to 128 printable ASCII characters");
        }
        if (options.ExpectedStateRevision is < 0)
        {
            throw SagatException.Invalid("expected state revision is non-negative");
        }
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("updateKey", options.UpdateKey);
            writer.WritePropertyName("args");
            Wire.WritePayload(writer, args, json);
            if (options.ExpectedRunId is { } expectedRunId)
            {
                writer.WriteString("expectedRunId", ExpectedRunId(expectedRunId));
            }
            if (options.ExpectedStateRevision is { } revision)
            {
                writer.WriteNumber("expectedStateRevision", revision);
            }
            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("update chain", HttpMethod.Post, path, "/namespaces/{namespaceId}/chains/{chainId}/updates/{name}", [], body, null, TraceOf(options.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "an update receipt");
        if (Str(raw, "status") != "APPLIED")
        {
            throw new SagatException(SagatErrorCode.Protocol, "status is invalid in the answer");
        }
        if (!raw.TryGetProperty("result", out var result))
        {
            throw new SagatException(SagatErrorCode.Protocol, "result is missing from the answer");
        }
        return new UpdateReceipt(
            Str(raw, "runId"), Str(raw, "operation"), Str(raw, "updateKey"), "APPLIED", result,
            Revision(raw, "beforeStateRevision", 0), Revision(raw, "afterStateRevision", 1),
            Revision(raw, "runVersion", 1),
            raw.TryGetProperty("replayed", out var replayed) && replayed.ValueKind == JsonValueKind.True, raw);
    }

    /// <inheritdoc/>
    public async Task<SignalResult> SignalAsync<TPayload>(string runId, string name, TPayload payload, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        options ??= new RequestOptions();
        var traceparent = TraceOf(options.Traceparent);
        var key = KeyOf(options.IdempotencyKey);
        var body = Wire.Write(writer => WriteObjectOrEmpty(writer, payload));
        var answer = await transport.SendAsync(
            new HttpCall("signal", HttpMethod.Post, RunPath(runId) + "/signals/" + Segment(name), "/runs/{runId}/signals/{name}", [], body, key, traceparent, SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "a signal result");
        var delivered = raw.TryGetProperty("deliveredTo", out var list) && list.ValueKind == JsonValueKind.Array
            ? list.EnumerateArray().Select(item => item.ValueKind == JsonValueKind.String ? item.GetString()! : item.GetRawText()).ToList()
            : [];
        return new SignalResult(delivered);
    }

    /// <inheritdoc/>
    public async Task<ChainInspection> InspectChainAsync(string namespaceId, string chainId, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var answer = await transport.SendAsync(
            new HttpCall("inspect chain", HttpMethod.Get, ChainPath(namespaceId, chainId), "/namespaces/{namespaceId}/chains/{chainId}", [], null, null, TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "a chain inspection");
        if (!raw.TryGetProperty("segments", out var values) || values.ValueKind != JsonValueKind.Array || values.GetArrayLength() == 0)
        {
            throw new SagatException(SagatErrorCode.Protocol, "segments is invalid in the answer");
        }
        var segments = new List<ChainSegment>();
        foreach (var value in values.EnumerateArray())
        {
            var segment = Object(value, "a chain segment");
            var index = Revision(segment, "index", 0);
            if (index > ushort.MaxValue)
            {
                throw new SagatException(SagatErrorCode.Protocol, "index is invalid in the answer");
            }
            segments.Add(new ChainSegment((int)index, Str(segment, "runId"), OptStr(segment, "predecessorRunId"), OptStr(segment, "successorRunId")));
        }
        return new ChainInspection(Str(raw, "chainId"), Str(raw, "currentRunId"), Revision(raw, "revision", 0),
            raw.TryGetProperty("cancelRequested", out var cancelled) && cancelled.ValueKind == JsonValueKind.True,
            OptStr(raw, "finalOutcome"), segments, raw);
    }

    /// <inheritdoc/>
    public async Task<ChainRunPage> ListChainRunsAsync(string namespaceId, string chainId, ChainRunListOptions? options = null, CancellationToken cancellationToken = default)
    {
        options ??= new ChainRunListOptions();
        var query = new List<KeyValuePair<string, string>>();
        if (options.Cursor is not null)
        {
            query.Add(new("cursor", options.Cursor));
        }

        if (options.Limit is { } limit)
        {
            query.Add(new("limit", limit.ToString(CultureInfo.InvariantCulture)));
        }

        var answer = await transport.SendAsync(
            new HttpCall("list chain runs", HttpMethod.Get, ChainPath(namespaceId, chainId) + "/runs", "/namespaces/{namespaceId}/chains/{chainId}/runs", query, null, null, TraceOf(options.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "a chain run page");
        if (!raw.TryGetProperty("items", out var values) || values.ValueKind != JsonValueKind.Array || values.GetArrayLength() > 200)
        {
            throw new SagatException(SagatErrorCode.Protocol, "items is invalid in the answer");
        }

        var items = new List<ChainRun>(values.GetArrayLength());
        foreach (var value in values.EnumerateArray())
        {
            var item = Object(value, "a chain run");
            var index = Revision(item, "index", 0);
            if (index > 9_999)
            {
                throw new SagatException(SagatErrorCode.Protocol, "index is invalid in the answer");
            }

            var created = OptDate(item, "createdAt") ?? throw new SagatException(SagatErrorCode.Protocol, "createdAt is missing from the answer");
            items.Add(new ChainRun(
                (int)index, Str(item, "runId"), Str(item, "status"), CompletionKind(item),
                OptStr(item, "predecessorRunId"), OptStr(item, "successorRunId"), created, OptDate(item, "finishedAt"), item));
        }

        return new ChainRunPage(items, OptStr(raw, "nextCursor"));
    }

    /// <inheritdoc/>
    public async IAsyncEnumerable<ChainRun> ListAllChainRunsAsync(
        string namespaceId,
        string chainId,
        ChainRunListOptions? options = null,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        options ??= new ChainRunListOptions();
        string? cursor = null;
        do
        {
            var page = await ListChainRunsAsync(namespaceId, chainId, options with { Cursor = cursor }, cancellationToken).ConfigureAwait(false);
            foreach (var item in page.Items)
            {
                yield return item;
            }

            cursor = page.NextCursor;
        }
        while (cursor is not null);
    }

    /// <inheritdoc/>
    public async Task<SignalResult> SignalChainAsync<TPayload>(string namespaceId, string chainId, string name, TPayload payload, ChainCommandOptions options, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(options);
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject(); writer.WriteString("expectedRunId", ExpectedRunId(options.ExpectedRunId));
            writer.WritePropertyName("payload"); Wire.WritePayload(writer, payload, json); writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(new HttpCall(
            "signal chain", HttpMethod.Post, ChainPath(namespaceId, chainId) + "/signals/" + Segment(name),
            "/namespaces/{namespaceId}/chains/{chainId}/signals/{name}", [], body, KeyOf(options.IdempotencyKey), TraceOf(options.Traceparent), SafeToRepeat: true), cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "a chain signal result");
        var delivered = raw.TryGetProperty("deliveredTo", out var list) && list.ValueKind == JsonValueKind.Array
            ? list.EnumerateArray().Select(item => item.GetString() ?? item.GetRawText()).ToList() : [];
        return new SignalResult(delivered);
    }

    /// <inheritdoc/>
    public async Task CancelChainAsync(string namespaceId, string chainId, string reason, ChainCommandOptions options, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(options);
        if (string.IsNullOrEmpty(reason) || reason.EnumerateRunes().Count() > 512)
        {
            throw SagatException.Invalid("a chain cancellation reason is 1 to 512 characters");
        }
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject(); writer.WriteString("expectedRunId", ExpectedRunId(options.ExpectedRunId));
            writer.WriteString("reason", reason); writer.WriteEndObject();
        });
        await transport.SendAsync(new HttpCall(
            "cancel chain", HttpMethod.Post, ChainPath(namespaceId, chainId) + "/cancel",
            "/namespaces/{namespaceId}/chains/{chainId}/cancel", [], body, KeyOf(options.IdempotencyKey), TraceOf(options.Traceparent), SafeToRepeat: true), cancellationToken).ConfigureAwait(false);
    }

    /// <inheritdoc/>
    public async Task<EventAccepted> PublishEventAsync(PublishEventRequest request, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(request);
        options ??= new RequestOptions();
        var traceparent = TraceOf(options.Traceparent);
        var eventId = request.EventId ?? Guid.NewGuid().ToString();
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("eventId", eventId);
            writer.WriteString("type", request.Type);
            writer.WritePropertyName("data");
            WriteObjectOrEmpty(writer, request.Data);
            if (request.Subject is not null)
            {
                writer.WriteString("subject", request.Subject);
            }

            if (request.OccurredAt is { } at)
            {
                writer.WriteString("occurredAt", Wire.Instant(at));
            }

            if (request.CorrelationKey is not null)
            {
                writer.WriteString("correlationKey", request.CorrelationKey);
            }

            if (request.DedupeTtlSeconds is { } ttl)
            {
                writer.WriteNumber("dedupeTtlSeconds", ttl);
            }

            writer.WriteEndObject();
        });

        // The event id is the key: the server answers a repetition DUPLICATE, so the call is safe to repeat.
        var answer = await transport.SendAsync(
            new HttpCall("publish-event", HttpMethod.Post, "/events", "/events", [], body, null, traceparent, SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "an event answer");
        return new EventAccepted(Str(raw, "eventId"), Str(raw, "source"), Str(raw, "status"));
    }

    /// <inheritdoc/>
    public async Task<JournalPage> EventsAsync(string runId, string? cursor = null, int? limit = null, CancellationToken cancellationToken = default)
    {
        var query = new List<KeyValuePair<string, string>>();
        if (cursor is not null)
        {
            query.Add(new("cursor", cursor));
        }

        if (limit is { } size)
        {
            query.Add(new("limit", size.ToString(CultureInfo.InvariantCulture)));
        }

        var answer = await transport.SendAsync(
            new HttpCall("events", HttpMethod.Get, RunPath(runId) + "/events", "/runs/{runId}/events", query, null, null, null, SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "a page");
        var items = raw.TryGetProperty("items", out var list) && list.ValueKind == JsonValueKind.Array
            ? list.EnumerateArray().Select(item => new JournalEvent(Object(item, "an event"))).ToList()
            : [];
        return new JournalPage(items, OptStr(raw, "nextCursor"));
    }

    /// <inheritdoc/>
    public async Task<Run> WaitForCompletionAsync(string runId, WaitOptions? options = null, CancellationToken cancellationToken = default)
    {
        options ??= new WaitOptions();
        var clock = Stopwatch.StartNew();
        for (; ; )
        {
            var run = await GetAsync(runId, cancellationToken).ConfigureAwait(false);
            if (run.IsTerminal)
            {
                return run;
            }

            if (clock.Elapsed >= options.Timeout)
            {
                throw new SagatException(
                    SagatErrorCode.Transport,
                    $"the run did not finish within {((long)options.Timeout.TotalMilliseconds).ToString(CultureInfo.InvariantCulture)} ms",
                    reason: "WAIT_TIMEOUT");
            }

            var pause = TimeSpan.FromMilliseconds(Math.Round(options.PollInterval.TotalMilliseconds * (0.8 + (Random.Shared.NextDouble() * 0.4))));
            await Task.Delay(pause, cancellationToken).ConfigureAwait(false);
        }
    }

    /// <inheritdoc/>
    public async Task<ChainInspection> WaitForChainCompletionAsync(string namespaceId, string chainId, WaitOptions? options = null, CancellationToken cancellationToken = default)
    {
        options ??= new WaitOptions();
        var clock = Stopwatch.StartNew();
        for (; ; )
        {
            var chain = await InspectChainAsync(namespaceId, chainId, cancellationToken: cancellationToken).ConfigureAwait(false);
            if (chain.FinalOutcome is not null)
            {
                return chain;
            }

            if (clock.Elapsed >= options.Timeout)
            {
                throw new SagatException(
                    SagatErrorCode.Transport,
                    $"the chain did not finish within {((long)options.Timeout.TotalMilliseconds).ToString(CultureInfo.InvariantCulture)} ms",
                    reason: "WAIT_TIMEOUT");
            }

            var pause = TimeSpan.FromMilliseconds(Math.Round(options.PollInterval.TotalMilliseconds * (0.8 + (Random.Shared.NextDouble() * 0.4))));
            await Task.Delay(pause, cancellationToken).ConfigureAwait(false);
        }
    }

    // ------------------------------------------------------------------------------------------ helpers

    /// <summary>The path of a run: its id is a UUID (SPEC section 10), so nothing in it can be a dot segment, a slash or a query.</summary>
    private static string RunPath(string runId) =>
        runId is not null && UuidPattern().IsMatch(runId)
            ? "/runs/" + runId
            : throw SagatException.Invalid("a run id is a UUID");

    private static string ChainPath(string namespaceId, string chainId) =>
        namespaceId is not null && UuidPattern().IsMatch(namespaceId) && chainId is not null && UuidPattern().IsMatch(chainId)
            ? "/namespaces/" + namespaceId.ToLowerInvariant() + "/chains/" + chainId.ToLowerInvariant()
            : throw SagatException.Invalid("namespace and chain ids are UUIDs");

    private static string ExpectedRunId(string runId) =>
        runId is not null && UuidPattern().IsMatch(runId) ? runId.ToLowerInvariant() : throw SagatException.Invalid("an expected run id is a UUID");

    private static string DeclaredOperationPath(string runId, string kind, string name) =>
        name is not null && DeclaredOperationNamePattern().IsMatch(name)
            ? RunPath(runId) + "/" + kind + "/" + Uri.EscapeDataString(name)
            : throw SagatException.Invalid("an operation name is 1 to 128 permitted characters and starts with a letter or digit");

    /// <summary>A name in a path (a signal's): percent-encoded, and never empty, <c>.</c> or <c>..</c>.</summary>
    private static string Segment(string name) =>
        string.IsNullOrEmpty(name) || name is "." or ".."
            ? throw SagatException.Invalid("a name in a path is not empty, \".\" or \"..\"")
            : Uri.EscapeDataString(name);

    private static string ActionName(RunAction action) => action switch
    {
        RunAction.Pause => "PAUSE",
        RunAction.Resume => "RESUME",
        RunAction.Cancel => "CANCEL",
        RunAction.RetryStep => "RETRY_STEP",
        RunAction.RetryCompensation => "RETRY_COMPENSATION",
        RunAction.SkipCompensation => "SKIP_COMPENSATION",
        RunAction.ResolveStep => "RESOLVE_STEP",
        RunAction.RerunAsNew => "RERUN_AS_NEW",
        _ => throw new ArgumentOutOfRangeException(nameof(action)),
    };

    /// <summary>An explicit <c>traceparent</c> is checked before anything is sent; the ambient one is read at send time.</summary>
    private static string? TraceOf(string? explicitValue) => explicitValue is null ? null : Traceparent.Check(explicitValue);

    private static string KeyOf(string? given)
    {
        var key = given ?? Guid.NewGuid().ToString();
        return KeyPattern().IsMatch(key) ? key : throw SagatException.Invalid("an idempotency key is 8 to 160 printable ASCII characters");
    }

    private void WriteObjectOrEmpty<T>(System.Text.Json.Utf8JsonWriter writer, T value)
    {
        if (value is null)
        {
            writer.WriteStartObject();
            writer.WriteEndObject();
            return;
        }

        Wire.WritePayload(writer, value, json);
    }

    private static JsonElement Object(JsonElement? value, string what) =>
        value is { ValueKind: JsonValueKind.Object } element
            ? element
            : throw new SagatException(SagatErrorCode.Protocol, $"{what} is not an object");

    private static string Str(JsonElement raw, string key) =>
        raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()!
            : throw new SagatException(SagatErrorCode.Protocol, $"{key} is missing from the answer");

    private static string? OptStr(JsonElement raw, string key) =>
        raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.String ? value.GetString() : null;

    private static DateTimeOffset? OptDate(JsonElement raw, string key) =>
        Wire.TryParseInstant(OptStr(raw, key), out var value) ? value : null;

    private static long Num(JsonElement raw, string key) =>
        raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.Number
            ? (value.TryGetInt64(out var integer) ? integer : (long)value.GetDouble())
            : 0;

    private static long Revision(JsonElement raw, string key, long minimum) =>
        raw.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.Number
            && value.TryGetInt64(out var revision) && revision >= minimum
                ? revision
                : throw new SagatException(SagatErrorCode.Protocol, $"{key} is invalid in the answer");

    internal static Run ToRun(JsonElement? value)
    {
        var raw = Object(value, "a run");
        var created = OptDate(raw, "createdAt") ?? throw new SagatException(SagatErrorCode.Protocol, "createdAt is missing from the answer");
        ParentRun? parent = null;
        if (raw.TryGetProperty("parent", out var p) && p.ValueKind == JsonValueKind.Object)
        {
            parent = new ParentRun(OptStr(p, "runId") ?? string.Empty, OptStr(p, "node") ?? string.Empty, OptStr(p, "cancelPolicy") ?? string.Empty);
        }

        return new Run
        {
            RunId = Str(raw, "runId"),
            WorkflowId = Str(raw, "workflowId"),
            VersionId = Str(raw, "versionId"),
            Status = Str(raw, "status"),
            Version = Num(raw, "version"),
            Paused = raw.TryGetProperty("paused", out var paused) && paused.ValueKind == JsonValueKind.True,
            StartedBy = OptStr(raw, "startedBy"),
            CreatedAt = created,
            UpdatedAt = OptDate(raw, "updatedAt"),
            FinishedAt = OptDate(raw, "finishedAt"),
            Deadline = OptDate(raw, "deadline"),
            TraceId = OptStr(raw, "traceId"),
            CorrelationId = OptStr(raw, "correlationId"),
            Depth = (int)Num(raw, "depth"),
            Parent = parent,
            Chain = ToRunChain(raw),
            RerunOf = OptStr(raw, "rerunOf"),
            Raw = raw,
        };
    }

    private static string? CompletionKind(JsonElement element)
    {
        var kind = OptStr(element, "completionKind");
        return kind is null or "CONTINUED" or "FINAL"
            ? kind
            : throw new SagatException(SagatErrorCode.Protocol, "completionKind is invalid in the answer");
    }

    private static RunChainSegment? ToRunChain(JsonElement run)
    {
        if (!run.TryGetProperty("chain", out var value) || value.ValueKind == JsonValueKind.Null)
        {
            return null;
        }

        var chain = Object(value, "a run chain segment");
        var index = Revision(chain, "segmentIndex", 0);
        if (index > 9_999)
        {
            throw new SagatException(SagatErrorCode.Protocol, "segmentIndex is invalid in the answer");
        }

        return new RunChainSegment(Str(chain, "chainId"), (int)index, CompletionKind(chain), OptStr(chain, "successorRunId"));
    }
}
