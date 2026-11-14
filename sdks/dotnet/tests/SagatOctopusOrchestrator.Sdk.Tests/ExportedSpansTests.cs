using System.Diagnostics;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>The collection a test's in-memory span exporter fills from other threads while the test reads it.</summary>
public sealed class ExportedSpansTests
{
    /// <summary>
    /// Catches: an exporter target that is read in place (a <see cref="List{T}"/>): a thread that adds while the reader enumerates makes the
    /// reader throw "Collection was modified", which failed <c>TelemetryTests</c> in CI when other test classes ran in parallel.
    /// </summary>
    [Fact]
    public async Task ASpanAddedWhileAnotherThreadReadsTheCollectionNeverBreaksTheReader()
    {
        var exported = new ExportedSpans();
        using var source = new ActivitySource("Tests.ExportedSpans");
        using var listener = new ActivityListener
        {
            ShouldListenTo = candidate => candidate.Name == "Tests.ExportedSpans",
            Sample = (ref ActivityCreationOptions<ActivityContext> _) => ActivitySamplingResult.AllData,
        };
        ActivitySource.AddActivityListener(listener);
        var activity = source.StartActivity("one")!;
        var writing = true;
        var writer = Task.Run(() =>
        {
            while (Volatile.Read(ref writing))
            {
                exported.Add(activity);
            }
        });

        var reads = 0;
        var deadline = DateTime.UtcNow.AddMilliseconds(500);
        while (DateTime.UtcNow < deadline)
        {
            // enumerated in place, as Assert.Single does
            reads += exported.Count(span => span.OperationName == "one") >= 0 ? 1 : 0;
        }

        Volatile.Write(ref writing, false);
        await writer;
        Assert.True(reads > 0);
        Assert.Equal(exported.Count, exported.Snapshot().Length);
    }
}
