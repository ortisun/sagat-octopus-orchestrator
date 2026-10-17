package workflow

import (
	"encoding/json"
	"math"
	"reflect"
	"regexp"
	"unicode/utf8"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
)

// DisclosurePolicy records the explicit authorization boundary of a declared operation result.
type DisclosurePolicy string

// DeclaredBusinessResultV1 permits an authorized caller to receive the declared, bounded result.
const DeclaredBusinessResultV1 DisclosurePolicy = "declared-business-result-v1"

// QueryDefinition declares a bounded snapshot projection. It cannot execute callbacks or I/O.
type QueryDefinition struct {
	ArgsSchema   map[string]any
	ResultSchema map[string]any
	Expression   string
	Disclosure   DisclosurePolicy
}

// UpdateDefinition declares an atomic validator, state reducer and result projection.
type UpdateDefinition struct {
	ArgsSchema   map[string]any
	Validate     string
	State        string
	Result       string
	ResultSchema map[string]any
	Disclosure   DisclosurePolicy
}

// StateDefinition declares bounded business state and its named operations.
type StateDefinition struct {
	Schema  map[string]any
	Initial string
	Queries map[string]QueryDefinition
	Updates map[string]UpdateDefinition
}

var operationName = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$`)

func stateExpression(value, subject string) (string, error) {
	if value == "" {
		return "", sagat.Invalid("%s needs an expression", subject)
	}
	if len([]byte(value)) > 1024 {
		return "", sagat.Invalid("%s expression exceeds 1024 UTF-8 bytes", subject)
	}
	return value, nil
}

func schemaInteger(value any, minimum, maximum int64, subject string) (int64, error) {
	number, ok := exactInteger(value)
	if !ok || number < minimum || number > maximum {
		return 0, sagat.Invalid("%s must be between %d and %d", subject, minimum, maximum)
	}
	return number, nil
}

func exactInteger(value any) (int64, bool) {
	switch number := value.(type) {
	case json.Number:
		parsed, err := number.Int64()
		return parsed, err == nil
	case int:
		return int64(number), true
	case int8:
		return int64(number), true
	case int16:
		return int64(number), true
	case int32:
		return int64(number), true
	case int64:
		return number, true
	case uint:
		if uint64(number) <= math.MaxInt64 {
			return int64(number), true
		}
	case uint8:
		return int64(number), true
	case uint16:
		return int64(number), true
	case uint32:
		return int64(number), true
	case uint64:
		if number <= math.MaxInt64 {
			return int64(number), true
		}
	case float32:
		value := float64(number)
		if math.Trunc(value) == value && value >= math.MinInt64 && value <= math.MaxInt64 {
			return int64(value), true
		}
	case float64:
		if math.Trunc(number) == number && number >= math.MinInt64 && number <= math.MaxInt64 {
			return int64(number), true
		}
	}
	return 0, false
}

func finiteNumber(value any) (float64, bool) {
	if value == nil {
		return 0, false
	}
	if number, ok := value.(json.Number); ok {
		parsed, err := number.Float64()
		return parsed, err == nil && !math.IsNaN(parsed) && !math.IsInf(parsed, 0)
	}
	number := reflect.ValueOf(value)
	switch number.Kind() {
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return float64(number.Int()), true
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		return float64(number.Uint()), true
	case reflect.Float32, reflect.Float64:
		value := number.Float()
		return value, !math.IsNaN(value) && !math.IsInf(value, 0)
	default:
		return 0, false
	}
}

func resultSchema(value map[string]any, subject string) (map[string]any, error) {
	if value == nil {
		return nil, sagat.Invalid("%s must be a closed bounded schema", subject)
	}
	if description, present := value["description"]; present {
		text, ok := description.(string)
		if !ok || utf8.RuneCountInString(text) > 1024 {
			return nil, sagat.Invalid("%s.description must be a string of at most 1024 characters", subject)
		}
	}
	allowed := map[string]bool{"type": true, "description": true}
	kind, _ := value["type"].(string)
	switch kind {
	case "object":
		allowed["properties"], allowed["required"], allowed["additionalProperties"] = true, true, true
		properties, ok := value["properties"].(map[string]any)
		if !ok {
			return nil, sagat.Invalid("%s.properties must be an object", subject)
		}
		closed, ok := value["additionalProperties"].(bool)
		if !ok || closed {
			return nil, sagat.Invalid("%s.additionalProperties must be false", subject)
		}
		for name, nested := range properties {
			nestedSchema, ok := nested.(map[string]any)
			if !ok {
				return nil, sagat.Invalid("%s.properties.%s must be a closed bounded schema", subject, name)
			}
			if _, err := resultSchema(nestedSchema, subject+".properties."+name); err != nil {
				return nil, err
			}
		}
		if required, present := value["required"]; present {
			names, ok := stringList(required)
			if !ok || len(names) > 10000 {
				return nil, sagat.Invalid("%s.required must name unique declared properties", subject)
			}
			seen := map[string]bool{}
			for _, name := range names {
				if seen[name] {
					return nil, sagat.Invalid("%s.required must name unique declared properties", subject)
				}
				if _, exists := properties[name]; !exists {
					return nil, sagat.Invalid("%s.required must name unique declared properties", subject)
				}
				seen[name] = true
			}
		}
	case "array":
		allowed["items"], allowed["minItems"], allowed["maxItems"] = true, true, true
		items, ok := value["items"].(map[string]any)
		if !ok {
			return nil, sagat.Invalid("%s.items must be a closed bounded schema", subject)
		}
		if _, err := resultSchema(items, subject+".items"); err != nil {
			return nil, err
		}
		maximum, err := schemaInteger(value["maxItems"], 0, 10000, subject+".maxItems")
		if err != nil {
			return nil, err
		}
		if minimum, present := value["minItems"]; present {
			parsed, err := schemaInteger(minimum, 0, 10000, subject+".minItems")
			if err != nil {
				return nil, err
			}
			if parsed > maximum {
				return nil, sagat.Invalid("%s.minItems must not exceed maxItems", subject)
			}
		}
	case "string":
		allowed["minLength"], allowed["maxLength"] = true, true
		maximum, err := schemaInteger(value["maxLength"], 0, 262144, subject+".maxLength")
		if err != nil {
			return nil, err
		}
		if minimum, present := value["minLength"]; present {
			parsed, err := schemaInteger(minimum, 0, 262144, subject+".minLength")
			if err != nil {
				return nil, err
			}
			if parsed > maximum {
				return nil, sagat.Invalid("%s.minLength must not exceed maxLength", subject)
			}
		}
	case "integer", "number":
		allowed["minimum"], allowed["maximum"] = true, true
		minimum, hasMinimum := value["minimum"]
		maximum, hasMaximum := value["maximum"]
		min, minOK := finiteNumber(minimum)
		max, maxOK := finiteNumber(maximum)
		if hasMinimum && !minOK {
			return nil, sagat.Invalid("%s.minimum must be a finite number", subject)
		}
		if hasMaximum && !maxOK {
			return nil, sagat.Invalid("%s.maximum must be a finite number", subject)
		}
		if hasMinimum && hasMaximum && min > max {
			return nil, sagat.Invalid("%s.minimum must not exceed maximum", subject)
		}
	case "boolean", "null":
	default:
		return nil, sagat.Invalid("%s.type is not a supported closed result type", subject)
	}
	for key := range value {
		if !allowed[key] {
			return nil, sagat.Invalid("%s.%s is not supported in a closed result schema", subject, key)
		}
	}
	encoded, err := json.Marshal(value)
	if err != nil {
		return nil, sagat.Invalid("%s cannot be written as JSON", subject)
	}
	if len(encoded) > 65536 {
		return nil, sagat.Invalid("%s exceeds 65536 UTF-8 bytes", subject)
	}
	return clone(value).(map[string]any), nil
}

func stringList(value any) ([]string, bool) {
	switch values := value.(type) {
	case []string:
		return values, true
	case []any:
		out := make([]string, len(values))
		for index, value := range values {
			text, ok := value.(string)
			if !ok {
				return nil, false
			}
			out[index] = text
		}
		return out, true
	default:
		return nil, false
	}
}

// State installs the definition without enabling operation transport or runtime execution.
func (b *Builder) State(definition StateDefinition) *Builder {
	if b.err != nil {
		return b
	}
	schema, err := resultSchema(definition.Schema, "state.schema")
	if err != nil {
		return b.fail(err)
	}
	initial, err := stateExpression(definition.Initial, "state.initial")
	if err != nil {
		return b.fail(err)
	}
	state := map[string]any{"schema": schema, "initial": initial}
	if len(definition.Queries) > 32 {
		return b.fail(sagat.Invalid("state supports at most 32 queries"))
	}
	if len(definition.Queries) > 0 {
		queries := make(map[string]any, len(definition.Queries))
		for name, query := range definition.Queries {
			if !operationName.MatchString(name) {
				return b.fail(sagat.Invalid("query name %q is invalid", name))
			}
			if query.ArgsSchema == nil {
				return b.fail(sagat.Invalid("query %s.argsSchema must be an object", name))
			}
			if query.Disclosure != DeclaredBusinessResultV1 {
				return b.fail(sagat.Invalid("query %s needs disclosure %s", name, DeclaredBusinessResultV1))
			}
			result, err := resultSchema(query.ResultSchema, "query "+name+".resultSchema")
			if err != nil {
				return b.fail(err)
			}
			expression, err := stateExpression(query.Expression, "query "+name)
			if err != nil {
				return b.fail(err)
			}
			queries[name] = map[string]any{
				"argsSchema": clone(query.ArgsSchema), "resultSchema": result,
				"expression": expression, "disclosure": string(query.Disclosure),
			}
		}
		state["queries"] = queries
	}
	if len(definition.Updates) > 32 {
		return b.fail(sagat.Invalid("state supports at most 32 updates"))
	}
	if len(definition.Updates) > 0 {
		updates := make(map[string]any, len(definition.Updates))
		for name, update := range definition.Updates {
			if !operationName.MatchString(name) {
				return b.fail(sagat.Invalid("update name %q is invalid", name))
			}
			if update.ArgsSchema == nil {
				return b.fail(sagat.Invalid("update %s.argsSchema must be an object", name))
			}
			if update.Disclosure != DeclaredBusinessResultV1 {
				return b.fail(sagat.Invalid("update %s needs disclosure %s", name, DeclaredBusinessResultV1))
			}
			stateExpressionValue, err := stateExpression(update.State, "update "+name+".state")
			if err != nil {
				return b.fail(err)
			}
			resultExpression, err := stateExpression(update.Result, "update "+name+".result")
			if err != nil {
				return b.fail(err)
			}
			result, err := resultSchema(update.ResultSchema, "update "+name+".resultSchema")
			if err != nil {
				return b.fail(err)
			}
			entry := map[string]any{
				"argsSchema": clone(update.ArgsSchema), "state": stateExpressionValue,
				"result": resultExpression, "resultSchema": result, "disclosure": string(update.Disclosure),
			}
			if update.Validate != "" {
				validator, err := stateExpression(update.Validate, "update "+name+".validate")
				if err != nil {
					return b.fail(err)
				}
				entry["validate"] = validator
			}
			updates[name] = entry
		}
		state["updates"] = updates
	}
	b.state = state
	return b
}
