//! Transaction port for explicitly declared business Query and Update operations.
//!
//! The port exposes only content-addressed, schema-checked values. An adapter must resolve current
//! authorization and the immutable declaration before reading a receipt or state, and must keep an
//! applied state change, its receipt, journal event, audit record and outbox notification in one
//! commit.

use std::fmt;
use std::future::Future;

use sagat_domain::audit::AuditRecord;
use sagat_domain::execution::{
    MAX_OPERATION_VALUE_BYTES, OperationError, OperationScope, OperationValue, QueryResponse,
    QuerySnapshot, ResultDisclosure, RunId, UpdateEvaluation, UpdateRejection, UpdateRequest,
    UpdateResolution,
};
use sagat_domain::identity::Subject;
use sagat_domain::workflow::{ActivityName, Digest, NodeId};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::definitions::MAX_IR_BYTES;
use crate::error::StoreError;
use crate::operation_evaluation::{
    DeclaredOperationArguments, MAX_OPERATION_CONTEXT_BYTES, RetainedOperationValue,
};

/// Immutable declaration category used to prevent a Query from being admitted as an Update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredOperationKind {
    Query,
    Update,
}

/// Immutable operation metadata persisted with a published workflow version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedOperationDefinition {
    kind: DeclaredOperationKind,
    name: ActivityName,
    declaration_digest: Digest,
    result_schema_digest: Digest,
    disclosure: ResultDisclosure,
}

impl PublishedOperationDefinition {
    #[must_use]
    pub const fn new(
        kind: DeclaredOperationKind,
        name: ActivityName,
        declaration_digest: Digest,
        result_schema_digest: Digest,
        disclosure: ResultDisclosure,
    ) -> Self {
        Self {
            kind,
            name,
            declaration_digest,
            result_schema_digest,
            disclosure,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> DeclaredOperationKind {
        self.kind
    }

    #[must_use]
    pub const fn name(&self) -> &ActivityName {
        &self.name
    }

    #[must_use]
    pub const fn declaration_digest(&self) -> Digest {
        self.declaration_digest
    }

    #[must_use]
    pub const fn result_schema_digest(&self) -> Digest {
        self.result_schema_digest
    }

    #[must_use]
    pub const fn disclosure(&self) -> ResultDisclosure {
        self.disclosure
    }
}

/// Schema and named operations frozen together with one workflow version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedDeclaredState {
    state_schema_digest: Digest,
    operations: Vec<PublishedOperationDefinition>,
}

impl PublishedDeclaredState {
    /// Builds a publication only when operation identities are unique within their kind.
    ///
    /// # Errors
    /// Returns [`OperationError::InvalidStoredOperation`] for duplicate declarations.
    pub fn new(
        state_schema_digest: Digest,
        mut operations: Vec<PublishedOperationDefinition>,
    ) -> Result<Self, OperationError> {
        operations.sort_by(|left, right| {
            left.kind
                .as_str()
                .cmp(right.kind.as_str())
                .then_with(|| left.name.cmp(&right.name))
        });
        if operations
            .windows(2)
            .any(|pair| pair[0].kind == pair[1].kind && pair[0].name == pair[1].name)
        {
            return Err(OperationError::InvalidStoredOperation);
        }
        Ok(Self {
            state_schema_digest,
            operations,
        })
    }

    #[must_use]
    pub const fn state_schema_digest(&self) -> Digest {
        self.state_schema_digest
    }

    pub fn operations(&self) -> impl Iterator<Item = &PublishedOperationDefinition> {
        self.operations.iter()
    }
}

impl DeclaredOperationKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Query => "QUERY",
            Self::Update => "UPDATE",
        }
    }
}

/// One authorized Query address. Arguments and results stay outside the persistence command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredQuery {
    scope: OperationScope,
    expected_run_id: Option<RunId>,
    operation: ActivityName,
    actor: Subject,
}

