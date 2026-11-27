using System.Diagnostics;
using System.Globalization;
using System.Reflection;
using System.Security.Cryptography;
using Google.Protobuf;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using Xunit;
using Xunit.Abstractions;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>A test run by hand only: skipped unless <c>SAGAT_STEP_LOG_BENCH=1</c>, so neither the normal test command nor CI runs it.</summary>
public sealed class BenchmarkFactAttribute : FactAttribute
{
    public BenchmarkFactAttribute()
    {
        if (Environment.GetEnvironmentVariable("SAGAT_STEP_LOG_BENCH") != "1")
        {
            Skip = "run by hand: SAGAT_STEP_LOG_BENCH=1";
        }
    }
}

/// <summary>
/// The cost of one captured logger call (sdks/SPEC.md section 6.9, "The cost of a call"): an 8 KiB message redacted with a secret set of
/// 1,024 values, formatted, cut and sized. Run by hand, never in CI:
/// <c>SAGAT_STEP_LOG_BENCH=1 dotnet test -c Release --filter FullyQualifiedName~StepLogBenchmark --logger "console;verbosity=detailed"</c>.
/// 1,000 warm-up calls, then 10,000 measured on the monotonic clock; the p99 must be under 1 ms.
/// </summary>
[Collection(StepLogsRunAlone.Name)]
public sealed class StepLogBenchmark(ITestOutputHelper output)
{
    [BenchmarkFact]
    public void OneCapturedCallCostsUnderAMillisecondAtP99()
    {
        var secrets = StepLogInternals.NewSecretSet();
        secrets.Add([.. Enumerable.Range(0, 1024).Select(_ => StepLogFormatTests.Bytes(Base64Url(RandomNumberGenerator.GetBytes(12))))]);
        var redact = (Func<string, string>)Delegate.CreateDelegate(
            typeof(Func<string, string>), secrets.Target, secrets.Target.GetType().GetMethod("Redact", BindingFlags.Instance | BindingFlags.NonPublic)!);
        var build = StepLogInternals.TypeOf("StepLogFormat").GetMethod("BuildRecord", BindingFlags.Static | BindingFlags.NonPublic)!
            .CreateDelegate<Func<ulong, long, LogLevel, string, IReadOnlyList<KeyValuePair<string, object?>>, string, Func<string, string>, IMessage>>();
        var message = new string('x', (8 * 1024) - 32) + " an ordinary line of text";
        KeyValuePair<string, object?>[] fields = [new("order", "o-123"), new("amount", 1999), new("currency", "EUR")];
        int Call(ulong sequence) =>
            build(sequence, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(), LogLevel.Information, message, fields, string.Empty, redact).CalculateSize();

        for (var n = 1UL; n <= 1000; n++)
        {
            Call(n);
        }

        var samples = new double[10_000];
        for (var n = 0; n < samples.Length; n++)
        {
            var start = Stopwatch.GetTimestamp();
            Call((ulong)n + 1001);
            samples[n] = Stopwatch.GetElapsedTime(start).TotalMilliseconds;
        }

        Array.Sort(samples);
        double At(double q) => samples[Math.Min(samples.Length - 1, (int)Math.Ceiling(q * samples.Length) - 1)];
        var line = string.Format(
            CultureInfo.InvariantCulture,
            "csharp step-log call, 8 KiB message, 1,024 secrets: p50 {0:F4} ms, p99 {1:F4} ms, max {2:F4} ms",
            At(0.5),
            At(0.99),
            At(1));
        output.WriteLine(line);
        Console.WriteLine(line);
        Assert.True(At(0.99) < 1.0, line);
    }

    private static string Base64Url(byte[] bytes) => Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');
}
