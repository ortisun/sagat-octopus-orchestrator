using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Sdk.Workflows;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

public class WorkflowBuilderTests
{
    private static string Canon(JsonNode? node) => CanonicalJson.Write(node);

    private static JsonNode? Nodes(JsonObject document) => document["nodes"];

    [Fact]
    public void AChainOfStepsIsLinkedInOrderAndStartsAtTheFirst()
    {
        var document = Workflow.Create("order-flow")
            .Description("charges and ships")
            .Step("charge", "payments.charge", new StepOptions
            {
                Retry = new StepRetry
                {
                    MaxAttempts = 3,
                    Strategy = RetryStrategy.Exponential,
                    InitialDelay = TimeSpan.FromMilliseconds(100),
                    MaxDelay = TimeSpan.FromMilliseconds(1000),
                    Jitter = true,
                },
            })
            .Sleep("pause", TimeSpan.FromMilliseconds(1500))
            .Step("ship", "shipping.ship")
            .Build();
        Assert.Equal("charge", (string?)document["entrypoint"]);
        Assert.Equal("""{"description":"charges and ships","name":"order-flow"}""", Canon(document["metadata"]));
        Assert.Equal(
            """[{"activity":"payments.charge","id":"charge","next":"pause","retry":{"initialDelayMs":100,"jitter":true,"maxAttempts":3,"maxDelayMs":1000,"strategy":"exponential"},"type":"activity"},{"durationMs":1500,"id":"pause","next":"ship","type":"timer"},{"activity":"shipping.ship","id":"ship","type":"activity"}]""",
            Canon(Nodes(document)));
    }

    [Fact]
    public void AbsentOptionsAreLeftOutAndNeverNull()
    {
        var document = Workflow.Create("plain-flow").Step("a", "act.a", new StepOptions { CompensateWith = null }).Build();
        Assert.Equal("""[{"activity":"act.a","id":"a","type":"activity"}]""", Canon(Nodes(document)));
        Assert.DoesNotContain("null", document.ToJsonString(), StringComparison.Ordinal);
    }

    [Fact]
    public void CompensationReconciliationAndTheOtherStepOptionsAreWrittenAsTheSchemaNamesThem()
    {
        var document = Workflow.Create("saga-flow")
            .Step("reserve", "limit.reserve", new StepOptions
            {
                CompensateWith = "limit.release",
                ReconcileWith = "limit.lookup",
                IdempotencyTemplate = "reserve:${run.id}",
                Secrets = ["payments-key"],
                CompensationRetry = new StepRetry { MaxAttempts = 5 },
                CompensationTimeout = TimeSpan.FromSeconds(10),
                HighRisk = true,
                Timeout = TimeSpan.FromMinutes(10),
                OnError = "reserve",
            })
            .Build();
        Assert.Equal(
            """{"activity":"limit.reserve","compensateWith":"limit.release","compensationRetry":{"maxAttempts":5},"compensationTimeoutMs":10000,"highRisk":true,"id":"reserve","idempotencyTemplate":"reserve:${run.id}","onError":"reserve","reconcileWith":"limit.lookup","secrets":["payments-key"],"timeoutMs":600000,"type":"activity"}""",
            Canon(Nodes(document)![0]));
    }

    [Fact]
    public void AParallelListsTheFirstNodeOfEachBranchAndContinuesWithTheNextNodeOfItsFlow()
    {
        var document = Workflow.Create("fan-out")
            .Parallel(
                "split",
                JoinMode.All,
                Workflow.Flow().Step("card", "pay.card").Step("card-2", "pay.card2"),
                Workflow.Flow().Step("stock", "stock.hold"))
            .Step("done", "order.done")
            .Build();
        Assert.Equal(
            """[{"branches":["card","stock"],"id":"split","join":"all","next":"done","type":"parallel"},{"activity":"order.done","id":"done","type":"activity"},{"activity":"pay.card","id":"card","next":"card-2","type":"activity"},{"activity":"pay.card2","id":"card-2","type":"activity"},{"activity":"stock.hold","id":"stock","type":"activity"}]""",
            Canon(Nodes(document)));
    }

