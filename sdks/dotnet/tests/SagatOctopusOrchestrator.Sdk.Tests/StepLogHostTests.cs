using System.Diagnostics;
using System.Text;
using System.Text.Json.Nodes;
using Google.Protobuf;
using Grpc.Core;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workers;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>The step-log tests share the process's secret set and its bound of held bytes: they run alone, one after the other.</summary>
[CollectionDefinition(Name, DisableParallelization = true)]
public sealed class StepLogsRunAlone
{
    public const string Name = "step logs";
}

/// <summary>
/// Step-log capture through the host (design 041, sdks/SPEC.md section 6.9): what is captured and when, the batches and the answers of
/// AppendStepLogs, the carry and the wait before a report, against the in-memory worker service. Races are ordered by held answers and by
/// the test's own tick, never by a sleep.
/// </summary>
[Collection(StepLogsRunAlone.Name)]
public sealed class StepLogHostTests : IAsyncLifetime
{
    private readonly string name = "step-log-worker-" + Guid.NewGuid().ToString("N");
    private readonly List<WorkerHost> hosts = [];
    private FakeWorkerServer fake = null!;
    private StepLogDropCounter drops = null!;

    private FakeWorkerState State => fake.State;

    public async Task InitializeAsync()
    {
        fake = await FakeWorkerServer.StartAsync();
        State.AcceptedVersion = "1.7";
        State.StepLogs = new StepLogPolicy { Enabled = true };
        drops = new StepLogDropCounter(name);
    }

    public async Task DisposeAsync()
    {
        foreach (var host in hosts)
        {
            await host.StopAsync(TimeSpan.FromSeconds(1));
        }

        drops.Dispose();
        await fake.DisposeAsync();
    }

