//! The platform's validator of the supported subset of JSON Schema for run inputs (design 043, OD-2026-11-14-14,
//! ADR-0090 decisions 5 and 6, T043-2).
//!
//! A schema is first pre-checked and compiled ([`CompiledInputSchema::compile`]): its size and depth, a vocabulary
//! walk that records every schema position, the local `$ref`s decoded by the validator itself, a marked depth-first
//! search that refuses a cycle, an application path longer than [`MAX_APPLICATION_PATH`] and a schema every accepted
//! input of which would pass the budget, and every `pattern` against its syntax allowlist and size bounds. The first
//! violation is an [`Unusable`] naming the keyword and its JSON pointer in the schema (author vocabulary, never input).
//! The same pre-check runs at publication, at start (cached) and in `control-plane input-schema-inventory`.
//!
//! An input is then validated by the work-counted validator ([`CompiledInputSchema::validate`]), which stops at
//! [`RUN_INPUT_WORK_UNITS`]. `jsonschema` is not on this path: it is the oracle of a test only (D043-074, enforced by
//! `scripts/check-architecture.py`).
//!
//! [`InputSchemas`] keeps compiled schemas by tenant and schema digest, never shared across tenants, with bounded
//! entries and memory and a remembered [`Unusable`]. [`RunInputValidator`] implements the gate's async port
//! (`sagat_application::run_input::SchemaValidator`, T043-3): it waits for one of [`RUN_INPUT_PERMITS`] permits, shared
//! among tenants by the fair queue of [`permits`], then compiles and validates off the async workers holding it, so
//! that every compile and every validation, an ad hoc one included, runs under a permit (D043-071, G043-031).

mod compile;
mod counter;
mod pattern;
pub mod permits;
mod pointer;
mod validate;
mod value;
mod vocabulary;

use std::sync::Arc;
use std::time::Duration;

use sagat_domain::identity::TenantId;
use sagat_domain::workflow::InputSchema;
pub use sagat_domain::workflow::InputSchemaIssue;

use crate::classification::CacheStats;
use crate::classification::cache::{Cache, CacheMetrics, Limits as CacheLimits, Share, Weighed};

pub use compile::CompiledInputSchema;
pub use counter::{Counters, Limits};
/// Limits of `run-input-validation-v1` held by the pin test of D043-178 (D043-186).
pub use pattern::{MAX_PATTERN_NESTING, MAX_REPETITION};
pub use vocabulary::{MAX_MULTIPLE_OF, MAX_POINTER_BYTES};

/// The work budget of one validation, in units of the counter (NFR-043-BOUND, G043-006, G043-017).
pub const RUN_INPUT_WORK_UNITS: u64 = 1_000_000;
/// The backstop: past this much work the counter stops a validation, transiently (D043-050).
pub const RUN_INPUT_BACKSTOP: Duration = Duration::from_secs(5);
/// The counter reads the clock once every this many units.
pub const BACKSTOP_CLOCK_EVERY: u64 = 4_096;
/// The largest `inputSchema`, in canonical bytes.
pub const MAX_INPUT_SCHEMA_BYTES: usize = 64 * 1024;
/// The deepest an `inputSchema` may nest (the root is depth 1).
pub const MAX_INPUT_SCHEMA_DEPTH: usize = 32;
/// The longest application path from the root, in schema nodes.
pub const MAX_APPLICATION_PATH: usize = 128;
/// The longest name of `properties` or `required`, in bytes: a lookup compares names, and is charged by their length.
pub const MAX_NAME_BYTES: usize = 1024;
/// The deepest an input may nest before it is refused with `maxDepth`.
pub const MAX_INPUT_DEPTH: usize = 64;
/// The most `pattern`s one schema may hold.
pub const MAX_PATTERNS: usize = 32;
/// The largest compiled program of one `pattern`.
pub const MAX_PATTERN_PROGRAM_BYTES: usize = 16 * 1024;
/// The most NFA states of one `pattern`.
pub const MAX_PATTERN_STATES: usize = 512;
/// The most bytes of compiled programs of all the patterns of one schema.
pub const MAX_PATTERN_PROGRAMS_BYTES: usize = 256 * 1024;
/// Compiled schemas a process keeps.
pub const MAX_COMPILED_INPUT_SCHEMAS: usize = 256;
/// Estimated bytes the compiled schemas of a process keep.
pub const MAX_COMPILED_INPUT_SCHEMA_BYTES: usize = 64 * 1024 * 1024;
/// One tenant holds at most this fraction of the compiled schemas, by count and by weight (D043-105).
pub const TENANT_SHARE: usize = 4;
/// Validations that run at once in a process: the permits of the gate (T043-3 builds them on this constant). The runtime
/// caches of a pattern are sized for this many matchers, which a compile-time assertion of `pattern.rs` holds (D043-107).
pub const RUN_INPUT_PERMITS: usize = 4;
/// Unusable schemas a process remembers.
pub const MAX_REMEMBERED_UNUSABLE_SCHEMAS: usize = 1_024;