impl DeclaredQuery {
    #[must_use]
    pub const fn new(
        scope: OperationScope,
        expected_run_id: RunId,
        operation: ActivityName,
        actor: Subject,
    ) -> Self {
        Self {
            scope,
            expected_run_id: Some(expected_run_id),
            operation,
            actor,
        }
    }

    #[must_use]
    pub const fn for_chain(
        scope: OperationScope,
        expected_run_id: Option<RunId>,
        operation: ActivityName,
        actor: Subject,
    ) -> Self {
        Self {
            scope,
            expected_run_id,
            operation,
            actor,
        }
    }

    #[must_use]
    pub const fn scope(&self) -> &OperationScope {
        &self.scope
    }

    #[must_use]
    pub const fn expected_run_id(&self) -> Option<RunId> {
        self.expected_run_id
    }

    #[must_use]
    pub const fn operation(&self) -> &ActivityName {
        &self.operation
    }

    #[must_use]
    pub const fn actor(&self) -> &Subject {
        &self.actor
    }
}

/// One immutable retained JSON value used to build a transactional operation snapshot.
#[derive(Clone, PartialEq, Eq)]
pub struct OperationSnapshotPayload {
    digest: Digest,
    content: Vec<u8>,
}

/// Canonical immutable workflow document pinned by the run's published version.
#[derive(Clone, PartialEq, Eq)]
pub struct DeclaredOperationDocument {
    digest: Digest,
    content: Vec<u8>,
}

impl DeclaredOperationDocument {
    /// Restores a canonical document only when its bytes match the version digest and IR limit.
    ///
    /// # Errors
    /// Returns [`OperationError::InvalidStoredOperation`] for inconsistent storage.
    pub fn restore(digest: Digest, content: Vec<u8>) -> Result<Self, OperationError> {
        let actual: [u8; 32] = Sha256::digest(&content).into();
        if content.len() > MAX_IR_BYTES || digest != Digest::new(actual) {
            return Err(OperationError::InvalidStoredOperation);
        }
        Ok(Self { digest, content })
    }

    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    #[must_use]
    pub fn content(&self) -> &[u8] {
        &self.content
    }
}

impl fmt::Debug for DeclaredOperationDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeclaredOperationDocument")
            .field("digest", &self.digest)
            .field("size_bytes", &self.content.len())
            .finish_non_exhaustive()
    }
}

impl OperationSnapshotPayload {
    /// Restores a payload only when its bytes match the stored content address and value limit.
    ///
    /// # Errors
    /// Returns [`OperationError::InvalidStoredOperation`] for inconsistent storage.
    pub fn restore(digest: Digest, content: Vec<u8>) -> Result<Self, OperationError> {
        let actual: [u8; 32] = Sha256::digest(&content).into();
        if content.len() > MAX_OPERATION_VALUE_BYTES as usize || digest != Digest::new(actual) {
            return Err(OperationError::InvalidStoredOperation);
        }
        Ok(Self { digest, content })
    }

    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    #[must_use]
    pub fn content(&self) -> &[u8] {
        &self.content
    }
}

impl fmt::Debug for OperationSnapshotPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationSnapshotPayload")
            .field("size_bytes", &self.content.len())
            .finish_non_exhaustive()
    }
}

/// One completed root-node result and its retained payload provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedOperationResult {
    node: NodeId,
    payload: OperationSnapshotPayload,
}

impl CompletedOperationResult {
    #[must_use]
    pub const fn new(node: NodeId, payload: OperationSnapshotPayload) -> Self {
        Self { node, payload }
    }

    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }

    #[must_use]
    pub const fn payload(&self) -> &OperationSnapshotPayload {
        &self.payload
    }
}

/// State, input and completed results read under one run lock and journal version.
#[derive(Clone, PartialEq, Eq)]
pub struct DeclaredOperationSnapshot {
    snapshot: QuerySnapshot,
    declaration_digest: Digest,
    definition: DeclaredOperationDocument,
    workflow_input: OperationSnapshotPayload,
    state_content: Vec<u8>,
    completed_results: Vec<CompletedOperationResult>,
}

