//! Bounded JSON Schema classification interpreter.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use sagat_application::classification::{
    CarrierPointer, ClassificationAnalysis, ClassificationCacheKey, ClassificationInterpreter,
    ClassifiedPayload, MAX_CARRIER_BYTES, MAX_CLASSIFICATION_CACHE_BYTES,
    MAX_CLASSIFICATION_CACHE_ENTRIES, MAX_PAYLOAD_BYTES, MAX_PROJECTION_DURATION, MAX_SCHEMA_DEPTH,
    MAX_WALK_STEPS, ProjectionFailure, ProjectionRequest,
};
use serde_json::Value;

use numbers::{Lexemes, NumericProfile};
use walk::{Projector, Walk};

const PATTERN_SIZE_LIMIT: usize = 1024 * 1024;

struct CompiledCarrier {
    schema: Value,
    validator: jsonschema::Validator,
    schema_bytes: usize,
    numeric: NumericProfile,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<ClassificationCacheKey, Arc<CompiledCarrier>>,
    order: VecDeque<ClassificationCacheKey>,
    /// D281 bounds serialized carrier bytes. Compiled-validator heap is measured separately.
    schema_bytes: usize,
}

impl Cache {
    fn promote(&mut self, key: &ClassificationCacheKey) -> Option<Arc<CompiledCarrier>> {
        let entry = self.entries.get(key).cloned()?;
        self.order.retain(|candidate| candidate != key);
        self.order.push_back(key.clone());
        Some(entry)
    }
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
    /// Admits one expensive compiler while the independent cache mutex keeps hits available.
    compilation: Mutex<()>,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl Default for Classifier {
    fn default() -> Self {
        Self {
            cache: Mutex::new(Cache::default()),
            compilation: Mutex::new(()),
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
        self.compiled_with(key, || {}, || compile_carrier(key, document))
    }

    fn compiled_with<BeforeAdmission, Compile>(
        &self,
        key: &ClassificationCacheKey,
        before_admission: BeforeAdmission,
        compile: Compile,
    ) -> Result<Arc<CompiledCarrier>, ProjectionFailure>
    where
        BeforeAdmission: FnOnce(),
        Compile: FnOnce() -> Result<Arc<CompiledCarrier>, ProjectionFailure>,
    {
        if let Some(entry) = self.cached(key) {
            self.record_hit();
            return Ok(entry);
        }

        before_admission();
        let _compilation = self
            .compilation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if let Some(entry) = self.cached(key) {
            self.record_hit();
            return Ok(entry);
        }

        self.record_miss();
        let compiled = compile()?;
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Keep insertion safe if another private insertion path is added later: one key has one
        // LRU position and contributes its serialized schema bytes exactly once.
        if let Some(entry) = cache.promote(key) {
            return Ok(entry);
        }

        let schema_bytes = compiled.schema_bytes;
        while cache.entries.len() >= MAX_CLASSIFICATION_CACHE_ENTRIES
            || cache.schema_bytes.saturating_add(schema_bytes) > MAX_CLASSIFICATION_CACHE_BYTES
        {
            let Some(oldest) = cache.order.pop_front() else {
                break;
            };
            if let Some(removed) = cache.entries.remove(&oldest) {
                cache.schema_bytes = cache.schema_bytes.saturating_sub(removed.schema_bytes);
                self.evictions.fetch_add(1, Ordering::Relaxed);
                metrics::counter!("classification_cache_total", "outcome" => "evict").increment(1);
            }
        }
        if schema_bytes <= MAX_CLASSIFICATION_CACHE_BYTES {
            cache.schema_bytes = cache.schema_bytes.saturating_add(schema_bytes);
            cache.order.push_back(key.clone());
            let replaced = cache.entries.insert(key.clone(), Arc::clone(&compiled));
            debug_assert!(
                replaced.is_none(),
                "cache key was rechecked before insertion"
            );
        }
        Ok(compiled)
    }

    fn cached(&self, key: &ClassificationCacheKey) -> Option<Arc<CompiledCarrier>> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .promote(key)
    }

    fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
        metrics::counter!("classification_cache_total", "outcome" => "hit").increment(1);
    }

    fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
        metrics::counter!("classification_cache_total", "outcome" => "miss").increment(1);
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
        schema_bytes,
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
        if request.rules == sagat_domain::classification::ClassificationRules::NONE {
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
#[path = "classification/numbers.rs"]
mod numbers;
#[path = "classification/walk.rs"]
mod walk;

#[path = "classification/publication.rs"]
mod publication;

pub use publication::{
    analyze_value, resolve_classification_location, visit_classification_locations,
};

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod cache_tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Condvar, Mutex, mpsc};

    use sagat_application::classification::{
        CarrierPointer, ClassificationCacheKey, MAX_CARRIER_BYTES, MAX_CLASSIFICATION_CACHE_BYTES,
        MAX_CLASSIFICATION_CACHE_ENTRIES, ProjectionFailure,
    };
    use sagat_domain::definition::WorkflowVersionId;
    use serde_json::json;

    use super::{Cache, Classifier, CompiledCarrier, NumericProfile};

    fn key() -> ClassificationCacheKey {
        ClassificationCacheKey {
            version: WorkflowVersionId::generate(),
            carrier: CarrierPointer::Input,
        }
    }

    fn compiled(schema_bytes: usize) -> Arc<CompiledCarrier> {
        let schema = json!({"type": "null"});
        Arc::new(CompiledCarrier {
            validator: jsonschema::options()
                .build(&schema)
                .expect("fixture schema"),
            schema,
            schema_bytes,
            numeric: NumericProfile::of(&json!(null)),
        })
    }

    fn assert_consistent(cache: &Cache) {
        assert_eq!(cache.order.len(), cache.entries.len());
        assert_eq!(
            cache.order.iter().cloned().collect::<HashSet<_>>().len(),
            cache.order.len(),
            "an LRU key must appear exactly once"
        );
        assert_eq!(
            cache.order.iter().cloned().collect::<HashSet<_>>(),
            cache.entries.keys().cloned().collect::<HashSet<_>>()
        );
        assert_eq!(
            cache.schema_bytes,
            cache
                .entries
                .values()
                .map(|entry| entry.schema_bytes)
                .sum::<usize>()
        );
    }

