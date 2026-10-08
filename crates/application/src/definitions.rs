//! Workflow definition use cases: drafts, publication and immutable versions.
//!
//! Every use case receives an authenticated [`Principal`], authorizes before touching storage and
//! derives the tenant from it. Writes are audited in the same transaction as the change, and the
//! `Idempotency-Key` check is atomic with the write (the store receives the key and a fingerprint of
//! the request; a repeat returns the original result, a different request under the same key is a
//! conflict).

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{Workflow, WorkflowId, WorkflowVersion, WorkflowVersionId};
use sagat_domain::identity::{Namespace, NamespaceId, Permission, Principal, Slug, TenantId};
use sagat_domain::workflow::{DefinitionWarning, Digest, WorkflowDefinition, WorkflowError};
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::{Page, PageRequest, RequestContext};

/// Largest accepted workflow document.
pub const MAX_IR_BYTES: usize = 1024 * 1024;
/// Longest release note.
pub const MAX_RELEASE_NOTES: usize = 2048;

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
    /// A node kind that exists in the schema but that this version of the engine does not run.
    #[error("node '{node}' is a '{kind}', which is not supported yet: {reason}")]
    Unsupported {
        node: String,
        kind: String,
        reason: &'static str,
    },
    #[error("{0}")]
    Invalid(WorkflowError),
    #[error("node '{node}', field '{field}': {message}")]
    Expression {
        node: String,
        field: &'static str,
        message: String,
    },
    #[error("the document names the workflow '{document}' but this workflow is '{workflow}'")]
    NameMismatch { document: String, workflow: String },
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
    /// # Errors
    /// An [`IrError`] describing why the document cannot be published.
    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError>;
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
pub trait DefinitionStore: Send + Sync {
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
    ) -> impl Future<Output = Result<Applied<()>, StoreError>> + Send;

    fn get_workflow(
        &self,
        tenant: TenantId,
        id: WorkflowId,
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
    /// re-activates the existing version with the same digest. Also audits and records
    /// `idempotency`, all in one transaction. `None` when the workflow does not exist.
    #[allow(clippy::too_many_arguments)]
    fn publish(
        &self,
        principal: &Principal,
        id: WorkflowId,
        draft_revision: u64,
        version_id: WorkflowVersionId,
        digest: Digest,
        document: &[u8],
        release_notes: Option<&str>,
        audit: &AuditRecord,
        idempotency: &Idempotency,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<Applied<(WorkflowVersion, bool)>>, StoreError>> + Send;

    fn get_version(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        number: u32,
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
        check_draft(draft)?;
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
        match self
            .store
            .create_workflow(&workflow, draft, &audit, &idempotency)
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
        check_draft(draft)?;
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

    /// Publishes the draft as an immutable version and makes it active.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:publish`; [`AppError::NotFound`];
    /// [`AppError::RevisionConflict`] when the draft moved on; [`AppError::InvalidDefinition`]
    /// when the document cannot be published; [`AppError::IdempotencyKeyReused`].
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
        if release_notes.is_some_and(|n| n.len() > MAX_RELEASE_NOTES) {
            return Err(AppError::NotAllowed("release notes are too long"));
        }
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
        .with_reason(format!("digest {}", parsed.digest));
        let idempotency = Idempotency {
            scope: format!("workflow.publish:{id}"),
            key: key.to_owned(),
            fingerprint: fingerprint(&[
                &draft_revision.to_be_bytes(),
                parsed.digest.as_bytes(),
                release_notes.unwrap_or_default().as_bytes(),
            ]),
        };
        let outcome = self
            .store
            .publish(
                principal,
                id,
                draft_revision,
                version_id,
                parsed.digest,
                &parsed.canonical,
                release_notes,
                &audit,
                &idempotency,
                now,
            )
            .await?;
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

/// A draft is any JSON object up to [`MAX_IR_BYTES`]; full validation happens at publish.
fn check_draft(draft: &[u8]) -> Result<(), AppError> {
    if draft.len() > MAX_IR_BYTES {
        return Err(AppError::InvalidDefinition(Box::new(IrError::TooLarge)));
    }
    if draft.first() != Some(&b'{') {
        return Err(AppError::InvalidDefinition(Box::new(IrError::Malformed)));
    }
    Ok(())
}

/// Length-prefixed concatenation, so `("ab", "c")` and `("a", "bc")` differ.
fn fingerprint(parts: &[&[u8]]) -> Vec<u8> {
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
