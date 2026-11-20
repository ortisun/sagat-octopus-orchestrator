using System.Text.Json;
using SagatOctopusOrchestrator.Sdk.Conformance;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>
/// The conformance runner's <c>awaitJournal</c> reads the whole journal of a run: an event on a later page than the first is found, and each
/// page is asked for with the cursor the page before it gave.
/// </summary>
public sealed class ConformanceJournalTests
{
    private static JournalEvent Event(string json) => new(JsonDocument.Parse(json).RootElement.Clone());

    private static (Func<string?, Task<JournalPage>> Read, List<string?> Asked) TwoPages()
    {
        var asked = new List<string?>();
        Task<JournalPage> Read(string? cursor)
        {
            asked.Add(cursor);
            return Task.FromResult(cursor == "second"
                ? new JournalPage([Event("""{"sequence":2,"eventType":"step.started","stepId":"held","attempt":1}""")], null)
                : new JournalPage([Event("""{"sequence":1,"eventType":"step.leased","stepId":"held","attempt":1}""")], "second"));
        }

        return (Read, asked);
    }

    [Fact]
    public async Task AwaitJournalFindsAnEventOnTheSecondPageAskedForWithTheCursorOfTheFirst()
    {
        var (read, asked) = TwoPages();
        Assert.True(await RunMode.JournalHasAsync(read, "step.started", "held"));
        Assert.Equal([null, "second"], asked);
    }

    [Fact]
    public async Task AwaitJournalReadsEveryPageBeforeItSaysTheJournalHasNoSuchEvent()
    {
        var (read, asked) = TwoPages();
        Assert.False(await RunMode.JournalHasAsync(read, "step.completed", "held"));
        Assert.Equal([null, "second"], asked);
    }
}
