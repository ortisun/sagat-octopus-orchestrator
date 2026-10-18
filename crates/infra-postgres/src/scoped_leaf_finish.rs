//! Dormant exact OrdinaryLeafV1 scoped-path Finish. No caller or runtime grant is enabled.

use sagat_application::{
    error::StoreError,
    leaf_certification::LeafCollectionSuccessError as SourceError,
    scoped_leaf_finish::{
        ScopedLeafFinishCommand, ScopedLeafFinishError as Error, ScopedLeafFinishResult,
        ScopedLeafFinishStore,
    },
};
use sagat_domain::{
    audit::{AuditAction, AuditId, AuditRecord, ResourceType},
    definition::WorkflowVersionId,
    execution::{
        CapacityGroup, CapacityGroupError, CapacityMemberId, CapacityMemberSnapshot,
        CapacityMembership, CapacityPossession, MAX_ITERATION_INDEX,
    },
    workflow::NodeId,
};
use sagat_telemetry::{CAPACITY_FINISH_SPAN, CapacityFinishOutcome};
use sqlx::{Decode, PgPool, Postgres, Row, Transaction, Type, postgres::PgRow};
use time::OffsetDateTime;
use tracing::Instrument;
use uuid::Uuid;

use crate::{
    execution::{lock_member_chain_sql, locked_scheduler_time_sql},
    leaf_certification::read_canonical_leaf_success,
    pool::BeginSafely,
};

#[derive(Clone)]
pub struct PgScopedLeafFinishStore {
    pool: PgPool,
}

impl PgScopedLeafFinishStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl ScopedLeafFinishStore for PgScopedLeafFinishStore {
    async fn finish_scoped_leaf(
        &self,
        command: &ScopedLeafFinishCommand,
    ) -> Result<ScopedLeafFinishResult, Error> {
        let identity = &command.identity;
        // This operational span inherits the caller's trace. It carries only retained identity;
        // the renewable scheduler owner and source body never enter telemetry.
        let span = tracing::info_span!(
            CAPACITY_FINISH_SPAN,
            sagat.tenant.id = %identity.tenant_id(),
            sagat.namespace.id = %identity.namespace_id(),
            sagat.capacity.group_id = %command.group_id,
            sagat.capacity.member_id = %command.member_id,
            sagat.run.id = %identity.run_id(),
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
                    Error::UnsupportedPath
                    | Error::InvalidRevision
                    | Error::CounterExhausted
                    | Error::Missing
                    | Error::BindingMismatch
                    | Error::SchedulerFenceLost
                    | Error::UnsupportedRunContext
                    | Error::SourceEvidenceMissing
                    | Error::UnsupportedStoredProfile
                    | Error::ContradictorySource
                    | Error::FingerprintConflict
                    | Error::ContradictoryProjection
                    | Error::StaleGroupRevision { .. }
                    | Error::StaleGroupEpoch { .. }
                    | Error::StaleRunRevision { .. }
                    | Error::InactiveMember,
                ) => CapacityFinishOutcome::Refused,
            };
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

impl PgScopedLeafFinishStore {
    async fn finish_transaction(
        &self,
        command: &ScopedLeafFinishCommand,
    ) -> Result<ScopedLeafFinishResult, Error> {
        // Structural checks and the canonical logical-operation hash precede BEGIN.
        let fingerprint = command.fingerprint()?;
        let mut tx = self.pool.begin_safely().await.map_err(storage)?;
        let result = match apply(&mut tx, command, fingerprint).await {
            Ok(result) => result,
            Err(error) => {
                // A failed rollback replaces the earlier semantic result with a sanitized storage
                // result: the adapter can no longer state that the attempted transaction aborted.
                tx.rollback().await.map_err(storage)?;
                return Err(error);
            }
        };
        // Cancellation after COMMIT dispatch has no returned outcome. Reconciliation must retain
        // and replay the complete original command through this same entry point.
        match tx.commit().await {
            Ok(()) => Ok(result),
            Err(error) if confirmed_abort(&error) => Err(storage(error)),
            Err(_) => Err(Error::CommitOutcomeUnknown {
                operation_key: command.operation_key,
            }),
        }
    }
}

fn storage(error: sqlx::Error) -> Error {
    // Only a syntactically valid SQLSTATE crosses this boundary. Driver text, constraint detail,
    // SQL and bind values are deliberately discarded and are not logged here.
    let code = error
        .as_database_error()
        .and_then(|database| database.code())
        .filter(|code| code.len() == 5 && code.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    Error::Storage(StoreError::Internal {
        detail: code.map_or_else(
            || "scoped capacity finish storage operation failed".into(),
            |code| format!("scoped capacity finish storage failure (SQLSTATE {code})"),
        ),
    })
}

fn reader_contract_violation() -> Error {
    Error::Storage(StoreError::Internal {
        detail: "canonical leaf reader violated scoped capacity finish contract".into(),
    })
}

fn source_error(error: SourceError) -> Error {
    match error {
        SourceError::UnsupportedStoredProfile => Error::UnsupportedStoredProfile,
        SourceError::ContradictorySource => Error::ContradictorySource,
        SourceError::Storage(error) => Error::Storage(error),
        SourceError::UnsupportedPath
        | SourceError::Missing
        | SourceError::UnsupportedShape(_)
        | SourceError::SchedulerFenceLost
        | SourceError::StaleRunRevision { .. }
        | SourceError::SourceNotClosed
        | SourceError::SourceEvidenceMissing
        | SourceError::InvalidRevision
        | SourceError::CommitOutcomeUnknown { .. } => reader_contract_violation(),
    }
}

// A server constraint violation, serialization abort or deadlock is a confirmed abort. Transport,
// protocol, cancellation and unknown-resolution failures do not prove rollback.
fn confirmed_abort(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|database| database.code())
        .is_some_and(|code| code.starts_with("23") || matches!(code.as_ref(), "40001" | "40P01"))
}

fn projection<'row, T>(row: &'row PgRow, column: &str) -> Result<T, Error>
where
    T: Decode<'row, Postgres> + Type<Postgres>,
{
    row.try_get(column)
        .map_err(|_| Error::ContradictoryProjection)
}

