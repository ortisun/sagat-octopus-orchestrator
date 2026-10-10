namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>
/// How an activity fails on purpose. <see cref="Code"/> is a stable machine code (<c>[A-Z0-9_]{1,64}</c>; any other is reported as
/// <c>WORKER_ERROR</c>); the message is recorded with the step, so it must not carry a payload or a secret and is cut to 512 characters.
/// Any other exception an activity throws is reported as <c>ACTIVITY_ERROR</c> with only its type name.
/// </summary>
#pragma warning disable CA1710 // The name is the SPEC's (ActivityFailure.Retryable / Permanent / OutcomeUnknown), shared by the five SDKs.
public sealed class ActivityFailure : Exception
#pragma warning restore CA1710
{
    /// <summary>Creates a failure. Prefer <see cref="Retryable"/>, <see cref="Permanent"/> and <see cref="OutcomeUnknown"/>.</summary>
    public ActivityFailure(string code, string message, bool retryable = true)
        : base(message)
    {
        Code = code;
        IsRetryable = retryable;
    }

    /// <summary>The stable machine code of the failure.</summary>
    public string Code { get; }

    /// <summary>Whether the step may be tried again as its node's retry policy says.</summary>
    public bool IsRetryable { get; }

    /// <summary>The step may be tried again as its node's retry policy says.</summary>
    public static ActivityFailure Retryable(string code, string message) => new(code, message, retryable: true);

    /// <summary>The step is failed for good: the policy goes to its error branch or its saga compensates.</summary>
    public static ActivityFailure Permanent(string code, string message) => new(code, message, retryable: false);

    /// <summary>
    /// The activity sent a request and cannot tell whether it was applied (a timeout after connecting, an answer that never arrived).
    /// The platform then reconciles the step, when it has a <c>reconcileWith</c>, instead of retrying it blindly.
    /// </summary>
    public static ActivityFailure OutcomeUnknown(string message) => new("OUTCOME_UNKNOWN", message, retryable: false);
}
