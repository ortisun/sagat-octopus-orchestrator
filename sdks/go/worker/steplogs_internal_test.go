package worker

// The pure parts of step-log capture (sdks/SPEC.md section 6.9): sizes, text, fields, the secret set and its
// redaction, against the constants of the specification. The host-level tests are in steplogs_test.go.

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"math"
	"math/big"
	"os"
	"reflect"
	"slices"
	"sort"
	"strings"
	"testing"
	"time"

	"google.golang.org/protobuf/proto"

	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
)

func TestStepLogSizesCountedAreTheBytesProtobufWrites(t *testing.T) {
	records := []*pb.StepLogRecord{
		{Sequence: 1, WorkerTimeEpochMs: 1_795_779_247_123, Level: pb.StepLogLevel_STEP_LOG_LEVEL_INFO, Message: "é"},
		{Sequence: 1 << 40, Level: pb.StepLogLevel_STEP_LOG_LEVEL_TRACE, Fields: []*pb.StepLogField{
			{Key: "zero", Value: &pb.StepLogField_IntValue{IntValue: 0}}, {Key: "neg", Value: &pb.StepLogField_IntValue{IntValue: -5}},
			{Key: "big", Value: &pb.StepLogField_IntValue{IntValue: math.MaxInt64}}, {Key: "no", Value: &pb.StepLogField_BoolValue{}},
			{Key: "empty", Value: &pb.StepLogField_StringValue{}}, {Key: "d", Value: &pb.StepLogField_DoubleValue{DoubleValue: 0.25}},
			{Key: "", Value: &pb.StepLogField_StringValue{StringValue: "x"}},
		}, Exception: "boom\nstack", Truncated: true},
		{Sequence: 300, WorkerTimeEpochMs: 5, Level: pb.StepLogLevel_STEP_LOG_LEVEL_ERROR, Message: strings.Repeat("m", 8192)},
	}
	sizes := make([]int, len(records))
	for i, record := range records {
		encoded, err := proto.Marshal(record)
		if err != nil {
			t.Fatal(err)
		}
		sizes[i] = proto.Size(record)
		if sizes[i] != len(encoded) {
			t.Errorf("record %d: counted %d, encoded %d", record.GetSequence(), sizes[i], len(encoded))
		}
	}
	carry, _ := proto.Marshal(&pb.StepLogCarry{Records: records, HighestSequence: 1 << 40})
	if got := carrySize(sizes, 1<<40); got != len(carry) {
		t.Errorf("carry: counted %d, encoded %d", got, len(carry))
	}
	empty, _ := proto.Marshal(&pb.StepLogCarry{HighestSequence: 300})
	if got := carrySize(nil, 300); got != len(empty) {
		t.Errorf("empty carry: counted %d, encoded %d", got, len(empty))
	}
	capture := &attemptCapture{identity: attemptIdentity{
		taskID: "run-1/charge/1", runID: "0195b2c0-0000-7000-8000-000000000001", stepID: "0195b2c0-0000-7000-8000-000000000002",
		leaseToken: "lt-123", attempt: 3, generation: 77,
		instance: &pb.ExecutionInstanceIdentity{DefinitionNodeId: "each", InstancePath: []*pb.ScopePathSegment{
			{ScopeNodeId: "items"}, {ScopeNodeId: "inner", IterationIndex: 129},
		}},
	}}
	overhead := proto.Size(capture.request(nil, "worker-1", "session-token"))
	full, _ := proto.Marshal(capture.request(records, "worker-1", "session-token"))
	counted := overhead
	for _, size := range sizes {
		counted += delimited(size)
	}
	if counted != len(full) {
		t.Errorf("request: counted %d, encoded %d", counted, len(full))
	}
}

func TestStepLogMessageIsCutAtACharacterBoundaryAndMadeWellFormed(t *testing.T) {
	cut, wasCut := cutUTF8("a"+strings.Repeat("é", 4500), limitMessageBytes)
	if !wasCut || len(cut) != 8191 || cut != "a"+strings.Repeat("é", 4095) {
		t.Errorf("cut %v to %d bytes", wasCut, len(cut))
	}
	if cut, wasCut := cutUTF8("short", limitMessageBytes); cut != "short" || wasCut {
		t.Errorf("%q %v", cut, wasCut)
	}
	// NUL and each invalid sequence become U+FFFD, one for each byte utf8.DecodeRune refuses (as encoding/json writes
	// them): a lone byte is one, a surrogate written in UTF-8 is three; the rest is kept.
	if got := wellFormed("a\x00b\xffc\xed\xa0\x80d😀"); got != "a�b�c���d😀" {
		t.Errorf("%q", got)
	}
}

