using System.Diagnostics;
using System.Text.Json.Nodes;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

public sealed class ClientTests : IAsyncLifetime
{
    private const string Workflow = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    private const string Trace = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

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

    private static FakeAnswer Reply(int status, object? body = null, Dictionary<string, string>? headers = null) =>
        new(status, body switch { null => null, string text => text, _ => body is JsonNode node ? node.ToJsonString() : System.Text.Json.JsonSerializer.Serialize(body) }, headers);

    private static JsonObject Run(Action<JsonObject>? change = null)
    {
        var run = new JsonObject
        {
            ["runId"] = "0190aaaa-0000-7000-8000-000000000001",
            ["workflowId"] = Workflow,
            ["versionId"] = "0190aaaa-0000-7000-8000-000000000002",
            ["status"] = "RUNNING",
            ["version"] = 3,
            ["paused"] = false,
            ["startedBy"] = "alice",
            ["createdAt"] = "2026-10-10T10:00:00Z",
            ["updatedAt"] = "2026-10-10T10:00:01+02:00",
            ["finishedAt"] = null,
            ["deadline"] = null,
            ["traceId"] = "abcdefabcdefabcdefabcdefabcdefab",
            ["correlationId"] = null,
            ["depth"] = 0,
            ["parent"] = null,
            ["futureField"] = 7,
        };
        change?.Invoke(run);
        return run;
    }

    private static JsonObject Envelope(string code, string message, bool retryable, string? reason = null, string traceId = "") =>
        new()
        {
            ["code"] = code,
            ["message"] = message,
            ["retryable"] = retryable,
            ["traceId"] = traceId.Length == 0 ? new string('e', 32) : traceId,
            ["details"] = reason is null ? null : new JsonObject { ["reason"] = reason },
        };

    private SagatClient NewClient(Action<SagatOptions>? change = null)
    {
        var options = new SagatOptions { ApiUrl = server.BaseUrl, Namespace = "payments", Token = "tok-1" };
        options.Retry.InitialDelay = TimeSpan.FromMilliseconds(5);
        options.Retry.MaxDelay = TimeSpan.FromMilliseconds(20);
        change?.Invoke(options);
        var client = new SagatClient(options);
        clients.Add(client);
        return client;
    }

    [Fact]
    public async Task AStartSendsTheHeadersAndTheBodyTheApiExpectsAndReturnsATypedRunThatKeepsTheRawAnswer()
    {
        server.Handler = _ => Reply(202, Run());
        var result = await NewClient().StartAsync(
            Workflow,
            new { orderId = "o-1" },
            new StartOptions
            {
                IdempotencyKey = "key-12345678",
                CorrelationId = "corr-1",
                Traceparent = Trace,
                Deadline = new DateTimeOffset(2030, 1, 1, 2, 30, 0, TimeSpan.FromHours(2.5)),
            });
        var request = Assert.Single(server.Requests);
        Assert.Equal("POST", request.Method);
        Assert.Equal("/v1/runs", request.PathAndQuery);
        Assert.Equal("Bearer tok-1", request.Header("authorization"));
        Assert.Equal("payments", request.Header("x-octopus-namespace"));
        Assert.Equal("key-12345678", request.Header("idempotency-key"));
        Assert.Equal(Trace, request.Header("traceparent"));
        Assert.Equal("application/json", request.Header("accept"));
        Assert.StartsWith("application/json", request.Header("content-type"), StringComparison.Ordinal);
        Assert.Matches(@"^sagat-sdk-csharp/\d+\.\d+\.\d+$", request.Header("user-agent"));
        Assert.Equal(
            $$"""{"workflowId":"{{Workflow}}","input":{"orderId":"o-1"},"correlationId":"corr-1","deadline":"2030-01-01T00:00:00.000Z"}""",
            request.BodyText);
        Assert.Equal(RunStatuses.Running, result.Status);
        Assert.Equal(new DateTimeOffset(2026, 10, 10, 10, 0, 0, TimeSpan.Zero), result.CreatedAt);
        Assert.Equal(TimeSpan.Zero, result.CreatedAt.Offset);
        Assert.Equal(new DateTimeOffset(2026, 10, 10, 8, 0, 1, TimeSpan.Zero), result.UpdatedAt);
        Assert.Equal(TimeSpan.Zero, result.UpdatedAt!.Value.Offset);
        Assert.Equal(3, result.Version);
        Assert.Equal(7, result.Raw.GetProperty("futureField").GetInt32());
    }

