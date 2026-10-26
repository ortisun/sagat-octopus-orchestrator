//! Workflow definition use cases: drafts, publication and immutable versions.
//!
//! Every use case receives an authenticated [`Principal`], authorizes before touching storage and
//! derives the tenant from it. Writes are audited in the same transaction as the change, and the
//! `Idempotency-Key` check is atomic with the write (the store receives the key and a fingerprint of
//! the request; a repeat returns the original result, a different request under the same key is a
//! conflict).

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::classification::ClassificationRules;
use sagat_domain::definition::{Workflow, WorkflowId, WorkflowVersion, WorkflowVersionId};
use sagat_domain::identity::{Namespace, NamespaceId, Permission, Principal, Slug, TenantId};
use sagat_domain::saga::{SagaPlan, compensation_plans};
use sagat_domain::workflow::{
    DefinitionWarning, Digest, NodeId, NodeKind, WorkflowDefinition, WorkflowError,
};
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::dispatch::{TriggerRegistration, triggers_of};
use crate::error::{AppError, StoreError};
use crate::identity::{Page, PageRequest, RequestContext};
use crate::quotas::QuotaLookup;
use crate::scripts::ScriptLookup;
use crate::secrets::{SecretAccessLookup, SecretProblem, check_usable};
use sagat_domain::quota::QuotaResource;

/// Largest accepted workflow document.
pub const MAX_IR_BYTES: usize = 1024 * 1024;

/// The problems of the nodes that pin an artifact in `missing`, found in one pass over the nodes
/// (a document can pin thousands of distinct artifacts, and one pass per artifact would hold a
/// thread of the platform for as long as it takes to compare every pair).
#[must_use]
pub fn unknown_script_problems(
    definition: &sagat_domain::workflow::WorkflowDefinition,
    missing: &[sagat_domain::scripts::Digest],
) -> Vec<Problem> {
    let missing: std::collections::BTreeSet<&sagat_domain::scripts::Digest> =
        missing.iter().collect();
    definition
        .nodes()
        .filter_map(|node| {
            let NodeKind::Activity { activity, .. } = node.kind() else {
                return None;
            };
            let digest = sagat_domain::scripts::digest_of_activity(activity.as_str())?;
            missing.contains(&digest).then(|| {
                Problem::error(
                    "UNKNOWN_SCRIPT",
                    format!("the script artifact {digest} was not built for this tenant"),
                )
                .at_node(node.id().as_str())
            })
        })
        .collect()
}
/// Longest release note.
pub const MAX_RELEASE_NOTES: usize = 2048;

/// Release notes are text for people to read, in the history of a workflow among other places: up
/// to [`MAX_RELEASE_NOTES`] bytes, with new lines and tabs but no other control character and none
/// of the format characters that reorder or hide text.
pub(crate) fn check_release_notes(notes: Option<&str>) -> Result<(), AppError> {
    let Some(notes) = notes else {
        return Ok(());
    };
    if notes.len() > MAX_RELEASE_NOTES {
        return Err(AppError::NotAllowed("release notes are too long"));
    }
    if !notes
        .chars()
        .all(|c| crate::operations::is_plain(c) || c == '\n' || c == '\t')
    {
        return Err(AppError::NotAllowed(
            "release notes are plain text: no control or format characters",
        ));
    }
    Ok(())
}

/// One problem found in a workflow document. `path` is a JSON pointer; `message` never contains
/// values from the document beyond a clipped fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrIssue {
    pub path: String,
    pub message: String,
}

/// Why a workflow document cannot be published.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IrError {
    #[error("the document is larger than {MAX_IR_BYTES} bytes")]
    TooLarge,
    #[error("the document is not valid JSON")]
    Malformed,
    #[error("the document does not match the workflow schema ({} problem(s))", .0.len())]
    Schema(Vec<IrIssue>),
    #[error("classification annotations are invalid ({} finding(s))", .0.len())]
    Classification(Vec<crate::classification::ClassificationFinding>),
    /// A node kind that exists in the schema but that this version of the engine does not run.
    #[error("node '{node}' is a '{kind}', which is not supported yet: {reason}")]
    Unsupported {
        node: String,
        kind: String,
        reason: &'static str,
    },
    /// A combination the accepted design allows but whose prerequisite is not built yet (decision
    /// D295: an approval inside a collection or loop body waits for scoped compensation). Unlike
    /// [`Self::Unsupported`] it names what has to exist first, so a client can tell a closed door
    /// from a missing feature.
    #[error("node '{node}' is a '{kind}', which waits for {dependency}: {reason}")]
    Pending {
        node: String,
        kind: String,
        /// The closed name of the missing prerequisite (`SCOPED_COMPENSATION`).
        dependency: &'static str,
        reason: &'static str,
    },
    #[error("declaration '{field}' is not supported yet: {reason}")]
    UnsupportedDeclaration {
        field: &'static str,
        reason: &'static str,
    },
    #[error("{0}")]
    Invalid(WorkflowError),
    /// Unlike an ordinary missing root target, this edge leaves its named body's closed graph.
    #[error("body '{body}': node '{from}' references '{to}' outside its declared body")]
    EscapingBodyEdge {
        body: NodeId,
        from: NodeId,
        to: NodeId,
    },
    #[error("node '{node}', field '{field}': {message}")]
    Expression {
        node: String,
        field: &'static str,
        message: String,
    },
    #[error("the document names the workflow '{document}' but this workflow is '{workflow}'")]
    NameMismatch { document: String, workflow: String },
    /// A `script` node pins an artifact that no ready version of the tenant built.
    #[error("a script node pins the artifact {digest}, which was not built for this tenant")]
    UnknownScript { digest: String },
}

/// How serious something a dry-run validation found is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The document cannot be published.
    Error,
    /// It can, and the author should know.
    Warning,
}

impl Severity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

/// One thing a dry-run validation found in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub severity: Severity,
    /// Stable, upper snake case; a client may branch on it.
    pub code: &'static str,
    /// Safe to show: never a value of the document beyond a clipped fragment.
    pub message: String,
    /// The node the problem is about, when a rule names one.
    pub node: Option<String>,
    /// A JSON pointer into the document, for a schema problem.
    pub path: Option<String>,
}

