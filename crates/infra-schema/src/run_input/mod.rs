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
//! entries and memory and a remembered [`Unusable`]. The permits, the wait, the off-worker step and the async port are
//! the gate's task (T043-3); this module is synchronous.

mod compile;
mod counter;
mod pattern;
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
use crate::classification::cache::{Cache, CacheMetrics, Limits as CacheLimits, Weighed};

pub use compile::CompiledInputSchema;
pub use counter::{Counters, Limits};

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
/// Unusable schemas a process remembers.
pub const MAX_REMEMBERED_UNUSABLE_SCHEMAS: usize = 1_024;

/// Why a schema is outside the subset: the closed issue, the keyword and its JSON pointer in the schema. Both come
/// from the schema, which is author vocabulary; neither ever holds an input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unusable {
    pub issue: InputSchemaIssue,
    /// The keyword that is refused (empty for the size, a depth or the cost of the whole schema).
    pub keyword: String,
    /// Where, as a JSON pointer into the schema (empty for the root).
    pub at: String,
}

impl Unusable {
    pub(crate) fn new(issue: InputSchemaIssue, keyword: &str, at: &str) -> Self {
        Self {
            issue,
            keyword: keyword.chars().take(64).collect(),
            at: at.to_owned(),
        }
    }
}

impl std::fmt::Display for Unusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.issue.message())?;
        if !self.keyword.is_empty() {
            write!(f, " (keyword '{}')", self.keyword)?;
        }
        Ok(())
    }
}

/// The keyword that failed, from a closed set (D043-049): what a refusal says beside its pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rule {
    Type,
    Required,
    Enum,
    Const,
    Minimum,
    Maximum,
    ExclusiveMinimum,
    ExclusiveMaximum,
    MultipleOf,
    MinLength,
    MaxLength,
    Pattern,
    MinItems,
    MaxItems,
    MinProperties,
    MaxProperties,
    AdditionalProperties,
    AnyOf,
    OneOf,
    /// The validation reached the work budget.
    WorkLimit,
    /// The input nests deeper than [`MAX_INPUT_DEPTH`].
    MaxDepth,
    /// A `false` schema, or what names nothing more precise.
    Other,
}

impl Rule {
    /// The keyword's name, as `details.rule` carries it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Required => "required",
            Self::Enum => "enum",
            Self::Const => "const",
            Self::Minimum => "minimum",
            Self::Maximum => "maximum",
            Self::ExclusiveMinimum => "exclusiveMinimum",
            Self::ExclusiveMaximum => "exclusiveMaximum",
            Self::MultipleOf => "multipleOf",
            Self::MinLength => "minLength",
            Self::MaxLength => "maxLength",
            Self::Pattern => "pattern",
            Self::MinItems => "minItems",
            Self::MaxItems => "maxItems",
            Self::MinProperties => "minProperties",
            Self::MaxProperties => "maxProperties",
            Self::AdditionalProperties => "additionalProperties",
            Self::AnyOf => "anyOf",
            Self::OneOf => "oneOf",
            Self::WorkLimit => "workLimit",
            Self::MaxDepth => "maxDepth",
            Self::Other => "other",
        }
    }
}

/// One segment of the instance pointer of a violation: a name declared under `properties` (the schema's), or an
/// array index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    Property(Arc<str>),
    Index(usize),
}

/// The JSON pointer of `segments` (`/` for none).
#[must_use]
pub fn pointer_of(segments: &[PathSegment]) -> String {
    if segments.is_empty() {
        return "/".to_owned();
    }
    segments
        .iter()
        .map(|segment| match segment {
            PathSegment::Property(name) => format!("/{}", pointer::escape(name)),
            PathSegment::Index(index) => format!("/{index}"),
        })
        .collect()
}

/// Why an input was not accepted (the port's `SchemaRefusal` of design 043 but `Busy`, which comes with the permits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaRefusal {
    /// The input does not satisfy the schema: the first violation.
    Invalid {
        segments: Vec<PathSegment>,
        rule: Rule,
    },
    /// The schema is outside the subset.
    Unusable(Unusable),
    /// The input nests deeper than [`MAX_INPUT_DEPTH`]: `maxDepth`, final.
    TooDeep,
    /// The validation reached the budget: `workLimit`, final the first time (it is deterministic).
    WorkLimit,
    /// The backstop stopped it: transient, never remembered.
    Backstop,
}

/// The pre-check of design 043 on the canonical bytes of an `inputSchema`, as publication, `sagat validate` and the
/// inventory run it.
///
/// # Errors
/// The first violation.
pub fn precheck(schema: &[u8]) -> Result<(), Unusable> {
    CompiledInputSchema::compile(schema).map(|_| ())
}

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
    cache: Cache<Key, CompiledInputSchema, Unusable>,
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
                Unusable::new(InputSchemaIssue::KeywordUnsupported, "", ""),
            ),
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
    ) -> Result<Arc<CompiledInputSchema>, Unusable> {
        if !shared {
            metrics::counter!("run_input_schema_cache_total", "result" => "unshared").increment(1);
            return CompiledInputSchema::compile(schema.bytes()).map(Arc::new);
        }
        let key = Key {
            tenant,
            digest: *schema.digest(),
        };
        self.cache.get_or_compile(
            &key,
            || {},
            || CompiledInputSchema::compile(schema.bytes()).map(Arc::new),
        )
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
            .map_err(SchemaRefusal::Unusable)?;
        compiled.validate(input)
    }

    /// The cache counters: compiles (misses), hits, evictions, entries, weight, remembered failures.
    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats()
    }
}
