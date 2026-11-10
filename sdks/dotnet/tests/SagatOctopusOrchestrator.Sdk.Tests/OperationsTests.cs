using System.Text.Json;
using System.Text.Json.Nodes;
using Microsoft.Extensions.DependencyInjection;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>
/// The client surface of feature 018 against a recording server: what each call sends, what it keeps across a retry, what it refuses before
/// sending, and how it reads an answer that is newer than the SDK. (The TypeScript suite is <c>sdks/typescript/test/operations.test.ts</c>.)
/// </summary>
public sealed class OperationsTests : IAsyncLifetime
{
    private const string Ns = "0190aaaa-0000-7000-8000-00000000aaaa";
    private const string Wf = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    private const string ScheduleId = "0190aaaa-0000-7000-8000-0000000000a1";
    private const string RunId = "0190aaaa-0000-7000-8000-000000000001";
    private const string BatchId = "0190aaaa-0000-7000-8000-0000000000b1";

    private FakeRestServer server = null!;
    private readonly List<IDisposable> clients = [];

    public async Task InitializeAsync() => server = await FakeRestServer.StartAsync();

    public async Task DisposeAsync()
    {
        foreach (var client in clients)
        {
            client.Dispose();
        }

        await server.DisposeAsync();
    }

    private static FakeAnswer Reply(int status, JsonNode? body = null) => new(status, body?.ToJsonString());

    private static JsonObject Run(Action<JsonObject>? change = null)
    {
        var run = new JsonObject
        {
            ["runId"] = RunId,
            ["workflowId"] = Wf,
            ["versionId"] = "0190aaaa-0000-7000-8000-000000000002",
            ["status"] = "RUNNING",
            ["version"] = 3,
            ["paused"] = false,
            ["startedBy"] = "alice",
            ["createdAt"] = "2026-10-10T10:00:00Z",
            ["updatedAt"] = "2026-10-10T10:00:01Z",
            ["finishedAt"] = null,
            ["deadline"] = null,
            ["traceId"] = "abcdefabcdefabcdefabcdefabcdefab",
            ["correlationId"] = null,
            ["depth"] = 0,
            ["parent"] = null,
        };
        change?.Invoke(run);
        return run;
    }

    private static JsonObject Schedule(Action<JsonObject>? change = null)
    {
        var schedule = new JsonObject
        {
            ["scheduleId"] = ScheduleId,
            ["name"] = "nightly",
            ["origin"] = "API",
            ["state"] = "ACTIVE",
            ["revision"] = 2,
            ["createdAt"] = "2026-10-10T10:00:00Z",
            ["action"] = new JsonObject { ["workflowId"] = Wf, ["versionPolicy"] = "ACTIVE", ["inputDigest"] = new string('a', 64) },
            ["note"] = null,
            ["remainingActions"] = null,
            ["nextActionAt"] = "2099-01-01T09:00:00Z",
            ["updatedAt"] = "2026-10-10T10:00:00Z",
        };
        change?.Invoke(schedule);
        return schedule;
    }

    private static JsonObject Action(Action<JsonObject>? change = null)
    {
        var action = new JsonObject
        {
            ["nominalTime"] = "2026-10-10T10:00:00Z",
            ["actualTime"] = "2026-10-10T10:00:01Z",
            ["kind"] = "TRIGGERED",
            ["origin"] = "MANUAL",
            ["revision"] = 2,
            ["outcome"] = "STARTED",
            ["reason"] = null,
            ["runId"] = RunId,
            ["backfillId"] = null,
            ["decidedAt"] = "2026-10-10T10:00:01Z",
        };
        change?.Invoke(action);
        return action;
    }

    private static JsonObject Unavailable() =>
        new() { ["code"] = "UNAVAILABLE", ["message"] = "busy", ["retryable"] = true, ["traceId"] = new string('e', 32) };

    private SagatClient NewClient()
    {
        var options = new SagatOptions { ApiUrl = server.BaseUrl, Namespace = "payments", Token = "tok-1" };
        options.Retry.InitialDelay = TimeSpan.FromMilliseconds(5);
        options.Retry.MaxDelay = TimeSpan.FromMilliseconds(20);
        var client = new SagatClient(options);
        clients.Add(client);
        return client;
    }

    private static void AssertBody(string expected, SeenRequest request) =>
        Assert.True(JsonNode.DeepEquals(JsonNode.Parse(expected), request.Body), $"expected {expected} but the request carried {request.BodyText}");

    private static async Task<SagatException> RefusedAsync(Func<Task> call, SagatErrorCode code = SagatErrorCode.ValidationFailed)
    {
        var error = await Assert.ThrowsAsync<SagatException>(call);
        Assert.Equal(code, error.Code);
        return error;
    }

    // ---------------------------------------------------------------------------------------------- start options

    [Fact]
    public async Task AStartSendsTheBusinessIdentifierItsPoliciesAndTheDelayOnceAndSaysWhetherItCreatedTheRun()
    {
        server.Handler = _ => Reply(202, Run(r => { r["businessId"] = "order-1"; r["startDelayedUntil"] = "2026-10-10T10:00:05Z"; }));
        var started = await NewClient().StartAsync(
            Wf,
            new { a = 1 },
            new StartOptions
            {
                IdempotencyKey = "key-12345678",
                BusinessId = "Order/1 ",
                BusinessIdConflictPolicy = BusinessIdConflictPolicies.UseExisting,
                BusinessIdReusePolicy = BusinessIdReusePolicies.RejectDuplicate,
                StartDelay = TimeSpan.FromMilliseconds(5000.9),
            });
        var request = Assert.Single(server.Requests);
        Assert.Equal(
            $$$"""{"workflowId":"{{{Wf}}}","input":{"a":1},"businessId":"Order/1 ","businessIdConflictPolicy":"USE_EXISTING","businessIdReusePolicy":"REJECT_DUPLICATE","startDelayMs":5000}""",
            request.BodyText);
        Assert.True(started.Started);
        Assert.Equal("order-1", started.BusinessId);
        Assert.Equal(new DateTimeOffset(2026, 10, 10, 10, 0, 5, TimeSpan.Zero), started.StartDelayedUntil);

        server.Handler = _ => Reply(202, Run());
        var plain = await NewClient().StartAsync(Wf, new { }, new StartOptions { IdempotencyKey = "key-12345678" });
        Assert.Null(plain.BusinessId);
        Assert.Null(plain.StartDelayedUntil);
    }

