//! What the schedule and batch passes count (`T018-017`, `FR-018-044`): a decision once it is
//! committed, an item once its outcome is, how late a time was acted on, and the gauges of the
//! monitor. The labels are closed sets and none is a tenant, a schedule, a job or a run. A recorder of
//! the test's own, on a runtime of one thread, so that what other tests count in this process does not
//! reach the numbers read here. Real PostgreSQL.
#![allow(clippy::expect_used, clippy::panic)]

use std::future::Future;
use std::time::Duration as StdDuration;

use sagat_application::batch_executor::Stepped;
use sagat_application::identity::RequestContext;
use sagat_application::schedule_firing::Fired;
use sagat_application::schedules::ShardFence;
use sagat_application::shards::Lease;
use sagat_domain::batch::{Action, JobState};
use sagat_domain::sharding::Shard;
use sagat_scheduler::{Monitor, MonitorSettings};
use sagat_telemetry::Metrics;
use serde_json::json;
use time::Duration;

use crate::batch_executor::{
    create, due, everything, executor, job, live, runs_started, service, write,
};
use crate::firing::{Fixture, fixture, hourly};

/// What `body` counts, as the Prometheus page: a recorder of its own on a runtime of one thread.
pub(crate) fn recorded<F: Future<Output = ()>>(body: impl FnOnce() -> F) -> String {
    let (metrics, recorder) = Metrics::detached(StdDuration::from_secs(60));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    metrics::with_local_recorder(&recorder, || runtime.block_on(body()));
    metrics.render()
}

/// The value of the series of `name` whose labels are exactly `labels`, on the rendered page.
pub(crate) fn series(page: &str, name: &str, labels: &[(&str, &str)]) -> Option<f64> {
    page.lines()
        .filter(|line| line.starts_with(name) && line[name.len()..].starts_with('{'))
        .find(|line| {
            let inside = &line[name.len() + 1..line.rfind('}').unwrap_or(0)];
            let found: Vec<&str> = inside.split(',').collect();
            found.len() == labels.len()
                && labels
                    .iter()
                    .all(|(key, value)| found.contains(&format!("{key}=\"{value}\"").as_str()))
        })
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|value| value.parse().ok())
}

/// How long a step may be asked again for a job that has not moved. A selecting job waits for the oldest
/// open transaction of the whole server, not of its database, so a test running beside this one can hold
/// it back for a while: the bound is a time, not a number of steps (50 or 400 of them pass in a few
/// milliseconds, as in the CI run that failed `254170ce`'s two waits).
const GIVE_UP_AFTER: StdDuration = StdDuration::from_secs(120);

/// Steps the job, a second at a time, until it has the state `until`.
async fn drive(f: &Fixture, id: uuid::Uuid, until: JobState) {
    let exec = executor(f);
    let record = job(f, id).await;
    let give_up = std::time::Instant::now() + GIVE_UP_AFTER;
    loop {
        if exec.step(&due(&record), None).await.expect("step") != Stepped::Acted {
            f.clock.advance(Duration::seconds(1));
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }
        if job(f, id).await.state == until {
            return;
        }
        assert!(
            std::time::Instant::now() < give_up,
            "the job never reached {until:?}"
        );
    }
}

/// No label on a series of the passes is anything but a closed set: no tenant, schedule, job or run.
fn assert_labels_are_closed(page: &str) {
    for line in page.lines().filter(|line| {
        [
            "schedule_actions_total",
            "batch_items_total",
            "schedule_action_lag",
            "batch_item_duration",
        ]
        .iter()
        .any(|name| line.starts_with(name))
    }) {
        let keys: Vec<&str> = line
            [line.find('{').map_or(0, |i| i + 1)..line.rfind('}').unwrap_or(0)]
            .split(',')
            .filter_map(|pair| pair.split('=').next())
            .collect();
        assert!(
            keys.iter()
                .all(|key| ["kind", "outcome", "action", "le"].contains(key)),
            "an open label on `{line}`"
        );
    }
}

