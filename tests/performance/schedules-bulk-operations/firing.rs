//! Firing lag (NFR-018-005, T018-017): N schedules due at the same instant, or spread over a minute, the schedule pass of one
//! engine replica (or of two on the same database) running in the engine's own loop, real runs started through the run service.
//!
//! Lag is read three ways, so that no number rests on the process that is measured: what the product measures about itself
//! (`schedule_action_lag_seconds`, recorded in the process), what the database says (`schedule_actions.decided_at` less the
//! nominal time; `runs.created_at` is the same instant) and what an outside observer sees (a separate connection that asks, every
//! 100 ms, for how many schedules have a run it can see). The gauge the alert reads
//! (`scheduler_lag_seconds{pass="schedules"}`) is sampled once a second, as the monitor of an engine does.
//!
//! The schedules are one-minute intervals whose offset within the minute places each (the same instant for all, or spread over a
//! minute), with one action each, so that a burst is one time of every schedule and not a load that goes on every minute; the
//! first time is a whole minute 10 to 70 s after the start, by the clock of the database, and the passes are up and idle before it.
//!
//! Bounds asserted (the spec promises none, NFR-018-005): the p95 of the lag stays within 60 s, the threshold of
//! `SagatSchedulerLate`, and the gauge does not stay above 60 s for the 5 minutes that alert waits; every schedule starts one run;
//! no step is refused or fails.
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use sagat_scheduler::ScheduleSettings;
use serde_json::json;

use crate::harness::{
    Observer, Sampler, ScheduleReplica, bench, capture, create_schedules, make_due,
    share_the_shards,
};
use crate::measure::{Summary, Verdict, conclude, environment, load_now, ms, summary_json};
use crate::support::bounded::within;

/// The alert `SagatSchedulerLate`: `max(scheduler_lag_seconds) > 60` for 5 minutes (`infra/observability/alerts.yaml`).
const ALERT_THRESHOLD_SECONDS: f64 = 60.0;
const ALERT_FOR_SECONDS: f64 = 300.0;
/// How long before the nominal time the passes are idle and waiting: the engines are up before a time comes.
const LEAD_SECONDS: f64 = 10.0;
const SAMPLE_EVERY: Duration = Duration::from_secs(1);

fn settings() -> ScheduleSettings {
    let defaults = ScheduleSettings::default();
    ScheduleSettings {
        batch: std::env::var("SAGAT_PERF_SCHEDULES_BATCH")
            .ok()
            .map_or(defaults.batch, |value| value.parse().expect("a number")),
        round_actions: std::env::var("SAGAT_PERF_SCHEDULES_ROUND_ACTIONS")
            .ok()
            .map_or(defaults.round_actions, |value| {
                value.parse().expect("a number")
            }),
        ..defaults
    }
}

/// The seconds the gauge stayed above the threshold, and the longest stretch of it, from samples taken `every` apart.
fn above(samples: &[(f64, f64, f64)], threshold: f64, every: Duration) -> (f64, f64) {
    let (mut total, mut streak, mut longest) = (0.0, 0.0, 0.0_f64);
    for (_, schedules, _) in samples {
        if *schedules > threshold {
            total += every.as_secs_f64();
            streak += every.as_secs_f64();
            longest = longest.max(streak);
        } else {
            streak = 0.0;
        }
    }
    (total, longest)
}

