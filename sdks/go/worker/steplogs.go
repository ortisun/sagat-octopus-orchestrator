package worker

// Step-log capture (design 041, sdks/SPEC.md section 6.9): what an activity writes to TaskFrom(ctx).Logger is numbered,
// redacted with the process's secret set, cut to the line ceilings, buffered per attempt, sent through AppendStepLogs
// while the attempt runs and, for its last records, carried on its report. Nothing here blocks a logger call on the
// network or panics into the activity: a failure of capture is counted and dropped (Requirement 041-005).

import (
	"bytes"
	"context"
	"encoding"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"math"
	"math/big"
	"math/rand/v2"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"
	"unicode/utf8"

	"google.golang.org/protobuf/encoding/protowire"
	"google.golang.org/protobuf/proto"

	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
)

// LevelTrace is the trace level of an activity's logger: log/slog has no level below Debug, so the SDK names
// slog.Level(-8). A record below -4 is captured as trace, -4 to -1 as debug, 0 to 3 as info, 4 to 7 as warn and 8 and
// above as error.
const LevelTrace = slog.Level(-8)

// WithStepLogs sets step-log capture (sdks/SPEC.md section 6.9): whether the attempts of this host capture what their
// logger writes, at protocol 1.7 and when the server's policy is on, and from which level. The level is one of
// LevelTrace, slog.LevelDebug, slog.LevelInfo, slog.LevelWarn and slog.LevelError; New refuses any other. By default
// capture is on, at slog.LevelInfo.
func WithStepLogs(capture bool, level slog.Level) Option {
	return func(c *config) { c.stepLogs = stepLogOptions{capture: capture, level: level} }
}

// EventStepLogsDropped is the event of the step-log records the SDK dropped for an attempt, once per attempt and
// reason when the attempt ends: Code is the reason (report_deadline, buffer_full, send_failed, after_ceiling,
// process_bound or secret_set_full) and Records how many.
const EventStepLogsDropped EventType = "steplogs.dropped"

type stepLogOptions struct {
	capture bool
	level   slog.Level
}

func validStepLogLevel(level slog.Level) bool {
	switch level {
	case LevelTrace, slog.LevelDebug, slog.LevelInfo, slog.LevelWarn, slog.LevelError:
		return true
	}
	return false
}

// Why the SDK dropped a record (data-model.md section 7 of design 041): counted locally, never sent.
const (
	dropReportDeadline = "report_deadline"
	dropBufferFull     = "buffer_full"
	dropSendFailed     = "send_failed"
	dropAfterCeiling   = "after_ceiling"
	dropProcessBound   = "process_bound"
	dropSecretSetFull  = "secret_set_full"
)

// The constants of sdks/SPEC.md section 6.9 (Requirement 041-015; never sent by the server).
const (
	stepLogMinor        = 7
	limitAttemptRecords = 1000
	limitAttemptBytes   = 256 * 1024
	limitProcessBytes   = 16 * 1024 * 1024
	limitFlushRecords   = 256
	limitFlushEvery     = time.Second
	limitBatchRecords   = 500
	limitBatchBytes     = 192 * 1024
	limitBatchAge       = 10 * time.Second
	limitCarryRecords   = 64
	limitCarryBytes     = 16 * 1024
	limitReportWait     = 2 * time.Second
	limitMessageBytes   = 8192
	limitFieldKeys      = 32
	limitFieldBytes     = 2048
	limitSecretValues   = 1024
	limitSecretBytes    = 256 * 1024
	limitSecretMinBytes = 6
	limitShutdownFlush  = 1500 * time.Millisecond
	redactedText        = "[REDACTED]"
)

// The waits before the two retries of a failed call, each with a jitter of 20 percent.
var stepLogRetryDelays = [...]time.Duration{200 * time.Millisecond, time.Second}

// wireLevel maps a slog level onto the five levels of a step log.
func wireLevel(level slog.Level) pb.StepLogLevel {
	switch {
	case level < slog.LevelDebug:
		return pb.StepLogLevel_STEP_LOG_LEVEL_TRACE
	case level < slog.LevelInfo:
		return pb.StepLogLevel_STEP_LOG_LEVEL_DEBUG
	case level < slog.LevelWarn:
		return pb.StepLogLevel_STEP_LOG_LEVEL_INFO
	case level < slog.LevelError:
		return pb.StepLogLevel_STEP_LOG_LEVEL_WARN
	}
	return pb.StepLogLevel_STEP_LOG_LEVEL_ERROR
}

// ---- protobuf sizes ------------------------------------------------------------------------------------------------

// delimited is the size of one length-delimited field holding size bytes, its tag included (every tag here is one
// byte).
func delimited(size int) int { return 1 + protowire.SizeVarint(uint64(size)) + size }

// carrySize is the encoded size of a StepLogCarry holding records of these sizes.
func carrySize(recordSizes []int, highest uint64) int {
	size := 0
	if highest != 0 {
		size = 1 + protowire.SizeVarint(highest)
	}
	for _, record := range recordSizes {
		size += delimited(record)
	}
	return size
}

// ---- text: well formed, cut at a character boundary ----------------------------------------------------------------

// lenient is the UTF-8 text of bytes as the SDK hands it out: each invalid sequence (a run of bytes that is not
// UTF-8) replaced by one U+FFFD, as strings.ToValidUTF8 does. A Go string has no surrogates; an invalid sequence is
// their counterpart.
func lenient(text string) string {
	if utf8.ValidString(text) {
		return text
	}
	return strings.ToValidUTF8(text, "\uFFFD")
}

// wellFormed is step 5 of a call (sdks/SPEC.md section 6.9): NUL and each invalid UTF-8 sequence become U+FFFD, so
// that what is sent is valid UTF-8 without a NUL.
func wellFormed(text string) string {
	text = lenient(text)
	if strings.IndexByte(text, 0) < 0 {
		return text
	}
	return strings.ReplaceAll(text, "\x00", "\uFFFD")
}

// clean is steps 4 and 5 for one text: redacted as formatted, so that a value holding a NUL or an invalid sequence is
// still found, then made well formed; when making it well formed changed it, it is redacted again, so that a value the
// set holds in its lenient form (a secret that is not UTF-8, logged from its raw bytes) is found in that form too.
func clean(text string, secrets redactor) string {
	redacted := secrets.redact(text)
	formed := wellFormed(redacted)
	if formed != redacted {
		formed = secrets.redact(formed)
	}
	return formed
}

// cutUTF8 is the longest prefix of a valid UTF-8 text of at most limit bytes that ends at a character boundary, and
// whether it cut.
func cutUTF8(text string, limit int) (string, bool) {
	if len(text) <= limit {
		return text, false
	}
	end := limit
	for end > 0 && !utf8.RuneStart(text[end]) {
		end--
	}
	return text[:end], true
}

// ---- the process's secret set and its redaction --------------------------------------------------------------------