    [Fact]
    public async Task TheUserAgentSuffixIsAppended()
    {
        server.Handler = _ => Reply(200, Run());
        await NewClient(o => o.UserAgent = "my-service/2.1").GetAsync(Ids.Run);
        Assert.Equal($"sagat-sdk-csharp/{SdkInfo.Version} my-service/2.1", server.Requests[0].Header("user-agent"));
    }

    [Fact]
    public async Task ACallWithoutAKeyGetsAFreshOnePerCallAndKeepsItAcrossItsRetries()
    {
        var attempts = 0;
        server.Handler = _ => ++attempts < 3 ? Reply(503, Envelope("UNAVAILABLE", "busy", retryable: true)) : Reply(202, Run());
        var client = NewClient();
        await client.StartAsync(Workflow, new { });
        Assert.Equal(3, server.Requests.Count);
        var keys = server.Requests.Select(r => r.Header("idempotency-key")).Distinct().ToList();
        var key = Assert.Single(keys);
        Assert.Matches("^[0-9a-f-]{36}$", key);

        attempts = 0;
        await client.StartAsync(Workflow, new { });
        Assert.NotEqual(key, server.Requests[3].Header("idempotency-key"));
    }

    [Fact]
    public async Task AnEventWithoutAnIdGetsOnePerCallKeptAcrossRetriesSoARepetitionIsADuplicateAndNotASecondEvent()
    {
        var attempts = 0;
        server.Handler = _ => ++attempts < 2 ? Reply(503, "upstream down") : Reply(202, new JsonObject { ["eventId"] = "e", ["source"] = "api", ["status"] = "ACCEPTED" });
        var accepted = await NewClient().PublishEventAsync(new PublishEventRequest { Type = "order.created", Data = new { id = 1 } });
        Assert.Equal("ACCEPTED", accepted.Status);
        Assert.Equal(2, server.Requests.Count);
        var ids = server.Requests.Select(r => (string?)r.Body!["eventId"]).ToList();
        Assert.Equal(ids[0], ids[1]);
        Assert.Matches("^[0-9a-f-]{36}$", ids[0]!);
        Assert.Equal("""{"id":1}""", server.Requests[0].Body!["data"]!.ToJsonString());
    }