impl Problem {
    fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            node: None,
            path: None,
        }
    }

    fn at_node(mut self, node: impl Into<String>) -> Self {
        self.node = Some(node.into());
        self
    }

    fn at_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

/// What a dry-run validation found: every problem, and the digest of the canonical document when
/// it is publishable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Validation {
    pub problems: Vec<Problem>,
    pub warnings: Vec<Problem>,
    pub digest: Option<String>,
    pub classification: Option<crate::classification::ClassificationAnalysis>,
}

impl Validation {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.problems.is_empty()
    }
}

/// The problems that make `error` unpublishable, one per thing wrong that the codec found.
#[must_use]
pub fn problems_of(error: &IrError) -> Vec<Problem> {
    match error {
        IrError::TooLarge => vec![Problem::error("DOCUMENT_TOO_LARGE", error.to_string())],
        IrError::Malformed => vec![Problem::error("DOCUMENT_MALFORMED", error.to_string())],
        IrError::Classification(findings) => {
            let mut problems: Vec<_> = findings
                .iter()
                .filter(|finding| {
                    finding.code.severity() == crate::classification::FindingSeverity::Error
                })
                .map(|finding| {
                    Problem::error(finding.code.as_str(), finding.code.message())
                        .at_path(finding.path.as_str())
                })
                .collect();
            if problems.is_empty() {
                problems.push(Problem::error(
                    "CLASSIFICATION_TOO_COMPLEX",
                    "classification carrier exceeds a fixed complexity bound",
                ));
            }
            problems
        }
        IrError::Schema(issues) => issues
            .iter()
            .map(|issue| {
                Problem::error("SCHEMA", issue.message.clone()).at_path(issue.path.clone())
            })
            .collect(),
        IrError::Unsupported { node, .. } | IrError::Pending { node, .. } => {
            vec![Problem::error("UNSUPPORTED_NODE", error.to_string()).at_node(node.clone())]
        }
        IrError::UnsupportedDeclaration { field, .. } => {
            vec![Problem::error("UNSUPPORTED_NODE", error.to_string()).at_path(format!("/{field}"))]
        }
        // The message names the field: a `path` is a pointer into the document, and the codec knows the
        // node by its id, not where it stands in the list.
        IrError::Expression { node, .. } => {
            vec![Problem::error("EXPRESSION", error.to_string()).at_node(node.clone())]
        }
        IrError::NameMismatch { .. } => vec![Problem::error("NAME_MISMATCH", error.to_string())],
        IrError::UnknownScript { .. } => vec![Problem::error("UNKNOWN_SCRIPT", error.to_string())],
        IrError::Invalid(rule) => {
            let (code, node) = graph_rule(rule);
            let problem = Problem::error(code, rule.to_string());
            vec![match node {
                Some(node) => problem.at_node(node),
                None => problem,
            }]
        }
        IrError::EscapingBodyEdge { body, from, .. } => vec![
            Problem::error("UNKNOWN_TARGET", error.to_string())
                .at_node(from.to_string())
                .at_path(format!("/bodies/{body}")),
        ],
    }
}

/// The stable code of a graph rule and the node it names, if it names one.
fn graph_rule(error: &WorkflowError) -> (&'static str, Option<String>) {
    use WorkflowError as E;
    match error {
        E::InvalidDatabaseNode => ("INVALID_DATABASE_NODE", None),
        E::InvalidNodeId(id) => ("INVALID_NODE_ID", Some(id.clone())),
        E::InvalidActivityName(_) => ("INVALID_ACTIVITY_NAME", None),
        E::InvalidEventType(_) => ("INVALID_EVENT_TYPE", None),
        E::InvalidExpression(_) => ("INVALID_EXPRESSION", None),
        E::ExpressionByteLimit { node, .. } => ("EXPRESSION", Some(node.to_string())),
        E::NoNodes => ("NO_NODES", None),
        E::TooManyNodes(_) => ("TOO_MANY_NODES", None),
        E::TooManyParallels(_) => ("TOO_MANY_PARALLELS", None),
        E::TooManySecrets(_) => ("TOO_MANY_SECRETS", None),
        E::DuplicateNode(node) => ("DUPLICATE_NODE", Some(node.to_string())),
        E::UnknownEntrypoint(node) => ("UNKNOWN_ENTRYPOINT", Some(node.to_string())),
        E::UnknownTarget { from, .. } => ("UNKNOWN_TARGET", Some(from.to_string())),
        E::Cycle(node) => ("CYCLE", Some(node.to_string())),
        E::Unreachable(node) => ("UNREACHABLE", Some(node.to_string())),
        E::InvalidRetry(_) => ("INVALID_RETRY", None),
        E::DurationOutOfRange { node, .. } => ("DURATION_OUT_OF_RANGE", Some(node.to_string())),
        E::InvalidBranchCount(node) => ("INVALID_BRANCH_COUNT", Some(node.to_string())),
        E::BranchOverlap { parallel, .. } => ("BRANCH_OVERLAP", Some(parallel.to_string())),
        E::BranchBoundary { parallel, .. } => ("BRANCH_BOUNDARY", Some(parallel.to_string())),
        E::EndInsideBranch { parallel, .. } => ("END_INSIDE_BRANCH", Some(parallel.to_string())),
        E::SagaBoundary { saga, .. } => ("SAGA_BOUNDARY", Some(saga.to_string())),
        E::EndInsideSaga { saga, .. } => ("END_INSIDE_SAGA", Some(saga.to_string())),
        E::TooManySagas(_) => ("TOO_MANY_SAGAS", None),
        E::IrreversibleWithCompensation(node) => {
            ("IRREVERSIBLE_WITH_COMPENSATION", Some(node.to_string()))
        }
        E::EffectOnNonActivity(node) => ("EFFECT_ON_NON_ACTIVITY", Some(node.to_string())),
        E::InvalidTrigger(_) => ("INVALID_TRIGGER", None),
        E::TooManyTriggers(_) => ("TOO_MANY_TRIGGERS", None),
        E::DuplicateTrigger => ("DUPLICATE_TRIGGER", None),
        E::InvalidScope { node, .. } => ("INVALID_SCOPE", Some(node.to_string())),
        E::DuplicateBody(body) => ("DUPLICATE_BODY", Some(body.to_string())),
        E::UnknownBody { node, .. } => ("UNKNOWN_BODY", Some(node.to_string())),
        E::UnreachableBody(body) => ("UNREACHABLE_BODY", Some(body.to_string())),
        E::RecursiveBody(body) => ("RECURSIVE_BODY", Some(body.to_string())),
        E::BodyDepthExceeded(_) => ("BODY_DEPTH_EXCEEDED", None),
        E::InvalidWorkflowLimit(_) => ("INVALID_WORKFLOW_LIMIT", None),
        E::InvalidContinuationPolicy(_) => ("INVALID_CONTINUATION_POLICY", None),
        E::InvalidSwitch { node, .. } => ("INVALID_SWITCH", Some(node.to_string())),
    }
}

