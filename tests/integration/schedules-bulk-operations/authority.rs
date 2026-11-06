//! Whose authority a schedule acts under (`T018-021`, `FR-018-015`, `FR-018-016`, `FR-018-041`,
//! `SC-018-004`): the subject that last created, updated or resumed it. Every action checks that
//! subject's `run:start` again, under the membership lock of the transaction that admits the run, and
//! for a workflow with managed database steps its feature 017 authority too; when it is gone the time
//! is recorded `AUTHORITY_REVOKED`, the schedule is paused with a system note and no run starts.
//! Real PostgreSQL, a real run service, a clock the test moves by hand.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::clock::SystemClock;
use sagat_application::database::{ConnectionService, ConnectionWrite};
use sagat_application::identity::RequestContext;
use sagat_application::schedule_firing::Fired;
use sagat_application::schedules::ScheduleState;
use sagat_domain::database::{ConnectionConfig, ConnectionStatus, DatabaseLimits, TlsPolicy};
use sagat_domain::definition::WorkflowId;
use sagat_domain::identity::{Principal, Role, Slug};
use sagat_domain::schedules::Note;
use sagat_domain::secrets::SecretName;
use serde_json::json;
use time::Duration;
use uuid::Uuid;

use crate::firing::{Fixture, fixture, hourly, outcomes_of};
use crate::support::engine::{ctx, document};

async fn remove(f: &Fixture, who: &Principal) {
    sqlx::query("DELETE FROM memberships WHERE tenant_id = $1 AND subject = $2")
        .bind(who.tenant_id().as_uuid())
        .bind(who.subject().as_str())
        .execute(&f.w.db.pool)
        .await
        .expect("remove the member");
}

async fn demote(f: &Fixture, who: &Principal, role: &str) {
    sqlx::query("UPDATE memberships SET role = $3 WHERE tenant_id = $1 AND subject = $2")
        .bind(who.tenant_id().as_uuid())
        .bind(who.subject().as_str())
        .bind(role)
        .execute(&f.w.db.pool)
        .await
        .expect("change the role");
}

/// A workflow with one managed database step, published by `publisher`, who may use the connection.
async fn database_workflow(f: &Fixture, publisher: &Principal) -> WorkflowId {
    let ns = f.w.namespace.id();
    f.w.put_secret("db-credentials", "credential-canary-018")
        .await;
    // A connection may only name a secret that OpenBao holds. Admission reads the catalog and the
    // memberships and never the credential, so the test marks the secret instead of starting a provider.
    sqlx::query(
        "UPDATE secrets SET authority = 'openbao' \
         WHERE tenant_id = $1 AND namespace_id = $2 AND name = 'db-credentials'",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(ns.as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("mark the secret as held by the provider");
    ConnectionService::new(f.w.store.clone(), SystemClock)
        .put(
            &f.w.admin,
            ns,
            ConnectionWrite {
                name: Slug::parse("scheduled-db").expect("slug"),
                expected_revision: 0,
                request_id: Uuid::new_v4(),
                status: ConnectionStatus::Active,
                config: ConnectionConfig::postgres(
                    "db.example.com",
                    5432,
                    "orders",
                    TlsPolicy::VerifyFull { ca_pem: None },
                    SecretName::parse("db-credentials").expect("secret name"),
                    DatabaseLimits::default(),
                )
                .expect("config"),
            },
            &ctx(),
        )
        .await
        .expect("the connection");
    f.w.secrets
        .grant(
            &f.w.admin,
            ns,
            "db-credentials",
            publisher.subject().as_str(),
            &ctx(),
        )
        .await
        .expect("the publisher may use the credentials");
    let (workflow, _) =
        f.w.try_publish_as(
            publisher,
            document(
                "scheduled-database",
                "database",
                vec![json!({
                    "id": "database", "type": "database",
                    "connectionRef": {"name": "scheduled-db"},
                    "operation": "query", "statement": "SELECT 1"
                })],
            ),
        )
        .await
        .expect("publish the database workflow");
    workflow
}

fn target_of(workflow: WorkflowId) -> serde_json::Value {
    json!({"workflowId": workflow.to_string()})
}

// --- the authority is the subject, and it is checked every time ---------------------------------

#[tokio::test]
async fn a_subject_removed_from_the_tenant_pauses_the_schedule_with_a_system_note_and_an_audit_record()
 {
    let f = fixture().await;
    let author = f.w.member("author-removed", Role::Developer).await;
    let schedule = f
        .create_as(&author, "removed", &hourly(), &json!({}), None)
        .await;
    assert_eq!(schedule.authorized_by, author.subject().as_str());
    remove(&f, &author).await;

    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;

    assert!(f.runs(&schedule).await.is_empty(), "no run starts");
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("AUTHORITY_REVOKED", 1)])
    );
    let paused = f.read(&schedule).await;
    assert_eq!(paused.state, ScheduleState::Paused);
    assert_eq!(
        paused.note.as_deref(),
        Some("the authorizing subject can no longer start runs"),
        "the system note says why"
    );
    let audit: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT action, actor, reason FROM audit_records \
         WHERE tenant_id = $1 AND resource_id = $2 AND action = 'schedule.auto_paused'",
    )
    .bind(schedule.tenant_id.as_uuid())
    .bind(schedule.id.to_string())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        audit,
        vec![(
            "schedule.auto_paused".to_owned(),
            format!("system:schedule:{}", schedule.id),
            Some("authority revoked".to_owned())
        )],
        "one audit record, by the schedule itself, with no payload"
    );

    f.clock.advance(Duration::hours(3));
    f.drain(&schedule).await;
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("AUTHORITY_REVOKED", 1)]),
        "a paused schedule stays quiet"
    );
}

