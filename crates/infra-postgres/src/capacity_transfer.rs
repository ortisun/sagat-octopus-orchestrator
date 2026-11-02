//! One dormant ChildRoot transfer protocol; operation-specific ports preserve their Rust contracts.
use crate::{
    capacity_finish::confirmed_abort,
    execution::locked_scheduler_time,
    identity::{insert_audit, store_error},
    pool::BeginSafely,
};
use sagat_application::{
    capacity_acquire::CapacityAcquireError,
    capacity_finish::{CapacityFinishExpectations, CapacityFinishIdentity},
    capacity_vacate::CapacityVacateError as Error,
    error::StoreError,
    primitives::SchedulerAuthority,
};
use sagat_domain::audit::{AuditAction, AuditId, AuditRecord, ResourceType};
use sagat_domain::execution::{
    CapacityGroup, CapacityGroupError, CapacityMemberId, CapacityMemberSnapshot,
    CapacityMembership, CapacityPossession, CapacityTransferRefusal, RunId, RunStatus,
    acquire_gate, vacate_gate,
};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgRow};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub(crate) enum TransferOperation {
    Vacate,
    Acquire,
}
impl TransferOperation {
    const fn kind(self) -> &'static str {
        match self {
            Self::Vacate => "VACATE",
            Self::Acquire => "ACQUIRE",
        }
    }
    const fn event_type(self) -> &'static str {
        match self {
            Self::Vacate => "capacity.member.vacated",
            Self::Acquire => "capacity.member.acquired",
        }
    }
    const fn audit_action(self) -> AuditAction {
        match self {
            Self::Vacate => AuditAction::CapacityMemberVacated,
            Self::Acquire => AuditAction::CapacityMemberAcquired,
        }
    }
    const fn audit_message(self) -> &'static str {
        match self {
            Self::Vacate => "current child root capacity member vacated",
            Self::Acquire => "current child root capacity member acquired",
        }
    }
}

/// Internal immutable receipt, projected only at the existing operation-specific port boundary.
pub(crate) struct TransferReceipt {
    pub revision: u64,
    pub epoch: u64,
    pub active_members: u32,
    pub possession: CapacityPossession,
    pub transitioned_at: OffsetDateTime,
    pub audit_id: AuditId,
    pub outbox_id: Uuid,
    pub replayed: bool,
}

#[derive(Debug)]
pub(crate) enum TransferError {
    Shared(Error),
    Occupied,
    NotMovable,
}
impl From<Error> for TransferError {
    fn from(error: Error) -> Self {
        Self::Shared(error)
    }
}
impl From<StoreError> for TransferError {
    fn from(error: StoreError) -> Self {
        Self::Shared(Error::Storage(error))
    }
}
pub(crate) fn vacate_error(error: TransferError) -> Error {
    match error {
        TransferError::Shared(error) => error,
        TransferError::Occupied | TransferError::NotMovable => Error::ContradictoryProjection,
    }
}
pub(crate) fn acquire_error(error: TransferError) -> CapacityAcquireError {
    match error {
        TransferError::Occupied => CapacityAcquireError::Occupied,
        TransferError::NotMovable => CapacityAcquireError::NotMovable,
        TransferError::Shared(error) => match error {
            Error::Busy => CapacityAcquireError::Busy,
            Error::Missing => CapacityAcquireError::Missing,
            Error::BindingMismatch => CapacityAcquireError::BindingMismatch,
            Error::StaleHead { current } => CapacityAcquireError::StaleHead { current },
            Error::StaleGroupRevision { current } => {
                CapacityAcquireError::StaleGroupRevision { current }
            }
            Error::StaleGroupEpoch { current } => CapacityAcquireError::StaleGroupEpoch { current },
            Error::StaleChainRevision { current } => {
                CapacityAcquireError::StaleChainRevision { current }
            }
            Error::StaleRunRevision { current } => {
                CapacityAcquireError::StaleRunRevision { current }
            }
            Error::SchedulerFenceLost => CapacityAcquireError::SchedulerFenceLost,
            Error::NotHolder | Error::NotQuiescent | Error::ContradictoryProjection => {
                CapacityAcquireError::ContradictoryProjection
            }
            Error::InactiveMember => CapacityAcquireError::InactiveMember,
            Error::ChainFinal => CapacityAcquireError::ChainFinal,
            Error::FingerprintConflict => CapacityAcquireError::FingerprintConflict,
            Error::CounterExhausted => CapacityAcquireError::CounterExhausted,
            Error::Storage(error) => CapacityAcquireError::Storage(error),
            Error::CommitOutcomeUnknown { operation_key } => {
                CapacityAcquireError::CommitOutcomeUnknown { operation_key }
            }
        },
    }
}