// identity is a redactor that replaces nothing.
type identity struct{}

func (identity) redact(text string) string { return text }

// replacing is a redactor of fixed values, for the tests of the fields.
type replacing []string

func (r replacing) redact(text string) string {
	for _, value := range r {
		text = strings.ReplaceAll(text, value, redactedText)
	}
	return text
}

func fieldsOf(attrs ...slog.Attr) collected {
	var got collected
	for _, attr := range attrs {
		got.flatten("", attr)
	}
	return got
}

func describe(fields []*pb.StepLogField) []string {
	out := make([]string, 0, len(fields))
	for _, field := range fields {
		switch value := field.GetValue().(type) {
		case *pb.StepLogField_StringValue:
			out = append(out, fmt.Sprintf("%s=string:%s", field.GetKey(), value.StringValue))
		case *pb.StepLogField_IntValue:
			out = append(out, fmt.Sprintf("%s=int:%d", field.GetKey(), value.IntValue))
		case *pb.StepLogField_DoubleValue:
			out = append(out, fmt.Sprintf("%s=double:%v", field.GetKey(), value.DoubleValue))
		case *pb.StepLogField_BoolValue:
			out = append(out, fmt.Sprintf("%s=bool:%v", field.GetKey(), value.BoolValue))
		}
	}
	return out
}

func TestStepLogFieldsKeepTheirTypesDropANullAndWriteAnythingElseAsCompactJSON(t *testing.T) {
	huge, _ := new(big.Int).SetString("18446744073709551616", 10)
	type kind int
	got := fieldsOf(
		slog.String("text", "plain"), slog.Int("count", 42), slog.Float64("ratio", 0.25), slog.Bool("flag", true),
		slog.Any("nothing", nil), slog.Any("nilmap", map[string]int(nil)), slog.Any("list", []any{1, "two", true, "<a&b>"}),
		slog.Float64("nan", math.NaN()), slog.Float64("inf", math.Inf(1)), slog.Float64("minf", math.Inf(-1)),
		slog.String("nul", "a\x00b"), slog.Int64("big", math.MaxInt64), slog.Uint64("unsigned", math.MaxUint64),
		slog.Any("huge", huge), slog.Any("raw", []byte("raw\xffbytes")), slog.Duration("wait", 1500*time.Millisecond),
		slog.Any("named", kind(3)), slog.Any("object", map[string]any{"b": 1, "a": []string{"x"}}),
	)
	fields, truncated := buildFields(got.fields, identity{})
	want := []string{
		"big=int:9223372036854775807", "count=int:42", "flag=bool:true", "huge=string:18446744073709551616",
		"inf=string:Infinity", `list=string:[1,"two",true,"<a&b>"]`, "minf=string:-Infinity", "named=int:3", "nan=string:NaN",
		"nul=string:a�b", `object=string:{"a":["x"],"b":1}`, "ratio=double:0.25", "raw=string:raw�bytes", "text=string:plain",
		"unsigned=string:18446744073709551615", "wait=int:1500000000",
	}
	if truncated || fmt.Sprint(describe(fields)) != fmt.Sprint(want) {
		t.Errorf("truncated %v\n got %v\nwant %v", truncated, describe(fields), want)
	}
}

func TestStepLogFieldsStopAt32KeysAndBefore2048Bytes(t *testing.T) {
	var attrs []slog.Attr
	var keys []string
	for n := 40; n >= 1; n-- {
		attrs = append(attrs, slog.Int(fmt.Sprintf("k%02d", n), n))
	}
	for n := 1; n <= 32; n++ {
		keys = append(keys, fmt.Sprintf("k%02d", n))
	}
	fields, truncated := buildFields(fieldsOf(attrs...).fields, identity{})
	kept := make([]string, len(fields))
	for i, field := range fields {
		kept[i] = field.GetKey()
	}
	if !truncated || fmt.Sprint(kept) != fmt.Sprint(keys) {
		t.Errorf("%v %v", truncated, kept)
	}
	big, cut := buildFields(fieldsOf(slog.String("a", strings.Repeat("x", 1100)), slog.String("b", strings.Repeat("y", 1100)), slog.Bool("c", true)).fields, identity{})
	if !cut || len(big) != 1 || big[0].GetKey() != "a" {
		t.Errorf("%v %d", cut, len(big))
	}
	// Exactly 2,048 bytes is kept: {"a":"x..."} is 8 bytes of frame and 2,040 of value.
	if _, cut := buildFields(fieldsOf(slog.String("a", strings.Repeat("x", 2040))).fields, identity{}); cut {
		t.Error("2,048 bytes were cut")
	}
	if _, cut := buildFields(fieldsOf(slog.String("a", strings.Repeat("x", 2041))).fields, identity{}); !cut {
		t.Error("2,049 bytes were kept")
	}
}

