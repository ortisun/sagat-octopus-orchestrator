//! Bounded JSON Schema classification interpreter.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use sagat_application::classification::{
    CarrierPointer, ClassificationAnalysis, ClassificationCacheKey, ClassificationInterpreter,
    ClassifiedChild, ClassifiedNode, ClassifiedPayload, DisclosedLevels, JsonNumber,
    MAX_CARRIER_BYTES, MAX_CLASSIFICATION_CACHE_BYTES, MAX_CLASSIFICATION_CACHE_ENTRIES,
    MAX_CONTAINER_CHILDREN, MAX_DISCLOSED_STRING_BYTES, MAX_PAYLOAD_BYTES, MAX_PROJECTION_DEPTH,
    MAX_PROJECTION_NODES, MAX_REF_EXPANSIONS, MAX_RESPONSE_BYTES, MAX_SCHEMA_DEPTH,
    MAX_STRING_BYTES, ProjectionFailure, ProjectionRequest, ScalarValue, ValueKind,
};
use sagat_domain::classification::{Level, Visibility, WithheldReason, visibility};
use serde_json::Value;

const PATTERN_SIZE_LIMIT: usize = 1024 * 1024;
const VALUE_NODE_BYTES: usize = 128;
const CONTAINER_NODE_BYTES: usize = 128;
const WITHHELD_NODE_BYTES: usize = 128;
const CHILD_BYTES: usize = 48;