pub use sagat_application::run_input::{
    MAX_POINTER_CHARS, PathSegment, Rule, SchemaRefusal, Unusable, pointer_of,
};

/// The pre-check of design 043 on the canonical bytes of an `inputSchema`, as publication, `sagat validate` and the
/// inventory run it.
///
/// # Errors
/// The first violation ([`PrecheckRefusal::Unusable`]); [`PrecheckRefusal::Busy`] when no permit came within the wait;
/// [`PrecheckRefusal::Interrupted`] when the compile panicked.
///
/// It compiles up to [`MAX_PATTERNS`] patterns, so it runs off the async workers on a permit of the process's one pool,
/// the starts' (security review of 2026-11-16, Major 2; round 2, architecture 4): a start and a pre-check together hold
/// at most [`RUN_INPUT_PERMITS`], and a pre-check waits at most [`permits::RUN_INPUT_PERMIT_WAIT`]. Counted in
/// `run_input_prechecks_total{outcome}` (`admitted`, `unusable`, `busy`, `interrupted`).
pub fn precheck(schema: &[u8]) -> Result<(), PrecheckRefusal> {
    precheck_on(RunInputValidator::process().permits(), schema, true)
}

/// Why a pre-check did not admit a schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrecheckRefusal {
    /// The schema is outside the subset: final.
    Unusable(Unusable),
    /// No permit within the wait: transient, the caller may try again.
    Busy,
    /// The compile panicked, a defect of the validator: transient, logged at `error`.
    Interrupted,
}

/// [`precheck`] with the permits of `pool`.
///
/// # Errors
/// As [`precheck`].
pub fn precheck_in(pool: &permits::Permits, schema: &[u8]) -> Result<(), PrecheckRefusal> {
    precheck_on(pool, schema, false)
}

/// [`precheck_in`]; `process` says the pool is the process's, whose pre-checks a unit test counts (only those, so a test
/// on a pool of its own cannot raise that count).
fn precheck_on(
    pool: &permits::Permits,
    schema: &[u8],
    #[cfg_attr(not(test), allow(unused_variables))] process: bool,
) -> Result<(), PrecheckRefusal> {
    off_worker(|| {
        let _permit = pool.acquire_blocking(PRECHECKS).map_err(|permits::Busy| {
            metrics::counter!("run_input_prechecks_total", "outcome" => "busy").increment(1);
            PrecheckRefusal::Busy
        })?;
        #[cfg(test)]
        let _seen = process.then(tests::Concurrent::enter);
        // a panic of the compile is the validator's defect, never the schema's: transient, logged with the size only
        // (re-read of 2026-11-16, architecture Nit 6; D043-186)
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            #[cfg(test)]
            if tests::PANIC_ON_COMPILE.with(std::cell::Cell::get) {
                panic!("a compiler fault the test injects");
            }
            CompiledInputSchema::compile(schema)
        }))
        .map_or_else(
            |_| {
                tracing::error!(
                    schema_bytes = schema.len(),
                    "the run-input pre-check panicked: a defect of the validator, reported as transient"
                );
                Err(PrecheckRefusal::Interrupted)
            },
            |compiled| compiled.map(|_| ()).map_err(PrecheckRefusal::Unusable),
        );
        let outcome = match &answer {
            Ok(()) => "admitted",
            Err(PrecheckRefusal::Unusable(_)) => "unusable",
            Err(_) => "interrupted",
        };
        metrics::counter!("run_input_prechecks_total", "outcome" => outcome).increment(1);
        answer
    })
}

/// The owner the pre-checks outside a start queue under in the starts' pool: the parser that runs them does not know the
/// tenant, so they share one place in the fair queue, which keeps them from crowding out the starts of any tenant and
/// the starts from starving them (round 2 of 2026-11-16, architecture 4; D043-177). No tenant has the nil identifier.
const PRECHECKS: TenantId = TenantId::new(uuid::Uuid::nil());