// matcher is an Aho–Corasick automaton over the bytes of the set's values: it tells in one pass whether any value
// occurs in a text, so that a text holding none (the usual case) costs one scan whatever the size of the set.
type matcher struct {
	root     [256]int32
	children [][]matcherEdge
	fail     []int32
	out      []bool
}

type matcherEdge struct {
	b  byte
	to int32
}

func newMatcher(values [][]byte) *matcher {
	m := &matcher{children: [][]matcherEdge{nil}, fail: []int32{0}, out: []bool{false}}
	for _, value := range values {
		state := int32(0)
		for _, b := range value {
			next := m.child(state, b)
			if next < 0 {
				next = int32(len(m.children))
				m.children = append(m.children, nil)
				m.fail = append(m.fail, 0)
				m.out = append(m.out, false)
				m.children[state] = append(m.children[state], matcherEdge{b: b, to: next})
			}
			state = next
		}
		m.out[state] = true
	}
	for _, edge := range m.children[0] {
		m.root[edge.b] = edge.to
	}
	queue := make([]int32, 0, len(m.children))
	for _, edge := range m.children[0] {
		queue = append(queue, edge.to)
	}
	for head := 0; head < len(queue); head++ {
		state := queue[head]
		for _, edge := range m.children[state] {
			m.fail[edge.to] = m.step(m.fail[state], edge.b)
			if m.out[m.fail[edge.to]] {
				m.out[edge.to] = true
			}
			queue = append(queue, edge.to)
		}
	}
	return m
}

func (m *matcher) child(state int32, b byte) int32 {
	for _, edge := range m.children[state] {
		if edge.b == b {
			return edge.to
		}
	}
	return -1
}

func (m *matcher) step(state int32, b byte) int32 {
	for state != 0 {
		if next := m.child(state, b); next >= 0 {
			return next
		}
		state = m.fail[state]
	}
	return m.root[b]
}

func (m *matcher) occursIn(text string) bool {
	state := int32(0)
	for i := 0; i < len(text); i++ {
		state = m.step(state, text[i])
		if m.out[state] {
			return true
		}
	}
	return false
}

// secretSet holds the values ResolveSecrets returned in this process (sdks/SPEC.md section 6.7): distinct values of
// at least 6 UTF-8 bytes, at most 1,024 of them or 256 KiB. It is unexported, never printed (its Format, String,
// GoString and LogValue show a count), never handed to activity code; when a value does not fit, capture stops for the
// process.
type secretSet struct {
	full atomic.Bool

	mu      sync.RWMutex
	values  [][]byte // in the order they joined
	bytes   int
	warned  bool
	matcher *matcher
	ordered [][]byte // the longest first, equal lengths in the order they joined
}

// add adds what one resolution returned; it reports whether the set stopped capture with it (once).
func (s *secretSet) add(values [][]byte) (stopped bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	changed := false
	for _, raw := range values {
		// A value that is not UTF-8 joins as the text Secrets.Text hands out for it (sdks/SPEC.md section 6.7); a
		// value under 6 bytes is never replaced.
		value := raw
		if !utf8.Valid(raw) {
			value = []byte(lenient(string(raw)))
		}
		if len(value) < limitSecretMinBytes || s.holds(value) {
			continue
		}
		if s.full.Load() || len(s.values)+1 > limitSecretValues || s.bytes+len(value) > limitSecretBytes {
			if !s.full.Load() {
				stopped = true
			}
			s.full.Store(true)
			continue
		}
		s.values = append(s.values, bytes.Clone(value))
		s.bytes += len(value)
		changed = true
	}
	if changed {
		s.matcher = newMatcher(s.values)
		s.ordered = append([][]byte(nil), s.values...)
		sort.SliceStable(s.ordered, func(i, j int) bool { return len(s.ordered[i]) > len(s.ordered[j]) })
	}
	return stopped
}

func (s *secretSet) holds(value []byte) bool {
	for _, held := range s.values {
		if bytes.Equal(held, value) {
			return true
		}
	}
	return false
}

// redact is the rule of sagat_domain::scripts::redact without its credential shapes, equal to it on every text
// (sdks/SPEC.md section 6.9): when the automaton finds no value the text is the result; otherwise every value, the
// longest first, is replaced by [REDACTED] in the text the previous replacement left.
func (s *secretSet) redact(text string) string {
	if len(text) < limitSecretMinBytes {
		return text
	}
	s.mu.RLock()
	defer s.mu.RUnlock()
	if s.matcher == nil || !s.matcher.occursIn(text) {
		return text
	}
	out := []byte(text)
	placeholder := []byte(redactedText)
	for _, value := range s.ordered {
		if bytes.Contains(out, value) {
			next := bytes.ReplaceAll(out, value, placeholder)
			clear(out)
			out = next
		}
	}
	result := string(out)
	clear(out)
	return result
}

// wipe drops every value, its bytes cleared.
func (s *secretSet) wipe() {
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, value := range s.values {
		clear(value)
	}
	s.values, s.ordered, s.matcher, s.bytes, s.warned = nil, nil, nil, 0, false
	s.full.Store(false)
}

func (s *secretSet) count() int {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return len(s.values)
}

func (s *secretSet) String() string             { return fmt.Sprintf("secretSet(%d values)", s.count()) }
func (s *secretSet) GoString() string           { return s.String() }
func (s *secretSet) Format(f fmt.State, _ rune) { _, _ = fmt.Fprint(f, s.String()) }
func (s *secretSet) LogValue() slog.Value       { return slog.StringValue(s.String()) }
func (p *processStepLogs) String() string {
	return fmt.Sprintf("processStepLogs(%s)", p.secrets.String())
}
func (p *processStepLogs) GoString() string           { return p.String() }
func (p *processStepLogs) Format(f fmt.State, _ rune) { _, _ = fmt.Fprint(f, p.String()) }
func (p *processStepLogs) LogValue() slog.Value       { return slog.StringValue(p.String()) }

// processStepLogs is what the whole process shares: the secret set, the bytes every attempt holds and the warning
// lines already written. The set lives for the process: a host started again, or a second host, still redacts the
// values an earlier attempt resolved, which an activity may keep in memory (sdks/SPEC.md section 6.7, decision
// D041-164). Nothing of it is exported; the host only adds to it.
type processStepLogs struct {
	secrets secretSet
	held    atomic.Int64

	mu     sync.Mutex
	warned map[string]bool
}

var processCapture = &processStepLogs{warned: map[string]bool{}}

// warnOnce reports whether this line is written for the first time in the process.
func (p *processStepLogs) warnOnce(line string) bool {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.warned[line] {
		return false
	}
	p.warned[line] = true
	return true
}

// ---- formatting one call -------------------------------------------------------------------------------------------

// redactor is what formatting needs of the secret set; the tests give it their own.
type redactor interface{ redact(text string) string }

