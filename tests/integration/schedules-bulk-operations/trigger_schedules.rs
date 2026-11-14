//! Trigger-owned schedules (`T018-060`, `T018-061`, `FR-018-017`, `SC-018-005`): the cron triggers of feature 004 are
//! `TRIGGER` schedules that fire with the 004 identity, publication keeps them equal to the triggers in the publishing
//! transaction, a reconciliation covers what publication did not, and their control waits for the operator's handover.
//! Real PostgreSQL (the runtime role in the sweep), the real legacy cron pass of the world, a clock the test moves by hand.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::clock::Clock as _;
use sagat_application::error::AppError;
use sagat_application::quotas::QuotaStore as _;
use sagat_application::schedule_codec::{policies_from_json, spec_from_json, target_from_json};
use sagat_application::schedule_firing::{Fired, TriggerRequest};
use sagat_application::schedules::{ScheduleOrigin, ScheduleState, ScheduleWrite};
use sagat_domain::quota::QuotaResource;
use sagat_domain::schedules::{CatchupMode, Ceilings, Note, Overlap, ScheduleName};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::engine::{chain, ctx};
use crate::support::trigger_schedules::{Triggers, cron_workflow, triggers};

/// A workflow whose second version keeps the cron trigger of the first (a new entry node makes it another version).
fn second_version(name: &str, expression: &str, zone: &str) -> Value {
    let mut doc = crate::support::engine::document(
        name,
        "work2",
        vec![crate::support::engine::activity("work2")],
    );
    doc["triggers"] = json!([{"kind": "cron", "cron": expression, "timezone": zone}]);
    doc
}

fn without_trigger(name: &str) -> Value {
    crate::support::engine::document(
        name,
        "work3",
        vec![crate::support::engine::activity("work3")],
    )
}

impl Triggers {
    /// A schedule of the API, the kind the quota counts.
    pub(crate) async fn api_schedule(&self, name: &str) -> Result<(), AppError> {
        let write = self.api_write(name).await;
        self.service
            .create(&self.w.admin, self.w.namespace.id(), write, &ctx())
            .await
            .map(|_| ())
    }

    /// The write of an API schedule of this name.
    pub(crate) async fn api_write(&self, name: &str) -> ScheduleWrite {
        let (workflow, _) = self.w.publish(chain(&format!("target-{name}"))).await;
        ScheduleWrite {
            name: ScheduleName::new(name).expect("name"),
            spec: spec_from_json(
                &json!({"intervals": [{"everyMs": 3_600_000}]}),
                &Ceilings::HARD,
            )
            .expect("spec"),
            policies: policies_from_json(&json!({}), &Ceilings::HARD).expect("policies"),
            target: target_from_json(&json!({"workflowId": workflow.to_string()})).expect("target"),
            input: json!({}),
            paused: false,
            note: None,
            remaining_actions: None,
        }
    }
}

// --- publication ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn publishing_a_cron_trigger_makes_its_schedule_with_the_policies_of_feature_004() {
    let t = triggers().await;
    let (workflow, _) = t
        .publish(cron_workflow(
            "every-five",
            "*/5 * * * *",
            "America/Sao_Paulo",
        ))
        .await;
    let (trigger, version, expression, zone) = t.trigger_of("every-five").await;
    let schedule = t
        .schedule_of(trigger)
        .await
        .expect("the trigger has a schedule");

    assert_eq!(schedule.origin, ScheduleOrigin::Trigger);
    assert_eq!(schedule.trigger_id, Some(trigger));
    assert_eq!(schedule.state, ScheduleState::Active);
    assert_eq!(schedule.revision, 1);
    assert_eq!(
        schedule.authorized_by, "system:engine",
        "the system principal of the legacy pass"
    );
    assert_eq!(schedule.target.workflow(), workflow);
    assert_eq!(
        (
            schedule.spec.cron()[0].expression(),
            schedule.spec.timezone()
        ),
        (expression.as_str(), zone.as_str())
    );
    assert_eq!(schedule.spec.cron().len(), 1);
    assert_eq!(schedule.spec.jitter_ms(), 0, "no jitter");
    let dst = schedule.spec.dst();
    assert_eq!(
        (dst.gap(), dst.ambiguous()),
        (
            sagat_domain::schedules::DstGap::Skip,
            sagat_domain::schedules::DstAmbiguous::OnceEarlier
        ),
        "SKIP and ONCE_EARLIER"
    );
    assert_eq!(schedule.policies.overlap(), Overlap::AllowAll);
    assert_eq!(schedule.policies.catchup_mode(), CatchupMode::One);
    assert_eq!(
        schedule.policies.catchup_window_ms(),
        366 * 24 * 3_600 * 1_000
    );
    assert!(
        schedule.target.nominal_time_field().is_none() && schedule.target.business_id().is_none()
    );
    assert_eq!(
        (schedule.next_action_at, schedule.next_actual_at),
        (None, None),
        "publication has no calendar: the pass that plans the trigger gives it its first time"
    );
    let _ = version;

    // it is in the list the API serves, with the trigger it follows
    let listed = t.listed().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].trigger_id, Some(trigger));

    // and the publishing transaction wrote its audit, as the engine
    let (actor, reason): (String, Option<String>) = sqlx::query_as(
        "SELECT actor, reason FROM audit_records WHERE action = 'schedule.created' AND resource_id = $1",
    )
    .bind(schedule.id.to_string())
    .fetch_one(&t.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(actor, "system:engine");
    assert!(reason.is_some_and(|r| r.contains("cron trigger")));
}

