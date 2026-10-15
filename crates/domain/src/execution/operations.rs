//! Pure state transitions for declared Query and Update operations.
//!
//! Values are opaque, content-addressed references. Schema validation, expression evaluation,
//! authorization, auditing and persistence stay at their boundaries; this module records only the
//! deterministic revision and receipt decision that those boundaries commit atomically.

use std::collections::BTreeMap;

use crate::identity::{NamespaceId, Subject, TenantId};
use crate::workflow::{ActivityName, Digest};

use super::{RunId, RunStatus};

pub const MAX_OPERATION_NAME_BYTES: usize = 128;
pub const MAX_UPDATE_KEY_BYTES: usize = 128;
pub const MAX_OPERATION_VALUE_BYTES: u32 = 256 * 1024;

/// The only disclosure declaration accepted for a named business operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultDisclosure {
    DeclaredBusinessResult,
}

impl ResultDisclosure {
    pub const DECLARED_BUSINESS_RESULT: &'static str = "declared-business-result-v1";

    /// # Errors
    /// Returns [`OperationError::UnsupportedDisclosure`] for every implicit or unknown policy.
    pub fn parse(value: &str) -> Result<Self, OperationError> {
        if value == Self::DECLARED_BUSINESS_RESULT {
            Ok(Self::DeclaredBusinessResult)
        } else {
            Err(OperationError::UnsupportedDisclosure)
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DeclaredBusinessResult => Self::DECLARED_BUSINESS_RESULT,
        }
    }
}

/// Caller-chosen idempotency key of an Update.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UpdateKey(String);

impl UpdateKey {
    /// # Errors
    /// Returns [`OperationError::InvalidUpdateKey`] unless the key is bounded printable ASCII
    /// without spaces.
    pub fn parse(value: &str) -> Result<Self, OperationError> {
        if value.is_empty()
            || value.len() > MAX_UPDATE_KEY_BYTES
            || !value.chars().all(|character| character.is_ascii_graphic())
        {
            Err(OperationError::InvalidUpdateKey)
        } else {
            Ok(Self(value.to_owned()))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque reference to a schema-checked state or operation result retained by an adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationValue {
    digest: Digest,
    schema_digest: Digest,
    size_bytes: u32,
}

impl OperationValue {
    /// # Errors
    /// Returns [`OperationError::ValueTooLarge`] when the serialized value exceeds 256 KiB.
    pub const fn new(
        digest: Digest,
        schema_digest: Digest,
        size_bytes: u32,
    ) -> Result<Self, OperationError> {
        if size_bytes > MAX_OPERATION_VALUE_BYTES {
            Err(OperationError::ValueTooLarge)
        } else {
            Ok(Self {
                digest,
                schema_digest,
                size_bytes,
            })
        }
    }

    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    #[must_use]
    pub const fn schema_digest(&self) -> Digest {
        self.schema_digest
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u32 {
        self.size_bytes
    }
}

/// Whether an operation addresses one exact run or a chain whose head is resolved transactionally.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperationAddress {
    Run(RunId),
    Chain(RunId),
}

/// Tenant-scoped coordinates included in the durable receipt identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationScope {
    tenant_id: TenantId,
    namespace_id: NamespaceId,
    address: OperationAddress,
}

impl OperationScope {
    #[must_use]
    pub const fn new(
        tenant_id: TenantId,
        namespace_id: NamespaceId,
        address: OperationAddress,
    ) -> Self {
        Self {
            tenant_id,
            namespace_id,
            address,
        }
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn namespace_id(&self) -> NamespaceId {
        self.namespace_id
    }

    #[must_use]
    pub const fn address(&self) -> &OperationAddress {
        &self.address
    }
}

/// One committed state snapshot observed by a Query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuerySnapshot {
    run_id: RunId,
    run_version: u64,
    state_revision: u64,
    state: OperationValue,
}

impl QuerySnapshot {
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn run_version(&self) -> u64 {
        self.run_version
    }

    #[must_use]
    pub const fn state_revision(&self) -> u64 {
        self.state_revision
    }

    #[must_use]
    pub const fn state(&self) -> &OperationValue {
        &self.state
    }
}

/// Declared Query result paired with the exact snapshot metadata it observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryResponse {
    snapshot: QuerySnapshot,
    result: OperationValue,
}

impl QueryResponse {
    #[must_use]
    pub const fn new(snapshot: QuerySnapshot, result: OperationValue) -> Self {
        Self { snapshot, result }
    }

    #[must_use]
    pub const fn snapshot(&self) -> &QuerySnapshot {
        &self.snapshot
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.snapshot.run_id
    }

    #[must_use]
    pub const fn run_version(&self) -> u64 {
        self.snapshot.run_version
    }

    #[must_use]
    pub const fn state_revision(&self) -> u64 {
        self.snapshot.state_revision
    }

