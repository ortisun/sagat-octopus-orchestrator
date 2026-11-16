//! The run-input gate (design 043, ADR-0090, T043-3): every path that creates a run checks the run's input against the
//! `inputSchema` of the exact version the run pins, through [`RunInputGate::check`], before the transaction that
//! creates the run opens.
//!
//! Two pieces, so that the port is narrow and the policy is in one place (D043-017):
//!
//! - [`SchemaValidator`], an object-safe async port implemented by `crates/infra-schema` (the platform's validator of
//!   the supported subset). The adapter owns the permits, their work-conserving fair queue, the wait, the off-worker
//!   step and the backstop (D043-071): this module names no runtime (ADR-0092).
//! - [`RunInputGate`], the concrete gate: it owns the mode ([`InputValidationMode`]), the metrics and the only non-test
//!   constructor of [`CheckedRunInput`], whose fields are private to this module. The creation inputs of a run
//!   (`NewRun.input`, `ContinuationSelection`, `EvaluatedChainHandoffCommand`) are a [`CheckedRunInput`], so a new
//!   creator that skips the check does not compile, and `scripts/check-architecture.py` fails on a construction of the
//!   type anywhere else (FR-043-016).
//!
//! Nothing here ever formats an input value: a refusal names a closed [`Rule`] and a pointer whose segments are names
//! the schema declares or array indices (D043-049, FR-043-014).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::RunId;
use sagat_domain::identity::TenantId;
use sagat_domain::workflow::{InputSchema, InputSchemaIssue, WorkflowDefinition};

pub use crate::definitions::InputValidationMode;

/// The reason of a refused input that does not satisfy its schema (`details.reason`).
pub const RUN_INPUT_INVALID: &str = "RUN_INPUT_INVALID";
/// The reason of a refused start whose version's schema is outside the supported subset (`details.reason`, and the
/// failure code of a child step or a continuation).
pub const RUN_INPUT_SCHEMA_UNUSABLE: &str = "RUN_INPUT_SCHEMA_UNUSABLE";
/// The failure code of a parent step whose child's input was refused (FR-043-008).
pub const CHILD_INPUT_INVALID: &str = "CHILD_INPUT_INVALID";
/// The failure code of a run whose continuation's input was refused (FR-043-009).
pub const CONTINUATION_INPUT_INVALID: &str = "CONTINUATION_INPUT_INVALID";
/// The capability a process lists in the instance registry only while it runs in `enforce` (FR-043-013).
pub const CAPABILITY: &str = "run-input-validation-v1";

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
    /// The input nests deeper than the validator reads.
    MaxDepth,
    /// A `false` schema, or what names nothing more precise.
    Other,
}

impl Rule {
    /// Every rule, in the order of the contract's closed enum (`details.rule`).
    pub const ALL: [Self; 22] = [
        Self::Type,
        Self::Required,
        Self::Enum,
        Self::Const,
        Self::Minimum,
        Self::Maximum,
        Self::ExclusiveMinimum,
        Self::ExclusiveMaximum,
        Self::MultipleOf,
        Self::MinLength,
        Self::MaxLength,
        Self::Pattern,
        Self::MinItems,
        Self::MaxItems,
        Self::MinProperties,
        Self::MaxProperties,
        Self::AdditionalProperties,
        Self::AnyOf,
        Self::OneOf,
        Self::WorkLimit,
        Self::MaxDepth,
        Self::Other,
    ];

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
/// array index. Never a key the caller chose.
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
            PathSegment::Property(name) => {
                format!("/{}", name.replace('~', "~0").replace('/', "~1"))
            }
            PathSegment::Index(index) => format!("/{index}"),
        })
        .collect()
}

/// The most characters of a pointer an [`Unusable`] keeps; a longer one ends in `…` (D043-104).
pub const MAX_POINTER_CHARS: usize = 512;

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
    /// The finding `issue` at the keyword `keyword` of the schema position `at`, both cut to a bounded length.
    #[must_use]
    pub fn new(issue: InputSchemaIssue, keyword: &str, at: &str) -> Self {
        Self {
            issue,
            keyword: keyword.chars().take(64).collect(),
            at: truncated(at, MAX_POINTER_CHARS),
        }
    }
}

impl fmt::Display for Unusable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.issue.message())?;
        if !self.keyword.is_empty() {
            write!(f, " (keyword '{}')", self.keyword)?;
        }
        Ok(())
    }
}

fn truncated(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(limit - 1).collect();
    out.push('…');
    out
}

