using System.Collections.Concurrent;
using System.Diagnostics;
using System.Globalization;
using System.Numerics;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization.Metadata;
using Google.Protobuf;
using Grpc.Core;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Internal;

// Step-log capture (design 041, sdks/SPEC.md section 6.9): what an activity writes to its logger is numbered, redacted with the
// process's secret set, cut to the line ceilings, buffered per attempt, sent through AppendStepLogs while the attempt runs and, for its
// last records, carried on its report. Nothing here blocks a logger call on the network or throws into the activity: a failure of
// capture is counted and dropped (Requirement 041-005).
namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>The constants of SPEC section 6.9 (Requirement 041-015; never sent by the server).</summary>
internal static class StepLogLimits
{
    internal const int Minor = 7;
    internal const int AttemptRecords = 1000;
    internal const int AttemptBytes = 256 * 1024;
    internal const long ProcessBytes = 16 * 1024 * 1024;
    internal const int FlushRecords = 256;
    internal const int BatchRecords = 500;
    internal const int BatchBytes = 192 * 1024;
    internal const int CarryRecords = 64;
    internal const int CarryBytes = 16 * 1024;
    internal const int MessageBytes = 8192;
    internal const int FieldKeys = 32;
    internal const int FieldBytes = 2048;
    internal const int SecretValues = 1024;
    internal const int SecretBytes = 256 * 1024;
    internal const int SecretMinBytes = 6;

    // The longest session token: a batch's 192 KiB are counted with it, so that a resend under a new token stays within the bound.
    internal const int SessionTokenBytes = 128;
    internal const string Redacted = "[REDACTED]";

    internal static readonly TimeSpan FlushEvery = TimeSpan.FromSeconds(1);
    internal static readonly TimeSpan BatchAge = TimeSpan.FromSeconds(10);
    internal static readonly TimeSpan ReportWait = TimeSpan.FromSeconds(2);
    internal static readonly TimeSpan ShutdownFlush = TimeSpan.FromMilliseconds(1500);
    internal static readonly TimeSpan[] RetryDelays = [TimeSpan.FromMilliseconds(200), TimeSpan.FromSeconds(1)];
}

/// <summary>Why the SDK dropped a record (data-model.md section 7 of design 041): counted locally, never sent.</summary>
internal static class StepLogDrops
{
    internal const string ReportDeadline = "report_deadline";
    internal const string BufferFull = "buffer_full";
    internal const string SendFailed = "send_failed";
    internal const string AfterCeiling = "after_ceiling";
    internal const string ProcessBound = "process_bound";
    internal const string SecretSetFull = "secret_set_full";
}

// ---------------------------------------------------------------------------------------------- text and sizes

/// <summary>Text made well formed, cut at a character boundary, and ordered by its UTF-8 bytes; the protobuf sizes the buffers count.</summary>
internal static class StepLogText
{
    /// <summary>NUL and every lone surrogate become U+FFFD, so that what is sent is valid UTF-8 without a NUL (SPEC section 6.9, step 5).</summary>
    internal static string WellFormed(string text)
    {
        var i = 0;
        for (; i < text.Length; i++)
        {
            var c = text[i];
            if (c == '\0' || (char.IsHighSurrogate(c) && (i + 1 >= text.Length || !char.IsLowSurrogate(text[i + 1]))) || char.IsLowSurrogate(c))
            {
                break;
            }

            if (char.IsHighSurrogate(c))
            {
                i++;
            }
        }

        if (i == text.Length)
        {
            return text;
        }

        var builder = new StringBuilder(text.Length);
        builder.Append(text, 0, i);
        for (; i < text.Length; i++)
        {
            var c = text[i];
            if (char.IsHighSurrogate(c) && i + 1 < text.Length && char.IsLowSurrogate(text[i + 1]))
            {
                builder.Append(c).Append(text[++i]);
            }
            else if (c == '\0' || char.IsSurrogate(c))
            {
                builder.Append('\uFFFD');
            }
            else
            {
                builder.Append(c);
            }
        }

        return builder.ToString();
    }

    /// <summary>
    /// The one lenient UTF-8 decoding of the SDK (each invalid sequence replaced by U+FFFD): of a secret joining the set, of a secret's text
    /// handed to an activity and of a captured byte buffer, so that a value is found in the form it was logged in.
    /// </summary>
    internal static string Lenient(ReadOnlySpan<byte> bytes) => Encoding.UTF8.GetString(bytes);

    /// <summary>
    /// Steps 4 and 5 for one text: redacted as formatted (a value holding a NUL or a lone surrogate is still found), made well formed, and
    /// redacted again when that changed it (a value the set holds in its lenient form is found in that form too).
    /// </summary>
    internal static string Clean(string text, Func<string, string> redact)
    {
        var redacted = redact(text);
        var formed = WellFormed(redacted);
        return ReferenceEquals(formed, redacted) ? formed : redact(formed);
    }

    /// <summary>The longest prefix of a well-formed <paramref name="text"/> of at most <paramref name="max"/> UTF-8 bytes that ends at a character boundary, and whether it cut.</summary>
    internal static (string Text, bool Cut) CutUtf8(string text, int max)
    {
        if (text.Length * 3 <= max || Encoding.UTF8.GetByteCount(text) <= max)
        {
            return (text, false);
        }

        var bytes = 0;
        var end = 0;
        while (end < text.Length)
        {
            Rune.DecodeFromUtf16(text.AsSpan(end), out var rune, out var consumed);
            var size = rune.Utf8SequenceLength;
            if (bytes + size > max)
            {
                break;
            }

            bytes += size;
            end += consumed;
        }

        return (text[..end], true);
    }

    /// <summary>The order of two texts by their UTF-8 bytes (the order of their code points; a lone surrogate counts as U+FFFD, as it is encoded).</summary>
    internal static int CompareUtf8(string a, string b)
    {
        var i = 0;
        var j = 0;
        while (i < a.Length && j < b.Length)
        {
            Rune.DecodeFromUtf16(a.AsSpan(i), out var left, out var leftUnits);
            Rune.DecodeFromUtf16(b.AsSpan(j), out var right, out var rightUnits);
            if (left.Value != right.Value)
            {
                return left.Value < right.Value ? -1 : 1;
            }

            i += leftUnits;
            j += rightUnits;
        }

        return (a.Length - i).CompareTo(b.Length - j) switch
        {
            < 0 => -1,
            > 0 => 1,
            _ => 0,
        };
    }

    internal static readonly IComparer<string> Utf8Order = Comparer<string>.Create(CompareUtf8);

    /// <summary>The size of one length-delimited field of <paramref name="size"/> bytes, its tag included (every tag here is one byte).</summary>
    internal static int Delimited(int size) => 1 + CodedOutputStream.ComputeLengthSize(size) + size;

    /// <summary>The encoded size of a <c>StepLogCarry</c> holding records of these sizes.</summary>
    internal static int CarrySize(IEnumerable<int> recordSizes, ulong highest)
    {
        var size = highest == 0 ? 0 : 1 + CodedOutputStream.ComputeUInt64Size(highest);
        foreach (var record in recordSizes)
        {
            size += Delimited(record);
        }

        return size;
    }

    /// <summary>The encoded size of an <c>AppendStepLogsRequest</c> that holds records of these sizes besides what <paramref name="withoutRecords"/> holds.</summary>
    internal static int RequestSize(AppendStepLogsRequest withoutRecords, IEnumerable<int> recordSizes)
    {
        var size = withoutRecords.CalculateSize();
        foreach (var record in recordSizes)
        {
            size += Delimited(record);
        }

        return size;
    }
}

// ------------------------------------------------------------------------ the process's secret set and its redaction

/// <summary>
/// An Aho–Corasick automaton over UTF-16 code units: it tells whether any value occurs in a text in one pass, so that a text holding none
/// (the usual case) costs one scan whatever the size of the set. Immutable once built. Its memory is linear in the code units of the set
/// (one state and one edge each, in one table, whatever the length of a value), which the set bounds at 256 KiB.
/// </summary>
internal sealed class StepLogMatcher
{
    private readonly int[] root = new int[char.MaxValue + 1];
    private readonly Dictionary<long, int> edges;
    private readonly int[] fail;
    private readonly bool[] output;