func TestStepLogKeysCollidingAfterRedactionTakeTheNextSuffixAndANumberHoldingASecretIsHidden(t *testing.T) {
	secrets := replacing{"canary-key-1", "12345678"}
	got := fieldsOf(slog.String("canary-key-1", "second"), slog.String("[REDACTED]", "first"), slog.Int("n", 12345678),
		slog.Float64("d", 1e15), slog.Int("ok", 7), slog.Float64("f", 0.12345678))
	fields, _ := buildFields(got.fields, secrets)
	want := []string{"[REDACTED]=string:first", "[REDACTED]~2=string:second", "d=double:1e+15", "f=string:[REDACTED]", "n=string:[REDACTED]", "ok=int:7"}
	if fmt.Sprint(describe(fields)) != fmt.Sprint(want) {
		t.Errorf("\n got %v\nwant %v", describe(fields), want)
	}
}

func TestStepLogSecretSetRedactsByTheRuleOfRedact(t *testing.T) {
	var set secretSet
	set.add([][]byte{[]byte("xxxxxx"), []byte("D]yyyyy"), []byte("short"), []byte("abcdefgh"), []byte("abcdef")})
	// "D]yyyyy" (7 bytes) goes before "xxxxxx" (6): the match the second replacement creates is not looked for again.
	for text, want := range map[string]string{
		"xxxxxxyyyyy":                   "[REDACTED]yyyyy",
		"abcdefgh and abcdef and short": "[REDACTED] and [REDACTED] and short",
		"nothing here":                  "nothing here",
		"abcde":                         "abcde",
	} {
		if got := set.redact(text); got != want {
			t.Errorf("%q: %q, want %q", text, got, want)
		}
	}
}

func TestStepLogSecretSetHolds1024ValuesOr256KiBAndStopsCaptureOnce(t *testing.T) {
	var set secretSet
	values := make([][]byte, limitSecretValues)
	for n := range values {
		values[n] = []byte(fmt.Sprintf("secret-%06d", n))
	}
	if set.add(values) {
		t.Fatal("1,024 values stopped capture")
	}
	if set.add(values[:1]) || set.full.Load() {
		t.Fatal("a value already held took room")
	}
	if !set.add([][]byte{[]byte("one-value-too-many")}) || !set.full.Load() {
		t.Fatal("the 1,025th value did not stop capture")
	}
	if set.add([][]byte{[]byte("and-another-one")}) {
		t.Fatal("stopped twice")
	}
	set.wipe()
	if set.add([][]byte{bytes.Repeat([]byte("x"), limitSecretBytes)}) {
		t.Fatal("256 KiB stopped capture")
	}
	if !set.add([][]byte{[]byte("yyyyyy")}) {
		t.Fatal("past 256 KiB did not stop capture")
	}
}

func TestStepLogSecretSetIsNeverPrinted(t *testing.T) {
	var set processStepLogs
	set.warned = map[string]bool{}
	set.secrets.add([][]byte{[]byte("canary-printed-1")})
	var out bytes.Buffer
	slog.New(slog.NewJSONHandler(&out, nil)).Info("x", "set", &set, "secrets", &set.secrets)
	printed := fmt.Sprintf("%v %+v %#v %s %v %+v %#v", &set, &set, &set, &set.secrets, &set.secrets, &set.secrets, &set.secrets) + out.String()
	if strings.Contains(printed, "canary-printed-1") || !strings.Contains(printed, "1 values") {
		t.Errorf("%s", printed)
	}
}

