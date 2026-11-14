//! The costs of the trigger handover that the reviews estimated and nobody measured (G2023: 100,000 triggers; T018-060, ADR-0083):
//! the convergence count the handover and the gauge use, the windows of the drift read a reconciliation round makes, the drift view,
//! and what a backlog of triggers without a schedule costs to work through.
//!
//! Two worlds for each size:
//!
//! * **steady**: every cron trigger has a schedule equal to it but one in four thousand (the state after a reconciliation that
//!   has worked). The triggers, their schedules and their revisions are written by bulk `INSERT ... SELECT` over `generate_series`
//!   as the owner, with the foreign keys and the user triggers of the three tables off for the session that writes (the per-row
//!   guards and counters make a bulk write minutes long; nothing writes after the seeding and the statements measured are reads).
//!   Tenants are 200 invented identifiers, as in `trigger_scale.rs`.
//! * **after an upgrade**: the triggers exist and no schedule does (an upgrade that leaves the reconciliation all its work). The
//!   triggers belong to the two real tenants of the world (their namespaces, workflows and versions) so that the reconciliation can
//!   really write the schedules; they are written by one `INSERT ... SELECT` from the trigger a real publication made, with the
//!   guards of the table on (`workflow_triggers` keeps its quota counters). The engine's own loop (`loops::run_triggers`) then works
//!   through them for a fixed time, under its own pacing (a pause after a round that wrote, at 100 writes a second), and the writes
//!   a second it achieved are what is reported (the estimate of G2023 was about 17 minutes for 100,000).
//!
//! Bounds asserted: every statement stays within the 4 s statement timeout the code sets on it; the count stays within 2 s and a
//! window of the drift within 1 s (the bounds of `trigger_scale.rs`, which measured 20,000); the reconciliation under its default
//! settings writes at least half the 100 a second it advertises (`TriggerRound::WRITES_PER_SECOND`) when its round is full; no
//! write fails or is refused.
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use sagat_application::clock::SystemClock;
use sagat_application::schedule_triggers::{
    Drift, DriftWindow, ScanCursor, TriggerReconciler, TriggerScheduleStore as _,
};
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::{DatabaseClock, PgScheduleStore};
use sagat_scheduler::{
    Keeper, KeeperSettings, Placement, TriggerPass, TriggerRound, TriggerSettings,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::watch;
use uuid::Uuid;

use crate::harness::{Bench, RoundStats, bench};
use crate::measure::{Summary, Verdict, conclude, environment, load_now, ms, summary_json};
use crate::support::bounded::within;
use crate::support::engine::{activity, document};

const STATEMENT_TIMEOUT_MS: f64 = 4_000.0;
const SAMPLES: usize = 20;

fn all_shards() -> Vec<sagat_domain::sharding::Shard> {
    sagat_domain::sharding::Shard::all().collect()
}

fn cron_workflow(name: &str) -> Value {
    let mut doc = document(name, "work", vec![activity("work")]);
    doc["triggers"] = json!([{"kind": "cron", "cron": "0 * * * *", "timezone": "UTC"}]);
    doc
}

/// `triggers` cron triggers over 200 invented tenants, a schedule equal to each but every four-thousandth, as `trigger_scale.rs`.
async fn seed_steady(pool: &PgPool, triggers: usize) -> Duration {
    let seeding = Instant::now();
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
    .bind(i64::try_from(triggers).expect("small"))
    .execute(&mut *conn)
    .await
    .expect("the triggers");
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
    seeding.elapsed()
}

/// `triggers` cron triggers of the two real tenants, no schedule: each tenant's share is copied from the trigger its own
/// publication made, so that the namespace, the workflow and the version are real.
async fn seed_after_upgrade(b: &Bench, triggers: usize) -> Duration {
    let seeding = Instant::now();
    b.w.publish(cron_workflow("bench-cron-a")).await;
    b.w.publish_as_other_tenant(cron_workflow("bench-cron-b"))
        .await;
    for (tenant, share) in [
        (b.w.admin.tenant_id().as_uuid(), triggers - triggers / 2),
        (b.w.other_admin.tenant_id().as_uuid(), triggers / 2),
    ] {
        sqlx::query(
            "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, cron, timezone) \
             SELECT gen_random_uuid(), t.tenant_id, t.namespace_id, t.workflow_id, t.version_id, 'cron', \
                    (n % 60)::text || ' * * * *', 'UTC' \
               FROM generate_series(1, $2) AS n, \
                    (SELECT * FROM workflow_triggers WHERE tenant_id = $1 AND kind = 'cron' ORDER BY id LIMIT 1) t",
        )
        .bind(tenant)
        .bind(i64::try_from(share).expect("small"))
        .execute(&b.w.db.pool)
        .await
        .expect("the triggers");
    }
    sqlx::query("ANALYZE workflow_triggers")
        .execute(&b.w.db.pool)
        .await
        .expect("analyze");
    seeding.elapsed()
}

/// The cron triggers that have no schedule, counted without the cap of 100,000 the product's count has (an owner's statement that
/// the benchmark runs to check what the rounds wrote, outside what it measures).
async fn unconverged_exactly(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM workflow_triggers t WHERE t.kind = 'cron' \
           AND NOT EXISTS (SELECT 1 FROM schedules s WHERE s.trigger_id = t.id)",
    )
    .fetch_one(pool)
    .await
    .expect("count the triggers without a schedule")
}