    internal StepLogMatcher(IReadOnlyCollection<string> values)
    {
        var units = 1;
        foreach (var value in values)
        {
            units += value.Length;
        }

        edges = new Dictionary<long, int>(units);
        fail = new int[units];
        output = new bool[units];
        var states = 1;
        foreach (var value in values)
        {
            var state = 0;
            foreach (var unit in value)
            {
                if (!edges.TryGetValue(Edge(state, unit), out var to))
                {
                    to = states++;
                    edges[Edge(state, unit)] = to;
                    if (state == 0)
                    {
                        root[unit] = to;
                    }
                }

                state = to;
            }

            output[state] = true;
        }

        // Breadth first, so that a state's failure is known before its children's: the edges are grouped by their source state.
        var children = new List<(char Unit, int To)>[states];
        foreach (var (edge, to) in edges)
        {
            var from = (int)(edge >> 16);
            (children[from] ??= []).Add(((char)(edge & 0xFFFF), to));
        }

        var queue = new Queue<int>();
        foreach (var (_, to) in children[0] ?? [])
        {
            queue.Enqueue(to);
        }

        while (queue.Count > 0)
        {
            var state = queue.Dequeue();
            foreach (var (unit, to) in children[state] ?? [])
            {
                fail[to] = Step(fail[state], unit);
                output[to] |= output[fail[to]];
                queue.Enqueue(to);
            }
        }
    }

    internal bool OccursIn(string text)
    {
        var state = 0;
        foreach (var unit in text)
        {
            state = Step(state, unit);
            if (output[state])
            {
                return true;
            }
        }

        return false;
    }

    private static long Edge(int state, char unit) => ((long)state << 16) | unit;

    private int Step(int state, char unit)
    {
        while (state != 0)
        {
            if (edges.TryGetValue(Edge(state, unit), out var to))
            {
                return to;
            }

            state = fail[state];
        }

        return root[unit];
    }
}

/// <summary>
/// The values <c>ResolveSecrets</c> returned in the process (SPEC section 6.7): distinct values of at least 6 UTF-8 bytes, at most 1,024 of
/// them or 256 KiB. Never printed (its string and debugger forms show a count), never handed to activity code; when a value does not fit,
/// capture stops for the process. An addition builds the new automaton outside anything a logger call reads under, and publishes it with
/// its values at once, before it returns; an addition that fails commits nothing and stops capture for the process.
/// </summary>
[DebuggerDisplay("{ToString(),nq}")]
internal sealed class StepLogSecretSet
{
    private readonly object adding = new();
    private int bytes;
    private volatile bool full;

    // The values in the order they joined, the automaton and the values longest first (equal lengths in the order they joined): one immutable
    // snapshot, replaced whole by an addition.
    [DebuggerBrowsable(DebuggerBrowsableState.Never)]
    private volatile Snapshot current = Snapshot.Empty;

    internal bool Full => full;

    /// <summary>Adds what one resolution returned; true when the set stopped capture with it (once).</summary>
    internal bool Add(IEnumerable<ReadOnlyMemory<byte>> resolved)
    {
        lock (adding)
        {
            var wasFull = full;
            var held = current;
            var values = new List<string>(held.Values);
            var distinct = new HashSet<string>(held.Values, StringComparer.Ordinal);
            var size = bytes;
            var overflow = false;
            foreach (var raw in resolved)
            {
                // A value that is not UTF-8 joins as the text the accessor hands out for it (each invalid sequence replaced by U+FFFD): that is
                // the text an activity can log. The minimum and the bound count that text (SPEC section 6.7).
                var text = StepLogText.Lenient(raw.Span);
                var length = Encoding.UTF8.GetByteCount(text);
                if (length < StepLogLimits.SecretMinBytes || distinct.Contains(text))
                {
                    continue;
                }

                if (wasFull || overflow || values.Count + 1 > StepLogLimits.SecretValues || size + length > StepLogLimits.SecretBytes)
                {
                    overflow = true;
                    continue;
                }

                values.Add(text);
                distinct.Add(text);
                size += length;
            }

            if (values.Count != held.Values.Length)
            {
#pragma warning disable CA1031 // An addition that cannot be built commits nothing: capture stops for the process rather than let a value through.
                try
                {
                    current = Snapshot.Of(values);
                    bytes = size;
                }
                catch (Exception)
                {
                    overflow = true;
                }
#pragma warning restore CA1031
            }

            if (overflow)
            {
                full = true;
            }

            return overflow && !wasFull;
        }
    }

    /// <summary>The rule of <c>sagat_domain::scripts::redact</c> without its credential shapes, equal to it on every text (SPEC section 6.9).</summary>
    internal string Redact(string text)
    {
        var snapshot = current;

        // A value has at least 6 UTF-8 bytes, so at least 2 UTF-16 code units.
        if (text.Length < 2 || snapshot.Matcher is not { } matcher || !matcher.OccursIn(text))
        {
            return text;
        }

        var result = text;
        foreach (var value in snapshot.Ordered)
        {
            if (result.Contains(value, StringComparison.Ordinal))
            {
                result = result.Replace(value, StepLogLimits.Redacted, StringComparison.Ordinal);
            }
        }

        return result;
    }

    /// <summary>How many values it holds, and nothing of them.</summary>
    public override string ToString() => $"StepLogSecretSet({current.Values.Length.ToString(CultureInfo.InvariantCulture)} values)";

    private sealed class Snapshot(string[] values, StepLogMatcher? matcher, string[] ordered)
    {
        internal static Snapshot Empty { get; } = new([], null, []);

        internal string[] Values { get; } = values;

        internal StepLogMatcher? Matcher { get; } = matcher;

        internal string[] Ordered { get; } = ordered;

        internal static Snapshot Of(List<string> values) =>
            new([.. values], new StepLogMatcher(values), [.. values.OrderByDescending(value => Encoding.UTF8.GetByteCount(value))]);

        public override string ToString() => "StepLogSecretSet.Snapshot";
    }
}

/// <summary>
/// What the whole process shares: its secret set, which lives for the process (SPEC section 6.7, decision D041-164: a host started again, or
/// a second host, still redacts what an earlier attempt resolved), the bytes every attempt holds, and the warning lines already written. The
/// host only adds to the set.
/// </summary>
internal static class ProcessStepLogs
{
    // Replaced only by the tests, by reflection, to play a set that fills and give the process its own back after.
    private static StepLogSecretSet secrets = new();
    private static readonly ConcurrentDictionary<string, byte> Warned = new(StringComparer.Ordinal);
    private static long held;

    /// <summary>Adds the values one resolution returned; true when the set stopped capture for the process with them (once).</summary>
    internal static bool AddSecrets(IEnumerable<ReadOnlyMemory<byte>> values) => Volatile.Read(ref secrets).Add(values);

    /// <summary>Whether capture stopped for the process because its secret set is full.</summary>
    internal static bool CaptureStopped => Volatile.Read(ref secrets).Full;

    internal static long Held => Interlocked.Read(ref held);

    internal static void Hold(long size) => Interlocked.Add(ref held, size);

    /// <summary>True the first time a line is asked for in the process: the one local line per process (SPEC section 6.9).</summary>
    internal static bool FirstTime(string line) => Warned.TryAdd(line, 0);

    internal static string Redact(string text) => Volatile.Read(ref secrets).Redact(text);
}

// ------------------------------------------------------------------------------------------ formatting one call

/// <summary>Steps 3 to 6 of a call (SPEC section 6.9): format, redact, make well formed, cut; the fields by data-model.md section 5.</summary>
internal static class StepLogFormat
{
    private const string OriginalFormat = "{OriginalFormat}";

    // The JSON of a value that is not a scalar is written from a copy whose strings and keys were redacted first, never from the value's own
    // text (decisions D041-164 and D041-166): the copy reads the members System.Text.Json would write, through its contracts.
    private static readonly JsonSerializerOptions Contracts = new() { TypeInfoResolver = new DefaultJsonTypeInfoResolver() };

    // Deep enough to write a copy of 64 levels, its type names and the brackets around them.
    private static readonly JsonSerializerOptions Compact = new() { Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping, MaxDepth = 128 };

