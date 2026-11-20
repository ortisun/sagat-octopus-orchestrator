//! Changing a schedule in PostgreSQL (`T018-014`, `T018-023`, `T018-031`): a new revision under a
//! compare-and-swap, pause and resume with a note, and delete as a tombstone that frees the name.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::{ConflictKind, StoreError};
use sagat_application::schedule_codec::spec_from_json;
use sagat_application::schedules::{
    NewSchedule, Revise, ScheduleRecord, ScheduleState, ScheduleStore, StateChange, StateNote,
};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Principal, Role, Subject, TenantId};
use sagat_domain::schedules::{Ceilings, ScheduleName};
use sagat_infra_postgres::PgScheduleStore;
use serde_json::json;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::{TestDb, migrated_db};

struct World {
    db: TestDb,
    tenant: TenantId,
    namespace: NamespaceId,
    store: PgScheduleStore,
}

async fn world() -> World {
    let db = migrated_db().await;
    let (tenant, workspace, namespace) = (
        TenantId::generate(),
        Uuid::now_v7(),
        NamespaceId::generate(),
    );
    let short = tenant.as_uuid().simple().to_string()[..12].to_owned();
    seed(&db.pool, tenant, workspace, namespace, &short).await;
    let store = PgScheduleStore::new(db.pool.clone());
    World {
        db,
        tenant,
        namespace,
        store,
    }
}

