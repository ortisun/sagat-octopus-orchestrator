//! In-process benchmark for the bounded payload-classification interpreter (T016-053).
//!
//! Run deliberately; the test writes the measured report and records the active
//! Cargo profile when `SAGAT_PERF_REPORT` is set:
//!
//! ```sh
//! SAGAT_PERF_REPORT=tests/performance/reports/payload-classification.md \
//!   cargo test -p sagat-tests --test perf_payload_classification -- --ignored --nocapture
//! ```
#![allow(
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]

use std::time::{Duration, Instant};

use sagat_application::classification::{
    CarrierPointer, ClassificationCacheKey, ClassificationInterpreter, ClassifiedNode,
    ClassifiedPayload, MAX_CARRIER_BYTES, MAX_CARRIER_PATTERNS, MAX_CLASSIFICATION_CACHE_BYTES,
    MAX_CLASSIFICATION_CACHE_ENTRIES, MAX_PAYLOAD_BYTES, MAX_RESPONSE_BYTES, PiiRevealFacts,
    ProjectionRequest, ScalarValue, ValueKind,
};
use sagat_application::payload_disclosure::PAYLOAD_DISCLOSURE_POLICY;
use sagat_domain::classification::{Audience, ClassificationRules, WithheldReason};
use sagat_domain::definition::WorkflowVersionId;
use sagat_infra_schema::classification::{CacheStats, Classifier};
use sagat_telemetry::Metrics;
use serde_json::{Map, Value, json};

const COLD_SAMPLES: usize = 60;
const WARM_SAMPLES: usize = 300;
const WIDE_CHILDREN: usize = 128;
const DEEP_ARRAYS: usize = 17;

struct Scenario {
    name: &'static str,
    document: Vec<u8>,
    payload: Vec<u8>,
    declared_properties: Option<usize>,
}

fn pad_carrier(mut schema: Value) -> Value {
    schema["$comment"] = json!("");
    let base = serde_json::to_vec(&schema).expect("schema").len();
    assert!(
        base <= MAX_CARRIER_BYTES,
        "base carrier is already too large"
    );
    schema["$comment"] = json!("x".repeat(MAX_CARRIER_BYTES - base));
    assert_eq!(
        serde_json::to_vec(&schema).expect("padded schema").len(),
        MAX_CARRIER_BYTES
    );
    schema
}

fn wide_scenario() -> Scenario {
    let mut properties = Map::new();
    let mut payload = Map::new();
    for index in 0..WIDE_CHILDREN {
        let name = format!("p{index:03}");
        properties.insert(
            name.clone(),
            json!({"type": "string", "minLength": 0, "maxLength": 4096}),
        );
        payload.insert(name, json!(""));
    }
    let empty_bytes = serde_json::to_vec(&Value::Object(payload.clone()))
        .expect("wide payload")
        .len();
    let extra = MAX_PAYLOAD_BYTES - empty_bytes;
    for (index, value) in payload.values_mut().enumerate() {
        let length = extra / WIDE_CHILDREN + usize::from(index < extra % WIDE_CHILDREN);
        *value = json!("v".repeat(length));
    }
    let payload = serde_json::to_vec(&Value::Object(payload)).expect("wide payload");
    assert_eq!(payload.len(), MAX_PAYLOAD_BYTES);

    let mut schema = json!({
        "type": "object",
        "x-sagat-classification": "public",
        "properties": properties,
        "additionalProperties": false
    });
    let mut unused = 0usize;
    loop {
        let name = format!("unused{unused:04}");
        schema["properties"]
            .as_object_mut()
            .expect("properties")
            .insert(
                name.clone(),
                json!({"type": "string", "minLength": 0, "maxLength": 4096}),
            );
        if serde_json::to_vec(&schema).expect("active carrier").len()
            > MAX_CARRIER_BYTES.saturating_sub(128)
        {
            schema["properties"]
                .as_object_mut()
                .expect("properties")
                .remove(&name);
            break;
        }
        unused += 1;
    }
    let declared_properties = schema["properties"].as_object().expect("properties").len();
    let schema = pad_carrier(schema);
    let carrier_bytes = serde_json::to_vec(&schema).expect("wide carrier");
    assert_eq!(carrier_bytes.len(), MAX_CARRIER_BYTES);
    Scenario {
        name: "wide (128 public strings)",
        document: serde_json::to_vec(&json!({"inputSchema": schema})).expect("document"),
        payload,
        declared_properties: Some(declared_properties),
    }
}

