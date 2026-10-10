namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>What a reconciler says about the effect of the step it looks up (SPEC section 6.6).</summary>
public enum ReconciliationEffect
{
    /// <summary>The effect happened.</summary>
    Applied,

    /// <summary>The effect did not happen: the platform may try it again, with the same key.</summary>
    NotApplied,

    /// <summary>Cannot tell: the decision goes to a person.</summary>
    Unknown,
}

/// <summary>The answer of a reconciler. The host completes the task with exactly <c>{"effect": …, "output": …}</c> and nothing else.</summary>
public sealed record Verdict
{
    private Verdict(ReconciliationEffect effect, object? output, bool hasOutput)
    {
        Effect = effect;
        Output = output;
        HasOutput = hasOutput;
    }

    /// <summary>What the reconciler found.</summary>
    public ReconciliationEffect Effect { get; }

    /// <summary>For <see cref="ReconciliationEffect.Applied"/>: what the step would have returned.</summary>
    public object? Output { get; }

    /// <summary>True when <see cref="Output"/> is part of the answer (it may then be null, written as JSON null).</summary>
    public bool HasOutput { get; }

    /// <summary>The effect happened; the answer has no output.</summary>
    public static Verdict Applied() => new(ReconciliationEffect.Applied, null, hasOutput: false);

    /// <summary>The effect happened; <paramref name="output"/> is what the step would have returned.</summary>
    public static Verdict Applied(object? output) => new(ReconciliationEffect.Applied, output, hasOutput: true);

    /// <summary>The effect did not happen.</summary>
    public static Verdict NotApplied() => new(ReconciliationEffect.NotApplied, null, hasOutput: false);

    /// <summary>Cannot tell.</summary>
    public static Verdict Unknown() => new(ReconciliationEffect.Unknown, null, hasOutput: false);

    internal string EffectName => Effect switch
    {
        ReconciliationEffect.Applied => "applied",
        ReconciliationEffect.NotApplied => "not_applied",
        _ => "unknown",
    };
}
