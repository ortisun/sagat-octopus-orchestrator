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
        })
    }

    async fn reveals_personal_data(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &sagat_domain::identity::Subject,
    ) -> Result<bool, StoreError> {
        // feature 009's rule (`has_effective_payload_reveal_grant`) read in one snapshot without its locks: the role
        // the subject holds in the tenant now may use a reveal grant, and it holds one for the namespace
        let roles: Vec<String> = sqlx::query_scalar(
            "SELECT m.role FROM memberships m \
              WHERE m.tenant_id = $1 AND m.subject = $3 \
                AND EXISTS (SELECT 1 FROM payload_reveal_grants g \
                             WHERE g.tenant_id = $1 AND g.namespace_id = $2 AND g.subject = $3)",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject.as_str())
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        Ok(roles.iter().any(|role| {
            role.parse::<sagat_domain::identity::Role>()
                .is_ok_and(|role| {
                    role.grants(sagat_domain::identity::Permission::PayloadReadInternal)
                })
        }))
    }

    fn flow_control_key_ring(&self) -> Option<&sagat_application::keyed_digest::KeyRing> {
        self.flow_control_keys()
    }
}

/// The bound of one page of the key listing (D022-058): a page is at most 201 probes of three indexes.
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
