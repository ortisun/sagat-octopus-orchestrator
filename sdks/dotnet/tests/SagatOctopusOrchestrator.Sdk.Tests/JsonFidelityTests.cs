using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workers;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>
/// Numbers, strings and instants are not reformatted (SPEC section 8): <c>600000</c> stays <c>600000</c> and not <c>600000.0</c>, a big
/// integer stays an integer, <c>"19.99"</c> stays a string, and what a JavaScript SDK writes for the same value is what this one writes.
/// </summary>
public sealed class JsonFidelityTests
{
    private static SortedSet<string> NumberTokensOfTheScenarios()
    {
        var tokens = new SortedSet<string>(StringComparer.Ordinal);
        foreach (var path in Directory.GetFiles(Repository.Path("tests", "contract", "sdk-conformance", "scenarios"), "*.json"))
        {
            using var document = JsonDocument.Parse(File.ReadAllBytes(path));
            Collect(document.RootElement, tokens);
        }

        return tokens;
    }

    private static void Collect(JsonElement element, SortedSet<string> tokens)
    {
        switch (element.ValueKind)
        {
            case JsonValueKind.Number:
                tokens.Add(element.GetRawText());
                break;
            case JsonValueKind.Object:
                foreach (var property in element.EnumerateObject())
                {
                    Collect(property.Value, tokens);
                }

                break;
            case JsonValueKind.Array:
                foreach (var item in element.EnumerateArray())
                {
                    Collect(item, tokens);
                }

                break;
            default:
                break;
        }
    }

    [Fact]
    public void EveryNumberOfTheConformanceScenariosIsWrittenAsJavaScriptWritesIt()
    {
        var tokens = NumberTokensOfTheScenarios().ToList();
        Assert.Contains("0.5", tokens);
        Assert.Contains("0.1", tokens);
        Assert.Contains("600000", tokens);
        Assert.Contains("9007199254740991", tokens);
        Assert.Contains("-42", tokens);
        Assert.Contains("1e+21", tokens);
        Assert.Contains("-1e-07", tokens);
        foreach (var token in tokens)
        {
            var parsed = JsonNode.Parse(token);

            // A node read from the file is written back unchanged (the SDK never reformats what it was given) ...
            Assert.Equal(token, parsed!.ToJsonString());

            // ... the canonical text is the same number, spelled as ECMAScript spells it: the token itself when it is already in that form ...
            var canonical = Canonical(parsed);
            Assert.Equal(double.Parse(token, CultureInfo.InvariantCulture), double.Parse(canonical, CultureInfo.InvariantCulture));
            if (!token.Contains('e', StringComparison.Ordinal))
            {
                Assert.Equal(token, canonical);
            }

            // ... and so is the same value held in a .NET number.
            if (!token.Contains('e', StringComparison.Ordinal))
            {
                var typed = token.Contains('.', StringComparison.Ordinal)
                    ? JsonSerializer.Serialize(double.Parse(token, CultureInfo.InvariantCulture))
                    : JsonSerializer.Serialize(long.Parse(token, CultureInfo.InvariantCulture));
                Assert.Equal(token, typed);
            }
        }

        // The exponent forms of the scenario files (written by Python) are spelled as JavaScript spells them in the canonical text.
        Assert.Equal("1e+21", Canonical(JsonNode.Parse("1e+21")));
        Assert.Equal("1e-7", Canonical(JsonNode.Parse("1e-07")));
        Assert.Equal("0.000001", Canonical(JsonNode.Parse("1e-06")));
        Assert.Equal("-1e-7", Canonical(JsonNode.Parse("-1e-07")));
    }

    private static string Canonical(JsonNode? node) => CanonicalJson.Write(node);

