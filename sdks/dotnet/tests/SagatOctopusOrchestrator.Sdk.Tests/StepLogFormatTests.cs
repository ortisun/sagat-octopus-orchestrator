using System.Diagnostics;
using System.Numerics;
using System.Reflection;
using System.Text;
using System.Text.Json;
using Google.Protobuf;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>
/// One logger call of step-log capture (design 041, sdks/SPEC.md section 6.9): the sizes the buffers count, the cut at a character boundary,
/// the fields by data-model.md section 5, and the redaction with a secret set, exact to the byte against the constants.
/// </summary>
public sealed class StepLogFormatTests
{
    private static KeyValuePair<string, object?> F(string key, object? value) => new(key, value);

    private static ulong[] Sequences(IEnumerable<StepLogRecord> records) => [.. records.Select(record => record.Sequence)];

    [Fact]
    public void TheSizesTheSdkCountsAreTheBytesProtobufWritesDefaultsAndOneofValuesIncluded()
    {
        StepLogRecord[] records =
        [
            StepLogInternals.BuildRecord("é", epochMs: 1_795_779_247_123),
            StepLogInternals.BuildRecord(
                string.Empty,
                [F("zero", 0), F("neg", -5), F("big", long.MaxValue), F("no", false), F("empty", string.Empty), F("d", 0.25), F(string.Empty, "x")],
                "System.Exception: x\n   at y",
                level: Microsoft.Extensions.Logging.LogLevel.Trace,
                sequence: 1UL << 40,
                epochMs: 0),
            StepLogInternals.BuildRecord(new string('m', 9000), level: Microsoft.Extensions.Logging.LogLevel.Error, sequence: 300, epochMs: 5),
        ];
        Assert.True(records[2].Truncated);
        Assert.Equal(7, records[1].Fields.Count);
        var sizes = records.Select(record => record.CalculateSize()).ToArray();
        for (var i = 0; i < records.Length; i++)
        {
            Assert.Equal(records[i].ToByteArray().Length, sizes[i]);
        }

        var carry = new StepLogCarry { HighestSequence = 1UL << 40 };
        carry.Records.AddRange(records);
        Assert.Equal(carry.ToByteArray().Length, StepLogInternals.CarrySize(sizes, 1UL << 40));
        Assert.Equal(new StepLogCarry { HighestSequence = 300 }.ToByteArray().Length, StepLogInternals.CarrySize([], 300));

        var identity = new AppendStepLogsRequest
        {
            WorkerId = "worker-1",
            SessionToken = "session-token",
            TaskId = "run-1/charge/1",
            RunId = "0195b2c0-0000-7000-8000-000000000001",
            StepId = "0195b2c0-0000-7000-8000-000000000002",
            Attempt = 3,
            Generation = 77,
            LeaseToken = "lt-123",
            Instance = new ExecutionInstanceIdentity
            {
                DefinitionNodeId = "each",
                InstancePath = { new ScopePathSegment { ScopeNodeId = "items" }, new ScopePathSegment { ScopeNodeId = "inner", IterationIndex = 129 } },
            },
        };
        var full = identity.Clone();
        full.Records.AddRange(records);
        Assert.Equal(full.ToByteArray().Length, StepLogInternals.RequestSize(identity, sizes));
    }

    [Fact]
    public void AMessageIsCutAtTheLastCharacterBoundaryUnderItsCeilingAndTextIsMadeWellFormed()
    {
        var (cut, wasCut) = StepLogInternals.CutUtf8("a" + new string('é', 4500), 8192);
        Assert.True(wasCut);
        Assert.Equal(8191, Encoding.UTF8.GetByteCount(cut));
        Assert.Equal("a" + new string('é', 4095), cut);
        Assert.Equal(("short", false), StepLogInternals.CutUtf8("short", 8192));
        Assert.Equal("a\uFFFDb\uFFFDc\uFFFDd😀", StepLogInternals.WellFormed("a\0b\ud800c\udc00d😀"));

        // A record's message and exception are cut at 8,192 bytes, and the cut sets truncated.
        var record = StepLogInternals.BuildRecord("a" + new string('é', 4500), exception: new string('x', 8192));
        Assert.Equal(8191, Encoding.UTF8.GetByteCount(record.Message));
        Assert.Equal(8192, record.Exception.Length);
        Assert.True(record.Truncated);
        Assert.False(StepLogInternals.BuildRecord("fits", exception: new string('x', 8192)).Truncated);
    }