    [Fact]
    public void ASagaBeginsAtItsBodyHoldsNoEndAndContinuesWithTheNextNodeOfItsFlow()
    {
        var document = Workflow.Create("saga-flow")
            .Saga(
                "tx",
                new SagaOptions { OnCompensationFailure = CompensationFailurePolicy.Fail, CancelPolicy = SagaCancelPolicy.AbortNoCompensation },
                Workflow.Flow().Step("reserve", "a.reserve", new StepOptions { CompensateWith = "a.release" }).Step("submit", "a.submit"))
            .Step("notify", "a.notify")
            .Build();
        var nodes = Nodes(document)!.AsArray();
        Assert.Equal(
            """{"begin":"reserve","cancelPolicy":"abort_no_compensation","id":"tx","next":"notify","onCompensationFailure":"fail","type":"saga"}""",
            Canon(nodes[0]));
        Assert.Equal(["tx", "notify", "reserve", "submit"], nodes.Select(n => (string?)n!["id"]));
        Assert.Null(nodes[3]!["next"]);
    }

    [Fact]
    public void WaitForAnEventInvokeAChildDecideAndEnd()
    {
        var document = Workflow.Create("mixed-flow")
            .WaitForEvent("paid", new WaitForEventOptions
            {
                EventType = "payment.confirmed",
                CorrelationExpr = "input.orderId",
                Timeout = TimeSpan.FromMinutes(1),
                OnTimeout = "cancel",
                Lookback = TimeSpan.FromSeconds(1),
            })
            .InvokeChild("ship", "shipping-flow", new ChildOptions { Version = 2, CancelPolicy = ChildCancelPolicy.Detach, Input = "input.order" })
            .Condition("big", "input.amount > 100", "review", "cancel")
            .Step("review", "order.review", new StepOptions { OnError = "cancel" })
            .Step("cancel", "order.cancel")
            .End("done", "steps.ship.output")
            .Build();
        var nodes = Nodes(document)!.AsArray();
        Assert.Null(nodes[2]!["next"]);
        Assert.Equal(
            """{"correlationExpr":"input.orderId","eventType":"payment.confirmed","id":"paid","lookbackMs":1000,"next":"ship","onTimeout":"cancel","timeoutMs":60000,"type":"waitEvent"}""",
            Canon(nodes[0]));
        Assert.Equal(
            """{"cancelPolicy":"detach","id":"ship","input":"input.order","next":"big","type":"child","version":2,"workflowId":"shipping-flow"}""",
            Canon(nodes[1]));
        Assert.Equal("""{"id":"done","outputExpr":"steps.ship.output","type":"end"}""", Canon(nodes[5]));
        Assert.Equal("done", (string?)nodes[4]!["next"]);
    }

    [Fact]
    public void ASwitchKeepsCaseOrderCopiesItsCasesAndHasNoImplicitNext()
    {
        var cases = new List<SwitchCase>
        {
            new("nonnegative", "input.amount >= 0", "positive"),
            new("one", "input.amount == 1", "also-positive"),
        };
        var builder = Workflow.Create("switch-flow")
            .Switch("route", cases, "fallback")
            .Step("positive", "amount.positive")
            .Step("also-positive", "amount.also-positive")
            .Step("fallback", "amount.fallback");
        cases[0] = new SwitchCase("tampered", "false", "tampered");

        var node = builder.Build()["nodes"]!.AsArray()[0]!;
        Assert.Equal(
            """{"cases":[{"id":"nonnegative","target":"positive","when":"input.amount >= 0"},{"id":"one","target":"also-positive","when":"input.amount == 1"}],"default":"fallback","id":"route","type":"switch"}""",
            Canon(node));
        Assert.Null(node["next"]);
        Workflow.Create("local-case-ids")
            .Switch("first", [new SwitchCase("same", "true", "done")], "done")
            .Switch("second", [new SwitchCase("same", "false", "done")], "done")
            .End("done")
            .Build();
    }

