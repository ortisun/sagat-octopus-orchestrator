using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workflows;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

public class IcebergBuilderTests
{
    private static IcebergWriteOptions Base() => new()
    {
        ConnectionRef = new ConnectionRef("lake-main", 3),
        Table = new IcebergTable(["sales"], "orders"),
        Records = IcebergRecords.Inline("steps.extract.output.rows"),
    };

    private static JsonNode NodeOf(IcebergWriteOptions options) => Workflow.Create("lake").IcebergWrite("w", options).End("done").Build()["nodes"]![0]!;

    [Fact]
    public void MatchesTheSharedValidDocument()
    {
        var fixture = JsonNode.Parse(File.ReadAllText(Repository.Path("tests", "contract", "iceberg-writes", "fixtures", "ir", "valid", "append-inline.json")))!;
        var built = Workflow.Create((string)fixture["metadata"]!["name"]!).Step("extract", "orders.extract").IcebergWrite("load", Base()).End("done").ToJson();
        Assert.Equal(CanonicalJson.Write(fixture), built);
    }

    [Fact]
    public void TheModeIsAlwaysAppendAndAMinimalWriteCarriesNothingItWasNotGiven()
    {
        var node = NodeOf(Base());
        Assert.Equal("append", (string?)node["mode"]);
        Assert.Equal("icebergWrite", (string?)node["type"]);
        foreach (var absent in new[] { "ref", "mapping", "limits", "compensation", "ignoreUnknownFields", "retry", "secrets", "overwrite" })
        {
            Assert.Null(node[absent]);
        }
    }

    [Fact]
    public void EveryOptionIsCarriedAndTheThreeRecordSourcesAreAccepted()
    {
        var node = Workflow.Create("lake").IcebergWrite("w", Base() with
        {
            Ref = "staging",
            Mapping = [new IcebergMapping("/amount", "amount"), new IcebergMapping("/customer/name", "customer_name")],
            IgnoreUnknownFields = true,
            Limits = new IcebergLimits { MaxRecordBytes = 1024, MaxRecords = 100000000, Timeout = TimeSpan.FromMilliseconds(3600000) },
            Compensation = IcebergCompensation.Revert,
            OnError = "recover",
        }).Step("recover", "orders.recover").End("done").Build()["nodes"]![0]!;
        Assert.Equal("staging", (string?)node["ref"]);
        Assert.Equal(1024, (long?)node["limits"]!["maxRecordBytes"]);
        Assert.Equal(100000000, (long?)node["limits"]!["maxRecords"]);
        Assert.Equal(3600000, (long?)node["limits"]!["timeoutMs"]);
        Assert.Equal("revert", (string?)node["compensation"]);
        Assert.Equal("recover", (string?)node["onError"]);
        Assert.True((bool?)node["ignoreUnknownFields"]);
        foreach (var records in new[] { IcebergRecords.Inline("input.rows"), IcebergRecords.Blobs("steps.a.output.parts"), IcebergRecords.DatabaseResult("read") })
        {
            NodeOf(Base() with { Records = records });
        }
    }

    [Fact]
    public void TheNodeIsASnapshotLaterChangesToTheOptionsDoNotReachIt()
    {
        var namespaceLevels = new List<string> { "sales" };
        var mapping = new List<IcebergMapping> { new("/a", "a") };
        var builder = Workflow.Create("lake").IcebergWrite("w", Base() with { Table = new IcebergTable(namespaceLevels, "orders"), Mapping = mapping }).End("done");
        namespaceLevels[0] = "changed";
        mapping[0] = new IcebergMapping("/changed", "changed");
        Assert.DoesNotContain("changed", builder.ToJson(), StringComparison.Ordinal);
    }

