//! Scripts, their versions, artifacts, build jobs, test runs and executions in PostgreSQL
//! (`FR-005-001`, `FR-005-005`, `FR-005-007`, `FR-005-008`).
//!
//! What matters here is what happens in one transaction: a version and the job that builds it are
//! created together; a build finishes (the artifact is stored, the version becomes ready, the job
//! succeeds, the audit entry is written) or does not; a lease is taken under a lock that skips what
//! another runner is taking, and everything its holder reports is checked against the hash of the
//! token only that holder knows.

use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::scripts::{
    ArtifactRecord, ArtifactSummary, BuildJobRecord, BuiltArtifact, ClaimedBuild, ClaimedTest,
    Deleted, Draft, DraftUpdate, ExecutionRecord, ExecutionStatus, ExecutionSummary, Finished, Hold,
    MAX_BUILD_ATTEMPTS, NewExecution, NewVersion, RunnerStore, ScriptLookup, ScriptRecord,
    ScriptStore, TEST_QUEUE_WAIT, TestReport, TestRunRecord, TestRunSummary, VersionRecord,
};
use sagat_domain::audit::AuditRecord;
use sagat_domain::identity::{NamespaceId, Slug, TenantId};
use sagat_domain::scripts::{
    BuildJobId, Digest, Failure, JobStatus, Language, Lockfile, Policy, Retention, ScriptId,
    ScriptVersionId, Source, TestRunId, VersionNumber, VersionStatus,
};
use sqlx::PgConnection;
use sqlx::postgres::PgRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::quotas::{
    ensure_room_for_script, ensure_room_for_script_job, lock_script_jobs, lock_scripts,
};

/// How many times a test run is taken before its runner is presumed lost for good.
const MAX_TEST_ATTEMPTS: i32 = 3;

macro_rules! select_scripts {
    ($tail:literal) => {
        concat!(
            "SELECT id, tenant_id, namespace_id, name, language, source, lockfile, input_schema, ",
            "output_schema, policy, retention_days, revision, latest_version, created_by, ",
            "created_at, updated_at FROM scripts ",
            $tail
        )
    };
}

macro_rules! select_versions {
    ($tail:literal) => {
        concat!(
            "SELECT id, tenant_id, namespace_id, script_id, number, language, status, source, ",
            "lockfile, input_schema, output_schema, policy, retention_days, content_digest, ",
            "build_key, artifact_digest, key_id, toolchain, build_log, failure, created_by, ",
            "created_at, built_at FROM script_versions ",
            $tail
        )
    };
}

macro_rules! select_tests {
    ($tail:literal) => {
        concat!(
            "SELECT id, tenant_id, namespace_id, script_id, language, status, source, lockfile, ",
            "input_schema, output_schema, policy, retention_days, input, result, failure, log, ",
            "log_truncated, denied, exit_code, wall_ms, start_ms, memory_peak_bytes, ",
            "inferred_input_schema, inferred_output_schema, attempts, created_by, created_at, ",
            "finished_at, lease_expires_at FROM script_test_runs ",
            $tail
        )
    };
}

fn language_of(row: &PgRow) -> Result<Language, StoreError> {
    col::<String>(row, "language")?
        .parse()
        .map_err(|_| corrupt("script language"))
}

fn digest_of(text: &str, what: &str) -> Result<Digest, StoreError> {
    Digest::parse(text).map_err(|_| corrupt(what))
}

fn optional_digest(row: &PgRow, column: &str) -> Result<Option<Digest>, StoreError> {
    col::<Option<String>>(row, column)?
        .map(|text| digest_of(&text, "script digest"))
        .transpose()
}

fn draft_of(row: &PgRow) -> Result<Draft, StoreError> {
    let source: String = col(row, "source")?;
    let lockfile: Option<String> = col(row, "lockfile")?;
    let policy: String = col(row, "policy")?;
    let retention: i16 = col(row, "retention_days")?;
    Ok(Draft {
        source: Source::parse(&source).map_err(|_| corrupt("script source"))?,
        lockfile: lockfile
            .map(|text| Lockfile::parse(&text).map_err(|_| corrupt("script lockfile")))
            .transpose()?,
        input_schema: col(row, "input_schema")?,
        output_schema: col(row, "output_schema")?,
        policy: Policy::parse_canonical(&policy).map_err(|_| corrupt("script policy"))?,
        retention: u16::try_from(retention)
            .ok()
            .and_then(|days| Retention::days(days).ok())
            .ok_or_else(|| corrupt("script retention"))?,
    })
}

fn script_of(row: &PgRow) -> Result<ScriptRecord, StoreError> {
    let name: String = col(row, "name")?;
    let revision: i64 = col(row, "revision")?;
    let latest: Option<i32> = col(row, "latest_version")?;
    Ok(ScriptRecord {
        id: ScriptId::new(col::<Uuid>(row, "id")?),
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        name: Slug::parse(&name).map_err(|_| corrupt("script name"))?,
        language: language_of(row)?,
        draft: draft_of(row)?,
        revision: u64::try_from(revision).map_err(|_| corrupt("script revision"))?,
        latest_version: latest
            .map(|n| {
                u32::try_from(n)
                    .ok()
                    .and_then(|n| VersionNumber::new(n).ok())
                    .ok_or_else(|| corrupt("script latest version"))
            })
            .transpose()?,
        created_by: col(row, "created_by")?,
        created_at: col(row, "created_at")?,
        updated_at: col(row, "updated_at")?,
    })
}