/// A statement, asked `SAMPLES` times: milliseconds each.
async fn timed<T, F, Fut>(mut statement: F) -> (Vec<f64>, T)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let mut samples = Vec::new();
    let mut last = None;
    for _ in 0..SAMPLES {
        let started = Instant::now();
        last = Some(statement().await);
        samples.push(ms(started.elapsed()));
    }
    (samples, last.expect("a sample"))
}

fn window(after: Option<ScanCursor>, settings: &TriggerSettings) -> DriftWindow {
    DriftWindow {
        shards: all_shards(),
        exclude: Vec::new(),
        per_tenant: settings.per_tenant,
        limit: settings.batch,
        scan: settings.scan,
        after,
    }
}

/// A whole sweep, in windows of the reconciliation's own size, every trigger once: the milliseconds of each window, the triggers
/// the sweep found drifting, and the seconds the sweep took.
async fn sweep(
    store: &PgScheduleStore,
    settings: &TriggerSettings,
) -> (Vec<f64>, std::collections::BTreeSet<Uuid>, f64) {
    let mut after: Option<ScanCursor> = None;
    let (mut found, mut windows_ms) = (std::collections::BTreeSet::new(), Vec::new());
    let began = Instant::now();
    loop {
        let started = Instant::now();
        let page = store
            .drift(&window(after, settings))
            .await
            .expect("a window");
        windows_ms.push(ms(started.elapsed()));
        found.extend(page.drift.iter().map(Drift::trigger));
        after = page.next;
        if !page.more {
            break;
        }
        assert!(windows_ms.len() < 100_000, "the sweep does not end");
    }
    (windows_ms, found, began.elapsed().as_secs_f64())
}

/// Gives a schedule to every cron trigger that has none, by the bulk statements of `seed_steady`: the state the handover needs.
async fn converge_the_stragglers(pool: &PgPool) {
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query("SET session_replication_role = replica")
        .execute(&mut *conn)
        .await
        .expect("replica role");
    sqlx::query(
        "INSERT INTO schedules (tenant_id, namespace_id, id, name, origin, trigger_id, state, head_revision, state_version, \
                                authorized_by, next_action_at, next_actual_at, created_at, updated_at) \
         SELECT t.tenant_id, t.namespace_id, gen_random_uuid(), 'trigger-' || replace(t.id::text, '-', ''), 'TRIGGER', t.id, \
                'ACTIVE', 1, 1, 'system:engine', now() + interval '1 hour', now() + interval '1 hour', now(), now() \
           FROM workflow_triggers t \
          WHERE t.kind = 'cron' AND NOT EXISTS (SELECT 1 FROM schedules x WHERE x.trigger_id = t.id)",
    )
    .execute(&mut *conn)
    .await
    .expect("the schedules");
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
    sqlx::query("RESET session_replication_role")
        .execute(&mut *conn)
        .await
        .expect("reset");
    drop(conn);
    for table in ["schedules", "schedule_revisions"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("ANALYZE {table}")))
            .execute(pool)
            .await
            .expect("analyze");
    }
}

