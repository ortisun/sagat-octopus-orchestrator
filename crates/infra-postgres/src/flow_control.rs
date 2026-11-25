//! The key rows of feature 022's gates, as a deciding transaction files them (T022-095; FR-022-053, FR-022-056,
//! ADR-0089 decision 6, D022-038, D022-042).
//!
//! A deciding transaction (admission, readiness, grant, timeout) reads the digest key versions in force in its own
//! transaction ([`key_versions`]), digests each key under them ([`sagat_application::keyed_digest::KeyRing::candidates`])
//! and files its keys of one table with [`file_keys`]: during a rotation the row that exists under the retiring version
//! wins, else the key is filed under the active one, so no key is split between two rows (until an operator forces the
//! retiring version out, ADR-0093 decision 5: then a key has at most two until the old one drains). The keys are locked in
//! one order, by the digest each one resolved to (pool, scope, group, digest), which is the order every other writer of
//! the same rows uses (D022-038); semaphores are filed before windows by the caller.
//!
//! A key row that this transaction creates and leaves empty (no holder, waiter or grant) is not pruned: a caller that
//! refuses after filing rolls back.

use sagat_application::error::StoreError;
use sagat_application::keyed_digest::{KeyCandidates, KeyVersions};
use sagat_domain::execution::GatePool;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::{GateLevel, ResolvedGroup};
use sqlx::PgConnection;

use crate::identity::{corrupt, store_error};

/// The table of a gate's key rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTable {
    /// `concurrency_semaphores`.
    Semaphores,
    /// `rate_limit_windows`.
    Windows,
}

impl KeyTable {
    /// The statements of each table, static (no SQL is assembled at run time).
    const fn insert(self) -> &'static str {
        match self {
            Self::Semaphores => {
                "INSERT INTO concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest, \
                 key_version, key_shard, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, clock_timestamp()) \
                 ON CONFLICT DO NOTHING"
            }
            Self::Windows => {
                "INSERT INTO rate_limit_windows (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest, \
                 key_version, key_shard, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, clock_timestamp()) \
                 ON CONFLICT DO NOTHING"
            }
        }
    }

    const fn exists(self) -> &'static str {
        match self {
            Self::Semaphores => {
                "SELECT EXISTS (SELECT 1 FROM concurrency_semaphores WHERE tenant_id = $1 AND namespace_id = $2 \
                 AND pool = $3 AND permit_scope = $4 AND group_name = $5 AND key_digest = $6)"
            }
            Self::Windows => {
                "SELECT EXISTS (SELECT 1 FROM rate_limit_windows WHERE tenant_id = $1 AND namespace_id = $2 \
                 AND pool = $3 AND limit_scope = $4 AND group_name = $5 AND key_digest = $6)"
            }
        }
    }

    const fn lock(self) -> &'static str {
        match self {
            Self::Semaphores => {
                "SELECT key_version FROM concurrency_semaphores WHERE tenant_id = $1 AND namespace_id = $2 \
                 AND pool = $3 AND permit_scope = $4 AND group_name = $5 AND key_digest = $6 FOR UPDATE"
            }
            Self::Windows => {
                "SELECT key_version FROM rate_limit_windows WHERE tenant_id = $1 AND namespace_id = $2 \
                 AND pool = $3 AND limit_scope = $4 AND group_name = $5 AND key_digest = $6 FOR UPDATE"
            }
        }
    }
}

/// One key a deciding transaction needs, before it is filed.
#[derive(Debug, Clone)]
pub struct KeyToFile {
    pub tenant: TenantId,
    pub namespace: NamespaceId,
    pub pool: GatePool,
    pub level: GateLevel,
    pub group: ResolvedGroup,
    pub candidates: KeyCandidates,
    /// The home shard of a key this transaction creates (ADR-0089 decision 3); a row that exists keeps its own.
    pub shard: i16,
}

/// A key filed and locked (`FOR UPDATE`) for the rest of the transaction. Its `Debug` shows the digest only as its
/// label.
#[derive(Clone, PartialEq, Eq)]
pub struct FiledKey {
    /// The position of the key in the slice given to [`file_keys`].
    pub index: usize,
    /// The digest the row is filed under, and its version.
    pub digest: [u8; 32],
    pub version: u16,
    /// Whether this transaction created the row (the caller judges the namespace's live-keys ceiling, T022-013).
    pub created: bool,
}

impl std::fmt::Debug for FiledKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FiledKey")
            .field("index", &self.index)
            .field(
                "label",
                &sagat_domain::execution::KeyLabel::of_digest(&self.digest),
            )
            .field("version", &self.version)
            .field("created", &self.created)
            .finish()
    }
}

/// The error of a key whose retiring row went away between its resolution and its lock: filing it under the active
/// version at that point would lock it out of the one order, so the deciding transaction is rolled back and retried.
pub const KEY_ROW_VANISHED: &str =
    "a flow-control key row went away during a rotation; retry the transaction";

/// The digest key versions in force, read in the caller's transaction: the active one taken shared, so that a
/// rotation waits for this transaction (FR-022-053).
///
/// # Errors
/// [`StoreError::Unavailable`] when no version was ever activated: a gate cannot decide a key (fail closed).
pub async fn key_versions(conn: &mut PgConnection) -> Result<KeyVersions, StoreError> {
    let (active, retiring): (Option<i16>, Option<i16>) =
        sqlx::query_as("SELECT active, retiring FROM flow_control_key_versions_in_force()")
            .fetch_one(&mut *conn)
            .await
            .map_err(store_error)?;
    let active = active.ok_or_else(|| StoreError::Unavailable {
        detail: "no flow-control digest key version is active".into(),
    })?;
    let version = |v: i16| u16::try_from(v).map_err(|_| corrupt("digest key version"));
    Ok(KeyVersions {
        active: version(active)?,
        retiring: retiring.map(version).transpose()?,
    })
}

