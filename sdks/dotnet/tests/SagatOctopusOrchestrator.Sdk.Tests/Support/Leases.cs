using System.Text.Json;
using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Protocol.V1;

namespace SagatOctopusOrchestrator.Sdk.Tests.Support;

internal static class Leases
{
    internal const string Traceparent = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

    /// <summary>A lease for the activity <c>charge</c> as the worker service sends it, with the step document the engine builds.</summary>
    internal static TaskLease Lease(Action<TaskLease>? change = null)
    {
        var lease = new TaskLease
        {
            TaskId = "run-1/charge/1",
            RunId = "run-1",
            StepId = "charge",
            Attempt = 1,
            Generation = 1,
            LeaseToken = "lt-secret-lease-token",
            ExpiresAtEpochMs = DateTimeOffset.UtcNow.AddSeconds(60).ToUnixTimeMilliseconds(),
            ActivityName = "charge",
            InputJson = Document(new JsonObject { ["amount"] = 5 }, "charge", new JsonObject { ["earlier"] = new JsonObject { ["output"] = new JsonObject { ["ok"] = true } } }),
            Traceparent = Traceparent,
            IdempotencyKey = "run-1:charge",
        };
        change?.Invoke(lease);
        return lease;
    }

    /// <summary>The step document: <c>{run: {id}, node, input, steps: {&lt;node&gt;: {output}}}</c>.</summary>
    internal static string Document(JsonNode? input, string node, JsonObject? steps = null) =>
        new JsonObject
        {
            ["run"] = new JsonObject { ["id"] = "run-1" },
            ["node"] = node,
            ["input"] = input,
            ["steps"] = steps ?? new JsonObject(),
        }.ToJsonString(new JsonSerializerOptions());
}
