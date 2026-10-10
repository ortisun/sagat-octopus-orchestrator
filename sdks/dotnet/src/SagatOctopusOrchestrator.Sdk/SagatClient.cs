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
            raw);
    }

    /// <inheritdoc/>
    public Task<OperationResult> CancelAsync(string runId, string reason, OperateOptions? options = null, CancellationToken cancellationToken = default) =>
        OperateAsync(runId, RunAction.Cancel, reason, options, cancellationToken);

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

    // ------------------------------------------------------------------------------------------ helpers

    /// <summary>The path of a run: its id is a UUID (SPEC section 10), so nothing in it can be a dot segment, a slash or a query.</summary>
    private static string RunPath(string runId) =>
        runId is not null && UuidPattern().IsMatch(runId)
            ? "/runs/" + runId
            : throw SagatException.Invalid("a run id is a UUID");

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
            Raw = raw,
        };
    }
}
