//! Steps 3 to 5 of the pre-check and the compiled form (D043-056, D043-057, D043-059, D043-062, D043-069).
//!
//! The schema is compiled into an arena of nodes, one per schema position the vocabulary walk recorded, and a `$ref`
//! becomes the index of its target node, never an inlined copy, so a DAG of references stays the size of the
//! document. A depth-first search with white, grey and black marks over the application edges (references and every
//! child schema) refuses a cycle at a grey node, and stores with each node as it turns black its longest application
//! path and a lower bound, in units, of what every input the node accepts pays there, so the pass is linear in the
//! arena. The compiled form keeps document order in vectors and sorted keys in its lookup tables: no iteration depends
//! on a hash seed, so every replica reports the same first violation.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use super::pattern::{self, Pattern};
use super::value::{Num, heap, size};
use super::vocabulary::{self, Position};
use super::{
    InputSchemaIssue, MAX_APPLICATION_PATH, MAX_PATTERN_PROGRAMS_BYTES, MAX_PATTERNS,
    RUN_INPUT_WORK_UNITS, Unusable, pointer,
};

/// The type names, as bits.
pub(super) mod types {
    pub(in super::super) const NULL: u8 = 1;
    pub(in super::super) const BOOLEAN: u8 = 2;
    pub(in super::super) const OBJECT: u8 = 4;
    pub(in super::super) const ARRAY: u8 = 8;
    pub(in super::super) const NUMBER: u8 = 16;
    pub(in super::super) const STRING: u8 = 32;
    pub(in super::super) const INTEGER: u8 = 64;
}

/// A value of `enum` or `const`, with its size for the counter.
pub(super) struct Literal {
    pub(super) value: Value,
    pub(super) size: u64,
}

/// One schema position, compiled.
#[derive(Default)]
pub(super) struct Node {
    /// `true` or `false` as a schema.
    pub(super) boolean: Option<bool>,
    pub(super) reference: Option<usize>,
    pub(super) types: Option<(u8, u64)>,
    pub(super) enumeration: Option<Vec<Literal>>,
    /// The largest member of `enumeration`: the measure of the instance stops there.
    pub(super) enumeration_largest: u64,
    pub(super) constant: Option<Literal>,
    pub(super) minimum: Option<Num>,
    pub(super) maximum: Option<Num>,
    pub(super) exclusive_minimum: Option<Num>,
    pub(super) exclusive_maximum: Option<Num>,
    pub(super) multiple_of: Option<u64>,
    pub(super) min_length: Option<u64>,
    pub(super) max_length: Option<u64>,
    pub(super) pattern: Option<usize>,
    pub(super) min_items: Option<u64>,
    pub(super) max_items: Option<u64>,
    pub(super) items: Option<usize>,
    pub(super) min_properties: Option<u64>,
    pub(super) max_properties: Option<u64>,
    pub(super) required: Vec<Arc<str>>,
    /// Sorted by name, for a binary search.
    pub(super) properties: Vec<(Arc<str>, usize)>,
    /// The length of the longest name of `properties`: what one comparison of a lookup can cost.
    pub(super) longest_property: usize,
    pub(super) additional: Option<usize>,
    pub(super) all_of: Vec<usize>,
    pub(super) any_of: Vec<usize>,
    pub(super) one_of: Vec<usize>,
}

impl Node {
    /// What the node keeps beyond its own size: its literals, its names and its lists (D043-105).
    fn heap(&self) -> usize {
        let literal = |literal: &Literal| std::mem::size_of::<Literal>() + heap(&literal.value);
        let name = |name: &Arc<str>| name.len() + 16;
        self.enumeration
            .iter()
            .flatten()
            .map(literal)
            .sum::<usize>()
            + self.constant.iter().map(literal).sum::<usize>()
            + self
                .required
                .iter()
                .map(|n| name(n) + std::mem::size_of::<Arc<str>>())
                .sum::<usize>()
            + self
                .properties
                .iter()
                .map(|(n, _)| name(n) + std::mem::size_of::<(Arc<str>, usize)>())
                .sum::<usize>()
            + (self.all_of.len() + self.any_of.len() + self.one_of.len())
                * std::mem::size_of::<usize>()
    }

