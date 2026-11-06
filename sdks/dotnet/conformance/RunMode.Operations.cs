using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;

namespace SagatOctopusOrchestrator.Sdk.Conformance;

// The steps of feature 018 (schedules, business identifiers, the with-start calls, batch jobs, reset) and the references a step may hold.
// The results have the field names of the TypeScript runner (sdks/typescript/conformance/runner.ts), which the goldens were written from.
internal sealed partial class RunMode
{
    [GeneratedRegex(@"^\{\{workflow:([^}]+)\}\}\z", RegexOptions.CultureInvariant)]
    private static partial Regex WorkflowRef();

    [GeneratedRegex(@"^\{\{step:([^:}]+):([^}]+)\}\}\z", RegexOptions.CultureInvariant)]
    private static partial Regex StepRef();

    [GeneratedRegex(@"^\{\{now:(-?\d+)\}\}\z", RegexOptions.CultureInvariant)]
    private static partial Regex NowRef();

    [GeneratedRegex(@"^\{\{hour:(-?\d+)\}\}\z", RegexOptions.CultureInvariant)]
    private static partial Regex HourRef();

    private string NamespaceId => (string)config["namespaceId"]!;

    // ------------------------------------------------------------------------------------------ references

    /// <summary>
    /// References inside the parameters of a step: a value an earlier step produced (<c>{{step:id:field}}</c>), a workflow id
    /// (<c>{{workflow:key}}</c>), the namespace id, and instants computed when the step runs (<c>{{now:offsetMs}}</c>, and
    /// <c>{{hour:offsetHours}}</c> for the start of a UTC hour). A string that is exactly one reference becomes its value.
    /// </summary>
    private JsonNode? Refs(JsonNode? node) => node switch
    {
        null => null,
        JsonObject obj => new JsonObject(obj.Select(pair => KeyValuePair.Create(pair.Key, Refs(pair.Value)))),
        JsonArray array => new JsonArray([.. array.Select(Refs)]),
        JsonValue value when value.GetValueKind() == JsonValueKind.String => RefOf(value.GetValue<string>()),
        _ => node.DeepClone(),
    };

    private JsonNode? RefOf(string text)
    {
        if (text == "{{namespaceId}}")
        {
            return JsonValue.Create(NamespaceId);
        }

        if (WorkflowRef().Match(text) is { Success: true } workflow)
        {
            var key = workflow.Groups[1].Value;
            return config["workflows"]?[key]?["workflowId"]?.DeepClone() ?? throw new InvalidOperationException($"no workflow {key}");
        }

        if (StepRef().Match(text) is { Success: true } step)
        {
            var id = step.Groups[1].Value;
            var field = step.Groups[2].Value;
            return results.TryGetValue(id, out var result) && result.TryGetPropertyValue(field, out var found)
                ? found?.DeepClone()
                : throw new InvalidOperationException($"step {id} has no {field}");
        }

        if (NowRef().Match(text) is { Success: true } now)
        {
            return JsonValue.Create(Iso(DateTimeOffset.UtcNow.AddMilliseconds(long.Parse(now.Groups[1].Value, CultureInfo.InvariantCulture))));
        }

        if (HourRef().Match(text) is { Success: true } hour)
        {
            var at = DateTimeOffset.UtcNow;
            var start = new DateTimeOffset(at.Year, at.Month, at.Day, at.Hour, 0, 0, TimeSpan.Zero);
            return JsonValue.Create(Iso(start.AddHours(long.Parse(hour.Groups[1].Value, CultureInfo.InvariantCulture))));
        }

        return JsonValue.Create(text);
    }

    /// <summary>The id of the step whose result holds this run, or <c>?</c> for a run no step of the scenario started. In the order the steps ran.</summary>
    private string StepOfRun(string runId)
    {
        foreach (var (id, found) in results)
        {
            if (TextOf(found, "runId") == runId || TextOf(found, "newRunId") == runId)
            {
                return id;
            }
        }

        return "?";
    }

    private static string? TextOf(JsonObject observed, string field) =>
        observed[field] is JsonValue value && value.TryGetValue<string>(out var text) ? text : null;