    #[must_use]
    pub const fn result(&self) -> &OperationValue {
        &self.result
    }
}

/// Immutable input used to identify, authorize and deduplicate one Update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRequest {
    scope: OperationScope,
    resolved_run_id: RunId,
    operation: ActivityName,
    update_key: UpdateKey,
    request_hash: Digest,
    expected_state_revision: Option<u64>,
    actor: Subject,
}

impl UpdateRequest {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        scope: OperationScope,
        resolved_run_id: RunId,
        operation: ActivityName,
        update_key: UpdateKey,
        request_hash: Digest,
        expected_state_revision: Option<u64>,
        actor: Subject,
    ) -> Self {
        Self {
            scope,
            resolved_run_id,
            operation,
            update_key,
            request_hash,
            expected_state_revision,
            actor,
        }
    }

    #[must_use]
    pub const fn scope(&self) -> &OperationScope {
        &self.scope
    }

    #[must_use]
    pub const fn resolved_run_id(&self) -> RunId {
        self.resolved_run_id
    }

    #[must_use]
    pub const fn operation(&self) -> &ActivityName {
        &self.operation
    }

    #[must_use]
    pub const fn update_key(&self) -> &UpdateKey {
        &self.update_key
    }

    #[must_use]
    pub const fn request_hash(&self) -> Digest {
        self.request_hash
    }

    #[must_use]
    pub const fn expected_state_revision(&self) -> Option<u64> {
        self.expected_state_revision
    }

    #[must_use]
    pub const fn actor(&self) -> &Subject {
        &self.actor
    }
}

/// A business outcome that may be retained as a rejected receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateRejection {
    RevisionConflict { expected: u64, actual: u64 },
    ValidatorRejected,
}

impl UpdateRejection {
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::RevisionConflict { .. } => "REVISION_CONFLICT",
            Self::ValidatorRejected => "UPDATE_REJECTED",
        }
    }
}

/// Schema-checked evaluator output. It can only apply a value or reject the business decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateEvaluation {
    Applied {
        new_state: OperationValue,
        result: OperationValue,
    },
    Rejected(UpdateRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateReceiptStatus {
    Applied,
    Rejected,
}

/// Durable result of one admitted Update request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReceipt {
    scope: OperationScope,
    resolved_run_id: RunId,
    operation: ActivityName,
    update_key: UpdateKey,
    request_hash: Digest,
    status: UpdateReceiptStatus,
    rejection: Option<UpdateRejection>,
    result: Option<OperationValue>,
    before_state_revision: u64,
    after_state_revision: u64,
    run_version: u64,
    actor: Subject,
}

impl UpdateReceipt {
    /// Rebuilds a receipt from trusted storage after checking its durable invariants.
    ///
    /// # Errors
    /// Returns [`OperationError::InvalidStoredOperation`] when status, result, rejection,
    /// revisions or the resolved run contradict each other.
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        scope: OperationScope,
        resolved_run_id: RunId,
        operation: ActivityName,
        update_key: UpdateKey,
        request_hash: Digest,
        status: UpdateReceiptStatus,
        rejection: Option<UpdateRejection>,
        result: Option<OperationValue>,
        before_state_revision: u64,
        after_state_revision: u64,
        run_version: u64,
        actor: Subject,
    ) -> Result<Self, OperationError> {
        let receipt = Self {
            scope,
            resolved_run_id,
            operation,
            update_key,
            request_hash,
            status,
            rejection,
            result,
            before_state_revision,
            after_state_revision,
            run_version,
            actor,
        };
        receipt.validate_stored()?;
        Ok(receipt)
    }

    fn validate_stored(&self) -> Result<(), OperationError> {
        if matches!(self.scope.address, OperationAddress::Run(run) if run != self.resolved_run_id) {
            return Err(OperationError::InvalidStoredOperation);
        }
        let valid = match self.status {
            UpdateReceiptStatus::Applied => {
                self.rejection.is_none()
                    && self.result.is_some()
                    && self.before_state_revision.checked_add(1) == Some(self.after_state_revision)
            }
            UpdateReceiptStatus::Rejected => {
                self.rejection.is_some()
                    && self.result.is_none()
                    && self.before_state_revision == self.after_state_revision
            }
        };
        if valid {
            Ok(())
        } else {
            Err(OperationError::InvalidStoredOperation)
        }
    }

    #[must_use]
    pub const fn scope(&self) -> &OperationScope {
        &self.scope
    }

    #[must_use]
    pub const fn resolved_run_id(&self) -> RunId {
        self.resolved_run_id
    }

    #[must_use]
    pub const fn operation(&self) -> &ActivityName {
        &self.operation
    }

    #[must_use]
    pub const fn update_key(&self) -> &UpdateKey {
        &self.update_key
    }

    #[must_use]
    pub const fn request_hash(&self) -> Digest {
        self.request_hash
    }

    #[must_use]
    pub const fn status(&self) -> UpdateReceiptStatus {
        self.status
    }

    #[must_use]
    pub const fn rejection(&self) -> Option<&UpdateRejection> {
        self.rejection.as_ref()
    }

    #[must_use]
    pub const fn result(&self) -> Option<&OperationValue> {
        self.result.as_ref()
    }

    #[must_use]
    pub const fn before_state_revision(&self) -> u64 {
        self.before_state_revision
    }

    #[must_use]
    pub const fn after_state_revision(&self) -> u64 {
        self.after_state_revision
    }

    #[must_use]
    pub const fn run_version(&self) -> u64 {
        self.run_version
    }

    #[must_use]
    pub const fn actor(&self) -> &Subject {
        &self.actor
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ReceiptIdentity {
    scope: OperationScope,
    update_key: UpdateKey,
}

/// The result of resolving an Update, distinguishing a stored replay from a new decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateResolution {
    receipt: UpdateReceipt,
    replayed: bool,
}

