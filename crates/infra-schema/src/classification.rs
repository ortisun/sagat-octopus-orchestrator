//! Bounded JSON Schema classification interpreter.

use std::sync::Arc;
use std::time::Instant;

use sagat_application::classification::{
    CarrierPointer, ClassificationAnalysis, ClassificationCacheKey, ClassificationInterpreter,
    ClassifiedPayload, MAX_CARRIER_BYTES, MAX_CARRIER_PATTERNS, MAX_PAYLOAD_BYTES,
    MAX_PROJECTION_DURATION, MAX_SCHEMA_DEPTH, MAX_WALK_STEPS, ProjectionFailure,
    ProjectionRequest,
};
use sagat_domain::classification::ClassificationRules;
use serde_json::Value;

use cache::CarrierCache;
use numbers::{Lexemes, NumericProfile};
use walk::{Projector, Walk};

const PATTERN_SIZE_LIMIT: usize = 64 * 1024;

struct CompiledCarrier {
    schema: Value,
    validator: jsonschema::Validator,
    /// Estimated bytes the compiled validator keeps: what the cache budget is spent on.
    weight: usize,
    numeric: NumericProfile,
}

/// Bytes one compiled validator keeps beyond its own size, whatever the carrier holds.
const COMPILED_BASE_BYTES: usize = 4 * 1024;
/// Compiled bytes per serialized schema byte, rounded up from the largest ratio measured across a
/// wide object, a 3,000-branch `allOf`, a large `enum` and a minimal schema (21 to 54).
const COMPILED_BYTES_PER_SCHEMA_BYTE: usize = 64;
/// What one regular expression keeps: its program and one lazy-DFA cache, each at its limit.
const PATTERN_WEIGHT: usize = 2 * PATTERN_SIZE_LIMIT;

fn compiled_weight(schema_bytes: usize, patterns: usize) -> usize {
    COMPILED_BASE_BYTES
        .saturating_add(schema_bytes.saturating_mul(COMPILED_BYTES_PER_SCHEMA_BYTE))
        .saturating_add(patterns.saturating_mul(PATTERN_WEIGHT))
}

/// `pattern` keywords and `patternProperties` keys: every one compiles to an automaton.
fn count_patterns(schema: &Value) -> usize {
    match schema {
        Value::Object(object) => object
            .iter()
            .map(|(keyword, value)| {
                let own = match (keyword.as_str(), value) {
                    ("pattern", Value::String(_)) => 1,
                    ("patternProperties", Value::Object(patterns)) => patterns.len(),
                    _ => 0,
                };
                own + count_patterns(value)
            })
            .sum(),
        Value::Array(values) => values.iter().map(count_patterns).sum(),
        _ => 0,
    }
}

/// Process-local interpreter and bounded compiled-validator cache.
pub struct Classifier {
    cache: CarrierCache,
}

impl Default for Classifier {
    fn default() -> Self {
        Self {
            cache: CarrierCache::default(),
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
        self.cache.stats()
    }

    fn compiled(
        &self,
        key: &ClassificationCacheKey,
        document: &[u8],
    ) -> Result<Arc<CompiledCarrier>, ProjectionFailure> {
        self.cache
            .get_or_compile(key, || {}, || compile_carrier(key, document))
    }
}

fn compile_carrier(
    key: &ClassificationCacheKey,
    document: &[u8],
) -> Result<Arc<CompiledCarrier>, ProjectionFailure> {
    let document: Value =
        serde_json::from_slice(document).map_err(|_| ProjectionFailure::SchemaUnusable)?;
    let schema = carrier(&document, &key.carrier)
        .ok_or(ProjectionFailure::NoCarrier)?
        .clone();
    let schema_bytes = serde_json::to_vec(&schema)
        .map_err(|_| ProjectionFailure::SchemaUnusable)?
        .len();
    if schema_bytes > MAX_CARRIER_BYTES || schema_depth(&schema) > MAX_SCHEMA_DEPTH {
        return Err(ProjectionFailure::SchemaUnusable);
    }
    let patterns = count_patterns(&schema);
    if patterns > MAX_CARRIER_PATTERNS {
        return Err(ProjectionFailure::SchemaUnusable);
    }
    // A dry expansion catches cycles, outside references and the fixed expansion bound before
    // the validator sees the schema.
    Walk::new(&schema, MAX_WALK_STEPS, None).expand(&[&schema])?;
    let validator = jsonschema::options()
        .with_pattern_options(
            jsonschema::PatternOptions::regex()
                .size_limit(PATTERN_SIZE_LIMIT)
                .dfa_size_limit(PATTERN_SIZE_LIMIT),
        )
        .build(&schema)
        .map_err(|_| ProjectionFailure::SchemaUnusable)?;
    let numeric = NumericProfile::of(&schema);
    let compiled = Arc::new(CompiledCarrier {
        schema,
        validator,
        weight: compiled_weight(schema_bytes, patterns),
        numeric,
    });
    Ok(compiled)
}

/// Cost counters of one projection. They are reported so tests and the benchmark can assert
/// bounds on the work done, not on a timer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkCounters {
    /// Steps charged by the walk before and during projection.
    pub steps: usize,
    /// The step limit that applied.
    pub limit: usize,
    /// Bytes kept to remember how numbers were spelled: linear in the payload.
    pub retained_bytes: usize,
}