/// The digest key versions the database has in force (active, and retiring during a rotation) that `ring` lacks, read
/// once by a process before it serves a gate (D022-057): one that served without them would refuse or never grant
/// every key of that version. An empty answer before any version was activated.
///
/// # Errors
/// A store error.
pub async fn missing_key_versions(
    pool: &sqlx::PgPool,
    ring: &sagat_application::keyed_digest::KeyRing,
) -> Result<Vec<u16>, StoreError> {
    let mut tx = crate::pool::BeginSafely::begin_safely(pool)
        .await
        .map_err(store_error)?;
    let (active, retiring): (Option<i16>, Option<i16>) =
        sqlx::query_as("SELECT active, retiring FROM flow_control_key_versions_in_force()")
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
    tx.rollback().await.map_err(store_error)?;
    let held: Vec<u16> = ring.versions().collect();
    [active, retiring]
        .into_iter()
        .flatten()
        .map(|version| u16::try_from(version).map_err(|_| corrupt("digest key version")))
        .filter(|version| {
            version
                .as_ref()
                .map_or(true, |version| !held.contains(version))
        })
        .collect()
}

/// Files and locks the keys of one table, in the one order: each key resolves to the row that exists under the
/// retiring version, else to its digest under the active version (created when absent); then the rows are locked sorted
/// by (pool, scope, group, digest), byte order (SQL paths that lock the same rows sort `group_name COLLATE "C"`).
///
/// # Errors
/// A store error; [`StoreError::Unavailable`] ([`KEY_ROW_VANISHED`], retryable) when a key's retiring row went away
/// between its resolution and its lock, or a row keeps vanishing under the transaction (three tries): the caller rolls
/// back and runs the deciding transaction again, which then resolves the key to its active digest.
pub async fn file_keys(
    conn: &mut PgConnection,
    table: KeyTable,
    keys: &[KeyToFile],
) -> Result<Vec<FiledKey>, StoreError> {
    file_keys_with(conn, table, keys, &[]).await
}

/// A key row that exists and that the transaction gives something back under (a permit released, a slot returned, a
/// wait dequeued): locked in the same sorted pass as the keys being decided, so that one transaction takes every key row
/// of a table once, in the one order (D022-038).
#[derive(Clone, PartialEq, Eq)]
pub struct ExistingKey {
    pub tenant: TenantId,
    pub namespace: NamespaceId,
    pub pool: GatePool,
    pub level: GateLevel,
    pub group: ResolvedGroup,
    pub digest: [u8; 32],
}

/// [`file_keys`], with `existing` rows locked in the same sorted pass.
///
/// # Errors
/// As [`file_keys`]; an `existing` row that is not there is a corrupt projection.
pub async fn file_keys_with(
    conn: &mut PgConnection,
    table: KeyTable,
    keys: &[KeyToFile],
    existing: &[ExistingKey],
) -> Result<Vec<FiledKey>, StoreError> {
    if keys.is_empty() && existing.is_empty() {
        return Ok(Vec::new());
    }
    let mut resolved = Vec::with_capacity(keys.len());
    for (index, key) in keys.iter().enumerate() {
        let old = match &key.candidates.retiring {
            Some(old) if row_exists(conn, table, key, old.as_bytes()).await? => Some(old),
            _ => None,
        };
        let digest = old.unwrap_or(&key.candidates.active);
        resolved.push((index, *digest.as_bytes(), old.is_some()));
    }
    // one sorted pass over the keys to decide and the rows to give back under
    enum Entry<'a> {
        Decide(usize, [u8; 32], bool),
        Exists(&'a ExistingKey),
    }
    let mut entries: Vec<Entry<'_>> = resolved
        .into_iter()
        .map(|(index, digest, retiring)| Entry::Decide(index, digest, retiring))
        .chain(existing.iter().map(Entry::Exists))
        .collect();
    let order = |entry: &Entry<'_>| match entry {
        Entry::Decide(index, digest, _) => {
            let key = &keys[*index];
            (
                key.pool.as_str(),
                key.level,
                key.group.as_str().to_owned(),
                *digest,
            )
        }
        Entry::Exists(row) => (
            row.pool.as_str(),
            row.level,
            row.group.as_str().to_owned(),
            row.digest,
        ),
    };
    entries.sort_by_key(order);
    let mut filed = Vec::with_capacity(keys.len());
    for entry in entries {
        let (index, digest, retiring) = match entry {
            Entry::Decide(index, digest, retiring) => (index, digest, retiring),
            Entry::Exists(row) => {
                lock_row(
                    conn,
                    table,
                    (row.tenant, row.namespace, row.pool, row.level, &row.group),
                    &row.digest,
                )
                .await?
                .ok_or_else(|| corrupt("a key row a transaction gives back under"))?;
                continue;
            }
        };
        let key = &keys[index];
        if retiring {
            let version =
                lock(conn, table, key, &digest)
                    .await?
                    .ok_or_else(|| StoreError::Unavailable {
                        detail: KEY_ROW_VANISHED.into(),
                    })?;
            filed.push(FiledKey {
                index,
                digest,
                version,
                created: false,
            });
            continue;
        }
        let active = *key.candidates.active.as_bytes();
        filed.push(file_active(conn, table, key, index, &active).await?);
    }
    Ok(filed)
}

async fn file_active(
    conn: &mut PgConnection,
    table: KeyTable,
    key: &KeyToFile,
    index: usize,
    digest: &[u8; 32],
) -> Result<FiledKey, StoreError> {
    for _ in 0..3 {
        let inserted = sqlx::query(table.insert())
            .bind(key.tenant.as_uuid())
            .bind(key.namespace.as_uuid())
            .bind(key.pool.as_str())
            .bind(key.level.as_str())
            .bind(key.group.as_str())
            .bind(digest.as_slice())
            .bind(
                i16::try_from(key.candidates.active.version())
                    .map_err(|_| corrupt("digest key version"))?,
            )
            .bind(key.shard)
            .execute(&mut *conn)
            .await
            .map_err(store_error)?
            .rows_affected()
            == 1;
        if let Some(version) = lock(conn, table, key, digest).await? {
            return Ok(FiledKey {
                index,
                digest: *digest,
                version,
                created: inserted,
            });
        }
    }
    Err(StoreError::Unavailable {
        detail: "a flow-control key row kept vanishing under the transaction".into(),
    })
}

async fn row_exists(
    conn: &mut PgConnection,
    table: KeyTable,
    key: &KeyToFile,
    digest: &[u8; 32],
) -> Result<bool, StoreError> {
    sqlx::query_scalar(table.exists())
        .bind(key.tenant.as_uuid())
        .bind(key.namespace.as_uuid())
        .bind(key.pool.as_str())
        .bind(key.level.as_str())
        .bind(key.group.as_str())
        .bind(digest.as_slice())
        .fetch_one(&mut *conn)
        .await
        .map_err(store_error)
}

