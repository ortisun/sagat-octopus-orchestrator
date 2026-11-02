//! O706 possession prerequisite, shared by root and exact-path worker transactions.
//! The caller already retains chain → own run. Membership writers must use that same prefix.
use super::{RunInfo, StoreError, col, corrupt, store_error};
use sagat_domain::execution::{
    CapacityGroup, CapacityGroupId, CapacityMemberId, CapacityMemberSnapshot, CapacityMembership,
    CapacityPossession,
};
use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Clone, Copy)]
enum Refusal {
    UnsupportedScope,
    AmbiguousMembership,
    FinishedMembership,
    StaleHead,
    GroupBusy,
    NotHolder,
    MembershipChanged,
}

impl Refusal {
    const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedScope => "capacity_scope_unsupported",
            Self::AmbiguousMembership => "capacity_membership_ambiguous",
            Self::FinishedMembership => "capacity_member_finished",
            Self::StaleHead => "capacity_head_changed",
            Self::GroupBusy => "capacity_group_busy",
            Self::NotHolder => "capacity_not_holder",
            Self::MembershipChanged => "capacity_membership_changed",
        }
    }
}

fn refuse(reason: Refusal) -> bool {
    metrics::counter!("worker_claim_skipped_total", "reason" => reason.as_str()).increment(1);
    tracing::debug!(
        reason = reason.as_str(),
        "worker capacity prerequisite refused"
    );
    false
}

#[tracing::instrument(name = "worker.capacity.check", skip_all,
    fields(tenant_id = %info.tenant_id, run_id = %info.run_id))]
pub(super) async fn available(conn: &mut PgConnection, info: &RunInfo) -> Result<bool, StoreError> {
    // This increment implements ChildRoot only. Do not silently treat unsupported scoped
    // registrations as legacy; their creator, ancestry and authority protocol remain pending.
    let scoped: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM capacity_group_members \
         WHERE tenant_id=$1 AND namespace_id=$2 AND original_run_id=$3 \
           AND member_kind='SCOPED_PATH')",
    )
    .bind(info.tenant_id.as_uuid())
    .bind(info.namespace_id.as_uuid())
    .bind(info.run_id.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    if scoped {
        return Ok(refuse(Refusal::UnsupportedScope));
    }
    let chain: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT s.chain_id,c.current_run_id FROM chain_segments s JOIN execution_chains c \
         ON c.tenant_id=s.tenant_id AND c.namespace_id=s.namespace_id AND c.chain_id=s.chain_id \
         WHERE s.tenant_id=$1 AND s.namespace_id=$2 AND s.run_id=$3",
    )
    .bind(info.tenant_id.as_uuid())
    .bind(info.namespace_id.as_uuid())
    .bind(info.run_id.as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    let Some((chain_id, head)) = chain else {
        return Ok(true);
    };
    // Uniqueness is group-scoped. LIMIT 2 detects ambiguity without hydrating history.
    let active: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT group_id,member_id FROM capacity_group_members \
         WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 \
           AND member_kind='CHILD_ROOT' AND state='ACTIVE' \
         ORDER BY group_id,member_id LIMIT 2",
    )
    .bind(info.tenant_id.as_uuid())
    .bind(info.namespace_id.as_uuid())
    .bind(chain_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(store_error)?;
    if active.len() > 1 {
        return Ok(refuse(Refusal::AmbiguousMembership));
    }
    let Some(&(group_id, member_id)) = active.first() else {
        let history: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM capacity_group_members \
             WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 \
               AND member_kind='CHILD_ROOT')",
        )
        .bind(info.tenant_id.as_uuid())
        .bind(info.namespace_id.as_uuid())
        .bind(chain_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(store_error)?;
        return Ok(!history || refuse(Refusal::FinishedMembership));
    };
    if head != info.run_id.as_uuid() {
        return Ok(refuse(Refusal::StaleHead));
    }
    let group = sqlx::query(
        "SELECT revision,epoch,active_members,possession,holder_member_id \
         FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE SKIP LOCKED",
    )
    .bind(info.tenant_id.as_uuid())
    .bind(group_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    let Some(group) = group else {
        return Ok(refuse(Refusal::GroupBusy));
    };
    // Reread the exact fact under G. A receipt or discovery snapshot is not present authority.
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM capacity_group_members \
         WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3 AND namespace_id=$4 \
           AND chain_id=$5 AND member_kind='CHILD_ROOT' AND state='ACTIVE' \
           AND finished_at IS NULL)",
    )
    .bind(info.tenant_id.as_uuid())
    .bind(group_id)
    .bind(member_id)
    .bind(info.namespace_id.as_uuid())
    .bind(chain_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    if !active {
        return Ok(refuse(Refusal::MembershipChanged));
    }
    if col::<String>(&group, "possession")? != "HELD"
        || col::<Option<Uuid>>(&group, "holder_member_id")? != Some(member_id)
    {
        return Ok(refuse(Refusal::NotHolder));
    }
    let fact = CapacityMemberSnapshot::new(
        CapacityGroupId::new(group_id),
        CapacityMemberId::new(member_id),
        CapacityMembership::Active,
    );
    let group = CapacityGroup::restore(
        fact.group_id(),
        info.tenant_id,
        u64::try_from(col::<i64>(&group, "revision")?)
            .map_err(|_| corrupt("capacity group revision"))?,
        u64::try_from(col::<i64>(&group, "epoch")?).map_err(|_| corrupt("capacity group epoch"))?,
        u32::try_from(col::<i64>(&group, "active_members")?)
            .map_err(|_| corrupt("capacity group active count"))?,
        CapacityPossession::Held(fact.member_id()),
        Some(fact),
    )
    .map_err(|_| corrupt("capacity group header"))?;
    // G is retained by the enclosing transaction through the actual external lease COMMIT.
    Ok(group.check_worker_possession(fact).is_ok())
}
