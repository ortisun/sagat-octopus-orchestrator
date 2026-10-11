using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workers;
using SagatOctopusOrchestrator.Sdk.Workflows;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>SPEC section 7 and 8: what the canonical text refuses, and the names and durations the builders refuse.</summary>
public sealed class NamesAndCanonicalTests
{
    private static string Canonical(JsonNode? node) => CanonicalJson.Write(node);

    private static WorkflowBuilder WithSchema(JsonNode? maximum) =>
        Workflow.Create("canonical-flow").Step("a", "act.a").InputSchema(new JsonObject { ["maximum"] = maximum });

    // ------------------------------------------------------------------------- 9. integers beyond 2^53 - 1

    [Theory]
    [InlineData("9007199254740992")]
    [InlineData("9007199254740993")]
    [InlineData("12345678901234567890")]
    [InlineData("-9007199254740993")]
    [InlineData("123456789012345678")]
    [InlineData("100000000000000000000")]
    [InlineData("999999999999999900000")]
    public void AnIntegerBeyondTwoToThe53MinusOneIsRefusedByTheCanonicalText(string token)
    {
        var builder = WithSchema(JsonNode.Parse(token));
        var document = builder.Build();
        Assert.NotNull(document);
        var error = Assert.Throws<SagatException>(() => builder.ToJson());
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Throws<SagatException>(() => Canonical(JsonNode.Parse(token)));
    }

    [Fact]
    public void ALongBeyondTwoToThe53MinusOneIsRefusedToo()
    {
        Assert.Throws<SagatException>(() => Canonical(JsonValue.Create(9007199254740993L)));
        Assert.Throws<SagatException>(() => Canonical(JsonValue.Create(long.MaxValue)));
        Assert.Equal("9007199254740991", Canonical(JsonValue.Create(9007199254740991L)));
        Assert.Equal("-9007199254740991", Canonical(JsonValue.Create(-9007199254740991L)));
    }

    [Theory]
    [InlineData("1e21", "1e+21")]
    [InlineData("1e22", "1e+22")]
    [InlineData("5e-324", "5e-324")]
    [InlineData("0.5", "0.5")]
    [InlineData("1.0", "1")]
    [InlineData("9007199254740991", "9007199254740991")]
    public void ANumberThatIsNotAnIntegerBeyondTheSafeRangeIsStillWrittenAsEcmaScriptWritesIt(string token, string expected)
    {
        Assert.Equal(expected, Canonical(JsonNode.Parse(token)));
    }

    [Fact]
    public void AnExponentIsNeverExpandedInMemory()
    {
        var clock = System.Diagnostics.Stopwatch.StartNew();
        Assert.Throws<SagatException>(() => Canonical(JsonNode.Parse("1e999999999")));
        Assert.Equal("0", Canonical(JsonNode.Parse("0e999999999")));
        Assert.True(clock.ElapsedMilliseconds < 2000);
    }

    // ----------------------------------------------------------------------------------- 9. a lone surrogate

    [Fact]
    public void ALoneSurrogateIsRefusedByTheCanonicalTextInAValueAndInAKey()
    {
        var value = Workflow.Create("canonical-flow").Description("x\ud800y").Step("a", "act.a");
        Assert.NotNull(value.Build());
        Assert.Equal(SagatErrorCode.ValidationFailed, Assert.Throws<SagatException>(() => value.ToJson()).Code);

        var key = Workflow.Create("canonical-flow").Label("k\udc00", "v").Step("a", "act.a");
        Assert.Equal(SagatErrorCode.ValidationFailed, Assert.Throws<SagatException>(() => key.ToJson()).Code);

        Assert.Throws<SagatException>(() => Canonical(JsonValue.Create("\ud800")));
        Assert.Throws<SagatException>(() => Canonical(JsonValue.Create("ok \ud83d")));
        Assert.Equal("\"😀\"", Canonical(JsonValue.Create("😀")));
    }

    // -------------------------------------------------------------------------------- 9. whole milliseconds only