    [Fact]
    public void FieldsKeepTheirTypesDropANullWriteAnythingElseAsCompactJsonAndNameTheNonFiniteDoubles()
    {
        var (fields, truncated) = StepLogInternals.Fields(
        [
            F("text", "plain"), F("count", 42), F("ratio", 0.25), F("flag", true), F("nothing", null), F("list", new object[] { 1, "two", true }),
            F("nan", double.NaN), F("inf", double.PositiveInfinity), F("minf", float.NegativeInfinity), F("nul", "a\0b"), F("big", long.MaxValue),
            F("huge", BigInteger.Pow(2, 64)), F("unsigned", ulong.MaxValue), F("small", (byte)7), F("map", new Dictionary<string, object?> { ["b"] = 2, ["a"] = null }),
        ]);
        Assert.False(truncated);
        Assert.Equal(
            new StepLogField[]
            {
                new StepLogField { Key = "big", IntValue = long.MaxValue },
                new StepLogField { Key = "count", IntValue = 42 },
                new StepLogField { Key = "flag", BoolValue = true },
                new StepLogField { Key = "huge", StringValue = "18446744073709551616" },
                new StepLogField { Key = "inf", StringValue = "Infinity" },
                new StepLogField { Key = "list", StringValue = "[1,\"two\",true]" },
                new StepLogField { Key = "map", StringValue = "{\"b\":2,\"a\":null}" },
                new StepLogField { Key = "minf", StringValue = "-Infinity" },
                new StepLogField { Key = "nan", StringValue = "NaN" },
                new StepLogField { Key = "nul", StringValue = "a\uFFFDb" },
                new StepLogField { Key = "ratio", DoubleValue = 0.25 },
                new StepLogField { Key = "small", IntValue = 7 },
                new StepLogField { Key = "text", StringValue = "plain" },
                new StepLogField { Key = "unsigned", StringValue = "18446744073709551615" },
            },
            fields);

        // The message template of a structured call is not a field.
        Assert.Empty(StepLogInternals.Fields([F("{OriginalFormat}", "Paid {Amount}")]).Fields);
    }

    [Fact]
    public void FieldsOver32KeysKeepTheFirst32InKeyOrderAndOver2048BytesStopBeforeTheOneThatPasses()
    {
        var many = Enumerable.Range(1, 40).Select(n => F($"k{n:D2}", n)).Reverse().ToList();
        var keys = StepLogInternals.Fields(many);
        Assert.True(keys.Truncated);
        Assert.Equal(Enumerable.Range(1, 32).Select(n => $"k{n:D2}"), keys.Fields.Select(field => field.Key));

        var big = StepLogInternals.Fields([F("a", new string('x', 1100)), F("b", new string('y', 1100)), F("c", true)]);
        Assert.True(big.Truncated);
        Assert.Equal(new[] { new StepLogField { Key = "a", StringValue = new string('x', 1100) } }, big.Fields);

        // Exactly 2,048 bytes is kept: {"a":"x…"} is 8 bytes of frame and 2,040 of value.
        Assert.False(StepLogInternals.Fields([F("a", new string('x', 2040))]).Truncated);
        Assert.True(StepLogInternals.Fields([F("a", new string('x', 2041))]).Truncated);

        // An escaped character counts as JSON writes it: a quote is 2 bytes, a control character 6.
        Assert.False(StepLogInternals.Fields([F("a", new string('"', 1020))]).Truncated);
        Assert.True(StepLogInternals.Fields([F("a", new string('"', 1021))]).Truncated);
        Assert.False(StepLogInternals.Fields([F("a", new string('\u0001', 340))]).Truncated);
        Assert.True(StepLogInternals.Fields([F("a", new string('\u0001', 341))]).Truncated);
    }

