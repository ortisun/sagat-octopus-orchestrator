using System.Net.Http.Json;
using System.Text.Json;
using System.Text.Json.Nodes;
using SagatOctopusOrchestrator.Sdk.Workers;

namespace SagatOctopusOrchestrator.Sdk.Conformance;

/// <summary>`run.sh run`: plays a scenario against the platform with the public API of the SDK and writes what it observed.</summary>
internal sealed class RunMode : IAsyncDisposable
{
    private readonly JsonObject scenario;
    private readonly JsonObject config;
    private readonly string unique;
    private readonly FaultPlan plan;
    private readonly Dictionary<string, SagatClient> clients = new(StringComparer.Ordinal);
    private readonly Dictionary<string, HostEntry> hosts = new(StringComparer.Ordinal);
    private readonly Dictionary<string, JsonObject> results = new(StringComparer.Ordinal);
    private readonly JsonArray observations = [];
    private readonly HttpClient control = new();

    internal RunMode(JsonObject scenario, JsonObject config)
    {
        this.config = config;
        unique = (string)config["unique"]!;
        this.scenario = (JsonObject)Templated(scenario, unique)!;
        plan = new FaultPlan((this.scenario["faults"]?.AsArray() ?? []).Select(fault => Fault.Of(fault!.AsObject())));
    }

    private sealed class HostEntry(JsonObject definition, List<JsonObject> log)
    {
        internal JsonObject Definition { get; } = definition;

        internal List<JsonObject> Log { get; } = log;

        internal WorkerHost? Host { get; set; }

        internal Behaviors? Behaviors { get; set; }

        internal WorkerMeasurements Measurements { get; } = new();
    }

    /// <summary>Replaces <c>{{unique}}</c> in every string value of the scenario.</summary>
    private static JsonNode? Templated(JsonNode? node, string unique) => node switch
    {
        null => null,
        JsonObject obj => new JsonObject(obj.Select(pair => KeyValuePair.Create(pair.Key, Templated(pair.Value, unique)))),
        JsonArray array => new JsonArray([.. array.Select(item => Templated(item, unique))]),
        JsonValue value when value.GetValueKind() == JsonValueKind.String => JsonValue.Create(value.GetValue<string>().Replace("{{unique}}", unique, StringComparison.Ordinal)),
        _ => node.DeepClone(),
    };

    public async ValueTask DisposeAsync()
    {
        foreach (var entry in hosts.Values)
        {
            if (entry.Host is { } host)
            {
                await host.DisposeAsync().ConfigureAwait(false);
            }
        }

        foreach (var client in clients.Values)
        {
            client.Dispose();
        }

        control.Dispose();
    }

    private SagatClient ClientFor(string token)
    {
        if (clients.TryGetValue(token, out var existing))
        {
            return existing;
        }

        var apiPath = new Uri((string)config["apiUrl"]!).AbsolutePath.TrimEnd('/');
        var options = new SagatOptions
        {
            ApiUrl = (string)config["apiUrl"]!,
            Namespace = (string)config["namespace"]!,
            Token = (string)config[token switch { "bad" => "badToken", "other" => "otherTenantToken", _ => "clientToken" }]!,
            HttpMessageHandler = new FaultHandler(plan, apiPath, new SocketsHttpHandler { AllowAutoRedirect = false }),
        };
        options.Retry.InitialDelay = TimeSpan.FromMilliseconds(20);
        options.Retry.MaxDelay = TimeSpan.FromMilliseconds(200);
        return clients[token] = new SagatClient(options);
    }

