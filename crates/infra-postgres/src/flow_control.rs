//! The key rows of feature 022's gates, as a deciding transaction files them (T022-095; FR-022-053, FR-022-056,
//! ADR-0089 decision 6, D022-038, D022-042).
//!
//! A deciding transaction (admission, readiness, grant, timeout) reads the digest key versions in force in its own
//! transaction ([`key_versions`]), digests each key under them ([`sagat_application::keyed_digest::KeyRing::candidates`])
//! and files its keys of one table with [`file_keys`]: during a rotation the row that exists under the retiring version
//! wins, else the key is filed under the active one, so no key is ever split between two rows. The keys are locked in
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

/// A key filed and locked (`FOR UPDATE`) for the rest of the transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiledKey {
    /// The position of the key in the slice given to [`file_keys`].
    pub index: usize,
    /// The digest the row is filed under, and its version.
    pub digest: [u8; 32],
    pub version: u16,
    /// Whether this transaction created the row (the caller judges the namespace's live-keys ceiling, T022-013).
    pub created: bool,
}

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

/// Files and locks the keys of one table, in the one order: each key resolves to the row that exists under the
/// retiring version, else to its digest under the active version (created when absent); then the rows are locked sorted
/// by (pool, scope, group, digest). A key whose retiring row went away between the two steps is filed under the active
/// version.
///
/// # Errors
/// A store error; [`StoreError::Unavailable`] if a row keeps vanishing under the transaction (three tries).
pub async fn file_keys(
    conn: &mut PgConnection,
    table: KeyTable,
    keys: &[KeyToFile],
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
    resolved.sort_by(|(a, da, _), (b, db, _)| {
        let (ka, kb) = (&keys[*a], &keys[*b]);
        (ka.pool.as_str(), ka.level, ka.group.as_str(), da).cmp(&(
            kb.pool.as_str(),
            kb.level,
            kb.group.as_str(),
            db,
        ))
    });
    let mut filed = Vec::with_capacity(keys.len());
    for (index, digest, retiring) in resolved {
        let key = &keys[index];
        if retiring && let Some(version) = lock(conn, table, key, &digest).await? {
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
    let version: Option<i16> = sqlx::query_scalar(table.lock())
        .bind(key.tenant.as_uuid())
        .bind(key.namespace.as_uuid())
        .bind(key.pool.as_str())
        .bind(key.level.as_str())
        .bind(key.group.as_str())
        .bind(digest.as_slice())
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?;
    version
        .map(|v| u16::try_from(v).map_err(|_| corrupt("digest key version")))
        .transpose()
}
