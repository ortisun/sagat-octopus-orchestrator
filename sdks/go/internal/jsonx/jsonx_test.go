package jsonx

import (
	"encoding/json"
	"math"
	"strings"
	"testing"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
)

func TestEncodeKeepsIntegersTextAndDoesNotEscapeHTML(t *testing.T) {
	input := json.RawMessage(`{"big":9007199254740993,"ratio":0.1,"money":"19.99","text":"a<b>&c ação € 😀","nothing":null,"list":[1,"two",null,{"deep":[true,false]}],"zero":0,"empty":{}}`)
	out, err := Encode(map[string]any{"input": input, "n": json.Number("600000"), "i": int64(1) << 62, "s": "<&>"})
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{`"big":9007199254740993`, `"ratio":0.1`, `"money":"19.99"`, `"n":600000`, `"i":4611686018427387904`, `"s":"<&>"`, `"text":"a<b>&c ação € 😀"`, `"nothing":null`} {
		if !strings.Contains(string(out), want) {
			t.Errorf("%s is missing from %s", want, out)
		}
	}
	if strings.Contains(string(out), "600000.0") || strings.HasSuffix(string(out), "\n") {
		t.Errorf("an integer became a float, or a newline was left: %s", out)
	}
}

func TestEncodeRefusesWhatIsNotJSONAndNamesThePlaceNotTheValue(t *testing.T) {
	cases := map[string]any{
		"nan":              map[string]any{"x": math.NaN()},
		"infinity":         []any{math.Inf(1)},
		"invalid utf-8":    map[string]any{"x": "secret-\xff-value"},
		"invalid key":      map[string]any{"k\xff": 1},
		"nested struct":    struct{ A []float64 }{A: []float64{1, math.NaN()}},
		"raw invalid json": json.RawMessage(`{"a":`),
		"raw invalid utf8": json.RawMessage("{\"a\":\"\xff\"}"),
		"raw twin keys":    json.RawMessage(`{"a":1,"b":{"c":1,"c":2}}`),
		"raw twin escaped": json.RawMessage(`{"a":1,"a":2}`),
		"unsupported type": map[string]any{"f": func() {}},
	}
	for name, value := range cases {
		_, err := Encode(value)
		if !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("%s: %v", name, err)
			continue
		}
		if strings.Contains(err.Error(), "secret-") {
			t.Errorf("%s: the value is in the message: %v", name, err)
		}
	}
	if _, err := Encode(map[string]any{"x": json.RawMessage(`{"a":1,"b":[{"a":1},{"a":2}]}`)}); err != nil {
		t.Errorf("the same key in two objects is fine: %v", err)
	}
}

func TestEncodeStopsAtAnImpossibleDepth(t *testing.T) {
	var deep any = "x"
	for i := 0; i < 2000; i++ {
		deep = []any{deep}
	}
	if _, err := Encode(deep); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("%v", err)
	}
}

func TestInstantsAreUTCWithAZWhateverTheZone(t *testing.T) {
	zone := time.FixedZone("Asia/Kolkata", 5*3600+1800)
	local := time.Date(2030, 1, 1, 5, 30, 0, 0, zone)
	if got := Instant(local); got != "2030-01-01T00:00:00.000Z" {
		t.Errorf("%s", got)
	}
	parsed, ok := ParseInstant("2026-10-10T15:30:00.5+05:30")
	if !ok || parsed.Location() != time.UTC || parsed.Format(time.RFC3339Nano) != "2026-10-10T10:00:00.5Z" {
		t.Errorf("%v %v", parsed, ok)
	}
	if _, ok := ParseInstant("yesterday"); ok {
		t.Error("garbage was parsed")
	}
}

