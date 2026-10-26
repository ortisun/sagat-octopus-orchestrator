//! One transaction validates the fenced runner target and appends the bounded audit evidence.

use sagat_application::error::StoreError;
use sagat_application::scripts::{IncidentCategory, StoredIncidentTarget};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::binding::ActivityPattern;
use sagat_domain::identity::Principal;
use sagat_domain::scripts::digest_of_activity;
use sqlx::PgConnection;
use sqlx::Row as _;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{corrupt, insert_audit, store_error};
use crate::pool::BeginSafely;

const CATEGORIES: [&str; 7] = [
    "EGRESS_DISABLED",
    "UNSUPPORTED_SCHEME",
    "INVALID_HOST",
    "PORT_NOT_ALLOWED",
    "NOT_ALLOWLISTED",
    "FORBIDDEN_ADDRESS",
    "NO_ADDRESS",
];

/// Whether this production attempt already has category-only evidence. A terminal runner report
/// can then keep its diagnostic `denied` list while omitting the older URL-bearing audit duplicate.
pub(super) async fn production_was_recorded(
    conn: &mut PgConnection,
    tenant: sagat_domain::identity::TenantId,
    actor: &str,
    run_id: Uuid,
    node: &str,
    attempt: u32,
) -> Result<bool, StoreError> {
    // `steps` is the current projection, but `step_id` is stable across retries. The terminal
    // report can arrive after the projection advanced to another attempt, so use only the stable
    // identity here and match the reported historical attempt in the immutable audit resource.
    let step_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT step_id FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
    )
    .bind(tenant.as_uuid())
    .bind(run_id)
    .bind(node)
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    let Some(step_id) = step_id else {
        return Ok(false);
    };
    let resource_prefix = format!("run:{run_id}/step:{step_id}/attempt:{attempt}/generation:");
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM audit_records \
         WHERE tenant_id = $1 AND actor = $2 AND action = 'script.violation' \
           AND resource_type = 'run' AND starts_with(resource_id, $3) AND reason = ANY($4))",
    )
    .bind(tenant.as_uuid())
    .bind(actor)
    .bind(resource_prefix)
    .bind(CATEGORIES.as_slice())
    .fetch_one(conn)
    .await
    .map_err(store_error)
}

/// Test-run form of [`production_was_recorded`].
pub(super) async fn test_was_recorded(
    conn: &mut PgConnection,
    tenant: sagat_domain::identity::TenantId,
    actor: &str,
    run_id: Uuid,
    attempt: i32,
) -> Result<bool, StoreError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM audit_records \
         WHERE tenant_id = $1 AND actor = $2 AND action = 'script.violation' \
           AND resource_type = 'script' AND resource_id LIKE $3 AND reason = ANY($4))",
    )
    .bind(tenant.as_uuid())
    .bind(actor)
    .bind(format!("script:%/test:{run_id}/attempt:{attempt}"))
    .bind(CATEGORIES.as_slice())
    .fetch_one(conn)
    .await
    .map_err(store_error)
}