    // ---------------------------------------------------------------------------------------------- where a run came from

    [Fact]
    public async Task ARunSaysWhichScheduleStartedItByWhichKindOfActionForWhichNominalTimeAndUnderWhichRevisionAndARunNoScheduleStartedHasNone()
    {
        server.Handler = _ => Reply(200, Run(r =>
        {
            r["startedBy"] = $"system:schedule:{ScheduleId}";
            r["schedule"] = new JsonObject { ["scheduleId"] = ScheduleId, ["nominalTime"] = "2026-11-04T02:00:00.123456Z", ["kind"] = "BACKFILL", ["revision"] = 3 };
        }));
        var scheduled = await NewClient().GetAsync(RunId);
        var schedule = Assert.IsType<RunSchedule>(scheduled.Schedule);
        Assert.Equal(ScheduleId, schedule.ScheduleId);
        Assert.Equal("BACKFILL", schedule.Kind);
        Assert.Equal(3, schedule.Revision);
        // the instant is parsed, not carried: a DateTimeOffset holds the microseconds of the wire (ticks are 100 ns)
        Assert.Equal(new DateTimeOffset(2026, 11, 4, 2, 0, 0, TimeSpan.Zero).AddTicks(1_234_560), schedule.NominalTime);
        Assert.Equal(TimeSpan.Zero, schedule.NominalTime.Offset);
        Assert.Equal("2026-11-04T02:00:00.123456Z", scheduled.Raw.GetProperty("schedule").GetProperty("nominalTime").GetString());
        Assert.Null(scheduled.ResetFrom);

        // an offset other than Z is the same instant in UTC
        server.Handler = _ => Reply(200, Run(r => r["schedule"] = new JsonObject { ["scheduleId"] = ScheduleId, ["nominalTime"] = "2026-11-04T04:00:00+02:00", ["kind"] = "SCHEDULED", ["revision"] = 1 }));
        Assert.Equal(new DateTimeOffset(2026, 11, 4, 2, 0, 0, TimeSpan.Zero), (await NewClient().GetAsync(RunId)).Schedule!.NominalTime);

        // a kind this SDK does not know is kept as the server sent it, and a plain run has neither field
        server.Handler = _ => Reply(200, Run(r => r["schedule"] = new JsonObject { ["scheduleId"] = ScheduleId, ["nominalTime"] = "2026-11-04T02:00:00Z", ["kind"] = "SOMETHING_NEW", ["revision"] = 1 }));
        Assert.Equal("SOMETHING_NEW", (await NewClient().GetAsync(RunId)).Schedule!.Kind);
        server.Handler = _ => Reply(200, Run());
        var plain = await NewClient().GetAsync(RunId);
        Assert.Null(plain.Schedule);
        Assert.Null(plain.ResetFrom);
        // a JSON null is an absent field
        server.Handler = _ => Reply(200, Run(r => { r["schedule"] = null; r["resetFrom"] = null; }));
        var nulls = await NewClient().GetAsync(RunId);
        Assert.Null(nulls.Schedule);
        Assert.Null(nulls.ResetFrom);
    }

    [Fact]
    public async Task ARunAResetMadeSaysTheRunItCameFromAndTheJournalSequenceItKeeps()
    {
        const string Source = "0190aaaa-0000-7000-8000-0000000000aa";
        server.Handler = _ => Reply(200, Run(r =>
        {
            r["rerunOf"] = Source;
            r["resetFrom"] = new JsonObject { ["runId"] = Source, ["sequence"] = 4 };
        }));
        var reset = await NewClient().GetAsync(RunId);
        Assert.Equal(new RunResetFrom(Source, 4), reset.ResetFrom);
        Assert.Equal(Source, reset.RerunOf);
        Assert.Null(reset.Schedule);

        // started again, not reset: RerunOf and nothing else
        server.Handler = _ => Reply(200, Run(r => r["rerunOf"] = Source));
        var again = await NewClient().GetAsync(RunId);
        Assert.Equal(Source, again.RerunOf);
        Assert.Null(again.ResetFrom);
    }

    [Fact]
    public async Task AnAnswerWhoseScheduleOrResetIsNotWhatTheContractSaysIsAProtocolErrorNotARunWithHalfOfIt()
    {
        var broken = new Action<JsonObject>[]
        {
            r => r["schedule"] = new JsonObject { ["scheduleId"] = ScheduleId, ["nominalTime"] = "yesterday", ["kind"] = "SCHEDULED", ["revision"] = 1 },
            r => r["schedule"] = new JsonObject { ["scheduleId"] = ScheduleId, ["kind"] = "SCHEDULED", ["revision"] = 1 },
            r => r["schedule"] = new JsonObject { ["scheduleId"] = ScheduleId, ["nominalTime"] = "2026-11-04T02:00:00Z", ["kind"] = "SCHEDULED", ["revision"] = 0 },
            r => r["schedule"] = new JsonObject { ["scheduleId"] = ScheduleId, ["nominalTime"] = "2026-11-04T02:00:00Z", ["kind"] = "SCHEDULED" },
            r => r["schedule"] = new JsonObject { ["nominalTime"] = "2026-11-04T02:00:00Z", ["kind"] = "SCHEDULED", ["revision"] = 1 },
            r => r["schedule"] = new JsonObject { ["scheduleId"] = ScheduleId, ["nominalTime"] = "2026-11-04T02:00:00Z", ["revision"] = 1 },
            r => r["schedule"] = "nightly",
            r => r["resetFrom"] = new JsonObject { ["runId"] = RunId },
            r => r["resetFrom"] = new JsonObject { ["runId"] = RunId, ["sequence"] = 0 },
            r => r["resetFrom"] = new JsonObject { ["sequence"] = 4 },
            r => r["resetFrom"] = "the-run",
        };
        foreach (var change in broken)
        {
            server.Handler = _ => Reply(200, Run(change));
            await RefusedAsync(() => NewClient().GetAsync(RunId), SagatErrorCode.Protocol);
        }
    }

