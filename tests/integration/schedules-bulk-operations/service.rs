//! The schedule service over a real PostgreSQL (`T018-016`, `T018-018`..`T018-023`, `T018-031`):
//! who may do what, what a write is checked against, and what the next time of a schedule is.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::clock::Clock;
use sagat_application::error::{AppError, ConflictKind};
use sagat_application::identity::RequestContext;
use sagat_application::schedule_codec::{policies_from_json, spec_from_json, target_from_json};
use sagat_application::schedules::{ScheduleService, ScheduleState, ScheduleWrite};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Role, Subject, TenantId};
use sagat_domain::schedules::{
    Ceilings, Note, RemainingActions, ScheduleId, ScheduleName, jitter_offset_ms, jitter_seed,
};
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::PgScheduleStore;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::{TestDb, migrated_db};

/// A clock the test moves by hand.
#[derive(Clone)]
struct TestClock(Arc<Mutex<OffsetDateTime>>);

impl TestClock {
    fn at(t: OffsetDateTime) -> Self {
        Self(Arc::new(Mutex::new(t)))
    }

    fn advance(&self, by: Duration) {
        let mut now = self.0.lock().expect("clock");
        *now += by;
    }
}

impl Clock for TestClock {
    fn now(&self) -> OffsetDateTime {
        *self.0.lock().expect("clock")
    }
}

type Service = ScheduleService<PgScheduleStore, SpecCalendar, TestClock>;

struct World {
    db: TestDb,
    tenant: TenantId,
    namespace: NamespaceId,
    workflow: Uuid,
    clock: TestClock,
    service: Service,
}