impl UpdateResolution {
    #[must_use]
    pub const fn receipt(&self) -> &UpdateReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn replayed(&self) -> bool {
        self.replayed
    }
}

/// Business state and committed receipts for one serialized operation aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredOperationsState {
    state: OperationValue,
    state_revision: u64,
    last_changed_run_version: u64,
    receipts: BTreeMap<ReceiptIdentity, UpdateReceipt>,
}

impl DeclaredOperationsState {
    /// Initializes declared state at revision zero in the run-start transaction.
    #[must_use]
    pub const fn new(state: OperationValue, run_version: u64) -> Self {
        Self {
            state,
            state_revision: 0,
            last_changed_run_version: run_version,
            receipts: BTreeMap::new(),
        }
    }

    /// Rebuilds state from a projection and its retained receipts.
    ///
    /// # Errors
    /// Returns [`OperationError::InvalidStoredOperation`] when a receipt is malformed, duplicated
    /// or names a revision newer than the projected state.
    pub fn restore<I>(
        state: OperationValue,
        state_revision: u64,
        last_changed_run_version: u64,
        receipts: I,
    ) -> Result<Self, OperationError>
    where
        I: IntoIterator<Item = UpdateReceipt>,
    {
        let mut restored = BTreeMap::new();
        for receipt in receipts {
            receipt.validate_stored()?;
            if receipt.after_state_revision > state_revision {
                return Err(OperationError::InvalidStoredOperation);
            }
            let identity = ReceiptIdentity {
                scope: receipt.scope.clone(),
                update_key: receipt.update_key.clone(),
            };
            if restored.insert(identity, receipt).is_some() {
                return Err(OperationError::InvalidStoredOperation);
            }
        }
        Ok(Self {
            state,
            state_revision,
            last_changed_run_version,
            receipts: restored,
        })
    }

    #[must_use]
    pub const fn state(&self) -> &OperationValue {
        &self.state
    }

    #[must_use]
    pub const fn state_revision(&self) -> u64 {
        self.state_revision
    }

    #[must_use]
    pub const fn last_changed_run_version(&self) -> u64 {
        self.last_changed_run_version
    }

    #[must_use]
    pub fn query_snapshot(&self, run_id: RunId, run_version: u64) -> QuerySnapshot {
        QuerySnapshot {
            run_id,
            run_version,
            state_revision: self.state_revision,
            state: self.state.clone(),
        }
    }

    pub fn receipts(&self) -> impl Iterator<Item = &UpdateReceipt> {
        self.receipts.values()
    }

    /// Resolves a serialized Update. The callback runs only after replay, mutability and expected
    /// revision checks, and any callback error leaves state and receipts unchanged.
    ///
    /// Persistence must commit the resulting state, receipt, audit, journal and notification in a
    /// single transaction before exposing this resolution.
    ///
    /// # Errors
    /// Returns a typed pre-admission, idempotency, evaluation or revision-overflow error.
    pub fn resolve_update<F>(
        &mut self,
        run_status: RunStatus,
        request: &UpdateRequest,
        commit_run_version: u64,
        evaluate: F,
    ) -> Result<UpdateResolution, OperationError>
    where
        F: FnOnce(&OperationValue) -> Result<UpdateEvaluation, OperationError>,
    {
        self.resolve_update_inner(
            run_status,
            request,
            commit_run_version,
            Some(commit_run_version),
            evaluate,
        )
    }

