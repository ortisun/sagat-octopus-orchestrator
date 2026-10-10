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
}
