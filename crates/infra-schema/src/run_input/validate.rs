//! The validator of the subset (D043-047, D043-049, D043-058, D043-067): it charges the counter before every step and
//! reports its first violation as a closed [`Rule`] named by the keyword that failed, with the instance pointer it
//! built itself. A segment is added for a member matched through `properties` and for an index matched through
//! `items`; nothing for a member reached through `additionalProperties`, whose key the caller chose; a `required`
//! violation appends the missing name, which the schema holds. No input value is ever formatted.
//!
//! The charges: one unit per subschema application (an empty schema included) and per `$ref` followed; one per name
//! of `required` and of a `type` list, one per branch of `allOf`, `anyOf` and `oneOf`; one per instance member for its
//! `properties` lookup and one for its `additionalProperties` decision, one per array element for `items`; for `enum`,
//! one per member plus the smaller of the member's size and the value's; for `const`, the smaller size; for a length
//! check, the length in code points; for a `pattern`, `(bytes + 1) × NFA states`, charged before the match. A charge
//! that would pass the budget ends the whole validation with `workLimit`, never one branch.

use std::cmp::Ordering;

use serde_json::{Map, Value};

use super::compile::{CompiledInputSchema, Node, types};
use super::counter::{Counter, Counters, Limits, Stop};
use super::value::{Num, equal, size_counted};
use super::{MAX_INPUT_DEPTH, PathSegment, Rule, SchemaRefusal};

impl CompiledInputSchema {
    /// Validates `input` (JSON bytes) within the production budget and backstop.
    ///
    /// # Errors
    /// The first violation, `workLimit`, `maxDepth` or the backstop, as a [`SchemaRefusal`]. It holds no input value.
    pub fn validate(&self, input: &[u8]) -> Result<(), SchemaRefusal> {
        self.validate_with(input, Limits::default()).0
    }

    /// [`Self::validate`] within `limits`, with what the validation spent.
    pub fn validate_with(
        &self,
        input: &[u8],
        limits: Limits,
    ) -> (Result<(), SchemaRefusal>, Counters) {
        let mut counter = Counter::new(limits);
        if deeper_than(input, MAX_INPUT_DEPTH) {
            return (Err(SchemaRefusal::TooDeep), counter.counters());
        }
        let Ok(value) = serde_json::from_slice::<Value>(input) else {
            // Not JSON at all: the creators check that first, so this is a refusal that names nothing.
            return (
                Err(SchemaRefusal::Invalid {
                    segments: Vec::new(),
                    rule: Rule::Other,
                }),
                counter.counters(),
            );
        };
        let mut run = Run {
            schema: self,
            counter: &mut counter,
            violation: None,
        };
        let outcome = run.apply(0, &value, true);
        let violation = run.violation.take();
        let result = match outcome {
            Ok(()) => Ok(()),
            Err(Stop::WorkLimit) => Err(SchemaRefusal::WorkLimit),
            Err(Stop::Backstop) => Err(SchemaRefusal::Backstop),
            Err(Stop::Fail) => {
                let (rule, mut segments) = violation.unwrap_or((Rule::Other, Vec::new()));
                segments.reverse();
                Err(SchemaRefusal::Invalid { segments, rule })
            }
        };
        (result, counter.counters())
    }
}

