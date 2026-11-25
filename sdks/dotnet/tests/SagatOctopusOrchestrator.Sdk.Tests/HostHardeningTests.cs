using System.Diagnostics;
using System.Diagnostics.Metrics;
using System.Text.Json.Nodes;
using Grpc.Core;
using Grpc.Core.Interceptors;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workers;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>
/// The worker host against a server that misbehaves and against a host application whose own hooks (token provider, logger, metrics and
/// trace listeners) fail: it survives, and it does not drop a task it was leased.
/// </summary>
public sealed class HostHardeningTests : IAsyncLifetime
{
    private readonly string name = "hardening-" + Guid.NewGuid().ToString("N");
    private readonly List<WorkerHost> hosts = [];
    private FakeWorkerServer fake = null!;

    private FakeWorkerState State => fake.State;

    public async Task InitializeAsync() => fake = await FakeWorkerServer.StartAsync();

    public async Task DisposeAsync()
    {
        foreach (var host in hosts)
        {
            try
            {
                await host.StopAsync(TimeSpan.FromSeconds(1));
            }
            catch (Exception error) when (error is SagatException or OperationCanceledException)
            {
                // A host that did not start has nothing to stop.
            }
        }

        await fake.DisposeAsync();
    }

    private WorkerHost Host(Action<SagatOptions>? change = null, ILoggerFactory? logs = null)
    {
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "payments", Token = "tok" };
        options.Worker.Name = name;
        options.Retry.InitialDelay = TimeSpan.FromMilliseconds(5);
        options.Retry.MaxDelay = TimeSpan.FromMilliseconds(20);
        change?.Invoke(options);
        var host = new WorkerHost(options, logs);
        hosts.Add(host);
        return host;
    }

    private static TaskLease Lease(Action<TaskLease>? change = null) => Leases.Lease(change);

    // ---------------------------------------------------------------------------- 1. the re-registration storm

    [Fact]
    public async Task AServerThatRegistersTheWorkerAndRefusesItAtOnceIsNotHammeredWithRegistrations()
    {
        State.FailAlways("PollTasks", new Scripted(StatusCode.Unauthenticated));
        var host = Host();
        var clock = new RecordingClock().On(host);
        clock.Seen = () => (State.Registrations, State.Polls);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await host.StartAsync();

        // The waits the host asks for between registrations are read, not counted in a window of two seconds. The first refusal is
        // registered again at once; from the second refusal in a row each registration waits the backoff of the retry policy (5 ms
        // doubled per refusal, halved at most by the jitter), and the waits grow.
        await Eventually.UntilAsync(() => State.Registrations >= 6, "six registrations");
        var waits = clock.Waits;
        Assert.DoesNotContain(waits, wait => wait.Registrations == 1);
        for (var registrations = 2; registrations <= 5; registrations++)
        {
            var ceiling = 5 * Math.Pow(2, registrations - 2);
            var after = waits.Where(wait => wait.Registrations == registrations).Select(wait => wait.Due.TotalMilliseconds).ToList();
            Assert.True(
                after.Count == 1 && after[0] >= Math.Round(ceiling / 2) && after[0] <= ceiling,
                $"after registration {registrations} the host waited [{string.Join(", ", after)}] ms, not one backoff of {ceiling / 2} to {ceiling} ms");
        }
    }

    [Fact]
    public async Task TheBackoffOfTheReRegistrationIsResetByAPollThatSucceeds()
    {
        var host = Host(o =>
        {
            o.Retry.InitialDelay = TimeSpan.FromMilliseconds(400);
            o.Retry.MaxDelay = TimeSpan.FromSeconds(5);
        });
        var clock = new RecordingClock().On(host);
        clock.Seen = () => (State.Registrations, State.Polls);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("PollTasks", new Scripted(StatusCode.Unauthenticated), new Scripted(StatusCode.Unauthenticated), new Scripted(StatusCode.Unauthenticated));
        await host.StartAsync();

        // Three refusals in a row grow the wait ...
        await Eventually.UntilAsync(() => State.Registrations == 4, "the registrations after three refused polls", 15_000);
        await Eventually.UntilAsync(() => State.Polls >= 5, "polls that succeed");

        // ... a poll that succeeded resets it: the next lost session is registered again at once, not after the grown wait. The waits the
        // host asked its clock for are read, not the time it took: between the refused poll and the registration it asks for no backoff
        // (the shortest is half of 400 ms; the pause after an empty poll is at most 60 ms).
        var before = State.Registrations;
        var polls = State.Polls;
        State.Fail("PollTasks", new Scripted(StatusCode.Unauthenticated));
        await Eventually.UntilAsync(() => State.Registrations == before + 1, "the registration after the refusal", 15_000);
        var backoffs = clock.Waits.Where(wait => wait.Registrations == before && wait.Polls > polls && wait.Due >= TimeSpan.FromMilliseconds(200)).ToList();
        Assert.True(backoffs.Count == 0, $"the registration after a good poll waited {string.Join(", ", backoffs.Select(wait => wait.Due))}");
        Assert.True(clock.Waits.Count(wait => wait.Due >= TimeSpan.FromMilliseconds(200)) >= 2, "the second and the third refusal in a row were backed off");
    }

    // --------------------------------------------------------------------------- 2. the hooks of the caller

    [Fact]
    public async Task ATokenProviderThatThrowsWhileAHeartbeatIsDueDoesNotEndTheHeartbeat()
    {
        var throwing = 0;
        var refusals = 0;
        var beatsWhenItRecovered = 0;
        var host = Host(o =>
        {
            o.Token = null;
            o.TokenProvider = _ =>
            {
                if (Volatile.Read(ref throwing) == 1)
                {
                    Interlocked.Increment(ref refusals);
                    throw new HttpRequestException("the identity provider is down");
                }

                return ValueTask.FromResult("tok");
            };
            o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(30);

            // A full host does not poll: every call to the provider while the task runs is for a heartbeat.
            o.Worker.MaxInflight = 1;
        });

        // The handler holds the task on barriers, not on sleeps: the beats it waits for are what the test claims, so a loaded machine
        // only makes it slower (AGENTS.md rule 18).
        host.RegisterActivity<JsonNode?, int>("slow", async (_, _, _) =>
        {
            await Eventually.ReachedAsync(() => State.Of("Heartbeat").Count >= 1);
            Volatile.Write(ref throwing, 1);
            await Eventually.ReachedAsync(() => Volatile.Read(ref refusals) >= 2);
            Volatile.Write(ref beatsWhenItRecovered, State.Of("Heartbeat").Count);
            Volatile.Write(ref throwing, 0);
            await Eventually.ReachedAsync(() => State.Of("Heartbeat").Count >= Volatile.Read(ref beatsWhenItRecovered) + 3);
            return 1;
        });
        State.Lease(LeaseFor("slow"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion", 40_000);
        Assert.True(Volatile.Read(ref refusals) >= 2, $"{refusals} beats were refused by the provider during the outage");
        var afterTheOutage = State.Of("Heartbeat").Count - Volatile.Read(ref beatsWhenItRecovered);
        Assert.True(afterTheOutage >= 3, $"{afterTheOutage} heartbeats after the provider recovered");
        Assert.Empty(State.Of("FailStep"));
    }

    [Fact]
    public async Task ATokenProviderThatThrowsOnceWhileTheResultIsReportedDoesNotLoseTheResult()
    {
        var armed = 0;
        var host = Host(o =>
        {
            o.Token = null;
            o.TokenProvider = _ => Interlocked.Exchange(ref armed, 0) == 1 ? throw new HttpRequestException("the identity provider hiccuped") : ValueTask.FromResult("tok");
            o.Worker.MaxInflight = 1;
        });
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) =>
        {
            Volatile.Write(ref armed, 1);
            return Task.FromResult(1);
        });
        State.Lease(Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count >= 1, "the report, repeated after the hiccup", 5000);
        Assert.Empty(State.Of("FailStep"));
    }

    [Fact]
    public async Task ATokenProviderThatThrowsIsUnauthenticatedWithAReasonAndNeverTheProvidersOwnException()
    {
        var host = Host(o =>
        {
            o.Token = null;
            o.TokenProvider = _ => throw new InvalidOperationException("secret-canary-from-the-provider");
            o.Retry.MaxAttempts = 1;
        });
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        var error = await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        Assert.Equal(SagatErrorCode.Unauthenticated, error.Code);
        Assert.Equal("TOKEN_PROVIDER_FAILED", error.Reason);
        Assert.Null(error.InnerException);
        Assert.DoesNotContain("secret-canary", error.ToString(), StringComparison.Ordinal);
    }

    private sealed class ThrowingLogger : ILoggerFactory, ILogger
    {
        public volatile bool Armed;

        public void AddProvider(ILoggerProvider provider)
        {
        }

        public ILogger CreateLogger(string categoryName) => this;

        public void Dispose()
        {
        }

        public IDisposable? BeginScope<TState>(TState state)
            where TState : notnull => Armed ? throw new IOException("the log sink is down") : null;

        public bool IsEnabled(LogLevel logLevel) => Armed ? throw new IOException("the log sink is down") : true;

        public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
        {
            if (Armed)
            {
                throw new IOException("the log sink is down");
            }
        }
    }

    [Fact]
    public async Task ALoggerThatThrowsNeitherKillsTheHostNorDropsATask()
    {
        var logger = new ThrowingLogger();
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(30), logger);
        var ran = 0;
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, ct) =>
        {
            Interlocked.Increment(ref ran);
            context.Logger.LogInformation("the handler logs too");
            await Task.Delay(120, ct);
            return 1;
        });
        await host.StartAsync();
        logger.Armed = true;
        await Task.Delay(200);
        var polls = State.Polls;
        State.Lease(Lease());
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion although the logger throws");
        Assert.Equal(1, ran);
        Assert.True(State.Polls > polls, "the loop kept polling");
        Assert.True(host.IsRunning);
        await host.StopAsync(TimeSpan.FromSeconds(1));
    }

    [Fact]
    public async Task AMeterListenerThatThrowsNeitherKillsTheHostNorDropsATask()
    {
        using var listener = new MeterListener();
        listener.InstrumentPublished = (instrument, l) =>
        {
            if (instrument.Meter.Name == SdkInfo.MeterName)
            {
                l.EnableMeasurementEvents(instrument);
            }
        };
        void Explode(Instrument instrument, ReadOnlySpan<KeyValuePair<string, object?>> tags)
        {
            foreach (var tag in tags)
            {
                if (tag.Key == "sagat.worker" && (string?)tag.Value == name)
                {
                    throw new InvalidOperationException("the metrics sink exploded");
                }
            }
        }

        listener.SetMeasurementEventCallback<long>((instrument, _, tags, _) => Explode(instrument, tags));
        listener.SetMeasurementEventCallback<double>((instrument, _, tags, _) => Explode(instrument, tags));
        listener.Start();

        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(30));
        var ran = 0;
        host.RegisterActivity<JsonNode?, int>("charge", async (_, _, ct) =>
        {
            Interlocked.Increment(ref ran);
            await Task.Delay(100, ct);
            return 1;
        });
        await host.StartAsync();
        State.Lease(Lease());
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion although the meter listener throws");
        Assert.Equal(1, ran);
        Assert.True(host.IsRunning);
        var polls = State.Polls;
        await Eventually.UntilAsync(() => State.Polls > polls + 1, "polls after the task");
    }

    // ------------------------------------------------------------------------- 4. the same task, leased again

    private static TaskLease LeaseFor(string activity, Action<TaskLease>? change = null) =>
        Leases.Lease(lease =>
        {
            lease.ActivityName = activity;
            lease.StepId = activity;
            lease.TaskId = $"run-1/{activity}/1";
            change?.Invoke(lease);
        });

    [Fact]
    public async Task AStepLeasedAgainUnderANewGenerationWhileItRunsIsAnotherTaskInFlightAndStopWaitsForAndCancelsBoth()
    {
        // The task id is run/node/generation: the new generation is another task. A lease under the task id of the running one is a
        // duplicate and is ignored (HandBackTests).
        var host = Host(o => o.Worker.MaxInflight = 2);
        var gates = new[] { new TaskCompletionSource(), new TaskCompletionSource() };
        var started = 0;
        var reasons = new string?[2];
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, ct) =>
        {
            var index = Interlocked.Increment(ref started) - 1;
            try
            {
                await gates[index].Task.WaitAsync(ct);
            }
            catch (OperationCanceledException)
            {
                reasons[index] = context.CancellationReason;
                throw;
            }

            return 1;
        });
        State.Lease(Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => started == 1, "the first run");
        State.Lease(Lease(l =>
        {
            l.TaskId = "run-1/charge/2";
            l.Generation = 2;
        }));
        await Eventually.UntilAsync(() => started == 2, "the second run");

        // The first ends; the second is still in flight, and the host knows it: one slot is free, not two.
        gates[0].SetResult();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the first completion");
        var seenAfter = State.Polls;
        await Eventually.UntilAsync(() => State.Polls > seenAfter + 1, "polls after it");
        Assert.Equal(1, ((PollTasksRequest)State.Of("PollTasks")[^1].Request).MaxTasks);

        // StopAsync waits for what is in flight and then cancels it.
        var clock = Stopwatch.StartNew();
        await host.StopAsync(TimeSpan.FromMilliseconds(300));
        Assert.True(clock.ElapsedMilliseconds >= 250, $"stop did not wait for the second task ({clock.ElapsedMilliseconds} ms)");
        Assert.Equal("shutdown", reasons[1]);
    }

    // -------------------------------------------------------------------------------- 7. more tasks than asked

    [Fact]
    public async Task ATasksAnswerLargerThanTheRequestRunsTheFirstOnesAndLogsTheRest()
    {
        State.IgnoreMax = true;
        var provider = new CapturingLoggerProvider();
        using var factory = LoggerFactory.Create(builder => builder.AddProvider(provider));
        var host = Host(o => o.Worker.MaxInflight = 1, factory);
        var running = 0;
        var peak = 0;
        var seen = new List<string>();
        var gate = new TaskCompletionSource();
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, ct) =>
        {
            var now = Interlocked.Increment(ref running);
            int seenPeak;
            while ((seenPeak = Volatile.Read(ref peak)) < now && Interlocked.CompareExchange(ref peak, now, seenPeak) != seenPeak)
            {
            }

            lock (seen)
            {
                seen.Add(context.TaskId);
            }

            await gate.Task.WaitAsync(ct);
            Interlocked.Decrement(ref running);
            return 1;
        });
        State.Lease(Enumerable.Range(1, 5).Select(i => Lease(l => { l.TaskId = $"run-{i}/charge/1"; l.RunId = $"run-{i}"; })).ToArray());
        await host.StartAsync();
        await Eventually.UntilAsync(() => seen.Count >= 1, "the first task");
        await Task.Delay(400);
        Assert.Equal(1, Volatile.Read(ref peak));
        Assert.Equal(["run-1/charge/1"], seen);
        Assert.Contains(provider.Lines, line => line.Contains("5 tasks", StringComparison.Ordinal) && line.Contains("asked for 1", StringComparison.Ordinal) && line.Contains("4 others", StringComparison.Ordinal));
        gate.SetResult();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the one completion");
        await Task.Delay(200);
        Assert.Single(State.Of("CompleteStep"));
    }

    // --------------------------------------------------------------- 16. what the mutants of the review found untested

    [Fact]
    public async Task ThePollAsksForExactlyTheFreeCapacityAndNoMoreThanSixtyFour()
    {
        var idle = Host(o => o.Worker.MaxInflight = 100);
        idle.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await idle.StartAsync();
        await Eventually.UntilAsync(() => State.Of("PollTasks").Count >= 1, "a poll");
        Assert.Equal(64, ((PollTasksRequest)State.Of("PollTasks")[0].Request).MaxTasks);
        await idle.StopAsync(TimeSpan.FromSeconds(1));

        var busy = Host(o => o.Worker.MaxInflight = 3);
        var gate = new TaskCompletionSource();
        busy.RegisterActivity<JsonNode?, int>("charge", async (_, _, ct) =>
        {
            await gate.Task.WaitAsync(ct);
            return 1;
        });
        var before = State.Of("PollTasks").Count;
        State.Lease(Lease());
        await busy.StartAsync();
        await Eventually.UntilAsync(() => State.Of("PollTasks").Count >= before + 3, "polls with one task in flight");
        var asks = State.Of("PollTasks").Skip(before).Select(c => ((PollTasksRequest)c.Request).MaxTasks).ToList();
        Assert.Equal(3, asks[0]);
        Assert.All(asks.Skip(1), ask => Assert.Equal(2, ask));
        gate.SetResult();
    }

    [Fact]
    public async Task StopAsyncWakesAPauseParkedByAnEmptyPoll()
    {
        State.PollRetryAfterMs = 60_000;
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Polls >= 1, "the empty poll");
        await Task.Delay(300);
        var clock = Stopwatch.StartNew();
        await host.StopAsync(TimeSpan.FromSeconds(30));
        Assert.True(clock.ElapsedMilliseconds < 3000, $"stop took {clock.ElapsedMilliseconds} ms, the pause is 60 s");
        Assert.Equal(1, State.Polls);
    }

    [Theory]
    [InlineData(int.MaxValue)]
    [InlineData(60_000)]
    [InlineData(600_000)]
    public async Task APauseTheServerAsksForIsHonouredUpToAMinuteAndNeverBreaksTheLoop(int retryAfterMs)
    {
        State.PollRetryAfterMs = retryAfterMs;
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Polls >= 1, "the empty poll");
        await Task.Delay(500);
        Assert.True(host.IsRunning);
        Assert.Equal(1, State.Polls);
        var clock = Stopwatch.StartNew();
        await host.StopAsync(TimeSpan.FromSeconds(5));
        Assert.True(clock.ElapsedMilliseconds < 3000);
    }

    [Fact]
    public async Task TheFirstReasonOfCancellationWins()
    {
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20));
        var release = new TaskCompletionSource();
        var cancelledByTheRun = new TaskCompletionSource();
        string? reasonAfterShutdown = null;
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, ct) =>
        {
            await Task.Delay(Timeout.Infinite, ct).ContinueWith(_ => { }, TaskScheduler.Default);
            cancelledByTheRun.TrySetResult();
            await release.Task;
            reasonAfterShutdown = context.CancellationReason;
            return 1;
        });
        State.CancelRequested = true;
        State.Lease(Lease());
        await host.StartAsync();
        // The stop begins once the handler has seen the cancellation of the run, not 100 ms after the beat that carries it: on a loaded
        // machine the answer could still be on its way, and the shutdown would then be the first reason.
        await cancelledByTheRun.Task.WaitAsync(TimeSpan.FromSeconds(10));
        var stopping = host.StopAsync(TimeSpan.FromMilliseconds(100));
        await Task.Delay(400);
        release.SetResult();
        await stopping;
        Assert.Equal("run-cancelled", reasonAfterShutdown);
    }

    [Fact]
    public async Task AFailedResolutionOfTheSecretsIsNotRememberedForTheRestOfTheAttempt()
    {
        State.Secrets.Add(new ResolvedSecret { Name = "k", Value = Google.Protobuf.ByteString.CopyFromUtf8("v"), Version = 1 });
        State.Fail("ResolveSecrets", new Scripted(StatusCode.Unavailable, "UNAVAILABLE"));
        var host = Host();
        SagatException? first = null;
        string? second = null;
        host.RegisterActivity<JsonNode?, int>("pay", async (_, context, ct) =>
        {
            try
            {
                await context.GetSecretsAsync(ct);
            }
            catch (SagatException error)
            {
                first = error;
            }

            second = (await context.GetSecretsAsync(ct)).GetText("k");
            return 1;
        });
        State.Lease(LeaseFor("pay"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        Assert.Equal(SagatErrorCode.Unavailable, first!.Code);
        Assert.Equal("v", second);
        Assert.Equal(2, State.Of("ResolveSecrets").Count);
    }

    [Fact]
    public async Task ARestartedHostPollsAgainAndRunsTheTasksItIsGiven()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await host.StartAsync();
        State.Lease(Lease());
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the first completion");
        await host.StopAsync(TimeSpan.FromSeconds(1));
        var pollsAtStop = State.Polls;
        await Task.Delay(300);
        Assert.Equal(pollsAtStop, State.Polls);

        await host.StartAsync();
        Assert.Equal(2, State.Registrations);
        State.Lease(Lease(l => l.TaskId = "run-2/charge/1"));
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "the completion after the restart");
        Assert.True(State.Polls > pollsAtStop);
    }

    // ---------------------------------------------------------------------- the lost wake of a chain of quick steps

    [Fact]
    public async Task AChainOfQuickStepsIsNotSlowedByAWakeThatCameWhileThePollWasInFlight()
    {
        State.PollDelay = TimeSpan.FromMilliseconds(40);
        State.TakeBeforeDelay = true;
        State.PollRetryAfterMs = 1000;
        const int Steps = 12;
        var next = 1;
        State.OnComplete = () =>
        {
            var i = Interlocked.Increment(ref next);
            if (i <= Steps)
            {
                State.Lease(Lease(l => { l.TaskId = $"run-{i}/charge/1"; l.RunId = $"run-{i}"; }));
            }
        };
        var host = Host();
        var clock = new RecordingClock().On(host);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Lease(Lease(l => { l.TaskId = "run-1/charge/1"; l.RunId = "run-1"; }));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == Steps, "the chain", 60_000);

        // A lost wake shows as a pause of the loop (the second the empty poll asks for, 800 to 1200 ms with its jitter) waited out to its
        // end while the next step was ready. Read from the clock of the host, not from the time the chain took on a loaded machine: every
        // pause begun during the chain must have been cut short by the step that finished, or not taken at all. The heartbeat waits (10 s
        // for these leases) are not pauses of the loop and are left out.
        var chainEnded = State.Of("CompleteStep")[^1].At;
        var waitedOut = clock.FiredWaits
            .Where(wait => wait.Due >= TimeSpan.FromMilliseconds(800) && wait.Due <= TimeSpan.FromMilliseconds(1200) && wait.At < chainEnded)
            .ToList();
        Assert.True(waitedOut.Count == 0, $"{waitedOut.Count} pauses of the loop ran to their end during the chain of {Steps} quick steps (a lost wake costs a second each)");
    }

    // -------------------------------------------------------------- round 4: the heartbeat, the message, the session, the kind

    [Theory]
    [InlineData(100)]
    [InlineData(60 * 24 * 60 * 60 * 1000L)]
    public async Task AHeartbeatIntervalAboveAnHourIsRefusedAndAnHourIsNot(long milliseconds)
    {
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(milliseconds));
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        if (milliseconds <= 3_600_000)
        {
            await host.StartAsync();
            return;
        }

        var error = await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Contains("HeartbeatInterval", error.Message, StringComparison.Ordinal);
        Assert.Equal(0, State.Registrations);
    }

    [Fact]
    public async Task AHeartbeatIntervalOfExactlyAnHourIsAccepted()
    {
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromHours(1));
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await host.StartAsync();
        Assert.True(host.IsRunning);
    }

    private sealed class FailingInterceptor(string method, int times) : Interceptor
    {
        private int seen;

        internal int Calls => Volatile.Read(ref seen);

        public override AsyncUnaryCall<TResponse> AsyncUnaryCall<TRequest, TResponse>(
            TRequest request,
            ClientInterceptorContext<TRequest, TResponse> context,
            AsyncUnaryCallContinuation<TRequest, TResponse> continuation)
        {
            if (context.Method.Name == method && Interlocked.Increment(ref seen) <= times)
            {
                throw new InvalidOperationException("a hook of the application raised inside the beat");
            }

            return continuation(request, context);
        }
    }

    [Fact]
    public async Task ABeatThatRaisesSomethingThatIsNotASagatExceptionIsTriedAgainOnTheNextBeatAndDoesNotSpin()
    {
        var interceptor = new FailingInterceptor("Heartbeat", 2);
        var host = Host(o =>
        {
            o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(100);
            o.Worker.Interceptors.Add(interceptor);
        });
        var clock = new RecordingClock().On(host);

        // The handler holds the task until a beat after the two raised ones has reached the server (a barrier, not a sleep).
        host.RegisterActivity<JsonNode?, int>("slow", async (_, _, _) =>
        {
            await Eventually.ReachedAsync(() => interceptor.Calls >= 3 && State.Of("Heartbeat").Count >= 1);
            return 1;
        });
        State.Lease(LeaseFor("slow"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion", 40_000);

        // The two raised beats reached no server; the ones after them did (the loop did not die), and none came faster than the interval:
        // every beat tried waited the interval of 100 ms first, and each raised one also the pause of 100 ms after it. The calls are read
        // before the waits, so a beat tried after the read only adds waits.
        var tried = interceptor.Calls;
        var waits = clock.Waits.Count(wait => wait.Due == TimeSpan.FromMilliseconds(100));
        Assert.True(tried >= 3, $"{tried} beats were tried");
        Assert.True(State.Of("Heartbeat").Count >= 1, "a beat after the failed ones reached the server");
        Assert.True(waits >= tried + 2, $"{waits} waits of 100 ms for {tried} beats, two of them raised: the loop spins");
        Assert.Empty(State.Of("FailStep"));
    }

    [Fact]
    public async Task ABeatThatRaisesSomethingThatIsNotASagatExceptionPausesBeforeTheNextOneEvenWhenTheIntervalIsTiny()
    {
        // An interval of one tick waits for nothing: only the pause inside the failure handler keeps the loop from spinning (mutant N04).
        var interceptor = new FailingInterceptor("Heartbeat", int.MaxValue);
        var host = Host(o =>
        {
            o.Worker.HeartbeatInterval = TimeSpan.FromTicks(1);
            o.Worker.Interceptors.Add(interceptor);
        });
        var clock = new RecordingClock().On(host);
        host.RegisterActivity<JsonNode?, int>("slow", async (_, _, _) =>
        {
            await Eventually.ReachedAsync(() => interceptor.Calls >= 4);
            return 1;
        });
        State.Lease(LeaseFor("slow"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion", 40_000);

        // Every raised beat is followed by the pause of 100 ms before the next one (the newest may not have asked for its pause yet).
        var tried = interceptor.Calls;
        var pauses = clock.Waits.Count(wait => wait.Due == TimeSpan.FromMilliseconds(100));
        Assert.True(tried >= 4, $"{tried} beats were tried");
        Assert.True(pauses >= tried - 1, $"{pauses} pauses after {tried} raised beats: the loop spins");
        Assert.Empty(State.Of("FailStep"));
    }

    [Fact]
    public async Task ABeatThatFailsWithASagatExceptionPausesBeforeTheNextOneEvenWhenTheIntervalIsTiny()
    {
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromTicks(1));
        var clock = new RecordingClock().On(host);
        host.RegisterActivity<JsonNode?, int>("slow", async (_, _, _) =>
        {
            await Eventually.ReachedAsync(() => State.Of("Heartbeat").Count >= 4);
            return 1;
        });
        State.FailAlways("Heartbeat", new Scripted(StatusCode.Unavailable));
        State.Lease(LeaseFor("slow"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion", 40_000);

        // Every refused beat is followed by the pause of 100 ms before the next one (the newest may not have asked for its pause yet).
        var refused = State.Of("Heartbeat").Count;
        var pauses = clock.Waits.Count(wait => wait.Due == TimeSpan.FromMilliseconds(100));
        Assert.True(refused >= 4, $"{refused} beats reached the server");
        Assert.True(pauses >= refused - 1, $"{pauses} pauses after {refused} refused beats: the loop spins");
        Assert.Empty(State.Of("FailStep"));
    }

    [Fact]
    public async Task AMessageIsCutByCodePointsNeverInsideASurrogatePairAndAlwaysValidUtf8()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("emoji", (_, _, _) => throw ActivityFailure.Permanent("TOO_LONG", string.Concat(Enumerable.Repeat("\U0001F600", 600))));
        host.RegisterActivity<JsonNode?, int>("odd", (_, _, _) => throw ActivityFailure.Permanent("ODD", string.Concat(Enumerable.Repeat("x", 511)) + "\U0001F600tail"));
        host.RegisterActivity<JsonNode?, int>("lone", (_, _, _) => throw ActivityFailure.Permanent("LONE", "a\ud800b\udc00c"));
        host.RegisterActivity<JsonNode?, int>("short", (_, _, _) => throw ActivityFailure.Permanent("SHORT", "ação \U0001F600"));
        State.Lease(LeaseFor("emoji"), LeaseFor("odd"), LeaseFor("lone"), LeaseFor("short"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 4, "four failures");
        string Message(string step) => State.Of("FailStep").Select(c => (FailStepRequest)c.Request).First(r => r.StepId == step).SafeMessage;

        var emoji = Message("emoji");
        Assert.Equal(512, emoji.EnumerateRunes().Count());
        Assert.Equal(1024, emoji.Length);
        Assert.All(emoji.EnumerateRunes(), rune => Assert.Equal(0x1F600, rune.Value));

        // 511 letters and an emoji: 512 code points, the emoji whole (512 UTF-16 units would have cut it in two).
        var odd = Message("odd");
        Assert.Equal(512, odd.EnumerateRunes().Count());
        Assert.EndsWith("\U0001F600", odd, StringComparison.Ordinal);
        Assert.DoesNotContain("tail", odd, StringComparison.Ordinal);

        // A lone surrogate is not text: it becomes U+FFFD, so that what is sent is valid UTF-8.
        Assert.Equal("a\ufffdb\ufffdc", Message("lone"));
        Assert.Equal("ação \U0001F600", Message("short"));
        foreach (var step in new[] { "emoji", "odd", "lone", "short" })
        {
            var text = Message(step);
            Assert.Equal(text, new System.Text.UTF8Encoding(false, true).GetString(new System.Text.UTF8Encoding(false, true).GetBytes(text)));
        }
    }

    [Fact]
    public async Task TheSessionTokenIsHiddenFromTheDebuggerAndGoneWhenTheHostStops()
    {
        var field = typeof(WorkerHost).GetField("sessionToken", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!;
        var browsable = field.GetCustomAttributes(typeof(DebuggerBrowsableAttribute), inherit: false).Cast<DebuggerBrowsableAttribute>().SingleOrDefault();
        Assert.True(browsable is { State: DebuggerBrowsableState.Never }, "the session token is shown by the debugger");

        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await host.StartAsync();
        Assert.Equal("session-1", (string?)field.GetValue(host));
        Assert.DoesNotContain("session-1", host.ToString(), StringComparison.Ordinal);
        await host.StopAsync(TimeSpan.FromSeconds(1));
        Assert.Equal(string.Empty, (string?)field.GetValue(host));

        // A restart registers again and has a session once more.
        await host.StartAsync();
        Assert.Equal("session-1", (string?)field.GetValue(host));
    }

    [Theory]
    [InlineData("1")]
    [InlineData("1.1")]
    public async Task TheKindAndTheTargetOfATaskAreNotReadBelowProtocol12(string accepted)
    {
        State.AcceptedVersion = accepted;
        var host = Host();
        ActivityExecutionContext? seen = null;
        host.RegisterCompensation<JsonNode?, int>("limit.release", (_, context, _) =>
        {
            seen = context;
            return Task.FromResult(1);
        });
        State.Lease(LeaseFor("limit.release", l =>
        {
            l.TaskKind = "COMPENSATION";
            l.TargetNode = "reserve";
            l.TargetIdempotencyKey = "run-1:reserve";
        }));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        Assert.Equal(TaskKind.Forward, seen!.TaskKind);
        Assert.Equal(string.Empty, seen.TargetNode);
        Assert.Equal(string.Empty, seen.TargetIdempotencyKey);
    }

    [Fact]
    public async Task TheKindAndTheTargetOfATaskAreReadAtProtocol12()
    {
        var host = Host();
        ActivityExecutionContext? seen = null;
        host.RegisterCompensation<JsonNode?, int>("limit.release", (_, context, _) =>
        {
            seen = context;
            return Task.FromResult(1);
        });
        State.Lease(LeaseFor("limit.release", l =>
        {
            l.TaskKind = "COMPENSATION";
            l.TargetNode = "reserve";
            l.TargetIdempotencyKey = "run-1:reserve";
        }));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        Assert.Equal((TaskKind.Compensation, "reserve", "run-1:reserve"), (seen!.TaskKind, seen.TargetNode, seen.TargetIdempotencyKey));
    }
}