/// Why the port did not accept an input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaRefusal {
    /// The input does not satisfy the schema: the first violation.
    Invalid {
        segments: Vec<PathSegment>,
        rule: Rule,
    },
    /// The schema is outside the subset: final, remembered per key by the adapter.
    Unusable(Unusable),
    /// The input nests deeper than the validator reads: `maxDepth`, final.
    TooDeep,
    /// The validation reached the budget: `workLimit`, final the first time (it is deterministic).
    WorkLimit,
    /// No permit within the adapter's wait: transient, never remembered (D043-050, D043-071).
    Busy,
    /// The backstop stopped it: transient, never remembered.
    Backstop,
    /// The compile it waited for panicked: transient, never remembered (D043-106).
    Interrupted,
}

/// The key of a compiled schema, built by the gate: the tenant, the digest the [`InputSchema`] carries, and whether the
/// compiled form may be kept for other starts (false for an ad hoc start, which makes a version per call, D043-041).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SchemaKey {
    pub tenant: TenantId,
    pub digest: [u8; 32],
    pub shared: bool,
}

/// The future the port answers with.
pub type SchemaCheck<'a> = Pin<Box<dyn Future<Output = Result<(), SchemaRefusal>> + Send + 'a>>;

/// The validator port (D043-076): object-safe, held as `Arc<dyn SchemaValidator>`. The adapter waits for a permit of its
/// bounded pool, runs the work off the async workers and stops it at its budget or its backstop.
pub trait SchemaValidator: Send + Sync {
    fn check<'a>(
        &'a self,
        key: SchemaKey,
        schema: &'a InputSchema,
        input: &'a [u8],
    ) -> SchemaCheck<'a>;
}

/// Who creates the run: the closed `creator` label of `run_input_validations_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Creator {
    /// A start through the API: `POST /runs`, an uploaded input, Signal-with-Start, Update-with-Start (rows 1 to 3).
    Api,
    /// A run started again: a rerun, a batch rerun, a reset, a pending reset (rows 4 and 5).
    Repeat,
    /// A schedule's action, backfill, trigger-now or write (rows 6 to 8 and 14).
    Schedule,
    /// An event, webhook, message or cron trigger (rows 9 and 10).
    Trigger,
    /// A child run (row 11).
    Child,
    /// A continue-as-new (row 12).
    Continuation,
    /// An ad hoc database execution, a connection probe, a private rerun (row 13): never cached.
    AdHoc,
}

impl Creator {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Repeat => "repeat",
            Self::Schedule => "schedule",
            Self::Trigger => "trigger",
            Self::Child => "child",
            Self::Continuation => "continuation",
            Self::AdHoc => "ad_hoc",
        }
    }
}

/// What a refusal may say: `Show`, the pointer and the rule, for an input the caller sent; `Root`, `/` and `other`, for
/// every start whose input the caller did not send (a rerun, a batch rerun, a reset, a pending reset, a trigger-now, a
/// private rerun), whatever the caller may read (D043-121, D043-144, D043-148; the graded check of D043-037 is not
/// built, G043-040).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldPolicy {
    Show,
    Root,
}

/// Why the gate refused an input. Nothing in it is a value of the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputRefusal {
    /// The input does not satisfy the schema, or reached the work budget (`workLimit`) or the depth limit
    /// (`maxDepth`): final.
    Invalid { field: String, rule: Rule },
    /// The schema is outside the subset: final.
    SchemaUnusable,
    /// No permit within the wait, a backstop stop or an interrupted compile: transient, try again later.
    Unavailable,
}

impl InputRefusal {
    /// `details.reason` of the refusal (`RUN_INPUT_INVALID`, `RUN_INPUT_SCHEMA_UNUSABLE`); `None` when it is transient.
    #[must_use]
    pub const fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Invalid { .. } => Some(RUN_INPUT_INVALID),
            Self::SchemaUnusable => Some(RUN_INPUT_SCHEMA_UNUSABLE),
            Self::Unavailable => None,
        }
    }

    /// Whether the refusal is final: an input or a schema the version refuses. Nothing tries it again.
    #[must_use]
    pub const fn is_final(&self) -> bool {
        !matches!(self, Self::Unavailable)
    }

    /// The text an audit record or a debug log carries: the reason and the rule, never the field or a value.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Invalid { rule, .. } => format!("{RUN_INPUT_INVALID} rule={}", rule.as_str()),
            Self::SchemaUnusable => RUN_INPUT_SCHEMA_UNUSABLE.to_owned(),
            Self::Unavailable => "UNAVAILABLE".to_owned(),
        }
    }
}

impl From<InputRefusal> for crate::error::AppError {
    fn from(refusal: InputRefusal) -> Self {
        match refusal {
            InputRefusal::Invalid { field, rule } => Self::InvalidRunInput {
                reason: RUN_INPUT_INVALID,
                field: Some(field),
                rule: Some(rule.as_str()),
            },
            InputRefusal::SchemaUnusable => Self::InvalidRunInput {
                reason: RUN_INPUT_SCHEMA_UNUSABLE,
                field: None,
                rule: None,
            },
            InputRefusal::Unavailable => Self::Unavailable,
        }
    }
}

