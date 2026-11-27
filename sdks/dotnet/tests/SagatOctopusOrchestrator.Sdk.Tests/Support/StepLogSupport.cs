using System.Collections.Concurrent;
using System.Diagnostics.Metrics;
using System.Reflection;
using Google.Protobuf;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Workers;

namespace SagatOctopusOrchestrator.Sdk.Tests.Support;

/// <summary>
/// The internals of step-log capture (design 041), reached by reflection: the tests compile the protocol types themselves, so they cannot be
/// given the SDK's internals. A protocol message crosses as its bytes, into the type of the side that reads it.
/// </summary>
internal static class StepLogInternals
{
    private const BindingFlags Static = BindingFlags.Static | BindingFlags.NonPublic | BindingFlags.Public;
    private const BindingFlags Instance = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public;
    private static readonly Assembly Sdk = typeof(WorkerHost).Assembly;

    internal static Type TypeOf(string name) => Sdk.GetType("SagatOctopusOrchestrator.Sdk.Workers." + name, throwOnError: true)!;

    internal static string WellFormed(string text) => (string)Call("StepLogText", "WellFormed", text)!;

    internal static (string Text, bool Cut) CutUtf8(string text, int max) => ((string, bool))Call("StepLogText", "CutUtf8", text, max)!;

    internal static string ExceptionText(Exception? exception) => (string)Call("StepLogFormat", "ExceptionText", exception)!;

    internal static StepLogRecord BuildRecord(
        string message,
        IReadOnlyList<KeyValuePair<string, object?>>? fields = null,
        string exception = "",
        Func<string, string>? redact = null,
        LogLevel level = LogLevel.Information,
        ulong sequence = 1,
        long epochMs = 1) =>
        FromSdk<StepLogRecord>(Call("StepLogFormat", "BuildRecord", sequence, epochMs, level, message, fields ?? [], exception, redact ?? (text => text))!);

    /// <summary>The fields of a call as the SDK builds them, and whether it cut them.</summary>
    internal static (List<StepLogField> Fields, bool Truncated) Fields(IReadOnlyList<KeyValuePair<string, object?>> fields, Func<string, string>? redact = null)
    {
        var record = BuildRecord(string.Empty, fields, redact: redact);
        return ([.. record.Fields], record.Truncated);
    }

    internal static int CarrySize(IEnumerable<int> recordSizes, ulong highest) => (int)Call("StepLogText", "CarrySize", recordSizes, highest)!;

    internal static int RequestSize(AppendStepLogsRequest withoutRecords, IEnumerable<int> recordSizes) =>
        (int)Call("StepLogText", "RequestSize", ToSdk(withoutRecords), recordSizes)!;

    /// <summary>A secret set of its own, not the process's.</summary>
    internal static SecretSet NewSecretSet() => new(Activator.CreateInstance(TypeOf("StepLogSecretSet"), nonPublic: true)!);

    /// <summary>Gives the process a fresh secret set until the returned scope is disposed, which gives it back its own.</summary>
    internal static IDisposable SwapProcessSecrets(SecretSet replacement)
    {
        var field = TypeOf("ProcessStepLogs").GetField("secrets", Static)!;
        var original = field.GetValue(null);
        field.SetValue(null, replacement.Target);
        return new Restore(() => field.SetValue(null, original));
    }

    /// <summary>Replaces the tick of every attempt's sender of a host by one the test drives.</summary>
    internal static ManualTicker TickByHand(WorkerHost host)
    {
        var ticker = new ManualTicker();
        // A host without it ticks nothing by hand: a test that ticks says so (Tick answers 0).
        typeof(WorkerHost).GetField("stepLogTicker", Instance)?.SetValue(host, (Func<Action, IDisposable>)ticker.Start);
        return ticker;
    }

    /// <summary>Registers the host again on the channel it holds, as the receive loop does when its session is gone.</summary>
    internal static async Task RegisterAgainAsync(WorkerHost host)
    {
        var session = typeof(WorkerHost).GetField("current", Instance)!.GetValue(host)!;
        var rpc = session.GetType().GetProperty("Rpc", Instance)!.GetValue(session)!;
        var register = typeof(WorkerHost).GetMethod("RegisterAsync", Instance)!;
        await (Task)register.Invoke(host, [rpc, CancellationToken.None])!;
    }

    /// <summary>Whether the batches of the attempt that captures now stopped at UNIMPLEMENTED.</summary>
    internal static bool BatchesStopped(WorkerHost host)
    {
        var captures = (System.Collections.IDictionary)typeof(WorkerHost).GetField("captures", Instance)!.GetValue(host)!;
        foreach (var capture in captures.Keys)
        {
            if ((bool)capture.GetType().GetProperty("BatchesStopped", Instance)!.GetValue(capture)!)
            {
                return true;
            }
        }

        return false;
    }

