//! Dormant exact ChildRoot Finish. No creator or scheduler call site is enabled.

use sagat_application::capacity_finish::{
    CapacityFinishCommand, CapacityFinishError as Error, CapacityFinishResult,
    CurrentChildRootFinishStore,
};
use sagat_domain::audit::{AuditAction, AuditId, AuditRecord, ResourceType};
use sagat_domain::execution::{
    CapacityGroup, CapacityGroupError, CapacityMemberId, CapacityMemberSnapshot,
    CapacityMembership, CapacityPossession, RunId,
};
use sagat_telemetry::{CAPACITY_FINISH_SPAN, CapacityFinishOutcome};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgRow};
use time::OffsetDateTime;
use tracing::Instrument;
use uuid::Uuid;

use crate::{
    execution::locked_scheduler_time,
    identity::{insert_audit, store_error},
    pool::BeginSafely,
};

#[derive(Clone)]
pub struct PgCapacityFinishStore {
    pool: PgPool,
}
impl PgCapacityFinishStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
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

impl CurrentChildRootFinishStore for PgCapacityFinishStore {
    async fn finish_current_child_root(
        &self,
        command: &CapacityFinishCommand,
    ) -> Result<CapacityFinishResult, Error> {
        let i = command.identity;
        // Inherits a caller's trace. Untraced direct calls still emit metrics but no OTLP span;
        // a future production caller must establish trace context before entering this port.
        let span = tracing::info_span!(
            CAPACITY_FINISH_SPAN,
            sagat.tenant.id = %i.scope.tenant_id(),
            sagat.namespace.id = %i.scope.namespace_id(),
            sagat.capacity.group_id = %i.group_id,
            sagat.capacity.member_id = %i.member_id,
            sagat.chain.id = %i.chain_id,
            sagat.run.id = %i.head_run_id,
            sagat.outcome = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
        );
        let entered = span.clone();
        async {
            let started = std::time::Instant::now();
            let result = self.finish_transaction(command).await;
            let outcome = match &result {
                Ok(result) if result.replayed => CapacityFinishOutcome::Replayed,
                Ok(_) => CapacityFinishOutcome::Applied,
                Err(Error::Busy) => CapacityFinishOutcome::Busy,
                Err(Error::Storage(_)) => CapacityFinishOutcome::StorageError,
                Err(Error::CommitOutcomeUnknown { .. }) => CapacityFinishOutcome::Unknown,
                Err(
                    Error::Missing
                    | Error::BindingMismatch
                    | Error::StaleHead { .. }
                    | Error::StaleGroupRevision { .. }
                    | Error::StaleGroupEpoch { .. }
                    | Error::StaleChainRevision { .. }
                    | Error::StaleRunRevision { .. }
                    | Error::SchedulerFenceLost
                    | Error::NotFinal
                    | Error::ContradictoryProjection
                    | Error::InactiveMember
                    | Error::FingerprintConflict
                    | Error::CounterExhausted,
                ) => CapacityFinishOutcome::Refused,
            };
            // A disabled child leaves the caller current. Record only on this exact handle.
            span.record("sagat.outcome", outcome.as_str());
            if matches!(
                outcome,
                CapacityFinishOutcome::StorageError | CapacityFinishOutcome::Unknown
            ) {
                span.record("otel.status_code", "error");
            }
            outcome.record(started.elapsed());
            tracing::debug!(
                outcome = outcome.as_str(),
                "capacity Finish attempt completed"
            );
            result
        }
        .instrument(entered)
        .await
    }
}

impl PgCapacityFinishStore {
    async fn finish_transaction(
        &self,
        command: &CapacityFinishCommand,
    ) -> Result<CapacityFinishResult, Error> {
        let mut tx = self.pool.begin_safely().await.map_err(storage)?;
        let result = match apply(&mut tx, command).await {
            Ok(result) => result,
            Err(error) => {
                tx.rollback().await.map_err(storage)?;
                return Err(error);
            }
        };
        // A dropped future after dispatch has no returned outcome. Callers must reconcile the
        // retained original command even when cancellation occurred while this await was pending.
        match tx.commit().await {
            Ok(()) => Ok(result),
            Err(error) if confirmed_abort(&error) => Err(storage(error)),
            Err(_) => Err(Error::CommitOutcomeUnknown {
                operation_key: command.operation_key,
            }),
        }
    }
}