/// The steady state: the statements of the handover and of a sweep, at `triggers` triggers.
async fn steady(scenario: &str, triggers: usize) {
    let before = load_now();
    let b = bench().await;
    let environment = environment(&b.w.db.pool).await;
    let seeded_in = seed_steady(&b.w.db.pool, triggers).await;
    let store = PgScheduleStore::new(b.w.execution_pool.clone());
    let settings = TriggerSettings::default();
    // the schedule insert leaves out every four-thousandth trigger
    let expected = u32::try_from(triggers / 4_000).expect("small");

    // the count the handover and the gauge use
    let (count_ms, counted) =
        timed(|| async { store.convergence().await.expect("convergence") }).await;
    assert_eq!(
        counted.unconverged, expected,
        "the triggers that have no schedule"
    );

    // the check the handover makes under its lock (EXISTS, stops at the first row)
    let (exists_ms, found): (Vec<f64>, bool) = timed(|| async {
        sqlx::query_scalar("SELECT schedule_trigger_unconverged()")
            .fetch_one(&b.w.execution_pool)
            .await
            .expect("unconverged")
    })
    .await;
    assert!(found || expected == 0);

    // the drift view as an operator would read it: unbounded, not a statement of the product (no timeout is set on it here)
    let view_samples = 5;
    let mut view_ms = Vec::new();
    for _ in 0..view_samples {
        let started = Instant::now();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM schedule_trigger_drift")
            .fetch_one(&b.w.db.pool)
            .await
            .expect("the view");
        view_ms.push(ms(started.elapsed()));
        assert_eq!(n, i64::from(expected));
    }

    // a whole sweep: windows of the reconciliation's own size, every trigger once
    let (windows_ms, found, sweep_s) = sweep(&store, &settings).await;
    assert_eq!(
        found.len(),
        usize::try_from(expected).expect("small"),
        "the sweep found exactly what drifts"
    );

    // the state the handover needs: every cron trigger has a schedule equal to it. The check under the lock then has to look at
    // every trigger and find none (EXISTS stops at the first row only when there is one)
    converge_the_stragglers(&b.w.db.pool).await;
    let (converged_exists_ms, still): (Vec<f64>, bool) = {
        let mut samples = Vec::new();
        let mut last = true;
        for _ in 0..5 {
            let started = Instant::now();
            last = within(
                "the handover check on a converged fleet",
                Duration::from_secs(600),
                async {
                    sqlx::query_scalar("SELECT schedule_trigger_unconverged()")
                        .fetch_one(&b.w.execution_pool)
                        .await
                        .expect("unconverged")
                },
            )
            .await;
            samples.push(ms(started.elapsed()));
        }
        (samples, last)
    };
    assert!(!still, "every trigger has a schedule now");
    let (converged_count_ms, converged_count) =
        timed(|| async { store.convergence().await.expect("convergence") }).await;
    assert_eq!(converged_count.unconverged, 0);
    let (converged_windows_ms, converged_found, converged_sweep_s) = sweep(&store, &settings).await;
    assert!(converged_found.is_empty(), "nothing drifts");
    let plan: Vec<String> = within(
        "the plan of the view under EXISTS",
        Duration::from_secs(600),
        async {
            sqlx::query_scalar(
                "EXPLAIN (ANALYZE, COSTS OFF, TIMING OFF) SELECT EXISTS (SELECT 1 FROM schedule_trigger_drift)",
            )
            .fetch_all(&b.w.execution_pool)
            .await
            .expect("the plan")
        },
    )
    .await;

    let slowest = windows_ms.iter().copied().fold(0.0, f64::max);
    let count = Summary::of(&count_ms).expect("samples");
    let fragment = json!({
        "triggers": triggers, "state": "steady (converged but one in four thousand)", "tenants": 200,
        "seeded_in_s": seeded_in.as_secs_f64(), "unconverged": counted.unconverged,
        "convergence_ms": summary_json(&count_ms),
        "unconverged_exists_ms": summary_json(&exists_ms),
        "drift_view_count_ms": summary_json(&view_ms),
        "window": {"scan": settings.scan, "limit": settings.batch, "per_tenant": settings.per_tenant},
        "sweep": {"windows": windows_ms.len(), "window_ms": summary_json(&windows_ms), "total_s": sweep_s,
                  "triggers_per_s": triggers as f64 / sweep_s,
                  "at_the_pause_of_a_sweep_s": windows_ms.len() as f64 * TriggerRound::SWEEP_PAUSE.as_secs_f64() + sweep_s},
        "converged": {
            "handover_check_exists_ms": summary_json(&converged_exists_ms),
            "convergence_ms": summary_json(&converged_count_ms),
            "sweep": {"windows": converged_windows_ms.len(), "window_ms": summary_json(&converged_windows_ms),
                      "total_s": converged_sweep_s},
            "plan_of_the_view_under_exists": plan.iter().take(60).collect::<Vec<_>>(),
        },
        "statement_timeout_ms": STATEMENT_TIMEOUT_MS,
    });
    let verdicts = [
        Verdict::at_most(
            "the_handover_check_on_a_converged_fleet_stays_within_the_statement_timeout",
            "slowest EXISTS check (what the handover asks under its lock) when every trigger has a schedule",
            converged_exists_ms.iter().copied().fold(0.0, f64::max),
            STATEMENT_TIMEOUT_MS,
            "ms",
        ),
        Verdict::at_most(
            "the_count_stays_within_the_statement_timeout",
            "slowest convergence count",
            count_ms.iter().copied().fold(0.0, f64::max),
            STATEMENT_TIMEOUT_MS,
            "ms",
        ),
        Verdict::at_most(
            "the_count_stays_within_two_seconds",
            "p95 of the count",
            count.p95,
            2_000.0,
            "ms",
        ),
        Verdict::at_most(
            "a_window_stays_within_a_second",
            "slowest window of the sweep",
            slowest,
            1_000.0,
            "ms",
        ),
        Verdict::at_most(
            "a_window_stays_within_the_statement_timeout",
            "slowest window of the sweep",
            slowest,
            STATEMENT_TIMEOUT_MS,
            "ms",
        ),
    ];
    println!(
        "{scenario}: {triggers} triggers: count p50 {:.1} ms p95 {:.1} ms; {} windows, slowest {slowest:.1} ms, sweep {sweep_s:.1} s",
        count.p50,
        count.p95,
        windows_ms.len()
    );
    conclude(scenario, environment, before, fragment, &verdicts);
}