fn storage(error: sqlx::Error) -> Error {
    Error::Storage(store_error(error))
}
fn unsigned(value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| Error::ContradictoryProjection)
}
fn sql_counter(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::CounterExhausted)
}

pub(crate) async fn transfer(
    pool: &PgPool,
    operation: TransferOperation,
    operation_key: Uuid,
    identity: CapacityFinishIdentity,
    expected: CapacityFinishExpectations,
    scheduler: &SchedulerAuthority,
    fingerprint: [u8; 32],
) -> Result<TransferReceipt, TransferError> {
    let mut tx = pool.begin_safely().await.map_err(storage)?;
    let result = match apply(
        &mut tx,
        operation,
        operation_key,
        identity,
        expected,
        scheduler,
        fingerprint,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            tx.rollback().await.map_err(storage)?;
            return Err(error);
        }
    };
    // Cancellation after COMMIT dispatch has no returned outcome. Retain this original key.
    match tx.commit().await {
        Ok(()) => Ok(result),
        Err(error) if confirmed_abort(&error) => Err(storage(error).into()),
        Err(_) => Err(Error::CommitOutcomeUnknown { operation_key }.into()),
    }
}

async fn member(
    tx: &mut Transaction<'_, Postgres>,
    identity: CapacityFinishIdentity,
) -> Result<PgRow, Error> {
    let i = identity;
    let row=sqlx::query("SELECT namespace_id,member_kind,chain_id,state,finished_at FROM capacity_group_members WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).bind(i.member_id.as_uuid())
        .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::Missing)?;
    if row.get::<Uuid, _>("namespace_id") != i.scope.namespace_id().as_uuid()
        || row.get::<String, _>("member_kind") != "CHILD_ROOT"
        || row.get::<Uuid, _>("chain_id") != i.chain_id.as_uuid()
    {
        return Err(Error::BindingMismatch);
    }
    Ok(row)
}