fn warning_problem(warning: &DefinitionWarning) -> Problem {
    let (code, node) = match warning {
        DefinitionWarning::CompensationOutsideSaga { node } => {
            ("COMPENSATION_OUTSIDE_SAGA", node.to_string())
        }
        DefinitionWarning::RecursiveChild { node } => ("RECURSIVE_CHILD", node.to_string()),
    };
    Problem {
        severity: Severity::Warning,
        code,
        message: warning.to_string(),
        node: Some(node),
        path: None,
    }
}

/// One side of a comparison, as the caller named it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRef {
    Draft,
    Version(u32),
}

/// A validated workflow document.
#[derive(Debug, Clone)]
pub struct ParsedDefinition {
    pub definition: WorkflowDefinition,
    /// The canonical JSON the digest is computed over; this is what a version stores.
    pub canonical: Vec<u8>,
    pub digest: Digest,
    pub warnings: Vec<DefinitionWarning>,
}

/// Parses, validates and canonicalises a workflow document (implemented by `sagat-infra-ir`).
pub trait IrCodec: Send + Sync {
    /// Cheap check that a draft is storable: well-formed JSON, an object, within
    /// [`MAX_IR_BYTES`]. Whether it is *publishable* is [`Self::parse`]'s job.
    ///
    /// # Errors
    /// [`IrError::TooLarge`] or [`IrError::Malformed`].
    fn check_draft(&self, document: &[u8]) -> Result<(), IrError>;

    /// # Errors
    /// An [`IrError`] describing why the document cannot be published.
    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError>;

    /// Restores the executable IR of an already persisted immutable version. Publication-time
    /// metadata checks must not reinterpret an older version's business execution. Syntax,
    /// structural limits, expressions and supported execution capabilities still fail closed.
    /// A codec with no distinct authoring-only rules uses its ordinary parser.
    ///
    /// # Errors
    /// An [`IrError`] for an unreadable/unsupported execution definition.
    fn parse_stored(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        self.parse(document)
    }

    /// Bounded authoring report, independent of tenant data and persistence.
    fn classification_analysis(
        &self,
        _document: &[u8],
    ) -> Option<crate::classification::ClassificationAnalysis> {
        None
    }

    /// Returns immutable declared-state metadata when the document contains that capability.
    /// Codecs that do not implement declared state remain closed and return no publication.
    ///
    /// # Errors
    /// An [`IrError`] describing an invalid declaration.
    fn declared_state_publication(
        &self,
        _document: &[u8],
    ) -> Result<Option<crate::declared_operations::PublishedDeclaredState>, IrError> {
        Ok(None)
    }

    /// Evaluates a schema-checked initial state from the exact input of a new run.
    /// Codecs that do not implement declared state return no state.
    ///
    /// # Errors
    /// A value-free evaluator error; raw workflow input is never included in the error.
    fn initial_declared_state(
        &self,
        _document: &[u8],
        _input: &[u8],
    ) -> Result<
        Option<crate::declared_operations::EvaluatedOperationWrite>,
        crate::operation_evaluation::OperationEvaluationError,
    > {
        Ok(None)
    }

    /// What differs between two documents, compared as written (so a draft that does not
    /// validate can be compared with the version it came from), both sides under the current
    /// classification rules.
    ///
    /// # Errors
    /// [`IrError::TooLarge`] or [`IrError::Malformed`] when a side is not a JSON object within
    /// [`MAX_IR_BYTES`].
    fn diff(&self, from: &[u8], to: &[u8]) -> Result<crate::diff::DocumentDiff, IrError>;

    /// [`IrCodec::diff`] told which classification rules each side was published under, so that a
    /// side under rules `0` (which discloses no value) is compared as such. An adapter that does
    /// not read classification annotations keeps the default, which ignores `rules`.
    ///
    /// # Errors
    /// As [`IrCodec::diff`].
    fn diff_with_rules(
        &self,
        from: &[u8],
        to: &[u8],
        _rules: crate::diff::DiffRules,
    ) -> Result<crate::diff::DocumentDiff, IrError> {
        self.diff(from, to)
    }
}

/// How an idempotent write is recognised: the key the client chose and the bytes that identify the
/// request. The adapter hashes `fingerprint`; it never leaves the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idempotency {
    pub scope: String,
    pub key: String,
    pub fingerprint: Vec<u8>,
}

/// Result of a write that may be a replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied<T> {
    /// This call performed the write.
    Done(T),
    /// An earlier call with the same key and request did; carries the resource id it recorded.
    Replayed(String),
}

/// A workflow together with its draft document and its active version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRecord {
    pub workflow: Workflow,
    /// The draft exactly as stored (any JSON object; validated only when published).
    pub draft: Vec<u8>,
    pub active: Option<WorkflowVersion>,
}

/// A published version with the canonical document it froze.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRecord {
    pub version: WorkflowVersion,
    pub document: Vec<u8>,
    pub release_notes: Option<String>,
}

/// The compensation plan of one saga as stored with a version (`FR-003-002`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPlan {
    pub saga: sagat_domain::workflow::NodeId,
    pub on_compensation_failure: sagat_domain::saga::OnCompensationFailure,
    pub cancel_policy: sagat_domain::saga::SagaCancelPolicy,
    /// Forward nodes whose effect cannot be undone.
    pub irreversible: Vec<sagat_domain::workflow::NodeId>,
    /// In an order a worker could follow one by one.
    pub entries: Vec<StoredPlanEntry>,
}