    [Fact]
    public void CollectionsUseNamedBodiesAndMapWritesTheSameForEachNode()
    {
        var line = Workflow.Body().Step("price", "line.price").Return("return", "steps.price.output");
        var tax = Workflow.Body().Step("price", "tax.price").Return("return", "steps.price.output");
        var document = Workflow.Create("collection-flow")
            .Body("line", line)
            .Body("tax", tax)
            .ForEach("notify-lines", "input.lines", "line", "bodyResult", new CollectionOptions { MaxItems = 17, MaxConcurrency = 3 })
            .Map("prices", "input.lines", "tax", "bodyResult.amount", new CollectionOptions { MaxItems = 17, MaxConcurrency = 2 })
            .End("done", "steps.prices.output")
            .Build();

        Assert.Equal(
            """[{"body":"line","id":"notify-lines","items":"input.lines","maxConcurrency":3,"maxItems":17,"next":"prices","result":"bodyResult","type":"forEach"},{"body":"tax","id":"prices","items":"input.lines","maxConcurrency":2,"maxItems":17,"next":"done","result":"bodyResult.amount","type":"forEach"},{"id":"done","outputExpr":"steps.prices.output","type":"end"}]""",
            Canon(document["nodes"]));
        Assert.Equal(
            """{"line":{"entrypoint":"price","nodes":[{"activity":"line.price","id":"price","next":"return","type":"activity"},{"id":"return","result":"steps.price.output","type":"return"}]},"tax":{"entrypoint":"price","nodes":[{"activity":"tax.price","id":"price","next":"return","type":"activity"},{"id":"return","result":"steps.price.output","type":"return"}]}}""",
            Canon(document["bodies"]));
    }