fn version_of(row: &PgRow) -> Result<VersionRecord, StoreError> {
    let number: i32 = col(row, "number")?;
    let status: String = col(row, "status")?;
    let content: String = col(row, "content_digest")?;
    Ok(VersionRecord {
        id: ScriptVersionId::new(col::<Uuid>(row, "id")?),
        script_id: ScriptId::new(col::<Uuid>(row, "script_id")?),
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        number: u32::try_from(number)
            .ok()
            .and_then(|n| VersionNumber::new(n).ok())
            .ok_or_else(|| corrupt("script version number"))?,
        language: language_of(row)?,
        status: status
            .parse::<VersionStatus>()
            .map_err(|_| corrupt("script version status"))?,
        draft: draft_of(row)?,
        content_digest: digest_of(&content, "script content digest")?,
        build_key: optional_digest(row, "build_key")?,
        artifact: optional_digest(row, "artifact_digest")?,
        key_id: col(row, "key_id")?,
        toolchain: col(row, "toolchain")?,
        build_log: col(row, "build_log")?,
        failure: col(row, "failure")?,
        created_by: col(row, "created_by")?,
        created_at: col(row, "created_at")?,
        built_at: col(row, "built_at")?,
    })
}

fn count(value: Option<i64>) -> Option<u64> {
    value.and_then(|v| u64::try_from(v).ok())
}

fn test_run_of(row: &PgRow) -> Result<TestRunRecord, StoreError> {
    let status: String = col(row, "status")?;
    let failure: Option<String> = col(row, "failure")?;
    let attempts: i32 = col(row, "attempts")?;
    Ok(TestRunRecord {
        id: TestRunId::new(col::<Uuid>(row, "id")?),
        script_id: ScriptId::new(col::<Uuid>(row, "script_id")?),
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        language: language_of(row)?,
        status: status
            .parse::<JobStatus>()
            .map_err(|_| corrupt("test run status"))?,
        draft: draft_of(row)?,
        input: col(row, "input")?,
        result: col(row, "result")?,
        failure: failure
            .map(|code| Failure::from_code(&code).ok_or_else(|| corrupt("test run failure")))
            .transpose()?,
        log: col(row, "log")?,
        log_truncated: col(row, "log_truncated")?,
        denied: col(row, "denied")?,
        exit_code: col(row, "exit_code")?,
        wall_ms: count(col(row, "wall_ms")?),
        start_ms: count(col(row, "start_ms")?),
        memory_peak_bytes: count(col(row, "memory_peak_bytes")?),
        inferred_input_schema: col(row, "inferred_input_schema")?,
        inferred_output_schema: col(row, "inferred_output_schema")?,
        attempts: u32::try_from(attempts).unwrap_or(0),
        created_by: col(row, "created_by")?,
        created_at: col(row, "created_at")?,
        finished_at: col(row, "finished_at")?,
    })
}

fn artifact_of(row: &PgRow) -> Result<ArtifactRecord, StoreError> {
    let digest: String = col(row, "digest")?;
    Ok(ArtifactRecord {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        digest: digest_of(&digest, "artifact digest")?,
        language: language_of(row)?,
        content: col(row, "content")?,
        signature: col(row, "signature")?,
        key_id: col(row, "key_id")?,
        sbom: col(row, "sbom")?,
        toolchain: col(row, "toolchain")?,
        created_at: col(row, "created_at")?,
    })
}

fn execution_summary_of(row: &PgRow) -> Result<ExecutionSummary, StoreError> {
    let record = execution_of_columns(row, false)?;
    let e = record.execution;
    Ok(ExecutionSummary {
        id: record.id,
        digest: e.digest,
        run_id: e.run_id,
        node: e.node,
        attempt: e.attempt,
        status: e.status,
        failure: e.failure,
        exit_code: e.exit_code,
        log_truncated: e.log_truncated,
        denied: e.denied,
        wall_ms: e.wall_ms,
        start_ms: e.start_ms,
        memory_peak_bytes: e.memory_peak_bytes,
        trace_id: e.trace_id,
        created_at: record.created_at,
        expires_at: record.expires_at,
    })
}

fn test_run_summary_of(row: &PgRow) -> Result<TestRunSummary, StoreError> {
    let status: String = col(row, "status")?;
    let failure: Option<String> = col(row, "failure")?;
    Ok(TestRunSummary {
        id: TestRunId::new(col::<Uuid>(row, "id")?),
        script_id: ScriptId::new(col::<Uuid>(row, "script_id")?),
        language: language_of(row)?,
        status: status
            .parse::<JobStatus>()
            .map_err(|_| corrupt("test run status"))?,
        failure: failure
            .map(|code| Failure::from_code(&code).ok_or_else(|| corrupt("test run failure")))
            .transpose()?,
        wall_ms: count(col(row, "wall_ms")?),
        start_ms: count(col(row, "start_ms")?),
        created_by: col(row, "created_by")?,
        created_at: col(row, "created_at")?,
        finished_at: col(row, "finished_at")?,
    })
}

fn execution_of(row: &PgRow) -> Result<ExecutionRecord, StoreError> {
    execution_of_columns(row, true)
}

fn execution_of_columns(row: &PgRow, with_log: bool) -> Result<ExecutionRecord, StoreError> {
    let digest: String = col(row, "digest")?;
    let status: String = col(row, "status")?;
    let failure: Option<String> = col(row, "failure")?;
    let attempt: Option<i32> = col(row, "attempt")?;
    let wall: i64 = col(row, "wall_ms")?;
    let start: i64 = col(row, "start_ms")?;
    Ok(ExecutionRecord {
        id: col(row, "id")?,
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        created_at: col(row, "created_at")?,
        expires_at: col(row, "expires_at")?,
        execution: NewExecution {
            namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
            digest: digest_of(&digest, "execution digest")?,
            run_id: col(row, "run_id")?,
            node: col(row, "node")?,
            attempt: attempt.and_then(|a| u32::try_from(a).ok()),
            status: match status.as_str() {
                "SUCCEEDED" => ExecutionStatus::Succeeded,
                "FAILED" => ExecutionStatus::Failed,
                _ => return Err(corrupt("execution status")),
            },
            failure: failure
                .map(|code| Failure::from_code(&code).ok_or_else(|| corrupt("execution failure")))
                .transpose()?,
            exit_code: col(row, "exit_code")?,
            log: if with_log { col(row, "log")? } else { String::new() },
            log_truncated: col(row, "log_truncated")?,
            denied: col(row, "denied")?,
            wall_ms: u64::try_from(wall).unwrap_or(0),
            start_ms: u64::try_from(start).unwrap_or(0),
            memory_peak_bytes: count(col(row, "memory_peak_bytes")?),
            trace_id: col(row, "trace_id")?,
        },
    })
}