// Conservatively recognize server-confirmed constraint/serialization/deadlock aborts only.
// Transport/protocol/cancellation and transaction-resolution-unknown never imply rollback.
pub(crate) fn confirmed_abort(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|db| db.code())
        .is_some_and(|code| code.starts_with("23") || matches!(code.as_ref(), "40001" | "40P01"))
}

async fn member(
    tx: &mut Transaction<'_, Postgres>,
    c: &CapacityFinishCommand,
) -> Result<PgRow, Error> {
    let i = c.identity;
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
    c: &CapacityFinishCommand,
) -> Result<CapacityFinishResult, Error> {
    // Command counters must fit the persistence contract, including refused/replayed attempts.
    for counter in [
        c.expected.group_revision,
        c.expected.group_epoch,
        c.expected.chain_revision,
        c.expected.run_revision,
    ] {
        sql_counter(counter)?;
    }
    let i = c.identity;
    member(tx, c).await?; // Discovery only; repeat under G before trusting registry state.
    // Lock the discovered exact chain directly: a mismatched supplied run must never redirect
    // the lock to another chain. No history, steps, parent business run or other group is loaded.
    let chain=sqlx::query("SELECT current_run_id,revision,final_outcome FROM execution_chains WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR UPDATE")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.scope.namespace_id().as_uuid()).bind(i.chain_id.as_uuid())
        .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::Missing)?;
    let current = RunId::new(chain.get("current_run_id"));
    if current != i.head_run_id {
        return Err(Error::StaleHead { current });
    }
    let segment=sqlx::query("SELECT completion_kind,successor_run_id,completed_at FROM chain_segments WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 AND run_id=$4")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.scope.namespace_id().as_uuid()).bind(i.chain_id.as_uuid()).bind(i.head_run_id.as_uuid())
        .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::ContradictoryProjection)?;
    let run=sqlx::query("SELECT status,run_version,finished_at,parent_run_id,trace_id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.scope.namespace_id().as_uuid()).bind(i.head_run_id.as_uuid())
        .fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(Error::Missing)?;
    let header=sqlx::query("SELECT revision,epoch,active_members,possession,holder_member_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE NOWAIT")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).fetch_optional(&mut **tx).await
        .map_err(|error|if error.as_database_error().and_then(|db|db.code()).as_deref()==Some("55P03") {Error::Busy}else{storage(error)})?
        .ok_or(Error::Missing)?;
    let selected = member(tx, c).await?;
    let now = locked_scheduler_time(tx, i.head_run_id, c.scheduler.owner(), c.scheduler.lease())
        .await?
        .ok_or(Error::SchedulerFenceLost)?;
    let parent:Option<Uuid>=sqlx::query_scalar("SELECT parent_run_id FROM child_chain_links WHERE tenant_id=$1 AND namespace_id=$2 AND child_chain_id=$3")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.scope.namespace_id().as_uuid()).bind(i.chain_id.as_uuid())
        .fetch_optional(&mut **tx).await.map_err(storage)?;
    if parent.is_none() || parent != run.get::<Option<Uuid>, _>("parent_run_id") {
        return Err(Error::BindingMismatch);
    }
    finality(&chain, &run, &segment)?;
    let fingerprint = c.fingerprint();
    if let Some(row)=sqlx::query("SELECT fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,resulting_possession,resulting_holder_member_id,finished_at,audit_id,outbox_id FROM capacity_group_finish_receipts WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).bind(c.operation_key)
        .fetch_optional(&mut **tx).await.map_err(storage)? {
        if row.get::<Vec<u8>,_>("fingerprint")!=fingerprint {return Err(Error::FingerprintConflict);}
        return replay(&row,c);
    }
    let revision = unsigned(header.get("revision"))?;
    let epoch = unsigned(header.get("epoch"))?;
    if revision != c.expected.group_revision {
        return Err(Error::StaleGroupRevision { current: revision });
    }
    if epoch != c.expected.group_epoch {
        return Err(Error::StaleGroupEpoch { current: epoch });
    }
    let chain_revision = unsigned(chain.get("revision"))?;
    if chain_revision != c.expected.chain_revision {
        return Err(Error::StaleChainRevision {
            current: chain_revision,
        });
    }
    let run_revision = unsigned(run.get("run_version"))?;
    if run_revision != c.expected.run_revision {
        return Err(Error::StaleRunRevision {
            current: run_revision,
        });
    }
    if selected.get::<String, _>("state") != "ACTIVE" {
        return Err(Error::InactiveMember);
    }
    if selected
        .get::<Option<OffsetDateTime>, _>("finished_at")
        .is_some()
    {
        return Err(Error::ContradictoryProjection);
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
            return Err(Error::ContradictoryProjection);
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
    .map_err(domain_error)?;
    group
        .finish(
            revision,
            CapacityMemberSnapshot::new(i.group_id, i.member_id, CapacityMembership::Active),
        )
        .map_err(domain_error)?;
    let revision_sql = sql_counter(group.revision())?;
    let epoch_sql = sql_counter(group.epoch())?;
    let (possession, holder) = possession_sql(group.possession());
    let audit = AuditRecord::engine(
        i.scope.tenant_id(),
        AuditAction::CapacityMemberFinished,
        ResourceType::CapacityGroup,
        i.group_id.to_string(),
        "current child root capacity member finished",
        run.get("trace_id"),
        now,
    );
    let outbox = Uuid::now_v7();
    let changed=sqlx::query("UPDATE capacity_groups SET revision=$3,epoch=$4,active_members=$5,possession=$6,holder_member_id=$7 WHERE tenant_id=$1 AND group_id=$2 AND revision=$8 AND epoch=$9")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).bind(revision_sql).bind(epoch_sql)
        .bind(i64::from(group.active_members())).bind(possession).bind(holder).bind(sql_counter(revision)?).bind(sql_counter(epoch)?)
        .execute(&mut **tx).await.map_err(storage)?.rows_affected();
    if changed != 1 {
        return Err(Error::ContradictoryProjection);
    }
    let changed=sqlx::query("UPDATE capacity_group_members SET state='FINISHED',finished_at=$6 WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3 AND namespace_id=$4 AND chain_id=$5 AND member_kind='CHILD_ROOT' AND state='ACTIVE' AND finished_at IS NULL")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).bind(i.member_id.as_uuid()).bind(i.scope.namespace_id().as_uuid())
        .bind(i.chain_id.as_uuid()).bind(now).execute(&mut **tx).await.map_err(storage)?.rows_affected();
    if changed != 1 {
        return Err(Error::ContradictoryProjection);
    }
    insert_audit(tx, &audit).await?;
    sqlx::query("INSERT INTO event_outbox(id,tenant_id,aggregate_id,event_type,payload) VALUES($1,$2,$3,'capacity.member.finished',$4)")
        .bind(outbox).bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid())
        .bind(serde_json::json!({"tenantId":i.scope.tenant_id().to_string(),"namespaceId":i.scope.namespace_id().to_string(),
            "groupId":i.group_id.to_string(),"memberId":i.member_id.to_string(),"chainId":i.chain_id.to_string(),
            "headRunId":i.head_run_id.to_string(),"resultingRevision":group.revision(),"epoch":group.epoch(),
            "retired":group.possession()==CapacityPossession::Retired,"traceId":run.get::<Option<String>,_>("trace_id")}))
        .execute(&mut **tx).await.map_err(storage)?;
    sqlx::query("INSERT INTO capacity_group_finish_receipts(tenant_id,group_id,operation_key,fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,resulting_possession,resulting_holder_member_id,finished_at,audit_id,outbox_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)")
        .bind(i.scope.tenant_id().as_uuid()).bind(i.group_id.as_uuid()).bind(c.operation_key).bind(fingerprint.as_slice())
        .bind(i.scope.namespace_id().as_uuid()).bind(i.member_id.as_uuid()).bind(i.chain_id.as_uuid()).bind(i.head_run_id.as_uuid())
        .bind(sql_counter(c.expected.group_revision)?).bind(sql_counter(c.expected.group_epoch)?).bind(sql_counter(chain_revision)?)
        .bind(sql_counter(run_revision)?).bind(revision_sql).bind(epoch_sql).bind(i64::from(group.active_members())).bind(possession)
        .bind(holder).bind(now).bind(audit.id().as_uuid()).bind(outbox).execute(&mut **tx).await.map_err(storage)?;
    Ok(CapacityFinishResult {
        group_id: i.group_id,
        member_id: i.member_id,
        head_run_id: i.head_run_id,
        revision: group.revision(),
        epoch: group.epoch(),
        active_members: group.active_members(),
        possession: group.possession(),
        finished_at: now,
        audit_id: audit.id(),
        outbox_id: outbox,
        replayed: false,
    })
}