// givenField is a field of a call before redaction: its original key, and its value on the wire (a string as
// formatted, not yet redacted nor made well formed) or, for a value that is not a scalar, the value itself, which
// buildFields encodes from a redacted copy.
type givenField struct {
	key   string
	field *pb.StepLogField
	other any
}

// keyedText is an exception text of a call and the original key of the attribute it came from.
type keyedText struct {
	key  string
	text string
}

func stringField(text string) *pb.StepLogField {
	return &pb.StepLogField{Value: &pb.StepLogField_StringValue{StringValue: text}}
}

func intField(n int64) *pb.StepLogField {
	return &pb.StepLogField{Value: &pb.StepLogField_IntValue{IntValue: n}}
}

func floatField(f float64) *pb.StepLogField {
	switch {
	case math.IsNaN(f):
		return stringField("NaN")
	case math.IsInf(f, 1):
		return stringField("Infinity")
	case math.IsInf(f, -1):
		return stringField("-Infinity")
	}
	return &pb.StepLogField{Value: &pb.StepLogField_DoubleValue{DoubleValue: f}}
}

func uintField(u uint64) *pb.StepLogField {
	if u <= math.MaxInt64 {
		return intField(int64(u))
	}
	return stringField(strconv.FormatUint(u, 10))
}

// fieldOf is a slog value as the wire carries it, before redaction (sdks/SPEC.md section 6.9: a string, an integer
// that fits 64 bits, a larger one as its decimal string, a double, a boolean; a null is dropped; a byte buffer is its
// UTF-8 text; anything else is its compact JSON text from a redacted copy, else its string form). A nil field and a
// nil other drop it.
func fieldOf(value slog.Value) (*pb.StepLogField, any) {
	switch value.Kind() {
	case slog.KindString:
		return stringField(value.String()), nil
	case slog.KindInt64:
		return intField(value.Int64()), nil
	case slog.KindUint64:
		return uintField(value.Uint64()), nil
	case slog.KindFloat64:
		return floatField(value.Float64()), nil
	case slog.KindBool:
		return &pb.StepLogField{Value: &pb.StepLogField_BoolValue{BoolValue: value.Bool()}}, nil
	case slog.KindDuration:
		return intField(int64(value.Duration())), nil
	}
	return anyField(value.Any())
}

var (
	jsonMarshaler = reflect.TypeFor[json.Marshaler]()
	textMarshaler = reflect.TypeFor[encoding.TextMarshaler]()
	jsonNumber    = reflect.TypeFor[json.Number]()
)

func anyField(value any) (*pb.StepLogField, any) {
	if value == nil {
		return nil, nil
	}
	if n, ok := value.(*big.Int); ok {
		if n == nil {
			return nil, nil
		}
		if n.IsInt64() {
			return intField(n.Int64()), nil
		}
		return stringField(n.String()), nil
	}
	reflected := reflect.ValueOf(value)
	if reflected.Type().Implements(jsonMarshaler) || reflected.Type().Implements(textMarshaler) {
		if canBeNil(reflected) && reflected.IsNil() {
			return nil, nil
		}
		return nil, value
	}
	switch reflected.Kind() {
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return intField(reflected.Int()), nil
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		return uintField(reflected.Uint()), nil
	case reflect.Float32, reflect.Float64:
		return floatField(reflected.Float()), nil
	case reflect.Bool:
		return &pb.StepLogField{Value: &pb.StepLogField_BoolValue{BoolValue: reflected.Bool()}}, nil
	case reflect.String:
		return stringField(reflected.String()), nil
	case reflect.Slice:
		if reflected.IsNil() {
			return nil, nil
		}
		if reflected.Type().Elem().Kind() == reflect.Uint8 {
			// A byte buffer is its UTF-8 text, never numbers or base64 (decision D041-164).
			return stringField(lenient(string(reflected.Bytes()))), nil
		}
	case reflect.Pointer, reflect.Interface, reflect.Map:
		if reflected.IsNil() {
			return nil, nil
		}
	}
	return nil, value
}

func canBeNil(value reflect.Value) bool {
	switch value.Kind() {
	case reflect.Pointer, reflect.Interface, reflect.Map, reflect.Slice, reflect.Func, reflect.Chan:
		return true
	}
	return false
}

// errNotEncoded says that a value has no JSON text from a redacted copy (a cycle, a function, a channel).
var errNotEncoded = errors.New("not encoded")

// otherText is the text of a value that is not a scalar: its compact JSON text, encoded from a copy whose every string
// and every key was redacted first (an encoder's escaping of a quote, a backslash or a control character would hide a
// value from redaction; decision D041-164), else its string form. buildFields redacts the text again.
func otherText(value any, secrets redactor) string {
	if copied, err := redactedCopy(reflect.ValueOf(value), secrets, 0); err == nil {
		if text, ok := compactJSON(copied); ok {
			return text
		}
	}
	switch reflect.ValueOf(value).Kind() {
	case reflect.Map, reflect.Slice, reflect.Array, reflect.Struct, reflect.Pointer, reflect.Interface:
		// fmt does not stop at a cycle of maps or slices: their type is all that is said of them.
		return fmt.Sprintf("%T", value)
	}
	return stringForm(value)
}

// redactedCopy is a JSON-shaped copy of a value (maps of string keys, lists, strings, numbers, booleans, null) in
// which every string and every key is redacted and every byte buffer, at any depth, is its UTF-8 text, never base64. A
// struct is copied field by field as encoding/json names them (structCopy); a value with its own JSON or text form is
// encoded by encoding/json, decoded and copied in turn.
func redactedCopy(value reflect.Value, secrets redactor, depth int) (copied any, err error) {
	if depth > 64 {
		return nil, errNotEncoded
	}
	if !value.IsValid() {
		return nil, nil
	}
	if value.Type() == jsonNumber {
		if text := value.String(); secrets.redact(text) != text {
			return redactedText, nil
		}
		return json.Number(value.String()), nil
	}
	if canBeNil(value) && value.IsNil() {
		return nil, nil
	}
	if value.Type().Implements(jsonMarshaler) || value.Type().Implements(textMarshaler) {
		decoded, err := decodedForm(value.Interface())
		if err != nil {
			return nil, err
		}
		return redactedCopy(reflect.ValueOf(decoded), secrets, depth+1)
	}
	switch value.Kind() {
	case reflect.Struct:
		return structCopy(value, secrets, depth)
	case reflect.Pointer, reflect.Interface:
		return redactedCopy(value.Elem(), secrets, depth+1)
	case reflect.String:
		return secrets.redact(value.String()), nil
	case reflect.Bool:
		return value.Bool(), nil
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return value.Int(), nil
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		return value.Uint(), nil
	case reflect.Float32, reflect.Float64:
		if f := value.Float(); !math.IsNaN(f) && !math.IsInf(f, 0) {
			return f, nil
		}
		return nil, nil // as JSON.stringify writes a number that is not finite
	case reflect.Slice, reflect.Array:
		if value.Kind() == reflect.Slice && value.Type().Elem().Kind() == reflect.Uint8 {
			return secrets.redact(lenient(string(value.Bytes()))), nil
		}
		list := make([]any, value.Len())
		for i := range list {
			if list[i], err = redactedCopy(value.Index(i), secrets, depth+1); err != nil {
				return nil, err
			}
		}
		return list, nil
	case reflect.Map:
		object := make(map[string]any, value.Len())
		iterator := value.MapRange()
		for iterator.Next() {
			key, err := mapKey(iterator.Key())
			if err != nil {
				return nil, err
			}
			if object[secrets.redact(key)], err = redactedCopy(iterator.Value(), secrets, depth+1); err != nil {
				return nil, err
			}
		}
		return object, nil
	}
	return nil, errNotEncoded
}