/// Locks a key row for the rest of the transaction; `None` when it does not exist (or was deleted meanwhile).
async fn lock(
    conn: &mut PgConnection,
    table: KeyTable,
    key: &KeyToFile,
    digest: &[u8; 32],
) -> Result<Option<u16>, StoreError> {
    lock_row(
        conn,
        table,
        (key.tenant, key.namespace, key.pool, key.level, &key.group),
        digest,
    )
    .await
}

/// Locks the key rows of `rows`, sorted, for the rest of the transaction: what a transaction that only gives back takes.
///
/// # Errors
/// A store error; a row that is not there is a corrupt projection.
pub async fn lock_existing(
    conn: &mut PgConnection,
    table: KeyTable,
    rows: &[ExistingKey],
) -> Result<(), StoreError> {
    file_keys_with(conn, table, &[], rows).await.map(|_| ())
}

async fn lock_row(
    conn: &mut PgConnection,
    table: KeyTable,
    (tenant, namespace, pool, level, group): (
        TenantId,
        NamespaceId,
        GatePool,
        GateLevel,
        &ResolvedGroup,
    ),
    digest: &[u8; 32],
) -> Result<Option<u16>, StoreError> {
    let version: Option<i16> = sqlx::query_scalar(table.lock())
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(pool.as_str())
        .bind(level.as_str())
        .bind(group.as_str())
        .bind(digest.as_slice())
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?;
    version
        .map(|v| u16::try_from(v).map_err(|_| corrupt("digest key version")))
        .transpose()
}

/// The namespace's queued runs as the `queued_runs` ceiling counts them (feature 022, T022-013, D022-051): a run waiting
/// for both gates counts once. The one statement the ceiling's check, its quota usage and the key listing read, so they
/// never disagree; `$1` is the tenant, `$2` the namespace.
pub(crate) const QUEUED_RUNS_SQL: &str = "SELECT count(*) FROM ( \
         SELECT run_id FROM concurrency_waiters \
          WHERE tenant_id = $1 AND namespace_id = $2 AND permit_scope = 'RUN' \
         UNION \
         SELECT run_id FROM rate_limit_waiters \
          WHERE tenant_id = $1 AND namespace_id = $2 AND limit_scope = 'RUN') waiting";

/// The run-level permit of a run, as `get` and `list` read it beside the run's row (T022-026): one lateral probe of the
/// holder and waiter primary keys, which yields nothing for a run that neither holds nor waits. Invariants 1 and 2 allow
/// at most one of the two rows; `verify_run` checks it, and the read prefers the holder if both were ever there.
macro_rules! run_permit_lateral_sql {
    () => {
        " LEFT JOIN LATERAL ( \
           SELECT p.cc_state, p.cc_pool, p.cc_group, p.cc_key, p.cc_since, p.cc_deadline FROM ( \
             SELECT 'HOLDING' AS cc_state, h.pool AS cc_pool, h.group_name AS cc_group, h.key_digest AS cc_key, \
                    h.acquired_at AS cc_since, NULL::timestamptz AS cc_deadline \
               FROM concurrency_holders h \
              WHERE h.tenant_id = runs.tenant_id AND h.run_id = runs.id AND h.holder_scope = 'RUN' AND h.node = '' \
             UNION ALL \
             SELECT 'WAITING', w.pool, w.group_name, w.key_digest, w.enqueued_at, w.deadline_at \
               FROM concurrency_waiters w \
              WHERE w.tenant_id = runs.tenant_id AND w.run_id = runs.id AND w.permit_scope = 'RUN' AND w.node = '' \
           ) p ORDER BY p.cc_state LIMIT 1) permit ON true "
    };
}
pub(crate) use run_permit_lateral_sql;

/// The run-level rate-limit slot of a run, as `get` and `list` read it beside the run's row (T022-078, decision D022-067
/// item 1): its grant while it counts at the statement's instant (`leased_at > now() - per`, the gate's half-open window;
/// a run's grant counts from its grant) or its waiter, one probe of each primary key; nothing for a run with neither.
macro_rules! run_rate_lateral_sql {
    () => {
        " LEFT JOIN LATERAL ( \
           SELECT q.rl_state, q.rl_pool, q.rl_group, q.rl_key, q.rl_since, q.rl_deadline, q.rl_limit, q.rl_period \
             FROM ( \
             SELECT 'GRANTED' AS rl_state, g.pool AS rl_pool, g.group_name AS rl_group, g.key_digest AS rl_key, \
                    g.granted_at AS rl_since, NULL::timestamptz AS rl_deadline, g.declared_limit AS rl_limit, \
                    g.period_ms AS rl_period \
               FROM rate_limit_grants g \
              WHERE g.tenant_id = runs.tenant_id AND g.run_id = runs.id AND g.node = '' AND g.attempt = 0 \
                AND g.leased_at > now() - make_interval(secs => g.period_ms / 1000.0) \
             UNION ALL \
             SELECT 'WAITING', w.pool, w.group_name, w.key_digest, w.enqueued_at, w.deadline_at, \
                    coalesce(w.pinned_limit, w.declared_limit), w.period_ms \
               FROM rate_limit_waiters w \
              WHERE w.tenant_id = runs.tenant_id AND w.run_id = runs.id AND w.node = '' \
           ) q ORDER BY q.rl_state LIMIT 1) slot ON true "
    };
}
pub(crate) use run_rate_lateral_sql;