/// One compensation of a stored plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPlanEntry {
    pub node: sagat_domain::workflow::NodeId,
    /// Stable for the version and the node: the handle a provider can recognise across runs.
    pub compensation_id: uuid::Uuid,
    pub activity: sagat_domain::workflow::ActivityName,
    /// The forward nodes whose compensations must finish before this one starts.
    pub after: Vec<sagat_domain::workflow::NodeId>,
}

/// What a publication is on behalf of, recorded with the change of the active pointer it makes
/// (FR-011-005): a request that was approved, a version promoted from another namespace, the
/// reason given, and the script artifacts the document pins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublishContext {
    pub request: Option<sagat_domain::versioning::RequestId>,
    pub promoted_from: Option<WorkflowVersionId>,
    pub reason: Option<String>,
    pub scripts: Vec<sagat_domain::scripts::Digest>,
}

/// What publishing returned.
#[derive(Debug, Clone)]
pub struct Published {
    pub version: WorkflowVersion,
    /// False when the document was identical to an existing version, which is re-activated.
    pub created: bool,
    pub warnings: Vec<DefinitionWarning>,
}

/// Storage port for definitions. Implemented by `sagat-infra-postgres`.
///
/// Every method takes the tenant explicitly; adapters MUST apply it as a filter on every statement.
pub trait DefinitionStore: QuotaLookup + SecretAccessLookup + ScriptLookup + Send + Sync {
    /// Current connection/secret authority, for author feedback before publication. The publisher
    /// repeats the check in its transaction. Adapters without managed support fail closed.
    fn check_database_access(
        &self,
        _principal: &Principal,
        _namespace: NamespaceId,
        definition: &WorkflowDefinition,
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        async move {
            if definition
                .nodes()
                .any(|node| node.database_operation().is_some())
            {
                Err(StoreError::DatabaseUnavailable)
            } else {
                Ok(())
            }
        }
    }
    /// Resolves a namespace by slug inside `tenant`.
    fn find_namespace_by_slug(
        &self,
        tenant: TenantId,
        slug: &Slug,
    ) -> impl Future<Output = Result<Option<Namespace>, StoreError>> + Send;

    /// Persists the workflow, its first draft and the audit record in one transaction.
    fn create_workflow(
        &self,
        workflow: &Workflow,
        draft: &[u8],
        audit: &AuditRecord,
        idempotency: &Idempotency,
        max_in_namespace: u64,
    ) -> impl Future<Output = Result<Applied<()>, StoreError>> + Send;

    fn get_workflow(
        &self,
        tenant: TenantId,
        id: WorkflowId,
    ) -> impl Future<Output = Result<Option<WorkflowRecord>, StoreError>> + Send;

    /// A workflow by its name inside one namespace (names are unique there).
    fn find_workflow_by_name(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &Slug,
    ) -> impl Future<Output = Result<Option<WorkflowRecord>, StoreError>> + Send;

