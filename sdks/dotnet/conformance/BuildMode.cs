using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Sdk.Workflows;

namespace SagatOctopusOrchestrator.Sdk.Conformance;

/// <summary>`run.sh build`: every workflow of the scenario, written by the builders of the SDK (and only by them).</summary>
internal static class BuildMode
{
    internal static JsonObject Build(JsonObject scenario)
    {
        var workflows = new JsonObject();
        var canonical = new JsonObject();
        foreach (var (key, definition) in scenario["workflows"]?.AsObject() ?? [])
        {
            var builder = BuilderOf(definition!.AsObject());
            workflows[key] = builder.Build();

            // The canonical text (RFC 8785, SPEC section 7): the harness compares it byte for byte across the SDKs and with the golden file.
            canonical[key] = builder.ToJson();
        }

        var errors = new JsonObject();
        foreach (var (key, invalid) in scenario["invalidWorkflows"]?.AsObject() ?? [])
        {
            var contains = (string?)invalid!["contains"] ?? string.Empty;
            try
            {
                // The canonical text is part of what the builders write (an integer the other SDKs cannot hold is refused when it is written).
                var builder = BuilderOf(invalid["workflow"]!.AsObject());
                builder.Build();
                builder.ToJson();
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

        return errors.Count > 0
            ? new JsonObject { ["workflows"] = workflows, ["canonical"] = canonical, ["errors"] = errors }
            : new JsonObject { ["workflows"] = workflows, ["canonical"] = canonical };
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

        if (definition["limits"] is JsonObject limits)
        {
            builder.Limits(new WorkflowLimitsOptions
            {
                MaxInstances = (int?)limits["maxInstances"],
                MaxDepth = (int?)limits["maxDepth"],
            });
        }

        if (definition["state"] is JsonObject state)
        {
            builder.State(StateOf(state));
        }

        if (definition["chainPolicy"] is JsonObject chainPolicy)
        {
            builder.ChainPolicy(new ChainPolicyOptions
            {
                MaxSegments = (int)chainPolicy["maxSegments"]!,
                Timeout = Ms(chainPolicy["timeoutMs"]),
            });
        }

        foreach (var (name, bodyDefinition) in definition["bodies"]?.AsObject() ?? [])
        {
            var nested = Workflow.Body();
            Apply(nested, bodyDefinition!["program"]!.AsArray());
            builder.Body(name, nested);
        }

        Apply(builder, definition["program"]!.AsArray());
        return builder;
    }

    private static StateDefinition StateOf(JsonObject state)
    {
        var queries = new Dictionary<string, QueryDefinition>(StringComparer.Ordinal);
        foreach (var (name, value) in state["queries"]?.AsObject() ?? [])
        {
            var query = value!.AsObject();
            queries[name] = new QueryDefinition
            {
                ArgsSchema = query["argsSchema"] as JsonObject,
                ResultSchema = query["resultSchema"] as JsonObject,
                Expression = (string?)query["expression"],
                Disclosure = (string?)query["disclosure"],
            };
        }

        var updates = new Dictionary<string, UpdateDefinition>(StringComparer.Ordinal);
        foreach (var (name, value) in state["updates"]?.AsObject() ?? [])
        {
            var update = value!.AsObject();
            updates[name] = new UpdateDefinition
            {
                ArgsSchema = update["argsSchema"] as JsonObject,
                Validate = (string?)update["validate"],
                State = (string?)update["state"],
                Result = (string?)update["result"],
                ResultSchema = update["resultSchema"] as JsonObject,
                Disclosure = (string?)update["disclosure"],
            };
        }

        return new StateDefinition
        {
            Schema = state["schema"] as JsonObject,
            Initial = (string?)state["initial"],
            Queries = queries,
            Updates = updates,
        };
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
                case "database":
                    target.Database(id, DatabaseOptionsOf(options!));
                    break;
                case "script":
                    target.Script(id, (string)op["artifactDigest"]!, new ScriptOptions
                    {
                        Language = (string?)options?["language"],
                        Retry = RetryOf(options?["retry"] as JsonObject),
                        CompensateWith = (string?)options?["compensateWith"],
                        Secrets = options?["secrets"] is JsonArray secrets ? [.. secrets.Select(secret => (string)secret!)] : null,
                        OnError = (string?)options?["onError"],
                        OutputSchema = options?["outputSchema"]?.DeepClone().AsObject(),
                    });
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
                        OutputSchema = options["outputSchema"]?.DeepClone().AsObject(),
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
                        OutputSchema = options?["outputSchema"]?.DeepClone().AsObject(),
                    });
                    break;
                case "condition":
                    target.Condition(id, (string)op["expression"]!, (string)op["ifTrue"]!, (string)op["ifFalse"]!);
                    break;
                case "switch":
                    target.Switch(
                        id,
                        op["cases"]!.AsArray().Select(item => new SwitchCase(
                            (string)item!["id"]!,
                            (string)item["when"]!,
                            (string)item["target"]!)),
                        (string)op["default"]!);
                    break;
                case "forEach":
                    target.ForEach(id, (string)op["items"]!, (string)op["body"]!, (string)op["result"]!, new CollectionOptions
                    {
                        MaxItems = (int)op["maxItems"]!,
                        MaxConcurrency = (int)op["maxConcurrency"]!,
                    });
                    break;
                case "map":
                    target.Map(id, (string)op["items"]!, (string)op["body"]!, (string)op["result"]!, new CollectionOptions
                    {
                        MaxItems = (int)op["maxItems"]!,
                        MaxConcurrency = (int)op["maxConcurrency"]!,
                    });
                    break;
                case "while":
                    target.While(id, (string)op["condition"]!, (string)op["body"]!, (string)op["initial"]!, (string)op["carry"]!, LoopOptionsOf(op));
                    break;
                case "repeatUntil":
                    target.RepeatUntil(id, (string)op["condition"]!, (string)op["body"]!, (string)op["initial"]!, (string)op["carry"]!, LoopOptionsOf(op));
                    break;
                case "approval":
                    var policy = op["policy"]!.AsObject();
                    target.Approval(id, new ApprovalOptions
                    {
                        Policy = new ApprovalPolicy
                        {
                            Subjects = policy["subjects"] is JsonArray subjects ? [.. subjects.Select(subject => (string)subject!)] : null,
                            Roles = policy["roles"] is JsonArray roles ? [.. roles.Select(role => (string)role! switch
                            {
                                "admin" => ApprovalRole.Admin,
                                "developer" => ApprovalRole.Developer,
                                "operator" => ApprovalRole.Operator,
                                _ => (ApprovalRole)(-1),
                            })] : null,
                            ExcludeInitiator = (bool?)policy["excludeInitiator"],
                        },
                        Timeout = Ms(op["timeoutMs"]),
                        OnApprove = (string)op["onApprove"]!,
                        OnReject = (string)op["onReject"]!,
                        OnExpire = (string)op["onExpire"]!,
                    });
                    break;
                case "continueAsNew":
                    target.ContinueAsNew(id, (string)op["input"]!, (string?)op["state"]);
                    break;
                case "return" when target is Body bodyTarget:
                    bodyTarget.Return(id, (string)op["result"]!);
                    break;
                case "return":
                    throw new InvalidOperationException("return is only valid in a named body");
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

    private static DatabaseOptions DatabaseOptionsOf(JsonObject options)
    {
        try { return ParseDatabaseOptions(options); }
        catch (Exception error) when (error is InvalidOperationException or FormatException or OverflowException)
        {
            throw new SagatException(SagatErrorCode.ValidationFailed, "invalid managed database operation");
        }
    }

    private static DatabaseOptions ParseDatabaseOptions(JsonObject options) => new()
    {
        ConnectionRef = new ConnectionRef((string)options["connectionRef"]!["name"]!, (int?)options["connectionRef"]!["revision"]),
        Operation = Enum.TryParse<DatabaseOperation>((string?)options["operation"], true, out var operation) ? operation : (DatabaseOperation)(-1),
        Statement = (string)options["statement"]!,
        Parameters = options["parameters"]?.AsArray().Select(p => p!["literal"] is { } literal
            ? DatabaseParameter.Literal(DatabaseTypeOf(literal["type"]), literal["value"])
            : DatabaseParameter.Expression(DatabaseTypeOf(p["type"]), (string)p["expression"]!)).ToArray(),
        Limits = options["limits"]?.AsObject().ToDictionary(p => p.Key, p => (int)p.Value!, StringComparer.Ordinal),
        Timeout = options["timeoutMs"] is { } timeout ? Ms(timeout) : null,
        OnError = (string?)options["onError"],
    };

    private static DatabaseType DatabaseTypeOf(JsonNode? node) => Enum.TryParse<DatabaseType>((string?)node, true, out var type) ? type : (DatabaseType)(-1);

    private static TimeSpan Ms(JsonNode? value) => TimeSpan.FromMilliseconds((long)value!);

    private static LoopOptions LoopOptionsOf(JsonObject op) => new()
    {
        MaxIterations = (int)op["maxIterations"]!,
        Timeout = Ms(op["timeoutMs"]),
        Delay = op["delayMs"] is { } delay ? Ms(delay) : null,
        OnError = (string?)op["onError"],
    };

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
            OutputSchema = options["outputSchema"]?.DeepClone().AsObject(),
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
