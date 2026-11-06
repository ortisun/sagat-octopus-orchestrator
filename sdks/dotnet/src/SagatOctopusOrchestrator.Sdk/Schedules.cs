using System.Diagnostics.CodeAnalysis;
using System.Runtime.CompilerServices;
using System.Text.Json;
using SagatOctopusOrchestrator.Sdk.Internal;
using static SagatOctopusOrchestrator.Sdk.Internal.Answers;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>What happens to an action of a schedule while a run the schedule started is still open (<c>OverlapPolicy</c>). The set only grows: a value of a newer server is kept as the string it came as.</summary>
public static class OverlapPolicies
{
    /// <summary>The action is skipped.</summary>
    public const string Skip = "SKIP";

    /// <summary>One waiting time is kept; the next is dropped.</summary>
    public const string BufferOne = "BUFFER_ONE";

    /// <summary>Every waiting time is kept.</summary>
    public const string BufferAll = "BUFFER_ALL";

    /// <summary>The open run is cancelled and a new one starts.</summary>
    public const string CancelOther = "CANCEL_OTHER";

    /// <summary>Another run starts.</summary>
    public const string AllowAll = "ALLOW_ALL";
}

/// <summary>A range of a calendar field: <c>Start</c>, optionally to <c>End</c>, optionally every <c>Step</c>.</summary>
public sealed record ScheduleRange(long Start, long? End = null, long? Step = null);

/// <summary>A calendar of a schedule: each field is a list of ranges, and a field left out matches any value.</summary>
public sealed record ScheduleCalendarSpec
{
    /// <summary>The seconds.</summary>
    public IReadOnlyList<ScheduleRange>? Second { get; init; }

    /// <summary>The minutes.</summary>
    public IReadOnlyList<ScheduleRange>? Minute { get; init; }

    /// <summary>The hours.</summary>
    public IReadOnlyList<ScheduleRange>? Hour { get; init; }

    /// <summary>The days of the month.</summary>
    public IReadOnlyList<ScheduleRange>? DayOfMonth { get; init; }

    /// <summary>The months.</summary>
    public IReadOnlyList<ScheduleRange>? Month { get; init; }

    /// <summary>The years.</summary>
    public IReadOnlyList<ScheduleRange>? Year { get; init; }

    /// <summary>The days of the week.</summary>
    public IReadOnlyList<ScheduleRange>? DayOfWeek { get; init; }

    /// <summary>A comment kept with the calendar.</summary>
    public string? Comment { get; init; }
}

/// <summary>An interval of a schedule: every <c>EveryMs</c> milliseconds, from the epoch shifted by <c>OffsetMs</c>.</summary>
public sealed record ScheduleInterval(long EveryMs, long? OffsetMs = null);

/// <summary>What a calendar does at a daylight-saving change.</summary>
public sealed record ScheduleDst
{
    /// <summary>A time that does not exist: <c>SKIP</c> or <c>SHIFT_FORWARD</c>.</summary>
    public string? Gap { get; init; }

    /// <summary>A time that exists twice: <c>ONCE_EARLIER</c> or <c>TWICE</c>.</summary>
    public string? Ambiguous { get; init; }
}

/// <summary>When a schedule acts: calendars, cron strings and intervals, less the exclusions, between <see cref="StartAt"/> and <see cref="EndAt"/>.</summary>
public sealed record ScheduleSpec
{
    /// <summary>Calendars.</summary>
    public IReadOnlyList<ScheduleCalendarSpec>? Calendars { get; init; }

    /// <summary>Cron strings.</summary>
    public IReadOnlyList<string>? Cron { get; init; }

    /// <summary>Intervals.</summary>
    public IReadOnlyList<ScheduleInterval>? Intervals { get; init; }

    /// <summary>Calendars whose times never act.</summary>
    public IReadOnlyList<ScheduleCalendarSpec>? Exclusions { get; init; }

    /// <summary>The schedule does not act before this instant. Sent in UTC.</summary>
    public Instant? StartAt { get; init; }

    /// <summary>The schedule does not act from this instant. Sent in UTC.</summary>
    public Instant? EndAt { get; init; }

    /// <summary>An IANA time zone (default UTC).</summary>
    public string? Timezone { get; init; }

    /// <summary>The most a time may be moved by, in milliseconds.</summary>
    public long? JitterMs { get; init; }

