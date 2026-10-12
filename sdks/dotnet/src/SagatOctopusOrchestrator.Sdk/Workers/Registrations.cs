using System.Text;
using System.Text.Json;
using SagatOctopusOrchestrator.Sdk.Internal;

namespace SagatOctopusOrchestrator.Sdk.Workers;

internal enum RegistrationKind
{
    Activity,
    Compensation,
    Reconciler,
}

/// <summary>What a handler returned, before it is written as the JSON of a report.</summary>
internal abstract class ActivityResult
{
    /// <summary>The JSON text of the output; null for "no value" (an empty string on the wire). Raises <c>VALIDATION_FAILED</c> when it cannot be written.</summary>
    internal abstract string? ToJson(JsonSerializerOptions options);

    internal virtual BlobDescriptor? Blob => null;
}

internal sealed class NoResult : ActivityResult
{
    internal static readonly NoResult Instance = new();

    internal override string? ToJson(JsonSerializerOptions options) => null;
}

internal sealed class ValueResult<T>(T value) : ActivityResult
{
    internal override BlobDescriptor? Blob => value is BlobOutput output ? output.Descriptor : null;

    internal override string? ToJson(JsonSerializerOptions options) => Wire.Serialize(value, options);
}

internal sealed class VerdictResult(Verdict? verdict) : ActivityResult
{
    internal Verdict? Verdict { get; } = verdict;

    internal override string? ToJson(JsonSerializerOptions options)
    {
        var answer = Verdict ?? throw new SagatException(SagatErrorCode.ValidationFailed, "a reconciler answers applied, notApplied or unknown");
        var bytes = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("effect", answer.EffectName);
            if (answer.Effect == ReconciliationEffect.Applied && answer.HasOutput)
            {
                writer.WritePropertyName("output");
                Wire.WritePayload(writer, answer.Output, options);
            }

            writer.WriteEndObject();
        });
        return Encoding.UTF8.GetString(bytes);
    }
}

/// <summary>A registered handler: how to read its input from the step document and how to call it.</summary>
internal abstract class ActivityRegistration(RegistrationKind kind)
{
    internal RegistrationKind Kind { get; } = kind;

    /// <summary>Turns the run input into the handler's type; anything that raises is reported as <c>INPUT_INVALID</c>.</summary>
    internal abstract object? ParseInput(JsonElement? input, JsonSerializerOptions options);

    internal abstract Task<ActivityResult> InvokeAsync(object? input, ActivityExecutionContext context, CancellationToken cancellationToken);

    protected static T? Read<T>(JsonElement? input, JsonSerializerOptions options)
    {
        if (typeof(T) == typeof(JsonElement))
        {
            // A JsonElement of the step document dies with the document: hand the handler its own copy.
            return (T)(object)(input?.Clone() ?? JsonDocument.Parse("null").RootElement);
        }

        return input is { } element ? JsonSerializer.Deserialize<T>(element, options) : JsonSerializer.Deserialize<T>("null", options);
    }
}

internal sealed class ValueRegistration<TInput, TOutput>(RegistrationKind kind, Func<TInput, ActivityExecutionContext, CancellationToken, Task<TOutput>> handler)
    : ActivityRegistration(kind)
{
    internal override object? ParseInput(JsonElement? input, JsonSerializerOptions options) => Read<TInput>(input, options);

    internal override async Task<ActivityResult> InvokeAsync(object? input, ActivityExecutionContext context, CancellationToken cancellationToken) =>
        new ValueResult<TOutput>(await handler((TInput)input!, context, cancellationToken).ConfigureAwait(false));
}

internal sealed class VoidRegistration<TInput>(RegistrationKind kind, Func<TInput, ActivityExecutionContext, CancellationToken, Task> handler)
    : ActivityRegistration(kind)
{
    internal override object? ParseInput(JsonElement? input, JsonSerializerOptions options) => Read<TInput>(input, options);

    internal override async Task<ActivityResult> InvokeAsync(object? input, ActivityExecutionContext context, CancellationToken cancellationToken)
    {
        await handler((TInput)input!, context, cancellationToken).ConfigureAwait(false);
        return NoResult.Instance;
    }
}

internal sealed class ReconcilerRegistration<TInput>(Func<TInput, ActivityExecutionContext, CancellationToken, Task<Verdict>> handler)
    : ActivityRegistration(RegistrationKind.Reconciler)
{
    internal override object? ParseInput(JsonElement? input, JsonSerializerOptions options) => Read<TInput>(input, options);

    internal override async Task<ActivityResult> InvokeAsync(object? input, ActivityExecutionContext context, CancellationToken cancellationToken) =>
        new VerdictResult(await handler((TInput)input!, context, cancellationToken).ConfigureAwait(false));
}