async fn seed(
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

fn principal(w: &World) -> Principal {
    Principal::new(
        w.tenant,
        Subject::parse("alice").expect("subject"),
        Role::Developer,
    )
}

fn audit(w: &World, action: AuditAction, id: impl ToString) -> AuditRecord {
    AuditRecord::by(
        &principal(w),
        action,
        ResourceType::Schedule,
        id.to_string(),
        None,
        OffsetDateTime::now_utc(),
    )
}

/// A schedule built the way the service will build it.
async fn create(w: &World, name: &str) -> ScheduleRecord {
    let mut record = crate::store_support::record(w.tenant, w.namespace, name);
    record.spec =
        spec_from_json(&json!({"cron": ["0 9 * * *"]}), &Ceilings::default()).expect("spec");
    w.store
        .create_schedule(
            &NewSchedule {
                record: record.clone(),
                input: None,
                request_id: None,
            },
            100,
            &audit(w, AuditAction::ScheduleCreated, record.id),
        )
        .await
        .expect("create");
    w.store
        .schedule(w.tenant, record.id)
        .await
        .expect("read")
        .expect("exists")
}

fn revise(from: &ScheduleRecord, expected_revision: u32, name: &str, cron: &str) -> Revise {
    let mut desired = from.clone();
    desired.name = ScheduleName::new(name).expect("name");
    desired.revision = expected_revision + 1;
    desired.spec = spec_from_json(&json!({"cron": [cron]}), &Ceilings::default()).expect("spec");
    desired.authorized_by = "bob".to_owned();
    desired.updated_at = OffsetDateTime::now_utc();
    Revise {
        record: desired,
        expected_revision,
        input: None,
        request_id: Some("req-2".to_owned()),
    }
}

#[tokio::test]
async fn a_revision_moves_the_head_and_keeps_the_old_one() {
    let w = world().await;
    let created = create(&w, "reports").await;
    let request = revise(&created, 1, "reports", "30 6 * * 1");
    let changed = w
        .store
        .revise_schedule(
            &request,
            &audit(&w, AuditAction::ScheduleUpdated, created.id),
        )
        .await
        .expect("revise")
        .expect("exists");
    assert_eq!(changed.revision, 2);
    assert_eq!(changed.state_version, created.state_version + 1);
    assert_eq!(changed.spec.cron()[0].expression(), "30 6 * * 1");
    assert_eq!(changed.authorized_by, "bob");

    let revisions: Vec<(i32, String)> = sqlx::query_as(
        "SELECT revision, actor FROM schedule_revisions WHERE tenant_id = $1 AND schedule_id = $2 ORDER BY revision",
    )
    .bind(w.tenant.as_uuid())
    .bind(created.id.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("revisions");
    assert_eq!(
        revisions,
        vec![(1, "alice".to_owned()), (2, "bob".to_owned())]
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND resource_id = $2 AND action = 'schedule.updated'",
    )
    .bind(w.tenant.as_uuid())
    .bind(created.id.to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("audits");
    assert_eq!(audits, 1);
}

#[tokio::test]
async fn a_stale_expected_revision_is_a_conflict_that_changes_nothing() {
    let w = world().await;
    let created = create(&w, "stale").await;
    w.store
        .revise_schedule(
            &revise(&created, 1, "stale", "0 1 * * *"),
            &audit(&w, AuditAction::ScheduleUpdated, created.id),
        )
        .await
        .expect("first")
        .expect("exists");
    let refused = w
        .store
        .revise_schedule(
            &revise(&created, 1, "stale", "0 2 * * *"),
            &audit(&w, AuditAction::ScheduleUpdated, created.id),
        )
        .await
        .expect_err("the revision moved");
    assert!(
        matches!(refused, StoreError::RevisionConflict { current: 2 }),
        "{refused:?}"
    );
    let read = w
        .store
        .schedule(w.tenant, created.id)
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(
        (read.revision, read.spec.cron()[0].expression()),
        (2, "0 1 * * *")
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = 'schedule.updated'",
    )
    .bind(w.tenant.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("audits");
    assert_eq!(audits, 1, "the refused change left no audit record");
}

#[tokio::test]
async fn of_two_revisions_from_the_same_revision_exactly_one_wins() {
    let w = world().await;
    let created = create(&w, "racing").await;
    let mut tasks = Vec::new();
    for n in 0..6 {
        let store = w.store.clone();
        let request = revise(&created, 1, "racing", &format!("{n} 3 * * *"));
        let record = audit(&w, AuditAction::ScheduleUpdated, created.id);
        tasks.push(tokio::spawn(async move {
            store.revise_schedule(&request, &record).await
        }));
    }
    let (mut won, mut lost) = (0, 0);
    for task in tasks {
        match task.await.expect("task") {
            Ok(Some(_)) => won += 1,
            Err(StoreError::RevisionConflict { .. }) => lost += 1,
            other => panic!("unexpected: {other:?}"),
        }
    }
    assert_eq!((won, lost), (1, 5));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM schedule_revisions WHERE schedule_id = $1")
            .bind(created.id.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(count, 2, "one revision was added, not six");
}

#[tokio::test]
async fn a_revision_can_rename_but_not_to_a_name_that_is_taken() {
    let w = world().await;
    let first = create(&w, "alpha").await;
    create(&w, "beta").await;
    let taken = w
        .store
        .revise_schedule(
            &revise(&first, 1, "beta", "0 4 * * *"),
            &audit(&w, AuditAction::ScheduleUpdated, first.id),
        )
        .await
        .expect_err("beta exists");
    assert!(
        matches!(taken, StoreError::Conflict(ConflictKind::ScheduleName)),
        "{taken:?}"
    );
    let renamed = w
        .store
        .revise_schedule(
            &revise(&first, 1, "gamma", "0 4 * * *"),
            &audit(&w, AuditAction::ScheduleUpdated, first.id),
        )
        .await
        .expect("rename")
        .expect("exists");
    assert_eq!(renamed.name.as_str(), "gamma");
}

#[tokio::test]
async fn pausing_records_who_why_and_when_and_resuming_clears_nothing_it_should_keep() {
    let w = world().await;
    let created = create(&w, "maintenance").await;
    let at = OffsetDateTime::now_utc();
    let paused = w
        .store
        .set_schedule_state(
            &StateChange {
                tenant_id: w.tenant,
                id: created.id,
                state: ScheduleState::Paused,
                note: Some(StateNote {
                    text: "database upgrade".to_owned(),
                    by: "bob".to_owned(),
                    at,
                }),
                authorized_by: None,
                next_action_at: None,
                next_actual_at: None,
                updated_at: at,
            },
            &audit(&w, AuditAction::SchedulePaused, created.id),
        )
        .await
        .expect("pause")
        .expect("exists");
    assert_eq!(paused.state, ScheduleState::Paused);
    assert_eq!(paused.note.as_deref(), Some("database upgrade"));
    assert_eq!(paused.note_by.as_deref(), Some("bob"));
    assert_eq!(paused.state_version, created.state_version + 1);
    assert_eq!(paused.revision, 1, "pausing is not a revision");
    assert_eq!(
        paused.authorized_by, "alice",
        "pausing leaves the authority alone"
    );

    let resumed_at = at + Duration::hours(2);
    let next = resumed_at + Duration::hours(1);
    let resumed = w
        .store
        .set_schedule_state(
            &StateChange {
                tenant_id: w.tenant,
                id: created.id,
                state: ScheduleState::Active,
                note: Some(StateNote {
                    text: "done".to_owned(),
                    by: "carol".to_owned(),
                    at: resumed_at,
                }),
                authorized_by: Some("carol".to_owned()),
                next_action_at: Some(next),
                next_actual_at: Some(next),
                updated_at: resumed_at,
            },
            &audit(&w, AuditAction::ScheduleResumed, created.id),
        )
        .await
        .expect("resume")
        .expect("exists");
    assert_eq!(resumed.state, ScheduleState::Active);
    assert_eq!(
        resumed.authorized_by, "carol",
        "the resumer becomes the authority"
    );
    assert_eq!(resumed.note.as_deref(), Some("done"));
    assert!(resumed.next_action_at.is_some());
}

#[tokio::test]
async fn a_deleted_schedule_cannot_be_paused_or_revised() {
    let w = world().await;
    let created = create(&w, "gone").await;
    assert!(
        w.store
            .delete_schedule(
                w.tenant,
                created.id,
                None,
                OffsetDateTime::now_utc(),
                &audit(&w, AuditAction::ScheduleDeleted, created.id)
            )
            .await
            .expect("delete")
    );
    let paused = w
        .store
        .set_schedule_state(
            &StateChange {
                tenant_id: w.tenant,
                id: created.id,
                state: ScheduleState::Paused,
                note: None,
                authorized_by: None,
                next_action_at: None,
                next_actual_at: None,
                updated_at: OffsetDateTime::now_utc(),
            },
            &audit(&w, AuditAction::SchedulePaused, created.id),
        )
        .await
        .expect("no error");
    assert!(paused.is_none(), "a tombstone is not found for a change");
    let revised = w
        .store
        .revise_schedule(
            &revise(&created, 1, "gone", "0 5 * * *"),
            &audit(&w, AuditAction::ScheduleUpdated, created.id),
        )
        .await
        .expect("no error");
    assert!(revised.is_none());
}

#[tokio::test]
async fn deleting_leaves_a_tombstone_drops_waiting_actions_and_frees_the_name() {
    let w = world().await;
    let created = create(&w, "temporary").await;
    for (nominal, outcome) in [
        (1_000, "BUFFERED"),
        (2_000, "BUFFERED"),
        (3_000, "SKIPPED_OVERLAP"),
    ] {
        sqlx::query(
            "INSERT INTO schedule_actions (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, decided_at) \
             VALUES ($1, $2, 'SPEC', to_timestamp($3), 'SCHEDULED', 1, $4, now())",
        )
        .bind(w.tenant.as_uuid())
        .bind(created.id.as_uuid())
        .bind(f64::from(nominal))
        .bind(outcome)
        .execute(&w.db.pool)
        .await
        .expect("action");
    }
    sqlx::query("UPDATE schedules SET buffered = 2 WHERE tenant_id = $1 AND id = $2")
        .bind(w.tenant.as_uuid())
        .bind(created.id.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("buffered");

    let now = OffsetDateTime::now_utc();
    assert!(
        w.store
            .delete_schedule(
                w.tenant,
                created.id,
                None,
                now,
                &audit(&w, AuditAction::ScheduleDeleted, created.id)
            )
            .await
            .expect("delete")
    );
    let tombstone = w
        .store
        .schedule(w.tenant, created.id)
        .await
        .expect("read")
        .expect("kept");
    assert_eq!(tombstone.state, ScheduleState::Deleted);
    assert_eq!(
        (
            tombstone.buffered,
            tombstone.next_action_at,
            tombstone.next_actual_at
        ),
        (0, None, None)
    );
    let outcomes: Vec<(String, i64)> = sqlx::query_as(
        "SELECT outcome, count(*) FROM schedule_actions WHERE schedule_id = $1 GROUP BY outcome ORDER BY outcome",
    )
    .bind(created.id.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("outcomes");
    assert_eq!(
        outcomes,
        vec![
            ("DROPPED_DELETED".to_owned(), 2),
            ("SKIPPED_OVERLAP".to_owned(), 1)
        ],
        "waiting actions are dropped, final ones stay"
    );
    assert!(
        w.store
            .schedules(w.tenant, w.namespace, None, 10)
            .await
            .expect("list")
            .is_empty()
    );
    create(&w, "temporary").await;
    assert!(
        !w.store
            .delete_schedule(
                w.tenant,
                created.id,
                None,
                now,
                &audit(&w, AuditAction::ScheduleDeleted, created.id)
            )
            .await
            .expect("repeat"),
        "deleting twice is a no-op, not an error"
    );
}

#[tokio::test]
async fn nothing_changes_for_a_schedule_of_another_tenant() {
    let w = world().await;
    let created = create(&w, "mine").await;
    let stranger = TenantId::generate();
    let paused = w
        .store
        .set_schedule_state(
            &StateChange {
                tenant_id: stranger,
                id: created.id,
                state: ScheduleState::Paused,
                note: None,
                authorized_by: None,
                next_action_at: None,
                next_actual_at: None,
                updated_at: OffsetDateTime::now_utc(),
            },
            &audit(&w, AuditAction::SchedulePaused, created.id),
        )
        .await
        .expect("no error");
    assert!(paused.is_none());
    assert!(
        !w.store
            .delete_schedule(
                stranger,
                created.id,
                None,
                OffsetDateTime::now_utc(),
                &audit(&w, AuditAction::ScheduleDeleted, created.id)
            )
            .await
            .expect("no error")
    );
    let read = w
        .store
        .schedule(w.tenant, created.id)
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(read.state, ScheduleState::Active);
}

#[tokio::test]
async fn the_runtime_role_can_change_a_schedule() {
    let w = world().await;
    let created = create(&w, "runtime").await;
    let store = PgScheduleStore::new(w.db.runtime_pool().await);
    store
        .revise_schedule(
            &revise(&created, 1, "runtime", "0 7 * * *"),
            &audit(&w, AuditAction::ScheduleUpdated, created.id),
        )
        .await
        .expect("revise as the runtime role")
        .expect("exists");
    assert!(
        store
            .delete_schedule(
                w.tenant,
                created.id,
                None,
                OffsetDateTime::now_utc(),
                &audit(&w, AuditAction::ScheduleDeleted, created.id)
            )
            .await
            .expect("delete as the runtime role")
    );
}