impl Classifier {
    /// Projects like [`ClassificationInterpreter::project`] and also reports the cost counters.
    pub fn project_counted(
        &self,
        request: ProjectionRequest<'_>,
    ) -> (Result<ClassifiedPayload, ProjectionFailure>, WalkCounters) {
        let mut counters = WalkCounters {
            limit: MAX_WALK_STEPS,
            ..WalkCounters::default()
        };
        let result = self.run(&request, &mut counters);
        (result, counters)
    }

    fn run(
        &self,
        request: &ProjectionRequest<'_>,
        counters: &mut WalkCounters,
    ) -> Result<ClassifiedPayload, ProjectionFailure> {
        if request.rules == ClassificationRules::NONE {
            return Err(ProjectionFailure::LegacyVersion);
        }
        if request.payload.len() > MAX_PAYLOAD_BYTES {
            return Err(ProjectionFailure::PayloadTooLarge);
        }
        let compiled = self.compiled(&request.cache_key, request.document)?;
        let payload: Value = serde_json::from_slice(request.payload)
            .map_err(|_| ProjectionFailure::UnsupportedPayload)?;
        let lexemes = Lexemes::scan(request.payload, compiled.numeric.checks())?;
        counters.retained_bytes = lexemes.retained_bytes;
        // The validator holds numbers as machine values. Refuse a payload whose spelling could
        // make a numeric keyword decide on a different value than the one the author sent.
        if !compiled.numeric.admits(&lexemes) {
            return Err(ProjectionFailure::UnsupportedPayload);
        }
        let mut walk = Walk::new(
            &compiled.schema,
            MAX_WALK_STEPS,
            Some(Instant::now() + MAX_PROJECTION_DURATION),
        );
        // Charge the validator's work and apply the integer rule before the validator runs, so a
        // carrier and payload whose product is unaffordable never reach it.
        let checked = walk::precheck(
            &mut walk,
            &compiled.schema,
            &payload,
            &lexemes.root,
            lexemes.nodes,
        );
        counters.steps = walk.steps();
        checked?;
        if !compiled.validator.is_valid(&payload) {
            return Err(ProjectionFailure::SchemaMismatch);
        }
        let mut projector = Projector::new(walk, request);
        let projected = projector.run(&compiled.schema, &payload, &lexemes.root);
        counters.steps = projector.walk.steps();
        let tree = projected?.ok_or(ProjectionFailure::NotDisclosable)?;
        Ok(ClassifiedPayload {
            tree,
            truncated: projector.budget.truncated,
            disclosed: projector.budget.disclosed,
        })
    }
}

impl ClassificationInterpreter for Classifier {
    fn analyze(&self, document: &[u8]) -> ClassificationAnalysis {
        crate::classification::publication::analyze(document)
    }

    fn prepare(&self, key: &ClassificationCacheKey, rules: ClassificationRules, document: &[u8]) {
        if rules != ClassificationRules::NONE {
            // The outcome, a failure included, is cached for `project` to reproduce.
            let _ = self.compiled(key, document);
        }
    }

    fn project(
        &self,
        request: ProjectionRequest<'_>,
    ) -> Result<ClassifiedPayload, ProjectionFailure> {
        self.project_counted(request).0
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
#[path = "classification/cache.rs"]
mod cache;
#[path = "classification/numbers.rs"]
mod numbers;
#[path = "classification/walk.rs"]
mod walk;

#[path = "classification/publication.rs"]
mod publication;

pub use cache::CacheStats;
pub use publication::{
    analyze_value, resolve_classification_location, visit_classification_locations,
};