    private string StepResult(JsonNode? id, string field) =>
        results.TryGetValue((string)id!, out var found) && found[field] is JsonValue value && value.TryGetValue<string>(out var text)
            ? text
            : throw new InvalidOperationException($"step {(string?)id} has no {field}");

    private static async Task AwaitingAsync(string what, long timeoutMs, Func<Task<bool>> done, CancellationToken cancellationToken)
    {
        var end = DateTimeOffset.UtcNow.AddMilliseconds(timeoutMs);
        while (!await done().ConfigureAwait(false))
        {
            if (DateTimeOffset.UtcNow > end)
            {
                throw new InvalidOperationException($"timed out waiting for {what}");
            }

            await Task.Delay(50, cancellationToken).ConfigureAwait(false);
        }
    }

    // ------------------------------------------------------------------------------------------------ steps

    private async Task<JsonObject?> OperationAsync(JsonObject step, SagatClient client, CancellationToken cancellationToken)
    {
        var key = (string?)step["idempotencyKey"];
        switch ((string)step["op"]!)
        {
            case "scheduleCreate":
                return ScheduleJson(await client.Schedules.CreateAsync(NamespaceId, WriteOf(step["write"]!.AsObject()), new RequestOptions { IdempotencyKey = key }, cancellationToken).ConfigureAwait(false));
            case "scheduleGet":
                return DescriptionJson(await client.Schedules.GetAsync(
                    NamespaceId, StepResult(step["schedule"], "scheduleId"),
                    new ScheduleDescribeOptions { NextCount = (int?)step["nextCount"], RecentCount = (int?)step["recentCount"] }, cancellationToken).ConfigureAwait(false));
            case "scheduleList":
                {
                    var names = new JsonArray();
                    void Read(Schedule schedule) => names.Add(new JsonObject
                    {
                        ["name"] = schedule.Name,
                        ["state"] = schedule.State,
                        ["origin"] = schedule.Origin,
                        ["revision"] = schedule.Revision,
                    });
                    var filter = step["filter"] as JsonObject;
                    var options = new ScheduleListOptions
                    {
                        Name = (string?)filter?["name"],
                        State = (string?)filter?["state"],
                        Origin = (string?)filter?["origin"],
                        WorkflowId = (string?)filter?["workflowId"],
                        Limit = (int?)step["limit"],
                    };
                    string? nextCursor = null;
                    if ((bool?)step["all"] == true)
                    {
                        await foreach (var item in client.Schedules.ListAllAsync(NamespaceId, options, cancellationToken).ConfigureAwait(false))
                        {
                            Read(item);
                        }
                    }
                    else
                    {
                        var listed = await client.Schedules.ListAsync(NamespaceId, options, cancellationToken).ConfigureAwait(false);
                        foreach (var item in listed.Items)
                        {
                            Read(item);
                        }

                        nextCursor = listed.NextCursor;
                    }

                    return new JsonObject { ["items"] = names, ["nextCursor"] = Cursor(nextCursor) };
                }

            case "scheduleUpdate":
                return ScheduleJson(await client.Schedules.UpdateAsync(
                    NamespaceId, StepResult(step["schedule"], "scheduleId"), WriteOf(step["write"]!.AsObject()), (long)step["expectedRevision"]!,
                    new RequestOptions { IdempotencyKey = key }, cancellationToken).ConfigureAwait(false));
            case "scheduleDelete":
                await client.Schedules.DeleteAsync(
                    NamespaceId, StepResult(step["schedule"], "scheduleId"), (long)step["expectedRevision"]!,
                    new RequestOptions { IdempotencyKey = key }, cancellationToken).ConfigureAwait(false);
                return new JsonObject { ["deleted"] = true };
            case "schedulePause":
                return ScheduleJson(await client.Schedules.PauseAsync(
                    NamespaceId, StepResult(step["schedule"], "scheduleId"), (string)step["note"]!, new RequestOptions { IdempotencyKey = key }, cancellationToken).ConfigureAwait(false));
            case "scheduleResume":
                return ScheduleJson(await client.Schedules.ResumeAsync(
                    NamespaceId, StepResult(step["schedule"], "scheduleId"), (string)step["note"]!, new RequestOptions { IdempotencyKey = key }, cancellationToken).ConfigureAwait(false));
            case "scheduleTrigger":
                return ActionJson(await client.Schedules.TriggerAsync(
                    NamespaceId, StepResult(step["schedule"], "scheduleId"),
                    new ScheduleTriggerOptions { Overlap = (string?)step["overlap"], IdempotencyKey = key }, cancellationToken).ConfigureAwait(false));
            case "scheduleBackfill":
                return BackfillJson(await client.Schedules.BackfillAsync(
                    NamespaceId, StepResult(step["schedule"], "scheduleId"), (string)step["start"]!, (string)step["end"]!,
                    new ScheduleBackfillOptions { Overlap = (string?)step["overlap"], IdempotencyKey = key }, cancellationToken).ConfigureAwait(false));
            case "scheduleCancelBackfill":
                return BackfillJson(await client.Schedules.CancelBackfillAsync(
                    NamespaceId, StepResult(step["schedule"], "scheduleId"), StepResult(step["backfill"], "backfillId"),
                    new RequestOptions { IdempotencyKey = key }, cancellationToken).ConfigureAwait(false));
            case "scheduleBackfills":
                return new JsonObject
                {
                    ["items"] = new JsonArray([.. (await client.Schedules.ListBackfillsAsync(NamespaceId, StepResult(step["schedule"], "scheduleId"), null, cancellationToken).ConfigureAwait(false))
                        .Select(backfill => (JsonNode?)BackfillJson(backfill))]),
                };
            case "scheduleActions":
                {
                    var listed = await client.Schedules.ListActionsAsync(
                        NamespaceId, StepResult(step["schedule"], "scheduleId"),
                        new ScheduleActionListOptions { Outcome = (string?)step["outcome"], Limit = (int?)step["limit"] }, cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["items"] = new JsonArray([.. listed.Items.Select(action => (JsonNode?)ActionJson(action))]),
                        ["nextCursor"] = Cursor(listed.NextCursor),
                    };
                }

            case "awaitSchedule":
                {
                    // The scheduler decides the times of a backfill: wait for its counters, never for a delay.
                    var id = StepResult(step["schedule"], "scheduleId");
                    var counter = (string)step["counter"]!;
                    var atLeast = (long?)step["atLeast"] ?? 1;
                    await AwaitingAsync($"schedule counter {counter}", (long?)step["timeoutMs"] ?? 30_000, async () =>
                    {
                        var described = await client.Schedules.GetAsync(NamespaceId, id, new ScheduleDescribeOptions { NextCount = 0, RecentCount = 0 }, cancellationToken).ConfigureAwait(false);
                        return described.Counters.GetValueOrDefault(counter) >= atLeast;
                    }, cancellationToken).ConfigureAwait(false);
                    return null;
                }

            case "awaitBackfill":
                {
                    // A backfill counts the times it found already acted on in its own counts, not in the schedule's counters.
                    var schedule = StepResult(step["schedule"], "scheduleId");
                    var id = StepResult(step["backfill"], "backfillId");
                    var state = (string)step["state"]!;
                    await AwaitingAsync($"backfill state {state}", (long?)step["timeoutMs"] ?? 30_000, async () =>
                        (await client.Schedules.ListBackfillsAsync(NamespaceId, schedule, null, cancellationToken).ConfigureAwait(false))
                            .Any(backfill => backfill.BackfillId == id && backfill.State == state), cancellationToken).ConfigureAwait(false);
                    return null;
                }

            case "businessIdGet":
                {
                    var binding = await client.BusinessIds.GetAsync(NamespaceId, (string)step["businessId"]!, null, cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["businessId"] = binding.BusinessId,
                        ["executionId"] = binding.ExecutionId,
                        ["currentRunId"] = binding.CurrentRunId,
                        ["open"] = binding.Open,
                        ["lastStatus"] = binding.LastStatus,
                        ["workflowId"] = binding.WorkflowId,
                        ["boundAt"] = Iso(binding.BoundAt),
                        ["reusableAfter"] = Iso(binding.ReusableAfter),
                    };
                }

            case "signalWithStart":
                {
                    var signal = step["signal"]!.AsObject();
                    var answered = await client.SignalWithStartAsync(
                        WorkflowIdOf(step), (string)step["businessId"]!, step["input"]?.DeepClone() ?? new JsonObject(),
                        (string)signal["name"]!, signal["payload"]?.DeepClone() ?? new JsonObject(),
                        new WithStartOptions { IdempotencyKey = key, BusinessIdReusePolicy = (string?)step["businessIdReusePolicy"] }, cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["runId"] = answered.RunId,
                        ["started"] = answered.Started,
                        ["deliveredTo"] = new JsonArray([.. answered.DeliveredTo.Select(node => (JsonNode?)JsonValue.Create(node))]),
                    };
                }

            case "updateWithStart":
                {
                    var update = step["update"]!.AsObject();
                    var answered = await client.UpdateWithStartAsync(
                        WorkflowIdOf(step), (string)step["businessId"]!, step["input"]?.DeepClone() ?? new JsonObject(),
                        (string)update["name"]!, (string)update["updateKey"]!, update["args"]?.DeepClone(),
                        new UpdateWithStartOptions
                        {
                            IdempotencyKey = key,
                            BusinessIdConflictPolicy = (string?)step["businessIdConflictPolicy"],
                            BusinessIdReusePolicy = (string?)step["businessIdReusePolicy"],
                            StartDelay = DelayOf(step),
                            ExpectedStateRevision = (long?)update["expectedStateRevision"],
                        }, cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["runId"] = answered.RunId,
                        ["started"] = answered.Started,
                        ["update"] = new JsonObject
                        {
                            ["outcome"] = answered.Update.Outcome,
                            ["reason"] = answered.Update.Reason,
                            ["receipt"] = ReceiptJson(answered.Update.Receipt),
                        },
                    };
                }

            case "batchPreview":
                {
                    var preview = await client.Batch.PreviewAsync(NamespaceId, SelectorOf(step["selector"]!.AsObject()), (long?)step["maxItems"], null, cancellationToken).ConfigureAwait(false);
                    // The sample is a set of runs in the order the server chose; the scenario compares which of its own runs it holds.
                    var steps = preview.Sample.Select(StepOfRun).ToArray();
                    Array.Sort(steps, StringComparer.Ordinal);
                    return new JsonObject
                    {
                        ["matched"] = preview.Matched,
                        ["capped"] = preview.Capped,
                        ["sampleSize"] = preview.Sample.Count,
                        ["sampleSteps"] = new JsonArray([.. steps.Select(name => (JsonNode?)JsonValue.Create(name))]),
                    };
                }

            case "batchCreate":
                {
                    // What does not change while the job runs (its state and counters do): the job as it was accepted.
                    var job = await client.Batch.CreateAsync(NamespaceId, BatchRequestOf(step["request"]!.AsObject()), new RequestOptions { IdempotencyKey = key }, cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["batchId"] = job.BatchId,
                        ["action"] = job.Action,
                        ["createdBy"] = job.CreatedBy,
                        ["hasPayload"] = job.HasPayload,
                        ["maxItems"] = job.MaxItems,
                        ["itemsPerSecond"] = job.ItemsPerSecond,
                    };
                }

            case "batchGet":
                return JobJson(await client.Batch.GetAsync(NamespaceId, StepResult(step["batch"], "batchId"), null, cancellationToken).ConfigureAwait(false));
            case "batchList":
                {
                    var listed = await client.Batch.ListAsync(NamespaceId, new BatchListOptions { State = (string?)step["state"], Limit = (int?)step["limit"] }, cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["items"] = new JsonArray([.. listed.Items.Select(job => (JsonNode?)new JsonObject
                        {
                            ["action"] = job.Action, ["state"] = job.State, ["counts"] = CountsJson(job.Counts),
                        })]),
                        ["nextCursor"] = Cursor(listed.NextCursor),
                    };
                }

            case "batchCancel":
                return JobJson(await client.Batch.CancelAsync(
                    NamespaceId, StepResult(step["batch"], "batchId"), (string)step["reason"]!, new RequestOptions { IdempotencyKey = key }, cancellationToken).ConfigureAwait(false));
            case "batchItems":
                {
                    var listed = await client.Batch.ListItemsAsync(
                        NamespaceId, StepResult(step["batch"], "batchId"),
                        new BatchListOptions { State = (string?)step["state"], Limit = (int?)step["limit"] }, cancellationToken).ConfigureAwait(false);
                    // The order of the items is the server's; which of the scenario's runs sits at which ordinal is not part of the
                    // contract, so the items are named by the step that started the run and sorted by it.
                    var items = listed.Items
                        .Select(item => (RunStep: StepOfRun(item.RunId), Item: item))
                        .OrderBy(pair => pair.RunStep, StringComparer.Ordinal)
                        .Select(pair => (JsonNode?)new JsonObject
                        {
                            ["runStep"] = pair.RunStep,
                            ["state"] = pair.Item.State,
                            ["reason"] = pair.Item.Reason,
                            ["errorCode"] = pair.Item.ErrorCode,
                            ["attempts"] = pair.Item.Attempts,
                            ["newRunId"] = pair.Item.NewRunId,
                        });
                    return new JsonObject { ["items"] = new JsonArray([.. items]), ["nextCursor"] = Cursor(listed.NextCursor) };
                }

            case "awaitBatch":
                {
                    var id = StepResult(step["batch"], "batchId");
                    var state = (string)step["state"]!;
                    await AwaitingAsync($"batch state {state}", (long?)step["timeoutMs"] ?? 30_000, async () =>
                        (await client.Batch.GetAsync(NamespaceId, id, null, cancellationToken).ConfigureAwait(false)).State == state, cancellationToken).ConfigureAwait(false);
                    return null;
                }

            case "resetPoints":
                {
                    var listed = await client.ResetPointsAsync(RunOf(step), new ResetPointListOptions { Limit = (int?)step["limit"] }, cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["items"] = new JsonArray([.. listed.Items.Select(point => (JsonNode?)new JsonObject
                        {
                            ["sequence"] = point.Sequence, ["kind"] = point.Kind, ["node"] = point.Node, ["occurredAt"] = Iso(point.OccurredAt),
                        })]),
                        ["nextCursor"] = Cursor(listed.NextCursor),
                    };
                }

            case "resetPreview":
                {
                    var plan = await client.PreviewResetAsync(RunOf(step), (long)step["sequence"]!, (string?)step["reapply"], null, cancellationToken).ConfigureAwait(false);
                    return new JsonObject
                    {
                        ["sequence"] = plan.Sequence,
                        ["restored"] = new JsonArray([.. plan.Restored.Select(restored => (JsonNode?)new JsonObject { ["node"] = restored.Node, ["kind"] = restored.Kind })]),
                        ["reexecuted"] = new JsonArray([.. plan.Reexecuted.Select(rerun => (JsonNode?)new JsonObject
                        {
                            ["node"] = rerun.Node,
                            ["completedInSource"] = rerun.CompletedInSource,
                            ["markers"] = new JsonArray([.. rerun.Markers.Select(marker => (JsonNode?)JsonValue.Create(marker))]),
                        })]),
                        ["reapply"] = new JsonArray([.. plan.Reapply.Select(Node)]),
                        ["requiresAcknowledgement"] = plan.RequiresAcknowledgement,
                        ["prefixEvents"] = plan.PrefixEvents,
                    };
                }

            case "reset":
                return ResetJson(await client.ResetAsync(
                    RunOf(step),
                    new ResetRequest
                    {
                        Sequence = (long)step["sequence"]!,
                        Reason = (string)step["reason"]!,
                        Reapply = (string?)step["reapply"],
                        SourcePolicy = (string?)step["sourcePolicy"],
                        BusinessId = (string?)step["businessId"],
                        AcknowledgeReexecution = (bool?)step["acknowledgeReexecution"],
                    },
                    new RequestOptions { IdempotencyKey = key }, cancellationToken).ConfigureAwait(false));
            case "resets":
                return new JsonObject
                {
                    ["items"] = new JsonArray([.. (await client.ListResetsAsync(RunOf(step), null, cancellationToken).ConfigureAwait(false)).Select(reset => (JsonNode?)ResetJson(reset))]),
                };
            case "getReset":
                return ResetJson(await client.GetResetAsync(RunOf(step), StepResult(step["reset"], "resetId"), null, cancellationToken).ConfigureAwait(false));
            default:
                throw new InvalidOperationException($"unknown step op {(string)step["op"]!}");
        }
    }

