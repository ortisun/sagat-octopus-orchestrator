using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using SagatOctopusOrchestrator.Sdk.Internal;

namespace SagatOctopusOrchestrator.Sdk.Workflows;

/// <summary>
/// A chain of nodes: the body of a workflow, of a parallel branch or of a saga. Pure and deterministic: building a flow has no
/// side effect and the same program produces the same document in the five SDKs (SPEC section 7).
/// </summary>
/// <typeparam name="TSelf">The concrete flow, so that the calls chain.</typeparam>
public abstract partial class FlowBase<TSelf>
    where TSelf : FlowBase<TSelf>
{
    private readonly List<JsonObject> nodes = [];
    private readonly List<(Flow Flow, string? SagaId)> embedded = [];
    private readonly HashSet<string> ids = new(StringComparer.Ordinal);

    internal IReadOnlyList<JsonObject> Nodes => nodes;

    internal IEnumerable<string> AllIds => ids;

    [GeneratedRegex("^[a-z][a-z0-9._-]{0,63}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex SecretPattern();

    /// <summary>Step.Run: an activity executed by a worker.</summary>
    public TSelf Step(string id, string activity, StepOptions? options = null)
    {
        options ??= new StepOptions();
        if (string.IsNullOrEmpty(activity))
        {
            throw Invalid($"step {id} needs an activity name");
        }

        if (options.Irreversible == true && !string.IsNullOrEmpty(options.CompensateWith))
        {
            throw Invalid($"step {id} cannot be irreversible and have a compensation");
        }

        foreach (var secret in options.Secrets ?? [])
        {
            if (!SecretPattern().IsMatch(secret))
            {
                throw Invalid($"step {id} names a secret that is not a name: {Quote(secret)}");
            }
        }

        var node = new JsonObject { ["id"] = id, ["type"] = "activity", ["activity"] = activity };
        if (options.Retry is { } retry)
        {
            node["retry"] = RetryNode(retry);
        }

        Set(node, "timeoutMs", Millis(options.Timeout, $"step {id} timeout"));
        Set(node, "compensateWith", options.CompensateWith);
        Set(node, "reconcileWith", options.ReconcileWith);
        Set(node, "idempotencyTemplate", options.IdempotencyTemplate);
        if (options.Secrets is { } secrets)
        {
            node["secrets"] = new JsonArray([.. secrets.Select(secret => (JsonNode?)JsonValue.Create(secret))]);
        }

        if (options.CompensationRetry is { } compensationRetry)
        {
            node["compensationRetry"] = RetryNode(compensationRetry);
        }

        Set(node, "compensationTimeoutMs", Millis(options.CompensationTimeout, $"step {id} compensation timeout"));
        Set(node, "irreversible", options.Irreversible);
        Set(node, "highRisk", options.HighRisk);
        Set(node, "onError", options.OnError);
        return Push(node);
    }

    /// <summary>Sleep: waits for a duration. A timer, not a process: nothing is kept alive.</summary>
    public TSelf Sleep(string id, TimeSpan duration)
    {
        var ms = Millis(duration, $"sleep {id} duration");
        return Push(new JsonObject { ["id"] = id, ["type"] = "timer", ["durationMs"] = ms });
    }

    /// <summary>WaitForEvent: waits for an event of a type with a correlation key, or for the timeout.</summary>
    public TSelf WaitForEvent(string id, WaitForEventOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);
        if (string.IsNullOrEmpty(options.EventType) || string.IsNullOrEmpty(options.CorrelationExpr))
        {
            throw Invalid($"waitForEvent {id} needs an eventType and a correlationExpr");
        }

        var timeout = Millis(options.Timeout, $"waitForEvent {id} timeout");
        var node = new JsonObject
        {
            ["id"] = id,
            ["type"] = "waitEvent",
            ["eventType"] = options.EventType,
            ["correlationExpr"] = options.CorrelationExpr,
            ["timeoutMs"] = timeout,
        };
        Set(node, "onTimeout", options.OnTimeout);
        if (options.Lookback is { } lookback)
        {
            node["lookbackMs"] = WholeMillis(lookback, $"waitForEvent {id} lookback");
        }

        return Push(node);
    }

    /// <summary>InvokeChild: starts another workflow of the namespace (by name) and waits for it.</summary>
    public TSelf InvokeChild(string id, string workflow, ChildOptions? options = null)
    {
        if (string.IsNullOrEmpty(workflow))
        {
            throw Invalid($"invokeChild {id} needs the name of a workflow");
        }

        options ??= new ChildOptions();
        var node = new JsonObject { ["id"] = id, ["type"] = "child", ["workflowId"] = workflow };
        Set(node, "version", options.Version);
        Set(node, "input", options.Input);
        if (options.CancelPolicy is { } policy)
        {
            node["cancelPolicy"] = policy switch
            {
                ChildCancelPolicy.Propagate => "propagate",
                ChildCancelPolicy.Detach => "detach",
                _ => "request_cancel",
            };
        }

        return Push(node);
    }

    /// <summary>A decision over the run's data: goes to <paramref name="ifTrue"/> or to <paramref name="ifFalse"/>. It has no next node.</summary>
    public TSelf Condition(string id, string expression, string ifTrue, string ifFalse)
    {
        if (string.IsNullOrEmpty(expression))
        {
            throw Invalid($"condition {id} needs an expression");
        }

        return Push(new JsonObject { ["id"] = id, ["type"] = "condition", ["expression"] = expression, ["ifTrue"] = ifTrue, ["ifFalse"] = ifFalse });
    }

    /// <summary>Parallel: runs branches at once; <see cref="JoinMode.All"/> waits for every one, <see cref="JoinMode.Any"/> for the first.</summary>
    public TSelf Parallel(string id, JoinMode join, params Flow[] branches) => Parallel(id, join, (IEnumerable<Flow>)branches);

    /// <summary>Parallel: runs branches at once; <see cref="JoinMode.All"/> waits for every one, <see cref="JoinMode.Any"/> for the first.</summary>
    public TSelf Parallel(string id, JoinMode join, IEnumerable<Flow> branches)
    {
        ArgumentNullException.ThrowIfNull(branches);
        var list = branches.ToList();
        if (list.Count < 2)
        {
            throw Invalid($"parallel {id} needs at least two branches");
        }

        if (list.Any(branch => branch.Nodes.Count == 0))
        {
            throw Invalid($"parallel {id} has an empty branch");
        }

        var node = new JsonObject
        {
            ["id"] = id,
            ["type"] = "parallel",
            ["branches"] = new JsonArray([.. list.Select(branch => (JsonNode?)JsonValue.Create((string)branch.Nodes[0]["id"]!))]),
            ["join"] = join == JoinMode.All ? "all" : "any",
        };
        Push(node);
        foreach (var branch in list)
        {
            Embed(branch);
        }

        return (TSelf)this;
    }

    /// <summary>
    /// Saga: the body is compensated, in reverse order of completion, when it fails or the run is cancelled; a step in it names its
    /// undo with <c>CompensateWith</c>.
    /// </summary>
    public TSelf Saga(string id, SagaOptions? options, Flow body)
    {
        ArgumentNullException.ThrowIfNull(body);
        if (body.Nodes.Count == 0)
        {
            throw Invalid($"saga {id} has an empty body");
        }

        if (body.Nodes.Any(node => (string?)node["type"] == "end"))
        {
            throw Invalid($"saga {id} cannot hold an end node");
        }

        options ??= new SagaOptions();
        var node = new JsonObject { ["id"] = id, ["type"] = "saga", ["begin"] = (string)body.Nodes[0]["id"]! };
        if (options.OnCompensationFailure is { } failure)
        {
            node["onCompensationFailure"] = failure == CompensationFailurePolicy.ManualReview ? "manual_review" : "fail";
        }

        if (options.CancelPolicy is { } policy)
        {
            node["cancelPolicy"] = policy switch
            {
                SagaCancelPolicy.CompensateCompleted => "compensate_completed",
                SagaCancelPolicy.AbortNoCompensation => "abort_no_compensation",
                _ => "wait_in_flight_then_compensate",
            };
        }

        Push(node);
        Embed(body, id);
        return (TSelf)this;
    }

    /// <summary>An explicit end of the flow, optionally with the run's output expression.</summary>
    public TSelf End(string id, string? outputExpr = null)
    {
        var node = new JsonObject { ["id"] = id, ["type"] = "end" };
        Set(node, "outputExpr", outputExpr);
        return Push(node);
    }

    /// <summary>
    /// Every node of the flow and of the flows inside it, in order, linked: node i gets <c>next</c> = node i+1, except a condition
    /// (it names its two branches) and an end (it ends); the last node of a flow has none.
    /// </summary>
    internal List<JsonObject> Flatten()
    {
        var output = new List<JsonObject>();
        for (var i = 0; i < nodes.Count; i++)
        {
            var linked = (JsonObject)nodes[i].DeepClone();
            var type = (string?)linked["type"];
            if (i + 1 < nodes.Count && type is not ("condition" or "end") && !linked.ContainsKey("next"))
            {
                linked["next"] = (string)nodes[i + 1]["id"]!;
            }

            output.Add(linked);
        }

        foreach (var (inner, sagaId) in embedded)
        {
            // A flow is read now, not when it was joined: what was added to the body of a saga since may not be an end node.
            var flat = inner.Flatten();
            if (sagaId is not null && flat.Any(node => (string?)node["type"] == "end"))
            {
                throw Invalid($"saga {sagaId} cannot hold an end node");
            }

            output.AddRange(flat);
        }

        return output;
    }

    internal static SagatException Invalid(string message) => SagatException.Invalid(message);

    private TSelf Push(JsonObject node)
    {
        var id = (string?)node["id"];
        if (string.IsNullOrEmpty(id))
        {
            throw Invalid("a node needs an id");
        }

        Claim(id);
        nodes.Add(node);
        return (TSelf)this;
    }

    private void Embed(Flow flow, string? sagaId = null)
    {
        foreach (var id in flow.AllIds)
        {
            Claim(id);
        }

        embedded.Add((flow, sagaId));
    }

    private void Claim(string id)
    {
        if (!ids.Add(id))
        {
            throw Invalid($"the node id {Quote(id)} is used twice");
        }
    }

    private static string Quote(string text) => Wire.Quote(text);

    private static void Set(JsonObject node, string name, string? value)
    {
        if (value is not null)
        {
            node[name] = value;
        }
    }

    private static void Set(JsonObject node, string name, long? value)
    {
        if (value is { } number)
        {
            node[name] = number;
        }
    }

    private static void Set(JsonObject node, string name, bool? value)
    {
        if (value is { } flag)
        {
            node[name] = flag;
        }
    }

    /// <summary>A duration in milliseconds, which must be a whole number of them: the document has no fractions, and a fraction is never truncated.</summary>
    private static long WholeMillis(TimeSpan span, string what)
    {
        if (span.Ticks % TimeSpan.TicksPerMillisecond != 0)
        {
            throw Invalid($"{what} must be a whole number of milliseconds");
        }

        return span.Ticks / TimeSpan.TicksPerMillisecond;
    }

    private static long? Millis(TimeSpan? span, string subject)
    {
        if (span is null)
        {
            return null;
        }

        var ms = WholeMillis(span.Value, subject);
        return ms >= 1 ? ms : throw Invalid($"{subject} must be at least 1 ms");
    }

    private static long Millis(TimeSpan span, string subject) => Millis((TimeSpan?)span, subject)!.Value;

    private static JsonObject RetryNode(StepRetry retry)
    {
        var node = new JsonObject();
        if (retry.MaxAttempts is { } attempts)
        {
            node["maxAttempts"] = attempts;
        }

        if (retry.InitialDelay is { } initial)
        {
            node["initialDelayMs"] = WholeMillis(initial, "a retry initial delay");
        }

        if (retry.MaxDelay is { } max)
        {
            node["maxDelayMs"] = WholeMillis(max, "a retry maximum delay");
        }

        if (retry.Strategy is { } strategy)
        {
            node["strategy"] = strategy == RetryStrategy.Fixed ? "fixed" : "exponential";
        }

        if (retry.Jitter is { } jitter)
        {
            node["jitter"] = jitter;
        }

        return node;
    }
}

/// <summary>A flow for a parallel branch or a saga body: <c>Workflow.Flow()</c>.</summary>
public sealed class Flow : FlowBase<Flow>
{
    internal Flow()
    {
    }
}

/// <summary>Builds the workflow IR (<c>workflow.schema.json</c>) without any side effect (SPEC section 7).</summary>
public sealed partial class WorkflowBuilder : FlowBase<WorkflowBuilder>
{
    private readonly string name;
    private readonly SortedDictionary<string, string> labels = new(StringComparer.Ordinal);
    private readonly List<Trigger> triggers = [];
    private string? descriptionText;
    private JsonObject? input;
    private JsonObject? output;

    internal WorkflowBuilder(string name)
    {
        if (!NamePattern().IsMatch(name ?? string.Empty))
        {
            throw Invalid($"a workflow name is lower case letters, digits and dashes, 3 to 64: {Wire.Quote(name)}");
        }

        this.name = name!;
    }

    [GeneratedRegex("^[a-z][a-z0-9-]{2,63}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex NamePattern();

    /// <summary>The description of the workflow.</summary>
    public WorkflowBuilder Description(string text)
    {
        descriptionText = text;
        return this;
    }

    /// <summary>A label of the workflow.</summary>
    public WorkflowBuilder Label(string key, string value)
    {
        labels[key] = value;
        return this;
    }

    /// <summary>Adds a trigger.</summary>
    public WorkflowBuilder Trigger(Trigger trigger)
    {
        ArgumentNullException.ThrowIfNull(trigger);
        triggers.Add(trigger);
        return this;
    }

    /// <summary>The JSON schema of the run input.</summary>
    public WorkflowBuilder InputSchema(JsonObject schema)
    {
        input = (JsonObject)schema.DeepClone();
        return this;
    }

    /// <summary>The JSON schema of the run output.</summary>
    public WorkflowBuilder OutputSchema(JsonObject schema)
    {
        output = (JsonObject)schema.DeepClone();
        return this;
    }

    /// <summary>The document. Raises <see cref="SagatErrorCode.ValidationFailed"/> for what the server would refuse and the builder can see.</summary>
    public JsonObject Build()
    {
        if (Nodes.Count == 0)
        {
            throw Invalid("a workflow needs at least one node");
        }

        var all = Flatten();
        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (var node in all)
        {
            var id = (string)node["id"]!;
            if (!seen.Add(id))
            {
                throw Invalid($"the node id {Wire.Quote(id)} is used twice");
            }
        }

        void Reference(string from, string field, JsonNode? target)
        {
            if (target is JsonValue value && value.TryGetValue<string>(out var text) && !seen.Contains(text))
            {
                throw Invalid($"{from}.{field} points at {Wire.Quote(text)}, which is not a node");
            }
        }

        foreach (var node in all)
        {
            var id = (string)node["id"]!;
            foreach (var field in new[] { "next", "onError", "onTimeout", "ifTrue", "ifFalse", "begin" })
            {
                Reference(id, field, node[field]);
            }

            if (node["branches"] is JsonArray branches)
            {
                foreach (var branch in branches)
                {
                    Reference(id, "branches", branch);
                }
            }
        }

        var metadata = new JsonObject { ["name"] = name };
        if (descriptionText is not null)
        {
            metadata["description"] = descriptionText;
        }

        if (labels.Count > 0)
        {
            var labelNode = new JsonObject();
            foreach (var (key, value) in labels)
            {
                labelNode[key] = value;
            }

            metadata["labels"] = labelNode;
        }

        var document = new JsonObject
        {
            ["apiVersion"] = "sagat-octopus-orchestrator.io/v1",
            ["kind"] = "Workflow",
            ["metadata"] = metadata,
            ["entrypoint"] = (string)Nodes[0]["id"]!,
            ["nodes"] = new JsonArray([.. all.Select(node => (JsonNode?)node)]),
        };
        if (triggers.Count > 0)
        {
            document["triggers"] = new JsonArray([.. triggers.Select(t => (JsonNode?)TriggerNode(t))]);
        }

        if (input is not null)
        {
            document["inputSchema"] = input.DeepClone();
        }

        if (output is not null)
        {
            document["outputSchema"] = output.DeepClone();
        }

        return document;
    }

    /// <summary>The canonical text of <see cref="Build"/>: UTF-8, keys sorted, no whitespace. The same in the five SDKs.</summary>
    public string ToJson() => CanonicalJson.Write(Build());

    private static JsonObject TriggerNode(Trigger trigger)
    {
        var node = new JsonObject
        {
            ["kind"] = trigger.Kind switch
            {
                TriggerKind.Manual => "manual",
                TriggerKind.Event => "event",
                TriggerKind.Webhook => "webhook",
                TriggerKind.Message => "message",
                _ => "cron",
            },
        };
        void Put(string key, string? value)
        {
            if (value is not null)
            {
                node[key] = value;
            }
        }

        Put("eventType", trigger.EventType);
        Put("source", trigger.Source);
        Put("endpoint", trigger.Endpoint);
        Put("connector", trigger.Connector);
        Put("filter", trigger.Filter);
        Put("cron", trigger.Cron);
        Put("timezone", trigger.Timezone);
        return node;
    }
}

/// <summary>Entry points of the workflow builders.</summary>
public static class Workflow
{
    /// <summary>Starts a workflow definition. The name is lower case letters, digits and dashes, 3 to 64, starting with a letter.</summary>
    public static WorkflowBuilder Create(string name) => new(name);

    /// <summary>A flow for a parallel branch or a saga body.</summary>
    public static Flow Flow() => new();
}