    [Fact]
    public void NamedBodiesCannotReferenceThemselves()
    {
        var recursive = Workflow.Body()
            .ForEach("nested", "input.items", "line", "bodyResult", new CollectionOptions { MaxItems = 2, MaxConcurrency = 1 })
            .Return("return", "steps.nested.output");
        var error = Assert.Throws<SagatException>(() => Workflow.Create("recursive-body")
            .Body("line", recursive)
            .ForEach("lines", "input.items", "line", "bodyResult", new CollectionOptions { MaxItems = 2, MaxConcurrency = 1 })
            .End("done")
            .Build());
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Contains("body reference cycle", error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void WhileAndRepeatUntilWriteBoundedPreTestAndPostTestLoopNodes()
    {
        var iteration = Workflow.Body().Step("increment", "counter.increment").Return("return", "steps.increment.output");
        var document = Workflow.Create("loop-flow")
            .Body("iteration", iteration)
            .While("pre", "carry < 3", "iteration", "input.start", "bodyResult", new LoopOptions
            {
                MaxIterations = 3,
                Timeout = TimeSpan.FromMinutes(1),
                Delay = TimeSpan.FromMilliseconds(250),
                OnError = "failed",
            })
            .RepeatUntil("post", "carry >= 5", "iteration", "steps.pre.output", "bodyResult", new LoopOptions
            {
                MaxIterations = 5,
                Timeout = TimeSpan.FromMinutes(2),
            })
            .End("done")
            .End("failed")
            .Build();

        Assert.Equal(
            """[{"body":"iteration","carry":"bodyResult","condition":"carry < 3","delayMs":250,"id":"pre","initial":"input.start","maxIterations":3,"next":"post","onError":"failed","timeoutMs":60000,"type":"while"},{"body":"iteration","carry":"bodyResult","condition":"carry >= 5","id":"post","initial":"steps.pre.output","maxIterations":5,"next":"done","timeoutMs":120000,"type":"repeatUntil"},{"id":"done","type":"end"},{"id":"failed","type":"end"}]""",
            Canon(document["nodes"]));
    }

    [Fact]
    public void ALookbackOfZeroIsWrittenAndAnAbsentOneIsNot()
    {
        var zero = Workflow.Create("zero-look").WaitForEvent("w", new WaitForEventOptions { EventType = "e", CorrelationExpr = "x", Timeout = TimeSpan.FromSeconds(1), Lookback = TimeSpan.Zero }).Build();
        Assert.Equal(0L, (long?)Nodes(zero)![0]!["lookbackMs"]);
        var absent = Workflow.Create("no-look").WaitForEvent("w", new WaitForEventOptions { EventType = "e", CorrelationExpr = "x", Timeout = TimeSpan.FromSeconds(1) }).Build();
        Assert.False(Nodes(absent)![0]!.AsObject().ContainsKey("lookbackMs"));
    }

    [Fact]
    public void TriggersLabelsAndSchemasGoWhereTheDocumentKeepsThem()
    {
        var document = Workflow.Create("with-meta")
            .Label("tier", "gold")
            .Label("team", "payments")
            .Trigger(Trigger.OnEvent("order.created", "api"))
            .Trigger(Trigger.OnSchedule("0 * * * *", "Europe/Lisbon"))
            .InputSchema(new JsonObject { ["type"] = "object" })
            .OutputSchema(new JsonObject { ["type"] = "object" })
            .Step("a", "act.a")
            .Build();
        Assert.Equal("""{"labels":{"team":"payments","tier":"gold"},"name":"with-meta"}""", Canon(document["metadata"]));
        Assert.Equal("""[{"eventType":"order.created","kind":"event","source":"api"},{"cron":"0 * * * *","kind":"cron","timezone":"Europe/Lisbon"}]""", Canon(document["triggers"]));
        Assert.Equal("""{"type":"object"}""", Canon(document["inputSchema"]));
        Assert.Equal("""{"type":"object"}""", Canon(document["outputSchema"]));
    }

    [Fact]
    public void BuildingTwiceGivesTheSameDocumentAndSideEffectsOfOneDoNotReachTheOther()
    {
        var builder = Workflow.Create("twice-flow").Step("a", "act.a").Step("b", "act.b");
        var first = builder.Build();
        first["nodes"]!.AsArray()[0]!["activity"] = "tampered";
        Assert.Equal(builder.ToJson(), Workflow.Create("twice-flow").Step("a", "act.a").Step("b", "act.b").ToJson());
        Assert.Contains("act.a", builder.ToJson(), StringComparison.Ordinal);
    }

    public static TheoryData<string, Action> Refusals => new()
    {
        { "workflow name", () => Workflow.Create("Bad Name") },
        { "workflow name", () => Workflow.Create("ab") },
        { "at least one node", () => Workflow.Create("empty-flow").Build() },
        { "used twice", () => Workflow.Create("dup-ids").Step("a", "x").Step("a", "y").Build() },
        { "used twice", () => Workflow.Create("dup-nested").Step("a", "x").Parallel("p", JoinMode.All, Workflow.Flow().Step("a", "y"), Workflow.Flow().Step("b", "z")).Build() },
        { "used twice", () => Workflow.Create("dup-siblings").Parallel("p", JoinMode.All, Workflow.Flow().Step("a", "y"), Workflow.Flow().Step("a", "z")) },
        { "two branches", () => Workflow.Create("one-branch").Parallel("p", JoinMode.All, Workflow.Flow().Step("a", "x")) },
        { "empty branch", () => Workflow.Create("empty-branch").Parallel("p", JoinMode.All, Workflow.Flow().Step("a", "x"), Workflow.Flow()) },
        { "empty body", () => Workflow.Create("empty-saga").Saga("s", null, Workflow.Flow()) },
        { "end node", () => Workflow.Create("saga-end").Saga("s", null, Workflow.Flow().Step("a", "x").End("e")) },
        { "irreversible", () => Workflow.Create("both").Step("a", "x", new StepOptions { Irreversible = true, CompensateWith = "undo" }) },
        { "secret", () => Workflow.Create("bad-secret").Step("a", "x", new StepOptions { Secrets = ["Bad Secret"] }) },
        { "duration", () => Workflow.Create("bad-timer").Sleep("t", TimeSpan.Zero) },
        { "timeout", () => Workflow.Create("bad-timeout").WaitForEvent("w", new WaitForEventOptions { EventType = "e", CorrelationExpr = "x", Timeout = TimeSpan.Zero }) },
        { "not a node", () => Workflow.Create("dangling").Condition("c", "true", "nowhere", "a").Step("a", "x").Build() },
        { "between 1 and 32 cases", () => Workflow.Create("no-switch-cases").Switch("s", [], "a") },
        { "between 1 and 32 cases", () => Workflow.Create("too-many-cases").Switch("s", Enumerable.Range(0, 33).Select(i => new SwitchCase($"case-{i}", "true", "a")), "a") },
        { "case id", () => Workflow.Create("duplicate-cases").Switch("s", [new SwitchCase("same", "true", "a"), new SwitchCase("same", "false", "b")], "a") },
        { "without an id", () => Workflow.Create("empty-case-id").Switch("s", [new SwitchCase(string.Empty, "true", "a")], "a") },
        { "when expression", () => Workflow.Create("empty-case-when").Switch("s", [new SwitchCase("one", string.Empty, "a")], "a") },
        { "needs a target", () => Workflow.Create("empty-case-target").Switch("s", [new SwitchCase("one", "true", string.Empty)], "a") },
        { "default target", () => Workflow.Create("empty-switch-default").Switch("s", [new SwitchCase("one", "true", "a")], string.Empty) },
        { "not a node", () => Workflow.Create("dangling-switch-case").Switch("s", [new SwitchCase("one", "true", "nowhere")], "a").Step("a", "x").Build() },
        { "not a node", () => Workflow.Create("dangling-switch-default").Switch("s", [new SwitchCase("one", "true", "a")], "nowhere").Step("a", "x").Build() },
        { "not a node", () => Workflow.Create("dangling-timeout").WaitForEvent("w", new WaitForEventOptions { EventType = "e", CorrelationExpr = "x", Timeout = TimeSpan.FromMilliseconds(1), OnTimeout = "nowhere" }).Build() },
        { "activity name", () => Workflow.Create("no-activity").Step("a", string.Empty) },
        { "needs an id", () => Workflow.Create("no-id").Step(string.Empty, "x") },
    };

    [Theory]
    [MemberData(nameof(Refusals))]
    public void WhatTheServerWouldRefuseAndTheBuilderCanSeeIsRefusedHere(string mentions, Action build)
    {
        var error = Assert.Throws<SagatException>(build);
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Contains(mentions, error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void TheCanonicalTextSortsKeysAtEveryDepthAndLeavesOutWhitespace()
    {
        var node = JsonNode.Parse("""{"b":1,"a":{"d":[2,{"z":1,"y":2}],"c":null},"e":[null,{"k":null}]}""");
        Assert.Equal("""{"a":{"c":null,"d":[2,{"y":2,"z":1}]},"b":1,"e":[null,{"k":null}]}""", Canon(node));
        var text = Workflow.Create("canon-flow").Step("a", "act.a", new StepOptions { Retry = new StepRetry { MaxAttempts = 2 } }).ToJson();
        Assert.StartsWith("""{"apiVersion":""", text, StringComparison.Ordinal);
        Assert.DoesNotContain(" ", text, StringComparison.Ordinal);
    }

    // The expected text is what `JSON.stringify(JSON.parse(input))` prints in Node (ECMAScript Number::toString, the rule of RFC 8785).
    [Theory]
    [InlineData("1.0", "1")]
    [InlineData("1e3", "1000")]
    [InlineData("100", "100")]
    [InlineData("0.1", "0.1")]
    [InlineData("1e21", "1e+21")]
    [InlineData("1e-7", "1e-7")]
    [InlineData("0.000001", "0.000001")]
    [InlineData("5e-324", "5e-324")]
    [InlineData("1.7976931348623157e308", "1.7976931348623157e+308")]
    [InlineData("-1.5", "-1.5")]
    [InlineData("4.35", "4.35")]
    [InlineData("1.5e300", "1.5e+300")]
    [InlineData("0.30000000000000004", "0.30000000000000004")]
    [InlineData("1e22", "1e+22")]
    [InlineData("12345.6789", "12345.6789")]
    [InlineData("-0", "0")]
    [InlineData("0", "0")]
    [InlineData("2.5e-7", "2.5e-7")]
    [InlineData("0.00001234", "0.00001234")]
    [InlineData("1.2e-6", "0.0000012")]
    [InlineData("1e100", "1e+100")]
    [InlineData("3.141592653589793", "3.141592653589793")]
    [InlineData("-1e-7", "-1e-7")]
    [InlineData("600000", "600000")]
    [InlineData("600000.0", "600000")]
    [InlineData("0.5", "0.5")]
    [InlineData("1E3", "1000")]
    [InlineData("1.5E+2", "150")]
    [InlineData("123456789.123456789", "123456789.12345679")]
    [InlineData("4.9e-324", "5e-324")]
    [InlineData("0.000001234", "0.000001234")]
    [InlineData("1e-6", "0.000001")]
    [InlineData("0.0000001", "1e-7")]
    public void CanonicalNumbersAreWrittenAsEcmaScriptWritesThem(string input, string expected)
    {
        Assert.Equal(expected, Canon(JsonNode.Parse(input)));
    }

    [Fact]
    public void ANumberOutsideTheRangeOfADoubleHasNoCanonicalSpelling()
    {
        var error = Assert.Throws<SagatException>(() => Canon(JsonNode.Parse("1e999")));
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
    }

    [Fact]
    public void KeysAreSortedByUtf16CodeUnitsAndNotByCulture()
    {
        // RFC 8785 sorts by UTF-16 code units: U+1F600 is a surrogate pair whose first unit (0xD83D) is below U+FF5E, though by code point it would come after.
        var node = JsonNode.Parse("{\"b\":1,\"B\":2,\"a\":3,\"\uff5e\":4,\"\ud83d\ude00\":5,\"_\":6,\"10\":7,\"9\":8}");
        Assert.Equal("{\"10\":7,\"9\":8,\"B\":2,\"_\":6,\"a\":3,\"b\":1,\"\ud83d\ude00\":5,\"\uff5e\":4}", Canon(node));
    }

    [Fact]
    public void TheCanonicalTextEscapesAsJavaScriptDoesAndKeepsNumbersAsTheyWereRead()
    {
        var node = JsonNode.Parse("""{"text":"a\u00e7\u00e3o \u20ac \ud83d\ude00 <>&'","ctl":"\u0001\b\f\n\r\t\"\\","n":[600000,0.5,9007199254740991,1e3]}""");
        Assert.Equal(
            "{\"ctl\":\"\\u0001\\b\\f\\n\\r\\t\\\"\\\\\",\"n\":[600000,0.5,9007199254740991,1000],\"text\":\"a\u00e7\u00e3o \u20ac \ud83d\ude00 <>&'\"}",
            Canon(node));
        Assert.Throws<SagatException>(() => Canon(JsonValue.Create("\ud800")));
    }

    // ---------------------------------------------------------------------------- a flow is read when Build() is called (SPEC section 7)

    [Fact]
    public void AStepAddedToABranchAfterItWasJoinedIsPartOfTheDocument()
    {
        var left = Workflow.Flow().Step("card", "pay.card");
        var right = Workflow.Flow().Step("stock", "stock.hold");
        var builder = Workflow.Create("late-flow").Parallel("split", JoinMode.All, left, right).Step("done", "order.done");

        left.Step("card-2", "pay.card2");
        right.Sleep("wait", TimeSpan.FromMilliseconds(50));
        var document = builder.Build();

        Assert.Equal(
            """[{"branches":["card","stock"],"id":"split","join":"all","next":"done","type":"parallel"},{"activity":"order.done","id":"done","type":"activity"},{"activity":"pay.card","id":"card","next":"card-2","type":"activity"},{"activity":"pay.card2","id":"card-2","type":"activity"},{"activity":"stock.hold","id":"stock","next":"wait","type":"activity"},{"durationMs":50,"id":"wait","type":"timer"}]""",
            Canon(document["nodes"]));
    }

    [Fact]
    public void AStepAddedToASagaBodyAfterItWasJoinedIsPartOfTheDocumentAndALateEndOrAClashIsRefusedByBuild()
    {
        var body = Workflow.Flow().Step("reserve", "a.reserve", new StepOptions { CompensateWith = "a.release" });
        var builder = Workflow.Create("late-saga").Saga("tx", null, body).Step("notify", "a.notify");
        body.Step("submit", "a.submit");
        var nodes = builder.Build()["nodes"]!.AsArray();
        Assert.Equal(["tx", "notify", "reserve", "submit"], nodes.Select(n => (string?)n!["id"]));
        Assert.Equal("submit", (string?)nodes[2]!["next"]);
        Assert.Null(nodes[3]!["next"]);
        Assert.Equal("reserve", (string?)nodes[0]!["begin"]);

        body.End("late-end");
        var end = Assert.Throws<SagatException>(() => builder.Build());
        Assert.Equal(SagatErrorCode.ValidationFailed, end.Code);
        Assert.Contains("end node", end.Message, StringComparison.Ordinal);

        var clash = Workflow.Flow().Step("a", "x");
        var other = Workflow.Create("late-clash").Parallel("p", JoinMode.Any, clash, Workflow.Flow().Step("b", "y"));
        clash.Step("p", "z");
        var error = Assert.Throws<SagatException>(() => other.Build());
        Assert.Contains("used twice", error.Message, StringComparison.Ordinal);
    }
}