/// Bounds the limit each waiting run-level slot of `records` shows by the namespace's current ceiling of its window's
/// length, so a waiting run shows the limit the grant routine decides it with, the lower of its declaration and that
/// ceiling (decision D022-076 item 5; the lateral read gives the declaration, or the filed limit of an older row): one
/// read of the ceilings per namespace with a waiting slot, none for runs without one.
///
/// # Errors
/// A store error.
pub(crate) async fn bound_waiting_rate_slots(
    pool: &sqlx::PgPool,
    records: &mut [sagat_application::execution::RunRecord],
) -> Result<(), StoreError> {
    use sagat_application::flow_control_keys::RunPermitState;
    let mut ceilings: Vec<(NamespaceId, sagat_domain::workflow::FlowControlCeilings)> = Vec::new();
    for record in records {
        let (tenant, namespace) = (record.info.tenant_id, record.info.namespace_id);
        let Some(slot) = record.rate_limit.as_mut() else {
            continue;
        };
        if !matches!(slot.state, RunPermitState::Waiting { .. }) {
            continue;
        }
        let of = match ceilings.iter().find(|(n, _)| *n == namespace) {
            Some((_, of)) => *of,
            None => {
                let mut conn = pool.acquire().await.map_err(store_error)?;
                let of = crate::quotas::step_gate_ceilings(&mut conn, tenant, namespace).await?;
                ceilings.push((namespace, of));
                of
            }
        };
        let ceiling = match slot.per {
            sagat_domain::workflow::RatePeriod::Second => of.rate_per_second,
            sagat_domain::workflow::RatePeriod::Minute => of.rate_per_minute,
            sagat_domain::workflow::RatePeriod::Hour => of.rate_per_hour,
        };
        slot.limit = u32::try_from(ceiling.min(u64::from(slot.limit))).unwrap_or(slot.limit);
    }
    Ok(())
}

/// The run-level rate-limit slot of a row that `get` or `list` read with [`run_rate_lateral_sql`]; `None` for a row
/// without those columns (a command transaction's) or a run that neither holds a counting slot nor waits.
pub(crate) fn run_rate_slot(
    row: &sqlx::postgres::PgRow,
) -> Result<Option<sagat_application::flow_control_keys::RunRateSlot>, StoreError> {
    use sagat_application::flow_control_keys::{RunPermitState, RunRateSlot};
    use sqlx::Row as _;
    let Some(state) = crate::execution::selected::<String>(row, "rl_state")? else {
        return Ok(None);
    };
    let pool: String = row.try_get("rl_pool").map_err(store_error)?;
    let group: String = row.try_get("rl_group").map_err(store_error)?;
    let digest: Vec<u8> = row.try_get("rl_key").map_err(store_error)?;
    let since: time::OffsetDateTime = row.try_get("rl_since").map_err(store_error)?;
    let deadline: Option<time::OffsetDateTime> = row.try_get("rl_deadline").map_err(store_error)?;
    let limit: i32 = row.try_get("rl_limit").map_err(store_error)?;
    let period: i32 = row.try_get("rl_period").map_err(store_error)?;
    let digest: [u8; 32] = digest
        .try_into()
        .map_err(|_| corrupt("rate-limit key digest"))?;
    let state = match (state.as_str(), deadline) {
        ("GRANTED", None) => RunPermitState::Holding,
        ("WAITING", Some(deadline)) => RunPermitState::Waiting { deadline },
        _ => return Err(corrupt("run rate-limit slot")),
    };
    let per = u64::try_from(period)
        .ok()
        .and_then(sagat_domain::workflow::RatePeriod::from_millis)
        .ok_or_else(|| corrupt("rate-limit period"))?;
    Ok(Some(RunRateSlot::new(
        GatePool::parse(&pool).ok_or_else(|| corrupt("rate-limit pool"))?,
        ResolvedGroup::parse(&group).map_err(|_| corrupt("rate-limit group"))?,
        digest,
        state,
        since,
        (
            u32::try_from(limit).map_err(|_| corrupt("rate-limit limit"))?,
            per,
        ),
    )))
}

/// The run-level permit of a row that `get` or `list` read with [`run_permit_lateral_sql`]; `None` for a row without
/// those columns (a command transaction's) or a run that neither holds nor waits.
pub(crate) fn run_permit(
    row: &sqlx::postgres::PgRow,
) -> Result<Option<sagat_application::flow_control_keys::RunPermit>, StoreError> {
    use sagat_application::flow_control_keys::{RunPermit, RunPermitState};
    use sqlx::Row as _;
    // a row without the columns (a command transaction's) has no permit; one whose columns cannot be read is corrupt
    let Some(state) = crate::execution::selected::<String>(row, "cc_state")? else {
        return Ok(None);
    };
    let pool: String = row.try_get("cc_pool").map_err(store_error)?;
    let group: String = row.try_get("cc_group").map_err(store_error)?;
    let digest: Vec<u8> = row.try_get("cc_key").map_err(store_error)?;
    let since: time::OffsetDateTime = row.try_get("cc_since").map_err(store_error)?;
    let deadline: Option<time::OffsetDateTime> = row.try_get("cc_deadline").map_err(store_error)?;
    let digest: [u8; 32] = digest
        .try_into()
        .map_err(|_| corrupt("concurrency key digest"))?;
    let state = match (state.as_str(), deadline) {
        ("HOLDING", None) => RunPermitState::Holding,
        ("WAITING", Some(deadline)) => RunPermitState::Waiting { deadline },
        _ => return Err(corrupt("run permit")),
    };
    Ok(Some(RunPermit::new(
        GatePool::parse(&pool).ok_or_else(|| corrupt("concurrency pool"))?,
        ResolvedGroup::parse(&group).map_err(|_| corrupt("concurrency group"))?,
        digest,
        state,
        since,
    )))
}