/// Wednesday 2026-10-28 12:00 UTC.
fn start() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_793_188_800).expect("instant")
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
    let workflow = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO workflows (id, tenant_id, namespace_id, name, draft, created_at, updated_at) \
         VALUES ($1, $2, $3, 'report', '{}'::jsonb, now(), now())",
    )
    .bind(workflow)
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .execute(&db.pool)
    .await
    .expect("workflow");
    let clock = TestClock::at(start());
    let service = ScheduleService::new(
        PgScheduleStore::new(db.pool.clone()),
        SpecCalendar,
        clock.clone(),
        Ceilings::default(),
    );
    World {
        db,
        tenant,
        namespace,
        workflow,
        clock,
        service,
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

fn who(w: &World, subject: &str, role: Role) -> Principal {
    Principal::new(w.tenant, Subject::parse(subject).expect("subject"), role)
}

fn ctx() -> RequestContext {
    RequestContext::default()
}

fn write(w: &World, name: &str, spec: &Value) -> ScheduleWrite {
    ScheduleWrite {
        name: ScheduleName::new(name).expect("name"),
        spec: spec_from_json(spec, &Ceilings::default()).expect("spec"),
        policies: policies_from_json(&json!({}), &Ceilings::default()).expect("policies"),
        target: target_from_json(&json!({"workflowId": w.workflow.to_string()})).expect("target"),
        input: json!({"region": "eu"}),
        paused: false,
        note: None,
        remaining_actions: None,
    }
}

fn daily_nine() -> Value {
    json!({"calendars": [{"hour": [{"start": 9}]}], "timezone": "UTC"})
}

fn reason(error: &AppError) -> (&'static str, Option<&str>) {
    match error {
        AppError::InvalidSchedule { reason, field } => (reason, field.as_deref()),
        other => panic!("not a schedule validation error: {other:?}"),
    }
}

// --- creating ---------------------------------------------------------------------------------

#[tokio::test]
async fn a_new_schedule_knows_its_next_time_and_whose_authority_it_runs_under() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let created = w
        .service
        .create(
            &alice,
            w.namespace,
            write(&w, "daily", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("create");
    assert_eq!((created.revision, created.state_version), (1, 1));
    assert_eq!(created.state, ScheduleState::Active);
    assert_eq!(created.authorized_by, "alice");
    // it is Wednesday noon: the next 09:00 UTC is Thursday
    assert_eq!(created.next_action_at, Some(start() + Duration::hours(21)));
    assert_eq!(created.next_actual_at, created.next_action_at, "no jitter");
    assert!(created.input_digest.is_some());
    let again = w.service.get(&alice, created.id).await.expect("get");
    assert_eq!(again, created);
}

#[tokio::test]
async fn jitter_moves_the_actual_time_by_a_function_of_the_schedule_and_the_nominal_time() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let spec =
        json!({"calendars": [{"hour": [{"start": 9}]}], "timezone": "UTC", "jitterMs": 600_000});
    let created = w
        .service
        .create(&alice, w.namespace, write(&w, "jittered", &spec), &ctx())
        .await
        .expect("create");
    let nominal = created.next_action_at.expect("nominal");
    let digest: [u8; 32] = Sha256::digest(jitter_seed(created.id.as_uuid(), nominal)).into();
    let offset = jitter_offset_ms(&digest, 600_000, Some(86_400_000));
    assert_eq!(
        created.next_actual_at,
        Some(nominal + Duration::milliseconds(i64::try_from(offset).expect("ms")))
    );
    assert!(offset < 600_000);
}

#[tokio::test]
async fn only_people_who_write_and_start_workflows_may_create() {
    let w = world().await;
    for (role, allowed) in [
        (Role::Admin, true),
        (Role::Developer, true),
        (Role::Operator, false),
        (Role::Viewer, false),
    ] {
        let outcome = w
            .service
            .create(
                &who(&w, "someone", role),
                w.namespace,
                write(&w, &format!("by-{role}"), &daily_nine()),
                &ctx(),
            )
            .await;
        match (allowed, outcome) {
            (true, Ok(_)) => {}
            (false, Err(AppError::Forbidden { permission })) => {
                assert_eq!(permission, Permission::ScheduleWrite)
            }
            (_, other) => panic!("{role}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_namespace_that_does_not_exist_for_the_tenant_is_not_found() {
    let w = world().await;
    let error = w
        .service
        .create(
            &who(&w, "alice", Role::Developer),
            NamespaceId::generate(),
            write(&w, "lost", &daily_nine()),
            &ctx(),
        )
        .await
        .expect_err("no such namespace");
    assert!(matches!(error, AppError::NotFound), "{error:?}");
}

#[tokio::test]
async fn a_workflow_that_is_not_in_the_namespace_is_refused_where_it_is_named() {
    let w = world().await;
    let mut request = write(&w, "nowhere", &daily_nine());
    request.target =
        target_from_json(&json!({"workflowId": Uuid::now_v7().to_string()})).expect("target");
    let error = w
        .service
        .create(
            &who(&w, "alice", Role::Developer),
            w.namespace,
            request,
            &ctx(),
        )
        .await
        .expect_err("no such workflow");
    assert_eq!(
        reason(&error),
        ("SCHEDULE_SPEC_INVALID", Some("/action/workflowId"))
    );
}

#[tokio::test]
async fn a_spec_the_calendar_cannot_accept_is_refused_with_the_reason_of_the_contract() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let cases: [(Value, &str); 4] = [
        (
            json!({"cron": ["0 9 * * *"], "timezone": "Mars/Olympus_Mons"}),
            "SCHEDULE_ZONE_UNKNOWN",
        ),
        (
            json!({"calendars": [{"month": [{"start": 2}], "dayOfMonth": [{"start": 30}]}]}),
            "SCHEDULE_NEVER_FIRES",
        ),
        (
            json!({"intervals": [{"everyMs": 10_000}]}),
            "SCHEDULE_TOO_FREQUENT",
        ),
        (
            json!({"cron": ["0 9 * * *"], "endAt": "2020-01-01T00:00:00Z"}),
            "SCHEDULE_NEVER_FIRES",
        ),
    ];
    for (n, (spec, expected)) in cases.into_iter().enumerate() {
        let error = w
            .service
            .create(
                &alice,
                w.namespace,
                write(&w, &format!("bad-{n}"), &spec),
                &ctx(),
            )
            .await
            .expect_err(expected);
        assert_eq!(reason(&error).0, expected, "{spec}");
    }
}

#[tokio::test]
async fn an_input_must_be_an_object_and_stay_small() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let mut request = write(&w, "input-array", &daily_nine());
    request.input = json!([1, 2, 3]);
    let error = w
        .service
        .create(&alice, w.namespace, request, &ctx())
        .await
        .expect_err("array");
    assert_eq!(
        reason(&error),
        ("SCHEDULE_INPUT_INVALID", Some("/action/input"))
    );

    let mut request = write(&w, "input-huge", &daily_nine());
    request.input = json!({"blob": "x".repeat(70 * 1024)});
    let error = w
        .service
        .create(&alice, w.namespace, request, &ctx())
        .await
        .expect_err("too large");
    assert!(matches!(error, AppError::PayloadTooLarge(_)), "{error:?}");
}

#[tokio::test]
async fn the_second_schedule_with_a_name_is_a_conflict_and_the_namespace_quota_is_enforced() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    w.service
        .create(
            &alice,
            w.namespace,
            write(&w, "twin", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("first");
    let again = w
        .service
        .create(
            &alice,
            w.namespace,
            write(&w, "twin", &daily_nine()),
            &ctx(),
        )
        .await
        .expect_err("same name");
    assert!(
        matches!(again, AppError::Conflict(ConflictKind::ScheduleName)),
        "{again:?}"
    );

    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'schedules', 2, 'admin')",
    )
    .bind(w.tenant.as_uuid())
    .bind(w.namespace.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("limit");
    w.service
        .create(
            &alice,
            w.namespace,
            write(&w, "second", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("second");
    let full = w
        .service
        .create(
            &alice,
            w.namespace,
            write(&w, "third", &daily_nine()),
            &ctx(),
        )
        .await
        .expect_err("the namespace is full");
    assert!(matches!(full, AppError::QuotaExceeded(_)), "{full:?}");
}

#[tokio::test]
async fn a_schedule_created_paused_has_no_next_time_until_it_is_resumed() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let mut request = write(&w, "waiting", &daily_nine());
    request.paused = true;
    request.note = Some(Note::new("not yet").expect("note"));
    request.remaining_actions = Some(RemainingActions::new(3).expect("remaining"));
    let created = w
        .service
        .create(&alice, w.namespace, request, &ctx())
        .await
        .expect("create");
    assert_eq!(created.state, ScheduleState::Paused);
    assert_eq!(
        (created.next_action_at, created.next_actual_at),
        (None, None)
    );
    assert_eq!(created.note.as_deref(), Some("not yet"));
    assert_eq!(created.note_by.as_deref(), Some("alice"));
    assert_eq!(created.remaining_actions, Some(3));
}

// --- reading ----------------------------------------------------------------------------------

#[tokio::test]
async fn anyone_who_reads_runs_reads_schedules_and_nobody_reads_another_tenants() {
    let w = world().await;
    let created = w
        .service
        .create(
            &who(&w, "alice", Role::Developer),
            w.namespace,
            write(&w, "shared", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("create");
    let viewer = who(&w, "vera", Role::Viewer);
    assert_eq!(
        w.service.get(&viewer, created.id).await.expect("get").id,
        created.id
    );
    assert_eq!(
        w.service
            .list(&viewer, w.namespace, None, 10)
            .await
            .expect("list")
            .len(),
        1
    );

    let worker = who(&w, "robot", Role::Worker);
    assert!(matches!(
        w.service.get(&worker, created.id).await,
        Err(AppError::Forbidden {
            permission: Permission::ScheduleRead
        })
    ));
    let stranger = Principal::new(
        TenantId::generate(),
        Subject::parse("eve").expect("subject"),
        Role::Admin,
    );
    assert!(matches!(
        w.service.get(&stranger, created.id).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        w.service.get(&viewer, ScheduleId::generate()).await,
        Err(AppError::NotFound)
    ));
}

// --- updating ---------------------------------------------------------------------------------

#[tokio::test]
async fn an_update_is_a_new_revision_from_the_first_time_after_it() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let created = w
        .service
        .create(
            &alice,
            w.namespace,
            write(&w, "evolving", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("create");
    w.clock.advance(Duration::hours(2)); // 14:00
    let bob = who(&w, "bob", Role::Admin);
    let spec = json!({"calendars": [{"hour": [{"start": 16}]}], "timezone": "UTC"});
    let updated = w
        .service
        .update(&bob, created.id, write(&w, "evolving", &spec), 1, &ctx())
        .await
        .expect("update");
    assert_eq!(updated.revision, 2);
    assert_eq!(
        updated.authorized_by, "bob",
        "the updater is the authority now"
    );
    assert_eq!(
        updated.next_action_at,
        Some(start() + Duration::hours(4)),
        "16:00 today, after the update"
    );
}

#[tokio::test]
async fn updating_from_a_stale_revision_names_the_current_one() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let created = w
        .service
        .create(
            &alice,
            w.namespace,
            write(&w, "stale", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("create");
    w.service
        .update(
            &alice,
            created.id,
            write(&w, "stale", &daily_nine()),
            1,
            &ctx(),
        )
        .await
        .expect("first update");
    let error = w
        .service
        .update(
            &alice,
            created.id,
            write(&w, "stale", &daily_nine()),
            1,
            &ctx(),
        )
        .await
        .expect_err("stale");
    assert!(
        matches!(error, AppError::RevisionConflict { current: 2 }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_schedule_owned_by_a_cron_trigger_is_changed_through_its_workflow() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let created = w
        .service
        .create(
            &alice,
            w.namespace,
            write(&w, "owned", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("create");
    sqlx::query("ALTER TABLE schedules DISABLE TRIGGER schedules_identity")
        .execute(&w.db.pool)
        .await
        .expect("the test turns the identity guard off to make a trigger-owned schedule");
    sqlx::query("UPDATE schedules SET origin = 'TRIGGER', trigger_id = $2 WHERE id = $1")
        .bind(created.id.as_uuid())
        .bind(Uuid::now_v7())
        .execute(&w.db.pool)
        .await
        .expect("make it trigger-owned");
    for outcome in [
        w.service
            .update(
                &alice,
                created.id,
                write(&w, "owned", &daily_nine()),
                1,
                &ctx(),
            )
            .await
            .map(|_| ()),
        w.service.delete(&alice, created.id, None, &ctx()).await,
    ] {
        assert!(
            matches!(
                outcome,
                Err(AppError::ScheduleConflict {
                    reason: "SCHEDULE_TRIGGER_OWNED"
                })
            ),
            "{outcome:?}"
        );
    }
}

// --- pause, resume, delete --------------------------------------------------------------------

#[tokio::test]
async fn operators_pause_and_resume_with_a_note_and_the_next_time_follows_the_resume() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let created = w
        .service
        .create(&alice, w.namespace, write(&w, "ops", &daily_nine()), &ctx())
        .await
        .expect("create");
    let olga = who(&w, "olga", Role::Operator);
    let paused = w
        .service
        .pause(
            &olga,
            created.id,
            Note::new("database upgrade").expect("note"),
            &ctx(),
        )
        .await
        .expect("pause");
    assert_eq!(paused.state, ScheduleState::Paused);
    assert_eq!(paused.note_by.as_deref(), Some("olga"));
    assert_eq!((paused.next_action_at, paused.next_actual_at), (None, None));
    assert_eq!(paused.authorized_by, "alice");

    w.clock.advance(Duration::days(3));
    let resumed = w
        .service
        .resume(&olga, created.id, Note::new("done").expect("note"), &ctx())
        .await
        .expect("resume");
    assert_eq!(resumed.state, ScheduleState::Active);
    assert_eq!(resumed.authorized_by, "olga");
    let expected = start() + Duration::days(3) + Duration::hours(21);
    assert_eq!(
        resumed.next_action_at,
        Some(expected),
        "continues from the first time after the resume, nothing caught up"
    );
}

#[tokio::test]
async fn pausing_needs_a_note_and_the_operate_permission() {
    let w = world().await;
    let created = w
        .service
        .create(
            &who(&w, "alice", Role::Developer),
            w.namespace,
            write(&w, "noted", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("create");
    let viewer = who(&w, "vera", Role::Viewer);
    assert!(matches!(
        w.service
            .pause(&viewer, created.id, Note::new("x").expect("note"), &ctx())
            .await,
        Err(AppError::Forbidden {
            permission: Permission::ScheduleOperate
        })
    ));
    let error = w
        .service
        .pause(
            &who(&w, "olga", Role::Operator),
            created.id,
            Note::new("").expect("an empty note is a Note"),
            &ctx(),
        )
        .await
        .expect_err("an empty note");
    assert_eq!(reason(&error), ("SCHEDULE_SPEC_INVALID", Some("/note")));
}

#[tokio::test]
async fn deleting_is_for_writers_and_leaves_nothing_to_read() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let created = w
        .service
        .create(
            &alice,
            w.namespace,
            write(&w, "doomed", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("create");
    assert!(matches!(
        w.service
            .delete(&who(&w, "olga", Role::Operator), created.id, None, &ctx())
            .await,
        Err(AppError::Forbidden {
            permission: Permission::ScheduleWrite
        })
    ));
    w.service
        .delete(&alice, created.id, None, &ctx())
        .await
        .expect("delete");
    assert!(matches!(
        w.service.get(&alice, created.id).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        w.service.delete(&alice, created.id, None, &ctx()).await,
        Err(AppError::NotFound)
    ));
    assert!(
        w.service
            .list(&alice, w.namespace, None, 10)
            .await
            .expect("list")
            .is_empty()
    );
    w.service
        .create(
            &alice,
            w.namespace,
            write(&w, "doomed", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("the name is free");
}

#[tokio::test]
async fn every_change_is_audited_with_who_did_it_and_never_a_payload() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let created = w
        .service
        .create(
            &alice,
            w.namespace,
            write(&w, "audited", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("create");
    let olga = who(&w, "olga", Role::Operator);
    w.service
        .pause(
            &olga,
            created.id,
            Note::new("secret maintenance window").expect("note"),
            &ctx(),
        )
        .await
        .expect("pause");
    w.service
        .resume(&olga, created.id, Note::new("back").expect("note"), &ctx())
        .await
        .expect("resume");
    w.service
        .delete(&alice, created.id, None, &ctx())
        .await
        .expect("delete");
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT action, actor, reason FROM audit_records WHERE tenant_id = $1 AND resource_id = $2 ORDER BY created_at, id",
    )
    .bind(w.tenant.as_uuid())
    .bind(created.id.to_string())
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    let actions: Vec<(&str, &str)> = rows
        .iter()
        .map(|(a, who, _)| (a.as_str(), who.as_str()))
        .collect();
    assert_eq!(
        actions,
        vec![
            ("schedule.created", "alice"),
            ("schedule.paused", "olga"),
            ("schedule.resumed", "olga"),
            ("schedule.deleted", "alice"),
        ]
    );
    for (_, _, reason) in &rows {
        let reason = reason.as_deref().unwrap_or_default();
        assert!(
            !reason.contains("secret maintenance window") && !reason.contains("eu"),
            "{reason}"
        );
    }
}

// --- describing and deleting at a revision ---------------------------------------------------------

#[tokio::test]
async fn a_description_has_the_next_times_the_counters_and_the_latest_decisions() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let created = w
        .service
        .create(
            &alice,
            w.namespace,
            write(&w, "described", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("create");
    let quiet = w
        .service
        .describe(&alice, created.id, 3, 5)
        .await
        .expect("describe");
    assert_eq!(quiet.record, created);
    assert_eq!(
        quiet.next,
        (0..3)
            .map(|d| {
                let t = start() + Duration::hours(21) + Duration::days(d);
                (t, t)
            })
            .collect::<Vec<_>>(),
        "Thursday, Friday and Saturday at 09:00, no jitter"
    );
    assert!(quiet.activity.recent.is_empty() && quiet.activity.counters.is_empty());

    // three decisions, one of them a summary, and the counters the pass keeps
    for (nominal, outcome, reason, summary) in [
        (1_000, "SKIPPED_OVERLAP", Some("OVERLAP"), None),
        (2_000, "DROPPED_BUFFER_FULL", None, None),
        (3_000, "COALESCED", None, Some((4, 2_900))),
    ] {
        sqlx::query(
            "INSERT INTO schedule_actions (tenant_id, schedule_id, origin, nominal_at, kind, revision, \
             outcome, reason, decided_at, summary_count, summary_last_at) \
             VALUES ($1, $2, 'SPEC', to_timestamp($3), 'SCHEDULED', 1, $4, $5, to_timestamp($3), $6, \
             to_timestamp($7))",
        )
        .bind(w.tenant.as_uuid())
        .bind(created.id.as_uuid())
        .bind(f64::from(nominal))
        .bind(outcome)
        .bind(reason)
        .bind(summary.map(|(n, _)| n))
        .bind(summary.map(|(_, t)| f64::from(t)))
        .execute(&w.db.pool)
        .await
        .expect("action");
    }
    sqlx::query(
        "UPDATE schedules SET counters = '{\"SKIPPED_OVERLAP\": 1, \"STARTED\": 7}' WHERE id = $1",
    )
    .bind(created.id.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("counters");

    let busy = w
        .service
        .describe(&alice, created.id, 0, 2)
        .await
        .expect("describe");
    assert!(busy.next.is_empty(), "none were asked for");
    assert_eq!(
        busy.activity
            .recent
            .iter()
            .map(|a| (a.outcome.as_str(), a.reason.as_deref(), a.summary_count))
            .collect::<Vec<_>>(),
        vec![
            ("COALESCED", None, Some(4)),
            ("DROPPED_BUFFER_FULL", None, None)
        ],
        "the latest two decisions, newest first"
    );
    assert_eq!(
        busy.activity.recent[0].summary_last_nominal_at,
        Some(OffsetDateTime::from_unix_timestamp(2_900).expect("instant"))
    );
    assert_eq!(
        busy.activity.counters.into_iter().collect::<Vec<_>>(),
        vec![("SKIPPED_OVERLAP".to_owned(), 1), ("STARTED".to_owned(), 7)]
    );
}

#[tokio::test]
async fn a_paused_schedule_lists_no_next_times_and_reading_needs_schedule_read() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let created = w
        .service
        .create(
            &alice,
            w.namespace,
            write(&w, "resting", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("create");
    w.service
        .pause(
            &alice,
            created.id,
            Note::new("not now").expect("note"),
            &ctx(),
        )
        .await
        .expect("pause");
    let paused = w
        .service
        .describe(&alice, created.id, 10, 10)
        .await
        .expect("describe");
    assert!(paused.next.is_empty(), "nothing is acted on while paused");

    let worker = who(&w, "robot", Role::Worker);
    assert!(matches!(
        w.service.describe(&worker, created.id, 1, 1).await,
        Err(AppError::Forbidden {
            permission: Permission::ScheduleRead
        })
    ));
    let stranger = Principal::new(
        TenantId::generate(),
        Subject::parse("mallory").expect("subject"),
        Role::Admin,
    );
    assert!(matches!(
        w.service.describe(&stranger, created.id, 1, 1).await,
        Err(AppError::NotFound)
    ));
}

#[tokio::test]
async fn a_delete_at_a_stale_revision_deletes_nothing_and_at_the_head_deletes() {
    let w = world().await;
    let alice = who(&w, "alice", Role::Developer);
    let created = w
        .service
        .create(
            &alice,
            w.namespace,
            write(&w, "versioned", &daily_nine()),
            &ctx(),
        )
        .await
        .expect("create");
    let revised = w
        .service
        .update(
            &alice,
            created.id,
            write(&w, "versioned", &daily_nine()),
            1,
            &ctx(),
        )
        .await
        .expect("a second revision");
    assert_eq!(revised.revision, 2);
    let stale = w
        .service
        .delete(&alice, created.id, Some(1), &ctx())
        .await
        .expect_err("revision 1 is not the head");
    assert!(
        matches!(stale, AppError::RevisionConflict { current: 2 }),
        "{stale:?}"
    );
    assert!(
        w.service.get(&alice, created.id).await.is_ok(),
        "still there"
    );
    w.service
        .delete(&alice, created.id, Some(2), &ctx())
        .await
        .expect("delete at the head");
    assert!(matches!(
        w.service.get(&alice, created.id).await,
        Err(AppError::NotFound)
    ));
}
