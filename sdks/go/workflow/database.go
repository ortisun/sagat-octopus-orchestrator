package workflow

import (
	"bytes"
	"encoding/json"
	"math"
	"regexp"
	"strings"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// ConnectionRef selects a scoped logical resource, never a DSN or credential.
type ConnectionRef struct {
	Name     string  `json:"name"`
	Revision *uint32 `json:"revision,omitempty"`
}
type DatabaseOperation string

const (
	DatabaseQuery   DatabaseOperation = "query"
	DatabaseExecute DatabaseOperation = "execute"
)

type DatabaseType string

const (
	DatabaseNull      DatabaseType = "null"
	DatabaseBoolean   DatabaseType = "boolean"
	DatabaseInteger   DatabaseType = "integer"
	DatabaseDecimal   DatabaseType = "decimal"
	DatabaseFloat     DatabaseType = "float"
	DatabaseText      DatabaseType = "text"
	DatabaseTimestamp DatabaseType = "timestamp"
	DatabaseDate      DatabaseType = "date"
	DatabaseBinary    DatabaseType = "binary"
	DatabaseJSON      DatabaseType = "json"
)

// DatabaseValue carries explicit SQL binding semantics. Integer/decimal values are strings;
// binary is base64, timestamp is RFC3339, and date is YYYY-MM-DD.
type DatabaseValue struct {
	Type  DatabaseType `json:"type"`
	Value any          `json:"value"`
}

// Exactly one of Literal or the Type/Expression pair is supplied.
type DatabaseParameter struct {
	Literal    *DatabaseValue `json:"literal,omitempty"`
	Type       DatabaseType   `json:"type,omitempty"`
	Expression string         `json:"expression,omitempty"`
}
type DatabaseOptions struct {
	ConnectionRef ConnectionRef       `json:"connectionRef"`
	Operation     DatabaseOperation   `json:"operation"`
	Statement     string              `json:"statement"`
	Parameters    []DatabaseParameter `json:"parameters,omitempty"`
	Limits        map[string]uint32   `json:"limits,omitempty"`
	TimeoutMs     *uint32             `json:"timeoutMs,omitempty"`
	OnError       string              `json:"onError,omitempty"`
}

func (DatabaseOptions) String() string   { return "[managed database operation]" }
func (DatabaseParameter) String() string { return "[database parameter]" }

var databaseName = regexp.MustCompile(`^[a-z][a-z0-9-]{1,61}[a-z0-9]$`)
var integerText = regexp.MustCompile(`^-?[0-9]+$`)
var decimalText = regexp.MustCompile(`^-?[0-9]+(\.[0-9]+)?$`)
var databaseCeilings = map[string]uint32{"maxStatementBytes": 65536, "maxParameters": 256, "maxParameterBytes": 262144, "maxRows": 10000, "maxResultBytes": 4194304, "timeoutMs": 30000, "connectTimeoutMs": 10000, "poolSize": 16, "idleTimeoutMs": 60000}

func databaseType(t DatabaseType) bool {
	switch t {
	case DatabaseNull, DatabaseBoolean, DatabaseInteger, DatabaseDecimal, DatabaseFloat, DatabaseText, DatabaseTimestamp, DatabaseDate, DatabaseBinary, DatabaseJSON:
		return true
	}
	return false
}
func databaseParameter(p DatabaseParameter) bool {
	if p.Literal == nil {
		return databaseType(p.Type) && p.Expression != ""
	}
	if p.Type != "" || p.Expression != "" {
		return false
	}
	v := p.Literal.Value
	s, stringValue := v.(string)
	switch p.Literal.Type {
	case DatabaseNull:
		return v == nil
	case DatabaseBoolean:
		_, ok := v.(bool)
		return ok
	case DatabaseInteger:
		return stringValue && len(s) <= 40 && integerText.MatchString(s)
	case DatabaseDecimal:
		return stringValue && len(s) <= 16384 && decimalText.MatchString(s)
	case DatabaseText:
		return stringValue && !strings.ContainsRune(s, 0)
	case DatabaseTimestamp, DatabaseDate, DatabaseBinary:
		return stringValue
	case DatabaseJSON:
		return true
	case DatabaseFloat:
		switch number := v.(type) {
		case float64:
			return !math.IsNaN(number) && !math.IsInf(number, 0)
		case float32:
			return !math.IsNaN(float64(number)) && !math.IsInf(float64(number), 0)
		case json.Number:
			n, err := number.Float64()
			return err == nil && !math.IsNaN(n) && !math.IsInf(n, 0)
		}
	}
	return false
}

// Database adds a managed operation. SQL validation, authorization and retry semantics remain
// server-side; ordinary SDK workers never receive its credentials or execute its statement.
func (b *Builder) Database(id string, options DatabaseOptions) *Builder {
	if b.err != nil {
		return b
	}
	fail := func() *Builder { return b.fail(sagat.Invalid("invalid managed database operation")) }
	ref := options.ConnectionRef
	if !databaseName.MatchString(ref.Name) || strings.Contains(ref.Name, "--") || (ref.Revision != nil && (*ref.Revision == 0 || *ref.Revision > 2147483647)) {
		return fail()
	}
	if (options.Operation != DatabaseQuery && options.Operation != DatabaseExecute) || strings.TrimSpace(options.Statement) == "" || len(options.Statement) > 65536 || strings.ContainsRune(options.Statement, 0) || len(options.Parameters) > 256 {
		return fail()
	}
	for _, p := range options.Parameters {
		if !databaseParameter(p) {
			return fail()
		}
	}
	for k, v := range options.Limits {
		if v == 0 || v > databaseCeilings[k] {
			return fail()
		}
	}
	if options.TimeoutMs != nil && (*options.TimeoutMs == 0 || *options.TimeoutMs > 60000) {
		return fail()
	}
	params, err := jsonx.Canonical(options.Parameters)
	if err != nil || len(params) > 262144 {
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
	// A supplied empty parameter list is distinct from an omitted list in canonical authoring.
	if options.Parameters != nil && len(options.Parameters) == 0 {
		n["parameters"] = []any{}
	}
	if options.Limits != nil && len(options.Limits) == 0 {
		n["limits"] = map[string]any{}
	}
	n["id"] = id
	n["type"] = "database"
	return b.push(n)
}
