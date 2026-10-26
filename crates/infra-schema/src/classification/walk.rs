//! The bounded schema walk behind payload disclosure.
//!
//! The walk runs inside the disclosure read transaction, so its cost must be bounded by the payload
//! and the carrier together, never by their product. One [`Walk`] owns a step budget for a whole
//! projection: every expanded subschema is charged once per instance node it applies to, which is
//! also what the validator will spend on the same node. Work that does not depend on the element
//! (the schemas that govern every remaining array item, or every key no schema names) is computed
//! once per container.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use sagat_application::classification::{
    ClassifiedChild, ClassifiedNode, DisclosedLevels, JsonNumber, MAX_CONTAINER_CHILDREN,
    MAX_DISCLOSED_STRING_BYTES, MAX_PROJECTION_DEPTH, MAX_PROJECTION_NODES, MAX_REF_EXPANSIONS,
    MAX_RESPONSE_BYTES, MAX_STRING_BYTES, ProjectionFailure, ProjectionRequest, ScalarValue,
    ValueKind,
};
use sagat_domain::classification::{Level, Visibility, WithheldReason, visibility};
use serde_json::{Map, Value};

use super::numbers::{Lexeme, schema_requires_integer};

const VALUE_NODE_BYTES: usize = 128;
const CONTAINER_NODE_BYTES: usize = 128;
const WITHHELD_NODE_BYTES: usize = 128;
const CHILD_BYTES: usize = 48;

/// How many steps pass between two reads of the clock.
const CLOCK_INTERVAL: usize = 4_096;

/// Keywords whose subschemas the walk visits itself, node by node. Everything else attached to a
/// schema is evaluated by the validator at the same instance node and is charged as local cost.
const WALKED_KEYWORDS: [&str; 10] = [
    "properties",
    "patternProperties",
    "additionalProperties",
    "items",
    "prefixItems",
    "allOf",
    "anyOf",
    "oneOf",
    "$defs",
    "definitions",
];

/// Keywords that make a validator descend into children of the node it is evaluating.
const DESCENDING_KEYWORDS: [&str; 13] = [
    "properties",
    "patternProperties",
    "additionalProperties",
    "items",
    "prefixItems",
    "additionalItems",
    "contains",
    "propertyNames",
    "unevaluatedProperties",
    "unevaluatedItems",
    "dependentSchemas",
    "dependencies",
    "$ref",
];

/// The step budget, clock and caches of one projection.
pub(super) struct Walk<'a> {
    root: &'a Value,
    steps: usize,
    limit: usize,
    next_clock: usize,
    deadline: Option<Instant>,
    costs: HashMap<usize, (usize, usize)>,
}

impl<'a> Walk<'a> {
    pub(super) fn new(root: &'a Value, limit: usize, deadline: Option<Instant>) -> Self {
        Self {
            root,
            steps: 0,
            limit,
            next_clock: CLOCK_INTERVAL,
            deadline,
            costs: HashMap::new(),
        }
    }

    pub(super) const fn steps(&self) -> usize {
        self.steps
    }

