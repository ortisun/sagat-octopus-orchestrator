using System.Text.Json;
using System.Text.Json.Nodes;
using Grpc.Core;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workers;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

public sealed record Charge(int Amount);

public sealed record Receipt(int Total);

public sealed class WorkerHostTests : IAsyncLifetime
{
    private readonly string name = "test-worker-" + Guid.NewGuid().ToString("N");
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

    private static TaskLease LeaseFor(string activity, Action<TaskLease>? change = null) =>
        Leases.Lease(lease =>
        {
            lease.ActivityName = activity;
            lease.StepId = activity;
            lease.TaskId = $"run-1/{activity}/1";
            change?.Invoke(lease);
        });

    [Fact]
    public async Task AWorkerRegistersItsActivitiesAndTheNewestProtocolItSpeaksWithTheTokenAsMetadata()
    {
        var host = Host(o => o.Worker.MaxInflight = 7);
        host.RegisterActivity<JsonNode?, int>("b.second", (_, _, _) => Task.FromResult(1))
            .RegisterCompensation<JsonNode?, int>("a.undo", (_, _, _) => Task.FromResult(1))
            .RegisterReconciler<JsonNode?>("c.lookup", (_, _, _) => Task.FromResult(Verdict.Unknown()));
        await host.StartAsync();

        var register = Assert.Single(State.Of("RegisterWorker"));
        var request = (RegisterWorkerRequest)register.Request;
        Assert.Equal(name, request.WorkerName);
        Assert.Equal("payments", request.Namespace);
        Assert.Equal(["csharp"], request.Capabilities.Languages);
        Assert.Equal(["a.undo", "b.second", "c.lookup"], request.Capabilities.Activities);
        Assert.Empty(request.Capabilities.Pools);
        Assert.Equal(7, request.Capabilities.MaxInflight);
        Assert.Equal("1.5", request.Capabilities.ProtocolVersion);
        Assert.Equal("Bearer tok", register.Headers.GetValue("authorization"));
        Assert.Equal("1.2", host.ProtocolVersion);
        Assert.True(host.IsRunning);
    }

