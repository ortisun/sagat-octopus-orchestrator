//! Cron triggers (FR-004-001): a schedule starts a run at its time, once, however the dispatchers
//! that share it are interrupted or raced.

use sagat_application::error::AppError;
use sagat_application::quotas::QuotaLookup as _;
use sagat_domain::execution::RunId;
use sagat_domain::quota::QuotaResource;
use serde_json::{Value, json};

use crate::support::engine::{World, activity, ctx, document, world};

fn scheduled(name: &str, cron: &str, timezone: &str) -> Value {
    let mut doc = document(name, "work", vec![activity("work")]);
    doc["triggers"] = json!([{"kind": "cron", "cron": cron, "timezone": timezone}]);
    doc
}

async fn runs_of(w: &World, name: &str) -> Vec<RunId> {
    sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT r.id FROM runs r JOIN workflows f ON f.id = r.workflow_id WHERE f.name = $1 ORDER BY r.id",
    )
    .bind(name)
    .fetch_all(&w.db.pool)
    .await
    .expect("runs")
    .into_iter()
    .map(RunId::new)
    .collect()
}

async fn next_fire(w: &World) -> Option<f64> {
    sqlx::query_scalar(
        "SELECT extract(epoch FROM next_fire_at - clock_timestamp())::float8 FROM workflow_triggers",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("next fire")
}

async fn due_since(w: &World, interval: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE workflow_triggers SET next_fire_at = clock_timestamp() - interval '{interval}'"
    )))
    .execute(&w.db.pool)
    .await
    .expect("due");
}

#[tokio::test]
async fn a_published_schedule_is_planned_without_firing_and_then_fires_at_its_time_once() {
    let w = world().await;
    w.publish(scheduled("every-five", "*/5 * * * *", "UTC"))
        .await;
    assert_eq!(
        next_fire(&w).await,
        None,
        "a trigger nobody has looked at has no time yet"
    );

    // The first look plans the next time and starts nothing.
    let round = w.dispatcher.tick().await;
    assert_eq!(round.runs_started, 0);
    let wait = next_fire(&w).await.expect("planned");
    assert!(
        (0.0..=300.5).contains(&wait),
        "within the next five minutes: {wait}"
    );
    assert!(runs_of(&w, "every-five").await.is_empty());

    // Its time comes: one run, started by the platform, given what fired it.
    due_since(&w, "1 second").await;
    let round = w.dispatcher.tick().await;
    assert_eq!(round.runs_started, 1);
    let runs = runs_of(&w, "every-five").await;
    assert_eq!(runs.len(), 1);
    let input: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM runs r JOIN payloads p ON p.tenant_id = r.tenant_id AND p.digest = r.input_digest WHERE r.id = $1",
    )
    .bind(runs[0].as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("input");
    let input: Value = serde_json::from_slice(&input).expect("json");
    assert_eq!(input["event"]["source"], "cron");
    assert_eq!(input["event"]["type"], "cron.fired");
    assert!(input["data"]["scheduledAt"].is_string() && input["data"]["firedAt"].is_string());
    let started_by: String = sqlx::query_scalar("SELECT started_by FROM runs WHERE id = $1")
        .bind(runs[0].as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("started by");
    assert_eq!(started_by, "system:engine");

    // And the schedule moved on: nothing more until the next time.
    let wait = next_fire(&w).await.expect("moved");
    assert!(wait > 0.0, "the next time is in the future: {wait}");
    assert!(!w.dispatcher.tick().await.did_work());
    assert_eq!(runs_of(&w, "every-five").await.len(), 1);
}

#[tokio::test]
async fn a_dispatcher_that_died_between_firing_and_moving_the_schedule_starts_nothing_twice() {
    let w = world().await;
    w.publish(scheduled("hourly", "0 * * * *", "UTC")).await;
    w.dispatcher.tick().await;
    due_since(&w, "1 minute").await;
    let scheduled_for: time::OffsetDateTime =
        sqlx::query_scalar("SELECT next_fire_at FROM workflow_triggers")
            .fetch_one(&w.db.pool)
            .await
            .expect("time");
    assert_eq!(w.dispatcher.tick().await.runs_started, 1);

    // The schedule is put back as it was when the dispatcher died: the same firing is due again.
    sqlx::query("UPDATE workflow_triggers SET next_fire_at = $1")
        .bind(scheduled_for)
        .execute(&w.db.pool)
        .await
        .expect("rewind");
    let round = w.dispatcher.tick().await;
    assert_eq!(
        round.runs_started, 0,
        "the firing is known, so nothing starts"
    );
    assert_eq!(runs_of(&w, "hourly").await.len(), 1);
    assert!(
        next_fire(&w).await.expect("moved") > 0.0,
        "but the schedule moves on"
    );
}

#[tokio::test]
async fn two_dispatchers_that_find_a_trigger_due_together_start_one_run() {
    let w = world().await;
    w.publish(scheduled("busy", "* * * * *", "UTC")).await;
    w.dispatcher.tick().await;
    due_since(&w, "1 second").await;
    let other = w.dispatcher_with(sagat_scheduler::EventDispatchSettings::default());
    let (a, b) = tokio::join!(w.dispatcher.tick(), other.tick());
    assert_eq!(a.runs_started + b.runs_started, 1);
    assert_eq!(runs_of(&w, "busy").await.len(), 1);
}

#[tokio::test]
async fn a_schedule_missed_while_nobody_was_looking_fires_once_and_not_for_every_time_it_missed() {
    let w = world().await;
    w.publish(scheduled("every-minute", "* * * * *", "UTC"))
        .await;
    w.dispatcher.tick().await;
    due_since(&w, "3 hours").await;
    assert_eq!(w.dispatcher.tick().await.runs_started, 1);
    assert_eq!(
        runs_of(&w, "every-minute").await.len(),
        1,
        "one run, not 180"
    );
    let wait = next_fire(&w).await.expect("moved");
    assert!((0.0..=60.5).contains(&wait), "continues from now: {wait}");
}

#[tokio::test]
async fn a_firing_that_cannot_start_is_tried_again_later_and_does_not_hammer() {
    let w = world().await;
    w.publish(scheduled("blocked", "* * * * *", "UTC")).await;
    w.dispatcher.tick().await;
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            0,
            &ctx(),
        )
        .await
        .expect("limit");
    assert_eq!(
        w.store
            .effective_limit(
                w.admin.tenant_id(),
                Some(w.namespace.id()),
                QuotaResource::ActiveRuns
            )
            .await,
        Ok(0)
    );
    due_since(&w, "1 second").await;
    assert_eq!(w.dispatcher.tick().await.runs_started, 0);
    let wait: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM retry_at - clock_timestamp())::float8 FROM workflow_triggers",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("retry planned");
    assert!(
        (20.0..=31.0).contains(&wait),
        "a retry in about 30 seconds, not at once: {wait}"
    );
    assert!(
        !w.dispatcher.tick().await.did_work(),
        "and not again before then"
    );

    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            100,
            &ctx(),
        )
        .await
        .expect("limit");
    sqlx::query("UPDATE workflow_triggers SET retry_at = clock_timestamp()")
        .execute(&w.db.pool)
        .await
        .expect("time passes");
    assert_eq!(w.dispatcher.tick().await.runs_started, 1);
}