    private string WorkflowIdOf(JsonObject step) =>
        step["workflow"] is { } key
            ? (string)config["workflows"]![(string)key!]!["workflowId"]!
            : (string)step["workflowId"]!;

    /// <summary>The scenario gives a start delay in milliseconds (a fraction included); the SDK takes a duration.</summary>
    private static TimeSpan? DelayOf(JsonObject step) =>
        step["startDelayMs"] is { } milliseconds ? TimeSpan.FromMilliseconds((double)milliseconds) : null;

    // -------------------------------------------------------------------------------- scenario to typed requests

    private static ScheduleWrite WriteOf(JsonObject write) => new()
    {
        Name = (string)write["name"]!,
        Spec = SpecOf(write["spec"]!.AsObject()),
        Policies = write["policies"] is JsonObject policies
            ? new SchedulePolicies
            {
                Overlap = (string?)policies["overlap"],
                CatchupWindowMs = (long?)policies["catchupWindowMs"],
                CatchupMode = (string?)policies["catchupMode"],
                PauseOnFailure = (bool?)policies["pauseOnFailure"],
                BufferLimit = (long?)policies["bufferLimit"],
            }
            : null,
        Action = ScheduleActionOf(write["action"]!.AsObject()),
        Paused = (bool?)write["paused"],
        Note = (string?)write["note"],
        RemainingActions = (long?)write["remainingActions"],
    };

