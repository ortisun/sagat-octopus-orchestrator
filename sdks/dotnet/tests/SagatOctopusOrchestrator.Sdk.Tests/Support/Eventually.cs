namespace SagatOctopusOrchestrator.Sdk.Tests.Support;

internal static class Eventually
{
    /// <summary>Waits for a condition the code under test brings about on its own threads; fails with what it was waiting for.</summary>
    internal static async Task UntilAsync(Func<bool> condition, string what, int milliseconds = 10_000)
    {
        var end = DateTime.UtcNow.AddMilliseconds(milliseconds);
        while (!condition())
        {
            if (DateTime.UtcNow > end)
            {
                Xunit.Assert.Fail($"timed out waiting for {what}");
            }

            await Task.Delay(10);
        }
    }

    /// <summary>
    /// Waits for a condition and says whether it came, without failing: the barrier a handler holds the task on until what the test claims
    /// has happened (so the claim is not raced against a fixed sleep). The test then asserts the condition itself, with its own message.
    /// </summary>
    internal static async Task<bool> ReachedAsync(Func<bool> condition, int milliseconds = 10_000)
    {
        var end = DateTime.UtcNow.AddMilliseconds(milliseconds);
        while (!condition())
        {
            if (DateTime.UtcNow > end)
            {
                return false;
            }

            await Task.Delay(10);
        }

        return true;
    }
}
