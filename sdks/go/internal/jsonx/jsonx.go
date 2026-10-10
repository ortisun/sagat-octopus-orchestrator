// Package jsonx is how the SDK writes and reads JSON: the rules of sdks/SPEC.md section 8 in one
// place. Integers stay integers (a json.Number or a RawMessage is written as it is), nothing is
// HTML-escaped, instants are RFC 3339 in UTC with a Z, and what is not JSON (NaN, infinities,
// invalid UTF-8, duplicate keys) is an error on the way out.
package jsonx

import (
	"bytes"
	"encoding"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"reflect"
	"strings"
	"time"
	"unicode/utf8"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
)

const maxDepth = 512

var (
	marshalerType     = reflect.TypeOf((*json.Marshaler)(nil)).Elem()
	textMarshalerType = reflect.TypeOf((*encoding.TextMarshaler)(nil)).Elem()
	rawMessageType    = reflect.TypeOf(json.RawMessage(nil))
)

// Encode returns the JSON text of v, compact, UTF-8, without HTML escaping. A value that cannot be
// written faithfully is a VALIDATION_FAILED error whose message names the place, never the value.
func Encode(v any) ([]byte, error) {
	if err := check(reflect.ValueOf(v), "$", 0); err != nil {
		return nil, err
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		return nil, sagat.NewError(sagat.CodeValidationFailed, "the value cannot be written as JSON ("+classify(err)+")").WithCause(err)
	}
	return bytes.TrimRight(buf.Bytes(), "\n"), nil
}

// classify names what went wrong without repeating the value.
func classify(err error) string {
	var unsupported *json.UnsupportedValueError
	var unsupportedType *json.UnsupportedTypeError
	var marshaler *json.MarshalerError
	switch {
	case errors.As(err, &unsupported):
		return "unsupported value"
	case errors.As(err, &unsupportedType):
		return "unsupported type " + unsupportedType.Type.String()
	case errors.As(err, &marshaler):
		return "a MarshalJSON method failed"
	default:
		return "encoding failed"
	}
}

func check(v reflect.Value, path string, depth int) error {
	if depth > maxDepth {
		return sagat.Invalid("%s is nested too deeply or refers to itself", path)
	}
	if !v.IsValid() {
		return nil
	}
	t := v.Type()
	if t == rawMessageType {
		return checkRaw(v.Bytes(), path)
	}
	switch v.Kind() {
	case reflect.Interface, reflect.Pointer:
		if v.IsNil() {
			return nil
		}
		if t.Implements(marshalerType) || t.Implements(textMarshalerType) {
			return nil
		}
		return check(v.Elem(), path, depth+1)
	}
	if t.Implements(marshalerType) || t.Implements(textMarshalerType) || reflect.PointerTo(t).Implements(marshalerType) {
		return nil
	}
	switch v.Kind() {
	case reflect.String:
		if t.PkgPath() == "encoding/json" && t.Name() == "Number" {
			return nil
		}
		if !utf8.ValidString(v.String()) {
			return sagat.Invalid("%s is not valid UTF-8", path)
		}
	case reflect.Float32, reflect.Float64:
		f := v.Float()
		if math.IsNaN(f) || math.IsInf(f, 0) {
			return sagat.Invalid("%s is not a finite number", path)
		}
	case reflect.Slice, reflect.Array:
		if t.Elem().Kind() == reflect.Uint8 {
			return nil
		}
		for i := 0; i < v.Len(); i++ {
			if err := check(v.Index(i), fmt.Sprintf("%s[%d]", path, i), depth+1); err != nil {
				return err
			}
		}
	case reflect.Map:
		iter := v.MapRange()
		for iter.Next() {
			key := iter.Key()
			if key.Kind() == reflect.String && !utf8.ValidString(key.String()) {
				return sagat.Invalid("a key under %s is not valid UTF-8", path)
			}
			name := "?"
			if key.Kind() == reflect.String {
				name = key.String()
			}
			if err := check(iter.Value(), path+"."+name, depth+1); err != nil {
				return err
			}
		}
	case reflect.Struct:
		for i := 0; i < v.NumField(); i++ {
			field := t.Field(i)
			if !field.IsExported() || field.Tag.Get("json") == "-" {
				continue
			}
			if err := check(v.Field(i), path+"."+field.Name, depth+1); err != nil {
				return err
			}
		}
	}
	return nil
}