    public static TheoryData<Action> FractionsOfAMillisecond => new()
    {
        () => Workflow.Create("ms-flow").Sleep("t", TimeSpan.FromTicks(15_000)),
        () => Workflow.Create("ms-flow").Step("a", "x", new StepOptions { Timeout = TimeSpan.FromTicks(15_000) }),
        () => Workflow.Create("ms-flow").Step("a", "x", new StepOptions { CompensationTimeout = TimeSpan.FromTicks(20_001) }),
        () => Workflow.Create("ms-flow").Step("a", "x", new StepOptions { Retry = new StepRetry { InitialDelay = TimeSpan.FromTicks(4_000) } }),
        () => Workflow.Create("ms-flow").Step("a", "x", new StepOptions { Retry = new StepRetry { MaxDelay = TimeSpan.FromTicks(10_001) } }),
        () => Workflow.Create("ms-flow").Step("a", "x", new StepOptions { CompensationRetry = new StepRetry { InitialDelay = TimeSpan.FromTicks(5) } }),
        () => Workflow.Create("ms-flow").WaitForEvent("w", new WaitForEventOptions { EventType = "e", CorrelationExpr = "x", Timeout = TimeSpan.FromTicks(15_000) }),
        () => Workflow.Create("ms-flow").WaitForEvent("w", new WaitForEventOptions { EventType = "e", CorrelationExpr = "x", Timeout = TimeSpan.FromSeconds(1), Lookback = TimeSpan.FromTicks(15_000) }),
    };

    [Theory]
    [MemberData(nameof(FractionsOfAMillisecond))]
    public void ADurationThatIsNotAWholeNumberOfMillisecondsIsRefusedAndNeverTruncated(Action build)
    {
        var error = Assert.Throws<SagatException>(build);
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Contains("millisecond", error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void AWholeNumberOfMillisecondsIsAccepted()
    {
        var text = Workflow.Create("ms-flow")
            .Sleep("t", TimeSpan.FromMilliseconds(2))
            .Step("a", "x", new StepOptions { Timeout = TimeSpan.FromMilliseconds(1500), Retry = new StepRetry { InitialDelay = TimeSpan.FromMilliseconds(1), MaxDelay = TimeSpan.FromSeconds(1) } })
            .ToJson();
        Assert.Contains("\"durationMs\":2", text, StringComparison.Ordinal);
        Assert.Contains("\"timeoutMs\":1500", text, StringComparison.Ordinal);
    }

    // ------------------------------------------------------------------------------- 8. a regex that ends with $

    [Fact]
    public void ANameOrASecretWithATrailingNewlineIsRefused()
    {
        Assert.Throws<SagatException>(() => Workflow.Create("abc-flow\n"));
        Assert.Throws<SagatException>(() => Workflow.Create("abc-flow\r\n"));
        Assert.Throws<SagatException>(() => Workflow.Create("abc-flow").Step("a", "x", new StepOptions { Secrets = ["payments-key\n"] }));
        Assert.NotNull(Workflow.Create("abc-flow").Step("a", "x", new StepOptions { Secrets = ["payments-key"] }));
    }

    [Fact]
    public async Task AnErrorCodeAndAProtocolVersionWithATrailingNewlineAreNotAccepted()
    {
        await using var fake = await FakeWorkerServer.StartAsync();
        fake.State.AcceptedVersion = "1.2\n";
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "p", Token = "t", Retry = { MaxAttempts = 1 } };
        options.Worker.Name = "newline-" + Guid.NewGuid().ToString("N");
        options.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(20);
        await using var host = new WorkerHost(options);
        SagatException? secrets = null;
        host.RegisterActivity<JsonNode?, int>("charge", async (_, context, ct) =>
        {
            try
            {
                await context.GetSecretsAsync(ct);
            }
            catch (SagatException error)
            {
                secrets = error;
            }

            throw new ActivityFailure("ABC\n", "x", retryable: false);
        });
        fake.State.Lease(Leases.Lease());
        await host.StartAsync();
        await Eventually.UntilAsync(() => fake.State.Of("FailStep").Count == 1, "the failure");

        // "1.2\n" is not "1.2": the host runs at 1.0, which has no ResolveSecrets, and the heartbeat has no step identity.
        Assert.Equal(SagatErrorCode.UnsupportedProtocol, secrets!.Code);
        Assert.Equal("WORKER_ERROR", ((FailStepRequest)fake.State.Of("FailStep")[0].Request).ErrorCode);
    }
}