/// Whether the JSON text nests arrays and objects deeper than `limit`, read without parsing it, so an input deeper
/// than `serde_json` parses (its recursion limit is 128) is `maxDepth` too and never an internal error.
fn deeper_than(input: &[u8], limit: usize) -> bool {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in input {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > limit {
                    return true;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

struct Run<'s, 'c> {
    schema: &'s CompiledInputSchema,
    counter: &'c mut Counter,
    /// The first violation, built while the evaluation unwinds: the rule and the segments, innermost first.
    violation: Option<(Rule, Vec<PathSegment>)>,
}

impl Run<'_, '_> {
    /// A failure with `rule` here. Only a reporting evaluation records it; a branch of `anyOf` or `oneOf` does not.
    fn fail(&mut self, report: bool, rule: Rule) -> Result<(), Stop> {
        if report {
            self.violation = Some((rule, Vec::new()));
        }
        Err(Stop::Fail)
    }

    /// Adds `segment` to the violation as it unwinds through a member or an element.
    fn through(
        &mut self,
        report: bool,
        outcome: Result<(), Stop>,
        segment: impl FnOnce() -> PathSegment,
    ) -> Result<(), Stop> {
        if report
            && outcome == Err(Stop::Fail)
            && let Some((_, segments)) = self.violation.as_mut()
        {
            segments.push(segment());
        }
        outcome
    }

    fn apply(&mut self, at: usize, value: &Value, report: bool) -> Result<(), Stop> {
        let schema = self.schema;
        let node = &schema.nodes[at];
        self.counter.charge(1)?;
        self.counter.op(1);
        if let Some(accepts) = node.boolean {
            return if accepts {
                Ok(())
            } else {
                self.fail(report, Rule::Other)
            };
        }
        if let Some(target) = node.reference {
            self.counter.charge(1)?;
            // A `$ref` stands beside annotations and containers only: nothing else of this node applies.
            return self.apply(target, value, report);
        }
        if let Some((bits, names)) = node.types {
            self.counter.charge(names)?;
            self.counter.op(names);
            if !type_matches(bits, value) {
                return self.fail(report, Rule::Type);
            }
        }
        if let Some(members) = &node.enumeration {
            self.counter.charge(members.len() as u64)?;
            let (measured, steps) = size_counted(value, node.enumeration_largest);
            self.counter.op(steps);
            // the walk is bounded by the largest member, and charged (D043-100)
            self.counter.charge(steps)?;
            let mut found = false;
            for member in members {
                self.counter.charge(member.size.min(measured))?;
                if equal(&member.value, value, self.counter) {
                    found = true;
                    break;
                }
            }
            if !found {
                return self.fail(report, Rule::Enum);
            }
        }
        if let Some(constant) = &node.constant {
            let (measured, steps) = size_counted(value, constant.size);
            self.counter.op(steps);
            self.counter.charge(steps)?;
            self.counter.charge(constant.size.min(measured).max(1))?;
            if !equal(&constant.value, value, self.counter) {
                return self.fail(report, Rule::Const);
            }
        }
        match value {
            Value::Number(number) => self.number(node, Num::of(number), report)?,
            Value::String(text) => self.string(node, text, report)?,
            Value::Array(items) => self.array(node, items, report)?,
            Value::Object(members) => self.object(node, members, report)?,
            Value::Null | Value::Bool(_) => {}
        }
        for &branch in &node.all_of {
            self.counter.charge(1)?;
            self.apply(branch, value, report)?;
        }
        if !node.any_of.is_empty() {
            let mut passed = false;
            for &branch in &node.any_of {
                self.counter.charge(1)?;
                match self.apply(branch, value, false) {
                    Ok(()) => {
                        passed = true;
                        break;
                    }
                    Err(Stop::Fail) => {}
                    Err(stop) => return Err(stop),
                }
            }
            if !passed {
                return self.fail(report, Rule::AnyOf);
            }
        }
        if !node.one_of.is_empty() {
            let mut passed = 0usize;
            for &branch in &node.one_of {
                self.counter.charge(1)?;
                match self.apply(branch, value, false) {
                    Ok(()) => passed += 1,
                    Err(Stop::Fail) => {}
                    Err(stop) => return Err(stop),
                }
            }
            if passed != 1 {
                return self.fail(report, Rule::OneOf);
            }
        }
        Ok(())
    }

    fn number(&mut self, node: &Node, number: Num, report: bool) -> Result<(), Stop> {
        self.counter.op(5);
        if node
            .minimum
            .is_some_and(|limit| number.compare(limit) == Ordering::Less)
        {
            return self.fail(report, Rule::Minimum);
        }
        if node
            .maximum
            .is_some_and(|limit| number.compare(limit) == Ordering::Greater)
        {
            return self.fail(report, Rule::Maximum);
        }
        if node
            .exclusive_minimum
            .is_some_and(|limit| number.compare(limit) != Ordering::Greater)
        {
            return self.fail(report, Rule::ExclusiveMinimum);
        }
        if node
            .exclusive_maximum
            .is_some_and(|limit| number.compare(limit) != Ordering::Less)
        {
            return self.fail(report, Rule::ExclusiveMaximum);
        }
        if node
            .multiple_of
            .is_some_and(|divisor| !number.is_multiple_of(divisor))
        {
            return self.fail(report, Rule::MultipleOf);
        }
        Ok(())
    }

    fn string(&mut self, node: &Node, text: &str, report: bool) -> Result<(), Stop> {
        // The lengths first, so a long string fails on its length before a pattern is charged for it.
        if node.min_length.is_some() || node.max_length.is_some() {
            let length = text.chars().count() as u64;
            self.counter.charge(length)?;
            self.counter.op(length);
            if node.min_length.is_some_and(|limit| length < limit) {
                return self.fail(report, Rule::MinLength);
            }
            if node.max_length.is_some_and(|limit| length > limit) {
                return self.fail(report, Rule::MaxLength);
            }
        }
        if let Some(index) = node.pattern {
            let pattern = &self.schema.patterns[index];
            let scanned = (text.len() as u64).saturating_add(1);
            self.counter
                .charge(scanned.saturating_mul(pattern.states))?;
            self.counter.op(scanned);
            if !pattern.is_match(text) {
                return self.fail(report, Rule::Pattern);
            }
        }
        Ok(())
    }

    fn array(&mut self, node: &Node, items: &[Value], report: bool) -> Result<(), Stop> {
        let count = items.len() as u64;
        if node.min_items.is_some_and(|limit| count < limit) {
            return self.fail(report, Rule::MinItems);
        }
        if node.max_items.is_some_and(|limit| count > limit) {
            return self.fail(report, Rule::MaxItems);
        }
        if let Some(schema) = node.items {
            for (index, item) in items.iter().enumerate() {
                self.counter.charge(1)?;
                let outcome = self.apply(schema, item, report);
                self.through(report, outcome, || PathSegment::Index(index))?;
            }
        }
        Ok(())
    }

    fn object(
        &mut self,
        node: &Node,
        members: &Map<String, Value>,
        report: bool,
    ) -> Result<(), Stop> {
        let count = members.len() as u64;
        if node.min_properties.is_some_and(|limit| count < limit) {
            return self.fail(report, Rule::MinProperties);
        }
        if node.max_properties.is_some_and(|limit| count > limit) {
            return self.fail(report, Rule::MaxProperties);
        }
        for name in &node.required {
            // a lookup compares the name with about log2(n) keys, each comparison up to the name's length (D043-102)
            let comparisons = log2(members.len()) * (1 + name.len() as u64 / 64);
            self.counter.charge(comparisons)?;
            self.counter.op(comparisons);
            if !members.contains_key(name.as_ref()) {
                if report {
                    // The missing name is the schema's, not the caller's.
                    self.violation =
                        Some((Rule::Required, vec![PathSegment::Property(name.clone())]));
                }
                return Err(Stop::Fail);
            }
        }
        if node.properties.is_empty() && node.additional.is_none() {
            return Ok(());
        }
        // Members in `serde_json::Map` order (a `BTreeMap`: the workspace does not enable `preserve_order`).
        for (key, member) in members {
            let declared = if node.properties.is_empty() {
                None
            } else {
                // a binary search compares the key with about log2(n) names, each up to the shorter of the two
                let comparisons = log2(node.properties.len())
                    * (1 + key.len().min(node.longest_property) as u64 / 64);
                self.counter.charge(comparisons)?;
                self.counter.op(comparisons);
                node.properties
                    .binary_search_by(|(name, _)| name.as_ref().cmp(key.as_str()))
                    .ok()
                    .map(|found| &node.properties[found])
            };
            if let Some((name, schema)) = declared {
                let outcome = self.apply(*schema, member, report);
                self.through(report, outcome, || PathSegment::Property(name.clone()))?;
            } else if let Some(additional) = node.additional {
                self.counter.charge(1)?;
                // The key is the caller's: the violation names the object and the rule, nothing beneath.
                match self.apply(additional, member, false) {
                    Ok(()) => {}
                    Err(Stop::Fail) => return self.fail(report, Rule::AdditionalProperties),
                    Err(stop) => return Err(stop),
                }
            }
        }
        Ok(())
    }
}

fn type_matches(bits: u8, value: &Value) -> bool {
    let kind = match value {
        Value::Null => types::NULL,
        Value::Bool(_) => types::BOOLEAN,
        Value::Object(_) => types::OBJECT,
        Value::Array(_) => types::ARRAY,
        Value::String(_) => types::STRING,
        Value::Number(number) => {
            return bits & types::NUMBER != 0
                || (bits & types::INTEGER != 0 && Num::of(number).is_integral());
        }
    };
    bits & kind != 0
}

fn log2(n: usize) -> u64 {
    u64::from(usize::BITS - n.leading_zeros()).max(1)
}