/// How an input passed the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    /// The version has no `inputSchema`: only today's checks (an object of at most 256 KiB) apply.
    NoSchema,
    /// The input satisfies the schema.
    Valid,
    /// `report` mode: the input was refused, the refusal is counted and audited with the run, and the run is created.
    Reported(InputRefusal),
}

/// The creation input of a run, checked against the schema of the version it names (ADR-0090 decision 2). Its fields
/// and its only non-test constructor are private to this module; it has no `Default` and no `Deserialize`.
#[derive(Clone, PartialEq, Eq)]
pub struct CheckedRunInput {
    version: WorkflowVersionId,
    bytes: Vec<u8>,
    outcome: Checked,
}

impl CheckedRunInput {
    /// The version the input was checked against: the stores refuse it for a run of another (FR-043-017).
    #[must_use]
    pub const fn version(&self) -> WorkflowVersionId {
        self.version
    }

    /// The input, as checked.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn outcome(&self) -> &Checked {
        &self.outcome
    }

    /// The audit record of a refusal reported in `report` mode, a follow-up of the run's own `run_audit` about `run`,
    /// written in the transaction that creates the run so that a retry or a replay does not audit twice (D043-039).
    /// `None` for an input that was not reported.
    #[must_use]
    pub fn reported_audit(&self, run_audit: &AuditRecord, run: RunId) -> Option<AuditRecord> {
        self.reported()
            .map(|reported| reported.audit(run_audit, run))
    }

    /// The refusal reported in `report` mode, kept apart from the input for a start that an open holder answers: no run
    /// is created then, so the audit is written with the holder's answer (correctness review of 2026-11-15, Major 2).
    #[must_use]
    pub fn reported(&self) -> Option<ReportedRefusal> {
        let Checked::Reported(refusal) = &self.outcome else {
            return None;
        };
        Some(ReportedRefusal {
            summary: refusal.summary(),
        })
    }

    /// An input that no gate checked, for a test that builds a creation by hand.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn unchecked(version: WorkflowVersionId, bytes: Vec<u8>) -> Self {
        Self {
            version,
            bytes,
            outcome: Checked::NoSchema,
        }
    }
}

/// What `check_before_its_start` counts: the start that follows counts the rest.
const BEFORE_ITS_START: Counting = Counting::BeforeItsStart;
/// What `check_ahead` counts.
const AHEAD: Counting = Counting::Ahead;

/// What one call of the gate counts in `run_input_validations_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Counting {
    /// Every outcome.
    All,
    /// A start an earlier call checked: a refusal and a report only.
    AtStart,
    /// A check its start repeats: all but a report, which the start counts with its audit.
    BeforeItsStart,
    /// A check of a request whose start is checked and counted later: a refusal only.
    Ahead,
}

/// A refusal reported in `report` mode: the reason and the rule, never the field or a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedRefusal {
    summary: String,
}

impl ReportedRefusal {
    /// The audit record of the refusal, a follow-up of the start's own `run_audit` about `run`: the run created, or the
    /// holder that answered the start in its place.
    #[must_use]
    pub fn audit(&self, run_audit: &AuditRecord, run: RunId) -> AuditRecord {
        run_audit.follow_up(
            AuditAction::RunInputRefused,
            ResourceType::Run,
            run.to_string(),
            format!("reported: {}", self.summary),
        )
    }
}

/// The bytes of an input are a classified payload: only their size and how they passed are shown.
impl fmt::Debug for CheckedRunInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckedRunInput")
            .field("version", &self.version)
            .field("len", &self.bytes.len())
            .field("outcome", &self.outcome)
            .finish()
    }
}

/// The gate every creator calls (ADR-0090 decision 1). Not generic: creators hold an `Arc<RunInputGate>`.
pub struct RunInputGate {
    validator: Arc<dyn SchemaValidator>,
    mode: InputValidationMode,
}

impl fmt::Debug for RunInputGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunInputGate")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

/// The port of a gate nobody wired: every schema is answered [`SchemaRefusal::Busy`], so a start of a version with an
/// `inputSchema` is refused as transient (`503`, or tried again later by the engine) and never admitted unchecked
/// (D043-120). A version without one passes, as it would through a wired gate.
struct Unwired;

impl SchemaValidator for Unwired {
    fn check<'a>(&'a self, _: SchemaKey, _: &'a InputSchema, _: &'a [u8]) -> SchemaCheck<'a> {
        Box::pin(async { Err(SchemaRefusal::Busy) })
    }
}

impl RunInputGate {
    #[must_use]
    pub fn new(validator: Arc<dyn SchemaValidator>, mode: InputValidationMode) -> Self {
        Self { validator, mode }
    }

