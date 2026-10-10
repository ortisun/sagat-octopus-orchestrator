using System.Diagnostics;
using System.Text;
using System.Text.Json.Nodes;
using Grpc.Core;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workers;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>The client against a server that misbehaves and against a caller that is careless with names, ids and URLs.</summary>
public sealed class ClientHardeningTests : IAsyncLifetime
{
    private const string Run = """{"runId":"0190aaaa-0000-7000-8000-000000000001","workflowId":"0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z"}""";
    private const string RunId = "0190aaaa-0000-7000-8000-000000000001";
    private const string WorkflowId = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";

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

    private SagatClient NewClient(Action<SagatOptions>? change = null)
    {
        var options = new SagatOptions { ApiUrl = server.BaseUrl, Namespace = "payments", Token = "tok-1" };
        options.Retry.MaxAttempts = 3;
        options.Retry.InitialDelay = TimeSpan.FromMilliseconds(2);
        options.Retry.MaxDelay = TimeSpan.FromMilliseconds(5);
        change?.Invoke(options);
        var client = new SagatClient(options);
        clients.Add(client);
        return client;
    }

    // ------------------------------------------------------------------------------ 5. the URL in the warning

    [Fact]
    public async Task ThePlainTextWarningNamesTheHostAndNothingElseOfTheUrl()
    {
        var provider = new CapturingLoggerProvider();
        using var factory = LoggerFactory.Create(builder => builder.AddProvider(provider));
        using var client = new SagatClient(
            new SagatOptions { ApiUrl = "http://alice:S3CR3T-canary@api.example.test:8080/v1/secret-path?apikey=Q123-canary", Namespace = "p", Token = "t" },
            factory.CreateLogger<SagatClient>());
        var warning = Assert.Single(provider.Lines, line => line.Contains("plain text", StringComparison.Ordinal));
        Assert.Contains("api.example.test", warning, StringComparison.Ordinal);
        foreach (var leak in new[] { "alice", "S3CR3T", "canary", "apikey", "Q123", "secret-path", "/v1", ":8080" })
        {
            Assert.DoesNotContain(leak, warning, StringComparison.Ordinal);
        }

        var worker = new SagatOptions { GrpcUrl = "http://bob:W0RKER-canary@worker.example.invalid:50051/path?token=abc-canary", Namespace = "p", Token = "t", Retry = { MaxAttempts = 1 } };
        worker.Worker.Name = "n";
        await using var host = new WorkerHost(worker, factory);
        host.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
        await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        var everything = string.Join('\n', provider.Lines);
        Assert.Contains("worker.example.invalid", everything, StringComparison.Ordinal);
        foreach (var leak in new[] { "bob", "W0RKER", "canary", "token=abc", "/path", ":50051" })
        {
            Assert.DoesNotContain(leak, everything, StringComparison.Ordinal);
        }
    }

    // ---------------------------------------------------------------- 6. waits and sizes that come from the server

    [Theory]
    [InlineData("retry-after-ms", "1000000000000000")]
    [InlineData("retry-after-ms", "99999999999999999999999")]
    [InlineData("retry-after-ms", "9223372036854775807")]
    [InlineData("Retry-After", "1000000000")]
    [InlineData("Retry-After", "99999999999999999999")]
    public async Task AWaitTheServerAsksForIsCappedAtAMinuteAndNeverBreaksTheCall(string header, string value)
    {
        server.Handler = _ => new FakeAnswer(503, """{"code":"UNAVAILABLE","message":"x","retryable":true}""", new Dictionary<string, string> { [header] = value });
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient(o => o.Retry.MaxAttempts = 1).GetAsync(RunId));
        Assert.Equal(SagatErrorCode.Unavailable, error.Code);
        Assert.Equal(TimeSpan.FromSeconds(60), error.RetryAfter);

