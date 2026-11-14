using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Sdk.Conformance;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>
/// The conformance runner (<c>sdks/dotnet/conformance</c>) played against a fake server: what it passes to the typed SDK from a scenario step, and
/// what it prints of what the SDK read back. The real suite needs the platform; these prove the runner's own mapping.
/// </summary>
public sealed class ConformanceRunnerTests : IAsyncLifetime
{
    private const string Ns = "0190aaaa-0000-7000-8000-00000000aaaa";
    private const string Wf = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    private const string ScheduleId = "0190aaaa-0000-7000-8000-0000000000a1";

    private FakeRestServer server = null!;
    private JsonObject? keptAction;

    public async Task InitializeAsync() => server = await FakeRestServer.StartAsync();

    public async Task DisposeAsync() => await server.DisposeAsync();

    /// <summary>A server that keeps what it was written and answers it back the way the platform does: the business-id fields of the action only when the write had a prefix, the policies with their defaults.</summary>
    private FakeAnswer Platform(SeenRequest request)
    {
        if (request.Method is "POST" or "PUT")
        {
            var given = ((JsonObject)request.Body!)["action"] as JsonObject;
            var action = new JsonObject { ["workflowId"] = Wf, ["versionPolicy"] = "ACTIVE", ["inputDigest"] = new string('a', 64) };
            if (given?["businessIdPrefix"] is { } prefix)
            {
                action["businessIdPrefix"] = prefix.DeepClone();
                action["businessIdConflictPolicy"] = given["businessIdConflictPolicy"]?.DeepClone() ?? "FAIL";
                action["businessIdReusePolicy"] = given["businessIdReusePolicy"]?.DeepClone() ?? "ALLOW_DUPLICATE";
            }

            keptAction = action;
        }

        var schedule = new JsonObject
        {
            ["scheduleId"] = ScheduleId,
            ["name"] = "nightly",
            ["origin"] = "API",
            ["state"] = "ACTIVE",
            ["revision"] = request.Method == "PUT" ? 2 : 1,
            ["createdAt"] = "2026-10-10T10:00:00Z",
            ["action"] = keptAction!.DeepClone(),
        };
        return new FakeAnswer(request.Method == "POST" ? 201 : 200, schedule.ToJsonString());
    }

    private static JsonObject Write(JsonObject action) => new()
    {
        ["name"] = "nightly",
        ["spec"] = new JsonObject { ["cron"] = new JsonArray("0 9 * * *") },
        ["action"] = action,
    };

    private async Task<JsonObject> PlayAsync(params JsonObject[] steps)
    {
        var scenario = new JsonObject { ["name"] = "business-id-prefix", ["steps"] = new JsonArray([.. steps.Select(step => (JsonNode?)step)]) };
        var config = new JsonObject
        {
            ["apiUrl"] = server.BaseUrl,
            ["namespace"] = "payments",
            ["namespaceId"] = Ns,
            ["unique"] = "u1",
            ["clientToken"] = "tok-1",
        };
        await using var run = new RunMode(scenario, config);
        using var guard = new CancellationTokenSource(TimeSpan.FromMinutes(1));
        var output = await run.PlayAsync(guard.Token);
        foreach (var observation in output["observations"]!.AsArray())
        {
            Assert.True((bool)observation!["ok"]!, observation.ToJsonString());
        }

        return output;
    }

    private static JsonObject Observed(JsonObject output, string id) =>
        output["observations"]!.AsArray().Select(o => o!.AsObject()).Single(o => (string)o["id"]! == id);