func TestMembersAreReadWithoutTurningIntegersIntoFloats(t *testing.T) {
	m, err := Object([]byte(`{"s":"x","n":9007199254740993,"b":true,"t":"2026-10-10T10:00:00Z","z":null}`))
	if err != nil {
		t.Fatal(err)
	}
	if s, ok := String(m, "s"); !ok || s != "x" {
		t.Error("String")
	}
	if n, ok := Int(m, "n"); !ok || n != 9007199254740993 {
		t.Errorf("Int %d", n)
	}
	if b, ok := Bool(m, "b"); !ok || !b {
		t.Error("Bool")
	}
	if _, ok := Time(m, "t"); !ok {
		t.Error("Time")
	}
	if _, ok := String(m, "z"); ok {
		t.Error("null is not a string")
	}
	if !IsNull(m["z"]) || !IsNull(nil) || IsNull(m["s"]) {
		t.Error("IsNull")
	}
	if _, err := Object([]byte(`[1]`)); err == nil {
		t.Error("an array is not an object")
	}
	if _, err := Object([]byte(`null`)); err == nil {
		t.Error("null is not an object")
	}
}

func TestCanonicalIsRFC8785(t *testing.T) {
	// The numbers example of RFC 8785, section 3.2.4, and the cases the ECMAScript rules are about.
	numbers := json.RawMessage(`[333333333.33333329, 1E30, 4.50, 2e-3, 0.000000000000000000000000001]`)
	got, err := Canonical(numbers)
	if err != nil || string(got) != `[333333333.3333333,1e+30,4.5,0.002,1e-27]` {
		t.Errorf("%s %v", got, err)
	}
	cases := map[string]string{
		`1.0`: `1`, `600000`: `600000`, `6e5`: `600000`, `-0`: `0`, `0.1`: `0.1`, `1e21`: `1e+21`, `1e20`: `100000000000000000000`,
		`1.2345678901234568e20`: `123456789012345680000`, `0.000001`: `0.000001`, `1e-7`: `1e-7`, `-1.5e-9`: `-1.5e-9`,
		`1.7976931348623157e308`: `1.7976931348623157e+308`, `5e-324`: `5e-324`,
		`100`: `100`, `1.25e2`: `125`, `12345.6789`: `12345.6789`,
	}
	for in, want := range cases {
		got, err := Canonical(json.RawMessage(in))
		if err != nil || string(got) != want {
			t.Errorf("%s: %s (%v), want %s", in, got, err, want)
		}
	}
	if _, err := Canonical(json.RawMessage(`1e999`)); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("a number past a double is refused: %v", err)
	}
}

func TestCanonicalSortsKeysByUTF16CodeUnitsNotByCodePoints(t *testing.T) {
	// U+1F600 is the surrogates D83D DE00 in UTF-16, so it sorts before U+FFFD (and before U+E000), while
	// its UTF-8 bytes (F0...) sort after theirs (EF...).
	doc := map[string]any{"�": 1, "\U0001F600": 2, "": 3, "b": 4, "a": 5, "": 6, "aa": 7, "Z": 8}
	got, err := Canonical(doc)
	if err != nil {
		t.Fatal(err)
	}
	want := `{"":6,"Z":8,"a":5,"aa":7,"b":4,"😀":2,"` + "" + `":3,"` + "�" + `":1}`
	if string(got) != want {
		t.Errorf("\n got %s\nwant %s", got, want)
	}
	// What encoding/json alone would write sorts the emoji last.
	plain, _ := json.Marshal(doc)
	if string(plain) == want {
		t.Error("the test must tell the two orders apart")
	}
}

func TestCanonicalEscapesAsJSONStringifyDoesAndKeepsNestingAndNulls(t *testing.T) {
	doc := map[string]any{
		"text": "tab\t nl\n cr\r bs\b ff\f quote\" back\\ ctl\x01\x1f del\x7f <&> é €  ",
		"list": []any{nil, true, false, map[string]any{"z": json.Number("1.0"), "y": []any{}}},
		"obj":  map[string]any{},
	}
	got, err := Canonical(doc)
	if err != nil {
		t.Fatal(err)
	}
	want := `{"list":[null,true,false,{"y":[],"z":1}],"obj":{},"text":"tab\t nl\n cr\r bs\b ff\f quote\" back\\ ctl\u0001\u001f del` + "\x7f" + ` <&> é € ` + " " + `"}`
	if string(got) != want {
		t.Errorf("\n got %q\nwant %q", got, want)
	}
	if _, err := Canonical(map[string]any{"x": "\xff"}); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("invalid UTF-8 is refused: %v", err)
	}
}

