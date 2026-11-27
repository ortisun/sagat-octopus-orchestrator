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
	"unsafe"

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

// lenient is the UTF-8 text of bytes as the SDK hands it out, everywhere (the secret set, Secrets.Text, every
// captured string and key): each byte utf8.DecodeRune refuses replaced by one U+FFFD, as encoding/json writes it, so
// that a secret that is not UTF-8 has one form wherever it is met. A Go string has no surrogates; an invalid sequence
// is their counterpart.
func lenient(text string) string {
	if utf8.ValidString(text) {
		return text
	}
	var out strings.Builder
	out.Grow(len(text) + 8)
	for i := 0; i < len(text); {
		r, size := utf8.DecodeRuneInString(text[i:])
		if r == utf8.RuneError && size == 1 {
			out.WriteString("\uFFFD")
		} else {
			out.WriteString(text[i : i+size])
		}
		i += size
	}
	return out.String()
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

// redactLenient redacts a text as formatted, makes it lenient and, when that changed it, redacts it again, so that a
// value the set holds in its lenient form (a secret that is not UTF-8, logged from its raw bytes) is found too.
func redactLenient(text string, secrets redactor) string {
	redacted := secrets.redact(text)
	formed := lenient(redacted)
	if formed != redacted {
		formed = secrets.redact(formed)
	}
	return formed
}

// clean is steps 4 and 5 for one text: redacted as formatted, made lenient and redacted again, then NUL replaced, so
// that a value holding a NUL, an invalid sequence or both is still found.
func clean(text string, secrets redactor) string {
	formed := redactLenient(text, secrets)
	if strings.IndexByte(formed, 0) < 0 {
		return formed
	}
	return strings.ReplaceAll(formed, "\x00", "\uFFFD")
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
	// The automaton and the ordered values are read under the lock and used after it: add replaces them, never
	// changes them, so a long redaction does not hold back a resolution.
	s.mu.RLock()
	found, ordered := s.matcher, s.ordered
	s.mu.RUnlock()
	if found == nil || !found.occursIn(text) {
		return text
	}
	out := []byte(text)
	placeholder := []byte(redactedText)
	for _, value := range ordered {
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

// wipe drops every value (for the tests: the set lives for the process). The bytes are not cleared, since a redaction
// running outside the lock may still read them.
func (s *secretSet) wipe() {
	s.mu.Lock()
	defer s.mu.Unlock()
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
	case reflect.Slice, reflect.Array:
		if reflected.Kind() == reflect.Slice && reflected.IsNil() {
			return nil, nil
		}
		// A byte buffer or array is its UTF-8 text, a rune slice or array its string: never numbers or base64.
		if text, ok := textOfElements(reflected); ok {
			return stringField(text), nil
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
	if copied, err := redactedCopy(reflect.ValueOf(value), secrets, 0, map[visit]bool{}); err == nil {
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
// which every string and every key is redacted, made lenient and redacted again, and every byte buffer or byte array,
// at any depth, is its UTF-8 text and every rune slice or array its string, never numbers or base64. A struct is copied
// field by field as encoding/json names them (structCopy); a value with its own JSON or text form is encoded by
// encoding/json, decoded and copied in turn. A map, slice or pointer met a second time (a cycle, or one value held
// twice) is its type name, so that the copy never walks a structure more than once.
func redactedCopy(value reflect.Value, secrets redactor, depth int, seen map[visit]bool) (copied any, err error) {
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
	if once, ok := visitOf(value); ok {
		if seen[once] {
			return value.Type().String(), nil
		}
		seen[once] = true
	}
	if value.Type().Implements(jsonMarshaler) || value.Type().Implements(textMarshaler) {
		if !value.CanInterface() {
			return value.Type().String(), nil
		}
		decoded, err := decodedForm(value.Interface())
		if err != nil {
			return nil, err
		}
		return redactedCopy(reflect.ValueOf(decoded), secrets, depth+1, seen)
	}
	switch value.Kind() {
	case reflect.Struct:
		return structCopy(value, secrets, depth, seen)
	case reflect.Pointer, reflect.Interface:
		return redactedCopy(value.Elem(), secrets, depth+1, seen)
	case reflect.String:
		return redactLenient(value.String(), secrets), nil
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
		if text, ok := textOfElements(value); ok {
			return redactLenient(text, secrets), nil
		}
		list := make([]any, value.Len())
		for i := range list {
			if list[i], err = redactedCopy(value.Index(i), secrets, depth+1, seen); err != nil {
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
			if object[redactLenient(key, secrets)], err = redactedCopy(iterator.Value(), secrets, depth+1, seen); err != nil {
				return nil, err
			}
		}
		return object, nil
	}
	return nil, errNotEncoded
}

// visit names a map, a slice or a pointer by what it points at, for redactedCopy's record of what it walked.
type visit struct {
	at     unsafe.Pointer
	of     reflect.Type
	length int
}

func visitOf(value reflect.Value) (visit, bool) {
	switch value.Kind() {
	case reflect.Map, reflect.Pointer:
		return visit{at: value.UnsafePointer(), of: value.Type()}, true
	case reflect.Slice:
		// A byte or rune slice is a text, a leaf: copying it twice walks nothing.
		if elements := value.Type().Elem().Kind(); value.Len() == 0 || elements == reflect.Uint8 || elements == reflect.Int32 {
			return visit{}, false
		}
		return visit{at: value.UnsafePointer(), of: value.Type(), length: value.Len()}, true
	}
	return visit{}, false
}

// textOfElements is the text of a slice or an array of bytes (its UTF-8 text, made lenient) or of runes (its string),
// whatever its length: a buffer is text, never a list of numbers (decision D041-164 and the security review of
// PR #303).
func textOfElements(value reflect.Value) (string, bool) {
	switch value.Type().Elem().Kind() {
	case reflect.Uint8:
		raw := make([]byte, value.Len())
		for i := range raw {
			raw[i] = byte(value.Index(i).Uint())
		}
		text := lenient(string(raw))
		clear(raw)
		return text, true
	case reflect.Int32:
		runes := make([]rune, value.Len())
		for i := range runes {
			runes[i] = rune(value.Index(i).Int())
		}
		return string(runes), true
	}
	return "", false
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
func structCopy(value reflect.Value, secrets redactor, depth int, seen map[visit]bool) (any, error) {
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
		copied, err := redactedCopy(member, secrets, depth+1, seen)
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
	if key.Type().Implements(textMarshaler) && key.CanInterface() {
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

// stepLogBatch is a batch on its way. settled says that its fate is decided (answered, or let go and counted), so
// that its records are counted once and a late answer is ignored.
type stepLogBatch struct {
	held      []heldRecord
	done      chan struct{}
	firstSent time.Time
	once      bool // the batch of the shutdown: sent once, as long as the channel lives
	settled   bool // guarded by the attempt's lock
}

const (
	captureOn = iota
	captureFinalOnly
	captureStopped
)

// The one local line per process when the platform stops capture, whatever answer stopped it (sdks/SPEC.md section
// 6.9).
const stoppedLine = "the platform stopped step-log capture for this session"

// sessionTokenBound is the longest session token: a batch's 192 KiB are counted with it, so that a resend under a new
// token stays within the bound (sdks/SPEC.md section 6.9, decision D041-165).
const sessionTokenBound = 128

// abandonedStatus is the outcome of a call the host abandoned itself, told apart from a CANCELLED the server sent.
const abandonedStatus = "ABANDONED"

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
	drops          map[string]int // counted since the last emission: at close, and after it when a late count comes
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
	if records > 0 {
		c.drops[reason] += records
	}
}

// letGoLocked counts a batch's records once, for the reason it was given up.
func (c *attemptCapture) letGoLocked(batch *stepLogBatch, reason string) {
	if !batch.settled {
		batch.settled = true
		c.count(reason, len(batch.held))
	}
}

func (c *attemptCapture) letGo(batch *stepLogBatch, reason string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.letGoLocked(batch, reason)
}

// settled marks a batch answered; it reports false when the batch was let go meanwhile (its answer is ignored).
func (c *attemptCapture) settle(batch *stepLogBatch) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	if batch.settled {
		return false
	}
	batch.settled = true
	return true
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
	c.emitLate()
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

// buffered is step 7: the record joins the buffer in the order of its sequence, and the call that buffers the 256th
// record cuts the batch. After UNIMPLEMENTED the buffer keeps only the newest records that fit one carry.
func (c *attemptCapture) buffered(record *pb.StepLogRecord, size int) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed || c.state != captureOn || c.stoppedLocked() {
		// The state changed while the record was formatted: it is let go as the buffer was.
		switch {
		case c.state == captureFinalOnly:
			c.count(dropAfterCeiling, 1)
		default:
			c.count(dropSendFailed, 1)
		}
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
	if c.batchesStopped {
		c.keepCarryLocked()
		return
	}
	if !c.ticking && !c.ended.Load() {
		c.startTick()
	}
	if len(c.buffer) >= limitFlushRecords && c.inFlight == nil && !c.setFullLocked() {
		c.send(c.cut(len(c.buffer)))
	}
}

// keepCarryLocked drops the oldest buffered records that one carry will not hold (report_deadline).
func (c *attemptCapture) keepCarryLocked() {
	if beyond := len(c.buffer) - c.carryFits(); beyond > 0 {
		c.count(dropReportDeadline, beyond)
		release(c.buffer[:beyond])
		for _, one := range c.buffer[:beyond] {
			c.bufferBytes -= one.size
		}
		c.buffer = append([]heldRecord(nil), c.buffer[beyond:]...)
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
// encoded AppendStepLogsRequest counted with a session token of 128 bytes.
func (c *attemptCapture) cut(limit int) []heldRecord {
	workerID, token := c.host.session()
	if len(token) < sessionTokenBound {
		token = strings.Repeat("t", sessionTokenBound)
	}
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
	go c.deliver(batch, c.host.timeout)
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

// mayResend reports whether the batch is sent (again): not once it was let go, its attempt closed (a shutdown batch
// excepted), its attempt or its session stopped capture, or the process's capture stopped (decision D041-165).
func (c *attemptCapture) mayResend(batch *stepLogBatch) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	switch {
	case batch.settled:
		return false
	case processCapture.secrets.full.Load():
		c.letGoLocked(batch, dropSecretSetFull)
		return false
	case (c.closed && !batch.once) || c.stoppedLocked():
		c.letGoLocked(batch, dropSendFailed)
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

// deliver sends one batch until an answer settles it (contracts.md section 4). A shutdown batch is sent a single
// time: an answer that would send it again drops it instead.
func (c *attemptCapture) deliver(batch *stepLogBatch, within time.Duration) {
	defer c.emitLate()
	defer c.finish(batch)
	defer func() {
		if recover() != nil {
			c.letGo(batch, dropSendFailed)
		}
	}()
	records := make([]*pb.StepLogRecord, len(batch.held))
	for i, one := range batch.held {
		records[i] = one.record
	}
	workerID, token := c.host.session()
	request := c.request(records, workerID, token)
	retries, sessionResent := 0, false
	for {
		if !c.mayResend(batch) {
			return
		}
		left := limitBatchAge - time.Since(batch.firstSent)
		if left <= 0 {
			c.letGo(batch, dropSendFailed)
			return
		}
		sends := c.sends
		if batch.once {
			// The shutdown batch lives as long as the channel, not as its attempt, which the shutdown is ending.
			sends = c.host.life
		}
		outcome := c.host.append(sends, request, min(within, left))
		if batch.once {
			// Sent once, with no resend: a batch the answer did not take is counted as not sent.
			if outcome.answered && outcome.verdict == pb.StepLogVerdict_STEP_LOG_VERDICT_CONTINUE {
				c.settle(batch)
			} else {
				c.letGo(batch, dropSendFailed)
			}
			return
		}
		var wait time.Duration
		if outcome.answered {
			switch outcome.verdict {
			case pb.StepLogVerdict_STEP_LOG_VERDICT_CONTINUE:
				c.settle(batch)
				return
			case pb.StepLogVerdict_STEP_LOG_VERDICT_FINAL_ONLY:
				if c.settle(batch) {
					c.finalOnly()
				}
				return
			case pb.StepLogVerdict_STEP_LOG_VERDICT_STOP_ATTEMPT:
				if c.settle(batch) {
					c.stop()
				}
				return
			case pb.StepLogVerdict_STEP_LOG_VERDICT_STOP_CAPTURE:
				nowID, now := c.host.session()
				if outcome.stopReason == pb.StepLogStopReason_STEP_LOG_STOP_REASON_SESSION && !sessionResent && now != token {
					sessionResent, token = true, now
					request = c.request(records, nowID, now)
					continue
				}
				if c.settle(batch) {
					c.stopSession()
				}
				return
			case pb.StepLogVerdict_STEP_LOG_VERDICT_RETRY_LATER:
				wait = clampRetry(time.Duration(outcome.retryMs) * time.Millisecond)
			}
		} else {
			switch outcome.status {
			case abandonedStatus:
				// The host abandoned the call because it is stopping: dropped at once, not retried.
				c.letGo(batch, dropSendFailed)
				return
			case "RESOURCE_EXHAUSTED":
				wait = 200 * time.Millisecond
				if outcome.retryAfter != nil {
					wait = *outcome.retryAfter
				}
				wait = clampRetry(wait)
			case "INVALID_ARGUMENT":
				c.letGo(batch, dropSendFailed)
				return
			case "UNAUTHENTICATED", "PERMISSION_DENIED":
				c.letGo(batch, dropSendFailed)
				c.stopSession()
				return
			case "UNIMPLEMENTED":
				c.mu.Lock()
				c.letGoLocked(batch, dropSendFailed)
				c.batchesStopped = true
				c.keepCarryLocked()
				c.mu.Unlock()
				return
			}
		}
		if wait > 0 {
			// RETRY_LATER and RESOURCE_EXHAUSTED: the same batch again, not counted as a retry.
			if !c.pause(wait) {
				c.letGo(batch, dropSendFailed)
				return
			}
			continue
		}
		// UNAVAILABLE, DEADLINE_EXCEEDED, CANCELLED from the server, a transport failure, any other status or
		// verdict: two retries.
		if retries >= len(stepLogRetryDelays) {
			c.letGo(batch, dropSendFailed)
			return
		}
		if !c.pause(jittered(stepLogRetryDelays[retries])) {
			c.letGo(batch, dropSendFailed)
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

func (c *attemptCapture) stopSession() {
	c.session.stopped.Store(true)
	c.host.warnOnce(stoppedLine)
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
			if left > 0 {
				c.mu.Unlock()
				locked = false
				timer := time.NewTimer(left)
				select {
				case <-batch.done:
				case <-timer.C:
				}
				timer.Stop()
				c.mu.Lock()
				locked = true
			}
			if c.inFlight == batch {
				// The 2 seconds ended with the batch on its way: it is given up, its answer ignored.
				c.letGoLocked(batch, dropReportDeadline)
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
	c.keepCarryLocked()
	records := make([]*pb.StepLogRecord, len(c.buffer))
	for i, one := range c.buffer {
		records[i] = one.record
	}
	// The carried records leave the buffer: what close finds there is what an attempt without a report let go.
	release(c.buffer)
	c.buffer, c.bufferBytes = nil, 0
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

// close ends the attempt: nothing more is sent but a shutdown batch, what it still holds is let go (send_failed: it
// ends without a report, or after one that carried the rest), and its drops are counted once.
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
	if batch := c.inFlight; batch != nil && !batch.once {
		c.letGoLocked(batch, dropSendFailed)
	}
	c.cancelSends()
	c.dropBufferLocked(dropSendFailed)
	drops := c.drops
	c.drops = map[string]int{}
	c.mu.Unlock()
	c.emit(drops)
}

// emitLate emits what was counted after the attempt closed (a shutdown batch's fate, a late let-go).
func (c *attemptCapture) emitLate() {
	c.mu.Lock()
	if !c.closed || len(c.drops) == 0 {
		c.mu.Unlock()
		return
	}
	drops := c.drops
	c.drops = map[string]int{}
	c.mu.Unlock()
	c.emit(drops)
}

func (c *attemptCapture) emit(drops map[string]int) {
	reasons := make([]string, 0, len(drops))
	for reason := range drops {
		reasons = append(reasons, reason)
	}
	sort.Strings(reasons)
	for _, reason := range reasons {
		c.host.dropped(reason, drops[reason])
	}
}

// shutdownBatch is the batch an attempt whose handler still runs sends when the host stops after its grace, unless it
// has one in flight: its oldest buffered records, as its batch in flight, sent once; what does not fit stays buffered
// for its report. An attempt whose handler ended is left to its report (decision D041-165).
func (c *attemptCapture) shutdownBatch() *stepLogBatch {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed || c.ended.Load() || c.stoppedLocked() || c.state != captureOn || c.batchesStopped || c.inFlight != nil || len(c.buffer) == 0 || c.setFullLocked() {
		return nil
	}
	batch := &stepLogBatch{held: c.cut(len(c.buffer)), done: make(chan struct{}), firstSent: time.Now(), once: true}
	c.inFlight = batch
	return batch
}