#[tokio::test]
async fn republishing_keeps_the_schedule_and_a_new_one_is_planned() {
    let w = world().await;
    let (workflow, _) = w
        .publish(scheduled("keeps-time", "*/10 * * * *", "UTC"))
        .await;
    w.dispatcher.tick().await;
    let planned: time::OffsetDateTime =
        sqlx::query_scalar("SELECT next_fire_at FROM workflow_triggers")
            .fetch_one(&w.db.pool)
            .await
            .expect("planned");
    let id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM workflow_triggers")
        .fetch_one(&w.db.pool)
        .await
        .expect("id");

    // A new version with the same schedule is the same trigger, with the same next time.
    let mut again = scheduled("keeps-time", "*/10 * * * *", "UTC");
    again["nodes"][0]["retry"] = json!({"maxAttempts": 2});
    w.publish_new_version(workflow, again).await;
    let (same, still): (uuid::Uuid, time::OffsetDateTime) =
        sqlx::query_as("SELECT id, next_fire_at FROM workflow_triggers")
            .fetch_one(&w.db.pool)
            .await
            .expect("row");
    assert_eq!((same, still), (id, planned));

    // Another schedule is another trigger, planned on its own.
    w.publish_new_version(
        workflow,
        scheduled("keeps-time", "0 3 * * *", "America/Sao_Paulo"),
    )
    .await;
    let rows: Vec<(uuid::Uuid, Option<time::OffsetDateTime>)> =
        sqlx::query_as("SELECT id, next_fire_at FROM workflow_triggers")
            .fetch_all(&w.db.pool)
            .await
            .expect("rows");
    assert_eq!(rows.len(), 1, "the old schedule is gone");
    assert_ne!(rows[0].0, id);
    assert_eq!(
        rows[0].1, None,
        "and the new one is not planned until it is looked at"
    );
}