// jsonField is a field of a struct as encoding/json encodes it: its name, the index path to it through embedded
// structs, and its tag's options.
type jsonField struct {
	name      string
	index     []int
	depth     int
	tagged    bool
	omitEmpty bool
	quoted    bool
}

// jsonFields lists the fields encoding/json encodes for a struct type: exported fields under their tag's name or their
// own, "-" left out, the fields of an untagged embedded struct promoted, and of several fields of one name the
// shallowest, or the one tagged among the shallowest, kept (none when that does not settle it), as encoding/json does.
func jsonFields(t reflect.Type) []jsonField {
	var all []jsonField
	var walk func(t reflect.Type, index []int, depth int, seen map[reflect.Type]bool)
	walk = func(t reflect.Type, index []int, depth int, seen map[reflect.Type]bool) {
		if seen[t] {
			return
		}
		seen[t] = true
		defer delete(seen, t)
		for i := 0; i < t.NumField(); i++ {
			field := t.Field(i)
			tag := field.Tag.Get("json")
			if tag == "-" {
				continue
			}
			name, options, _ := strings.Cut(tag, ",")
			path := append(append([]int(nil), index...), i)
			if field.Anonymous {
				embedded := field.Type
				if embedded.Kind() == reflect.Pointer {
					embedded = embedded.Elem()
				}
				if !field.IsExported() && embedded.Kind() != reflect.Struct {
					continue
				}
				if name == "" && embedded.Kind() == reflect.Struct {
					walk(embedded, path, depth+1, seen)
					continue
				}
			} else if !field.IsExported() {
				continue
			}
			tagged := name != ""
			if !tagged {
				name = field.Name
			}
			all = append(all, jsonField{
				name: name, index: path, depth: depth, tagged: tagged,
				omitEmpty: strings.Contains(","+options+",", ",omitempty,"), quoted: strings.Contains(","+options+",", ",string,"),
			})
		}
	}
	walk(t, nil, 0, map[reflect.Type]bool{})
	byName := map[string][]jsonField{}
	var names []string
	for _, field := range all {
		if _, known := byName[field.name]; !known {
			names = append(names, field.name)
		}
		byName[field.name] = append(byName[field.name], field)
	}
	kept := make([]jsonField, 0, len(names))
	for _, name := range names {
		candidates := byName[name]
		shallowest := candidates[0].depth
		for _, field := range candidates {
			shallowest = min(shallowest, field.depth)
		}
		var top, tagged []jsonField
		for _, field := range candidates {
			if field.depth == shallowest {
				top = append(top, field)
				if field.tagged {
					tagged = append(tagged, field)
				}
			}
		}
		switch {
		case len(top) == 1:
			kept = append(kept, top[0])
		case len(tagged) == 1:
			kept = append(kept, tagged[0])
		}
	}
	return kept
}

// emptyJSON reports whether omitempty leaves a value out, as encoding/json decides it.
func emptyJSON(value reflect.Value) bool {
	switch value.Kind() {
	case reflect.Array, reflect.Map, reflect.Slice, reflect.String:
		return value.Len() == 0
	case reflect.Bool:
		return !value.Bool()
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return value.Int() == 0
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		return value.Uint() == 0
	case reflect.Float32, reflect.Float64:
		return value.Float() == 0
	case reflect.Interface, reflect.Pointer:
		return value.IsNil()
	}
	return false
}

// structCopy is the redacted copy of a struct, built by reflection under the names encoding/json gives its fields, so
// that a byte buffer in a field, in an embedded struct or behind a pointer is its UTF-8 text and never base64.
func structCopy(value reflect.Value, secrets redactor, depth int) (any, error) {
	object := map[string]any{}
	for _, field := range jsonFields(value.Type()) {
		member, present := value, true
		for n, i := range field.index {
			if n > 0 && member.Kind() == reflect.Pointer {
				if member.IsNil() {
					present = false
					break
				}
				member = member.Elem()
			}
			member = member.Field(i)
		}
		if !present || (field.omitEmpty && emptyJSON(member)) {
			continue
		}
		copied, err := redactedCopy(member, secrets, depth+1)
		if err != nil {
			return nil, err
		}
		if field.quoted {
			switch member.Kind() {
			case reflect.Bool, reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64, reflect.Uint,
				reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr, reflect.Float32, reflect.Float64, reflect.String:
				if text, ok := compactJSON(copied); ok {
					copied = text
				}
			}
		}
		object[secrets.redact(field.name)] = copied
	}
	return object, nil
}

// mapKey is the text encoding/json gives a map key.
func mapKey(key reflect.Value) (string, error) {
	if key.Kind() == reflect.String {
		return key.String(), nil
	}
	if key.Type().Implements(textMarshaler) {
		text, err := key.Interface().(encoding.TextMarshaler).MarshalText()
		return string(text), err
	}
	switch key.Kind() {
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return strconv.FormatInt(key.Int(), 10), nil
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		return strconv.FormatUint(key.Uint(), 10), nil
	}
	return "", errNotEncoded
}

// decodedForm is a value encoded by encoding/json and decoded into maps, lists and json.Number; a marshaller that
// fails or panics gives an error.
func decodedForm(value any) (decoded any, err error) {
	defer func() {
		if recover() != nil {
			decoded, err = nil, errNotEncoded
		}
	}()
	text, err := json.Marshal(value)
	if err != nil {
		return nil, err
	}
	decoder := json.NewDecoder(bytes.NewReader(text))
	decoder.UseNumber()
	err = decoder.Decode(&decoded)
	return decoded, err
}

// compactJSON is the value's JSON text with no insignificant whitespace and no HTML escaping, or false when it has
// none (a cycle, an unsupported type, a marshaller that fails or panics).
func compactJSON(value any) (text string, ok bool) {
	defer func() {
		if recover() != nil {
			text, ok = "", false
		}
	}()
	var out bytes.Buffer
	encoder := json.NewEncoder(&out)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(value); err != nil {
		return "", false
	}
	return strings.TrimSuffix(out.String(), "\n"), true
}

// stringForm is the value as fmt prints it; a value whose printing panics past fmt's own guard is its type.
func stringForm(value any) (text string) {
	defer func() {
		if recover() != nil {
			text = fmt.Sprintf("%T", value)
		}
	}()
	return fmt.Sprint(value)
}

