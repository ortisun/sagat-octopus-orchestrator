package jsonx

import (
	"bytes"
	"encoding/json"
	"errors"
	"math"
	"sort"
	"strconv"
	"strings"
	"unicode/utf16"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
)

// Canonical returns the canonical text of v: RFC 8785 (JSON Canonicalization Scheme). UTF-8, no
// insignificant whitespace, the keys of every object sorted by their UTF-16 code units, strings
// escaped as JSON.stringify does, and every number as ECMAScript prints it (1.0 is 1, 600000 stays
// 600000, 1e21 is 1e+21). Numbers are IEEE-754 doubles, as the RFC says: an integer past 2^53 is
// rounded to the double nearest to it, which is why requests do not go through here.
//
// Go's encoding/json sorts keys by their UTF-8 bytes, which is the order of code points, and that
// differs from the UTF-16 order above U+FFFF (an emoji sorts before U+FFFD in UTF-16 and after it in
// UTF-8): the keys are sorted here explicitly.
func Canonical(v any) ([]byte, error) {
	text, err := Encode(v)
	if err != nil {
		return nil, err
	}
	decoder := json.NewDecoder(bytes.NewReader(text))
	decoder.UseNumber()
	var tree any
	if err := decoder.Decode(&tree); err != nil {
		return nil, sagat.NewError(sagat.CodeInternal, "the value could not be read back as JSON").WithCause(err)
	}
	var out bytes.Buffer
	if err := writeCanonical(&out, tree); err != nil {
		return nil, err
	}
	return out.Bytes(), nil
}

func writeCanonical(out *bytes.Buffer, value any) error {
	switch v := value.(type) {
	case nil:
		out.WriteString("null")
	case bool:
		out.WriteString(strconv.FormatBool(v))
	case string:
		writeString(out, v)
	case json.Number:
		f, err := strconv.ParseFloat(string(v), 64)
		if err != nil || math.IsInf(f, 0) || math.IsNaN(f) {
			return sagat.Invalid("a number is outside the range of a double")
		}
		if wholeBeyondExact(f) {
			return sagat.Invalid("a whole number beyond 2^53 - 1 cannot be written in canonical text; send it as a string")
		}
		out.WriteString(ecmaNumber(f))
	case []any:
		out.WriteByte('[')
		for i, item := range v {
			if i > 0 {
				out.WriteByte(',')
			}
			if err := writeCanonical(out, item); err != nil {
				return err
			}
		}
		out.WriteByte(']')
	case map[string]any:
		type member struct {
			key   string
			units []uint16
		}
		members := make([]member, 0, len(v))
		for key := range v {
			members = append(members, member{key: key, units: utf16.Encode([]rune(key))})
		}
		sort.Slice(members, func(i, j int) bool { return lessUnits(members[i].units, members[j].units) })
		out.WriteByte('{')
		for i, m := range members {
			if i > 0 {
				out.WriteByte(',')
			}
			writeString(out, m.key)
			out.WriteByte(':')
			if err := writeCanonical(out, v[m.key]); err != nil {
				return err
			}
		}
		out.WriteByte('}')
	default:
		return errors.New("jsonx: an unexpected value in a decoded document")
	}
	return nil
}

func lessUnits(a, b []uint16) bool {
	for i := 0; i < len(a) && i < len(b); i++ {
		if a[i] != b[i] {
			return a[i] < b[i]
		}
	}
	return len(a) < len(b)
}

const hexDigits = "0123456789abcdef"

// writeString writes a JSON string as JSON.stringify does: only the quote, the backslash and the
// control characters are escaped.
func writeString(out *bytes.Buffer, s string) {
	out.WriteByte('"')
	for _, r := range s {
		switch {
		case r == '"':
			out.WriteString(`\"`)
		case r == '\\':
			out.WriteString(`\\`)
		case r == '\b':
			out.WriteString(`\b`)
		case r == '\f':
			out.WriteString(`\f`)
		case r == '\n':
			out.WriteString(`\n`)
		case r == '\r':
			out.WriteString(`\r`)
		case r == '\t':
			out.WriteString(`\t`)
		case r < 0x20:
			out.WriteString(`\u00`)
			out.WriteByte(hexDigits[r>>4])
			out.WriteByte(hexDigits[r&0xf])
		default:
			out.WriteRune(r)
		}
	}
	out.WriteByte('"')
}

// ecmaNumber prints a finite double as Number.prototype.toString does (ECMA-262, Number::toString).
func ecmaNumber(f float64) string {
	if f == 0 {
		return "0" // also -0
	}
	sign := ""
	if f < 0 {
		sign, f = "-", -f
	}
	// The shortest digits that read back as f, and the exponent: d.ddde+XX.
	shortest := strconv.FormatFloat(f, 'e', -1, 64)
	mantissa, exponent, _ := strings.Cut(shortest, "e")
	digits := strings.Replace(mantissa, ".", "", 1)
	exp10, _ := strconv.Atoi(exponent)
	k, n := len(digits), exp10+1 // f = 0.digits * 10^n
	switch {
	case k <= n && n <= 21:
		return sign + digits + strings.Repeat("0", n-k)
	case 0 < n && n <= 21:
		return sign + digits[:n] + "." + digits[n:]
	case -6 < n && n <= 0:
		return sign + "0." + strings.Repeat("0", -n) + digits
	}
	exponentSign := "+"
	if n-1 < 0 {
		exponentSign = "-"
	}
	e := n - 1
	if e < 0 {
		e = -e
	}
	if k == 1 {
		return sign + digits + "e" + exponentSign + strconv.Itoa(e)
	}
	return sign + digits[:1] + "." + digits[1:] + "e" + exponentSign + strconv.Itoa(e)
}

// wholeBeyondExact reports whether a number is refused in canonical text (sdks/SPEC.md, section 8): a whole number
// beyond 2^53 - 1 that ECMAScript prints in digits, that is, below 1e21. The rule is on the value, not on how it was
// written: 9007199254740993, 9007199254740992.0 and 1e20 are all refused, because the text of a document must be the
// same bytes in every language and no language but TypeScript's can print the nearest double and call it exact.
// 1e21 and above, which ECMAScript prints with an exponent, and every number with a fraction, are numbers.
func wholeBeyondExact(f float64) bool {
	abs := math.Abs(f)
	return abs >= 1<<53 && abs < 1e21 && abs == math.Trunc(abs)
}