fn finality(chain: &PgRow, run: &PgRow, segment: &PgRow) -> Result<(), Error> {
    let kind: Option<String> = segment.get("completion_kind");
    let successor: Option<Uuid> = segment.get("successor_run_id");
    let completed: Option<OffsetDateTime> = segment.get("completed_at");
    let finished: Option<OffsetDateTime> = run.get("finished_at");
    let outcome: Option<String> = chain.get("final_outcome");
    let status: String = run.get("status");
    let terminal = matches!(
        status.as_str(),
        "SUCCEEDED"
            | "FAILED"
            | "CANCELLED"
            | "TIMED_OUT"
            | "COMPENSATED"
            | "PARTIAL"
            | "COMPENSATION_FAILED"
    );
    if kind.as_deref() == Some("FINAL")
        && successor.is_none()
        && completed.is_some()
        && finished.is_some()
        && terminal
        && outcome.as_deref() == Some(status.as_str())
    {
        return Ok(());
    }
    if kind.as_deref() == Some("CONTINUED")
        || (kind.is_none()
            && successor.is_none()
            && completed.is_none()
            && finished.is_none()
            && outcome.is_none()
            && !terminal)
    {
        return Err(Error::NotFinal);
    }
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
fn possession_sql(value: CapacityPossession) -> (&'static str, Option<Uuid>) {
    match value {
        CapacityPossession::Held(holder) => ("HELD", Some(holder.as_uuid())),
        CapacityPossession::Vacant => ("VACANT", None),
        CapacityPossession::Retired => ("RETIRED", None),
    }
}
fn domain_error(error: CapacityGroupError) -> Error {
    match error {
        CapacityGroupError::CounterExhausted => Error::CounterExhausted,
        CapacityGroupError::InactiveMember | CapacityGroupError::Retired => Error::InactiveMember,
        _ => Error::ContradictoryProjection,
    }
}
fn replay(row: &PgRow, c: &CapacityFinishCommand) -> Result<CapacityFinishResult, Error> {
    let i = c.identity;
    if row.get::<Uuid, _>("namespace_id") != i.scope.namespace_id().as_uuid()
        || row.get::<Uuid, _>("member_id") != i.member_id.as_uuid()
        || row.get::<Uuid, _>("chain_id") != i.chain_id.as_uuid()
        || row.get::<Uuid, _>("head_run_id") != i.head_run_id.as_uuid()
        || unsigned(row.get("expected_group_revision"))? != c.expected.group_revision
        || unsigned(row.get("expected_group_epoch"))? != c.expected.group_epoch
        || unsigned(row.get("accepted_chain_revision"))? != c.expected.chain_revision
        || unsigned(row.get("accepted_run_revision"))? != c.expected.run_revision
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
    if revision < 2
        || epoch == 0
        || epoch > revision
        || u64::from(active_members) > revision
        || (possession == CapacityPossession::Retired) != (active_members == 0)
    {
        return Err(Error::ContradictoryProjection);
    }
    Ok(CapacityFinishResult {
        group_id: i.group_id,
        member_id: i.member_id,
        head_run_id: i.head_run_id,
        revision,
        epoch,
        active_members,
        possession,
        finished_at: row.get("finished_at"),
        audit_id: AuditId::new(row.get("audit_id")),
        outbox_id: row.get("outbox_id"),
        replayed: true,
    })
}
