//! Compiled-validator cache with per-key single flight and bounded estimated memory.
//!
//! Compiling a carrier is the expensive step of a disclosure. It must never run while a read
//! transaction holds locks, and it must not serialise unrelated versions: one key compiles once
//! while its other callers wait for that result, different keys compile concurrently up to a small
//! number of permits, and a cached entry (or a remembered compile failure) is a map lookup.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use sagat_application::classification::{
    ClassificationCacheKey, MAX_CLASSIFICATION_CACHE_BYTES, MAX_CLASSIFICATION_CACHE_ENTRIES,
    MAX_CLASSIFICATION_FAILURE_ENTRIES, ProjectionFailure,
};

use super::CompiledCarrier;

/// Compiles that may run at once. More would turn a burst of cold versions into a memory spike.
const MAX_CONCURRENT_COMPILES: usize = 4;

pub(super) type Outcome = Result<Arc<CompiledCarrier>, ProjectionFailure>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    /// Compiled carriers currently cached.
    pub entries: usize,
    /// Estimated compiled bytes of those carriers: what the cache budget bounds.
    pub weight_bytes: usize,
    /// Carriers remembered as unusable.
    pub failures: usize,
}

#[derive(Default)]
struct State {
    entries: HashMap<ClassificationCacheKey, Arc<CompiledCarrier>>,
    order: VecDeque<ClassificationCacheKey>,
    weight: usize,
    failures: HashMap<ClassificationCacheKey, ProjectionFailure>,
    failure_order: VecDeque<ClassificationCacheKey>,
}

impl State {
    fn lookup(&mut self, key: &ClassificationCacheKey) -> Option<Outcome> {
        if let Some(entry) = self.entries.get(key).cloned() {
            self.order.retain(|candidate| candidate != key);
            self.order.push_back(key.clone());
            return Some(Ok(entry));
        }
        self.failures.get(key).map(|failure| Err(*failure))
    }
}

/// The result of one compile, shared with everyone who asked for the same key meanwhile.
#[derive(Default)]
struct Flight {
    outcome: Mutex<Option<Outcome>>,
    ready: Condvar,
}

impl Flight {
    fn wait(&self) -> Outcome {
        let mut outcome = lock(&self.outcome);
        loop {
            if let Some(done) = outcome.as_ref() {
                return done.clone();
            }
            outcome = self
                .ready
                .wait(outcome)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn complete(&self, outcome: Outcome) {
        *lock(&self.outcome) = Some(outcome);
        self.ready.notify_all();
    }
}

struct Permits {
    available: Mutex<usize>,
    freed: Condvar,
}

struct Permit<'a>(&'a Permits);

impl Permits {
    fn acquire(&self) -> Permit<'_> {
        let mut available = lock(&self.available);
        while *available == 0 {
            available = self
                .freed
                .wait(available)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *available -= 1;
        Permit(self)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *lock(&self.0.available) += 1;
        self.0.freed.notify_one();
    }
}

/// Completes a flight even when the compiler panics, so followers never wait forever.
struct Finish<'a> {
    cache: &'a CarrierCache,
    key: &'a ClassificationCacheKey,
    flight: &'a Flight,
    completed: bool,
}

impl Finish<'_> {
    fn complete(mut self, outcome: Outcome) {
        self.finish(outcome);
    }

    fn finish(&mut self, outcome: Outcome) {
        self.completed = true;
        // The result is cached before the flight leaves the table, so a caller that no longer
        // finds the flight finds the entry.
        lock(&self.cache.inflight).remove(self.key);
        self.flight.complete(outcome);
    }
}

impl Drop for Finish<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.finish(Err(ProjectionFailure::SchemaUnusable));
        }
    }
}

pub(super) struct CarrierCache {
    state: Mutex<State>,
    inflight: Mutex<HashMap<ClassificationCacheKey, Arc<Flight>>>,
    permits: Permits,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl Default for CarrierCache {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            inflight: Mutex::new(HashMap::new()),
            permits: Permits {
                available: Mutex::new(MAX_CONCURRENT_COMPILES),
                freed: Condvar::new(),
            },
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }
}

enum Role {
    Leader(Arc<Flight>),
    Follower(Arc<Flight>),
}

