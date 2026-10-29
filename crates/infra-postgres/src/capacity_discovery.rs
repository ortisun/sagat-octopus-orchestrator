//! Read-only discovery; membership hints never grant possession or execution authority.
use crate::{PgRunStore, identity::store_error};
use sagat_application::{
    capacity_discovery::{
        CAPACITY_DISCOVERY_PAGE_MAX, CapacityCandidate, CapacityDiscoveryCursor,
        CapacityDiscoveryError as Error, CapacityDiscoveryPage, CapacityDiscoveryStore,
    },
    maintenance::Window,
};
use sagat_domain::{
    execution::{CapacityGroupId, CapacityMemberId, RunId},
    identity::{NamespaceId, TenantId},
    sharding::Shard,
};
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;

// Only these two fixed cursor clauses are used. The materialized input bounds the joins before
// filtering possession/shards/tenants. The partial index excludes retained FINISHED history.
macro_rules! discovery_sql {
    ($cursor:literal) => { concat!(
        "WITH page AS MATERIALIZED (SELECT tenant_id,group_id,member_id,namespace_id,chain_id ",
        "FROM capacity_group_members WHERE state='ACTIVE' AND member_kind='CHILD_ROOT' ",
        $cursor,
        " ORDER BY tenant_id,group_id,member_id LIMIT $1) ",
        "SELECT p.tenant_id,p.group_id,p.member_id,p.namespace_id,p.chain_id,",
        "g.revision AS group_revision,g.epoch AS group_epoch,g.active_members,g.possession,g.holder_member_id,",
        "c.current_run_id,c.revision AS chain_revision,s.run_id AS segment_run_id,r.run_version ",
        "FROM page p LEFT JOIN capacity_groups g ON g.tenant_id=p.tenant_id AND g.group_id=p.group_id ",
        "LEFT JOIN execution_chains c ON c.tenant_id=p.tenant_id AND c.namespace_id=p.namespace_id AND c.chain_id=p.chain_id ",
        "LEFT JOIN chain_segments s ON s.tenant_id=p.tenant_id AND s.namespace_id=p.namespace_id AND s.chain_id=p.chain_id AND s.run_id=c.current_run_id ",
        "LEFT JOIN runs r ON r.tenant_id=p.tenant_id AND r.namespace_id=p.namespace_id AND r.id=c.current_run_id ",
        "ORDER BY p.tenant_id,p.group_id,p.member_id"
    )};
}

fn counter(row: &PgRow, name: &str) -> Result<u64, Error> {
    let value: i64 = row
        .try_get(name)
        .map_err(|_| Error::ContradictoryProjection)?;
    u64::try_from(value).map_err(|_| Error::ContradictoryProjection)
}
fn decode(row: &PgRow) -> Result<(CapacityCandidate, bool), Error> {
    macro_rules! field {
        ($name:literal,$ty:ty) => {
            row.try_get::<$ty, _>($name)
                .map_err(|_| Error::ContradictoryProjection)?
        };
    }
    let cursor = CapacityDiscoveryCursor {
        tenant_id: TenantId::new(field!("tenant_id", Uuid)),
        group_id: CapacityGroupId::new(field!("group_id", Uuid)),
        member_id: CapacityMemberId::new(field!("member_id", Uuid)),
    };
    let group_revision = counter(row, "group_revision")?;
    let group_epoch = counter(row, "group_epoch")?;
    let chain_revision = counter(row, "chain_revision")?;
    let active = counter(row, "active_members")?;
    if group_revision == 0
        || group_epoch == 0
        || group_epoch > group_revision
        || active == 0
        || active > u64::from(u32::MAX)
        || active > group_revision
    {
        return Err(Error::ContradictoryProjection);
    }
    let holder = field!("holder_member_id", Option<Uuid>);
    let vacant = match field!("possession", String).as_str() {
        "VACANT" if holder.is_none() => true,
        "HELD" if holder.is_some() => false,
        _ => return Err(Error::ContradictoryProjection),
    };
    let head = field!("current_run_id", Uuid);
    if head != field!("segment_run_id", Uuid) {
        return Err(Error::ContradictoryProjection);
    }
    Ok((
        CapacityCandidate {
            cursor,
            namespace_id: NamespaceId::new(field!("namespace_id", Uuid)),
            chain_id: RunId::new(field!("chain_id", Uuid)),
            head_run_id: RunId::new(head),
            group_revision,
            group_epoch,
            chain_revision,
            run_revision: counter(row, "run_version")?,
        },
        vacant,
    ))
}

impl CapacityDiscoveryStore for PgRunStore {
    #[tracing::instrument(name="scheduler.action",skip_all,fields(kind="capacity_discovery",scanned=tracing::field::Empty,selected=tracing::field::Empty))]
    async fn discover_vacant_child_roots(
        &self,
        window: &Window,
        after: Option<CapacityDiscoveryCursor>,
    ) -> Result<CapacityDiscoveryPage, Error> {
        if !(1..=CAPACITY_DISCOVERY_PAGE_MAX).contains(&window.limit)
            || window.exclude.len() > CAPACITY_DISCOVERY_PAGE_MAX as usize
        {
            return Err(Error::InvalidWindow);
        }
        let limit = i64::from(window.limit) + 1;
        let query = if let Some(cursor) = after {
            sqlx::query(discovery_sql!(
                "AND (tenant_id,group_id,member_id)>($2,$3,$4)"
            ))
            .bind(limit)
            .bind(cursor.tenant_id.as_uuid())
            .bind(cursor.group_id.as_uuid())
            .bind(cursor.member_id.as_uuid())
        } else {
            sqlx::query(discovery_sql!("")).bind(limit)
        };
        let rows = query.fetch_all(&self.pool).await.map_err(store_error)?;
        let has_more = rows.len() > window.limit as usize;
        let scanned = rows.len().min(window.limit as usize);
        let mut page = CapacityDiscoveryPage {
            candidates: Vec::with_capacity(scanned),
            scanned_through: None,
            has_more,
        };
        for row in rows.into_iter().take(window.limit as usize) {
            let (candidate, vacant) = decode(&row)?;
            page.scanned_through = Some(candidate.cursor);
            if vacant
                && window
                    .shards
                    .contains(Shard::of(candidate.head_run_id.as_uuid()))
                && !window.exclude.contains(&candidate.cursor.tenant_id)
            {
                page.candidates.push(candidate);
            }
        }
        tracing::Span::current()
            .record("scanned", scanned)
            .record("selected", page.candidates.len());
        Ok(page)
    }
}