    /// Charges `amount` steps. Running out of steps or time ends the walk closed.
    pub(super) fn charge(&mut self, amount: usize) -> Result<(), ProjectionFailure> {
        if self.steps.saturating_add(amount) > self.limit {
            self.steps = self.limit;
            return Err(ProjectionFailure::SchemaUnusable);
        }
        self.steps += amount;
        if self.steps >= self.next_clock {
            self.next_clock = self.steps.saturating_add(CLOCK_INTERVAL);
            if self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err(ProjectionFailure::SchemaUnusable);
            }
        }
        Ok(())
    }

    /// Applicators that govern one location: the schemas themselves, every `$ref` target and
    /// every `allOf`/`anyOf`/`oneOf` branch. The reference limit holds per location, so a long
    /// array of referenced items never exhausts it; the step budget bounds the whole walk.
    pub(super) fn expand(
        &mut self,
        schemas: &[&'a Value],
    ) -> Result<Vec<&'a Value>, ProjectionFailure> {
        let mut expansion = Expansion {
            references: 0,
            active: Vec::new(),
        };
        let mut expanded = Vec::new();
        for schema in schemas {
            self.expand_one(schema, &mut expansion, &mut expanded)?;
        }
        Ok(expanded)
    }

    fn expand_one(
        &mut self,
        schema: &'a Value,
        expansion: &mut Expansion<'a>,
        expanded: &mut Vec<&'a Value>,
    ) -> Result<(), ProjectionFailure> {
        self.charge(1)?;
        expanded.push(schema);
        let Some(object) = schema.as_object() else {
            return Ok(());
        };
        if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
            if !reference.starts_with("#/") {
                return Err(ProjectionFailure::SchemaUnusable);
            }
            expansion.references += 1;
            if expansion.references > MAX_REF_EXPANSIONS || expansion.active.contains(&reference) {
                return Err(ProjectionFailure::SchemaUnusable);
            }
            expansion.active.push(reference);
            let target = self
                .root
                .pointer(&reference[1..])
                .ok_or(ProjectionFailure::SchemaUnusable)?;
            self.expand_one(target, expansion, expanded)?;
            expansion.active.pop();
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(branches) = object.get(keyword).and_then(Value::as_array) {
                for branch in branches {
                    self.expand_one(branch, expansion, expanded)?;
                }
            }
        }
        Ok(())
    }

    /// `(flat, deep)` validator work a schema adds at its own instance node. `deep` is work in
    /// keywords outside the walked set that also descend into children, so it repeats per
    /// descendant.
    fn local_cost(&mut self, schema: &'a Value) -> (usize, usize) {
        let key = std::ptr::from_ref(schema) as usize;
        if let Some(cost) = self.costs.get(&key) {
            return *cost;
        }
        let mut flat = 1usize;
        let mut deep = 0usize;
        if let Some(object) = schema.as_object() {
            for (keyword, value) in object {
                // A `$ref` is resolved by `expand`, which lists its target as a schema of its own.
                if keyword == "$ref" || WALKED_KEYWORDS.contains(&keyword.as_str()) {
                    continue;
                }
                let size = subtree_size(value);
                if descends(value) || DESCENDING_KEYWORDS.contains(&keyword.as_str()) {
                    deep = deep.saturating_add(size);
                } else {
                    flat = flat.saturating_add(size);
                }
            }
        }
        self.costs.insert(key, (flat, deep));
        (flat, deep)
    }
}

struct Expansion<'a> {
    references: usize,
    active: Vec<&'a str>,
}

fn subtree_size(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(subtree_size).sum::<usize>(),
        Value::Object(values) => 1 + values.values().map(subtree_size).sum::<usize>(),
        _ => 1,
    }
}

fn descends(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(descends),
        Value::Object(values) => values.iter().any(|(keyword, value)| {
            DESCENDING_KEYWORDS.contains(&keyword.as_str()) || descends(value)
        }),
        _ => false,
    }
}

/// Counts instance nodes below `value`, stopping once `cap` is passed.
fn descendants(value: &Value, cap: usize) -> usize {
    let mut counted = 0usize;
    let mut pending = vec![value];
    while let Some(next) = pending.pop() {
        counted += 1;
        if counted > cap {
            return counted;
        }
        match next {
            Value::Array(values) => pending.extend(values),
            Value::Object(values) => pending.extend(values.values()),
            _ => {}
        }
    }
    counted
}

/// Names that at least one expanded schema declares in `properties`.
fn declared_names<'a>(
    walk: &mut Walk<'a>,
    schemas: &[&'a Value],
) -> Result<HashSet<&'a str>, ProjectionFailure> {
    let mut names = HashSet::new();
    for schema in schemas {
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            walk.charge(properties.len())?;
            names.extend(properties.keys().map(String::as_str));
        }
    }
    Ok(names)
}

fn ensure_names<'a>(
    walk: &mut Walk<'a>,
    expanded: &[&'a Value],
    names: &mut Option<HashSet<&'a str>>,
) -> Result<(), ProjectionFailure> {
    if names.is_none() {
        *names = Some(declared_names(walk, expanded)?);
    }
    Ok(())
}