fn source_projection<'row, T>(row: &'row PgRow, column: &str) -> Result<T, Error>
where
    T: Decode<'row, Postgres> + Type<Postgres>,
{
    row.try_get(column).map_err(|_| Error::ContradictorySource)
}

fn nonnegative(value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| Error::ContradictoryProjection)
}

fn source_nonnegative(value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| Error::ContradictorySource)
}

fn positive(value: i64) -> Result<u64, Error> {
    let value = nonnegative(value)?;
    if value == 0 {
        Err(Error::ContradictoryProjection)
    } else {
        Ok(value)
    }
}

fn sql_counter(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::CounterExhausted)
}

fn is_busy(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|database| database.code())
        .as_deref()
        == Some("55P03")
}

async fn member(
    tx: &mut Transaction<'_, Postgres>,
    command: &ScopedLeafFinishCommand,
) -> Result<PgRow, Error> {
    let identity = &command.identity;
    let row = sqlx::query(
        "SELECT member_kind,chain_id,original_run_id,root_owner_node,iteration_index,state,\
         finished_at IS NULL AS finished_at_is_null \
         FROM capacity_group_members \
         WHERE tenant_id=$1 AND namespace_id=$2 AND group_id=$3 AND member_id=$4",
    )
    .bind(identity.tenant_id().as_uuid())
    .bind(identity.namespace_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.member_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)?
    .ok_or(Error::Missing)?;
    check_member_binding(&row, command)?;
    Ok(row)
}