    [Fact]
    public async Task AScenarioActionWithTheThreeFieldsReachesTheTypedWriteOfACreateAndOfAnUpdateAndTheRunnerPrintsWhatTheSdkReadBack()
    {
        server.Handler = Platform;
        var action = new JsonObject
        {
            ["workflowId"] = Wf,
            ["input"] = new JsonObject(),
            ["businessIdPrefix"] = "Nightly-u1-",
            ["businessIdConflictPolicy"] = "USE_EXISTING",
            ["businessIdReusePolicy"] = "REJECT_DUPLICATE",
        };
        var output = await PlayAsync(
            new JsonObject { ["id"] = "create", ["op"] = "scheduleCreate", ["idempotencyKey"] = "create-key-0001", ["write"] = Write(action) },
            new JsonObject { ["id"] = "update", ["op"] = "scheduleUpdate", ["schedule"] = "create", ["expectedRevision"] = 1L, ["idempotencyKey"] = "update-key-0001", ["write"] = Write(action.DeepClone().AsObject()) },
            new JsonObject { ["id"] = "get", ["op"] = "scheduleGet", ["schedule"] = "create" });

        // What the typed SDK sent: the three keys of the scenario, as given.
        foreach (var request in server.Requests.Where(r => r.Method is "POST" or "PUT"))
        {
            var sent = request.Body!["action"]!.AsObject();
            Assert.Equal("Nightly-u1-", (string?)sent["businessIdPrefix"]);
            Assert.Equal("USE_EXISTING", (string?)sent["businessIdConflictPolicy"]);
            Assert.Equal("REJECT_DUPLICATE", (string?)sent["businessIdReusePolicy"]);
        }

        Assert.Equal(["POST", "PUT", "GET"], server.Requests.Select(r => r.Method).ToArray());

        // What the runner printed of each schedule: the three keys, with the strings the server sent, inside result.action.
        foreach (var id in new[] { "create", "update", "get" })
        {
            var observation = Observed(output, id);
            Assert.True((bool)observation["ok"]!, observation.ToJsonString());
            var printed = observation["result"]!["action"]!.AsObject();
            Assert.Equal("Nightly-u1-", (string?)printed["businessIdPrefix"]);
            Assert.Equal("USE_EXISTING", (string?)printed["businessIdConflictPolicy"]);
            Assert.Equal("REJECT_DUPLICATE", (string?)printed["businessIdReusePolicy"]);
            Assert.Equal(Wf, (string?)printed["workflowId"]);
        }
    }

    [Fact]
    public async Task AScenarioActionWithOnlyAPrefixPrintsTheDefaultsTheServerAppliedAndAnActionWithNonePrintsNoneOfTheThreeKeys()
    {
        server.Handler = Platform;
        var output = await PlayAsync(
            new JsonObject
            {
                ["id"] = "prefix-only",
                ["op"] = "scheduleCreate",
                ["idempotencyKey"] = "create-key-0001",
                ["write"] = Write(new JsonObject { ["workflowId"] = Wf, ["input"] = new JsonObject(), ["businessIdPrefix"] = "Nightly-u1-" }),
            },
            new JsonObject
            {
                ["id"] = "plain",
                ["op"] = "scheduleCreate",
                ["idempotencyKey"] = "create-key-0002",
                ["write"] = Write(new JsonObject { ["workflowId"] = Wf, ["input"] = new JsonObject() }),
            },
            new JsonObject { ["id"] = "plain-update", ["op"] = "scheduleUpdate", ["schedule"] = "plain", ["expectedRevision"] = 1L, ["idempotencyKey"] = "update-key-0001", ["write"] = Write(new JsonObject { ["workflowId"] = Wf, ["input"] = new JsonObject() }) },
            new JsonObject { ["id"] = "plain-get", ["op"] = "scheduleGet", ["schedule"] = "plain" });

        var only = Observed(output, "prefix-only")["result"]!["action"]!.AsObject();
        Assert.Equal("Nightly-u1-", (string?)only["businessIdPrefix"]);
        Assert.Equal("FAIL", (string?)only["businessIdConflictPolicy"]);
        Assert.Equal("ALLOW_DUPLICATE", (string?)only["businessIdReusePolicy"]);
        var sent = server.Requests[0].Body!["action"]!.AsObject();
        Assert.Equal(["workflowId", "input", "businessIdPrefix"], sent.Select(pair => pair.Key).ToArray());

        foreach (var id in new[] { "plain", "plain-update", "plain-get" })
        {
            var printed = Observed(output, id)["result"]!["action"]!.AsObject();
            Assert.False(printed.ContainsKey("businessIdPrefix"), printed.ToJsonString());
            Assert.False(printed.ContainsKey("businessIdConflictPolicy"), printed.ToJsonString());
            Assert.False(printed.ContainsKey("businessIdReusePolicy"), printed.ToJsonString());

            // Every other key is what it was before the three existed, so a golden written before them stays byte-identical.
            Assert.Equal(
                ["workflowId", "versionPolicy", "versionId", "inputDigest", "nominalTimeField", "runTimeoutMs"],
                printed.Select(pair => pair.Key).ToArray());
        }

        foreach (var request in server.Requests.Skip(1))
        {
            if (request.Method is "POST" or "PUT")
            {
                var plain = request.Body!["action"]!.AsObject();
                Assert.Equal(["workflowId", "input"], plain.Select(pair => pair.Key).ToArray());
            }
        }
    }
}
