//! Worker bindings in PostgreSQL (FR-009-002, gap G31).

use crate::pool::BeginSafely;
use sagat_application::bindings::{BindingStore, Revoked, StoredBinding};
use sagat_application::error::StoreError;
use sagat_domain::audit::AuditRecord;
use sagat_domain::binding::{ActivityPattern, BindingProfile, first_uncovered};
use sagat_domain::identity::{NamespaceId, TenantId};
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{insert_audit, store_error};

/// Hold the same authority key that binding replacement takes exclusively, through this commit.
pub(crate) async fn hold_worker_binding(
    conn: &mut sqlx::PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    subject: &str,
) -> Result<(), StoreError> {
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
        .bind(format!("binding:{tenant}:{namespace}:{subject}"))
        .execute(conn)
        .await
        .map_err(store_error)?;
    Ok(())
}

impl BindingStore for PgRunStore {
    async fn bindings(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Vec<StoredBinding>, StoreError> {
        let rows: Vec<(String, Vec<String>, Vec<String>, String)> = sqlx::query_as(
            "SELECT b.subject, array_agg(b.pattern ORDER BY b.pattern), \
                    COALESCE((SELECT array_agg(p.pool ORDER BY p.pool) FROM worker_binding_pools p \
                              WHERE p.tenant_id = b.tenant_id AND p.namespace_id = b.namespace_id \
                                AND p.subject = b.subject), '{}'), \
                    min(b.profile) \
             FROM worker_bindings b \
             WHERE b.tenant_id = $1 AND b.namespace_id = $2 \
             GROUP BY b.tenant_id, b.namespace_id, b.subject ORDER BY b.subject",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.into_iter()
            .map(|(subject, patterns, pools, profile)| {
                Ok(StoredBinding {
                    subject,
                    patterns,
                    pools,
                    profile: BindingProfile::parse(&profile).ok_or_else(|| {
                        StoreError::Internal {
                            detail: "a stored binding profile is not a profile".into(),
                        }
                    })?,
                })
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)] // the port's signature
    async fn replace_binding(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
        patterns: &[String],
        pools: Option<&[String]>,
        profile: Option<BindingProfile>,
        changed_by: &str,
        audit: &AuditRecord,
    ) -> Result<Option<Revoked>, StoreError> {
        let parsed = patterns
            .iter()
            .map(|text| ActivityPattern::parse(text))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| StoreError::Internal {
                detail: "a binding pattern does not parse".into(),
            })?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // One change at a time per identity and namespace, so two administrators cannot interleave
        // a replace with a revocation.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("binding:{tenant}:{namespace}:{subject}"))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        // An identity is of one profile in the whole tenant, not one per namespace: a stolen token of a lake runner must not be
        // a SQL runner in the namespace next door. Changes of one identity's profile are serialised on their own key, taken
        // after the namespace's (the order every writer of bindings uses), so two namespaces cannot disagree by racing.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("binding-profile:{tenant}:{subject}"))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let parse_all = |texts: Vec<String>| -> Result<Vec<BindingProfile>, StoreError> {
            texts
                .iter()
                .map(|text| {
                    BindingProfile::parse(text).ok_or_else(|| StoreError::Internal {
                        detail: "a stored binding profile is not a profile".into(),
                    })
                })
                .collect()
        };
        // The profile the identity has here is kept unless the caller names one; all its rows carry the same, and rows that
        // disagree are not guessed at.
        let here = parse_all(
            sqlx::query_scalar(
                "SELECT DISTINCT profile FROM worker_bindings \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
            )
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(subject)
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?,
        )?;
        // What it has in the other namespaces of the tenant.
        let elsewhere = parse_all(
            sqlx::query_scalar(
                "SELECT DISTINCT profile FROM worker_bindings \
                 WHERE tenant_id = $1 AND namespace_id <> $2 AND subject = $3",
            )
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(subject)
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?,
        )?;
        let disagree = |profiles: &[BindingProfile]| profiles.len() > 1;
        // (An unbind needs no profile, so a state that disagrees can still be cleared by removing its bindings.)
        if !patterns.is_empty() && (disagree(&elsewhere) || (profile.is_none() && disagree(&here)))
        {
            return Err(StoreError::Internal {
                detail: "the bindings of one identity disagree about its profile".into(),
            });
        }
        let profile = match profile {
            Some(named) => named,
            None => here
                .first()
                .or(elsewhere.first())
                .copied()
                .unwrap_or_default(),
        };
        if !patterns.is_empty() && elsewhere.first().is_some_and(|other| *other != profile) {
            return Err(StoreError::StateConflict(
                "the identity is bound with another profile in another namespace; unbind it there first",
            ));
        }
        let removed = sqlx::query(
            "DELETE FROM worker_bindings \
             WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if patterns.is_empty() && removed == 0 {
            // Nothing to remove: the transaction ends without a trace.
            return Ok(None);
        }
        // Pool grants go with the binding; a replacement that does not name pools keeps them.
        if patterns.is_empty() || pools.is_some() {
            sqlx::query(
                "DELETE FROM worker_binding_pools \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
            )
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(subject)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        if let Some(pools) = pools.filter(|pools| !pools.is_empty() && !patterns.is_empty()) {
            sqlx::query(
                "INSERT INTO worker_binding_pools \
                 (tenant_id, namespace_id, subject, pool, updated_by) \
                 SELECT $1, $2, $3, p, $5 FROM unnest($4::text[]) AS t(p)",
            )
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(subject)
            .bind(pools)
            .bind(changed_by)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        if !patterns.is_empty() {
            sqlx::query(
                "INSERT INTO worker_bindings (tenant_id, namespace_id, subject, pattern, updated_by, profile) \
                 SELECT $1, $2, $3, p, $5, $6 FROM unnest($4::text[]) AS t(p)",
            )
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(subject)
            .bind(patterns)
            .bind(changed_by)
            .bind(profile.as_str())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }

        // The sessions of this identity in this namespace that the new binding no longer covers are
        // closed, and the attempts they hold are released: the scheduler reclaims them as lost
        // leases, and the fencing makes the revoked worker's late reports stale.
        let sessions: Vec<(Uuid, Vec<String>)> = sqlx::query_as(
            "SELECT id, activities FROM worker_sessions \
             WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let doomed: Vec<Uuid> = sessions
            .into_iter()
            .filter(|(_, activities)| {
                // No longer covered, or a dedicated runner of another profile than the binding now has.
                first_uncovered(&parsed, activities).is_some()
                    || sagat_application::worker::managed_profile(activities)
                        .is_some_and(|offered| offered != profile)
            })
            .map(|(id, _)| id)
            .collect();
        let mut revoked = Revoked::default();
        if !doomed.is_empty() {
            let names: Vec<String> = doomed.iter().map(Uuid::to_string).collect();
            revoked.leases = sqlx::query(
                "UPDATE steps SET lease_expires_at = clock_timestamp(), updated_at = clock_timestamp() \
                 WHERE tenant_id = $1 AND status IN ('LEASED', 'RUNNING') AND leased_by = ANY($2)",
            )
            .bind(tenant.as_uuid())
            .bind(&names)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?
            .rows_affected();
            revoked.sessions =
                sqlx::query("DELETE FROM worker_sessions WHERE tenant_id = $1 AND id = ANY($2)")
                    .bind(tenant.as_uuid())
                    .bind(&doomed)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?
                    .rows_affected();
        }
        // The audit names the profile in force, whether the caller named it or the store kept it.
        let audit = if patterns.is_empty() {
            audit.clone()
        } else {
            audit.clone().with_reason(format!(
                "{}; profile: {profile}",
                audit.reason().unwrap_or_default()
            ))
        };
        insert_audit(&mut tx, &audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(Some(revoked))
    }
}