    /// <summary>What a calendar does at a daylight-saving change.</summary>
    public ScheduleDst? Dst { get; init; }
}

/// <summary>What a schedule does when an action meets an open run, and how it catches up.</summary>
public sealed record SchedulePolicies
{
    /// <summary>See <see cref="OverlapPolicies"/>.</summary>
    public string? Overlap { get; init; }

    /// <summary>How far back a missed time is still acted on, in milliseconds.</summary>
    public long? CatchupWindowMs { get; init; }

    /// <summary><c>ONE</c> or <c>ALL</c>.</summary>
    public string? CatchupMode { get; init; }

    /// <summary>Whether the schedule pauses when the run it started fails.</summary>
    public bool? PauseOnFailure { get; init; }

    /// <summary>How many times may wait under a buffering overlap policy.</summary>
    public long? BufferLimit { get; init; }
}

/// <summary>The run a schedule starts.</summary>
public sealed record ScheduleAction
{
    /// <summary>The workflow (a UUID).</summary>
    public required string WorkflowId { get; init; }

    /// <summary><c>ACTIVE</c> (default) or <c>PINNED</c>.</summary>
    public string? VersionPolicy { get; init; }

    /// <summary>The pinned version.</summary>
    public string? VersionId { get; init; }

    /// <summary>The input of every run, at most 64 KiB; anything System.Text.Json can write as an object. It is never returned: only its digest is.</summary>
    public required object Input { get; init; }

    /// <summary>The field of the input that receives the nominal time.</summary>
    public string? NominalTimeField { get; init; }

    /// <summary>The timeout of every run, in milliseconds.</summary>
    public long? RunTimeoutMs { get; init; }
}

/// <summary>What a schedule is written as (<c>createSchedule</c>, <c>updateSchedule</c>). A field left <c>null</c> is left out of the request, never sent as null.</summary>
public sealed record ScheduleWrite
{
    /// <summary>The name, unique in the namespace.</summary>
    public required string Name { get; init; }

    /// <summary>When it acts.</summary>
    public required ScheduleSpec Spec { get; init; }

    /// <summary>Overlap and catch-up policies.</summary>
    public SchedulePolicies? Policies { get; init; }

    /// <summary>The run it starts.</summary>
    public required ScheduleAction Action { get; init; }

    /// <summary>Whether it is created paused.</summary>
    public bool? Paused { get; init; }

    /// <summary>A note kept with the revision.</summary>
    public string? Note { get; init; }

    /// <summary>How many actions remain before the schedule is exhausted.</summary>
    public long? RemainingActions { get; init; }
}

/// <summary>The action of a schedule as the server keeps it: without the input, with its digest.</summary>
public sealed record ScheduleActionSummary(
    string? WorkflowId, string? VersionPolicy, string? VersionId, string? InputDigest, string? NominalTimeField, long? RunTimeoutMs);

/// <summary>A schedule. <see cref="Raw"/> keeps the answer as the server sent it.</summary>
public record Schedule
{
    /// <summary>The id.</summary>
    public required string ScheduleId { get; init; }

    /// <summary>The name.</summary>
    public required string Name { get; init; }

    /// <summary><c>API</c> or <c>TRIGGER</c>.</summary>
    public required string Origin { get; init; }

    /// <summary><c>ACTIVE</c> or <c>PAUSED</c>; a newer server may send another.</summary>
    public required string State { get; init; }

    /// <summary>The revision (the first is 1); the <c>expectedRevision</c> of an update or a delete.</summary>
    public required long Revision { get; init; }

    /// <summary>Whether it has no action left.</summary>
    public bool Exhausted { get; init; }

    /// <summary>The action, without its input.</summary>
    public required ScheduleActionSummary Action { get; init; }

    /// <summary>The note of the latest revision.</summary>
    public string? Note { get; init; }

    /// <summary>Who wrote the note.</summary>
    public string? NoteBy { get; init; }

    /// <summary>The actions left, or <c>null</c> for no limit.</summary>
    public long? RemainingActions { get; init; }

    /// <summary>The next nominal time (UTC).</summary>
    public DateTimeOffset? NextActionAt { get; init; }

    /// <summary>When the schedule was created (UTC).</summary>
    public required DateTimeOffset CreatedAt { get; init; }

    /// <summary>When it last changed (UTC).</summary>
    public DateTimeOffset? UpdatedAt { get; init; }