impl sagat_application::flow_control_keys::FlowControlKeyReader for crate::PgRunStore {
    async fn concurrency_keys(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        group: Option<&ResolvedGroup>,
        after: Option<&sagat_application::flow_control_keys::KeyPosition>,
        limit: u32,
    ) -> Result<sagat_application::flow_control_keys::ConcurrencyKeyPage, StoreError> {
        use sagat_application::flow_control_keys::{
            ConcurrencyKeyPage, ConcurrencyKeyView, KeyPosition,
        };
        use sqlx::Row as _;
        // one snapshot for the page and the count, so a page's keys and its queued runs are of one instant
        let mut tx = crate::pool::BeginSafely::begin_safely(self.pool())
            .await
            .map_err(store_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        // a page is a few index probes; one that takes longer is refused as unavailable (a `503`), never left to hold
        // an API connection (D022-058)
        sqlx::query(LISTING_TIMEOUT_SQL)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let after_since = after
            .and_then(|p| p.oldest_wait_us)
            .map(|us| {
                time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(us) * 1_000)
                    .map_err(|_| corrupt("concurrency key cursor"))
            })
            .transpose()?;
        // a key nobody waits for sorts after every key somebody waits for ('infinity'); the five sort columns are the
        // row's key within the namespace, so the order is total and a page resumes exactly after its last key. The
        // order is an index's (`concurrency_semaphores_listing_idx`, or `_group_listing_idx` for one group), so a page
        // reads its own keys and stops: its cost does not grow with the namespace's live keys (D022-058).
        let sql = if group.is_some() {
            KEY_LISTING_OF_GROUP_SQL
        } else {
            KEY_LISTING_SQL
        };
        // From the first key, the position before every key ('-infinity'); after a key nobody waits for, 'infinity'.
        // One statement either way, its position always an index condition (never an `OR` a generic plan would
        // turn into a filter of every key before the cursor).
        let after_bucket = match after {
            None => "-infinity",
            Some(_) => "infinity",
        };
        let rows = sqlx::query(sql)
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(group.map(ResolvedGroup::as_str))
            .bind(after_bucket)
            .bind(after_since)
            .bind(after.map_or("", |p| p.pool.as_str()))
            .bind(after.map_or("", |p| p.scope.as_str()))
            .bind(after.map_or_else(
                || group.map_or("", ResolvedGroup::as_str),
                |p| p.group.as_str(),
            ))
            .bind(after.map_or(&[][..], |p| p.digest.as_slice()))
            .bind(i64::from(limit))
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?;
        let queued_runs: i64 = sqlx::query_scalar(QUEUED_RUNS_SQL)
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        // the version that seals the page's cursor: the active one, which every process of a rotation holds (D022-059);
        // read plainly (a read-only snapshot takes no lock)
        let cursor_version: Option<i16> = sqlx::query_scalar(
            "SELECT version FROM flow_control_key_versions WHERE state = 'ACTIVE'",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            let pool = GatePool::parse(&row.try_get::<String, _>("pool").map_err(store_error)?)
                .ok_or_else(|| corrupt("concurrency pool"))?;
            let scope = match row
                .try_get::<String, _>("permit_scope")
                .map_err(store_error)?
                .as_str()
            {
                "RUN" => GateLevel::Run,
                "STEP" => GateLevel::Step,
                _ => return Err(corrupt("concurrency scope")),
            };
            let group = ResolvedGroup::parse(
                &row.try_get::<String, _>("group_name")
                    .map_err(store_error)?,
            )
            .map_err(|_| corrupt("concurrency group"))?;
            let digest: [u8; 32] = row
                .try_get::<Vec<u8>, _>("key_digest")
                .map_err(store_error)?
                .try_into()
                .map_err(|_| corrupt("concurrency key digest"))?;
            let count = |name: &str| -> Result<u32, StoreError> {
                u32::try_from(row.try_get::<i32, _>(name).map_err(store_error)?)
                    .map_err(|_| corrupt("concurrency counter"))
            };
            let oldest_wait: Option<time::OffsetDateTime> =
                row.try_get("oldest_wait").map_err(store_error)?;
            let oldest_wait_us = oldest_wait
                .map(|at| i64::try_from(at.unix_timestamp_nanos() / 1_000))
                .transpose()
                .map_err(|_| corrupt("concurrency wait instant"))?;
            items.push(ConcurrencyKeyView {
                // the service shows it to whom it may (D022-060)
                label: None,
                holders: count("holders")?,
                waiters: count("waiters")?,
                oldest_wait_since: oldest_wait,
                oldest_holder_since: row.try_get("oldest_holder").map_err(store_error)?,
                position: KeyPosition {
                    oldest_wait_us,
                    pool,
                    scope,
                    group: group.clone(),
                    digest,
                },
                pool,
                scope,
                group,
            });
        }
        Ok(ConcurrencyKeyPage {
            items,
            queued_runs: u64::try_from(queued_runs).map_err(|_| corrupt("queued runs"))?,
            cursor_version: cursor_version
                .map(|version| u16::try_from(version).map_err(|_| corrupt("digest key version")))
                .transpose()?,
        })
    }

    async fn rate_limit_keys(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        group: Option<&ResolvedGroup>,
        after: Option<&sagat_application::flow_control_keys::KeyPosition>,
        limit: u32,
    ) -> Result<sagat_application::flow_control_keys::RateLimitKeyPage, StoreError> {
        use sagat_application::flow_control_keys::{
            KeyPosition, RateLimitKeyPage, RateLimitKeyView,
        };
        use sqlx::Row as _;
        // one snapshot and one instant (`now()`, the transaction's) for the page, its counts and its queued runs
        let mut tx = crate::pool::BeginSafely::begin_safely(self.pool())
            .await
            .map_err(store_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        sqlx::query(LISTING_TIMEOUT_SQL)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let after_since = after
            .and_then(|p| p.oldest_wait_us)
            .map(|us| {
                time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(us) * 1_000)
                    .map_err(|_| corrupt("rate-limit key cursor"))
            })
            .transpose()?;
        let sql = if group.is_some() {
            RATE_KEY_LISTING_OF_GROUP_SQL
        } else {
            RATE_KEY_LISTING_SQL
        };
        let after_bucket = match after {
            None => "-infinity",
            Some(_) => "infinity",
        };
        let rows = sqlx::query(sql)
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(group.map(ResolvedGroup::as_str))
            .bind(after_bucket)
            .bind(after_since)
            .bind(after.map_or("", |p| p.pool.as_str()))
            .bind(after.map_or("", |p| p.scope.as_str()))
            .bind(after.map_or_else(
                || group.map_or("", ResolvedGroup::as_str),
                |p| p.group.as_str(),
            ))
            .bind(after.map_or(&[][..], |p| p.digest.as_slice()))
            .bind(i64::from(limit))
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?;
        let queued_runs: i64 = sqlx::query_scalar(QUEUED_RUNS_SQL)
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        let cursor_version: Option<i16> = sqlx::query_scalar(
            "SELECT version FROM flow_control_key_versions WHERE state = 'ACTIVE'",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            let pool = GatePool::parse(&row.try_get::<String, _>("pool").map_err(store_error)?)
                .ok_or_else(|| corrupt("rate-limit pool"))?;
            let scope = match row
                .try_get::<String, _>("limit_scope")
                .map_err(store_error)?
                .as_str()
            {
                "RUN" => GateLevel::Run,
                "STEP" => GateLevel::Step,
                _ => return Err(corrupt("rate-limit scope")),
            };
            let group = ResolvedGroup::parse(
                &row.try_get::<String, _>("group_name")
                    .map_err(store_error)?,
            )
            .map_err(|_| corrupt("rate-limit group"))?;
            let digest: [u8; 32] = row
                .try_get::<Vec<u8>, _>("key_digest")
                .map_err(store_error)?
                .try_into()
                .map_err(|_| corrupt("rate-limit key digest"))?;
            let count = |name: &str| -> Result<u32, StoreError> {
                u32::try_from(row.try_get::<i64, _>(name).map_err(store_error)?)
                    .map_err(|_| corrupt("rate-limit counter"))
            };
            let waiters = u32::try_from(row.try_get::<i32, _>("waiters").map_err(store_error)?)
                .map_err(|_| corrupt("rate-limit counter"))?;
            let unleased = count("unleased")?;
            let oldest_wait: Option<time::OffsetDateTime> =
                row.try_get("oldest_wait").map_err(store_error)?;
            let oldest_wait_us = oldest_wait
                .map(|at| i64::try_from(at.unix_timestamp_nanos() / 1_000))
                .transpose()
                .map_err(|_| corrupt("rate-limit wait instant"))?;
            items.push(RateLimitKeyView {
                label: None,
                grants_in_window: unleased.saturating_add(count("leased_counting")?),
                unleased_grants: unleased,
                waiters,
                oldest_wait_since: oldest_wait,
                next_slot_at: row.try_get("next_slot").map_err(store_error)?,
                position: KeyPosition {
                    oldest_wait_us,
                    pool,
                    scope,
                    group: group.clone(),
                    digest,
                },
                pool,
                scope,
                group,
            });
        }
        Ok(RateLimitKeyPage {
            items,
            queued_runs: u64::try_from(queued_runs).map_err(|_| corrupt("queued runs"))?,
            cursor_version: cursor_version
                .map(|version| u16::try_from(version).map_err(|_| corrupt("digest key version")))
                .transpose()?,
        })
    }

