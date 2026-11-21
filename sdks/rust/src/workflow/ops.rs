//! The operator factory of a collection (design 042, FR-042-028). Each function returns one stage of the
//! `operators` of a `forEach`, an object with exactly one key, the operator's name; pass the stages to
//! [`Builder::for_each_with_operators`](super::Builder::for_each_with_operators) or
//! [`Builder::map_with_operators`](super::Builder::map_with_operators).
//!
//! The factory only assembles a stage; the server validates the chain (its length, the ranges of the
//! counts and the variables an expression reads). Stage expressions read `element`, `position`,
//! `input`, `steps`, `run` and `state`.
use serde_json::{Map, Value, json};

/// One stage of a collection's operator chain, as `workflow.schema.json` names it.
#[derive(Clone, Debug, PartialEq)]
pub struct Operator {
    name: &'static str,
    parameters: Map<String, Value>,
}

impl Operator {
    fn new(name: &'static str, parameters: Value) -> Self {
        let parameters = match parameters {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        Self { name, parameters }
    }
    /// The operator's name as the IR writes it: `filter`, `map`, `flatMap`, `skip`, `take`,
    /// `distinct` or `window`.
    pub fn name(&self) -> &str {
        self.name
    }
    /// The stage as the IR writes it, `{name: parameters}`.
    pub fn to_value(&self) -> Value {
        let mut stage = Map::new();
        stage.insert(self.name.to_owned(), Value::Object(self.parameters.clone()));
        Value::Object(stage)
    }
}

/// Keeps the elements for which `predicate` (the IR's `where`) is true.
pub fn filter(predicate: impl Into<String>) -> Operator {
    Operator::new("filter", json!({"where": predicate.into()}))
}
/// Replaces each element by the value of `select`.
pub fn map(select: impl Into<String>) -> Operator {
    Operator::new("map", json!({"select": select.into()}))
}
/// `select` yields a list for each element; the lists are concatenated in order, one level deep.
pub fn flat_map(select: impl Into<String>) -> Operator {
    Operator::new("flatMap", json!({"select": select.into()}))
}
/// Drops the first `count` elements.
pub fn skip(count: u32) -> Operator {
    Operator::new("skip", json!({"count": count}))
}
/// Keeps the first `count` elements.
pub fn take(count: u32) -> Operator {
    Operator::new("take", json!({"count": count}))
}
/// Keeps the first occurrence of each element.
pub fn distinct() -> Operator {
    Operator::new("distinct", json!({}))
}
/// Keeps the first element of each value of `by`.
pub fn distinct_by(by: impl Into<String>) -> Operator {
    Operator::new("distinct", json!({"by": by.into()}))
}
/// Replaces the list by consecutive lists of `count` elements, the last one possibly shorter.
pub fn window(count: u32) -> Operator {
    Operator::new("window", json!({"count": count}))
}