    // A byte or character sequence that is not a collection (a generator) is read up to this many items.
    private const int SequenceItems = 1 << 20;

    /// <summary>The five levels of a step log: <c>Information</c> is info, <c>Warning</c> warn, <c>Error</c> and <c>Critical</c> error.</summary>
    internal static StepLogLevel WireLevel(LogLevel level) => level switch
    {
        LogLevel.Trace => StepLogLevel.Trace,
        LogLevel.Debug => StepLogLevel.Debug,
        LogLevel.Information => StepLogLevel.Info,
        LogLevel.Warning => StepLogLevel.Warn,
        _ => StepLogLevel.Error,
    };

    /// <summary>A capture level the option accepts: <c>Trace</c> to <c>Critical</c>.</summary>
    internal static bool IsCaptureLevel(LogLevel level) => level is >= LogLevel.Trace and <= LogLevel.Critical;

    /// <summary>The exception text of SPEC section 6.9 for C#: the exception's <c>ToString()</c> (type, message, stack, inner exceptions).</summary>
    internal static string ExceptionText(Exception? exception)
    {
        if (exception is null)
        {
            return string.Empty;
        }

#pragma warning disable CA1031 // An exception's ToString is the application's code: whatever it raises costs its text, never the activity.
        try
        {
            return exception.ToString();
        }
        catch (Exception error) when (error is not OutOfMemoryException)
        {
            return TypeName(exception);
        }
#pragma warning restore CA1031
    }

    /// <summary>Steps 3 to 6 of a call: the record of one logger call, formatted, redacted with <paramref name="redact"/>, made well formed and cut.</summary>
    internal static StepLogRecord BuildRecord(
        ulong sequence,
        long epochMs,
        LogLevel level,
        string message,
        IReadOnlyList<KeyValuePair<string, object?>> fields,
        string exception,
        Func<string, string> redact)
    {
        var (cutMessage, messageCut) = StepLogText.CutUtf8(StepLogText.Clean(message, redact), StepLogLimits.MessageBytes);
        var (cutException, exceptionCut) = StepLogText.CutUtf8(StepLogText.Clean(exception, redact), StepLogLimits.MessageBytes);
        var (built, fieldsCut) = BuildFields(fields, redact);
        var record = new StepLogRecord
        {
            Sequence = sequence,
            WorkerTimeEpochMs = epochMs,
            Level = WireLevel(level),
            Message = cutMessage,
            Exception = cutException,
            Truncated = messageCut || exceptionCut || fieldsCut,
        };
        record.Fields.AddRange(built);
        return record;
    }

    /// <summary>
    /// The fields of a call by data-model.md section 5: taken in the order of their original keys' UTF-8 bytes, keys redacted and made unique
    /// with <c>~2</c>, <c>~3</c> and so on, string values and the text of numbers redacted, then kept in key order while they hold at most 32
    /// keys and 2,048 bytes of canonical JSON. A key <c>{OriginalFormat}</c> is the message template, not a field.
    /// </summary>
    internal static (List<StepLogField> Fields, bool Truncated) BuildFields(IReadOnlyList<KeyValuePair<string, object?>> given, Func<string, string> redact)
    {
        if (given.Count == 0)
        {
            return ([], false);
        }

        var values = new List<(string Key, StepLogField Field)>(given.Count);
        foreach (var (key, value) in given)
        {
            if (key is null || key == OriginalFormat || FieldOf(value, redact) is not { } field)
            {
                continue;
            }

            values.Add((key, field));
        }

        // The order sent: the original keys' UTF-8 bytes, the same in every language; equal keys keep the order given (a stable sort).
        var ordered = values.OrderBy(entry => entry.Key, StepLogText.Utf8Order).ToList();
        var used = new HashSet<string>(StringComparer.Ordinal);
        var made = new List<StepLogField>(ordered.Count);
        foreach (var (original, field) in ordered)
        {
            // Redacted on the text as given, then made well formed (SPEC section 6.9, steps 4 and 5).
            var redactedKey = StepLogText.Clean(original, redact);
            var key = redactedKey;
            for (var n = 2; used.Contains(key); n++)
            {
                key = redactedKey + "~" + n.ToString(CultureInfo.InvariantCulture);
            }

            used.Add(key);
            field.Key = key;
            switch (field.ValueCase)
            {
                case StepLogField.ValueOneofCase.StringValue:
                    field.StringValue = StepLogText.Clean(field.StringValue, redact);
                    break;
                case StepLogField.ValueOneofCase.IntValue:
                case StepLogField.ValueOneofCase.DoubleValue:
                    // A number whose canonical text holds a value of the set is the string [REDACTED].
                    var text = NumberText(field);
                    if (redact(text) != text)
                    {
                        field.StringValue = StepLogLimits.Redacted;
                    }

                    break;
                default:
                    break;
            }

            made.Add(field);
        }

        made.Sort((a, b) => StepLogText.CompareUtf8(a.Key, b.Key));
        var size = 2; // the braces
        for (var i = 0; i < made.Count; i++)
        {
            var entry = JsonStringSize(made[i].Key) + 1 + JsonValueSize(made[i]) + (i > 0 ? 1 : 0);
            if (i + 1 > StepLogLimits.FieldKeys || size + entry > StepLogLimits.FieldBytes)
            {
                return (made.GetRange(0, i), true);
            }

            size += entry;
        }

        return (made, false);
    }

    /// <summary>
    /// A field value as the wire carries it, its string not yet redacted (SPEC section 6.9): a string; an integer that fits 64 bits (a larger
    /// one as its decimal string); a double (not finite: <c>NaN</c>, <c>Infinity</c>, <c>-Infinity</c>); a boolean; null dropped; a byte
    /// buffer as its UTF-8 text; anything else as its compact JSON text from a redacted copy, else its string form.
    /// </summary>
    private static StepLogField? FieldOf(object? value, Func<string, string> redact)
    {
        switch (value)
        {
            case null:
                return null;
            case string text:
                return new StepLogField { StringValue = text };
            case char unit:
                return new StepLogField { StringValue = unit.ToString() };
            case bool flag:
                return new StepLogField { BoolValue = flag };
            case sbyte n:
                return new StepLogField { IntValue = n };
            case byte n:
                return new StepLogField { IntValue = n };
            case short n:
                return new StepLogField { IntValue = n };
            case ushort n:
                return new StepLogField { IntValue = n };
            case int n:
                return new StepLogField { IntValue = n };
            case uint n:
                return new StepLogField { IntValue = n };
            case long n:
                return new StepLogField { IntValue = n };
            case nint n:
                return new StepLogField { IntValue = n };
            case ulong n:
                return n <= long.MaxValue ? new StepLogField { IntValue = (long)n } : new StepLogField { StringValue = n.ToString(CultureInfo.InvariantCulture) };
            case nuint n:
                return (ulong)n <= long.MaxValue ? new StepLogField { IntValue = (long)n } : new StepLogField { StringValue = n.ToString(CultureInfo.InvariantCulture) };
            case Int128 n:
                return n >= long.MinValue && n <= long.MaxValue ? new StepLogField { IntValue = (long)n } : new StepLogField { StringValue = n.ToString(CultureInfo.InvariantCulture) };
            case UInt128 n:
                return n <= (UInt128)long.MaxValue ? new StepLogField { IntValue = (long)n } : new StepLogField { StringValue = n.ToString(CultureInfo.InvariantCulture) };
            case BigInteger n:
                return n >= long.MinValue && n <= long.MaxValue ? new StepLogField { IntValue = (long)n } : new StepLogField { StringValue = n.ToString(CultureInfo.InvariantCulture) };
            case double d:
                return DoubleField(d);
            case float f:
                return DoubleField(f);
            case Half h:
                return DoubleField((double)h);
            default:
                // A byte collection is its UTF-8 text and a character collection its text, never numbers or base64; anything else is the
                // compact JSON of its redacted copy.
                return new StepLogField { StringValue = TextOf(value) ?? new RedactedCopy(redact).Text(value) };
        }
    }

    private static StepLogField DoubleField(double value) =>
        double.IsNaN(value) ? new StepLogField { StringValue = "NaN" }
        : double.IsPositiveInfinity(value) ? new StepLogField { StringValue = "Infinity" }
        : double.IsNegativeInfinity(value) ? new StepLogField { StringValue = "-Infinity" }
        : new StepLogField { DoubleValue = value };