    public static TheoryData<string, IcebergWriteOptions> Refused()
    {
        var data = new TheoryData<string, IcebergWriteOptions>();
        var options = Base();
        void Add(string name, IcebergWriteOptions refused) => data.Add(name, refused);
        Add("connection is a url", options with { ConnectionRef = new ConnectionRef("postgres://credential-canary@host/db") });
        Add("revision zero", options with { ConnectionRef = new ConnectionRef("lake-main", 0) });
        Add("double dash", options with { ConnectionRef = new ConnectionRef("lake--main") });
        Add("no namespace", options with { Table = new IcebergTable([], "orders") });
        Add("nine levels", options with { Table = new IcebergTable(["a", "b", "c", "d", "e", "f", "g", "h", "i"], "orders") });
        Add("empty name", options with { Table = new IcebergTable(["sales"], "") });
        Add("dot dot name", options with { Table = new IcebergTable(["sales"], "..") });
        Add("control in a level", options with { Table = new IcebergTable(["sales\0"], "orders") });
        Add("empty ref", options with { Ref = "" });
        Add("long ref", options with { Ref = new string('r', 256) });
        Add("empty expression", options with { Records = IcebergRecords.Inline("") });
        Add("expression of 1025 bytes", options with { Records = IcebergRecords.Inline(new string('x', 1025)) });
        Add("513 two-byte letters", options with { Records = IcebergRecords.Blobs(new string('é', 513)) });
        Add("empty database result", options with { Records = IcebergRecords.DatabaseResult("") });
        Add("pointer without slash", options with { Mapping = [new IcebergMapping("amount", "amount")] });
        Add("pointer with empty segment", options with { Mapping = [new IcebergMapping("/a//b", "a")] });
        Add("pointer with trailing slash", options with { Mapping = [new IcebergMapping("/a/", "a")] });
        Add("pointer with a bad escape", options with { Mapping = [new IcebergMapping("/a~2", "a")] });
        Add("pointer with 17 segments", options with { Mapping = [new IcebergMapping("/" + string.Concat(Enumerable.Repeat("a/", 16)) + "a", "a")] });
        Add("empty column", options with { Mapping = [new IcebergMapping("/a", "")] });
        Add("column twice", options with { Mapping = [new IcebergMapping("/a", "c"), new IcebergMapping("/b", "c")] });
        Add("field twice", options with { Mapping = [new IcebergMapping("/a", "c"), new IcebergMapping("/a", "d")] });
        Add("zero records", options with { Limits = new IcebergLimits { MaxRecords = 0 } });
        Add("too many records", options with { Limits = new IcebergLimits { MaxRecords = 100000001 } });
        Add("record bytes below the edge", options with { Limits = new IcebergLimits { MaxRecordBytes = 1023 } });
        Add("record bytes above the edge", options with { Limits = new IcebergLimits { MaxRecordBytes = 2147483649L } });
        Add("timeout below the edge", options with { Limits = new IcebergLimits { Timeout = TimeSpan.FromMilliseconds(999) } });
        Add("timeout above the edge", options with { Limits = new IcebergLimits { Timeout = TimeSpan.FromMilliseconds(3600001) } });
        Add("timeout with a fraction of a millisecond", options with { Limits = new IcebergLimits { Timeout = TimeSpan.FromTicks((TimeSpan.TicksPerMillisecond * 2000) + 1) } });
        Add("empty on error", options with { OnError = "" });
        return data;
    }

    [Theory]
    [MemberData(nameof(Refused))]
    public void WhatTheServerWouldRefuseIsRefusedHereWithOneFixedMessageThatShowsNoContent(string name, IcebergWriteOptions bad)
    {
        var error = Assert.Throws<SagatException>(() => Workflow.Create("lake").IcebergWrite("w", bad));
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Contains("invalid iceberg write", error.Message, StringComparison.Ordinal);
        Assert.DoesNotContain("credential-canary", error.ToString(), StringComparison.Ordinal);
        Assert.NotNull(name);
    }

    [Fact]
    public void ANullOptionsObjectAndNullPartsAreRefusedTheSameWay()
    {
        Assert.Throws<SagatException>(() => Workflow.Create("lake").IcebergWrite("w", null!));
        Assert.Throws<SagatException>(() => Workflow.Create("lake").IcebergWrite("w", Base() with { Records = null! }));
        Assert.Throws<SagatException>(() => Workflow.Create("lake").IcebergWrite("w", Base() with { Table = null! }));
        Assert.Throws<SagatException>(() => Workflow.Create("lake").IcebergWrite("w", Base() with { Table = new IcebergTable(null!, "orders") }));
        Assert.Throws<SagatException>(() => Workflow.Create("lake").IcebergWrite("w", Base() with { Mapping = [null!] }));
    }

    [Fact]
    public void TheEdgesThatAreAcceptedAreAccepted()
    {
        var edges = new[]
        {
            Base() with { Table = new IcebergTable(["a", "b", "c", "d", "e", "f", "g", "h"], new string('n', 255)) },
            Base() with { Ref = new string('r', 255) },
            Base() with { Records = IcebergRecords.Inline(new string('x', 1024)) },
            Base() with { Records = IcebergRecords.Inline(new string('é', 512)) },
            Base() with { Mapping = [new IcebergMapping("/" + string.Concat(Enumerable.Repeat("a/", 15)) + "a", new string('c', 255))] },
            Base() with { Mapping = [new IcebergMapping("/a~0b/c~1d", "c")] },
            Base() with { Limits = new IcebergLimits { MaxRecordBytes = 2147483648L, MaxRecords = 1, Timeout = TimeSpan.FromMilliseconds(1000) } },
            Base() with { ConnectionRef = new ConnectionRef("abc") },
        };
        foreach (var options in edges)
        {
            NodeOf(options);
        }

        var empty = NodeOf(Base() with { Mapping = [], Limits = new IcebergLimits() });
        Assert.Empty(empty["mapping"]!.AsArray());
        Assert.Empty(empty["limits"]!.AsObject());
    }

    [Fact]
    public void ANodeChainsLikeAnyOtherNextIsLinkedByTheFlow()
    {
        var nodes = Workflow.Create("lake").Step("extract", "orders.extract").IcebergWrite("load", Base()).Step("notify", "notify.done").End("done").Build()["nodes"]!.AsArray();
        Assert.Equal("load", (string?)nodes[0]!["next"]);
        Assert.Equal("notify", (string?)nodes[1]!["next"]);
    }
}
