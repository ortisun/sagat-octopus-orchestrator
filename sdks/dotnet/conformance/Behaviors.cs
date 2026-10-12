using System.Text.Json.Nodes;
using System.Security.Cryptography;
using SagatOctopusOrchestrator.Sdk.Workers;

namespace SagatOctopusOrchestrator.Sdk.Conformance;

/// <summary>The behaviours of the activities a scenario registers (README, "Worker activities"), as handlers of the SDK.</summary>
internal sealed class Behaviors(string workerKey, List<JsonObject> log)
{
    private readonly object gate = new();

    internal string WorkerKey { get; } = workerKey;

    internal async Task<object?> ActivityAsync(string name, string kind, JsonObject behavior, JsonNode? input, ActivityExecutionContext context, CancellationToken cancellationToken)
    {
        var outcome = "returned";
        try
        {
            var result = await RunAsync(behavior, input, context, value => outcome = value, cancellationToken).ConfigureAwait(false);
            return result;
        }
        finally
        {
            Record(name, kind, outcome, input, context);
        }
    }

    internal async Task<Verdict> ReconcilerAsync(string name, JsonObject behavior, JsonNode? input, ActivityExecutionContext context, CancellationToken cancellationToken)
    {
        var outcome = "returned";
        try
        {
            var result = await RunAsync(behavior, input, context, value => outcome = value, cancellationToken).ConfigureAwait(false);
            return result as Verdict ?? throw new InvalidOperationException("a reconciler behaviour answers a verdict");
        }
        finally
        {
            Record(name, "reconciler", outcome, input, context);
        }
    }