    private static string NumberText(StepLogField field) => field.ValueCase == StepLogField.ValueOneofCase.IntValue
        ? field.IntValue.ToString(CultureInfo.InvariantCulture)
        : CanonicalJson.EcmaScriptNumber(field.DoubleValue);

    /// <summary>The size of a text as the canonical JSON of data-model.md section 5 writes it: only the quote, the backslash and U+0000 to U+001F are escaped.</summary>
    private static int JsonStringSize(string text)
    {
        var size = 2 + Encoding.UTF8.GetByteCount(text);
        foreach (var c in text)
        {
            if (c is '"' or '\\')
            {
                size += 1;
            }
            else if (c < 0x20)
            {
                size += 5;
            }
        }

        return size;
    }

    private static int JsonValueSize(StepLogField field) => field.ValueCase switch
    {
        StepLogField.ValueOneofCase.StringValue => JsonStringSize(field.StringValue),
        StepLogField.ValueOneofCase.IntValue or StepLogField.ValueOneofCase.DoubleValue => NumberText(field).Length,
        StepLogField.ValueOneofCase.BoolValue => field.BoolValue ? 4 : 5,
        _ => 4,
    };

    /// <summary>A byte collection as its lenient UTF-8 text, a character collection as its text; null for any other value.</summary>
    private static string? TextOf(object value) => value switch
    {
        string => null,
        byte[] buffer => StepLogText.Lenient(buffer),
        ReadOnlyMemory<byte> memory => StepLogText.Lenient(memory.Span),
        Memory<byte> memory => StepLogText.Lenient(memory.Span),
        ArraySegment<byte> segment => StepLogText.Lenient(segment),
        char[] characters => new string(characters),
        ReadOnlyMemory<char> memory => new string(memory.Span),
        Memory<char> memory => new string(memory.Span),
        ArraySegment<char> segment => new string(segment),
        IEnumerable<byte> sequence => StepLogText.Lenient([.. sequence is ICollection<byte> ? sequence : sequence.Take(SequenceItems)]),
        IEnumerable<char> sequence => new string([.. sequence is ICollection<char> ? sequence : sequence.Take(SequenceItems)]),
        _ => null,
    };

    /// <summary>The name of a value's type: what stands for a value the copy cannot open, never its own text.</summary>
    private static string TypeName(object value)
    {
#pragma warning disable CA1031 // see ExceptionText
        try
        {
            return value.GetType().ToString();
        }
        catch (Exception error) when (error is not OutOfMemoryException)
        {
            return "object";
        }
#pragma warning restore CA1031
    }

    /// <summary>
    /// The copy of a value that is not a scalar, in which every string and every key is redacted, written as compact JSON (decision D041-166).
    /// It fails closed: a value it cannot open, a member that throws, a cycle and a value nested deeper than 64 stand for their type's name, and
    /// nothing uncopied is ever printed (no <c>ToString()</c>). Each object is walked once and its copy reused where it is shared; a copy that
    /// would print more than 4,096 values, counted along every path, is the type's name of the whole value.
    /// </summary>
    private sealed class RedactedCopy(Func<string, string> redact)
    {
        private const int MaxDepth = 64;
        private const int MaxNodes = 4096;
        private readonly Dictionary<object, (JsonNode? Node, int Size)> copied = new(ReferenceEqualityComparer.Instance);
        private readonly HashSet<object> open = new(ReferenceEqualityComparer.Instance);
        private int spent;

        internal string Text(object value)
        {
#pragma warning disable CA1031 // A copy over its bound, or a writer that fails, is the value's type's name.
            try
            {
                var node = Copy(value, 0).Node;

                // A value closed as a whole is its type's name, as text, not as a JSON string.
                return node is JsonValue closed && closed.TryGetValue<string>(out var name) && name == TypeName(value)
                    ? name
                    : node?.ToJsonString(Compact) ?? "null";
            }
            catch (Exception error) when (error is not OutOfMemoryException)
            {
                return TypeName(value);
            }
#pragma warning restore CA1031
        }

        private static JsonValue Closed(object value) => JsonValue.Create(TypeName(value));

        private void Spend(int nodes)
        {
            spent += nodes;
            if (spent > MaxNodes)
            {
                throw new OverBoundException();
            }
        }

        private (JsonNode? Node, int Size) Copy(object? value, int depth)
        {
            Spend(1);
            if (value is null)
            {
                return (null, 1);
            }

            if (value is string text)
            {
                return (JsonValue.Create(redact(text)), 1);
            }

            if ((value is double d && !double.IsFinite(d)) || (value is float f && !float.IsFinite(f)) || (value is Half h && !Half.IsFinite(h)))
            {
                // A number that is not finite is written null, as JavaScript's JSON.stringify writes it.
                return (null, 1);
            }

            if (Scalar(value) is { } scalar)
            {
                return (scalar, 1);
            }

            if (TextOf(value) is { } bytes)
            {
                return (JsonValue.Create(redact(bytes)), 1);
            }

            if (depth > MaxDepth)
            {
                return (Closed(value), 1);
            }

            var reference = !value.GetType().IsValueType;
            if (reference)
            {
                if (copied.TryGetValue(value, out var known))
                {
                    // Shared: its copy is reused, and counted again, since its text is printed again.
                    Spend(known.Size - 1);
                    return (known.Node?.DeepClone(), known.Size);
                }

                if (!open.Add(value))
                {
                    // A cycle: the value already being copied stands for its type's name here.
                    return (Closed(value), 1);
                }
            }

            (JsonNode? Node, int Size) made;
#pragma warning disable CA1031 // A member of the application's object that throws closes that object, never the activity.
            try
            {
                made = Open(value, depth);
            }
            catch (OverBoundException)
            {
                throw;
            }
            catch (Exception error) when (error is not OutOfMemoryException)
            {
                made = (Closed(value), 1);
            }
            finally
            {
                if (reference)
                {
                    open.Remove(value);
                }
            }
#pragma warning restore CA1031

            if (reference)
            {
                copied[value] = made;
            }

            return made;
        }

        /// <summary>A number, a boolean, a character or a plain value whose JSON System.Text.Json writes itself (a date, a GUID, an enumeration's number, a URI).</summary>
        private JsonNode? Scalar(object value) => value switch
        {
            bool flag => JsonValue.Create(flag),
            char unit => JsonValue.Create(redact(unit.ToString())),
            sbyte or byte or short or ushort or int or uint or long or ulong or nint or nuint or decimal
                or Int128 or UInt128 or BigInteger => JsonNode.Parse(Convert.ToString(value, CultureInfo.InvariantCulture)!),
            double d => JsonValue.Create(d),
            float f => JsonValue.Create(f),
            Half h => JsonValue.Create((double)h),
            Enum or DateTime or DateTimeOffset or DateOnly or TimeOnly or TimeSpan or Guid or Uri or Version =>
                Redacted(JsonSerializer.SerializeToNode(value, value.GetType(), Contracts)),
            _ => null,
        };

        private JsonNode? Redacted(JsonNode? node) =>
            node is JsonValue plain && plain.TryGetValue<string>(out var text) ? JsonValue.Create(redact(text)) : node;

        private (JsonNode? Node, int Size) Open(object value, int depth)
        {
            switch (value)
            {
                case Delegate or Type or System.Reflection.MemberInfo or System.Reflection.Assembly or Stream or Task:
                    return (Closed(value), 1);
                case System.Collections.IDictionary map:
                    {
                        var members = new JsonObject();
                        var size = 1;
                        foreach (System.Collections.DictionaryEntry entry in map)
                        {
                            var key = KeyText(entry.Key);
                            var (node, nodes) = Copy(entry.Value, depth + 1);
                            members[redact(key)] = node;
                            size += nodes;
                        }

                        return (members, size);
                    }

                case System.Collections.IEnumerable items:
                    {
                        var list = new JsonArray();
                        var size = 1;
                        foreach (var item in items)
                        {
                            var (node, nodes) = Copy(item, depth + 1);
                            list.Add(node);
                            size += nodes;
                        }

                        return (list, size);
                    }

                default:
                    {
                        var contract = Contracts.GetTypeInfo(value.GetType());
                        if (contract.Kind != JsonTypeInfoKind.Object)
                        {
                            // A type with a converter of its own, or one System.Text.Json cannot write: not opened.
                            return (Closed(value), 1);
                        }

                        var members = new JsonObject();
                        var size = 1;
                        foreach (var property in contract.Properties)
                        {
                            if (property.Get is null)
                            {
                                continue;
                            }

                            var (node, nodes) = Copy(property.Get(value), depth + 1);
                            members[redact(property.Name)] = node;
                            size += nodes;
                        }

                        return (members, size);
                    }
            }
        }