    [Fact]
    public async Task TheTwoHundredOfUseExistingIsARunThatWasNotStartedHereAndAStartWithoutTheNewOptionsSendsNoneOfThem()
    {
        server.Handler = _ => Reply(200, Run());
        var existing = await NewClient().StartAsync(
            Wf, new { }, new StartOptions { IdempotencyKey = "key-12345678", BusinessId = "o-1", BusinessIdConflictPolicy = BusinessIdConflictPolicies.UseExisting });
        Assert.False(existing.Started);

        server.Handler = _ => Reply(202, Run());
        await NewClient().StartAsync(Wf, new { }, new StartOptions { IdempotencyKey = "key-12345678" });
        Assert.Equal($$$"""{"workflowId":"{{{Wf}}}","input":{}}""", server.Requests[^1].BodyText);
    }

    [Fact]
    public async Task AStartDelayIsTruncatedTowardZeroToMillisecondsOnceAndARetrySendsThatSameBodyAndKey()
    {
        // A TimeSpan is always a finite number, so there is no value the SDK refuses: the range (1 ms to 366 days) is the server's to judge.
        server.Handler = _ => Reply(202, Run());
        var client = NewClient();
        await client.StartAsync(Wf, new { }, new StartOptions { StartDelay = TimeSpan.FromTicks(9_999) });
        await client.StartAsync(Wf, new { }, new StartOptions { StartDelay = TimeSpan.FromTicks(-15_000) });
        await client.StartAsync(Wf, new { }, new StartOptions { StartDelay = TimeSpan.FromDays(366) + TimeSpan.FromMilliseconds(1) });
        Assert.Equal(0, (long)server.Requests[0].Body!["startDelayMs"]!);
        Assert.Equal(-1, (long)server.Requests[1].Body!["startDelayMs"]!);
        Assert.Equal(31_622_400_001, (long)server.Requests[2].Body!["startDelayMs"]!);

        var attempts = 0;
        server.Handler = _ => ++attempts < 2 ? Reply(503, Unavailable()) : Reply(202, Run());
        var before = server.Requests.Count;
        await client.StartAsync(Wf, new { }, new StartOptions { BusinessId = "o-2", StartDelay = TimeSpan.FromMilliseconds(1500) });
        var retried = server.Requests.Skip(before).ToList();
        Assert.Equal(2, retried.Count);
        Assert.Equal(retried[0].BodyText, retried[1].BodyText);
        Assert.Matches("^[0-9a-f-]{36}$", retried[0].Header("idempotency-key"));
        Assert.Equal(retried[0].Header("idempotency-key"), retried[1].Header("idempotency-key"));
        Assert.Equal(1500, (long)retried[1].Body!["startDelayMs"]!);
    }

    // ---------------------------------------------------------------------------------------------- schedules

    private static ScheduleWrite NightlyWrite(ScheduleSpec? spec = null) => new()
    {
        Name = "nightly",
        Spec = spec ?? new ScheduleSpec { Cron = ["0 9 * * *"] },
        Action = new ScheduleAction { WorkflowId = Wf, Input = new { } },
    };

    [Fact]
    public async Task CreateSendsTheScheduleWithItsInstantsInUtcAndOneKeyAndReadsTheScheduleItGetsBack()
    {
        server.Handler = _ => Reply(201, Schedule(s => s["futureField"] = 1));
        var created = await NewClient().Schedules.CreateAsync(
            Ns,
            new ScheduleWrite
            {
                Name = "nightly",
                Spec = new ScheduleSpec
                {
                    Calendars = [new ScheduleCalendarSpec { Hour = [new ScheduleRange(9)], Year = [new ScheduleRange(2099)] }],
                    StartAt = Instant.Parse("2099-01-01T01:00:00+01:00"),
                    Timezone = "UTC",
                },
                Policies = new SchedulePolicies { Overlap = OverlapPolicies.Skip },
                Action = new ScheduleAction { WorkflowId = Wf, Input = new { n = 1 } },
            },
            new RequestOptions { IdempotencyKey = "sched-key-1" });
        var request = Assert.Single(server.Requests);
        Assert.Equal("POST", request.Method);
        Assert.Equal($"/v1/namespaces/{Ns}/schedules", request.PathAndQuery);
        Assert.Equal("sched-key-1", request.Header("idempotency-key"));
        AssertBody(
            $$$"""
            {"name":"nightly","spec":{"calendars":[{"hour":[{"start":9}],"year":[{"start":2099}]}],"startAt":"2099-01-01T00:00:00.000Z","timezone":"UTC"},
             "policies":{"overlap":"SKIP"},"action":{"workflowId":"{{{Wf}}}","input":{"n":1} } }
            """,
            request);
        Assert.Equal(2, created.Revision);
        Assert.Equal(new DateTimeOffset(2099, 1, 1, 9, 0, 0, TimeSpan.Zero), created.NextActionAt);
        Assert.Equal(new string('a', 64), created.Action.InputDigest);
        Assert.Equal(1, created.Raw.GetProperty("futureField").GetInt32());
    }