    private WorkerHost HostOf(HostEntry entry)
    {
        var definition = entry.Definition;
        var options = new SagatOptions
        {
            GrpcUrl = (string)config["grpcUrl"]!,
            Namespace = (string)config["namespace"]!,
            Token = (string)config[(string?)definition["token"] == "unbound" ? "unboundWorkerToken" : "workerToken"]!,
        };
        options.Retry.InitialDelay = TimeSpan.FromMilliseconds(20);
        options.Retry.MaxDelay = TimeSpan.FromMilliseconds(200);
        options.Worker.Name = (string)definition["workerName"]!;
        options.Worker.MaxInflight = (int?)definition["maxInflight"] ?? 4;
        options.Worker.ProtocolVersion = (string?)definition["protocolVersion"];
        options.Worker.Transport = ((string?)definition["transport"]) switch
        {
            "poll" => WorkerTransport.Poll,
            "stream" => WorkerTransport.Stream,
            _ => WorkerTransport.Auto,
        };
        options.Worker.HeartbeatInterval = definition["heartbeatIntervalMs"] is { } beat ? TimeSpan.FromMilliseconds((int)beat) : null;
        if (plan.HasGrpc)
        {
            options.Worker.Interceptors.Add(new FaultInterceptor(plan));
        }

        var host = new WorkerHost(options);
        var behaviors = entry.Behaviors!;
        foreach (var (name, activityNode) in definition["activities"]!.AsObject())
        {
            var activity = activityNode!.AsObject();
            var kind = (string?)activity["as"] ?? "activity";
            var behavior = activity["behavior"]?.AsObject() ?? [];
            var activityName = name;
            switch (kind)
            {
                case "compensation":
                    host.RegisterCompensation<JsonNode?, object?>(name, (input, context, token) => behaviors.ActivityAsync(activityName, kind, behavior, input, context, token));
                    break;
                case "reconciler":
                    host.RegisterReconciler<JsonNode?>(name, (input, context, token) => behaviors.ReconcilerAsync(activityName, behavior, input, context, token));
                    break;
                default:
                    host.RegisterActivity<JsonNode?, object?>(name, (input, context, token) => behaviors.ActivityAsync(activityName, kind, behavior, input, context, token));
                    break;
            }
        }

        return host;
    }

    private async Task<JsonObject> StartWorkerAsync(string key)
    {
        if (!hosts.TryGetValue(key, out var entry))
        {
            throw new InvalidOperationException($"no worker {key}");
        }

        var host = HostOf(entry);
        try
        {
            await host.StartAsync().ConfigureAwait(false);
        }
        catch
        {
            await host.DisposeAsync().ConfigureAwait(false);
            throw;
        }

        entry.Host = host;
        return new JsonObject { ["protocolVersion"] = host.ProtocolVersion };
    }

    private string RunOf(JsonObject step)
    {
        var key = (string)step["run"]!;
        return results.TryGetValue(key, out var result) && result["runId"] is JsonValue id && id.TryGetValue<string>(out var text)
            ? text
            : throw new InvalidOperationException($"step {key} has no run");
    }

    private string ApprovalOf(JsonObject step)
    {
        if (step["approvalId"] is JsonValue direct && direct.TryGetValue<string>(out var approvalId))
        {
            return approvalId;
        }

        var key = (string)step["approval"]!;
        return results.TryGetValue(key, out var result)
            && result["items"] is JsonArray { Count: > 0 } items
            && items[0] is JsonObject item
            && item["approvalId"] is JsonValue id
            && id.TryGetValue<string>(out var text)
            ? text
            : throw new InvalidOperationException($"step {key} has no approval");
    }

    private string ChainRef(JsonNode? value)
    {
        var text = (string)value!;
        if (text == "{{namespaceId}}")
        {
            return (string)config["namespaceId"]!;
        }
        string? field = null;
        string? key = null;
        if (text.StartsWith("{{chainId:", StringComparison.Ordinal) && text.EndsWith("}}", StringComparison.Ordinal))
        {
            field = "runId";
            key = text[10..^2];
        }
        else if (text.StartsWith("{{currentRunId:", StringComparison.Ordinal) && text.EndsWith("}}", StringComparison.Ordinal))
        {
            field = "currentRunId";
            key = text[15..^2];
        }
        if (field is null)
        {
            return text;
        }
        return results.TryGetValue(key!, out var result) && result[field] is JsonValue id && id.TryGetValue<string>(out var resolved)
            ? resolved
            : throw new InvalidOperationException($"step {key} has no {field}");
    }