        /// <summary>The text of a dictionary key: a string, or a plain value's invariant text; any other key closes the dictionary.</summary>
        private static string KeyText(object key) => key switch
        {
            string text => text,
            bool or char or sbyte or byte or short or ushort or int or uint or long or ulong or decimal or Enum or Guid =>
                Convert.ToString(key, CultureInfo.InvariantCulture)!,
            _ => throw new NotSupportedException("a key that is not a plain value"),
        };
    }

    private sealed class OverBoundException : Exception
    {
    }
}

// ----------------------------------------------------------------------------------------------- one attempt

/// <summary>The capture state a registration gives a session: shared by its attempts, stopped by <c>STOP_CAPTURE</c> until the next registration.</summary>
internal sealed class CaptureSession(bool enabled)
{
    private volatile bool stopped;

    internal static CaptureSession Off { get; } = new(false);

    internal bool Enabled { get; } = enabled;

    internal bool Stopped
    {
        get => stopped;
        set => stopped = value;
    }
}

/// <summary>The answer of one <c>AppendStepLogs</c>, or the status of its failure, or the host's own abandonment of the call.</summary>
internal sealed record AppendOutcome(bool Answered, StepLogVerdict Verdict, StepLogStopReason StopReason, int RetryAfterMs, StatusCode Status, int? TrailerRetryAfterMs, bool Abandoned)
{
    /// <summary>The host gave the call up because it is stopping (or the attempt ended): dropped at once, never retried.</summary>
    internal static AppendOutcome HostAbandoned { get; } = new(false, StepLogVerdict.Unspecified, StepLogStopReason.Unspecified, 0, StatusCode.Cancelled, null, true);

    internal static AppendOutcome Of(AppendStepLogsResponse answer) =>
        new(true, answer.Verdict, answer.StopReason, answer.RetryAfterMs, StatusCode.OK, null, false);

    internal static AppendOutcome Failed(StatusCode status, int? retryAfterMs) =>
        new(false, StepLogVerdict.Unspecified, StepLogStopReason.Unspecified, 0, status, retryAfterMs, false);
}

/// <summary>What an attempt needs of its host.</summary>
internal sealed class CaptureHost
{
    /// <summary>The session id and token the host holds now.</summary>
    internal required Func<(string WorkerId, string Token)> Session { get; init; }

    /// <summary>One <c>AppendStepLogs</c> with its deadline and token; never throws.</summary>
    internal required Func<AppendStepLogsRequest, TimeSpan, CancellationToken, Task<AppendOutcome>> Append { get; init; }

    /// <summary>The life of the channel: a shutdown batch is sent as long as it lives.</summary>
    internal required CancellationToken Life { get; init; }

    /// <summary>The configured call timeout.</summary>
    internal required TimeSpan Timeout { get; init; }

    /// <summary>Counted drops, once per attempt and reason.</summary>
    internal required Action<string, int> Dropped { get; init; }

    /// <summary>The one local line per process when the server stops capture.</summary>
    internal required Action<string> WarnOnce { get; init; }

    /// <summary>Calls the action every second until disposed (the tests tick by hand).</summary>
    internal required Func<Action, IDisposable> Ticker { get; init; }

    internal required TimeProvider Time { get; init; }
}