    async fn rate_place(
        &self,
        tenant: TenantId,
        run: sagat_domain::execution::RunId,
    ) -> Result<Option<sagat_application::flow_control_keys::PlaceReading>, StoreError> {
        use sagat_application::flow_control_keys::PlaceReading;
        // its own read-only transaction and the listing's timeout: an estimate must never hold a reader long (decision
        // D022-068 item 2; the caller shows no place on an error)
        let mut tx = crate::pool::BeginSafely::begin_safely(self.pool())
            .await
            .map_err(store_error)?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        sqlx::query(LISTING_TIMEOUT_SQL)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        // the namespace's current ceilings, read in this transaction: the estimate uses the limit the grant routine
        // decides the waiter with (decision D022-075 item 3)
        let namespace: Option<uuid::Uuid> =
            sqlx::query_scalar("SELECT namespace_id FROM runs WHERE tenant_id = $1 AND id = $2")
                .bind(tenant.as_uuid())
                .bind(run.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_error)?;
        let Some(namespace) = namespace else {
            tx.commit().await.map_err(store_error)?;
            return Ok(None);
        };
        let ceilings =
            crate::quotas::step_gate_ceilings(&mut tx, tenant, NamespaceId::new(namespace)).await?;
        let bound = |ceiling: u64| i64::try_from(ceiling).unwrap_or(i64::MAX);
        #[allow(clippy::type_complexity)]
        let row: Option<(
            i32,
            i32,
            i64,
            i64,
            time::OffsetDateTime,
            Option<time::OffsetDateTime>,
            time::OffsetDateTime,
            bool,
        )> = sqlx::query_as(RATE_PLACE_SQL)
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .bind(bound(ceilings.rate_per_second))
            .bind(bound(ceilings.rate_per_minute))
            .bind(bound(ceilings.rate_per_hour))
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        let Some((limit, period, counted, ahead, t, kth_lease, deadline, skipped)) = row else {
            return Ok(None);
        };
        let per = u64::try_from(period)
            .ok()
            .and_then(sagat_domain::workflow::RatePeriod::from_millis)
            .ok_or_else(|| corrupt("rate-limit period"))?;
        Ok(Some(PlaceReading {
            t,
            counted: u64::try_from(counted).map_err(|_| corrupt("rate-limit count"))?,
            ahead: u64::try_from(ahead).map_err(|_| corrupt("rate-limit waiters"))?,
            limit: u32::try_from(limit).map_err(|_| corrupt("rate-limit limit"))?,
            kth_lease,
            per,
            deadline,
            skipped,
        }))
    }

    async fn key_version_in_force(&self, version: u16) -> Result<bool, StoreError> {
        // the one reading of the versions in force (`flow_control_key_versions_in_force`): a forced or ended retirement
        // is out of force at once (ADR-0093 decision 5)
        // a version no `smallint` holds is in force nowhere: a `400`, not a `500` (re-review of 39157608, nit)
        let Ok(version) = i16::try_from(version) else {
            return Ok(false);
        };
        sqlx::query_scalar(
            "SELECT coalesce(active = $1, false) OR coalesce(retiring = $1, false) \
               FROM flow_control_key_versions_in_force()",
        )
        .bind(version)
        .fetch_one(self.pool())
        .await
        .map_err(store_error)
    }

    async fn reveals_personal_data(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &sagat_domain::identity::Subject,
    ) -> Result<bool, StoreError> {
        // feature 009's rule, its one implementation (D022-060)
        crate::reveal_grants::may_reveal_payload_unlocked(self.pool(), tenant, namespace, subject)
            .await
    }

    fn flow_control_key_ring(&self) -> Option<&sagat_application::keyed_digest::KeyRing> {
        self.flow_control_keys()
    }
}

/// The bound of one statement of the key listings' pages and of a run's rate-limit place (D022-058, D022-068 item 2).
const LISTING_TIMEOUT_SQL: &str = "SET LOCAL statement_timeout = '2s'";