type Pass = TriggerPass<PgScheduleStore, SpecCalendar, DatabaseClock<SystemClock>>;

struct Engine {
    clock: DatabaseClock<SystemClock>,
    pass: Arc<Pass>,
}

async fn engine(b: &Bench, settings: TriggerSettings) -> Engine {
    let replica = b.w.replica_with_pool(16).await;
    let clock = DatabaseClock::new(replica.execution_pool.clone(), SystemClock);
    clock.sync().await.expect("the database clock");
    let placement = Placement::new(format!("engine-{}", Uuid::now_v7()));
    Keeper::new(
        replica.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(6 * 3_600),
        },
    )
    .round()
    .await
    .expect("keeper round");
    let reconciler = TriggerReconciler::new(
        PgScheduleStore::new(replica.execution_pool.clone()),
        SpecCalendar,
        clock.clone(),
    );
    Engine {
        clock,
        pass: Arc::new(TriggerPass::new(reconciler, placement, settings)),
    }
}

/// The engine's own loop (`loops::run_triggers`) for `budget`: what it wrote, and how long its rounds took.
async fn work_for(engine: &Engine, budget: Duration) -> (Arc<RoundStats>, f64) {
    let (stop, receiver) = watch::channel(false);
    let stats = Arc::new(RoundStats::default());
    let (clock, pass, recorded) = (
        engine.clock.clone(),
        Arc::clone(&engine.pass),
        Arc::clone(&stats),
    );
    let began = Instant::now();
    let task = tokio::spawn(async move {
        sagat_execution_engine::loops::run_triggers(
            || {
                let (clock, pass, recorded) =
                    (clock.clone(), Arc::clone(&pass), Arc::clone(&recorded));
                async move {
                    let started = Instant::now();
                    let round = sagat_execution_engine::loops::trigger_round(&clock, &pass).await;
                    recorded.record(
                        round.writes(),
                        round.refused,
                        round.failed,
                        started.elapsed(),
                    );
                    round
                }
            },
            sagat_execution_engine::loops::TRIGGER_POLL,
            receiver,
        )
        .await;
    });
    tokio::time::sleep(budget).await;
    let _ = stop.send(true);
    task.await.expect("the loop ends");
    (stats, began.elapsed().as_secs_f64())
}