    internal static T FromSdk<T>(object sdkMessage)
        where T : IMessage<T>, new() => new MessageParser<T>(() => new T()).ParseFrom(((IMessage)sdkMessage).ToByteArray());

    internal static object ToSdk(IMessage message)
    {
        var type = Sdk.GetType("SagatOctopusOrchestrator.Protocol.V1." + message.Descriptor.Name, throwOnError: true)!;
        var parser = (MessageParser)type.GetProperty("Parser", Static)!.GetValue(null)!;
        return parser.ParseFrom(message.ToByteArray());
    }

    private static object? Call(string type, string method, params object?[] arguments)
    {
        try
        {
            return TypeOf(type).GetMethod(method, Static)!.Invoke(null, arguments);
        }
        catch (TargetInvocationException error) when (error.InnerException is not null)
        {
            throw error.InnerException;
        }
    }

    /// <summary>The SDK's secret set, driven through its three internal members.</summary>
    internal sealed class SecretSet(object target)
    {
        internal object Target { get; } = target;

        internal bool Full => (bool)Target.GetType().GetProperty("Full", Instance)!.GetValue(Target)!;

        internal bool Add(params byte[][] values) =>
            (bool)Target.GetType().GetMethod("Add", Instance)!.Invoke(Target, [values.Select(value => (ReadOnlyMemory<byte>)value).ToArray()])!;

        internal string Redact(string text) => (string)Target.GetType().GetMethod("Redact", Instance)!.Invoke(Target, [text])!;

        public override string ToString() => Target.ToString()!;
    }

    private sealed class Restore(Action restore) : IDisposable
    {
        public void Dispose() => restore();
    }
}

/// <summary>The tick of the second of every sender of a host, driven by the test (a race ordered by hand, never by a sleep).</summary>
internal sealed class ManualTicker
{
    private readonly ConcurrentDictionary<Registration, byte> active = new();

    internal int Active => active.Count;

    internal IDisposable Start(Action action)
    {
        var registration = new Registration(this, action);
        active.TryAdd(registration, 0);
        return registration;
    }

    /// <summary>Ticks every sender once; how many it ticked.</summary>
    internal int Tick()
    {
        var ticked = 0;
        foreach (var registration in active.Keys)
        {
            registration.Action();
            ticked++;
        }

        return ticked;
    }

    internal sealed class Registration(ManualTicker owner, Action action) : IDisposable
    {
        internal Action Action { get; } = action;

        public void Dispose() => owner.active.TryRemove(this, out _);
    }
}

/// <summary>The step-log records a worker's attempts dropped, by reason, from the SDK's meter (<c>sagat.worker.steplogs.dropped</c>).</summary>
internal sealed class StepLogDropCounter : IDisposable
{
    private readonly MeterListener listener = new();
    private readonly ConcurrentDictionary<string, long> totals = new(StringComparer.Ordinal);
    private readonly string worker;

    internal StepLogDropCounter(string worker)
    {
        this.worker = worker;
        listener.InstrumentPublished = (instrument, subscribed) =>
        {
            if (instrument.Meter.Name == SdkInfo.MeterName && instrument.Name == "sagat.worker.steplogs.dropped")
            {
                subscribed.EnableMeasurementEvents(instrument);
            }
        };
        listener.SetMeasurementEventCallback<long>(OnMeasurement);
        listener.Start();
    }

    /// <summary>The drops by reason, as <c>reason:records</c> in the order of the reasons, joined by a space.</summary>
    public override string ToString() => string.Join(' ', totals.OrderBy(pair => pair.Key, StringComparer.Ordinal).Select(pair => $"{pair.Key}:{pair.Value}"));

    public void Dispose() => listener.Dispose();

    private void OnMeasurement(System.Diagnostics.Metrics.Instrument instrument, long measurement, ReadOnlySpan<KeyValuePair<string, object?>> tags, object? state)
    {
        var ours = false;
        string? reason = null;
        foreach (var tag in tags)
        {
            if (tag.Key == "sagat.worker" && (string?)tag.Value == worker)
            {
                ours = true;
            }
            else if (tag.Key == "sagat.steplogs.reason")
            {
                reason = tag.Value as string;
            }
        }

        if (ours && reason is not null)
        {
            totals.AddOrUpdate(reason, measurement, (_, total) => total + measurement);
        }
    }
}