fn ensure_prefix(
    walk: &mut Walk<'_>,
    expanded: &[&Value],
    prefix: &mut Option<usize>,
) -> Result<usize, ProjectionFailure> {
    if let Some(known) = *prefix {
        return Ok(known);
    }
    walk.charge(expanded.len())?;
    let length = prefix_length(expanded);
    *prefix = Some(length);
    Ok(length)
}

/// Longest `prefixItems` among the schemas: every index past it has the same governing schemas.
fn prefix_length(schemas: &[&Value]) -> usize {
    schemas
        .iter()
        .filter_map(|schema| schema.get("prefixItems").and_then(Value::as_array))
        .map(Vec::len)
        .max()
        .unwrap_or(0)
}

/// Child schemas of an object key, whether any schema names it, and whether a pattern map governs
/// it without being interpreted (the key is then treated as at least `pii`).
fn object_children<'a>(
    walk: &mut Walk<'a>,
    name: &str,
    schemas: &[&'a Value],
) -> Result<(Vec<&'a Value>, bool, bool), ProjectionFailure> {
    walk.charge(schemas.len())?;
    let mut children = Vec::new();
    let mut named = false;
    let mut unsupported_governance = false;
    for schema in schemas {
        if let Some(property) = schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(name))
        {
            named = true;
            children.push(property);
        } else {
            let patterns = schema
                .get("patternProperties")
                .and_then(Value::as_object)
                .is_some_and(|patterns| !patterns.is_empty());
            if patterns {
                unsupported_governance = true;
            } else if let Some(additional) = schema.get("additionalProperties")
                && additional.is_object()
            {
                children.push(additional);
            }
        }
    }
    Ok((children, named, unsupported_governance))
}

fn array_children<'a>(
    walk: &mut Walk<'a>,
    index: usize,
    schemas: &[&'a Value],
) -> Result<Vec<&'a Value>, ProjectionFailure> {
    walk.charge(schemas.len())?;
    let mut children = Vec::new();
    for schema in schemas {
        if let Some(prefix) = schema.get("prefixItems").and_then(Value::as_array)
            && let Some(item) = prefix.get(index)
        {
            children.push(item);
            continue;
        }
        if let Some(items) = schema.get("items")
            && items.is_object()
        {
            children.push(items);
        }
    }
    Ok(children)
}

/// The expanded schemas of one instance location and the validator work they imply.
struct Shape<'a> {
    expanded: Vec<&'a Value>,
    requires_integer: bool,
    flat: usize,
    deep: usize,
    names: Option<HashSet<&'a str>>,
    prefix: Option<usize>,
}

fn shape<'a>(walk: &mut Walk<'a>, schemas: &[&'a Value]) -> Result<Shape<'a>, ProjectionFailure> {
    let expanded = walk.expand(schemas)?;
    let requires_integer = expanded
        .iter()
        .any(|schema| schema_requires_integer(schema));
    let mut flat = 0usize;
    let mut deep = 0usize;
    for schema in &expanded {
        let (local_flat, local_deep) = walk.local_cost(schema);
        flat = flat.saturating_add(local_flat);
        deep = deep.saturating_add(local_deep);
    }
    Ok(Shape {
        expanded,
        requires_integer,
        flat,
        deep,
        names: None,
        prefix: None,
    })
}

/// Charges the validator's work for every node, then rejects what only the lexemes can see: an
/// integer keyword over a number the author wrote with a fraction. Runs before the validator so
/// that a carrier and payload whose product is unaffordable never reach it.
pub(super) fn precheck<'a>(
    walk: &mut Walk<'a>,
    schema: &'a Value,
    value: &Value,
    lexeme: &Lexeme,
    instance_nodes: usize,
) -> Result<(), ProjectionFailure> {
    let mut root = shape(walk, &[schema])?;
    precheck_node(walk, value, lexeme, &mut root, instance_nodes)
}