    #[test]
    fn concurrent_same_key_compiles_once_and_accounts_once() {
        let classifier = Arc::new(Classifier::default());
        let key = key();
        let compiled = compiled(41);
        let before_admission = Arc::new(Barrier::new(2));
        let compilations = Arc::new(AtomicUsize::new(0));

        let handles = (0..2)
            .map(|_| {
                let classifier = Arc::clone(&classifier);
                let key = key.clone();
                let compiled = Arc::clone(&compiled);
                let before_admission = Arc::clone(&before_admission);
                let compilations = Arc::clone(&compilations);
                std::thread::spawn(move || {
                    classifier
                        .compiled_with(
                            &key,
                            || {
                                before_admission.wait();
                            },
                            || {
                                compilations.fetch_add(1, Ordering::SeqCst);
                                Ok(compiled)
                            },
                        )
                        .expect("compiled carrier")
                })
            })
            .collect::<Vec<_>>();

        let entries = handles
            .into_iter()
            .map(|handle| handle.join().expect("worker"))
            .collect::<Vec<_>>();
        assert!(Arc::ptr_eq(&entries[0], &entries[1]));
        assert_eq!(compilations.load(Ordering::SeqCst), 1);
        assert_eq!(classifier.cache_stats().misses, 1);
        assert_eq!(classifier.cache_stats().hits, 1);

        let cache = classifier
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_consistent(&cache);
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.schema_bytes, 41);
    }

    #[test]
    fn different_key_compiles_are_serial_and_do_not_block_cached_hits() {
        let classifier = Arc::new(Classifier::default());
        let cached_key = key();
        classifier
            .compiled_with(&cached_key, || {}, || Ok(compiled(3)))
            .expect("prime cached key");

        let keys = [key(), key()];
        let before_admission = Arc::new(Barrier::new(2));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum_active = Arc::new(AtomicUsize::new(0));
        let compilations = Arc::new(AtomicUsize::new(0));
        let release_first = Arc::new((Mutex::new(false), Condvar::new()));
        let (first_entered_tx, first_entered_rx) = mpsc::sync_channel(1);

        let handles = keys
            .iter()
            .cloned()
            .map(|key| {
                let classifier = Arc::clone(&classifier);
                let before_admission = Arc::clone(&before_admission);
                let active = Arc::clone(&active);
                let maximum_active = Arc::clone(&maximum_active);
                let compilations = Arc::clone(&compilations);
                let release_first = Arc::clone(&release_first);
                let first_entered_tx = first_entered_tx.clone();
                std::thread::spawn(move || {
                    classifier.compiled_with(
                        &key,
                        || {
                            before_admission.wait();
                        },
                        || {
                            let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                            maximum_active.fetch_max(now_active, Ordering::SeqCst);
                            let sequence = compilations.fetch_add(1, Ordering::SeqCst);
                            if sequence == 0 {
                                first_entered_tx.send(()).expect("first entered");
                                let (released, wake) = &*release_first;
                                let released = released
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                drop(
                                    wake.wait_while(released, |released| !*released)
                                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                                );
                            }
                            active.fetch_sub(1, Ordering::SeqCst);
                            Ok(compiled(5))
                        },
                    )
                })
            })
            .collect::<Vec<_>>();

        first_entered_rx.recv().expect("first compilation entered");
        assert!(classifier.compilation.try_lock().is_err());
        classifier
            .compiled_with(
                &cached_key,
                || panic!("a cached hit must not enter compilation admission"),
                || panic!("a cached hit must not compile"),
            )
            .expect("cached hit remains available");
        assert_eq!(compilations.load(Ordering::SeqCst), 1);
        {
            let (released, wake) = &*release_first;
            *released
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
            wake.notify_one();
        }

        for handle in handles {
            handle
                .join()
                .expect("worker")
                .expect("different key compilation");
        }
        assert_eq!(compilations.load(Ordering::SeqCst), 2);
        assert_eq!(maximum_active.load(Ordering::SeqCst), 1);

        let cache = classifier
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_consistent(&cache);
        assert_eq!(cache.entries.len(), 3);
        assert_eq!(cache.schema_bytes, 13);
    }

    #[test]
    fn failed_compile_releases_admission_without_changing_cache() {
        let classifier = Classifier::default();
        let key = key();
        assert!(matches!(
            classifier.compiled_with(&key, || {}, || Err(ProjectionFailure::SchemaUnusable)),
            Err(ProjectionFailure::SchemaUnusable)
        ));
        {
            let cache = classifier
                .cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_consistent(&cache);
            assert!(cache.entries.is_empty());
        }

        classifier
            .compiled_with(&key, || {}, || Ok(compiled(7)))
            .expect("a later compilation can recover");
        assert_eq!(classifier.cache_stats().misses, 2);
        let cache = classifier
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_consistent(&cache);
        assert_eq!(cache.schema_bytes, 7);
    }

    #[test]
    fn hit_promotes_one_key_and_the_257th_insert_evicts_the_oldest() {
        let classifier = Classifier::default();
        let keys = (0..=MAX_CLASSIFICATION_CACHE_ENTRIES)
            .map(|_| key())
            .collect::<Vec<_>>();
        let entry = compiled(MAX_CARRIER_BYTES);

        for key in &keys[..MAX_CLASSIFICATION_CACHE_ENTRIES] {
            classifier
                .compiled_with(key, || {}, || Ok(Arc::clone(&entry)))
                .expect("fill cache");
        }
        classifier
            .compiled_with(
                &keys[0],
                || panic!("cache hit entered admission"),
                || panic!("cache hit compiled"),
            )
            .expect("promote first key");
        classifier
            .compiled_with(
                &keys[MAX_CLASSIFICATION_CACHE_ENTRIES],
                || {},
                || Ok(Arc::clone(&entry)),
            )
            .expect("insert 257th key");

        let cache = classifier
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_consistent(&cache);
        assert_eq!(cache.entries.len(), MAX_CLASSIFICATION_CACHE_ENTRIES);
        assert_eq!(cache.schema_bytes, MAX_CLASSIFICATION_CACHE_BYTES);
        assert!(cache.entries.contains_key(&keys[0]));
        assert!(!cache.entries.contains_key(&keys[1]));
        assert!(
            cache
                .entries
                .contains_key(&keys[MAX_CLASSIFICATION_CACHE_ENTRIES])
        );
        assert_eq!(classifier.cache_stats().hits, 1);
        assert_eq!(
            classifier.cache_stats().misses,
            (MAX_CLASSIFICATION_CACHE_ENTRIES + 1) as u64
        );
        assert_eq!(classifier.cache_stats().evictions, 1);
    }
}