    /// <summary>The answer as the server sent it.</summary>
    public JsonElement Raw { get; init; }
}

/// <summary>A nominal time and the instant the schedule will act on it.</summary>
public sealed record ScheduleNextAction(DateTimeOffset NominalTime, DateTimeOffset ActualTime);

/// <summary>What a schedule decided about one nominal time. <see cref="Outcome"/> is one of the outcomes the SDK knows or another string: the set only grows.</summary>
public sealed record ScheduleActionRecord
{
    /// <summary>The nominal time (UTC).</summary>
    public required DateTimeOffset NominalTime { get; init; }

    /// <summary>When the action was due (UTC).</summary>
    public DateTimeOffset? ActualTime { get; init; }

    /// <summary>The kind of action (<c>SCHEDULED</c>, <c>TRIGGERED</c>, <c>BACKFILL</c>, ...).</summary>
    public required string Kind { get; init; }

    /// <summary>What asked for it.</summary>
    public string? Origin { get; init; }

    /// <summary>The revision of the schedule that decided.</summary>
    public required long Revision { get; init; }

    /// <summary>What became of the action: <c>STARTED</c>, <c>SKIPPED_OVERLAP</c>, <c>ALREADY_ACTED</c>, ...</summary>
    public required string Outcome { get; init; }

    /// <summary>Why, when the outcome needs it.</summary>
    public string? Reason { get; init; }

    /// <summary>The run it started.</summary>
    public string? RunId { get; init; }

    /// <summary>The backfill it belongs to.</summary>
    public string? BackfillId { get; init; }

    /// <summary>How many times a summary record stands for.</summary>
    public long? SummaryCount { get; init; }

    /// <summary>The latest nominal time a summary record stands for.</summary>
    public DateTimeOffset? SummaryLastNominalTime { get; init; }

    /// <summary>When the decision was made (UTC).</summary>
    public required DateTimeOffset DecidedAt { get; init; }

    /// <summary>The answer as the server sent it.</summary>
    public JsonElement Raw { get; init; }
}

/// <summary>A schedule with its next times, its latest decisions, the runs it has open and its counters.</summary>
public sealed record ScheduleDescription : Schedule
{
    /// <summary>Copies a schedule and adds what the description says.</summary>
    [SetsRequiredMembers]
    internal ScheduleDescription(Schedule schedule)
        : base(schedule)
    {
    }

    /// <summary>The next times the schedule will act.</summary>
    public IReadOnlyList<ScheduleNextAction> NextActions { get; init; } = [];

    /// <summary>The latest decisions, newest first.</summary>
    public IReadOnlyList<ScheduleActionRecord> RecentActions { get; init; } = [];

    /// <summary>The runs the schedule started that are still open.</summary>
    public IReadOnlyList<string> OpenRuns { get; init; } = [];

    /// <summary>How many times wait under a buffering overlap policy.</summary>
    public long BufferedCount { get; init; }

    /// <summary>How many actions had each outcome, by outcome name.</summary>
    public IReadOnlyDictionary<string, long> Counters { get; init; } = new Dictionary<string, long>();
}

/// <summary>A backfill: the nominal times of a past range, each acted on once.</summary>
public sealed record ScheduleBackfill
{
    /// <summary>The id.</summary>
    public required string BackfillId { get; init; }

    /// <summary>The start of the range (UTC).</summary>
    public required DateTimeOffset Start { get; init; }

    /// <summary>The end of the range (UTC).</summary>
    public required DateTimeOffset End { get; init; }

    /// <summary>How far the backfill has got (UTC).</summary>
    public DateTimeOffset? Cursor { get; init; }

    /// <summary><c>PENDING</c>, <c>COMPLETED</c>, <c>CANCELLED</c>, ...</summary>
    public required string State { get; init; }

    /// <summary>The overlap policy of the backfill.</summary>
    public required string Overlap { get; init; }

    /// <summary>How many times had each outcome, by outcome name.</summary>
    public IReadOnlyDictionary<string, long> Counts { get; init; } = new Dictionary<string, long>();

    /// <summary>The answer as the server sent it.</summary>
    public JsonElement Raw { get; init; }
}

/// <summary>The filter of a listing of schedules.</summary>
public sealed record ScheduleListOptions
{
    /// <summary>The <see cref="Page{T}.NextCursor"/> of the previous page; opaque.</summary>
    public string? Cursor { get; init; }