    [Fact]
    public async Task Protocol15EchoesScopedIdentityAndProtocol14OmitsIt()
    {
        var instance = new SagatOctopusOrchestrator.Protocol.V1.ExecutionInstanceIdentity { DefinitionNodeId = "charge" };
        instance.InstancePath.Add(new SagatOctopusOrchestrator.Protocol.V1.ScopePathSegment { ScopeNodeId = "lines", IterationIndex = 7 });
        State.AcceptVersions("1.5", "1.4");

        var current = Host();
        ActivityExecutionContext? currentContext = null;
        current.RegisterActivity<JsonNode?, int>("charge", (_, context, _) =>
        {
            currentContext = context;
            return Task.FromResult(1);
        });
        State.Lease(Lease(lease =>
        {
            lease.Instance = instance;
            lease.InputJson = """{"input":{"order":7},"item":{"sku":"one"},"index":7,"steps":{}}""";
        }));
        await current.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the scoped completion");
        Assert.Equal(instance, ((CompleteStepRequest)State.Of("CompleteStep")[0].Request).Instance);
        Assert.Equal("charge", currentContext?.Instance?.DefinitionNodeId);
        Assert.Equal(new ExecutionScope("lines", 7), Assert.Single(currentContext!.Instance!.InstancePath));
        Assert.Equal("one", currentContext.Item?.GetProperty("sku").GetString());
        Assert.Equal((uint)7, currentContext.Index);
        await current.StopAsync();

        var legacy = Host(options => options.Worker.ProtocolVersion = "1.4");
        ActivityExecutionContext? legacyContext = null;
        legacy.RegisterActivity<JsonNode?, int>("charge", (_, context, _) =>
        {
            legacyContext = context;
            return Task.FromResult(1);
        });
        State.Lease(Lease(lease =>
        {
            lease.Instance = instance;
            lease.InputJson = """{"input":{"order":7},"item":{"sku":"one"},"index":7,"steps":{}}""";
        }));
        await legacy.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "the legacy completion");
        Assert.Null(((CompleteStepRequest)State.Of("CompleteStep")[1].Request).Instance);
        Assert.Null(legacyContext?.Instance);
        Assert.Null(legacyContext?.Item);
        Assert.Null(legacyContext?.Index);
    }

    [Fact]
    public async Task ATaskIsRunWithItsContextAndCompletedOnceWithAKeyThatIsTheSameOnEveryRetry()
    {
        ActivityExecutionContext? seen = null;
        Charge? received = null;
        DateTimeOffset expires = default;
        var host = Host();
        host.RegisterActivity<Charge, Receipt>("charge", (input, context, _) =>
        {
            seen = context;
            received = input;
            expires = context.Deadline;
            return Task.FromResult(new Receipt(input.Amount * 2));
        });
        State.Fail("CompleteStep", new Scripted(StatusCode.Unavailable), new Scripted(StatusCode.ResourceExhausted, "RATE_LIMITED", 1));
        var lease = Lease();
        State.Lease(lease);
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 3, "three attempts to report");

        Assert.Equal(new Charge(5), received);
        Assert.NotNull(seen);
        Assert.Equal("run-1", seen.RunId);
        Assert.Equal("charge", seen.Node);
        Assert.Equal("charge", seen.StepId);
        Assert.Equal("charge", seen.ActivityName);
        Assert.Equal("run-1/charge/1", seen.TaskId);
        Assert.Equal(1, seen.Attempt);
        Assert.Equal(1, seen.Generation);
        Assert.Equal("run-1:charge", seen.IdempotencyKey);
        Assert.Equal(TaskKind.Forward, seen.TaskKind);
        Assert.Equal(Leases.Traceparent, seen.Traceparent);
        Assert.Equal(DateTimeOffset.FromUnixTimeMilliseconds(lease.ExpiresAtEpochMs), expires);
        Assert.Equal("""{"ok":true}""", seen.Steps["earlier"].GetRawText());
        Assert.Null(seen.CancellationReason);

        var requests = State.Of("CompleteStep").Select(c => (CompleteStepRequest)c.Request).ToList();
        Assert.Equal(["done-run-1/charge/1"], requests.Select(r => r.IdempotencyKey).Distinct());
        var last = requests[2];
        Assert.Equal("""{"total":10}""", last.OutputJson);
        Assert.Equal("lt-secret-lease-token", last.LeaseToken);
        Assert.Equal(1, last.Generation);
        Assert.Equal(Leases.Traceparent, last.Traceparent);
        await Eventually.UntilAsync(() => meters.Total("sagat.worker.tasks.completed") == 1, "the completed metric");
        Assert.Equal(1, meters.Total("sagat.worker.tasks.taken"));
        Assert.Equal(1, meters.Total("sagat.worker.activity.duration"));
    }

    [Fact]
    public async Task EveryReportCarriesTheTraceContextOfTheRunInTheMessageAndInTheMetadata()
    {
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20));
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, ct) =>
        {
            await Task.Delay(100, ct);
            await context.GetSecretsAsync(ct);
            return 1;
        });
        State.Secrets.Add(new ResolvedSecret { Name = "k", Value = Google.Protobuf.ByteString.CopyFromUtf8("v"), Version = 1 });
        State.Lease(Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        foreach (var method in new[] { "Heartbeat", "CompleteStep", "ResolveSecrets" })
        {
            var call = State.Of(method)[0];
            Assert.Equal(Leases.Traceparent, call.Headers.GetValue("traceparent"));
        }

        Assert.Equal(Leases.Traceparent, ((CompleteStepRequest)State.Of("CompleteStep")[0].Request).Traceparent);
        Assert.Equal(Leases.Traceparent, ((ResolveSecretsRequest)State.Of("ResolveSecrets")[0].Request).Traceparent);
        Assert.Equal(string.Empty, State.Of("PollTasks")[0].Headers.GetValue("traceparent") ?? string.Empty);
    }

    [Fact]
    public async Task AFailureOnPurposeCarriesItsCodeAndWhetherItMayBeRetriedAnyOtherErrorSaysOnlyItsType()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("permanent", (_, _, _) => throw ActivityFailure.Permanent("INSUFFICIENT_FUNDS", "not enough"));
        host.RegisterActivity<JsonNode?, int>("unknown", (_, _, _) => throw ActivityFailure.OutcomeUnknown("the provider timed out"));
        host.RegisterActivity<JsonNode?, int>("boom", (_, _, _) => throw new InvalidOperationException("secret card 4111 leaked in the message"));
        host.RegisterActivity<JsonNode?, int>("badcode", (_, _, _) => throw new ActivityFailure("not a code!", "x", retryable: false));
        host.RegisterActivity<JsonNode?, int>("retryable", (_, _, _) => throw ActivityFailure.Retryable("PROVIDER_BUSY", "try later"));
        host.RegisterActivity<JsonNode?, int>("long", (_, _, _) => throw ActivityFailure.Permanent("TOO_LONG", new string('m', 2000)));
        State.Lease(new[] { "permanent", "unknown", "boom", "badcode", "retryable", "long" }.Select((n, i) => LeaseFor(n, l => l.TaskId = $"run-{i}/{n}/1")).ToArray());
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 6, "six failures");
        var by = State.Of("FailStep").Select(c => (FailStepRequest)c.Request).ToDictionary(r => r.StepId);
        Assert.Equal(("INSUFFICIENT_FUNDS", false, "not enough"), (by["permanent"].ErrorCode, by["permanent"].Retryable, by["permanent"].SafeMessage));
        Assert.Equal(("OUTCOME_UNKNOWN", false), (by["unknown"].ErrorCode, by["unknown"].Retryable));
        Assert.Equal(("ACTIVITY_ERROR", true, "InvalidOperationException"), (by["boom"].ErrorCode, by["boom"].Retryable, by["boom"].SafeMessage));
        Assert.Equal("WORKER_ERROR", by["badcode"].ErrorCode);
        Assert.Equal(("PROVIDER_BUSY", true), (by["retryable"].ErrorCode, by["retryable"].Retryable));
        Assert.Equal(512, by["long"].SafeMessage.Length);
        Assert.Equal("failed-run-0/permanent/1", by["permanent"].IdempotencyKey);
        Assert.Equal(Leases.Traceparent, by["permanent"].Traceparent);
    }

    [Theory]
    [InlineData(StatusCode.Aborted, "STALE_ATTEMPT")]
    [InlineData(StatusCode.Aborted, null)]
    [InlineData(StatusCode.FailedPrecondition, null)]
    public async Task AStaleReportIsDroppedAndNeverRetried(StatusCode code, string? reason)
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("CompleteStep", new Scripted(code, reason));
        State.Lease(Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => meters.Total("sagat.worker.tasks.stale") == 1, "the stale metric");
        await Task.Delay(100);
        Assert.Single(State.Of("CompleteStep"));
        Assert.Equal(0, meters.Total("sagat.worker.tasks.completed"));
    }

    [Fact]
    public async Task AReportRefusedWithoutAnyRetryableCodeIsNotRepeatedAndIsCountedAsDropped()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("CompleteStep", new Scripted(StatusCode.Aborted, "IDEMPOTENCY_KEY_REUSED"));
        State.Lease(Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => meters.Total("sagat.worker.tasks.dropped") == 1, "the dropped metric");
        Assert.Single(State.Of("CompleteStep"));
    }

    [Fact]
    public async Task AReportThatKeepsFailingIsRepeatedAtMostFiveTimes()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("CompleteStep", Enumerable.Repeat(new Scripted(StatusCode.Unavailable), 20).ToArray());
        State.Lease(Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => meters.Total("sagat.worker.tasks.dropped") == 1, "the dropped metric");
        Assert.Equal(5, State.Of("CompleteStep").Count);
    }

    [Fact]
    public async Task AnInputTheHandlerCannotTakeAnActivityNobodyRegisteredAndAnOutputTooLargeAreFailuresForGood()
    {
        var host = Host();
        host.RegisterActivity<Charge, int>("typed", (_, _, _) => Task.FromResult(1));
        host.RegisterActivity<JsonNode?, string>("huge", (_, _, _) => Task.FromResult(new string('x', 300 * 1024)));
        host.RegisterActivity<JsonNode?, object>("bad-number", (_, _, _) => Task.FromResult<object>(new { n = double.NaN }));
        host.RegisterActivity<JsonNode?, string>("bad-text", (_, _, _) => Task.FromResult("lone \ud800 surrogate"));
        host.RegisterActivity<Charge, int>("junk", (_, _, _) => Task.FromResult(1));
        host.RegisterActivity<JsonNode?, string>("exactly", (_, _, _) => Task.FromResult(new string('x', (256 * 1024) - 2)));
        State.Lease(
            LeaseFor("typed", l => l.InputJson = Leases.Document(JsonValue.Create("not an order"), "typed")),
            LeaseFor("huge"),
            LeaseFor("nobody"),
            LeaseFor("bad-number"),
            LeaseFor("bad-text"),
            LeaseFor("junk", l => l.InputJson = "{nope"),
            LeaseFor("exactly"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 6 && State.Of("CompleteStep").Count == 1, "six failures and a completion");
        string? Code(string step) => State.Of("FailStep").Select(c => (FailStepRequest)c.Request).FirstOrDefault(r => r.StepId == step)?.ErrorCode;
        Assert.Equal("INPUT_INVALID", Code("typed"));
        Assert.Equal("PAYLOAD_TOO_LARGE", Code("huge"));
        Assert.Equal("NO_HANDLER", Code("nobody"));
        Assert.Equal("OUTPUT_INVALID", Code("bad-number"));
        Assert.Equal("OUTPUT_INVALID", Code("bad-text"));
        Assert.Equal("INPUT_INVALID", Code("junk"));
        Assert.All(State.Of("FailStep"), c => Assert.False(((FailStepRequest)c.Request).Retryable));
        Assert.Equal("exactly", ((CompleteStepRequest)State.Of("CompleteStep")[0].Request).StepId);
    }

    [Fact]
    public async Task TheHeartbeatStartsBeforeTheHandlerEndsCarriesTheStepIdentityAndMovesTheDeadline()
    {
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20));
        var deadlines = new List<DateTimeOffset>();
        host.RegisterActivity<JsonNode?, JsonNode>("slow", async (_, context, ct) =>
        {
            deadlines.Add(context.Deadline);
            await Task.Delay(300, ct);
            deadlines.Add(context.Deadline);
            return new JsonObject { ["ok"] = true };
        });
        State.HeartbeatExpiresMs = DateTimeOffset.UtcNow.AddSeconds(120).ToUnixTimeMilliseconds();
        State.Lease(LeaseFor("slow"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        var beats = State.Of("Heartbeat");
        Assert.True(beats.Count >= 3, $"{beats.Count} beats");
        var first = (HeartbeatRequest)beats[0].Request;
        Assert.Equal(("run-1", "slow", 1, "lt-secret-lease-token", "w-1"), (first.RunId, first.StepId, first.Attempt, first.LeaseToken, first.WorkerId));
        Assert.True(deadlines[1] > deadlines[0], "the lease was extended");
    }

    [Fact]
    public async Task AHeartbeatThatFailsWithATransportErrorIsTriedAgainOnTheNextBeatAndTheLeaseIsKept()
    {
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20));
        host.RegisterActivity<JsonNode?, int>("slow", async (_, _, ct) =>
        {
            await Task.Delay(900, ct);
            return 1;
        });
        State.Fail("Heartbeat", new Scripted(StatusCode.Unavailable), new Scripted(StatusCode.Unavailable), new Scripted(StatusCode.Internal));
        State.Lease(LeaseFor("slow"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        Assert.True(State.Of("Heartbeat").Count >= 4);
        Assert.Empty(State.Of("FailStep"));
    }

    [Fact]
    public async Task ACancellationReachesTheActivityThroughItsTokenWithTheReasonRunCancelledAndItEndsHoweverItLikes()
    {
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20));
        string? reason = null;
        host.RegisterActivity<JsonNode?, JsonNode>("long", async (_, context, ct) =>
        {
            await Task.Delay(Timeout.Infinite, ct).ContinueWith(_ => { }, TaskScheduler.Default);
            reason = context.CancellationReason;
            return new JsonObject { ["stopped"] = true };
        });
        State.CancelRequested = true;
        State.Lease(LeaseFor("long"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        Assert.Equal("run-cancelled", reason);
        Assert.Equal("""{"stopped":true}""", ((CompleteStepRequest)State.Of("CompleteStep")[0].Request).OutputJson);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task AnActivityThatRaisesItsCancellationIsFailedForGoodAsCancelled(bool asSagatError)
    {
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20));
        host.RegisterActivity<JsonNode?, int>("long", async (_, _, ct) =>
        {
            await Task.Delay(Timeout.Infinite, ct).ContinueWith(_ => { }, TaskScheduler.Default);
            return asSagatError
                ? throw new SagatException(SagatErrorCode.Cancelled, "stopped")
                : ct.IsCancellationRequested ? throw new OperationCanceledException(ct) : 1;
        });
        State.CancelRequested = true;
        State.Lease(LeaseFor("long"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 1, "the failure");
        var fail = (FailStepRequest)State.Of("FailStep")[0].Request;
        Assert.Equal(("CANCELLED", false), (fail.ErrorCode, fail.Retryable));
    }

    [Fact]
    public async Task ACancellationThatNobodyAskedForIsAnOrdinaryError()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("odd", (_, _, _) => throw new OperationCanceledException("a token of its own"));
        State.Lease(LeaseFor("odd"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 1, "the failure");
        var fail = (FailStepRequest)State.Of("FailStep")[0].Request;
        Assert.Equal(("ACTIVITY_ERROR", true, "OperationCanceledException"), (fail.ErrorCode, fail.Retryable, fail.SafeMessage));
    }

    [Theory]
    [InlineData(StatusCode.Aborted, "STALE_ATTEMPT")]
    [InlineData(StatusCode.FailedPrecondition, null)]
    public async Task ALeaseLostDuringTheActivityStopsItWithTheReasonLeaseLostAndItsResultIsNeverReported(StatusCode code, string? reason)
    {
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20));
        string? seen = null;
        host.RegisterActivity<JsonNode?, JsonNode>("slow", async (_, context, ct) =>
        {
            await Task.Delay(Timeout.Infinite, ct).ContinueWith(_ => { }, TaskScheduler.Default);
            seen = context.CancellationReason;
            return new JsonObject { ["late"] = true };
        });
        State.Fail("Heartbeat", new Scripted(code, reason));
        State.Lease(LeaseFor("slow"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => meters.Total("sagat.worker.tasks.stale") == 1, "the stale metric");
        Assert.Equal("lease-lost", seen);
        Assert.Empty(State.Of("CompleteStep"));
        Assert.Empty(State.Of("FailStep"));
    }

    [Fact]
    public async Task ACompensationSeesItsKindAndTheEffectItUndoesAndAReconcilerAnswersExactlyAVerdict()
    {
        var seen = new List<(TaskKind Kind, string Target, string TargetKey, string Key)>();
        var host = Host();
        host.RegisterCompensation<JsonNode?, JsonNode>("limit.release", (_, context, _) =>
        {
            seen.Add((context.TaskKind, context.TargetNode, context.TargetIdempotencyKey, context.IdempotencyKey));
            return Task.FromResult<JsonNode>(new JsonObject { ["released"] = true });
        });
        host.RegisterReconciler<JsonNode?>("limit.lookup", (_, _, _) => Task.FromResult(Verdict.Applied(new { id = "r-9" })));
        host.RegisterReconciler<JsonNode?>("limit.bare", (_, _, _) => Task.FromResult(Verdict.Applied()));
        host.RegisterReconciler<JsonNode?>("limit.none", (_, _, _) => Task.FromResult(Verdict.NotApplied()));
        host.RegisterReconciler<JsonNode?>("limit.unknown", (_, _, _) => Task.FromResult(Verdict.Unknown()));
        host.RegisterReconciler<JsonNode?>("limit.bad", (_, _, _) => Task.FromResult<Verdict>(null!));
        State.Lease(
            LeaseFor("limit.release", l =>
            {
                l.StepId = "reserve~c";
                l.TaskKind = "COMPENSATION";
                l.TargetNode = "reserve";
                l.TargetIdempotencyKey = "run-1:reserve";
                l.IdempotencyKey = "compensation:c1:run-1";
            }),
            LeaseFor("limit.lookup", l => { l.StepId = "a~r"; l.TaskKind = "RECONCILIATION"; l.TargetNode = "a"; l.TargetIdempotencyKey = "k"; }),
            LeaseFor("limit.bare", l => { l.StepId = "bare~r"; l.TaskKind = "RECONCILIATION"; }),
            LeaseFor("limit.none", l => { l.StepId = "b~r"; l.TaskKind = "RECONCILIATION"; }),
            LeaseFor("limit.unknown", l => { l.StepId = "u~r"; l.TaskKind = "RECONCILIATION"; }),
            LeaseFor("limit.bad", l => { l.StepId = "c~r"; l.TaskKind = "RECONCILIATION"; }));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 5 && State.Of("FailStep").Count == 1, "every report");
        Assert.Equal([(TaskKind.Compensation, "reserve", "run-1:reserve", "compensation:c1:run-1")], seen);
        string? Out(string step) => State.Of("CompleteStep").Select(c => (CompleteStepRequest)c.Request).FirstOrDefault(r => r.StepId == step)?.OutputJson;
        Assert.Equal("""{"effect":"applied","output":{"id":"r-9"}}""", Out("a~r"));
        Assert.Equal("""{"effect":"applied"}""", Out("bare~r"));
        Assert.Equal("""{"effect":"not_applied"}""", Out("b~r"));
        Assert.Equal("""{"effect":"unknown"}""", Out("u~r"));
        Assert.Equal("""{"released":true}""", Out("reserve~c"));
        Assert.Equal("RECONCILIATION_ANSWER_INVALID", ((FailStepRequest)State.Of("FailStep")[0].Request).ErrorCode);
    }

    [Fact]
    public async Task AnOutputOfNullIsTheTextNullAndNoOutputIsAnEmptyString()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, string?>("nothing", (_, _, _) => Task.FromResult<string?>(null));
        host.RegisterActivity<JsonNode?>("void", (_, _, _) => Task.CompletedTask);
        State.Lease(LeaseFor("nothing"), LeaseFor("void"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "both completions");
        string Out(string step) => State.Of("CompleteStep").Select(c => (CompleteStepRequest)c.Request).First(r => r.StepId == step).OutputJson;
        Assert.Equal("null", Out("nothing"));
        Assert.Equal(string.Empty, Out("void"));
    }

    [Fact]
    public async Task TheHostRunsAtTheVersionTheServerAgreedToAgainst10ItSendsNoStepIdentityAndCannotResolveSecrets()
    {
        State.AcceptedVersion = "1";
        var host = Host(o =>
        {
            o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20);
            o.Worker.ProtocolVersion = "1.9";
        });
        object? secretError = null;
        host.RegisterActivity<JsonNode?, int>("slow", async (_, context, ct) =>
        {
            await Task.Delay(80, ct);
            try
            {
                await context.GetSecretsAsync(ct);
            }
            catch (SagatException error)
            {
                secretError = error;
            }

            return 1;
        });
        State.Lease(LeaseFor("slow"));
        await host.StartAsync();
        Assert.Equal("1.9", ((RegisterWorkerRequest)State.Of("RegisterWorker")[0].Request).Capabilities.ProtocolVersion);
        Assert.Equal("1", host.ProtocolVersion);
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        var beat = (HeartbeatRequest)State.Of("Heartbeat")[0].Request;
        Assert.Equal((string.Empty, string.Empty, 0), (beat.RunId, beat.StepId, beat.Attempt));
        var error = Assert.IsType<SagatException>(secretError);
        Assert.Equal(SagatErrorCode.UnsupportedProtocol, error.Code);
        Assert.Empty(State.Of("ResolveSecrets"));
    }

    [Fact]
    public async Task Against11TheHeartbeatCarriesTheIdentityButSecretsAreStillUnavailable()
    {
        State.AcceptedVersion = "1.1";
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20));
        host.RegisterActivity<JsonNode?, int>("slow", async (_, context, ct) =>
        {
            await Task.Delay(80, ct);
            var error = await Assert.ThrowsAsync<SagatException>(() => context.GetSecretsAsync(ct));
            Assert.Equal(SagatErrorCode.UnsupportedProtocol, error.Code);
            return 1;
        });
        State.Lease(LeaseFor("slow"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        Assert.Equal("slow", ((HeartbeatRequest)State.Of("Heartbeat")[0].Request).StepId);
    }

    [Fact]
    public async Task SecretsAreResolvedOncePerAttemptOnFirstUseAsAReadOnlyMapAndClearedWhenTheHandlerReturns()
    {
        var host = Host();
        string text = string.Empty;
        ActivitySecrets? kept = null;
        host.RegisterActivity<JsonNode?, JsonNode>("pay", async (_, context, ct) =>
        {
            var first = await context.GetSecretsAsync(ct);
            var second = await context.GetSecretsAsync(ct);
            Assert.Same(first, second);
            text = first.GetText("stripe-key");
            kept = first;
            Assert.Equal("sk_live_x"u8.ToArray(), first.Get("stripe-key").ToArray());
            Assert.Throws<SagatException>(() => first.Get("missing"));
            return new JsonObject { ["names"] = new JsonArray([.. first.Names.Select(n => (JsonNode?)JsonValue.Create(n))]) };
        });
        State.Secrets.Add(new ResolvedSecret { Name = "stripe-key", Value = Google.Protobuf.ByteString.CopyFromUtf8("sk_live_x"), Version = 3 });
        State.Lease(LeaseFor("pay"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        Assert.Equal("sk_live_x", text);
        Assert.Single(State.Of("ResolveSecrets"));
        var resolve = (ResolveSecretsRequest)State.Of("ResolveSecrets")[0].Request;
        Assert.Equal(("run-1", "pay", 1, 1), (resolve.RunId, resolve.StepId, resolve.Attempt, (int)resolve.Generation));
        Assert.Equal("""{"names":["stripe-key"]}""", ((CompleteStepRequest)State.Of("CompleteStep")[0].Request).OutputJson);
        Assert.DoesNotContain("sk_live", ((CompleteStepRequest)State.Of("CompleteStep")[0].Request).OutputJson, StringComparison.Ordinal);
        Assert.Empty(kept!.Names);
    }

    [Fact]
    public async Task ASecretTheStepDoesNotDeclareIsRefusedByTheServerWithTheCanonicalError()
    {
        var host = Host();
        SagatException? seen = null;
        host.RegisterActivity<JsonNode?, int>("pay", async (_, context, ct) =>
        {
            try
            {
                await context.GetSecretsAsync(ct);
            }
            catch (SagatException error)
            {
                seen = error;
            }

            return 1;
        });
        State.Fail("ResolveSecrets", new Scripted(StatusCode.PermissionDenied, "SECRET_NOT_USABLE"));
        State.Lease(LeaseFor("pay"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        Assert.Equal(SagatErrorCode.SecretNotUsable, seen!.Code);
        Assert.Equal("PERMISSION_DENIED", seen.GrpcStatus);
    }

    [Fact]
    public async Task ASessionThatIsGoneIsRegisteredAgainAndTheWorkerGoesOn()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await host.StartAsync();
        State.Fail("PollTasks", new Scripted(StatusCode.Unauthenticated));
        await Eventually.UntilAsync(() => State.Registrations == 2, "a second registration");
        State.Lease(Lease());
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the task after it");
        Assert.True(meters.Total("sagat.worker.reconnects") >= 1);
    }

    [Fact]
    public async Task Protocol14StreamsTwoFramesDispatchesBeforeEofAndCancelsTheIdleReceiveOnStop()
    {
        State.AcceptedVersion = "1.4";
        State.StreamFrameSize = 1;
        State.HoldStreamOpen = true;
        State.Lease(
            Lease(l => { l.TaskId = "run-1/charge/1"; l.RunId = "run-1"; }),
            Lease(l => { l.TaskId = "run-2/charge/1"; l.RunId = "run-2"; }));
        var host = Host(o => o.Worker.MaxInflight = 3);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "both streamed tasks before EOF");

        var stream = Assert.Single(State.Of("StreamTasks"));
        Assert.Equal(3, ((PollTasksRequest)stream.Request).MaxTasks);
        Assert.Empty(State.Of("PollTasks"));
        var clock = System.Diagnostics.Stopwatch.StartNew();
        await host.StopAsync(TimeSpan.FromSeconds(1));
        Assert.True(clock.Elapsed < TimeSpan.FromSeconds(2), $"stop took {clock.Elapsed}");
        Assert.True(State.StreamCancelled);
    }

    [Fact]
    public async Task AStreamErrorAfterAFramePreservesTheHandlerAndReconnectsOnlyReceiving()
    {
        State.AcceptedVersion = "1.4";
        State.StreamFailureAfterFirst = new Scripted(StatusCode.Unavailable);
        State.HoldStreamOpen = true;
        State.Lease(Lease());
        var host = Host(o => o.Worker.MaxInflight = 2);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the partial delivery");
        await Eventually.UntilAsync(() => State.Of("StreamTasks").Count >= 2, "a receive reconnect");
        Assert.Empty(State.Of("PollTasks"));
    }

    [Fact]
    public async Task AutoFallsBackOnlyWhenStreamingIsUnimplemented()
    {
        State.AcceptedVersion = "1.4";
        State.Fail("StreamTasks", new Scripted(StatusCode.Unimplemented));
        State.Lease(Lease());
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the task through polling fallback");
        Assert.Single(State.Of("StreamTasks"));
        Assert.NotEmpty(State.Of("PollTasks"));
    }

    [Fact]
    public async Task ExplicitStreamingReportsUnimplementedWithoutPolling()
    {
        State.AcceptedVersion = "1.4";
        State.FailAlways("StreamTasks", new Scripted(StatusCode.Unimplemented));
        var logs = new CapturingLoggerProvider();
        using var factory = LoggerFactory.Create(builder => builder.AddProvider(logs));
        var host = Host(o => o.Worker.Transport = WorkerTransport.Stream, factory);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(() => logs.Lines.Any(line => line.Contains("UNSUPPORTED_PROTOCOL", StringComparison.Ordinal)), "the unsupported diagnostic");
        Assert.Empty(State.Of("PollTasks"));
    }

    [Fact]
    public async Task ExplicitStreamingRefusesAnOlderNegotiatedServerBeforeReceiving()
    {
        State.AcceptedVersion = "1.3";
        var host = Host(o => o.Worker.Transport = WorkerTransport.Stream);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        var error = await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        Assert.Equal(SagatErrorCode.UnsupportedProtocol, error.Code);
        Assert.Empty(State.Of("StreamTasks"));
        Assert.Empty(State.Of("PollTasks"));
    }

    [Fact]
    public async Task ExplicitStreamingNeverPollsAfterReregistrationNegotiatesAnOlderProtocol()
    {
        State.AcceptVersions("1.4", "1.3");
        State.Fail("StreamTasks", new Scripted(StatusCode.Unauthenticated));
        var logs = new CapturingLoggerProvider();
        using var factory = LoggerFactory.Create(builder => builder.AddProvider(logs));
        var host = Host(o => o.Worker.Transport = WorkerTransport.Stream, factory);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Registrations >= 2, "the replacement registration");
        await Eventually.UntilAsync(
            () => logs.Lines.Any(line => line.Contains("UNSUPPORTED_PROTOCOL", StringComparison.Ordinal)),
            "the downgrade diagnostic");
        await Task.Delay(100);

        Assert.Empty(State.Of("PollTasks"));
        Assert.Single(State.Of("StreamTasks"));
    }

    [Fact]
    public async Task AStreamOverCapacityRunsOnlyTheCreditItWasGiven()
    {
        State.AcceptedVersion = "1.4";
        State.IgnoreMax = true;
        State.HoldStreamOpen = true;
        State.Lease(
            Lease(l => { l.TaskId = "run-1/charge/1"; l.RunId = "run-1"; }),
            Lease(l => { l.TaskId = "run-2/charge/1"; l.RunId = "run-2"; }));
        var host = Host(o => o.Worker.MaxInflight = 1);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "one bounded task");
        await Task.Delay(100);
        Assert.Single(State.Of("CompleteStep"));
        Assert.Equal(1, ((PollTasksRequest)State.Of("StreamTasks")[0].Request).MaxTasks);
    }

    [Fact]
    public async Task AStreamRequestsAtMost64Credits()
    {
        State.AcceptedVersion = "1.4";
        State.HoldStreamOpen = true;
        var host = Host(o => o.Worker.MaxInflight = 100);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("StreamTasks").Count == 1, "the receive window");
        Assert.Equal(64, ((PollTasksRequest)State.Of("StreamTasks")[0].Request).MaxTasks);
    }

    [Fact]
    public async Task AStreamCannotExceedTheAggregateEightMiBInputBudget()
    {
        State.AcceptedVersion = "1.4";
        State.StreamFrameSize = 1;
        var document = "{\"input\":\"" + new string('x', 4 * 1024 * 1024) + "\"}";
        State.Lease(
            Lease(l => { l.TaskId = "run-1/charge/1"; l.RunId = "run-1"; l.InputJson = document; }),
            Lease(l => { l.TaskId = "run-2/charge/1"; l.RunId = "run-2"; l.InputJson = document; }));
        var handled = 0;
        var host = Host(o => o.Worker.MaxInflight = 2);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) =>
        {
            Interlocked.Increment(ref handled);
            return Task.FromResult(1);
        });

        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the first frame within the aggregate budget");
        await Task.Delay(150);
        Assert.Equal(1, Volatile.Read(ref handled));
        Assert.Single(State.Of("CompleteStep"));
        Assert.Empty(State.Of("FailStep"));
    }

    [Fact]
    public async Task APollRefusedUnavailableIsFollowedByAnotherAfterThePauseAndTheHostGoesOn()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("PollTasks", new Scripted(StatusCode.Unavailable), new Scripted(StatusCode.Unavailable), new Scripted(StatusCode.ResourceExhausted, null, 20));
        State.Lease(Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the task after the failed polls");
        Assert.True(State.Of("PollTasks").Count >= 4);
    }

    [Fact]
    public async Task AnEmptyPollIsFollowedByAWaitNeverByAnotherPollAtOnce()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await host.StartAsync();
        await Task.Delay(400);
        var polls = State.Of("PollTasks").Count;
        Assert.InRange(polls, 2, 10);
    }

    [Fact]
    public async Task NoMoreTasksAreTakenThanTheHostHasRoomForAndStopWaitsForTheOnesInFlight()
    {
        var host = Host(o => o.Worker.MaxInflight = 2);
        var running = 0;
        var peak = 0;
        host.RegisterActivity<JsonNode?, int>("charge", async (_, _, ct) =>
        {
            var now = Interlocked.Increment(ref running);
            int seenPeak;
            while ((seenPeak = Volatile.Read(ref peak)) < now && Interlocked.CompareExchange(ref peak, now, seenPeak) != seenPeak)
            {
            }

            await Task.Delay(100, ct);
            Interlocked.Decrement(ref running);
            return 1;
        });
        State.Lease(Enumerable.Range(1, 4).Select(i => Lease(l => { l.TaskId = $"run-{i}/charge/1"; l.RunId = $"run-{i}"; })).ToArray());
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("PollTasks").Count > 0, "a poll");
        Assert.True(((PollTasksRequest)State.Of("PollTasks")[0].Request).MaxTasks <= 2);
        await host.StopAsync(TimeSpan.FromSeconds(5));
        Assert.True(peak <= 2, $"peak {peak}");
        Assert.Equal(0, Volatile.Read(ref running));
        Assert.False(host.IsRunning);
    }

    [Fact]
    public async Task ManyTasksAtOnceAreAllRunAndReportedOnceEach()
    {
        var host = Host(o => o.Worker.MaxInflight = 10);
        host.RegisterActivity<JsonNode?, int>("charge", async (_, _, ct) =>
        {
            await Task.Delay(20, ct);
            return 1;
        });
        State.Lease(Enumerable.Range(1, 60).Select(i => Lease(l => { l.TaskId = $"run-{i}/charge/1"; l.RunId = $"run-{i}"; })).ToArray());
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 60, "sixty completions", 20_000);
        var keys = State.Of("CompleteStep").Select(c => ((CompleteStepRequest)c.Request).IdempotencyKey).Distinct().Count();
        Assert.Equal(60, keys);
        Assert.All(State.Of("PollTasks"), c => Assert.InRange(((PollTasksRequest)c.Request).MaxTasks, 1, 10));
    }

    [Fact]
    public async Task StopWhileAPollIsInFlightDoesNotWaitOutThePauseAnEmptyAnswerAsksFor()
    {
        State.PollDelay = TimeSpan.FromMilliseconds(150);
        State.PollRetryAfterMs = 10_000;
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("PollTasks").Count == 1, "the first poll");
        var clock = System.Diagnostics.Stopwatch.StartNew();
        await host.StopAsync(TimeSpan.FromSeconds(1));
        Assert.True(clock.ElapsedMilliseconds < 1500, $"stop took {clock.ElapsedMilliseconds} ms");
    }

    [Fact]
    public async Task StopCancelsWhatIsLeftAfterTheTimeoutWithTheReasonShutdown()
    {
        var host = Host();
        var started = new TaskCompletionSource();
        string? reason = null;
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, ct) =>
        {
            started.SetResult();
            await Task.Delay(Timeout.Infinite, ct).ContinueWith(_ => { }, TaskScheduler.Default);
            reason = context.CancellationReason;
            return 1;
        });
        State.Lease(Lease());
        await host.StartAsync();
        await started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        await host.StopAsync(TimeSpan.FromMilliseconds(100));
        Assert.Equal("shutdown", reason);
        Assert.False(host.IsRunning);
    }

    [Fact]
    public async Task StopEndsAtOnceWhenTheCallersTokenIsCancelledAndTheHostIsRestartable()
    {
        var host = Host();
        var started = new TaskCompletionSource();
        host.RegisterActivity<JsonNode?, int>("charge", async (_, _, ct) =>
        {
            started.TrySetResult();
            await Task.Delay(Timeout.Infinite, ct).ContinueWith(_ => { }, TaskScheduler.Default);
            return 1;
        });
        State.Lease(Lease());
        await host.StartAsync();
        await started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        using var impatient = new CancellationTokenSource(TimeSpan.FromMilliseconds(100));
        var clock = System.Diagnostics.Stopwatch.StartNew();
        await host.StopAsync(TimeSpan.FromSeconds(30), impatient.Token);
        Assert.True(clock.ElapsedMilliseconds < 5000, $"stop took {clock.ElapsedMilliseconds} ms");

        await host.StartAsync();
        Assert.Equal(2, State.Registrations);
        Assert.True(host.IsRunning);
    }

    [Fact]
    public async Task ARegistrationThatNeverLeftIsRepeatedAndOneThatWasRefusedForGoodIsNot()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("RegisterWorker", new Scripted(StatusCode.Unavailable), new Scripted(StatusCode.Unavailable));
        await host.StartAsync();
        Assert.Equal(3, State.Registrations);
        Assert.True(host.IsRunning);
    }

    [Fact]
    public async Task RegistrationIsRefusedClearlyAnUnsupportedProtocolAnUnboundIdentity()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("RegisterWorker", new Scripted(StatusCode.InvalidArgument, "UNSUPPORTED_PROTOCOL"));
        var unsupported = await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        Assert.Equal(SagatErrorCode.UnsupportedProtocol, unsupported.Code);
        Assert.False(unsupported.Retryable);
        Assert.Equal("INVALID_ARGUMENT", unsupported.GrpcStatus);
        Assert.Equal("UNSUPPORTED_PROTOCOL", unsupported.Reason);
        Assert.Equal(1, State.Registrations);
        Assert.False(host.IsRunning);

        State.Fail("RegisterWorker", new Scripted(StatusCode.PermissionDenied, "NOT_BOUND"));
        var unbound = await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        Assert.Equal(SagatErrorCode.NotBound, unbound.Code);
        Assert.Equal(2, State.Registrations);
    }

    [Fact]
    public async Task ATokenTheProviderCanRefreshGetsOneMoreChanceAtRegistration()
    {
        var calls = 0;
        var host = Host(o =>
        {
            o.Token = null;
            o.TokenProvider = _ => ValueTask.FromResult($"tok-{Interlocked.Increment(ref calls)}");
        });
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("RegisterWorker", new Scripted(StatusCode.Unauthenticated));
        await host.StartAsync();
        Assert.Equal(["Bearer tok-1", "Bearer tok-2"], State.Of("RegisterWorker").Select(c => c.Headers.GetValue("authorization")));

        var second = Host(o => o.Token = "static");
        second.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("RegisterWorker", new Scripted(StatusCode.Unauthenticated));
        var error = await Assert.ThrowsAsync<SagatException>(() => second.StartAsync());
        Assert.Equal(SagatErrorCode.Unauthenticated, error.Code);
    }

    public static TheoryData<StatusCode, string?, long?, SagatErrorCode, string, bool> Mapping => new()
    {
        { StatusCode.Unauthenticated, null, null, SagatErrorCode.Unauthenticated, "UNAUTHENTICATED", false },
        { StatusCode.PermissionDenied, null, null, SagatErrorCode.Forbidden, "PERMISSION_DENIED", false },
        { StatusCode.PermissionDenied, "NOT_BOUND", null, SagatErrorCode.NotBound, "PERMISSION_DENIED", false },
        { StatusCode.PermissionDenied, "SECRET_NOT_USABLE", null, SagatErrorCode.SecretNotUsable, "PERMISSION_DENIED", false },
        { StatusCode.InvalidArgument, null, null, SagatErrorCode.ValidationFailed, "INVALID_ARGUMENT", false },
        { StatusCode.InvalidArgument, "UNSUPPORTED_PROTOCOL", null, SagatErrorCode.UnsupportedProtocol, "INVALID_ARGUMENT", false },
        { StatusCode.Aborted, null, null, SagatErrorCode.StaleAttempt, "ABORTED", false },
        { StatusCode.Aborted, "IDEMPOTENCY_KEY_REUSED", null, SagatErrorCode.IdempotencyKeyReused, "ABORTED", false },
        { StatusCode.Aborted, "SOMETHING_ELSE", null, SagatErrorCode.Conflict, "ABORTED", false },
        { StatusCode.FailedPrecondition, null, null, SagatErrorCode.StaleAttempt, "FAILED_PRECONDITION", false },
        { StatusCode.NotFound, null, null, SagatErrorCode.NotFound, "NOT_FOUND", false },
        { StatusCode.ResourceExhausted, null, 250, SagatErrorCode.RateLimited, "RESOURCE_EXHAUSTED", true },
        { StatusCode.ResourceExhausted, "QUOTA_EXCEEDED", null, SagatErrorCode.QuotaExceeded, "RESOURCE_EXHAUSTED", false },
        { StatusCode.Unavailable, null, null, SagatErrorCode.Transport, "UNAVAILABLE", true },
        { StatusCode.Unavailable, "UNAVAILABLE", null, SagatErrorCode.Unavailable, "UNAVAILABLE", true },
        { StatusCode.DeadlineExceeded, null, null, SagatErrorCode.Transport, "DEADLINE_EXCEEDED", true },
        { StatusCode.Internal, null, null, SagatErrorCode.Internal, "INTERNAL", false },
        { StatusCode.DataLoss, "BLOB_CORRUPT", null, SagatErrorCode.Protocol, "DATA_LOSS", false },
    };

    [Theory]
    [MemberData(nameof(Mapping))]
    public async Task AGrpcFailureIsMappedOntoTheCanonicalErrors(StatusCode code, string? trailer, long? retryAfterMs, SagatErrorCode expected, string status, bool retryable)
    {
        var host = Host(o => o.Retry.MaxAttempts = 1);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("RegisterWorker", new Scripted(code, trailer, retryAfterMs));
        var error = await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        Assert.Equal(expected, error.Code);
        Assert.Equal(status, error.GrpcStatus);
        Assert.Null(error.HttpStatus);
        Assert.Equal(trailer, error.Reason);
        Assert.Equal(retryable, error.Retryable);
        Assert.Equal(retryAfterMs is { } ms ? TimeSpan.FromMilliseconds(ms) : null, error.RetryAfter);
        Assert.NotEmpty(error.Remediation);
        Assert.Equal(1, State.Registrations);
    }

    [Fact]
    public async Task RegisteringTwiceOrAfterTheStartIsAMistakeAndSaysSo()
    {
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
        Assert.Equal(SagatErrorCode.ValidationFailed, Assert.Throws<SagatException>(() => host.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(2))).Code);
        Assert.Throws<SagatException>(() => host.RegisterCompensation<JsonNode?, int>("a", (_, _, _) => Task.FromResult(2)));
        Assert.Throws<ArgumentException>(() => host.RegisterActivity<JsonNode?, int>(" ", (_, _, _) => Task.FromResult(2)));
        Assert.Throws<ArgumentNullException>(() => host.RegisterActivity<JsonNode?, int>("b", null!));
        await host.StartAsync();
        Assert.Throws<SagatException>(() => host.RegisterActivity<JsonNode?, int>("later", (_, _, _) => Task.FromResult(2)));
    }

    [Fact]
    public async Task TheOptionsAreValidatedWhenTheHostIsCreatedOrStarted()
    {
        Assert.Throws<SagatException>(() => Host(o => o.Worker.MaxInflight = 0));
        Assert.Throws<SagatException>(() => Host(o => o.Worker.MaxInflight = 1001));
        var empty = await Assert.ThrowsAsync<SagatException>(() => Host().StartAsync());
        Assert.Contains("at least one activity", empty.Message, StringComparison.Ordinal);

        var noName = Host(o => o.Worker.Name = null);
        noName.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
        Assert.Contains("Worker.Name", (await Assert.ThrowsAsync<SagatException>(() => noName.StartAsync())).Message, StringComparison.Ordinal);

        var noUrl = Host(o => o.GrpcUrl = null);
        noUrl.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
        Assert.Contains("GrpcUrl", (await Assert.ThrowsAsync<SagatException>(() => noUrl.StartAsync())).Message, StringComparison.Ordinal);
        Assert.Equal(0, State.Registrations);
    }

    [Fact]
    public async Task NoLogLineHoldsATokenALeaseTokenASecretOrAPayload()
    {
        var provider = new CapturingLoggerProvider();
        using var factory = LoggerFactory.Create(builder => builder.AddProvider(provider).SetMinimumLevel(LogLevel.Trace));
        var host = Host(
            o =>
            {
                o.Token = "tok-very-secret-token";
                o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20);
            },
            factory);
        host.RegisterActivity<JsonNode?, int>("boom", (input, context, _) =>
        {
            context.Logger.LogInformation("running {Node}", context.Node);
            throw new InvalidOperationException("payload-marker " + input);
        });
        host.RegisterActivity<JsonNode?, string>("secretive", async (_, context, ct) =>
        {
            var secrets = await context.GetSecretsAsync(ct);
            return secrets.GetText("k");
        });
        State.Secrets.Add(new ResolvedSecret { Name = "k", Value = Google.Protobuf.ByteString.CopyFromUtf8("secret-value-marker"), Version = 1 });
        State.Fail("PollTasks", new Scripted(StatusCode.Unavailable));
        State.Lease(
            LeaseFor("boom", l => l.InputJson = Leases.Document(new JsonObject { ["card"] = "payload-marker-card" }, "boom")),
            LeaseFor("secretive"));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 1 && State.Of("CompleteStep").Count == 1, "both reports");
        await host.StopAsync();
        var everything = string.Join('\n', provider.Lines);
        Assert.NotEmpty(provider.Lines);
        Assert.Contains("running boom", everything, StringComparison.Ordinal);
        foreach (var forbidden in new[] { "tok-very-secret-token", "lt-secret-lease-token", "payload-marker", "secret-value-marker", "session-1" })
        {
            Assert.DoesNotContain(forbidden, everything, StringComparison.Ordinal);
        }
    }

    [Fact]
    public async Task TheHandlerSeesTheRawInputAsJsonWithNumbersExactlyAsTheyWereSent()
    {
        JsonElement? raw = null;
        var host = Host();
        host.RegisterActivity<JsonElement, JsonElement>("echo", (input, _, _) =>
        {
            raw = input;
            return Task.FromResult(input);
        });
        State.Lease(LeaseFor("echo", l => l.InputJson = """{"run":{"id":"r"},"node":"echo","input":{"a":600000,"b":0.5,"c":9007199254740993,"d":"19.99","e":1.0,"f":[1,null,{"g":1e3}]},"steps":{}}"""));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 1, "the completion");
        Assert.Equal("""{"a":600000,"b":0.5,"c":9007199254740993,"d":"19.99","e":1.0,"f":[1,null,{"g":1e3}]}""", raw!.Value.GetRawText());
        Assert.Equal(raw.Value.GetRawText(), ((CompleteStepRequest)State.Of("CompleteStep")[0].Request).OutputJson);
    }

    [Fact]
    public async Task TheNodeComesFromTheStepDocumentAndFallsBackToTheStepId()
    {
        var nodes = new List<string>();
        var host = Host();
        host.RegisterActivity<JsonNode?, int>("a", (_, context, _) =>
        {
            lock (nodes)
            {
                nodes.Add(context.Node);
            }

            return Task.FromResult(1);
        });
        State.Lease(
            LeaseFor("a", l => { l.TaskId = "t1"; l.StepId = "0190-uuid"; l.InputJson = Leases.Document(new JsonObject(), "named-node"); }),
            LeaseFor("a", l => { l.TaskId = "t2"; l.StepId = "0191-uuid"; l.InputJson = string.Empty; }));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Count == 2, "both completions");
        Assert.Equal(["0191-uuid", "named-node"], nodes.Order(StringComparer.Ordinal));
    }

    [Fact]
    public async Task TheTokenProviderIsAskedForBeforeEveryCall()
    {
        var calls = 0;
        var host = Host(o =>
        {
            o.Token = null;
            o.TokenProvider = _ => ValueTask.FromResult($"tok-{Interlocked.Increment(ref calls)}");
        });
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("PollTasks").Count >= 3, "three polls");
        var tokens = State.Of("PollTasks").Select(c => c.Headers.GetValue("authorization")).ToList();
        Assert.Equal(tokens.Count, tokens.Distinct().Count());
    }

    [Fact]
    public async Task APlainTextEndpointOutsideLoopbackIsWarnedAboutAtStartAndALoopbackOneIsNot()
    {
        var provider = new CapturingLoggerProvider();
        using var factory = LoggerFactory.Create(builder => builder.AddProvider(provider));
        var remote = Host(
            o =>
            {
                o.GrpcUrl = "http://worker.example.invalid:50051";
                o.Retry.MaxAttempts = 1;
            },
            factory);
        remote.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
        var error = await Assert.ThrowsAsync<SagatException>(() => remote.StartAsync());
        Assert.Equal(SagatErrorCode.Transport, error.Code);
        Assert.Single(provider.Lines, line => line.Contains("plain text outside loopback", StringComparison.Ordinal));
        Assert.DoesNotContain("worker.example.invalid:50051/", string.Join('\n', provider.Lines), StringComparison.Ordinal);

        var local = Host(null, factory);
        local.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
        await local.StartAsync();
        Assert.Single(provider.Lines, line => line.Contains("plain text outside loopback", StringComparison.Ordinal));
    }

    [Fact]
    public async Task AHandlerStoppedByTheShutdownThatRaisesItsCancellationReportsNothingAndItsLeaseLapses()
    {
        var host = Host();
        var started = new TaskCompletionSource();
        host.RegisterActivity<JsonNode?, int>("charge", async (_, _, ct) =>
        {
            started.SetResult();
            await Task.Delay(Timeout.Infinite, ct);
            return 1;
        });
        State.Lease(Lease());
        await host.StartAsync();
        await started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        await host.StopAsync(TimeSpan.FromMilliseconds(100));
        await Task.Delay(200);
        Assert.Empty(State.Of("FailStep"));
        Assert.Empty(State.Of("CompleteStep"));
        Assert.Equal(1, meters.Total("sagat.worker.tasks.dropped"));
    }

    [Fact]
    public async Task AHandlerCancelledByTheRunThatRaisesItsCancellationIsReportedAsCancelledButNotWhenTheShutdownCameFirst()
    {
        var host = Host(o => o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20));
        host.RegisterActivity<JsonNode?, int>("charge", async (_, _, ct) =>
        {
            await Task.Delay(Timeout.Infinite, ct);
            return 1;
        });
        State.CancelRequested = true;
        State.Lease(Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("FailStep").Count == 1, "the failure");
        var fail = (FailStepRequest)State.Of("FailStep")[0].Request;
        Assert.Equal(("CANCELLED", false), (fail.ErrorCode, fail.Retryable));
    }

    [Fact]
    public async Task TheHeartbeatKeepsRunningUntilTheReportIsMadeAndNotJustUntilTheHandlerEnds()
    {
        var host = Host(o =>
        {
            o.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20);
            o.Retry.InitialDelay = TimeSpan.FromMilliseconds(100);
            o.Retry.MaxDelay = TimeSpan.FromMilliseconds(400);
        });
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Fail("CompleteStep", new Scripted(StatusCode.Unavailable), new Scripted(StatusCode.Unavailable), new Scripted(StatusCode.Unavailable));
        State.Lease(Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => meters.Total("sagat.worker.tasks.completed") == 1, "the completion");
        var reports = State.Of("CompleteStep");
        Assert.Equal(4, reports.Count);
        var first = reports[0].At;
        var last = reports[^1].At;
        var during = State.Of("Heartbeat").Count(call => call.At > first && call.At < last);
        Assert.True(during >= 3, $"{during} heartbeats while the report was being retried");

        // ... and none once the report is made.
        var after = State.Of("Heartbeat").Count;
        await Task.Delay(150);
        Assert.Equal(after, State.Of("Heartbeat").Count);
    }

    [Fact]
    public async Task ThePauseAfterAnEmptyPollEndsEarlyWhenATaskFinishes()
    {
        State.PollRetryAfterMs = 10_000;
        var host = Host(o => o.Worker.MaxInflight = 2);
        var slow = new TaskCompletionSource();
        host.RegisterActivity<JsonNode?, int>("slow", async (_, _, ct) =>
        {
            await slow.Task.WaitAsync(ct);
            return 1;
        });
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        State.Lease(LeaseFor("slow"));
        await host.StartAsync();

        // The first poll took the slow task; the second finds nothing and the answer asks for ten seconds.
        await Eventually.UntilAsync(() => State.Of("PollTasks").Count >= 2, "the empty poll");
        State.Lease(LeaseFor("charge"));
        var clock = System.Diagnostics.Stopwatch.StartNew();
        slow.SetResult();
        await Eventually.UntilAsync(() => State.Of("CompleteStep").Any(c => ((CompleteStepRequest)c.Request).StepId == "charge"), "the task that was waiting for the pause to end", 5000);
        Assert.True(clock.ElapsedMilliseconds < 4000, $"took {clock.ElapsedMilliseconds} ms");
    }

    [Theory]
    [InlineData(-5000, 500)]
    [InlineData(0, 500)]
    [InlineData(1000, 500)]
    [InlineData(1500, 500)]
    [InlineData(3000, 1000)]
    [InlineData(6000, 2000)]
    [InlineData(30_000, 10_000)]
    [InlineData(60_000, 10_000)]
    [InlineData(3_600_000, 10_000)]
    public void TheHeartbeatIntervalIsAThirdOfTheTimeLeftOnTheLeaseNeverUnderHalfASecondNorOverTenSeconds(long leftMs, long expectedMs)
    {
        Assert.Equal(TimeSpan.FromMilliseconds(expectedMs), WorkerHost.HeartbeatIntervalFor(TimeSpan.FromMilliseconds(leftMs)));
    }

    [Theory]
    [InlineData(3000, 850, 1700)]
    [InlineData(1200, 400, 950)]
    public async Task TheFirstHeartbeatComesAThirdOfTheLeaseAfterTheTaskWasTaken(long leaseMs, long fromMs, long toMs)
    {
        var host = Host();
        var taken = 0L;
        host.RegisterActivity<JsonNode?, int>("slow", async (_, _, ct) =>
        {
            taken = System.Diagnostics.Stopwatch.GetTimestamp();
            await Task.Delay(Timeout.Infinite, ct).ContinueWith(_ => { }, TaskScheduler.Default);
            return 1;
        });
        State.Lease(LeaseFor("slow", l => l.ExpiresAtEpochMs = DateTimeOffset.UtcNow.AddMilliseconds(leaseMs).ToUnixTimeMilliseconds()));
        await host.StartAsync();
        await Eventually.UntilAsync(() => State.Of("Heartbeat").Count >= 1, "the first heartbeat");
        var elapsed = System.Diagnostics.Stopwatch.GetElapsedTime(taken, State.Of("Heartbeat")[0].At).TotalMilliseconds;
        Assert.InRange(elapsed, fromMs, toMs);
    }
}