// SPEC section 8: the canonical text refuses an integer beyond 2^53 - 1, in every SDK, because no language but
// TypeScript's can print the nearest double and call it exact.
func TestCanonicalRefusesAnIntegerBeyond2To53MinusOneAndNeverExpandsAnExponent(t *testing.T) {
	for _, in := range []string{
		"9007199254740992", "9007199254740993", "-9007199254740992", "-9007199254740993", "12345678901234567890", "-12345678901234567890",
		"9223372036854775807", "99999999999999999999999999999999999999",
	} {
		if _, err := Canonical(json.RawMessage(in)); !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("%s was written: %v", in, err)
		}
		if _, err := Canonical(map[string]any{"deep": []any{json.Number(in)}}); !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("%s inside a document was written: %v", in, err)
		}
	}
	for in, want := range map[string]string{
		"9007199254740991": "9007199254740991", "-9007199254740991": "-9007199254740991", "0": "0", "-0": "0",
		// A literal with a fraction or an exponent is a double, as RFC 8785 says; it is not an integer literal.
		"9007199254740993.0": "9007199254740992", "9.007199254740993e15": "9007199254740992", "1e22": "1e+22", "9223372036854775807e0": "9223372036854776000",
	} {
		got, err := Canonical(json.RawMessage(in))
		if err != nil || string(got) != want {
			t.Errorf("%s: %s (%v), want %s", in, got, err, want)
		}
	}
	if _, err := Canonical(map[string]any{"n": int64(1) << 60}); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("an int64 beyond 2^53-1: %v", err)
	}
	if got, err := Canonical(map[string]any{"n": int64(1)<<53 - 1}); err != nil || string(got) != `{"n":9007199254740991}` {
		t.Errorf("%s %v", got, err)
	}
	// An exponent is a number, not a billion digits: the answer is immediate, and it is an error (past a double).
	began := time.Now()
	for _, in := range []string{"1e999999999", "-1e999999999", "1E400", "123456789e999999999"} {
		if _, err := Canonical(json.RawMessage(in)); !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("%s: %v", in, err)
		}
	}
	if got, err := Canonical(json.RawMessage("1e-999999999")); err != nil || string(got) != "0" {
		t.Errorf("an exponent that underflows is zero: %s %v", got, err)
	}
	if took := time.Since(began); took > time.Second {
		t.Errorf("took %v: an exponent was expanded", took)
	}
	// A 100 000-digit integer is refused by its length, without being parsed as a number.
	if _, err := Canonical(json.RawMessage(strings.Repeat("9", 100_000))); !sagat.IsCode(err, sagat.CodeValidationFailed) {
		t.Errorf("%v", err)
	}
}

// SPEC section 8: invalid UTF-8, a lone surrogate included, is an error on the way out. A Go string cannot hold a
// surrogate, but a JSON text can write one as an escape.
func TestALoneSurrogateEscapeInAJSONTextIsRefusedAndAPairIsNot(t *testing.T) {
	for _, bad := range []string{
		`"\ud800"`, `"\udbff"`, `"\udc00"`, `"\udfff"`, `"\uD83D"`, `"\ude00"`, `"a\ud83d b"`, `"\ud83d\u0041"`, `"\ud83d\ud83d"`, `"\ude00\ud83d"`,
		`{"k\ud800": 1}`, `{"k": ["x", "\ud83d"]}`,
	} {
		if _, err := Encode(json.RawMessage(bad)); !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("%s was written: %v", bad, err)
		}
		if _, err := Canonical(json.RawMessage(bad)); !sagat.IsCode(err, sagat.CodeValidationFailed) {
			t.Errorf("%s was written in canonical text: %v", bad, err)
		}
	}
	for _, good := range []string{
		`"\ud83d\ude00"`, `"\uD83D\uDE00"`, `"\\ud800"`, `"\\\\ud800"`, `"\u0041"`, `"\ue000"`, `"\ud7ff"`, `"x\ud83d\ude00y"`, `{"k": "\ud83d\ude00"}`,
		`"plain"`, `"😀"`,
	} {
		if _, err := Encode(json.RawMessage(good)); err != nil {
			t.Errorf("%s was refused: %v", good, err)
		}
	}
}