fn nested_array(value: Value) -> Value {
    (0..DEEP_ARRAYS).fold(value, |child, _| Value::Array(vec![child]))
}

fn deep_scenario() -> Scenario {
    let empty = serde_json::to_vec(&nested_array(json!("")))
        .expect("deep payload")
        .len();
    let payload = serde_json::to_vec(&nested_array(json!("d".repeat(MAX_PAYLOAD_BYTES - empty))))
        .expect("deep payload");
    assert_eq!(payload.len(), MAX_PAYLOAD_BYTES);

    let mut schema = json!({"type": "string"});
    for _ in 0..DEEP_ARRAYS {
        schema = json!({"type": "array", "items": schema});
    }
    schema["x-sagat-classification"] = json!("public");
    let schema = pad_carrier(schema);
    let carrier_bytes = serde_json::to_vec(&schema).expect("deep carrier");
    assert_eq!(carrier_bytes.len(), MAX_CARRIER_BYTES);
    Scenario {
        name: "deep (17 nested arrays)",
        document: serde_json::to_vec(&json!({"inputSchema": schema})).expect("document"),
        payload,
        declared_properties: None,
    }
}

fn request<'a>(
    scenario: &'a Scenario,
    version: WorkflowVersionId,
    payload: &'a [u8],
) -> ProjectionRequest<'a> {
    ProjectionRequest {
        cache_key: ClassificationCacheKey {
            version,
            carrier: CarrierPointer::Input,
        },
        rules: ClassificationRules::CURRENT,
        document: &scenario.document,
        payload,
        audience: Audience::new(true, false, true),
        reveal: PiiRevealFacts {
            granted: false,
            requested: false,
        },
    }
}

fn value_kind(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Object => "object",
        ValueKind::Array => "array",
        ValueKind::String => "string",
        ValueKind::Number => "number",
        ValueKind::Boolean => "boolean",
        ValueKind::Null => "null",
    }
}

fn wire_node(node: &ClassifiedNode) -> Value {
    match node {
        ClassifiedNode::Value {
            level,
            kind,
            value,
            value_truncated,
            original_length,
        } => {
            let mut body = json!({
                "kind": "value",
                "type": value_kind(*kind),
                "level": level.as_str(),
                "value": match value {
                    ScalarValue::String(value) => json!(value),
                    ScalarValue::Number(value) => serde_json::from_str(value.as_str())
                        .expect("the classifier returned a JSON number"),
                    ScalarValue::Boolean(value) => json!(value),
                    ScalarValue::Null => Value::Null,
                }
            });
            if *value_truncated {
                body["valueTruncated"] = json!(true);
                if let Some(length) = original_length {
                    body["length"] = json!(length);
                }
            }
            body
        }
        ClassifiedNode::Container {
            level,
            kind,
            child_count,
            omitted_children,
            children,
        } => json!({
            "kind": "container",
            "type": value_kind(*kind),
            "level": level.as_str(),
            "childCount": child_count,
            "omittedChildren": omitted_children,
            "children": children.iter().map(|child| {
                let mut body = json!({
                    "ordinal": child.ordinal,
                    "node": wire_node(&child.node)
                });
                if let Some(name) = &child.name {
                    body["name"] = json!(name);
                }
                body
            }).collect::<Vec<_>>()
        }),
        ClassifiedNode::Withheld {
            level,
            kind,
            reason,
        } => json!({
            "kind": "withheld",
            "type": value_kind(*kind),
            "level": level.as_str(),
            "reason": match reason {
                WithheldReason::LevelNotPermitted => "LEVEL_NOT_PERMITTED",
                WithheldReason::RevealNotRequested => "REVEAL_NOT_REQUESTED",
                WithheldReason::Secret => "SECRET",
            }
        }),
    }
}