    /// Workflows of one namespace with `id > after`, ascending by id, at most `limit`.
    fn list_workflows(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        after: Option<WorkflowId>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<WorkflowRecord>, StoreError>> + Send;

    /// Replaces the draft if its revision is still `expected_revision` (compare-and-swap in one
    /// statement), bumping the revision. [`StoreError::RevisionConflict`] otherwise.
    fn update_draft(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        expected_revision: u64,
        draft: &[u8],
        audit: &AuditRecord,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<WorkflowRecord>, StoreError>> + Send;

    /// Publishes `parsed` as the next version if the draft is still at `draft_revision`, or
    /// re-activates the existing version with the same digest. Also stores the compensation `plans`
    /// of a new version with it, registers its `triggers` as the ones in force for the workflow
    /// (replacing those of the version it succeeds), audits and records `idempotency`, all in one
    /// transaction. `None` when the workflow does not exist.
    ///
    /// `context` says what the publication is on behalf of; the same transaction records the
    /// change of the active pointer (when it moves), the artifacts the version pins and, for a
    /// request, that it was published. A request is published only if it is approved and has the
    /// approvals the policy in force asks for: that is read again inside the transaction.
    #[allow(clippy::too_many_arguments)]
    fn publish(
        &self,
        principal: &Principal,
        id: WorkflowId,
        draft_revision: u64,
        version_id: WorkflowVersionId,
        digest: Digest,
        document: &[u8],
        declared_state: Option<&crate::declared_operations::PublishedDeclaredState>,
        definition: &WorkflowDefinition,
        release_notes: Option<&str>,
        plans: &[SagaPlan],
        triggers: &[TriggerRegistration],
        context: &PublishContext,
        audit: &AuditRecord,
        idempotency: &Idempotency,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<Applied<(WorkflowVersion, bool)>>, StoreError>> + Send;

    /// How many approvals a publication into the namespace needs now: what an administrator set,
    /// or the default of its environment. `None` when the namespace does not exist in `tenant`.
    fn publication_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<Option<sagat_domain::versioning::ApprovalPolicy>, StoreError>> + Send;

    fn get_version(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        number: u32,
    ) -> impl Future<Output = Result<Option<VersionRecord>, StoreError>> + Send;

    /// The compensation plans stored with a version, one per saga, in the order of the sagas.
    fn compensation_plans(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
    ) -> impl Future<Output = Result<Vec<StoredPlan>, StoreError>> + Send;

    /// A version by its identifier (runs pin the version they use by id).
    fn get_version_by_id(
        &self,
        tenant: TenantId,
        id: WorkflowVersionId,
    ) -> impl Future<Output = Result<Option<VersionRecord>, StoreError>> + Send;

    /// Versions of one workflow with `number > after`, ascending, at most `limit`.
    fn list_versions(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        after: Option<u32>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<VersionRecord>, StoreError>> + Send;

    /// Whether the workflow exists in a tenant *other than* `tenant` (decides whether a miss
    /// deserves a cross-tenant audit record; never returned to clients).
    fn workflow_exists_outside(
        &self,
        tenant: TenantId,
        id: WorkflowId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Best-effort, deduplicated cross-tenant denial record (see `IdentityStore::record_denial`).
    fn record_denial(
        &self,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// Definition use cases.
#[derive(Debug, Clone)]
pub struct DefinitionService<S, C, X> {
    store: S,
    clock: C,
    codec: X,
}

impl<S: DefinitionStore, C: Clock, X: IrCodec> DefinitionService<S, C, X> {
    pub const fn new(store: S, clock: C, codec: X) -> Self {
        Self {
            store,
            clock,
            codec,
        }
    }

    /// A draft is any JSON object up to [`MAX_IR_BYTES`]; full validation happens at publish.
    fn check_draft(&self, draft: &[u8]) -> Result<(), AppError> {
        self.codec
            .check_draft(draft)
            .map_err(|e| AppError::InvalidDefinition(Box::new(e)))
    }

    async fn namespace(&self, principal: &Principal, slug: &str) -> Result<Namespace, AppError> {
        // A malformed slug cannot match anything: it is "not found", like an unknown namespace.
        let slug = Slug::parse(slug).map_err(|_| AppError::NotFound)?;
        self.store
            .find_namespace_by_slug(principal.tenant_id(), &slug)
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Creates a workflow with its first draft.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:write`; [`AppError::NotFound`] for an unknown
    /// namespace; [`AppError::Invalid`] for a bad name; [`AppError::InvalidDefinition`] when the
    /// draft is not a JSON object or is too large; [`AppError::Conflict`] when the name is taken;
    /// [`AppError::IdempotencyKeyReused`].
    pub async fn create_workflow(
        &self,
        principal: &Principal,
        namespace: &str,
        name: &str,
        draft: &[u8],
        key: &str,
        ctx: &RequestContext,
    ) -> Result<WorkflowRecord, AppError> {
        principal.authorize(Permission::DefinitionWrite)?;
        self.check_draft(draft)?;
        let name = Slug::parse(name)?;
        let namespace = self.namespace(principal, namespace).await?;
        let now = self.clock.now();
        let workflow = Workflow::create(principal, &namespace, WorkflowId::generate(), name, now)?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::WorkflowCreated,
            ResourceType::Workflow,
            workflow.id().to_string(),
            ctx.trace_id.clone(),
            now,
        );
        let idempotency = Idempotency {
            scope: format!("workflow.create:{}", namespace.id()),
            key: key.to_owned(),
            fingerprint: fingerprint(&[workflow.name().as_str().as_bytes(), draft]),
        };
        let max_in_namespace = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(namespace.id()),
                QuotaResource::Workflows,
            )
            .await?;
        match self
            .store
            .create_workflow(&workflow, draft, &audit, &idempotency, max_in_namespace)
            .await?
        {
            Applied::Done(()) => Ok(WorkflowRecord {
                workflow,
                draft: draft.to_vec(),
                active: None,
            }),
            Applied::Replayed(id) => {
                let id = id.parse::<WorkflowId>().map_err(|_| AppError::Internal)?;
                self.store
                    .get_workflow(principal.tenant_id(), id)
                    .await?
                    .ok_or(AppError::Internal)
            }
        }
    }

    /// Replaces the draft if it is still at `expected_revision`.
    ///
    /// # Errors
    /// As [`Self::create_workflow`], plus [`AppError::NotFound`] for an unknown workflow and
    /// [`AppError::RevisionConflict`] when someone else changed the draft first.
    pub async fn update_draft(
        &self,
        principal: &Principal,
        id: WorkflowId,
        expected_revision: u64,
        draft: &[u8],
        ctx: &RequestContext,
    ) -> Result<WorkflowRecord, AppError> {
        principal.authorize(Permission::DefinitionWrite)?;
        self.check_draft(draft)?;
        let now = self.clock.now();
        let audit = AuditRecord::by(
            principal,
            AuditAction::WorkflowDraftUpdated,
            ResourceType::Workflow,
            id.to_string(),
            ctx.trace_id.clone(),
            now,
        );
        match self
            .store
            .update_draft(
                principal.tenant_id(),
                id,
                expected_revision,
                draft,
                &audit,
                now,
            )
            .await?
        {
            Some(record) => Ok(record),
            None => Err(self.missing(principal, id, ctx).await),
        }
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `definition:read`; [`AppError::NotFound`] when the workflow
    /// is not in the caller's tenant (a miss on another tenant's workflow is audited).
    pub async fn get_workflow(
        &self,
        principal: &Principal,
        id: WorkflowId,
        ctx: &RequestContext,
    ) -> Result<WorkflowRecord, AppError> {
        principal.authorize(Permission::DefinitionRead)?;
        match self.store.get_workflow(principal.tenant_id(), id).await? {
            Some(record) => Ok(record),
            None => Err(self.missing(principal, id, ctx).await),
        }
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `definition:read`; [`AppError::NotFound`] for an unknown
    /// namespace; [`AppError::Invalid`] for a bad cursor.
    pub async fn list_workflows(
        &self,
        principal: &Principal,
        namespace: &str,
        page: &PageRequest,
    ) -> Result<Page<WorkflowRecord>, AppError> {
        principal.authorize(Permission::DefinitionRead)?;
        let namespace = self.namespace(principal, namespace).await?;
        let after = page
            .cursor
            .as_deref()
            .map(str::parse::<WorkflowId>)
            .transpose()?;
        let limit = page.effective_limit();
        let rows = self
            .store
            .list_workflows(principal.tenant_id(), namespace.id(), after, limit + 1)
            .await?;
        Ok(paginate(rows, limit, |r| r.workflow.id().to_string()))
    }

    /// Publishes the draft as an immutable version and makes it active, into a namespace that asks
    /// for no approval.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:publish`; [`AppError::NotFound`];
    /// [`AppError::RevisionConflict`] when the draft moved on; [`AppError::InvalidDefinition`]
    /// when the document cannot be published; [`AppError::IdempotencyKeyReused`];
    /// [`AppError::Versioning`] with `ApprovalRequired` when the namespace asks for approvals
    /// (the draft goes through a publication request).
    pub async fn publish(
        &self,
        principal: &Principal,
        id: WorkflowId,
        draft_revision: u64,
        release_notes: Option<&str>,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<Published, AppError> {
        principal.authorize(Permission::DefinitionPublish)?;
        let record = self.get_workflow(principal, id, ctx).await?;
        let policy = self
            .store
            .publication_policy(principal.tenant_id(), record.workflow.namespace_id())
            .await?
            .ok_or(AppError::NotFound)?;
        if policy.asks() {
            return Err(AppError::Versioning(
                crate::error::Refusal::ApprovalRequired {
                    required: policy.required(),
                },
            ));
        }
        self.publish_through(
            principal,
            id,
            draft_revision,
            release_notes,
            key,
            ctx,
            PublishContext::default(),
        )
        .await
    }

    /// Checks that the draft is publishable as it stands at `draft_revision`: the revision, the
    /// schema and the graph, the secrets the caller may use and the scripts that were built. What
    /// publishing and opening a request both stand on.
    ///
    /// # Errors
    /// As [`Self::publish`], without the policy.
    pub(crate) async fn prepare(
        &self,
        principal: &Principal,
        id: WorkflowId,
        draft_revision: u64,
        ctx: &RequestContext,
    ) -> Result<(WorkflowRecord, ParsedDefinition), AppError> {
        let record = self.get_workflow(principal, id, ctx).await?;
        if record.workflow.draft_revision() != draft_revision {
            return Err(AppError::RevisionConflict {
                current: record.workflow.draft_revision(),
            });
        }
        let parsed = self
            .codec
            .parse(&record.draft)
            .map_err(|e| AppError::InvalidDefinition(Box::new(e)))?;
        if parsed.definition.name() != record.workflow.name() {
            return Err(AppError::InvalidDefinition(Box::new(
                IrError::NameMismatch {
                    document: parsed.definition.name().as_str().to_owned(),
                    workflow: record.workflow.name().as_str().to_owned(),
                },
            )));
        }
        self.check_secrets(principal, &record, &parsed.definition, ctx)
            .await?;
        let database_check = self
            .store
            .check_database_access(
                principal,
                record.workflow.namespace_id(),
                &parsed.definition,
            )
            .await;
        if let Err(error) = database_check {
            self.audit_database_denial(principal, id, &error, ctx).await;
            return Err(error.into());
        }
        self.check_scripts(principal, &parsed.definition).await?;
        Ok((record, parsed))
    }

    /// Publishes without asking the policy: the caller has done what the policy asks (an approved
    /// request, a promotion that needed none). The store checks a request again inside the
    /// transaction.
    ///
    /// # Errors
    /// As [`Self::publish`], without the policy.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn publish_through(
        &self,
        principal: &Principal,
        id: WorkflowId,
        draft_revision: u64,
        release_notes: Option<&str>,
        key: &str,
        ctx: &RequestContext,
        mut context: PublishContext,
    ) -> Result<Published, AppError> {
        principal.authorize(Permission::DefinitionPublish)?;
        check_release_notes(release_notes)?;
        let (_, parsed) = self.prepare(principal, id, draft_revision, ctx).await?;
        let declared_state = self
            .codec
            .declared_state_publication(&parsed.canonical)
            .map_err(|error| AppError::InvalidDefinition(Box::new(error)))?;
        context.scripts = parsed.definition.script_digests().into_iter().collect();
        if context.reason.is_none() {
            context.reason = release_notes.map(str::to_owned);
        }
        let now = self.clock.now();
        let version_id = WorkflowVersionId::generate();
        let audit = AuditRecord::by(
            principal,
            AuditAction::WorkflowPublished,
            ResourceType::WorkflowVersion,
            version_id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(match context.request {
            Some(request) => format!("digest {}, by request {request}", parsed.digest),
            None => format!("digest {}", parsed.digest),
        });
        let idempotency =
            publication_idempotency(id, draft_revision, parsed.digest, release_notes, key);
        let outcome = self
            .store
            .publish(
                principal,
                id,
                draft_revision,
                version_id,
                parsed.digest,
                &parsed.canonical,
                declared_state.as_ref(),
                &parsed.definition,
                release_notes,
                &compensation_plans(&parsed.definition),
                &triggers_of(id, &parsed.definition),
                &context,
                &audit,
                &idempotency,
                now,
            )
            .await;
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                self.audit_database_denial(principal, id, &error, ctx).await;
                return Err(error.into());
            }
        };
        match outcome {
            Some(Applied::Done((version, created))) => Ok(Published {
                version,
                created,
                warnings: parsed.warnings,
            }),
            Some(Applied::Replayed(version)) => {
                let number = version.parse::<u32>().map_err(|_| AppError::Internal)?;
                let stored = self
                    .store
                    .get_version(principal.tenant_id(), id, number)
                    .await?
                    .ok_or(AppError::Internal)?;
                Ok(Published {
                    version: stored.version,
                    created: false,
                    warnings: parsed.warnings,
                })
            }
            None => Err(self.missing(principal, id, ctx).await),
        }
    }

    /// A dry run of what [`Self::publish`] checks: the document given (or, with none, the stored
    /// draft) against the schema, the graph rules, the expressions, the secrets the caller may use
    /// and the scripts that were built. Nothing is stored, nothing is audited, and a document that
    /// does not validate is an answer, not an error.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:write`; [`AppError::NotFound`] for an unknown
    /// workflow (another tenant's is also not found, and audited).
    pub async fn validate(
        &self,
        principal: &Principal,
        id: WorkflowId,
        document: Option<&[u8]>,
        ctx: &RequestContext,
    ) -> Result<Validation, AppError> {
        principal.authorize(Permission::DefinitionWrite)?;
        let record = self.get_workflow(principal, id, ctx).await?;
        let bytes = document.unwrap_or(&record.draft);
        let classification = self.codec.classification_analysis(bytes);
        let classification_warnings = classification.as_ref().map_or_else(Vec::new, |analysis| {
            analysis
                .findings
                .iter()
                .filter(|finding| {
                    finding.code.severity() == crate::classification::FindingSeverity::Warning
                })
                .map(|finding| Problem {
                    severity: Severity::Warning,
                    code: finding.code.as_str(),
                    message: finding.code.message().to_owned(),
                    node: None,
                    path: Some(finding.path.as_str().to_owned()),
                })
                .collect::<Vec<_>>()
        });
        let parsed = match self.codec.parse(bytes) {
            Ok(parsed) => parsed,
            Err(error) => {
                return Ok(Validation {
                    problems: problems_of(&error),
                    warnings: classification_warnings,
                    classification,
                    ..Validation::default()
                });
            }
        };
        let mut problems = Vec::new();
        if parsed.definition.name() != record.workflow.name() {
            problems.extend(problems_of(&IrError::NameMismatch {
                document: parsed.definition.name().as_str().to_owned(),
                workflow: record.workflow.name().as_str().to_owned(),
            }));
        }
        problems.extend(
            self.unusable_secrets(principal, &record, &parsed.definition)
                .await?,
        );
        problems.extend(self.unknown_scripts(principal, &parsed.definition).await?);
        match self
            .store
            .check_database_access(
                principal,
                record.workflow.namespace_id(),
                &parsed.definition,
            )
            .await
        {
            Ok(()) => {}
            Err(StoreError::DatabaseUnavailable) => problems.push(Problem::error(
                "DATABASE_UNAVAILABLE",
                "a database connection is inactive, unavailable or may not be used",
            )),
            Err(StoreError::DatabaseLimit) => problems.push(Problem::error(
                "DATABASE_LIMIT_EXCEEDED",
                "a database operation exceeds connection limits",
            )),
            Err(error) => return Err(error.into()),
        }
        let valid = problems.is_empty();
        let mut warnings: Vec<_> = parsed.warnings.iter().map(warning_problem).collect();
        warnings.extend(classification_warnings);
        Ok(Validation {
            problems,
            warnings,
            digest: valid.then(|| parsed.digest.to_string()),
            classification,
        })
    }

    /// Compares two documents of a workflow, the draft or a published version each, so an author
    /// sees what changed, what breaks callers and what somebody must prepare before publishing.
    /// By default the active version is compared with the draft; a workflow that has no version
    /// compares an empty document with its draft.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:read`; [`AppError::NotFound`] for an unknown
    /// workflow or version; [`AppError::InvalidDefinition`] when a side is not a JSON object.
    pub async fn diff(
        &self,
        principal: &Principal,
        id: WorkflowId,
        from: Option<DiffRef>,
        to: Option<DiffRef>,
        ctx: &RequestContext,
    ) -> Result<crate::diff::DefinitionDiff, AppError> {
        use crate::diff::{DefinitionDiff, DiffSide};
        principal.authorize(Permission::DefinitionRead)?;
        let record = self.get_workflow(principal, id, ctx).await?;
        let side = |reference: Option<DiffRef>, default: Option<DiffRef>| reference.or(default);
        let default_from = record
            .active
            .as_ref()
            .map(|version| DiffRef::Version(version.number()));
        let from = side(from, default_from);
        let to = side(to, Some(DiffRef::Draft));
        // A draft is published under the current rules; a version under the ones it pinned.
        let resolve = async |reference: Option<DiffRef>| -> Result<
            (DiffSide, Vec<u8>, ClassificationRules),
            AppError,
        > {
            match reference {
                None => Ok((
                    DiffSide::Empty,
                    b"{}".to_vec(),
                    ClassificationRules::CURRENT,
                )),
                Some(DiffRef::Draft) => Ok((
                    DiffSide::Draft(record.workflow.draft_revision()),
                    record.draft.clone(),
                    ClassificationRules::CURRENT,
                )),
                Some(DiffRef::Version(number)) => {
                    let version = self
                        .store
                        .get_version(principal.tenant_id(), id, number)
                        .await?
                        .ok_or(AppError::NotFound)?;
                    Ok((
                        DiffSide::Version(number),
                        version.document,
                        version.version.classification_rules(),
                    ))
                }
            }
        };
        let (from_side, from_bytes, from_rules) = resolve(from).await?;
        let (to_side, to_bytes, to_rules) = resolve(to).await?;
        let diff = self
            .codec
            .diff_with_rules(
                &from_bytes,
                &to_bytes,
                crate::diff::DiffRules {
                    from: from_rules,
                    to: to_rules,
                },
            )
            .map_err(|e| AppError::InvalidDefinition(Box::new(e)))?;
        Ok(DefinitionDiff {
            from: from_side,
            to: to_side,
            diff,
        })
    }

    /// The declared secrets the caller may not use, as problems (the same answer for a secret
    /// that does not exist). Quiet: a dry run records no denial.
    async fn unusable_secrets(
        &self,
        principal: &Principal,
        record: &WorkflowRecord,
        definition: &WorkflowDefinition,
    ) -> Result<Vec<Problem>, AppError> {
        let declared: Vec<_> = definition.declared_secrets().into_iter().cloned().collect();
        if declared.is_empty() {
            return Ok(Vec::new());
        }
        let states = self
            .store
            .secret_states(
                principal.tenant_id(),
                record.workflow.namespace_id(),
                &declared,
                principal.subject().as_str(),
            )
            .await?;
        let mut problems = Vec::new();
        for (name, state) in states {
            if check_usable(Some(principal.role()), &[(name.clone(), state)]).is_ok() {
                continue;
            }
            for node in definition
                .nodes()
                .filter(|node| node.secrets().contains(&name))
            {
                problems.push(
                    Problem::error(
                        "SECRET_UNAVAILABLE",
                        format!("the secret '{name}' does not exist or you may not use it"),
                    )
                    .at_node(node.id().as_str()),
                );
            }
        }
        Ok(problems)
    }

    /// The nodes that pin an artifact no build of this tenant produced.
    async fn unknown_scripts(
        &self,
        principal: &Principal,
        definition: &WorkflowDefinition,
    ) -> Result<Vec<Problem>, AppError> {
        let pinned: Vec<_> = definition.script_digests().into_iter().collect();
        if pinned.is_empty() {
            return Ok(Vec::new());
        }
        let missing = self
            .store
            .missing_artifacts(principal.tenant_id(), &pinned)
            .await?;
        Ok(unknown_script_problems(definition, &missing))
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `definition:read`; [`AppError::NotFound`].
    pub async fn get_version(
        &self,
        principal: &Principal,
        id: WorkflowId,
        number: u32,
        ctx: &RequestContext,
    ) -> Result<VersionRecord, AppError> {
        principal.authorize(Permission::DefinitionRead)?;
        match self
            .store
            .get_version(principal.tenant_id(), id, number)
            .await?
        {
            Some(record) => Ok(record),
            None => Err(self.missing(principal, id, ctx).await),
        }
    }

    /// The compensation plans a version was published with: what each saga undoes, in which
    /// order, and the durable id of every compensation.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:read`; [`AppError::NotFound`].
    pub async fn compensation_plans(
        &self,
        principal: &Principal,
        id: WorkflowId,
        number: u32,
        ctx: &RequestContext,
    ) -> Result<Vec<StoredPlan>, AppError> {
        let record = self.get_version(principal, id, number, ctx).await?;
        Ok(self
            .store
            .compensation_plans(principal.tenant_id(), record.version.id())
            .await?)
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `definition:read`; [`AppError::NotFound`];
    /// [`AppError::Invalid`] for a bad cursor.
    pub async fn list_versions(
        &self,
        principal: &Principal,
        id: WorkflowId,
        page: &PageRequest,
        ctx: &RequestContext,
    ) -> Result<Page<VersionRecord>, AppError> {
        principal.authorize(Permission::DefinitionRead)?;
        // Existence (and the cross-tenant audit) first, so an empty list is not ambiguous.
        self.get_workflow(principal, id, ctx).await?;
        let after = page
            .cursor
            .as_deref()
            .map(|c| c.parse::<u32>().map_err(|_| AppError::NotFound))
            .transpose()?;
        let limit = page.effective_limit();
        let rows = self
            .store
            .list_versions(principal.tenant_id(), id, after, limit + 1)
            .await?;
        Ok(paginate(rows, limit, |r| r.version.number().to_string()))
    }

    pub(crate) async fn audit_database_denial(
        &self,
        principal: &Principal,
        workflow: WorkflowId,
        error: &StoreError,
        ctx: &RequestContext,
    ) {
        if !matches!(
            error,
            StoreError::DatabaseUnavailable | StoreError::DatabaseLimit
        ) {
            return;
        }
        let audit = AuditRecord::by(
            principal,
            AuditAction::DatabaseDenied,
            ResourceType::Workflow,
            workflow.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason("database publication refused".to_owned());
        if let Err(error) = self.store.record_denial(&audit).await {
            tracing::error!(%error, "failed to record database denial");
        }
    }

    /// A workflow that declares secrets can be published only by someone who may use every one of
    /// them (FR-009-002, US02). The answer for a secret that does not exist is the same as for one
    /// the publisher may not use.
    async fn check_secrets(
        &self,
        principal: &Principal,
        record: &WorkflowRecord,
        definition: &sagat_domain::workflow::WorkflowDefinition,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        let declared: Vec<_> = definition.declared_secrets().into_iter().cloned().collect();
        if declared.is_empty() {
            return Ok(());
        }
        let states = self
            .store
            .secret_states(
                principal.tenant_id(),
                record.workflow.namespace_id(),
                &declared,
                principal.subject().as_str(),
            )
            .await?;
        if let Err(SecretProblem(name)) = check_usable(Some(principal.role()), &states) {
            let denial = AuditRecord::by(
                principal,
                AuditAction::SecretDenied,
                ResourceType::Secret,
                format!("{}/{name}", record.workflow.namespace_id()),
                ctx.trace_id.clone(),
                self.clock.now(),
            )
            .with_reason(format!("publishing {}", record.workflow.name()));
            // Recorded off the response path and collapsed when repeated.
            if let Err(error) = self.store.record_denial(&denial).await {
                tracing::error!(%error, action = %denial.action(), "failed to record audit entry");
            }
            return Err(AppError::SecretUnavailable {
                name: name.to_string(),
            });
        }
        Ok(())
    }

    /// A workflow that pins a script can be published only if the script was built for this tenant:
    /// a digest nobody built (a typo, a build that failed, another tenant's artifact) is refused
    /// now, and not when the first run reaches the step.
    async fn check_scripts(
        &self,
        principal: &Principal,
        definition: &sagat_domain::workflow::WorkflowDefinition,
    ) -> Result<(), AppError> {
        let pinned: Vec<_> = definition.script_digests().into_iter().collect();
        if pinned.is_empty() {
            return Ok(());
        }
        let missing = self
            .store
            .missing_artifacts(principal.tenant_id(), &pinned)
            .await?;
        match missing.first() {
            None => Ok(()),
            Some(digest) => Err(AppError::InvalidDefinition(Box::new(
                IrError::UnknownScript {
                    digest: digest.to_string(),
                },
            ))),
        }
    }

    async fn missing(
        &self,
        principal: &Principal,
        id: WorkflowId,
        ctx: &RequestContext,
    ) -> AppError {
        if let Ok(true) = self
            .store
            .workflow_exists_outside(principal.tenant_id(), id)
            .await
        {
            let record = AuditRecord::by(
                principal,
                AuditAction::CrossTenantAccessDenied,
                ResourceType::Workflow,
                id.to_string(),
                ctx.trace_id.clone(),
                self.clock.now(),
            )
            .with_reason("resource not accessible from this tenant");
            if let Err(error) = self.store.record_denial(&record).await {
                tracing::error!(%error, action = %record.action(), "failed to record audit entry");
            }
        }
        AppError::NotFound
    }
}

/// The identity of a publication, also available from an immutable publication request after
/// its draft has changed. Keep writes and recovery on the same fingerprint.
pub(crate) fn publication_idempotency(
    id: WorkflowId,
    revision: u64,
    digest: Digest,
    notes: Option<&str>,
    key: &str,
) -> Idempotency {
    Idempotency {
        scope: format!("workflow.publish:{id}"),
        key: key.to_owned(),
        fingerprint: fingerprint(&[
            &revision.to_be_bytes(),
            digest.as_bytes(),
            notes.unwrap_or_default().as_bytes(),
        ]),
    }
}

/// Length-prefixed concatenation, so `("ab", "c")` and `("a", "bc")` differ.
pub(crate) fn fingerprint(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend_from_slice(&(part.len() as u64).to_be_bytes());
        out.extend_from_slice(part);
    }
    out
}

fn paginate<T>(mut rows: Vec<T>, limit: u32, id_of: impl Fn(&T) -> String) -> Page<T> {
    let limit = limit as usize;
    if rows.len() > limit {
        rows.truncate(limit);
        let next_cursor = rows.last().map(id_of);
        Page {
            items: rows,
            next_cursor,
        }
    } else {
        Page {
            items: rows,
            next_cursor: None,
        }
    }
}

/// Minimal check used by callers that already hold a validated definition.
#[must_use]
pub fn warnings_text(warnings: &[DefinitionWarning]) -> Vec<String> {
    warnings.iter().map(ToString::to_string).collect()
}