impl DeclaredOperationSnapshot {
    /// Restores a complete transactional snapshot after checking addresses, bounds and duplicate
    /// completed-result names.
    ///
    /// # Errors
    /// Returns [`OperationError::InvalidStoredOperation`] for inconsistent retained data.
    pub fn restore(
        snapshot: QuerySnapshot,
        declaration_digest: Digest,
        definition: DeclaredOperationDocument,
        workflow_input: OperationSnapshotPayload,
        state_content: Vec<u8>,
        mut completed_results: Vec<CompletedOperationResult>,
    ) -> Result<Self, OperationError> {
        RetainedOperationValue::new(snapshot.state(), &state_content)
            .map_err(|_| OperationError::InvalidStoredOperation)?;
        completed_results.sort_by(|left, right| left.node.cmp(&right.node));
        if completed_results
            .windows(2)
            .any(|pair| pair[0].node == pair[1].node)
        {
            return Err(OperationError::InvalidStoredOperation);
        }
        let total = workflow_input
            .content
            .len()
            .checked_add(state_content.len())
            .and_then(|size| {
                completed_results.iter().try_fold(size, |total, result| {
                    total.checked_add(result.payload.content.len())
                })
            })
            .ok_or(OperationError::ValueTooLarge)?;
        if total > MAX_OPERATION_CONTEXT_BYTES {
            return Err(OperationError::ValueTooLarge);
        }
        Ok(Self {
            snapshot,
            declaration_digest,
            definition,
            workflow_input,
            state_content,
            completed_results,
        })
    }

    #[must_use]
    pub const fn snapshot(&self) -> &QuerySnapshot {
        &self.snapshot
    }

    #[must_use]
    pub const fn declaration_digest(&self) -> Digest {
        self.declaration_digest
    }

    #[must_use]
    pub const fn definition(&self) -> &DeclaredOperationDocument {
        &self.definition
    }

    #[must_use]
    pub const fn workflow_input(&self) -> &OperationSnapshotPayload {
        &self.workflow_input
    }

    #[must_use]
    pub fn state_content(&self) -> &[u8] {
        &self.state_content
    }

    pub fn completed_results(&self) -> impl Iterator<Item = &CompletedOperationResult> {
        self.completed_results.iter()
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.snapshot.run_id()
    }

    #[must_use]
    pub const fn run_version(&self) -> u64 {
        self.snapshot.run_version()
    }

    #[must_use]
    pub const fn state_revision(&self) -> u64 {
        self.snapshot.state_revision()
    }

    #[must_use]
    pub const fn state(&self) -> &OperationValue {
        self.snapshot.state()
    }
}

impl fmt::Debug for DeclaredOperationSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeclaredOperationSnapshot")
            .field("run_id", &self.run_id())
            .field("run_version", &self.run_version())
            .field("state_revision", &self.state_revision())
            .field("definition_digest", &self.definition.digest)
            .field("workflow_input_size", &self.workflow_input.content.len())
            .field("state_size", &self.state_content.len())
            .field("completed_result_count", &self.completed_results.len())
            .finish_non_exhaustive()
    }
}

/// Bytes produced by an evaluator together with the domain value retained by the transaction.
#[derive(Clone, PartialEq, Eq)]
pub struct EvaluatedOperationWrite {
    value: OperationValue,
    content: Vec<u8>,
}

impl EvaluatedOperationWrite {
    /// # Errors
    /// Returns [`OperationError::InvalidStoredOperation`] unless bytes match the value reference.
    pub fn restore(value: OperationValue, content: Vec<u8>) -> Result<Self, OperationError> {
        RetainedOperationValue::new(&value, &content)
            .map_err(|_| OperationError::InvalidStoredOperation)?;
        Ok(Self { value, content })
    }