    /// The gate of a service nobody wired: fail-closed for every version with an `inputSchema` (D043-120). The
    /// composition roots wire the platform's validator.
    #[must_use]
    pub fn unwired() -> Arc<Self> {
        Arc::new(Self::new(Arc::new(Unwired), InputValidationMode::Enforce))
    }

    #[must_use]
    pub const fn mode(&self) -> InputValidationMode {
        self.mode
    }

    /// Checks `input` for a run of `version`, whose parsed definition is `definition`.
    ///
    /// # Errors
    /// An [`InputRefusal`]: final for an input or a schema the version refuses, [`InputRefusal::Unavailable`] when the
    /// validator had no room. In `report` mode a final refusal is answered `Ok` with [`Checked::Reported`].
    pub async fn check(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
        definition: &WorkflowDefinition,
        input: Vec<u8>,
        creator: Creator,
        policy: FieldPolicy,
    ) -> Result<CheckedRunInput, InputRefusal> {
        self.check_schema(
            tenant,
            version,
            definition.input_schema(),
            input,
            creator,
            policy,
        )
        .await
    }

    /// As [`Self::check`], for a creator that holds only the schema of the definition (a continuation reads it under
    /// the run's lock and checks after the lock is released).
    ///
    /// # Errors
    /// As [`Self::check`].
    pub async fn check_schema(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
        schema: Option<&InputSchema>,
        input: Vec<u8>,
        creator: Creator,
        policy: FieldPolicy,
    ) -> Result<CheckedRunInput, InputRefusal> {
        self.checked(
            tenant,
            version,
            schema,
            input,
            creator,
            policy,
            Counting::All,
        )
        .await
    }

    /// As [`Self::check`], for a check ahead of a start that [`Self::recheck`] checks again (a schedule's action before
    /// its decision): it counts what the start will not count, an acceptance and a refusal, but not a report, which the
    /// start counts with the audit it writes (architecture and correctness reviews of 2026-11-16, round 2).
    ///
    /// # Errors
    /// As [`Self::check`].
    pub async fn check_before_its_start(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
        definition: &WorkflowDefinition,
        input: Vec<u8>,
        creator: Creator,
        policy: FieldPolicy,
    ) -> Result<CheckedRunInput, InputRefusal> {
        self.checked(
            tenant,
            version,
            definition.input_schema(),
            input,
            creator,
            policy,
            BEFORE_ITS_START,
        )
        .await
    }

    /// As [`Self::check`], for a request whose start comes later and is checked and counted then (a `CANCEL_SOURCE`
    /// reset): only a refusal, which ends the request, is counted here.
    ///
    /// # Errors
    /// As [`Self::check`].
    pub async fn check_ahead(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
        definition: &WorkflowDefinition,
        input: Vec<u8>,
        creator: Creator,
        policy: FieldPolicy,
    ) -> Result<CheckedRunInput, InputRefusal> {
        self.checked(
            tenant,
            version,
            definition.input_schema(),
            input,
            creator,
            policy,
            AHEAD,
        )
        .await
    }

    /// As [`Self::check`], for the start of an input the creator already checked, and counted, a moment before (a
    /// schedule's action, checked before its decision): the result is the same, the validation is counted once
    /// (review of phase 2, D043-147).
    ///
    /// # Errors
    /// As [`Self::check`].
    pub async fn recheck(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
        definition: &WorkflowDefinition,
        input: Vec<u8>,
        creator: Creator,
        policy: FieldPolicy,
    ) -> Result<CheckedRunInput, InputRefusal> {
        self.checked(
            tenant,
            version,
            definition.input_schema(),
            input,
            creator,
            policy,
            Counting::AtStart,
        )
        .await
    }