    [Fact]
    public void KeysCollidingAfterRedactionTakeTheNextSuffixInTheOrderOfTheirOriginalBytesAndANumberHoldingASecretIsHidden()
    {
        static string Redact(string text) => text.Replace("canary-key-1", "[REDACTED]", StringComparison.Ordinal).Replace("12345678", "[REDACTED]", StringComparison.Ordinal);
        var (fields, _) = StepLogInternals.Fields(
            [F("canary-key-1", "second"), F("[REDACTED]", "first"), F("n", 12345678), F("d", 1e15), F("x", 12345678.5), F("ok", 7), F("[REDACTED]~2", "third")],
            Redact);
        Assert.Equal(
            new StepLogField[]
            {
                new StepLogField { Key = "[REDACTED]", StringValue = "first" },
                new StepLogField { Key = "[REDACTED]~2", StringValue = "third" },
                new StepLogField { Key = "[REDACTED]~3", StringValue = "second" },
                new StepLogField { Key = "d", DoubleValue = 1e15 },
                new StepLogField { Key = "n", StringValue = "[REDACTED]" },
                new StepLogField { Key = "ok", IntValue = 7 },
                new StepLogField { Key = "x", StringValue = "[REDACTED]" },
            },
            fields);

        // The order of the original keys is their UTF-8 bytes: U+FF5E (3 bytes) sorts before U+1F600 (4 bytes), which UTF-16 orders the
        // other way, and both after "~".
        var order = StepLogInternals.Fields([F("\U0001F600", 1), F("\uFF5E", 2), F("~", 3)]).Fields.Select(field => field.Key);
        Assert.Equal(new[] { "~", "\uFF5E", "\U0001F600" }, order);
    }

    [Fact]
    public void TheSecretSetRedactsByTheRuleOfRedactLongestFirstValuesUnder6BytesIgnoredInTheTextLeft()
    {
        var secrets = StepLogInternals.NewSecretSet();
        secrets.Add(Bytes("xxxxxx"), Bytes("D]yyyyy"), Bytes("short"), Bytes("abcdefgh"), Bytes("abcdef"));

        // "D]yyyyy" (7 bytes) goes before "xxxxxx" (6): the match the second replacement creates is not looked for again.
        Assert.Equal("[REDACTED]yyyyy", secrets.Redact("xxxxxxyyyyy"));
        Assert.Equal("[REDACTED] and [REDACTED] and short", secrets.Redact("abcdefgh and abcdef and short"));
        Assert.Equal("nothing here", secrets.Redact("nothing here"));

        // Equal lengths in the order they joined: the first to join is replaced first, and leaves nothing for the other.
        var ties = StepLogInternals.NewSecretSet();
        ties.Add(Bytes("aaaaaa"), Bytes("aaaaab"));
        Assert.Equal("[REDACTED]b", ties.Redact("aaaaaab"));
        var reversed = StepLogInternals.NewSecretSet();
        reversed.Add(Bytes("aaaaab"), Bytes("aaaaaa"));
        Assert.Equal("a[REDACTED]", reversed.Redact("aaaaaab"));

        // Never printed: its string form, its debugger form and its JSON hold no value.
        foreach (var shown in new[] { secrets.ToString(), JsonSerializer.Serialize(secrets.Target), DebuggerDisplayOf(secrets.Target) })
        {
            Assert.DoesNotContain("abcdef", shown, StringComparison.Ordinal);
            Assert.DoesNotContain("xxxxxx", shown, StringComparison.Ordinal);
        }

        Assert.Equal("StepLogSecretSet(4 values)", secrets.ToString());
        foreach (var field in secrets.Target.GetType().GetFields(BindingFlags.Instance | BindingFlags.NonPublic))
        {
            if (field.FieldType != typeof(object) && !field.FieldType.IsValueType)
            {
                Assert.Equal(DebuggerBrowsableState.Never, field.GetCustomAttribute<DebuggerBrowsableAttribute>()?.State);
            }
        }
    }

    [Fact]
    public void TheProcessSecretSetIsReachedByNoPublicMemberAndTheProcessScopeExposesOnlyAnAdd()
    {
        var set = StepLogInternals.TypeOf("StepLogSecretSet");
        Assert.False(set.IsPublic);
        foreach (var type in typeof(SdkInfo).Assembly.GetExportedTypes())
        {
            foreach (var member in type.GetMembers(BindingFlags.Public | BindingFlags.Instance | BindingFlags.Static))
            {
                var returned = member switch
                {
                    PropertyInfo property => property.PropertyType,
                    FieldInfo field => field.FieldType,
                    MethodInfo method => method.ReturnType,
                    _ => null,
                };
                Assert.NotEqual(set, returned);
            }
        }

        // The process's set is a private field; nothing of the scope hands it out.
        var scope = StepLogInternals.TypeOf("ProcessStepLogs");
        var held = Assert.Single(scope.GetFields(BindingFlags.Static | BindingFlags.NonPublic | BindingFlags.Public), field => field.FieldType == set);
        Assert.True(held.IsPrivate);
        Assert.DoesNotContain(scope.GetMembers(BindingFlags.Static | BindingFlags.NonPublic | BindingFlags.Public), member =>
            member is MethodInfo { IsPrivate: false } method && method.ReturnType == set);
    }

