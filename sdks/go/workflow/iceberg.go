package workflow

import (
	"bytes"
	"encoding/json"
	"strings"
	"unicode"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// IcebergTable is a table of an Iceberg catalog: a namespace of 1 to 8 levels and a name.
type IcebergTable struct {
	Namespace []string `json:"namespace"`
	Name      string   `json:"name"`
}

// IcebergRecords says where the records of the write come from: exactly one of an expression that evaluates to a list of
// objects (Inline), one that evaluates to 1 to 32 blob descriptors (Blobs), or the id of an earlier root-level database step
// (DatabaseResult). The fields are pointers so that an empty value is told from an absent one.
type IcebergRecords struct {
	Inline         *string `json:"inline,omitempty"`
	Blobs          *string `json:"blobs,omitempty"`
	DatabaseResult *string `json:"databaseResult,omitempty"`
}

// IcebergMapping is a field of a record (a JSON pointer, /customer/name) and the column it is written to.
type IcebergMapping struct {
	Field  string `json:"field"`
	Column string `json:"column"`
}

// IcebergLimits are optional tighter limits on one write.
type IcebergLimits struct {
	MaxRecordBytes *uint32 `json:"maxRecordBytes,omitempty"`
	MaxRecords     *uint32 `json:"maxRecords,omitempty"`
	TimeoutMs      *uint32 `json:"timeoutMs,omitempty"`
}

// IcebergWriteOptions is an append to an Iceberg table. Only a scoped logical connection name and revision appear in a
// workflow: no catalog, bucket, credential or secret. The mode is append; overwrite and upsert are not built yet.
type IcebergWriteOptions struct {
	ConnectionRef ConnectionRef `json:"connectionRef"`
	Table         IcebergTable  `json:"table"`
	// Ref is the branch or tag to commit to; main when absent.
	Ref                 *string          `json:"ref,omitempty"`
	Records             IcebergRecords   `json:"records"`
	Mapping             []IcebergMapping `json:"mapping,omitempty"`
	IgnoreUnknownFields *bool            `json:"ignoreUnknownFields,omitempty"`
	Limits              *IcebergLimits   `json:"limits,omitempty"`
	// Compensation is "revert" (the step is undone by removing its commit) or absent.
	Compensation string  `json:"compensation,omitempty"`
	OnError      *string `json:"onError,omitempty"`
}

// icebergPlain is 1 to limit bytes of UTF-8 without a control character: the platform's plain text.
func icebergPlain(text string, limit int) bool {
	if text == "" || len(text) > limit {
		return false
	}
	for _, r := range text {
		if unicode.IsControl(r) {
			return false
		}
	}
	return true
}

// icebergPart is a level or a name of a table: plain, and not a path component of its own.
func icebergPart(text string) bool {
	return icebergPlain(text, 255) && text != "." && text != ".."
}

// icebergPointer is a JSON pointer into a record whose escapes are ~0 and ~1, of at most 16 segments.
func icebergPointer(text string) bool {
	if !icebergPlain(text, 512) || !strings.HasPrefix(text, "/") {
		return false
	}
	segments := strings.Split(text[1:], "/")
	if len(segments) > 16 {
		return false
	}
	for _, s := range segments {
		if s == "" {
			return false
		}
	}
	for i := 0; i < len(text); i++ {
		if text[i] == '~' && (i+1 >= len(text) || (text[i+1] != '0' && text[i+1] != '1')) {
			return false
		}
	}
	return true
}

func icebergBounded(value *uint32, low, high uint32) bool {
	return value == nil || (*value >= low && *value <= high)
}

// IcebergWrite adds an append to an Iceberg table through the isolated Sagat lake runner. Shape validation only: the
// catalog, access, modes and classification belong to the server.
func (b *Builder) IcebergWrite(id string, options IcebergWriteOptions) *Builder {
	if b.err != nil {
		return b
	}
	fail := func() *Builder { return b.fail(sagat.Invalid("invalid iceberg write")) }
	if !validConnectionRef(options.ConnectionRef) {
		return fail()
	}
	table := options.Table
	if len(table.Namespace) < 1 || len(table.Namespace) > 8 || !icebergPart(table.Name) {
		return fail()
	}
	for _, level := range table.Namespace {
		if !icebergPart(level) {
			return fail()
		}
	}
	if options.Ref != nil && !icebergPlain(*options.Ref, 255) {
		return fail()
	}
	records := options.Records
	sources := 0
	for _, source := range []*string{records.Inline, records.Blobs} {
		if source == nil {
			continue
		}
		sources++
		// an expression of 1 to 1024 UTF-8 bytes, as everywhere else: the schema says 4096, the engine takes 1024
		if len(*source) < 1 || len(*source) > 1024 {
			return fail()
		}
	}
	if records.DatabaseResult != nil {
		sources++
		if !icebergPlain(*records.DatabaseResult, 64) {
			return fail()
		}
	}
	if sources != 1 {
		return fail()
	}
	if len(options.Mapping) > 1024 {
		return fail()
	}
	fields, columns := map[string]bool{}, map[string]bool{}
	for _, pair := range options.Mapping {
		if !icebergPointer(pair.Field) || !icebergPlain(pair.Column, 255) {
			return fail()
		}
		// a field or a column twice is two rows of the mapping that disagree about one thing
		if fields[pair.Field] || columns[pair.Column] {
			return fail()
		}
		fields[pair.Field], columns[pair.Column] = true, true
	}
	if limits := options.Limits; limits != nil &&
		(!icebergBounded(limits.MaxRecordBytes, 1024, 2147483648) || !icebergBounded(limits.MaxRecords, 1, 100000000) || !icebergBounded(limits.TimeoutMs, 1000, 3600000)) {
		return fail()
	}
	if options.Compensation != "" && options.Compensation != "revert" {
		return fail()
	}
	if options.OnError != nil && *options.OnError == "" {
		return fail()
	}
	// Snapshot at the public boundary: later caller mutations do not retarget a built node.
	raw, err := jsonx.Encode(options)
	if err != nil {
		return fail()
	}
	var n node
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	if err = decoder.Decode(&n); err != nil {
		return fail()
	}
	// A supplied empty mapping is distinct from an omitted one in canonical authoring.
	if options.Mapping != nil && len(options.Mapping) == 0 {
		n["mapping"] = []any{}
	}
	n["id"] = id
	n["type"] = "icebergWrite"
	n["mode"] = "append"
	return b.push(n)
}