fn precheck_node<'a>(
    walk: &mut Walk<'a>,
    value: &Value,
    lexeme: &Lexeme,
    shape_here: &mut Shape<'a>,
    instance_nodes: usize,
) -> Result<(), ProjectionFailure> {
    let mut cost = shape_here.flat;
    if shape_here.deep > 0 {
        let below = descendants(value, instance_nodes);
        walk.charge(below)?;
        cost = cost.saturating_add(shape_here.deep.saturating_mul(below));
    }
    walk.charge(cost.max(1))?;
    if value.is_number() && shape_here.requires_integer && !lexeme.is_integer() {
        return Err(ProjectionFailure::SchemaMismatch);
    }
    match value {
        Value::Object(values) => {
            if values.is_empty() {
                return Ok(());
            }
            ensure_names(walk, &shape_here.expanded, &mut shape_here.names)?;
            let mut fallback: Option<Shape<'a>> = None;
            for (name, child) in values {
                walk.charge(1)?;
                let child_lexeme = lexeme.child(name);
                let declared = shape_here
                    .names
                    .as_ref()
                    .is_some_and(|names| names.contains(name.as_str()));
                if declared {
                    let children = precheck_object_children(walk, name, &shape_here.expanded)?;
                    let mut child_shape = shape(walk, &children)?;
                    precheck_node(walk, child, child_lexeme, &mut child_shape, instance_nodes)?;
                } else {
                    if fallback.is_none() {
                        let children = precheck_object_children(walk, name, &shape_here.expanded)?;
                        fallback = Some(shape(walk, &children)?);
                    }
                    if let Some(child_shape) = &mut fallback {
                        precheck_node(walk, child, child_lexeme, child_shape, instance_nodes)?;
                    }
                }
            }
        }
        Value::Array(values) => {
            if values.is_empty() {
                return Ok(());
            }
            let prefix = ensure_prefix(walk, &shape_here.expanded, &mut shape_here.prefix)?;
            let mut remaining: Option<Shape<'a>> = None;
            for (index, child) in values.iter().enumerate() {
                walk.charge(1)?;
                let child_lexeme = lexeme.item(index);
                if index < prefix {
                    let children = array_children(walk, index, &shape_here.expanded)?;
                    let mut child_shape = shape(walk, &children)?;
                    precheck_node(walk, child, child_lexeme, &mut child_shape, instance_nodes)?;
                } else {
                    if remaining.is_none() {
                        let children = array_children(walk, index, &shape_here.expanded)?;
                        remaining = Some(shape(walk, &children)?);
                    }
                    if let Some(child_shape) = &mut remaining {
                        precheck_node(walk, child, child_lexeme, child_shape, instance_nodes)?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Every schema that could govern `name` for an integer check: the declared property, or every
/// pattern map entry and `additionalProperties` schema of a schema that does not declare it.
fn precheck_object_children<'a>(
    walk: &mut Walk<'a>,
    name: &str,
    schemas: &[&'a Value],
) -> Result<Vec<&'a Value>, ProjectionFailure> {
    walk.charge(schemas.len())?;
    let mut children = Vec::new();
    for schema in schemas {
        if let Some(property) = schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(name))
        {
            children.push(property);
        } else {
            if let Some(patterns) = schema.get("patternProperties").and_then(Value::as_object) {
                walk.charge(patterns.len())?;
                children.extend(patterns.values());
            }
            if let Some(additional) = schema.get("additionalProperties")
                && additional.is_object()
            {
                children.push(additional);
            }
        }
    }
    Ok(children)
}

#[derive(Clone, Copy, Default)]
pub(super) struct ProjectionBudget {
    nodes: usize,
    string_bytes: usize,
    response_bytes: usize,
    pub(super) truncated: bool,
    pub(super) disclosed: DisclosedLevels,
}

impl ProjectionBudget {
    fn reserve(&mut self, bytes: usize) -> bool {
        if self.response_bytes.saturating_add(bytes) > MAX_RESPONSE_BYTES {
            self.truncated = true;
            return false;
        }
        self.response_bytes += bytes;
        true
    }

    fn admit_node(&mut self, bytes: usize) -> bool {
        if self.nodes >= MAX_PROJECTION_NODES || !self.reserve(bytes) {
            self.truncated = true;
            return false;
        }
        self.nodes += 1;
        true
    }

    fn remaining_response_bytes(&self) -> usize {
        MAX_RESPONSE_BYTES.saturating_sub(self.response_bytes)
    }

    fn disclosed(&mut self, level: Level) {
        self.disclosed = self.disclosed.union(match level {
            Level::Public => DisclosedLevels::PUBLIC,
            Level::Internal => DisclosedLevels::INTERNAL,
            Level::Pii => DisclosedLevels::PII,
            Level::Secret => DisclosedLevels::NONE,
        });
    }
}

/// The governing schemas of one instance node, their strictest level and what the audience gets.
struct Plan<'a> {
    expanded: Vec<&'a Value>,
    effective: Level,
    decision: Visibility,
    names: Option<HashSet<&'a str>>,
    prefix: Option<usize>,
}

/// A computed child context, reusable for every child with the same governing schemas.
struct ChildPlan<'a> {
    named: bool,
    plan: Plan<'a>,
}

pub(super) struct Projector<'r, 'a> {
    pub(super) walk: Walk<'a>,
    request: &'r ProjectionRequest<'r>,
    pub(super) budget: ProjectionBudget,
}

impl<'r, 'a> Projector<'r, 'a> {
    pub(super) fn new(walk: Walk<'a>, request: &'r ProjectionRequest<'r>) -> Self {
        Self {
            walk,
            request,
            budget: ProjectionBudget::default(),
        }
    }

    pub(super) fn run(
        &mut self,
        schema: &'a Value,
        value: &Value,
        lexeme: &Lexeme,
    ) -> Result<Option<ClassifiedNode>, ProjectionFailure> {
        let mut plan = self.plan(&[schema], Level::UNMARKED, 0)?;
        self.node(value, lexeme, &mut plan, 0)
    }

    fn plan(
        &mut self,
        schemas: &[&'a Value],
        inherited: Level,
        depth: usize,
    ) -> Result<Plan<'a>, ProjectionFailure> {
        let expanded = self.walk.expand(schemas)?;
        let mut effective = if depth == 0 {
            expanded
                .iter()
                .filter_map(|schema| schema.get("x-sagat-classification"))
                .filter_map(Value::as_str)
                .filter_map(|level| Level::parse(level).ok())
                .max()
                .unwrap_or(Level::UNMARKED)
        } else {
            inherited
        };
        for schema in &expanded {
            if let Some(annotation) = schema.get("x-sagat-classification") {
                let text = annotation
                    .as_str()
                    .ok_or(ProjectionFailure::SchemaUnusable)?;
                effective = effective
                    .stricter(Level::parse(text).map_err(|_| ProjectionFailure::SchemaUnusable)?);
            }
        }
        let decision = match visibility(effective, self.request.audience) {
            Visibility::Withhold(WithheldReason::LevelNotPermitted) if effective == Level::Pii => {
                Visibility::Withhold(self.request.reveal.withheld_reason())
            }
            other => other,
        };
        Ok(Plan {
            expanded,
            effective,
            decision,
            names: None,
            prefix: None,
        })
    }

    fn node(
        &mut self,
        value: &Value,
        lexeme: &Lexeme,
        plan: &mut Plan<'a>,
        depth: usize,
    ) -> Result<Option<ClassifiedNode>, ProjectionFailure> {
        self.walk.charge(1)?;
        let effective = plan.effective;
        let kind = kind_of(value);
        match plan.decision {
            Visibility::Omit => return Ok(None),
            Visibility::Withhold(reason) => {
                return Ok(self.budget.admit_node(WITHHELD_NODE_BYTES).then_some(
                    ClassifiedNode::Withheld {
                        level: effective,
                        kind,
                        reason,
                    },
                ));
            }
            Visibility::Disclose => {}
        }
        match value {
            Value::Null => Ok(self.scalar(effective, kind, ScalarValue::Null)),
            Value::Bool(value) => Ok(self.scalar(effective, kind, ScalarValue::Boolean(*value))),
            Value::Number(value) => {
                let number = lexeme
                    .number()
                    .cloned()
                    .or_else(|| JsonNumber::parse(&value.to_string()).ok())
                    .ok_or(ProjectionFailure::UnsupportedPayload)?;
                let bytes = VALUE_NODE_BYTES.saturating_add(number.as_str().len());
                if !self.budget.admit_node(bytes) {
                    return Ok(None);
                }
                self.budget.disclosed(effective);
                Ok(Some(ClassifiedNode::Value {
                    level: effective,
                    kind,
                    value: ScalarValue::Number(number),
                    value_truncated: false,
                    original_length: None,
                }))
            }
            Value::String(value) => Ok(self.string(effective, kind, value)),
            Value::Array(values) => {
                if !self.budget.admit_node(CONTAINER_NODE_BYTES) {
                    return Ok(None);
                }
                self.budget.disclosed(effective);
                let (children, omitted) = self.array(values, lexeme, plan, depth)?;
                Ok(Some(self.container(
                    effective,
                    kind,
                    values.len(),
                    children,
                    omitted,
                )))
            }
            Value::Object(values) => {
                if !self.budget.admit_node(CONTAINER_NODE_BYTES) {
                    return Ok(None);
                }
                self.budget.disclosed(effective);
                let (children, omitted) = self.object(values, lexeme, plan, depth)?;
                Ok(Some(self.container(
                    effective,
                    kind,
                    values.len(),
                    children,
                    omitted,
                )))
            }
        }
    }

    fn scalar(
        &mut self,
        level: Level,
        kind: ValueKind,
        value: ScalarValue,
    ) -> Option<ClassifiedNode> {
        if !self.budget.admit_node(VALUE_NODE_BYTES) {
            return None;
        }
        self.budget.disclosed(level);
        Some(ClassifiedNode::Value {
            level,
            kind,
            value,
            value_truncated: false,
            original_length: None,
        })
    }

    fn string(&mut self, level: Level, kind: ValueKind, value: &str) -> Option<ClassifiedNode> {
        let original = value.len();
        let remaining = MAX_DISCLOSED_STRING_BYTES.saturating_sub(self.budget.string_bytes);
        let maximum = MAX_STRING_BYTES.min(remaining);
        let response = self
            .budget
            .remaining_response_bytes()
            .saturating_sub(VALUE_NODE_BYTES);
        let end = escaped_prefix(value, maximum, response);
        let disclosed = value[..end].to_owned();
        let encoded = json_string_bytes(&disclosed);
        if !self
            .budget
            .admit_node(VALUE_NODE_BYTES.saturating_add(encoded))
        {
            return None;
        }
        self.budget.string_bytes += disclosed.len();
        let truncated = end < original;
        self.budget.truncated |= truncated;
        self.budget.disclosed(level);
        Some(ClassifiedNode::Value {
            level,
            kind,
            value: ScalarValue::String(disclosed),
            value_truncated: truncated,
            original_length: truncated.then(|| u32::try_from(original).unwrap_or(u32::MAX)),
        })
    }

    fn container(
        &self,
        level: Level,
        kind: ValueKind,
        total: usize,
        children: Vec<ClassifiedChild>,
        omitted: u32,
    ) -> ClassifiedNode {
        let structure = self.request.audience.structure;
        let count = if structure { total } else { children.len() };
        ClassifiedNode::Container {
            level,
            kind,
            child_count: u32::try_from(count).unwrap_or(u32::MAX),
            omitted_children: if structure { omitted } else { 0 },
            children,
        }
    }

    fn child_plan(
        &mut self,
        children: &[&'a Value],
        inherited: Level,
        unsupported_governance: bool,
        named: bool,
        depth: usize,
    ) -> Result<ChildPlan<'a>, ProjectionFailure> {
        let inherited = if children.is_empty() || unsupported_governance {
            inherited.stricter(Level::Pii)
        } else {
            inherited
        };
        let plan = self.plan(children, inherited, depth + 1)?;
        Ok(ChildPlan { named, plan })
    }

    fn object(
        &mut self,
        values: &Map<String, Value>,
        lexeme: &Lexeme,
        plan: &mut Plan<'a>,
        depth: usize,
    ) -> Result<(Vec<ClassifiedChild>, u32), ProjectionFailure> {
        if values.is_empty() {
            return Ok((Vec::new(), 0));
        }
        ensure_names(&mut self.walk, &plan.expanded, &mut plan.names)?;
        let mut fallback: Option<ChildPlan<'a>> = None;
        let mut children = Vec::new();
        for (ordinal, (name, value)) in values.iter().enumerate() {
            self.walk.charge(1)?;
            let named_here = plan
                .names
                .as_ref()
                .is_some_and(|names| names.contains(name.as_str()));
            let mut computed;
            let child: &mut ChildPlan<'a> = if named_here {
                let (schemas, named, unsupported) =
                    object_children(&mut self.walk, name, &plan.expanded)?;
                computed = self.child_plan(&schemas, plan.effective, unsupported, named, depth)?;
                &mut computed
            } else {
                if fallback.is_none() {
                    let (schemas, named, unsupported) =
                        object_children(&mut self.walk, name, &plan.expanded)?;
                    fallback = Some(self.child_plan(
                        &schemas,
                        plan.effective,
                        unsupported,
                        named,
                        depth,
                    )?);
                }
                match &mut fallback {
                    Some(child) => child,
                    None => return Err(ProjectionFailure::SchemaUnusable),
                }
            };
            if child.plan.decision == Visibility::Omit {
                continue;
            }
            if depth >= MAX_PROJECTION_DEPTH || children.len() >= MAX_CONTAINER_CHILDREN {
                self.budget.truncated = true;
                break;
            }
            let checkpoint = self.budget;
            let child_name_declared = child.named;
            if let Some(node) = self.node(value, lexeme.child(name), &mut child.plan, depth + 1)? {
                let disclosed = !matches!(node, ClassifiedNode::Withheld { .. });
                let child_name = (child_name_declared || disclosed).then(|| name.clone());
                let name_bytes = child_name.as_deref().map_or(0, json_string_bytes);
                if !self.budget.reserve(CHILD_BYTES.saturating_add(name_bytes)) {
                    self.budget = checkpoint;
                    self.budget.truncated = true;
                    continue;
                }
                children.push(ClassifiedChild {
                    ordinal: u32::try_from(if self.request.audience.structure {
                        ordinal
                    } else {
                        children.len()
                    })
                    .unwrap_or(u32::MAX),
                    name: child_name,
                    node: Box::new(node),
                });
            }
        }
        let omitted = values.len().saturating_sub(children.len());
        Ok((children, u32::try_from(omitted).unwrap_or(u32::MAX)))
    }

    fn array(
        &mut self,
        values: &[Value],
        lexeme: &Lexeme,
        plan: &mut Plan<'a>,
        depth: usize,
    ) -> Result<(Vec<ClassifiedChild>, u32), ProjectionFailure> {
        if values.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let prefix = ensure_prefix(&mut self.walk, &plan.expanded, &mut plan.prefix)?;
        let mut remaining: Option<ChildPlan<'a>> = None;
        let mut children = Vec::new();
        for (index, value) in values.iter().enumerate() {
            self.walk.charge(1)?;
            let mut computed;
            let child: &mut ChildPlan<'a> = if index < prefix {
                let schemas = array_children(&mut self.walk, index, &plan.expanded)?;
                computed = self.child_plan(&schemas, plan.effective, false, false, depth)?;
                &mut computed
            } else {
                if remaining.is_none() {
                    let schemas = array_children(&mut self.walk, index, &plan.expanded)?;
                    remaining =
                        Some(self.child_plan(&schemas, plan.effective, false, false, depth)?);
                }
                match &mut remaining {
                    Some(child) => child,
                    None => return Err(ProjectionFailure::SchemaUnusable),
                }
            };
            if child.plan.decision == Visibility::Omit {
                continue;
            }
            if depth >= MAX_PROJECTION_DEPTH || children.len() >= MAX_CONTAINER_CHILDREN {
                self.budget.truncated = true;
                break;
            }
            let checkpoint = self.budget;
            if let Some(node) = self.node(value, lexeme.item(index), &mut child.plan, depth + 1)? {
                if !self.budget.reserve(CHILD_BYTES) {
                    self.budget = checkpoint;
                    self.budget.truncated = true;
                    continue;
                }
                children.push(ClassifiedChild {
                    ordinal: u32::try_from(if self.request.audience.structure {
                        index
                    } else {
                        children.len()
                    })
                    .unwrap_or(u32::MAX),
                    name: None,
                    node: Box::new(node),
                });
            }
        }
        let omitted = values.len().saturating_sub(children.len());
        Ok((children, u32::try_from(omitted).unwrap_or(u32::MAX)))
    }
}

