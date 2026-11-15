// Package ops is the operator factory of a collection (design 042, FR-042-028): each function returns
// one stage of the operators of a forEach, written as an object with exactly one key, the operator's
// name. Pass the stages to workflow.Builder.ForEach or Map after maxConcurrency.
//
// The factory only assembles a stage; the server validates the chain (its length, the ranges of the
// counts and the variables an expression reads). Stage expressions read element, position, input,
// steps, run and state.
package ops

// Operator is one stage of a collection's operator chain. The zero value is no operator: a builder
// refuses it.
type Operator struct {
	name       string
	parameters map[string]any
}

// Name is the operator's name as the IR writes it (filter, map, flatMap, skip, take, distinct,
// window); empty for the zero value.
func (o Operator) Name() string {
	return o.name
}

// Stage is the stage as the IR writes it, {name: parameters}, a fresh copy on every call.
func (o Operator) Stage() map[string]any {
	parameters := make(map[string]any, len(o.parameters))
	for key, value := range o.parameters {
		parameters[key] = value
	}
	return map[string]any{o.name: parameters}
}

// Filter keeps the elements for which where is true.
func Filter(where string) Operator {
	return Operator{name: "filter", parameters: map[string]any{"where": where}}
}

// Map replaces each element by the value of selector.
func Map(selector string) Operator {
	return Operator{name: "map", parameters: map[string]any{"select": selector}}
}

// FlatMap evaluates selector to a list for each element and concatenates the lists in order, one
// level deep.
func FlatMap(selector string) Operator {
	return Operator{name: "flatMap", parameters: map[string]any{"select": selector}}
}

// Skip drops the first count elements.
func Skip(count int) Operator {
	return Operator{name: "skip", parameters: map[string]any{"count": count}}
}

// Take keeps the first count elements.
func Take(count int) Operator {
	return Operator{name: "take", parameters: map[string]any{"count": count}}
}

// Distinct keeps the first occurrence of each element.
func Distinct() Operator {
	return Operator{name: "distinct", parameters: map[string]any{}}
}

// DistinctBy keeps the first element of each value of by.
func DistinctBy(by string) Operator {
	return Operator{name: "distinct", parameters: map[string]any{"by": by}}
}

// Window replaces the list by consecutive lists of count elements, the last one possibly shorter.
func Window(count int) Operator {
	return Operator{name: "window", parameters: map[string]any{"count": count}}
}
