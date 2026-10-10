using System.Diagnostics;
using System.Diagnostics.Metrics;
using System.Text.Json.Nodes;
using Grpc.Core;
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
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await host.StartAsync();
        await Task.Delay(2000);
        var registrations = State.Registrations;
        Assert.InRange(registrations, 2, 14);
        Assert.InRange(State.Polls, 2, 40);
    }

    [Fact]
    public async Task TheBackoffOfTheReRegistrationIsResetByAPollThatSucceeds()
    {
        var host = Host(o =>
        {
            o.Retry.InitialDelay = TimeSpan.FromMilliseconds(400);
            o.Retry.MaxDelay = TimeSpan.FromSeconds(5);
        });
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("PollTasks", new Scripted(StatusCode.Unauthenticated), new Scripted(StatusCode.Unauthenticated), new Scripted(StatusCode.Unauthenticated));
        await host.StartAsync();

        // Three refusals in a row grow the wait ...
        await Eventually.UntilAsync(() => State.Registrations == 4, "the registrations after three refused polls", 15_000);
        await Eventually.UntilAsync(() => State.Polls >= 5, "polls that succeed");

        // ... a poll that succeeded resets it: the next lost session is registered again at once, not after the grown wait.
        var before = State.Registrations;
        State.Fail("PollTasks", new Scripted(StatusCode.Unauthenticated));
        var clock = Stopwatch.StartNew();
        await Eventually.UntilAsync(() => State.Registrations == before + 1, "a registration at once", 2000);
        Assert.True(clock.ElapsedMilliseconds < 1500, $"took {clock.ElapsedMilliseconds} ms");
    }

    // --------------------------------------------------------------------------- 2. the hooks of the caller

    [Fact]
    public async Task ATokenProviderThatThrowsWhileAHeartbeatIsDueDoesNotEndTheHeartbeat()
    {
        var throwing = false;
        var host = Host(o =>
        {
            o.Token = null;
            o.TokenProvider = _ => throwing ? throw new HttpRequestException("the identity provider is down") : ValueTask.FromResult("tok");
            o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(30);
        });
        host.RegisterActivity<JsonNode?, int>("slow", async (_, _, ct) =>
        {
            await Task.Delay(100, ct);
            throwing = true;
            await Task.Delay(250, ct);
            throwing = false;
            await Task.Delay(500, ct);
            return 1;
        });
        State.Lease(LeaseFor("slow"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion", 15_000);
        var beats = State.Of("Heartbeat");
        Assert.True(Stopwatch.GetElapsedTime(beats[^1].At).TotalMilliseconds < 5000);
        var last = beats[^1].At;
        var afterTheOutage = beats.Count(call => call.At > beats[0].At && Stopwatch.GetElapsedTime(call.At, last).TotalMilliseconds < 450);
        Assert.True(afterTheOutage >= 3, $"{afterTheOutage} heartbeats in the last 450 ms of the handler");
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
    public async Task ATaskLeasedAgainWhileItRunsIsAnotherTaskInFlightAndStopWaitsForAndCancelsBoth()
    {
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
        State.Lease(Lease(l => l.Generation = 2));
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
        string? reasonAfterShutdown = null;
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, ct) =>
        {
            await Task.Delay(Timeout.Infinite, ct).ContinueWith(_ => { }, TaskScheduler.Default);
            await release.Task;
            reasonAfterShutdown = context.CancellationReason;
            return 1;
        });
        State.CancelRequested = true;
        State.Lease(Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("Heartbeat").Count >= 1, "the heartbeat that cancels");
        await Task.Delay(100);
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
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Lease(Lease(l => { l.TaskId = "run-1/charge/1"; l.RunId = "run-1"; }));
        var clock = Stopwatch.StartNew();
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == Steps, "the chain", 60_000);
        Assert.True(clock.ElapsedMilliseconds < 4000, $"{Steps} chained quick steps took {clock.ElapsedMilliseconds} ms (a lost wake costs a second each)");
    }
}