    /// <summary>The page size.</summary>
    public int? Limit { get; init; }

    /// <summary><c>ACTIVE</c> or <c>PAUSED</c>.</summary>
    public string? State { get; init; }

    /// <summary><c>API</c> or <c>TRIGGER</c>.</summary>
    public string? Origin { get; init; }

    /// <summary>Only the schedules of this workflow (a UUID).</summary>
    public string? WorkflowId { get; init; }

    /// <summary>Names that start with this text.</summary>
    public string? Name { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>Options of the description of a schedule.</summary>
public sealed record ScheduleDescribeOptions
{
    /// <summary>How many next times to include.</summary>
    public int? NextCount { get; init; }

    /// <summary>How many recent decisions to include.</summary>
    public int? RecentCount { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>Options of the history of a schedule.</summary>
public sealed record ScheduleActionListOptions
{
    /// <summary>The <see cref="Page{T}.NextCursor"/> of the previous page; opaque.</summary>
    public string? Cursor { get; init; }

    /// <summary>The page size.</summary>
    public int? Limit { get; init; }

    /// <summary>Only the decisions with this outcome.</summary>
    public string? Outcome { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>Options of a trigger.</summary>
public sealed record ScheduleTriggerOptions
{
    /// <summary>The overlap policy for this action only; the schedule's own when <c>null</c>.</summary>
    public string? Overlap { get; init; }

    /// <summary>8 to 160 printable ASCII characters. A fresh one per call when null, reused by every retry of that call.</summary>
    public string? IdempotencyKey { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>Options of a backfill.</summary>
public sealed record ScheduleBackfillOptions
{
    /// <summary>The overlap policy of the backfill; the schedule's own when <c>null</c>.</summary>
    public string? Overlap { get; init; }

    /// <summary>8 to 160 printable ASCII characters. A fresh one per call when null, reused by every retry of that call.</summary>
    public string? IdempotencyKey { get; init; }

    /// <summary>An explicit W3C <c>traceparent</c>.</summary>
    public string? Traceparent { get; init; }
}

/// <summary>
/// The schedules of a namespace (feature 018). Every call that changes something carries one <c>Idempotency-Key</c>, given by the caller or
/// generated once per call and reused by every retry of it; an id that becomes a path segment must be a UUID.
/// </summary>
[System.Diagnostics.DebuggerDisplay("{ToString(),nq}")]
public sealed class SchedulesClient
{
    [System.Diagnostics.DebuggerBrowsable(System.Diagnostics.DebuggerBrowsableState.Never)]
    private readonly HttpTransport transport;
    [System.Diagnostics.DebuggerBrowsable(System.Diagnostics.DebuggerBrowsableState.Never)]
    private readonly JsonSerializerOptions json;

    internal SchedulesClient(HttpTransport transport, JsonSerializerOptions json)
    {
        this.transport = transport;
        this.json = json;
    }

    /// <summary>The group; it shows nothing of the client that holds it.</summary>
    public override string ToString() => nameof(SchedulesClient);

    /// <summary>Creates a schedule. A name in use is <c>CONFLICT</c>; the same key and body answers what the first call did.</summary>
    public async Task<Schedule> CreateAsync(string namespaceId, ScheduleWrite write, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId);
        var body = Body(write, expectedRevision: null);
        var answer = await transport.SendAsync(
            new HttpCall("create schedule", HttpMethod.Post, path, "/namespaces/{namespaceId}/schedules", [], body, SagatClient.KeyOf(options?.IdempotencyKey), SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToSchedule(answer.Body);
    }

    /// <summary>A schedule with its next times, its latest decisions, the runs it has open and its counters.</summary>
    public async Task<ScheduleDescription> GetAsync(string namespaceId, string scheduleId, ScheduleDescribeOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId, scheduleId);
        var query = new QueryBuilder().Add("nextCount", options?.NextCount).Add("recentCount", options?.RecentCount).Build();
        var answer = await transport.SendAsync(
            new HttpCall("get schedule", HttpMethod.Get, path, "/namespaces/{namespaceId}/schedules/{scheduleId}", query, null, null, SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = ObjectOf(answer.Body, "a schedule description");
        return new ScheduleDescription(ToSchedule(raw))
        {
            NextActions = ListOf(raw, "nextActions", item =>
            {
                var entry = ObjectOf(item, "a next action");
                return new ScheduleNextAction(Date(entry, "nominalTime"), Date(entry, "actualTime"));
            }),
            RecentActions = ListOf(raw, "recentActions", item => ToActionRecord(item)),
            OpenRuns = Strings(raw, "openRuns"),
            BufferedCount = Num(raw, "bufferedCount"),
            Counters = Counters(raw, "counters"),
        };
    }

    /// <summary>One page of schedules.</summary>
    public async Task<Page<Schedule>> ListAsync(string namespaceId, ScheduleListOptions? filter = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId);
        var query = new QueryBuilder()
            .Add("cursor", filter?.Cursor)
            .Add("limit", filter?.Limit)
            .Add("state", filter?.State)
            .Add("origin", filter?.Origin)
            .Add("name", filter?.Name)
            .Add("workflowId", filter?.WorkflowId is { } workflowId ? Requests.Uuid(workflowId, "a workflow id") : null)
            .Build();
        var answer = await transport.SendAsync(
            new HttpCall("list schedules", HttpMethod.Get, path, "/namespaces/{namespaceId}/schedules", query, null, null, SagatClient.TraceOf(filter?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return PageOf(answer.Body, "a page of schedules", item => ToSchedule(item));
    }

    /// <summary>Every schedule the filter selects, following the opaque cursors.</summary>
    public async IAsyncEnumerable<Schedule> ListAllAsync(string namespaceId, ScheduleListOptions? filter = null, [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        var current = (filter ?? new ScheduleListOptions()) with { Cursor = null };
        do
        {
            var page = await ListAsync(namespaceId, current, cancellationToken).ConfigureAwait(false);
            foreach (var schedule in page.Items)
            {
                yield return schedule;
            }

            current = current with { Cursor = page.NextCursor };
        }
        while (current.Cursor is not null);
    }

    /// <summary>Replaces the spec, the policies and the action as a new revision; a stale <paramref name="expectedRevision"/> is <c>CONFLICT</c> with the reason <c>REVISION_CONFLICT</c>.</summary>
    public async Task<Schedule> UpdateAsync(
        string namespaceId, string scheduleId, ScheduleWrite write, long expectedRevision, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var revision = Requests.ExpectedRevision(expectedRevision);
        var path = PathOf(namespaceId, scheduleId);
        var body = Body(write, revision);
        var answer = await transport.SendAsync(
            new HttpCall("update schedule", HttpMethod.Put, path, "/namespaces/{namespaceId}/schedules/{scheduleId}", [], body, SagatClient.KeyOf(options?.IdempotencyKey), SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToSchedule(answer.Body);
    }

    /// <summary>Deletes a schedule (it answers 204). A stale <paramref name="expectedRevision"/> is <c>CONFLICT</c>; the name is free afterwards.</summary>
    public async Task DeleteAsync(string namespaceId, string scheduleId, long expectedRevision, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var revision = Requests.ExpectedRevision(expectedRevision);
        var path = PathOf(namespaceId, scheduleId);
        await transport.SendAsync(
            new HttpCall(
                "delete schedule", HttpMethod.Delete, path, "/namespaces/{namespaceId}/schedules/{scheduleId}",
                new QueryBuilder().Add("expectedRevision", revision).Build(), null, SagatClient.KeyOf(options?.IdempotencyKey), SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
    }

    /// <summary>Pauses a schedule; the note is required (not empty).</summary>
    public Task<Schedule> PauseAsync(string namespaceId, string scheduleId, string note, RequestOptions? options = null, CancellationToken cancellationToken = default) =>
        NoteAsync(namespaceId, scheduleId, "pause", note, options, cancellationToken);

    /// <summary>Resumes a schedule; the note is required (not empty).</summary>
    public Task<Schedule> ResumeAsync(string namespaceId, string scheduleId, string note, RequestOptions? options = null, CancellationToken cancellationToken = default) =>
        NoteAsync(namespaceId, scheduleId, "resume", note, options, cancellationToken);

    private async Task<Schedule> NoteAsync(string namespaceId, string scheduleId, string verb, string note, RequestOptions? options, CancellationToken cancellationToken)
    {
        if (string.IsNullOrEmpty(note))
        {
            throw SagatException.Invalid("a note is a non-empty text");
        }

        var path = PathOf(namespaceId, scheduleId, "/" + verb);
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("note", note);
            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall(verb + " schedule", HttpMethod.Post, path, "/namespaces/{namespaceId}/schedules/{scheduleId}/" + verb, [], body, SagatClient.KeyOf(options?.IdempotencyKey), SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToSchedule(answer.Body);
    }

    /// <summary>Acts on the schedule now, also while it is paused. The answer says what became of the action.</summary>
    public async Task<ScheduleActionRecord> TriggerAsync(string namespaceId, string scheduleId, ScheduleTriggerOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId, scheduleId, "/trigger");
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            if (options?.Overlap is { } overlap)
            {
                writer.WriteString("overlap", overlap);
            }

            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("trigger schedule", HttpMethod.Post, path, "/namespaces/{namespaceId}/schedules/{scheduleId}/trigger", [], body, SagatClient.KeyOf(options?.IdempotencyKey), SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToActionRecord(answer.Body);
    }

    /// <summary>Acts on every nominal time of the past range <c>[start, end)</c> that has no action yet.</summary>
    public async Task<ScheduleBackfill> BackfillAsync(
        string namespaceId, string scheduleId, Instant start, Instant end, ScheduleBackfillOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId, scheduleId, "/backfills");
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("start", start.ToString());
            writer.WriteString("end", end.ToString());
            if (options?.Overlap is { } overlap)
            {
                writer.WriteString("overlap", overlap);
            }

            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(
            new HttpCall("backfill schedule", HttpMethod.Post, path, "/namespaces/{namespaceId}/schedules/{scheduleId}/backfills", [], body, SagatClient.KeyOf(options?.IdempotencyKey), SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToBackfill(answer.Body);
    }

    /// <summary>The same, with the range given as RFC 3339 text; text that is not an instant is <c>VALIDATION_FAILED</c> and nothing is sent.</summary>
    public Task<ScheduleBackfill> BackfillAsync(
        string namespaceId, string scheduleId, string start, string end, ScheduleBackfillOptions? options = null, CancellationToken cancellationToken = default) =>
        BackfillAsync(namespaceId, scheduleId, Instant.Parse(start), Instant.Parse(end), options, cancellationToken);

    /// <summary>Cancels a backfill that has not finished.</summary>
    public async Task<ScheduleBackfill> CancelBackfillAsync(string namespaceId, string scheduleId, string backfillId, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId, scheduleId, "/backfills/" + Requests.Uuid(backfillId, "a backfill id") + "/cancel");
        var answer = await transport.SendAsync(
            new HttpCall("cancel backfill", HttpMethod.Post, path, "/namespaces/{namespaceId}/schedules/{scheduleId}/backfills/{backfillId}/cancel", [], null, SagatClient.KeyOf(options?.IdempotencyKey), SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ToBackfill(answer.Body);
    }

    /// <summary>At most the latest 100 backfills of the schedule.</summary>
    public async Task<IReadOnlyList<ScheduleBackfill>> ListBackfillsAsync(string namespaceId, string scheduleId, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId, scheduleId, "/backfills");
        var answer = await transport.SendAsync(
            new HttpCall("list backfills", HttpMethod.Get, path, "/namespaces/{namespaceId}/schedules/{scheduleId}/backfills", [], null, null, SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return ListOf(ObjectOf(answer.Body, "a list of backfills"), "items", item => ToBackfill(item));
    }

    /// <summary>The decisions the schedule has made, newest first.</summary>
    public async Task<Page<ScheduleActionRecord>> ListActionsAsync(string namespaceId, string scheduleId, ScheduleActionListOptions? filter = null, CancellationToken cancellationToken = default)
    {
        var path = PathOf(namespaceId, scheduleId, "/actions");
        var query = new QueryBuilder().Add("cursor", filter?.Cursor).Add("limit", filter?.Limit).Add("outcome", filter?.Outcome).Build();
        var answer = await transport.SendAsync(
            new HttpCall("list schedule actions", HttpMethod.Get, path, "/namespaces/{namespaceId}/schedules/{scheduleId}/actions", query, null, null, SagatClient.TraceOf(filter?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        return PageOf(answer.Body, "a page of schedule actions", item => ToActionRecord(item));
    }

    // ------------------------------------------------------------------------------------------ requests

    private static string PathOf(string namespaceId, string? scheduleId = null, string tail = "")
    {
        var path = "/namespaces/" + Requests.Uuid(namespaceId, "a namespace id") + "/schedules";
        return scheduleId is null ? path : path + "/" + Requests.Uuid(scheduleId, "a schedule id") + tail;
    }

    /// <summary>The write of a schedule on the wire: what was not given is left out, and the instants of the spec are <c>Z</c> instants.</summary>
    private byte[] Body(ScheduleWrite write, long? expectedRevision)
    {
        ArgumentNullException.ThrowIfNull(write);
        ArgumentNullException.ThrowIfNull(write.Spec);
        ArgumentNullException.ThrowIfNull(write.Action);
        return Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("name", write.Name);
            WriteSpec(writer, write.Spec);
            if (write.Policies is { } policies)
            {
                writer.WriteStartObject("policies");
                WriteString(writer, "overlap", policies.Overlap);
                WriteNumber(writer, "catchupWindowMs", policies.CatchupWindowMs);
                WriteString(writer, "catchupMode", policies.CatchupMode);
                if (policies.PauseOnFailure is { } pause)
                {
                    writer.WriteBoolean("pauseOnFailure", pause);
                }

                WriteNumber(writer, "bufferLimit", policies.BufferLimit);
                writer.WriteEndObject();
            }

            WriteAction(writer, write.Action);
            if (write.Paused is { } paused)
            {
                writer.WriteBoolean("paused", paused);
            }

            WriteString(writer, "note", write.Note);
            WriteNumber(writer, "remainingActions", write.RemainingActions);
            WriteNumber(writer, "expectedRevision", expectedRevision);
            writer.WriteEndObject();
        });
    }

    private void WriteAction(Utf8JsonWriter writer, ScheduleAction action)
    {
        writer.WriteStartObject("action");
        writer.WriteString("workflowId", action.WorkflowId);
        WriteString(writer, "versionPolicy", action.VersionPolicy);
        WriteString(writer, "versionId", action.VersionId);
        writer.WritePropertyName("input");
        if (action.Input is null)
        {
            writer.WriteStartObject();
            writer.WriteEndObject();
        }
        else
        {
            Wire.WritePayload(writer, action.Input, json);
        }

        WriteString(writer, "nominalTimeField", action.NominalTimeField);
        WriteNumber(writer, "runTimeoutMs", action.RunTimeoutMs);
        writer.WriteEndObject();
    }

    private static void WriteSpec(Utf8JsonWriter writer, ScheduleSpec spec)
    {
        writer.WriteStartObject("spec");
        WriteCalendars(writer, "calendars", spec.Calendars);
        if (spec.Cron is { } cron)
        {
            writer.WriteStartArray("cron");
            foreach (var line in cron)
            {
                writer.WriteStringValue(line);
            }

            writer.WriteEndArray();
        }

        if (spec.Intervals is { } intervals)
        {
            writer.WriteStartArray("intervals");
            foreach (var interval in intervals)
            {
                writer.WriteStartObject();
                writer.WriteNumber("everyMs", interval.EveryMs);
                WriteNumber(writer, "offsetMs", interval.OffsetMs);
                writer.WriteEndObject();
            }

            writer.WriteEndArray();
        }

        WriteCalendars(writer, "exclusions", spec.Exclusions);
        if (spec.StartAt is { } startAt)
        {
            writer.WriteString("startAt", startAt.ToString());
        }

        if (spec.EndAt is { } endAt)
        {
            writer.WriteString("endAt", endAt.ToString());
        }

        WriteString(writer, "timezone", spec.Timezone);
        WriteNumber(writer, "jitterMs", spec.JitterMs);
        if (spec.Dst is { } dst)
        {
            writer.WriteStartObject("dst");
            WriteString(writer, "gap", dst.Gap);
            WriteString(writer, "ambiguous", dst.Ambiguous);
            writer.WriteEndObject();
        }

        writer.WriteEndObject();
    }

    private static void WriteCalendars(Utf8JsonWriter writer, string name, IReadOnlyList<ScheduleCalendarSpec>? calendars)
    {
        if (calendars is null)
        {
            return;
        }

        writer.WriteStartArray(name);
        foreach (var calendar in calendars)
        {
            writer.WriteStartObject();
            WriteField(writer, "second", calendar.Second);
            WriteField(writer, "minute", calendar.Minute);
            WriteField(writer, "hour", calendar.Hour);
            WriteField(writer, "dayOfMonth", calendar.DayOfMonth);
            WriteField(writer, "month", calendar.Month);
            WriteField(writer, "year", calendar.Year);
            WriteField(writer, "dayOfWeek", calendar.DayOfWeek);
            WriteString(writer, "comment", calendar.Comment);
            writer.WriteEndObject();
        }

        writer.WriteEndArray();
    }

    private static void WriteField(Utf8JsonWriter writer, string name, IReadOnlyList<ScheduleRange>? ranges)
    {
        if (ranges is null)
        {
            return;
        }

        writer.WriteStartArray(name);
        foreach (var range in ranges)
        {
            writer.WriteStartObject();
            writer.WriteNumber("start", range.Start);
            WriteNumber(writer, "end", range.End);
            WriteNumber(writer, "step", range.Step);
            writer.WriteEndObject();
        }

        writer.WriteEndArray();
    }

    private static void WriteString(Utf8JsonWriter writer, string name, string? value)
    {
        if (value is not null)
        {
            writer.WriteString(name, value);
        }
    }

    private static void WriteNumber(Utf8JsonWriter writer, string name, long? value)
    {
        if (value is { } number)
        {
            writer.WriteNumber(name, number);
        }
    }

    // -------------------------------------------------------------------------------------------- answers

    private static Schedule ToSchedule(JsonElement? value)
    {
        var raw = ObjectOf(value, "a schedule");
        var action = raw.TryGetProperty("action", out var found) && found.ValueKind == JsonValueKind.Object ? found : default;
        return new Schedule
        {
            ScheduleId = Str(raw, "scheduleId"),
            Name = Str(raw, "name"),
            Origin = Str(raw, "origin"),
            State = Str(raw, "state"),
            Revision = Revision(raw, "revision", 1),
            Exhausted = Flag(raw, "exhausted"),
            Action = new ScheduleActionSummary(
                action.ValueKind == JsonValueKind.Object ? OptStr(action, "workflowId") : null,
                action.ValueKind == JsonValueKind.Object ? OptStr(action, "versionPolicy") : null,
                action.ValueKind == JsonValueKind.Object ? OptStr(action, "versionId") : null,
                action.ValueKind == JsonValueKind.Object ? OptStr(action, "inputDigest") : null,
                action.ValueKind == JsonValueKind.Object ? OptStr(action, "nominalTimeField") : null,
                action.ValueKind == JsonValueKind.Object ? OptNum(action, "runTimeoutMs") : null),
            Note = OptStr(raw, "note"),
            NoteBy = OptStr(raw, "noteBy"),
            RemainingActions = OptNum(raw, "remainingActions"),
            NextActionAt = OptDate(raw, "nextActionAt"),
            CreatedAt = Date(raw, "createdAt"),
            UpdatedAt = OptDate(raw, "updatedAt"),
            Raw = raw,
        };
    }

    private static ScheduleActionRecord ToActionRecord(JsonElement? value)
    {
        var raw = ObjectOf(value, "a schedule action");
        return new ScheduleActionRecord
        {
            NominalTime = Date(raw, "nominalTime"),
            ActualTime = OptDate(raw, "actualTime"),
            Kind = Str(raw, "kind"),
            Origin = OptStr(raw, "origin"),
            Revision = Revision(raw, "revision", 1),
            Outcome = Str(raw, "outcome"),
            Reason = OptStr(raw, "reason"),
            RunId = OptStr(raw, "runId"),
            BackfillId = OptStr(raw, "backfillId"),
            SummaryCount = OptNum(raw, "summaryCount"),
            SummaryLastNominalTime = OptDate(raw, "summaryLastNominalTime"),
            DecidedAt = Date(raw, "decidedAt"),
            Raw = raw,
        };
    }

    private static ScheduleBackfill ToBackfill(JsonElement? value)
    {
        var raw = ObjectOf(value, "a backfill");
        return new ScheduleBackfill
        {
            BackfillId = Str(raw, "backfillId"),
            Start = Date(raw, "start"),
            End = Date(raw, "end"),
            Cursor = OptDate(raw, "cursor"),
            State = Str(raw, "state"),
            Overlap = Str(raw, "overlap"),
            Counts = Counters(raw, "counts"),
            Raw = raw,
        };
    }
}