// exceptionText is the exception text of an error (sdks/SPEC.md section 6.9 for Go): err.Error(), or %+v when the
// error implements fmt.Formatter (the stack of the errors packages that print one). It never panics.
func exceptionText(err error) (text string) {
	defer func() {
		if recover() != nil {
			text = fmt.Sprintf("%T", err)
		}
	}()
	if _, formats := err.(fmt.Formatter); formats {
		return fmt.Sprintf("%+v", err)
	}
	return err.Error()
}

func errorText(err error) (text string) {
	defer func() {
		if recover() != nil {
			text = fmt.Sprintf("%T", err)
		}
	}()
	return err.Error()
}

// collected is what the attributes of one call give: its fields and its exception texts.
type collected struct {
	fields     []givenField
	exceptions []keyedText
}

// flatten adds one attribute: a group is flattened as group.key, an attribute whose value is an error gives an
// exception text and is also kept as a field, and the empty attribute is ignored, as slog's handlers ignore it.
func (c *collected) flatten(prefix string, attr slog.Attr) {
	attr.Value = attr.Value.Resolve()
	if attr.Equal(slog.Attr{}) {
		return
	}
	if attr.Value.Kind() == slog.KindGroup {
		inner := prefix
		if attr.Key != "" {
			inner = prefix + attr.Key + "."
		}
		for _, member := range attr.Value.Group() {
			c.flatten(inner, member)
		}
		return
	}
	key := prefix + attr.Key
	if attr.Value.Kind() == slog.KindAny {
		if err, isError := attr.Value.Any().(error); isError && err != nil {
			c.exceptions = append(c.exceptions, keyedText{key: key, text: exceptionText(err)})
			c.fields = append(c.fields, givenField{key: key, field: stringField(errorText(err))})
			return
		}
	}
	if field, other := fieldOf(attr.Value); field != nil || other != nil {
		c.fields = append(c.fields, givenField{key: key, field: field, other: other})
	}
}

// exceptionOf joins the exception texts of a call in the order of their original keys' UTF-8 bytes.
func exceptionOf(texts []keyedText) string {
	switch len(texts) {
	case 0:
		return ""
	case 1:
		return texts[0].text
	}
	sort.SliceStable(texts, func(i, j int) bool { return texts[i].key < texts[j].key })
	parts := make([]string, len(texts))
	for i, text := range texts {
		parts[i] = text.text
	}
	return strings.Join(parts, "\n")
}

// jsonStringSize is the size of a text as the canonical JSON of data-model.md section 5 writes it: only the quote, the
// backslash and U+0000 to U+001F are escaped.
func jsonStringSize(text string) int {
	size := 2
	for i := 0; i < len(text); i++ {
		switch b := text[i]; {
		case b == '"' || b == '\\':
			size += 2
		case b < 0x20:
			size += 6
		default:
			size++
		}
	}
	return size
}

func jsonValueSize(field *pb.StepLogField) int {
	switch value := field.GetValue().(type) {
	case *pb.StepLogField_StringValue:
		return jsonStringSize(value.StringValue)
	case *pb.StepLogField_IntValue:
		return len(strconv.FormatInt(value.IntValue, 10))
	case *pb.StepLogField_DoubleValue:
		return len(jsonx.NumberText(value.DoubleValue))
	case *pb.StepLogField_BoolValue:
		if value.BoolValue {
			return 4
		}
		return 5
	}
	return 4
}

// buildFields makes the fields of a call by data-model.md section 5: taken in the order of their original keys' UTF-8
// bytes, keys redacted and made unique with ~2, ~3..., string values and the text of numbers redacted, then kept in
// key order while they hold at most 32 keys and 2,048 bytes of canonical JSON.
func buildFields(given []givenField, secrets redactor) ([]*pb.StepLogField, bool) {
	if len(given) == 0 {
		return nil, false
	}
	sort.SliceStable(given, func(i, j int) bool { return given[i].key < given[j].key })
	used := make(map[string]bool, len(given))
	made := make([]*pb.StepLogField, 0, len(given))
	for _, one := range given {
		redactedKey := clean(one.key, secrets)
		key := redactedKey
		for n := 2; used[key]; n++ {
			key = redactedKey + "~" + strconv.Itoa(n)
		}
		used[key] = true
		field := one.field
		if field == nil {
			field = stringField(otherText(one.other, secrets))
		}
		field.Key = key
		switch value := field.GetValue().(type) {
		case *pb.StepLogField_StringValue:
			value.StringValue = clean(value.StringValue, secrets)
		case *pb.StepLogField_IntValue:
			if text := strconv.FormatInt(value.IntValue, 10); secrets.redact(text) != text {
				field.Value = &pb.StepLogField_StringValue{StringValue: redactedText}
			}
		case *pb.StepLogField_DoubleValue:
			if text := jsonx.NumberText(value.DoubleValue); secrets.redact(text) != text {
				field.Value = &pb.StepLogField_StringValue{StringValue: redactedText}
			}
		}
		made = append(made, field)
	}
	sort.Slice(made, func(i, j int) bool { return made[i].Key < made[j].Key })
	size := 2 // the braces
	for i, field := range made {
		entry := jsonStringSize(field.Key) + 1 + jsonValueSize(field)
		if i > 0 {
			entry++ // the comma
		}
		if i+1 > limitFieldKeys || size+entry > limitFieldBytes {
			return made[:i:i], true
		}
		size += entry
	}
	return made, false
}

// buildRecord is steps 3 to 6 of a call (sdks/SPEC.md section 6.9): formatted, redacted, made well formed, cut.
func buildRecord(sequence uint64, epochMs int64, level slog.Level, message string, got collected, secrets redactor) *pb.StepLogRecord {
	cutMessage, messageCut := cutUTF8(clean(message, secrets), limitMessageBytes)
	cutException, exceptionCut := cutUTF8(clean(exceptionOf(got.exceptions), secrets), limitMessageBytes)
	fields, fieldsCut := buildFields(got.fields, secrets)
	return &pb.StepLogRecord{
		Sequence: sequence, WorkerTimeEpochMs: epochMs, Level: wireLevel(level), Message: cutMessage,
		Fields: fields, Exception: cutException, Truncated: messageCut || exceptionCut || fieldsCut,
	}
}

// ---- the logger of an attempt --------------------------------------------------------------------------------------

// stepLogHandler is the per-attempt slog.Handler of TaskFrom(ctx).Logger: it captures each record at or above the
// capture level of a capturing attempt and forwards it to the host's handler only when that one is enabled, so the host
// logs exactly what it logged before. The attributes the SDK added before it (worker, task, activity, run, node,
// attempt) are the host handler's and are not captured; those of With and the call are.
type stepLogHandler struct {
	host    slog.Handler
	capture *attemptCapture
	attrs   []prefixedAttr
	prefix  string
}

