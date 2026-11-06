using System.Text.Json;
using SagatOctopusOrchestrator.Sdk.Internal;
using static SagatOctopusOrchestrator.Sdk.Internal.Answers;

namespace SagatOctopusOrchestrator.Sdk;

// The calls of feature 018 that sit on the client itself: the start calls that name a business identifier, and the reset of a run.
// Rules shared with SPEC section 3: every call that changes something carries one Idempotency-Key, given or generated once and kept by every
// retry; a value of an enumeration this SDK does not know is kept as the raw string it came as; every typed answer keeps the JSON it was read from.
public sealed partial class SagatClient
{
    /// <summary>
    /// A start delay given as a duration: the integer number of milliseconds sent, computed once before the first attempt and truncated toward
    /// zero. A <see cref="TimeSpan"/> is always finite; the range (1 ms to 366 days) is the server's to judge.
    /// </summary>
    private static long? DelayMs(TimeSpan? delay) =>
        delay is { } span ? span.Ticks / TimeSpan.TicksPerMillisecond : null;

    /// <inheritdoc/>
    public async Task<SignalWithStartResult> SignalWithStartAsync<TInput, TPayload>(
        string workflow, string businessId, TInput input, string signalName, TPayload payload, WithStartOptions? options = null, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(businessId);
        ArgumentNullException.ThrowIfNull(signalName);
        options ??= new WithStartOptions();
        var key = KeyOf(options.IdempotencyKey);
        var traceparent = TraceOf(options.Traceparent);
        var workflowId = await ResolveWorkflowAsync(workflow, cancellationToken).ConfigureAwait(false);
        var deadline = DeadlineOf(options);
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("workflowId", workflowId);
            writer.WriteString("businessId", businessId);
            writer.WritePropertyName("input");
            WriteObjectOrEmpty(writer, input);
            writer.WriteStartObject("signal");
            writer.WriteString("name", signalName);
            writer.WritePropertyName("payload");
            WriteObjectOrEmpty(writer, payload);
            writer.WriteEndObject();
            WriteWithStartFields(writer, options, deadline);
            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("signal-with-start", HttpMethod.Post, "/runs/signal-with-start", "/runs/signal-with-start", [], body, key, traceparent, SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = ObjectOf(answer.Body, "a signal-with-start result");
        return new SignalWithStartResult(Str(raw, "runId"), Flag(raw, "started"), Strings(raw, "deliveredTo"), raw);
    }

    /// <inheritdoc/>
    public async Task<UpdateWithStartResult> UpdateWithStartAsync<TInput, TArgs>(
        string workflow, string businessId, TInput input, string updateName, string updateKey, TArgs args, UpdateWithStartOptions? options = null, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(businessId);
        if (string.IsNullOrEmpty(updateKey) || !UpdateKeyPattern().IsMatch(updateKey))
        {
            throw SagatException.Invalid("an update key is 1 to 128 printable ASCII characters");
        }

        options ??= new UpdateWithStartOptions();
        if (options.ExpectedStateRevision is < 0)
        {
            throw SagatException.Invalid("expected state revision is non-negative");
        }

        if (updateName is null || !DeclaredOperationNamePattern().IsMatch(updateName))
        {
            throw SagatException.Invalid("an operation name is 1 to 128 permitted characters and starts with a letter or digit");
        }

        var key = KeyOf(options.IdempotencyKey);
        var traceparent = TraceOf(options.Traceparent);
        var startDelayMs = DelayMs(options.StartDelay);
        var workflowId = await ResolveWorkflowAsync(workflow, cancellationToken).ConfigureAwait(false);
        var deadline = DeadlineOf(options);
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("workflowId", workflowId);
            writer.WriteString("businessId", businessId);
            writer.WritePropertyName("input");
            WriteObjectOrEmpty(writer, input);
            writer.WriteStartObject("update");
            writer.WriteString("name", updateName);
            writer.WriteString("updateKey", updateKey);
            writer.WritePropertyName("args");
            Wire.WritePayload(writer, args, json);
            if (options.ExpectedStateRevision is { } expected)
            {
                writer.WriteNumber("expectedStateRevision", expected);
            }

            writer.WriteEndObject();
            WriteWithStartFields(writer, options, deadline);
            if (options.BusinessIdConflictPolicy is not null)
            {
                writer.WriteString("businessIdConflictPolicy", options.BusinessIdConflictPolicy);
            }

            if (startDelayMs is { } delay)
            {
                writer.WriteNumber("startDelayMs", delay);
            }

            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("update-with-start", HttpMethod.Post, "/runs/update-with-start", "/runs/update-with-start", [], body, key, traceparent, SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = ObjectOf(answer.Body, "an update-with-start result");
        var update = ObjectOf(raw.TryGetProperty("update", out var found) ? found : null, "the update of the result");
        UpdateWithStartReceipt? receipt = null;
        if (update.TryGetProperty("receipt", out var receiptValue) && receiptValue.ValueKind != JsonValueKind.Null)
        {
            var r = ObjectOf(receiptValue, "an update receipt");
            receipt = new UpdateWithStartReceipt(
                Str(r, "runId"), Str(r, "operation"), Str(r, "updateKey"), Str(r, "status"),
                r.TryGetProperty("result", out var result) ? result.Clone() : JsonSerializer.SerializeToElement<object?>(null),
                Revision(r, "beforeStateRevision", 0), Revision(r, "afterStateRevision", 0), Revision(r, "runVersion", 0), Flag(r, "replayed"), r);
        }

        return new UpdateWithStartResult(
            Str(raw, "runId"), Flag(raw, "started"), new UpdateWithStartOutcome(Str(update, "outcome"), receipt, OptStr(update, "reason")), raw);
    }

    private static DateTimeOffset? DeadlineOf(WithStartOptions options) =>
        options.DeadlineIn is { } span ? DateTimeOffset.UtcNow + span : options.Deadline;

    private static void WriteWithStartFields(Utf8JsonWriter writer, WithStartOptions options, DateTimeOffset? deadline)
    {
        if (options.VersionId is { Length: > 0 } versionId)
        {
            writer.WriteString("versionId", versionId);
        }

        if (options.CorrelationId is not null)
        {
            writer.WriteString("correlationId", options.CorrelationId);
        }

        if (deadline is { } at)
        {
            writer.WriteString("deadline", Wire.Instant(at));
        }

        if (options.BusinessIdReusePolicy is not null)
        {
            writer.WriteString("businessIdReusePolicy", options.BusinessIdReusePolicy);
        }
    }

    // --------------------------------------------------------------------------------------------- reset

    /// <inheritdoc/>
    public async Task<Page<ResetPoint>> ResetPointsAsync(string runId, ResetPointListOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = "/runs/" + Requests.Run(runId) + "/reset-points";
        var query = new QueryBuilder().Add("cursor", options?.Cursor).Add("limit", options?.Limit).Build();
        var answer = await transport.SendAsync(
            new HttpCall("reset points", HttpMethod.Get, path, "/runs/{runId}/reset-points", query, null, null, TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return PageOf(answer.Body, "a page of reset points", item =>
        {
            var raw = ObjectOf(item, "a reset point");
            return new ResetPoint
            {
                Sequence = Revision(raw, "sequence", 1),
                Kind = Str(raw, "kind"),
                Node = OptStr(raw, "node"),
                OccurredAt = Date(raw, "occurredAt"),
                Raw = raw,
            };
        });
    }

    /// <inheritdoc/>
    public async Task<ResetPlan> PreviewResetAsync(string runId, long sequence, string? reapply = null, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        if (sequence < 1)
        {
            throw SagatException.Invalid("sequence is a positive integer");
        }

        var path = "/runs/" + Requests.Run(runId) + "/reset-previews";
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteNumber("sequence", sequence);
            if (reapply is not null)
            {
                writer.WriteString("reapply", reapply);
            }

            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("preview reset", HttpMethod.Post, path, "/runs/{runId}/reset-previews", [], body, null, TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = ObjectOf(answer.Body, "a reset plan");
        return new ResetPlan
        {
            Sequence = Revision(raw, "sequence", 1),
            Restored = ListOf(raw, "restored", item =>
            {
                var step = ObjectOf(item, "a restored step");
                return new ResetRestoredStep(Str(step, "node"), Str(step, "kind"));
            }),
            Reexecuted = ListOf(raw, "reexecuted", item =>
            {
                var step = ObjectOf(item, "a re-executed step");
                return new ResetReexecutedStep(Str(step, "node"), Flag(step, "completedInSource"), Strings(step, "markers"));
            }),
            Reapply = Elements(raw, "reapply"),
            RequiresAcknowledgement = Flag(raw, "requiresAcknowledgement"),
            PrefixEvents = Revision(raw, "prefixEvents", 1),
            Raw = raw,
        };
    }

    /// <inheritdoc/>
    public async Task<Reset> ResetAsync(string runId, ResetRequest request, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(request);
        if (request.Sequence < 1)
        {
            throw SagatException.Invalid("sequence is a positive integer");
        }

        var path = "/runs/" + Requests.Run(runId) + "/resets";
        var key = KeyOf(options?.IdempotencyKey);
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteNumber("sequence", request.Sequence);
            writer.WriteString("reason", request.Reason);
            if (request.Reapply is not null)
            {
                writer.WriteString("reapply", request.Reapply);
            }

            if (request.SourcePolicy is not null)
            {
                writer.WriteString("sourcePolicy", request.SourcePolicy);
            }

            if (request.BusinessId is not null)
            {
                writer.WriteString("businessId", request.BusinessId);
            }

            if (request.AcknowledgeReexecution is { } acknowledge)
            {
                writer.WriteBoolean("acknowledgeReexecution", acknowledge);
            }

            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("reset", HttpMethod.Post, path, "/runs/{runId}/resets", [], body, key, TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToReset(answer.Body);
    }

    /// <inheritdoc/>
    public async Task<IReadOnlyList<Reset>> ListResetsAsync(string runId, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = "/runs/" + Requests.Run(runId) + "/resets";
        var answer = await transport.SendAsync(
            new HttpCall("list resets", HttpMethod.Get, path, "/runs/{runId}/resets", [], null, null, TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ListOf(ObjectOf(answer.Body, "a list of resets"), "items", item => ToReset(item));
    }

    /// <inheritdoc/>
    public async Task<Reset> GetResetAsync(string runId, string resetId, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = "/runs/" + Requests.Run(runId) + "/resets/" + Requests.Uuid(resetId, "a reset id");
        var answer = await transport.SendAsync(
            new HttpCall("get reset", HttpMethod.Get, path, "/runs/{runId}/resets/{resetId}", [], null, null, TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToReset(answer.Body);
    }

    private static Reset ToReset(JsonElement? value)
    {
        var raw = ObjectOf(value, "a reset");
        return new Reset
        {
            ResetId = Str(raw, "resetId"),
            SourceRunId = Str(raw, "sourceRunId"),
            Sequence = Revision(raw, "sequence", 1),
            State = Str(raw, "state"),
            FailureReason = OptStr(raw, "failureReason"),
            NewRunId = OptStr(raw, "newRunId"),
            ReapplyReport = Elements(raw, "reapplyReport"),
            CreatedAt = OptDate(raw, "createdAt"),
            Raw = raw,
        };
    }
}
