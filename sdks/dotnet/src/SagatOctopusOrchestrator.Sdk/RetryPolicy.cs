namespace SagatOctopusOrchestrator.Sdk;

/// <summary>How many times, and how patiently, a call that is safe to repeat is repeated (SPEC section 5).</summary>
public sealed class RetryPolicy
{
    /// <summary>Attempts in all, the first included. 4 by default.</summary>
    public int MaxAttempts { get; set; } = 4;

    /// <summary>The wait after the first failed attempt, before the exponential growth. 200 ms by default.</summary>
    public TimeSpan InitialDelay { get; set; } = TimeSpan.FromMilliseconds(200);

    /// <summary>The ceiling of the backoff. 5 s by default.</summary>
    public TimeSpan MaxDelay { get; set; } = TimeSpan.FromSeconds(5);

    /// <summary>
    /// <c>min(MaxDelay, InitialDelay * 2^n)</c> scaled by a random factor in [0.5, 1.0]; <paramref name="n"/> is the attempt that
    /// just failed, counted from 0.
    /// </summary>
    public TimeSpan Backoff(int n, Func<double>? random = null)
    {
        random ??= Random.Shared.NextDouble;
        var ceiling = Math.Min(MaxDelay.TotalMilliseconds, InitialDelay.TotalMilliseconds * Math.Pow(2, Math.Min(n, 30)));
        return TimeSpan.FromMilliseconds(Math.Round(ceiling * (0.5 + (random() * 0.5))));
    }

    /// <summary>The wait before the next attempt: the backoff, or what the server asked for if that is longer.</summary>
    internal TimeSpan DelayAfter(int n, SagatException error, Func<double>? random = null)
    {
        var backoff = Backoff(n, random);
        return error.RetryAfter is { } asked && asked > backoff ? asked : backoff;
    }

    internal RetryPolicy Clone() => new() { MaxAttempts = MaxAttempts, InitialDelay = InitialDelay, MaxDelay = MaxDelay };
}