type prefixedAttr struct {
	prefix string
	attr   slog.Attr
}

func (h *stepLogHandler) Enabled(ctx context.Context, level slog.Level) bool {
	return h.capture.accepts(level) || h.host.Enabled(ctx, level)
}

func (h *stepLogHandler) Handle(ctx context.Context, record slog.Record) error {
	if h.capture.accepts(record.Level) {
		h.capture.log(record.Level, record.Message, func() collected {
			var got collected
			for _, attr := range h.attrs {
				got.flatten(attr.prefix, attr.attr)
			}
			record.Attrs(func(attr slog.Attr) bool {
				got.flatten(h.prefix, attr)
				return true
			})
			return got
		})
	}
	if h.host.Enabled(ctx, record.Level) {
		return h.host.Handle(ctx, record)
	}
	return nil
}

func (h *stepLogHandler) WithAttrs(attrs []slog.Attr) slog.Handler {
	if len(attrs) == 0 {
		return h
	}
	next := *h
	next.host = h.host.WithAttrs(attrs)
	if h.capture != nil {
		next.attrs = make([]prefixedAttr, 0, len(h.attrs)+len(attrs))
		next.attrs = append(next.attrs, h.attrs...)
		for _, attr := range attrs {
			next.attrs = append(next.attrs, prefixedAttr{prefix: h.prefix, attr: attr})
		}
	}
	return &next
}

func (h *stepLogHandler) WithGroup(name string) slog.Handler {
	if name == "" {
		return h
	}
	next := *h
	next.host = h.host.WithGroup(name)
	next.prefix = h.prefix + name + "."
	return &next
}

// ---- one attempt ---------------------------------------------------------------------------------------------------

// captureSession is the capture state a registration gives a session: shared by its attempts, stopped by
// STOP_CAPTURE until the next registration.
type captureSession struct {
	enabled bool
	stopped atomic.Bool
}

// appendOutcome is the answer of one AppendStepLogs, or the gRPC status of its failure.
type appendOutcome struct {
	answered   bool
	verdict    pb.StepLogVerdict
	stopReason pb.StepLogStopReason
	retryMs    int32          // with RETRY_LATER
	status     string         // the gRPC status name when not answered
	retryAfter *time.Duration // the retry-after-ms trailer of a failure
}

// captureHost is what an attempt needs of its host.
type captureHost struct {
	life     context.Context // the link's life: sends end with it
	session  func() (workerID, token string)
	append   func(ctx context.Context, request *pb.AppendStepLogsRequest, timeout time.Duration) appendOutcome
	timeout  time.Duration
	dropped  func(reason string, records int)
	warnOnce func(line string)
	ticker   func() (<-chan time.Time, func())
}

// attemptIdentity names the attempt as Heartbeat names it.
type attemptIdentity struct {
	taskID, runID, stepID, leaseToken string
	attempt                           int32
	generation                        int64
	instance                          *pb.ExecutionInstanceIdentity
}

type heldRecord struct {
	record *pb.StepLogRecord
	size   int
}

type stepLogBatch struct {
	held      []heldRecord
	done      chan struct{}
	firstSent time.Time
}

const (
	captureOn = iota
	captureFinalOnly
	captureStopped
)

// attemptCapture is the capture of one attempt: its sequences, its buffer, its sender and, at its end, its carry.
type attemptCapture struct {
	host        captureHost
	session     *captureSession
	identity    attemptIdentity
	level       pb.StepLogLevel
	sends       context.Context
	cancelSends context.CancelFunc
	ended       atomic.Bool

	mu             sync.Mutex
	state          int
	last           uint64
	buffer         []heldRecord
	bufferBytes    int
	inFlight       *stepLogBatch
	batchesStopped bool
	closed         bool
	endedAt        time.Time
	ticking        bool
	tickQuit       chan struct{}
	tickStop       func()
	drops          map[string]int
}

func newAttemptCapture(host captureHost, session *captureSession, identity attemptIdentity, level slog.Level) *attemptCapture {
	sends, cancel := context.WithCancel(host.life)
	return &attemptCapture{host: host, session: session, identity: identity, level: wireLevel(level), sends: sends, cancelSends: cancel, drops: map[string]int{}}
}

// accepts reports whether a call at this level is captured: the attempt captures, its handler has not ended, and the
// level is at or above the capture level.
func (c *attemptCapture) accepts(level slog.Level) bool {
	return c != nil && !c.ended.Load() && wireLevel(level) >= c.level
}

func (c *attemptCapture) count(reason string, records int) {
	if records > 0 && c.drops != nil {
		c.drops[reason] += records
	}
}

func (c *attemptCapture) stoppedLocked() bool {
	return c.state == captureStopped || c.session.stopped.Load()
}

// setFullLocked reports whether the process's capture stopped (the secret set is full); then nothing is sent at all,
// and what the attempt buffered is dropped as secret_set_full (decision D041-164).
func (c *attemptCapture) setFullLocked() bool {
	if !processCapture.secrets.full.Load() {
		return false
	}
	c.dropBufferLocked(dropSecretSetFull)
	return true
}

// log is a logger call (sdks/SPEC.md section 6.9, "A call, in order"). collect gives the call's fields and exception
// texts; it runs outside the attempt's lock, so that a value whose formatting logs again cannot block the call. log
// never panics and never waits for the network.
func (c *attemptCapture) log(level slog.Level, message string, collect func() collected) {
	defer func() { _ = recover() }() // a bug of the capture path costs the record, never the activity
	if !c.accepts(level) {
		return
	}
	sequence, ok := c.take()
	if !ok {
		return
	}
	record := buildRecord(sequence, time.Now().UnixMilli(), level, message, collect(), &processCapture.secrets)
	c.buffered(record, proto.Size(record))
}

// take is steps 1 and 2: the sequence and the early drops; false when the call takes none or is dropped.
func (c *attemptCapture) take() (uint64, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.ended.Load() || c.closed || c.stoppedLocked() {
		return 0, false
	}
	c.last++
	switch {
	case c.state == captureFinalOnly:
		c.count(dropAfterCeiling, 1)
	case processCapture.secrets.full.Load():
		c.count(dropSecretSetFull, 1)
	case len(c.buffer) >= limitAttemptRecords || c.bufferBytes >= limitAttemptBytes:
		c.count(dropBufferFull, 1)
	case processCapture.held.Load() >= limitProcessBytes:
		c.count(dropProcessBound, 1)
	default:
		return c.last, true
	}
	return 0, false
}