    [Fact]
    public void ADoubleThatIsAWholeNumberIsWrittenWithoutAFractionAsJavaScriptDoes()
    {
        Assert.Equal("600000", JsonSerializer.Serialize(600000d));
        Assert.Equal("600000", JsonSerializer.Serialize(TimeSpan.FromMilliseconds(600000).TotalMilliseconds));
        Assert.Equal("19.99", JsonSerializer.Serialize(19.99m));
        Assert.Equal("9223372036854775807", JsonSerializer.Serialize(long.MaxValue));
    }

    [Fact]
    public async Task AStartKeepsEveryNumberAndEveryStringOfAJsonInputAsItWasGiven()
    {
        await using var server = await FakeRestServer.StartAsync();
        server.Handler = _ => new FakeAnswer(202, """{"runId":"r","workflowId":"w","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z"}""");
        using var client = new SagatClient(new SagatOptions { ApiUrl = server.BaseUrl, Namespace = "p", Token = "t" });
        var input = JsonNode.Parse(
            """{"ms":600000,"half":0.5,"tenth":0.1,"big":9007199254740993,"huge":12345678901234567890,"neg":-42,"money":"19.99","decimal":"12345678901234567890.123456789","text":"ação € 😀","esc":"q\" b\\ t\t n\n","nothing":null,"emptyObject":{},"emptyList":[],"list":[1,"two",null,{"deep":[true,false]}]}""");
        await client.StartAsync("0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee", input);
        var sent = JsonNode.Parse(server.Requests[0].BodyText)!["input"]!;
        Assert.Equal(input!.ToJsonString(), sent.ToJsonString());
        var text = server.Requests[0].BodyText;
        foreach (var literal in new[] { "\"ms\":600000", "\"half\":0.5", "\"tenth\":0.1", "\"big\":9007199254740993", "\"huge\":12345678901234567890", "\"neg\":-42", "\"money\":\"19.99\"", "\"decimal\":\"12345678901234567890.123456789\"" })
        {
            Assert.Contains(literal, text, StringComparison.Ordinal);
        }

        Assert.DoesNotContain("600000.0", text, StringComparison.Ordinal);
    }

    [Fact]
    public async Task ATypedInputIsWrittenInCamelCaseWithNumbersAsTheyAre()
    {
        await using var server = await FakeRestServer.StartAsync();
        server.Handler = _ => new FakeAnswer(202, """{"runId":"r","workflowId":"w","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z"}""");
        using var client = new SagatClient(new SagatOptions { ApiUrl = server.BaseUrl, Namespace = "p", Token = "t" });
        await client.StartAsync("0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee", new { OrderId = "o-1", AmountMs = 600000L, Rate = 0.5, Total = 19.99m, Money = "19.99", Missing = (string?)null });
        Assert.Equal(
            """{"orderId":"o-1","amountMs":600000,"rate":0.5,"total":19.99,"money":"19.99","missing":null}""",
            server.Requests[0].Body!["input"]!.ToJsonString());
    }

    [Fact]
    public async Task ARunInstantWithAnyOffsetIsReturnedInUtcAndEveryInstantTheSdkWritesIsUtcWithAZ()
    {
        await using var server = await FakeRestServer.StartAsync();
        server.Handler = request => request.PathAndQuery.EndsWith("/events", StringComparison.Ordinal)
            ? new FakeAnswer(202, """{"eventId":"evt-12345678","source":"api","status":"ACCEPTED"}""")
            : new FakeAnswer(200, """{"runId":"r","workflowId":"w","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T15:30:00.123456+05:30","updatedAt":"2026-10-10T10:00:00Z","deadline":"2026-10-10T12:00:00-03:00"}""");
        using var client = new SagatClient(new SagatOptions { ApiUrl = server.BaseUrl, Namespace = "p", Token = "t" });
        var run = await client.GetAsync(Ids.Run);
        Assert.Equal(new DateTimeOffset(2026, 10, 10, 10, 0, 0, TimeSpan.Zero).AddTicks(1234560), run.CreatedAt);
        Assert.Equal(TimeSpan.Zero, run.CreatedAt.Offset);
        Assert.Equal(TimeSpan.Zero, run.Deadline!.Value.Offset);
        Assert.Equal(new DateTimeOffset(2026, 10, 10, 15, 0, 0, TimeSpan.Zero), run.Deadline);

        await client.StartAsync("0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee", new { }, new StartOptions { Deadline = new DateTimeOffset(2030, 5, 6, 7, 8, 9, 10, TimeSpan.FromHours(-7)) });
        Assert.Equal("2030-05-06T14:08:09.010Z", (string?)server.Requests[^1].Body!["deadline"]);
        await client.PublishEventAsync(new PublishEventRequest { Type = "t", EventId = "evt-12345678", OccurredAt = new DateTimeOffset(2030, 5, 6, 7, 8, 9, TimeSpan.FromHours(2)) });
        Assert.Equal("2030-05-06T05:08:09.000Z", (string?)server.Requests[^1].Body!["occurredAt"]);
    }

