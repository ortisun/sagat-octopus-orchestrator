using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Sdk.Workflows;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary><c>reapplyOnReset</c> on an Update declaration (feature 018): an optional boolean, written only when true.</summary>
public class ReapplyOnResetTests
{
    private static JsonObject Integer() => JsonNode.Parse("""{"type":"integer"}""")!.AsObject();

    private static JsonObject Declared(bool reapplyOnReset)
    {
        var update = new UpdateDefinition
        {
            ArgsSchema = Integer(),
            State = "state",
            Result = "newState.balance",
            ResultSchema = Integer(),
            Disclosure = "declared-business-result-v1",
            ReapplyOnReset = reapplyOnReset,
        };
        var document = Workflow.Create("account-state")
            .State(new StateDefinition
            {
                Schema = JsonNode.Parse("""{"type":"object","properties":{"balance":{"type":"integer"}},"required":["balance"],"additionalProperties":false}""")!.AsObject(),
                Initial = "input.account",
                Updates = new Dictionary<string, UpdateDefinition> { ["balance.credit"] = update },
            })
            .End("done")
            .Build();
        return document["state"]!["updates"]!["balance.credit"]!.AsObject();
    }

    [Fact]
    public void AnUpdateThatOptsInWritesTheField()
    {
        Assert.True((bool?)Declared(true)["reapplyOnReset"]);
    }

    [Fact]
    public void TheDefaultIsNotWrittenSoADocumentThatDoesNotOptInIsTheOneMadeBeforeTheField()
    {
        Assert.False(Declared(false).ContainsKey("reapplyOnReset"));
        Assert.False(new UpdateDefinition { ArgsSchema = null, State = null, Result = null, ResultSchema = null, Disclosure = null }.ReapplyOnReset);
    }
}