/// One run of the scenario: `n` schedules, due over `spread` seconds (zero: all at the same instant), `replicas` engines.
pub async fn run(scenario: &str, n: usize, spread: f64, replicas: usize) {
    let before = load_now();
    let b = bench().await;
    let environment = environment(&b.w.db.pool).await;
    let spread_ms = (spread * 1_000.0) as u64;
    let created_in = create_schedules(&b, n, spread_ms).await;
    let settings = settings();
    let mut engines = Vec::new();
    for index in 0..replicas {
        engines.push(Arc::new(
            ScheduleReplica::new(&b.w, &format!("engine-{index}"), settings).await,
        ));
    }
    share_the_shards(&engines.iter().map(AsRef::as_ref).collect::<Vec<_>>()).await;
    let product = capture();
    drop(product.take("schedule_action_lag_seconds"));

    let first = make_due(&b.w, LEAD_SECONDS).await;
    let sampler = Sampler::spawn(&b.w, SAMPLE_EVERY);
    let observer = Observer::spawn(&b.w, Duration::from_millis(100));
    let began = Instant::now();
    let running: Vec<_> = engines
        .iter()
        .map(|engine| engine.spawn(Duration::from_millis(1_000)))
        .collect();
    let limit = Duration::from_secs(60 * 30);
    within(
        format!("every one of {n} schedules acts ({scenario})"),
        limit,
        async {
            let mut told = Instant::now();
            // a pass that has stopped making progress is a finding, not a reason to wait for the limit of the stage: the first
            // time is due LEAD_SECONDS to a minute and a half from the start, and nothing may then stand still for three minutes
            let (mut last_count, mut last_change) = (-1_i64, Instant::now());
            loop {
                let acted: i64 =
                    sqlx::query_scalar("SELECT count(DISTINCT schedule_id) FROM schedule_actions")
                        .fetch_one(&b.w.db.pool)
                        .await
                        .expect("count the actions");
                if usize::try_from(acted).expect("small") >= n {
                    break;
                }
                if acted != last_count {
                    (last_count, last_change) = (acted, Instant::now());
                }
                assert!(
                    last_change.elapsed() < Duration::from_secs(180),
                    "{scenario}: no schedule has acted for three minutes: {acted} of {n}; passes: {:?}",
                    running.iter().map(|r| r.stats.json()).collect::<Vec<_>>()
                );
                if told.elapsed() >= Duration::from_secs(10) {
                    told = Instant::now();
                    println!(
                        "{scenario}: {acted} of {n} acted after {:.0} s; passes: {:?}",
                        began.elapsed().as_secs_f64(),
                        running.iter().map(|r| r.stats.json()).collect::<Vec<_>>()
                    );
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        },
    )
    .await;
    let wall = began.elapsed();
    let mut stats = Vec::new();
    for loop_ in running {
        stats.push(loop_.stop().await);
    }
    let samples = sampler.stop().await;
    let seen = observer.stop().await;

    // the invariants: the first time of every schedule started one run, once (a schedule's second time is a minute after its
    // first and may start when the burst is over: those are counted apart and are not part of the lag)
    let (started, distinct, first_runs, later): (i64, i64, i64, i64) = sqlx::query_as(
        "WITH first AS (SELECT DISTINCT ON (schedule_id) * FROM schedule_actions ORDER BY schedule_id, nominal_at) \
         SELECT (SELECT count(*) FROM first WHERE outcome = 'STARTED'), \
                (SELECT count(DISTINCT schedule_id) FROM first), \
                (SELECT count(DISTINCT run_id) FROM first), \
                (SELECT count(*) FROM schedule_actions) - (SELECT count(*) FROM first)",
    )
    .fetch_one(&b.w.db.pool)
    .await
    .expect("the outcomes");
    let n_i64 = i64::try_from(n).expect("small");
    assert_eq!(
        (started, distinct, first_runs),
        (n_i64, n_i64, n_i64),
        "the first time of every schedule started one run"
    );

    // what the actions after the first of each schedule are (a schedule that the pass reaches more than a minute after its time has
    // two times due and records them in one step), and how many runs the schedules started
    let by_outcome: Vec<(String, i64)> = sqlx::query_as(
        "SELECT outcome, count(*) FROM schedule_actions GROUP BY outcome ORDER BY outcome",
    )
    .fetch_all(&b.w.db.pool)
    .await
    .expect("the outcomes");
    let runs_of_schedules: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE schedule_id IS NOT NULL")
            .fetch_one(&b.w.db.pool)
            .await
            .expect("the runs");

    let rows: Vec<(f64, f64, f64, f64)> = sqlx::query_as(
        "WITH first AS (SELECT DISTINCT ON (schedule_id) * FROM schedule_actions ORDER BY schedule_id, nominal_at) \
         SELECT extract(epoch FROM a.decided_at - a.nominal_at)::float8, \
                extract(epoch FROM r.created_at - a.nominal_at)::float8, \
                extract(epoch FROM a.decided_at)::float8, \
                extract(epoch FROM a.nominal_at)::float8 \
           FROM first a JOIN runs r ON r.tenant_id = a.tenant_id AND r.id = a.run_id \
          WHERE a.outcome = 'STARTED'",
    )
    .fetch_all(&b.w.db.pool)
    .await
    .expect("the lags");
    let decided_lag: Vec<f64> = rows.iter().map(|row| row.0).collect();
    let observed_lag: Vec<f64> = rows.iter().map(|row| row.1).collect();
    let decided_at: Vec<f64> = rows.iter().map(|row| row.2).collect();
    let nominal_at: Vec<f64> = rows.iter().map(|row| row.3).collect();
    let product_lag = product.take("schedule_action_lag_seconds");
    let last_decision = decided_at.iter().copied().fold(f64::MIN, f64::max);
    let first_decision = decided_at.iter().copied().fold(f64::MAX, f64::min);
    let first_nominal = nominal_at.iter().copied().fold(f64::MAX, f64::min);
    let last_nominal = nominal_at.iter().copied().fold(f64::MIN, f64::max);
    let burst_rate = n as f64 / (last_decision - first_decision).max(0.001);
    let behind = last_decision - last_nominal;

    let gauge: Vec<f64> = samples.iter().map(|sample| sample.1).collect();
    let (seconds_above, longest_above) = above(&samples, ALERT_THRESHOLD_SECONDS, SAMPLE_EVERY);
    let gauge_max = gauge.iter().copied().fold(0.0, f64::max);
    let decided = Summary::of(&decided_lag).expect("lags");

    let refused: u64 = stats
        .iter()
        .map(|s| s.refused.load(std::sync::atomic::Ordering::Relaxed))
        .sum();
    let failed: u64 = stats
        .iter()
        .map(|s| s.failed.load(std::sync::atomic::Ordering::Relaxed))
        .sum();

    let fragment = json!({
        "schedules": n,
        "arrival": if spread == 0.0 { "same instant".to_owned() } else { format!("spread over {spread} s") },
        "replicas": replicas,
        "settings": {"batch": settings.batch, "round_actions": settings.round_actions,
                     "steps_per_schedule": settings.steps_per_schedule, "poll_ms": 1000,
                     "pool_per_replica": 16, "lead_seconds": LEAD_SECONDS,
                     "schedule": "one-minute interval with an offset within the minute, one action each (remaining actions 1), overlap ALLOW_ALL",
                     "active_runs_quota": "raised to 1,000,000 (the default is 10,000 a namespace)"},
        "created_in_s": created_in.as_secs_f64(),
        "wall_s": wall.as_secs_f64(),
        "nominal_window_s": last_nominal - first_nominal,
        "first_nominal_at_unix": first.unix_timestamp(),
        "lag_s_database": summary_json(&decided_lag),
        "lag_s_observer": summary_json(&observed_lag),
        "outside_s_after_the_first_nominal_time_until_this_share_had_a_visible_run": {
            "50%": visible_after(&seen, n, 0.50, first_nominal),
            "95%": visible_after(&seen, n, 0.95, first_nominal),
            "100%": visible_after(&seen, n, 1.0, first_nominal),
        },
        "lag_s_product_histogram": summary_json(&product_lag),
        "product_histogram_samples": product_lag.len(),
        "actions_after_the_first_of_each_schedule": later,
        "actions_by_outcome": by_outcome.iter().map(|(outcome, n)| (outcome.clone(), json!(n))).collect::<serde_json::Map<_, _>>(),
        "runs_started_by_the_schedules": runs_of_schedules,
        "actions_per_second_burst": burst_rate,
        "seconds_behind_after_the_last_nominal_time": behind,
        "gauge_schedules_s": summary_json(&gauge),
        "gauge_samples": samples.len(),
        "gauge_max_s": gauge_max,
        "gauge_seconds_above_60": seconds_above,
        "gauge_longest_stretch_above_60": longest_above,
        "alert_would_fire": longest_above >= ALERT_FOR_SECONDS,
        "rounds": stats.iter().map(|s| s.json()).collect::<Vec<_>>(),
        "round_ms_of_one_pass_note": "work_round_ms is the duration of a round that started runs",
    });
    let verdicts = [
        Verdict::at_most(
            "p95_lag_within_the_alert_threshold",
            "p95 of the lag between the nominal time and the decision",
            percentile95(&decided_lag),
            ALERT_THRESHOLD_SECONDS,
            "s",
        ),
        Verdict::at_most(
            "gauge_does_not_stay_above_the_alert_threshold_for_five_minutes",
            "longest stretch of scheduler_lag_seconds{pass=\"schedules\"} above 60 s",
            longest_above,
            ALERT_FOR_SECONDS - 1.0,
            "s",
        ),
        Verdict::at_most(
            "nothing_refused_or_failed",
            "steps refused or failed",
            (refused + failed) as f64,
            0.0,
            "steps",
        ),
    ];
    println!(
        "{scenario}: {n} schedules, {replicas} replica(s), lag p50 {:.2} s p95 {:.2} s max {:.2} s, burst {burst_rate:.1} actions/s, \
         wall {:.1} s, took {:.0} ms to look at the last",
        decided.p50,
        decided.p95,
        decided.max,
        wall.as_secs_f64(),
        ms(Duration::from_secs_f64(behind.max(0.0))),
    );
    conclude(scenario, environment, before, fragment, &verdicts);
}

fn percentile95(values: &[f64]) -> f64 {
    Summary::of(values).map_or(0.0, |summary| summary.p95)
}

/// Seconds from `nominal` (unix seconds) until the observer first saw runs of `share` of the `n` schedules; `null` if never.
fn visible_after(seen: &[(f64, i64)], n: usize, share: f64, nominal: f64) -> serde_json::Value {
    let needed = (n as f64 * share).ceil() as i64;
    seen.iter()
        .find(|(_, count)| *count >= needed)
        .map_or(serde_json::Value::Null, |(at, _)| json!(at - nominal))
}
