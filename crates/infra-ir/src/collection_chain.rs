//! The operator chain of a `forEach`, applied once at scope entry (design 042, section 3; FR-042-004 to FR-042-007,
//! FR-042-011).
//!
//! One environment is built for the whole chain, from the variables `items` and every stage name, each parsed once
//! from the run's data. Each stage then consumes its whole input before the next one starts, binding `element` and
//! `position` per element on top of that environment (moved in and out, never copied). While a stage produces, its
//! running count and bytes are checked after every element, and the wall time before every element; the first
//! crossing stops the chain. Each element is serialised once, when it is produced: that measures it and gives the
//! bytes the snapshot stores. A failure names the stage, the operator and the position, never a value.

use std::collections::{BTreeSet, HashSet};
use std::time::Instant;

use sagat_application::collection_chain::{
    ChainAnswer, ChainCeiling, ChainCeilings, ChainFailure, ChainInput, ChainOutput,
    MAX_ELEMENT_DEPTH, StageFault, StageTally,
};
use sagat_application::collection_source::descriptor_from_json;
use sagat_application::execution::RunData;
use sagat_domain::operators::{Operator, OperatorChain, OperatorName};
use sagat_infra_expr::{Compiled, Env};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

/// The variables a stage binds per element.
const ELEMENT: &str = "element";
const POSITION: &str = "position";

/// One element of a stage list: its value and its compact JSON, written once when it was produced.
struct Element {
    value: Value,
    bytes: Vec<u8>,
}

impl Element {
    fn new(value: Value) -> Self {
        // Serialising a `Value` cannot fail: its keys are strings and it holds no non-finite number.
        let bytes = serde_json::to_vec(&value).unwrap_or_default();
        Self { value, bytes }
    }

    /// The element of `value`, unless its compact JSON is longer than `limit`: the writer stops at `limit + 1` bytes, so
    /// a value far over the ceiling is never written out in full.
    fn bounded(value: Value, limit: usize) -> Option<Self> {
        let mut writer = BoundedWriter {
            bytes: Vec::new(),
            limit,
        };
        serde_json::to_writer(&mut writer, &value).ok()?;
        Some(Self {
            value,
            bytes: writer.bytes,
        })
    }
}