// buffered is step 6: the record joins the buffer in the order of its sequence, and the call that buffers the 256th
// record cuts the batch.
func (c *attemptCapture) buffered(record *pb.StepLogRecord, size int) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed || c.state != captureOn || c.stoppedLocked() {
		return
	}
	at := len(c.buffer)
	for at > 0 && c.buffer[at-1].record.GetSequence() > record.GetSequence() {
		at--
	}
	c.buffer = append(c.buffer, heldRecord{})
	copy(c.buffer[at+1:], c.buffer[at:])
	c.buffer[at] = heldRecord{record: record, size: size}
	c.bufferBytes += size
	processCapture.held.Add(int64(size))
	if !c.ticking && !c.ended.Load() {
		c.startTick()
	}
	if len(c.buffer) >= limitFlushRecords && c.inFlight == nil && !c.batchesStopped && !c.setFullLocked() {
		c.send(c.cut(len(c.buffer)))
	}
}

func (c *attemptCapture) startTick() {
	c.ticking = true
	ticks, stop := c.host.ticker()
	quit := make(chan struct{})
	c.tickQuit, c.tickStop = quit, stop
	go func() {
		for {
			select {
			case <-quit:
				return
			case <-ticks:
				c.onTick()
			}
		}
	}()
}

func (c *attemptCapture) stopTickLocked() {
	if c.tickQuit != nil {
		close(c.tickQuit)
		c.tickStop()
		c.tickQuit, c.tickStop = nil, nil
	}
}

func (c *attemptCapture) onTick() {
	defer func() { _ = recover() }()
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed || c.ended.Load() || c.inFlight != nil || len(c.buffer) == 0 || c.state != captureOn || c.stoppedLocked() || c.batchesStopped || c.setFullLocked() {
		return
	}
	c.send(c.cut(len(c.buffer)))
}

func (c *attemptCapture) request(records []*pb.StepLogRecord, workerID, token string) *pb.AppendStepLogsRequest {
	return &pb.AppendStepLogsRequest{
		WorkerId: workerID, SessionToken: token, TaskId: c.identity.taskID, Generation: c.identity.generation,
		LeaseToken: c.identity.leaseToken, RunId: c.identity.runID, StepId: c.identity.stepID, Attempt: c.identity.attempt,
		Instance: c.identity.instance, Records: records,
	}
}

// cut takes the oldest of the first limit buffered records that one batch holds: at most 500, and at most 192 KiB of
// encoded AppendStepLogsRequest.
func (c *attemptCapture) cut(limit int) []heldRecord {
	workerID, token := c.host.session()
	size := proto.Size(c.request(nil, workerID, token))
	n := 0
	for n < min(limit, limitBatchRecords) {
		next := delimited(c.buffer[n].size)
		if n > 0 && size+next > limitBatchBytes {
			break
		}
		size += next
		n++
	}
	held := append([]heldRecord(nil), c.buffer[:n]...)
	c.buffer = append([]heldRecord(nil), c.buffer[n:]...)
	for _, one := range held {
		c.bufferBytes -= one.size
	}
	return held
}

func release(held []heldRecord) {
	total := 0
	for _, one := range held {
		total += one.size
	}
	processCapture.held.Add(-int64(total))
}

// send starts a batch; the caller holds the lock.
func (c *attemptCapture) send(held []heldRecord) {
	if len(held) == 0 {
		return
	}
	batch := &stepLogBatch{held: held, done: make(chan struct{}), firstSent: time.Now()}
	c.inFlight = batch
	go c.deliver(batch, false, c.host.timeout)
}

// finish lets a batch go: its bytes are released, and the answer cuts the next batch when 256 or more are buffered.
func (c *attemptCapture) finish(batch *stepLogBatch) {
	defer close(batch.done)
	c.mu.Lock()
	defer c.mu.Unlock()
	release(batch.held)
	if c.inFlight == batch {
		c.inFlight = nil
	}
	if !c.closed && !c.ended.Load() && c.state == captureOn && !c.stoppedLocked() && !c.batchesStopped &&
		len(c.buffer) >= limitFlushRecords && c.inFlight == nil && !c.setFullLocked() {
		c.send(c.cut(len(c.buffer)))
	}
}

func (c *attemptCapture) isClosed() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.closed
}

func (c *attemptCapture) failed(batch *stepLogBatch) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.count(dropSendFailed, len(batch.held))
}

// mayStillSend reports whether the batch may still be sent: the attempt is not closed (a shutdown batch, cut when the
// grace ended, is sent even when its attempt closes meanwhile) and the process's capture has not stopped; a batch the
// full set stops is dropped as secret_set_full.
func (c *attemptCapture) mayStillSend(batch *stepLogBatch, once bool) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed && !once {
		return false
	}
	if processCapture.secrets.full.Load() {
		c.count(dropSecretSetFull, len(batch.held))
		return false
	}
	return true
}

// pause waits d, or returns false when the attempt's sends end first.
func (c *attemptCapture) pause(d time.Duration) bool {
	timer := time.NewTimer(d)
	defer timer.Stop()
	select {
	case <-timer.C:
		return true
	case <-c.sends.Done():
		return false
	}
}

func clampRetry(d time.Duration) time.Duration {
	return min(5*time.Second, max(50*time.Millisecond, d))
}

func jittered(d time.Duration) time.Duration {
	return time.Duration(float64(d) * (0.8 + rand.Float64()*0.4))
}

// deliver sends one batch until an answer settles it (contracts.md section 4). once sends it a single time, as the
// shutdown does: an answer that would send it again drops it instead.
func (c *attemptCapture) deliver(batch *stepLogBatch, once bool, within time.Duration) {
	defer c.finish(batch)
	defer func() { _ = recover() }()
	records := make([]*pb.StepLogRecord, len(batch.held))
	for i, one := range batch.held {
		records[i] = one.record
	}
	workerID, token := c.host.session()
	request := c.request(records, workerID, token)
	retries, sessionResent := 0, false
	for {
		if !c.mayStillSend(batch, once) {
			return
		}
		left := limitBatchAge - time.Since(batch.firstSent)
		if left <= 0 {
			c.failed(batch)
			return
		}
		sends := c.sends
		if once {
			// The shutdown batch lives as long as the channel, not as its attempt, which the shutdown is ending.
			sends = c.host.life
		}
		outcome := c.host.append(sends, request, min(within, left))
		if once {
			// Sent once, with no resend: a batch the answer did not take is counted as not sent.
			if !outcome.answered || outcome.verdict != pb.StepLogVerdict_STEP_LOG_VERDICT_CONTINUE {
				c.failed(batch)
			}
			return
		}
		if c.isClosed() {
			return
		}
		var wait time.Duration
		if outcome.answered {
			switch outcome.verdict {
			case pb.StepLogVerdict_STEP_LOG_VERDICT_CONTINUE:
				return
			case pb.StepLogVerdict_STEP_LOG_VERDICT_FINAL_ONLY:
				c.finalOnly()
				return
			case pb.StepLogVerdict_STEP_LOG_VERDICT_STOP_ATTEMPT:
				c.stop()
				return
			case pb.StepLogVerdict_STEP_LOG_VERDICT_STOP_CAPTURE:
				nowID, now := c.host.session()
				if outcome.stopReason == pb.StepLogStopReason_STEP_LOG_STOP_REASON_SESSION && !sessionResent && !once && now != token {
					sessionResent, token = true, now
					request = c.request(records, nowID, now)
					continue
				}
				c.stopSession("the platform stopped step-log capture for this session")
				return
			case pb.StepLogVerdict_STEP_LOG_VERDICT_RETRY_LATER:
				wait = clampRetry(time.Duration(outcome.retryMs) * time.Millisecond)
			}
		} else {
			switch outcome.status {
			case "RESOURCE_EXHAUSTED":
				wait = 200 * time.Millisecond
				if outcome.retryAfter != nil {
					wait = *outcome.retryAfter
				}
				wait = clampRetry(wait)
			case "INVALID_ARGUMENT", "CANCELLED":
				c.failed(batch)
				return
			case "UNAUTHENTICATED", "PERMISSION_DENIED":
				c.stopSession("the platform refused step-log capture for this session")
				return
			case "UNIMPLEMENTED":
				c.mu.Lock()
				c.batchesStopped = true
				c.mu.Unlock()
				c.failed(batch)
				return
			}
		}
		if once {
			c.failed(batch)
			return
		}
		if wait > 0 {
			// RETRY_LATER and RESOURCE_EXHAUSTED: the same batch again, not counted as a retry.
			if !c.pause(wait) {
				return
			}
			continue
		}
		// UNAVAILABLE, DEADLINE_EXCEEDED, a transport failure, any other status or verdict: two retries.
		if retries >= len(stepLogRetryDelays) {
			c.failed(batch)
			return
		}
		if !c.pause(jittered(stepLogRetryDelays[retries])) {
			return
		}
		retries++
	}
}