    #[tracing::instrument(
        name = "run_input.check",
        skip_all,
        fields(
            sagat.run_input.creator = creator.as_str(),
            sagat.run_input.outcome = tracing::field::Empty,
            sagat.run_input.rule = tracing::field::Empty,
        )
    )]
    #[allow(clippy::too_many_arguments)]
    async fn checked(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
        schema: Option<&InputSchema>,
        input: Vec<u8>,
        creator: Creator,
        policy: FieldPolicy,
        counting: Counting,
    ) -> Result<CheckedRunInput, InputRefusal> {
        // a recheck counts only what the check before it could not have counted: a refusal (D043-152), or a report of
        // one in `report` mode, whose audit is written with the run (correctness review of 2026-11-16, Nit 6)
        let count = |creator: Creator, outcome: &'static str| {
            let counts = match counting {
                Counting::All => true,
                Counting::AtStart => matches!(outcome, "refused" | "schema_unusable" | "reported"),
                Counting::BeforeItsStart => outcome != "reported",
                Counting::Ahead => matches!(outcome, "refused" | "schema_unusable"),
            };
            if counts {
                count(creator, outcome);
            } else {
                tracing::Span::current().record("sagat.run_input.outcome", outcome);
            }
        };
        let Some(schema) = schema else {
            count(creator, "no_schema");
            return Ok(CheckedRunInput {
                version,
                bytes: input,
                outcome: Checked::NoSchema,
            });
        };
        let key = SchemaKey {
            tenant,
            digest: *schema.digest(),
            shared: creator != Creator::AdHoc,
        };
        let answer = self.validator.check(key, schema, &input).await;
        let refusal = match answer {
            Ok(()) => {
                count(creator, "accepted");
                return Ok(CheckedRunInput {
                    version,
                    bytes: input,
                    outcome: Checked::Valid,
                });
            }
            Err(refusal) => refused(refusal, policy),
        };
        let span = tracing::Span::current();
        if let InputRefusal::Invalid { rule, .. } = &refusal {
            span.record("sagat.run_input.rule", rule.as_str());
        }
        if !refusal.is_final() {
            count(creator, "deadline");
            tracing::debug!(%version, creator = creator.as_str(), "the run input could not be checked now");
            return Err(refusal);
        }
        tracing::debug!(
            %version,
            creator = creator.as_str(),
            refusal = %refusal.summary(),
            mode = self.mode.as_str(),
            "a run input was refused"
        );
        match self.mode {
            InputValidationMode::Report => {
                count(creator, "reported");
                Ok(CheckedRunInput {
                    version,
                    bytes: input,
                    outcome: Checked::Reported(refusal),
                })
            }
            InputValidationMode::Enforce => {
                count(
                    creator,
                    match refusal {
                        InputRefusal::SchemaUnusable => "schema_unusable",
                        _ => "refused",
                    },
                );
                Err(refusal)
            }
        }
    }
}

/// The refusal a caller is told, under its field policy.
fn refused(refusal: SchemaRefusal, policy: FieldPolicy) -> InputRefusal {
    let invalid = |field: String, rule: Rule| match policy {
        FieldPolicy::Show => InputRefusal::Invalid { field, rule },
        FieldPolicy::Root => InputRefusal::Invalid {
            field: "/".to_owned(),
            rule: Rule::Other,
        },
    };
    match refusal {
        SchemaRefusal::Invalid { segments, rule } => invalid(pointer_of(&segments), rule),
        // A budget miss and a depth past the limit are the same for a schema and an input every time: final, at the
        // root, with their own rule (D043-047, the input depth of Technical Plan 3).
        SchemaRefusal::WorkLimit => invalid("/".to_owned(), Rule::WorkLimit),
        SchemaRefusal::TooDeep => invalid("/".to_owned(), Rule::MaxDepth),
        SchemaRefusal::Unusable(_) => InputRefusal::SchemaUnusable,
        SchemaRefusal::Busy | SchemaRefusal::Backstop | SchemaRefusal::Interrupted => {
            InputRefusal::Unavailable
        }
    }
}

fn count(creator: Creator, outcome: &'static str) {
    tracing::Span::current().record("sagat.run_input.outcome", outcome);
    metrics::counter!(
        "run_input_validations_total",
        "creator" => creator.as_str(),
        "outcome" => outcome,
    )
    .increment(1);
}