/// <summary>The capture of one attempt: its sequences, its buffer, its sender and, at its end, its carry.</summary>
#pragma warning disable CA1001 // The token source is cancelled and disposed when the attempt closes, which every attempt does.
internal sealed class AttemptCapture
#pragma warning restore CA1001
{
    // The one local line per process when the platform stops capture, whatever answer stopped it (SPEC section 6.9).
    internal const string StoppedLine = "the platform stopped step-log capture for this session";

    private readonly object gate = new();
    private readonly CaptureHost host;
    private readonly CaptureSession session;
    private readonly AppendStepLogsRequest identity;
    private readonly LogLevel level;
    private readonly CancellationTokenSource sends;
    private readonly CancellationToken sendsToken;
    private readonly Dictionary<string, int> drops = new(StringComparer.Ordinal);
    private List<Held> buffer = [];
    private volatile bool ended;
    private State state;
    private ulong last;
    private int bufferBytes;
    private Batch? inFlight;
    private bool batchesStopped;
    private bool closed;
    private long endedAt;
    private IDisposable? tick;

    internal AttemptCapture(CaptureHost host, CaptureSession session, AppendStepLogsRequest identity, LogLevel level)
    {
        this.host = host;
        this.session = session;
        this.identity = identity;
        this.level = level;
        sends = CancellationTokenSource.CreateLinkedTokenSource(host.Life);
        sendsToken = sends.Token;
    }

    private enum State
    {
        Capturing,
        FinalOnly,
        Stopped,
    }

    /// <summary>Whether its batches stopped at <c>UNIMPLEMENTED</c> (for the tests).</summary>
    internal bool BatchesStopped
    {
        get
        {
            lock (gate)
            {
                return batchesStopped;
            }
        }
    }

    /// <summary>A call at this level is captured: the handler has not ended and the level is at or above the capture level.</summary>
    internal bool Accepts(LogLevel callLevel) => !ended && callLevel != LogLevel.None && callLevel >= level;

    /// <summary>
    /// A logger call (SPEC section 6.9, "A call, in order"). <paramref name="collect"/> gives the formatted message, the fields and the
    /// exception text; it runs only for a record that is kept, outside the attempt's lock. Never throws and never waits for the network.
    /// </summary>
    internal void Log(LogLevel callLevel, Func<(string Message, IReadOnlyList<KeyValuePair<string, object?>> Fields, string Exception)> collect)
    {
#pragma warning disable CA1031 // A bug of the capture path costs the record, never the activity.
        try
        {
            if (!Accepts(callLevel) || !Take(out var sequence, out var epochMs))
            {
                return;
            }

            var (message, fields, exception) = collect();
            var record = StepLogFormat.BuildRecord(sequence, epochMs, callLevel, message, fields, exception, ProcessStepLogs.Redact);
            Buffered(record, record.CalculateSize());

            // A record let go because the attempt closed while it was formatted is counted on its own.
            EmitLate();
        }
        catch (Exception error) when (error is not OutOfMemoryException)
        {
        }
#pragma warning restore CA1031
    }

    /// <summary>Steps 1 and 2: the sequence, the worker's time and the early drops; false when the call takes none or is dropped.</summary>
    private bool Take(out ulong sequence, out long epochMs)
    {
        sequence = 0;
        epochMs = 0;
        lock (gate)
        {
            if (ended || closed || StoppedLocked())
            {
                return false;
            }

            sequence = ++last;
            epochMs = host.Time.GetUtcNow().ToUnixTimeMilliseconds();
            var reason = state == State.FinalOnly ? StepLogDrops.AfterCeiling
                : ProcessStepLogs.CaptureStopped ? StepLogDrops.SecretSetFull
                : buffer.Count >= StepLogLimits.AttemptRecords || bufferBytes >= StepLogLimits.AttemptBytes ? StepLogDrops.BufferFull
                : ProcessStepLogs.Held >= StepLogLimits.ProcessBytes ? StepLogDrops.ProcessBound
                : null;
            if (reason is null)
            {
                return true;
            }

            Count(reason, 1);
            return false;
        }
    }

    /// <summary>
    /// Step 7: the record joins the buffer in the order of its sequence, and the call that buffers the 256th record cuts the batch. After
    /// <c>UNIMPLEMENTED</c> the buffer keeps only the newest records that fit one carry.
    /// </summary>
    private void Buffered(StepLogRecord record, int size)
    {
        lock (gate)
        {
            if (closed || state != State.Capturing || StoppedLocked())
            {
                // The state changed while the record was formatted: it is let go as the buffer was.
                Count(state == State.FinalOnly ? StepLogDrops.AfterCeiling : StepLogDrops.SendFailed, 1);
                return;
            }

            // The bounds again where the record joins: concurrent calls that all passed step 2 pass a bound by one record at most.
            if (buffer.Count >= StepLogLimits.AttemptRecords || bufferBytes >= StepLogLimits.AttemptBytes)
            {
                Count(StepLogDrops.BufferFull, 1);
                return;
            }

            if (ProcessStepLogs.Held >= StepLogLimits.ProcessBytes)
            {
                Count(StepLogDrops.ProcessBound, 1);
                return;
            }

            var at = buffer.Count;
            while (at > 0 && buffer[at - 1].Record.Sequence > record.Sequence)
            {
                at--;
            }

            buffer.Insert(at, new Held(record, size));
            bufferBytes += size;
            ProcessStepLogs.Hold(size);
            if (batchesStopped)
            {
                KeepOneCarryLocked();
                return;
            }

            if (tick is null && !ended)
            {
                tick = host.Ticker(OnTick);
            }

            if (buffer.Count >= StepLogLimits.FlushRecords && inFlight is null && !SetFullLocked())
            {
                SendLocked(Cut(buffer.Count));
            }
        }
    }

    private void OnTick()
    {
#pragma warning disable CA1031 // The next tick tries again.
        try
        {
            lock (gate)
            {
                if (closed || ended || inFlight is not null || buffer.Count == 0 || state != State.Capturing || StoppedLocked() || batchesStopped || SetFullLocked())
                {
                    return;
                }

                SendLocked(Cut(buffer.Count));
            }
        }
        catch (Exception error) when (error is not OutOfMemoryException)
        {
        }
#pragma warning restore CA1031
    }

    /// <summary>The request of a batch: the attempt as <c>Heartbeat</c> names it, the session, and the records.</summary>
    private AppendStepLogsRequest Request(IEnumerable<StepLogRecord> records, string workerId, string token)
    {
        var request = identity.Clone();
        request.WorkerId = workerId;
        request.SessionToken = token;
        request.Records.AddRange(records);
        return request;
    }

    /// <summary>Takes the oldest of the first <paramref name="limit"/> buffered records that one batch holds: at most 500, at most 192 KiB encoded, counted with a 128-byte token.</summary>
    private List<Held> Cut(int limit)
    {
        var (workerId, token) = host.Session();
        if (Encoding.UTF8.GetByteCount(token) < StepLogLimits.SessionTokenBytes)
        {
            token = new string('t', StepLogLimits.SessionTokenBytes);
        }

        var size = Request([], workerId, token).CalculateSize();
        var count = 0;
        while (count < Math.Min(limit, StepLogLimits.BatchRecords))
        {
            var next = StepLogText.Delimited(buffer[count].Size);
            if (count > 0 && size + next > StepLogLimits.BatchBytes)
            {
                break;
            }

            size += next;
            count++;
        }

        var held = buffer.GetRange(0, count);
        buffer.RemoveRange(0, count);
        foreach (var one in held)
        {
            bufferBytes -= one.Size;
        }

        return held;
    }

    private static void Release(IEnumerable<Held> held)
    {
        long total = 0;
        foreach (var one in held)
        {
            total += one.Size;
        }

        ProcessStepLogs.Hold(-total);
    }

    /// <summary>Starts a batch; the caller holds the lock.</summary>
    private void SendLocked(List<Held> held)
    {
        if (held.Count == 0)
        {
            return;
        }

        var batch = new Batch(held, host.Time.GetTimestamp(), once: false);
        inFlight = batch;
        _ = Task.Run(() => DeliverAsync(batch, host.Timeout));
    }

    /// <summary>A batch is let go: its bytes are released, and the answer cuts the next batch when 256 or more are buffered.</summary>
    private void Finish(Batch batch)
    {
        lock (gate)
        {
            Release(batch.Held);
            if (ReferenceEquals(inFlight, batch))
            {
                inFlight = null;
            }

            if (!closed && !ended && state == State.Capturing && !StoppedLocked() && !batchesStopped
                && buffer.Count >= StepLogLimits.FlushRecords && inFlight is null && !SetFullLocked())
            {
                SendLocked(Cut(buffer.Count));
            }
        }

        batch.Done.TrySetResult();
    }

    /// <summary>Whether the batch is sent (again): not once it was let go, its attempt closed (a shutdown batch excepted), its attempt or its session stopped capture, or the process's capture stopped.</summary>
    private bool MayResend(Batch batch)
    {
        lock (gate)
        {
            if (batch.Settled)
            {
                return false;
            }

            if (ProcessStepLogs.CaptureStopped)
            {
                LetGoLocked(batch, StepLogDrops.SecretSetFull);
                return false;
            }

            if ((closed && !batch.Once) || StoppedLocked())
            {
                LetGoLocked(batch, StepLogDrops.SendFailed);
                return false;
            }

            return true;
        }
    }

    /// <summary>Sends one batch until an answer settles it (contracts.md section 4). A shutdown batch is sent a single time.</summary>
    private async Task DeliverAsync(Batch batch, TimeSpan within)
    {
#pragma warning disable CA1031 // A failure of the sender costs the batch, counted, never the host.
        try
        {
            var records = batch.Held.Select(one => one.Record).ToList();
            var (workerId, token) = host.Session();
            var request = Request(records, workerId, token);
            var retries = 0;
            var sessionResent = false;
            while (true)
            {
                if (!MayResend(batch))
                {
                    return;
                }

                var left = StepLogLimits.BatchAge - host.Time.GetElapsedTime(batch.FirstSent);
                if (left <= TimeSpan.Zero)
                {
                    LetGo(batch, StepLogDrops.SendFailed);
                    return;
                }

                // The shutdown batch lives as long as the channel, not as its attempt, which the shutdown is ending.
                var outcome = await host.Append(request, within < left ? within : left, batch.Once ? host.Life : sendsToken).ConfigureAwait(false);
                if (batch.Once)
                {
                    // Sent once, with no resend: a batch the answer did not take is counted as not sent.
                    if (outcome is { Answered: true, Verdict: StepLogVerdict.Continue })
                    {
                        Settle(batch);
                    }
                    else
                    {
                        LetGo(batch, StepLogDrops.SendFailed);
                    }

                    return;
                }

                var wait = TimeSpan.Zero;
                if (outcome.Answered)
                {
                    switch (outcome.Verdict)
                    {
                        case StepLogVerdict.Continue:
                            Settle(batch);
                            return;
                        case StepLogVerdict.FinalOnly:
                            if (Settle(batch))
                            {
                                FinalOnly();
                            }

                            return;
                        case StepLogVerdict.StopAttempt:
                            if (Settle(batch))
                            {
                                Stop();
                            }

                            return;
                        case StepLogVerdict.StopCapture:
                            var now = host.Session();
                            if (outcome.StopReason == StepLogStopReason.Session && !sessionResent && now.Token != token)
                            {
                                // The session the batch sent is not the one the host holds now: once more, with the new one.
                                sessionResent = true;
                                token = now.Token;
                                request = Request(records, now.WorkerId, now.Token);
                                continue;
                            }

                            if (Settle(batch))
                            {
                                StopSession();
                            }

                            return;
                        case StepLogVerdict.RetryLater:
                            wait = Clamp(TimeSpan.FromMilliseconds(outcome.RetryAfterMs));
                            break;
                        default:
                            // An answer this SDK does not know is a failure of the call.
                            break;
                    }
                }
                else if (outcome.Abandoned)
                {
                    LetGo(batch, StepLogDrops.SendFailed);
                    return;
                }
                else
                {
                    switch (outcome.Status)
                    {
                        case StatusCode.ResourceExhausted:
                            wait = Clamp(TimeSpan.FromMilliseconds(outcome.TrailerRetryAfterMs ?? 200));
                            break;
                        case StatusCode.InvalidArgument:
                            LetGo(batch, StepLogDrops.SendFailed);
                            return;
                        case StatusCode.Unauthenticated:
                        case StatusCode.PermissionDenied:
                            // A batch the server refused is one the attempt gave up on; a late answer to a batch given up changes nothing.
                            if (LetGo(batch, StepLogDrops.SendFailed))
                            {
                                StopSession();
                            }

                            return;
                        case StatusCode.Unimplemented:
                            // An older API replica: no more batches, and from now on only the newest records that fit one carry are kept.
                            lock (gate)
                            {
                                if (!batch.Settled)
                                {
                                    LetGoLocked(batch, StepLogDrops.SendFailed);
                                    batchesStopped = true;
                                    KeepOneCarryLocked();
                                }
                            }

                            return;
                        default:
                            break;
                    }
                }

                if (wait > TimeSpan.Zero)
                {
                    // RETRY_LATER and RESOURCE_EXHAUSTED: the same batch again, not counted as a retry.
                    if (!await PauseAsync(wait).ConfigureAwait(false))
                    {
                        LetGo(batch, StepLogDrops.SendFailed);
                        return;
                    }

                    continue;
                }

                // UNAVAILABLE, DEADLINE_EXCEEDED, a CANCELLED the server sent, a transport failure, any other status or verdict: two retries.
                if (retries >= StepLogLimits.RetryDelays.Length)
                {
                    LetGo(batch, StepLogDrops.SendFailed);
                    return;
                }

                if (!await PauseAsync(Jittered(StepLogLimits.RetryDelays[retries])).ConfigureAwait(false))
                {
                    LetGo(batch, StepLogDrops.SendFailed);
                    return;
                }

                retries++;
            }
        }
        catch (Exception error) when (error is not OutOfMemoryException)
        {
            LetGo(batch, StepLogDrops.SendFailed);
        }
        finally
        {
            Finish(batch);
            EmitLate();
        }
#pragma warning restore CA1031
    }

    private async Task<bool> PauseAsync(TimeSpan wait)
    {
        try
        {
            await Task.Delay(wait, host.Time, sendsToken).ConfigureAwait(false);
            return true;
        }
        catch (OperationCanceledException)
        {
            return false;
        }
    }

    private static TimeSpan Clamp(TimeSpan wait) =>
        wait < TimeSpan.FromMilliseconds(50) ? TimeSpan.FromMilliseconds(50) : wait > TimeSpan.FromSeconds(5) ? TimeSpan.FromSeconds(5) : wait;

    private static TimeSpan Jittered(TimeSpan wait) => TimeSpan.FromMilliseconds(wait.TotalMilliseconds * (0.8 + (Random.Shared.NextDouble() * 0.4)));

    private void Count(string reason, int records)
    {
        if (records > 0)
        {
            drops[reason] = drops.GetValueOrDefault(reason) + records;
        }
    }

    /// <summary>Counts a batch's records once, for the reason it was given up; false when its fate was decided before.</summary>
    private bool LetGoLocked(Batch batch, string reason)
    {
        if (batch.Settled)
        {
            return false;
        }

        batch.Settled = true;
        Count(reason, batch.Held.Count);
        return true;
    }

    private bool LetGo(Batch batch, string reason)
    {
        lock (gate)
        {
            return LetGoLocked(batch, reason);
        }
    }

    /// <summary>Marks a batch answered; false when it was let go meanwhile (its answer is ignored).</summary>
    private bool Settle(Batch batch)
    {
        lock (gate)
        {
            if (batch.Settled)
            {
                return false;
            }

            batch.Settled = true;
            return true;
        }
    }

    private bool StoppedLocked() => state == State.Stopped || session.Stopped;

    /// <summary>Whether the process's capture stopped (the secret set is full): then nothing is sent at all, and what the attempt buffered is dropped.</summary>
    private bool SetFullLocked()
    {
        if (!ProcessStepLogs.CaptureStopped)
        {
            return false;
        }

        DropBufferLocked(StepLogDrops.SecretSetFull);
        return true;
    }

    private void DropBufferLocked(string reason)
    {
        Count(reason, buffer.Count);
        Release(buffer);
        buffer = [];
        bufferBytes = 0;
    }

    private void FinalOnly()
    {
        lock (gate)
        {
            if (state == State.Capturing)
            {
                state = State.FinalOnly;
                DropBufferLocked(StepLogDrops.AfterCeiling);
            }
        }
    }

    private void Stop()
    {
        lock (gate)
        {
            state = State.Stopped;

            // Records the server will not take: counted as records whose sending failed.
            DropBufferLocked(StepLogDrops.SendFailed);
        }
    }

    private void StopSession()
    {
        session.Stopped = true;
        host.WarnOnce(StoppedLine);
        Stop();
    }

    /// <summary>Drops the oldest buffered records that one carry will not hold (<c>report_deadline</c>).</summary>
    private void KeepOneCarryLocked()
    {
        var beyond = buffer.Count - CarryFitsLocked();
        if (beyond <= 0)
        {
            return;
        }

        var dropped = buffer.GetRange(0, beyond);
        buffer.RemoveRange(0, beyond);
        foreach (var one in dropped)
        {
            bufferBytes -= one.Size;
        }

        Release(dropped);
        Count(StepLogDrops.ReportDeadline, beyond);
    }

    /// <summary>How many of the newest buffered records fit one carry (64 records and 16 KiB encoded).</summary>
    private int CarryFitsLocked()
    {
        var size = 1 + CodedOutputStream.ComputeUInt64Size(last);
        var count = 0;
        for (var i = buffer.Count - 1; i >= 0 && count < StepLogLimits.CarryRecords; i--)
        {
            var next = StepLogText.Delimited(buffer[i].Size);
            if (size + next > StepLogLimits.CarryBytes)
            {
                break;
            }

            size += next;
            count++;
        }

        return count;
    }

    /// <summary>The handler ended: its logger captures nothing more.</summary>
    internal void End()
    {
        lock (gate)
        {
            EndLocked();
        }
    }

    private void EndLocked()
    {
        if (!ended)
        {
            ended = true;
            endedAt = host.Time.GetTimestamp();
        }

        tick?.Dispose();
        tick = null;
    }

    /// <summary>
    /// The carry of the attempt's report (SPEC section 6.9, "The carry"), after at most 2 seconds from the handler's end of waiting for its
    /// batch in flight and sending what does not fit one carry; null when the attempt sends none. It closes the attempt and never throws.
    /// </summary>
    internal async Task<StepLogCarry?> CarryAsync()
    {
        End();
#pragma warning disable CA1031 // A failure of capture costs the carry, never the report.
        try
        {
            return await CarryCoreAsync().ConfigureAwait(false);
        }
        catch (Exception error) when (error is not OutOfMemoryException)
        {
            return null;
        }
        finally
        {
            Close();
        }
#pragma warning restore CA1031
    }

    private async Task<StepLogCarry?> CarryCoreAsync()
    {
        long deadline;
        lock (gate)
        {
            if (closed || last == 0 || StoppedLocked())
            {
                return null;
            }

            if (state == State.FinalOnly || ProcessStepLogs.CaptureStopped)
            {
                return MarkOnlyLocked();
            }

            deadline = endedAt + (long)(StepLogLimits.ReportWait.TotalSeconds * host.Time.TimestampFrequency);
        }

        while (true)
        {
            Batch? waitFor = null;
            lock (gate)
            {
                if (inFlight is { } batch)
                {
                    waitFor = batch;
                }
                else
                {
                    var fits = CarryFitsLocked();
                    if (fits == buffer.Count || batchesStopped || host.Time.GetTimestamp() >= deadline)
                    {
                        break;
                    }

                    if (ProcessStepLogs.CaptureStopped)
                    {
                        return MarkOnlyLocked();
                    }

                    SendLocked(Cut(buffer.Count - fits));
                    continue;
                }
            }

            var left = TimeSpan.FromSeconds((double)(deadline - host.Time.GetTimestamp()) / host.Time.TimestampFrequency);
            if (left > TimeSpan.Zero)
            {
                using var timer = new CancellationTokenSource();
                var delay = Task.Delay(left, host.Time, timer.Token);
                await Task.WhenAny(waitFor.Done.Task, delay).ConfigureAwait(false);
                await timer.CancelAsync().ConfigureAwait(false);
            }

            lock (gate)
            {
                if (closed)
                {
                    // The attempt was closed while its report waited (the host stopped): it carries nothing.
                    return null;
                }

                if (ReferenceEquals(inFlight, waitFor))
                {
                    // The 2 seconds ended with the batch on its way: it is given up, its answer ignored.
                    LetGoLocked(waitFor, StepLogDrops.ReportDeadline);
                    break;
                }

                if (StoppedLocked())
                {
                    return null;
                }

                // The answer may have changed the state while the report waited.
                if (state == State.FinalOnly || ProcessStepLogs.CaptureStopped)
                {
                    return MarkOnlyLocked();
                }
            }
        }

        lock (gate)
        {
            if (closed)
            {
                return null;
            }

            KeepOneCarryLocked();
            var carry = new StepLogCarry { HighestSequence = last };
            carry.Records.AddRange(buffer.Select(one => one.Record));

            // The carried records leave the buffer: what Close finds there is what an attempt without a report let go.
            Release(buffer);
            buffer = [];
            bufferBytes = 0;
            return carry;
        }
    }

    private StepLogCarry MarkOnlyLocked()
    {
        DropBufferLocked(state == State.FinalOnly ? StepLogDrops.AfterCeiling : StepLogDrops.SecretSetFull);
        return new StepLogCarry { HighestSequence = last };
    }

    /// <summary>
    /// The attempt is over (its report was built, or it ends without one): nothing more is sent but a shutdown batch, what it still holds is let
    /// go (<c>send_failed</c>), and its drops are counted once.
    /// </summary>
    internal void Close()
    {
        Dictionary<string, int> counted;
        lock (gate)
        {
            if (closed)
            {
                return;
            }

            closed = true;
            EndLocked();
            if (inFlight is { Once: false } batch)
            {
                LetGoLocked(batch, StepLogDrops.SendFailed);
            }

            DropBufferLocked(StepLogDrops.SendFailed);
            counted = new Dictionary<string, int>(drops, StringComparer.Ordinal);
            drops.Clear();
        }

        // Outside the lock: a cancelled wait may resume its sender on this thread.
        Safe.Run(sends.Cancel);
        sends.Dispose();
        Emit(counted);
    }

    /// <summary>Emits what was counted after the attempt closed (a shutdown batch's fate, a late let-go).</summary>
    private void EmitLate()
    {
        Dictionary<string, int> counted;
        lock (gate)
        {
            if (!closed || drops.Count == 0)
            {
                return;
            }

            counted = new Dictionary<string, int>(drops, StringComparer.Ordinal);
            drops.Clear();
        }

        Emit(counted);
    }

    private void Emit(Dictionary<string, int> counted)
    {
        foreach (var (reason, records) in counted.OrderBy(pair => pair.Key, StringComparer.Ordinal))
        {
            host.Dropped(reason, records);
        }
    }

    /// <summary>
    /// The host stops with this attempt's handler still running and no batch in flight: its oldest buffered records go once, as its batch in
    /// flight with no resend; what does not fit stays for its report. An attempt whose handler has ended is left to its report (SPEC section 6.9).
    /// </summary>
    internal async Task FlushOnShutdownAsync(TimeSpan within)
    {
        Batch? batch;
        lock (gate)
        {
            if (closed || ended || StoppedLocked() || state != State.Capturing || batchesStopped || inFlight is not null || buffer.Count == 0 || SetFullLocked())
            {
                return;
            }

            batch = new Batch(Cut(buffer.Count), host.Time.GetTimestamp(), once: true);
            inFlight = batch;
        }

        _ = Task.Run(() => DeliverAsync(batch, within));
        using var timer = new CancellationTokenSource();
        await Task.WhenAny(batch.Done.Task, Task.Delay(within, host.Time, timer.Token)).ConfigureAwait(false);
        await timer.CancelAsync().ConfigureAwait(false);
    }

    private readonly record struct Held(StepLogRecord Record, int Size);

    /// <summary>A batch on its way. <see cref="Settled"/> says that its fate is decided (answered, or let go and counted), guarded by the attempt's lock.</summary>
    private sealed class Batch(List<Held> held, long firstSent, bool once)
    {
        internal List<Held> Held { get; } = held;

        internal long FirstSent { get; } = firstSent;

        internal bool Once { get; } = once;

        internal bool Settled { get; set; }

        internal TaskCompletionSource Done { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
    }
}

// ----------------------------------------------------------------------------------------------- the logger

/// <summary>
/// The per-attempt decorator of the activity's logger, inside its <see cref="SafeLogger"/> (SPEC section 6.9): it captures each call at or
/// above the capture level of a capturing attempt and forwards it to the host's logger only when that one is enabled, so the host logs exactly
/// what it logged before. Scopes begun on it are kept per attempt in an <see cref="AsyncLocal{T}"/> stack and captured as fields.
/// </summary>
internal sealed class StepLogLogger(ILogger inner, AttemptCapture capture) : ILogger
{
    private readonly AsyncLocal<Scope?> scopes = new();

    public IDisposable? BeginScope<TState>(TState state)
        where TState : notnull
    {
        IDisposable? hosted = null;
        Safe.Run(() => hosted = inner.BeginScope(state));
        var scope = new Scope(this, state, scopes.Value, hosted);
        scopes.Value = scope;
        return scope;
    }

    public bool IsEnabled(LogLevel logLevel) => capture.Accepts(logLevel) || InnerEnabled(logLevel);

    public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
    {
        if (capture.Accepts(logLevel))
        {
            var current = scopes.Value;
            capture.Log(logLevel, () => Collect(state, exception, formatter, current));
        }

        if (InnerEnabled(logLevel))
        {
            Safe.Run(() => inner.Log(logLevel, eventId, state, exception, formatter));
        }
    }

    private bool InnerEnabled(LogLevel logLevel)
    {
        var enabled = false;
        Safe.Run(() => enabled = inner.IsEnabled(logLevel));
        return enabled;
    }

    /// <summary>The message as the formatter gives it, the fields of the scopes (the outermost first) and of the state, and the exception text.</summary>
    private static (string, IReadOnlyList<KeyValuePair<string, object?>>, string) Collect<TState>(
        TState state, Exception? exception, Func<TState, Exception?, string>? formatter, Scope? current)
    {
        var message = string.Empty;
        Safe.Run(() => message = (formatter is not null ? formatter(state, exception) : state?.ToString()) ?? string.Empty);
        var fields = new List<KeyValuePair<string, object?>>();
        var chain = new Stack<object>();
        for (var scope = current; scope is not null; scope = scope.Parent)
        {
            chain.Push(scope.State);
        }

        foreach (var scoped in chain)
        {
            AddFields(scoped, fields);
        }

        AddFields(state, fields);
        return (message, fields, StepLogFormat.ExceptionText(exception));
    }

    private static void AddFields(object? state, List<KeyValuePair<string, object?>> fields)
    {
        if (state is not IEnumerable<KeyValuePair<string, object?>> pairs)
        {
            return;
        }

        // A state whose enumeration throws keeps the fields read before it.
        Safe.Run(() =>
        {
            foreach (var pair in pairs)
            {
                fields.Add(pair);
            }
        });
    }

    private sealed class Scope(StepLogLogger owner, object state, Scope? parent, IDisposable? hosted) : IDisposable
    {
        private int disposed;

        internal object State { get; } = state;

        internal Scope? Parent { get; } = parent;

        public void Dispose()
        {
            if (Interlocked.Exchange(ref disposed, 1) != 0)
            {
                return;
            }

            if (ReferenceEquals(owner.scopes.Value, this))
            {
                owner.scopes.Value = Parent;
            }

            if (hosted is not null)
            {
                Safe.Run(hosted.Dispose);
            }
        }
    }
}

/// <summary>The lines the SDK writes about capture, at the warning level, once per process each.</summary>
internal static partial class StepLogMessages
{
    [LoggerMessage(EventId = 40, Level = LogLevel.Warning, Message = "The platform stopped step-log capture for this session")]
    internal static partial void CaptureStopped(ILogger logger);

    [LoggerMessage(EventId = 41, Level = LogLevel.Warning, Message = "The secret set of the process is full: step-log capture stops for the process")]
    internal static partial void SecretSetFull(ILogger logger);
}
