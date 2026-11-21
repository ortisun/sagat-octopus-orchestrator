using System.Diagnostics;
using System.Reflection;
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
/// The hand-back of SPEC section 6.2: a lease the host received and will not run is reported with <c>FailStep</c> <c>TASK_NOT_STARTED</c>,
/// before any handler of it runs, once, and never for a task whose handler started.
/// </summary>
public sealed class HandBackTests : IAsyncLifetime
{
    private readonly string name = "hand-back-" + Guid.NewGuid().ToString("N");
    private readonly List<WorkerHost> hosts = [];
    private FakeWorkerServer fake = null!;
    private MeterCapture meters = null!;

    private FakeWorkerState State => fake.State;

    public async Task InitializeAsync()
    {
        fake = await FakeWorkerServer.StartAsync();
        meters = new MeterCapture(name);
    }

    public async Task DisposeAsync()
    {
        foreach (var host in hosts)
        {
            await host.StopAsync(TimeSpan.FromSeconds(1));
        }

        meters.Dispose();
        await fake.DisposeAsync();
    }

    private WorkerHost Host(int maxInflight, Interceptor? interceptor = null, ILoggerFactory? logs = null)
    {
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "payments", Token = "tok" };
        options.Worker.Name = name;
        options.Worker.MaxInflight = maxInflight;
        options.Retry.InitialDelay = TimeSpan.FromMilliseconds(5);
        options.Retry.MaxDelay = TimeSpan.FromMilliseconds(20);
        if (interceptor is not null)
        {
            options.Worker.Interceptors.Add(interceptor);
        }

