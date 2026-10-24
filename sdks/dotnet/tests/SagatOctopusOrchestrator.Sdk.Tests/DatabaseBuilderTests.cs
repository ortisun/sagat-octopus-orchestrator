using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workflows;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

public class DatabaseBuilderTests
{
    [Fact]
    public void SharedTypedFixtureAndCanonicalBytesArePreserved()
    {
        var fixture = JsonNode.Parse(File.ReadAllText(Repository.Path("tests", "contract", "database-connections", "typed-parameters.workflow.json")))!;
        var node = fixture["nodes"]![0]!;
        var options = new DatabaseOptions
        {
            ConnectionRef = new ConnectionRef((string)node["connectionRef"]!["name"]!, (int?)node["connectionRef"]!["revision"]),
            Operation = DatabaseOperation.Query,
            Statement = (string)node["statement"]!,
            Parameters = node["parameters"]!.AsArray().Select(p => p!["literal"] is { } literal
                ? DatabaseParameter.Literal(Enum.Parse<DatabaseType>((string)literal["type"]!, true), literal["value"])
                : DatabaseParameter.Expression(Enum.Parse<DatabaseType>((string)p["type"]!, true), (string)p["expression"]!)).ToArray(),
        };
        Assert.Equal(CanonicalJson.Write(fixture), Workflow.Create((string)fixture["metadata"]!["name"]!).Database((string)node["id"]!, options).ToJson());
    }

    [Fact]
    public void SnapshotsCallerValuesAndRejectsInvalidShapesWithoutPrintingThem()
    {
        var value = new JsonObject { ["amount"] = "9223372036854775807" };
        var options = new DatabaseOptions
        {
            ConnectionRef = new ConnectionRef("orders-db"),
            Operation = DatabaseOperation.Query,
            Statement = "SELECT $1",
            Parameters = [DatabaseParameter.Literal(DatabaseType.Json, value)],
        };
        var builder = Workflow.Create("database-chain").Database("q", options).End("done");
        value["amount"] = "changed";
        var node = builder.Build()["nodes"]![0]!;
        Assert.Equal("9223372036854775807", (string?)node["parameters"]![0]!["literal"]!["value"]!["amount"]);
        Assert.Equal("done", (string?)node["next"]);
        Assert.Null(node["retry"]);
        Assert.Null(node["secrets"]);
        foreach (var bad in new[]
        {
            options with { ConnectionRef = new ConnectionRef("postgres://credential-canary@host/db") },
            options with { Statement = "" },
            options with { Parameters = [DatabaseParameter.Literal(DatabaseType.Integer, 123)] },
            options with { Parameters = [DatabaseParameter.Literal(DatabaseType.Decimal, "credential-canary")] },
            options with { ConnectionRef = new ConnectionRef("orders-db", 0) },
        })
        {
            var error = Assert.Throws<SagatException>(() => Workflow.Create("database-invalid").Database("q", bad));
            Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
            Assert.DoesNotContain("credential-canary", error.ToString(), StringComparison.Ordinal);
        }
    }
}