impl Weighed for CompiledInputSchema {
    fn weight(&self) -> usize {
        CompiledInputSchema::weight(self)
    }
}

/// The key of a compiled schema: the tenant and the digest of the schema's canonical bytes. Identical schemas of one
/// tenant share an entry; two tenants never do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    tenant: TenantId,
    digest: [u8; 32],
}

/// The compiled schemas of one process, by tenant and digest (T043-2, SC-043-004).
pub struct InputSchemas {
    cache: Cache<Key, CompiledInputSchema, CompileFailure>,
}

/// Why no compiled schema was answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileFailure {
    /// The schema is outside the subset: final, remembered per key.
    Unusable(Unusable),
    /// The compile that this call waited for panicked: transient, never remembered (D043-106).
    Interrupted,
}

impl Default for InputSchemas {
    fn default() -> Self {
        Self {
            cache: Cache::new(
                CacheLimits {
                    entries: MAX_COMPILED_INPUT_SCHEMAS,
                    bytes: MAX_COMPILED_INPUT_SCHEMA_BYTES,
                    failures: MAX_REMEMBERED_UNUSABLE_SCHEMAS,
                },
                CacheMetrics {
                    name: "run_input_schema_cache_total",
                    label: "result",
                    hit: "hit",
                    miss: "miss",
                    evict: "evicted",
                    failure_hit: "failure_remembered",
                },
                CompileFailure::Interrupted,
            )
            .with_share(Share {
                owner: |key: &Key| key.tenant.as_uuid().as_u128(),
                entries: MAX_COMPILED_INPUT_SCHEMAS / TENANT_SHARE,
                bytes: MAX_COMPILED_INPUT_SCHEMA_BYTES / TENANT_SHARE,
            }),
        }
    }
}

impl std::fmt::Debug for InputSchemas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputSchemas")
            .field("cache", &self.cache.stats())
            .finish()
    }
}

/// The platform's validator behind the gate's port: the compiled schemas of the process and its permits.
#[derive(Debug, Default)]
pub struct RunInputValidator {
    schemas: InputSchemas,
    permits: permits::Permits,
}

impl RunInputValidator {
    /// The validator of this process: one cache and one pool of [`RUN_INPUT_PERMITS`] permits however many services
    /// hold it, so that "four validations at once per process" holds (NFR-043-BOUND).
    #[must_use]
    pub fn process() -> Arc<Self> {
        static PROCESS: std::sync::OnceLock<Arc<RunInputValidator>> = std::sync::OnceLock::new();
        Arc::clone(PROCESS.get_or_init(|| Arc::new(Self::default())))
    }

    /// A gate over the validator of this process, in `mode` (design 043): what a composition root wires.
    #[must_use]
    pub fn gate(
        mode: sagat_application::run_input::InputValidationMode,
    ) -> Arc<sagat_application::run_input::RunInputGate> {
        Arc::new(sagat_application::run_input::RunInputGate::new(
            Self::process(),
            mode,
        ))
    }

    /// A validator whose permits are `permits` (the tests' pools; [`Self::default`] is the process's).
    #[must_use]
    pub fn with_permits(permits: permits::Permits) -> Self {
        Self {
            schemas: InputSchemas::default(),
            permits,
        }
    }

    #[must_use]
    pub const fn schemas(&self) -> &InputSchemas {
        &self.schemas
    }

    #[must_use]
    pub const fn permits(&self) -> &permits::Permits {
        &self.permits
    }
}

impl sagat_application::run_input::SchemaValidator for RunInputValidator {
    fn check<'a>(
        &'a self,
        key: sagat_application::run_input::SchemaKey,
        schema: &'a InputSchema,
        input: &'a [u8],
    ) -> sagat_application::run_input::SchemaCheck<'a> {
        Box::pin(async move {
            let Ok(permit) = self.permits.acquire(key.tenant).await else {
                return Err(SchemaRefusal::Busy);
            };
            // A panic of the compile or of the validation is the validator's fault, never the input's: the caller is told
            // `Interrupted`, transient, like the followers of a compile that panicked, and the process goes on (security
            // and correctness reviews of 2026-11-16, Major 1). Logged with the digest only, never the schema or the input.
            let answer = off_worker(|| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.schemas.check(key.tenant, schema, key.shared, input)
                }))
                .unwrap_or_else(|_| {
                    let digest: String = schema.digest()[..8]
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    tracing::error!(
                        schema_digest = %digest,
                        "the run-input validator panicked: a defect of the validator, reported as transient"
                    );
                    Err(SchemaRefusal::Interrupted)
                })
            });
            drop(permit);
            answer
        })
    }
}