        var host = new WorkerHost(options, logs);
        hosts.Add(host);
        return host;
    }

    /// <summary>The lease of run <paramref name="n"/>, with a lease token of its own.</summary>
    private static TaskLease Numbered(int n, Action<TaskLease>? change = null) =>
        Leases.Lease(lease =>
        {
            lease.TaskId = $"run-{n}/charge/1";
            lease.RunId = $"run-{n}";
            lease.LeaseToken = $"LEASE-TOKEN-{n}";
            change?.Invoke(lease);
        });

    /// <summary>
    /// The task ids of the <c>FailStep</c> <c>TASK_NOT_STARTED</c> calls, sorted, after checking that each is the whole hand-back: retryable,
    /// the fixed message, the lease's fence and identity, and the idempotency key of every failure.
    /// </summary>
    private List<string> HandedBack()
    {
        var ids = new List<string>();
        foreach (var call in State.Of("FailStep"))
        {
            var request = (FailStepRequest)call.Request;
            if (request.ErrorCode != "TASK_NOT_STARTED")
            {
                continue;
            }

            Assert.True(request.Retryable, "a hand-back is retryable");
            Assert.Equal("the host did not start this task", request.SafeMessage);
            Assert.Equal("failed-" + request.TaskId, request.IdempotencyKey);
            Assert.Equal("LEASE-TOKEN-" + request.RunId["run-".Length..], request.LeaseToken);
            Assert.Equal("charge", request.StepId);
            Assert.Equal(1, request.Attempt);
            Assert.Equal(1, request.Generation);
            Assert.Equal(Leases.Traceparent, request.Traceparent);
            ids.Add(request.TaskId);
        }

        ids.Sort(StringComparer.Ordinal);
        return ids;
    }

    private int BeatsOf(string task) => State.Of("Heartbeat").Count(call => ((HeartbeatRequest)call.Request).TaskId == task);

    [Fact]
    public async Task APollAnswerBeyondTheCreditIsHandedBack()
    {
        State.IgnoreMax = true;
        State.Lease(Numbered(1), Numbered(2), Numbered(3));
        var ran = new List<string>();
        var host = Host(maxInflight: 1);
        host.RegisterActivity<JsonNode?, int>("charge", (_, context, _) =>
        {
            lock (ran)
            {
                ran.Add(context.TaskId);
            }

            return Task.FromResult(1);
        });

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the kept task completes");
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 2, "the two hand-backs");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        Assert.Equal(["run-2/charge/1", "run-3/charge/1"], HandedBack());
        Assert.Equal(["run-1/charge/1"], ran);
        Assert.Equal(0, BeatsOf("run-2/charge/1") + BeatsOf("run-3/charge/1"));
        Assert.Equal(2, meters.Total("sagat.worker.tasks.failed", "TASK_NOT_STARTED"));
        Assert.Equal(1, meters.Total("sagat.worker.tasks.taken"));

        // A hand-back is not an attempt: only the task that ran has a duration.
        Assert.Equal(1, meters.Total("sagat.worker.activity.duration"));
    }

    [Fact]
    public async Task AStreamFrameBeyondTheCreditIsHandedBackAndTheStartedTaskIsNot()
    {
        State.AcceptedVersion = "1.4";
        State.IgnoreMax = true;
        State.HoldStreamOpen = true;
        State.Lease(Numbered(1), Numbered(2), Numbered(3));
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var host = Host(maxInflight: 1);
        host.RegisterActivity<JsonNode?, int>("charge", async (_, _, ct) =>
        {
            await release.Task.WaitAsync(ct);
            return 1;
        });

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 2, "the two hand-backs");
        release.SetResult();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the kept task completes");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        Assert.Equal(["run-2/charge/1", "run-3/charge/1"], HandedBack());
        Assert.Equal(2, State.Of("FailStep").Count);
        Assert.Equal("run-1/charge/1", ((CompleteStepRequest)Assert.Single(State.Of("CompleteStep")).Request).TaskId);
    }

    [Fact]
    public async Task AFrameAfterTheCreditIsUsedUpIsReadAndHandedBack()
    {
        State.AcceptedVersion = "1.4";
        State.IgnoreMax = true;
        State.HoldStreamOpen = true;

        // One receive window: the first frame uses the whole credit, the second carries tasks beyond it.
        State.StreamFrameSizes.Enqueue(1);
        State.StreamFrameSizes.Enqueue(2);
        State.Lease(Numbered(1), Numbered(2), Numbered(3));
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var host = Host(maxInflight: 1);
        host.RegisterActivity<JsonNode?, int>("charge", async (_, _, ct) =>
        {
            await release.Task.WaitAsync(ct);
            return 1;
        });

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 2, "the two hand-backs");
        release.SetResult();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the kept task completes");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        Assert.Equal(["run-2/charge/1", "run-3/charge/1"], HandedBack());
        Assert.Equal(2, State.Of("FailStep").Count);
        Assert.Equal("run-1/charge/1", ((CompleteStepRequest)Assert.Single(State.Of("CompleteStep")).Request).TaskId);
    }

    [Fact]
    public async Task TheRestOfAFrameOverTheInputBudgetIsHandedBack()
    {
        State.AcceptedVersion = "1.4";
        State.HoldStreamOpen = true;
        var heavy = Leases.Document(new string('x', 5 * 1024 * 1024), "charge");
        State.Lease(Numbered(1, l => l.InputJson = heavy), Numbered(2, l => l.InputJson = heavy), Numbered(3));
        var host = Host(maxInflight: 3);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the kept task completes");
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 2, "the hand-backs");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        // The lease over the budget and the rest of the frame after it.
        Assert.Equal(["run-2/charge/1", "run-3/charge/1"], HandedBack());
        Assert.Equal("run-1/charge/1", ((CompleteStepRequest)Assert.Single(State.Of("CompleteStep")).Request).TaskId);
    }

    [Fact]
    public async Task AStreamFrameReadWhileStoppingIsHandedBackWhole()
    {
        State.AcceptedVersion = "1.4";
        State.Lease(Numbered(1), Numbered(2));
        var hold = new HoldUntilStop("StreamTasks", nonemptyAnswer: 1);
        var ran = 0;
        var host = Host(maxInflight: 4, hold);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) =>
        {
            Interlocked.Increment(ref ran);
            return Task.FromResult(1);
        });

        await host.StartAsync();
        await hold.Held.WaitAsync(TimeSpan.FromSeconds(10));

        // The stop begins (the host is stopping once StopAsync has returned its task) while the frame it read is held; then the frame
        // reaches the loop.
        await host.StopAsync(TimeSpan.FromSeconds(5));

        // Stop returned: the hand-backs were made before the channel closed.
        Assert.Equal(["run-1/charge/1", "run-2/charge/1"], HandedBack());
        Assert.Equal(0, Volatile.Read(ref ran));
        Assert.Empty(State.Of("CompleteStep"));
        Assert.Equal(0, meters.Total("sagat.worker.tasks.taken"));
    }

    [Fact]
    public async Task APollAnswerThatArrivesWhileStoppingIsHandedBackAndTheRunningTaskIsNot()
    {
        State.Lease(Numbered(1));
        var hold = new HoldUntilStop("PollTasks", nonemptyAnswer: 2);
        var started = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var host = Host(maxInflight: 4, hold);
        host.RegisterActivity<JsonNode?, int>("charge", async (_, _, ct) =>
        {
            started.TrySetResult();
            await release.Task.WaitAsync(ct);
            return 1;
        });

        await host.StartAsync();
        await started.Task.WaitAsync(TimeSpan.FromSeconds(10));

        // The next answer that carries leases is held until the stop has begun.
        State.Lease(Numbered(2), Numbered(3));
        await hold.Held.WaitAsync(TimeSpan.FromSeconds(10));
        var stopped = host.StopAsync(TimeSpan.FromSeconds(5));
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 2, "the hand-backs");
        release.SetResult();
        await stopped;

        Assert.Equal(["run-2/charge/1", "run-3/charge/1"], HandedBack());
        Assert.Equal(2, State.Of("FailStep").Count);
        Assert.Equal("run-1/charge/1", ((CompleteStepRequest)Assert.Single(State.Of("CompleteStep")).Request).TaskId);
    }

    [Fact]
    public async Task ADuplicateLeaseIsIgnoredNeverHandedBack()
    {
        State.AcceptedVersion = "1.4";
        State.IgnoreMax = true;
        State.HoldStreamOpen = true;

        // The second is the running task's own lease again; the fourth repeats a lease handed back.
        State.Lease(Numbered(1), Numbered(1), Numbered(2), Numbered(2));
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var host = Host(maxInflight: 1);
        host.RegisterActivity<JsonNode?, int>("charge", async (_, _, ct) =>
        {
            await release.Task.WaitAsync(ct);
            return 1;
        });

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count >= 1, "the hand-back");
        release.SetResult();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the kept task completes");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        // A duplicate is never reported, and a lease is handed back once.
        Assert.Equal(["run-2/charge/1"], HandedBack());
        Assert.Single(State.Of("FailStep"));
        Assert.Single(State.Of("CompleteStep"));
    }

    [Fact]
    public async Task ALeaseOfARunningTaskInALaterAnswerIsIgnoredNeverHandedBack()
    {
        State.IgnoreMax = true;
        State.Lease(Numbered(1));
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var ran = new List<string>();
        var host = Host(maxInflight: 2);
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, ct) =>
        {
            lock (ran)
            {
                ran.Add(context.TaskId);
            }

            await release.Task.WaitAsync(ct);
            return 1;
        });

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("PollTasks").Count >= 2 && ran.Count == 1, "the first task runs and the next poll asks for one");

        // The next answer repeats the running task's lease, then carries one task within the credit and one beyond it: the repeated lease,
        // whose fence may be the running handler's, is never reported.
        State.Lease(Numbered(1), Numbered(3), Numbered(4));
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 1, "the hand-back");
        release.SetResult();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "both started tasks complete");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        Assert.Equal(["run-4/charge/1"], HandedBack());
        Assert.Single(State.Of("FailStep"));
        lock (ran)
        {
            Assert.Equal(["run-1/charge/1", "run-3/charge/1"], ran);
        }
    }

    [Fact]
    public async Task SiblingIterationsThatShareATaskIdAreNotDuplicates()
    {
        State.AcceptedVersion = "1.5";
        State.IgnoreMax = true;
        State.HoldStreamOpen = true;
        static TaskLease Sibling(uint index) => Numbered(1, lease =>
        {
            lease.Instance = new SagatOctopusOrchestrator.Protocol.V1.ExecutionInstanceIdentity { DefinitionNodeId = "charge" };
            lease.Instance.InstancePath.Add(new SagatOctopusOrchestrator.Protocol.V1.ScopePathSegment { ScopeNodeId = "each", IterationIndex = index });
        });

        State.Lease(Sibling(0), Sibling(1), Sibling(2));
        var host = Host(maxInflight: 2);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "both iterations complete");
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 1, "the iteration beyond the credit is handed back");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        // The first two run although they share the task id; the third is handed back with its own instance identity.
        var completed = State.Of("CompleteStep").Select(call => ((CompleteStepRequest)call.Request).Instance.InstancePath[0].IterationIndex).Order();
        Assert.Equal([0u, 1u], completed);
        Assert.Equal(["run-1/charge/1"], HandedBack());
        var handedBack = (FailStepRequest)Assert.Single(State.Of("FailStep")).Request;
        Assert.Equal(2u, Assert.Single(handedBack.Instance.InstancePath).IterationIndex);
        Assert.Equal("charge", handedBack.Instance.DefinitionNodeId);
    }

    [Fact]
    public async Task AHandBackIsRetriedWithTheSameKeyAndEndsQuietlyWhenStale()
    {
        State.IgnoreMax = true;
        State.Lease(Numbered(1), Numbered(2), Numbered(3));
        State.Fail("FailStep", new Scripted(StatusCode.Unavailable), new Scripted(StatusCode.Aborted, "STALE_ATTEMPT"));
        var host = Host(maxInflight: 1);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(
            () => meters.Total("sagat.worker.tasks.failed") == 1 && meters.Total("sagat.worker.tasks.stale") == 1,
            "one hand-back counted as a failure and one stale");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        // Three calls for two leases: one answered UNAVAILABLE and repeated with its key, one stale.
        var calls = State.Of("FailStep").Select(call => (FailStepRequest)call.Request).ToList();
        Assert.Equal(3, calls.Count);
        Assert.Equal(["failed-run-2/charge/1", "failed-run-3/charge/1"], calls.Select(call => call.IdempotencyKey).Distinct().Order(StringComparer.Ordinal));
        Assert.All(calls, call => Assert.Equal("TASK_NOT_STARTED", call.ErrorCode));
        Assert.Equal(1, meters.Total("sagat.worker.tasks.failed", "TASK_NOT_STARTED"));
        Assert.Equal(1, meters.Total("sagat.worker.tasks.stale"));
        Assert.Equal(0, meters.Total("sagat.worker.tasks.dropped"));
        Assert.Single(State.Of("CompleteStep"));
    }

    [Fact]
    public async Task AtMostEightHandBacksAreOnTheirWayAtOnceAndTheOthersWaitTheirTurn()
    {
        State.IgnoreMax = true;
        State.FailStepHold = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        State.Lease(Enumerable.Range(1, 10_000).Select(n => Numbered(n)).ToArray());
        var host = Host(maxInflight: 1);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.FailStepsHeld >= 8, "eight hand-backs held at the server");

        // While the eight are held no ninth may arrive. Its absence cannot be awaited: the server is watched for a while, which orders
        // nothing (the eight are held on the barrier whatever the delay).
        await Task.Delay(300);
        Assert.Equal(8, State.Of("FailStep").Count);
        Assert.Equal(8, State.FailStepPeak);

        State.FailStepHold.SetResult();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 9_999, "every lease beyond the first is handed back", 60_000);
        await host.StopAsync(TimeSpan.FromSeconds(10));

        Assert.Equal(8, State.FailStepPeak);
        var handedBack = State.Of("FailStep").Select(call => ((FailStepRequest)call.Request).TaskId).ToList();
        Assert.Equal(9_999, handedBack.Distinct().Count());
        Assert.DoesNotContain("run-1/charge/1", handedBack);
        Assert.Equal("run-1/charge/1", ((CompleteStepRequest)Assert.Single(State.Of("CompleteStep")).Request).TaskId);
    }

    [Fact]
    public async Task ALeaseLeasedAgainWhileItsHandBackIsOnItsWayIsIgnored()
    {
        State.IgnoreMax = true;
        State.FailStepHold = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        State.Lease(Numbered(1), Numbered(2));
        var ran = new List<string>();
        var host = Host(maxInflight: 1);
        host.RegisterActivity<JsonNode?, int>("charge", (_, context, _) =>
        {
            lock (ran)
            {
                ran.Add(context.TaskId);
            }

            return Task.FromResult(1);
        });

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.FailStepsHeld == 1, "the hand-back of the second lease is held at the server");
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the first task completes");

        // A later answer leases the second task again while its TASK_NOT_STARTED is on its way. The poll after that answer shows the host
        // has looked at it.
        State.Lease(Numbered(2));
        await Eventually.UntilAsync(() => State.Pending == 0, "the answer that repeats the lease");
        var polls = State.Polls;
        await Eventually.UntilAsync(() => State.Polls > polls, "the poll after it");

        State.FailStepHold.SetResult();
        await Eventually.UntilAsync(() => State.FailStepsHeld == 0, "the hand-back ends");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        Assert.Equal(["run-2/charge/1"], HandedBack());
        Assert.Single(State.Of("FailStep"));
        lock (ran)
        {
            Assert.Equal(["run-1/charge/1"], ran);
        }

        Assert.Single(State.Of("CompleteStep"));
    }

    [Fact]
    public async Task ALeaseTheRuntimeWouldNotStartIsHandedBackAndTheOthersOfTheAnswerRun()
    {
        State.Lease(Numbered(1), Numbered(2));
        var provider = new CapturingLoggerProvider();
        using var factory = LoggerFactory.Create(builder => builder.AddProvider(provider));
        var ran = new List<string>();
        var host = Host(maxInflight: 2, logs: factory);
        host.RegisterActivity<JsonNode?, int>("charge", (_, context, _) =>
        {
            lock (ran)
            {
                ran.Add(context.TaskId);
            }

            return Task.FromResult(1);
        });

        // The runtime refuses the first task it is asked to start (the private seam of the host, replaced by reflection) and starts the
        // others.
        var refused = 0;
        Func<Func<Task>, Task> start = work => Interlocked.Increment(ref refused) == 1
            ? throw new InvalidOperationException("the runtime refused the task")
            : Task.Run(work);
        typeof(WorkerHost).GetField("startTask", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(host, start);

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 1 && State.Of("CompleteStep").Count == 1, "the hand-back and the other task");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        Assert.Equal(["run-1/charge/1"], HandedBack());
        Assert.Single(State.Of("FailStep"));
        Assert.Equal("run-2/charge/1", ((CompleteStepRequest)Assert.Single(State.Of("CompleteStep")).Request).TaskId);
        lock (ran)
        {
            Assert.Equal(["run-2/charge/1"], ran);
        }

        Assert.Equal(1, meters.Total("sagat.worker.tasks.taken"));
        Assert.Single(provider.Lines, line => line.Contains("hands back task run-1/charge/1", StringComparison.Ordinal) && line.Contains("not_started", StringComparison.Ordinal));
    }

    [Fact]
    public async Task HandBacksStillOnTheirWayWhenTheGraceEndsAreDroppedAndNeverReported()
    {
        State.IgnoreMax = true;
        State.FailStepHold = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        State.Lease(Enumerable.Range(1, 21).Select(n => Numbered(n)).ToArray());
        var provider = new CapturingLoggerProvider();
        using var factory = LoggerFactory.Create(builder => builder.AddProvider(provider));
        var host = Host(maxInflight: 1, logs: factory);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.FailStepsHeld == 8 && State.Of("CompleteStep").Count == 1, "eight hand-backs held, twelve queued");

        // The twenty hand-backs cannot end within the grace: eight are held at the server, the others wait for them.
        var grace = TimeSpan.FromMilliseconds(300);
        var clock = Stopwatch.StartNew();
        await host.StopAsync(grace);
        clock.Stop();

        // The grace, then the fixed two seconds the cancelled work is given, then the channel closes.
        Assert.True(clock.Elapsed < grace + TimeSpan.FromSeconds(2) + TimeSpan.FromSeconds(1.5), $"stop took {clock.Elapsed}");
        await Eventually.UntilAsync(() => meters.Total("sagat.worker.tasks.dropped") == 20, "every hand-back that was not made is dropped");
        State.FailStepHold.SetResult();
        await Eventually.UntilAsync(() => State.FailStepsHeld == 0, "the server lets the held calls go");

        // Every hand-back ended dropped, so no other call can follow: eight reached the server once each, none was counted as made.
        var calls = State.Of("FailStep").Select(call => ((FailStepRequest)call.Request).TaskId).ToList();
        Assert.Equal(8, calls.Count);
        Assert.Equal(8, calls.Distinct().Count());
        Assert.Equal(0, meters.Total("sagat.worker.tasks.failed"));
        var dropped = provider.Lines.Where(line => line.Contains("could not be reported", StringComparison.Ordinal)).ToList();
        Assert.Equal(20, dropped.Count);
        Assert.Equal(20, Enumerable.Range(2, 20).Count(n => dropped.Count(line => line.Contains($"task run-{n}/charge/1 ", StringComparison.Ordinal)) == 1));
    }

    [Fact]
    public async Task AHandBackQueuedWhileTheWorkerLeavesIsHandedBackOnce()
    {
        State.IgnoreMax = true;
        State.Lease(Numbered(1), Numbered(2));
        var host = Host(maxInflight: 1);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        // One worker place, and that worker held once it has found the queue empty and before it gives up its place (the private seams of
        // the host, set by reflection).
        var foundEmpty = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var leaving = 0;
        Func<Task> hold = () =>
        {
            if (Interlocked.Increment(ref leaving) != 1)
            {
                return Task.CompletedTask;
            }

            foundEmpty.SetResult();
            return release.Task;
        };
        typeof(WorkerHost).GetField("handBackBound", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(host, 1);
        typeof(WorkerHost).GetField("beforeHandBackWorkerLeaves", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(host, hold);

        await host.StartAsync();

        // The worker hands back run-2, finds the queue empty and is held there.
        await foundEmpty.Task.WaitAsync(TimeSpan.FromSeconds(10));
        Assert.Equal(["run-2/charge/1"], HandedBack());
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "run-1 completes");

        // The next answer runs run-3 and hands back run-4, which finds the only place still taken. The poll after that answer shows the
        // host has looked at it.
        State.Lease(Numbered(3), Numbered(4));
        await Eventually.UntilAsync(() => State.Pending == 0, "the next answer");
        var polls = State.Polls;
        await Eventually.UntilAsync(() => State.Polls > polls, "the poll after it");
        Assert.Single(State.Of("FailStep"));

        release.SetResult();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 2, "the queued hand-back");
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "run-3 completes");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        // Never stranded, never twice.
        Assert.Equal(["run-2/charge/1", "run-4/charge/1"], HandedBack());
        Assert.Equal(2, State.Of("FailStep").Count);
    }

    [Theory]
    [InlineData(true)]
    [InlineData(false)]
    public async Task AHandlerCannotSendTaskNotStartedAndARealHandBackStillDoes(bool retryable)
    {
        State.IgnoreMax = true;
        State.Lease(Numbered(1), Numbered(2));
        var host = Host(maxInflight: 1);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => throw new ActivityFailure("TASK_NOT_STARTED", "custom", retryable));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 2, "the handler's failure and the hand-back");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        // The code is reserved for the host (SPEC section 6.2): the handler's failure is reported as WORKER_ERROR, with its own message
        // and retryable flag; the lease the host did not start is still handed back with the code.
        var failures = State.Of("FailStep").Select(call => (FailStepRequest)call.Request).ToDictionary(request => request.TaskId);
        var handler = failures["run-1/charge/1"];
        Assert.Equal("WORKER_ERROR", handler.ErrorCode);
        Assert.Equal("custom", handler.SafeMessage);
        Assert.Equal(retryable, handler.Retryable);
        Assert.Equal(["run-2/charge/1"], HandedBack());
        Assert.Equal(1, meters.Total("sagat.worker.tasks.failed", "WORKER_ERROR"));
        Assert.Equal(1, meters.Total("sagat.worker.tasks.failed", "TASK_NOT_STARTED"));
    }

    [Fact]
    public async Task ACompensationOrAReconcilerCannotSendTaskNotStarted()
    {
        State.Lease(
            Numbered(1, lease =>
            {
                lease.ActivityName = "refund";
                lease.TaskKind = "COMPENSATION";
                lease.TargetNode = "charge";
            }),
            Numbered(2, lease =>
            {
                lease.ActivityName = "check";
                lease.TaskKind = "RECONCILIATION";
                lease.TargetNode = "charge";
            }));
        var host = Host(maxInflight: 2);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        host.RegisterCompensation<JsonNode?>("refund", (_, _, _) => throw ActivityFailure.Retryable("TASK_NOT_STARTED", "custom"));
        host.RegisterReconciler<JsonNode?>("check", (_, _, _) => throw ActivityFailure.Permanent("TASK_NOT_STARTED", "custom"));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 2, "both failures");
        await host.StopAsync(TimeSpan.FromSeconds(5));

        var failures = State.Of("FailStep").Select(call => (FailStepRequest)call.Request).ToDictionary(request => request.TaskId);
        Assert.Equal(("WORKER_ERROR", "custom", true), (failures["run-1/charge/1"].ErrorCode, failures["run-1/charge/1"].SafeMessage, failures["run-1/charge/1"].Retryable));
        Assert.Equal(("WORKER_ERROR", "custom", false), (failures["run-2/charge/1"].ErrorCode, failures["run-2/charge/1"].SafeMessage, failures["run-2/charge/1"].Retryable));
    }

    /// <summary>
    /// Holds the <paramref name="nonemptyAnswer"/>-th answer of <paramref name="method"/> that carries leases (a poll answer or a stream frame),
    /// once it was received, until the call's token is cancelled, which the host does when its stop begins: the loop gets the answer only then.
    /// </summary>
    private sealed class HoldUntilStop(string method, int nonemptyAnswer) : Interceptor
    {
        private readonly TaskCompletionSource held = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private int answers;

        /// <summary>Completes when the answer is held.</summary>
        internal Task Held => held.Task;

        public override AsyncUnaryCall<TResponse> AsyncUnaryCall<TRequest, TResponse>(
            TRequest request,
            ClientInterceptorContext<TRequest, TResponse> context,
            AsyncUnaryCallContinuation<TRequest, TResponse> continuation)
        {
            var call = continuation(request, context);
            if (context.Method.Name != method)
            {
                return call;
            }

            return new AsyncUnaryCall<TResponse>(
                HoldAsync(call.ResponseAsync, context.Options.CancellationToken),
                call.ResponseHeadersAsync,
                call.GetStatus,
                call.GetTrailers,
                call.Dispose);
        }

        public override AsyncServerStreamingCall<TResponse> AsyncServerStreamingCall<TRequest, TResponse>(
            TRequest request,
            ClientInterceptorContext<TRequest, TResponse> context,
            AsyncServerStreamingCallContinuation<TRequest, TResponse> continuation)
        {
            var call = continuation(request, context);
            if (context.Method.Name != method)
            {
                return call;
            }

            return new AsyncServerStreamingCall<TResponse>(
                new HoldingReader<TResponse>(call.ResponseStream, this),
                call.ResponseHeadersAsync,
                call.GetStatus,
                call.GetTrailers,
                call.Dispose);
        }

        private async Task<TResponse> HoldAsync<TResponse>(Task<TResponse> answer, CancellationToken stop)
        {
            var response = await answer;
            await HoldIfDueAsync(response, stop);
            return response;
        }

        private async Task HoldIfDueAsync(object? answer, CancellationToken stop)
        {
            // The answer is the SDK's own (internal) protocol type, not the one the fake server of the tests is generated with: its leases
            // are read through the descriptor.
            if (answer is not Google.Protobuf.IMessage message
                || message.Descriptor.FindFieldByName("tasks")?.Accessor.GetValue(message) is not System.Collections.IList { Count: > 0 }
                || Interlocked.Increment(ref answers) != nonemptyAnswer)
            {
                return;
            }

            held.TrySetResult();
            try
            {
                await Task.Delay(Timeout.Infinite, stop);
            }
            catch (OperationCanceledException)
            {
                // The stop began: the answer goes on to the loop.
            }
        }

        private sealed class HoldingReader<T>(IAsyncStreamReader<T> inner, HoldUntilStop owner) : IAsyncStreamReader<T>
        {
            public T Current => inner.Current;

            public async Task<bool> MoveNext(CancellationToken cancellationToken)
            {
                var more = await inner.MoveNext(cancellationToken);
                if (more)
                {
                    await owner.HoldIfDueAsync(inner.Current, cancellationToken);
                }

                return more;
            }
        }
    }
}