fn check_member_binding(row: &PgRow, command: &ScopedLeafFinishCommand) -> Result<(), Error> {
    let identity = &command.identity;
    let segment = &identity.path().segments()[0]; // fingerprint() proved this exact depth.
    let kind: String = projection(row, "member_kind")?;
    let chain: Option<Uuid> = projection(row, "chain_id")?;
    let original_run: Option<Uuid> = projection(row, "original_run_id")?;
    let owner: Option<String> = projection(row, "root_owner_node")?;
    let index: Option<i32> = projection(row, "iteration_index")?;

    match kind.as_str() {
        "CHILD_ROOT"
            if chain.is_some() && original_run.is_none() && owner.is_none() && index.is_none() =>
        {
            Err(Error::BindingMismatch)
        }
        "SCOPED_PATH" if chain.is_none() => {
            let (Some(original_run), Some(owner), Some(index)) = (original_run, owner, index)
            else {
                return Err(Error::ContradictoryProjection);
            };
            let index = u32::try_from(index).map_err(|_| Error::ContradictoryProjection)?;
            if index > MAX_ITERATION_INDEX || NodeId::parse(&owner).is_err() {
                return Err(Error::ContradictoryProjection);
            }
            if original_run != identity.run_id().as_uuid()
                || owner != segment.scope_node().as_str()
                || index != segment.iteration_index()
            {
                return Err(Error::BindingMismatch);
            }
            Ok(())
        }
        "CHILD_ROOT" | "SCOPED_PATH" => Err(Error::ContradictoryProjection),
        _ => Err(Error::ContradictoryProjection),
    }
}