func TestStepLogRedactionRunsOnTheTextAsFormatted(t *testing.T) {
	var set secretSet
	// A secret holding a NUL is found before NUL is replaced (decision D041-164 (3)).
	set.add([][]byte{[]byte("nul\x00secret")})
	if got := clean("before nul\x00secret after", &set); got != "before [REDACTED] after" {
		t.Errorf("%q", got)
	}
	// A secret that is not UTF-8 joins as its lenient text, what Secrets.Text hands out (decision D041-164 (4)); it is
	// found in that text and in its raw bytes once they are made well formed.
	set.add([][]byte{[]byte("tok\xff\xfeen-value")})
	text := (&Secrets{values: map[string][]byte{"k": []byte("tok\xff\xfeen-value")}})
	lenientText, _ := text.Text("k")
	if lenientText != "tok��en-value" {
		t.Fatalf("%q", lenientText)
	}
	for _, logged := range []string{"a " + lenientText, "a tok\xff\xfeen-value"} {
		if got := clean(logged, &set); got != "a [REDACTED]" {
			t.Errorf("%q gave %q", logged, got)
		}
	}
}

func TestStepLogAValueThatIsNotAScalarIsEncodedFromARedactedCopy(t *testing.T) {
	var set secretSet
	secret := "pa\"ss\nword-9Kx2" // a quote and a newline, which the JSON encoder escapes
	set.add([][]byte{[]byte(secret)})
	type inner struct {
		Token string `json:"token"`
		Raw   []byte `json:"-"`
	}
	got := fieldsOf(
		slog.Any("map", map[string]any{"token": secret, secret: 1, "list": []any{secret, []byte(secret)}}),
		slog.Any("bytes", []byte(secret)),
		slog.Any("struct", inner{Token: secret}),
		slog.Any("pointer", &inner{Token: "x " + secret}),
		slog.Any("number", json.Number("12")),
	)
	fields, _ := buildFields(got.fields, &set)
	encoded, _ := json.Marshal(secret)
	for _, field := range fields {
		text := field.GetStringValue()
		if strings.Contains(text, secret) || strings.Contains(text, strings.Trim(string(encoded), `"`)) || strings.Contains(text, "word-9Kx2") {
			t.Errorf("%s holds the secret: %s", field.GetKey(), text)
		}
	}
	want := []string{
		"bytes=string:[REDACTED]", `map=string:{"[REDACTED]":1,"list":["[REDACTED]","[REDACTED]"],"token":"[REDACTED]"}`,
		"number=string:12", `pointer=string:{"token":"x [REDACTED]"}`, `struct=string:{"token":"[REDACTED]"}`,
	}
	if fmt.Sprint(describe(fields)) != fmt.Sprint(want) {
		t.Errorf("\n got %v\nwant %v", describe(fields), want)
	}
}

// Embedded, Holder and Behind carry byte buffers in a struct field, an embedded struct and behind a pointer.
type Embedded struct {
	Key []byte `json:"key"`
}

type Behind struct {
	Raw []byte `json:"raw,omitempty"`
}

type Holder struct {
	Embedded
	*Behind
	Raw     []byte  `json:"raw"`
	Skipped []byte  `json:"-"`
	Empty   []byte  `json:"empty,omitempty"`
	Pointer *Behind `json:"pointer"`
	Count   int     `json:"count,string"`
	hidden  []byte
}

func TestStepLogAByteBufferInAStructIsItsTextAndNeverBase64(t *testing.T) {
	var set secretSet
	secret := []byte("struct-secret-7Qp")
	set.add([][]byte{secret})
	holder := Holder{
		Embedded: Embedded{Key: secret}, Behind: &Behind{Raw: []byte("promoted")}, Raw: secret, Skipped: secret,
		Pointer: &Behind{Raw: append([]byte("x "), secret...)}, Count: 7, hidden: secret,
	}
	got := fieldsOf(slog.Any("value", holder), slog.Any("pointer", &holder), slog.Any("embedded", Embedded{Key: secret}))
	fields, _ := buildFields(got.fields, &set)
	encoded := base64.StdEncoding.EncodeToString(secret)
	for _, field := range fields {
		text := field.GetStringValue()
		if strings.Contains(text, string(secret)) || strings.Contains(text, encoded) || strings.Contains(text, encoded[:12]) {
			t.Errorf("%s holds the secret: %s", field.GetKey(), text)
		}
	}
	// The names are those encoding/json gives the fields.
	var reference map[string]any
	plain, _ := json.Marshal(holder)
	_ = json.Unmarshal(plain, &reference)
	copied, err := structCopy(reflect.ValueOf(holder), identity{}, 0, map[visit]bool{})
	if err != nil {
		t.Fatal(err)
	}
	var names, referenceNames []string
	for name := range copied.(map[string]any) {
		names = append(names, name)
	}
	for name := range reference {
		referenceNames = append(referenceNames, name)
	}
	sort.Strings(names)
	sort.Strings(referenceNames)
	if !slices.Equal(names, referenceNames) {
		t.Errorf("names %v, encoding/json %v", names, referenceNames)
	}
	// The promoted field raw of the embedded *Behind loses to the outer raw, as encoding/json decides.
	holderText := `{"count":"7","key":"[REDACTED]","pointer":{"raw":"x [REDACTED]"},"raw":"[REDACTED]"}`
	want := []string{`embedded=string:{"key":"[REDACTED]"}`, "pointer=string:" + holderText, "value=string:" + holderText}
	if fmt.Sprint(describe(fields)) != fmt.Sprint(want) {
		t.Errorf("\n got %v\nwant %v", describe(fields), want)
	}
}