/// Doubles of the port for the tests of the creators (T043-21): a validator that answers what it is told, and one that
/// waits on a barrier before it answers.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::{InputSchema, SchemaCheck, SchemaKey, SchemaRefusal, SchemaValidator};

    /// Answers the refusals it was given, one per call, then accepts. Counts its calls.
    #[derive(Debug, Default)]
    pub struct Scripted {
        answers: Mutex<VecDeque<Result<(), SchemaRefusal>>>,
        calls: Mutex<Vec<SchemaKey>>,
    }

    impl Scripted {
        #[must_use]
        pub fn answering(answers: impl IntoIterator<Item = Result<(), SchemaRefusal>>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }

        /// The keys it was called with, in order.
        #[must_use]
        pub fn calls(&self) -> Vec<SchemaKey> {
            self.calls
                .lock()
                .map(|calls| calls.clone())
                .unwrap_or_default()
        }
    }

    impl SchemaValidator for Scripted {
        fn check<'a>(&'a self, key: SchemaKey, _: &'a InputSchema, _: &'a [u8]) -> SchemaCheck<'a> {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(key);
            }
            let answer = self
                .answers
                .lock()
                .ok()
                .and_then(|mut answers| answers.pop_front())
                .unwrap_or(Ok(()));
            Box::pin(async move { answer })
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::test_support::Scripted;
    use super::*;

    /// Counts `run_input_validations_total` by its labels, for one test.
    #[derive(Default)]
    struct Counted(std::sync::Mutex<std::collections::BTreeMap<String, u64>>);

    struct One(String, Arc<Counted>);

    impl metrics::CounterFn for One {
        fn increment(&self, value: u64) {
            *self.1.0.lock().unwrap().entry(self.0.clone()).or_default() += value;
        }
        fn absolute(&self, _: u64) {}
    }

    struct Recorder(Arc<Counted>);

    impl metrics::Recorder for Recorder {
        fn describe_counter(
            &self,
            _: metrics::KeyName,
            _: Option<metrics::Unit>,
            _: metrics::SharedString,
        ) {
        }
        fn describe_gauge(
            &self,
            _: metrics::KeyName,
            _: Option<metrics::Unit>,
            _: metrics::SharedString,
        ) {
        }
        fn describe_histogram(
            &self,
            _: metrics::KeyName,
            _: Option<metrics::Unit>,
            _: metrics::SharedString,
        ) {
        }
        fn register_counter(
            &self,
            key: &metrics::Key,
            _: &metrics::Metadata<'_>,
        ) -> metrics::Counter {
            let labels: Vec<String> = key
                .labels()
                .map(|l| format!("{}={}", l.key(), l.value()))
                .collect();
            metrics::Counter::from_arc(Arc::new(One(
                format!("{}{{{}}}", key.name(), labels.join(",")),
                Arc::clone(&self.0),
            )))
        }
        fn register_gauge(&self, _: &metrics::Key, _: &metrics::Metadata<'_>) -> metrics::Gauge {
            metrics::Gauge::noop()
        }
        fn register_histogram(
            &self,
            _: &metrics::Key,
            _: &metrics::Metadata<'_>,
        ) -> metrics::Histogram {
            metrics::Histogram::noop()
        }
    }

    fn schema() -> InputSchema {
        InputSchema::new(b"{\"type\":\"object\"}".to_vec(), [7; 32])
    }

    async fn check(
        validator: Arc<Scripted>,
        mode: InputValidationMode,
        creator: Creator,
        policy: FieldPolicy,
    ) -> Result<CheckedRunInput, InputRefusal> {
        let gate = RunInputGate::new(validator, mode);
        gate.check_schema(
            TenantId::generate(),
            WorkflowVersionId::generate(),
            Some(&schema()),
            b"{\"secret\":\"sentinel-value\"}".to_vec(),
            creator,
            policy,
        )
        .await
    }

    fn invalid(rule: Rule) -> Result<(), SchemaRefusal> {
        Err(SchemaRefusal::Invalid {
            segments: vec![
                PathSegment::Property("order/id".into()),
                PathSegment::Index(3),
            ],
            rule,
        })
    }

    /// Re-review item 2 (D043-152): a recheck that refuses (a version promoted after the check) is counted as a refusal,
    /// so an action recorded `INPUT_INVALID` has its refusal in the metric beside the accepted check.
    #[test]
    fn a_refusing_recheck_is_counted() {
        use futures_util::FutureExt as _;

        let counted = Arc::new(Counted::default());
        let recorder = Recorder(Arc::clone(&counted));
        let gate = RunInputGate::new(
            Arc::new(Scripted::answering([Ok(()), invalid(Rule::Required)])),
            InputValidationMode::Enforce,
        );
        let definition_schema = schema();
        metrics::with_local_recorder(&recorder, || {
            let tenant = TenantId::generate();
            let version = WorkflowVersionId::generate();
            gate.check_schema(
                tenant,
                version,
                Some(&definition_schema),
                b"{}".to_vec(),
                Creator::Schedule,
                FieldPolicy::Show,
            )
            .now_or_never()
            .unwrap()
            .unwrap();
            gate.checked(
                tenant,
                version,
                Some(&definition_schema),
                b"{}".to_vec(),
                Creator::Schedule,
                FieldPolicy::Show,
                Counting::AtStart,
            )
            .now_or_never()
            .unwrap()
            .unwrap_err();
        });
        let counts = counted.0.lock().unwrap().clone();
        assert_eq!(
            counts,
            std::collections::BTreeMap::from([
                (
                    "run_input_validations_total{creator=schedule,outcome=accepted}".to_owned(),
                    1
                ),
                (
                    "run_input_validations_total{creator=schedule,outcome=refused}".to_owned(),
                    1
                ),
            ])
        );
    }

    /// Correctness review of 2026-11-16, Nit 6: in `report` mode a recheck that reports is counted `reported`, as the
    /// audit it leads to is written, so the metric and the audit rows agree.
    #[test]
    fn a_reporting_recheck_is_counted() {
        use futures_util::FutureExt as _;

        let counted = Arc::new(Counted::default());
        let recorder = Recorder(Arc::clone(&counted));
        let gate = RunInputGate::new(
            Arc::new(Scripted::answering([invalid(Rule::Required)])),
            InputValidationMode::Report,
        );
        let definition_schema = schema();
        metrics::with_local_recorder(&recorder, || {
            let checked = gate
                .checked(
                    TenantId::generate(),
                    WorkflowVersionId::generate(),
                    Some(&definition_schema),
                    b"{}".to_vec(),
                    Creator::Schedule,
                    FieldPolicy::Show,
                    Counting::AtStart,
                )
                .now_or_never()
                .unwrap()
                .unwrap();
            assert!(matches!(checked.outcome(), Checked::Reported(_)));
        });
        let counts = counted.0.lock().unwrap().clone();
        assert_eq!(
            counts,
            std::collections::BTreeMap::from([(
                "run_input_validations_total{creator=schedule,outcome=reported}".to_owned(),
                1
            )])
        );
    }

    /// Round 2 of 2026-11-16 (architecture 3, correctness 1): in `report` mode a schedule's action checked before its
    /// decision and again at its start is counted `reported` once, as its one audit row; a `CANCEL_SOURCE` reset's
    /// request counts nothing that its completion will count.
    #[test]
    fn a_report_is_counted_once_per_start_whatever_checks_it_before() {
        use futures_util::FutureExt as _;

        let counted = Arc::new(Counted::default());
        let recorder = Recorder(Arc::clone(&counted));
        let gate = RunInputGate::new(
            Arc::new(Scripted::answering([
                invalid(Rule::Required),
                invalid(Rule::Required),
                invalid(Rule::Required),
                invalid(Rule::Required),
            ])),
            InputValidationMode::Report,
        );
        let schema = schema();
        metrics::with_local_recorder(&recorder, || {
            let tenant = TenantId::generate();
            let version = WorkflowVersionId::generate();
            // a schedule's action: the check before its decision, then the start's recheck
            // a cancelling reset: the check of its request, then the completion's check
            for (creator, ahead, start) in [
                (Creator::Schedule, BEFORE_ITS_START, Counting::AtStart),
                (Creator::Repeat, AHEAD, Counting::All),
            ] {
                for counting in [ahead, start] {
                    gate.checked(
                        tenant,
                        version,
                        Some(&schema),
                        b"{}".to_vec(),
                        creator,
                        FieldPolicy::Root,
                        counting,
                    )
                    .now_or_never()
                    .unwrap()
                    .unwrap();
                }
            }
        });
        let counts = counted.0.lock().unwrap().clone();
        assert_eq!(
            counts,
            std::collections::BTreeMap::from([
                (
                    "run_input_validations_total{creator=repeat,outcome=reported}".to_owned(),
                    1
                ),
                (
                    "run_input_validations_total{creator=schedule,outcome=reported}".to_owned(),
                    1
                ),
            ])
        );
    }

    #[tokio::test]
    async fn a_version_without_a_schema_is_not_sent_to_the_validator() {
        let validator = Arc::new(Scripted::default());
        let gate = RunInputGate::new(validator.clone(), InputValidationMode::Enforce);
        let version = WorkflowVersionId::generate();
        let checked = gate
            .check_schema(
                TenantId::generate(),
                version,
                None,
                b"{}".to_vec(),
                Creator::Api,
                FieldPolicy::Show,
            )
            .await
            .unwrap();
        assert_eq!(checked.outcome(), &Checked::NoSchema);
        assert_eq!(checked.version(), version);
        assert!(validator.calls().is_empty());
    }

    #[tokio::test]
    async fn an_invalid_input_names_the_pointer_and_the_rule_and_never_the_value() {
        let validator = Arc::new(Scripted::answering([invalid(Rule::Type)]));
        let refusal = check(
            validator,
            InputValidationMode::Enforce,
            Creator::Api,
            FieldPolicy::Show,
        )
        .await
        .unwrap_err();
        assert_eq!(
            refusal,
            InputRefusal::Invalid {
                field: "/order~1id/3".to_owned(),
                rule: Rule::Type
            }
        );
        assert!(!format!("{refusal:?}{}", refusal.summary()).contains("sentinel-value"));
    }

    #[tokio::test]
    async fn a_repeat_whose_source_is_withheld_is_told_the_root_and_other() {
        let validator = Arc::new(Scripted::answering([invalid(Rule::Pattern)]));
        let refusal = check(
            validator,
            InputValidationMode::Enforce,
            Creator::Repeat,
            FieldPolicy::Root,
        )
        .await
        .unwrap_err();
        assert_eq!(
            refusal,
            InputRefusal::Invalid {
                field: "/".to_owned(),
                rule: Rule::Other
            }
        );
    }

    #[tokio::test]
    async fn the_budget_and_the_depth_are_final_at_the_root_and_the_rest_transient() {
        for (answer, expected) in [
            (
                SchemaRefusal::WorkLimit,
                InputRefusal::Invalid {
                    field: "/".to_owned(),
                    rule: Rule::WorkLimit,
                },
            ),
            (
                SchemaRefusal::TooDeep,
                InputRefusal::Invalid {
                    field: "/".to_owned(),
                    rule: Rule::MaxDepth,
                },
            ),
            (
                SchemaRefusal::Unusable(Unusable::new(
                    InputSchemaIssue::KeywordUnsupported,
                    "not",
                    "",
                )),
                InputRefusal::SchemaUnusable,
            ),
            (SchemaRefusal::Busy, InputRefusal::Unavailable),
            (SchemaRefusal::Backstop, InputRefusal::Unavailable),
            (SchemaRefusal::Interrupted, InputRefusal::Unavailable),
        ] {
            let validator = Arc::new(Scripted::answering([Err(answer.clone())]));
            let refusal = check(
                validator,
                InputValidationMode::Enforce,
                Creator::Child,
                FieldPolicy::Show,
            )
            .await
            .unwrap_err();
            assert_eq!(refusal, expected, "{answer:?}");
            assert_eq!(refusal.is_final(), expected != InputRefusal::Unavailable);
        }
    }

    #[tokio::test]
    async fn report_mode_admits_a_final_refusal_and_audits_it_but_never_a_transient_one() {
        let validator = Arc::new(Scripted::answering([invalid(Rule::Required)]));
        let checked = check(
            validator,
            InputValidationMode::Report,
            Creator::Trigger,
            FieldPolicy::Show,
        )
        .await
        .unwrap();
        assert!(matches!(
            checked.outcome(),
            Checked::Reported(InputRefusal::Invalid { .. })
        ));
        let principal_audit = AuditRecord::engine(
            TenantId::generate(),
            AuditAction::RunStarted,
            ResourceType::Run,
            "run",
            "event",
            None,
            time::OffsetDateTime::UNIX_EPOCH,
        );
        let run = RunId::generate();
        let audit = checked.reported_audit(&principal_audit, run).unwrap();
        assert_eq!(audit.action(), AuditAction::RunInputRefused);
        assert_eq!(
            audit.reason(),
            Some("reported: RUN_INPUT_INVALID rule=required")
        );

        let validator = Arc::new(Scripted::answering([Err(SchemaRefusal::Busy)]));
        let refusal = check(
            validator,
            InputValidationMode::Report,
            Creator::Trigger,
            FieldPolicy::Show,
        )
        .await
        .unwrap_err();
        assert_eq!(refusal, InputRefusal::Unavailable);
    }

    #[tokio::test]
    async fn an_ad_hoc_key_is_never_shared_and_every_other_is() {
        for creator in [
            Creator::Api,
            Creator::Repeat,
            Creator::Schedule,
            Creator::Trigger,
            Creator::Child,
            Creator::Continuation,
            Creator::AdHoc,
        ] {
            let validator = Arc::new(Scripted::default());
            check(
                validator.clone(),
                InputValidationMode::Enforce,
                creator,
                FieldPolicy::Show,
            )
            .await
            .unwrap();
            let calls = validator.calls();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].shared, creator != Creator::AdHoc, "{creator:?}");
            assert_eq!(calls[0].digest, [7; 32]);
        }
    }

    #[tokio::test]
    async fn an_unwired_gate_refuses_a_schema_transiently_and_passes_a_version_without_one() {
        let gate = RunInputGate::unwired();
        let tenant = TenantId::generate();
        let version = WorkflowVersionId::generate();
        let refusal = gate
            .check_schema(
                tenant,
                version,
                Some(&schema()),
                b"{}".to_vec(),
                Creator::Api,
                FieldPolicy::Show,
            )
            .await
            .unwrap_err();
        assert_eq!(refusal, InputRefusal::Unavailable);
        assert!(
            gate.check_schema(
                tenant,
                version,
                None,
                b"{}".to_vec(),
                Creator::Api,
                FieldPolicy::Show
            )
            .await
            .is_ok()
        );
    }

    #[test]
    fn the_debug_of_a_checked_input_shows_no_byte_of_it() {
        let input = CheckedRunInput::unchecked(
            WorkflowVersionId::generate(),
            b"{\"k\":\"sentinel\"}".to_vec(),
        );
        assert!(!format!("{input:?}").contains("sentinel"));
    }

    #[test]
    fn a_refusal_maps_to_its_closed_reason() {
        let error: crate::error::AppError = InputRefusal::SchemaUnusable.into();
        assert_eq!(
            error,
            crate::error::AppError::InvalidRunInput {
                reason: RUN_INPUT_SCHEMA_UNUSABLE,
                field: None,
                rule: None
            }
        );
        let error: crate::error::AppError = InputRefusal::Unavailable.into();
        assert!(error.is_retryable());
    }
}