    [Fact]
    public void RedactionRunsOnTheTextAsFormattedASecretHoldingANulIsFoundBeforeTheNulIsReplaced()
    {
        var secrets = StepLogInternals.NewSecretSet();
        secrets.Add(Bytes("pa\0ssword123"));
        var record = StepLogInternals.BuildRecord(
            "x pa\0ssword123 y",
            [F("k\0pa\0ssword123", "pa\0ssword123")],
            StepLogInternals.ExceptionText(new InvalidOperationException("pa\0ssword123")),
            secrets.Redact);
        Assert.Equal("x [REDACTED] y", record.Message);
        Assert.Equal(new[] { new StepLogField { Key = "k\uFFFD[REDACTED]", StringValue = "[REDACTED]" } }, record.Fields);
        Assert.StartsWith("System.InvalidOperationException: [REDACTED]", record.Exception, StringComparison.Ordinal);
    }

    [Fact]
    public void ASecretInsideAnObjectAnArrayOrAByteBufferIsRedactedWhateverTheEncoderEscapes()
    {
        const string secret = "line\"one\\two\nthree";
        var secrets = StepLogInternals.NewSecretSet();
        secrets.Add(Bytes(secret), Bytes("bytes-secret-77"));
        var (fields, _) = StepLogInternals.Fields(
        [
            F("nested", new { inner = new object[] { secret, new Dictionary<string, int> { [secret] = 1 } }, plain = "ok" }),
            F("list", new[] { "a", secret }),
            F("raw", Bytes("bytes-secret-77")),
            F("view", new ReadOnlyMemory<byte>(Bytes("bytes-secret-77"))),
            F("deep", new { blob = Bytes("bytes-secret-77"), memory = new Memory<byte>(Bytes("plain-bytes")) }),
        ],
            secrets.Redact);
        var text = string.Join('\n', fields.Select(field => field.ToString()));
        Assert.DoesNotContain("line", text, StringComparison.Ordinal);
        Assert.DoesNotContain("three", text, StringComparison.Ordinal);
        Assert.DoesNotContain("bytes-secret-77", text, StringComparison.Ordinal);
        Assert.DoesNotContain(Convert.ToBase64String(Bytes("bytes-secret-77")), text, StringComparison.Ordinal);
        Assert.Equal(
            new StepLogField[]
            {
                new StepLogField { Key = "deep", StringValue = "{\"blob\":\"[REDACTED]\",\"memory\":\"plain-bytes\"}" },
                new StepLogField { Key = "list", StringValue = "[\"a\",\"[REDACTED]\"]" },
                new StepLogField { Key = "nested", StringValue = "{\"inner\":[\"[REDACTED]\",{\"[REDACTED]\":1}],\"plain\":\"ok\"}" },
                new StepLogField { Key = "raw", StringValue = "[REDACTED]" },
                new StepLogField { Key = "view", StringValue = "[REDACTED]" },
            },
            fields);
    }

    [Fact]
    public void ASecretThatIsNotValidUtf8JoinsTheSetAsTheTextTheAccessorHandsOut()
    {
        var secrets = StepLogInternals.NewSecretSet();
        byte[] bytes = [0xff, .. Bytes("s3cr3t-token-42")];
        secrets.Add(bytes);
        Assert.Equal("logged [REDACTED]", secrets.Redact("logged " + Encoding.UTF8.GetString(bytes)));

        // Its minimum counts the text that joins: one invalid byte and five letters are U+FFFD (3 bytes) and 5, so 8 bytes: kept.
        var lenient = StepLogInternals.NewSecretSet();
        lenient.Add([0xff, .. Bytes("abcde")]);
        Assert.Equal("[REDACTED]", lenient.Redact("\uFFFDabcde"));
    }

