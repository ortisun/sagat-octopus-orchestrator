//! The cost of a round of the reconciliation does not grow with the number of cron triggers (`T018-060`, D2072): a window of the
//! triggers is a range of an index, a sweep goes through all of them in windows, and the count the handover and the gauge
//! need is bounded by the statement timeout of four seconds. 20,000 triggers of 200 tenants, five of which have no schedule.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use sagat_application::schedule_triggers::{
    Drift, DriftWindow, ScanCursor, TriggerScheduleStore as _,
};
use uuid::Uuid;

use crate::support::trigger_schedules::{all_shards, cron_workflow, triggers};

const TRIGGERS: i64 = 20_000;

/// Writes `TRIGGERS` cron triggers over 200 tenants, and a schedule equal to each but every four-thousandth (those five are what the
/// sweep has to find). The rows have no tenant, workflow or version behind them, so the foreign keys are switched off for the
/// connection that writes them; nothing reads anything but the triggers and the schedules.
async fn seed(pool: &sqlx::PgPool) {
    let seeding = Instant::now();
    // the per-row guards and counters of the three tables are what makes a bulk write slow (a hundred seconds for what the
    // statements below do in a few); nothing writes after the seeding, and the database is this test's own
    for table in ["workflow_triggers", "schedules", "schedule_revisions"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {table} DISABLE TRIGGER USER"
        )))
        .execute(pool)
        .await
        .expect("disable the triggers of the table");
    }
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query("SET session_replication_role = replica")
        .execute(&mut *conn)
        .await
        .expect("replica role");
    sqlx::query(
        "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, cron, timezone) \
         SELECT gen_random_uuid(), md5((n % 200)::text)::uuid, md5('namespace')::uuid, md5('workflow')::uuid, \
                md5('version')::uuid, 'cron', (n % 60)::text || ' * * * *', 'UTC' \
           FROM generate_series(1, $1) AS n",
    )
    .bind(TRIGGERS)
    .execute(&mut *conn)
    .await
    .expect("the triggers");
    println!("seed: triggers at {:?}", seeding.elapsed());
    sqlx::query(
        "INSERT INTO schedules (tenant_id, namespace_id, id, name, origin, trigger_id, state, head_revision, state_version, \
                                authorized_by, next_action_at, next_actual_at, created_at, updated_at) \
         SELECT s.tenant_id, s.namespace_id, gen_random_uuid(), 'trigger-' || replace(s.id::text, '-', ''), 'TRIGGER', s.id, \
                'ACTIVE', 1, 1, 'system:engine', now() + interval '1 hour', now() + interval '1 hour', now(), now() \
           FROM (SELECT t.*, row_number() OVER (ORDER BY t.id) AS rn FROM workflow_triggers t \
                  WHERE t.kind = 'cron' AND NOT EXISTS (SELECT 1 FROM schedules x WHERE x.trigger_id = t.id)) s \
          WHERE s.rn % 4000 <> 0",
    )
    .execute(&mut *conn)
    .await
    .expect("the schedules");
    println!("seed: schedules at {:?}", seeding.elapsed());
    sqlx::query(
        "INSERT INTO schedule_revisions (tenant_id, schedule_id, revision, spec, policies, action, input_digest, actor, request_id, created_at) \
         SELECT s.tenant_id, s.id, 1, jsonb_build_object('cron', jsonb_build_array(t.cron), 'timezone', t.timezone), \
                '{}'::jsonb, '{}'::jsonb, NULL, 'system:engine', NULL, now() \
           FROM schedules s JOIN workflow_triggers t ON t.id = s.trigger_id \
          WHERE NOT EXISTS (SELECT 1 FROM schedule_revisions r WHERE r.tenant_id = s.tenant_id AND r.schedule_id = s.id)",
    )
    .execute(&mut *conn)
    .await
    .expect("the revisions");
    println!("seed: revisions at {:?}", seeding.elapsed());
    sqlx::query("RESET session_replication_role")
        .execute(&mut *conn)
        .await
        .expect("reset");
    drop(conn);
    for table in ["workflow_triggers", "schedules", "schedule_revisions"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("ANALYZE {table}")))
            .execute(pool)
            .await
            .expect("analyze");
    }
}

#[tokio::test]
async fn a_round_costs_the_window_it_reads_and_a_sweep_finds_every_drift_over_twenty_thousand_triggers()
 {
    let t = triggers().await;
    t.publish(cron_workflow("scale-seed", "0 * * * *", "UTC"))
        .await;
    t.reconcile().await;
    let seeding = Instant::now();
    seed(&t.w.db.pool).await;
    let seeded = seeding.elapsed();

    // the count the handover and the gauge use: one statement over every shard, well inside the statement timeout of four seconds
    let started = Instant::now();
    let counted = t.store.convergence().await.expect("convergence");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the count took {:?}",
        started.elapsed()
    );
    assert_eq!(counted.unconverged, 5, "the five triggers with no schedule");
    let counting = started.elapsed();

    // a sweep: windows of two thousand triggers, each well inside a second, together every trigger once
    let mut after: Option<ScanCursor> = None;
    let mut found = BTreeSet::new();
    let mut pages = 0_u32;
    let mut slowest = Duration::ZERO;
    loop {
        let started = Instant::now();
        let page = t
            .store
            .drift(&DriftWindow {
                shards: all_shards(),
                exclude: Vec::new(),
                per_tenant: 20,
                limit: 200,
                scan: 2_000,
                after,
            })
            .await
            .expect("a window");
        slowest = slowest.max(started.elapsed());
        pages += 1;
        found.extend(page.drift.iter().map(Drift::trigger));
        after = page.next;
        if !page.more {
            break;
        }
        assert!(pages < 200, "the sweep does not end");
    }
    assert!(
        pages >= 10,
        "twenty thousand triggers are read in windows of two thousand, not in one: {pages} windows"
    );
    assert!(
        slowest < Duration::from_secs(1),
        "a window took {slowest:?}"
    );
    assert_eq!(
        found.len(),
        5,
        "the sweep found exactly the five: {found:?}"
    );
    assert_eq!(
        after, None,
        "and ended at the last trigger, ready to start over"
    );
    // what the report and ADR-0083 quote: the machine and the load are the reader's to state
    println!(
        "{TRIGGERS} triggers: seeding {seeded:?}; the count {counting:?}; {pages} windows of 2,000, the slowest {slowest:?}"
    );
    let _ = Uuid::nil();
}

#[tokio::test]
async fn a_window_that_the_limit_cut_short_stays_where_it_is() {
    let t = triggers().await;
    for n in 0..4 {
        t.publish(cron_workflow(&format!("stay-{n}"), "0 * * * *", "UTC"))
            .await;
    }
    // four triggers with a schedule that is not planned (publication makes it so), a limit of two
    let cut = t
        .store
        .drift(&DriftWindow {
            shards: all_shards(),
            exclude: Vec::new(),
            per_tenant: 10,
            limit: 2,
            scan: 1_000,
            after: None,
        })
        .await
        .expect("a window");
    assert_eq!(cut.drift.len(), 2);
    assert_eq!(
        cut.next, None,
        "the window stays at the start (nothing was written, the same two are found again)"
    );
    assert!(cut.more, "and the loop goes on at once");
    // with the room for all of them the window is consumed and the sweep ends
    let all = t
        .store
        .drift(&DriftWindow {
            shards: all_shards(),
            exclude: Vec::new(),
            per_tenant: 10,
            limit: 10,
            scan: 1_000,
            after: None,
        })
        .await
        .expect("a window");
    assert_eq!(all.drift.len(), 4);
    assert!(!all.more);
}
