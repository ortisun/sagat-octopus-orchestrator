//! RED scaffold for the dormant native ChildRoot Acquire transaction.

use sagat_application::capacity_acquire::{
    CapacityAcquireCommand, CapacityAcquireError, CapacityAcquireResult,
    CurrentChildRootAcquireStore,
};
use sqlx::PgPool;

#[derive(Clone)]
pub struct PgCapacityAcquireStore {
    _pool: PgPool,
}

impl PgCapacityAcquireStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { _pool: pool }
    }
}

impl CurrentChildRootAcquireStore for PgCapacityAcquireStore {
    async fn acquire_current_child_root(
        &self,
        _command: &CapacityAcquireCommand,
    ) -> Result<CapacityAcquireResult, CapacityAcquireError> {
        // The native tests must reject this before the transaction is implemented.
        Err(CapacityAcquireError::Missing)
    }
}