    [Fact]
    public void TheSecretSetHolds1024DistinctValuesOr256KiBAndTheOneThatDoesNotFitStopsCaptureOnce()
    {
        var secrets = StepLogInternals.NewSecretSet();
        var values = Enumerable.Range(0, 1024).Select(n => Bytes($"secret-{n:D6}")).ToArray();
        Assert.False(secrets.Add(values));
        Assert.False(secrets.Add(values[0]), "a value already held takes no room");
        Assert.False(secrets.Full);
        Assert.True(secrets.Add(Bytes("one-value-too-many")));
        Assert.True(secrets.Full);
        Assert.False(secrets.Add(Bytes("and-another-one")), "stopped once");

        var bytes = StepLogInternals.NewSecretSet();
        Assert.False(bytes.Add(Bytes(new string('x', 256 * 1024))));
        Assert.True(bytes.Add(Bytes(new string('y', 6))), "past 256 KiB");

        var shortOnes = StepLogInternals.NewSecretSet();
        Assert.False(shortOnes.Add(Enumerable.Range(0, 5000).Select(n => Bytes($"{n % 100000:D5}")).ToArray()), "values under 6 bytes take no room");
        Assert.False(shortOnes.Full);
    }

    [Fact]
    public void TheExceptionTextIsTheExceptionsToStringWithItsInnerExceptionsAndAThrowingOneIsItsType()
    {
        Exception thrown;
        try
        {
            throw new InvalidOperationException("outer", new ArgumentException("inner"));
        }
        catch (InvalidOperationException error)
        {
            thrown = error;
        }

        var text = StepLogInternals.ExceptionText(thrown);
        Assert.StartsWith("System.InvalidOperationException: outer\n ---> System.ArgumentException: inner", text.Replace("\r\n", "\n", StringComparison.Ordinal), StringComparison.Ordinal);
        Assert.Contains(nameof(TheExceptionTextIsTheExceptionsToStringWithItsInnerExceptionsAndAThrowingOneIsItsType), text, StringComparison.Ordinal);
        Assert.Equal(typeof(ThrowingException).FullName, StepLogInternals.ExceptionText(new ThrowingException()));
        Assert.Equal(string.Empty, StepLogInternals.ExceptionText(null));
    }

    [Fact]
    public void LevelsMapOntoTheFiveAndAnyOtherValueIsItsCompactJsonOrItsStringForm()
    {
        Assert.Equal(StepLogLevel.Trace, StepLogInternals.BuildRecord("x", level: Microsoft.Extensions.Logging.LogLevel.Trace).Level);
        Assert.Equal(StepLogLevel.Debug, StepLogInternals.BuildRecord("x", level: Microsoft.Extensions.Logging.LogLevel.Debug).Level);
        Assert.Equal(StepLogLevel.Info, StepLogInternals.BuildRecord("x", level: Microsoft.Extensions.Logging.LogLevel.Information).Level);
        Assert.Equal(StepLogLevel.Warn, StepLogInternals.BuildRecord("x", level: Microsoft.Extensions.Logging.LogLevel.Warning).Level);
        Assert.Equal(StepLogLevel.Error, StepLogInternals.BuildRecord("x", level: Microsoft.Extensions.Logging.LogLevel.Error).Level);
        Assert.Equal(StepLogLevel.Error, StepLogInternals.BuildRecord("x", level: Microsoft.Extensions.Logging.LogLevel.Critical).Level);

        var circular = new Node();
        circular.Self = circular;
        var (fields, _) = StepLogInternals.Fields([F("cycle", circular), F("throwing", new ThrowingString()), F("getter", new ThrowingGetter())]);
        Assert.Equal(
            new StepLogField[]
            {
                new StepLogField { Key = "cycle", StringValue = "{\"Self\":\"" + typeof(Node).FullName + "\"}" },
                new StepLogField { Key = "getter", StringValue = typeof(ThrowingGetter).FullName },
                new StepLogField { Key = "throwing", StringValue = typeof(ThrowingString).FullName },
            },
            fields);
    }