func (c *attemptCapture) dropBufferLocked(reason string) {
	c.count(reason, len(c.buffer))
	release(c.buffer)
	c.buffer, c.bufferBytes = nil, 0
}

func (c *attemptCapture) finalOnly() {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.state == captureOn {
		c.state = captureFinalOnly
		c.dropBufferLocked(dropAfterCeiling)
	}
}

func (c *attemptCapture) stop() {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.state = captureStopped
	// Records the server will not take: counted as records whose sending failed.
	c.dropBufferLocked(dropSendFailed)
}

func (c *attemptCapture) stopSession(line string) {
	c.session.stopped.Store(true)
	c.host.warnOnce(line)
	c.stop()
}

// carryFits is how many of the newest buffered records fit one carry.
func (c *attemptCapture) carryFits() int {
	size := 1 + protowire.SizeVarint(c.last)
	n := 0
	for i := len(c.buffer) - 1; i >= 0 && n < limitCarryRecords; i-- {
		next := delimited(c.buffer[i].size)
		if size+next > limitCarryBytes {
			break
		}
		size += next
		n++
	}
	return n
}

// end is the end of the handler: its logger captures nothing more.
func (c *attemptCapture) end() {
	if c == nil {
		return
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	c.endLocked()
}

func (c *attemptCapture) endLocked() {
	if !c.ended.Load() {
		c.ended.Store(true)
		c.endedAt = time.Now()
	}
	c.stopTickLocked()
}

// carry is the carry of the attempt's report (sdks/SPEC.md section 6.9, "The carry"), after at most 2 seconds from
// the handler's end of waiting for its batch in flight and sending what does not fit one carry; nil when the attempt
// sends none. It closes the attempt and never panics.
func (c *attemptCapture) carry() (carried *pb.StepLogCarry) {
	if c == nil {
		return nil
	}
	c.end()
	defer c.close()
	defer func() {
		if recover() != nil {
			carried = nil
		}
	}()
	c.mu.Lock()
	locked := true
	defer func() {
		if locked {
			c.mu.Unlock()
		}
	}()
	if c.closed || c.last == 0 || c.stoppedLocked() {
		return nil
	}
	if c.state == captureFinalOnly || processCapture.secrets.full.Load() {
		return c.markOnlyLocked()
	}
	deadline := c.endedAt.Add(limitReportWait)
	for {
		if batch := c.inFlight; batch != nil {
			left := time.Until(deadline)
			if left <= 0 {
				break
			}
			c.mu.Unlock()
			locked = false
			timer := time.NewTimer(left)
			late := false
			select {
			case <-batch.done:
			case <-timer.C:
				late = true
			}
			timer.Stop()
			c.mu.Lock()
			locked = true
			if late {
				break
			}
			if c.stoppedLocked() {
				return nil
			}
			// The answer may have changed the state while the report waited.
			if c.state == captureFinalOnly || processCapture.secrets.full.Load() {
				return c.markOnlyLocked()
			}
			continue
		}
		fits := c.carryFits()
		if fits == len(c.buffer) || c.batchesStopped || !time.Now().Before(deadline) {
			break
		}
		if processCapture.secrets.full.Load() {
			return c.markOnlyLocked()
		}
		c.send(c.cut(len(c.buffer) - fits))
	}
	if beyond := len(c.buffer) - c.carryFits(); beyond > 0 {
		c.count(dropReportDeadline, beyond)
		release(c.buffer[:beyond])
		c.buffer = append([]heldRecord(nil), c.buffer[beyond:]...)
	}
	records := make([]*pb.StepLogRecord, len(c.buffer))
	for i, one := range c.buffer {
		records[i] = one.record
	}
	return &pb.StepLogCarry{Records: records, HighestSequence: c.last}
}

func (c *attemptCapture) markOnlyLocked() *pb.StepLogCarry {
	reason := dropSecretSetFull
	if c.state == captureFinalOnly {
		reason = dropAfterCeiling
	}
	c.dropBufferLocked(reason)
	return &pb.StepLogCarry{HighestSequence: c.last}
}

// close ends the attempt: nothing more is sent, its records are let go, and its drops are counted once.
func (c *attemptCapture) close() {
	if c == nil {
		return
	}
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return
	}
	c.closed = true
	c.endLocked()
	c.cancelSends()
	release(c.buffer)
	c.buffer, c.bufferBytes = nil, 0
	drops := c.drops
	c.drops = nil
	c.mu.Unlock()
	reasons := make([]string, 0, len(drops))
	for reason := range drops {
		reasons = append(reasons, reason)
	}
	sort.Strings(reasons)
	for _, reason := range reasons {
		c.host.dropped(reason, drops[reason])
	}
}

// shutdownBatch is the one batch an attempt still running sends when the host stops after its grace: its buffered
// records, cut now; nil when there is none to send.
func (c *attemptCapture) shutdownBatch() *stepLogBatch {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.endLocked()
	if c.closed || c.stoppedLocked() || c.state != captureOn || c.batchesStopped || c.inFlight != nil || len(c.buffer) == 0 || c.setFullLocked() {
		return nil
	}
	batch := &stepLogBatch{held: c.cut(len(c.buffer)), done: make(chan struct{}), firstSent: time.Now()}
	c.inFlight = batch
	return batch
}
