//! Business identifiers (feature 018, `FR-018-018..022`, ADR-0050 decision 7): the binding of an
//! identifier to the execution that holds it, claimed in the transaction that creates the run.
//!
//! The row of a binding is taken in the slot of the run-creating aggregates, after the schedule and
//! before any existing run, so a start that carries an identifier cannot be part of a lock cycle with
//! the transitions a run goes through: those never touch this table. Whether the holder is open is
//! read from the holder (a run, or a chain by its initial run), never copied here.
//!
//! The identifier is customer text. It is bound as a parameter and never put in an error, a log or a
//! span: a refusal carries the holder's id and the instant the id is free, not the identifier.

use sagat_application::error::StoreError;
use sagat_application::runs::{BusinessBinding, BusinessStart};
use sagat_domain::business_ids::{
    Admission, Binding, BusinessId, Held, HolderStatus, ReuseWindow, admit,
};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::{RunId, RunStatus};
use sagat_domain::identity::{NamespaceId, TenantId};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::identity::{col, store_error};

/// How a claim ended when it did not refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Claimed {
    /// The identifier is bound to the new run (it was free, or its holder had closed and was not
    /// remembered, or the policy allowed a new run).
    Bound,
    /// An open execution holds it and the start asked for `USE_EXISTING`: bind nothing, create
    /// nothing, return this one.
    Existing(Uuid),
}

/// Who holds a binding and how it stands, read where the lock on the binding's row is held.
async fn holder_status(
    conn: &mut PgConnection,
    tenant: TenantId,
    holder: Uuid,
) -> Result<Option<(HolderStatus, Option<OffsetDateTime>)>, StoreError> {
    // The holder's run, and its chain when the run is the initial run of one: a chain is open until
    // it has a final outcome, and it ended when its last segment did.
    let row = sqlx::query(
        "SELECT r.status, r.finished_at, c.chain_id IS NOT NULL AS chained, c.final_outcome, \
                cr.finished_at AS chain_finished_at \
         FROM runs r \
         LEFT JOIN execution_chains c ON c.tenant_id = r.tenant_id AND c.chain_id = r.id \
         LEFT JOIN runs cr ON cr.tenant_id = c.tenant_id AND cr.id = c.current_run_id \
         WHERE r.tenant_id = $1 AND r.id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(holder)
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(read_holder(&row)?))
}

/// The holder of a row of [`holder_status`]'s shape: the run, its chain when it begins one, and the
/// last segment's finish.
fn read_holder(row: &PgRow) -> Result<(HolderStatus, Option<OffsetDateTime>), StoreError> {
    let parse = |text: &str| -> Result<RunStatus, StoreError> {
        text.parse::<RunStatus>().map_err(|_| StoreError::Internal {
            detail: "a stored run status is not one".to_owned(),
        })
    };
    let chained: bool = col(row, "chained")?;
    let status: String = col(row, "status")?;
    let finished: Option<OffsetDateTime> = col(row, "finished_at")?;
    let outcome: Option<String> = col(row, "final_outcome")?;
    let chain_finished: Option<OffsetDateTime> = col(row, "chain_finished_at")?;
    Ok(if chained {
        match outcome {
            Some(outcome) => (
                HolderStatus::of(parse(&outcome)?, chain_finished),
                chain_finished,
            ),
            None => (HolderStatus::Open, None),
        }
    } else {
        (HolderStatus::of(parse(&status)?, finished), finished)
    })
}

