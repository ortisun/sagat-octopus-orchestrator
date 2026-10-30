//! The schedule store against a real PostgreSQL (`T018-014`): creating and reading schedules with
//! their revisions, the namespace quota, names, content accounting, tenant isolation and the audit.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::{ConflictKind, StoreError};
use sagat_application::schedule_codec::{policies_from_json, spec_from_json, target_from_json};
use sagat_application::schedules::{
    NewSchedule, ScheduleOrigin, ScheduleRecord, ScheduleState, ScheduleStore, StoredInput,
};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowId;
use sagat_domain::identity::{NamespaceId, Principal, Role, Subject, TenantId};
use sagat_domain::schedules::{Ceilings, ScheduleId, ScheduleName};
use sagat_infra_postgres::PgScheduleStore;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::support::{TestDb, migrated_db};

/// A tenant with a workspace and a namespace, seeded by the owner.
struct World {
    db: TestDb,
    tenant: TenantId,
    namespace: NamespaceId,
}

async fn world() -> World {
    let db = migrated_db().await;
    let (tenant, workspace, namespace) = (
        TenantId::generate(),
        Uuid::now_v7(),
        NamespaceId::generate(),
    );
    let short = &tenant.as_uuid().simple().to_string()[..12];
    seed_scope(&db.pool, tenant, workspace, namespace, short).await;
    World {
        db,
        tenant,
        namespace,
    }
}

async fn seed_scope(
    pool: &PgPool,
    tenant: TenantId,
    workspace: Uuid,
    namespace: NamespaceId,
    short: &str,
) {
    sqlx::query("INSERT INTO tenants (id, slug, created_at) VALUES ($1, $2, now())")
        .bind(tenant.as_uuid())
        .bind(format!("tn-{short}"))
        .execute(pool)
        .await
        .expect("tenant");
    sqlx::query(
        "INSERT INTO workspaces (id, tenant_id, slug, created_at) VALUES ($1, $2, 'main', now())",
    )
    .bind(workspace)
    .bind(tenant.as_uuid())
    .execute(pool)
    .await
    .expect("workspace");
    sqlx::query(
        "INSERT INTO namespaces (id, tenant_id, workspace_id, slug, environment, created_at) \
         VALUES ($1, $2, $3, 'prod', 'prod', now())",
    )
    .bind(namespace.as_uuid())
    .bind(tenant.as_uuid())
    .bind(workspace)
    .execute(pool)
    .await
    .expect("namespace");
}