async fn apply(
    tx: &mut Transaction<'_, Postgres>,
    operation: TransferOperation,
    operation_key: Uuid,
    identity: CapacityFinishIdentity,
    expected: CapacityFinishExpectations,
    scheduler: &SchedulerAuthority,
    fingerprint: [u8; 32],
) -> Result<TransferReceipt, TransferError> {
    // Command counters must fit the persistence contract, including refused/replayed attempts.
    for counter in [
        expected.group_revision,
        expected.group_epoch,
        expected.chain_revision,
        expected.run_revision,
    ] {
        sql_counter(counter)?;
    }
    let i = identity;
    member(tx, identity).await?; // Discovery only; repeat under G before trusting registry state.
    // Lock the discovered exact chain directly: a mismatched supplied run must never redirect
    // the lock to another chain. No history, steps, parent business run or other group is loaded.
    let chain=sqlx::query("SELECT current_run_id,revision,final_outcome FROM execution_chains WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR UPDATE")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.scope.namespace_id().as_uuid()).bind(i.chain_id.as_uuid())
        .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::Missing)?;
    let current = RunId::new(chain.get("current_run_id"));
    let segment=sqlx::query("SELECT completion_kind,successor_run_id,completed_at FROM chain_segments WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 AND run_id=$4")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.scope.namespace_id().as_uuid()).bind(i.chain_id.as_uuid()).bind(i.head_run_id.as_uuid())
        .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::BindingMismatch)?;
    let run=sqlx::query("SELECT status,run_version,finished_at,parent_run_id,trace_id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.scope.namespace_id().as_uuid()).bind(i.head_run_id.as_uuid())
        .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::Missing)?;
    let header=sqlx::query("SELECT revision,epoch,active_members,possession,holder_member_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE NOWAIT")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).fetch_optional(&mut **tx).await
        .map_err(|error|if error.as_database_error().and_then(|db|db.code()).as_deref()==Some("55P03") {Error::Busy}else{storage(error)})?
        .ok_or(Error::Missing)?;
    let selected = member(tx, identity).await?;
    let now = locked_scheduler_time(tx, i.head_run_id, scheduler.owner(), scheduler.lease())
        .await?
        .ok_or(Error::SchedulerFenceLost)?;
    let parent:Option<Uuid>=sqlx::query_scalar("SELECT parent_run_id FROM child_chain_links WHERE tenant_id=$1 AND namespace_id=$2 AND child_chain_id=$3")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.scope.namespace_id().as_uuid()).bind(i.chain_id.as_uuid())
        .fetch_optional(&mut **tx).await.map_err(storage)?;
    if parent.is_none() || parent != run.get::<Option<Uuid>, _>("parent_run_id") {
        return Err(Error::BindingMismatch.into());
    }
    if let Some(row)=sqlx::query("SELECT kind,fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,resulting_possession,resulting_holder_member_id,transitioned_at,audit_id,outbox_id FROM capacity_group_transfer_receipts WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).bind(operation_key)
        .fetch_optional(&mut **tx).await.map_err(storage)? {
        if row.get::<Vec<u8>,_>("fingerprint")!=fingerprint {return Err(Error::FingerprintConflict.into());}
        return replay(&row, operation, identity, expected).map_err(TransferError::Shared);
    }
    if current != i.head_run_id {
        return Err(Error::StaleHead { current }.into());
    }
    let revision = unsigned(header.get("revision"))?;
    let epoch = unsigned(header.get("epoch"))?;
    if revision != expected.group_revision {
        return Err(Error::StaleGroupRevision { current: revision }.into());
    }
    if epoch != expected.group_epoch {
        return Err(Error::StaleGroupEpoch { current: epoch }.into());
    }
    let chain_revision = unsigned(chain.get("revision"))?;
    if chain_revision != expected.chain_revision {
        return Err(Error::StaleChainRevision {
            current: chain_revision,
        }
        .into());
    }
    let run_revision = unsigned(run.get("run_version"))?;
    if run_revision != expected.run_revision {
        return Err(Error::StaleRunRevision {
            current: run_revision,
        }
        .into());
    }
    if selected.get::<String, _>("state") != "ACTIVE" {
        return Err(Error::InactiveMember.into());
    }
    if selected
        .get::<Option<OffsetDateTime>, _>("finished_at")
        .is_some()
    {
        return Err(Error::ContradictoryProjection.into());
    }
    let possession = possession(
        header.get::<String, _>("possession").as_str(),
        header.get("holder_member_id"),
    )?;
    let holder = if let CapacityPossession::Held(holder) = possession {
        let fact = if holder == i.member_id {
            selected
        } else {
            sqlx::query("SELECT state,finished_at FROM capacity_group_members WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3")
                .bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).bind(holder.as_uuid())
                .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::ContradictoryProjection)?
        };
        if fact.get::<String, _>("state") != "ACTIVE"
            || fact
                .get::<Option<OffsetDateTime>, _>("finished_at")
                .is_some()
        {
            return Err(Error::ContradictoryProjection.into());
        }
        Some(CapacityMemberSnapshot::new(
            i.group_id,
            holder,
            CapacityMembership::Active,
        ))
    } else {
        None
    };
    let count = u32::try_from(header.get::<i64, _>("active_members"))
        .map_err(|_| Error::ContradictoryProjection)?;
    let mut group = CapacityGroup::restore(
        i.group_id,
        i.scope.tenant_id(),
        revision,
        epoch,
        count,
        possession,
        holder,
    )
    .map_err(|error| domain_error(operation, error))?;
    match operation {
        TransferOperation::Vacate => {
            if group.possession() != CapacityPossession::Held(i.member_id) {
                return Err(Error::NotHolder.into());
            }
        }
        TransferOperation::Acquire => match group.possession() {
            CapacityPossession::Held(_) => return Err(TransferError::Occupied),
            CapacityPossession::Retired => return Err(Error::InactiveMember.into()),
            CapacityPossession::Vacant => {}
        },
    }
    let status = non_final(&chain, &run, &segment)?;
    let selected_member =
        CapacityMemberSnapshot::new(i.group_id, i.member_id, CapacityMembership::Active);
    match operation {
        TransferOperation::Vacate => {
            // Only Vacate needs this structural evidence. Expiry alone is not quiescence.
            let leased: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM steps WHERE tenant_id=$1 AND run_id=$2 AND status IN ('LEASED','RUNNING'))")
                .bind(i.scope.tenant_id().as_uuid()).bind(i.head_run_id.as_uuid())
                .fetch_one(&mut **tx).await.map_err(storage)?;
            vacate_gate(status, leased).map_err(|error| match error {
                CapacityTransferRefusal::HeadFinal => Error::ChainFinal,
                CapacityTransferRefusal::NotQuiescent => Error::NotQuiescent,
                CapacityTransferRefusal::NotMovable => Error::ContradictoryProjection,
            })?;
            group
                .vacate(revision, epoch, selected_member)
                .map_err(|error| domain_error(operation, error))?;
        }
        TransferOperation::Acquire => {
            // Possession does not assert runnable work and does not lease or release a step.
            acquire_gate(status).map_err(|error| match error {
                CapacityTransferRefusal::HeadFinal => TransferError::Shared(Error::ChainFinal),
                CapacityTransferRefusal::NotQuiescent => {
                    TransferError::Shared(Error::ContradictoryProjection)
                }
                CapacityTransferRefusal::NotMovable => TransferError::NotMovable,
            })?;
            group
                .acquire(revision, epoch, selected_member)
                .map_err(|error| domain_error(operation, error))?;
        }
    }
    let revision_sql = sql_counter(group.revision())?;
    let epoch_sql = sql_counter(group.epoch())?;
    let (possession, holder) = match group.possession() {
        CapacityPossession::Vacant => ("VACANT", None),
        CapacityPossession::Held(member) => ("HELD", Some(member.as_uuid())),
        CapacityPossession::Retired => return Err(Error::ContradictoryProjection.into()),
    };
    let audit = AuditRecord::engine(
        i.scope.tenant_id(),
        operation.audit_action(),
        ResourceType::CapacityGroup,
        i.group_id.to_string(),
        operation.audit_message(),
        run.get("trace_id"),
        now,
    );
    let outbox = Uuid::now_v7();
    let changed=sqlx::query("UPDATE capacity_groups SET revision=$3,epoch=$4,possession=$5,holder_member_id=$6 WHERE tenant_id=$1 AND group_id=$2 AND revision=$7 AND epoch=$8")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).bind(revision_sql).bind(epoch_sql)
        .bind(possession).bind(holder).bind(sql_counter(revision)?).bind(sql_counter(epoch)?)
        .execute(&mut **tx).await.map_err(storage)?.rows_affected();
    if changed != 1 {
        return Err(Error::ContradictoryProjection.into());
    }
    insert_audit(tx, &audit).await?;
    sqlx::query("INSERT INTO event_outbox(id,tenant_id,aggregate_id,event_type,payload) VALUES($1,$2,$3,$4,$5)")
        .bind(outbox).bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).bind(operation.event_type())
        .bind(serde_json::json!({"tenantId":i.scope.tenant_id().to_string(),"namespaceId":i.scope.namespace_id().to_string(),
            "groupId":i.group_id.to_string(),"memberId":i.member_id.to_string(),"chainId":i.chain_id.to_string(),
            "headRunId":i.head_run_id.to_string(),"resultingRevision":group.revision(),"epoch":group.epoch(),
            "traceId":run.get::<Option<String>,_>("trace_id")}))
        .execute(&mut **tx).await.map_err(storage)?;
    sqlx::query("INSERT INTO capacity_group_transfer_receipts(kind,tenant_id,group_id,operation_key,fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,resulting_possession,resulting_holder_member_id,transitioned_at,audit_id,outbox_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)")
        .bind(operation.kind()).bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).bind(operation_key).bind(fingerprint.as_slice())
        .bind(i.scope.namespace_id().as_uuid()).bind(i.member_id.as_uuid()).bind(i.chain_id.as_uuid()).bind(i.head_run_id.as_uuid())
        .bind(sql_counter(expected.group_revision)?).bind(sql_counter(expected.group_epoch)?).bind(sql_counter(chain_revision)?)
        .bind(sql_counter(run_revision)?).bind(revision_sql).bind(epoch_sql).bind(i64::from(group.active_members())).bind(possession)
        .bind(holder).bind(now).bind(audit.id().as_uuid()).bind(outbox).execute(&mut **tx).await.map_err(storage)?;
    Ok(TransferReceipt {
        revision: group.revision(),
        epoch: group.epoch(),
        active_members: group.active_members(),
        possession: group.possession(),
        transitioned_at: now,
        audit_id: audit.id(),
        outbox_id: outbox,
        replayed: false,
    })
}
fn non_final(chain: &PgRow, run: &PgRow, segment: &PgRow) -> Result<RunStatus, Error> {
    let kind: Option<String> = segment.get("completion_kind");
    let successor: Option<Uuid> = segment.get("successor_run_id");
    let completed: Option<OffsetDateTime> = segment.get("completed_at");
    let finished: Option<OffsetDateTime> = run.get("finished_at");
    let outcome: Option<String> = chain.get("final_outcome");
    let text: String = run.get("status");
    let status: RunStatus = text.parse().map_err(|_| Error::ContradictoryProjection)?;
    if kind.as_deref() == Some("FINAL")
        && successor.is_none()
        && completed.is_some()
        && finished.is_some()
        && status.is_terminal()
        && outcome.as_deref() == Some(text.as_str())
    {
        return Err(Error::ChainFinal);
    }
    if kind.is_none()
        && successor.is_none()
        && completed.is_none()
        && finished.is_none()
        && outcome.is_none()
        && !status.is_terminal()
    {
        return Ok(status);
    }
    // A historical CONTINUED segment may replay its receipt, but cannot be a fresh current head.
    Err(Error::ContradictoryProjection)
}
fn possession(value: &str, holder: Option<Uuid>) -> Result<CapacityPossession, Error> {
    match (value, holder) {
        ("HELD", Some(holder)) => Ok(CapacityPossession::Held(CapacityMemberId::new(holder))),
        ("VACANT", None) => Ok(CapacityPossession::Vacant),
        ("RETIRED", None) => Ok(CapacityPossession::Retired),
        _ => Err(Error::ContradictoryProjection),
    }
}
fn domain_error(operation: TransferOperation, error: CapacityGroupError) -> TransferError {
    match error {
        CapacityGroupError::StaleRevision { current } => {
            Error::StaleGroupRevision { current }.into()
        }
        CapacityGroupError::StaleEpoch { current } => Error::StaleGroupEpoch { current }.into(),
        CapacityGroupError::CounterExhausted => Error::CounterExhausted.into(),
        CapacityGroupError::InactiveMember | CapacityGroupError::Retired => {
            Error::InactiveMember.into()
        }
        CapacityGroupError::NotHolder => match operation {
            TransferOperation::Vacate => Error::NotHolder.into(),
            TransferOperation::Acquire => Error::ContradictoryProjection.into(),
        },
        CapacityGroupError::Occupied => match operation {
            TransferOperation::Vacate => Error::ContradictoryProjection.into(),
            TransferOperation::Acquire => TransferError::Occupied,
        },
        CapacityGroupError::ForeignGroup
        | CapacityGroupError::MemberAlreadyKnown
        | CapacityGroupError::InconsistentSnapshot => Error::ContradictoryProjection.into(),
    }
}