    [Fact]
    public void EveryByteOrCharacterCollectionIsCapturedAsItsTextAtAnyDepthNeverAsNumbersOrBase64()
    {
        const string secret = "bytes-secret-77";
        var secrets = StepLogInternals.NewSecretSet();
        secrets.Add(Bytes(secret));
        static IEnumerable<byte> Generated(string text)
        {
            foreach (var one in Encoding.UTF8.GetBytes(text))
            {
                yield return one;
            }
        }

        var (fields, _) = StepLogInternals.Fields(
        [
            F("list", new List<byte>(Bytes(secret))),
            F("chars", secret.ToCharArray()),
            F("generated", Generated(secret)),
            F("holder", new Holder { Raw = Bytes(secret), Listed = [.. Bytes("plain")], Characters = "chars".ToCharArray(), Segment = new ArraySegment<byte>(Bytes("xx" + secret), 2, secret.Length) }),
            F("nested", new Dictionary<string, object?> { ["deep"] = new object[] { new List<byte>(Bytes(secret)), new ReadOnlyMemory<char>("text".ToCharArray()) } }),
        ],
            secrets.Redact);
        var text = string.Join('\n', fields.Select(field => field.ToString()));
        Assert.DoesNotContain(secret, text, StringComparison.Ordinal);
        Assert.DoesNotContain(Convert.ToBase64String(Bytes(secret)), text, StringComparison.Ordinal);
        Assert.DoesNotContain("98,121", text, StringComparison.Ordinal);
        Assert.Equal(
            new[]
            {
                new StepLogField { Key = "chars", StringValue = "[REDACTED]" },
                new StepLogField { Key = "generated", StringValue = "[REDACTED]" },
                new StepLogField { Key = "holder", StringValue = "{\"Raw\":\"[REDACTED]\",\"Listed\":\"plain\",\"Characters\":\"chars\",\"Segment\":\"[REDACTED]\"}" },
                new StepLogField { Key = "list", StringValue = "[REDACTED]" },
                new StepLogField { Key = "nested", StringValue = "{\"deep\":[\"[REDACTED]\",\"text\"]}" },
            },
            fields);
    }

    [Fact]
    public void ANonUtf8SecretIsFoundWhereverItWasLoggedFromItsRawBytesOrMadeWellFormedFirst()
    {
        var secrets = StepLogInternals.NewSecretSet();
        byte[] raw = [0xff, .. Bytes("abcde-token")];
        secrets.Add(raw);

        // Logged from its raw bytes, nested in an object, the one lenient decoding finds it.
        var (fields, _) = StepLogInternals.Fields([F("inside", new { blob = raw }), F("bytes", raw)], secrets.Redact);
        Assert.Equal(
            new[] { new StepLogField { Key = "bytes", StringValue = "[REDACTED]" }, new StepLogField { Key = "inside", StringValue = "{\"blob\":\"[REDACTED]\"}" } },
            fields);

        // Logged with a lone surrogate where the invalid byte was: made well formed, it is the value the set holds, and is redacted again.
        var record = StepLogInternals.BuildRecord("key \ud800abcde-token", [F("k", "\udc00abcde-token")], "\ud800abcde-token", secrets.Redact);
        Assert.Equal("key [REDACTED]", record.Message);
        Assert.Equal("[REDACTED]", record.Exception);
        Assert.Equal(new[] { new StepLogField { Key = "k", StringValue = "[REDACTED]" } }, record.Fields);
    }

    [Fact]
    public void AValueTheCopyCannotOpenOrThatThrowsOrIsNestedDeeperThan64IsItsTypesNameNeverItsOwnText()
    {
        const string secret = "line\"one\\two";
        var secrets = StepLogInternals.NewSecretSet();
        secrets.Add(Bytes(secret));
        var chain = new Chain("leaf", null);
        for (var n = 0; n < 70; n++)
        {
            chain = new Chain("link", chain);
        }

        var (fields, _) = StepLogInternals.Fields(
            [F("escaping", new EscapingThrower(secret)), F("callback", (Func<int>)(() => 1)), F("map", new Dictionary<object, int> { [new object()] = 1 })],
            secrets.Redact);
        var text = string.Join('\n', fields.Select(field => field.ToString()));
        Assert.DoesNotContain("line", text, StringComparison.Ordinal);
        Assert.DoesNotContain("printed", text, StringComparison.Ordinal);
        Assert.Equal(typeof(EscapingThrower).FullName, fields.Single(field => field.Key == "escaping").StringValue);
        Assert.Equal(typeof(Func<int>).ToString(), fields.Single(field => field.Key == "callback").StringValue);
        Assert.Equal("System.Collections.Generic.Dictionary`2[System.Object,System.Int32]", fields.Single(field => field.Key == "map").StringValue);

        // The chain is copied 64 levels deep (its text is cut by the field bound, so its depth is read from the copy's own text).
        var copied = StepLogInternals.CopyText(chain, secrets.Redact);
        Assert.Contains("\"Next\":\"" + typeof(Chain).FullName + "\"", copied, StringComparison.Ordinal);
        Assert.Equal(64, CountOf("\"Next\":{", copied));
    }