    private static ScheduleSpec SpecOf(JsonObject spec) => new()
    {
        Calendars = CalendarsOf(spec["calendars"]),
        Cron = spec["cron"] is JsonArray cron ? [.. cron.Select(line => (string)line!)] : null,
        Intervals = spec["intervals"] is JsonArray intervals
            ? [.. intervals.Select(item => new ScheduleInterval((long)item!["everyMs"]!, (long?)item["offsetMs"]))]
            : null,
        Exclusions = CalendarsOf(spec["exclusions"]),
        StartAt = spec["startAt"] is { } startAt ? Instant.Parse((string)startAt!) : null,
        EndAt = spec["endAt"] is { } endAt ? Instant.Parse((string)endAt!) : null,
        Timezone = (string?)spec["timezone"],
        JitterMs = (long?)spec["jitterMs"],
        Dst = spec["dst"] is JsonObject dst ? new ScheduleDst { Gap = (string?)dst["gap"], Ambiguous = (string?)dst["ambiguous"] } : null,
    };

    private static List<ScheduleCalendarSpec>? CalendarsOf(JsonNode? node) =>
        node is JsonArray array
            ? [.. array.Select(item => new ScheduleCalendarSpec
            {
                Second = RangesOf(item!["second"]),
                Minute = RangesOf(item["minute"]),
                Hour = RangesOf(item["hour"]),
                DayOfMonth = RangesOf(item["dayOfMonth"]),
                Month = RangesOf(item["month"]),
                Year = RangesOf(item["year"]),
                DayOfWeek = RangesOf(item["dayOfWeek"]),
                Comment = (string?)item["comment"],
            })]
            : null;