    [Fact]
    public async Task DescribeAsksForTheCountsItWasGivenAndKeepsAnOutcomeItDoesNotKnowAsTheStringItCameAs()
    {
        server.Handler = _ => Reply(200, Schedule(s =>
        {
            s["nextActions"] = new JsonArray(new JsonObject { ["nominalTime"] = "2099-01-01T09:00:00Z", ["actualTime"] = "2099-01-01T09:00:00Z" });
            s["recentActions"] = new JsonArray(Action(), Action(a => a["outcome"] = "SOMETHING_NEWER"));
            s["openRuns"] = new JsonArray(RunId);
            s["bufferedCount"] = 1;
            s["counters"] = new JsonObject { ["STARTED"] = 1, ["SOMETHING_NEWER"] = 1 };
        }));
        var described = await NewClient().Schedules.GetAsync(Ns, ScheduleId, new ScheduleDescribeOptions { NextCount = 3, RecentCount = 5 });
        var request = Assert.Single(server.Requests);
        Assert.Equal("GET", request.Method);
        Assert.Equal($"/v1/namespaces/{Ns}/schedules/{ScheduleId}?nextCount=3&recentCount=5", request.PathAndQuery);
        Assert.Equal(DateTimeOffset.Parse("2099-01-01T09:00:00Z", System.Globalization.CultureInfo.InvariantCulture), described.NextActions[0].NominalTime);
        Assert.Equal("SOMETHING_NEWER", described.RecentActions[1].Outcome);
        Assert.Equal([RunId], described.OpenRuns);
        Assert.Equal(new Dictionary<string, long> { ["STARTED"] = 1, ["SOMETHING_NEWER"] = 1 }, described.Counters);
        Assert.Equal(1, described.BufferedCount);
        Assert.Equal(2, described.Revision);
    }

    [Fact]
    public async Task ListSendsTheFiltersItWasGivenAndListAllFollowsTheOpaqueCursorsToTheEnd()
    {
        var page = 0;
        server.Handler = _ => Reply(200, ++page == 1
            ? new JsonObject { ["items"] = new JsonArray(Schedule(s => s["name"] = "a-1")), ["nextCursor"] = "c1" }
            : new JsonObject { ["items"] = new JsonArray(Schedule(s => s["name"] = "a-2")), ["nextCursor"] = null });
        var names = new List<string>();
        await foreach (var item in NewClient().Schedules.ListAllAsync(Ns, new ScheduleListOptions { State = "PAUSED", Origin = "API", Name = "a-", Limit = 1 }))
        {
            names.Add(item.Name);
        }

        Assert.Equal(["a-1", "a-2"], names);
        Assert.Equal($"/v1/namespaces/{Ns}/schedules?limit=1&state=PAUSED&origin=API&name=a-", server.Requests[0].PathAndQuery);
        Assert.Equal($"/v1/namespaces/{Ns}/schedules?cursor=c1&limit=1&state=PAUSED&origin=API&name=a-", server.Requests[1].PathAndQuery);
    }

    [Fact]
    public async Task UpdateCarriesTheRevisionItExpectsInTheBodyDeleteInTheQueryAndBothCarryAKey()
    {
        server.Handler = _ => Reply(200, Schedule(s => s["revision"] = 3));
        var write = NightlyWrite();
        var updated = await NewClient().Schedules.UpdateAsync(Ns, ScheduleId, write, 2, new RequestOptions { IdempotencyKey = "upd-key-0001" });
        Assert.Equal(3, updated.Revision);
        var update = Assert.Single(server.Requests);
        Assert.Equal("PUT", update.Method);
        Assert.Equal($"/v1/namespaces/{Ns}/schedules/{ScheduleId}", update.PathAndQuery);
        Assert.Equal("upd-key-0001", update.Header("idempotency-key"));
        AssertBody($$$"""{"name":"nightly","spec":{"cron":["0 9 * * *"]},"action":{"workflowId":"{{{Wf}}}","input":{}},"expectedRevision":2}""", update);

        server.Handler = _ => Reply(204);
        await NewClient().Schedules.DeleteAsync(Ns, ScheduleId, 3, new RequestOptions { IdempotencyKey = "del-key-0001" });
        var delete = server.Requests[^1];
        Assert.Equal("DELETE", delete.Method);
        Assert.Equal($"/v1/namespaces/{Ns}/schedules/{ScheduleId}?expectedRevision=3", delete.PathAndQuery);
        Assert.Equal("del-key-0001", delete.Header("idempotency-key"));
    }

    [Fact]
    public async Task AStaleRevisionIsAConflictWithItsReasonAndARevisionThatCannotBeOneIsRefusedBeforeItIsSent()
    {
        server.Handler = _ => Reply(409, new JsonObject
        {
            ["code"] = "CONFLICT",
            ["message"] = "stale",
            ["retryable"] = false,
            ["traceId"] = new string('e', 32),
            ["details"] = new JsonObject { ["reason"] = "REVISION_CONFLICT", ["currentRevision"] = 4 },
        });
        var client = NewClient();
        var conflict = await RefusedAsync(() => client.Schedules.UpdateAsync(Ns, ScheduleId, NightlyWrite(), 2), SagatErrorCode.Conflict);
        Assert.Equal("REVISION_CONFLICT", conflict.Reason);
        Assert.Single(server.Requests);

        var before = server.Requests.Count;
        await RefusedAsync(() => client.Schedules.DeleteAsync(Ns, ScheduleId, 0));
        await RefusedAsync(() => client.Schedules.GetAsync(Ns, "../x"));
        await RefusedAsync(() => client.Schedules.GetAsync("payments", ScheduleId));
        Assert.Equal(before, server.Requests.Count);
    }

    [Fact]
    public async Task PauseAndResumeSendTheNoteTriggerSendsTheOverlapForThisActionOnlyAndATriggerRetryKeepsItsKey()
    {
        server.Handler = _ => Reply(200, Schedule(s => { s["state"] = "PAUSED"; s["note"] = "maintenance"; }));
        var client = NewClient();
        var paused = await client.Schedules.PauseAsync(Ns, ScheduleId, "maintenance", new RequestOptions { IdempotencyKey = "pause-key-01" });
        Assert.Equal("PAUSED", paused.State);
        var pause = Assert.Single(server.Requests);
        Assert.Equal($"/v1/namespaces/{Ns}/schedules/{ScheduleId}/pause", pause.PathAndQuery);
        Assert.Equal("""{"note":"maintenance"}""", pause.BodyText);
        Assert.Equal("pause-key-01", pause.Header("idempotency-key"));

        server.Handler = _ => Reply(200, Schedule());
        await client.Schedules.ResumeAsync(Ns, ScheduleId, "done", new RequestOptions { IdempotencyKey = "resume-key-1" });
        Assert.Equal($"/v1/namespaces/{Ns}/schedules/{ScheduleId}/resume", server.Requests[^1].PathAndQuery);
        var before = server.Requests.Count;
        await RefusedAsync(() => client.Schedules.PauseAsync(Ns, ScheduleId, string.Empty));
        Assert.Equal(before, server.Requests.Count);

        var attempts = 0;
        server.Handler = _ => ++attempts < 2 ? Reply(503, Unavailable()) : Reply(202, Action(a => a["outcome"] = "SKIPPED_OVERLAP"));
        var triggered = await client.Schedules.TriggerAsync(Ns, ScheduleId, new ScheduleTriggerOptions { Overlap = OverlapPolicies.Skip });
        Assert.Equal("SKIPPED_OVERLAP", triggered.Outcome);
        var tried = server.Requests.Skip(before).ToList();
        Assert.Equal(2, tried.Count);
        Assert.Equal(tried[0].Header("idempotency-key"), tried[1].Header("idempotency-key"));
        Assert.Matches("^[0-9a-f-]{36}$", tried[0].Header("idempotency-key"));
        Assert.Equal("""{"overlap":"SKIP"}""", tried[1].BodyText);

        server.Handler = _ => Reply(202, Action());
        await client.Schedules.TriggerAsync(Ns, ScheduleId);
        Assert.Equal("{}", server.Requests[^1].BodyText);
    }