// checkRaw refuses what a RawMessage must not hold: invalid JSON, invalid UTF-8, duplicate keys.
func checkRaw(raw []byte, path string) error {
	if len(raw) == 0 {
		return nil // encoding/json writes "null"
	}
	if !utf8.Valid(raw) {
		return sagat.Invalid("%s is not valid UTF-8", path)
	}
	if !json.Valid(raw) {
		return sagat.Invalid("%s is not valid JSON", path)
	}
	if err := noDuplicateKeys(raw); err != nil {
		return sagat.Invalid("%s has a key twice in an object", path)
	}
	return nil
}

var errDuplicate = errors.New("duplicate key")

func noDuplicateKeys(raw []byte) error {
	return scan(json.NewDecoder(bytes.NewReader(raw)))
}

// scan reads one JSON value and fails when an object in it has a key twice.
func scan(dec *json.Decoder) error {
	tok, err := dec.Token()
	if err != nil {
		return err
	}
	delim, ok := tok.(json.Delim)
	if !ok {
		return nil
	}
	switch delim {
	case '{':
		seen := map[string]struct{}{}
		for dec.More() {
			keyToken, err := dec.Token()
			if err != nil {
				return err
			}
			key, _ := keyToken.(string)
			if _, twice := seen[key]; twice {
				return errDuplicate
			}
			seen[key] = struct{}{}
			if err := scan(dec); err != nil {
				return err
			}
		}
	case '[':
		for dec.More() {
			if err := scan(dec); err != nil {
				return err
			}
		}
	}
	_, err = dec.Token() // the closing delimiter
	return err
}

// Instant is how the SDK writes an instant: RFC 3339 in UTC with a Z and millisecond precision,
// whatever the zone of the machine.
func Instant(t time.Time) string {
	return t.UTC().Format("2006-01-02T15:04:05.000Z")
}

// ParseInstant reads an RFC 3339 instant with any offset and returns it in UTC.
func ParseInstant(text string) (time.Time, bool) {
	t, err := time.Parse(time.RFC3339Nano, text)
	if err != nil {
		return time.Time{}, false
	}
	return t.UTC(), true
}

// Object decodes a JSON object into its members, each kept as the raw JSON it was, so that a number
// is not turned into a float and a member the SDK does not know is still reachable.
func Object(data []byte) (map[string]json.RawMessage, error) {
	var m map[string]json.RawMessage
	if err := json.Unmarshal(data, &m); err != nil || m == nil {
		return nil, errors.New("not a JSON object")
	}
	return m, nil
}

// String reads a member that is a JSON string; ok is false when it is absent, null or another type.
func String(m map[string]json.RawMessage, key string) (string, bool) {
	raw, ok := m[key]
	if !ok || IsNull(raw) {
		return "", false
	}
	var s string
	if err := json.Unmarshal(raw, &s); err != nil {
		return "", false
	}
	return s, true
}

// Int reads a member that is a JSON integer.
func Int(m map[string]json.RawMessage, key string) (int64, bool) {
	raw, ok := m[key]
	if !ok || IsNull(raw) {
		return 0, false
	}
	var n json.Number
	if err := json.Unmarshal(raw, &n); err != nil {
		return 0, false
	}
	i, err := n.Int64()
	if err != nil {
		f, ferr := n.Float64()
		if ferr != nil {
			return 0, false
		}
		return int64(f), true
	}
	return i, true
}

// Bool reads a member that is a JSON boolean.
func Bool(m map[string]json.RawMessage, key string) (value, ok bool) {
	raw, present := m[key]
	if !present || IsNull(raw) {
		return false, false
	}
	if err := json.Unmarshal(raw, &value); err != nil {
		return false, false
	}
	return value, true
}

// Time reads a member that is an RFC 3339 string.
func Time(m map[string]json.RawMessage, key string) (time.Time, bool) {
	s, ok := String(m, key)
	if !ok {
		return time.Time{}, false
	}
	return ParseInstant(s)
}

// IsNull reports whether the raw JSON is absent or the literal null.
func IsNull(raw json.RawMessage) bool {
	return len(raw) == 0 || strings.TrimSpace(string(raw)) == "null"
}