    private static List<ScheduleRange>? RangesOf(JsonNode? node) =>
        node is JsonArray array ? [.. array.Select(item => new ScheduleRange((long)item!["start"]!, (long?)item["end"], (long?)item["step"]))] : null;

    private static ScheduleAction ScheduleActionOf(JsonObject action) => new()
    {
        WorkflowId = (string)action["workflowId"]!,
        VersionPolicy = (string?)action["versionPolicy"],
        VersionId = (string?)action["versionId"],
        Input = action["input"]?.DeepClone() ?? new JsonObject(),
        NominalTimeField = (string?)action["nominalTimeField"],
        RunTimeoutMs = (long?)action["runTimeoutMs"],
    };

    private static BatchSelector SelectorOf(JsonObject selector)
    {
        if (selector["filter"] is JsonObject filter)
        {
            return BatchSelector.ByFilter(new BatchFilter
            {
                WorkflowId = (string?)filter["workflowId"],
                VersionId = (string?)filter["versionId"],
                Status = filter["status"] is JsonArray status ? [.. status.Select(item => (string)item!)] : null,
                Origin = (string?)filter["origin"],
                ScheduleId = (string?)filter["scheduleId"],
                CreatedAfter = filter["createdAfter"] is { } after ? Instant.Parse((string)after!) : null,
                CreatedBefore = filter["createdBefore"] is { } before ? Instant.Parse((string)before!) : null,
                BusinessIdPrefix = (string?)filter["businessIdPrefix"],
            });
        }

        if (selector["runIds"] is JsonArray runIds)
        {
            return BatchSelector.ByRunIds(runIds.Select(item => (string)item!));
        }

        return BatchSelector.ByQuery((string)selector["query"]!);
    }