    [Fact]
    public async Task ABackfillSendsItsRangeAsUtcInstantsACancelNamesTheBackfillAndTheHistoryCanBeFilteredByOutcome()
    {
        server.Handler = _ => Reply(202, new JsonObject
        {
            ["backfillId"] = BatchId,
            ["start"] = "2026-10-10T08:00:00Z",
            ["end"] = "2026-10-10T10:00:00Z",
            ["cursor"] = null,
            ["state"] = "PENDING",
            ["overlap"] = "ALLOW_ALL",
            ["counts"] = new JsonObject { ["STARTED"] = 2 },
        });
        var client = NewClient();
        // 13:30 at +05:30 is 08:00 UTC: the offset of the caller (or of the machine) never reaches the wire.
        var backfill = await client.Schedules.BackfillAsync(
            Ns, ScheduleId, new DateTimeOffset(2026, 10, 10, 13, 30, 0, TimeSpan.FromHours(5.5)), Instant.Parse("2026-10-10T10:00:00+00:00"),
            new ScheduleBackfillOptions { Overlap = OverlapPolicies.AllowAll, IdempotencyKey = "back-key-001" });
        var request = Assert.Single(server.Requests);
        Assert.Equal($"/v1/namespaces/{Ns}/schedules/{ScheduleId}/backfills", request.PathAndQuery);
        AssertBody("""{"start":"2026-10-10T08:00:00.000Z","end":"2026-10-10T10:00:00.000Z","overlap":"ALLOW_ALL"}""", request);
        Assert.Null(backfill.Cursor);
        Assert.Equal(new Dictionary<string, long> { ["STARTED"] = 2 }, backfill.Counts);
        var before = server.Requests.Count;
        await RefusedAsync(() => client.Schedules.BackfillAsync(Ns, ScheduleId, "not a time", "2026-10-10T10:00:00Z"));
        await RefusedAsync(() => client.Schedules.BackfillAsync(Ns, ScheduleId, "2026-10-10T10:00:00", "2026-10-10T11:00:00Z"));
        Assert.Equal(before, server.Requests.Count);

        server.Handler = _ => Reply(200, new JsonObject
        {
            ["backfillId"] = BatchId,
            ["start"] = "2026-10-10T08:00:00Z",
            ["end"] = "2026-10-10T10:00:00Z",
            ["state"] = "CANCELLED",
            ["overlap"] = "ALLOW_ALL",
            ["counts"] = new JsonObject(),
        });
        var cancelled = await client.Schedules.CancelBackfillAsync(Ns, ScheduleId, BatchId);
        Assert.Equal("CANCELLED", cancelled.State);
        Assert.Equal($"/v1/namespaces/{Ns}/schedules/{ScheduleId}/backfills/{BatchId}/cancel", server.Requests[^1].PathAndQuery);

        server.Handler = _ => Reply(200, new JsonObject { ["items"] = new JsonArray(Action(a => a["outcome"] = "ALREADY_ACTED")), ["nextCursor"] = null });
        var actions = await client.Schedules.ListActionsAsync(Ns, ScheduleId, new ScheduleActionListOptions { Outcome = "ALREADY_ACTED", Limit = 10 });
        Assert.Equal($"/v1/namespaces/{Ns}/schedules/{ScheduleId}/actions?limit=10&outcome=ALREADY_ACTED", server.Requests[^1].PathAndQuery);
        Assert.Equal("ALREADY_ACTED", actions.Items[0].Outcome);
    }

    // ---------------------------------------------------------------------------------------------- business ids

    [Fact]
    public async Task ABusinessIdentifierIsPercentEncodedAsGivenAndOneThatCannotBeAPathSegmentIsRefusedLocally()
    {
        server.Handler = _ => Reply(200, new JsonObject
        {
            ["businessId"] = "Ord/1#x?y",
            ["executionId"] = RunId,
            ["currentRunId"] = RunId,
            ["open"] = false,
            ["lastStatus"] = "SUCCEEDED",
            ["workflowId"] = Wf,
            ["boundAt"] = "2026-10-10T10:00:00Z",
            ["reusableAfter"] = "2026-11-09T10:00:00Z",
        });
        var client = NewClient();
        var binding = await client.BusinessIds.GetAsync(Ns, "Ord/1#x?y");
        Assert.Equal($"/v1/namespaces/{Ns}/business-ids/Ord%2F1%23x%3Fy", Assert.Single(server.Requests).PathAndQuery);
        Assert.False(binding.Open);
        Assert.Equal(new DateTimeOffset(2026, 11, 9, 10, 0, 0, TimeSpan.Zero), binding.ReusableAfter);

        var before = server.Requests.Count;
        foreach (var bad in new[] { string.Empty, " padded", "with space", new string('x', 201), "ünï", "tab\there" })
        {
            await RefusedAsync(() => client.BusinessIds.GetAsync(Ns, bad));
        }

        Assert.Equal(before, server.Requests.Count);

        // No trimming and no case folding: the longest allowed identifier and the case it came in go out as they are.
        await client.BusinessIds.GetAsync(Ns, new string('X', 200));
        Assert.Equal($"/v1/namespaces/{Ns}/business-ids/{new string('X', 200)}", server.Requests[^1].PathAndQuery);
    }