/// After an upgrade: no schedule exists for `triggers` triggers. The statements of the handover on that state, and the reconciliation at work.
async fn after_upgrade(scenario: &str, triggers: usize, budget: Duration) {
    let before = load_now();
    let b = bench().await;
    let environment = environment(&b.w.db.pool).await;
    let seeded_in = seed_after_upgrade(&b, triggers).await;
    let store = PgScheduleStore::new(b.w.execution_pool.clone());
    let defaults = TriggerSettings::default();
    let total = i64::try_from(triggers).expect("small");

    let (count_ms, counted) =
        timed(|| async { store.convergence().await.expect("convergence") }).await;
    // a publication of each tenant made one trigger of its own: those two are triggers too
    assert!(
        i64::from(counted.unconverged) >= total.min(100_000),
        "everything is unconverged ({} of {total})",
        counted.unconverged
    );
    let (first_window_ms, page) = timed(|| async {
        store
            .drift(&window(None, &defaults))
            .await
            .expect("a window")
    })
    .await;
    assert_eq!(
        page.drift.len(),
        usize::try_from(defaults.batch.min(2 * defaults.per_tenant)).expect("small")
    );
    let (exists_ms, _): (Vec<f64>, bool) = timed(|| async {
        sqlx::query_scalar("SELECT schedule_trigger_unconverged()")
            .fetch_one(&b.w.execution_pool)
            .await
            .expect("unconverged")
    })
    .await;

    // the reconciliation at work, twice: under its default settings (the tenant cap of 20 binds with two tenants), and with
    // the tenant cap raised so that a round fills its batch of 200
    let mut variants = Vec::new();
    let mut verdicts = vec![
        Verdict::at_most(
            "the_count_stays_within_the_statement_timeout",
            "slowest convergence count of a backlog",
            count_ms.iter().copied().fold(0.0, f64::max),
            STATEMENT_TIMEOUT_MS,
            "ms",
        ),
        Verdict::at_most(
            "a_window_stays_within_the_statement_timeout",
            "slowest first window of a backlog",
            first_window_ms.iter().copied().fold(0.0, f64::max),
            STATEMENT_TIMEOUT_MS,
            "ms",
        ),
    ];
    let mut failed_total = 0;
    for (label, settings) in [
        ("defaults (batch 200, 20 a tenant, window 2,000)", defaults),
        (
            "tenant cap 100 (a round fills its batch of 200)",
            TriggerSettings {
                per_tenant: 100,
                ..defaults
            },
        ),
    ] {
        // the engine of the previous variant has stopped but still holds its shards (a lease lasts hours): let them lapse, as they
        // would for an engine that died, so that the next one takes them
        sqlx::query(
            "UPDATE scheduler_shards SET expires_at = clock_timestamp() - interval '1 second'",
        )
        .execute(&b.w.db.pool)
        .await
        .expect("let the shards lapse");
        let engine = engine(&b, settings).await;
        let unconverged_before = store.convergence().await.expect("convergence").unconverged;
        let exact_before = unconverged_exactly(&b.w.db.pool).await;
        let (stats, wall) = work_for(&engine, budget).await;
        let unconverged_after = store.convergence().await.expect("convergence").unconverged;
        let exact_after = unconverged_exactly(&b.w.db.pool).await;
        let written = stats.steps.load(std::sync::atomic::Ordering::Relaxed);
        let rate = written as f64 / wall;
        failed_total += stats.failed.load(std::sync::atomic::Ordering::Relaxed)
            + stats.refused.load(std::sync::atomic::Ordering::Relaxed);
        let schedules: i64 =
            sqlx::query_scalar("SELECT count(*) FROM schedules WHERE origin = 'TRIGGER'")
                .fetch_one(&b.w.db.pool)
                .await
                .expect("count");
        let drain_hours = f64::from(unconverged_before.min(100_000)) / rate / 3_600.0;
        variants.push(json!({
            "settings": label, "budget_s": wall, "unconverged_before": unconverged_before,
            "unconverged_exactly_before": exact_before, "unconverged_exactly_after": exact_after,
            "unconverged_after": unconverged_after, "schedules_made": written, "schedules_in_the_table": schedules,
            "writes_per_second": rate, "rounds": stats.json(),
            "drain_of_the_backlog_at_this_rate_minutes": f64::from(unconverged_before) / rate / 60.0,
            "drain_hours_note": drain_hours,
        }));
        if settings.per_tenant == 100 {
            verdicts.push(Verdict::at_least(
                "a_full_round_keeps_half_the_advertised_pace",
                "schedules written a second by a reconciliation whose rounds are full",
                rate,
                0.5 * TriggerRound::WRITES_PER_SECOND as f64,
                "writes/s",
            ));
        }
        // Every trigger that lost its lack of a schedule was written by a round. A round may also write what is not a creation: the
        // two triggers a real publication made for the two tenants have schedules that a round repairs, so the writes may exceed
        // what the count lost by up to two. (The count the product reads stops at 100,000, so the exact one is used here.)
        let lost = exact_before - exact_after;
        let written_i64 = i64::try_from(written).expect("small");
        assert!(
            (lost..=lost + 2).contains(&written_i64),
            "{written_i64} writes for {lost} triggers that got a schedule"
        );
    }
    verdicts.push(Verdict::at_most(
        "no_write_fails_or_is_refused",
        "writes that failed or were refused",
        failed_total as f64,
        0.0,
        "writes",
    ));
    let fragment = json!({
        "triggers": triggers, "state": "after an upgrade (no schedule for any trigger)", "tenants": 2,
        "seeded_in_s": seeded_in.as_secs_f64(), "unconverged": counted.unconverged,
        "convergence_ms": summary_json(&count_ms), "unconverged_exists_ms": summary_json(&exists_ms),
        "first_window_ms": summary_json(&first_window_ms),
        "variants": variants, "statement_timeout_ms": STATEMENT_TIMEOUT_MS,
        "g2023_estimate": "about 17 minutes for 100,000 triggers of one tenant at 20 entries a round",
    });
    println!(
        "{scenario}: {triggers} triggers: {} ",
        serde_json::to_string(&fragment["variants"]).expect("json")
    );
    conclude(scenario, environment, before, fragment, &verdicts);
}

pub async fn steady_state(scenario: &str, sizes: &[usize]) {
    for size in sizes {
        steady(&format!("{scenario}-{size}"), *size).await;
    }
}

pub async fn backlog(scenario: &str, sizes: &[usize], budget: Duration) {
    for size in sizes {
        after_upgrade(&format!("{scenario}-{size}"), *size, budget).await;
    }
}