    private static BatchRequest BatchRequestOf(JsonObject request) => new()
    {
        Selector = SelectorOf(request["selector"]!.AsObject()),
        Action = (string)request["action"]!,
        Signal = request["signal"] is JsonObject signal ? new BatchSignal((string)signal["name"]!, signal["payload"]?.DeepClone()) : null,
        Update = request["update"] is JsonObject update ? new BatchUpdate((string)update["name"]!, update["args"]?.DeepClone()) : null,
        StepId = (string?)request["stepId"],
        BusinessId = (string?)request["businessId"],
        Reason = (string)request["reason"]!,
        MaxItems = (long?)request["maxItems"],
        ItemsPerSecond = (long?)request["itemsPerSecond"],
    };

    // ----------------------------------------------------------------------------------------- observations

    // Instants that depend on when the scenario ran are written as instants (the harness compares them as <time>); the ones a fixed spec
    // determines are written as epoch milliseconds, so that a wrong parse of an instant shows.
    private static long? Epoch(DateTimeOffset? value) => value?.ToUnixTimeMilliseconds();

    /// <summary>A page shows its cursor as the literal <c>&lt;cursor&gt;</c> when there is one: the cursor itself is opaque and differs per run.</summary>
    private static string? Cursor(string? nextCursor) => nextCursor is null ? null : "<cursor>";