    [Theory]
    [InlineData(".")]
    [InlineData("..")]
    public async Task ABusinessIdentifierThatAUrlWouldResolveToAnotherRouteNeverReachesOne(string businessId)
    {
        var client = NewClient();
        await RefusedAsync(() => client.BusinessIds.GetAsync(Ns, businessId));
        Assert.Empty(server.Requests);
    }

    // ---------------------------------------------------------------------------------------------- batch

    [Fact]
    public async Task ABatchIsPreviewedCreatedWithOneKeyReadCancelledAndItsItemsFilteredByState()
    {
        var job = new Func<Action<JsonObject>?, JsonObject>(change =>
        {
            var value = new JsonObject
            {
                ["batchId"] = BatchId,
                ["action"] = "CANCEL",
                ["state"] = "RUNNING",
                ["createdBy"] = "alice",
                ["createdAt"] = "2026-10-10T10:00:00Z",
                ["counts"] = new JsonObject { ["selected"] = 2, ["succeeded"] = 1, ["skipped"] = 0, ["failed"] = 0, ["cancelled"] = 0, ["deferred"] = 1 },
            };
            change?.Invoke(value);
            return value;
        });
        var client = NewClient();
        server.Handler = _ => Reply(200, new JsonObject { ["matched"] = 2, ["capped"] = false, ["sample"] = new JsonArray(RunId) });
        var preview = await client.Batch.PreviewAsync(Ns, BatchSelector.ByRunIds([RunId]), 50);
        var previewRequest = Assert.Single(server.Requests);
        Assert.Equal($"/v1/namespaces/{Ns}/batch-operations/preview", previewRequest.PathAndQuery);
        AssertBody($$$"""{"selector":{"runIds":["{{{RunId}}}"]},"maxItems":50}""", previewRequest);
        Assert.Equal((2, false), (preview.Matched, preview.Capped));
        Assert.Equal([RunId], preview.Sample);

        server.Handler = _ => Reply(202, job(null));
        var created = await client.Batch.CreateAsync(
            Ns,
            new BatchRequest
            {
                Selector = BatchSelector.ByFilter(new BatchFilter { Status = ["RUNNING"], CreatedAfter = Instant.Parse("2026-10-10T02:00:00+02:00") }),
                Action = "CANCEL",
                Reason = "clean up the day",
                MaxItems = 10,
            },
            new RequestOptions { IdempotencyKey = "batch-key-01" });
        var createRequest = server.Requests[^1];
        AssertBody(
            """{"selector":{"filter":{"status":["RUNNING"],"createdAfter":"2026-10-10T00:00:00.000Z"}},"action":"CANCEL","reason":"clean up the day","maxItems":10}""",
            createRequest);
        Assert.Equal("batch-key-01", createRequest.Header("idempotency-key"));
        Assert.Equal(new BatchCounts(2, 1, 0, 0, 0, 1), created.Counts);
        Assert.False(created.HasPayload);

        server.Handler = _ => Reply(202, job(j => j["state"] = "CANCELLING"));
        var cancelling = await client.Batch.CancelAsync(Ns, BatchId, "stop the clean up", new RequestOptions { IdempotencyKey = "batch-cancel-1" });
        var cancelRequest = server.Requests[^1];
        Assert.Equal($"/v1/namespaces/{Ns}/batch-operations/{BatchId}/cancel", cancelRequest.PathAndQuery);
        Assert.Equal("""{"reason":"stop the clean up"}""", cancelRequest.BodyText);
        Assert.Equal("CANCELLING", cancelling.State);

        server.Handler = _ => Reply(200, new JsonObject
        {
            ["items"] = new JsonArray(
                new JsonObject { ["ordinal"] = 0, ["runId"] = RunId, ["state"] = "SKIPPED", ["reason"] = "ALREADY_TERMINAL" },
                new JsonObject { ["ordinal"] = 1, ["runId"] = RunId, ["state"] = "NEWER_STATE" }),
            ["nextCursor"] = "n",
        });
        var items = await client.Batch.ListItemsAsync(Ns, BatchId, new BatchListOptions { State = "SKIPPED", Limit = 2 });
        Assert.Equal($"/v1/namespaces/{Ns}/batch-operations/{BatchId}/items?limit=2&state=SKIPPED", server.Requests[^1].PathAndQuery);
        Assert.Equal("ALREADY_TERMINAL", items.Items[0].Reason);
        Assert.Equal("NEWER_STATE", items.Items[1].State);
        Assert.Equal("n", items.NextCursor);
    }

    // ---------------------------------------------------------------------------------------------- with start

    [Fact]
    public async Task SignalWithStartNamesTheWorkflowByIdCarriesTheSignalAndOneKeyAndSaysWhatTheSignalWoke()
    {
        server.Handler = _ => Reply(202, new JsonObject { ["runId"] = RunId, ["started"] = true, ["deliveredTo"] = new JsonArray("wait") });
        var result = await NewClient().SignalWithStartAsync(
            Wf, "order-9", new { a = 1 }, "go", new { b = 2 },
            new WithStartOptions { IdempotencyKey = "sws-key-0001", BusinessIdReusePolicy = BusinessIdReusePolicies.AllowDuplicateFailedOnly });
        var request = Assert.Single(server.Requests);
        Assert.Equal("/v1/runs/signal-with-start", request.PathAndQuery);
        AssertBody(
            $$$"""{"workflowId":"{{{Wf}}}","businessId":"order-9","input":{"a":1},"signal":{"name":"go","payload":{"b":2}},"businessIdReusePolicy":"ALLOW_DUPLICATE_FAILED_ONLY"}""",
            request);
        Assert.Equal("sws-key-0001", request.Header("idempotency-key"));
        Assert.Equal((RunId, true), (result.RunId, result.Started));
        Assert.Equal(["wait"], result.DeliveredTo);
    }