    /// The nodes this one applies to the same or a child instance: its application edges.
    fn edges(&self) -> impl Iterator<Item = usize> + '_ {
        self.reference
            .iter()
            .chain(self.properties.iter().map(|(_, child)| child))
            .chain(self.items.iter())
            .chain(self.additional.iter())
            .chain(self.all_of.iter())
            .chain(self.any_of.iter())
            .chain(self.one_of.iter())
            .copied()
    }
}

/// A schema of the subset, compiled: what the validator runs and the cache keeps.
pub struct CompiledInputSchema {
    pub(super) nodes: Vec<Node>,
    pub(super) patterns: Vec<Pattern>,
    weight: usize,
    path: usize,
    bound: u64,
}

impl std::fmt::Debug for CompiledInputSchema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledInputSchema")
            .field("nodes", &self.nodes.len())
            .field("patterns", &self.patterns.len())
            .field("weight", &self.weight)
            .finish_non_exhaustive()
    }
}

impl CompiledInputSchema {
    /// Runs the whole pre-check on `schema` (the canonical bytes of an `inputSchema`) and compiles it.
    ///
    /// # Errors
    /// The first violation, as [`Unusable`]: the issue, the keyword and its JSON pointer in the schema.
    pub fn compile(schema: &[u8]) -> Result<Self, Unusable> {
        if schema.len() > super::MAX_INPUT_SCHEMA_BYTES {
            return Err(Unusable::new(InputSchemaIssue::TooLarge, "", ""));
        }
        let root: Value = serde_json::from_slice(schema)
            .map_err(|_| Unusable::new(InputSchemaIssue::KeywordUnsupported, "", ""))?;
        Self::compile_value(schema.len(), &root)
    }

    pub(super) fn compile_value(schema_bytes: usize, root: &Value) -> Result<Self, Unusable> {
        let positions = vocabulary::positions(schema_bytes, root)?;
        let index: BTreeMap<&str, usize> = positions
            .iter()
            .enumerate()
            .map(|(i, position)| (position.pointer.as_str(), i))
            .collect();
        let mut nodes = Vec::with_capacity(positions.len());
        let mut sources: Vec<(usize, &str)> = Vec::new();
        for (at, position) in positions.iter().enumerate() {
            nodes.push(node(position, &index, at, &mut sources)?);
        }
        let (path, bound) = search(&nodes, &positions)?;
        let mut patterns = Vec::with_capacity(sources.len());
        let mut programs = 0usize;
        let mut weight = schema_bytes
            .saturating_mul(8)
            .saturating_add(nodes.len().saturating_mul(std::mem::size_of::<Node>()))
            .saturating_add(nodes.iter().map(Node::heap).sum::<usize>());
        for (count, (at, source)) in sources.into_iter().enumerate() {
            let pointer = format!("{}/pattern", positions[at].pointer);
            let refused =
                || Unusable::new(InputSchemaIssue::PatternUnsupported, "pattern", &pointer);
            if count >= MAX_PATTERNS {
                return Err(refused());
            }
            let compiled = pattern::compile(source).map_err(|_| refused())?;
            programs = programs.saturating_add(compiled.program_bytes);
            if programs > MAX_PATTERN_PROGRAMS_BYTES {
                return Err(refused());
            }
            weight = weight.saturating_add(compiled.weight);
            nodes[at].pattern = Some(patterns.len());
            patterns.push(compiled);
        }
        Ok(Self {
            nodes,
            patterns,
            weight,
            path,
            bound,
        })
    }

    /// The nodes of the arena: one per schema position of the document, however often references reach them.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// The longest application path from the root, in nodes.
    #[must_use]
    pub const fn application_path(&self) -> usize {
        self.path
    }

    /// The lower bound, in units, of what any input the schema accepts pays (D043-069).
    #[must_use]
    pub const fn lower_bound(&self) -> u64 {
        self.bound
    }

    /// The NFA states of the first `pattern` of the schema, if it has one: what a match of it is charged per byte.
    #[must_use]
    pub fn pattern_states(&self) -> Option<u64> {
        self.patterns.first().map(|pattern| pattern.states)
    }

    /// The estimated bytes this compiled schema keeps, its patterns' runtime caches included (D043-064).
    #[must_use]
    pub const fn weight(&self) -> usize {
        self.weight
    }
}

fn number(value: &Value) -> Option<Num> {
    value.as_number().map(Num::of)
}

