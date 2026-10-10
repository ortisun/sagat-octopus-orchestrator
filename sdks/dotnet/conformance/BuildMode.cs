using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Sdk.Workflows;

namespace SagatOctopusOrchestrator.Sdk.Conformance;

/// <summary>`run.sh build`: every workflow of the scenario, written by the builders of the SDK (and only by them).</summary>
internal static class BuildMode
{
    internal static JsonObject Build(JsonObject scenario)
    {
        var workflows = new JsonObject();
        foreach (var (key, definition) in scenario["workflows"]?.AsObject() ?? [])
        {
            workflows[key] = BuilderOf(definition!.AsObject()).Build();
        }

        var errors = new JsonObject();
        foreach (var (key, invalid) in scenario["invalidWorkflows"]?.AsObject() ?? [])
        {
            var contains = (string?)invalid!["contains"] ?? string.Empty;
            try
            {
                BuilderOf(invalid["workflow"]!.AsObject()).Build();
                errors[key] = new JsonObject { ["raised"] = false };
            }
            catch (SagatException error)
            {
                errors[key] = new JsonObject
                {
                    ["raised"] = true,
                    ["code"] = error.Code.ToWireName(),
                    ["mentions"] = contains.Length == 0 || error.Message.Contains(contains, StringComparison.Ordinal),
                };
            }
        }

        return errors.Count > 0 ? new JsonObject { ["workflows"] = workflows, ["errors"] = errors } : new JsonObject { ["workflows"] = workflows };
    }

    private static WorkflowBuilder BuilderOf(JsonObject definition)
    {
        var builder = Workflow.Create((string)definition["name"]!);
        if ((string?)definition["description"] is { Length: > 0 } description)
        {
            builder.Description(description);
        }

        foreach (var (key, value) in definition["labels"]?.AsObject() ?? [])
        {
            builder.Label(key, (string)value!);
        }

        foreach (var trigger in definition["triggers"]?.AsArray() ?? [])
        {
            builder.Trigger(TriggerOf(trigger!.AsObject()));
        }

        if (definition["inputSchema"] is JsonObject input)
        {
            builder.InputSchema(input);
        }

        if (definition["outputSchema"] is JsonObject output)
        {
            builder.OutputSchema(output);
        }

        Apply(builder, definition["program"]!.AsArray());
        return builder;
    }