#[tokio::test]
async fn republishing_with_the_same_trigger_keeps_the_schedule_and_its_place() {
    let t = triggers().await;
    let (workflow, first) = t.publish(cron_workflow("kept", "0 * * * *", "UTC")).await;
    let (trigger, ..) = t.trigger_of("kept").await;
    assert_eq!(t.reconcile().await, 1, "the first time is planned");
    let before = t.schedule_of(trigger).await.expect("schedule");
    assert!(before.next_action_at.is_some());

    let second =
        t.w.publish_new_version(workflow, second_version("kept", "0 * * * *", "UTC"))
            .await;
    assert_ne!(first, second);
    let (same, version, ..) = t.trigger_of("kept").await;
    assert_eq!(same, trigger, "a kept trigger keeps its id");
    assert_eq!(version, second.as_uuid(), "and starts the version in force");
    let after = t.schedule_of(trigger).await.expect("schedule");
    assert_eq!(after.id, before.id, "the schedule is the same");
    assert_eq!(after.revision, before.revision);
    assert_eq!(
        after.state_version, before.state_version,
        "nothing touched it"
    );
    assert_eq!(
        after.next_action_at, before.next_action_at,
        "it keeps its place"
    );
    assert_eq!(t.listed().await.len(), 1);
}

#[tokio::test]
async fn a_schedule_that_is_paused_stays_paused_when_its_workflow_is_published_again() {
    let t = triggers().await;
    let (workflow, _) = t.publish(cron_workflow("paused", "0 * * * *", "UTC")).await;
    let (trigger, ..) = t.trigger_of("paused").await;
    t.reconcile().await;
    t.hand_over().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    t.service
        .pause(
            &t.w.admin,
            schedule.id,
            Note::new("maintenance").expect("note"),
            &ctx(),
        )
        .await
        .expect("pause after the handover");

    t.w.publish_new_version(workflow, second_version("paused", "0 * * * *", "UTC"))
        .await;
    let after = t.read(&schedule).await;
    assert_eq!(
        after.state,
        ScheduleState::Paused,
        "a pause is not undone by a publication"
    );
    assert_eq!(after.note.as_deref(), Some("maintenance"));
}