func TestStepLogAByteOrRuneArrayIsItsTextAndNeverNumbers(t *testing.T) {
	var set secretSet
	secret := "array-secret-9Zq"
	set.add([][]byte{[]byte(secret)})
	var key [32]byte
	copy(key[:], secret)
	var runes [16]rune
	copy(runes[:], []rune(secret))
	type sealed struct {
		Key   [32]byte `json:"key"`
		Runes []rune   `json:"runes"`
	}
	got := fieldsOf(slog.Any("key", key), slog.Any("runes", []rune(secret)), slog.Any("runeArray", runes),
		slog.Any("struct", sealed{Key: key, Runes: []rune(secret)}), slog.Any("list", []any{key}))
	fields, _ := buildFields(got.fields, &set)
	if len(fields) != 5 {
		t.Fatalf("%v", describe(fields))
	}
	for _, field := range fields {
		text := field.GetStringValue()
		if strings.Contains(text, secret) || strings.Contains(text, "97,114") || !strings.Contains(text, "[REDACTED]") {
			t.Errorf("%s: %q", field.GetKey(), text)
		}
	}
	nuls := strings.Repeat("\uFFFD", 16)
	if text := fields[0].GetStringValue(); fields[0].GetKey() != "key" || text != "[REDACTED]"+nuls {
		t.Errorf("%q", text)
	}
}

func TestStepLogASecretThatIsNotUTF8IsFoundInsideANestedValue(t *testing.T) {
	var set secretSet
	raw := []byte("k3y\xff\xfe-abc")
	set.add([][]byte{raw})
	got := fieldsOf(slog.Any("c", map[string]string{"k": string(raw), string(raw): "v"}), slog.Any("s", []string{"x " + string(raw)}))
	fields, _ := buildFields(got.fields, &set)
	want := []string{`c=string:{"[REDACTED]":"v","k":"[REDACTED]"}`, `s=string:["x [REDACTED]"]`}
	if fmt.Sprint(describe(fields)) != fmt.Sprint(want) {
		t.Errorf("\n got %v\nwant %v", describe(fields), want)
	}
}

func TestStepLogASecretHoldingANulAndAnInvalidSequenceIsFound(t *testing.T) {
	var set secretSet
	set.add([][]byte{[]byte("nu\x00l\xffsecret-x")})
	for _, logged := range []string{"a nu\x00l\xffsecret-x", "a nu\x00l\uFFFDsecret-x"} {
		if got := clean(logged, &set); got != "a [REDACTED]" {
			t.Errorf("%q gave %q", logged, got)
		}
	}
}