#[tokio::test]
async fn a_schedule_paused_for_lost_authority_runs_again_under_whoever_resumes_it() {
    let f = fixture().await;
    let author = f.w.member("author-gone", Role::Developer).await;
    let schedule = f
        .create_as(&author, "handover", &hourly(), &json!({}), None)
        .await;
    remove(&f, &author).await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;
    assert_eq!(f.read(&schedule).await.state, ScheduleState::Paused);

    let operator = f.w.member("operator-resumes", Role::Operator).await;
    let resumed = f
        .service
        .resume(
            &operator,
            schedule.id,
            Note::new("restored").expect("note"),
            &RequestContext::default(),
        )
        .await
        .expect("resume");
    assert_eq!(resumed.authorized_by, operator.subject().as_str());

    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;
    let runs = f.runs(&schedule).await;
    assert_eq!(
        runs.len(),
        1,
        "the creator is gone and the resumer holds run:start"
    );
    let started_by: String = sqlx::query_scalar("SELECT started_by FROM runs WHERE id = $1")
        .bind(runs[0])
        .fetch_one(&f.w.db.pool)
        .await
        .expect("run");
    assert_eq!(started_by, format!("system:schedule:{}", schedule.id));
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("AUTHORITY_REVOKED", 1), ("STARTED", 1)])
    );
}

// --- the check is made under the membership lock, in the transaction that admits the run -------

#[tokio::test]
async fn a_revocation_that_commits_after_the_action_read_the_membership_stops_the_run() {
    let f = fixture().await;
    let author = f.w.member("author-late", Role::Developer).await;
    let schedule = f
        .create_as(&author, "late", &hourly(), &json!({}), None)
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));

    let fired = f
        .revoked_while_the_action_waits(&schedule, author.subject().as_str(), f.fire(&schedule))
        .await;

    assert_eq!(fired, Fired::Acted, "the refusal is recorded as the action");
    assert!(
        f.runs(&schedule).await.is_empty(),
        "no run starts on authority that was revoked before the run committed"
    );
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("AUTHORITY_REVOKED", 1)])
    );
    assert_eq!(f.read(&schedule).await.state, ScheduleState::Paused);
}

#[tokio::test]
async fn the_oldest_waiting_action_is_released_only_under_a_current_authority() {
    let f = fixture().await;
    let author = f.w.member("author-waiting", Role::Developer).await;
    let schedule = f
        .create_as(
            &author,
            "waiting",
            &hourly(),
            &json!({"overlap": "BUFFER_ONE"}),
            None,
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await; // 13:00 starts
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 14:00 waits
    assert_eq!(f.read(&schedule).await.buffered, 1);
    let first = f.runs(&schedule).await[0];
    f.end(first, "SUCCEEDED").await;
    f.clock.advance(Duration::seconds(6));
    assert_eq!(
        f.fire(&schedule).await,
        Fired::Acted,
        "the pass forgets the ended run first"
    );

    let fired = f
        .revoked_while_the_action_waits(&schedule, author.subject().as_str(), f.fire(&schedule))
        .await;

    assert_eq!(fired, Fired::Acted);
    assert_eq!(
        f.runs(&schedule).await.len(),
        1,
        "the waiting action does not start"
    );
    assert_eq!(f.read(&schedule).await.state, ScheduleState::Paused);
    assert!(
        f.outcomes(&schedule)
            .await
            .contains(&("AUTHORITY_REVOKED".to_owned(), 1))
    );
}

// --- feature 017: the database authority of the subject that authorized the schedule -----------

#[tokio::test]
async fn a_workflow_with_a_database_step_starts_under_the_database_authority_of_the_authorizer() {
    let f = fixture().await;
    let author = f.w.member("author-database", Role::Developer).await;
    let workflow = database_workflow(&f, &author).await;
    let schedule = f
        .create_as(
            &author,
            "database",
            &hourly(),
            &json!({}),
            Some(target_of(workflow)),
        )
        .await;

    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;

    assert_eq!(
        f.runs(&schedule).await.len(),
        1,
        "the run is admitted by the authority of the subject, not of `system:schedule:<id>`"
    );
    assert_eq!(f.outcomes(&schedule).await, outcomes_of(&[("STARTED", 1)]));
}

#[tokio::test]
async fn an_authorizer_who_loses_database_use_pauses_a_schedule_with_a_database_step() {
    let f = fixture().await;
    let author = f.w.member("author-no-database", Role::Developer).await;
    let workflow = database_workflow(&f, &author).await;
    let schedule = f
        .create_as(
            &author,
            "no-database",
            &hourly(),
            &json!({}),
            Some(target_of(workflow)),
        )
        .await;
    // an operator still starts runs, but may not use a database connection
    demote(&f, &author, "operator").await;

    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;

    assert!(f.runs(&schedule).await.is_empty());
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("AUTHORITY_REVOKED", 1)]),
        "losing database authority is losing the authority of the schedule"
    );
    assert_eq!(f.read(&schedule).await.state, ScheduleState::Paused);
}