impl CarrierCache {
    pub(super) fn stats(&self) -> CacheStats {
        let state = lock(&self.state);
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            entries: state.entries.len(),
            weight_bytes: state.weight,
            failures: state.failures.len(),
        }
    }

    /// Returns the cached carrier, joins a compile already running for `key`, or compiles it.
    /// `before_admission` runs once the cache has missed and before the caller can lead or join a
    /// flight; tests use it to line callers up.
    pub(super) fn get_or_compile<Before, Compile>(
        &self,
        key: &ClassificationCacheKey,
        before_admission: Before,
        compile: Compile,
    ) -> Outcome
    where
        Before: FnOnce(),
        Compile: FnOnce() -> Outcome,
    {
        if let Some(outcome) = self.lookup(key) {
            self.record_hit();
            return outcome;
        }
        before_admission();
        let role = {
            let mut inflight = lock(&self.inflight);
            if let Some(outcome) = self.lookup(key) {
                drop(inflight);
                self.record_hit();
                return outcome;
            }
            match inflight.get(key) {
                Some(running) => Role::Follower(Arc::clone(running)),
                None => {
                    let flight = Arc::new(Flight::default());
                    inflight.insert(key.clone(), Arc::clone(&flight));
                    Role::Leader(flight)
                }
            }
        };
        match role {
            Role::Follower(flight) => {
                let outcome = flight.wait();
                self.record_hit();
                outcome
            }
            Role::Leader(flight) => {
                let finish = Finish {
                    cache: self,
                    key,
                    flight: &flight,
                    completed: false,
                };
                let permit = self.permits.acquire();
                self.record_miss();
                let outcome = compile();
                drop(permit);
                self.remember(key, &outcome);
                finish.complete(outcome.clone());
                outcome
            }
        }
    }

    fn lookup(&self, key: &ClassificationCacheKey) -> Option<Outcome> {
        lock(&self.state).lookup(key)
    }

    fn remember(&self, key: &ClassificationCacheKey, outcome: &Outcome) {
        let mut state = lock(&self.state);
        match outcome {
            Ok(compiled) => {
                if state.entries.contains_key(key) {
                    return;
                }
                let weight = compiled.weight;
                while state.entries.len() >= MAX_CLASSIFICATION_CACHE_ENTRIES
                    || state.weight.saturating_add(weight) > MAX_CLASSIFICATION_CACHE_BYTES
                {
                    let Some(oldest) = state.order.pop_front() else {
                        break;
                    };
                    if let Some(removed) = state.entries.remove(&oldest) {
                        state.weight = state.weight.saturating_sub(removed.weight);
                        self.evictions.fetch_add(1, Ordering::Relaxed);
                        metrics::counter!("classification_cache_total", "outcome" => "evict")
                            .increment(1);
                    }
                }
                if weight <= MAX_CLASSIFICATION_CACHE_BYTES {
                    state.weight = state.weight.saturating_add(weight);
                    state.order.push_back(key.clone());
                    state.entries.insert(key.clone(), Arc::clone(compiled));
                }
            }
            Err(failure) => {
                // A carrier is immutable, so a compile that failed fails the same way forever.
                if state.failures.contains_key(key) {
                    return;
                }
                while state.failures.len() >= MAX_CLASSIFICATION_FAILURE_ENTRIES {
                    let Some(oldest) = state.failure_order.pop_front() else {
                        break;
                    };
                    state.failures.remove(&oldest);
                }
                state.failure_order.push_back(key.clone());
                state.failures.insert(key.clone(), *failure);
            }
        }
    }

    fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
        metrics::counter!("classification_cache_total", "outcome" => "hit").increment(1);
    }

    fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
        metrics::counter!("classification_cache_total", "outcome" => "miss").increment(1);
    }

    #[cfg(test)]
    fn in_flight(&self) -> usize {
        lock(&self.inflight).len()
    }

    #[cfg(test)]
    fn assert_consistent(&self) {
        use std::collections::HashSet;

        let state = lock(&self.state);
        assert_eq!(state.order.len(), state.entries.len());
        assert_eq!(
            state.order.iter().cloned().collect::<HashSet<_>>(),
            state.entries.keys().cloned().collect::<HashSet<_>>(),
            "an LRU key appears exactly once"
        );
        assert_eq!(
            state.weight,
            state
                .entries
                .values()
                .map(|entry| entry.weight)
                .sum::<usize>()
        );
        assert_eq!(state.failure_order.len(), state.failures.len());
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Barrier, mpsc};

    use sagat_application::classification::{
        CarrierPointer, ClassificationCacheKey, MAX_CLASSIFICATION_CACHE_BYTES,
        MAX_CLASSIFICATION_CACHE_ENTRIES, MAX_CLASSIFICATION_FAILURE_ENTRIES, ProjectionFailure,
    };
    use sagat_domain::definition::WorkflowVersionId;
    use serde_json::json;

    use super::super::{CompiledCarrier, NumericProfile};
    use super::{CarrierCache, MAX_CONCURRENT_COMPILES, Ordering};

    fn key() -> ClassificationCacheKey {
        ClassificationCacheKey {
            version: WorkflowVersionId::generate(),
            carrier: CarrierPointer::Input,
        }
    }

    fn compiled(weight: usize) -> Arc<CompiledCarrier> {
        let schema = json!({"type": "null"});
        Arc::new(CompiledCarrier {
            validator: jsonschema::options()
                .build(&schema)
                .expect("fixture schema"),
            numeric: NumericProfile::of(&schema),
            schema,
            weight,
        })
    }

    #[test]
    fn concurrent_same_key_compiles_once_and_accounts_once() {
        let cache = Arc::new(CarrierCache::default());
        let key = key();
        let carrier = compiled(41);
        let before = Arc::new(Barrier::new(2));
        let compilations = Arc::new(AtomicUsize::new(0));

        let handles = (0..2)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let key = key.clone();
                let carrier = Arc::clone(&carrier);
                let before = Arc::clone(&before);
                let compilations = Arc::clone(&compilations);
                std::thread::spawn(move || {
                    cache
                        .get_or_compile(
                            &key,
                            || {
                                before.wait();
                            },
                            || {
                                compilations.fetch_add(1, Ordering::SeqCst);
                                Ok(carrier)
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
        let stats = cache.stats();
        assert_eq!((stats.misses, stats.hits), (1, 1));
        assert_eq!((stats.entries, stats.weight_bytes), (1, 41));
        cache.assert_consistent();
        assert_eq!(cache.in_flight(), 0);
    }

    /// Different versions compile at the same time, and a cached carrier is served while they do.
    /// The compilers wait for each other: a serial cache would never let the second one start.
    #[test]
    fn different_keys_compile_together_and_never_block_a_cached_hit() {
        let cache = Arc::new(CarrierCache::default());
        let cached_key = key();
        cache
            .get_or_compile(&cached_key, || {}, || Ok(compiled(3)))
            .expect("prime");

        let together = Arc::new(Barrier::new(2));
        let (entered_tx, entered_rx) = mpsc::channel();
        let handles = (0..2)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let together = Arc::clone(&together);
                let entered_tx = entered_tx.clone();
                let key = key();
                std::thread::spawn(move || {
                    cache.get_or_compile(
                        &key,
                        || {},
                        || {
                            entered_tx.send(()).expect("entered");
                            together.wait();
                            Ok(compiled(5))
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        entered_rx.recv().expect("first compile entered");
        cache
            .get_or_compile(
                &cached_key,
                || panic!("a cached hit must not wait for admission"),
                || panic!("a cached hit must not compile"),
            )
            .expect("hit while compiles are running");
        for handle in handles {
            handle.join().expect("worker").expect("compiled");
        }
        let stats = cache.stats();
        assert_eq!(stats.entries, 3);
        assert_eq!(stats.weight_bytes, 13);
        cache.assert_consistent();
    }

    #[test]
    fn compiles_beyond_the_permit_count_wait_instead_of_running() {
        let cache = Arc::new(CarrierCache::default());
        let running = Arc::new(AtomicUsize::new(0));
        let highest = Arc::new(AtomicUsize::new(0));
        let handles = (0..MAX_CONCURRENT_COMPILES * 3)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let running = Arc::clone(&running);
                let highest = Arc::clone(&highest);
                let key = key();
                std::thread::spawn(move || {
                    cache.get_or_compile(
                        &key,
                        || {},
                        || {
                            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                            highest.fetch_max(now, Ordering::SeqCst);
                            std::thread::yield_now();
                            running.fetch_sub(1, Ordering::SeqCst);
                            Ok(compiled(1))
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().expect("worker").expect("compiled");
        }
        assert!(highest.load(Ordering::SeqCst) <= MAX_CONCURRENT_COMPILES);
        cache.assert_consistent();
    }

    /// A broken carrier costs one compile, however many reads follow, and it does not take a
    /// place from a usable carrier.
    #[test]
    fn a_failed_compile_is_remembered_without_evicting_usable_entries() {
        let cache = CarrierCache::default();
        let usable = key();
        cache
            .get_or_compile(&usable, || {}, || Ok(compiled(7)))
            .expect("usable");
        let broken = key();
        let compilations = AtomicUsize::new(0);
        for _ in 0..3 {
            let outcome = cache.get_or_compile(
                &broken,
                || {},
                || {
                    compilations.fetch_add(1, Ordering::SeqCst);
                    Err(ProjectionFailure::SchemaUnusable)
                },
            );
            assert!(matches!(outcome, Err(ProjectionFailure::SchemaUnusable)));
        }
        assert_eq!(compilations.load(Ordering::SeqCst), 1);
        let stats = cache.stats();
        assert_eq!((stats.misses, stats.hits), (2, 2));
        assert_eq!((stats.entries, stats.failures), (1, 1));
        assert_eq!(stats.evictions, 0);
        cache.assert_consistent();
        assert_eq!(cache.in_flight(), 0);
    }

    #[test]
    fn remembered_failures_are_bounded() {
        let cache = CarrierCache::default();
        for _ in 0..=MAX_CLASSIFICATION_FAILURE_ENTRIES {
            let _ = cache.get_or_compile(&key(), || {}, || Err(ProjectionFailure::NoCarrier));
        }
        assert_eq!(cache.stats().failures, MAX_CLASSIFICATION_FAILURE_ENTRIES);
        cache.assert_consistent();
    }

    #[test]
    fn a_panicking_compile_releases_the_flight_and_leaves_the_cache_empty() {
        let cache = Arc::new(CarrierCache::default());
        let key = key();
        let panicking = {
            let cache = Arc::clone(&cache);
            let key = key.clone();
            std::thread::spawn(move || {
                let _ = cache.get_or_compile(&key, || {}, || panic!("compiler bug"));
            })
        };
        assert!(panicking.join().is_err());
        assert_eq!(
            cache.in_flight(),
            0,
            "the flight must not outlive its leader"
        );
        let recovered = cache
            .get_or_compile(&key, || {}, || Ok(compiled(9)))
            .expect("a later caller can compile");
        assert_eq!(recovered.weight, 9);
        cache.assert_consistent();
    }

    #[test]
    fn the_budget_counts_estimated_compiled_bytes_and_the_entry_limit_still_holds() {
        let cache = CarrierCache::default();
        let big = MAX_CLASSIFICATION_CACHE_BYTES / 4;
        let keys = (0..5).map(|_| key()).collect::<Vec<_>>();
        for key in &keys {
            cache
                .get_or_compile(key, || {}, || Ok(compiled(big)))
                .expect("compiled");
        }
        let stats = cache.stats();
        assert_eq!(
            stats.entries, 4,
            "four quarter-budget entries fit, the fifth evicts one"
        );
        assert_eq!(stats.weight_bytes, MAX_CLASSIFICATION_CACHE_BYTES);
        assert_eq!(stats.evictions, 1);
        cache.assert_consistent();

        let cache = CarrierCache::default();
        let keys = (0..=MAX_CLASSIFICATION_CACHE_ENTRIES)
            .map(|_| key())
            .collect::<Vec<_>>();
        let entry = compiled(1);
        for key in &keys[..MAX_CLASSIFICATION_CACHE_ENTRIES] {
            cache
                .get_or_compile(key, || {}, || Ok(Arc::clone(&entry)))
                .expect("fill");
        }
        cache
            .get_or_compile(
                &keys[0],
                || panic!("hit entered admission"),
                || panic!("hit compiled"),
            )
            .expect("promote first");
        cache
            .get_or_compile(
                &keys[MAX_CLASSIFICATION_CACHE_ENTRIES],
                || {},
                || Ok(Arc::clone(&entry)),
            )
            .expect("257th");
        let stats = cache.stats();
        assert_eq!(stats.entries, MAX_CLASSIFICATION_CACHE_ENTRIES);
        assert_eq!(stats.evictions, 1);
        assert_eq!(stats.hits, 1);
        cache.assert_consistent();
        let state = super::lock(&cache.state);
        assert!(state.entries.contains_key(&keys[0]));
        assert!(!state.entries.contains_key(&keys[1]));
    }
}
