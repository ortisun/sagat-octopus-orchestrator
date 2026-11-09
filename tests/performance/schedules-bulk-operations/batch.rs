//! Batch rate (NFR-018-005, T018-045, T018-046): a job over N runs (cancel) and several jobs at once, the batch pass of one
//! engine replica in the engine's own loop, the real run service applying every item.
//!
//! What is read: the items a second the job achieved against the rate it asked for (the rows of `batch_items`, by the time each
//! outcome was committed, and the gaps between them), the time the selection of the runs took, the time the drain of a cancelled
//! job took, whether a job starved another (the time to the first item and to the last, job against job), and what the product
//! measures about itself (`batch_item_duration_seconds`).
//!
//! Bounds asserted (the spec promises none): a job reaches at least 80% of the rate it asked for and never more than 110% of it
//! (the per-job rate is a ceiling the executor keeps); with several jobs no job reaches less than half its rate, none waits more
//! than 10 s for its first item and jobs of one size end within a factor of two of each other (a job that ends last is starved by
//! the ones before it); the selection of N runs ends in less time than the job at the highest rate would take to apply them
//! (N / 200 s), or the selection, and not the rate, is the limit.
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::time::{Duration, Instant};

use sagat_application::batch::BatchWrite;
use sagat_application::identity::RequestContext;
use sagat_domain::batch::{Action, Filter, JobState, Justification, Rate, Selector};
use sagat_scheduler::BatchSettings;
use serde_json::json;
use uuid::Uuid;

use crate::harness::{
    BatchReplica, batch_poll, batch_service, bench, capture, room_for, start_runs,
};
use crate::measure::{Summary, Verdict, conclude, environment, load_now, summary_json};
use crate::support::bounded::within;

/// The reason of every job: a justification is mandatory (8 to 2,000 characters).
const REASON: &str = "benchmark: cancel the runs of the namespace";

fn write(workflow: Option<Uuid>, items: usize, per_second: Option<u32>) -> BatchWrite {
    BatchWrite {
        selector: Selector::filter(Filter {
            workflow_id: workflow,
            ..Filter::default()
        })
        .expect("a selector"),
        action: Action::Cancel,
        step_id: None,
        instance_path: Vec::new(),
        signal: None,
        update: None,
        drop_business_id: false,
        repin: false,
        reason: Justification::new(REASON).expect("a reason"),
        max_items: Some(u32::try_from(items).expect("small")),
        rate: per_second.map(|rate| Rate::per_second(rate).expect("a rate")),
    }
}

fn is_over(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Cancelled | JobState::Failed
    )
}

/// What the items of one job say about its pace, read from the rows.
struct Pace {
    items: usize,
    /// Seconds from the creation of the job to its first and to its last item.
    first_s: f64,
    last_s: f64,
    /// Items a second between the first and the last.
    achieved: f64,
    /// Milliseconds between two items, in order.
    gaps_ms: Vec<f64>,
}

async fn pace_of(pool: &sqlx::PgPool, job: Uuid) -> Pace {
    let rows: Vec<(f64,)> = sqlx::query_as(
        "SELECT extract(epoch FROM i.updated_at)::float8 FROM batch_items i \
          WHERE i.job_id = $1 AND i.state = 'SUCCEEDED' ORDER BY i.updated_at, i.ordinal",
    )
    .bind(job)
    .fetch_all(pool)
    .await
    .expect("the items");
    let created: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM created_at)::float8 FROM batch_jobs WHERE id = $1",
    )
    .bind(job)
    .fetch_one(pool)
    .await
    .expect("the job");
    let times: Vec<f64> = rows.into_iter().map(|row| row.0).collect();
    let first = times.first().copied().unwrap_or(created);
    let last = times.last().copied().unwrap_or(created);
    let span = (last - first).max(0.001);
    Pace {
        items: times.len(),
        first_s: first - created,
        last_s: last - created,
        achieved: (times.len().saturating_sub(1)) as f64 / span,
        gaps_ms: times
            .windows(2)
            .map(|pair| (pair[1] - pair[0]) * 1_000.0)
            .collect(),
    }
}

