//! The blob catalogue against a real PostgreSQL (FR-002-008, ADR-0023): states that only move
//! forward, a per-tenant budget that racing uploads cannot overshoot, tenant isolation, and a sweep
//! that cannot remove what is being used.

use std::sync::Arc;
use std::time::Duration;

use sagat_application::blobs::{
    BlobAccess, BlobCatalog, BlobOwner, BlobScope, DEFAULT_DELETE_RETRY,
};
use sagat_application::error::StoreError;
use sagat_domain::blobs::{BlobId, BlobState, ContentType};
use sagat_domain::workflow::Digest;
use sagat_infra_postgres::PgBlobCatalog;
use serde_json::json;
use sqlx::Row;

use crate::support::engine::{activity, document, world};
use crate::support::{TestDb, db_error};

struct Fixture {
    db: TestDb,
    catalog: Arc<PgBlobCatalog>,
    scope: BlobScope,
    other_scope: BlobScope,
}

async fn fixture() -> Fixture {
    let platform = world().await;
    let (workflow, _) = platform
        .publish(document("blob-first", "store", vec![activity("store")]))
        .await;
    let run = platform.start(workflow, &json!({})).await;
    let other_workflow = platform
        .publish_as_other_tenant(document("blob-second", "store", vec![activity("store")]))
        .await;
    let other_run = platform
        .start_as_other_tenant(other_workflow, &json!({}))
        .await
        .expect("start other run");

    async fn scope_for(
        pool: &sqlx::PgPool,
        access: BlobAccess,
        run_id: sagat_domain::execution::RunId,
    ) -> BlobScope {
        let row = sqlx::query(
            "SELECT step_id, attempt, generation FROM steps WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(access.tenant_id.as_uuid())
        .bind(run_id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("step scope");
        BlobScope {
            access,
            run_id,
            step_id: row.get("step_id"),
            attempt: u32::try_from(row.get::<i32, _>("attempt")).expect("attempt"),
            generation: u64::try_from(row.get::<i64, _>("generation")).expect("generation"),
        }
    }

    let access = BlobAccess {
        tenant_id: platform.admin.tenant_id(),
        namespace_id: platform.namespace.id(),
    };
    let other_access = BlobAccess {
        tenant_id: platform.other_admin.tenant_id(),
        namespace_id: platform.other_namespace.id(),
    };
    let scope = scope_for(&platform.db.pool, access, run).await;
    let other_scope = scope_for(&platform.db.pool, other_access, other_run).await;
    let db = platform.db;
    Fixture {
        catalog: Arc::new(PgBlobCatalog::new(db.pool.clone())),
        scope,
        other_scope,
        db,
    }
}

fn json() -> ContentType {
    ContentType::parse("application/json").expect("valid")
}

fn digest(byte: u8) -> Digest {
    Digest::new([byte; 32])
}

async fn reserve(f: &Fixture, scope: BlobScope, max: u32) -> Result<BlobId, StoreError> {
    let id = BlobId::generate();
    f.catalog
        .reserve(BlobOwner::StepOutput(scope), id, &json(), "wendy", max)
        .await
        .map(|()| id)
}

async fn state_of(f: &Fixture, access: BlobAccess, id: BlobId) -> Option<BlobState> {
    f.catalog
        .find(access, id)
        .await
        .expect("find")
        .map(|record| record.state)
}

#[tokio::test]
async fn a_blob_is_reserved_then_confirmed_with_its_size_and_digest() {
    let f = fixture().await;
    let id = reserve(&f, f.scope, 10).await.expect("reserved");

    let reserved = f
        .catalog
        .find(f.scope.access, id)
        .await
        .expect("find")
        .expect("there");
    assert_eq!(reserved.state, BlobState::Uploading);
    assert_eq!(
        reserved.descriptor.content_type().as_str(),
        "application/json"
    );

    f.catalog
        .mark_uploaded(f.scope.access, id, 1_234_567, &digest(9))
        .await
        .expect("confirmed");

    let uploaded = f
        .catalog
        .find(f.scope.access, id)
        .await
        .expect("find")
        .expect("there");
    assert_eq!(uploaded.state, BlobState::Uploaded);
    assert_eq!(uploaded.descriptor.size(), 1_234_567);
    assert_eq!(uploaded.descriptor.digest(), &digest(9));
    let audit: (String, String, String) = sqlx::query_as(
        "SELECT actor, action, resource_id FROM audit_records \
         WHERE tenant_id = $1 AND action = 'blob.uploaded'",
    )
    .bind(f.scope.access.tenant_id.as_uuid())
    .fetch_one(&f.db.pool)
    .await
    .expect("upload audit");
    assert_eq!(audit.0, "wendy");
    assert_eq!(audit.1, "blob.uploaded");
    assert_eq!(audit.2, format!("{}/{}", f.scope.access.namespace_id, id));
}

#[tokio::test]
async fn an_upload_state_change_rolls_back_when_its_audit_cannot_be_appended() {
    let f = fixture().await;
    let id = reserve(&f, f.scope, 10).await.expect("reserved");
    sqlx::query(
        "CREATE FUNCTION reject_blob_upload_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.action = 'blob.uploaded' THEN RAISE EXCEPTION 'test audit failure'; \
         END IF; RETURN NEW; END $$",
    )
    .execute(&f.db.pool)
    .await
    .expect("failure function");
    sqlx::query(
        "CREATE TRIGGER reject_blob_upload_audit BEFORE INSERT ON audit_records \
         FOR EACH ROW EXECUTE FUNCTION reject_blob_upload_audit()",
    )
    .execute(&f.db.pool)
    .await
    .expect("failure trigger");

    assert!(
        f.catalog
            .mark_uploaded(f.scope.access, id, 5, &digest(1))
            .await
            .is_err()
    );
    assert_eq!(
        state_of(&f, f.scope.access, id).await,
        Some(BlobState::Uploading),
        "the catalog update and audit append are one transaction"
    );
}

#[tokio::test]
async fn confirming_twice_or_something_never_reserved_is_refused() {
    let f = fixture().await;
    let id = reserve(&f, f.scope, 10).await.expect("reserved");
    f.catalog
        .mark_uploaded(f.scope.access, id, 10, &digest(1))
        .await
        .expect("first");

    let again = f
        .catalog
        .mark_uploaded(f.scope.access, id, 10, &digest(1))
        .await;
    let unknown = f
        .catalog
        .mark_uploaded(f.scope.access, BlobId::generate(), 10, &digest(1))
        .await;

    assert_eq!(again, Err(StoreError::MissingReference));
    assert_eq!(unknown, Err(StoreError::MissingReference));
}

#[tokio::test]
async fn another_tenant_sees_nothing_and_changes_nothing() {
    let f = fixture().await;
    let id = reserve(&f, f.scope, 10).await.expect("reserved");

    assert_eq!(state_of(&f, f.other_scope.access, id).await, None);
    assert_eq!(
        f.catalog
            .mark_uploaded(f.other_scope.access, id, 1, &digest(1))
            .await,
        Err(StoreError::MissingReference)
    );
    assert!(
        !f.catalog
            .claim_delete(f.other_scope.access, id, DEFAULT_DELETE_RETRY)
            .await
            .expect("a no-op")
    );

    assert_eq!(
        state_of(&f, f.scope.access, id).await,
        Some(BlobState::Uploading)
    );
}

#[tokio::test]
async fn a_tenant_may_hold_only_so_many_blobs_nothing_uses_yet() {
    let f = fixture().await;
    for _ in 0..3 {
        reserve(&f, f.scope, 3).await.expect("within the budget");
    }

    let fourth = reserve(&f, f.scope, 3).await;
    let elsewhere = reserve(&f, f.other_scope, 3).await;

    assert_eq!(
        fourth,
        Err(StoreError::QuotaExceeded {
            what: "uploaded blobs not yet used"
        })
    );
    assert!(elsewhere.is_ok(), "the budget is per tenant");
}

#[tokio::test]
async fn racing_uploads_cannot_overshoot_the_budget() {
    let f = fixture().await;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let catalog = Arc::clone(&f.catalog);
        let scope = f.scope;
        tasks.push(tokio::spawn(async move {
            catalog
                .reserve(
                    BlobOwner::StepOutput(scope),
                    BlobId::generate(),
                    &json(),
                    "wendy",
                    5,
                )
                .await
        }));
    }
    let mut accepted = 0;
    for task in tasks {
        match task.await.expect("joined") {
            Ok(()) => accepted += 1,
            Err(StoreError::QuotaExceeded { .. }) => {}
            Err(other) => panic!("unexpected: {other:?}"),
        }
    }

    assert_eq!(accepted, 5, "exactly the budget, never more");
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM blobs")
        .fetch_one(&f.db.pool)
        .await
        .expect("count");
    assert_eq!(held, 5);
}

#[tokio::test]
async fn a_used_blob_stops_counting_against_the_budget() {
    let f = fixture().await;
    let mut ids = Vec::new();
    for _ in 0..2 {
        ids.push(reserve(&f, f.scope, 2).await.expect("reserved"));
    }
    assert!(reserve(&f, f.scope, 2).await.is_err(), "full");

    // The record that uses a blob commits it (what a completed step does).
    sqlx::query(
        "UPDATE blobs SET state = 'UPLOADED', size_bytes = 1, sha256 = $2, uploaded_at = now() \
         WHERE id = $1",
    )
    .bind(ids[0].as_uuid())
    .bind(vec![1u8; 32])
    .execute(&f.db.pool)
    .await
    .expect("uploaded");
    sqlx::query("UPDATE blobs SET state = 'COMMITTED', committed_at = now() WHERE id = $1")
        .bind(ids[0].as_uuid())
        .execute(&f.db.pool)
        .await
        .expect("committed");

    assert!(
        reserve(&f, f.scope, 2).await.is_ok(),
        "one place is free again"
    );
}

#[tokio::test]
async fn cleanup_claims_an_unused_blob_but_never_a_used_one() {
    let f = fixture().await;
    let unused = reserve(&f, f.scope, 10).await.expect("reserved");
    let used = reserve(&f, f.scope, 10).await.expect("reserved");
    sqlx::query(
        "UPDATE blobs SET state = 'COMMITTED', size_bytes = 1, sha256 = $2, \
         uploaded_at = now(), committed_at = now() WHERE id = $1",
    )
    .bind(used.as_uuid())
    .bind(vec![1u8; 32])
    .execute(&f.db.pool)
    .await
    .expect("committed");

    assert!(
        f.catalog
            .claim_delete(f.scope.access, unused, DEFAULT_DELETE_RETRY)
            .await
            .expect("claim")
    );
    assert!(
        !f.catalog
            .claim_delete(f.scope.access, used, DEFAULT_DELETE_RETRY)
            .await
            .expect("committed is not eligible")
    );
    assert_eq!(
        state_of(&f, f.scope.access, unused).await,
        Some(BlobState::Deleting),
        "the row remains until external deletion is confirmed"
    );
    f.catalog
        .finish_delete(f.scope.access, unused)
        .await
        .expect("confirm");
    f.catalog
        .finish_delete(f.scope.access, unused)
        .await
        .expect("confirmation is idempotent");

    assert_eq!(
        state_of(&f, f.scope.access, unused).await,
        Some(BlobState::Deleting)
    );
    assert_eq!(
        state_of(&f, f.scope.access, used).await,
        Some(BlobState::Committed)
    );
    let deleted: (String, String) = sqlx::query_as(
        "SELECT actor, resource_id FROM audit_records \
         WHERE tenant_id = $1 AND action = 'blob.deleted'",
    )
    .bind(f.scope.access.tenant_id.as_uuid())
    .fetch_one(&f.db.pool)
    .await
    .expect("deletion audit");
    assert_eq!(deleted.0, "system:engine");
    assert_eq!(
        deleted.1,
        format!("{}/{}", f.scope.access.namespace_id, unused)
    );
}

async fn age(f: &Fixture, id: BlobId, seconds: i64) {
    sqlx::query(
        "UPDATE blobs SET created_at = created_at - $2 * interval '1 second', \
             uploaded_at = uploaded_at - $2 * interval '1 second' WHERE id = $1",
    )
    .bind(id.as_uuid())
    .bind(seconds)
    .execute(&f.db.pool)
    .await
    .expect("aged");
}

const DAY: i64 = 24 * 3600;
/// How long an abandoned upload is kept, and how long an uploaded blob nobody used.
const HOUR: Duration = Duration::from_secs(3600);
const DAY_TTL: Duration = Duration::from_secs(24 * 3600);

#[tokio::test]
async fn the_sweep_claims_only_old_unused_blobs_and_retains_rows_until_confirmation() {
    let f = fixture().await;
    let old_unused = reserve(&f, f.scope, 10).await.expect("reserved");
    let old_other_tenant = reserve(&f, f.other_scope, 10).await.expect("reserved");
    let fresh = reserve(&f, f.scope, 10).await.expect("reserved");
    let old_used = reserve(&f, f.scope, 10).await.expect("reserved");
    sqlx::query(
        "UPDATE blobs SET state = 'COMMITTED', size_bytes = 1, sha256 = $2, \
         uploaded_at = now(), committed_at = now() WHERE id = $1",
    )
    .bind(old_used.as_uuid())
    .bind(vec![1u8; 32])
    .execute(&f.db.pool)
    .await
    .expect("committed");
    for id in [old_unused, old_other_tenant, old_used] {
        age(&f, id, 3 * DAY).await;
    }

    let mut claimed = f
        .catalog
        .claim_expired(HOUR, DAY_TTL, DEFAULT_DELETE_RETRY, 100)
        .await
        .expect("claimed");
    claimed.sort();

    let mut expected = vec![
        (f.scope.access, old_unused),
        (f.other_scope.access, old_other_tenant),
    ];
    expected.sort();
    assert_eq!(claimed, expected);
    assert_eq!(
        state_of(&f, f.scope.access, old_unused).await,
        Some(BlobState::Deleting)
    );
    assert_eq!(
        state_of(&f, f.scope.access, fresh).await,
        Some(BlobState::Uploading)
    );
    assert_eq!(
        state_of(&f, f.scope.access, old_used).await,
        Some(BlobState::Committed),
        "what a record uses is never swept, however old"
    );
    assert!(
        f.catalog
            .claim_expired(HOUR, DAY_TTL, DEFAULT_DELETE_RETRY, 100)
            .await
            .expect("again")
            .is_empty(),
        "a claimed blob is claimed once"
    );
    for (access, id) in claimed {
        f.catalog
            .finish_delete(access, id)
            .await
            .expect("backend confirmed deletion");
    }
    assert_eq!(
        state_of(&f, f.scope.access, old_unused).await,
        Some(BlobState::Deleting)
    );
}

#[tokio::test]
async fn the_sweep_respects_its_batch_size() {
    let f = fixture().await;
    for _ in 0..5 {
        let id = reserve(&f, f.scope, 50).await.expect("reserved");
        age(&f, id, 2 * DAY).await;
    }

    let first = f
        .catalog
        .claim_expired(HOUR, HOUR, DEFAULT_DELETE_RETRY, 3)
        .await
        .expect("first");
    let second = f
        .catalog
        .claim_expired(HOUR, HOUR, DEFAULT_DELETE_RETRY, 3)
        .await
        .expect("second");

    assert_eq!((first.len(), second.len()), (3, 2));
}

#[tokio::test]
async fn a_failed_external_delete_has_durable_attempt_and_retry_evidence() {
    let f = fixture().await;
    let id = reserve(&f, f.scope, 10).await.expect("reserved");
    age(&f, id, 2 * DAY).await;

    assert_eq!(
        f.catalog
            .claim_expired(HOUR, DAY_TTL, DEFAULT_DELETE_RETRY, 10)
            .await
            .expect("first claim"),
        vec![(f.scope.access, id)]
    );
    let evidence: (String, i32, bool, bool) = sqlx::query_as(
        "SELECT state, deletion_attempts, deletion_last_attempt_at IS NOT NULL, \
                delete_after > clock_timestamp() \
         FROM blobs WHERE tenant_id = $1 AND id = $2",
    )
    .bind(f.scope.access.tenant_id.as_uuid())
    .bind(id.as_uuid())
    .fetch_one(&f.db.pool)
    .await
    .expect("evidence");
    assert_eq!(evidence, ("DELETING".into(), 1, true, true));
    assert!(
        f.catalog
            .claim_expired(HOUR, DAY_TTL, DEFAULT_DELETE_RETRY, 10)
            .await
            .expect("not due")
            .is_empty(),
        "a provider outage does not cause a hot retry loop"
    );

    sqlx::query(
        "UPDATE blobs SET delete_after = clock_timestamp() - interval '1 second' \
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(f.scope.access.tenant_id.as_uuid())
    .bind(id.as_uuid())
    .execute(&f.db.pool)
    .await
    .expect("retry due");
    assert_eq!(
        f.catalog
            .claim_expired(HOUR, DAY_TTL, DEFAULT_DELETE_RETRY, 10)
            .await
            .expect("retry claim"),
        vec![(f.scope.access, id)]
    );
    let attempts: i32 =
        sqlx::query_scalar("SELECT deletion_attempts FROM blobs WHERE tenant_id = $1 AND id = $2")
            .bind(f.scope.access.tenant_id.as_uuid())
            .bind(id.as_uuid())
            .fetch_one(&f.db.pool)
            .await
            .expect("attempts");
    assert_eq!(attempts, 2);

    f.catalog
        .finish_delete(f.scope.access, id)
        .await
        .expect("backend confirmed deletion");
    f.catalog
        .finish_delete(f.scope.access, id)
        .await
        .expect("lost response retry is idempotent");
    assert_eq!(
        state_of(&f, f.scope.access, id).await,
        Some(BlobState::Deleting)
    );
}

#[tokio::test]
async fn a_blob_being_committed_is_skipped_by_the_sweep_and_never_lost() {
    let f = fixture().await;
    let id = reserve(&f, f.scope, 10).await.expect("reserved");
    f.catalog
        .mark_uploaded(f.scope.access, id, 5, &digest(4))
        .await
        .expect("uploaded");
    age(&f, id, 3 * DAY).await;

    // The transaction that uses the blob has locked its row and has not finished.
    let mut using = f.db.pool.begin().await.expect("begin");
    sqlx::query(
        "UPDATE blobs SET state = 'COMMITTED', committed_at = clock_timestamp() \
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(f.scope.access.tenant_id.as_uuid())
    .bind(id.as_uuid())
    .execute(&mut *using)
    .await
    .expect("locked");

    let swept = f
        .catalog
        .claim_expired(HOUR, DAY_TTL, DEFAULT_DELETE_RETRY, 100)
        .await
        .expect("sweep does not wait");
    using.commit().await.expect("commit");

    assert!(
        swept.is_empty(),
        "skipped, not removed under the user's feet"
    );
    assert_eq!(
        state_of(&f, f.scope.access, id).await,
        Some(BlobState::Committed)
    );
}

#[tokio::test]
async fn the_schema_refuses_states_that_cannot_exist() {
    let f = fixture().await;
    // Each update starts from a valid reservation and breaks exactly one rule, so the named
    // constraint is the invariant that rejected it.
    let attempts = [
        // Measured while still uploading.
        (
            "UPDATE blobs SET size_bytes = 5 WHERE id = $1",
            "blobs_measurement_consistent",
        ),
        // Uploaded without confirmed measurements.
        (
            "UPDATE blobs SET state = 'UPLOADED' WHERE id = $1",
            "blobs_measured_when_usable",
        ),
        // Committed without a time.
        (
            "UPDATE blobs SET state = 'COMMITTED', size_bytes = 5, \
                 sha256 = decode(repeat('ab', 32), 'hex'), uploaded_at = now() WHERE id = $1",
            "blobs_committed_has_a_time",
        ),
        // A state this version does not have.
        (
            "UPDATE blobs SET state = 'DELETED' WHERE id = $1",
            "blobs_state_valid",
        ),
        // A deleting state without a numbered attempt and retry time would lose cleanup work.
        (
            "UPDATE blobs SET state = 'DELETING' WHERE id = $1",
            "blobs_deletion_evidence",
        ),
        (
            "UPDATE blobs SET deletion_confirmed_at = now() WHERE id = $1",
            "blobs_deletion_confirmation_valid",
        ),
        // A digest that is not 32 bytes.
        (
            "UPDATE blobs SET state = 'UPLOADED', size_bytes = 5, sha256 = decode('abab', 'hex'), \
                 uploaded_at = now() WHERE id = $1",
            "blobs_sha256_length",
        ),
        // A content type that is not one.
        (
            "UPDATE blobs SET content_type = 'x' WHERE id = $1",
            "blobs_content_type_length",
        ),
    ];
    for (sql, constraint) in attempts {
        let id = reserve(&f, f.scope, 50).await.expect("valid reservation");
        let error = sqlx::query(sql)
            .bind(id.as_uuid())
            .execute(&f.db.pool)
            .await
            .expect_err(sql);
        let (code, named) = db_error(error);
        assert_eq!(code, "23514", "{sql}");
        assert_eq!(named.as_deref(), Some(constraint), "{sql}");
    }

    // And a blob cannot belong to a tenant that does not exist.
    let error = sqlx::query(
        "INSERT INTO blobs (tenant_id, namespace_id, id, state, content_type, created_by, \
                            run_id, step_id, attempt, generation) \
         VALUES ($1, $2, $3, 'UPLOADING', 'a/b', 'x', $4, $5, $6, $7)",
    )
    .bind(sagat_domain::identity::TenantId::generate().as_uuid())
    .bind(f.scope.access.namespace_id.as_uuid())
    .bind(BlobId::generate().as_uuid())
    .bind(f.scope.run_id.as_uuid())
    .bind(f.scope.step_id)
    .bind(i32::try_from(f.scope.attempt).expect("attempt"))
    .bind(i64::try_from(f.scope.generation).expect("generation"))
    .execute(&f.db.pool)
    .await
    .expect_err("no such tenant");
    assert_eq!(db_error(error).0, "23503");
}

#[tokio::test]
async fn an_unfinished_upload_expires_sooner_than_an_uploaded_blob() {
    let f = fixture().await;
    let unfinished = reserve(&f, f.scope, 10).await.expect("reserved");
    let uploaded = reserve(&f, f.scope, 10).await.expect("reserved");
    f.catalog
        .mark_uploaded(f.scope.access, uploaded, 5, &digest(2))
        .await
        .expect("uploaded");
    for id in [unfinished, uploaded] {
        age(&f, id, 3 * 3600).await;
    }

    let claimed = f
        .catalog
        .claim_expired(HOUR, DAY_TTL, DEFAULT_DELETE_RETRY, 100)
        .await
        .expect("claimed");

    assert_eq!(
        claimed,
        vec![(f.scope.access, unfinished)],
        "three hours is long for an upload, not for a blob"
    );
    assert_eq!(
        state_of(&f, f.scope.access, uploaded).await,
        Some(BlobState::Uploaded)
    );
}

#[tokio::test]
async fn confirmed_cleanup_of_an_unknown_upload_retains_a_retryable_tombstone() {
    let f = fixture().await;
    let id = reserve(&f, f.scope, 1).await.expect("reserved");
    f.catalog
        .claim_delete(f.scope.access, id, DEFAULT_DELETE_RETRY)
        .await
        .expect("claim");
    f.catalog
        .finish_delete(f.scope.access, id)
        .await
        .expect("first delete confirmed");
    f.catalog
        .finish_delete(f.scope.access, id)
        .await
        .expect("repeat confirmation");
    assert_eq!(
        state_of(&f, f.scope.access, id).await,
        Some(BlobState::Deleting)
    );
    assert!(
        matches!(
            f.catalog
                .mark_uploaded(f.scope.access, id, 12, &digest(1))
                .await,
            Err(StoreError::MissingReference)
        ),
        "late completion cannot revive the upload"
    );
    assert!(
        reserve(&f, f.scope, 1).await.is_ok(),
        "confirmed tombstone releases the active upload slot"
    );
    let first_audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = 'blob.deleted'")
            .fetch_one(&f.db.pool)
            .await
            .expect("audit count");
    assert_eq!(first_audits, 1);
    sqlx::query(
        "UPDATE blobs SET delete_after = clock_timestamp() - interval '1 second' WHERE id = $1",
    )
    .bind(id.as_uuid())
    .execute(&f.db.pool)
    .await
    .expect("retry due");
    assert_eq!(
        f.catalog
            .claim_expired(HOUR, DAY_TTL, DEFAULT_DELETE_RETRY, 10)
            .await
            .expect("retry"),
        vec![(f.scope.access, id)]
    );
    f.catalog
        .finish_delete(f.scope.access, id)
        .await
        .expect("late bytes deleted again");
    let repeated_audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = 'blob.deleted'")
            .fetch_one(&f.db.pool)
            .await
            .expect("audit count");
    assert_eq!(
        repeated_audits, 1,
        "reconciliation does not grow an identical audit trail"
    );
}

#[tokio::test]
async fn uploaded_retention_starts_when_the_upload_finishes() {
    let f = fixture().await;
    let id = reserve(&f, f.scope, 1).await.expect("reserved");
    age(&f, id, 2 * DAY).await;
    f.catalog
        .mark_uploaded(f.scope.access, id, 5, &digest(2))
        .await
        .expect("just uploaded");
    assert!(
        f.catalog
            .claim_expired(HOUR, DAY_TTL, DEFAULT_DELETE_RETRY, 10)
            .await
            .expect("sweep")
            .is_empty(),
        "a slow upload still receives its whole retention interval"
    );
    sqlx::query("UPDATE blobs SET uploaded_at = uploaded_at - interval '2 days' WHERE id = $1")
        .bind(id.as_uuid())
        .execute(&f.db.pool)
        .await
        .expect("retention elapsed");
    assert_eq!(
        f.catalog
            .claim_expired(HOUR, DAY_TTL, DEFAULT_DELETE_RETRY, 10)
            .await
            .expect("expired"),
        vec![(f.scope.access, id)]
    );
    f.catalog
        .finish_delete(f.scope.access, id)
        .await
        .expect("confirmed finalized upload deletion");
    assert_eq!(
        state_of(&f, f.scope.access, id).await,
        None,
        "finalized uploads have no late writer and need no tombstone"
    );
}

#[tokio::test]
async fn failed_cleanup_audit_keeps_the_tombstone_unconfirmed_and_the_slot_reserved() {
    let f = fixture().await;
    let id = reserve(&f, f.scope, 1).await.expect("reserved");
    f.catalog
        .claim_delete(f.scope.access, id, DEFAULT_DELETE_RETRY)
        .await
        .expect("claim");
    sqlx::query(
        "CREATE FUNCTION reject_blob_delete_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
        BEGIN IF NEW.action = 'blob.deleted' THEN RAISE EXCEPTION 'test audit failure'; \
        END IF; RETURN NEW; END $$",
    )
    .execute(&f.db.pool)
    .await
    .expect("failure function");
    sqlx::query(
        "CREATE TRIGGER reject_blob_delete_audit BEFORE INSERT ON audit_records \
        FOR EACH ROW EXECUTE FUNCTION reject_blob_delete_audit()",
    )
    .execute(&f.db.pool)
    .await
    .expect("failure trigger");
    assert!(f.catalog.finish_delete(f.scope.access, id).await.is_err());
    let confirmed: bool =
        sqlx::query_scalar("SELECT deletion_confirmed_at IS NOT NULL FROM blobs WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_one(&f.db.pool)
            .await
            .expect("still tracked");
    assert!(
        !confirmed,
        "audit and cleanup confirmation roll back together"
    );
    assert!(matches!(
        reserve(&f, f.scope, 1).await,
        Err(StoreError::QuotaExceeded { .. })
    ));
    sqlx::query("DROP TRIGGER reject_blob_delete_audit ON audit_records")
        .execute(&f.db.pool)
        .await
        .expect("remove failure");
    f.catalog
        .finish_delete(f.scope.access, id)
        .await
        .expect("retry confirmation");
    assert!(reserve(&f, f.scope, 1).await.is_ok());
}