fn node<'a>(
    position: &Position<'a>,
    index: &BTreeMap<&str, usize>,
    at: usize,
    sources: &mut Vec<(usize, &'a str)>,
) -> Result<Node, Unusable> {
    let mut node = Node::default();
    let object = match position.value {
        Value::Bool(value) => {
            node.boolean = Some(*value);
            return Ok(node);
        }
        other => vocabulary::object(other).ok_or_else(|| {
            Unusable::new(InputSchemaIssue::KeywordUnsupported, "", &position.pointer)
        })?,
    };
    let child = |keyword: &str, rest: &str| -> Result<usize, Unusable> {
        let pointer = format!("{}/{keyword}{rest}", position.pointer);
        index
            .get(pointer.as_str())
            .copied()
            .ok_or_else(|| Unusable::new(InputSchemaIssue::KeywordUnsupported, keyword, &pointer))
    };
    for (key, member) in object {
        match key.as_str() {
            "$ref" => {
                let reference = member.as_str().unwrap_or_default();
                let refused = || {
                    Unusable::new(
                        InputSchemaIssue::ReferenceInvalid,
                        "$ref",
                        &format!("{}/$ref", position.pointer),
                    )
                };
                let target = pointer::decode(reference).ok_or_else(refused)?;
                node.reference = Some(*index.get(target.as_str()).ok_or_else(refused)?);
            }
            "type" => {
                let names: Vec<&str> = match member {
                    Value::String(name) => vec![name.as_str()],
                    Value::Array(names) => names.iter().filter_map(Value::as_str).collect(),
                    _ => Vec::new(),
                };
                let mut bits = 0u8;
                for name in &names {
                    bits |= match *name {
                        "null" => types::NULL,
                        "boolean" => types::BOOLEAN,
                        "object" => types::OBJECT,
                        "array" => types::ARRAY,
                        "number" => types::NUMBER,
                        "string" => types::STRING,
                        _ => types::INTEGER,
                    };
                }
                node.types = Some((bits, names.len() as u64));
            }
            "enum" => {
                let members: Vec<Literal> = member
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .map(|value| Literal {
                                size: size(value),
                                value: value.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                node.enumeration_largest = members.iter().map(|m| m.size).max().unwrap_or(0);
                node.enumeration = Some(members);
            }
            "const" => {
                node.constant = Some(Literal {
                    size: size(member),
                    value: member.clone(),
                });
            }
            "minimum" => node.minimum = number(member),
            "maximum" => node.maximum = number(member),
            "exclusiveMinimum" => node.exclusive_minimum = number(member),
            "exclusiveMaximum" => node.exclusive_maximum = number(member),
            "multipleOf" => node.multiple_of = member.as_u64(),
            "minLength" => node.min_length = member.as_u64(),
            "maxLength" => node.max_length = member.as_u64(),
            "minItems" => node.min_items = member.as_u64(),
            "maxItems" => node.max_items = member.as_u64(),
            "minProperties" => node.min_properties = member.as_u64(),
            "maxProperties" => node.max_properties = member.as_u64(),
            "pattern" => sources.push((at, member.as_str().unwrap_or_default())),
            "required" => {
                node.required = member
                    .as_array()
                    .map(|names| {
                        names
                            .iter()
                            .filter_map(Value::as_str)
                            .map(Arc::from)
                            .collect()
                    })
                    .unwrap_or_default();
            }
            "properties" => {
                if let Some(names) = member.as_object() {
                    for name in names.keys() {
                        let target = child("properties", &format!("/{}", pointer::escape(name)))?;
                        node.properties.push((Arc::from(name.as_str()), target));
                    }
                }
                // The map is sorted by key already; sort again so nothing depends on the map type.
                node.properties.sort_by(|a, b| a.0.cmp(&b.0));
                node.longest_property = node
                    .properties
                    .iter()
                    .map(|(name, _)| name.len())
                    .max()
                    .unwrap_or(0);
            }
            "items" => node.items = Some(child("items", "")?),
            "additionalProperties" => node.additional = Some(child("additionalProperties", "")?),
            "allOf" | "anyOf" | "oneOf" => {
                let count = member.as_array().map_or(0, Vec::len);
                let branches = (0..count)
                    .map(|i| child(key, &format!("/{i}")))
                    .collect::<Result<Vec<_>, _>>()?;
                match key.as_str() {
                    "allOf" => node.all_of = branches,
                    "anyOf" => node.any_of = branches,
                    _ => node.one_of = branches,
                }
            }
            // Annotations, containers and the root's `$schema` and `$id`: nothing to evaluate.
            _ => {}
        }
    }
    Ok(node)
}

/// The marked depth-first search: refuses a cycle, a path past [`MAX_APPLICATION_PATH`] and a root whose lower bound
/// passes the budget, and answers the root's path and bound.
fn search(nodes: &[Node], positions: &[Position<'_>]) -> Result<(usize, u64), Unusable> {
    const WHITE: u8 = 0;
    const GREY: u8 = 1;
    const BLACK: u8 = 2;
    let mut mark = vec![WHITE; nodes.len()];
    let mut path = vec![0usize; nodes.len()];
    let mut bound = vec![0u64; nodes.len()];
    for start in 0..nodes.len() {
        if mark[start] != WHITE {
            continue;
        }
        // (node, its edges still to visit)
        let mut stack: Vec<(usize, Vec<usize>)> = vec![(start, nodes[start].edges().collect())];
        mark[start] = GREY;
        while let Some((current, pending)) = stack.last_mut() {
            let current = *current;
            if let Some(next) = pending.pop() {
                match mark[next] {
                    WHITE => {
                        mark[next] = GREY;
                        stack.push((next, nodes[next].edges().collect()));
                    }
                    GREY => {
                        // The cycle is the stack from `next` to `current`: name the first reference on it.
                        let on_cycle = stack
                            .iter()
                            .map(|(node, _)| *node)
                            .skip_while(|node| *node != next)
                            .find(|node| nodes[*node].reference.is_some())
                            .unwrap_or(current);
                        return Err(Unusable::new(
                            InputSchemaIssue::ReferenceInvalid,
                            "$ref",
                            &format!("{}/$ref", positions[on_cycle].pointer),
                        ));
                    }
                    _ => {}
                }
            } else {
                stack.pop();
                mark[current] = BLACK;
                let node = &nodes[current];
                path[current] = 1 + node.edges().map(|edge| path[edge]).max().unwrap_or(0);
                if path[current] > MAX_APPLICATION_PATH {
                    return Err(Unusable::new(
                        InputSchemaIssue::TooDeep,
                        "",
                        &positions[current].pointer,
                    ));
                }
                bound[current] = lower_bound(node, &bound);
            }
        }
    }
    let root_bound = bound.first().copied().unwrap_or(0);
    if root_bound > RUN_INPUT_WORK_UNITS {
        return Err(Unusable::new(InputSchemaIssue::TooCostly, "", ""));
    }
    Ok((path.first().copied().unwrap_or(0), root_bound))
}

/// What every input the node accepts pays there, at least (D043-069): the application, the target of its `$ref`, one
/// unit per `type` name and per `enum` member, one for `const`, every `allOf` branch, the cheapest `anyOf` branch, and the
/// cheapest `oneOf` branch with two units for each of its others. Every sum saturates. A refused input may stop earlier,
/// at its first violation; an accepted one cannot.
fn lower_bound(node: &Node, bound: &[u64]) -> u64 {
    let mut total: u64 = 1;
    if let Some(target) = node.reference {
        total = total.saturating_add(1).saturating_add(bound[target]);
    }
    if let Some((_, names)) = node.types {
        total = total.saturating_add(names);
    }
    if let Some(members) = &node.enumeration {
        total = total.saturating_add(members.len() as u64);
    }
    if node.constant.is_some() {
        total = total.saturating_add(1);
    }
    for branch in &node.all_of {
        total = total.saturating_add(1).saturating_add(bound[*branch]);
    }
    // `oneOf` evaluates every branch, but all of them but the one that passes can fail at their first keyword: the
    // cheapest branch whole, and two units (its branch and its application) for each of the others (D043-101).
    if let Some(cheapest) = node.one_of.iter().map(|branch| bound[*branch]).min() {
        let others = (node.one_of.len() as u64 - 1).saturating_mul(2);
        total = total
            .saturating_add(1)
            .saturating_add(cheapest)
            .saturating_add(others);
    }
    if let Some(cheapest) = node.any_of.iter().map(|branch| bound[*branch]).min() {
        total = total.saturating_add(1).saturating_add(cheapest);
    }
    total
}