        // With attempts left the call sleeps for the capped wait: it is cancelled by its caller, it does not blow up.
        using var cancel = new CancellationTokenSource(TimeSpan.FromMilliseconds(500));
        var clock = Stopwatch.StartNew();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => NewClient().GetAsync(RunId, cancel.Token));
        Assert.True(clock.ElapsedMilliseconds < 3000);
    }

    [Theory]
    [InlineData("retry-after-ms", "1500", 1500)]
    [InlineData("Retry-After", "2", 2000)]
    [InlineData("Retry-After", "0", 0)]
    public async Task AWaitInRangeIsReadAsItIs(string header, string value, int expectedMs)
    {
        server.Handler = _ => new FakeAnswer(503, "{}", new Dictionary<string, string> { [header] = value });
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient(o => o.Retry.MaxAttempts = 1).GetAsync(RunId));
        Assert.Equal(TimeSpan.FromMilliseconds(expectedMs), error.RetryAfter);
    }

    [Theory]
    [InlineData("Retry-After", "Wed, 21 Oct 2099 07:28:00 GMT")]
    [InlineData("Retry-After", "soon")]
    [InlineData("Retry-After", "-5")]
    [InlineData("retry-after-ms", "1.5")]
    [InlineData("retry-after-ms", "-1")]
    public async Task AWaitThatIsNotSecondsOrMillisecondsIsIgnored(string header, string value)
    {
        server.Handler = _ => new FakeAnswer(503, "{}", new Dictionary<string, string> { [header] = value });
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient(o => o.Retry.MaxAttempts = 1).GetAsync(RunId));
        Assert.Null(error.RetryAfter);
    }

    [Fact]
    public async Task ABodyLargerThanThirtyTwoMiBIsReadNoFurtherAndTheCallFailsAsProtocol()
    {
        const long Total = 160L * 1024 * 1024;
        long written = 0;
        server.Handler = _ => new FakeAnswer(200)
        {
            Stream = async (stream, ct) =>
            {
                var chunk = new byte[64 * 1024];
                Array.Fill(chunk, (byte)'a');
                await stream.WriteAsync(Encoding.ASCII.GetBytes("{\"runId\":\""), ct);
                while (Interlocked.Read(ref written) < Total)
                {
                    await stream.WriteAsync(chunk, ct);
                    Interlocked.Add(ref written, chunk.Length);
                }

                await stream.WriteAsync(Encoding.ASCII.GetBytes("\"}"), ct);
            },
        };
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient(o => o.Retry.MaxAttempts = 1).GetAsync(RunId));

        // What the server had written when the client gave up (afterwards a server writes into a closed connection as fast as it can).
        var writtenWhenItGaveUp = Interlocked.Read(ref written);
        Assert.Equal(SagatErrorCode.Protocol, error.Code);
        Assert.False(error.Retryable);
        Assert.True(writtenWhenItGaveUp < Total / 2, $"the server had written {writtenWhenItGaveUp} of {Total} bytes: the client kept reading far beyond 32 MiB");
        Assert.True(writtenWhenItGaveUp >= 30L * 1024 * 1024, "it read at least up to the limit");
    }

    [Fact]
    public async Task ABodyThatDeclaresMoreThanThirtyTwoMiBIsRefusedBeforeItIsRead()
    {
        server.Handler = _ => new FakeAnswer(200, null, new Dictionary<string, string> { ["content-length"] = "99999999999" })
        {
            Stream = (stream, ct) => stream.WriteAsync(Encoding.ASCII.GetBytes("{}"), ct).AsTask(),
        };
        var error = await Assert.ThrowsAnyAsync<Exception>(() => NewClient(o => o.Retry.MaxAttempts = 1).GetAsync(RunId));
        Assert.True(error is SagatException { Code: SagatErrorCode.Protocol or SagatErrorCode.Transport }, error.ToString());
    }

    [Fact]
    public async Task WhatTheServerSaysIsBoundedAndATraceIdThatIsNotHexIsNotRepeated()
    {
        server.Handler = _ => new FakeAnswer(
            404,
            "{\"code\":\"NOT_FOUND\",\"message\":\"" + new string('M', 5000) + "\",\"traceId\":\"" + new string('t', 100_000) + "\",\"details\":{\"reason\":\"" + new string('R', 200_000) + "\"}}",
            new Dictionary<string, string> { ["x-trace-id"] = "<script>alert(1)</script>" });
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient(o => o.Retry.MaxAttempts = 1).GetAsync(RunId));
        Assert.Equal(512, error.Message.Length);
        Assert.Equal(string.Empty, error.TraceId);
        Assert.Equal(128, error.Reason!.Length);
    }

    [Theory]
    [InlineData("0AF7651916CD43DD8448EB211C80319C")]
    [InlineData("0af7651916cd43dd8448eb211c80319")]
    [InlineData("0af7651916cd43dd8448eb211c80319c0")]
    [InlineData("0af7651916cd43dd8448eb211c80319\n")]
    [InlineData("zzf7651916cd43dd8448eb211c80319c")]
    public async Task OnlyThirtyTwoLowercaseHexCharactersAreAcceptedAsATraceId(string value)
    {
        server.Handler = _ => new FakeAnswer(404, "{\"code\":\"NOT_FOUND\",\"message\":\"x\",\"traceId\":\"" + value.Replace("\n", "\\n", StringComparison.Ordinal) + "\"}", new Dictionary<string, string> { ["x-trace-id"] = "no trace" });
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient(o => o.Retry.MaxAttempts = 1).GetAsync(RunId));
        Assert.Equal(string.Empty, error.TraceId);
    }

    [Fact]
    public async Task AValidTraceIdIsRepeatedFromTheEnvelopeAndFromTheHeader()
    {
        server.Handler = _ => new FakeAnswer(404, """{"code":"NOT_FOUND","message":"x","traceId":"0af7651916cd43dd8448eb211c80319c"}""");
        var fromEnvelope = await Assert.ThrowsAsync<SagatException>(() => NewClient(o => o.Retry.MaxAttempts = 1).GetAsync(RunId));
        Assert.Equal("0af7651916cd43dd8448eb211c80319c", fromEnvelope.TraceId);

        server.Handler = _ => new FakeAnswer(404, """{"code":"NOT_FOUND","message":"x","traceId":"not hex"}""", new Dictionary<string, string> { ["x-trace-id"] = "0af7651916cd43dd8448eb211c80319c" });
        var fromHeader = await Assert.ThrowsAsync<SagatException>(() => NewClient(o => o.Retry.MaxAttempts = 1).GetAsync(RunId));
        Assert.Equal("0af7651916cd43dd8448eb211c80319c", fromHeader.TraceId);
    }

    [Theory]
    [InlineData(long.MaxValue)]
    [InlineData(5_000_000_000)]
    [InlineData(3_600_000)]
    public async Task AWorkerHonoursAtMostAMinuteOfWhatTheServerAsksForAndTheErrorSaysSo(long retryAfterMs)
    {
        await using var fake = await FakeWorkerServer.StartAsync();
        fake.State.Fail("RegisterWorker", new Scripted(StatusCode.ResourceExhausted, null, retryAfterMs));
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "p", Token = "t", Retry = { MaxAttempts = 1 } };
        options.Worker.Name = "n";
        await using var host = new WorkerHost(options);
        host.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
        var error = await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        Assert.Equal(SagatErrorCode.RateLimited, error.Code);
        Assert.Equal(TimeSpan.FromSeconds(60), error.RetryAfter);
    }

    [Fact]
    public async Task AReasonOfAGrpcStatusIsBoundedToo()
    {
        await using var fake = await FakeWorkerServer.StartAsync();
        fake.State.Fail("RegisterWorker", new Scripted(StatusCode.Internal, new string('R', 50_000)));
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "p", Token = "t", Retry = { MaxAttempts = 1 } };
        options.Worker.Name = "n";
        await using var host = new WorkerHost(options);
        host.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
        var error = await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        Assert.Equal(128, error.Reason!.Length);
    }

    // -------------------------------------------------------------------------- 8. a regex that ends with $

    [Theory]
    [InlineData("0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")]
    public void ATraceparentWithATrailingNewlineIsNotValid(string value)
    {
        Assert.True(Traceparent.IsValid("00-" + value));
        Assert.False(Traceparent.IsValid("00-" + value + "\n"));
        Assert.False(Traceparent.IsValid("00-" + value + "\r\n"));
        Assert.Null(Traceparent.TraceIdOf("00-" + value + "\n"));
        Assert.Throws<SagatException>(() => Traceparent.Check("00-" + value + "\n"));
    }

    [Theory]
    [InlineData("abcdefgh\n")]
    [InlineData("abcdefgh ")]
    [InlineData("abcdefgh\t")]
    [InlineData("abcdefghç")]
    public async Task AnIdempotencyKeyWithAControlCharacterOrASpaceIsRefusedBeforeAnythingIsSent(string key)
    {
        server.Handler = _ => new FakeAnswer(202, Run);
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync(WorkflowId, new { }, new StartOptions { IdempotencyKey = key }));
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Empty(server.Requests);
    }

    [Fact]
    public async Task AWorkflowIdWithATrailingNewlineIsAWorkflowNameAndNotAnId()
    {
        server.Handler = request => request.PathAndQuery.StartsWith("/v1/workflows", StringComparison.Ordinal)
            ? new FakeAnswer(200, """{"items":[],"nextCursor":null}""")
            : new FakeAnswer(202, Run);
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync(WorkflowId + "\n", new { }));
        Assert.Equal("WORKFLOW_NOT_FOUND", error.Reason);
        Assert.Contains(server.Requests, r => r.PathAndQuery.StartsWith("/v1/workflows", StringComparison.Ordinal));
        Assert.DoesNotContain(server.Requests, r => r.Method == "POST");
    }

    [Fact]
    public async Task AUuidRunIdWithATrailingNewlineIsRefused()
    {
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(RunId + "\n"));
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Empty(server.Requests);
    }

    // ----------------------------------------------------------------------------- 10. names and ids in a path

    public static TheoryData<string> BadRunIds => new() { string.Empty, ".", "..", "r-1", "a/b", "a?b", "a#b", "a%2fb", "0190aaaa-0000-7000-8000-00000000000g", " ", RunId + " ", "../" + RunId };

    [Theory]
    [MemberData(nameof(BadRunIds))]
    public async Task ARunIdThatIsNotAUuidIsRefusedByEveryMethodBeforeAnythingIsSent(string runId)
    {
        var client = NewClient();
        server.Handler = _ => new FakeAnswer(200, Run);
        var calls = new Func<Task>[]
        {
            () => client.GetAsync(runId),
            () => client.CancelAsync(runId, "no longer needed by anyone"),
            () => client.OperateAsync(runId, RunAction.Pause, "maintenance window"),
            () => client.SignalAsync(runId, "payment-confirmed", new { }),
            () => client.EventsAsync(runId),
            () => client.WaitForCompletionAsync(runId, new WaitOptions { Timeout = TimeSpan.FromMilliseconds(10) }),
        };
        foreach (var call in calls)
        {
            var error = await Assert.ThrowsAsync<SagatException>(call);
            Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        }

        Assert.Empty(server.Requests);
    }

    [Theory]
    [InlineData("")]
    [InlineData(".")]
    [InlineData("..")]
    public async Task ASignalNameOfNothingOfADotOrOfTwoDotsIsRefusedBeforeAnythingIsSent(string name)
    {
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().SignalAsync(RunId, name, new { }));
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Empty(server.Requests);
    }

    [Fact]
    public async Task AWorkflowOfNoNameIsRefusedAndTheOtherNamesAreEncodedInThePath()
    {
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync(string.Empty, new { }));
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Empty(server.Requests);

        server.Handler = _ => new FakeAnswer(202, """{"deliveredTo":[]}""");
        await NewClient().SignalAsync(RunId, "a/b c?d#e%f", new { });
        Assert.Equal($"/v1/runs/{RunId}/signals/a%2Fb%20c%3Fd%23e%25f", server.Requests[0].PathAndQuery);
    }

    // --------------------------------------------------------------------------- 11. the retry rule is by code

    [Theory]
    [InlineData(409, "CONFLICT", true, 1)]
    [InlineData(422, "VALIDATION_FAILED", true, 1)]
    [InlineData(403, "FORBIDDEN", true, 1)]
    [InlineData(401, "UNAUTHENTICATED", true, 1)]
    [InlineData(404, "NOT_FOUND", true, 1)]
    [InlineData(400, "BAD_REQUEST", true, 1)]
    [InlineData(429, "QUOTA_EXCEEDED", true, 1)]
    [InlineData(429, "RATE_LIMITED", true, 3)]
    [InlineData(429, "RATE_LIMITED", false, 1)]
    [InlineData(500, "INTERNAL", true, 3)]
    [InlineData(500, "INTERNAL", false, 1)]
    [InlineData(503, "UNAVAILABLE", true, 3)]
    [InlineData(503, "UNAVAILABLE", false, 1)]
    public async Task ACallIsRepeatedOnlyWhenItsCodeIsOneOfTheFourAndItSaysItMayBe(int status, string code, bool retryable, int attempts)
    {
        server.Handler = _ => new FakeAnswer(status, $$"""{"code":"{{code}}","message":"x","retryable":{{(retryable ? "true" : "false")}}}""");
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(RunId));
        Assert.Equal(attempts, server.Requests.Count);
        Assert.Equal(attempts > 1, error.Retryable);
    }

    [Theory]
    [InlineData(408)]
    [InlineData(502)]
    [InlineData(503)]
    [InlineData(504)]
    public async Task AnAnswerOfTheStatusesThatMeanUnavailableIsRepeatedWithoutAnyEnvelope(int status)
    {
        server.Handler = _ => new FakeAnswer(status, "<html>no envelope</html>");
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(RunId));
        Assert.Equal(SagatErrorCode.Unavailable, error.Code);
        Assert.Equal(3, server.Requests.Count);
    }

    [Fact]
    public async Task AKeyedPostIsRepeatedOnUnavailableAndNeverOnAConflictThatClaimsToBeRetryable()
    {
        server.Handler = _ => new FakeAnswer(409, """{"code":"CONFLICT","message":"x","retryable":true}""");
        await Assert.ThrowsAsync<SagatException>(() => NewClient().StartAsync(WorkflowId, new { }, new StartOptions { IdempotencyKey = "key-12345678" }));
        Assert.Single(server.Requests);
    }

    // ------------------------------------------------------------------------------ 12. the call the server lacks

    [Theory]
    [InlineData(404, """{"code":"NOT_FOUND","message":"no such route","retryable":false,"details":{"reason":"ROUTE_NOT_FOUND"}}""")]
    [InlineData(405, """{"code":"BAD_REQUEST","message":"wrong method","retryable":false,"details":{"reason":"METHOD_NOT_ALLOWED"}}""")]
    [InlineData(501, "not implemented")]
    [InlineData(404, "<html>no route</html>")]
    [InlineData(404, "")]
    public async Task ACallTheServerDoesNotHaveIsNotFoundWithTheReasonUnsupportedByServer(int status, string body)
    {
        server.Handler = _ => new FakeAnswer(status, body);
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(RunId));
        Assert.Equal(SagatErrorCode.NotFound, error.Code);
        Assert.Equal("UNSUPPORTED_BY_SERVER", error.Reason);
        Assert.Equal(status, error.HttpStatus);
        Assert.Single(server.Requests);
    }

    [Theory]
    [InlineData("""{"code":"NOT_FOUND","message":"no such run","retryable":false}""", null)]
    [InlineData("""{"code":"NOT_FOUND","message":"no such run","retryable":false,"details":{"reason":"RUN_NOT_FOUND"}}""", "RUN_NOT_FOUND")]
    public async Task ANotFoundEnvelopeWithoutTheRouteReasonsIsAPlainNotFound(string body, string? reason)
    {
        server.Handler = _ => new FakeAnswer(404, body);
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(RunId));
        Assert.Equal(SagatErrorCode.NotFound, error.Code);
        Assert.Equal(reason, error.Reason);
    }

    [Fact]
    public async Task A405WithoutTheReasonIsNotMistakenForAnOldServer()
    {
        server.Handler = _ => new FakeAnswer(405, """{"code":"BAD_REQUEST","message":"wrong method","retryable":false}""");
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(RunId));
        Assert.NotEqual("UNSUPPORTED_BY_SERVER", error.Reason);
        Assert.Equal(405, error.HttpStatus);
    }

    // ------------------------------------------------------------ round 4: the sentinel of a provider that failed is private

    [Theory]
    [InlineData(400, "BAD_REQUEST")]
    [InlineData(401, "UNAUTHENTICATED")]
    [InlineData(403, "FORBIDDEN")]
    [InlineData(409, "CONFLICT")]
    [InlineData(422, "VALIDATION_FAILED")]
    public async Task AServerThatSaysTokenProviderFailedDoesNotMakeACallRepeatable(int status, string code)
    {
        server.Handler = _ => new FakeAnswer(status, $$$"""{"code":"{{{code}}}","message":"x","retryable":true,"details":{"reason":"TOKEN_PROVIDER_FAILED"}}""");
        var error = await Assert.ThrowsAsync<SagatException>(() => NewClient().GetAsync(RunId));
        Assert.Single(server.Requests);
        Assert.False(error.Retryable);
        Assert.Equal("TOKEN_PROVIDER_FAILED", error.Reason);
    }

    [Fact]
    public async Task ATokenProviderThatFailsIsRepeatedBeforeAnythingIsSentAndTheErrorSaysWhoFailedOnceTheAttemptsAreUsed()
    {
        server.Handler = _ => new FakeAnswer(200, Run);
        var calls = 0;
        using var once = NewClient(o =>
        {
            o.Token = null;
            o.TokenProvider = _ => Interlocked.Increment(ref calls) == 1 ? throw new HttpRequestException("the identity provider hiccuped") : ValueTask.FromResult("tok");
        });
        Assert.Equal("RUNNING", (await once.GetAsync(RunId)).Status);
        Assert.Equal(2, calls);
        Assert.Single(server.Requests);

        var always = 0;
        using var broken = NewClient(o =>
        {
            o.Token = null;
            o.TokenProvider = _ => Interlocked.Increment(ref always) > 0 ? throw new HttpRequestException("secret-canary") : ValueTask.FromResult("tok");
        });
        var error = await Assert.ThrowsAsync<SagatException>(() => broken.GetAsync(RunId));
        Assert.Equal(3, always);
        Assert.Equal(SagatErrorCode.Unauthenticated, error.Code);
        Assert.Equal("TOKEN_PROVIDER_FAILED", error.Reason);
        Assert.Null(error.InnerException);
        Assert.DoesNotContain("secret-canary", error.ToString(), StringComparison.Ordinal);
        Assert.Single(server.Requests);
    }

    [Theory]
    [InlineData(StatusCode.InvalidArgument)]
    [InlineData(StatusCode.PermissionDenied)]
    [InlineData(StatusCode.Unauthenticated)]
    [InlineData(StatusCode.NotFound)]
    public async Task AGrpcTrailerThatSaysTokenProviderFailedDoesNotMakeAReportRepeatable(StatusCode code)
    {
        await using var fake = await FakeWorkerServer.StartAsync();
        fake.State.FailAlways("CompleteStep", new Scripted(code, "TOKEN_PROVIDER_FAILED"));
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "p", Token = "t" };
        options.Retry.InitialDelay = TimeSpan.FromMilliseconds(2);
        options.Retry.MaxDelay = TimeSpan.FromMilliseconds(5);
        options.Worker.Name = "sentinel-" + Guid.NewGuid().ToString("N");
        await using var host = new WorkerHost(options);
        host.RegisterActivity<JsonNode?, int>("charge", (_, _, _) => Task.FromResult(1));
        fake.State.Lease(Leases.Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => fake.State.Of("CompleteStep").Count >= 1, "the report");
        await Task.Delay(300);
        Assert.Single(fake.State.Of("CompleteStep"));
    }
}