#[tokio::test]
async fn dropping_the_trigger_removes_its_schedule_in_the_publishing_transaction() {
    let t = triggers().await;
    let (workflow, _) = t
        .publish(cron_workflow("dropped", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("dropped").await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    // an action that waits, and one that is final: the first is dropped, the second stays as history
    for (nominal, outcome) in [(3_600_i64, "BUFFERED"), (7_200, "SKIPPED_OVERLAP")] {
        sqlx::query(
            "INSERT INTO schedule_actions (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, decided_at) \
             VALUES ($1, $2, 'SPEC', to_timestamp($3), 'SCHEDULED', 1, $4, now())",
        )
        .bind(schedule.tenant_id.as_uuid())
        .bind(schedule.id.as_uuid())
        .bind(nominal)
        .bind(outcome)
        .execute(&t.w.db.pool)
        .await
        .expect("an action");
    }

    t.w.publish_new_version(workflow, without_trigger("dropped"))
        .await;

    let gone = t.read(&schedule).await;
    assert_eq!(gone.state, ScheduleState::Deleted);
    assert_eq!((gone.next_action_at, gone.next_actual_at), (None, None));
    assert_eq!(gone.buffered, 0);
    let outcomes: Vec<(String,)> = sqlx::query_as(
        "SELECT outcome FROM schedule_actions WHERE schedule_id = $1 ORDER BY nominal_at",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&t.w.db.pool)
    .await
    .expect("outcomes");
    assert_eq!(
        outcomes.into_iter().map(|(o,)| o).collect::<Vec<_>>(),
        ["DROPPED_DELETED", "SKIPPED_OVERLAP"],
        "the waiting action is dropped and the history is kept"
    );
    let triggers_left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM workflow_triggers WHERE id = $1")
            .bind(trigger)
            .fetch_one(&t.w.db.pool)
            .await
            .expect("count");
    assert_eq!(triggers_left, 0);
    assert!(
        t.listed().await.is_empty(),
        "a removed schedule is not listed"
    );
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'schedule.deleted' AND resource_id = $1 AND actor = 'system:engine'",
    )
    .bind(schedule.id.to_string())
    .fetch_one(&t.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(audited, 1);
}

#[tokio::test]
async fn a_changed_expression_is_another_trigger_with_another_schedule() {
    let t = triggers().await;
    let (workflow, _) = t
        .publish(cron_workflow("changes", "*/5 * * * *", "UTC"))
        .await;
    let (old, ..) = t.trigger_of("changes").await;
    t.reconcile().await;
    let before = t.schedule_of(old).await.expect("schedule");

    t.w.publish_new_version(workflow, second_version("changes", "*/10 * * * *", "UTC"))
        .await;
    let (new, _, expression, _) = t.trigger_of("changes").await;
    assert_ne!(
        new, old,
        "feature 004: the id of a trigger is a hash of what it is about"
    );
    assert_eq!(expression, "*/10 * * * *");
    assert_eq!(t.read(&before).await.state, ScheduleState::Deleted);
    let now = t
        .schedule_of(new)
        .await
        .expect("the new trigger has its schedule");
    assert_ne!(now.id, before.id);
    assert_eq!(now.state, ScheduleState::Active);
    assert_eq!(now.spec.cron()[0].expression(), "*/10 * * * *");

    // and a changed zone is another trigger too
    t.w.publish_new_version(
        workflow,
        second_version("changes", "*/10 * * * *", "Europe/Lisbon"),
    )
    .await;
    let (zoned, ..) = t.trigger_of("changes").await;
    assert!(zoned != new && zoned != old);
    assert_eq!(t.read(&now).await.state, ScheduleState::Deleted);
}

#[tokio::test]
async fn a_trigger_that_comes_back_is_the_same_schedule_with_its_history() {
    let t = triggers().await;
    let (workflow, _) = t
        .publish(cron_workflow("returns", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("returns").await;
    t.reconcile().await;
    let first = t.schedule_of(trigger).await.expect("schedule");
    sqlx::query(
        "INSERT INTO schedule_actions (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, decided_at) \
         VALUES ($1, $2, 'SPEC', to_timestamp(3600), 'SCHEDULED', 1, 'SKIPPED_OVERLAP', now())",
    )
    .bind(first.tenant_id.as_uuid())
    .bind(first.id.as_uuid())
    .execute(&t.w.db.pool)
    .await
    .expect("history");

    t.w.publish_new_version(workflow, without_trigger("returns"))
        .await;
    assert_eq!(t.read(&first).await.state, ScheduleState::Deleted);
    t.w.publish_new_version(workflow, second_version("returns", "0 * * * *", "UTC"))
        .await;

    let (again, ..) = t.trigger_of("returns").await;
    assert_eq!(again, trigger, "the same trigger is the same id");
    let back = t.schedule_of(trigger).await.expect("schedule");
    assert_eq!(
        back.id, first.id,
        "and the same schedule: the unique index spans tombstones"
    );
    assert_eq!(back.state, ScheduleState::Active);
    assert_eq!(
        back.revision, 2,
        "the removal did not write a revision; the return wrote the next one"
    );
    assert_eq!(back.next_action_at, None, "it is planned again by the pass");
    let revisions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM schedule_revisions WHERE schedule_id = $1")
            .bind(first.id.as_uuid())
            .fetch_one(&t.w.db.pool)
            .await
            .expect("revisions");
    assert_eq!(revisions, 2);
    let history: i64 =
        sqlx::query_scalar("SELECT count(*) FROM schedule_actions WHERE schedule_id = $1")
            .bind(first.id.as_uuid())
            .fetch_one(&t.w.db.pool)
            .await
            .expect("history");
    assert_eq!(history, 1, "the history stayed");
    assert_eq!(t.listed().await.len(), 1);
    assert_eq!(t.reconcile().await, 1, "the pass plans its next time");
    assert!(t.read(&first).await.next_action_at.is_some());
}

#[tokio::test]
async fn trigger_owned_schedules_neither_count_against_nor_are_refused_by_the_schedules_quota() {
    let t = triggers().await;
    t.w.quotas
        .set(
            &t.w.admin,
            Some(t.w.namespace.id()),
            QuotaResource::Schedules,
            1,
            &ctx(),
        )
        .await
        .expect("limit");
    t.api_schedule("mine")
        .await
        .expect("the one schedule the quota allows");
    assert!(
        matches!(
            t.api_schedule("one-too-many").await,
            Err(AppError::QuotaExceeded(_))
        ),
        "the quota is full for the schedules of the API"
    );

    // publication is not refused for it, and makes a schedule each
    for name in ["trig-a", "trig-b", "trig-c"] {
        t.publish(cron_workflow(name, "0 * * * *", "UTC")).await;
        let (trigger, ..) = t.trigger_of(name).await;
        assert!(
            t.schedule_of(trigger).await.is_some(),
            "{name} has its schedule"
        );
    }
    assert_eq!(t.listed().await.len(), 3);
    assert_eq!(t.reconcile().await, 3, "their first times are planned");
    let used =
        t.w.store
            .usage(
                t.w.admin.tenant_id(),
                Some(t.w.namespace.id()),
                QuotaResource::Schedules,
            )
            .await
            .expect("usage");
    assert_eq!(used, 1, "only the schedule of the API is counted");
    // and the room for the next one of the API is what the quota leaves after the API's own, not after the triggers'
    t.w.quotas
        .set(
            &t.w.admin,
            Some(t.w.namespace.id()),
            QuotaResource::Schedules,
            2,
            &ctx(),
        )
        .await
        .expect("limit");
    t.api_schedule("room-for-one-more")
        .await
        .expect("three trigger schedules do not take the room of the second");
    assert!(
        matches!(
            t.api_schedule("and-no-more").await,
            Err(AppError::QuotaExceeded(_))
        ),
        "the quota of two is full of schedules of the API"
    );

    // and the reconciliation makes one for a trigger an older control plane published, quota full or not
    sqlx::query(
        "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, cron, timezone) \
         SELECT gen_random_uuid(), tenant_id, namespace_id, workflow_id, version_id, 'cron', '15 * * * *', 'UTC' \
         FROM workflow_triggers LIMIT 1",
    )
    .execute(&t.w.db.pool)
    .await
    .expect("an older control plane publishes");
    assert_eq!(t.reconcile().await, 1);
    assert_eq!(t.listed().await.len(), 4);
}

#[tokio::test]
async fn the_schedules_of_one_tenant_are_not_the_others() {
    let t = triggers().await;
    t.publish(cron_workflow("mine", "0 * * * *", "UTC")).await;
    // another tenant's workflow with a cron trigger
    let other = t.w.other_admin.clone();
    let doc = cron_workflow("theirs", "0 * * * *", "UTC");
    let bytes = serde_json::to_vec(&doc).expect("json");
    let theirs =
        t.w.defs
            .create_workflow(
                &other,
                "payments",
                "theirs",
                &bytes,
                &format!("k-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
            .expect("create");
    t.w.defs
        .publish(
            &other,
            theirs.workflow.id(),
            theirs.workflow.draft_revision(),
            None,
            &format!("p-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("publish");

    assert_eq!(
        t.listed().await.len(),
        1,
        "the first tenant sees its own trigger schedule only"
    );
    let (mine, ..) = t.trigger_of("mine").await;
    let theirs_trigger: Uuid = sqlx::query_scalar(
        "SELECT t.id FROM workflow_triggers t JOIN workflows w ON w.id = t.workflow_id WHERE w.name = 'theirs'",
    )
    .fetch_one(&t.w.db.pool)
    .await
    .expect("their trigger");
    let theirs_schedule = t.schedule_of(theirs_trigger).await;
    // read through the first tenant's store, it is not there; through its own tenant it is
    assert!(theirs_schedule.is_none(), "the first tenant cannot read it");
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM schedules WHERE trigger_id = $1 AND tenant_id = $2",
    )
    .bind(theirs_trigger)
    .bind(other.tenant_id().as_uuid())
    .fetch_one(&t.w.db.pool)
    .await
    .expect("count");
    assert_eq!(count, 1, "it is in its own tenant");
    assert!(t.schedule_of(mine).await.is_some());
}

// --- the control of a trigger-owned schedule waits for the handover -----------------------------------------------

#[tokio::test]
async fn pause_resume_trigger_and_backfill_are_refused_until_the_handover_and_a_spec_edit_always() {
    let t = triggers().await;
    t.publish(cron_workflow("control", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("control").await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    t.clock.set(OffsetDateTime::now_utc() + Duration::hours(3));

    let pending = |result: Result<(), AppError>, what: &str| {
        assert_eq!(
            result,
            Err(AppError::ScheduleConflict {
                reason: "SCHEDULE_TRIGGER_HANDOVER_PENDING"
            }),
            "{what} before the handover"
        );
    };
    let note = || Note::new("by an operator").expect("note");
    pending(
        t.service
            .pause(&t.w.admin, schedule.id, note(), &ctx())
            .await
            .map(|_| ()),
        "pause",
    );
    pending(
        t.service
            .resume(&t.w.admin, schedule.id, note(), &ctx())
            .await
            .map(|_| ()),
        "resume",
    );
    pending(
        t.firing
            .trigger(
                &t.w.admin,
                schedule.id,
                &TriggerRequest {
                    overlap: None,
                    key: "trigger-key-0001",
                },
                &ctx(),
            )
            .await
            .map(|_| ()),
        "trigger-now",
    );
    let backfill = sagat_application::schedules::BackfillRequest {
        start: t.clock.now() - Duration::hours(2),
        end: t.clock.now() - Duration::hours(1),
        overlap: None,
    };
    pending(
        t.service
            .create_backfill(&t.w.admin, schedule.id, &backfill, &ctx())
            .await
            .map(|_| ()),
        "backfill",
    );
    let edit = ScheduleWrite {
        name: ScheduleName::new("edited").expect("name"),
        spec: schedule.spec.clone(),
        policies: schedule.policies,
        target: schedule.target.clone(),
        input: json!({}),
        paused: false,
        note: None,
        remaining_actions: None,
    };
    let owned = Err(AppError::ScheduleConflict {
        reason: "SCHEDULE_TRIGGER_OWNED",
    });
    assert_eq!(
        t.service
            .update(&t.w.admin, schedule.id, edit, schedule.revision, &ctx())
            .await
            .map(|_| ()),
        owned
    );
    assert_eq!(
        t.service
            .delete(&t.w.admin, schedule.id, None, &ctx())
            .await,
        owned
    );
    assert_eq!(
        t.read(&schedule).await.state,
        ScheduleState::Active,
        "nothing changed"
    );

    t.hand_over().await;

    // after it, as any schedule
    let paused = t
        .service
        .pause(&t.w.admin, schedule.id, note(), &ctx())
        .await
        .expect("pause");
    assert_eq!(paused.state, ScheduleState::Paused);
    let resumed = t
        .service
        .resume(&t.w.admin, schedule.id, note(), &ctx())
        .await
        .expect("resume");
    assert_eq!(resumed.state, ScheduleState::Active);
    assert_eq!(
        resumed.authorized_by, "system:engine",
        "the authority of a trigger-owned schedule is the engine's whoever resumes it"
    );
    assert!(resumed.next_action_at.is_some());
    let action = t
        .firing
        .trigger(
            &t.w.admin,
            schedule.id,
            &TriggerRequest {
                overlap: None,
                key: "trigger-key-0002",
            },
            &ctx(),
        )
        .await
        .expect("trigger-now");
    assert_eq!(action.outcome, "STARTED");
    // its spec is still the trigger's to change: a spec edit and a delete stay refused
    let edit = ScheduleWrite {
        name: ScheduleName::new("edited").expect("name"),
        spec: schedule.spec.clone(),
        policies: schedule.policies,
        target: schedule.target.clone(),
        input: json!({}),
        paused: false,
        note: None,
        remaining_actions: None,
    };
    assert_eq!(
        t.service
            .update(&t.w.admin, schedule.id, edit, resumed.revision, &ctx())
            .await
            .map(|_| ()),
        owned
    );
    assert_eq!(
        t.service
            .delete(&t.w.admin, schedule.id, None, &ctx())
            .await,
        owned
    );
    // a trigger-now of a trigger-owned schedule starts the cron.fired document of 004 under the manual identity
    let run = action.run_id.expect("run");
    let input = input_of(&t, run).await;
    assert_eq!(input["event"]["type"], "cron.fired");
    assert_eq!(
        input["data"]["scheduledAt"], input["data"]["firedAt"],
        "asked for now"
    );
    let (scope, key): (String, String) =
        sqlx::query_as("SELECT scope, idempotency_key FROM command_dedupe WHERE resource_id = $1")
            .bind(run.to_string())
            .fetch_one(&t.w.db.pool)
            .await
            .expect("the key of the run");
    assert_eq!(
        scope,
        format!("schedule:{}", schedule.id),
        "manual identity of the schedule"
    );
    assert!(key.starts_with("manual-"), "{key}");
}

/// The document a run was started with.
async fn input_of(t: &Triggers, run: Uuid) -> Value {
    let bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM runs r JOIN payloads p ON p.tenant_id = r.tenant_id AND p.digest = r.input_digest WHERE r.id = $1",
    )
    .bind(run)
    .fetch_one(&t.w.db.pool)
    .await
    .expect("input");
    serde_json::from_slice(&bytes).expect("json")
}

#[tokio::test]
async fn a_backfill_of_a_trigger_owned_schedule_after_the_handover_starts_the_run_of_the_tick_once()
{
    let t = triggers().await;
    let (workflow, _) = t
        .publish(cron_workflow("backfilled", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("backfilled").await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    let now = OffsetDateTime::now_utc();
    let hour = now.unix_timestamp() - now.unix_timestamp() % 3_600;
    let start = OffsetDateTime::from_unix_timestamp(hour - 3 * 3_600).expect("instant");
    let end = OffsetDateTime::from_unix_timestamp(hour - 2 * 3_600).expect("instant");
    // the tick the legacy pass started before the handover is a run under the key of 004
    t.clock.set(now);
    sqlx::query("UPDATE workflow_triggers SET next_fire_at = $2 WHERE id = $1")
        .bind(trigger)
        .bind(start)
        .execute(&t.w.db.pool)
        .await
        .expect("due");
    t.w.dispatcher.tick().await;
    assert_eq!(
        t.runs_of(workflow).await.len(),
        1,
        "the legacy pass started the tick"
    );
    t.hand_over().await;

    let backfill = t
        .service
        .create_backfill(
            &t.w.admin,
            schedule.id,
            &sagat_application::schedules::BackfillRequest {
                start,
                end: end + Duration::seconds(1),
                overlap: Some(Overlap::AllowAll),
            },
            &ctx(),
        )
        .await
        .expect("backfill");
    let due = sagat_application::schedules::DueBackfill {
        tenant_id: schedule.tenant_id,
        schedule: schedule.id,
        id: backfill.id,
    };
    for _ in 0..5 {
        if t.firing.backfill_step(&due, None).await.expect("step") == Fired::Idle {
            break;
        }
    }
    assert_eq!(
        t.runs_of(workflow).await.len(),
        2,
        "the tick the legacy pass started is not started again; the other tick of the range is"
    );
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT idempotency_key FROM command_dedupe WHERE scope = $1 ORDER BY idempotency_key",
    )
    .bind(format!("trigger:{trigger}"))
    .fetch_all(&t.w.db.pool)
    .await
    .expect("keys");
    assert_eq!(
        keys.len(),
        2,
        "both under feature 004's scope and key: {keys:?}"
    );
    assert!(keys.iter().all(|k| k.starts_with("cron-")));
    let counters = t.read(&schedule).await;
    let _ = counters;
}