/// The listing of a namespace's keys (`$3` is null), in the order of `concurrency_semaphores_listing_idx`, after the
/// position `(COALESCE($5, $4), $6, $7, $8, $9)`.
pub const KEY_LISTING_SQL: &str = "SELECT s.pool, s.permit_scope, s.group_name, s.key_digest, s.holders, s.waiters, \
            s.oldest_wait_at AS oldest_wait, \
            (SELECT h.acquired_at FROM concurrency_holders h \
              WHERE h.tenant_id = s.tenant_id AND h.namespace_id = s.namespace_id AND h.pool = s.pool \
                AND h.holder_scope = s.permit_scope AND h.group_name = s.group_name \
                AND h.key_digest = s.key_digest \
              ORDER BY h.acquired_at LIMIT 1) AS oldest_holder \
       FROM concurrency_semaphores s \
      WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND $3::text IS NULL \
        AND (COALESCE(s.oldest_wait_at, 'infinity'::timestamptz), s.pool, s.permit_scope, s.group_name, s.key_digest) \
            > (COALESCE($5::timestamptz, $4::text::timestamptz), $6::text, $7::text, $8::text, $9::bytea) \
      ORDER BY COALESCE(s.oldest_wait_at, 'infinity'::timestamptz), s.pool, s.permit_scope, s.group_name, \
               s.key_digest \
      LIMIT $10";

/// The columns of one window of the rate-limit listing at the page's instant (`now()`, its transaction's; plan "Visibility
/// of the rate gate"): its unleased grants, its leased grants that count over its longest period, and, when somebody
/// waits and no grant is unleased, the first instant a counted grant stops counting for one of its first 256 waiters in
/// the queue's order ([`GRANTS_PER_KEY_PER_PASS`](sagat_application::flow_control::GRANTS_PER_KEY_PER_PASS), the most
/// one pass grants a key; decision D022-068 item 3): for each of their windows `p` (three exist), one probe of
/// `rate_limit_grants_count_idx` for the oldest lease with `leased_at > now() - p` (the half-open window, the column
/// bare so the index serves it), plus `p`.
/// The most waiters of one key the grant pass tries ([`GRANTS_PER_KEY_PER_PASS`]
/// (sagat_application::flow_control::GRANTS_PER_KEY_PER_PASS)), as SQL text: the listing reads the periods of that many
/// waiters, and the build checks the two are one number.
macro_rules! grants_per_key_per_pass {
    () => {
        "256"
    };
}

/// [`grants_per_key_per_pass!`] read as a number at build time.
const fn decimal(text: &str) -> u32 {
    let bytes = text.as_bytes();
    let mut value = 0_u32;
    let mut index = 0;
    while index < bytes.len() {
        value = value * 10 + (bytes[index] - b'0') as u32;
        index += 1;
    }
    value
}

const _: () = assert!(
    decimal(grants_per_key_per_pass!()) == sagat_application::flow_control::GRANTS_PER_KEY_PER_PASS,
    "the rate-limit listing reads the periods of the waiters one grant pass tries"
);

macro_rules! rate_key_columns_sql {
    () => {
        concat!("SELECT w.pool, w.limit_scope, w.group_name, w.key_digest, w.waiters, w.oldest_wait_at AS oldest_wait, \
                (SELECT count(*) FROM rate_limit_grants g \
                  WHERE g.tenant_id = w.tenant_id AND g.namespace_id = w.namespace_id AND g.pool = w.pool \
                    AND g.limit_scope = w.limit_scope AND g.group_name = w.group_name AND g.key_digest = w.key_digest \
                    AND g.leased_at IS NULL) AS unleased, \
                (SELECT count(*) FROM rate_limit_grants g \
                  WHERE g.tenant_id = w.tenant_id AND g.namespace_id = w.namespace_id AND g.pool = w.pool \
                    AND g.limit_scope = w.limit_scope AND g.group_name = w.group_name AND g.key_digest = w.key_digest \
                    AND g.leased_at > now() - make_interval(secs => w.max_period_ms / 1000.0)) AS leased_counting, \
                CASE WHEN w.waiters > 0 AND NOT EXISTS (SELECT 1 FROM rate_limit_grants g \
                       WHERE g.tenant_id = w.tenant_id AND g.namespace_id = w.namespace_id AND g.pool = w.pool \
                         AND g.limit_scope = w.limit_scope AND g.group_name = w.group_name \
                         AND g.key_digest = w.key_digest AND g.leased_at IS NULL) \
                     THEN (SELECT min(s.slot) \
                             FROM (SELECT DISTINCT f.period_ms \
                                     FROM (SELECT x.period_ms FROM rate_limit_waiters x \
                                            WHERE x.tenant_id = w.tenant_id AND x.namespace_id = w.namespace_id \
                                              AND x.pool = w.pool AND x.limit_scope = w.limit_scope \
                                              AND x.group_name = w.group_name AND x.key_digest = w.key_digest \
                                            ORDER BY x.enqueued_at, x.run_id, x.node LIMIT \
                                            ", grants_per_key_per_pass!(), ") f) p \
                            CROSS JOIN LATERAL ( \
                                   SELECT min(g.leased_at) + make_interval(secs => p.period_ms / 1000.0) AS slot \
                                     FROM rate_limit_grants g \
                                    WHERE g.tenant_id = w.tenant_id AND g.namespace_id = w.namespace_id \
                                      AND g.pool = w.pool AND g.limit_scope = w.limit_scope \
                                      AND g.group_name = w.group_name AND g.key_digest = w.key_digest \
                                      AND g.leased_at > now() - make_interval(secs => p.period_ms / 1000.0)) s) \
                END AS next_slot \
           FROM rate_limit_windows w ")
    };
}

/// The listing of a namespace's rate-limit windows (`$3` is null), in the order of `rate_limit_windows_listing_idx`,
/// after the position `(COALESCE($5, $4), $6, $7, $8, $9)` (as [`KEY_LISTING_SQL`]).
pub const RATE_KEY_LISTING_SQL: &str = concat!(
    rate_key_columns_sql!(),
    "WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND $3::text IS NULL \
        AND (COALESCE(w.oldest_wait_at, 'infinity'::timestamptz), w.pool, w.limit_scope, w.group_name, w.key_digest) \
            > (COALESCE($5::timestamptz, $4::text::timestamptz), $6::text, $7::text, $8::text, $9::bytea) \
      ORDER BY COALESCE(w.oldest_wait_at, 'infinity'::timestamptz), w.pool, w.limit_scope, w.group_name, \
               w.key_digest \
      LIMIT $10"
);