    [Fact]
    public void AStructureSharedAlongManyPathsIsWalkedOnceAndOneThatWouldPrintTooMuchIsItsTypesName()
    {
        var secrets = StepLogInternals.NewSecretSet();
        secrets.Add(Bytes("shared-secret"));
        var leaf = new Dictionary<string, object?> { ["a"] = "shared-secret" };
        Assert.Equal(
            "[{\"a\":\"[REDACTED]\"},{\"a\":\"[REDACTED]\"}]",
            StepLogInternals.Fields([F("pair", new object[] { leaf, leaf })], secrets.Redact).Fields[0].StringValue);

        // 2^40 paths through 40 levels of a node shared twice: walked once per node, and refused once it would print more than 4,096 values.
        object node = "end";
        for (var n = 0; n < 40; n++)
        {
            node = new object[] { node, node };
        }

        var started = Stopwatch.GetTimestamp();
        var (fields, _) = StepLogInternals.Fields([F("shared", node)], secrets.Redact);
        Assert.Equal("System.Object[]", fields[0].StringValue);
        Assert.True(Stopwatch.GetElapsedTime(started) < TimeSpan.FromSeconds(1), "the shared structure was walked along its paths");
    }

    [Fact]
    public void AValueOf256KiBJoinsTheSetAndIsFound()
    {
        var secrets = StepLogInternals.NewSecretSet();
        var value = string.Concat(Enumerable.Range(0, 256 * 1024 / 8).Select(n => $"{n % 10000000:D8}"));
        Assert.False(secrets.Add(Bytes(value)));
        Assert.False(secrets.Full);
        Assert.Equal("a [REDACTED] b", secrets.Redact("a " + value + " b"));
        Assert.Equal("a " + value[..^1] + " b", secrets.Redact("a " + value[..^1] + " b"));
    }

    private static int CountOf(string part, string text)
    {
        var count = 0;
        for (var at = text.IndexOf(part, StringComparison.Ordinal); at >= 0; at = text.IndexOf(part, at + 1, StringComparison.Ordinal))
        {
            count++;
        }

        return count;
    }

    internal static byte[] Bytes(string text) => Encoding.UTF8.GetBytes(text);

    internal sealed record Chain(string Name, Chain? Next);

    internal sealed class Holder
    {
        public byte[]? Raw { get; init; }

        public List<byte>? Listed { get; init; }

        public char[]? Characters { get; init; }

        public ArraySegment<byte> Segment { get; init; }
    }

    /// <summary>A value whose copy throws and whose own text shows a secret escaped: printing it would hide the secret from redaction.</summary>
    internal sealed class EscapingThrower(string secret)
    {
        public string Value => throw new InvalidOperationException(GetType().Name);

        public override string ToString() => "printed " + secret.Replace("\"", "\\\"", StringComparison.Ordinal);
    }

    private static string DebuggerDisplayOf(object value)
    {
        var display = value.GetType().GetCustomAttribute<DebuggerDisplayAttribute>();
        Assert.NotNull(display);
        Assert.Equal("{ToString(),nq}", display.Value);
        return value.ToString()!;
    }

    private sealed class ThrowingException : Exception
    {
        public override string ToString() => throw new InvalidOperationException("no text");
    }

    internal sealed class Node
    {
        public Node? Self { get; set; }

        public override string ToString() => "a node";
    }

    internal sealed class ThrowingString
    {
        public override string ToString() => throw new InvalidOperationException("no text");

        public string Value => throw new InvalidOperationException(GetType().Name);
    }

    internal sealed class ThrowingGetter
    {
        public string Value => throw new InvalidOperationException(GetType().Name);
    }
}