    /// Resolves an Update when a successful application advances the workflow journal but a
    /// durable business rejection does not. The receipt records the version actually committed by
    /// its outcome.
    ///
    /// # Errors
    /// Returns the same typed failures as [`Self::resolve_update`].
    pub fn resolve_update_at<F>(
        &mut self,
        run_status: RunStatus,
        request: &UpdateRequest,
        current_run_version: u64,
        applied_run_version: u64,
        evaluate: F,
    ) -> Result<UpdateResolution, OperationError>
    where
        F: FnOnce(&OperationValue) -> Result<UpdateEvaluation, OperationError>,
    {
        self.resolve_update_inner(
            run_status,
            request,
            current_run_version,
            Some(applied_run_version),
            evaluate,
        )
    }

    /// Resolves an Update whose applied outcome must consume the next journal sequence. Receipt
    /// replay and business rejection remain available when that sequence is exhausted because
    /// neither outcome advances the journal.
    ///
    /// # Errors
    /// Returns [`OperationError::RevisionExhausted`] only when the evaluator applies an update and
    /// the current run version has no successor, plus the errors of [`Self::resolve_update`].
    pub fn resolve_update_advancing<F>(
        &mut self,
        run_status: RunStatus,
        request: &UpdateRequest,
        current_run_version: u64,
        evaluate: F,
    ) -> Result<UpdateResolution, OperationError>
    where
        F: FnOnce(&OperationValue) -> Result<UpdateEvaluation, OperationError>,
    {
        self.resolve_update_inner(
            run_status,
            request,
            current_run_version,
            current_run_version.checked_add(1),
            evaluate,
        )
    }

    fn resolve_update_inner<F>(
        &mut self,
        run_status: RunStatus,
        request: &UpdateRequest,
        current_run_version: u64,
        applied_run_version: Option<u64>,
        evaluate: F,
    ) -> Result<UpdateResolution, OperationError>
    where
        F: FnOnce(&OperationValue) -> Result<UpdateEvaluation, OperationError>,
    {
        let identity = ReceiptIdentity {
            scope: request.scope.clone(),
            update_key: request.update_key.clone(),
        };
        if let Some(receipt) = self.receipts.get(&identity) {
            if receipt.operation == request.operation
                && receipt.request_hash == request.request_hash
            {
                return Ok(UpdateResolution {
                    receipt: receipt.clone(),
                    replayed: true,
                });
            }
            return Err(OperationError::UpdateKeyReused);
        }

        if !matches!(run_status, RunStatus::Running | RunStatus::Waiting) {
            return Err(OperationError::RunNotMutable(run_status));
        }

        let before = self.state_revision;
        let decision = match request.expected_state_revision {
            Some(expected) if expected != before => {
                UpdateEvaluation::Rejected(UpdateRejection::RevisionConflict {
                    expected,
                    actual: before,
                })
            }
            _ => evaluate(&self.state)?,
        };

        let (status, rejection, result, after, receipt_run_version, new_state) = match decision {
            UpdateEvaluation::Applied { new_state, result } => {
                if new_state.schema_digest != self.state.schema_digest {
                    return Err(OperationError::StateSchemaChanged);
                }
                let applied_run_version =
                    applied_run_version.ok_or(OperationError::RevisionExhausted)?;
                let after = before
                    .checked_add(1)
                    .ok_or(OperationError::RevisionExhausted)?;
                (
                    UpdateReceiptStatus::Applied,
                    None,
                    Some(result),
                    after,
                    applied_run_version,
                    Some(new_state),
                )
            }
            UpdateEvaluation::Rejected(rejection) => (
                UpdateReceiptStatus::Rejected,
                Some(rejection),
                None,
                before,
                current_run_version,
                None,
            ),
        };
        let receipt = UpdateReceipt {
            scope: request.scope.clone(),
            resolved_run_id: request.resolved_run_id,
            operation: request.operation.clone(),
            update_key: request.update_key.clone(),
            request_hash: request.request_hash,
            status,
            rejection,
            result,
            before_state_revision: before,
            after_state_revision: after,
            run_version: receipt_run_version,
            actor: request.actor.clone(),
        };

        if let Some(new_state) = new_state {
            self.state = new_state;
            self.state_revision = after;
            self.last_changed_run_version = receipt_run_version;
        }
        self.receipts.insert(identity, receipt.clone());
        Ok(UpdateResolution {
            receipt,
            replayed: false,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OperationError {
    #[error("operation disclosure is not explicitly declared")]
    UnsupportedDisclosure,
    #[error("update key must be 1..128 printable ASCII characters without spaces")]
    InvalidUpdateKey,
    #[error("operation state or result exceeds 256 KiB")]
    ValueTooLarge,
    #[error("an update cannot change the pinned state schema")]
    StateSchemaChanged,
    #[error("the update key was already used with different request content")]
    UpdateKeyReused,
    #[error("the run is {0} and does not admit a new update")]
    RunNotMutable(RunStatus),
    #[error("state revision is exhausted")]
    RevisionExhausted,
    #[error("declared operation evaluation failed")]
    EvaluationFailed,
    #[error("stored declared operation state is inconsistent")]
    InvalidStoredOperation,
}