    internal async Task<JsonObject> PlayAsync(CancellationToken cancellationToken)
    {
        foreach (var (key, definition) in scenario["workers"]?.AsObject() ?? [])
        {
            var log = new List<JsonObject>();
            hosts[key] = new HostEntry(definition!.AsObject(), log);
            hosts[key].Behaviors = new Behaviors(key, log, hosts[key].Measurements);
        }

        foreach (var (key, entry) in hosts)
        {
            if ((bool?)entry.Definition["autostart"] != false)
            {
                await StartWorkerAsync(key).ConfigureAwait(false);
            }
        }

        foreach (var stepNode in scenario["steps"]!.AsArray())
        {
            var step = stepNode!.AsObject();
            var observation = new JsonObject { ["id"] = (string)step["id"]!, ["op"] = (string)step["op"]! };
            if (Environment.GetEnvironmentVariable("SAGAT_RUNNER_TRACE") is { Length: > 0 })
            {
                await Console.Error.WriteLineAsync($"step {(string)step["id"]!}").ConfigureAwait(false);
            }

            try
            {
                var result = await StepAsync(step, cancellationToken).ConfigureAwait(false);
                observation["ok"] = true;
                if (result is not null)
                {
                    observation["result"] = result;
                    results[(string)step["id"]!] = (JsonObject)result.DeepClone();
                }
            }
            catch (Exception error) when (error is not OperationCanceledException || !cancellationToken.IsCancellationRequested)
            {
                observation["ok"] = false;
                observation["error"] = ErrorJson(error);
            }

            observations.Add(observation);
        }

        foreach (var entry in hosts.Values)
        {
            if (entry.Host is { } host)
            {
                try
                {
                    await host.StopAsync(TimeSpan.FromSeconds(2), cancellationToken).ConfigureAwait(false);
                }
                catch (Exception error) when (error is SagatException or OperationCanceledException)
                {
                    // The run is over; a host that does not stop cleanly is not an observation.
                }
            }
        }

        var workerLog = new JsonObject();
        var workerMeasurements = new JsonObject();
        foreach (var (key, entry) in hosts)
        {
            JsonObject[] copy;
            lock (entry.Log)
            {
                copy = [.. entry.Log];
            }

            workerLog[key] = new JsonArray([.. copy.Select(item => (JsonNode?)item)]);
            workerMeasurements[key] = entry.Measurements.Snapshot();
        }

        return new JsonObject
        {
            ["scenario"] = (string)scenario["name"]!,
            ["language"] = "dotnet",
            ["sdkVersion"] = SdkInfo.Version,
            ["observations"] = observations,
            ["workerLog"] = workerLog,
            ["workerMeasurements"] = workerMeasurements,
            ["faultsApplied"] = new JsonArray([.. plan.Applied().Select(count => (JsonNode?)JsonValue.Create(count))]),
        };
    }

