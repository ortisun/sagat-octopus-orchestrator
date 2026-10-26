//! The compiled-validator cache is bounded by what compiling keeps in memory (correctness review
//! C4) and remembers carriers that cannot be compiled (security review S5).
#![allow(clippy::expect_used)]

use sagat_application::classification::{
    CarrierPointer, ClassificationCacheKey, ClassificationInterpreter, MAX_CARRIER_BYTES,
    MAX_CARRIER_PATTERNS, MAX_CLASSIFICATION_CACHE_BYTES, PiiRevealFacts, ProjectionFailure,
    ProjectionRequest,
};
use sagat_domain::classification::{Audience, ClassificationRules};
use sagat_domain::definition::WorkflowVersionId;
use sagat_infra_schema::classification::Classifier;
use serde_json::{Map, Value, json};

fn project(
    classifier: &Classifier,
    version: WorkflowVersionId,
    schema: &Value,
    payload: &str,
) -> Result<(), ProjectionFailure> {
    let document = serde_json::to_vec(&json!({"inputSchema": schema})).expect("document");
    classifier
        .project(ProjectionRequest {
            cache_key: ClassificationCacheKey {
                version,
                carrier: CarrierPointer::Input,
            },
            rules: ClassificationRules::CURRENT,
            document: &document,
            payload: payload.as_bytes(),
            audience: Audience::new(false, false, false),
            reveal: PiiRevealFacts {
                granted: false,
                requested: false,
            },
        })
        .map(|_| ())
}

fn patterns(count: usize) -> Value {
    json!({
        "x-sagat-classification": "public",
        "allOf": (0..count)
            .map(|index| json!({"type": "string", "pattern": format!("^(a|b|c){{1,200}}x{index:03}$")}))
            .collect::<Vec<_>>()
    })
}

#[test]
fn a_carrier_may_hold_a_bounded_number_of_patterns() {
    let classifier = Classifier::default();
    assert_eq!(
        project(
            &classifier,
            WorkflowVersionId::generate(),
            &patterns(MAX_CARRIER_PATTERNS),
            "\"abx000\""
        ),
        Err(ProjectionFailure::SchemaMismatch),
        "the carrier compiles; this string only fails to match its patterns"
    );
    assert_eq!(
        project(
            &classifier,
            WorkflowVersionId::generate(),
            &patterns(MAX_CARRIER_PATTERNS + 1),
            "\"abx000\""
        ),
        Err(ProjectionFailure::SchemaUnusable)
    );
}

#[test]
fn pattern_keys_count_like_pattern_keywords() {
    let mut keys = Map::new();
    for index in 0..=MAX_CARRIER_PATTERNS {
        keys.insert(format!("^k{index:03}$"), json!({"type": "string"}));
    }
    let schema = json!({
        "type": "object", "x-sagat-classification": "public", "patternProperties": keys
    });
    assert_eq!(
        project(
            &Classifier::default(),
            WorkflowVersionId::generate(),
            &schema,
            "{}"
        ),
        Err(ProjectionFailure::SchemaUnusable)
    );
}

/// The weight of an entry covers what its patterns keep, so a handful of pattern-heavy carriers
/// fill the budget instead of looking like a few kilobytes of schema.
#[test]
fn a_pattern_heavy_carrier_is_weighed_far_above_its_serialized_size() {
    let classifier = Classifier::default();
    let schema = patterns(MAX_CARRIER_PATTERNS);
    let serialized = serde_json::to_vec(&schema).expect("schema").len();
    let _ = project(
        &classifier,
        WorkflowVersionId::generate(),
        &schema,
        "\"abx000\"",
    );
    let stats = classifier.cache_stats();
    assert_eq!(stats.entries, 1);
    assert!(
        stats.weight_bytes >= MAX_CARRIER_PATTERNS * 128 * 1024,
        "{} bytes for {} patterns",
        stats.weight_bytes,
        MAX_CARRIER_PATTERNS
    );
    assert!(stats.weight_bytes > 100 * serialized);
    assert!(stats.weight_bytes <= MAX_CLASSIFICATION_CACHE_BYTES / 4);
}

#[test]
fn the_largest_carrier_is_weighed_by_its_compiled_size_and_the_budget_evicts_by_weight() {
    let classifier = Classifier::default();
    let mut properties = Map::new();
    let mut index = 0;
    loop {
        properties.insert(
            format!("p{index:04}"),
            json!({"type": "string", "minLength": 0, "maxLength": 4096}),
        );
        index += 1;
        if serde_json::to_vec(&Value::Object(properties.clone()))
            .expect("properties")
            .len()
            > MAX_CARRIER_BYTES - 200
        {
            break;
        }
    }
    let schema = json!({
        "type": "object", "x-sagat-classification": "public", "properties": properties
    });
    assert!(serde_json::to_vec(&schema).expect("schema").len() <= MAX_CARRIER_BYTES);
    for _ in 0..20 {
        project(&classifier, WorkflowVersionId::generate(), &schema, "{}").expect("projection");
    }
    let stats = classifier.cache_stats();
    // 20 carriers of 64 KiB were 2.5 MiB of schema, well under the old 16 MiB limit; weighed by
    // compiled size they overflow the budget and the oldest are evicted.
    assert!(
        stats.weight_bytes <= MAX_CLASSIFICATION_CACHE_BYTES,
        "{stats:?}"
    );
    assert!(stats.evictions > 0, "{stats:?}");
    assert!(stats.entries < 20, "{stats:?}");
}

#[test]
fn an_unusable_carrier_compiles_once_and_does_not_take_a_usable_entry() {
    let classifier = Classifier::default();
    let usable = json!({"type": "string", "x-sagat-classification": "public"});
    let usable_version = WorkflowVersionId::generate();
    project(&classifier, usable_version, &usable, "\"x\"").expect("usable");

    let broken = json!({"x-sagat-classification": "public", "$ref": "#/$defs/missing"});
    let broken_version = WorkflowVersionId::generate();
    for _ in 0..5 {
        assert_eq!(
            project(&classifier, broken_version, &broken, "1"),
            Err(ProjectionFailure::SchemaUnusable)
        );
    }
    let stats = classifier.cache_stats();
    assert_eq!(stats.misses, 2, "one compile for each version");
    assert_eq!(stats.hits, 4);
    assert_eq!((stats.entries, stats.failures, stats.evictions), (1, 1, 0));
    project(&classifier, usable_version, &usable, "\"x\"").expect("still cached");
    assert_eq!(classifier.cache_stats().misses, 2);
}