fn replay(
    row: &PgRow,
    operation: TransferOperation,
    identity: CapacityFinishIdentity,
    expected: CapacityFinishExpectations,
) -> Result<TransferReceipt, Error> {
    let i = identity;
    if row.get::<String, _>("kind") != operation.kind()
        || row.get::<Uuid, _>("namespace_id") != i.scope.namespace_id().as_uuid()
        || row.get::<Uuid, _>("member_id") != i.member_id.as_uuid()
        || row.get::<Uuid, _>("chain_id") != i.chain_id.as_uuid()
        || row.get::<Uuid, _>("head_run_id") != i.head_run_id.as_uuid()
        || unsigned(row.get("expected_group_revision"))? != expected.group_revision
        || unsigned(row.get("expected_group_epoch"))? != expected.group_epoch
        || unsigned(row.get("accepted_chain_revision"))? != expected.chain_revision
        || unsigned(row.get("accepted_run_revision"))? != expected.run_revision
    {
        return Err(Error::ContradictoryProjection);
    }
    let revision = unsigned(row.get("resulting_revision"))?;
    let epoch = unsigned(row.get("resulting_epoch"))?;
    let active_members = u32::try_from(row.get::<i64, _>("resulting_active_members"))
        .map_err(|_| Error::ContradictoryProjection)?;
    let possession = possession(
        row.get::<String, _>("resulting_possession").as_str(),
        row.get("resulting_holder_member_id"),
    )?;
    if Some(revision) != expected.group_revision.checked_add(1)
        || Some(epoch) != expected.group_epoch.checked_add(1)
        || active_members == 0
        || u64::from(active_members) > revision
        || possession
            != match operation {
                TransferOperation::Vacate => CapacityPossession::Vacant,
                TransferOperation::Acquire => CapacityPossession::Held(i.member_id),
            }
    {
        return Err(Error::ContradictoryProjection);
    }
    Ok(TransferReceipt {
        revision,
        epoch,
        active_members,
        possession,
        transitioned_at: row.get("transitioned_at"),
        audit_id: AuditId::new(row.get("audit_id")),
        outbox_id: row.get("outbox_id"),
        replayed: true,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_domain_refusal_preserves_each_ports_explicit_meaning() {
        for (input, vacate, acquire) in [
            (
                CapacityGroupError::Retired,
                Error::InactiveMember,
                CapacityAcquireError::InactiveMember,
            ),
            (
                CapacityGroupError::StaleRevision { current: 9 },
                Error::StaleGroupRevision { current: 9 },
                CapacityAcquireError::StaleGroupRevision { current: 9 },
            ),
            (
                CapacityGroupError::StaleEpoch { current: 8 },
                Error::StaleGroupEpoch { current: 8 },
                CapacityAcquireError::StaleGroupEpoch { current: 8 },
            ),
            (
                CapacityGroupError::ForeignGroup,
                Error::ContradictoryProjection,
                CapacityAcquireError::ContradictoryProjection,
            ),
            (
                CapacityGroupError::MemberAlreadyKnown,
                Error::ContradictoryProjection,
                CapacityAcquireError::ContradictoryProjection,
            ),
            (
                CapacityGroupError::InactiveMember,
                Error::InactiveMember,
                CapacityAcquireError::InactiveMember,
            ),
            (
                CapacityGroupError::NotHolder,
                Error::NotHolder,
                CapacityAcquireError::ContradictoryProjection,
            ),
            (
                CapacityGroupError::Occupied,
                Error::ContradictoryProjection,
                CapacityAcquireError::Occupied,
            ),
            (
                CapacityGroupError::InconsistentSnapshot,
                Error::ContradictoryProjection,
                CapacityAcquireError::ContradictoryProjection,
            ),
            (
                CapacityGroupError::CounterExhausted,
                Error::CounterExhausted,
                CapacityAcquireError::CounterExhausted,
            ),
        ] {
            assert_eq!(
                vacate_error(domain_error(TransferOperation::Vacate, input)),
                vacate
            );
            assert_eq!(
                acquire_error(domain_error(TransferOperation::Acquire, input)),
                acquire
            );
        }
    }
    #[test]
    fn acquire_only_refusals_do_not_widen_the_vacate_error_contract() {
        assert_eq!(
            vacate_error(TransferError::Occupied),
            Error::ContradictoryProjection
        );
        assert_eq!(
            vacate_error(TransferError::NotMovable),
            Error::ContradictoryProjection
        );
        assert_eq!(
            acquire_error(TransferError::NotMovable),
            CapacityAcquireError::NotMovable
        );
        assert_eq!(
            acquire_error(TransferError::Shared(Error::NotHolder)),
            CapacityAcquireError::ContradictoryProjection
        );
        assert_eq!(
            acquire_error(TransferError::Shared(Error::NotQuiescent)),
            CapacityAcquireError::ContradictoryProjection
        );
    }
}
