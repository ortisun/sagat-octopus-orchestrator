//! The released legacy cron pass: v0.2.0's own dispatcher, wired as the v0.2.0 engine wires it (`with_cron(CronCalendar)`),
//! fires a tick before the upgrade and then stays up, idle, across the current migration, ticking only when the coordinator
//! asks, so that it and a current engine reach the same ticks in a known order (`T018-061`).
use super::inflight::control;
use crate::support::engine::{activity, document, world};
use serde_json::{Map, json};
use std::time::Duration;

/// The workflows of the scenario, one daily cron trigger each (midnight UTC: a real tick never comes due while the scenario runs, but at midnight): the tick after the upgrade, the released pass first, the
/// current pass first, both processes at once, and the tick after the handover.
const WORKFLOWS: [&str; 5] = [
    "cron-upgrade-default",
    "cron-upgrade-old-first",
    "cron-upgrade-new-first",
    "cron-upgrade-race",
    "cron-upgrade-handover",
];

#[tokio::test]
async fn fire_a_cron_tick_then_tick_on_request_across_the_upgrade() {
    assert_eq!(env!("SAGAT_RELEASE_PROFILE_COMMIT"), super::RELEASE);
    let mut stream = control::inherited_stream();
    let w =
        crate::support::bounded::within("released world", Duration::from_secs(60), world()).await;
    let mut triggers = Map::new();
    for name in WORKFLOWS {
        let mut doc = document(name, "work", vec![activity("work")]);
        doc["triggers"] = json!([{"kind": "cron", "cron": "0 0 * * *", "timezone": "UTC"}]);
        w.publish(doc).await;
        let (trigger, workflow): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
            "SELECT t.id, t.workflow_id FROM workflow_triggers t JOIN workflows f ON f.id = t.workflow_id \
             WHERE f.name = $1 AND t.kind = 'cron'",
        )
        .bind(name)
        .fetch_one(&w.db.pool)
        .await
        .expect("the released cron trigger");
        triggers.insert(
            name.to_owned(),
            json!({"trigger": trigger.to_string(), "workflow": workflow.to_string()}),
        );
    }
    // The released pass's first look plans every trigger and starts nothing.
    assert_eq!(w.dispatcher.tick().await.runs_started, 0);
    let unplanned: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workflow_triggers WHERE kind = 'cron' AND (next_fire_at IS NULL OR next_fire_at <= clock_timestamp())",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("planned triggers");
    assert_eq!(
        unplanned, 0,
        "the released pass planned the next midnight of every trigger"
    );
    // The midnight before today's is due for the first workflow only: the time is moved, not waited for.
    let today: i64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM date_trunc('day', clock_timestamp() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC')::bigint",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("today's midnight");
    let first = today - 86_400;
    let default_trigger: uuid::Uuid = triggers[WORKFLOWS[0]]["trigger"]
        .as_str()
        .expect("trigger")
        .parse()
        .expect("UUID");
    sqlx::query("UPDATE workflow_triggers SET next_fire_at = to_timestamp($2) WHERE id = $1")
        .bind(default_trigger)
        .bind(first as f64)
        .execute(&w.db.pool)
        .await
        .expect("the first tick is due");
    assert_eq!(
        w.dispatcher.tick().await.runs_started,
        1,
        "the released legacy pass fires the due tick"
    );
    let max_migration: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&w.db.pool)
            .await
            .expect("released schema boundary");
    control::send(
        &mut stream,
        &json!({
            "releaseCommit": super::RELEASE,
            "database": w.db.name,
            "tenant": w.admin.tenant_id().to_string(),
            "maxMigration": max_migration,
            "today": today,
            "first": first,
            "triggers": triggers,
        }),
    )
    .await;
    // Idle, connected, until asked: a tick of the released pass on whatever schema the database has now.
    crate::support::bounded::within(
        "released process commands",
        Duration::from_secs(280),
        async {
            loop {
                let command = control::receive(&mut stream).await;
                if command["stop"] == true {
                    break;
                }
                assert_eq!(command["tick"], true, "a known command");
                let round = w.dispatcher.tick().await;
                control::send(&mut stream, &json!({"runsStarted": round.runs_started})).await;
            }
        },
    )
    .await;
    drop(w);
    control::send(&mut stream, &json!({"stopped": true})).await;
}