#[tokio::test]
async fn a_schedule_that_means_nothing_is_refused_when_the_workflow_is_published() {
    let w = world().await;
    for (cron, zone) in [
        ("0 0 31 2 *", "UTC"),
        ("0 9 * * *", "Mars/Olympus_Mons"),
        ("*/0 * * * *", "UTC"),
        ("0 9 * *", "UTC"),
        ("0 0 9 * * *", "UTC"),
        ("@hourly", "UTC"),
    ] {
        let refused = w
            .try_publish_as(&w.admin, scheduled("nonsense", cron, zone))
            .await;
        assert!(
            matches!(refused, Err(AppError::InvalidDefinition(_))),
            "{cron:?} in {zone:?}: {refused:?}"
        );
        // The workflow was created before the publish was tried; free its name for the next try.
        sqlx::query("DELETE FROM workflows WHERE name = 'nonsense'")
            .execute(&w.db.pool)
            .await
            .expect("clean");
    }
}

/// A clock that is wrong by a fixed amount.
struct Ahead(time::Duration);

impl sagat_application::clock::Clock for Ahead {
    fn now(&self) -> time::OffsetDateTime {
        time::OffsetDateTime::now_utc() + self.0
    }
}

#[tokio::test]
async fn a_replica_whose_clock_is_ahead_of_the_databases_does_not_skip_a_tick() {
    use std::sync::Arc;
    let w = world().await;
    w.publish(scheduled("every-five", "*/5 * * * *", "UTC"))
        .await;
    w.dispatcher.tick().await;
    due_since(&w, "1 second").await;

    // Ten minutes ahead of the database that says when the trigger is due.
    let ahead = sagat_scheduler::EventDispatcher::new(
        w.store.clone(),
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        w.cache.clone(),
        sagat_scheduler::EventDispatchSettings::default(),
    )
    .with_cron(Arc::new(sagat_infra_ir::cron::CronCalendar))
    .with_clock(Arc::new(Ahead(time::Duration::minutes(10))));
    assert_eq!(ahead.tick().await.runs_started, 1);

    // The next time is the first one after the database's now, not after the replica's: the tick
    // that falls in the next five minutes is not given up.
    let wait = next_fire(&w).await.expect("moved");
    assert!(
        (0.0..=300.5).contains(&wait),
        "the next tick is within five minutes of the database's clock: {wait}"
    );
}

#[tokio::test]
async fn a_tick_that_could_not_start_is_tried_again_under_the_same_key_so_it_can_never_start_twice()
{
    let w = world().await;
    w.publish(scheduled("hourly", "0 * * * *", "UTC")).await;
    w.dispatcher.tick().await;
    due_since(&w, "1 minute").await;
    let tick: time::OffsetDateTime =
        sqlx::query_scalar("SELECT next_fire_at FROM workflow_triggers")
            .fetch_one(&w.db.pool)
            .await
            .expect("time");

    // No room for a run: the start fails.
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            0,
            &ctx(),
        )
        .await
        .expect("limit");
    assert_eq!(w.dispatcher.tick().await.runs_started, 0);
    assert!(runs_of(&w, "hourly").await.is_empty());
    let (still, retry_in): (time::OffsetDateTime, Option<f64>) = sqlx::query_as(
        "SELECT next_fire_at, extract(epoch FROM retry_at - clock_timestamp())::float8 \
         FROM workflow_triggers",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("row");
    assert_eq!(still, tick, "it is still the same tick that is owed");
    let retry_in = retry_in.expect("a time to try again");
    assert!(
        (1.0..=31.0).contains(&retry_in),
        "about half a minute: {retry_in}"
    );
    assert!(
        !w.dispatcher.tick().await.did_work(),
        "and not before that time"
    );

    // Room appears and the time comes: the tick starts its run, under the key of the tick.
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            100,
            &ctx(),
        )
        .await
        .expect("limit");
    sqlx::query("UPDATE workflow_triggers SET retry_at = clock_timestamp()")
        .execute(&w.db.pool)
        .await
        .expect("time passes");
    assert_eq!(w.dispatcher.tick().await.runs_started, 1);
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT idempotency_key FROM command_dedupe WHERE idempotency_key LIKE 'cron-%'",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("keys");
    assert_eq!(keys, [format!("cron-{}", tick.unix_timestamp())]);
    let (next, retry): (time::OffsetDateTime, Option<time::OffsetDateTime>) =
        sqlx::query_as("SELECT next_fire_at, retry_at FROM workflow_triggers")
            .fetch_one(&w.db.pool)
            .await
            .expect("row");
    assert!(
        next > tick && retry.is_none(),
        "the schedule moved on and forgot the retry"
    );
}