/// The listing of one group's rate-limit windows (`$3`), in the order of `rate_limit_windows_group_listing_idx`.
pub const RATE_KEY_LISTING_OF_GROUP_SQL: &str = concat!(
    rate_key_columns_sql!(),
    "WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.group_name = $3 AND $8::text = $3 \
        AND (COALESCE(w.oldest_wait_at, 'infinity'::timestamptz), w.pool, w.limit_scope, w.key_digest) \
            > (COALESCE($5::timestamptz, $4::text::timestamptz), $6::text, $7::text, $9::bytea) \
      ORDER BY COALESCE(w.oldest_wait_at, 'infinity'::timestamptz), w.pool, w.limit_scope, w.key_digest \
      LIMIT $10"
);

/// Where the run-level rate-limit waiter of run `$2` of tenant `$1` stands at one instant `t` (`now()`; plan
/// "Visibility of the rate gate", D022-014): its limit `L` (the lower of its declaration and the namespace's current
/// ceiling of its window length, `$3`, `$4` and `$5` for a second, a minute and an hour: the limit the grant routine
/// decides it with, decisions D022-075 item 3 and D022-076 item 7) and window, `n` the key's grants that count over its
/// window at `t`, `a` the waiters ahead of it that the grant routine counts ahead (not past their deadline, their run
/// running or waiting and not paused, and below the count of their own window by the lower of their declaration and the
/// ceiling of its length: decision D022-076 item 5), `t` (the statement's instant), the lease of the `k`-th oldest counted grant for
/// `k = n + a - limit + 1` (offset `k - 1`, at least 0), and its own deadline and whether the pass skips its run (decision
/// D022-068 item 1). A run's grant is leased at its grant (`rate_limit_grants_shape`), so its counted grants are those
/// leased within its window, one range of `rate_limit_grants_count_idx`.
pub const RATE_PLACE_SQL: &str = "WITH me AS ( \
         SELECT w.tenant_id, w.namespace_id, w.pool, w.limit_scope, w.group_name, w.key_digest, w.run_id, w.node, \
                w.enqueued_at, w.period_ms, w.deadline_at, \
                least(coalesce(w.pinned_limit, w.declared_limit)::bigint, CASE w.period_ms WHEN 1000 THEN $3::bigint \
                      WHEN 60000 THEN $4::bigint WHEN 3600000 THEN $5::bigint \
                      ELSE coalesce(w.pinned_limit, w.declared_limit)::bigint END)::integer AS l, \
                NOT (mr.status IN ('RUNNING', 'WAITING') AND NOT mr.paused) AS skipped, statement_timestamp() AS t \
           FROM rate_limit_waiters w JOIN runs mr ON mr.tenant_id = w.tenant_id AND mr.id = w.run_id \
          WHERE w.tenant_id = $1 AND w.run_id = $2 AND w.node = ''), \
       counts AS ( \
         SELECT p.ms, (SELECT count(*) FROM rate_limit_grants g \
                        WHERE g.tenant_id = me.tenant_id AND g.namespace_id = me.namespace_id AND g.pool = me.pool \
                          AND g.limit_scope = me.limit_scope AND g.group_name = me.group_name \
                          AND g.key_digest = me.key_digest \
                          AND g.leased_at > me.t - make_interval(secs => p.ms / 1000.0)) AS n \
           FROM (VALUES (1000), (60000), (3600000)) p(ms), me), \
       counted AS (SELECT c.n FROM counts c, me WHERE c.ms = me.period_ms), \
       ahead AS ( \
         SELECT count(*) AS a FROM me \
           JOIN rate_limit_waiters x \
             ON x.tenant_id = me.tenant_id AND x.namespace_id = me.namespace_id AND x.pool = me.pool \
            AND x.limit_scope = me.limit_scope AND x.group_name = me.group_name AND x.key_digest = me.key_digest \
           JOIN runs r ON r.tenant_id = x.tenant_id AND r.id = x.run_id \
           LEFT JOIN counts c ON c.ms = x.period_ms \
          WHERE (x.enqueued_at, x.run_id, x.node) < (me.enqueued_at, me.run_id, me.node) \
            AND x.deadline_at > me.t AND r.status IN ('RUNNING', 'WAITING') AND NOT r.paused \
            AND least(coalesce(x.pinned_limit, x.declared_limit)::bigint, CASE x.period_ms WHEN 1000 THEN $3::bigint \
                      WHEN 60000 THEN $4::bigint WHEN 3600000 THEN $5::bigint \
                      ELSE coalesce(x.pinned_limit, x.declared_limit)::bigint END) > coalesce(c.n, 0)) \
     SELECT me.l, me.period_ms, counted.n, ahead.a, me.t, \
            (SELECT g.leased_at FROM rate_limit_grants g \
              WHERE g.tenant_id = me.tenant_id AND g.namespace_id = me.namespace_id AND g.pool = me.pool \
                AND g.limit_scope = me.limit_scope AND g.group_name = me.group_name AND g.key_digest = me.key_digest \
                AND g.leased_at > me.t - make_interval(secs => me.period_ms / 1000.0) \
              ORDER BY g.leased_at OFFSET greatest(counted.n + ahead.a - me.l, 0) LIMIT 1) AS kth, \
            me.deadline_at, me.skipped \
       FROM me, counted, ahead";

/// The listing of one group's keys (`$3`), in the order of `concurrency_semaphores_group_listing_idx`: the order of
/// [`KEY_LISTING_SQL`] within one group, after a position of that group (`$8`).
pub const KEY_LISTING_OF_GROUP_SQL: &str = "SELECT s.pool, s.permit_scope, s.group_name, s.key_digest, s.holders, \
            s.waiters, s.oldest_wait_at AS oldest_wait, \
            (SELECT h.acquired_at FROM concurrency_holders h \
              WHERE h.tenant_id = s.tenant_id AND h.namespace_id = s.namespace_id AND h.pool = s.pool \
                AND h.holder_scope = s.permit_scope AND h.group_name = s.group_name \
                AND h.key_digest = s.key_digest \
              ORDER BY h.acquired_at LIMIT 1) AS oldest_holder \
       FROM concurrency_semaphores s \
      WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.group_name = $3 AND $8::text = $3 \
        AND (COALESCE(s.oldest_wait_at, 'infinity'::timestamptz), s.pool, s.permit_scope, s.key_digest) \
            > (COALESCE($5::timestamptz, $4::text::timestamptz), $6::text, $7::text, $9::bytea) \
      ORDER BY COALESCE(s.oldest_wait_at, 'infinity'::timestamptz), s.pool, s.permit_scope, s.key_digest \
      LIMIT $10";
