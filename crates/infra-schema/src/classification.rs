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

struct CompiledCarrier {
    schema: Value,
    validator: jsonschema::Validator,
    bytes: usize,
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
        let compiled = Arc::new(CompiledCarrier {
            schema,
            validator,
            bytes,
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

#[derive(Default)]
struct ProjectionBudget {
    nodes: usize,
    string_bytes: usize,
    response_bytes: usize,
    truncated: bool,
    disclosed: DisclosedLevels,
}

impl ProjectionBudget {
    fn admit(&mut self, bytes: usize) -> bool {
        if self.nodes >= MAX_PROJECTION_NODES
            || self.response_bytes.saturating_add(bytes) > MAX_RESPONSE_BYTES
        {
            self.truncated = true;
            return false;
        }
        self.nodes += 1;
        self.response_bytes += bytes;
        true
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
    let kind = kind_of(value);
    let decision = match visibility(effective, request.audience) {
        Visibility::Withhold(WithheldReason::LevelNotPermitted) if effective == Level::Pii => {
            Visibility::Withhold(request.reveal.withheld_reason())
        }
        other => other,
    };
    match decision {
        Visibility::Omit => return Ok(None),
        Visibility::Withhold(reason) => {
            return Ok(budget.admit(48).then_some(ClassifiedNode::Withheld {
                level: effective,
                kind,
                reason,
            }));
        }
        Visibility::Disclose => budget.disclosed(effective),
    }
    if !budget.admit(64) {
        return Ok(None);
    }
    match value {
        Value::Null => Ok(Some(ClassifiedNode::Value {
            level: effective,
            kind,
            value: ScalarValue::Null,
            value_truncated: false,
            original_length: None,
        })),
        Value::Bool(value) => Ok(Some(ClassifiedNode::Value {
            level: effective,
            kind,
            value: ScalarValue::Boolean(*value),
            value_truncated: false,
            original_length: None,
        })),
        Value::Number(value) => Ok(Some(ClassifiedNode::Value {
            level: effective,
            kind,
            value: ScalarValue::Number(
                numbers
                    .get(path)
                    .cloned()
                    .or_else(|| JsonNumber::parse(&value.to_string()).ok())
                    .ok_or(ProjectionFailure::UnsupportedPayload)?,
            ),
            value_truncated: false,
            original_length: None,
        })),
        Value::String(value) => {
            let original = value.len();
            let remaining = MAX_DISCLOSED_STRING_BYTES.saturating_sub(budget.string_bytes);
            let maximum = MAX_STRING_BYTES.min(remaining);
            let end = utf8_boundary(value, maximum);
            let disclosed = value[..end].to_owned();
            budget.string_bytes += disclosed.len();
            budget.response_bytes = budget.response_bytes.saturating_add(disclosed.len());
            let truncated = end < original;
            budget.truncated |= truncated;
            Ok(Some(ClassifiedNode::Value {
                level: effective,
                kind,
                value: ScalarValue::String(disclosed),
                value_truncated: truncated,
                original_length: truncated.then(|| u32::try_from(original).unwrap_or(u32::MAX)),
            }))
        }
        Value::Array(values) => {
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
                omitted_children: omitted,
                children,
            }))
        }
        Value::Object(values) => {
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
                omitted_children: omitted,
                children,
            }))
        }
    }
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
    if depth >= MAX_PROJECTION_DEPTH {
        budget.truncated |= !values.is_empty();
        return Ok((Vec::new(), u32::try_from(values.len()).unwrap_or(u32::MAX)));
    }
    let take = values.len().min(MAX_CONTAINER_CHILDREN);
    budget.truncated |= take < values.len();
    let mut children = Vec::new();
    for (ordinal, (name, value)) in values.iter().take(take).enumerate() {
        let mut child_schemas = Vec::new();
        let mut named = false;
        for schema in schemas {
            if let Some(property) = schema
                .get("properties")
                .and_then(Value::as_object)
                .and_then(|properties| properties.get(name))
            {
                named = true;
                child_schemas.push(property);
            } else if let Some(additional) = schema.get("additionalProperties")
                && additional.is_object()
            {
                child_schemas.push(additional);
            }
        }
        let child_inherited = if child_schemas.is_empty() {
            inherited.stricter(Level::Pii)
        } else {
            inherited
        };
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
            children.push(ClassifiedChild {
                ordinal: u32::try_from(ordinal).unwrap_or(u32::MAX),
                name: (named || disclosed).then(|| name.clone()),
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
    if depth >= MAX_PROJECTION_DEPTH {
        budget.truncated |= !values.is_empty();
        return Ok((Vec::new(), u32::try_from(values.len()).unwrap_or(u32::MAX)));
    }
    let take = values.len().min(MAX_CONTAINER_CHILDREN);
    budget.truncated |= take < values.len();
    let mut children = Vec::new();
    for (index, value) in values.iter().take(take).enumerate() {
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
            children.push(ClassifiedChild {
                ordinal: u32::try_from(index).unwrap_or(u32::MAX),
                name: None,
                node: Box::new(node),
            });
        }
    }
    let omitted = values.len().saturating_sub(children.len());
    Ok((children, u32::try_from(omitted).unwrap_or(u32::MAX)))
}

fn utf8_boundary(value: &str, maximum: usize) -> usize {
    let mut end = value.len().min(maximum);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    end
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

#[path = "classification/publication.rs"]
mod publication;

pub use publication::analyze_value;