/// One job over `n` runs at `per_second` (the default rate when none), applied by one engine replica to the end.
pub async fn one_job(scenario: &str, n: usize, per_second: Option<u32>) {
    let before = load_now();
    let b = bench().await;
    let environment = environment(&b.w.db.pool).await;
    let started_in = start_runs(&b, b.workflow, n, 16).await;
    let product = capture();
    drop(product.take("batch_item_duration_seconds"));
    let service = batch_service(&b.w).await;
    let engine =
        std::sync::Arc::new(BatchReplica::new(&b.w, "engine", BatchSettings::default()).await);
    engine.keep().await;
    let asked = per_second.unwrap_or(20);

    let created = service
        .create(
            &b.w.admin,
            b.w.namespace.id(),
            write(None, n, per_second),
            &format!("bench-{}", Uuid::now_v7()),
            &RequestContext::default(),
        )
        .await
        .expect("create the job");
    let began = Instant::now();
    let running = engine.spawn(batch_poll());
    let mut selected_after = None;
    // the time to wait is a bound on the stage, not a claim about the rate: a lone job has been seen to apply an item a second
    let limit =
        Duration::from_secs_f64((n as f64 * 1.5).max(n as f64 / f64::from(asked) * 3.0) + 300.0);
    within(format!("the job ends ({scenario})"), limit, async {
        loop {
            let job = service.get(&b.w.admin, created.id).await.expect("the job");
            if selected_after.is_none() && job.state != JobState::Selecting {
                selected_after = Some(began.elapsed());
            }
            if is_over(job.state) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    let wall = began.elapsed();
    let stats = running.stop().await;

    let job = service.get(&b.w.admin, created.id).await.expect("the job");
    assert_eq!(job.state, JobState::Completed, "the job completes");
    assert_eq!(
        (job.counts.selected, job.counts.succeeded, job.counts.failed),
        (
            u32::try_from(n).expect("small"),
            u32::try_from(n).expect("small"),
            0
        )
    );
    let cancelled: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE status = 'CANCELLED'")
        .fetch_one(&b.w.db.pool)
        .await
        .expect("count");
    assert_eq!(
        cancelled,
        i64::try_from(n).expect("small"),
        "every run cancelled once"
    );
    let pace = pace_of(&b.w.db.pool, created.id).await;
    let item_seconds = product.take("batch_item_duration_seconds");
    let selection = selected_after.unwrap_or(wall).as_secs_f64();

    let fragment = json!({
        "runs": n, "rate_asked_per_s": asked, "runs_started_in_s": started_in.as_secs_f64(),
        "settings": {"batch": BatchSettings::default().batch, "steps_per_job": BatchSettings::default().steps_per_job,
                     "round_actions": BatchSettings::default().round_actions, "poll_ms": batch_poll().as_millis(), "pool_per_replica": 16},
        "wall_s": wall.as_secs_f64(),
        "selection_s_until_the_state_left_selecting": selection,
        "first_item_s_after_creation": pace.first_s, "last_item_s_after_creation": pace.last_s,
        "achieved_items_per_s": pace.achieved,
        "gap_between_items_ms": summary_json(&pace.gaps_ms),
        "item_duration_s_product_histogram": summary_json(&item_seconds),
        "rounds": stats.json(),
    });
    let verdicts = [
        Verdict::at_least(
            "reaches_the_rate_it_asked_for",
            "items a second achieved",
            pace.achieved,
            0.8 * f64::from(asked),
            "items/s",
        ),
        Verdict::at_most(
            "never_exceeds_the_rate_it_asked_for",
            "items a second achieved",
            pace.achieved,
            1.1 * f64::from(asked),
            "items/s",
        ),
        Verdict::at_most(
            "selection_is_not_the_limit",
            "seconds to select the runs",
            selection,
            n as f64 / 200.0,
            "s",
        ),
    ];
    println!(
        "{scenario}: {n} runs at {asked}/s: {:.1} items/s achieved, selection {selection:.2} s, wall {:.1} s, item p50 {:.1} ms",
        pace.achieved,
        wall.as_secs_f64(),
        Summary::of(&item_seconds).map_or(0.0, |s| s.p50 * 1_000.0),
    );
    conclude(scenario, environment, before, fragment, &verdicts);
}

/// A job over `n` runs, selected at the highest rate and cancelled as soon as the selection ends: how long the selection of
/// `n` runs takes, and how long the drain of the items that were not applied takes.
pub async fn selection_and_drain(scenario: &str, n: usize) {
    let before = load_now();
    let b = bench().await;
    let environment = environment(&b.w.db.pool).await;
    let started_in = start_runs(&b, b.workflow, n, 16).await;
    let service = batch_service(&b.w).await;
    let engine =
        std::sync::Arc::new(BatchReplica::new(&b.w, "engine", BatchSettings::default()).await);
    engine.keep().await;
    // a rate of one item a second: the job selects and then waits, so that what is measured is the selection and the drain
    let created = service
        .create(
            &b.w.admin,
            b.w.namespace.id(),
            write(None, n, Some(1)),
            &format!("bench-{}", Uuid::now_v7()),
            &RequestContext::default(),
        )
        .await
        .expect("create the job");
    let began = Instant::now();
    let running = engine.spawn(batch_poll());
    let selection = within(
        format!("the selection ends ({scenario})"),
        Duration::from_secs(900),
        async {
            loop {
                let job = service.get(&b.w.admin, created.id).await.expect("the job");
                if job.state != JobState::Selecting {
                    break (began.elapsed(), job.counts.selected);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        },
    )
    .await;
    let cancelled_at = Instant::now();
    service
        .cancel(
            &b.w.admin,
            created.id,
            "benchmark: stop the job",
            &RequestContext::default(),
        )
        .await
        .expect("cancel the job");
    let drained = within(
        format!("the job drains ({scenario})"),
        Duration::from_secs(900),
        async {
            loop {
                let job = service.get(&b.w.admin, created.id).await.expect("the job");
                if is_over(job.state) {
                    break (cancelled_at.elapsed(), job);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        },
    )
    .await;
    let stats = running.stop().await;
    let (drain, job) = drained;
    let selected = i64::from(selection.1);
    let items: i64 = sqlx::query_scalar("SELECT count(*) FROM batch_items WHERE job_id = $1")
        .bind(created.id)
        .fetch_one(&b.w.db.pool)
        .await
        .expect("count");
    assert_eq!(items, selected, "an item for every run selected");
    assert_eq!(job.state, JobState::Cancelled);
    assert_eq!(
        u64::from(job.counts.succeeded) + u64::from(job.counts.cancelled),
        u64::from(job.counts.selected),
        "every item ends as applied or cancelled"
    );
    let selection_s = selection.0.as_secs_f64();
    let drain_s = drain.as_secs_f64();
    let fragment = json!({
        "runs": n, "selected": selected, "runs_started_in_s": started_in.as_secs_f64(),
        "selection_s": selection_s, "selection_runs_per_s": n as f64 / selection_s,
        "drain_s": drain_s, "drained_items": job.counts.cancelled, "drain_items_per_s": f64::from(job.counts.cancelled) / drain_s,
        "settings": {"select_page": 500, "drain_page": 500, "poll_ms": batch_poll().as_millis()},
        "rounds": stats.json(),
    });
    let verdicts = [
        Verdict::at_most(
            "selection_is_not_the_limit",
            "seconds to select the runs",
            selection_s,
            n as f64 / 200.0,
            "s",
        ),
        Verdict::at_most(
            "drain_is_not_the_limit",
            "seconds to cancel the items that were not applied",
            drain_s,
            n as f64 / 200.0,
            "s",
        ),
    ];
    println!("{scenario}: {n} runs: selection {selection_s:.2} s, drain {drain_s:.2} s");
    conclude(scenario, environment, before, fragment, &verdicts);
}

/// Several jobs at once, `sizes[i]` runs each (a workflow of its own for each), every job at `per_second`.
pub async fn jobs_at_once(scenario: &str, sizes: &[usize], per_second: Option<u32>, equal: bool) {
    let before = load_now();
    let b = bench().await;
    let environment = environment(&b.w.db.pool).await;
    let jobs = i64::try_from(sizes.len()).expect("small");
    room_for(&b.w, "batch_jobs", jobs).await;
    room_for(&b.w, "tenant_batch_jobs", jobs).await;
    let asked = per_second.unwrap_or(20);
    let mut workflows = Vec::new();
    let mut started_in = Duration::ZERO;
    for (index, size) in sizes.iter().enumerate() {
        let (workflow, _) =
            b.w.publish(crate::support::engine::chain(&format!(
                "bench-batch-{index}"
            )))
            .await;
        started_in += start_runs(&b, workflow, *size, 16).await;
        workflows.push(workflow);
    }
    let product = capture();
    drop(product.take("batch_item_duration_seconds"));
    let service = batch_service(&b.w).await;
    let engine =
        std::sync::Arc::new(BatchReplica::new(&b.w, "engine", BatchSettings::default()).await);
    engine.keep().await;
    let mut created = Vec::new();
    for (index, (workflow, size)) in workflows.iter().zip(sizes).enumerate() {
        let job = service
            .create(
                &b.w.admin,
                b.w.namespace.id(),
                write(Some(workflow.as_uuid()), *size, per_second),
                &format!("bench-{index}-{}", Uuid::now_v7()),
                &RequestContext::default(),
            )
            .await
            .expect("create the job");
        created.push(job.id);
    }
    let began = Instant::now();
    let running = engine.spawn(batch_poll());
    let total: usize = sizes.iter().sum();
    let limit = Duration::from_secs_f64(
        (total as f64 * 1.5).max(total as f64 / f64::from(asked) * 3.0) + 300.0,
    );
    // jobs of one size run to their end; with one big job beside small ones the big one is cancelled when the small ones have
    // ended (the point is whether the small ones waited for it, and it would take the whole of the time at a pace of its own)
    let waited_for = if equal { 0 } else { 1 };
    within(format!("the jobs end ({scenario})"), limit, async {
        loop {
            let mut open = 0;
            for id in &created[waited_for..] {
                if !is_over(service.get(&b.w.admin, *id).await.expect("the job").state) {
                    open += 1;
                }
            }
            if open == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await;
    let wall = began.elapsed();
    if !equal {
        service
            .cancel(
                &b.w.admin,
                created[0],
                "benchmark: the small jobs have ended",
                &RequestContext::default(),
            )
            .await
            .expect("cancel the big job");
        within(
            format!("the big job drains ({scenario})"),
            Duration::from_secs(600),
            async {
                while !is_over(
                    service
                        .get(&b.w.admin, created[0])
                        .await
                        .expect("the job")
                        .state,
                ) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
        )
        .await;
    }
    let stats = running.stop().await;

    let mut paces = Vec::new();
    for (index, (id, size)) in created.iter().zip(sizes).enumerate() {
        let job = service.get(&b.w.admin, *id).await.expect("the job");
        if equal || index > 0 {
            assert_eq!(job.state, JobState::Completed);
            assert_eq!(job.counts.succeeded, u32::try_from(*size).expect("small"));
        } else {
            assert_eq!(job.state, JobState::Cancelled);
            assert_eq!(
                job.counts.succeeded + job.counts.cancelled,
                u32::try_from(*size).expect("small")
            );
        }
        paces.push(pace_of(&b.w.db.pool, *id).await);
    }
    let aggregate_from = paces.iter().map(|p| p.first_s).fold(f64::MAX, f64::min);
    let aggregate_to = paces.iter().map(|p| p.last_s).fold(f64::MIN, f64::max);
    // the items the jobs applied (a cancelled big job did not apply all its items), over the time between the first and the last
    let applied: usize = paces.iter().map(|pace| pace.items).sum();
    let aggregate = applied as f64 / (aggregate_to - aggregate_from).max(0.001);
    let item_seconds = product.take("batch_item_duration_seconds");
    let achieved: Vec<f64> = paces.iter().map(|p| p.achieved).collect();
    let firsts: Vec<f64> = paces.iter().map(|p| p.first_s).collect();
    let lasts: Vec<f64> = paces.iter().map(|p| p.last_s).collect();
    let slowest_job = achieved.iter().copied().fold(f64::MAX, f64::min);
    let latest_first = firsts.iter().copied().fold(f64::MIN, f64::max);
    let spread = lasts.iter().copied().fold(f64::MIN, f64::max)
        / lasts.iter().copied().fold(f64::MAX, f64::min);

    let fragment = json!({
        "jobs": sizes.len(), "sizes": sizes, "rate_asked_per_job_per_s": asked,
        "demand_items_per_s": f64::from(asked) * sizes.len() as f64,
        "runs_started_in_s": started_in.as_secs_f64(),
        "settings": {"batch": BatchSettings::default().batch, "steps_per_job": BatchSettings::default().steps_per_job,
                     "round_actions": BatchSettings::default().round_actions, "poll_ms": batch_poll().as_millis(), "pool_per_replica": 16},
        "wall_s": wall.as_secs_f64(),
        "aggregate_items_per_s": aggregate,
        "per_job_items_per_s": summary_json(&achieved),
        "first_item_s_per_job": summary_json(&firsts),
        "last_item_s_per_job": summary_json(&lasts),
        "last_over_first_to_end": spread,
        "per_job": paces.iter().zip(sizes).map(|(p, size)| json!({
            "size": size, "items": p.items, "first_s": p.first_s, "last_s": p.last_s, "achieved_per_s": p.achieved,
        })).collect::<Vec<_>>(),
        "item_duration_s_product_histogram": summary_json(&item_seconds),
        "rounds": stats.json(),
    });
    let mut verdicts = vec![Verdict::at_most(
        "no_job_waits_for_the_others",
        "seconds to the first item of the job that waited longest",
        latest_first,
        10.0,
        "s",
    )];
    if equal {
        verdicts.push(Verdict::at_least(
            "every_job_reaches_half_its_rate",
            "items a second of the slowest job",
            slowest_job,
            0.5 * f64::from(asked),
            "items/s",
        ));
        verdicts.push(Verdict::at_most(
            "jobs_of_one_size_end_together",
            "the last job's end over the first job's end",
            spread,
            2.0,
            "x",
        ));
    } else {
        // jobs of unequal size: a small job must not wait for the big one. Each ends within three times its own time at the
        // rate it asked for, and ten seconds.
        let worst = paces
            .iter()
            .zip(sizes)
            .skip(1)
            .map(|(p, size)| p.last_s / (3.0 * *size as f64 / f64::from(asked) + 10.0))
            .fold(0.0, f64::max);
        verdicts.push(Verdict::at_most(
            "a_small_job_does_not_wait_for_a_big_one",
            "end of each job over three times its own time at its rate plus ten seconds",
            worst,
            1.0,
            "x",
        ));
    }
    println!(
        "{scenario}: {} jobs, sizes {sizes:?}, {asked}/s each: aggregate {aggregate:.1} items/s (demand {:.0}), slowest job {slowest_job:.1}/s, \
         latest first item {latest_first:.1} s, ends {spread:.2}x apart, wall {:.1} s",
        sizes.len(),
        f64::from(asked) * sizes.len() as f64,
        wall.as_secs_f64()
    );
    conclude(scenario, environment, before, fragment, &verdicts);
}