    [Fact]
    public async Task UpdateWithStartRequiresTheCallersKeyKeepsItAcrossARetryAndReadsARejectionAsAnAnswer()
    {
        var receipt = new Func<JsonObject>(() => new JsonObject
        {
            ["runId"] = RunId,
            ["operation"] = "set",
            ["updateKey"] = "u-1",
            ["status"] = "APPLIED",
            ["result"] = 7,
            ["beforeStateRevision"] = 0,
            ["afterStateRevision"] = 1,
            ["runVersion"] = 4,
            ["replayed"] = true,
        });
        var client = NewClient();
        server.Handler = _ => Reply(200, new JsonObject { ["runId"] = RunId, ["started"] = true, ["update"] = new JsonObject { ["outcome"] = "APPLIED", ["receipt"] = receipt() } });
        var applied = await client.UpdateWithStartAsync(
            Wf, "order-9", new { }, "set", "u-1", new { value = 7 },
            new UpdateWithStartOptions
            {
                IdempotencyKey = "uws-key-0001",
                BusinessIdConflictPolicy = BusinessIdConflictPolicies.Fail,
                StartDelay = TimeSpan.FromSeconds(2),
                ExpectedStateRevision = 0,
            });
        var request = Assert.Single(server.Requests);
        Assert.Equal("/v1/runs/update-with-start", request.PathAndQuery);
        AssertBody(
            $$$"""
            {"workflowId":"{{{Wf}}}","businessId":"order-9","input":{},"update":{"name":"set","updateKey":"u-1","args":{"value":7},"expectedStateRevision":0},
             "businessIdConflictPolicy":"FAIL","startDelayMs":2000}
            """,
            request);
        Assert.Equal("uws-key-0001", request.Header("idempotency-key"));
        Assert.Equal("APPLIED", applied.Update.Outcome);
        Assert.Equal(1, applied.Update.Receipt!.AfterStateRevision);
        Assert.Equal(7, applied.Update.Receipt.Result.GetInt32());
        Assert.True(applied.Update.Receipt.Replayed, "a replay says so");

        server.Handler = _ => Reply(200, new JsonObject { ["runId"] = RunId, ["started"] = false, ["update"] = new JsonObject { ["outcome"] = "REJECTED", ["reason"] = "REVISION_CONFLICT" } });
        var rejected = await client.UpdateWithStartAsync(Wf, "order-9", new { }, "set", "u-2", new { }, new UpdateWithStartOptions { IdempotencyKey = "uws-key-0002" });
        Assert.False(rejected.Started);
        Assert.Equal("REJECTED", rejected.Update.Outcome);
        Assert.Equal("REVISION_CONFLICT", rejected.Update.Reason);
        Assert.Null(rejected.Update.Receipt);

        var before = server.Requests.Count;
        await RefusedAsync(() => client.UpdateWithStartAsync(Wf, "order-9", new { }, "set", string.Empty, new { }));
        await RefusedAsync(() => client.UpdateWithStartAsync(Wf, "order-9", new { }, "set", new string('k', 129), new { }));
        await RefusedAsync(() => client.UpdateWithStartAsync(Wf, "order-9", new { }, "set", "u-3", new { }, new UpdateWithStartOptions { ExpectedStateRevision = -1 }));
        await RefusedAsync(() => client.UpdateWithStartAsync(Wf, "order-9", new { }, "not a name", "u-3", new { }));
        Assert.Equal(before, server.Requests.Count);

        var attempts = 0;
        server.Handler = _ => ++attempts < 2
            ? Reply(503, Unavailable())
            : Reply(200, new JsonObject { ["runId"] = RunId, ["started"] = true, ["update"] = new JsonObject { ["outcome"] = "APPLIED", ["receipt"] = receipt() } });
        await client.UpdateWithStartAsync(Wf, "order-9", new { }, "set", "u-4", new { });
        var retried = server.Requests.Skip(before).ToList();
        Assert.Equal(2, retried.Count);
        Assert.Equal(retried[0].BodyText, retried[1].BodyText);
        Assert.Matches("^[0-9a-f-]{36}$", retried[0].Header("idempotency-key"));
        Assert.Equal(retried[0].Header("idempotency-key"), retried[1].Header("idempotency-key"));
    }

    // ---------------------------------------------------------------------------------------------- reset