    private static async Task<object?> RunAsync(JsonObject behavior, JsonNode? input, ActivityExecutionContext context, Action<string> outcome, CancellationToken cancellationToken)
    {
        if (behavior["failsUntilAttempt"] is { } until && context.Attempt < (int)until)
        {
            outcome("failed");
            throw Failure(behavior["fails"]!.AsObject());
        }

        if (behavior["fails"] is JsonObject fails && behavior["failsUntilAttempt"] is null)
        {
            outcome("failed");
            throw Failure(fails);
        }

        if (behavior["throws"] is { } throws)
        {
            outcome("threw");
            throw new InvalidOperationException((string)throws!);
        }

        if (behavior["outcomeUnknown"] is { } unknown)
        {
            outcome("failed");
            throw ActivityFailure.OutcomeUnknown((string)unknown!);
        }

        if ((bool?)behavior["hang"] == true)
        {
            try
            {
                await Task.Delay(Timeout.Infinite, cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                // That is what it waits for.
            }

            outcome("cancelled");

            // A handler that is cancelled and says so ends by raising its cancellation error.
            if ((bool?)behavior["onCancel"]?["raises"] == true)
            {
                throw new OperationCanceledException(cancellationToken);
            }

            return null;
        }

        if (behavior["sleepMs"] is { } sleep)
        {
            try
            {
                // A handler that never looks at its token waits whatever happens, and what it returns is what is reported.
                await Task.Delay((int)sleep, (bool?)behavior["ignoresCancel"] == true ? CancellationToken.None : cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                outcome("cancelled");
                if ((bool?)behavior["onCancel"]?["raises"] == true)
                {
                    throw;
                }

                return behavior["onCancel"]?["returns"]?.DeepClone();
            }
        }

        if (behavior["readsSecret"] is { } secretName)
        {
            var answer = behavior["returns"] is JsonObject returns ? (JsonObject)returns.DeepClone() : [];
            ActivitySecrets secrets;
            try
            {
                secrets = await context.GetSecretsAsync(cancellationToken).ConfigureAwait(false);
            }
            catch (SagatException refusal) when (behavior["reportsSecretError"] is not null)
            {
                // `reportsSecretError`: the canonical code of the refusal is the output, so the scenario can compare it.
                answer["secretError"] = refusal.Code.ToWireName();
                return answer;
            }

            answer["secretLength"] = secrets.Get((string)secretName!).Length;
            return answer;
        }

        if (behavior["blobRoundTripBytes"] is { } blobLength)
        {
            var body = Enumerable.Repeat((byte)'x', (int)blobLength).ToArray();
            var sha256 = Convert.ToHexString(SHA256.HashData(body)).ToLowerInvariant();
            await using var source = new MemoryStream(body, writable: false);
            var descriptor = await context.WriteBlobAsync(
                source, new BlobWriteOptions("application/octet-stream", body.Length, sha256), cancellationToken).ConfigureAwait(false);
            using var read = new MemoryStream();
            await foreach (var chunk in context.ReadBlobAsync(descriptor, cancellationToken).ConfigureAwait(false))
            {
                await read.WriteAsync(chunk, cancellationToken).ConfigureAwait(false);
            }

            if (!read.ToArray().AsSpan().SequenceEqual(body))
            {
                throw new InvalidOperationException("blob round trip failed");
            }

            return new JsonObject { ["blobSize"] = descriptor.Size, ["blobSha256"] = descriptor.Sha256, ["roundTrip"] = true };
        }

        if (behavior["blobInputBytes"] is { } inputLength)
        {
            var raw = input!["$blob"]!.AsObject();
            var descriptor = new BlobDescriptor((string)raw["id"]!, (long)raw["size"]!, (string)raw["sha256"]!, (string)raw["contentType"]!);
            using var read = new MemoryStream();
            await foreach (var chunk in context.ReadBlobAsync(descriptor, cancellationToken).ConfigureAwait(false))
            {
                await read.WriteAsync(chunk, cancellationToken).ConfigureAwait(false);
            }
            if (!read.ToArray().AsSpan().SequenceEqual(Enumerable.Repeat((byte)'i', (int)inputLength).ToArray()))
            {
                throw new InvalidOperationException("run-input blob did not match");
            }
            return new JsonObject { ["inputBlobSize"] = descriptor.Size, ["inputBlobSha256"] = descriptor.Sha256 };
        }

        if (behavior["blobOutputBytes"] is { } outputLength)
        {
            await using var source = new MemoryStream(Enumerable.Repeat((byte)'y', (int)outputLength).ToArray(), writable: false);
            var descriptor = await context.WriteBlobAsync(
                source, new BlobWriteOptions("application/octet-stream"), cancellationToken).ConfigureAwait(false);
            return new BlobOutput(descriptor);
        }

        if (behavior["blobInlineBytes"] is { } inlineLength)
        {
            await using var source = new MemoryStream(Enumerable.Repeat((byte)'z', (int)inlineLength).ToArray(), writable: false);
            var descriptor = await context.WriteBlobAsync(
                source, new BlobWriteOptions("application/octet-stream"), cancellationToken).ConfigureAwait(false);
            return new JsonObject
            {
                ["$blob"] = new JsonObject
                {
                    ["id"] = descriptor.Id,
                    ["size"] = descriptor.Size,
                    ["sha256"] = descriptor.Sha256,
                    ["contentType"] = descriptor.ContentType,
                },
            };
        }

        if (behavior["verdict"] is { } verdict)
        {
            return (string)verdict! switch
            {
                "applied" => behavior.ContainsKey("output") ? Verdict.Applied(behavior["output"]?.DeepClone()) : Verdict.Applied(),
                "not_applied" => Verdict.NotApplied(),
                _ => Verdict.Unknown(),
            };
        }

        if ((bool?)behavior["echo"] == true)
        {
            return new JsonObject { ["input"] = input?.DeepClone(), ["steps"] = StepsOf(context) };
        }

        if ((bool?)behavior["echoInput"] == true)
        {
            return input?.DeepClone();
        }

        if (behavior["returnsText"] is { } length)
        {
            return JsonValue.Create(new string('x', (int)length));
        }

        return behavior["returns"]?.DeepClone();
    }

    private static ActivityFailure Failure(JsonObject fails) =>
        new((string)fails["code"]!, (string)fails["message"]!, (bool)fails["retryable"]!);

    private static JsonObject StepsOf(ActivityExecutionContext context)
    {
        var steps = new JsonObject();
        foreach (var (node, output) in context.Steps)
        {
            steps[node] = JsonNode.Parse(output.GetRawText());
        }

        return steps;
    }

    /// <summary>One entry per attempt, appended when the handler ends.</summary>
    private void Record(string activity, string kind, string outcome, JsonNode? input, ActivityExecutionContext context)
    {
        var parts = context.Traceparent.Split('-');
        var entry = new JsonObject
        {
            ["activity"] = activity,
            ["kind"] = kind,
            ["node"] = context.Node,
            ["attempt"] = context.Attempt,
            ["input"] = input?.DeepClone(),
            ["taskKind"] = context.TaskKind switch
            {
                TaskKind.Compensation => "COMPENSATION",
                TaskKind.Reconciliation => "RECONCILIATION",
                _ => "FORWARD",
            },
            ["targetNode"] = context.TargetNode,
            ["targetIdempotencyKey"] = context.TargetIdempotencyKey,
            ["idempotencyKey"] = context.IdempotencyKey,
            ["traceparentPresent"] = context.Traceparent.Length > 0,
            ["traceId"] = parts.Length > 1 ? parts[1] : string.Empty,
            ["steps"] = StepsOf(context),
            ["deadlineAhead"] = context.Deadline > DateTimeOffset.UtcNow,
            ["abortedReason"] = context.CancellationToken.IsCancellationRequested ? context.CancellationReason : null,
            ["outcome"] = outcome,
        };
        lock (gate)
        {
            log.Add(entry);
        }
    }

    internal int Count
    {
        get
        {
            lock (gate)
            {
                return log.Count;
            }
        }
    }
}