/// How an identifier stands in a namespace (`FR-018-022`), read without a lock: the holder, whether
/// it is open, how it ended and until when it is remembered. `None` for an identifier that is
/// unbound, whose holder is gone, or that has been free for longer than its window, the same three
/// cases in which a start finds it free.
pub(crate) async fn binding(
    pool: &PgPool,
    tenant: TenantId,
    namespace: NamespaceId,
    id: &BusinessId,
) -> Result<Option<BusinessBinding>, StoreError> {
    let Some(row) = sqlx::query(
        "SELECT b.execution_id, b.workflow_id, b.bound_at, b.reuse_window_s, \
                r.status, r.finished_at, c.chain_id IS NOT NULL AS chained, c.final_outcome, \
                c.current_run_id, cr.finished_at AS chain_finished_at, \
                clock_timestamp() AS looked_at \
         FROM business_ids b \
         JOIN runs r ON r.tenant_id = b.tenant_id AND r.id = b.execution_id \
         LEFT JOIN execution_chains c ON c.tenant_id = r.tenant_id AND c.chain_id = r.id \
         LEFT JOIN runs cr ON cr.tenant_id = c.tenant_id AND cr.id = c.current_run_id \
         WHERE b.tenant_id = $1 AND b.namespace_id = $2 AND b.business_id = $3",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(id.as_str())
    .fetch_optional(pool)
    .await
    .map_err(store_error)?
    else {
        return Ok(None);
    };
    let (holder, _) = read_holder(&row)?;
    let window: i32 = col(&row, "reuse_window_s")?;
    let window = ReuseWindow::from_seconds(u32::try_from(window).unwrap_or(0)).map_err(|_| {
        StoreError::Internal {
            detail: "a stored reuse window is out of range".to_owned(),
        }
    })?;
    let held = Held {
        binding: Binding::new(window),
        holder,
    };
    let reusable_after = held.reusable_after();
    let looked_at: OffsetDateTime = col(&row, "looked_at")?;
    if reusable_after.is_some_and(|until| looked_at >= until) {
        return Ok(None);
    }
    let execution: Uuid = col(&row, "execution_id")?;
    let current: Option<Uuid> = col(&row, "current_run_id")?;
    Ok(Some(BusinessBinding {
        business_id: id.clone(),
        execution_id: RunId::new(execution),
        current_run_id: RunId::new(current.unwrap_or(execution)),
        open: matches!(holder, HolderStatus::Open),
        last_status: match holder {
            HolderStatus::Open => None,
            HolderStatus::Closed { status, .. } => Some(status),
        },
        workflow_id: WorkflowId::new(col::<Uuid>(&row, "workflow_id")?),
        bound_at: col(&row, "bound_at")?,
        reusable_after,
    }))
}

/// Claims `business.id` for `run`, in the transaction that creates it.
///
/// Free, it is bound at once. Held, the row is locked (`FOR UPDATE`, which the runtime role may do
/// because it holds `UPDATE`) and the domain decides from the holder's status: bind again, return the
/// holder, or refuse with [`StoreError::BusinessIdHeld`]. A refusal leaves the transaction to the
/// caller, who rolls it back so the row is not held while the connection waits to be reused.
///
/// # Errors
/// [`StoreError::BusinessIdHeld`] (`BUSINESS_ID_IN_USE`, `BUSINESS_ID_REUSE_REJECTED`) and the
/// failures of the store.
pub(crate) async fn claim(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    workflow: Uuid,
    run: RunId,
    business: &BusinessStart,
) -> Result<Claimed, StoreError> {
    let id = business.id.as_str();
    // A concurrent claim of the same identifier that has not committed makes the insert wait for its
    // outcome, so of a thousand starts exactly one binds a free identifier and the others read it.
    for _ in 0..3 {
        let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *conn)
            .await
            .map_err(store_error)?;
        let inserted = sqlx::query(
            "INSERT INTO business_ids \
             (tenant_id, namespace_id, business_id, execution_id, workflow_id, bound_at, reuse_window_s) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (tenant_id, namespace_id, business_id) DO NOTHING",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(id)
        .bind(run.as_uuid())
        .bind(workflow)
        .bind(now)
        .bind(i32::try_from(business.window.seconds()).unwrap_or(i32::MAX))
        .execute(&mut *conn)
        .await
        .map_err(store_error)?
        .rows_affected();
        if inserted == 1 {
            return Ok(Claimed::Bound);
        }
        let Some(row) = sqlx::query(
            "SELECT execution_id, reuse_window_s FROM business_ids \
             WHERE tenant_id = $1 AND namespace_id = $2 AND business_id = $3 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?
        else {
            // purged between the insert and the lock: look again
            continue;
        };
        let holder: Uuid = col(&row, "execution_id")?;
        let window: i32 = col(&row, "reuse_window_s")?;
        let admission = match holder_status(conn, tenant, holder).await? {
            // a holder that is gone (purged by a retention policy) holds nothing
            None => Admission::Bind,
            Some((status, _)) => {
                let window = ReuseWindow::from_seconds(u32::try_from(window).unwrap_or(0))
                    .map_err(|_| StoreError::Internal {
                        detail: "a stored reuse window is out of range".to_owned(),
                    })?;
                let held = Held {
                    binding: Binding::new(window),
                    holder: status,
                };
                admit(Some(&held), business.conflict, business.reuse, now)
            }
        };
        return match admission {
            Admission::Bind => {
                sqlx::query(
                    "UPDATE business_ids SET execution_id = $4, workflow_id = $5, bound_at = $6, \
                     reuse_window_s = $7 \
                     WHERE tenant_id = $1 AND namespace_id = $2 AND business_id = $3",
                )
                .bind(tenant.as_uuid())
                .bind(namespace.as_uuid())
                .bind(id)
                .bind(run.as_uuid())
                .bind(workflow)
                .bind(now)
                .bind(i32::try_from(business.window.seconds()).unwrap_or(i32::MAX))
                .execute(&mut *conn)
                .await
                .map_err(store_error)?;
                Ok(Claimed::Bound)
            }
            Admission::UseExisting => Ok(Claimed::Existing(holder)),
            Admission::InUse => Err(StoreError::BusinessIdHeld {
                reason: "BUSINESS_ID_IN_USE",
                holder,
                reusable_after: None,
            }),
            Admission::ReuseRejected => {
                let reusable_after = holder_status(conn, tenant, holder)
                    .await?
                    .and_then(|(_, finished)| finished)
                    .and_then(|finished| {
                        finished.checked_add(Duration::seconds(i64::from(window)))
                    });
                Err(StoreError::BusinessIdHeld {
                    reason: "BUSINESS_ID_REUSE_REJECTED",
                    holder,
                    reusable_after,
                })
            }
        };
    }
    Err(StoreError::StateConflict(
        "the business identifier changed while it was being claimed",
    ))
}