    #[must_use]
    pub const fn value(&self) -> &OperationValue {
        &self.value
    }

    #[must_use]
    pub fn content(&self) -> &[u8] {
        &self.content
    }
}

impl fmt::Debug for EvaluatedOperationWrite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvaluatedOperationWrite")
            .field("size_bytes", &self.content.len())
            .finish_non_exhaustive()
    }
}

/// Evaluator output accepted by the serialized Update transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclaredUpdateDecision {
    Applied {
        new_state: EvaluatedOperationWrite,
        result: EvaluatedOperationWrite,
    },
    Rejected(UpdateRejection),
}

impl DeclaredUpdateDecision {
    #[must_use]
    pub fn domain_evaluation(&self) -> UpdateEvaluation {
        match self {
            Self::Applied { new_state, result } => UpdateEvaluation::Applied {
                new_state: new_state.value.clone(),
                result: result.value.clone(),
            },
            Self::Rejected(rejection) => UpdateEvaluation::Rejected(rejection.clone()),
        }
    }

    #[must_use]
    pub const fn writes(&self) -> Option<(&EvaluatedOperationWrite, &EvaluatedOperationWrite)> {
        match self {
            Self::Applied { new_state, result } => Some((new_state, result)),
            Self::Rejected(_) => None,
        }
    }
}

/// An Update and the non-sensitive evidence that must commit with its decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredUpdate {
    request: UpdateRequest,
    expected_run_id: Option<RunId>,
    arguments: DeclaredOperationArguments,
    audit: AuditRecord,
    outbox_id: Uuid,
    occurred_at: OffsetDateTime,
}

impl DeclaredUpdate {
    /// Builds an Update addressed to one exact run. Chain-addressed callers must use
    /// [`Self::for_chain`] so the receipt identity does not depend on the current head.
    #[must_use]
    pub fn new(
        request: UpdateRequest,
        arguments: DeclaredOperationArguments,
        audit: AuditRecord,
        outbox_id: Uuid,
        occurred_at: OffsetDateTime,
    ) -> Self {
        let request = UpdateRequest::new(
            request.scope().clone(),
            request.resolved_run_id(),
            request.operation().clone(),
            request.update_key().clone(),
            declared_update_request_hash(
                request.operation(),
                &arguments,
                request.expected_state_revision(),
                request.resolved_run_id(),
            ),
            request.expected_state_revision(),
            request.actor().clone(),
        );
        let expected_run_id = request.resolved_run_id();
        Self {
            request,
            expected_run_id: Some(expected_run_id),
            arguments,
            audit,
            outbox_id,
            occurred_at,
        }
    }