fn response_size(payload_bytes: usize, projection: &ClassifiedPayload) -> usize {
    serde_json::to_vec(&json!({
        "runId": "00000000-0000-0000-0000-000000000000",
        "runVersion": 1,
        "selection": {"source": "input"},
        "policy": PAYLOAD_DISCLOSURE_POLICY,
        "caller": {
            "disclosedLevels": ["public"],
            "revealPii": "not_granted",
            "revealed": false
        },
        "kind": "disclosed",
        "schema": {"carrier": "inputSchema", "versionNumber": 1, "rules": 1},
        "sizeBytes": payload_bytes,
        "truncated": projection.truncated,
        "tree": wire_node(&projection.tree)
    }))
    .expect("wire response")
    .len()
}

#[derive(Default)]
struct Samples {
    latency: Vec<Duration>,
    response_bytes: Vec<usize>,
    failures: usize,
    cache: CacheStats,
}

fn measure_one(
    samples: &mut Samples,
    classifier: &Classifier,
    scenario: &Scenario,
    version: WorkflowVersionId,
) {
    let started = Instant::now();
    let result = classifier.project(request(scenario, version, &scenario.payload));
    samples.latency.push(started.elapsed());
    match result {
        Ok(projection) => samples
            .response_bytes
            .push(response_size(scenario.payload.len(), &projection)),
        Err(_) => samples.failures += 1,
    }
}

fn cold(scenario: &Scenario) -> Samples {
    let mut samples = Samples::default();
    for _ in 0..COLD_SAMPLES {
        let classifier = Classifier::default();
        measure_one(
            &mut samples,
            &classifier,
            scenario,
            WorkflowVersionId::generate(),
        );
        let stats = classifier.cache_stats();
        samples.cache.hits += stats.hits;
        samples.cache.misses += stats.misses;
        samples.cache.evictions += stats.evictions;
    }
    samples
}

fn warm(scenario: &Scenario) -> Samples {
    let classifier = Classifier::default();
    let version = WorkflowVersionId::generate();
    classifier
        .project(request(scenario, version, &scenario.payload))
        .expect("warm the carrier cache");
    let mut samples = Samples::default();
    for _ in 0..WARM_SAMPLES {
        measure_one(&mut samples, &classifier, scenario, version);
    }
    samples.cache = classifier.cache_stats();
    samples
}