struct CompiledCarrier {
    schema: Value,
    validator: jsonschema::Validator,
    bytes: usize,
    has_numeric_constraints: bool,
    numeric_constraints_exact: bool,
    has_integer_types: bool,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<ClassificationCacheKey, Arc<CompiledCarrier>>,
    order: VecDeque<ClassificationCacheKey>,
    bytes: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

/// Process-local interpreter and bounded compiled-validator cache.
pub struct Classifier {
    cache: Mutex<Cache>,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl Default for Classifier {
    fn default() -> Self {
        Self {
            cache: Mutex::new(Cache::default()),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }
}

impl std::fmt::Debug for Classifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Classifier")
            .field("cache", &self.cache_stats())
            .finish()
    }
}

impl Classifier {
    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
        }
    }

    fn compiled(
        &self,
        key: &ClassificationCacheKey,
        document: &[u8],
    ) -> Result<Arc<CompiledCarrier>, ProjectionFailure> {
        {
            let mut cache = self
                .cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(entry) = cache.entries.get(key).cloned() {
                cache.order.retain(|candidate| candidate != key);
                cache.order.push_back(key.clone());
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Ok(entry);
            }
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        let document: Value =
            serde_json::from_slice(document).map_err(|_| ProjectionFailure::SchemaUnusable)?;
        let schema = carrier(&document, &key.carrier)
            .ok_or(ProjectionFailure::NoCarrier)?
            .clone();
        let bytes = serde_json::to_vec(&schema)
            .map_err(|_| ProjectionFailure::SchemaUnusable)?
            .len();
        if bytes > MAX_CARRIER_BYTES || schema_depth(&schema) > MAX_SCHEMA_DEPTH {
            return Err(ProjectionFailure::SchemaUnusable);
        }
        // A dry expansion catches cycles, outside references and the fixed expansion bound before
        // the validator sees the schema.
        let mut expansion = Expansion::new(&schema);
        expansion.expand_many(std::slice::from_ref(&&schema))?;
        let validator = jsonschema::options()
            .with_pattern_options(
                jsonschema::PatternOptions::regex()
                    .size_limit(PATTERN_SIZE_LIMIT)
                    .dfa_size_limit(PATTERN_SIZE_LIMIT),
            )
            .build(&schema)
            .map_err(|_| ProjectionFailure::SchemaUnusable)?;
        let has_numeric_constraints = has_numeric_constraints(&schema);
        let numeric_constraints_exact = numeric_constraints_exact(&schema);
        let has_integer_types = schema_contains_integer_type(&schema);
        let compiled = Arc::new(CompiledCarrier {
            schema,
            validator,
            bytes,
            has_numeric_constraints,
            numeric_constraints_exact,
            has_integer_types,
        });

        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while cache.entries.len() >= MAX_CLASSIFICATION_CACHE_ENTRIES
            || cache.bytes.saturating_add(bytes) > MAX_CLASSIFICATION_CACHE_BYTES
        {
            let Some(oldest) = cache.order.pop_front() else {
                break;
            };
            if let Some(removed) = cache.entries.remove(&oldest) {
                cache.bytes = cache.bytes.saturating_sub(removed.bytes);
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
        if bytes <= MAX_CLASSIFICATION_CACHE_BYTES {
            cache.bytes = cache.bytes.saturating_add(bytes);
            cache.order.push_back(key.clone());
            cache.entries.insert(key.clone(), Arc::clone(&compiled));
        }
        Ok(compiled)
    }
}

impl ClassificationInterpreter for Classifier {
    fn analyze(&self, document: &[u8]) -> ClassificationAnalysis {
        crate::classification::publication::analyze(document)
    }

    fn project(
        &self,
        request: ProjectionRequest<'_>,
    ) -> Result<ClassifiedPayload, ProjectionFailure> {
        if request.rules == sagat_domain::classification::ClassificationRules::NONE {
            return Err(ProjectionFailure::LegacyVersion);
        }
        if request.payload.len() > MAX_PAYLOAD_BYTES {
            return Err(ProjectionFailure::PayloadTooLarge);
        }
        let compiled = self.compiled(&request.cache_key, request.document)?;
        let payload: Value = serde_json::from_slice(request.payload)
            .map_err(|_| ProjectionFailure::UnsupportedPayload)?;
        let numbers = NumberLexemes::parse(request.payload)?;
        if numbers.has_numbers()
            && (!compiled.numeric_constraints_exact
                || (compiled.has_numeric_constraints && numbers.has_unsafe_comparison()))
        {
            return Err(ProjectionFailure::UnsupportedPayload);
        }
        let mut numeric_expansion = Expansion::new(&compiled.schema);
        validate_integer_types(
            &payload,
            &[&compiled.schema],
            "",
            &numbers,
            &mut numeric_expansion,
        )?;
        // The validator stores numbers as finite machine values. An integer constraint hidden
        // behind an applicator can therefore see a rounded integer for a fractional source
        // lexeme. The path-aware check above handles constraints it can prove directly; this
        // carrier-wide guard closes every remaining applicator conservatively.
        if compiled.has_integer_types && numbers.has_fractional() {
            return Err(ProjectionFailure::UnsupportedPayload);
        }
        if !compiled.validator.is_valid(&payload) {
            return Err(ProjectionFailure::SchemaMismatch);
        }
        let mut budget = ProjectionBudget::default();
        let mut expansion = Expansion::new(&compiled.schema);
        let tree = project_node(
            &payload,
            &[&compiled.schema],
            Level::UNMARKED,
            0,
            "",
            &numbers,
            &request,
            &mut expansion,
            &mut budget,
        )?
        .ok_or(ProjectionFailure::NotDisclosable)?;
        Ok(ClassifiedPayload {
            tree,
            truncated: budget.truncated,
            disclosed: budget.disclosed,
        })
    }
}

fn carrier<'a>(document: &'a Value, pointer: &CarrierPointer) -> Option<&'a Value> {
    match pointer {
        CarrierPointer::Input => document.get("inputSchema"),
        CarrierPointer::Result => document.get("outputSchema"),
        CarrierPointer::NodeOutput(id) => document
            .get("nodes")?
            .as_array()?
            .iter()
            .find(|node| node.get("id").and_then(Value::as_str) == Some(id.as_str()))
            .and_then(|node| node.get("outputSchema")),
        CarrierPointer::BodyNodeOutput { body, node } => document
            .get("bodies")?
            .as_object()?
            .get(body.as_str())?
            .get("nodes")?
            .as_array()?
            .iter()
            .find(|candidate| candidate.get("id").and_then(Value::as_str) == Some(node.as_str()))
            .and_then(|candidate| candidate.get("outputSchema")),
    }
}

fn schema_depth(root: &Value) -> usize {
    let mut maximum = 0;
    let mut pending = vec![(root, 1usize)];
    while let Some((value, depth)) = pending.pop() {
        maximum = maximum.max(depth);
        match value {
            Value::Array(values) => pending.extend(values.iter().map(|value| (value, depth + 1))),
            Value::Object(values) => {
                pending.extend(values.values().map(|value| (value, depth + 1)));
            }
            _ => {}
        }
    }
    maximum
}