func TestStepLogAValueHeldTwiceIsCopiedOnceThenNamedByItsType(t *testing.T) {
	// Thirty levels that each hold the level below twice: 2^30 paths, none deeper than the copy's bound.
	loop := map[string]any{}
	for level := 0; level < 30; level++ {
		loop = map[string]any{"a": loop, "b": loop}
	}
	done := make(chan []string, 1)
	go func() {
		fields, _ := buildFields(fieldsOf(slog.Any("loop", loop)).fields, identity{})
		done <- describe(fields)
	}()
	select {
	case got := <-done:
		if len(got) != 1 || !strings.Contains(got[0], "map[string]interface {}") {
			t.Errorf("%v", got)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("a self-referencing map was not encoded within 10 seconds")
	}
}

// formatted is an error that prints a stack under %+v, as the errors packages that implement fmt.Formatter do.
type formatted struct{}

func (formatted) Error() string { return "the payment provider refused" }
func (f formatted) Format(s fmt.State, verb rune) {
	if verb == 'v' && s.Flag('+') {
		_, _ = fmt.Fprint(s, "the payment provider refused\nmain.charge\n\tcharge.go:12")
		return
	}
	_, _ = fmt.Fprint(s, f.Error())
}

// panicking is an error whose Error panics.
type panicking struct{}

func (*panicking) Error() string { panic("Error") }

func TestStepLogAnErrorGivesItsExceptionTextAndIsKeptAsAField(t *testing.T) {
	got := fieldsOf(slog.Any("err", errors.New("plain failure")), slog.Any("cause", formatted{}), slog.Any("bad", &panicking{}))
	if exception := exceptionOf(got.exceptions); exception != "*worker.panicking\nthe payment provider refused\nmain.charge\n\tcharge.go:12\nplain failure" {
		t.Errorf("%q", exception)
	}
	fields, _ := buildFields(got.fields, identity{})
	want := []string{"bad=string:*worker.panicking", "cause=string:the payment provider refused", "err=string:plain failure"}
	if fmt.Sprint(describe(fields)) != fmt.Sprint(want) {
		t.Errorf("%v", describe(fields))
	}
}

func TestStepLogLevelsMapOntoTheFive(t *testing.T) {
	for level, want := range map[slog.Level]pb.StepLogLevel{
		-9: pb.StepLogLevel_STEP_LOG_LEVEL_TRACE, LevelTrace: pb.StepLogLevel_STEP_LOG_LEVEL_TRACE, -5: pb.StepLogLevel_STEP_LOG_LEVEL_TRACE,
		-4: pb.StepLogLevel_STEP_LOG_LEVEL_DEBUG, -1: pb.StepLogLevel_STEP_LOG_LEVEL_DEBUG, 0: pb.StepLogLevel_STEP_LOG_LEVEL_INFO,
		3: pb.StepLogLevel_STEP_LOG_LEVEL_INFO, 4: pb.StepLogLevel_STEP_LOG_LEVEL_WARN, 7: pb.StepLogLevel_STEP_LOG_LEVEL_WARN,
		8: pb.StepLogLevel_STEP_LOG_LEVEL_ERROR, 12: pb.StepLogLevel_STEP_LOG_LEVEL_ERROR,
	} {
		if got := wireLevel(level); got != want {
			t.Errorf("%d: %v", level, got)
		}
	}
}

// TestStepLogCallBenchmark is the micro-benchmark of sdks/SPEC.md section 6.9, "The cost of a call": one captured call
// (format, redact, cut, size) with an 8 KiB message and a secret set of 1,024 values, 1,000 warm-up calls then 10,000
// on the monotonic clock, the p99 printed. It is run by hand, never by the normal test command or CI:
//
//	SAGAT_STEP_LOG_BENCH=1 go test ./worker -run TestStepLogCallBenchmark -v
func TestStepLogCallBenchmark(t *testing.T) {
	if os.Getenv("SAGAT_STEP_LOG_BENCH") != "1" {
		t.Skip("run by hand with SAGAT_STEP_LOG_BENCH=1")
	}
	var set secretSet
	values := make([][]byte, limitSecretValues)
	for n := range values {
		values[n] = []byte(fmt.Sprintf("k%015x", uint64(n)*0x9e3779b97f4a7c15))
	}
	if set.add(values) || set.count() != limitSecretValues {
		t.Fatalf("the set holds %d values", set.count())
	}
	message := strings.Repeat("x", 8*1024-32) + " an ordinary line of text"
	call := func(sequence uint64) int {
		got := fieldsOf(slog.String("order", "o-123"), slog.Int("amount", 1999), slog.String("currency", "EUR"))
		return proto.Size(buildRecord(sequence, time.Now().UnixMilli(), slog.LevelInfo, message, got, &set))
	}
	for n := uint64(1); n <= 1000; n++ {
		call(n)
	}
	samples := make([]time.Duration, 0, 10_000)
	for n := uint64(1); n <= 10_000; n++ {
		start := time.Now() // carries the monotonic clock reading
		call(n)
		samples = append(samples, time.Since(start))
	}
	sort.Slice(samples, func(i, j int) bool { return samples[i] < samples[j] })
	at := func(q float64) time.Duration {
		return samples[min(len(samples)-1, int(math.Ceil(q*float64(len(samples))))-1)]
	}
	t.Logf("go step-log call, 8 KiB message, 1,024 secrets: p50 %v, p99 %v, max %v", at(0.5), at(0.99), at(1))
	if at(0.99) >= time.Millisecond {
		t.Errorf("p99 %v is not under 1 ms", at(0.99))
	}
}