    private static JsonNode? Node(JsonElement element) =>
        element.ValueKind == JsonValueKind.Undefined ? null : JsonNode.Parse(element.GetRawText());

    private static JsonObject ScheduleJson(Schedule schedule) => new()
    {
        ["scheduleId"] = schedule.ScheduleId,
        ["name"] = schedule.Name,
        ["origin"] = schedule.Origin,
        ["state"] = schedule.State,
        ["revision"] = schedule.Revision,
        ["exhausted"] = schedule.Exhausted,
        ["action"] = new JsonObject
        {
            ["workflowId"] = schedule.Action.WorkflowId,
            ["versionPolicy"] = schedule.Action.VersionPolicy,
            ["versionId"] = schedule.Action.VersionId,
            ["inputDigest"] = schedule.Action.InputDigest,
            ["nominalTimeField"] = schedule.Action.NominalTimeField,
            ["runTimeoutMs"] = schedule.Action.RunTimeoutMs,
        },
        ["note"] = schedule.Note,
        ["noteBy"] = schedule.NoteBy,
        ["remainingActions"] = schedule.RemainingActions,
        ["nextActionEpochMs"] = Epoch(schedule.NextActionAt),
        ["createdAt"] = Iso(schedule.CreatedAt),
        ["updatedAt"] = Iso(schedule.UpdatedAt),
    };