#[test]
fn decisions_items_and_lag_are_counted_once_committed_with_closed_labels() {
    let page = recorded(|| async {
        let f = fixture().await;

        // A time that came 90 s ago starts a run; the next, while that run is open, is skipped.
        let schedule = f.create("counted", &hourly(), &json!({}), None).await;
        let first = f
            .read(&schedule)
            .await
            .next_actual_at
            .expect("a first time");
        f.clock.set(first + Duration::seconds(90));
        assert_eq!(f.fire(&schedule).await, Fired::Acted);
        f.clock.advance(Duration::hours(1));
        assert_eq!(f.fire(&schedule).await, Fired::Acted);

        // A job over every run cancels the two it finds, one item each.
        runs_started(&f, 1).await;
        live(&f);
        let started = create(&f, write(Action::Cancel, everything()), "counted-job").await;
        drive(&f, started.id, JobState::Completed).await;

        // A time that came two minutes ago and has not been acted on is the schedule pass's lag.
        sqlx::query(
            "UPDATE schedules SET next_actual_at = clock_timestamp() - interval '120 seconds'",
        )
        .execute(&f.w.db.pool)
        .await
        .expect("age the schedule");
        let round = Monitor::new(f.w.store.clone(), MonitorSettings::default())
            .run_once()
            .await;
        assert!(!round.passive, "the monitor holds its lease");
    });

    let counted = |kind: &str, outcome: &str| {
        series(
            &page,
            "schedule_actions_total",
            &[("kind", kind), ("outcome", outcome)],
        )
    };
    assert_eq!(counted("scheduled", "started"), Some(1.0), "{page}");
    assert_eq!(counted("scheduled", "skipped_overlap"), Some(1.0), "{page}");

    // The lag of the time that was acted on: 90 s after it came, once.
    let lag = |suffix: &str| {
        series(
            &page,
            &format!("schedule_action_lag_seconds_{suffix}"),
            &[("kind", "scheduled")],
        )
    };
    assert_eq!(lag("count"), Some(1.0), "{page}");
    assert!(
        lag("sum").is_some_and(|sum| (sum - 90.0).abs() < 1e-6),
        "{page}"
    );

    // Two runs, two items, each timed.
    let items = series(
        &page,
        "batch_items_total",
        &[("action", "cancel"), ("outcome", "succeeded")],
    );
    assert_eq!(items, Some(2.0), "{page}");
    assert_eq!(
        series(
            &page,
            "batch_item_duration_seconds_count",
            &[("action", "cancel")]
        ),
        Some(2.0),
        "{page}"
    );
    assert!(
        series(
            &page,
            "batch_item_duration_seconds_sum",
            &[("action", "cancel")]
        )
        .is_some_and(|sum| sum > 0.0),
        "an item took some time: {page}"
    );

    // The monitor's gauges: the schedule pass is two minutes late, the batch pass is not, and no
    // job is active.
    assert!(
        series(&page, "scheduler_lag_seconds", &[("pass", "schedules")])
            .is_some_and(|lag| (119.0..300.0).contains(&lag)),
        "{page}"
    );
    assert_eq!(
        series(&page, "scheduler_lag_seconds", &[("pass", "batches")]),
        Some(0.0),
        "{page}"
    );
    let active = page
        .lines()
        .find_map(|line| line.strip_prefix("batch_jobs_active "))
        .and_then(|value| value.trim().parse::<f64>().ok());
    assert_eq!(active, Some(0.0), "{page}");

    assert_labels_are_closed(&page);
}

#[test]
fn the_items_a_cancelled_job_marks_are_counted_as_cancelled_and_the_one_that_ran_as_succeeded() {
    let page = recorded(|| async {
        let f = fixture().await;
        runs_started(&f, 4).await;
        live(&f);
        let job_ = create(&f, write(Action::Cancel, everything()), "counted-cancelled").await;
        // the selection, then one item
        let exec = executor(&f);
        let give_up = std::time::Instant::now() + GIVE_UP_AFTER;
        loop {
            exec.step(&due(&job_), None).await.expect("step");
            if job(&f, job_.id).await.counts.succeeded >= 1 {
                break;
            }
            assert!(
                std::time::Instant::now() < give_up,
                "the first item never ran"
            );
            f.clock.advance(Duration::seconds(1));
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }
        assert_eq!(job(&f, job_.id).await.counts.succeeded, 1);
        service(&f)
            .cancel(
                &f.w.admin,
                job_.id,
                "stop this batch job",
                &RequestContext::default(),
            )
            .await
            .expect("cancel");
        f.clock.advance(Duration::seconds(1));
        drive(&f, job_.id, JobState::Cancelled).await;
    });

    let count = |outcome: &str| {
        series(
            &page,
            "batch_items_total",
            &[("action", "cancel"), ("outcome", outcome)],
        )
    };
    assert_eq!(count("succeeded"), Some(1.0), "{page}");
    assert_eq!(count("cancelled"), Some(3.0), "{page}");
    assert_labels_are_closed(&page);
}

#[test]
fn a_decision_the_fence_refused_counts_nothing() {
    let page = recorded(|| async {
        let f = fixture().await;
        let schedule = f.create("fenced", &hourly(), &json!({}), None).await;
        let shard = Shard::of(schedule.id.as_uuid());
        sqlx::query(
            "UPDATE scheduler_shards SET owner = 'replica-b', generation = 4, \
             expires_at = clock_timestamp() + interval '1 hour' WHERE shard = $1",
        )
        .bind(i16::from(shard.index()))
        .execute(&f.w.db.pool)
        .await
        .expect("replica b holds the shard");
        let fence = |owner: &str, generation: u64| ShardFence {
            owner: owner.to_owned(),
            lease: Lease { shard, generation },
        };
        let due = f.due(&schedule);

        // The holder starts the run of the first time.
        f.clock.advance(Duration::hours(1) + Duration::seconds(1));
        assert_eq!(
            f.firing
                .fire(&due, Some(fence("replica-b", 4)))
                .await
                .expect("holder"),
            Fired::Acted
        );
        // The replica that lost the shard decides to skip the next time, while that run is open,
        // and is refused where it commits: nothing was decided.
        f.clock.advance(Duration::hours(1));
        let stale = f
            .firing
            .fire(&due, Some(fence("replica-a", 3)))
            .await
            .expect("stale");
        assert!(matches!(stale, Fired::Refused(_)), "{stale:?}");
        // The holder decides it.
        assert_eq!(
            f.firing
                .fire(&due, Some(fence("replica-b", 4)))
                .await
                .expect("holder again"),
            Fired::Acted
        );
    });

    let counted = |outcome: &str| {
        series(
            &page,
            "schedule_actions_total",
            &[("kind", "scheduled"), ("outcome", outcome)],
        )
    };
    assert_eq!(counted("started"), Some(1.0), "{page}");
    assert_eq!(
        counted("skipped_overlap"),
        Some(1.0),
        "the time was decided once, by the replica that held the shard: {page}"
    );
}