    private static void Apply<TFlow>(FlowBase<TFlow> target, JsonArray program)
        where TFlow : FlowBase<TFlow>
    {
        foreach (var node in program)
        {
            var op = node!.AsObject();
            var id = (string)op["id"]!;
            var options = op["options"] as JsonObject;
            switch ((string)op["op"]!)
            {
                case "step":
                    target.Step(id, (string)op["activity"]!, StepOptionsOf(options));
                    break;
                case "sleep":
                    target.Sleep(id, Ms(op["durationMs"]));
                    break;
                case "waitForEvent":
                    target.WaitForEvent(id, new WaitForEventOptions
                    {
                        EventType = (string)options!["eventType"]!,
                        CorrelationExpr = (string)options["correlationExpr"]!,
                        Timeout = Ms(options["timeoutMs"]),
                        OnTimeout = (string?)options["onTimeout"],
                        Lookback = options["lookbackMs"] is { } lookback ? Ms(lookback) : null,
                    });
                    break;
                case "invokeChild":
                    target.InvokeChild(id, (string)op["workflow"]!, new ChildOptions
                    {
                        Version = (int?)options?["version"],
                        Input = (string?)options?["input"],
                        CancelPolicy = (string?)options?["cancelPolicy"] switch
                        {
                            null => null,
                            "propagate" => ChildCancelPolicy.Propagate,
                            "detach" => ChildCancelPolicy.Detach,
                            "request_cancel" => ChildCancelPolicy.RequestCancel,
                            var other => throw new InvalidOperationException($"unknown child cancel policy {other}"),
                        },
                    });
                    break;
                case "condition":
                    target.Condition(id, (string)op["expression"]!, (string)op["ifTrue"]!, (string)op["ifFalse"]!);
                    break;
                case "parallel":
                    target.Parallel(
                        id,
                        (string)op["join"]! == "all" ? JoinMode.All : JoinMode.Any,
                        op["branches"]!.AsArray().Select(branch =>
                        {
                            var flow = Workflow.Flow();
                            Apply(flow, branch!.AsArray());
                            return flow;
                        }));
                    break;
                case "saga":
                    var body = Workflow.Flow();
                    Apply(body, op["body"]!.AsArray());
                    target.Saga(id, new SagaOptions
                    {
                        OnCompensationFailure = (string?)options?["onCompensationFailure"] switch
                        {
                            null => null,
                            "manual_review" => CompensationFailurePolicy.ManualReview,
                            "fail" => CompensationFailurePolicy.Fail,
                            var other => throw new InvalidOperationException($"unknown compensation failure policy {other}"),
                        },
                        CancelPolicy = (string?)options?["cancelPolicy"] switch
                        {
                            null => null,
                            "compensate_completed" => SagaCancelPolicy.CompensateCompleted,
                            "abort_no_compensation" => SagaCancelPolicy.AbortNoCompensation,
                            "wait_in_flight_then_compensate" => SagaCancelPolicy.WaitInFlightThenCompensate,
                            var other => throw new InvalidOperationException($"unknown saga cancel policy {other}"),
                        },
                    }, body);
                    break;
                case "end":
                    target.End(id, (string?)options?["outputExpr"]);
                    break;
                default:
                    throw new InvalidOperationException($"unknown builder op {(string?)op["op"]}");
            }
        }
    }

    private static TimeSpan Ms(JsonNode? value) => TimeSpan.FromMilliseconds((long)value!);

    private static StepOptions StepOptionsOf(JsonObject? options) => options is null
        ? new StepOptions()
        : new StepOptions
        {
            Retry = RetryOf(options["retry"] as JsonObject),
            Timeout = options["timeoutMs"] is { } timeout ? Ms(timeout) : null,
            CompensateWith = (string?)options["compensateWith"],
            ReconcileWith = (string?)options["reconcileWith"],
            IdempotencyTemplate = (string?)options["idempotencyTemplate"],
            Secrets = options["secrets"] is JsonArray secrets ? [.. secrets.Select(secret => (string)secret!)] : null,
            CompensationRetry = RetryOf(options["compensationRetry"] as JsonObject),
            CompensationTimeout = options["compensationTimeoutMs"] is { } compensation ? Ms(compensation) : null,
            Irreversible = (bool?)options["irreversible"],
            HighRisk = (bool?)options["highRisk"],
            OnError = (string?)options["onError"],
        };

    private static StepRetry? RetryOf(JsonObject? retry) => retry is null
        ? null
        : new StepRetry
        {
            MaxAttempts = (int?)retry["maxAttempts"],
            InitialDelay = retry["initialDelayMs"] is { } initial ? Ms(initial) : null,
            MaxDelay = retry["maxDelayMs"] is { } max ? Ms(max) : null,
            Strategy = (string?)retry["strategy"] switch
            {
                null => null,
                "fixed" => RetryStrategy.Fixed,
                "exponential" => RetryStrategy.Exponential,
                var other => throw new InvalidOperationException($"unknown retry strategy {other}"),
            },
            Jitter = (bool?)retry["jitter"],
        };

    private static Trigger TriggerOf(JsonObject trigger) =>
        new(Enum.Parse<TriggerKind>((string)trigger["kind"]!, ignoreCase: true))
        {
            EventType = (string?)trigger["eventType"],
            Source = (string?)trigger["source"],
            Endpoint = (string?)trigger["endpoint"],
            Connector = (string?)trigger["connector"],
            Filter = (string?)trigger["filter"],
            Cron = (string?)trigger["cron"],
            Timezone = (string?)trigger["timezone"],
        };
}