    private static JsonObject ActionJson(ScheduleActionRecord action) => new()
    {
        ["kind"] = action.Kind,
        ["origin"] = action.Origin,
        ["revision"] = action.Revision,
        ["outcome"] = action.Outcome,
        ["reason"] = action.Reason,
        ["runId"] = action.RunId,
        ["backfillId"] = action.BackfillId,
        ["nominalTime"] = Iso(action.NominalTime),
        ["actualTime"] = Iso(action.ActualTime),
        ["summaryCount"] = action.SummaryCount,
        ["summaryLastNominalTime"] = Iso(action.SummaryLastNominalTime),
        ["decidedAt"] = Iso(action.DecidedAt),
    };

    private static JsonObject DescriptionJson(ScheduleDescription description)
    {
        var observed = ScheduleJson(description);
        observed["nextActions"] = new JsonArray([.. description.NextActions.Select(next => (JsonNode?)new JsonObject
        {
            ["nominalTimeEpochMs"] = Epoch(next.NominalTime), ["actualTimeEpochMs"] = Epoch(next.ActualTime),
        })]);
        observed["recentActions"] = new JsonArray([.. description.RecentActions.Select(action => (JsonNode?)ActionJson(action))]);
        observed["openRuns"] = new JsonArray([.. description.OpenRuns.Select(run => (JsonNode?)JsonValue.Create(run))]);
        observed["bufferedCount"] = description.BufferedCount;
        observed["counters"] = CountersJson(description.Counters);
        return observed;
    }

    private static JsonObject CountersJson(IReadOnlyDictionary<string, long> counters)
    {
        var observed = new JsonObject();
        foreach (var (name, count) in counters)
        {
            observed[name] = count;
        }

        return observed;
    }

    private static JsonObject BackfillJson(ScheduleBackfill backfill) => new()
    {
        ["backfillId"] = backfill.BackfillId,
        ["start"] = Iso(backfill.Start),
        ["end"] = Iso(backfill.End),
        ["cursor"] = Iso(backfill.Cursor),
        ["state"] = backfill.State,
        ["overlap"] = backfill.Overlap,
        ["counts"] = CountersJson(backfill.Counts),
    };

    private static JsonObject CountsJson(BatchCounts counts) => new()
    {
        ["selected"] = counts.Selected,
        ["succeeded"] = counts.Succeeded,
        ["skipped"] = counts.Skipped,
        ["failed"] = counts.Failed,
        ["cancelled"] = counts.Cancelled,
        ["deferred"] = counts.Deferred,
    };

    private static JsonObject JobJson(BatchJob job) => new()
    {
        ["batchId"] = job.BatchId,
        ["action"] = job.Action,
        ["state"] = job.State,
        ["failureReason"] = job.FailureReason,
        ["createdBy"] = job.CreatedBy,
        ["hasPayload"] = job.HasPayload,
        ["maxItems"] = job.MaxItems,
        ["itemsPerSecond"] = job.ItemsPerSecond,
        ["counts"] = CountsJson(job.Counts),
        ["createdAt"] = Iso(job.CreatedAt),
        ["startedAt"] = Iso(job.StartedAt),
        ["finishedAt"] = Iso(job.FinishedAt),
        ["expiresAt"] = Iso(job.ExpiresAt),
    };

    private static JsonObject ResetJson(Reset reset) => new()
    {
        ["resetId"] = reset.ResetId,
        ["sourceRunId"] = reset.SourceRunId,
        ["sequence"] = reset.Sequence,
        ["state"] = reset.State,
        ["failureReason"] = reset.FailureReason,
        ["newRunId"] = reset.NewRunId,
        ["reapplyReport"] = new JsonArray([.. reset.ReapplyReport.Select(Node)]),
        ["createdAt"] = Iso(reset.CreatedAt),
    };

    private static JsonObject? ReceiptJson(UpdateWithStartReceipt? receipt) =>
        receipt is null
            ? null
            : new JsonObject
            {
                ["operation"] = receipt.Operation,
                ["updateKey"] = receipt.UpdateKey,
                ["status"] = receipt.Status,
                ["result"] = Node(receipt.Result),
                ["beforeStateRevision"] = receipt.BeforeStateRevision,
                ["afterStateRevision"] = receipt.AfterStateRevision,
                ["runVersion"] = receipt.RunVersion,
                ["replayed"] = receipt.Replayed,
            };
}