/// A writer that refuses to grow past `limit` bytes.
struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl std::io::Write for BoundedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len().saturating_add(buf.len()) > self.limit {
            return Err(std::io::Error::other("over the element ceiling"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// How many lists and maps `value` nests, the outermost included: what the JSON parser's depth limit counts.
fn nesting(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(nesting).max().unwrap_or(0),
        Value::Object(map) => 1 + map.values().map(nesting).max().unwrap_or(0),
        _ => 0,
    }
}

/// The output of one stage while it is produced, with its running ceilings.
struct Produced<'a> {
    stage: usize,
    operator: OperatorName,
    ceilings: &'a ChainCeilings,
    elements: Vec<Element>,
    bytes: usize,
}

impl<'a> Produced<'a> {
    fn new(stage: usize, operator: OperatorName, ceilings: &'a ChainCeilings) -> Self {
        Self {
            stage,
            operator,
            ceilings,
            elements: Vec::new(),
            bytes: 0,
        }
    }

    fn crossed(&self, ceiling: ChainCeiling, position: usize) -> ChainFailure {
        ChainFailure::Budget {
            ceiling,
            stage: Some(self.stage),
            operator: Some(self.operator),
            position: Some(position),
        }
    }

    /// Adds `element`, produced from the input element at `position`, or fails at the first ceiling it crosses: the
    /// count, then the element's bytes, then its depth, then the stage in all.
    fn push(&mut self, position: usize, element: Element) -> Result<(), ChainFailure> {
        if self.elements.len() >= self.ceilings.max_stage_elements {
            return Err(self.crossed(ChainCeiling::StageElements, position));
        }
        if element.bytes.len() > self.ceilings.max_element_bytes {
            return Err(self.crossed(ChainCeiling::ElementBytes, position));
        }
        self.push_measured(position, element)
    }

    /// Adds a value a stage built, written through a writer that stops past the element ceiling.
    fn push_value(&mut self, position: usize, value: Value) -> Result<(), ChainFailure> {
        if self.elements.len() >= self.ceilings.max_stage_elements {
            return Err(self.crossed(ChainCeiling::StageElements, position));
        }
        let element = Element::bounded(value, self.ceilings.max_element_bytes)
            .ok_or_else(|| self.crossed(ChainCeiling::ElementBytes, position))?;
        self.push_measured(position, element)
    }

    fn push_measured(&mut self, position: usize, element: Element) -> Result<(), ChainFailure> {
        if nesting(&element.value) > MAX_ELEMENT_DEPTH {
            return Err(self.crossed(ChainCeiling::ElementDepth, position));
        }
        self.bytes = self.bytes.saturating_add(element.bytes.len());
        if self.bytes > self.ceilings.max_stage_bytes {
            return Err(self.crossed(ChainCeiling::StageBytes, position));
        }
        self.elements.push(element);
        Ok(())
    }
}

/// The cooperative deadline of the chain, checked between elements.
struct Deadline {
    at: Option<Instant>,
}

impl Deadline {
    fn check(
        &self,
        stage: usize,
        operator: OperatorName,
        position: usize,
    ) -> Result<(), ChainFailure> {
        match self.at {
            Some(at) if Instant::now() < at => Ok(()),
            _ => Err(ChainFailure::Budget {
                ceiling: ChainCeiling::WallTime,
                stage: Some(stage),
                operator: Some(operator),
                position: Some(position),
            }),
        }
    }
}

fn input_crossed() -> ChainFailure {
    ChainFailure::Budget {
        ceiling: ChainCeiling::InputElements,
        stage: None,
        operator: None,
        position: None,
    }
}

/// Applies `chain` (see the module documentation).
pub(crate) fn apply(
    input: ChainInput<'_>,
    chain: &OperatorChain,
    data: &RunData,
    ceilings: &ChainCeilings,
) -> Result<ChainAnswer, ChainFailure> {
    let started = Instant::now();
    let items = match input {
        ChainInput::Items(expression) => {
            Some(Compiled::compile(expression.as_str()).map_err(|_| ChainFailure::Items)?)
        }
        ChainInput::Elements(_) => None,
    };
    let mut compiled = Vec::with_capacity(chain.len());
    for (stage, operator) in chain.stages().iter().enumerate() {
        let expression = operator
            .expression()
            .map(|(_, expression)| Compiled::compile(expression.as_str()))
            .transpose()
            .map_err(|_| ChainFailure::Stage {
                stage,
                operator: operator.name(),
                position: 0,
                fault: StageFault::Expression,
            })?;
        compiled.push(expression);
    }
    // The environment is built once, in parts, so that a variable that cannot be read names who reads it first:
    // `items`, or the first stage that does.
    let mut loaded: BTreeSet<String> = [ELEMENT.to_owned(), POSITION.to_owned()].into();
    let mut env = Env::new();
    let mut load = |names: BTreeSet<String>, env: &mut Env| -> Result<(), ()> {
        let fresh: BTreeSet<String> = names.difference(&loaded).cloned().collect();
        if fresh.is_empty() {
            return Ok(());
        }
        env.absorb(crate::evaluator::environment(&fresh, data, None).map_err(|_| ())?);
        loaded.extend(fresh);
        Ok(())
    };
    if let Some(items) = &items {
        load(items.variables(), &mut env).map_err(|()| ChainFailure::Items)?;
    }
    for (stage, (operator, expression)) in chain.stages().iter().zip(&compiled).enumerate() {
        if let Some(expression) = expression {
            load(expression.variables(), &mut env).map_err(|()| ChainFailure::Stage {
                stage,
                operator: operator.name(),
                position: 0,
                fault: StageFault::Expression,
            })?;
        }
    }

    let mut list = match (input, items) {
        (ChainInput::Items(_), Some(items)) => {
            match items.eval(&env).map_err(|_| ChainFailure::Items)? {
                Value::Array(values) => {
                    if values.len() > ceilings.max_input {
                        return Err(input_crossed());
                    }
                    values.into_iter().map(Element::new).collect::<Vec<_>>()
                }
                other => {
                    return descriptor_from_json(&other)
                        .map(ChainAnswer::Blob)
                        .ok_or(ChainFailure::Items);
                }
            }
        }
        (ChainInput::Elements(elements), _) => {
            if elements.len() > ceilings.max_input {
                return Err(input_crossed());
            }
            elements
                .iter()
                .map(|bytes| {
                    serde_json::from_slice(bytes)
                        .map(|value| Element {
                            value,
                            bytes: bytes.clone(),
                        })
                        .map_err(|_| ChainFailure::Items)
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        (ChainInput::Items(_), None) => return Err(ChainFailure::Items),
    };
    let source_count = u32::try_from(list.len()).map_err(|_| input_crossed())?;
    let deadline = Deadline {
        at: started.checked_add(ceilings.wall_time),
    };

    let mut tallies = Vec::with_capacity(chain.len());
    for (stage, (operator, expression)) in chain.stages().iter().zip(&compiled).enumerate() {
        let name = operator.name();
        let input_count = list.len();
        let mut out = Produced::new(stage, name, ceilings);
        let fault = |position, fault| ChainFailure::Stage {
            stage,
            operator: name,
            position,
            fault,
        };
        match (operator, expression) {
            (Operator::Filter { .. }, Some(predicate)) => {
                for (position, element) in list.into_iter().enumerate() {
                    deadline.check(stage, name, position)?;
                    let (result, value) = evaluate(predicate, &mut env, element.value, position);
                    match result {
                        Ok(Value::Bool(true)) => out.push(
                            position,
                            Element {
                                value,
                                bytes: element.bytes,
                            },
                        )?,
                        Ok(Value::Bool(false)) => {}
                        Ok(_) => return Err(fault(position, StageFault::NotBoolean)),
                        Err(()) => return Err(fault(position, StageFault::Expression)),
                    }
                }
            }
            (Operator::Map { .. }, Some(select)) => {
                for (position, element) in list.into_iter().enumerate() {
                    deadline.check(stage, name, position)?;
                    let (result, _) = evaluate(select, &mut env, element.value, position);
                    let value = result.map_err(|()| fault(position, StageFault::Expression))?;
                    out.push_value(position, value)?;
                }
            }
            (Operator::FlatMap { .. }, Some(select)) => {
                for (position, element) in list.into_iter().enumerate() {
                    deadline.check(stage, name, position)?;
                    let (result, _) = evaluate(select, &mut env, element.value, position);
                    match result {
                        Ok(Value::Array(values)) => {
                            for value in values {
                                out.push_value(position, value)?;
                            }
                        }
                        Ok(_) => return Err(fault(position, StageFault::NotAList)),
                        Err(()) => return Err(fault(position, StageFault::Expression)),
                    }
                }
            }
            (Operator::Skip { count }, None) => {
                let skip = usize::try_from(*count).unwrap_or(usize::MAX);
                for (position, element) in list.into_iter().enumerate().skip(skip) {
                    deadline.check(stage, name, position)?;
                    out.push(position, element)?;
                }
            }
            (Operator::Take { count }, None) => {
                let take = usize::try_from(*count).unwrap_or(usize::MAX);
                for (position, element) in list.into_iter().enumerate().take(take) {
                    deadline.check(stage, name, position)?;
                    out.push(position, element)?;
                }
            }
            (Operator::Distinct { by }, by_expression) => {
                let mut seen: HashSet<[u8; 32]> = HashSet::with_capacity(input_count);
                for (position, element) in list.into_iter().enumerate() {
                    deadline.check(stage, name, position)?;
                    let (key, value) = match (by, by_expression) {
                        (Some(_), Some(by)) => {
                            let (result, value) = evaluate(by, &mut env, element.value, position);
                            let key =
                                result.map_err(|()| fault(position, StageFault::Expression))?;
                            (distinct_digest(&key), value)
                        }
                        _ => (distinct_digest(&element.value), element.value),
                    };
                    if seen.insert(key) {
                        out.push(
                            position,
                            Element {
                                value,
                                bytes: element.bytes,
                            },
                        )?;
                    }
                }
            }
            (Operator::Window { count }, None) => {
                let size = usize::try_from(*count).unwrap_or(usize::MAX).max(1);
                let mut window: Vec<Element> = Vec::with_capacity(size.min(input_count));
                let mut first = 0;
                for (position, element) in list.into_iter().enumerate() {
                    deadline.check(stage, name, position)?;
                    if window.is_empty() {
                        first = position;
                    }
                    window.push(element);
                    if window.len() == size {
                        out.push(first, joined(std::mem::take(&mut window)))?;
                    }
                }
                if !window.is_empty() {
                    out.push(first, joined(window))?;
                }
            }
            // The domain pairs every operator with its expression; a mismatch is a defect, failed closed.
            _ => return Err(fault(0, StageFault::Expression)),
        }
        tallies.push(StageTally {
            operator: name,
            input: u32::try_from(input_count).unwrap_or(u32::MAX),
            output: u32::try_from(out.elements.len()).unwrap_or(u32::MAX),
        });
        list = out.elements;
    }
    Ok(ChainAnswer::Output(ChainOutput {
        source_count,
        items: list.into_iter().map(|element| element.bytes).collect(),
        chain_digest: crate::operators::chain_digest(chain),
        stages: tallies,
    }))
}

/// Evaluates `expression` with `element` and `position` bound, and gives the element back. The failure carries no
/// detail: an expression error may quote a value (D042-024).
fn evaluate(
    expression: &Compiled,
    env: &mut Env,
    element: Value,
    position: usize,
) -> (Result<Value, ()>, Value) {
    env.set(ELEMENT, element);
    env.set(POSITION, json!(position));
    let result = expression.eval(env).map_err(|_| ());
    let element = env.remove(ELEMENT).unwrap_or(Value::Null);
    (result, element)
}

/// A window: the list of its elements, whose compact JSON is that of the elements joined.
fn joined(window: Vec<Element>) -> Element {
    let mut bytes = Vec::with_capacity(window.iter().map(|e| e.bytes.len() + 1).sum::<usize>() + 1);
    bytes.push(b'[');
    let mut values = Vec::with_capacity(window.len());
    for (index, element) in window.into_iter().enumerate() {
        if index > 0 {
            bytes.push(b',');
        }
        bytes.extend_from_slice(&element.bytes);
        values.push(element.value);
    }
    bytes.push(b']');
    Element {
        value: Value::Array(values),
        bytes,
    }
}

/// The SHA-256 of [`distinct_form`], what `distinct` remembers; kept in memory only.
fn distinct_digest(value: &Value) -> [u8; 32] {
    Sha256::digest(distinct_form(value)).into()
}

/// The largest integer every double below it in magnitude represents exactly: 2^53.
const EXACT_DOUBLE: f64 = 9_007_199_254_740_992.0;

/// The canonical form `distinct` compares (FR-042-011, D042-025), written by this function alone so that it does not
/// depend on the `serde_json` features of the build: an integral double within plus or minus 2^53 is written as the
/// integer, `-0` as `0`, object keys in code-point order, strings as they are (no normalisation, as `==` does).
#[must_use]
pub fn distinct_form(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_form(value, &mut out);
    out
}

fn write_form(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                out.extend_from_slice(integer.to_string().as_bytes());
            } else if let Some(integer) = number.as_u64() {
                out.extend_from_slice(integer.to_string().as_bytes());
            } else if let Some(double) = number.as_f64() {
                if double.fract() == 0.0 && double.abs() <= EXACT_DOUBLE {
                    // Exact: an integral double of this magnitude is an integer `i64` holds; `-0.0` becomes `0`.
                    #[allow(clippy::cast_possible_truncation)]
                    let integer = double as i64;
                    out.extend_from_slice(integer.to_string().as_bytes());
                } else {
                    out.extend_from_slice(number.to_string().as_bytes());
                }
            }
        }
        Value::String(text) => write_string(text, out),
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_form(item, out);
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            // `str` orders by UTF-8 bytes, which is code-point order.
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push(b'{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_string(key, out);
                out.push(b':');
                write_form(item, out);
            }
            out.push(b'}');
        }
    }
}

fn write_string(text: &str, out: &mut Vec<u8>) {
    // Serialising a `&str` cannot fail.
    if let Ok(escaped) = serde_json::to_string(text) {
        out.extend_from_slice(escaped.as_bytes());
    }
}

#[cfg(test)]
mod tests;
