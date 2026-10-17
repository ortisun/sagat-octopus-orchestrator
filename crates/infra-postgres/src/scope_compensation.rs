//! Reads of the compensation of a collection or loop scope (feature 014), shared by the stores that
//! finish a scope: a scope that compensates cannot end while one of its compensations is open, and
//! the run ends with the worse of the stop and how the compensations ended (D385).

use sagat_domain::execution::{FailureCode, RunStatus, StopReason, scope_final_status};
use sagat_domain::saga::SagaOutcome;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

/// A scope compensation that is over.
#[derive(Debug, Clone)]
pub(crate) struct DoneSaga {
    pub cause: FailureCode,
    pub outcome: SagaOutcome,
    pub during_stop: bool,
}

/// Why the compensation of a scope could not be read as finished.
#[derive(Debug)]
pub(crate) enum DoneSagaError {
    /// There is none, or a compensation is still open.
    NotDone,
    /// The stored row does not describe a finished compensation.
    Corrupt(&'static str),
    Database(sqlx::Error),
}

/// The finished compensation of the scope owned by the instance `(owner_nodes, owner_indices,
/// owner_node)`, locking its row.
pub(crate) async fn read_done_saga(
    conn: &mut PgConnection,
    tenant: Uuid,
    run: Uuid,
    owner_nodes: &[String],
    owner_indices: &[i32],
    owner_node: &str,
) -> Result<DoneSaga, DoneSagaError> {
    let row = sqlx::query(
        "SELECT phase,cause_code,outcome,during_stop FROM scope_sagas \
         WHERE tenant_id=$1 AND run_id=$2 AND owner_path_nodes=$3 AND owner_path_indices=$4 \
           AND owner_node=$5 FOR UPDATE",
    )
    .bind(tenant)
    .bind(run)
    .bind(owner_nodes)
    .bind(owner_indices)
    .bind(owner_node)
    .fetch_optional(&mut *conn)
    .await
    .map_err(DoneSagaError::Database)?;
    let Some(row) = row.filter(|row| row.get::<String, _>("phase") == "DONE") else {
        return Err(DoneSagaError::NotDone);
    };
    Ok(DoneSaga {
        cause: FailureCode::parse(&row.get::<String, _>("cause_code"))
            .map_err(|_| DoneSagaError::Corrupt("scope compensation cause"))?,
        outcome: row
            .get::<Option<String>, _>("outcome")
            .and_then(|outcome| outcome.parse::<SagaOutcome>().ok())
            .ok_or(DoneSagaError::Corrupt("scope compensation outcome"))?,
        during_stop: row.get("during_stop"),
    })
}

/// How a run that is being stopped ends when its scope has compensated.
pub(crate) fn stopped_run_status(
    stop_reason: Option<&str>,
    stop_code: Option<&str>,
    saga: &DoneSaga,
) -> Result<RunStatus, DoneSagaError> {
    let reason = stop_reason
        .and_then(|reason| reason.parse::<StopReason>().ok())
        .ok_or(DoneSagaError::Corrupt("run stop reason"))?;
    let code = stop_code
        .map(FailureCode::parse)
        .transpose()
        .map_err(|_| DoneSagaError::Corrupt("run stop code"))?;
    Ok(scope_final_status(
        reason,
        code.as_ref(),
        &saga.cause,
        saga.outcome,
        saga.during_stop,
    ))
}