    /// Builds a chain-scoped Update whose durable identity is independent of the head resolved by
    /// the transaction. An explicit expected run remains part of that identity and acts as the CAS
    /// precondition for a new receipt.
    ///
    /// # Errors
    /// Returns [`DeclaredOperationStoreError::AddressUnavailable`] unless the scope addresses the
    /// supplied chain.
    #[allow(clippy::too_many_arguments)]
    pub fn for_chain(
        scope: OperationScope,
        expected_run_id: Option<RunId>,
        operation: ActivityName,
        update_key: sagat_domain::execution::UpdateKey,
        expected_state_revision: Option<u64>,
        actor: Subject,
        arguments: DeclaredOperationArguments,
        audit: AuditRecord,
        outbox_id: Uuid,
        occurred_at: OffsetDateTime,
    ) -> Result<Self, DeclaredOperationStoreError> {
        let chain_id = match scope.address() {
            sagat_domain::execution::OperationAddress::Chain(chain_id) => *chain_id,
            sagat_domain::execution::OperationAddress::Run(_) => {
                return Err(DeclaredOperationStoreError::AddressUnavailable);
            }
        };
        let request_hash = declared_chain_update_request_hash(
            &operation,
            &arguments,
            expected_state_revision,
            chain_id,
            expected_run_id,
        );
        let request = UpdateRequest::new(
            scope,
            expected_run_id.unwrap_or(chain_id),
            operation,
            update_key,
            request_hash,
            expected_state_revision,
            actor,
        );
        Ok(Self {
            request,
            expected_run_id,
            arguments,
            audit,
            outbox_id,
            occurred_at,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &UpdateRequest {
        &self.request
    }

    #[must_use]
    pub const fn expected_run_id(&self) -> Option<RunId> {
        self.expected_run_id
    }

    #[must_use]
    pub fn resolved_request(&self, resolved_run_id: RunId) -> UpdateRequest {
        UpdateRequest::new(
            self.request.scope().clone(),
            resolved_run_id,
            self.request.operation().clone(),
            self.request.update_key().clone(),
            self.request.request_hash(),
            self.request.expected_state_revision(),
            self.request.actor().clone(),
        )
    }

    #[must_use]
    pub const fn arguments(&self) -> &DeclaredOperationArguments {
        &self.arguments
    }

    #[must_use]
    pub const fn audit(&self) -> &AuditRecord {
        &self.audit
    }

    #[must_use]
    pub const fn outbox_id(&self) -> Uuid {
        self.outbox_id
    }

    #[must_use]
    pub const fn occurred_at(&self) -> OffsetDateTime {
        self.occurred_at
    }
}

/// Produces the stable request identity used by durable Update receipts.
#[must_use]
pub fn declared_update_request_hash(
    operation: &ActivityName,
    arguments: &DeclaredOperationArguments,
    expected_state_revision: Option<u64>,
    expected_run_id: RunId,
) -> Digest {
    let mut hash = Sha256::new();
    hash.update(b"sagat.declared-update.v1\0");
    hash.update((operation.as_str().len() as u64).to_be_bytes());
    hash.update(operation.as_str().as_bytes());
    hash.update((arguments.canonical().len() as u64).to_be_bytes());
    hash.update(arguments.canonical());
    match expected_state_revision {
        Some(revision) => {
            hash.update([1]);
            hash.update(revision.to_be_bytes());
        }
        None => hash.update([0]),
    }
    hash.update(expected_run_id.as_uuid().as_bytes());
    Digest::new(hash.finalize().into())
}

/// Produces the stable identity of a chain Update. The current head is deliberately absent: an
/// exact retry without a CAS precondition must find the original receipt after rollover.
#[must_use]
pub fn declared_chain_update_request_hash(
    operation: &ActivityName,
    arguments: &DeclaredOperationArguments,
    expected_state_revision: Option<u64>,
    chain_id: RunId,
    expected_run_id: Option<RunId>,
) -> Digest {
    let mut hash = Sha256::new();
    hash.update(b"sagat.declared-chain-update.v1\0");
    hash.update(chain_id.as_uuid().as_bytes());
    hash.update((operation.as_str().len() as u64).to_be_bytes());
    hash.update(operation.as_str().as_bytes());
    hash.update((arguments.canonical().len() as u64).to_be_bytes());
    hash.update(arguments.canonical());
    match expected_state_revision {
        Some(revision) => {
            hash.update([1]);
            hash.update(revision.to_be_bytes());
        }
        None => hash.update([0]),
    }
    match expected_run_id {
        Some(run_id) => {
            hash.update([1]);
            hash.update(run_id.as_uuid().as_bytes());
        }
        None => hash.update([0]),
    }
    Digest::new(hash.finalize().into())
}

/// Safe failure categories of the declared-operation transaction boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeclaredOperationStoreError {
    #[error("declared operation resource was not found")]
    NotFound,
    #[error("the current identity is not authorized for the declared operation")]
    Forbidden,
    #[error("the immutable declared operation is unavailable")]
    DefinitionUnavailable,
    #[error("the audit evidence does not match the operation")]
    AuditMismatch,
    #[error("the address kind is not available")]
    AddressUnavailable,
    #[error("the execution chain head changed")]
    ChainHeadChanged,
    #[error("declared operation rejected: {0}")]
    Operation(#[from] OperationError),
    #[error("declared operation storage failed: {0}")]
    Storage(#[from] StoreError),
}

/// Schema-checked Query result bytes bound to the exact audited snapshot.
#[derive(Clone, PartialEq, Eq)]
pub struct EvaluatedDeclaredQuery {
    response: QueryResponse,
    result: EvaluatedOperationWrite,
}

impl EvaluatedDeclaredQuery {
    /// # Errors
    /// Returns a retained-value error unless the response references the exact checked bytes.
    pub fn restore(
        response: QueryResponse,
        result: EvaluatedOperationWrite,
    ) -> Result<Self, crate::operation_evaluation::OperationEvaluationError> {
        if response.result() != result.value() {
            return Err(
                crate::operation_evaluation::OperationEvaluationError::RetainedValueMismatch,
            );
        }
        Ok(Self { response, result })
    }

    #[must_use]
    pub const fn response(&self) -> &QueryResponse {
        &self.response
    }

    #[must_use]
    pub const fn result(&self) -> &EvaluatedOperationWrite {
        &self.result
    }
}

impl fmt::Debug for EvaluatedDeclaredQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvaluatedDeclaredQuery")
            .field("run_id", &self.response.run_id())
            .field("run_version", &self.response.run_version())
            .field("state_revision", &self.response.state_revision())
            .field("result_size", &self.result.content.len())
            .finish_non_exhaustive()
    }
}

/// Capability-free evaluator boundary used by public declared-operation adapters.
pub trait DeclaredOperationEvaluator: Send + Sync {
    fn query(
        &self,
        snapshot: &DeclaredOperationSnapshot,
        operation: &ActivityName,
        arguments: &DeclaredOperationArguments,
    ) -> Result<EvaluatedDeclaredQuery, crate::operation_evaluation::OperationEvaluationError>;

    fn validate_update_arguments(
        &self,
        snapshot: &DeclaredOperationSnapshot,
        operation: &ActivityName,
        arguments: &DeclaredOperationArguments,
    ) -> Result<(), crate::operation_evaluation::OperationEvaluationError>;

    fn update(
        &self,
        snapshot: &DeclaredOperationSnapshot,
        operation: &ActivityName,
        arguments: &DeclaredOperationArguments,
    ) -> Result<DeclaredUpdateDecision, crate::operation_evaluation::OperationEvaluationError>;
}

/// Durable boundary for Query snapshots and serialized Update decisions.
pub trait DeclaredOperationStore: Send + Sync {
    /// Reads one consistent state snapshot and commits the access audit before returning it.
    fn query_snapshot(
        &self,
        query: &DeclaredQuery,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<DeclaredOperationSnapshot, DeclaredOperationStoreError>> + Send;

    /// Serializes on the addressed run, or on the chain before its resolved run, replays a receipt
    /// before mutability checks, and invokes `evaluate` at most once with the current persisted
    /// state for a new request.
    fn apply_update<V, F>(
        &self,
        update: &DeclaredUpdate,
        validate_arguments: V,
        evaluate: F,
    ) -> impl Future<Output = Result<UpdateResolution, DeclaredOperationStoreError>> + Send
    where
        V: FnOnce(
                &DeclaredOperationSnapshot,
                &DeclaredOperationArguments,
            ) -> Result<(), OperationError>
            + Send,
        F: FnOnce(&DeclaredOperationSnapshot) -> Result<DeclaredUpdateDecision, OperationError>
            + Send;

    /// Reads the checked result bytes of the exact durable resolution after rechecking current
    /// authorization. Rejected outcomes return `None`.
    fn update_result(
        &self,
        update: &DeclaredUpdate,
        resolution: &UpdateResolution,
    ) -> impl Future<Output = Result<Option<EvaluatedOperationWrite>, DeclaredOperationStoreError>> + Send;
}