    private async Task<JsonObject?> StepAsync(JsonObject step, CancellationToken cancellationToken)
    {
        var client = ClientFor((string?)step["token"] ?? "default");
        switch ((string)step["op"]!)
        {
            case "start":
                {
                    var workflowId = (string)config["workflows"]![(string)step["workflow"]!]!["workflowId"]!;
                    return RunJson(await client.StartAsync(workflowId, step["input"]?.DeepClone() ?? new JsonObject(), new StartOptions
                    {
                        IdempotencyKey = (string?)step["idempotencyKey"],
                        CorrelationId = (string?)step["correlationId"],
                        Traceparent = (string?)step["traceparent"],
                        DeadlineIn = step["deadlineInMs"] is { } deadline ? TimeSpan.FromMilliseconds((long)deadline) : null,
                    }, cancellationToken).ConfigureAwait(false));
                }

            case "startWithBlob":
                {
                    var workflowId = (string)config["workflows"]![(string)step["workflow"]!]!["workflowId"]!;
                    var body = Enumerable.Repeat((byte)'i', (int)step["inputBlobBytes"]!).ToArray();
                    await using var source = new MemoryStream(body, writable: false);
                    var descriptor = await client.UploadRunInputAsync(source, new RunInputBlobOptions(
                        ExpectedSize: body.Length,
                        ExpectedSha256: Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(body)).ToLowerInvariant()), cancellationToken).ConfigureAwait(false);
                    return RunJson(await client.StartWithBlobAsync(workflowId, descriptor,
                        new StartOptions
                        {
                            IdempotencyKey = (string?)step["idempotencyKey"],
                            CorrelationId = (string?)step["correlationId"],
                            Traceparent = (string?)step["traceparent"],
                            DeadlineIn = step["deadlineInMs"] is { } deadline ? TimeSpan.FromMilliseconds((long)deadline) : null,
                        }, cancellationToken).ConfigureAwait(false));
                }

            case "workflowByName":
                {
                    var name = (string)config["workflows"]![(string)step["workflow"]!]!["name"]!;
                    return RunJson(await client.StartAsync(name, step["input"]?.DeepClone() ?? new JsonObject(), new StartOptions { IdempotencyKey = (string?)step["idempotencyKey"] }, cancellationToken).ConfigureAwait(false));
                }

            case "get":
                return RunJson(await client.GetAsync(RunOf(step), cancellationToken).ConfigureAwait(false));
            case "list":
                {
                    var ids = new JsonArray();
                    var options = new ListOptions { Status = (string?)step["status"], Limit = (int?)step["limit"] };
                    if ((bool?)step["all"] == true)
                    {
                        await foreach (var item in client.ListAllAsync(options, cancellationToken).ConfigureAwait(false))
                        {
                            ids.Add(item.RunId);
                        }
                    }
                    else
                    {
                        foreach (var item in (await client.ListAsync(options, cancellationToken).ConfigureAwait(false)).Items)
                        {
                            ids.Add(item.RunId);
                        }
                    }

                    // In the order the server gave (newest first): the SDK keeps it, also across pages.
                    return new JsonObject { ["items"] = ids };
                }

            case "listApprovals":
                {
                    var page = await client.ListApprovalsAsync(
                        RunOf(step),
                        new ApprovalListOptions { Cursor = (string?)step["cursor"], Limit = (int?)step["limit"], Traceparent = (string?)step["traceparent"] },
                        cancellationToken).ConfigureAwait(false);
                    for (var attempt = 0; (bool?)step["waitForItem"] == true && page.Items.Count == 0 && attempt < 100; attempt++)
                    {
                        await Task.Delay(50, cancellationToken).ConfigureAwait(false);
                        page = await client.ListApprovalsAsync(
                            RunOf(step),
                            new ApprovalListOptions { Cursor = (string?)step["cursor"], Limit = (int?)step["limit"], Traceparent = (string?)step["traceparent"] },
                            cancellationToken).ConfigureAwait(false);
                    }
                    return new JsonObject
                    {
                        ["items"] = new JsonArray([.. page.Items.Select(item => (JsonNode?)new JsonObject
                        {
                            ["approvalId"] = item.ApprovalId,
                            ["runId"] = item.RunId,
                            ["nodeId"] = item.NodeId,
                            ["instancePath"] = new JsonArray([.. item.InstancePath.Select(scope => (JsonNode?)new JsonObject
                            {
                                ["scopeNodeId"] = scope.ScopeNodeId,
                                ["iterationIndex"] = scope.IterationIndex,
                            })]),
                            ["status"] = item.Status.ToString().ToUpperInvariant(),
                            ["deadline"] = item.Deadline.UtcDateTime.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'", System.Globalization.CultureInfo.InvariantCulture),
                            ["settledAt"] = item.SettledAt?.UtcDateTime.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'", System.Globalization.CultureInfo.InvariantCulture),
                        })]),
                        ["nextCursor"] = page.NextCursor,
                    };
                }

            case "cancel":
                {
                    var cancelled = await client.CancelAsync(RunOf(step), (string)step["reason"]!, new OperateOptions { ExpectedRunVersion = (long?)step["expectedRunVersion"] }, cancellationToken).ConfigureAwait(false);
                    return new JsonObject { ["accepted"] = cancelled.Accepted, ["runStatus"] = cancelled.RunStatus };
                }

            case "operate":
                {
                    var operated = await client.OperateAsync(
                        RunOf(step),
                        ActionOf((string)step["action"]!),
                        (string)step["reason"]!,
                        new OperateOptions
                        {
                            StepId = (string?)step["stepId"],
                            Outcome = (string?)step["outcome"] switch { null => null, "SUCCEEDED" => StepOutcome.Succeeded, _ => StepOutcome.Failed },
                            ErrorCode = (string?)step["errorCode"],
                        },
                        cancellationToken).ConfigureAwait(false);
                    return new JsonObject { ["accepted"] = operated.Accepted, ["runStatus"] = operated.RunStatus };
                }

            case "decideApproval":
                {
                    var receipt = await client.DecideApprovalAsync(
                        ApprovalOf(step),
                        (string)step["decision"]! == "APPROVE" ? ApprovalDecision.Approve : ApprovalDecision.Reject,
                        new ApprovalDecisionOptions
                        {
                            DecisionKey = (string)step["decisionKey"]!,
                            ReasonCode = (string?)step["reasonCode"],
                            Traceparent = (string?)step["traceparent"],
                        },
                        cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["receiptId"] = receipt.ReceiptId,
                        ["approvalId"] = receipt.ApprovalId,
                        ["decisionKey"] = receipt.DecisionKey,
                        ["decision"] = receipt.Decision == ApprovalDecision.Approve ? "APPROVE" : "REJECT",
                        ["reasonCode"] = receipt.ReasonCode,
                        ["actor"] = receipt.Actor,
                        ["decidedAt"] = receipt.DecidedAt.UtcDateTime.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'", System.Globalization.CultureInfo.InvariantCulture),
                        ["outcomeSequence"] = receipt.OutcomeSequence,
                        ["replayed"] = receipt.Replayed,
                    };
                }

            case "query":
                {
                    var queried = await client.QueryAsync(
                        RunOf(step), (string)step["name"]!, step["args"]?.DeepClone(),
                        new RequestOptions { Traceparent = (string?)step["traceparent"] }, cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["runVersion"] = queried.RunVersion,
                        ["stateRevision"] = queried.StateRevision,
                        ["result"] = JsonNode.Parse(queried.Result.GetRawText()),
                    };
                }

            case "queryChain":
                {
                    var queried = await client.QueryChainAsync(
                        ChainRef(step["namespaceId"]), ChainRef(step["chainId"]), (string)step["name"]!, step["args"]?.DeepClone(),
                        new ChainQueryOptions
                        {
                            ExpectedRunId = step["expectedRunId"] is null ? null : ChainRef(step["expectedRunId"]),
                            Traceparent = (string?)step["traceparent"],
                        }, cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["runId"] = queried.RunId,
                        ["runVersion"] = queried.RunVersion,
                        ["stateRevision"] = queried.StateRevision,
                        ["result"] = JsonNode.Parse(queried.Result.GetRawText()),
                    };
                }

            case "update":
                {
                    var updated = await client.UpdateAsync(
                        RunOf(step), (string)step["name"]!, step["args"]?.DeepClone(),
                        new UpdateOptions
                        {
                            UpdateKey = (string)step["updateKey"]!,
                            ExpectedStateRevision = (long?)step["expectedStateRevision"],
                            Traceparent = (string?)step["traceparent"],
                        }, cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["operation"] = updated.Operation, ["updateKey"] = updated.UpdateKey, ["status"] = updated.Status,
                        ["result"] = JsonNode.Parse(updated.Result.GetRawText()),
                        ["beforeStateRevision"] = updated.BeforeStateRevision, ["afterStateRevision"] = updated.AfterStateRevision,
                        ["runVersion"] = updated.RunVersion, ["replayed"] = updated.Replayed,
                    };
                }

            case "inspectChain":
                {
                    var namespaceId = ChainRef(step["namespaceId"]);
                    var chainId = ChainRef(step["chainId"]);
                    var options = new RequestOptions { Traceparent = (string?)step["traceparent"] };
                    var chain = await client.InspectChainAsync(namespaceId, chainId, options, cancellationToken).ConfigureAwait(false);
                    if (step["waitForHeadChange"] is not null)
                    {
                        var previous = ChainRef(step["waitForHeadChange"]);
                        for (var attempt = 0; chain.CurrentRunId == previous && attempt < 100; attempt++)
                        {
                            await Task.Delay(50, cancellationToken).ConfigureAwait(false);
                            chain = await client.InspectChainAsync(namespaceId, chainId, options, cancellationToken).ConfigureAwait(false);
                        }
                    }
                    return new JsonObject
                    {
                        ["chainId"] = chain.ChainId, ["currentRunId"] = chain.CurrentRunId, ["revision"] = chain.Revision,
                        ["cancelRequested"] = chain.CancelRequested, ["finalOutcome"] = chain.FinalOutcome,
                        ["segments"] = new JsonArray([.. chain.Segments.Select(item => (JsonNode?)new JsonObject
                        {
                            ["index"] = item.Index, ["runId"] = item.RunId, ["predecessorRunId"] = item.PredecessorRunId,
                            ["successorRunId"] = item.SuccessorRunId,
                        })]),
                    };
                }

            case "signalChain":
                {
                    var signal = await client.SignalChainAsync(
                        ChainRef(step["namespaceId"]), ChainRef(step["chainId"]), (string)step["name"]!, step["payload"]?.DeepClone(),
                        new ChainCommandOptions { ExpectedRunId = ChainRef(step["expectedRunId"]), IdempotencyKey = (string?)step["idempotencyKey"], Traceparent = (string?)step["traceparent"] },
                        cancellationToken).ConfigureAwait(false);
                    return new JsonObject { ["deliveredTo"] = new JsonArray([.. signal.DeliveredTo.Select(item => (JsonNode?)JsonValue.Create(item))]) };
                }

            case "cancelChain":
                await client.CancelChainAsync(
                    ChainRef(step["namespaceId"]), ChainRef(step["chainId"]), (string)step["reason"]!,
                    new ChainCommandOptions { ExpectedRunId = ChainRef(step["expectedRunId"]), IdempotencyKey = (string?)step["idempotencyKey"], Traceparent = (string?)step["traceparent"] },
                    cancellationToken).ConfigureAwait(false);
                return new JsonObject { ["accepted"] = true };

            case "signal":
                {
                    var signalled = await client.SignalAsync(RunOf(step), (string)step["name"]!, step["payload"]?.DeepClone() ?? new JsonObject(), null, cancellationToken).ConfigureAwait(false);
                    return new JsonObject { ["deliveredTo"] = new JsonArray([.. signalled.DeliveredTo.Select(item => (JsonNode?)JsonValue.Create(item))]) };
                }

            case "publishEvent":
                {
                    var accepted = await client.PublishEventAsync(
                        new PublishEventRequest
                        {
                            Type = (string)step["type"]!,
                            Data = step["data"]?.DeepClone() ?? new JsonObject(),
                            EventId = (string?)step["eventId"],
                            CorrelationKey = (string?)step["correlationKey"],
                        },
                        null,
                        cancellationToken).ConfigureAwait(false);
                    return new JsonObject { ["eventId"] = accepted.EventId, ["status"] = accepted.Status };
                }

            case "waitForCompletion":
                return RunJson(await client.WaitForCompletionAsync(
                    RunOf(step),
                    new WaitOptions { Timeout = TimeSpan.FromMilliseconds((long?)step["timeoutMs"] ?? 60_000), PollInterval = TimeSpan.FromMilliseconds(100) },
                    cancellationToken).ConfigureAwait(false));
            case "events":
                {
                    var page = await client.EventsAsync(RunOf(step), limit: 200, cancellationToken: cancellationToken).ConfigureAwait(false);
                    var events = new JsonArray();
                    foreach (var item in page.Items)
                    {
                        events.Add(new JsonObject { ["type"] = item.Type, ["stepId"] = item.StepId, ["attempt"] = item.Attempt });
                    }

                    return new JsonObject { ["events"] = events };
                }

            case "sleep":
                await Task.Delay((int)step["ms"]!, cancellationToken).ConfigureAwait(false);
                return null;
            case "control":
                {
                    var body = new JsonObject { ["action"] = (string)step["action"]! };
                    if (step["run"] is not null)
                    {
                        body["run"] = RunOf(step);
                    }

                    if (step["node"] is not null)
                    {
                        body["node"] = (string)step["node"]!;
                    }

                    using var response = await control.PostAsJsonAsync($"{(string)config["controlUrl"]!}/control", body, cancellationToken).ConfigureAwait(false);
                    return response.IsSuccessStatusCode ? null : throw new InvalidOperationException($"control {(string)step["action"]!} answered {(int)response.StatusCode}");
                }

            case "startWorker":
                return await StartWorkerAsync((string)step["worker"]!).ConfigureAwait(false);
            case "stopWorker":
                if (hosts.TryGetValue((string)step["worker"]!, out var stopping) && stopping.Host is { } host)
                {
                    await host.StopAsync(TimeSpan.FromMilliseconds((long?)step["timeoutMs"] ?? 5000), cancellationToken).ConfigureAwait(false);
                }

                return null;
            case "restartWorker":
                {
                    // The host is restartable (SPEC section 6.8): stop, then start, on the same host object.
                    var restarting = hosts[(string)step["worker"]!].Host ?? throw new InvalidOperationException("the worker was never started");
                    await restarting.StopAsync(TimeSpan.FromMilliseconds((long?)step["timeoutMs"] ?? 5000), cancellationToken).ConfigureAwait(false);
                    await restarting.StartAsync(cancellationToken).ConfigureAwait(false);
                    return new JsonObject { ["protocolVersion"] = restarting.ProtocolVersion };
                }

            case "probeWorker":
                return await ProbeAsync(step, cancellationToken).ConfigureAwait(false);
            case "awaitJournal":
                await AwaitJournalAsync(client, step, cancellationToken).ConfigureAwait(false);
                return null;
            case "awaitWorkerLog":
                {
                    var behaviors = hosts[(string)step["worker"]!].Behaviors!;
                    var count = (int)step["count"]!;
                    var end = DateTimeOffset.UtcNow.AddMilliseconds((long?)step["timeoutMs"] ?? 20_000);
                    while (behaviors.Count < count && DateTimeOffset.UtcNow < end)
                    {
                        await Task.Delay(20, cancellationToken).ConfigureAwait(false);
                    }

                    return behaviors.Count < count
                        ? throw new InvalidOperationException($"the worker log has {behaviors.Count} entries, not {count}")
                        : null;
                }

            default:
                throw new InvalidOperationException($"unknown step op {(string)step["op"]!}");
        }
    }

    private async Task<JsonObject> ProbeAsync(JsonObject step, CancellationToken cancellationToken)
    {
        var options = new SagatOptions
        {
            GrpcUrl = (string)config["grpcUrl"]!,
            Namespace = (string)config["namespace"]!,
            Token = (string)config["workerToken"]!,
        };
        options.Retry.MaxAttempts = 1;
        options.Worker.Name = $"probe-{unique}";
        options.Worker.ProtocolVersion = (string)step["protocolVersion"]!;
        var probe = new WorkerHost(options);
        probe.RegisterActivity<JsonNode?, JsonNode?>("conformance.probe", (_, _, _) => Task.FromResult<JsonNode?>(null));
        try
        {
            await probe.StartAsync(cancellationToken).ConfigureAwait(false);
            return new JsonObject { ["accepted"] = probe.ProtocolVersion };
        }
        finally
        {
            try
            {
                await probe.StopAsync(TimeSpan.FromMilliseconds(500), CancellationToken.None).ConfigureAwait(false);
            }
            catch (SagatException)
            {
                // A probe that never started has nothing to stop.
            }
        }
    }

    private async Task AwaitJournalAsync(SagatClient client, JsonObject step, CancellationToken cancellationToken)
    {
        var id = RunOf(step);
        var eventType = (string)step["eventType"]!;
        var stepId = (string?)step["stepId"];
        var end = DateTimeOffset.UtcNow.AddMilliseconds((long?)step["timeoutMs"] ?? 20_000);
        for (; ; )
        {
            var page = await client.EventsAsync(id, limit: 200, cancellationToken: cancellationToken).ConfigureAwait(false);
            if (page.Items.Any(item => item.Type == eventType && (stepId is null || item.StepId == stepId)))
            {
                return;
            }

            if (DateTimeOffset.UtcNow > end)
            {
                throw new InvalidOperationException($"the journal of {id} has no {eventType}");
            }

            await Task.Delay(25, cancellationToken).ConfigureAwait(false);
        }
    }

    private static RunAction ActionOf(string action) => action switch
    {
        "PAUSE" => RunAction.Pause,
        "RESUME" => RunAction.Resume,
        "CANCEL" => RunAction.Cancel,
        "RETRY_STEP" => RunAction.RetryStep,
        "RETRY_COMPENSATION" => RunAction.RetryCompensation,
        "SKIP_COMPENSATION" => RunAction.SkipCompensation,
        "RESOLVE_STEP" => RunAction.ResolveStep,
        _ => throw new InvalidOperationException($"unknown action {action}"),
    };

    internal static JsonObject RunJson(Run run) => new()
    {
        ["runId"] = run.RunId,
        ["workflowId"] = run.WorkflowId,
        ["versionId"] = run.VersionId,
        ["status"] = run.Status,
        ["version"] = run.Version,
        ["paused"] = run.Paused,
        ["startedBy"] = run.StartedBy,
        ["createdAt"] = Iso(run.CreatedAt),
        ["updatedAt"] = Iso(run.UpdatedAt),
        ["finishedAt"] = Iso(run.FinishedAt),
        ["deadline"] = Iso(run.Deadline),
        ["traceId"] = run.TraceId,
        ["correlationId"] = run.CorrelationId,
        ["depth"] = run.Depth,
        ["parent"] = run.Parent is { } parent ? new JsonObject { ["runId"] = parent.RunId, ["node"] = parent.Node, ["cancelPolicy"] = parent.CancelPolicy } : null,
    };

    /// <summary>An instant as the harness requires it: UTC, with a Z, whatever the time zone of the process.</summary>
    private static string? Iso(DateTimeOffset? value) =>
        value?.UtcDateTime.ToString("yyyy-MM-dd'T'HH:mm:ss.fff'Z'", System.Globalization.CultureInfo.InvariantCulture);

    private static JsonObject ErrorJson(Exception error)
    {
        if (error is SagatException sagat)
        {
            return new JsonObject
            {
                ["code"] = sagat.Code.ToWireName(),
                ["status"] = sagat.HttpStatus is { } http ? http : sagat.GrpcStatus,
                ["retryable"] = sagat.Retryable,
                ["traceId"] = sagat.TraceId,
                ["reason"] = sagat.Reason,
                ["retryAfterMs"] = sagat.RetryAfter is { } wait ? (long)wait.TotalMilliseconds : null,
                ["remediation"] = sagat.Remediation,
                ["message"] = sagat.Message,
            };
        }

        return new JsonObject { ["code"] = "UNEXPECTED", ["message"] = error.GetType().Name };
    }
}