struct Expansion<'a> {
    root: &'a Value,
    references: usize,
    active: HashSet<String>,
}

impl<'a> Expansion<'a> {
    fn new(root: &'a Value) -> Self {
        Self {
            root,
            references: 0,
            active: HashSet::new(),
        }
    }

    fn expand_many(&mut self, schemas: &[&'a Value]) -> Result<Vec<&'a Value>, ProjectionFailure> {
        let mut expanded = Vec::new();
        for schema in schemas {
            self.expand(schema, &mut expanded)?;
        }
        Ok(expanded)
    }

    fn expand(
        &mut self,
        schema: &'a Value,
        expanded: &mut Vec<&'a Value>,
    ) -> Result<(), ProjectionFailure> {
        expanded.push(schema);
        let Some(object) = schema.as_object() else {
            return Ok(());
        };
        if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
            if !reference.starts_with("#/") {
                return Err(ProjectionFailure::SchemaUnusable);
            }
            self.references += 1;
            if self.references > MAX_REF_EXPANSIONS || !self.active.insert(reference.to_owned()) {
                return Err(ProjectionFailure::SchemaUnusable);
            }
            let target = self
                .root
                .pointer(&reference[1..])
                .ok_or(ProjectionFailure::SchemaUnusable)?;
            self.expand(target, expanded)?;
            self.active.remove(reference);
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(branches) = object.get(keyword).and_then(Value::as_array) {
                for branch in branches {
                    self.expand(branch, expanded)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct ProjectionBudget {
    nodes: usize,
    string_bytes: usize,
    response_bytes: usize,
    truncated: bool,
    disclosed: DisclosedLevels,
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

fn classify_node<'a>(
    schemas: &[&'a Value],
    inherited: Level,
    depth: usize,
    request: &ProjectionRequest<'_>,
    expansion: &mut Expansion<'a>,
) -> Result<(Vec<&'a Value>, Level, Visibility), ProjectionFailure> {
    let expanded = expansion.expand_many(schemas)?;
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
    let decision = match visibility(effective, request.audience) {
        Visibility::Withhold(WithheldReason::LevelNotPermitted) if effective == Level::Pii => {
            Visibility::Withhold(request.reveal.withheld_reason())
        }
        other => other,
    };
    Ok((expanded, effective, decision))
}

#[allow(clippy::too_many_arguments)]
fn project_node<'a>(
    value: &Value,
    schemas: &[&'a Value],
    inherited: Level,
    depth: usize,
    path: &str,
    numbers: &NumberLexemes,
    request: &ProjectionRequest<'_>,
    expansion: &mut Expansion<'a>,
    budget: &mut ProjectionBudget,
) -> Result<Option<ClassifiedNode>, ProjectionFailure> {
    let (expanded, effective, decision) =
        classify_node(schemas, inherited, depth, request, expansion)?;
    let kind = kind_of(value);
    match decision {
        Visibility::Omit => return Ok(None),
        Visibility::Withhold(reason) => {
            return Ok(budget.admit_node(WITHHELD_NODE_BYTES).then_some(
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
        Value::Null => {
            if !budget.admit_node(VALUE_NODE_BYTES) {
                return Ok(None);
            }
            budget.disclosed(effective);
            Ok(Some(ClassifiedNode::Value {
                level: effective,
                kind,
                value: ScalarValue::Null,
                value_truncated: false,
                original_length: None,
            }))
        }
        Value::Bool(value) => {
            if !budget.admit_node(VALUE_NODE_BYTES) {
                return Ok(None);
            }
            budget.disclosed(effective);
            Ok(Some(ClassifiedNode::Value {
                level: effective,
                kind,
                value: ScalarValue::Boolean(*value),
                value_truncated: false,
                original_length: None,
            }))
        }
        Value::Number(value) => {
            let number = numbers
                .get(path)
                .cloned()
                .or_else(|| JsonNumber::parse(&value.to_string()).ok())
                .ok_or(ProjectionFailure::UnsupportedPayload)?;
            if !budget.admit_node(VALUE_NODE_BYTES.saturating_add(number.as_str().len())) {
                return Ok(None);
            }
            budget.disclosed(effective);
            Ok(Some(ClassifiedNode::Value {
                level: effective,
                kind,
                value: ScalarValue::Number(number),
                value_truncated: false,
                original_length: None,
            }))
        }
        Value::String(value) => {
            let original = value.len();
            let remaining = MAX_DISCLOSED_STRING_BYTES.saturating_sub(budget.string_bytes);
            let maximum = MAX_STRING_BYTES.min(remaining);
            let response = budget
                .remaining_response_bytes()
                .saturating_sub(VALUE_NODE_BYTES);
            let end = escaped_prefix(value, maximum, response);
            let disclosed = value[..end].to_owned();
            let encoded = json_string_bytes(&disclosed);
            if !budget.admit_node(VALUE_NODE_BYTES.saturating_add(encoded)) {
                return Ok(None);
            }
            budget.string_bytes += disclosed.len();
            let truncated = end < original;
            budget.truncated |= truncated;
            budget.disclosed(effective);
            Ok(Some(ClassifiedNode::Value {
                level: effective,
                kind,
                value: ScalarValue::String(disclosed),
                value_truncated: truncated,
                original_length: truncated.then(|| u32::try_from(original).unwrap_or(u32::MAX)),
            }))
        }
        Value::Array(values) => {
            if !budget.admit_node(CONTAINER_NODE_BYTES) {
                return Ok(None);
            }
            budget.disclosed(effective);
            let (children, omitted) = project_array(
                values, &expanded, effective, depth, path, numbers, request, expansion, budget,
            )?;
            let count = if request.audience.structure {
                values.len()
            } else {
                children.len()
            };
            Ok(Some(ClassifiedNode::Container {
                level: effective,
                kind,
                child_count: u32::try_from(count).unwrap_or(u32::MAX),
                omitted_children: if request.audience.structure {
                    omitted
                } else {
                    0
                },
                children,
            }))
        }
        Value::Object(values) => {
            if !budget.admit_node(CONTAINER_NODE_BYTES) {
                return Ok(None);
            }
            budget.disclosed(effective);
            let (children, omitted) = project_object(
                values, &expanded, effective, depth, path, numbers, request, expansion, budget,
            )?;
            let count = if request.audience.structure {
                values.len()
            } else {
                children.len()
            };
            Ok(Some(ClassifiedNode::Container {
                level: effective,
                kind,
                child_count: u32::try_from(count).unwrap_or(u32::MAX),
                omitted_children: if request.audience.structure {
                    omitted
                } else {
                    0
                },
                children,
            }))
        }
    }
}

fn object_child_context<'a>(
    name: &str,
    schemas: &[&'a Value],
    inherited: Level,
) -> (Vec<&'a Value>, Level, bool) {
    let mut child_schemas = Vec::new();
    let mut named = false;
    let mut unsupported_governance = false;
    for schema in schemas {
        if let Some(property) = schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(name))
        {
            named = true;
            child_schemas.push(property);
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
                child_schemas.push(additional);
            }
        }
    }
    let child_inherited = if child_schemas.is_empty() || unsupported_governance {
        inherited.stricter(Level::Pii)
    } else {
        inherited
    };
    (child_schemas, child_inherited, named)
}

fn array_child_context<'a>(
    index: usize,
    schemas: &[&'a Value],
    inherited: Level,
) -> (Vec<&'a Value>, Level) {
    let mut child_schemas = Vec::new();
    for schema in schemas {
        if let Some(prefix) = schema.get("prefixItems").and_then(Value::as_array)
            && let Some(item) = prefix.get(index)
        {
            child_schemas.push(item);
            continue;
        }
        if let Some(items) = schema.get("items")
            && items.is_object()
        {
            child_schemas.push(items);
        }
    }
    let child_inherited = if child_schemas.is_empty() {
        inherited.stricter(Level::Pii)
    } else {
        inherited
    };
    (child_schemas, child_inherited)
}

fn is_visible_child<'a>(
    schemas: &[&'a Value],
    inherited: Level,
    depth: usize,
    request: &ProjectionRequest<'_>,
    root: &'a Value,
) -> Result<bool, ProjectionFailure> {
    let mut expansion = Expansion::new(root);
    let (_, _, decision) = classify_node(schemas, inherited, depth, request, &mut expansion)?;
    Ok(decision != Visibility::Omit)
}

#[allow(clippy::too_many_arguments)]
fn project_object<'a>(
    values: &serde_json::Map<String, Value>,
    schemas: &[&'a Value],
    inherited: Level,
    depth: usize,
    path: &str,
    numbers: &NumberLexemes,
    request: &ProjectionRequest<'_>,
    expansion: &mut Expansion<'a>,
    budget: &mut ProjectionBudget,
) -> Result<(Vec<ClassifiedChild>, u32), ProjectionFailure> {
    let mut children = Vec::new();
    for (ordinal, (name, value)) in values.iter().enumerate() {
        let (child_schemas, child_inherited, named) =
            object_child_context(name, schemas, inherited);
        if !is_visible_child(
            &child_schemas,
            child_inherited,
            depth + 1,
            request,
            expansion.root,
        )? {
            continue;
        }
        if depth >= MAX_PROJECTION_DEPTH || children.len() >= MAX_CONTAINER_CHILDREN {
            budget.truncated = true;
            break;
        }
        let checkpoint = *budget;
        if let Some(node) = project_node(
            value,
            &child_schemas,
            child_inherited,
            depth + 1,
            &format!("{path}/{}", pointer_escape(name)),
            numbers,
            request,
            expansion,
            budget,
        )? {
            let disclosed = !matches!(node, ClassifiedNode::Withheld { .. });
            let child_name = (named || disclosed).then(|| name.clone());
            let name_bytes = child_name.as_deref().map_or(0, json_string_bytes);
            if !budget.reserve(CHILD_BYTES.saturating_add(name_bytes)) {
                *budget = checkpoint;
                budget.truncated = true;
                continue;
            }
            children.push(ClassifiedChild {
                ordinal: u32::try_from(if request.audience.structure {
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

#[allow(clippy::too_many_arguments)]
fn project_array<'a>(
    values: &[Value],
    schemas: &[&'a Value],
    inherited: Level,
    depth: usize,
    path: &str,
    numbers: &NumberLexemes,
    request: &ProjectionRequest<'_>,
    expansion: &mut Expansion<'a>,
    budget: &mut ProjectionBudget,
) -> Result<(Vec<ClassifiedChild>, u32), ProjectionFailure> {
    let mut children = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let (child_schemas, child_inherited) = array_child_context(index, schemas, inherited);
        if !is_visible_child(
            &child_schemas,
            child_inherited,
            depth + 1,
            request,
            expansion.root,
        )? {
            continue;
        }
        if depth >= MAX_PROJECTION_DEPTH || children.len() >= MAX_CONTAINER_CHILDREN {
            budget.truncated = true;
            break;
        }
        let checkpoint = *budget;
        if let Some(node) = project_node(
            value,
            &child_schemas,
            child_inherited,
            depth + 1,
            &format!("{path}/{index}"),
            numbers,
            request,
            expansion,
            budget,
        )? {
            if !budget.reserve(CHILD_BYTES) {
                *budget = checkpoint;
                budget.truncated = true;
                continue;
            }
            children.push(ClassifiedChild {
                ordinal: u32::try_from(if request.audience.structure {
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

struct NumberLexemes(HashMap<String, JsonNumber>);

impl NumberLexemes {
    fn parse(payload: &[u8]) -> Result<Self, ProjectionFailure> {
        let mut scanner = NumberScanner {
            payload,
            at: 0,
            numbers: HashMap::new(),
        };
        scanner.value("")?;
        scanner.whitespace();
        if scanner.at != payload.len() {
            return Err(ProjectionFailure::UnsupportedPayload);
        }
        Ok(Self(scanner.numbers))
    }

    fn get(&self, path: &str) -> Option<&JsonNumber> {
        self.0.get(path)
    }

    fn has_numbers(&self) -> bool {
        !self.0.is_empty()
    }

    fn has_unsafe_comparison(&self) -> bool {
        self.0
            .values()
            .any(|number| !safe_integer_comparison(number.as_str()))
    }

    fn has_fractional(&self) -> bool {
        self.0
            .values()
            .any(|number| !lexical_is_integer(number.as_str()))
    }

    fn is_integer(&self, path: &str) -> bool {
        self.get(path)
            .is_some_and(|number| lexical_is_integer(number.as_str()))
    }
}

struct NumberScanner<'a> {
    payload: &'a [u8],
    at: usize,
    numbers: HashMap<String, JsonNumber>,
}

impl NumberScanner<'_> {
    fn value(&mut self, path: &str) -> Result<(), ProjectionFailure> {
        self.whitespace();
        match self.payload.get(self.at) {
            Some(b'{') => self.object(path),
            Some(b'[') => self.array(path),
            Some(b'"') => self.string().map(|_| ()),
            Some(b't') => self.literal(b"true"),
            Some(b'f') => self.literal(b"false"),
            Some(b'n') => self.literal(b"null"),
            Some(b'-' | b'0'..=b'9') => self.number(path),
            _ => Err(ProjectionFailure::UnsupportedPayload),
        }
    }

    fn object(&mut self, path: &str) -> Result<(), ProjectionFailure> {
        self.at += 1;
        self.whitespace();
        if self.take(b'}') {
            return Ok(());
        }
        loop {
            self.whitespace();
            let name = self.string()?;
            self.whitespace();
            if !self.take(b':') {
                return Err(ProjectionFailure::UnsupportedPayload);
            }
            self.value(&format!("{path}/{}", pointer_escape(&name)))?;
            self.whitespace();
            if self.take(b'}') {
                return Ok(());
            }
            if !self.take(b',') {
                return Err(ProjectionFailure::UnsupportedPayload);
            }
        }
    }

    fn array(&mut self, path: &str) -> Result<(), ProjectionFailure> {
        self.at += 1;
        self.whitespace();
        if self.take(b']') {
            return Ok(());
        }
        let mut index = 0usize;
        loop {
            self.value(&format!("{path}/{index}"))?;
            index += 1;
            self.whitespace();
            if self.take(b']') {
                return Ok(());
            }
            if !self.take(b',') {
                return Err(ProjectionFailure::UnsupportedPayload);
            }
        }
    }

    fn string(&mut self) -> Result<String, ProjectionFailure> {
        let start = self.at;
        if !self.take(b'"') {
            return Err(ProjectionFailure::UnsupportedPayload);
        }
        loop {
            match self.payload.get(self.at).copied() {
                Some(b'"') => {
                    self.at += 1;
                    return serde_json::from_slice(&self.payload[start..self.at])
                        .map_err(|_| ProjectionFailure::UnsupportedPayload);
                }
                Some(b'\\') => {
                    self.at += 2;
                }
                Some(_) => self.at += 1,
                None => return Err(ProjectionFailure::UnsupportedPayload),
            }
        }
    }

    fn number(&mut self, path: &str) -> Result<(), ProjectionFailure> {
        let start = self.at;
        while matches!(
            self.payload.get(self.at),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.payload[start..self.at])
            .map_err(|_| ProjectionFailure::UnsupportedPayload)?;
        let number = JsonNumber::parse(text).map_err(|_| ProjectionFailure::UnsupportedPayload)?;
        self.numbers.insert(path.to_owned(), number);
        Ok(())
    }

    fn literal(&mut self, literal: &[u8]) -> Result<(), ProjectionFailure> {
        if self.payload.get(self.at..self.at + literal.len()) == Some(literal) {
            self.at += literal.len();
            Ok(())
        } else {
            Err(ProjectionFailure::UnsupportedPayload)
        }
    }

    fn whitespace(&mut self) {
        while matches!(
            self.payload.get(self.at),
            Some(b' ' | b'\n' | b'\r' | b'\t')
        ) {
            self.at += 1;
        }
    }

    fn take(&mut self, expected: u8) -> bool {
        if self.payload.get(self.at) == Some(&expected) {
            self.at += 1;
            true
        } else {
            false
        }
    }
}

fn pointer_escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn validate_integer_types<'a>(
    value: &Value,
    schemas: &[&'a Value],
    path: &str,
    numbers: &NumberLexemes,
    expansion: &mut Expansion<'a>,
) -> Result<(), ProjectionFailure> {
    let expanded = expansion.expand_many(schemas)?;
    if value.is_number()
        && expanded
            .iter()
            .any(|schema| schema_requires_integer(schema))
        && !numbers.is_integer(path)
    {
        return Err(ProjectionFailure::SchemaMismatch);
    }
    match value {
        Value::Object(values) => {
            for (name, value) in values {
                let mut children = Vec::new();
                for schema in &expanded {
                    if let Some(property) = schema
                        .get("properties")
                        .and_then(Value::as_object)
                        .and_then(|properties| properties.get(name))
                    {
                        children.push(property);
                    } else {
                        if let Some(patterns) =
                            schema.get("patternProperties").and_then(Value::as_object)
                        {
                            children.extend(patterns.values());
                        }
                        if let Some(additional) = schema.get("additionalProperties")
                            && additional.is_object()
                        {
                            children.push(additional);
                        }
                    }
                }
                validate_integer_types(
                    value,
                    &children,
                    &format!("{path}/{}", pointer_escape(name)),
                    numbers,
                    expansion,
                )?;
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                let mut children = Vec::new();
                for schema in &expanded {
                    if let Some(prefix) = schema.get("prefixItems").and_then(Value::as_array)
                        && let Some(item) = prefix.get(index)
                    {
                        children.push(item);
                    } else if let Some(items) = schema.get("items")
                        && items.is_object()
                    {
                        children.push(items);
                    }
                }
                validate_integer_types(
                    value,
                    &children,
                    &format!("{path}/{index}"),
                    numbers,
                    expansion,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn schema_requires_integer(schema: &Value) -> bool {
    match schema.get("type") {
        Some(Value::String(kind)) => kind == "integer",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind.as_str() == Some("integer")),
        _ => false,
    }
}

fn schema_contains_integer_type(schema: &Value) -> bool {
    schema_requires_integer(schema)
        || match schema {
            Value::Object(values) => values.values().any(schema_contains_integer_type),
            Value::Array(values) => values.iter().any(schema_contains_integer_type),
            _ => false,
        }
}

fn has_numeric_constraints(schema: &Value) -> bool {
    match schema {
        Value::Object(object) => object.iter().any(|(keyword, value)| {
            matches!(
                keyword.as_str(),
                "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" | "multipleOf"
            ) || (matches!(keyword.as_str(), "const" | "enum") && contains_number(value))
                || has_numeric_constraints(value)
        }),
        Value::Array(values) => values.iter().any(has_numeric_constraints),
        _ => false,
    }
}

fn numeric_constraints_exact(schema: &Value) -> bool {
    match schema {
        Value::Object(object) => object.iter().all(|(keyword, value)| {
            let local = match keyword.as_str() {
                "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" | "multipleOf" => {
                    exact_stored_numbers(value)
                }
                "const" | "enum" => exact_stored_numbers(value),
                _ => true,
            };
            local && numeric_constraints_exact(value)
        }),
        Value::Array(values) => values.iter().all(numeric_constraints_exact),
        _ => true,
    }
}

fn contains_number(value: &Value) -> bool {
    match value {
        Value::Number(_) => true,
        Value::Array(values) => values.iter().any(contains_number),
        Value::Object(values) => values.values().any(contains_number),
        _ => false,
    }
}

fn exact_stored_numbers(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.is_i64() || number.is_u64(),
        Value::Array(values) => values.iter().all(exact_stored_numbers),
        Value::Object(values) => values.values().all(exact_stored_numbers),
        _ => true,
    }
}

fn safe_integer_comparison(value: &str) -> bool {
    !value.contains(['.', 'e', 'E'])
        && (value.parse::<i64>().is_ok() || value.parse::<u64>().is_ok())
}

fn lexical_is_integer(value: &str) -> bool {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let exponent_at = unsigned.find(['e', 'E']);
    let (mantissa, exponent) = exponent_at.map_or((unsigned, 0_i64), |at| {
        let exponent = unsigned[at + 1..].parse::<i64>().unwrap_or_else(|_| {
            if unsigned[at + 1..].starts_with('-') {
                i64::MIN
            } else {
                i64::MAX
            }
        });
        (&unsigned[..at], exponent)
    });
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    if digits.bytes().all(|digit| digit == b'0') {
        return true;
    }
    let fractional_places = i128::try_from(fraction.len()).unwrap_or(i128::MAX);
    let scale = fractional_places - i128::from(exponent);
    if scale <= 0 {
        return true;
    }
    usize::try_from(scale)
        .ok()
        .filter(|scale| *scale <= digits.len())
        .is_some_and(|scale| {
            digits.as_bytes()[digits.len() - scale..]
                .iter()
                .all(|digit| *digit == b'0')
        })
}

#[path = "classification/publication.rs"]
mod publication;

pub use publication::{
    analyze_value, resolve_classification_location, visit_classification_locations,
};