async fn apply(
    tx: &mut Transaction<'_, Postgres>,
    command: &ScopedLeafFinishCommand,
    fingerprint: [u8; 32],
) -> Result<ScopedLeafFinishResult, Error> {
    let identity = &command.identity;
    let segment = &identity.path().segments()[0];

    // Candidate discovery is namespace-qualified. It grants no authority and is repeated under G.
    member(tx, command).await?;

    // Lock an applicable membership chain before the original Run. A flat run simply locks none.
    lock_member_chain_sql(
        tx,
        identity.tenant_id(),
        identity.namespace_id(),
        identity.run_id(),
    )
    .await
    .map_err(storage)?;

    let run = sqlx::query(
        "SELECT workflow_version_id,run_version,parent_run_id IS NULL AS parentless,\
         (trace_id IS NULL OR trace_id ~ '^[0-9a-f]{32}$') AS trace_valid,\
         CASE WHEN trace_id ~ '^[0-9a-f]{32}$' THEN trace_id END AS trace_id \
         FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE",
    )
    .bind(identity.tenant_id().as_uuid())
    .bind(identity.namespace_id().as_uuid())
    .bind(identity.run_id().as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)?
    .ok_or(Error::Missing)?;

    let header = sqlx::query(
        "SELECT revision,epoch,active_members,possession,holder_member_id \
         FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE NOWAIT",
    )
    .bind(identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| {
        if is_busy(&error) {
            Error::Busy
        } else {
            storage(error)
        }
    })?
    .ok_or(Error::Missing)?;

    // Registry authority is consumed only from this reread while G remains held.
    let selected = member(tx, command).await?;

    let now = locked_scheduler_time_sql(
        tx,
        identity.run_id(),
        command.scheduler.owner(),
        command.scheduler.lease(),
    )
    .await
    .map_err(storage)?
    .ok_or(Error::SchedulerFenceLost)?;

    let parentless: bool = projection(&run, "parentless")?;
    let chain_member: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM chain_segments \
         WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3)",
    )
    .bind(identity.tenant_id().as_uuid())
    .bind(identity.namespace_id().as_uuid())
    .bind(identity.run_id().as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(storage)?;
    if !parentless || chain_member {
        return Err(Error::UnsupportedRunContext);
    }

    // Decode source-owned Run pin fields only at the source stage, after the required lock,
    // binding, authority and flat-context precedence has been established.
    let workflow_version = WorkflowVersionId::new(source_projection(&run, "workflow_version_id")?);
    let run_revision = source_nonnegative(source_projection(&run, "run_version")?)?;

    // The canonical reader validates its exact profile, qualified key, structural metadata and
    // actual guarded definition/revision pin. It is a reader only and is never invoked as a writer.
    if read_canonical_leaf_success(tx, identity, workflow_version, run_revision)
        .await
        .map_err(source_error)?
        .is_none()
    {
        return Err(Error::SourceEvidenceMissing);
    }

    if let Some(receipt) = read_receipt(tx, command).await? {
        return replay(&receipt, command, fingerprint);
    }

    // Creation-only comparisons deliberately follow receipt lookup in this order.
    let revision = positive(projection(&header, "revision")?)?;
    if revision != command.expected.group_revision {
        return Err(Error::StaleGroupRevision { current: revision });
    }
    let epoch = positive(projection(&header, "epoch")?)?;
    if epoch != command.expected.group_epoch {
        return Err(Error::StaleGroupEpoch { current: epoch });
    }
    if run_revision != command.expected.run_revision {
        return Err(Error::StaleRunRevision {
            current: run_revision,
        });
    }

    check_selected_active(&selected)?;
    let possession = decode_possession(
        &projection::<String>(&header, "possession")?,
        projection(&header, "holder_member_id")?,
    )?;
    let holder_snapshot = match possession {
        CapacityPossession::Held(holder) => {
            if holder != command.member_id {
                let holder_row = sqlx::query(
                    "SELECT state,finished_at IS NULL AS finished_at_is_null \
                     FROM capacity_group_members \
                     WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3",
                )
                .bind(identity.tenant_id().as_uuid())
                .bind(command.group_id.as_uuid())
                .bind(holder.as_uuid())
                .fetch_optional(&mut **tx)
                .await
                .map_err(storage)?
                .ok_or(Error::ContradictoryProjection)?;
                check_holder_active(&holder_row)?;
            }
            // If selected holds possession, its ACTIVE fact was already decoded above.
            Some(CapacityMemberSnapshot::new(
                command.group_id,
                holder,
                CapacityMembership::Active,
            ))
        }
        CapacityPossession::Vacant | CapacityPossession::Retired => None,
    };
    let active_members = u32::try_from(projection::<i64>(&header, "active_members")?)
        .map_err(|_| Error::ContradictoryProjection)?;
    let mut group = CapacityGroup::restore(
        command.group_id,
        identity.tenant_id(),
        revision,
        epoch,
        active_members,
        possession,
        holder_snapshot,
    )
    .map_err(domain_error)?;
    group
        .finish(
            revision,
            CapacityMemberSnapshot::new(
                command.group_id,
                command.member_id,
                CapacityMembership::Active,
            ),
        )
        .map_err(domain_error)?;

    // Validate every domain-to-SQL conversion before the first DML statement.
    let expected_revision_sql = sql_counter(revision)?;
    let expected_epoch_sql = sql_counter(epoch)?;
    let accepted_run_revision_sql = sql_counter(run_revision)?;
    let resulting_revision_sql = sql_counter(group.revision())?;
    let resulting_epoch_sql = sql_counter(group.epoch())?;
    let resulting_active_members_sql = i64::from(group.active_members());
    let iteration_sql =
        i32::try_from(segment.iteration_index()).map_err(|_| Error::ContradictoryProjection)?;
    let (resulting_possession, resulting_holder) = possession_sql(group.possession());

    // Current trace is creation-only. The guarded SQL projection keeps malformed raw text out of
    // Rust; this explicit check prevents an invalid non-NULL value from becoming an absent trace.
    if !projection::<bool>(&run, "trace_valid")? {
        return Err(Error::ContradictoryProjection);
    }
    let trace: Option<String> = projection(&run, "trace_id")?;
    if trace.as_deref().is_some_and(|value| !valid_trace_id(value)) {
        return Err(Error::ContradictoryProjection);
    }

    let audit = AuditRecord::engine(
        identity.tenant_id(),
        AuditAction::CapacityMemberFinished,
        ResourceType::CapacityGroup,
        command.group_id.to_string(),
        "ordinary leaf path capacity member finished",
        trace.clone(),
        now,
    );
    let outbox_id = Uuid::now_v7();

    let changed = sqlx::query(
        "UPDATE capacity_groups \
         SET revision=$3,epoch=$4,active_members=$5,possession=$6,holder_member_id=$7 \
         WHERE tenant_id=$1 AND group_id=$2 AND revision=$8 AND epoch=$9",
    )
    .bind(identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(resulting_revision_sql)
    .bind(resulting_epoch_sql)
    .bind(resulting_active_members_sql)
    .bind(resulting_possession)
    .bind(resulting_holder)
    .bind(expected_revision_sql)
    .bind(expected_epoch_sql)
    .execute(&mut **tx)
    .await
    .map_err(storage)?
    .rows_affected();
    if changed != 1 {
        return Err(Error::ContradictoryProjection);
    }

    let changed = sqlx::query(
        "UPDATE capacity_group_members SET state='FINISHED',finished_at=$8 \
         WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3 AND namespace_id=$4 \
         AND member_kind='SCOPED_PATH' AND chain_id IS NULL AND original_run_id=$5 \
         AND root_owner_node=$6 AND iteration_index=$7 \
         AND state='ACTIVE' AND finished_at IS NULL",
    )
    .bind(identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.member_id.as_uuid())
    .bind(identity.namespace_id().as_uuid())
    .bind(identity.run_id().as_uuid())
    .bind(segment.scope_node().as_str())
    .bind(iteration_sql)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(storage)?
    .rows_affected();
    if changed != 1 {
        return Err(Error::ContradictoryProjection);
    }

    insert_audit(tx, &audit).await?;

    sqlx::query(
        "INSERT INTO event_outbox(id,tenant_id,aggregate_id,event_type,payload) \
         VALUES($1,$2,$3,'capacity.scoped_member.finished',$4)",
    )
    .bind(outbox_id)
    .bind(identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(serde_json::json!({
        "tenantId": identity.tenant_id().to_string(),
        "namespaceId": identity.namespace_id().to_string(),
        "groupId": command.group_id.to_string(),
        "memberId": command.member_id.to_string(),
        "originalRunId": identity.run_id().to_string(),
        "rootOwnerNode": segment.scope_node().as_str(),
        "iterationIndex": segment.iteration_index(),
        "resultingRevision": group.revision(),
        "epoch": group.epoch(),
        "retired": group.possession() == CapacityPossession::Retired,
        "traceId": trace,
    }))
    .execute(&mut **tx)
    .await
    .map_err(storage)?;

    sqlx::query(
        "INSERT INTO capacity_group_finish_receipts(\
         tenant_id,group_id,operation_key,fingerprint,namespace_id,member_id,member_kind,\
         chain_id,head_run_id,original_run_id,root_owner_node,iteration_index,\
         expected_group_revision,expected_group_epoch,accepted_chain_revision,\
         accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,\
         resulting_possession,resulting_holder_member_id,finished_at,audit_id,outbox_id) \
         VALUES($1,$2,$3,$4,$5,$6,'SCOPED_PATH',$7,$8,$9,$10,$11,$12,$13,$14,$15,\
         $16,$17,$18,$19,$20,$21,$22,$23)",
    )
    .bind(identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .bind(fingerprint.as_slice())
    .bind(identity.namespace_id().as_uuid())
    .bind(command.member_id.as_uuid())
    .bind(None::<Uuid>)
    .bind(None::<Uuid>)
    .bind(identity.run_id().as_uuid())
    .bind(segment.scope_node().as_str())
    .bind(iteration_sql)
    .bind(expected_revision_sql)
    .bind(expected_epoch_sql)
    .bind(None::<i64>)
    .bind(accepted_run_revision_sql)
    .bind(resulting_revision_sql)
    .bind(resulting_epoch_sql)
    .bind(resulting_active_members_sql)
    .bind(resulting_possession)
    .bind(resulting_holder)
    .bind(now)
    .bind(audit.id().as_uuid())
    .bind(outbox_id)
    .execute(&mut **tx)
    .await
    .map_err(storage)?;

    Ok(ScopedLeafFinishResult {
        group_id: command.group_id,
        member_id: command.member_id,
        original_run_id: identity.run_id(),
        revision: group.revision(),
        epoch: group.epoch(),
        active_members: group.active_members(),
        possession: group.possession(),
        finished_at: now,
        audit_id: audit.id(),
        outbox_id,
        replayed: false,
    })
}

async fn read_receipt(
    tx: &mut Transaction<'_, Postgres>,
    command: &ScopedLeafFinishCommand,
) -> Result<Option<PgRow>, Error> {
    let identity = &command.identity;
    sqlx::query(
        "SELECT fingerprint,member_kind,namespace_id,member_id,chain_id,head_run_id,\
         accepted_chain_revision,original_run_id,root_owner_node,iteration_index,\
         expected_group_revision,expected_group_epoch,accepted_run_revision,resulting_revision,\
         resulting_epoch,resulting_active_members,resulting_possession,resulting_holder_member_id,\
         CASE WHEN isfinite(finished_at) \
             AND finished_at<TIMESTAMPTZ '10000-01-01 00:00:00+00' \
             THEN finished_at END AS finished_at,audit_id,outbox_id \
         FROM capacity_group_finish_receipts \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)
}

fn replay(
    row: &PgRow,
    command: &ScopedLeafFinishCommand,
    fingerprint: [u8; 32],
) -> Result<ScopedLeafFinishResult, Error> {
    let stored: Vec<u8> = projection(row, "fingerprint")?;
    let stored: [u8; 32] = stored
        .try_into()
        .map_err(|_| Error::ContradictoryProjection)?;
    if stored != fingerprint {
        return Err(Error::FingerprintConflict);
    }

    let identity = &command.identity;
    let segment = &identity.path().segments()[0];
    let kind: String = projection(row, "member_kind")?;
    let namespace: Uuid = projection(row, "namespace_id")?;
    let member: Uuid = projection(row, "member_id")?;
    let chain: Option<Uuid> = projection(row, "chain_id")?;
    let head: Option<Uuid> = projection(row, "head_run_id")?;
    let accepted_chain: Option<i64> = projection(row, "accepted_chain_revision")?;
    let original_run: Option<Uuid> = projection(row, "original_run_id")?;
    let owner: Option<String> = projection(row, "root_owner_node")?;
    let index: Option<i32> = projection(row, "iteration_index")?;
    let (Some(original_run), Some(owner), Some(index)) = (original_run, owner, index) else {
        return Err(Error::ContradictoryProjection);
    };
    let index = u32::try_from(index).map_err(|_| Error::ContradictoryProjection)?;
    if kind != "SCOPED_PATH"
        || namespace != identity.namespace_id().as_uuid()
        || member != command.member_id.as_uuid()
        || chain.is_some()
        || head.is_some()
        || accepted_chain.is_some()
        || original_run != identity.run_id().as_uuid()
        || NodeId::parse(&owner).is_err()
        || owner != segment.scope_node().as_str()
        || index > MAX_ITERATION_INDEX
        || index != segment.iteration_index()
    {
        return Err(Error::ContradictoryProjection);
    }

    let expected_revision = positive(projection(row, "expected_group_revision")?)?;
    let expected_epoch = positive(projection(row, "expected_group_epoch")?)?;
    let accepted_run_revision = positive(projection(row, "accepted_run_revision")?)?;
    if expected_epoch > expected_revision
        || expected_revision != command.expected.group_revision
        || expected_epoch != command.expected.group_epoch
        || accepted_run_revision != command.expected.run_revision
    {
        return Err(Error::ContradictoryProjection);
    }

    let revision = positive(projection(row, "resulting_revision")?)?;
    let epoch = positive(projection(row, "resulting_epoch")?)?;
    let active_members = u32::try_from(projection::<i64>(row, "resulting_active_members")?)
        .map_err(|_| Error::ContradictoryProjection)?;
    let epoch_delta = epoch
        .checked_sub(expected_epoch)
        .ok_or(Error::ContradictoryProjection)?;
    if expected_revision.checked_add(1) != Some(revision)
        || epoch > revision
        || epoch_delta > 1
        || u64::from(active_members) > revision
    {
        return Err(Error::ContradictoryProjection);
    }

    let possession_text: String = projection(row, "resulting_possession")?;
    let holder_uuid: Option<Uuid> = projection(row, "resulting_holder_member_id")?;
    let possession = decode_possession(&possession_text, holder_uuid)?;
    let valid_result = match possession {
        CapacityPossession::Held(holder) => {
            active_members > 0 && holder != command.member_id && epoch == expected_epoch
        }
        CapacityPossession::Vacant => active_members > 0,
        CapacityPossession::Retired => {
            active_members == 0 && expected_epoch.checked_add(1) == Some(epoch)
        }
    };
    if !valid_result {
        return Err(Error::ContradictoryProjection);
    }

    let finished_at: Option<OffsetDateTime> = projection(row, "finished_at")?;
    let finished_at = finished_at.ok_or(Error::ContradictoryProjection)?;
    let audit_id: Uuid = projection(row, "audit_id")?;
    let outbox_id: Uuid = projection(row, "outbox_id")?;
    Ok(ScopedLeafFinishResult {
        group_id: command.group_id,
        member_id: command.member_id,
        original_run_id: identity.run_id(),
        revision,
        epoch,
        active_members,
        possession,
        finished_at,
        audit_id: AuditId::new(audit_id),
        outbox_id,
        replayed: true,
    })
}

fn check_selected_active(row: &PgRow) -> Result<(), Error> {
    let state: String = projection(row, "state")?;
    let finished_at_is_null: bool = projection(row, "finished_at_is_null")?;
    match (state.as_str(), finished_at_is_null) {
        ("ACTIVE", true) => Ok(()),
        ("FINISHED", false) => Err(Error::InactiveMember),
        ("ACTIVE" | "FINISHED", _) => Err(Error::ContradictoryProjection),
        _ => Err(Error::ContradictoryProjection),
    }
}

fn check_holder_active(row: &PgRow) -> Result<(), Error> {
    let state: String = projection(row, "state")?;
    let finished_at_is_null: bool = projection(row, "finished_at_is_null")?;
    if state == "ACTIVE" && finished_at_is_null {
        Ok(())
    } else {
        Err(Error::ContradictoryProjection)
    }
}

fn decode_possession(value: &str, holder: Option<Uuid>) -> Result<CapacityPossession, Error> {
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
        CapacityGroupError::ForeignGroup
        | CapacityGroupError::MemberAlreadyKnown
        | CapacityGroupError::NotHolder
        | CapacityGroupError::Occupied
        | CapacityGroupError::InconsistentSnapshot
        | CapacityGroupError::StaleRevision { .. }
        | CapacityGroupError::StaleEpoch { .. } => Error::ContradictoryProjection,
    }
}

fn valid_trace_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

async fn insert_audit(
    tx: &mut Transaction<'_, Postgres>,
    audit: &AuditRecord,
) -> Result<(), Error> {
    // This transition is fixed and noncollapsing. Keep raw driver text out of the generic audit
    // helper's logging path and classify the SQL failure locally.
    sqlx::query(
        "INSERT INTO audit_records(\
         id,tenant_id,actor,action,resource_type,resource_id,reason,trace_id,created_at) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(audit.id().as_uuid())
    .bind(audit.tenant_id().as_uuid())
    .bind(audit.actor().as_str())
    .bind(audit.action().as_str())
    .bind(audit.resource_type().as_str())
    .bind(audit.resource_id())
    .bind(audit.reason())
    .bind(audit.trace_id())
    .bind(audit.created_at())
    .execute(&mut **tx)
    .await
    .map_err(storage)?;
    Ok(())
}