fn escaped_prefix(value: &str, maximum_raw: usize, maximum_encoded: usize) -> usize {
    if maximum_encoded < 2 {
        return 0;
    }
    let mut raw = 0;
    let mut encoded = 2;
    for character in value.chars() {
        let next_raw = raw + character.len_utf8();
        let next_encoded = encoded + escaped_char_bytes(character);
        if next_raw > maximum_raw || next_encoded > maximum_encoded {
            break;
        }
        raw = next_raw;
        encoded = next_encoded;
    }
    raw
}

fn escaped_char_bytes(character: char) -> usize {
    match character {
        '"' | '\\' | '\u{0008}' | '\u{000c}' | '\n' | '\r' | '\t' => 2,
        '\u{0000}'..='\u{001f}' => 6,
        _ => character.len_utf8(),
    }
}

fn json_string_bytes(value: &str) -> usize {
    2 + value.chars().map(escaped_char_bytes).sum::<usize>()
}

fn kind_of(value: &Value) -> ValueKind {
    match value {
        Value::Object(_) => ValueKind::Object,
        Value::Array(_) => ValueKind::Array,
        Value::String(_) => ValueKind::String,
        Value::Number(_) => ValueKind::Number,
        Value::Bool(_) => ValueKind::Boolean,
        Value::Null => ValueKind::Null,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use sagat_application::classification::ProjectionFailure;
    use serde_json::json;

    use super::{CLOCK_INTERVAL, Walk};

    #[test]
    fn running_out_of_steps_ends_the_walk_with_the_limit_recorded() {
        let schema = json!({});
        let mut walk = Walk::new(&schema, 10, None);
        assert_eq!(walk.charge(6), Ok(()));
        assert_eq!(walk.charge(5), Err(ProjectionFailure::SchemaUnusable));
        assert_eq!(
            walk.steps(),
            10,
            "the counter never reports more than the limit"
        );
    }

    #[test]
    fn an_expired_deadline_ends_the_walk_at_the_next_clock_read() {
        let schema = json!({});
        let past = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("a monotonic clock that has run for a second");
        let mut walk = Walk::new(&schema, usize::MAX, Some(past));
        assert_eq!(
            walk.charge(CLOCK_INTERVAL - 1),
            Ok(()),
            "the clock is not read on every step"
        );
        assert_eq!(walk.charge(1), Err(ProjectionFailure::SchemaUnusable));
    }

    #[test]
    fn a_future_deadline_does_not_end_the_walk() {
        let schema = json!({});
        let mut walk = Walk::new(
            &schema,
            usize::MAX,
            Some(Instant::now() + Duration::from_secs(3_600)),
        );
        assert_eq!(walk.charge(CLOCK_INTERVAL * 3), Ok(()));
    }

    #[test]
    fn the_reference_limit_is_per_location_and_cycles_stay_unusable() {
        let schema = json!({
            "$defs": {"a": {"$ref": "#/$defs/b"}, "b": {"$ref": "#/$defs/a"}, "t": {}},
        });
        let reference = json!({"$ref": "#/$defs/t"});
        let cycle = json!({"$ref": "#/$defs/a"});
        let mut walk = Walk::new(&schema, usize::MAX, None);
        for _ in 0..3 {
            assert_eq!(walk.expand(&[&reference]).map(|found| found.len()), Ok(2));
        }
        assert_eq!(
            walk.expand(&[&cycle]).map(|_| ()),
            Err(ProjectionFailure::SchemaUnusable)
        );
    }
}
