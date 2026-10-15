//! Transaction port for explicitly declared business Query and Update operations.
//!
//! The port exposes only content-addressed, schema-checked values. An adapter must resolve current
//! authorization and the immutable declaration before reading a receipt or state, and must keep an
//! applied state change, its receipt, journal event, audit record and outbox notification in one
//! commit.

use std::future::Future;

use sagat_domain::audit::AuditRecord;
use sagat_domain::execution::{
    OperationError, OperationScope, OperationValue, QuerySnapshot, RunId, UpdateEvaluation,
    UpdateRequest, UpdateResolution,
};
use sagat_domain::identity::Subject;
use sagat_domain::workflow::ActivityName;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StoreError;

/// Immutable declaration category used to prevent a Query from being admitted as an Update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredOperationKind {
    Query,
    Update,
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
    resolved_run_id: RunId,
    operation: ActivityName,
    actor: Subject,
}

impl DeclaredQuery {
    #[must_use]
    pub const fn new(
        scope: OperationScope,
        resolved_run_id: RunId,
        operation: ActivityName,
        actor: Subject,
    ) -> Self {
        Self {
            scope,
            resolved_run_id,
            operation,
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
    pub const fn actor(&self) -> &Subject {
        &self.actor
    }
}

/// An Update and the non-sensitive evidence that must commit with its decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredUpdate {
    request: UpdateRequest,
    audit: AuditRecord,
    outbox_id: Uuid,
    occurred_at: OffsetDateTime,
}

impl DeclaredUpdate {
    #[must_use]
    pub const fn new(
        request: UpdateRequest,
        audit: AuditRecord,
        outbox_id: Uuid,
        occurred_at: OffsetDateTime,
    ) -> Self {
        Self {
            request,
            audit,
            outbox_id,
            occurred_at,
        }
    }

    #[must_use]
    pub const fn request(&self) -> &UpdateRequest {
        &self.request
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
    #[error("declared operation rejected: {0}")]
    Operation(#[from] OperationError),
    #[error("declared operation storage failed: {0}")]
    Storage(#[from] StoreError),
}

/// Durable boundary for Query snapshots and serialized Update decisions.
pub trait DeclaredOperationStore: Send + Sync {
    /// Reads one consistent state snapshot and commits the access audit before returning it.
    fn query_snapshot(
        &self,
        query: &DeclaredQuery,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<QuerySnapshot, DeclaredOperationStoreError>> + Send;

    /// Serializes on the addressed run, replays a receipt before mutability checks, and invokes
    /// `evaluate` at most once with the current persisted state for a new request.
    fn apply_update<F>(
        &self,
        update: &DeclaredUpdate,
        evaluate: F,
    ) -> impl Future<Output = Result<UpdateResolution, DeclaredOperationStoreError>> + Send
    where
        F: FnOnce(&OperationValue) -> Result<UpdateEvaluation, OperationError> + Send;
}