    [Fact]
    public async Task AnActivityThatEchoesItsJsonInputGivesBackTheSameNumbersAndStrings()
    {
        await using var fake = await FakeWorkerServer.StartAsync();
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "p", Token = "t" };
        options.Worker.Name = "fidelity-" + Guid.NewGuid().ToString("N");
        await using var host = new WorkerHost(options);
        host.RegisterActivity<JsonNode?, JsonNode?>("echo", (input, _, _) => Task.FromResult(input?.DeepClone()));
        var document = """{"decimal":"12345678901234567890.123456789","money":"19.90","maxSafe":9007199254740991,"negative":-42,"fraction":0.1,"half":0.5,"zero":0,"text":"ação € 😀","escapes":"quote \" backslash \\ tab \t newline \n","nothing":null,"emptyObject":{},"emptyList":[],"list":[1,"two",null,{"deep":[true,false]}]}""";
        fake.State.Lease(Leases.Lease(l =>
        {
            l.ActivityName = "echo";
            l.InputJson = "{\"run\":{\"id\":\"r\"},\"node\":\"echo\",\"input\":" + document + ",\"steps\":{}}";
        }));
        await host.StartAsync();
        await Eventually.UntilAsync(() => fake.State.Of("CompleteStep").Count == 1, "the completion");
        var output = ((CompleteStepRequest)fake.State.Of("CompleteStep")[0].Request).OutputJson;
        Assert.Equal(CanonicalJson.Write(JsonNode.Parse(document)), CanonicalJson.Write(JsonNode.Parse(output)));
        Assert.Contains("\"half\":0.5", output, StringComparison.Ordinal);
        Assert.Contains("\"maxSafe\":9007199254740991", output, StringComparison.Ordinal);
        Assert.Contains("\"money\":\"19.90\"", output, StringComparison.Ordinal);
        Assert.Contains("\"zero\":0", output, StringComparison.Ordinal);
    }

    [Fact]
    public void AStepTimeoutInMillisecondsIsAnIntegerInTheDocument()
    {
        var text = Workflows.Workflow.Create("integers-flow")
            .Step("a", "act.a", new Workflows.StepOptions { Timeout = TimeSpan.FromMilliseconds(600000), CompensationTimeout = TimeSpan.FromSeconds(10), Retry = new Workflows.StepRetry { MaxAttempts = 3, InitialDelay = TimeSpan.FromMilliseconds(50) } })
            .Sleep("s", TimeSpan.FromMilliseconds(250))
            .ToJson();
        Assert.Contains("\"timeoutMs\":600000", text, StringComparison.Ordinal);
        Assert.Contains("\"compensationTimeoutMs\":10000", text, StringComparison.Ordinal);
        Assert.Contains("\"initialDelayMs\":50", text, StringComparison.Ordinal);
        Assert.Contains("\"durationMs\":250", text, StringComparison.Ordinal);
        Assert.DoesNotContain(".0", text, StringComparison.Ordinal);
    }
}