    private (WorkerHost Host, ManualTicker Ticker) Host(Action<SagatOptions>? change = null, ILoggerFactory? logs = null)
    {
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "payments", Token = "tok" };
        options.Worker.Name = name;
        options.Worker.MaxInflight = 1;
        options.Worker.Transport = WorkerTransport.Poll;
        options.Retry.InitialDelay = TimeSpan.FromMilliseconds(5);
        options.Retry.MaxDelay = TimeSpan.FromMilliseconds(20);
        change?.Invoke(options);
        var host = new WorkerHost(options, logs);
        hosts.Add(host);
        return (host, StepLogInternals.TickByHand(host));
    }

    private static TaskLease Lease(int attempt = 1) => Leases.Lease(lease =>
    {
        lease.TaskId = $"run-1/charge/{attempt}";
        lease.Attempt = attempt;
    });

    /// <summary>A handler that writes <paramref name="count"/> lines named <c>line n</c> and returns 1, or what <paramref name="end"/> returns.</summary>
    private static Func<JsonNode?, ActivityExecutionContext, CancellationToken, Task<int>> Lines(int count, Func<ActivityExecutionContext, Task<int>>? end = null) =>
        async (_, context, _) =>
        {
            for (var n = 1; n <= count; n++)
            {
                Say(context.Logger, $"line {n}");
            }

            return end is null ? 1 : await end(context);
        };

    /// <summary>One information line whose message is exactly <paramref name="text"/> and which has no field.</summary>
    private static void Say(ILogger logger, string text) => logger.Log(LogLevel.Information, default, text, null, static (state, _) => state);

    /// <summary>Runs one activity that writes <paramref name="count"/> lines and returns the carry of its report.</summary>
    private async Task<StepLogCarry?> RunOneAsync(int count, Action<SagatOptions>? change = null, Func<ActivityExecutionContext, Task<int>>? end = null)
    {
        var (host, _) = Host(change);
        host.RegisterActivity("charge", Lines(count, end));
        await host.StartAsync();
        State.Lease(Lease());
        return await ReportAsync();
    }

    private async Task<StepLogCarry?> ReportAsync()
    {
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count + State.Of("FailStep").Count > 0, "the report", 15_000);
        return State.Of("CompleteStep") is [var completed, ..]
            ? ((CompleteStepRequest)completed.Request).StepLogs
            : ((FailStepRequest)State.Of("FailStep")[0].Request).StepLogs;
    }

    private List<AppendStepLogsRequest> Appends() => [.. State.Of("AppendStepLogs").Select(call => (AppendStepLogsRequest)call.Request)];

    private static ulong[] Sequences(IEnumerable<StepLogRecord> records) => [.. records.Select(record => record.Sequence)];

    private static string[] Messages(IEnumerable<StepLogRecord> records) => [.. records.Select(record => record.Message)];

    private static ulong[] Span(ulong from, ulong to) => [.. Enumerable.Range((int)from, (int)(to - from + 1)).Select(n => (ulong)n)];

    private static Task<bool> Until(Func<bool> condition) => Eventually.ReachedAsync(condition, 15_000);

    private async Task DropsAsync(string expected) =>
        await Eventually.UntilAsync(() => drops.ToString() == expected, $"the drops {expected} (counted {drops})");

    [Fact]
    public async Task AShortStepAt17CarriesItsLinesOnItsReportAndMakesNoAppendStepLogsCall()
    {
        var carry = await RunOneAsync(3);
        Assert.Empty(State.Of("AppendStepLogs"));
        Assert.NotNull(carry);
        Assert.Equal(new[] { "line 1", "line 2", "line 3" }, Messages(carry.Records));
        Assert.Equal(Span(1, 3), Sequences(carry.Records));
        Assert.Equal(3UL, carry.HighestSequence);
        Assert.Equal(StepLogLevel.Info, carry.Records[0].Level);
        Assert.True(carry.Records[0].WorkerTimeEpochMs > 1_700_000_000_000);
        Assert.Equal("1.7", ((RegisterWorkerRequest)State.Of("RegisterWorker")[0].Request).Capabilities.ProtocolVersion);
        Assert.Equal(string.Empty, drops.ToString());
    }

    [Theory]
    [InlineData("accepted 1.5")]
    [InlineData("policy off")]
    [InlineData("no policy")]
    [InlineData("option off")]
    public async Task NothingIsCapturedBelow17WithThePolicyOffWithoutAPolicyOrWithTheOptionOff(string setup)
    {
        switch (setup)
        {
            case "accepted 1.5":
                State.AcceptedVersion = "1.5";
                break;
            case "policy off":
                State.StepLogs = new StepLogPolicy { Enabled = false };
                break;
            case "no policy":
                State.StepLogs = null;
                break;
            default:
                break;
        }

        var carry = await RunOneAsync(300, options => options.Worker.StepLogs.Capture = setup != "option off");
        Assert.Null(carry);
        Assert.Empty(State.Of("AppendStepLogs"));
    }

    [Fact]
    public void ACaptureLevelThatIsNotOneOfTheLevelsIsRefusedWhenTheHostIsCreated()
    {
        foreach (var level in new[] { LogLevel.None, (LogLevel)42, (LogLevel)(-1) })
        {
            var refused = Assert.Throws<SagatException>(() => Host(options => options.Worker.StepLogs.Level = level));
            Assert.Equal(SagatErrorCode.ValidationFailed, refused.Code);
        }

        foreach (var level in new[] { LogLevel.Trace, LogLevel.Critical })
        {
            Host(options => options.Worker.StepLogs.Level = level);
        }
    }

    [Fact]
    public async Task ALongStepSendsTheBatchIts256thCallCutWaitsForItsAnswerAndCarriesTheRest()
    {
        var handlerDone = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        State.AppendAnswers.Enqueue(new AppendAnswer(Hold: handlerDone.Task));
        var carry = await RunOneAsync(260, end: _ =>
        {
            handlerDone.TrySetResult();
            return Task.FromResult(1);
        });
        var batch = Assert.Single(Appends());
        Assert.Equal(Span(1, 256), Sequences(batch.Records));
        Assert.Equal(("w-1", "session-1", "run-1/charge/1", "run-1", "charge", 1, 1L, "lt-secret-lease-token"), (batch.WorkerId, batch.SessionToken, batch.TaskId, batch.RunId, batch.StepId, batch.Attempt, batch.Generation, batch.LeaseToken));
        Assert.NotNull(carry);
        Assert.Equal(Span(257, 260), Sequences(carry.Records));
        Assert.Equal(260UL, carry.HighestSequence);

        // The report waited for the answer of the batch, which was held until the handler had returned.
        Assert.True(State.Of("AppendStepLogs answered")[0].At < State.Of("CompleteStep")[0].At, "the report left before the answer of the batch");
    }

    [Fact]
    public async Task AnAnswerHeldPast2SecondsIsGivenUpAndTheReportGoesWithTheNewestRecordsThatFit()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(Hold: Until(() => State.Of("CompleteStep").Count > 0)));
        long ended = 0;
        var carry = await RunOneAsync(260, end: _ =>
        {
            ended = Stopwatch.GetTimestamp();
            return Task.FromResult(1);
        });

        // The wait is the SDK's own: from the handler's end, not from anything before it.
        var waited = Stopwatch.GetElapsedTime(ended, State.Of("CompleteStep")[0].At);
        Assert.True(waited >= TimeSpan.FromMilliseconds(1999), $"reported {waited.TotalMilliseconds} ms after the handler ended");
        Assert.NotNull(carry);
        Assert.Equal(Span(257, 260), Sequences(carry.Records));
        await DropsAsync("report_deadline:256");
        Assert.Single(State.Of("AppendStepLogs"));
    }

    [Fact]
    public async Task StoppingTheHostLeavesAnAttemptInItsReportWaitToItsReportAndFlushesOneStillRunningAsItsBatchInFlight()
    {
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        State.AppendAnswers.Enqueue(new AppendAnswer(Hold: release.Task));
        var (host, _) = Host(options => options.Worker.MaxInflight = 2);
        var handlerEnded = false;
        var running = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, cancellationToken) =>
        {
            if (context.Attempt == 1)
            {
                for (var n = 1; n <= 260; n++)
                {
                    Say(context.Logger, $"waiting {n}");
                }

                Volatile.Write(ref handlerEnded, true);
                return 1;
            }

            for (var n = 1; n <= 10; n++)
            {
                Say(context.Logger, $"running {n}");
            }

            running.TrySetResult();
            try
            {
                await Task.Delay(Timeout.Infinite, cancellationToken);
            }
            catch (OperationCanceledException)
            {
                // The host is stopping: the handler still reports.
            }

            return 2;
        });
        await host.StartAsync();
        State.Lease(Lease(1), Lease(2));
        Assert.True(await Until(() => Volatile.Read(ref handlerEnded) && State.Of("AppendStepLogs").Count == 1), "the held batch of the attempt in its report wait");
        await running.Task;
        var stopping = host.StopAsync(TimeSpan.FromMilliseconds(200));
        Assert.True(await Until(() => State.Of("AppendStepLogs").Count == 2), "the shutdown flush of the running attempt");
        release.TrySetResult();
        await stopping;

        var flushed = Appends()[1];
        Assert.Equal(Enumerable.Range(1, 10).Select(n => $"running {n}"), Messages(flushed.Records));
        var waiting = State.Of("CompleteStep").Select(call => (CompleteStepRequest)call.Request).Single(report => report.Attempt == 1);
        Assert.Equal(Span(257, 260), Sequences(waiting.StepLogs.Records));
        var stopped = State.Of("CompleteStep").Select(call => (CompleteStepRequest)call.Request).Single(report => report.Attempt == 2);
        Assert.Empty(stopped.StepLogs.Records);
        Assert.Equal(10UL, stopped.StepLogs.HighestSequence);
    }

    [Fact]
    public async Task ABatchHoldsAtMost500RecordsWhenMoreAreBufferedAsItsTurnComes()
    {
        var logged = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        State.AppendAnswers.Enqueue(new AppendAnswer(Hold: logged.Task));
        var carry = await RunOneAsync(856, end: async _ =>
        {
            logged.TrySetResult();
            Assert.True(await Until(() => State.Of("AppendStepLogs").Count == 2), "the second batch");
            return 1;
        });
        var batches = Appends();
        Assert.Equal(3, batches.Count);
        Assert.Equal(Span(1, 256), Sequences(batches[0].Records));
        Assert.Equal(Span(257, 756), Sequences(batches[1].Records));
        Assert.Equal(Span(757, 792), Sequences(batches[2].Records));
        Assert.NotNull(carry);
        Assert.Equal(Span(793, 856), Sequences(carry.Records));
    }

    [Fact]
    public async Task FinalOnlyDropsWhatIsBufferedAndLaterCallsAndTheReportCarriesTheFinalMarkWithNoRecord()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(StepLogVerdict.FinalOnly));
        var carry = await RunOneAsync(300);
        Assert.Single(State.Of("AppendStepLogs"));
        Assert.NotNull(carry);
        Assert.Empty(carry.Records);
        Assert.Equal(300UL, carry.HighestSequence);
        await DropsAsync("after_ceiling:44");
    }

    [Fact]
    public async Task StopAttemptSendsNoCarry()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(StepLogVerdict.StopAttempt, StepLogStopReason.Lease));
        Assert.Null(await RunOneAsync(300));
        Assert.Single(State.Of("AppendStepLogs"));
    }

    [Fact]
    public async Task StopCaptureStopsTheNextAttemptOfTheSession()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(StepLogVerdict.StopCapture, StepLogStopReason.CaptureOff));
        var (host, _) = Host();
        host.RegisterActivity("charge", Lines(300));
        await host.StartAsync();
        State.Lease(Lease(1));
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the first report");
        State.Lease(Lease(2));
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "both reports");
        Assert.Single(State.Of("AppendStepLogs"));
        Assert.All(State.Of("CompleteStep"), report => Assert.Null(((CompleteStepRequest)report.Request).StepLogs));
    }

    [Fact]
    public async Task StopCaptureForTheSessionIsAnsweredBySendingOnceMoreWithTheTokenTheHostNowHolds()
    {
        State.NumberSessions = true;
        var reregistered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        State.AppendAnswers.Enqueue(new AppendAnswer(StepLogVerdict.StopCapture, StepLogStopReason.Session, Hold: reregistered.Task));
        var (host, _) = Host();
        host.RegisterActivity("charge", Lines(260));
        await host.StartAsync();
        State.Lease(Lease());
        await Eventually.UntilAsync(() => State.Of("AppendStepLogs").Count == 1, "the batch");
        await StepLogInternals.RegisterAgainAsync(host);
        reregistered.TrySetResult();
        var carry = await ReportAsync();
        var batches = Appends();
        Assert.Equal(2, batches.Count);
        Assert.Equal("session-1", batches[0].SessionToken);
        Assert.Equal("session-2", batches[1].SessionToken);
        Assert.Equal(Span(1, 256), Sequences(batches[1].Records));
        Assert.NotNull(carry);
        Assert.Equal(Span(257, 260), Sequences(carry.Records));
    }

    [Fact]
    public async Task StopCaptureForTheSessionWithTheTokenTheHostStillHoldsStopsCaptureAndSendsNoCarry()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(StepLogVerdict.StopCapture, StepLogStopReason.Session));
        Assert.Null(await RunOneAsync(260));
        Assert.Single(State.Of("AppendStepLogs"));
    }

    [Fact]
    public async Task RetryLaterAndResourceExhaustedSendTheSameBatchAgainAfterTheirDelayWithoutCountingARetry()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(StepLogVerdict.RetryLater, RetryAfterMs: 60));
        State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.ResourceExhausted, TrailerRetryAfterMs: 60));
        State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.Unavailable));
        State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.Unavailable));
        var carry = await RunOneAsync(260);
        var calls = State.Of("AppendStepLogs");
        Assert.Equal(5, calls.Count);
        foreach (var call in calls)
        {
            Assert.Equal(((AppendStepLogsRequest)calls[0].Request).Records, ((AppendStepLogsRequest)call.Request).Records);
        }

        Assert.True(Stopwatch.GetElapsedTime(calls[0].At, calls[1].At) >= TimeSpan.FromMilliseconds(55), "RETRY_LATER waited its delay");
        Assert.True(Stopwatch.GetElapsedTime(calls[1].At, calls[2].At) >= TimeSpan.FromMilliseconds(55), "RESOURCE_EXHAUSTED waited its trailer");
        Assert.NotNull(carry);
        Assert.Equal(Span(257, 260), Sequences(carry.Records));
    }

    [Fact]
    public async Task UnavailableThreeTimesDropsTheBatchAfterItsTwoRetriesAndTheCarryStillGoes()
    {
        for (var n = 0; n < 3; n++)
        {
            State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.Unavailable));
        }

        var carry = await RunOneAsync(260);
        Assert.Equal(3, State.Of("AppendStepLogs").Count);
        Assert.NotNull(carry);
        Assert.Equal(Span(257, 260), Sequences(carry.Records));
        await DropsAsync("send_failed:256");
    }

    [Fact]
    public async Task InvalidArgumentDropsTheBatchAtOnceAndGoesOn()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.InvalidArgument));
        var carry = await RunOneAsync(260);
        Assert.Single(State.Of("AppendStepLogs"));
        Assert.NotNull(carry);
        Assert.Equal(Span(257, 260), Sequences(carry.Records));
        await DropsAsync("send_failed:256");
    }

    [Fact]
    public async Task UnimplementedStopsTheBatchesOfTheAttemptAndTheReportCarriesTheNewest64Records()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.Unimplemented));
        var carry = await RunOneAsync(400);
        Assert.Single(State.Of("AppendStepLogs"));
        Assert.NotNull(carry);
        Assert.Equal(Span(337, 400), Sequences(carry.Records));
        Assert.Equal(400UL, carry.HighestSequence);
        await DropsAsync("report_deadline:80 send_failed:256");
    }

    [Fact]
    public async Task AfterUnimplementedTheAttemptKeepsTheNewestRecordsThatFitOneCarryWhateverItLogsAfter()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.Unimplemented));
        var (host, _) = Host();
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, _) =>
        {
            for (var n = 1; n <= 256; n++)
            {
                Say(context.Logger, $"line {n}");
            }

            Assert.True(await Until(() => StepLogInternals.BatchesStopped(host)), "the refused batch");

            // 2,000 calls after UNIMPLEMENTED: the buffer never fills, the oldest go as they come.
            for (var n = 257; n <= 2256; n++)
            {
                Say(context.Logger, $"line {n}");
            }

            return 1;
        });
        await host.StartAsync();
        State.Lease(Lease());
        var carry = await ReportAsync();
        Assert.Single(State.Of("AppendStepLogs"));
        Assert.NotNull(carry);
        Assert.Equal(2256UL, carry.HighestSequence);
        Assert.Equal(Span(2193, 2256), Sequences(carry.Records));
        await DropsAsync("report_deadline:1936 send_failed:256");
    }

    [Fact]
    public async Task ABatchInItsBackOffIsNotSentAgainOnceTheSessionStoppedCapture()
    {
        // Attempt 1 is in its 1-second back-off after two UNAVAILABLE when attempt 2's batch is answered STOP_CAPTURE.
        State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.Unavailable));
        State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.Unavailable));
        State.AppendAnswers.Enqueue(new AppendAnswer(StepLogVerdict.StopCapture, StepLogStopReason.CaptureOff));
        var (host, _) = Host(options => options.Worker.MaxInflight = 2);
        var secondMayLog = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, _) =>
        {
            if (context.Attempt == 2)
            {
                await secondMayLog.Task;
            }

            for (var n = 1; n <= 256; n++)
            {
                Say(context.Logger, $"line {n}");
            }

            if (context.Attempt == 1)
            {
                Assert.True(await Until(() => State.Of("AppendStepLogs").Count == 2), "the first retry");
                secondMayLog.TrySetResult();
                Assert.True(await Until(() => State.Of("AppendStepLogs").Count == 3), "the stopping answer");
            }

            return context.Attempt;
        });
        await host.StartAsync();
        State.Lease(Lease(1), Lease(2));
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "both reports", 15_000);
        Assert.Equal(3, State.Of("AppendStepLogs").Count);
        Assert.All(State.Of("CompleteStep"), report => Assert.Null(((CompleteStepRequest)report.Request).StepLogs));
        await DropsAsync("send_failed:256");
    }

    [Fact]
    public async Task ACancelledStatusTheServerSentIsRetriedAsAnyStatusNotListed()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.Cancelled));
        var carry = await RunOneAsync(260);
        Assert.Equal(2, State.Of("AppendStepLogs").Count);
        Assert.NotNull(carry);
        Assert.Equal(Span(257, 260), Sequences(carry.Records));
    }

    [Fact]
    public async Task ABatchGivenUpAtTheEndOfTheWaitCountsReportDeadlineAndIsNotSentAgain()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.Unavailable, Hold: Until(() => State.Of("CompleteStep").Count > 0)));
        var carry = await RunOneAsync(260);
        Assert.NotNull(carry);
        Assert.Equal(Span(257, 260), Sequences(carry.Records));
        await DropsAsync("report_deadline:256");
        Assert.Single(State.Of("AppendStepLogs"));
    }

    [Fact]
    public async Task ALostAnswerToACarryingReportSendsTheSameCarryAgain()
    {
        State.Fail("CompleteStep", new Scripted(StatusCode.Unavailable));
        await RunOneAsync(3);
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "the report sent again");
        var reports = State.Of("CompleteStep").Select(call => (CompleteStepRequest)call.Request).ToList();
        Assert.NotNull(reports[0].StepLogs);
        Assert.Equal(reports[0].StepLogs, reports[1].StepLogs);
        Assert.Equal(3, reports[1].StepLogs.Records.Count);
    }

    [Fact]
    public async Task AFailureCarriesItsLinesToo()
    {
        var carry = await RunOneAsync(2, end: _ => throw ActivityFailure.Permanent("BROKEN", "broken"));
        Assert.Single(State.Of("FailStep"));
        Assert.NotNull(carry);
        Assert.Equal(new[] { "line 1", "line 2" }, Messages(carry.Records));
    }

    [Fact]
    public async Task TheCaptureLevelIsTheOptionBelowItACallTakesNoSequenceAndTheHostLoggerLevelDoesNotMatter()
    {
        var output = new CapturingLoggerProvider();
        using var logs = LoggerFactory.Create(builder => builder.AddProvider(output).SetMinimumLevel(LogLevel.Error));
        var (host, _) = Host(options => options.Worker.StepLogs.Level = LogLevel.Debug, logs);
        var enabled = new List<bool>();
        host.RegisterActivity<JsonNode?, int>("charge", (_, context, _) =>
        {
            var logger = context.Logger;
            // Trace is below the capture level and the host's level; Debug and Information are captured though the host logs only Error.
            enabled.AddRange([logger.IsEnabled(LogLevel.Trace), logger.IsEnabled(LogLevel.Debug), logger.IsEnabled(LogLevel.Information)]);
            logger.LogTrace("trace line");
            logger.LogDebug("debug line");
            using (logger.BeginScope(new Dictionary<string, object?> { ["order"] = "o-1" }))
            using (logger.BeginScope("scope {Region}", "eu"))
            {
                logger.LogInformation("info line {K}", 1);
            }

            logger.LogWarning("warn line");
            logger.LogError(new InvalidOperationException("the payment provider refused"), "error line");
            logger.Log(LogLevel.None, "none line");
            logger.LogCritical("critical line");
            return Task.FromResult(1);
        });
        await host.StartAsync();
        State.Lease(Lease());
        var carry = await ReportAsync();
        Assert.Equal(new[] { false, true, true }, enabled);
        Assert.NotNull(carry);
        Assert.Equal(new[] { "debug line", "info line 1", "warn line", "error line", "critical line" }, Messages(carry.Records));
        Assert.Equal(Span(1, 5), Sequences(carry.Records));
        Assert.Equal(
            new[] { new StepLogField { Key = "K", IntValue = 1 }, new StepLogField { Key = "Region", StringValue = "eu" }, new StepLogField { Key = "order", StringValue = "o-1" } },
            carry.Records[1].Fields);
        Assert.StartsWith("System.InvalidOperationException: the payment provider refused", carry.Records[3].Exception, StringComparison.Ordinal);
        Assert.Equal(StepLogLevel.Error, carry.Records[3].Level);
        Assert.Equal(StepLogLevel.Error, carry.Records[4].Level);
        Assert.Empty(carry.Records[2].Exception);

        // The host still logs what it logged: its logger is at Error (and its filter lets None through, as it did before capture).
        var activityLines = output.Lines.Where(line => line.StartsWith("SagatOctopusOrchestrator.Activity.charge ", StringComparison.Ordinal)).ToList();
        Assert.Equal(new[] { "error line", "none line", "critical line" }, activityLines.Select(line => line.Split(' ')[2] + " line"));

        // No line of the SDK's own holds a captured line.
        Assert.DoesNotContain(output.Lines, line => !line.StartsWith("SagatOctopusOrchestrator.Activity.", StringComparison.Ordinal) && line.Contains(" line", StringComparison.Ordinal));
    }

    [Fact]
    public async Task ALineIsRedactedWithEverySecretTheProcessResolvedByAnyAttemptAlsoAfterTheHostStartedAgain()
    {
        const string canary = "canary-value-9Kx2";
        State.Secrets.Add(new ResolvedSecret { Name = "key", Value = ByteString.CopyFromUtf8(canary) });
        var (host, _) = Host();
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, cancellationToken) =>
        {
            if (context.Attempt == 1)
            {
                using var secrets = await context.GetSecretsAsync(cancellationToken);
                return 1;
            }

            context.Logger.Log(
                LogLevel.Information,
                default,
                new List<KeyValuePair<string, object?>> { new(canary, canary) },
                new InvalidOperationException("failure " + canary),
                static (_, _) => canary);
            return 2;
        });
        await host.StartAsync();
        State.Lease(Lease(1), Lease(2));
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "both reports");
        var report = (CompleteStepRequest)State.Of("CompleteStep")[1].Request;
        var record = Assert.Single(report.StepLogs.Records);
        Assert.Equal("[REDACTED]", record.Message);
        Assert.Equal(new[] { new StepLogField { Key = "[REDACTED]", StringValue = "[REDACTED]" } }, record.Fields);
        Assert.StartsWith("System.InvalidOperationException: failure [REDACTED]", record.Exception, StringComparison.Ordinal);
        Assert.DoesNotContain(canary, Encoding.UTF8.GetString(report.ToByteArray()), StringComparison.Ordinal);

        // The same host, stopped and started again in the process, still redacts what an earlier attempt resolved.
        await host.StopAsync();
        await host.StartAsync();
        State.Lease(Lease(2));
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 3, "the report after the restart");
        Assert.Equal("[REDACTED]", ((CompleteStepRequest)State.Of("CompleteStep")[2].Request).StepLogs.Records[0].Message);
    }

    [Fact]
    public async Task ALoggerCallNeverThrowsIntoTheActivityWhateverItIsHanded()
    {
        var circular = new StepLogFormatTests.Node();
        circular.Self = circular;
        using var logs = LoggerFactory.Create(builder => builder.AddProvider(new ThrowingLoggerProvider()));
        var (host, _) = Host(logs: logs);
        host.RegisterActivity<JsonNode?, int>("charge", (_, context, _) =>
        {
            var logger = context.Logger;
            logger.Log(
                LogLevel.Information,
                default,
                new List<KeyValuePair<string, object?>> { new("circular", circular), new("getter", new StepLogFormatTests.ThrowingGetter()), new("string", new StepLogFormatTests.ThrowingString()) },
                null,
                static (_, _) => "values");
            logger.Log(LogLevel.Information, default, "formatter", null, static (_, _) => throw new InvalidOperationException("formatter"));
            logger.Log(LogLevel.Information, default, new ThrowingFields(), null, static (_, _) => "enumeration");
            logger.LogInformation(new ThrowingException(), "exception");
            using (logger.BeginScope(new ThrowingFields()))
            {
                logger.LogInformation("scoped");
            }

            return Task.FromResult(1);
        });
        await host.StartAsync();
        State.Lease(Lease());
        var carry = await ReportAsync();
        Assert.Single(State.Of("CompleteStep"));
        Assert.Empty(State.Of("FailStep"));
        Assert.NotNull(carry);
        Assert.Equal(5UL, carry.HighestSequence);
        Assert.Equal(new[] { "values", string.Empty, "enumeration", "exception", "scoped" }, Messages(carry.Records));
        Assert.Equal(typeof(ThrowingException).FullName, carry.Records[3].Exception);
        Assert.Equal(new[] { new StepLogField { Key = "before", StringValue = "kept" } }, carry.Records[2].Fields);
    }

    [Fact]
    public async Task ABatchFillsUpToButNotOver192KiBAndTheAttemptBufferStopsAtItsBound()
    {
        var big = new string('m', 8000);
        var logged = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var (host, ticker) = Host();
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, _) =>
        {
            // 8 KB lines: the attempt buffer (256 KiB) is full long before 256 records, so the tick sends.
            for (var n = 1; n <= 100; n++)
            {
                Say(context.Logger, $"{n} {big}");
            }

            logged.TrySetResult();
            Assert.True(await Until(() => State.Of("AppendStepLogs").Count >= 1), "the batch of the tick");
            return 1;
        });
        await host.StartAsync();
        State.Lease(Lease());
        await logged.Task;
        Assert.Equal(1, ticker.Tick());
        await ReportAsync();
        var batch = Appends()[0];
        var size = batch.CalculateSize();
        Assert.True(size <= 192 * 1024, $"{size} bytes");
        Assert.True(size > (192 * 1024) - 8100, $"the batch is not filled up to its bound: {size} bytes");

        // 33 records of about 8,020 bytes reach 256 KiB: the 34th to the 100th are dropped.
        Assert.Equal(24, batch.Records.Count);
        await DropsAsync("buffer_full:67");
    }

    [Fact]
    public async Task OnceTheSecretSetIsFullNothingIsSentAndTheReportCarriesTheFinalMarkWithNoRecord()
    {
        using var swapped = StepLogInternals.SwapProcessSecrets(StepLogInternals.NewSecretSet());
        for (var n = 0; n <= 1024; n++)
        {
            State.Secrets.Add(new ResolvedSecret { Name = $"s{n}", Value = ByteString.CopyFromUtf8($"full-set-{n:D6}") });
        }

        var output = new CapturingLoggerProvider();
        using var logs = LoggerFactory.Create(builder => builder.AddProvider(output));
        var (host, ticker) = Host(logs: logs);
        var resolved = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var ticked = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, cancellationToken) =>
        {
            for (var n = 1; n <= 10; n++)
            {
                Say(context.Logger, $"line {n}");
            }

            using (await context.GetSecretsAsync(cancellationToken))
            {
                resolved.TrySetResult();
                await ticked.Task;
            }

            Say(context.Logger, "after");
            return 1;
        });
        await host.StartAsync();
        State.Lease(Lease());
        await resolved.Task;

        // The tick of the second, with ten records buffered and the set full, sends nothing.
        Assert.Equal(1, ticker.Tick());
        ticked.TrySetResult();
        var carry = await ReportAsync();
        Assert.Empty(State.Of("AppendStepLogs"));
        Assert.NotNull(carry);
        Assert.Empty(carry.Records);
        Assert.Equal(11UL, carry.HighestSequence);
        await DropsAsync("secret_set_full:11");
        Assert.Single(output.Lines, line => line.Contains("The secret set of the process is full", StringComparison.Ordinal));
        Assert.DoesNotContain(output.Lines, line => line.Contains("full-set-", StringComparison.Ordinal));
    }

    [Fact]
    public async Task StopSendsWhatAnAttemptStillRunningBufferedOnceAsOneBatchWithNoResend()
    {
        State.AppendAnswers.Enqueue(new AppendAnswer(Code: StatusCode.Unavailable));
        var (host, _) = Host();
        var logged = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, cancellationToken) =>
        {
            for (var n = 1; n <= 10; n++)
            {
                Say(context.Logger, $"line {n}");
            }

            logged.TrySetResult();
            await Task.Delay(Timeout.Infinite, cancellationToken);
            return 1;
        });
        await host.StartAsync();
        State.Lease(Lease());
        await logged.Task;
        await host.StopAsync(TimeSpan.FromMilliseconds(50));
        var batch = Assert.Single(Appends());
        Assert.Equal(Span(1, 10), Sequences(batch.Records));
        Assert.Empty(State.Of("CompleteStep"));
        Assert.Empty(State.Of("FailStep"));
        await DropsAsync("send_failed:10");
    }

    [Fact]
    public async Task StopSendsOneBatchAndTheReportCarriesWhatDidNotFit()
    {
        var big = new string('m', 8000);
        var (host, _) = Host();
        var logged = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, cancellationToken) =>
        {
            // 30 lines of 8 KB: one batch of 192 KiB holds 24 of them, and none is cut before the stop.
            for (var n = 1; n <= 30; n++)
            {
                Say(context.Logger, $"{n} {big}");
            }

            logged.TrySetResult();
            try
            {
                await Task.Delay(Timeout.Infinite, cancellationToken);
            }
            catch (OperationCanceledException)
            {
                // The handler still reports after the shutdown.
            }

            return 1;
        });
        await host.StartAsync();
        State.Lease(Lease());
        await logged.Task;
        await host.StopAsync(TimeSpan.FromMilliseconds(50));
        var batches = Appends();
        Assert.Equal(2, batches.Count);
        Assert.Equal(Span(1, 24), Sequences(batches[0].Records));
        Assert.Equal(Span(25, 28), Sequences(batches[1].Records));
        var report = (CompleteStepRequest)Assert.Single(State.Of("CompleteStep")).Request;
        Assert.Equal(Span(29, 30), Sequences(report.StepLogs.Records));
        Assert.Equal(30UL, report.StepLogs.HighestSequence);
        Assert.True(report.StepLogs.CalculateSize() <= 16 * 1024);
    }

    private sealed class ThrowingException : Exception
    {
        public override string ToString() => throw new InvalidOperationException("no text");
    }

    /// <summary>A state whose enumeration throws after its first pair.</summary>
    private sealed class ThrowingFields : IReadOnlyList<KeyValuePair<string, object?>>
    {
        public int Count => 2;

        public KeyValuePair<string, object?> this[int index] => index == 0 ? new("before", "kept") : throw new InvalidOperationException("enumeration");

        public IEnumerator<KeyValuePair<string, object?>> GetEnumerator()
        {
            yield return this[0];
            yield return this[1];
        }

        System.Collections.IEnumerator System.Collections.IEnumerable.GetEnumerator() => GetEnumerator();

        public override string ToString() => "throwing fields";
    }

    /// <summary>The host's logging, broken: every call of its loggers throws.</summary>
    private sealed class ThrowingLoggerProvider : ILoggerProvider
    {
        public ILogger CreateLogger(string categoryName) => new Throwing();

        public void Dispose()
        {
        }

        private sealed class Throwing : ILogger
        {
            public IDisposable? BeginScope<TState>(TState state)
                where TState : notnull => throw new InvalidOperationException("scope");

            public bool IsEnabled(LogLevel logLevel) => logLevel >= LogLevel.Information ? true : throw new InvalidOperationException("enabled");

            public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter) =>
                throw new InvalidOperationException("log");
        }
    }
}