/// Runs `work` where it does not hold an async worker: through `block_in_place` on a multi-thread runtime, in place
/// on a current-thread one (where `block_in_place` would panic and there is no other worker to protect) and outside a
/// runtime. The twin of the private helper of `crates/infra-postgres/src/payload_disclosure.rs` (D043-071). The work is
/// bounded by the counter and its backstop, so nothing has to be cancelled.
fn off_worker<T>(work: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(work)
        }
        _ => work(),
    }
}

impl InputSchemas {
    /// The compiled form of `schema` for `tenant`: from the cache when `shared`, compiled for the call alone when not
    /// (an ad hoc start makes a version per call, so it would only fill the cache).
    ///
    /// # Errors
    /// The pre-check's first violation, remembered per key when `shared`.
    pub fn compiled(
        &self,
        tenant: TenantId,
        schema: &InputSchema,
        shared: bool,
    ) -> Result<Arc<CompiledInputSchema>, CompileFailure> {
        let compile = || {
            #[cfg(test)]
            if tests::PANIC_ON_COMPILE.with(std::cell::Cell::get) {
                panic!("a compiler fault the test injects");
            }
            CompiledInputSchema::compile(schema.bytes())
                .map(Arc::new)
                .map_err(CompileFailure::Unusable)
        };
        if !shared {
            metrics::counter!("run_input_schema_cache_total", "result" => "unshared").increment(1);
            return compile();
        }
        let key = Key {
            tenant,
            digest: *schema.digest(),
        };
        self.cache.get_or_compile(&key, || {}, compile)
    }

    /// Checks `input` against `schema` for `tenant`.
    ///
    /// # Errors
    /// A [`SchemaRefusal`]; [`SchemaRefusal::Unusable`] when the schema is outside the subset.
    pub fn check(
        &self,
        tenant: TenantId,
        schema: &InputSchema,
        shared: bool,
        input: &[u8],
    ) -> Result<(), SchemaRefusal> {
        let compiled = self
            .compiled(tenant, schema, shared)
            .map_err(|failure| match failure {
                CompileFailure::Unusable(unusable) => SchemaRefusal::Unusable(unusable),
                CompileFailure::Interrupted => SchemaRefusal::Interrupted,
            })?;
        compiled.validate(input)
    }