fn principal(world: &World) -> Principal {
    Principal::new(
        world.tenant,
        Subject::parse("alice").expect("subject"),
        Role::Developer,
    )
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn record(world: &World, name: &str) -> ScheduleRecord {
    let ceilings = Ceilings::default();
    let workflow = WorkflowId::generate();
    let now = OffsetDateTime::now_utc();
    ScheduleRecord {
        id: ScheduleId::generate(),
        tenant_id: world.tenant,
        namespace_id: world.namespace,
        name: ScheduleName::new(name).expect("name"),
        origin: ScheduleOrigin::Api,
        trigger_id: None,
        state: ScheduleState::Active,
        revision: 1,
        state_version: 1,
        spec: spec_from_json(
            &json!({"cron": ["0 9 * * 1-5"], "timezone": "Europe/Lisbon", "jitterMs": 1000}),
            &ceilings,
        )
        .expect("spec"),
        policies: policies_from_json(
            &json!({"overlap": "BUFFER_ONE", "pauseOnFailure": true}),
            &ceilings,
        )
        .expect("policies"),
        target: target_from_json(&json!({
            "workflowId": workflow.to_string(),
            "nominalTimeField": "scheduledFor",
            "businessIdPrefix": "report"
        }))
        .expect("target"),
        input_digest: None,
        note: None,
        note_by: None,
        note_at: None,
        authorized_by: "alice".to_owned(),
        remaining_actions: Some(10),
        next_action_at: Some(now),
        next_actual_at: Some(now),
        retry_at: None,
        recheck_at: None,
        tracked_runs: vec![],
        buffered: 0,
        created_at: now,
        updated_at: now,
    }
}

fn audit(world: &World, schedule: &ScheduleRecord) -> AuditRecord {
    AuditRecord::by(
        &principal(world),
        AuditAction::ScheduleCreated,
        ResourceType::Schedule,
        schedule.id.to_string(),
        None,
        schedule.created_at,
    )
}

fn new(record: ScheduleRecord) -> NewSchedule {
    NewSchedule {
        record,
        input: None,
        request_id: Some("req-1".to_owned()),
    }
}

async fn create(
    world: &World,
    store: &PgScheduleStore,
    name: &str,
    limit: u64,
) -> Result<ScheduleRecord, StoreError> {
    let record = record(world, name);
    store
        .create_schedule(&new(record.clone()), limit, &audit(world, &record))
        .await?;
    Ok(record)
}

#[tokio::test]
async fn a_schedule_is_read_back_exactly_as_it_was_stored() {
    let w = world().await;
    let store = PgScheduleStore::new(w.db.pool.clone());
    let created = create(&w, &store, "weekday-report", 10)
        .await
        .expect("create");
    let read = store
        .schedule(w.tenant, created.id)
        .await
        .expect("read")
        .expect("the schedule exists");
    // timestamps lose sub-microsecond precision in the database
    let trim = |t: OffsetDateTime| {
        t.replace_nanosecond(t.nanosecond() / 1000 * 1000)
            .expect("trim")
    };
    let mut expected = created.clone();
    expected.created_at = trim(expected.created_at);
    expected.updated_at = trim(expected.updated_at);
    expected.next_action_at = expected.next_action_at.map(trim);
    expected.next_actual_at = expected.next_actual_at.map(trim);
    assert_eq!(read, expected);
    assert_eq!(read.spec.timezone(), "Europe/Lisbon");
    assert_eq!(
        read.target.business_id().expect("business id").prefix(),
        "report"
    );
}

#[tokio::test]
async fn the_first_revision_and_the_audit_record_commit_with_the_schedule() {
    let w = world().await;
    let store = PgScheduleStore::new(w.db.pool.clone());
    let created = create(&w, &store, "audited", 10).await.expect("create");
    let (revision, actor, request): (i32, String, Option<String>) = sqlx::query_as(
        "SELECT revision, actor, request_id FROM schedule_revisions WHERE tenant_id = $1 AND schedule_id = $2",
    )
    .bind(w.tenant.as_uuid())
    .bind(created.id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("revision");
    assert_eq!(
        (revision, actor.as_str(), request.as_deref()),
        (1, "alice", Some("req-1"))
    );
    let audits: Vec<(String, String)> = sqlx::query_as(
        "SELECT action, resource_type FROM audit_records WHERE tenant_id = $1 AND resource_id = $2",
    )
    .bind(w.tenant.as_uuid())
    .bind(created.id.to_string())
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        audits,
        vec![("schedule.created".to_owned(), "schedule".to_owned())]
    );
}

#[tokio::test]
async fn a_name_is_taken_once_per_namespace_and_free_after_the_schedule_is_deleted() {
    let w = world().await;
    let store = PgScheduleStore::new(w.db.pool.clone());
    let first = create(&w, &store, "nightly", 10).await.expect("create");
    let again = create(&w, &store, "nightly", 10)
        .await
        .expect_err("the name is taken");
    assert!(
        matches!(again, StoreError::Conflict(ConflictKind::ScheduleName)),
        "{again:?}"
    );
    // nothing of the refused attempt was left behind
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM schedules WHERE tenant_id = $1")
        .bind(w.tenant.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = 'schedule.created'",
    )
    .bind(w.tenant.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("audits");
    assert_eq!(audits, 1, "a refused creation leaves no audit record");

    sqlx::query("UPDATE schedules SET state = 'DELETED', deleted_at = now() WHERE tenant_id = $1 AND id = $2")
        .bind(w.tenant.as_uuid())
        .bind(first.id.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("delete");
    create(&w, &store, "nightly", 10)
        .await
        .expect("the name is free again");
}

#[tokio::test]
async fn the_namespace_quota_counts_active_and_paused_schedules_and_not_deleted_ones() {
    let w = world().await;
    let store = PgScheduleStore::new(w.db.pool.clone());
    let first = create(&w, &store, "one", 2).await.expect("one");
    let second = create(&w, &store, "two", 2).await.expect("two");
    sqlx::query("UPDATE schedules SET state = 'PAUSED' WHERE tenant_id = $1 AND id = $2")
        .bind(w.tenant.as_uuid())
        .bind(second.id.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("pause");
    let full = create(&w, &store, "three", 2)
        .await
        .expect_err("the namespace is full");
    assert!(matches!(full, StoreError::QuotaExceeded { .. }), "{full:?}");

    sqlx::query("UPDATE schedules SET state = 'DELETED', deleted_at = now() WHERE tenant_id = $1 AND id = $2")
        .bind(w.tenant.as_uuid())
        .bind(first.id.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("delete");
    create(&w, &store, "three", 2)
        .await
        .expect("a deleted schedule gives its place back");
}

#[tokio::test]
async fn the_quota_holds_when_many_creations_race_for_the_last_place() {
    let w = world().await;
    let store = PgScheduleStore::new(w.db.pool.clone());
    let mut tasks = Vec::new();
    for n in 0..6 {
        let store = store.clone();
        let rec = record(&w, &format!("racer-{n}"));
        let audit = audit(&w, &rec);
        tasks.push(tokio::spawn(async move {
            store.create_schedule(&new(rec), 3, &audit).await
        }));
    }
    let mut created = 0;
    let mut refused = 0;
    for task in tasks {
        match task.await.expect("task") {
            Ok(()) => created += 1,
            Err(StoreError::QuotaExceeded { .. }) => refused += 1,
            Err(other) => panic!("unexpected: {other:?}"),
        }
    }
    assert_eq!(
        (created, refused),
        (3, 3),
        "the fence lets in exactly the limit"
    );
}

#[tokio::test]
async fn the_name_and_the_note_of_a_schedule_are_charged_to_the_tenant_content() {
    let w = world().await;
    let store = PgScheduleStore::new(w.db.pool.clone());
    let before: i64 = sqlx::query_scalar(
        "SELECT coalesce(content_bytes, 0)::bigint FROM tenant_retained_usage WHERE tenant_id = $1",
    )
    .bind(w.tenant.as_uuid())
    .fetch_optional(&w.db.pool)
    .await
    .expect("usage")
    .unwrap_or(0);
    let mut rec = record(&w, "charged-name");
    rec.note = Some("on call this week".to_owned());
    rec.note_by = Some("bob".to_owned());
    rec.note_at = Some(OffsetDateTime::now_utc());
    store
        .create_schedule(&new(rec.clone()), 10, &audit(&w, &rec))
        .await
        .expect("create");
    let after: i64 = sqlx::query_scalar(
        "SELECT coalesce(content_bytes, 0)::bigint FROM tenant_retained_usage WHERE tenant_id = $1",
    )
    .bind(w.tenant.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("usage");
    // the name, the note, and the spec and the action of the revision
    assert!(
        after - before
            >= i64::try_from("charged-name".len() + "on call this week".len()).expect("len"),
        "charged {} bytes",
        after - before
    );
}

#[tokio::test]
async fn an_input_is_kept_as_a_payload_of_the_tenant_and_shared_by_digest() {
    let w = world().await;
    let store = PgScheduleStore::new(w.db.pool.clone());
    let body = br#"{"region":"eu"}"#.to_vec();
    for name in ["with-input-a", "with-input-b"] {
        let mut rec = record(&w, name);
        rec.input_digest = Some(sha256(&body));
        let mut request = new(rec.clone());
        request.input = Some(StoredInput {
            digest: sha256(&body),
            bytes: body.clone(),
        });
        store
            .create_schedule(&request, 10, &audit(&w, &rec))
            .await
            .expect("create");
    }
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM payloads WHERE tenant_id = $1 AND digest = $2")
            .bind(w.tenant.as_uuid())
            .bind(sha256(&body).to_vec())
            .fetch_one(&w.db.pool)
            .await
            .expect("payloads");
    assert_eq!(stored, 1, "the same input is one payload");
    let read = store
        .schedules(w.tenant, w.namespace, None, 10)
        .await
        .expect("list");
    assert!(read.iter().all(|s| s.input_digest == Some(sha256(&body))));
}

#[tokio::test]
async fn an_input_whose_digest_is_not_the_digest_of_its_bytes_is_refused_and_leaves_nothing() {
    let w = world().await;
    let store = PgScheduleStore::new(w.db.pool.clone());
    let mut rec = record(&w, "forged-input");
    rec.input_digest = Some(sha256(b"something else"));
    let mut request = new(rec.clone());
    request.input = Some(StoredInput {
        digest: sha256(b"something else"),
        bytes: br#"{"region":"eu"}"#.to_vec(),
    });
    let refused = store
        .create_schedule(&request, 10, &audit(&w, &rec))
        .await
        .expect_err("the digest does not match");
    assert!(
        matches!(refused, StoreError::StateConflict(_)),
        "{refused:?}"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM schedules WHERE tenant_id = $1")
        .bind(w.tenant.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn a_schedule_of_another_tenant_is_never_seen() {
    let w = world().await;
    let store = PgScheduleStore::new(w.db.pool.clone());
    let created = create(&w, &store, "private", 10).await.expect("create");
    let stranger = TenantId::generate();
    assert!(
        store
            .schedule(stranger, created.id)
            .await
            .expect("read")
            .is_none()
    );
    assert!(
        store
            .schedules(stranger, w.namespace, None, 10)
            .await
            .expect("list")
            .is_empty()
    );
}

#[tokio::test]
async fn the_list_pages_by_id_and_hides_deleted_schedules() {
    let w = world().await;
    let store = PgScheduleStore::new(w.db.pool.clone());
    let mut ids = Vec::new();
    for n in 0..5 {
        ids.push(
            create(&w, &store, &format!("page-{n}"), 20)
                .await
                .expect("create")
                .id,
        );
    }
    sqlx::query("UPDATE schedules SET state = 'DELETED', deleted_at = now() WHERE tenant_id = $1 AND id = $2")
        .bind(w.tenant.as_uuid())
        .bind(ids[2].as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("delete");
    ids.remove(2);
    ids.sort();

    let first = store
        .schedules(w.tenant, w.namespace, None, 3)
        .await
        .expect("page 1");
    assert_eq!(
        first.iter().map(|s| s.id).collect::<Vec<_>>(),
        ids[..3].to_vec()
    );
    let second = store
        .schedules(w.tenant, w.namespace, Some(first[2].id), 3)
        .await
        .expect("page 2");
    assert_eq!(
        second.iter().map(|s| s.id).collect::<Vec<_>>(),
        ids[3..].to_vec()
    );
}

#[tokio::test]
async fn the_runtime_role_can_do_all_of_it() {
    let w = world().await;
    let runtime = w.db.runtime_pool().await;
    let store = PgScheduleStore::new(runtime);
    let created = create(&w, &store, "as-runtime", 10)
        .await
        .expect("create as the runtime role");
    assert!(
        store
            .schedule(w.tenant, created.id)
            .await
            .expect("read")
            .is_some()
    );
    assert_eq!(
        store
            .schedules(w.tenant, w.namespace, None, 10)
            .await
            .expect("list")
            .len(),
        1
    );
}