fn percentile<T: Copy + Ord>(values: &[T], percentile: usize) -> T {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = (sorted.len() * percentile).div_ceil(100).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn latency_row(scenario: &Scenario, temperature: &str, samples: &Samples) -> String {
    format!(
        "| {} | {temperature} | {} | {:.3} ms | {:.3} ms | {:.3} ms | {} | {} |\n",
        scenario.name,
        samples.latency.len(),
        milliseconds(percentile(&samples.latency, 50)),
        milliseconds(percentile(&samples.latency, 95)),
        milliseconds(percentile(&samples.latency, 99)),
        samples.response_bytes.iter().copied().max().unwrap_or(0),
        samples.failures,
    )
}

fn hit_ratio(stats: CacheStats) -> f64 {
    let total = stats.hits + stats.misses;
    if total == 0 {
        0.0
    } else {
        stats.hits as f64 * 100.0 / total as f64
    }
}

struct Capacity {
    baseline: Option<HeapSnapshot>,
    full_heap: Option<HeapSnapshot>,
    after_drop_heap: Option<HeapSnapshot>,
    retained_bytes: i64,
    retained_allocations: i64,
    released_after_drop_bytes: i64,
    full: CacheStats,
    after_eviction: CacheStats,
    after_revisit: CacheStats,
}

#[derive(Clone, Copy)]
struct HeapSnapshot {
    allocations: u64,
    bytes: u64,
}

fn heap_snapshot() -> Option<HeapSnapshot> {
    let output = std::process::Command::new("heap")
        .arg(std::process::id().to_string())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let summary = text
        .lines()
        .rev()
        .find(|line| line.starts_with("All zones: ") && line.contains(" nodes ("))?;
    let allocations = summary
        .strip_prefix("All zones: ")?
        .split_once(" nodes")?
        .0
        .parse()
        .ok()?;
    let bytes = summary
        .split_once(" nodes (")?
        .1
        .strip_suffix(" bytes)")?
        .parse()
        .ok()?;
    Some(HeapSnapshot { allocations, bytes })
}

fn signed_delta(after: u64, before: u64) -> i64 {
    i64::try_from(after).unwrap_or(i64::MAX) - i64::try_from(before).unwrap_or(i64::MAX)
}

fn cache_capacity(scenario: &Scenario) -> Capacity {
    let classifier = Classifier::default();
    let empty_payload = b"{}";
    let baseline = heap_snapshot();
    let first = WorkflowVersionId::generate();
    classifier
        .project(request(scenario, first, empty_payload))
        .expect("first cache entry");
    for _ in 1..MAX_CLASSIFICATION_CACHE_ENTRIES {
        classifier
            .project(request(
                scenario,
                WorkflowVersionId::generate(),
                empty_payload,
            ))
            .expect("fill cache");
    }
    let full_heap = heap_snapshot();
    let full = classifier.cache_stats();
    classifier
        .project(request(
            scenario,
            WorkflowVersionId::generate(),
            empty_payload,
        ))
        .expect("evict oldest");
    let after_eviction = classifier.cache_stats();
    classifier
        .project(request(scenario, first, empty_payload))
        .expect("oldest entry was evicted");
    let after_revisit = classifier.cache_stats();
    drop(classifier);
    let after_drop_heap = heap_snapshot();
    let retained_bytes = baseline
        .zip(full_heap)
        .map_or(0, |(before, after)| signed_delta(after.bytes, before.bytes));
    let retained_allocations = baseline.zip(full_heap).map_or(0, |(before, after)| {
        signed_delta(after.allocations, before.allocations)
    });
    let released_after_drop_bytes = full_heap
        .zip(after_drop_heap)
        .map_or(0, |(before, after)| signed_delta(before.bytes, after.bytes));
    Capacity {
        baseline,
        full_heap,
        after_drop_heap,
        retained_bytes,
        retained_allocations,
        released_after_drop_bytes,
        full,
        after_eviction,
        after_revisit,
    }
}

struct WeightCheck {
    name: &'static str,
    serialized_bytes: usize,
    estimated_bytes: usize,
    measured_bytes: i64,
}

/// Retained heap of one compiled carrier against the weight the cache charged for it. The weight
/// must be an upper bound, or the cache budget would not bound memory.
fn weight_check(name: &'static str, schema: &Value, payload: &str) -> WeightCheck {
    let document = serde_json::to_vec(&json!({"inputSchema": schema})).expect("document");
    let serialized_bytes = serde_json::to_vec(schema).expect("schema").len();
    assert!(serialized_bytes <= MAX_CARRIER_BYTES, "{name} is too large");
    let classifier = Classifier::default();
    let before = heap_snapshot();
    // The projection result is not asserted: a payload that does not match still compiles the
    // carrier and, for patterns, builds the matcher caches.
    let _ = classifier.project(ProjectionRequest {
        cache_key: ClassificationCacheKey {
            version: WorkflowVersionId::generate(),
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
    });
    let after = heap_snapshot();
    let stats = classifier.cache_stats();
    assert_eq!(stats.entries, 1, "{name} must be cached");
    WeightCheck {
        name,
        serialized_bytes,
        estimated_bytes: stats.weight_bytes,
        measured_bytes: before.zip(after).map_or(i64::MAX, |(before, after)| {
            signed_delta(after.bytes, before.bytes)
        }),
    }
}

fn weight_checks() -> Vec<WeightCheck> {
    let mut wide = Map::new();
    let mut index = 0usize;
    loop {
        wide.insert(
            format!("p{index:04}"),
            json!({"type": "string", "minLength": 0, "maxLength": 4096}),
        );
        index += 1;
        if serde_json::to_vec(&Value::Object(wide.clone()))
            .expect("properties")
            .len()
            > MAX_CARRIER_BYTES - 200
        {
            break;
        }
    }
    let patterns: Vec<Value> = (0..MAX_CARRIER_PATTERNS)
        .map(|index| {
            json!({"type": "string", "pattern": format!("^(a|b|c|d|e|f|g){{1,200}}x{index:03}$")})
        })
        .collect();
    vec![
        weight_check(
            "wide object (largest carrier)",
            &json!({"type": "object", "x-sagat-classification": "public", "properties": wide}),
            "{}",
        ),
        weight_check(
            "3,000-branch allOf",
            &json!({"x-sagat-classification": "public", "allOf": vec![json!({"type": "integer"}); 3_000]}),
            "1",
        ),
        weight_check(
            "large enum",
            &json!({"x-sagat-classification": "public", "enum": (0..8_000).collect::<Vec<_>>()}),
            "1",
        ),
        weight_check(
            "32 patterns (the per-carrier cap)",
            &json!({"x-sagat-classification": "public", "allOf": patterns}),
            "\"abx000\"",
        ),
        weight_check(
            "minimal object",
            &json!({"type": "object", "x-sagat-classification": "public", "properties": {"a": {"type": "string"}}}),
            "{}",
        ),
    ]
}

fn sh(command: &str, arguments: &[&str]) -> String {
    std::process::Command::new(command)
        .args(arguments)
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn repository_root() -> std::path::PathBuf {
    std::env::var_os("SAGAT_PERF_REPOSITORY").map_or_else(
        || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".."),
        Into::into,
    )
}

fn git(arguments: &[&str]) -> String {
    std::process::Command::new("git")
        .current_dir(repository_root())
        .args(arguments)
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn environment() -> String {
    let cpu = if cfg!(target_os = "macos") {
        sh("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        sh(
            "sh",
            &["-c", "grep -m1 'model name' /proc/cpuinfo | cut -d: -f2"],
        )
    };
    let cores = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    let load = sh("sh", &["-c", "uptime | sed 's/.*load averages*: //' "]);
    let mut commit = git(&["rev-parse", "--short", "HEAD"]);
    if !git(&["status", "--porcelain", "--untracked-files=no"])
        .trim()
        .is_empty()
    {
        commit.push_str(" plus uncommitted changes to tracked files");
    }
    let profile = if cfg!(debug_assertions) {
        "debug (unoptimised)"
    } else {
        "release"
    };
    let harness = if env!("CARGO_PKG_NAME") == "sagat-tests" {
        "the registered `perf_payload_classification` target"
    } else {
        "a temporary minimal Cargo harness pinned to the repository lock; the registered target's unrelated dependencies did not compile at the measured base"
    };
    format!(
        "- CPU: {cpu} ({cores} logical cores)\n- OS: {}\n- Rust: {}\n- Load average when the run began: {load} (other work on the machine adds noise)\n- Build profile: {profile}\n- Commit: {commit}\n- Harness: {harness}\n- Execution: one in-process thread, monotonic `Instant`; no database or network\n- Allocation measurement: macOS `heap <pid>` live-allocation snapshots; counts requested live nodes and bytes, not allocator fragmentation or RSS",
        sh("uname", &["-srm"]),
        sh("rustc", &["--version"]),
    )
}

fn harness_failure() -> &'static str {
    if env!("CARGO_PKG_NAME") == "sagat-tests" {
        "none"
    } else {
        "the registered `sagat-tests` target did not compile at the measured base because `infra-postgres` lacked `PathExecutionStore::{Tx, begin_path}` and `PathExecutionSnapshot.data`; the exact benchmark source ran through the minimal pinned harness described above"
    }
}

fn metric_value(page: &str, outcome: &str) -> u64 {
    let prefix = format!("classification_cache_total{{outcome=\"{outcome}\"}} ");
    page.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn mib(bytes: i64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn heap_cell(snapshot: Option<HeapSnapshot>) -> String {
    snapshot.map_or_else(
        || "unavailable".to_owned(),
        |snapshot| {
            format!(
                "{} allocations / {} bytes",
                snapshot.allocations, snapshot.bytes
            )
        },
    )
}

#[test]
#[ignore = "benchmark: run on purpose, see the module documentation"]
fn maximum_carriers_and_payloads_stay_inside_the_measured_bounds() {
    let (telemetry, recorder) = Metrics::detached(Duration::from_secs(600));
    let (wide, deep, wide_cold, wide_warm, deep_cold, deep_warm, capacity, weights) =
        metrics::with_local_recorder(&recorder, || {
            for outcome in ["hit", "miss", "evict"] {
                metrics::counter!("classification_cache_total", "outcome" => outcome).increment(0);
            }
            let wide = wide_scenario();
            let deep = deep_scenario();
            let wide_cold = cold(&wide);
            let wide_warm = warm(&wide);
            let deep_cold = cold(&deep);
            let deep_warm = warm(&deep);
            let capacity = cache_capacity(&wide);
            let weights = weight_checks();
            (
                wide, deep, wide_cold, wide_warm, deep_cold, deep_warm, capacity, weights,
            )
        });

    for (scenario, cold, warm) in [
        (&wide, &wide_cold, &wide_warm),
        (&deep, &deep_cold, &deep_warm),
    ] {
        assert_eq!(scenario.payload.len(), MAX_PAYLOAD_BYTES);
        assert_eq!(cold.failures + warm.failures, 0, "{}", scenario.name);
        assert!(
            cold.response_bytes
                .iter()
                .chain(&warm.response_bytes)
                .all(|size| *size <= MAX_RESPONSE_BYTES),
            "{} response exceeded its bound",
            scenario.name
        );
        assert_eq!(cold.cache.hits, 0);
        assert_eq!(cold.cache.misses, COLD_SAMPLES as u64);
        assert_eq!(warm.cache.hits, WARM_SAMPLES as u64);
        assert_eq!(warm.cache.misses, 1);
    }
    // The budget is estimated compiled bytes, so the 256th carrier does not fit: the cache
    // evicted while it filled, held fewer entries than the entry limit, and stayed in budget.
    assert_eq!(
        capacity.full.misses,
        MAX_CLASSIFICATION_CACHE_ENTRIES as u64
    );
    assert!(capacity.full.evictions > 0, "{:?}", capacity.full);
    assert!(capacity.full.entries < MAX_CLASSIFICATION_CACHE_ENTRIES);
    assert!(capacity.full.weight_bytes <= MAX_CLASSIFICATION_CACHE_BYTES);
    assert_eq!(
        capacity.after_eviction.evictions,
        capacity.full.evictions + 1
    );
    assert_eq!(
        capacity.after_revisit.misses,
        capacity.after_eviction.misses + 1,
        "the first carrier was evicted long ago"
    );
    assert!(
        capacity.baseline.is_some()
            && capacity.full_heap.is_some()
            && capacity.after_drop_heap.is_some(),
        "macOS heap snapshots are required for the allocation evidence"
    );

    let metrics_page = telemetry.render();
    let metric_hit = metric_value(&metrics_page, "hit");
    let metric_miss = metric_value(&metrics_page, "miss");
    let metric_evict = metric_value(&metrics_page, "evict");
    assert_eq!(
        metric_hit,
        wide_cold.cache.hits
            + wide_warm.cache.hits
            + deep_cold.cache.hits
            + deep_warm.cache.hits
            + capacity.after_revisit.hits,
        "{metrics_page}"
    );
    assert_eq!(
        metric_miss,
        wide_cold.cache.misses
            + wide_warm.cache.misses
            + deep_cold.cache.misses
            + deep_warm.cache.misses
            + capacity.after_revisit.misses
            + weights.len() as u64,
        "{metrics_page}"
    );
    assert_eq!(
        metric_evict, capacity.after_revisit.evictions,
        "{metrics_page}"
    );
    assert!(
        !metrics_page.contains("tenant")
            && !metrics_page.contains("payload")
            && !metrics_page.contains("subject"),
        "cache metrics carry only their closed outcome: {metrics_page}"
    );

    for check in &weights {
        assert!(
            check.measured_bytes <= i64::try_from(check.estimated_bytes).expect("weight fits"),
            "{}: measured {} bytes against an estimated weight of {}",
            check.name,
            check.measured_bytes,
            check.estimated_bytes
        );
    }
    assert!(
        capacity.retained_bytes <= i64::try_from(capacity.full.weight_bytes).expect("weight fits"),
        "the retained heap {} exceeds the estimated weight {}",
        capacity.retained_bytes,
        capacity.full.weight_bytes
    );
    let weight_rows = weights
        .iter()
        .map(|check| {
            format!(
                "| {} | {} | {} | {} | {:.1}% |\n",
                check.name,
                check.serialized_bytes,
                check.estimated_bytes,
                check.measured_bytes,
                check.measured_bytes as f64 * 100.0 / check.estimated_bytes as f64
            )
        })
        .collect::<String>();
    let retained_observation = format!(
        "The cache held **{} entries** (limit {MAX_CLASSIFICATION_CACHE_ENTRIES}) with an estimated weight of **{:.2} MiB** (budget {:.0} MiB) after {} evictions, and the allocator-requested live bytes it retained were **{:.2} MiB**. The estimate is an upper bound on every shape in the table above, so the budget bounds compiled-validator heap. It is not process RSS: `heap` does not count allocator fragmentation, resident pages or the per-thread caches that regex matchers build while serving requests.",
        capacity.full.entries,
        mib(i64::try_from(capacity.full.weight_bytes).expect("weight fits")),
        mib(i64::try_from(MAX_CLASSIFICATION_CACHE_BYTES).expect("budget fits")),
        capacity.full.evictions,
        mib(capacity.retained_bytes)
    );

    let mut latency = String::new();
    for (scenario, cold, warm) in [
        (&wide, &wide_cold, &wide_warm),
        (&deep, &deep_cold, &deep_warm),
    ] {
        latency.push_str(&latency_row(scenario, "cold", cold));
        latency.push_str(&latency_row(scenario, "warm", warm));
    }

    let report = format!(
        "# Benchmark: payload classification (feature 016)\n\nMeasured by `tests/performance/payload-classification/main.rs`. These are observations on one shared machine, not latency or capacity promises.\n\n## Environment\n\n{}\n\n## Dataset and method\n\n- Two JSON payloads of exactly **{} bytes**: a wide object with {WIDE_CHILDREN} public string values against {} declared string subschemas, and {DEEP_ARRAYS} nested public arrays with a terminal string beyond the response-depth boundary.\n- Each carrier is exactly **{} bytes**. The wide carrier spends almost all of that limit on compiled property/type/length constraints; `$comment` only fills the final sub-128-byte remainder. The deep carrier uses `$comment` padding and measures the depth boundary, not worst-case validator complexity.\n- Cold rows create a new classifier/version for each of {COLD_SAMPLES} samples. Warm rows prime one immutable `(version, carrier)` and then measure {WARM_SAMPLES} cache hits. Timing covers `Classifier::project`; response encoding is performed afterward and reported separately.\n- Live allocations are macOS `heap <pid>` snapshots: before filling the cache, while the cache holds what its budget admits after 256 distinct versions of the wide carrier were compiled, and after the classifier is dropped. Five further carriers are compiled one at a time and their retained heap is compared with the weight the cache charged for them.\n\n## Latency and response size\n\n| Shape | Cache | Samples | p50 | p95 | p99 | Maximum encoded response | Failures |\n|---|---|---:|---:|---:|---:|---:|---:|\n{latency}\n## Estimated weight against measured heap\n\n| Carrier | Serialized bytes | Estimated weight (bytes) | Measured retained heap (bytes) | Measured / estimated |\n|---|---:|---:|---:|---:|\n{weight_rows}\n## Cache behavior and retained heap\n\n| Measurement | Result |\n|---|---:|\n| wide warm hit ratio (prime included) | {:.3}% ({} hits, {} miss) |\n| deep warm hit ratio (prime included) | {:.3}% ({} hits, {} miss) |\n| cache after 256 distinct wide carriers | {} entries, estimated weight {} bytes of a {} byte budget, {} evictions |\n| baseline live heap | {} |\n| live heap with the cache full | {} |\n| after dropping the classifier | {} |\n| full-cache delta | {} allocations / {} bytes ({:.2} MiB) |\n| live bytes released when the classifier was dropped | {} bytes ({:.2} MiB) |\n| one more distinct carrier | {} total evictions |\n| revisiting the first carrier | {} total evictions (it had been evicted, so it compiled again) |\n| emitted `classification_cache_total` during this run | hit={metric_hit}, miss={metric_miss}, evict={metric_evict} |\n\n{retained_observation}\n\n## Bounds and failures\n\n- Both maximum payloads and both maximum carriers were accepted. All {} measured projections succeeded.\n- Maximum encoded responses were {} bytes (wide) and {} bytes (deep), below the {}-byte response bound. Both are truncated by documented disclosure bounds; truncation is a successful bounded response.\n- Benchmark harness failure: {}.\n- No latency threshold existed before this run (NFR-016-003), so the percentile table establishes a baseline rather than claiming an SLO.\n- `heap` reports requested live allocation nodes and bytes. It does not include allocator fragmentation or resident pages, and the live-byte result must not be presented as RSS.\n\n## Gate status\n\nSC-016-008's required measurements are present for this environment: maximum payload/schema, p50/p95/p99, cache ratio, live allocation count, retained logical heap, encoded response size and failures. The numeric bounds held in this run. The cache budget is an estimate of compiled bytes (decision D643): the table above shows it above the retained heap for every measured shape, in the profile named under Environment. It is a bound on allocator-requested heap, not on RSS, and it assumes the shapes measured here are representative; a carrier shape that compiles larger than 64 bytes per serialized byte plus the pattern allowance would break it, which is why the measurement is repeated whenever the carrier limits or the validator version change.\n",
        environment(),
        MAX_PAYLOAD_BYTES,
        wide.declared_properties.expect("wide properties"),
        MAX_CARRIER_BYTES,
        hit_ratio(wide_warm.cache),
        wide_warm.cache.hits,
        wide_warm.cache.misses,
        hit_ratio(deep_warm.cache),
        deep_warm.cache.hits,
        deep_warm.cache.misses,
        capacity.full.entries,
        capacity.full.weight_bytes,
        MAX_CLASSIFICATION_CACHE_BYTES,
        capacity.full.evictions,
        heap_cell(capacity.baseline),
        heap_cell(capacity.full_heap),
        heap_cell(capacity.after_drop_heap),
        capacity.retained_allocations,
        capacity.retained_bytes,
        mib(capacity.retained_bytes),
        capacity.released_after_drop_bytes,
        mib(capacity.released_after_drop_bytes),
        capacity.after_eviction.evictions,
        capacity.after_revisit.evictions,
        2 * (COLD_SAMPLES + WARM_SAMPLES),
        wide_cold
            .response_bytes
            .iter()
            .chain(&wide_warm.response_bytes)
            .copied()
            .max()
            .unwrap_or(0),
        deep_cold
            .response_bytes
            .iter()
            .chain(&deep_warm.response_bytes)
            .copied()
            .max()
            .unwrap_or(0),
        MAX_RESPONSE_BYTES,
        harness_failure(),
    );
    println!("{report}");
    if let Ok(path) = std::env::var("SAGAT_PERF_REPORT") {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let target = if std::path::Path::new(&path).is_absolute() {
            path.into()
        } else {
            root.join(path)
        };
        std::fs::write(target, report).expect("write report");
    }
}