    /// The cache counters: compiles (misses), hits, evictions, entries, weight, remembered failures.
    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats()
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    thread_local! {
        /// Makes the next compile of this thread panic, as a compiler fault would.
        pub(super) static PANIC_ON_COMPILE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    static PRECHECKS_NOW: AtomicUsize = AtomicUsize::new(0);
    static PRECHECKS_PEAK: AtomicUsize = AtomicUsize::new(0);

    /// Counts the pre-checks that run at once, for the test of their bound.
    pub(super) struct Concurrent;

    impl Concurrent {
        pub(super) fn enter() -> Self {
            let now = PRECHECKS_NOW.fetch_add(1, Ordering::SeqCst) + 1;
            PRECHECKS_PEAK.fetch_max(now, Ordering::SeqCst);
            Self
        }
    }

    impl Drop for Concurrent {
        fn drop(&mut self) {
            PRECHECKS_NOW.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn heavy_schema() -> Vec<u8> {
        let mut properties = serde_json::Map::new();
        for i in 0..32 {
            properties.insert(
                format!("p{i:02}"),
                serde_json::json!({"type": "string", "pattern": format!("q{i:02}[ab]{{12}}c{{40}}d{{40}}")}),
            );
        }
        serde_json::to_vec(&serde_json::json!({"type": "object", "properties": properties}))
            .expect("json")
    }

    /// Security review of 2026-11-16, Major 2: the pre-check a parse runs (publication, a dry run, an ad hoc
    /// document) compiles up to 32 patterns; it runs off the async workers and at most `RUN_INPUT_PERMITS` at once
    /// per process, like a start's compile.
    #[test]
    fn prechecks_outside_a_start_run_at_most_four_at_once() {
        let schema = Arc::new(heavy_schema());
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let schema = Arc::clone(&schema);
                std::thread::spawn(move || {
                    for _ in 0..4 {
                        precheck(&schema).expect("admitted");
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("joined");
        }
        let peak = PRECHECKS_PEAK.load(Ordering::SeqCst);
        assert!(
            peak <= RUN_INPUT_PERMITS,
            "{peak} pre-checks ran at once, more than {RUN_INPUT_PERMITS}"
        );
    }

    /// Round 2 of 2026-11-16 (architecture 4, critic 3, intermediate zoom-out 2): a pre-check outside a start takes its
    /// permit from the starts' pool, so the two never hold more than its permits together, and it waits at most the pool's
    /// wait: with every permit held it is answered `Busy`, transient, instead of waiting without a limit.
    #[test]
    fn a_precheck_waits_in_the_starts_pool_and_is_busy_after_its_wait() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime");
        let pool = permits::Permits::new(2, std::time::Duration::from_millis(200));
        let held: Vec<_> = runtime.block_on(async {
            let mut held = Vec::new();
            for _ in 0..2 {
                held.push(pool.acquire(TenantId::generate()).await.expect("free"));
            }
            held
        });
        let schema = heavy_schema();
        let started = std::time::Instant::now();
        let answer = precheck_in(&pool, &schema);
        assert_eq!(
            answer,
            Err(PrecheckRefusal::Busy),
            "the starts hold every permit"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "bounded by the pool's wait"
        );
        assert_eq!(pool.waiting(), 0, "the pre-check left the queue");
        drop(held);
        assert_eq!(precheck_in(&pool, &schema), Ok(()));
        assert_eq!(pool.free(), 2, "its permit went back");
    }

    /// A pre-check whose compile panics is answered `Interrupted`, transient, and the process goes on (re-read of
    /// 2026-11-16, architecture Nit 6).
    #[test]
    fn a_precheck_whose_compile_panics_is_interrupted() {
        let pool = permits::Permits::new(1, std::time::Duration::from_millis(200));
        PANIC_ON_COMPILE.with(|flag| flag.set(true));
        let answer = std::panic::catch_unwind(|| precheck_in(&pool, br#"{"type": "object"}"#));
        PANIC_ON_COMPILE.with(|flag| flag.set(false));
        assert_eq!(
            answer.ok(),
            Some(Err(PrecheckRefusal::Interrupted)),
            "it did not unwind"
        );
        assert_eq!(pool.free(), 1, "its permit went back");
        assert_eq!(precheck_in(&pool, br#"{"type": "object"}"#), Ok(()));
    }

    /// Security and correctness reviews of 2026-11-16, Major 1: the caller whose compile panics (the leader of the
    /// single flight) is answered `Interrupted`, transient, as its followers are, and the process does not unwind;
    /// nothing is remembered.
    #[test]
    fn the_caller_whose_compile_panics_is_answered_interrupted() {
        use sagat_application::run_input::{SchemaKey, SchemaValidator};

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime");
        let validator = RunInputValidator::default();
        let bytes = br#"{"type": "object"}"#.to_vec();
        let schema = InputSchema::new(bytes, [7; 32]);
        let key = SchemaKey {
            tenant: TenantId::generate(),
            digest: [7; 32],
            shared: true,
        };
        let answer = runtime.block_on(async {
            PANIC_ON_COMPILE.with(|flag| flag.set(true));
            let answer = validator.check(key, &schema, b"{}").await;
            PANIC_ON_COMPILE.with(|flag| flag.set(false));
            answer
        });
        assert_eq!(answer, Err(SchemaRefusal::Interrupted));
        assert_eq!(validator.schemas().cache_stats().failures, 0);
        let again = runtime.block_on(validator.check(key, &schema, b"{}"));
        assert_eq!(again, Ok(()), "the next start compiles and passes");
    }

    /// A compile that panics leaves its waiting callers a transient outcome, never a final `Unusable`, and nothing is
    /// remembered for the key (D043-106).
    #[test]
    fn a_compile_that_panics_is_transient_for_those_who_waited() {
        let schemas = InputSchemas::default();
        let key = Key {
            tenant: TenantId::generate(),
            digest: [1; 32],
        };
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = schemas
                .cache
                .get_or_compile(&key, || {}, || panic!("compiler bug"));
        }));
        assert!(panicked.is_err());
        assert_eq!(schemas.cache_stats().failures, 0, "nothing is remembered");
        assert_eq!(
            schemas.cache.panicked_outcome(),
            CompileFailure::Interrupted
        );
    }
}
