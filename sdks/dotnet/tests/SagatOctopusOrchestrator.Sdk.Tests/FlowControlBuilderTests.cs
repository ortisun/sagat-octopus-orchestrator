using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workflows;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>
/// Feature 022 (FR-022-035, FR-022-050): <c>concurrency</c> and <c>rateLimit</c> at the workflow level and on activity and
/// script steps. The builders assemble the policy with the IR's names; the server checks the bounds at publication.
/// </summary>
public class FlowControlBuilderTests
{
    private static readonly string Digest = "sha256:" + new string('0', 64);

    private static WorkflowBuilder ProviderSync() => Workflow.Create("provider-sync")
        .Concurrency(new ConcurrencyPolicy { Key = "input.accountId", Limit = 1 })
        .RateLimit(new RateLimitPolicy { Group = "provider-starts", Key = "input.providerId", Limit = 100, Per = RatePeriod.Minute, QueueTimeout = TimeSpan.FromMinutes(15) })
        .Step("fetch", "provider.fetch", new StepOptions { RateLimit = new RateLimitPolicy { Key = "input.providerId", Limit = 10, Per = RatePeriod.Second } })
        .End("done");

    private static WorkflowBuilder AccountSerial() => Workflow.Create("account-serial")
        .Concurrency(new ConcurrencyPolicy { Group = "orders-by-account", Key = "input.accountId", Limit = 1, QueueTimeout = TimeSpan.FromMinutes(10) })
        .Step("price", "catalog.price")
        .Step("notify", "crm.notify", new StepOptions { Concurrency = new ConcurrencyPolicy { Key = "steps.price.output.region", Limit = 2 } })
        .End("done");

    private static JsonNode Fixture(string name) =>
        JsonNode.Parse(File.ReadAllText(Repository.Path("tests", "contract", "flow-control-step-policies", "fixtures", $"{name}.workflow.json")))!;

    [Fact]
    public void TheBuildersWriteTheSharedFixturesByteForByte()
    {
        Assert.Equal(CanonicalJson.Write(Fixture("provider-sync")), ProviderSync().ToJson());
        Assert.Equal(CanonicalJson.Write(Fixture("account-serial")), AccountSerial().ToJson());
        Assert.True(JsonNode.DeepEquals(Fixture("provider-sync"), ProviderSync().Build()));
    }

    [Fact]
    public void NoPolicyWritesNoFieldSoEarlierDocumentsAreUnchanged()
    {
        var document = Workflow.Create("plain-flow").Step("a", "act.a").Script("s", Digest).End("done").Build();
        foreach (var key in new[] { "concurrency", "rateLimit" })
        {
            Assert.Null(document[key]);
            foreach (var node in document["nodes"]!.AsArray())
            {
                Assert.Null(node![key]);
            }
        }
    }

    [Fact]
    public void AScriptCarriesBothPoliciesWithEveryMember()
    {
        var node = Workflow.Create("gated-script").Script("s", Digest, new ScriptOptions
        {
            Concurrency = new ConcurrencyPolicy { Key = "input.tenant", Limit = 10000, Group = "scripts.a-b", QueueTimeout = TimeSpan.FromDays(1) },
            RateLimit = new RateLimitPolicy { Key = "input.tenant", Limit = 3, Per = RatePeriod.Hour, Group = "scripts.a-b", QueueTimeout = TimeSpan.FromMilliseconds(1) },
        }).End("done").Build()["nodes"]![0]!;
        Assert.Equal("""{"group":"scripts.a-b","key":"input.tenant","limit":10000,"queueTimeoutMs":86400000}""", CanonicalJson.Write(node["concurrency"]));
        Assert.Equal("""{"group":"scripts.a-b","key":"input.tenant","limit":3,"per":"hour","queueTimeoutMs":1}""", CanonicalJson.Write(node["rateLimit"]));
    }

    [Fact]
    public void TheLastWorkflowLevelCallWins()
    {
        var document = Workflow.Create("replaced-flow")
            .Concurrency(new ConcurrencyPolicy { Limit = 9 }).Concurrency(new ConcurrencyPolicy { Limit = 2 })
            .RateLimit(new RateLimitPolicy { Limit = 9, Per = RatePeriod.Hour }).RateLimit(new RateLimitPolicy { Limit = 4, Per = RatePeriod.Second })
            .Step("a", "act.a").End("done").Build();
        Assert.Equal("""{"limit":2}""", CanonicalJson.Write(document["concurrency"]));
        Assert.Equal("""{"limit":4,"per":"second"}""", CanonicalJson.Write(document["rateLimit"]));
    }

    [Fact]
    public void TheBuildersOnlyAssembleTheBoundsAreTheServers()
    {
        var document = Workflow.Create("unchecked-flow")
            .Concurrency(new ConcurrencyPolicy { Limit = 0, Group = "Not A Group" })
            .RateLimit(new RateLimitPolicy { Limit = 101, Per = RatePeriod.Second, QueueTimeout = TimeSpan.FromMilliseconds(86_400_001) })
            .Step("a", "act.a", new StepOptions { Concurrency = new ConcurrencyPolicy { Limit = 10001, Key = string.Empty } })
            .End("done").Build();
        Assert.Equal("""{"group":"Not A Group","limit":0}""", CanonicalJson.Write(document["concurrency"]));
        Assert.Equal("""{"limit":101,"per":"second","queueTimeoutMs":86400001}""", CanonicalJson.Write(document["rateLimit"]));
        Assert.Equal("""{"key":"","limit":10001}""", CanonicalJson.Write(document["nodes"]![0]!["concurrency"]));
    }

    [Fact]
    public void AQueueTimeoutThatIsNotWholeMillisecondsAndAnUndefinedPeriodAreRefused()
    {
        var odd = TimeSpan.FromTicks(15_000);
        foreach (var build in new Func<WorkflowBuilder>[]
        {
            () => Workflow.Create("odd-flow").Concurrency(new ConcurrencyPolicy { Limit = 1, QueueTimeout = odd }),
            () => Workflow.Create("odd-flow").RateLimit(new RateLimitPolicy { Limit = 1, Per = RatePeriod.Second, QueueTimeout = odd }),
            () => Workflow.Create("odd-flow").Step("a", "act.a", new StepOptions { Concurrency = new ConcurrencyPolicy { Limit = 1, QueueTimeout = odd } }),
            () => Workflow.Create("odd-flow").Script("s", Digest, new ScriptOptions { RateLimit = new RateLimitPolicy { Limit = 1, Per = RatePeriod.Second, QueueTimeout = odd } }),
        })
        {
            var error = Assert.Throws<SagatException>(build);
            Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
            Assert.Contains("queueTimeoutMs", error.Message, StringComparison.Ordinal);
        }

        var undefined = Assert.Throws<SagatException>(() => Workflow.Create("odd-flow").RateLimit(new RateLimitPolicy { Limit = 1, Per = (RatePeriod)7 }));
        Assert.Equal(SagatErrorCode.ValidationFailed, undefined.Code);
        Assert.Contains("rateLimit per", undefined.Message, StringComparison.Ordinal);
    }
}