fn seconds(duration: Duration) -> f64 {
    duration.as_secs_f64()
}

fn bound(value: Option<u64>) -> Option<i64> {
    value.map(|v| i64::try_from(v).unwrap_or(i64::MAX))
}

/// A build job found by its token, with its version.
async fn held_build_in(
    conn: &mut PgConnection,
    tenant: TenantId,
    job: BuildJobId,
    token_hash: &[u8],
    lock: bool,
) -> Result<Option<(ClaimedBuild, JobStatus)>, StoreError> {
    let row = if lock {
        sqlx::query(
            "SELECT j.id AS job_id, j.version_id, j.attempts, j.status AS job_status, \
                    j.lease_expires_at \
             FROM script_build_jobs j \
             WHERE j.tenant_id = $1 AND j.id = $2 AND j.lease_token_hash = $3 FOR UPDATE",
        )
    } else {
        sqlx::query(
            "SELECT j.id AS job_id, j.version_id, j.attempts, j.status AS job_status, \
                    j.lease_expires_at \
             FROM script_build_jobs j \
             WHERE j.tenant_id = $1 AND j.id = $2 AND j.lease_token_hash = $3",
        )
    }
    .bind(tenant.as_uuid())
    .bind(job.as_uuid())
    .bind(token_hash)
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    let Some(row) = row else { return Ok(None) };
    let version_id: Uuid = col(&row, "version_id")?;
    let attempts: i32 = col(&row, "attempts")?;
    let status: String = col(&row, "job_status")?;
    let expires: Option<OffsetDateTime> = col(&row, "lease_expires_at")?;
    let version = sqlx::query(select_versions!("WHERE tenant_id = $1 AND id = $2"))
        .bind(tenant.as_uuid())
        .bind(version_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(store_error)?;
    Ok(Some((
        ClaimedBuild {
            job: BuildJobRecord {
                id: job,
                version_id: ScriptVersionId::new(version_id),
                attempts: u32::try_from(attempts).unwrap_or(0),
                lease_expires_at: expires.unwrap_or(OffsetDateTime::UNIX_EPOCH),
            },
            version: version_of(&version)?,
        },
        status.parse().map_err(|_| corrupt("build job status"))?,
    )))
}

async fn missing(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    digests: &[Digest],
) -> Result<Vec<Digest>, StoreError> {
    let wanted: Vec<&str> = digests.iter().map(Digest::as_str).collect();
    let missing: Vec<String> = sqlx::query_scalar(
        "SELECT wanted FROM unnest($2::text[]) AS wanted \
         WHERE NOT EXISTS (SELECT 1 FROM script_versions v \
                            WHERE v.tenant_id = $1 AND v.artifact_digest = wanted \
                              AND v.status = 'READY')",
    )
    .bind(tenant.as_uuid())
    .bind(&wanted)
    .fetch_all(pool)
    .await
    .map_err(store_error)?;
    missing
        .iter()
        .map(|text| digest_of(text, "script digest"))
        .collect()
}

impl ScriptLookup for PgRunStore {
    async fn missing_artifacts(
        &self,
        tenant: TenantId,
        digests: &[Digest],
    ) -> Result<Vec<Digest>, StoreError> {
        missing(&self.pool, tenant, digests).await
    }
}

impl ScriptLookup for crate::definitions::PgDefinitionStore {
    async fn missing_artifacts(
        &self,
        tenant: TenantId,
        digests: &[Digest],
    ) -> Result<Vec<Digest>, StoreError> {
        missing(self.pool(), tenant, digests).await
    }
}

impl ScriptStore for PgRunStore {
    async fn create_script(
        &self,
        script: &ScriptRecord,
        limit: u64,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        lock_scripts(&mut tx, script.tenant_id, script.namespace_id).await?;
        ensure_room_for_script(&mut tx, script.tenant_id, script.namespace_id, limit).await?;
        let draft = &script.draft;
        sqlx::query(
            "INSERT INTO scripts \
             (id, tenant_id, namespace_id, name, language, source, lockfile, input_schema, \
              output_schema, policy, retention_days, revision, created_by, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(script.id.as_uuid())
        .bind(script.tenant_id.as_uuid())
        .bind(script.namespace_id.as_uuid())
        .bind(script.name.as_str())
        .bind(script.language.as_str())
        .bind(draft.source.as_str())
        .bind(draft.lockfile.as_ref().map(Lockfile::as_str))
        .bind(draft.input_schema.as_deref())
        .bind(draft.output_schema.as_deref())
        .bind(draft.policy.canonical())
        .bind(i16::try_from(draft.retention.get()).unwrap_or(14))
        .bind(i64::try_from(script.revision).unwrap_or(1))
        .bind(&script.created_by)
        .bind(script.created_at)
        .bind(script.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn scripts(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ScriptRecord>, StoreError> {
        sqlx::query(select_scripts!(
            "WHERE tenant_id = $1 AND namespace_id = $2 AND ($3::text IS NULL OR name > $3) \
             ORDER BY name LIMIT $4"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(after)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(script_of)
        .collect()
    }

    async fn script(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        id: ScriptId,
    ) -> Result<Option<ScriptRecord>, StoreError> {
        sqlx::query(select_scripts!(
            "WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .map(|row| script_of(&row))
        .transpose()
    }

    async fn update_draft(
        &self,
        update: DraftUpdate<'_>,
        audit: &AuditRecord,
    ) -> Result<Option<ScriptRecord>, StoreError> {
        let DraftUpdate {
            tenant,
            namespace,
            id,
            draft,
            expected_revision,
            now,
        } = update;
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        let updated = sqlx::query(
            "UPDATE scripts SET source = $5, lockfile = $6, input_schema = $7, output_schema = $8, \
                 policy = $9, retention_days = $10, revision = revision + 1, updated_at = $11 \
             WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 AND revision = $4 \
             RETURNING id, tenant_id, namespace_id, name, language, source, lockfile, \
                       input_schema, output_schema, policy, retention_days, revision, \
                       latest_version, created_by, created_at, updated_at",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(id.as_uuid())
        .bind(i64::try_from(expected_revision).unwrap_or(i64::MAX))
        .bind(draft.source.as_str())
        .bind(draft.lockfile.as_ref().map(Lockfile::as_str))
        .bind(draft.input_schema.as_deref())
        .bind(draft.output_schema.as_deref())
        .bind(draft.policy.canonical())
        .bind(i16::try_from(draft.retention.get()).unwrap_or(14))
        .bind(now)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        if let Some(row) = updated {
            insert_audit(&mut tx, audit).await?;
            let script = script_of(&row)?;
            tx.commit().await.map_err(store_error)?;
            return Ok(Some(script));
        }
        let current: Option<i64> = sqlx::query_scalar(
            "SELECT revision FROM scripts WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        match current {
            Some(current) => Err(StoreError::RevisionConflict {
                current: u64::try_from(current).unwrap_or(0),
            }),
            None => Ok(None),
        }
    }

    async fn delete_script(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        id: ScriptId,
        audit: &AuditRecord,
    ) -> Result<Deleted, StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        // The lock a publication takes, so a version cannot appear between the check and the delete.
        let exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM scripts WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        if exists.is_none() {
            return Ok(Deleted::NotFound);
        }
        let has_versions: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM script_versions WHERE tenant_id = $1 AND script_id = $2)",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if has_versions {
            return Ok(Deleted::HasVersions);
        }
        sqlx::query("DELETE FROM scripts WHERE tenant_id = $1 AND id = $2")
            .bind(tenant.as_uuid())
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(Deleted::Done)
    }

    async fn publish_version(
        &self,
        version: &NewVersion,
        jobs_limit: u64,
        audit: &AuditRecord,
    ) -> Result<Option<(VersionRecord, bool)>, StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        // The script row is the lock of its versions: numbers are assigned one at a time.
        let row = sqlx::query(
            "SELECT latest_version, revision FROM scripts \
             WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 FOR UPDATE",
        )
        .bind(version.tenant_id.as_uuid())
        .bind(version.namespace_id.as_uuid())
        .bind(version.script_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(row) = row else { return Ok(None) };
        let latest: Option<i32> = col(&row, "latest_version")?;
        let revision: i64 = col(&row, "revision")?;
        if u64::try_from(revision).ok() != Some(version.draft_revision) {
            return Err(StoreError::RevisionConflict {
                current: u64::try_from(revision).unwrap_or(0),
            });
        }
        if let Some(latest) = latest {
            let existing = sqlx::query(select_versions!(
                "WHERE tenant_id = $1 AND script_id = $2 AND number = $3"
            ))
            .bind(version.tenant_id.as_uuid())
            .bind(version.script_id.as_uuid())
            .bind(latest)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            let existing = version_of(&existing)?;
            // Publishing what is already the latest version is not another version. A version whose
            // build failed is the exception: the same source may build now (the runner was down).
            if existing.content_digest == version.content_digest
                && existing.status != VersionStatus::Failed
            {
                return Ok(Some((existing, false)));
            }
        }
        lock_script_jobs(&mut tx, version.tenant_id, version.namespace_id).await?;
        ensure_room_for_script_job(&mut tx, version.tenant_id, version.namespace_id, jobs_limit)
            .await?;
        let number = latest.unwrap_or(0) + 1;
        let draft = &version.draft;
        sqlx::query(
            "INSERT INTO script_versions \
             (id, tenant_id, namespace_id, script_id, number, language, status, source, lockfile, \
              input_schema, output_schema, policy, retention_days, content_digest, created_by, \
              created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, 'BUILDING', $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(version.id.as_uuid())
        .bind(version.tenant_id.as_uuid())
        .bind(version.namespace_id.as_uuid())
        .bind(version.script_id.as_uuid())
        .bind(number)
        .bind(version.language.as_str())
        .bind(draft.source.as_str())
        .bind(draft.lockfile.as_ref().map(Lockfile::as_str))
        .bind(draft.input_schema.as_deref())
        .bind(draft.output_schema.as_deref())
        .bind(draft.policy.canonical())
        .bind(i16::try_from(draft.retention.get()).unwrap_or(14))
        .bind(version.content_digest.as_str())
        .bind(&version.created_by)
        .bind(version.created_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO script_build_jobs \
             (id, tenant_id, namespace_id, version_id, language, status, created_at) \
             VALUES ($1, $2, $3, $4, $5, 'QUEUED', $6)",
        )
        .bind(version.job_id.as_uuid())
        .bind(version.tenant_id.as_uuid())
        .bind(version.namespace_id.as_uuid())
        .bind(version.id.as_uuid())
        .bind(version.language.as_str())
        .bind(version.created_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query("UPDATE scripts SET latest_version = $3 WHERE tenant_id = $1 AND id = $2")
            .bind(version.tenant_id.as_uuid())
            .bind(version.script_id.as_uuid())
            .bind(number)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        let stored = sqlx::query(select_versions!("WHERE tenant_id = $1 AND id = $2"))
            .bind(version.tenant_id.as_uuid())
            .bind(version.id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        let stored = version_of(&stored)?;
        tx.commit().await.map_err(store_error)?;
        Ok(Some((stored, true)))
    }

    async fn versions(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        script: ScriptId,
        before: Option<VersionNumber>,
        limit: u32,
    ) -> Result<Vec<VersionRecord>, StoreError> {
        sqlx::query(select_versions!(
            "WHERE tenant_id = $1 AND namespace_id = $2 AND script_id = $3 \
               AND ($4::int IS NULL OR number < $4) ORDER BY number DESC LIMIT $5"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(script.as_uuid())
        .bind(before.map(|n| i32::try_from(n.get()).unwrap_or(i32::MAX)))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(version_of)
        .collect()
    }

    async fn version(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        script: ScriptId,
        number: VersionNumber,
    ) -> Result<Option<VersionRecord>, StoreError> {
        sqlx::query(select_versions!(
            "WHERE tenant_id = $1 AND namespace_id = $2 AND script_id = $3 AND number = $4"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(script.as_uuid())
        .bind(i32::try_from(number.get()).unwrap_or(i32::MAX))
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .map(|row| version_of(&row))
        .transpose()
    }

    async fn create_test_run(
        &self,
        run: &TestRunRecord,
        jobs_limit: u64,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        lock_script_jobs(&mut tx, run.tenant_id, run.namespace_id).await?;
        // Test runs nobody took in time are failed now, so they do not hold a place in the quota.
        sqlx::query(
            "UPDATE script_test_runs SET status = 'FAILED', failure = 'SCRIPT_SANDBOX_UNAVAILABLE', \
                 log = 'no runner took the test run in time', finished_at = $3 \
             WHERE tenant_id = $1 AND namespace_id = $2 AND status = 'QUEUED' AND deadline_at < $3",
        )
        .bind(run.tenant_id.as_uuid())
        .bind(run.namespace_id.as_uuid())
        .bind(run.created_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        ensure_room_for_script_job(&mut tx, run.tenant_id, run.namespace_id, jobs_limit).await?;
        let draft = &run.draft;
        sqlx::query(
            "INSERT INTO script_test_runs \
             (id, tenant_id, namespace_id, script_id, language, status, source, lockfile, \
              input_schema, output_schema, policy, retention_days, input, created_by, created_at, \
              deadline_at) \
             VALUES ($1, $2, $3, $4, $5, 'QUEUED', $6, $7, $8, $9, $10, $11, $12, $13, $14, \
                     $14 + make_interval(secs => $15))",
        )
        .bind(run.id.as_uuid())
        .bind(run.tenant_id.as_uuid())
        .bind(run.namespace_id.as_uuid())
        .bind(run.script_id.as_uuid())
        .bind(run.language.as_str())
        .bind(draft.source.as_str())
        .bind(draft.lockfile.as_ref().map(Lockfile::as_str))
        .bind(draft.input_schema.as_deref())
        .bind(draft.output_schema.as_deref())
        .bind(draft.policy.canonical())
        .bind(i16::try_from(draft.retention.get()).unwrap_or(14))
        .bind(&run.input)
        .bind(&run.created_by)
        .bind(run.created_at)
        .bind(seconds(TEST_QUEUE_WAIT))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn test_run(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        script: ScriptId,
        id: TestRunId,
        now: OffsetDateTime,
    ) -> Result<Option<TestRunRecord>, StoreError> {
        // A run that waited too long reads as failed: nobody is going to take it.
        sqlx::query(
            "UPDATE script_test_runs SET status = 'FAILED', failure = 'SCRIPT_SANDBOX_UNAVAILABLE', \
                 log = 'no runner took the test run in time', finished_at = $5 \
             WHERE tenant_id = $1 AND namespace_id = $2 AND script_id = $3 AND id = $4 \
               AND status = 'QUEUED' AND deadline_at < $5",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(script.as_uuid())
        .bind(id.as_uuid())
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        sqlx::query(select_tests!(
            "WHERE tenant_id = $1 AND namespace_id = $2 AND script_id = $3 AND id = $4"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(script.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .map(|row| test_run_of(&row))
        .transpose()
    }

    async fn test_runs(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        script: ScriptId,
        limit: u32,
    ) -> Result<Vec<TestRunSummary>, StoreError> {
        sqlx::query(
            "SELECT id, script_id, language, status, failure, wall_ms, start_ms, created_by, \
                    created_at, finished_at \
             FROM script_test_runs \
             WHERE tenant_id = $1 AND namespace_id = $2 AND script_id = $3 \
             ORDER BY id DESC LIMIT $4",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(script.as_uuid())
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(test_run_summary_of)
        .collect()
    }

    async fn executions(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        digest: Option<&Digest>,
        before: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<ExecutionSummary>, StoreError> {
        sqlx::query(
            "SELECT id, tenant_id, namespace_id, digest, run_id, node, attempt, status, failure, \
                    exit_code, log_truncated, denied, wall_ms, start_ms, memory_peak_bytes, \
                    trace_id, created_at, expires_at \
             FROM script_executions \
             WHERE tenant_id = $1 AND namespace_id = $2 AND ($3::text IS NULL OR digest = $3) \
               AND ($4::uuid IS NULL OR id < $4) \
             ORDER BY id DESC LIMIT $5",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(digest.map(Digest::as_str))
        .bind(before)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(execution_summary_of)
        .collect()
    }

    async fn execution(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        id: Uuid,
    ) -> Result<Option<ExecutionRecord>, StoreError> {
        sqlx::query(
            "SELECT id, tenant_id, namespace_id, digest, run_id, node, attempt, status, failure, \
                    exit_code, log, log_truncated, denied, wall_ms, start_ms, memory_peak_bytes, \
                    trace_id, created_at, expires_at \
             FROM script_executions \
             WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .map(|row| execution_of(&row))
        .transpose()
    }

    async fn artifact_summary(
        &self,
        tenant: TenantId,
        digest: &Digest,
    ) -> Result<Option<ArtifactSummary>, StoreError> {
        let Some(row) = sqlx::query(
            "SELECT digest, language, octet_length(content) AS size, signature, key_id, sbom, \
                    toolchain, created_at \
             FROM script_artifacts WHERE tenant_id = $1 AND digest = $2",
        )
        .bind(tenant.as_uuid())
        .bind(digest.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let size: i32 = col(&row, "size")?;
        let found: String = col(&row, "digest")?;
        Ok(Some(ArtifactSummary {
            digest: digest_of(&found, "artifact digest")?,
            language: language_of(&row)?,
            size: u64::try_from(size).unwrap_or(0),
            signature: col(&row, "signature")?,
            key_id: col(&row, "key_id")?,
            sbom: col(&row, "sbom")?,
            toolchain: col(&row, "toolchain")?,
            created_at: col(&row, "created_at")?,
        }))
    }
}

impl RunnerStore for PgRunStore {
    async fn claim_build(
        &self,
        tenant: TenantId,
        languages: &[Language],
        token_hash: &[u8],
        lease: Duration,
        now: OffsetDateTime,
    ) -> Result<Option<ClaimedBuild>, StoreError> {
        let names: Vec<&str> = languages.iter().map(|l| l.as_str()).collect();
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        // A build whose runner vanished and whose attempts are used up is over.
        let lost: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE script_build_jobs SET status = 'FAILED', finished_at = $3, lease_token_hash = NULL \
             WHERE tenant_id = $1 AND status = 'RUNNING' AND lease_expires_at < $3 AND attempts >= $2 \
             RETURNING version_id",
        )
        .bind(tenant.as_uuid())
        .bind(i32::try_from(MAX_BUILD_ATTEMPTS).unwrap_or(3))
        .bind(now)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        for version in lost {
            sqlx::query(
                "UPDATE script_versions SET status = 'FAILED', built_at = $3, \
                     failure = 'unavailable: the build was lost by its runner too many times' \
                 WHERE tenant_id = $1 AND id = $2 AND status = 'BUILDING'",
            )
            .bind(tenant.as_uuid())
            .bind(version)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        let Some(row) = sqlx::query(
            "SELECT id, version_id FROM script_build_jobs \
             WHERE tenant_id = $1 AND language = ANY($2) \
               AND (status = 'QUEUED' OR (status = 'RUNNING' AND lease_expires_at < $3)) \
             ORDER BY created_at, id LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .bind(tenant.as_uuid())
        .bind(&names)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        else {
            tx.commit().await.map_err(store_error)?;
            return Ok(None);
        };
        let id: Uuid = col(&row, "id")?;
        sqlx::query(
            "UPDATE script_build_jobs SET status = 'RUNNING', attempts = attempts + 1, \
                 lease_token_hash = $3, lease_expires_at = $4 + make_interval(secs => $5) \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(token_hash)
        .bind(now)
        .bind(seconds(lease))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let held = held_build_in(&mut tx, tenant, BuildJobId::new(id), token_hash, false).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(held.map(|(claimed, _)| claimed))
    }

    async fn held_build(
        &self,
        tenant: TenantId,
        job: BuildJobId,
        token_hash: &[u8],
    ) -> Result<Option<ClaimedBuild>, StoreError> {
        let mut conn = self.pool.acquire().await.map_err(store_error)?;
        Ok(held_build_in(&mut conn, tenant, job, token_hash, false)
            .await?
            .map(|(claimed, _)| claimed))
    }

    async fn heartbeat_build(
        &self,
        tenant: TenantId,
        job: BuildJobId,
        token_hash: &[u8],
        lease: Duration,
        now: OffsetDateTime,
    ) -> Result<Option<OffsetDateTime>, StoreError> {
        sqlx::query_scalar(
            "UPDATE script_build_jobs SET lease_expires_at = $4 + make_interval(secs => $5) \
             WHERE tenant_id = $1 AND id = $2 AND lease_token_hash = $3 AND status = 'RUNNING' \
             RETURNING lease_expires_at",
        )
        .bind(tenant.as_uuid())
        .bind(job.as_uuid())
        .bind(token_hash)
        .bind(now)
        .bind(seconds(lease))
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn built_with(
        &self,
        tenant: TenantId,
        key: &Digest,
    ) -> Result<Option<Digest>, StoreError> {
        let found: Option<String> = sqlx::query_scalar(
            "SELECT artifact_digest FROM script_versions \
             WHERE tenant_id = $1 AND build_key = $2 AND status = 'READY' LIMIT 1",
        )
        .bind(tenant.as_uuid())
        .bind(key.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        found
            .map(|text| digest_of(&text, "script digest"))
            .transpose()
    }

    async fn finish_build_cached(
        &self,
        hold: Hold<'_>,
        key: &Digest,
        artifact: &Digest,
        audit: &AuditRecord,
    ) -> Result<Finished, StoreError> {
        let Hold {
            tenant,
            job,
            token_hash,
            now,
        } = hold;
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        let Some((claimed, status)) = held_build_in(&mut tx, tenant, job, token_hash, true).await?
        else {
            return Ok(Finished::Stale);
        };
        if status.is_final() {
            return Ok(Finished::AlreadyDone);
        }
        let done = sqlx::query(
            "UPDATE script_versions v SET status = 'READY', build_key = $3, artifact_digest = $4, \
                 key_id = a.key_id, toolchain = a.toolchain, built_at = $5, \
                 build_log = 'reused the artifact of an identical build' \
             FROM script_artifacts a \
             WHERE v.tenant_id = $1 AND v.id = $2 AND v.status = 'BUILDING' \
               AND a.tenant_id = v.tenant_id AND a.digest = $4",
        )
        .bind(tenant.as_uuid())
        .bind(claimed.version.id.as_uuid())
        .bind(key.as_str())
        .bind(artifact.as_str())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if done == 0 {
            return Ok(Finished::Stale);
        }
        sqlx::query(
            "UPDATE script_build_jobs SET status = 'SUCCEEDED', finished_at = $3 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(job.as_uuid())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(Finished::Done)
    }

    async fn finish_build(
        &self,
        hold: Hold<'_>,
        built: &BuiltArtifact,
        audit: &AuditRecord,
    ) -> Result<Finished, StoreError> {
        let Hold {
            tenant,
            job,
            token_hash,
            now,
        } = hold;
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        let Some((claimed, status)) = held_build_in(&mut tx, tenant, job, token_hash, true).await?
        else {
            return Ok(Finished::Stale);
        };
        if status.is_final() {
            return Ok(Finished::AlreadyDone);
        }
        let artifact = &built.artifact;
        // An artifact with this digest may exist already (the same bytes from another build): it
        // stays as it is, with the signature it has.
        sqlx::query(
            "INSERT INTO script_artifacts \
             (tenant_id, digest, language, content, signature, key_id, sbom, toolchain, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (tenant_id, digest) DO NOTHING",
        )
        .bind(tenant.as_uuid())
        .bind(artifact.digest.as_str())
        .bind(artifact.language.as_str())
        .bind(&artifact.content)
        .bind(&artifact.signature)
        .bind(&artifact.key_id)
        .bind(&artifact.sbom)
        .bind(&artifact.toolchain)
        .bind(artifact.created_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE script_versions v SET status = 'READY', build_key = $3, artifact_digest = $4, \
                 key_id = a.key_id, toolchain = $5, built_at = $6, build_log = $7 \
             FROM script_artifacts a \
             WHERE v.tenant_id = $1 AND v.id = $2 AND v.status = 'BUILDING' \
               AND a.tenant_id = v.tenant_id AND a.digest = $4",
        )
        .bind(tenant.as_uuid())
        .bind(claimed.version.id.as_uuid())
        .bind(built.build_key.as_str())
        .bind(artifact.digest.as_str())
        .bind(&artifact.toolchain)
        .bind(now)
        .bind(&built.log)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE script_build_jobs SET status = 'SUCCEEDED', finished_at = $3 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(job.as_uuid())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(Finished::Done)
    }

    async fn fail_build(
        &self,
        hold: Hold<'_>,
        reason: &str,
        log: &str,
        retry: bool,
        audit: &AuditRecord,
    ) -> Result<Finished, StoreError> {
        let Hold {
            tenant,
            job,
            token_hash,
            now,
        } = hold;
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        let Some((claimed, status)) = held_build_in(&mut tx, tenant, job, token_hash, true).await?
        else {
            return Ok(Finished::Stale);
        };
        if status.is_final() {
            return Ok(Finished::AlreadyDone);
        }
        if retry && claimed.job.attempts < MAX_BUILD_ATTEMPTS {
            // Another runner (or this one, later) takes it again.
            sqlx::query(
                "UPDATE script_build_jobs SET status = 'QUEUED', lease_token_hash = NULL, \
                     lease_expires_at = NULL \
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(job.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            tx.commit().await.map_err(store_error)?;
            return Ok(Finished::Done);
        }
        sqlx::query(
            "UPDATE script_versions SET status = 'FAILED', failure = $3, build_log = $4, built_at = $5 \
             WHERE tenant_id = $1 AND id = $2 AND status = 'BUILDING'",
        )
        .bind(tenant.as_uuid())
        .bind(claimed.version.id.as_uuid())
        .bind(reason)
        .bind(log)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE script_build_jobs SET status = 'FAILED', finished_at = $3 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(job.as_uuid())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(Finished::Done)
    }

    async fn claim_test(
        &self,
        tenant: TenantId,
        languages: &[Language],
        token_hash: &[u8],
        lease: Duration,
        now: OffsetDateTime,
    ) -> Result<Option<ClaimedTest>, StoreError> {
        let names: Vec<&str> = languages.iter().map(|l| l.as_str()).collect();
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        // Runs nobody took in time, and runs whose runner vanished too many times, are over.
        sqlx::query(
            "UPDATE script_test_runs SET status = 'FAILED', failure = 'SCRIPT_SANDBOX_UNAVAILABLE', \
                 log = 'the sandbox could not run the test', finished_at = $3, lease_token_hash = NULL \
             WHERE tenant_id = $1 AND finished_at IS NULL \
               AND ((status = 'QUEUED' AND deadline_at < $3) \
                 OR (status = 'RUNNING' AND lease_expires_at < $3 AND attempts >= $2))",
        )
        .bind(tenant.as_uuid())
        .bind(MAX_TEST_ATTEMPTS)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(id) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM script_test_runs \
             WHERE tenant_id = $1 AND language = ANY($2) \
               AND (status = 'QUEUED' OR (status = 'RUNNING' AND lease_expires_at < $3)) \
             ORDER BY created_at, id LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .bind(tenant.as_uuid())
        .bind(&names)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        else {
            tx.commit().await.map_err(store_error)?;
            return Ok(None);
        };
        let row = sqlx::query(
            "UPDATE script_test_runs SET status = 'RUNNING', attempts = attempts + 1, \
                 lease_token_hash = $3, lease_expires_at = $4 + make_interval(secs => $5) \
             WHERE tenant_id = $1 AND id = $2 \
             RETURNING id, tenant_id, namespace_id, script_id, language, status, source, lockfile, \
                       input_schema, output_schema, policy, retention_days, input, result, failure, \
                       log, log_truncated, denied, exit_code, wall_ms, start_ms, memory_peak_bytes, \
                       inferred_input_schema, inferred_output_schema, attempts, created_by, \
                       created_at, finished_at, lease_expires_at",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(token_hash)
        .bind(now)
        .bind(seconds(lease))
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(Some(ClaimedTest {
            run: test_run_of(&row)?,
            lease_expires_at: col(&row, "lease_expires_at")?,
        }))
    }

    async fn heartbeat_test(
        &self,
        tenant: TenantId,
        run: TestRunId,
        token_hash: &[u8],
        lease: Duration,
        now: OffsetDateTime,
    ) -> Result<Option<OffsetDateTime>, StoreError> {
        sqlx::query_scalar(
            "UPDATE script_test_runs SET lease_expires_at = $4 + make_interval(secs => $5) \
             WHERE tenant_id = $1 AND id = $2 AND lease_token_hash = $3 AND status = 'RUNNING' \
             RETURNING lease_expires_at",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(token_hash)
        .bind(now)
        .bind(seconds(lease))
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn finish_test(
        &self,
        tenant: TenantId,
        run: TestRunId,
        token_hash: &[u8],
        report: &TestReport,
        now: OffsetDateTime,
        audits: &[AuditRecord],
    ) -> Result<Finished, StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        let held = sqlx::query(
            "SELECT status FROM script_test_runs \
             WHERE tenant_id = $1 AND id = $2 AND lease_token_hash = $3 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(token_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(held) = held else {
            return Ok(Finished::Stale);
        };
        let status: String = col(&held, "status")?;
        if status != "RUNNING" {
            return Ok(Finished::AlreadyDone);
        }
        let (result, failure) = match &report.result {
            Ok(bytes) => (Some(bytes.as_slice()), None),
            Err(failure) => (None, Some(failure.code())),
        };
        sqlx::query(
            "UPDATE script_test_runs SET status = $3, result = $4, failure = $5, log = $6, \
                 log_truncated = $7, denied = $8, exit_code = $9, wall_ms = $10, start_ms = $11, \
                 memory_peak_bytes = $12, inferred_input_schema = $13, inferred_output_schema = $14, \
                 finished_at = $15 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(if report.result.is_ok() { "SUCCEEDED" } else { "FAILED" })
        .bind(result)
        .bind(failure)
        .bind(&report.log)
        .bind(report.log_truncated)
        .bind(&report.denied)
        .bind(report.exit_code)
        .bind(i64::try_from(report.wall_ms).unwrap_or(i64::MAX))
        .bind(i64::try_from(report.start_ms).unwrap_or(i64::MAX))
        .bind(bound(report.memory_peak_bytes))
        .bind(report.inferred_input_schema.as_deref())
        .bind(report.inferred_output_schema.as_deref())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        for audit in audits {
            insert_audit(&mut tx, audit).await?;
        }
        tx.commit().await.map_err(store_error)?;
        Ok(Finished::Done)
    }

    async fn artifact(
        &self,
        tenant: TenantId,
        digest: &Digest,
    ) -> Result<Option<ArtifactRecord>, StoreError> {
        sqlx::query(
            "SELECT tenant_id, digest, language, content, signature, key_id, sbom, toolchain, \
                    created_at \
             FROM script_artifacts WHERE tenant_id = $1 AND digest = $2",
        )
        .bind(tenant.as_uuid())
        .bind(digest.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .map(|row| artifact_of(&row))
        .transpose()
    }

    async fn pending_activities(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        limit: u32,
    ) -> Result<Vec<String>, StoreError> {
        // `.` follows `-` in ASCII, so the range is exactly the names that start with the prefix.
        sqlx::query_scalar(
            "SELECT DISTINCT activity FROM steps \
             WHERE tenant_id = $1 AND namespace_id = $2 AND status = 'READY' \
               AND activity >= 'script.sha256-' AND activity < 'script.sha256.' \
             ORDER BY activity LIMIT $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn retention_of(
        &self,
        tenant: TenantId,
        digest: &Digest,
    ) -> Result<Option<Retention>, StoreError> {
        let days: Option<i16> = sqlx::query_scalar(
            "SELECT max(retention_days) FROM script_versions \
             WHERE tenant_id = $1 AND artifact_digest = $2 AND status = 'READY'",
        )
        .bind(tenant.as_uuid())
        .bind(digest.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)?;
        days.map(|days| {
            u16::try_from(days)
                .ok()
                .and_then(|days| Retention::days(days).ok())
                .ok_or_else(|| corrupt("script retention"))
        })
        .transpose()
    }

    async fn record_execution(
        &self,
        record: &ExecutionRecord,
        audits: &[AuditRecord],
    ) -> Result<bool, StoreError> {
        let e = &record.execution;
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        let inserted = sqlx::query(
            "INSERT INTO script_executions \
             (id, tenant_id, namespace_id, digest, run_id, node, attempt, status, failure, \
              exit_code, log, log_truncated, denied, wall_ms, start_ms, memory_peak_bytes, \
              trace_id, created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
                     $18, $19) \
             ON CONFLICT (tenant_id, run_id, node, attempt) WHERE run_id IS NOT NULL DO NOTHING",
        )
        .bind(record.id)
        .bind(record.tenant_id.as_uuid())
        .bind(e.namespace_id.as_uuid())
        .bind(e.digest.as_str())
        .bind(e.run_id)
        .bind(e.node.as_deref())
        .bind(e.attempt.map(|a| i32::try_from(a).unwrap_or(i32::MAX)))
        .bind(e.status.as_str())
        .bind(e.failure.map(Failure::code))
        .bind(e.exit_code)
        .bind(&e.log)
        .bind(e.log_truncated)
        .bind(&e.denied)
        .bind(i64::try_from(e.wall_ms).unwrap_or(i64::MAX))
        .bind(i64::try_from(e.start_ms).unwrap_or(i64::MAX))
        .bind(bound(e.memory_peak_bytes))
        .bind(e.trace_id.as_deref())
        .bind(record.created_at)
        .bind(record.expires_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected()
            > 0;
        if inserted {
            for audit in audits {
                insert_audit(&mut tx, audit).await?;
            }
        }
        tx.commit().await.map_err(store_error)?;
        Ok(inserted)
    }
}
