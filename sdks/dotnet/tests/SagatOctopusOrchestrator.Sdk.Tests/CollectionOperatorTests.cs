using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Sdk.Workflows;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>The operator factory of a collection and the <c>Operators</c> of its options (design 042).</summary>
public class CollectionOperatorTests
{
    private static Body Line() => Workflow.Body().Step("price", "line.price").Return("return", "steps.price.output");

    [Fact]
    public void TheFactoryWritesOneStagePerOperatorKeyedByItsName()
    {
        Assert.Equal("""{"filter":{"where":"element.total > 0"}}""", Ops.Filter("element.total > 0").ToJson());
        Assert.Equal("""{"map":{"select":"element.lines"}}""", Ops.Map("element.lines").ToJson());
        Assert.Equal("""{"flatMap":{"select":"element"}}""", Ops.FlatMap("element").ToJson());
        Assert.Equal("""{"skip":{"count":0}}""", Ops.Skip(0).ToJson());
        Assert.Equal("""{"take":{"count":1000}}""", Ops.Take(1000).ToJson());
        Assert.Equal("""{"distinct":{"by":"element.sku"}}""", Ops.Distinct("element.sku").ToJson());
        Assert.Equal("""{"distinct":{}}""", Ops.Distinct().ToJson());
        Assert.Equal("""{"window":{"count":25}}""", Ops.Window(25).ToJson());
        Assert.Equal("flatMap", Ops.FlatMap("element").Name);
    }

    [Fact]
    public void TheFactoryOnlyAssemblesAndLeavesRangesToTheServer()
    {
        Assert.Equal("""{"window":{"count":0}}""", Ops.Window(0).ToJson());
        Assert.Equal("""{"take":{"count":1001}}""", Ops.Take(1001).ToJson());
    }

    [Fact]
    public void ForEachAndMapCarryTheOperatorsInTheOrderGivenAsACopy()
    {
        var document = Workflow.Create("chained-flow")
            .Body("line", Line())
            .ForEach("lines", "input.lines", "line", "bodyResult", new CollectionOptions
            {
                MaxItems = 9,
                MaxConcurrency = 2,
                Operators = [Ops.Filter("element.ok"), Ops.Distinct(), Ops.Window(2)],
            })
            .Map("prices", "input.lines", "line", "bodyResult", new CollectionOptions { MaxItems = 9, MaxConcurrency = 2, Operators = [Ops.Take(3)] })
            .End("done")
            .Build();
        var nodes = (JsonArray)document["nodes"]!;
        Assert.Equal(
            """[{"filter":{"where":"element.ok"}},{"distinct":{}},{"window":{"count":2}}]""",
            CanonicalJson.Write(nodes[0]!["operators"]));
        Assert.Equal("""[{"take":{"count":3}}]""", CanonicalJson.Write(nodes[1]!["operators"]));
        Assert.Equal("forEach", (string)nodes[1]!["type"]!);

        var shared = Ops.Filter("element.ok");
        var first = Workflow.Create("shared-flow").Body("line", Line())
            .ForEach("a", "input.lines", "line", "bodyResult", new CollectionOptions { MaxItems = 2, MaxConcurrency = 1, Operators = [shared] })
            .ForEach("b", "input.lines", "line", "bodyResult", new CollectionOptions { MaxItems = 2, MaxConcurrency = 1, Operators = [shared] })
            .End("done")
            .Build();
        ((JsonObject)((JsonArray)first["nodes"]![0]!["operators"]!)[0]!["filter"]!)["where"] = "true";
        Assert.Equal("""{"filter":{"where":"element.ok"}}""", shared.ToJson());
    }

    [Fact]
    public void EveryPresentCollectionCallStillBuildsWithoutOperators()
    {
        var present = new CollectionOptions { MaxItems = 17, MaxConcurrency = 3 };
        var document = Workflow.Create("plain-flow")
            .Body("line", Line())
            .ForEach("lines", "input.lines", "line", "bodyResult", present)
            .Map("prices", "input.lines", "line", "bodyResult", new CollectionOptions { MaxItems = 17, MaxConcurrency = 3 })
            .ForEach("empty", "input.lines", "line", "bodyResult", present with { Operators = [] })
            .End("done")
            .Build();
        var nodes = (JsonArray)document["nodes"]!;
        for (var i = 0; i < 3; i++)
        {
            Assert.False(((JsonObject)nodes[i]!).ContainsKey("operators"));
        }

        Assert.Null(present.Operators);
    }

    [Fact]
    public void ANullStageIsAValidationFailure()
    {
        var error = Assert.Throws<SagatException>(() => Workflow.Create("null-flow").Body("line", Line())
            .ForEach("lines", "input.lines", "line", "bodyResult", new CollectionOptions { MaxItems = 2, MaxConcurrency = 1, Operators = [Ops.Take(1), null!] }));
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Contains("forEach lines operator 1", error.Message, StringComparison.Ordinal);
    }
}