    [Fact]
    public async Task AGetIsRetriedOnUnavailabilityAndOnRateLimitingHonouringRetryAfterAndGivesUpAfterTheAttempts()
    {
        var attempts = 0;
        server.Handler = _ => ++attempts == 1
            ? Reply(429, Envelope("RATE_LIMITED", "slow", retryable: true), new Dictionary<string, string> { ["retry-after"] = "0" })
            : Reply(200, Run());
        var got = await NewClient().GetAsync(Ids.Run);
        Assert.Equal("0190aaaa-0000-7000-8000-000000000001", got.RunId);
        Assert.Equal(2, server.Requests.Count);

        server.Handler = _ => Reply(503, "<html>bad gateway</html>");
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(Ids.Run));
        Assert.Equal(SagatErrorCode.Unavailable, error.Code);
        Assert.Equal(503, error.HttpStatus);
        Assert.Equal(6, server.Requests.Count);
    }

    [Fact]
    public async Task RetryAfterIsHonouredWhenItIsLongerThanTheBackoff()
    {
        var attempts = 0;
        server.Handler = _ => ++attempts == 1
            ? Reply(429, Envelope("RATE_LIMITED", "slow", retryable: true), new Dictionary<string, string> { ["retry-after"] = "1" })
            : Reply(200, Run());
        var clock = Stopwatch.StartNew();
        await NewClient().GetAsync(Ids.Run);
        Assert.True(clock.ElapsedMilliseconds >= 900, $"waited {clock.ElapsedMilliseconds} ms");
    }

    [Fact]
    public async Task A4xxOtherThan429IsNeverRetriedAndKeepsWhatTheServerSaid()
    {
        server.Handler = _ => Reply(409, Envelope("CONFLICT", "the idempotency key was used for another request", retryable: false, "IDEMPOTENCY_KEY_REUSED", new string('c', 32)));
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync(Workflow, new { }, new StartOptions { IdempotencyKey = "key-12345678" }));
        Assert.Equal(SagatErrorCode.Conflict, error.Code);
        Assert.Equal("IDEMPOTENCY_KEY_REUSED", error.Reason);
        Assert.Equal(new string('c', 32), error.TraceId);
        Assert.False(error.Retryable);
        Assert.NotEmpty(error.Remediation);
        Assert.Single(server.Requests);
    }

    [Theory]
    [InlineData(401, "UNAUTHENTICATED", SagatErrorCode.Unauthenticated)]
    [InlineData(403, "FORBIDDEN", SagatErrorCode.Forbidden)]
    [InlineData(404, "NOT_FOUND", SagatErrorCode.NotFound)]
    [InlineData(422, "VALIDATION_FAILED", SagatErrorCode.ValidationFailed)]
    [InlineData(400, "BAD_REQUEST", SagatErrorCode.BadRequest)]
    public async Task ARefusedTokenAMissingResourceAndAValidationErrorAreToldApart(int status, string wire, SagatErrorCode expected)
    {
        server.Handler = _ => Reply(status, Envelope(wire, "no", retryable: false, traceId: new string('b', 32)));
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(Ids.Run));
        Assert.Equal(expected, error.Code);
        Assert.Equal(status, error.HttpStatus);
        Assert.Equal(new string('b', 32), error.TraceId);
        Assert.Single(server.Requests);
    }

    [Fact]
    public async Task AStatusWithoutAnEnvelopeIsClassifiedByTheCatalogAndTheTraceComesFromTheHeader()
    {
        server.Handler = _ => Reply(502, "<html>bad gateway</html>");
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient(o => o.Retry.MaxAttempts = 1).GetAsync(Ids.Run));
        Assert.Equal(SagatErrorCode.Unavailable, error.Code);
        Assert.Equal(new string('f', 32), error.TraceId);
        Assert.Equal("the server answered 502", error.Message);
    }

    [Fact]
    public async Task ACallTheServerDoesNotHaveIsNotFoundWithTheReasonUnsupportedByServer()
    {
        server.Handler = _ => Reply(501, "not implemented");
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(Ids.Run));
        Assert.Equal(SagatErrorCode.NotFound, error.Code);
        Assert.Equal("UNSUPPORTED_BY_SERVER", error.Reason);
        Assert.Single(server.Requests);

        server.Handler = _ => Reply(404, "<html>no route</html>");
        error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(Ids.Run));
        Assert.Equal("UNSUPPORTED_BY_SERVER", error.Reason);

        server.Handler = _ => Reply(404, Envelope("NOT_FOUND", "no such run", retryable: false));
        error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(Ids.Run));
        Assert.Null(error.Reason);
    }

    [Fact]
    public async Task TheTokenIsAskedForBeforeEveryRequestSoARotatedOneIsUsedAtOnce()
    {
        var n = 0;
        server.Handler = _ => Reply(200, Run());
        var client = NewClient(o =>
        {
            o.Token = null;
            o.TokenProvider = _ => ValueTask.FromResult($"tok-{Interlocked.Increment(ref n)}");
        });
        await client.GetAsync(Ids.Run);
        await client.GetAsync(Ids.Run);
        Assert.Equal(["Bearer tok-1", "Bearer tok-2"], server.Requests.Select(r => r.Header("authorization")));
    }

    [Fact]
    public async Task ANameIsResolvedToTheIdOfItsWorkflowThroughEveryPageAndRemembered()
    {
        server.Handler = request =>
        {
            if (request.PathAndQuery.StartsWith("/v1/workflows", StringComparison.Ordinal))
            {
                return request.PathAndQuery.Contains("cursor=c2", StringComparison.Ordinal)
                    ? Reply(200, new JsonObject { ["items"] = new JsonArray(new JsonObject { ["id"] = Workflow, ["name"] = "order-flow" }), ["nextCursor"] = null })
                    : Reply(200, new JsonObject { ["items"] = new JsonArray(new JsonObject { ["id"] = "0190aaaa-0000-7000-8000-00000000ffff", ["name"] = "other" }), ["nextCursor"] = "c2" });
            }

            return Reply(202, Run());
        };
        var client = NewClient();
        await client.StartAsync("order-flow", new { });
        await client.StartAsync("order-flow", new { });
        Assert.Equal(2, server.Requests.Count(r => r.PathAndQuery.StartsWith("/v1/workflows", StringComparison.Ordinal)));
        Assert.Equal(Workflow, (string?)server.Requests.First(r => r.Method == "POST").Body!["workflowId"]);

        server.Handler = _ => Reply(200, new JsonObject { ["items"] = new JsonArray(), ["nextCursor"] = null });
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync("nothing-here", new { }));
        Assert.Equal(SagatErrorCode.NotFound, error.Code);
        Assert.Equal("WORKFLOW_NOT_FOUND", error.Reason);
    }

    [Fact]
    public async Task AWorkflowIdIsUsedAsItIsAndLowerCased()
    {
        server.Handler = _ => Reply(202, Run());
        await NewClient().StartAsync(Workflow.ToUpperInvariant(), new { });
        Assert.Equal(Workflow, (string?)server.Requests[0].Body!["workflowId"]);
        Assert.Single(server.Requests);
    }

    [Fact]
    public async Task AMalformedTraceparentOrKeyIsRefusedBeforeAnythingIsSent()
    {
        server.Handler = _ => Reply(202, Run());
        var badTrace = await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync(Workflow, new { }, new StartOptions { Traceparent = "not-a-trace" }));
        Assert.Equal(SagatErrorCode.BadRequest, badTrace.Code);
        var badKey = await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync(Workflow, new { }, new StartOptions { IdempotencyKey = "short" }));
        Assert.Equal(SagatErrorCode.ValidationFailed, badKey.Code);
        var nonAscii = await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync(Workflow, new { }, new StartOptions { IdempotencyKey = "key-with-a-ç-in-it" }));
        Assert.Equal(SagatErrorCode.ValidationFailed, nonAscii.Code);
        Assert.Empty(server.Requests);
    }

    [Fact]
    public async Task TheAmbientTraceContextIsUsedWhenTheCallHasNone()
    {
        server.Handler = _ => Reply(200, Run());
        using var ambient = new Activity("caller").SetIdFormat(ActivityIdFormat.W3C);
        ambient.Start();
        await NewClient().GetAsync(Ids.Run);
        // The call is in the caller's trace. Its span id and its flags may differ from the ambient activity's: when
        // something listens to the SDK's ActivitySource (a test of its telemetry that runs at the same time, or an
        // application's tracer) the call has a span of its own, a child of the ambient one, and it is sampled.
        var parts = server.Requests[0].Header("traceparent")!.Split('-');
        Assert.Equal(4, parts.Length);
        Assert.Equal("00", parts[0]);
        Assert.Equal(ambient.TraceId.ToHexString(), parts[1]);
        Assert.Matches("^[0-9a-f]{16}$", parts[2]);
    }

    [Fact]
    public async Task AnExplicitTraceparentWinsOverTheAmbientOne()
    {
        server.Handler = _ => Reply(200, Run());
        using var ambient = new Activity("caller").SetIdFormat(ActivityIdFormat.W3C);
        ambient.Start();
        await NewClient().StartAsync(Workflow, new { }, new StartOptions { Traceparent = Trace });
        Assert.Equal(Trace, server.Requests[0].Header("traceparent"));
    }

    [Fact]
    public async Task AListFollowsItsCursorADeadlineGivenAsADurationIsComputedOnceAndWhatCannotBeWrittenAsJsonIsRefused()
    {
        server.Handler = request => request.PathAndQuery.Contains("cursor=n2", StringComparison.Ordinal)
            ? Reply(200, new JsonObject { ["items"] = new JsonArray(Run(r => r["runId"] = "r-2")), ["nextCursor"] = null })
            : Reply(200, new JsonObject { ["items"] = new JsonArray(Run(r => r["runId"] = "r-1")), ["nextCursor"] = "n2" });
        var ids = new List<string>();
        await foreach (var item in NewClient().ListAllAsync(new ListOptions { Status = RunStatuses.Running, Limit = 1 }))
        {
            ids.Add(item.RunId);
        }

        Assert.Equal(["r-1", "r-2"], ids);
        Assert.Contains("status=RUNNING", server.Requests[0].PathAndQuery, StringComparison.Ordinal);
        Assert.Contains("limit=1", server.Requests[0].PathAndQuery, StringComparison.Ordinal);

        server.Handler = _ => Reply(202, Run());
        var before = DateTimeOffset.UtcNow;
        await NewClient().StartAsync(Workflow, new { }, new StartOptions { DeadlineIn = TimeSpan.FromMinutes(1) });
        var sent = DateTimeOffset.Parse((string)server.Requests[^1].Body!["deadline"]!, System.Globalization.CultureInfo.InvariantCulture);
        Assert.True(sent >= before.AddMinutes(1).AddSeconds(-1) && sent < before.AddMinutes(1).AddSeconds(2), sent.ToString("o"));
        Assert.EndsWith("Z", (string)server.Requests[^1].Body!["deadline"]!, StringComparison.Ordinal);

        var count = server.Requests.Count;
        var notFinite = await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync(Workflow, new { nan = double.NaN }));
        Assert.Equal(SagatErrorCode.ValidationFailed, notFinite.Code);
        var infinite = await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync(Workflow, new { inf = double.PositiveInfinity }));
        Assert.Equal(SagatErrorCode.ValidationFailed, infinite.Code);
        var unicode = await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync(Workflow, new { text = "lone \ud800 surrogate" }));
        Assert.Equal(SagatErrorCode.ValidationFailed, unicode.Code);
        Assert.Equal(count, server.Requests.Count);
    }

    [Fact]
    public async Task ALostAnswerIsRetriedOnlyBecauseTheCallCarriesAKey()
    {
        server.Handler = _ => Reply(202, Run());
        var calls = 0;
        var lossy = new LossyHandler(() => ++calls == 1);
        var result = await NewClient(o => o.HttpMessageHandler = lossy).StartAsync(Workflow, new { }, new StartOptions { IdempotencyKey = "key-12345678" });
        Assert.Equal(RunStatuses.Running, result.Status);
        Assert.Equal(2, server.Requests.Count);
        Assert.Equal(server.Requests[0].Header("idempotency-key"), server.Requests[1].Header("idempotency-key"));
    }

    [Fact]
    public async Task NoAnswerAtAllIsARetryableTransportErrorWithNoStatus()
    {
        using var client = new SagatClient(new SagatOptions
        {
            ApiUrl = "http://127.0.0.1:1/v1",
            Namespace = "p",
            Token = "t",
            Retry = { MaxAttempts = 2, InitialDelay = TimeSpan.FromMilliseconds(1), MaxDelay = TimeSpan.FromMilliseconds(2) },
        });
        var error = await Assert.ThrowsAsync<SagatException>(() => client.GetAsync(Ids.Run));
        Assert.Equal(SagatErrorCode.Transport, error.Code);
        Assert.True(error.Retryable);
        Assert.Null(error.HttpStatus);
        Assert.Null(error.GrpcStatus);
        Assert.Equal(string.Empty, error.TraceId);
    }

    [Fact]
    public async Task ACallThatTakesLongerThanTheTimeoutIsATransportErrorAndAPostWithAKeyIsRepeated()
    {
        server.Delay = TimeSpan.FromMilliseconds(400);
        server.Handler = _ => Reply(200, Run());
        var client = NewClient(o =>
        {
            o.Timeout = TimeSpan.FromMilliseconds(100);
            o.Retry.MaxAttempts = 2;
        });
        var error = await Assert.ThrowsAsync<SagatException>(() => client.GetAsync(Ids.Run));
        Assert.Equal(SagatErrorCode.Transport, error.Code);
        Assert.Equal("the request timed out", error.Message);
        await Eventually.UntilAsync(() => server.Requests.Count == 2, "two attempts");
    }

    [Fact]
    public async Task ACallCancelledWithTheCallersTokenRaisesOperationCanceledAndIsNotRetried()
    {
        server.Delay = TimeSpan.FromSeconds(5);
        server.Handler = _ => Reply(200, Run());
        using var cancel = new CancellationTokenSource();
        var call = NewClient().GetAsync(Ids.Run, cancel.Token);

        // The caller gives up once the request has reached the server.
        await Eventually.UntilAsync(() => server.Requests.Count == 1, "the request");
        await cancel.CancelAsync();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => call);
        await Task.Delay(100);
        Assert.Single(server.Requests);

        using var already = new CancellationTokenSource();
        await already.CancelAsync();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => NewClient().StartAsync(Workflow, new { }, null, already.Token));
    }

    [Fact]
    public async Task CancelAndSignalAreOperationsWithAReasonAndAPayloadAndTheResultIsTyped()
    {
        server.Handler = request => request.PathAndQuery.EndsWith("/operations", StringComparison.Ordinal)
            ? Reply(202, new JsonObject { ["operationId"] = "op-1", ["accepted"] = true, ["runStatus"] = "CANCELLING", ["runVersion"] = 9 })
            : Reply(202, new JsonObject { ["deliveredTo"] = new JsonArray("wait") });
        var client = NewClient();
        var cancelled = await client.CancelAsync(Ids.Run, "no longer needed", new OperateOptions { ExpectedRunVersion = 3 });
        Assert.Equal(("op-1", true, "CANCELLING", 9L), (cancelled.OperationId, cancelled.Accepted, cancelled.RunStatus, cancelled.RunVersion));
        Assert.Equal("""{"action":"CANCEL","reason":"no longer needed","expectedRunVersion":3}""", server.Requests[0].BodyText);
        Assert.Matches("^[0-9a-f-]{36}$", server.Requests[0].Header("idempotency-key"));

        var signalled = await client.SignalAsync(Ids.Run, "payment-confirmed", new { paymentId = "p-1" });
        Assert.Equal(["wait"], signalled.DeliveredTo);
        Assert.Equal($"/v1/runs/{Ids.Run}/signals/payment-confirmed", server.Requests[1].PathAndQuery);
        Assert.Equal("""{"paymentId":"p-1"}""", server.Requests[1].BodyText);
    }

    [Fact]
    public async Task AnApprovalDecisionUsesItsBodyKeyAndReturnsATypedReceipt()
    {
        const string approvalId = "0190aaaa-0000-7000-8000-000000000099";
        server.Handler = _ => Reply(200, new JsonObject
        {
            ["receiptId"] = "0190aaaa-0000-7000-8000-000000000100",
            ["approvalId"] = approvalId,
            ["decisionKey"] = "review-1",
            ["decision"] = "APPROVE",
            ["reasonCode"] = "POLICY_OK",
            ["actor"] = "alice",
            ["decidedAt"] = "2026-10-15T10:00:00Z",
            ["outcomeSequence"] = 7,
            ["replayed"] = false,
            ["futureField"] = "kept",
        });
        var receipt = await NewClient().DecideApprovalAsync(
            approvalId,
            ApprovalDecision.Approve,
            new ApprovalDecisionOptions { DecisionKey = "review-1", ReasonCode = "POLICY_OK", Traceparent = Trace });
        var request = Assert.Single(server.Requests);
        Assert.Equal($"/v1/approvals/{approvalId}/decisions", request.PathAndQuery);
        Assert.Equal("""{"decisionKey":"review-1","decision":"APPROVE","reasonCode":"POLICY_OK"}""", request.BodyText);
        Assert.Empty(request.Header("idempotency-key"));
        Assert.Equal(new DateTimeOffset(2026, 10, 15, 10, 0, 0, TimeSpan.Zero), receipt.DecidedAt);
        Assert.Equal(7, receipt.OutcomeSequence);
        Assert.Equal("kept", receipt.Raw.GetProperty("futureField").GetString());
    }

    [Theory]
    [InlineData(403, SagatErrorCode.Forbidden, "NOT_ELIGIBLE")]
    [InlineData(409, SagatErrorCode.Conflict, "IDEMPOTENCY_KEY_REUSED")]
    public async Task AnApprovalDecisionSurfacesAuthorizationAndIdempotencyFailures(int status, SagatErrorCode code, string reason)
    {
        server.Handler = _ => Reply(status, Envelope(code.ToString().ToUpperInvariant(), "refused", false, reason));
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().DecideApprovalAsync(
            "0190aaaa-0000-7000-8000-000000000099",
            ApprovalDecision.Reject,
            new ApprovalDecisionOptions { DecisionKey = "review-1" }));
        Assert.Equal((status, code, reason), (error.HttpStatus, error.Code, error.Reason));
    }

    [Fact]
    public async Task MalformedApprovalDecisionInputIsRejectedBeforeAnythingIsSent()
    {
        var client = NewClient();
        await Assert.ThrowsAsync<SagatException>(() => client.DecideApprovalAsync(
            "../other", ApprovalDecision.Approve, new ApprovalDecisionOptions { DecisionKey = "review-1" }));
        await Assert.ThrowsAsync<SagatException>(() => client.DecideApprovalAsync(
            "0190aaaa-0000-7000-8000-000000000099", ApprovalDecision.Approve, new ApprovalDecisionOptions { DecisionKey = "bad key" }));
        Assert.Empty(server.Requests);
    }

    [Fact]
    public async Task OperationsOtherThanCancelNameTheirActionAndCarryTheirArguments()
    {
        server.Handler = _ => Reply(202, new JsonObject { ["operationId"] = "op-2", ["accepted"] = true });
        var client = NewClient();
        await client.OperateAsync(Ids.Run, RunAction.Pause, "maintenance window");
        await client.OperateAsync(Ids.Run, RunAction.ResolveStep, "fixed by hand", new OperateOptions { StepId = "charge", Outcome = StepOutcome.Failed, ErrorCode = "MANUAL" });
        Assert.Equal("""{"action":"PAUSE","reason":"maintenance window"}""", server.Requests[0].BodyText);
        Assert.Equal("""{"action":"RESOLVE_STEP","reason":"fixed by hand","stepId":"charge","outcome":"FAILED","errorCode":"MANUAL"}""", server.Requests[1].BodyText);
    }

    [Fact]
    public async Task TheJournalIsReadAPageAtATimeWithItsTypedFields()
    {
        server.Handler = _ => Reply(200, new JsonObject
        {
            ["items"] = new JsonArray(
                new JsonObject { ["eventType"] = "run.started" },
                new JsonObject { ["eventType"] = "step.leased", ["stepId"] = "charge", ["attempt"] = 2 }),
            ["nextCursor"] = "next",
        });
        var page = await NewClient().EventsAsync(Ids.Run, "c0", 50);
        Assert.Equal($"/v1/runs/{Ids.Run}/events?cursor=c0&limit=50", server.Requests[0].PathAndQuery);
        Assert.Equal("next", page.NextCursor);
        Assert.Equal(("run.started", null, null), (page.Items[0].Type, page.Items[0].StepId, page.Items[0].Attempt));
        Assert.Equal(("step.leased", "charge", 2), (page.Items[1].Type, page.Items[1].StepId, page.Items[1].Attempt));
    }

    [Fact]
    public async Task WaitingForCompletionPollsUntilTheRunIsTerminal()
    {
        var polls = 0;
        server.Handler = _ => Reply(200, Run(r => r["status"] = ++polls < 3 ? "RUNNING" : "SUCCEEDED"));
        var done = await NewClient().WaitForCompletionAsync(Ids.Run, new WaitOptions { PollInterval = TimeSpan.FromMilliseconds(5), Timeout = TimeSpan.FromSeconds(5) });
        Assert.Equal(RunStatuses.Succeeded, done.Status);
        Assert.True(done.IsTerminal);
        Assert.Equal(3, polls);

        server.Handler = _ => Reply(200, Run());
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().WaitForCompletionAsync(Ids.Run, new WaitOptions { PollInterval = TimeSpan.FromMilliseconds(5), Timeout = TimeSpan.FromMilliseconds(30) }));
        Assert.Equal("WAIT_TIMEOUT", error.Reason);
    }

    [Fact]
    public async Task WaitingForCompletionStopsWhenTheCallersTokenIsCancelled()
    {
        server.Handler = _ => Reply(200, Run());
        using var cancel = new CancellationTokenSource(TimeSpan.FromMilliseconds(150));
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() =>
            NewClient().WaitForCompletionAsync(Ids.Run, new WaitOptions { PollInterval = TimeSpan.FromMilliseconds(20), Timeout = TimeSpan.FromSeconds(30) }, cancel.Token));
    }

    [Fact]
    public async Task AnErrorNeverRepeatsTheTokenOrThePayloadItWasGiven()
    {
        server.Handler = _ => Reply(422, Envelope("VALIDATION_FAILED", "an input field is invalid", retryable: false));
        var error = await Assert.ThrowsAsync<SagatException>(() =>
            NewClient(o => o.Token = "tok-very-secret").StartAsync(Workflow, new { card = "4111111111111111-marker" }));
        var everything = error.ToString() + error.Message + error.Remediation + error.Reason + error.TraceId;
        Assert.DoesNotContain("tok-very-secret", everything, StringComparison.Ordinal);
        Assert.DoesNotContain("4111111111111111-marker", everything, StringComparison.Ordinal);

        server.Handler = _ => Reply(500, "<html>oops 4111111111111111-marker tok-very-secret</html>");
        var opaque = await Assert.ThrowsAsync<SagatException>(() => NewClient(o => o.Token = "tok-very-secret").GetAsync(Ids.Run));
        Assert.DoesNotContain("tok-very-secret", opaque.ToString(), StringComparison.Ordinal);
        Assert.DoesNotContain("marker", opaque.ToString(), StringComparison.Ordinal);
    }

    [Fact]
    public void TheOptionsAreValidatedWhenTheClientIsCreated()
    {
        Assert.Throws<SagatException>(() => new SagatClient(new SagatOptions { Namespace = "p", Token = "t" }));
        Assert.Throws<SagatException>(() => new SagatClient(new SagatOptions { ApiUrl = "ftp://x/v1", Namespace = "p", Token = "t" }));
        Assert.Throws<SagatException>(() => new SagatClient(new SagatOptions { ApiUrl = "http://x/v1", Token = "t" }));
        Assert.Throws<SagatException>(() => new SagatClient(new SagatOptions { ApiUrl = "http://x/v1", Namespace = "p" }));
        Assert.Throws<SagatException>(() => new SagatClient(new SagatOptions { ApiUrl = "http://x/v1", Namespace = "p", Token = "t", Retry = { MaxAttempts = 0 } }));
    }

    [Fact]
    public async Task ABackoffGrowsUpToTheCeilingWithJitterBetweenHalfAndWhole()
    {
        var policy = new RetryPolicy { InitialDelay = TimeSpan.FromMilliseconds(200), MaxDelay = TimeSpan.FromSeconds(5) };
        Assert.Equal(TimeSpan.FromMilliseconds(100), policy.Backoff(0, () => 0.0));
        Assert.Equal(TimeSpan.FromMilliseconds(200), policy.Backoff(0, () => 1.0));
        Assert.Equal(TimeSpan.FromMilliseconds(400), policy.Backoff(1, () => 1.0));
        Assert.Equal(TimeSpan.FromMilliseconds(5000), policy.Backoff(10, () => 1.0));
        Assert.Equal(TimeSpan.FromMilliseconds(5000), policy.Backoff(1000, () => 1.0));
        await Task.CompletedTask;
    }

    [Fact]
    public void APlainTextEndpointOutsideLoopbackIsWarnedAboutOnceAtStartAndTlsOrLoopbackIsNot()
    {
        var provider = new CapturingLoggerProvider();
        using var factory = LoggerFactory.Create(builder => builder.AddProvider(provider));
        var logger = factory.CreateLogger<SagatClient>();
        using var plain = new SagatClient(new SagatOptions { ApiUrl = "http://api.example.test/v1", Namespace = "p", Token = "t-never-logged" }, logger);
        var warning = Assert.Single(provider.Lines, line => line.Contains("plain text outside loopback", StringComparison.Ordinal));
        Assert.DoesNotContain("t-never-logged", warning, StringComparison.Ordinal);
        using var secure = new SagatClient(new SagatOptions { ApiUrl = "https://api.example.test/v1", Namespace = "p", Token = "t" }, logger);
        using var local = new SagatClient(new SagatOptions { ApiUrl = "http://127.0.0.1:8080/v1", Namespace = "p", Token = "t" }, logger);
        using var named = new SagatClient(new SagatOptions { ApiUrl = "http://localhost:8080/v1", Namespace = "p", Token = "t" }, logger);
        Assert.Single(provider.Lines, line => line.Contains("plain text outside loopback", StringComparison.Ordinal));
    }

    /// <summary>A transport that lets the first call through and loses its answer: the request reached the server.</summary>
    private sealed class LossyHandler(Func<bool> lose) : DelegatingHandler(new HttpClientHandler { AllowAutoRedirect = false })
    {
        protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
        {
            var response = await base.SendAsync(request, cancellationToken);
            if (lose())
            {
                response.Dispose();
                throw new HttpRequestException("connection reset");
            }

            return response;
        }
    }
}