    [Fact]
    public async Task TheResetCallsListThePointsPreviewThePlanResetWithAKeyAndReadTheRegister()
    {
        var client = NewClient();
        server.Handler = _ => Reply(200, new JsonObject
        {
            ["items"] = new JsonArray(
                new JsonObject { ["sequence"] = 1, ["kind"] = "RUN_STARTED", ["node"] = null, ["occurredAt"] = "2026-10-10T10:00:00Z" },
                new JsonObject { ["sequence"] = 4, ["kind"] = "SOMETHING_NEWER", ["node"] = "a", ["occurredAt"] = "2026-10-10T10:00:01Z" }),
            ["nextCursor"] = null,
        });
        var points = await client.ResetPointsAsync(RunId, new ResetPointListOptions { Limit = 5 });
        Assert.Equal($"/v1/runs/{RunId}/reset-points?limit=5", Assert.Single(server.Requests).PathAndQuery);
        Assert.Equal([(1L, "RUN_STARTED", null), (4L, "SOMETHING_NEWER", "a")], points.Items.Select(p => (p.Sequence, p.Kind, p.Node)).ToArray());

        server.Handler = _ => Reply(200, new JsonObject
        {
            ["sequence"] = 4,
            ["restored"] = new JsonArray(new JsonObject { ["node"] = "a", ["kind"] = "STEP" }),
            ["reexecuted"] = new JsonArray(new JsonObject { ["node"] = "b", ["completedInSource"] = true, ["markers"] = new JsonArray("irreversible") }),
            ["reapply"] = new JsonArray(),
            ["requiresAcknowledgement"] = true,
            ["prefixEvents"] = 6,
        });
        var plan = await client.PreviewResetAsync(RunId, 4, ResetReapplyPolicies.None);
        Assert.Equal("""{"sequence":4,"reapply":"NONE"}""", server.Requests[^1].BodyText);
        Assert.True(plan.RequiresAcknowledgement);
        var rerun = Assert.Single(plan.Reexecuted);
        Assert.Equal(("b", true), (rerun.Node, rerun.CompletedInSource));
        Assert.Equal(["irreversible"], rerun.Markers);
        Assert.Equal(("a", "STEP"), (plan.Restored[0].Node, plan.Restored[0].Kind));
        var before = server.Requests.Count;
        await RefusedAsync(() => client.PreviewResetAsync(RunId, 0));
        Assert.Equal(before, server.Requests.Count);

        server.Handler = _ => Reply(202, new JsonObject
        {
            ["resetId"] = BatchId,
            ["sourceRunId"] = RunId,
            ["sequence"] = 4,
            ["state"] = "CREATED",
            ["newRunId"] = ScheduleId,
            ["reapplyReport"] = new JsonArray(),
            ["createdAt"] = "2026-10-10T10:00:02Z",
        });
        var made = await client.ResetAsync(
            RunId,
            new ResetRequest { Sequence = 4, Reason = "start over from a", AcknowledgeReexecution = true, BusinessId = BusinessIdHandling.Drop },
            new RequestOptions { IdempotencyKey = "reset-key-001" });
        var resetRequest = server.Requests[^1];
        Assert.Equal($"/v1/runs/{RunId}/resets", resetRequest.PathAndQuery);
        Assert.Equal("reset-key-001", resetRequest.Header("idempotency-key"));
        AssertBody("""{"sequence":4,"reason":"start over from a","businessId":"DROP","acknowledgeReexecution":true}""", resetRequest);
        Assert.Equal(("CREATED", ScheduleId, null), (made.State, made.NewRunId, made.FailureReason));

        server.Handler = _ => Reply(202, new JsonObject { ["resetId"] = BatchId, ["sourceRunId"] = RunId, ["sequence"] = 1, ["state"] = "PENDING_SOURCE" });
        await client.ResetAsync(RunId, new ResetRequest { Sequence = 1, Reason = "start over from the top" });
        Assert.Equal("""{"sequence":1,"reason":"start over from the top"}""", server.Requests[^1].BodyText);

        server.Handler = _ => Reply(200, new JsonObject
        {
            ["items"] = new JsonArray(new JsonObject { ["resetId"] = BatchId, ["sourceRunId"] = RunId, ["sequence"] = 4, ["state"] = "CREATED" }),
        });
        Assert.Single(await client.ListResetsAsync(RunId));
        server.Handler = _ => Reply(200, new JsonObject { ["resetId"] = BatchId, ["sourceRunId"] = RunId, ["sequence"] = 4, ["state"] = "CREATED" });
        Assert.Equal(BatchId, (await client.GetResetAsync(RunId, BatchId)).ResetId);
        Assert.Equal($"/v1/runs/{RunId}/resets/{BatchId}", server.Requests[^1].PathAndQuery);
    }

    // ------------------------------------------------------------------------------------------------ the rest

    [Fact]
    public void TheTokenDoesNotAppearInAnythingAGroupOfTheClientExposes()
    {
        var client = NewClient();
        foreach (var group in new object[] { client.Schedules, client.Batch, client.BusinessIds })
        {
            Assert.DoesNotContain("tok-1", JsonSerializer.Serialize(group, group.GetType()), StringComparison.Ordinal);
            Assert.DoesNotContain("tok-1", group.ToString(), StringComparison.Ordinal);
            foreach (var field in group.GetType().GetFields(System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic))
            {
                var attribute = field.GetCustomAttributes(typeof(System.Diagnostics.DebuggerBrowsableAttribute), inherit: false)
                    .Cast<System.Diagnostics.DebuggerBrowsableAttribute>().SingleOrDefault();
                Assert.True(attribute is { State: System.Diagnostics.DebuggerBrowsableState.Never }, $"{group.GetType().Name}.{field.Name} is visible to the debugger");
            }
        }

        // Nothing reaches the wire on its own: a client that was only built has sent nothing.
        Assert.Empty(server.Requests);
    }

    [Fact]
    public void AnInstantGivenAsTextIsReadInUtcOrRefused()
    {
        Assert.Equal("2026-10-10T08:00:00.000Z", Instant.Parse("2026-10-10T13:30:00+05:30").ToString());
        Assert.Equal("2026-10-10T10:00:00.123Z", Instant.Parse("2026-10-10T10:00:00.123456Z").ToString());
        Assert.Equal("2026-10-10T08:00:00.000Z", ((Instant)new DateTimeOffset(2026, 10, 10, 13, 30, 0, TimeSpan.FromHours(5.5))).ToString());
        foreach (var text in new[] { string.Empty, "not a time", "2026-10-10", "2026-10-10T10:00:00", "10:00:00Z", "2026-13-40T10:00:00Z" })
        {
            Assert.Equal(SagatErrorCode.ValidationFailed, Assert.Throws<SagatException>(() => Instant.Parse(text)).Code);
        }
    }

    [Fact]
    public async Task TheClientThatTheContainerHandsOutExposesTheGroupsThroughItsInterface()
    {
        var services = new ServiceCollection();
        services.AddLogging();
        services.AddSagatOctopusOrchestrator(o =>
        {
            o.ApiUrl = server.BaseUrl;
            o.Namespace = "payments";
            o.Token = "tok-1";
        });
        await using var provider = services.BuildServiceProvider();
        var viaInterface = provider.GetRequiredService<ISagatClient>();
        var viaClass = provider.GetRequiredService<SagatClient>();
        Assert.Same(viaClass.Schedules, viaInterface.Schedules);
        Assert.Same(viaClass.Batch, viaInterface.Batch);
        Assert.Same(viaClass.BusinessIds, viaInterface.BusinessIds);

        server.Handler = _ => Reply(200, Schedule());
        Assert.Equal(ScheduleId, (await viaInterface.Schedules.GetAsync(Ns, ScheduleId)).ScheduleId);
    }
}