pub(super) async fn record(
    store: &PgRunStore,
    principal: &Principal,
    target: &StoredIncidentTarget,
    category: IncidentCategory,
    trace_id: Option<&str>,
    now: OffsetDateTime,
) -> Result<bool, StoreError> {
    match target {
        StoredIncidentTarget::Production {
            namespace_id,
            run_id,
            step_id,
            attempt,
            generation,
            token_hash,
        } => {
            let mut tx = store.pool().begin_safely().await.map_err(store_error)?;
            let tenant = principal.tenant_id();
            let subject = principal.subject().as_str();

            crate::execution::lock_member_chain(&mut tx, tenant, *namespace_id, *run_id).await?;

            let run_namespace: Option<Uuid> = sqlx::query_scalar(
                "SELECT namespace_id FROM runs WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
            )
            .bind(tenant.as_uuid())
            .bind(run_id.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            if run_namespace != Some(namespace_id.as_uuid()) {
                return Ok(false);
            }

            // Match report/heartbeat entry: member chain, own run, binding, then own step.
            // Binding revocation locks sessions/steps, but never acquires a run or chain.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!("binding:{tenant}:{namespace_id}:{subject}"))
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;

            // Every supplied fence field is checked against the actual current step. The expiry is
            // compared with PostgreSQL's clock so an application clock cannot extend authority.
            let row = sqlx::query(
                "SELECT namespace_id, activity, leased_by, lease_expires_at FROM steps \
                 WHERE tenant_id = $1 AND run_id = $2 AND step_id = $3 \
                   AND namespace_id = $4 AND attempt = $5 AND generation = $6 \
                   AND lease_token_hash = $7 AND status IN ('LEASED', 'RUNNING') FOR UPDATE",
            )
            .bind(tenant.as_uuid())
            .bind(run_id.as_uuid())
            .bind(step_id)
            .bind(namespace_id.as_uuid())
            .bind(i32::try_from(*attempt).unwrap_or(i32::MAX))
            .bind(i64::try_from(*generation).unwrap_or(i64::MAX))
            .bind(token_hash)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            let Some(row) = row else {
                return Ok(false);
            };
            let activity: Option<String> = row.try_get("activity").map_err(store_error)?;
            let leased_by: Option<String> = row.try_get("leased_by").map_err(store_error)?;
            let lease_expires_at: Option<OffsetDateTime> =
                row.try_get("lease_expires_at").map_err(store_error)?;
            let (Some(activity), Some(leased_by)) = (activity, leased_by) else {
                return Err(corrupt("a leased script step has no activity or worker"));
            };
            let Some(lease_expires_at) = lease_expires_at else {
                return Err(corrupt("a leased script step has no expiry"));
            };
            // This comparison runs only after the row lock has been acquired. A lease that expires
            // while this transaction waited for completion/timeout is stale even if no row changed.
            let unexpired: bool = sqlx::query_scalar("SELECT $1::timestamptz > clock_timestamp()")
                .bind(lease_expires_at)
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
            if !unexpired {
                return Ok(false);
            }
            let Some(digest) = digest_of_activity(&activity) else {
                return Ok(false);
            };
            let artifact_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM script_versions \
                 WHERE tenant_id = $1 AND artifact_digest = $2 AND status = 'READY')",
            )
            .bind(tenant.as_uuid())
            .bind(digest.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            if !artifact_exists {
                return Ok(false);
            }
            let worker_id = Uuid::parse_str(&leased_by)
                .map_err(|_| corrupt("a leased step has a malformed worker id"))?;

            let session_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM worker_sessions \
                 WHERE tenant_id = $1 AND id = $2 AND namespace_id = $3 AND subject = $4 \
                   AND expires_at > clock_timestamp())",
            )
            .bind(tenant.as_uuid())
            .bind(worker_id)
            .bind(namespace_id.as_uuid())
            .bind(subject)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            if !session_exists {
                return Ok(false);
            }

            let patterns: Vec<String> = sqlx::query_scalar(
                "SELECT pattern FROM worker_bindings \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
            )
            .bind(tenant.as_uuid())
            .bind(namespace_id.as_uuid())
            .bind(subject)
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?;
            let covered = patterns
                .iter()
                .map(|pattern| ActivityPattern::parse(pattern))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| corrupt("a stored binding pattern does not parse"))?
                .iter()
                .any(|pattern| pattern.matches(&activity));
            if !covered {
                return Ok(false);
            }

            let resource =
                format!("run:{run_id}/step:{step_id}/attempt:{attempt}/generation:{generation}");
            let audit = AuditRecord::by(
                principal,
                AuditAction::ScriptViolation,
                ResourceType::Run,
                resource,
                trace_id.map(str::to_owned),
                now,
            )
            .with_reason(category.as_str());
            insert_audit(&mut tx, &audit).await?;
            tx.commit().await.map_err(store_error)?;
            Ok(true)
        }
        StoredIncidentTarget::Test {
            namespace_id,
            test_run_id,
            token_hash,
        } => {
            let mut tx = store.pool().begin_safely().await.map_err(store_error)?;
            let row = sqlx::query(
                "SELECT script_id, attempts, lease_expires_at FROM script_test_runs \
                 WHERE tenant_id = $1 AND id = $2 AND namespace_id = $3 AND status = 'RUNNING' \
                   AND lease_token_hash = $4 FOR UPDATE",
            )
            .bind(principal.tenant_id().as_uuid())
            .bind(test_run_id.as_uuid())
            .bind(namespace_id.as_uuid())
            .bind(token_hash)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            let Some(row) = row else {
                return Ok(false);
            };
            let script_id: Uuid = row.try_get("script_id").map_err(store_error)?;
            let attempt: i32 = row.try_get("attempts").map_err(store_error)?;
            let lease_expires_at: Option<OffsetDateTime> =
                row.try_get("lease_expires_at").map_err(store_error)?;
            if attempt < 1 {
                return Err(corrupt("a running script test has no attempt"));
            }
            let Some(lease_expires_at) = lease_expires_at else {
                return Err(corrupt("a running script test has no lease expiry"));
            };
            let unexpired: bool = sqlx::query_scalar("SELECT $1::timestamptz > clock_timestamp()")
                .bind(lease_expires_at)
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
            if !unexpired {
                return Ok(false);
            }
            let resource = format!("script:{script_id}/test:{test_run_id}/attempt:{attempt}");
            let audit = AuditRecord::by(
                principal,
                AuditAction::ScriptViolation,
                ResourceType::Script,
                resource,
                trace_id.map(str::to_owned),
                now,
            )
            .with_reason(category.as_str());
            insert_audit(&mut tx, &audit).await?;
            tx.commit().await.map_err(store_error)?;
            Ok(true)
        }
    }
}
