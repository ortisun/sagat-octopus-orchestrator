//! Reset at 1,000, 5,000 and 20,000 events (NFR-018-005, T018-053, T018-054; `RESET_PREFIX_EVENTS = 5_000`, ceiling 20,000):
//! ended runs of that many journal events, built by the engine (a chain of activities completed one after the other by a worker),
//! then the points, the preview and the reset at the largest point the ceiling accepts, and the refusal beyond it.
//!
//! What is read: the time of each call to the run service (points, preview, reset, refusal), how long the row of the source run is
//! locked by the creation transaction (a probe that asks for the row `FOR UPDATE SKIP LOCKED` in a tight loop on its own connection
//! while the reset runs: the stretch for which it finds the row taken; its resolution is one round trip, and each probe takes the row for
//! the instant of its statement), the rows the new run holds (copied with `restored`), and the check that replay equals projection
//! (`verify_run`) on the new run.
//!
//! Bounds asserted (the spec promises none): a reset of the largest accepted prefix completes in 5 s (the statement timeout of the
//! pool) and holds the source row no longer than 1 s; the refusal beyond the ceiling is typed (`RESET_PREFIX_TOO_LONG`), creates
//! no run and is answered within 1 s; replay equals projection on every run a reset made.
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sagat_application::error::AppError;
use sagat_application::runs::{
    RESET_PREFIX_EVENTS, RerunBusinessId, ResetRequest, ResetSourcePolicy,
};
use sagat_domain::execution::{ResetReapply, RunId, RunStatus};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::harness::{Bench, bench};
use crate::measure::{Verdict, conclude, environment, load_now, ms, summary_json};
use crate::support::engine::{ctx, document};

const SAMPLES: usize = 10;
const RESETS: usize = 5;

async fn events_of(b: &Bench, run: RunId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM run_events WHERE run_id = $1")
        .bind(run.as_uuid())
        .fetch_one(&b.w.db.pool)
        .await
        .expect("count the events")
}

/// A chain of `steps` activities, every one the same activity so that one worker session serves them all, run to its end.
async fn ended_chain(b: &Bench, steps: usize, tag: &str) -> RunId {
    let nodes: Vec<Value> = (0..steps)
        .map(|index| {
            let mut node =
                json!({"id": format!("s{index}"), "type": "activity", "activity": "act.work"});
            if index + 1 < steps {
                node["next"] = json!(format!("s{}", index + 1));
            }
            node
        })
        .collect();
    let (workflow, _) =
        b.w.publish(document(&format!("reset-chain-{tag}-{steps}"), "s0", nodes))
            .await;
    let run = b.w.start(workflow, &json!({"k": 1})).await;
    let session = b.w.session(&["act.work"]).await;
    for _ in 0..steps {
        let task = b.w.one_task_of(&session, "act.work").await;
        b.w.complete(&session, &task, &json!({"done": true}))
            .await
            .expect("the step completes");
    }
    assert_eq!(b.w.run_status(run).await, RunStatus::Succeeded);
    run
}

fn request(sequence: u64) -> ResetRequest<'static> {
    ResetRequest {
        sequence,
        reason: "benchmark: the last steps used a wrong price list",
        acknowledge_reexecution: true,
        business: RerunBusinessId::Keep,
        reapply: ResetReapply::None,
        source_policy: ResetSourcePolicy::RequireTerminal,
    }
}

/// Asks, in a tight loop on a connection of its own, for the row of `run` with `FOR UPDATE SKIP LOCKED`; records for how long
/// (in milliseconds) it found the row taken, stretch by stretch.
struct Probe {
    stop: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<Vec<f64>>,
}

impl Probe {
    fn start(pool: sqlx::PgPool, run: Uuid) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let task = tokio::spawn(async move {
            let mut holds = Vec::new();
            let mut taken_since: Option<Instant> = None;
            while !flag.load(Ordering::Relaxed) {
                let asked = Instant::now();
                let free: Option<i32> =
                    sqlx::query_scalar("SELECT 1 FROM runs WHERE id = $1 FOR UPDATE SKIP LOCKED")
                        .bind(run)
                        .fetch_optional(&pool)
                        .await
                        .expect("the probe");
                match (free, taken_since) {
                    (None, None) => taken_since = Some(asked),
                    (Some(_), Some(since)) => {
                        holds.push(ms(asked.duration_since(since)));
                        taken_since = None;
                    }
                    _ => {}
                }
            }
            if let Some(since) = taken_since {
                holds.push(ms(since.elapsed()));
            }
            holds
        });
        Self { stop, task }
    }

    async fn finish(self) -> Vec<f64> {
        self.stop.store(true, Ordering::Relaxed);
        self.task.await.expect("the probe ends")
    }
}

struct Source {
    run: RunId,
    events: i64,
    points: Vec<u64>,
}

pub async fn at_sizes(scenario: &str, sizes: &[usize]) {
    let before = load_now();
    let b = bench().await;
    let environment = environment(&b.w.db.pool).await;

    // how many events a step makes, from two small chains (the journal of a chain is a fixed part and a part for each step)
    let two = events_of(&b, ended_chain(&b, 2, "calibrate").await).await;
    let ten = events_of(&b, ended_chain(&b, 10, "calibrate").await).await;
    let per_step = (ten - two) as f64 / 8.0;
    let base = two as f64 - 2.0 * per_step;

    let mut verdicts = Vec::new();
    let mut cases = Vec::new();
    for wanted in sizes {
        let steps = (((*wanted as f64 - base) / per_step).ceil() as usize).max(2);
        let built = Instant::now();
        let run = ended_chain(&b, steps, "source").await;
        let build_s = built.elapsed().as_secs_f64();
        let events = events_of(&b, run).await;

        let asked = Instant::now();
        let points: Vec<u64> =
            b.w.runs
                .reset_points(&b.w.admin, run, &ctx())
                .await
                .expect("the points")
                .iter()
                .map(|point| point.sequence)
                .collect();
        let points_ms = ms(asked.elapsed());
        let source = Source {
            run,
            events,
            points,
        };
        let ceiling = u64::try_from(RESET_PREFIX_EVENTS).expect("small");
        let accepted = source
            .points
            .iter()
            .copied()
            .filter(|sequence| *sequence <= ceiling)
            .max();
        let first_refused = source
            .points
            .iter()
            .copied()
            .find(|sequence| *sequence > ceiling);
        let near_twenty_thousand = source
            .points
            .iter()
            .copied()
            .find(|sequence| *sequence >= 20_000);
        let mut case = json!({
            "journal_events": source.events, "steps": steps, "points": source.points.len(),
            "built_in_s": build_s, "points_ms": points_ms,
        });

        if let Some(sequence) = accepted {
            // the preview, SAMPLES times
            let mut previews = Vec::new();
            for _ in 0..SAMPLES {
                let asked = Instant::now();
                b.w.runs
                    .reset_preview(&b.w.admin, source.run, sequence, ResetReapply::None, &ctx())
                    .await
                    .expect("the preview");
                previews.push(ms(asked.elapsed()));
            }
            // the reset, RESETS times, with the probe on the row of the source
            let (mut totals, mut holds, mut restored_rows, mut verifies) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for _ in 0..RESETS {
                let probe = Probe::start(b.w.db.pool.clone(), source.run.as_uuid());
                tokio::time::sleep(Duration::from_millis(20)).await;
                let asked = Instant::now();
                let made =
                    b.w.runs
                        .reset(
                            &b.w.admin,
                            source.run,
                            request(sequence),
                            &format!("reset-{}", Uuid::now_v7()),
                            &ctx(),
                        )
                        .await
                        .expect("the reset");
                totals.push(ms(asked.elapsed()));
                let held = probe.finish().await;
                holds.push(held.iter().copied().fold(0.0, f64::max));
                let new_run = made.run.expect("a run").info.run_id;
                let restored: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM run_events WHERE run_id = $1 AND detail->>'restored' = 'true'",
                )
                .bind(new_run.as_uuid())
                .fetch_one(&b.w.db.pool)
                .await
                .expect("count");
                restored_rows.push(restored as f64);
                let verifying = Instant::now();
                b.w.assert_consistent(new_run).await;
                verifies.push(ms(verifying.elapsed()));
                assert_eq!(
                    restored,
                    i64::try_from(sequence).expect("small"),
                    "the prefix is copied row for row"
                );
            }
            case["largest_accepted_point"] = json!(sequence);
            case["preview_ms"] = summary_json(&previews);
            case["reset_ms"] = summary_json(&totals);
            case["source_row_locked_ms"] = summary_json(&holds);
            case["rows_copied"] = summary_json(&restored_rows);
            case["verify_run_ms"] = summary_json(&verifies);
            case["replay_equals_projection"] = json!(true);
            if source.events >= i64::try_from(RESET_PREFIX_EVENTS / 2).expect("small")
                && sequence * 10 >= ceiling * 9
            {
                // the largest prefix the ceiling accepts: this is the case the bounds are about
                let slowest = totals.iter().copied().fold(0.0, f64::max);
                let held = holds.iter().copied().fold(0.0, f64::max);
                // the same prefix of a journal that goes on past the ceiling is a case of its own: the reset reads the whole journal
                let long = source.events > 2 * i64::try_from(RESET_PREFIX_EVENTS).expect("small");
                verdicts.push(Verdict::at_most(
                    if long {
                        "a_reset_at_the_ceiling_of_a_journal_twice_as_long_completes_within_the_statement_timeout"
                    } else {
                        "the_largest_accepted_reset_completes_within_the_statement_timeout"
                    },
                    "slowest reset near the ceiling",
                    slowest,
                    5_000.0,
                    "ms",
                ));
                verdicts.push(Verdict::at_most(
                    if long {
                        "a_reset_at_the_ceiling_of_a_journal_twice_as_long_holds_the_source_row_briefly"
                    } else {
                        "the_largest_accepted_reset_holds_the_source_row_briefly"
                    },
                    "longest hold of the source row near the ceiling",
                    held,
                    1_000.0,
                    "ms",
                ));
            }
        }

        // the refusals: the first point past the ceiling and the point nearest 20,000 events
        for (label, sequence) in [
            ("first_past_the_ceiling", first_refused),
            ("near_20000", near_twenty_thousand),
        ] {
            let Some(sequence) = sequence else { continue };
            let runs_before: i64 =
                sqlx::query_scalar("SELECT count(*) FROM runs WHERE rerun_of = $1")
                    .bind(source.run.as_uuid())
                    .fetch_one(&b.w.db.pool)
                    .await
                    .expect("count");
            let (mut previews, mut resets) = (Vec::new(), Vec::new());
            for _ in 0..SAMPLES {
                let asked = Instant::now();
                let refused =
                    b.w.runs
                        .reset_preview(&b.w.admin, source.run, sequence, ResetReapply::None, &ctx())
                        .await
                        .expect_err("past the ceiling");
                previews.push(ms(asked.elapsed()));
                assert!(
                    matches!(
                        refused,
                        AppError::ResetRefused {
                            reason: "RESET_PREFIX_TOO_LONG",
                            ..
                        }
                    ),
                    "the typed reason: {refused:?}"
                );
                let asked = Instant::now();
                let refused =
                    b.w.runs
                        .reset(
                            &b.w.admin,
                            source.run,
                            request(sequence),
                            &format!("reset-{}", Uuid::now_v7()),
                            &ctx(),
                        )
                        .await
                        .expect_err("past the ceiling");
                resets.push(ms(asked.elapsed()));
                assert!(
                    matches!(
                        refused,
                        AppError::ResetRefused {
                            reason: "RESET_PREFIX_TOO_LONG",
                            ..
                        }
                    ),
                    "the typed reason: {refused:?}"
                );
            }
            let runs_after: i64 =
                sqlx::query_scalar("SELECT count(*) FROM runs WHERE rerun_of = $1")
                    .bind(source.run.as_uuid())
                    .fetch_one(&b.w.db.pool)
                    .await
                    .expect("count");
            assert_eq!(runs_before, runs_after, "a refusal creates no run");
            let slowest = previews.iter().chain(&resets).copied().fold(0.0, f64::max);
            case[format!("refused_{label}")] = json!({
                "sequence": sequence, "reason": "RESET_PREFIX_TOO_LONG", "runs_created": 0,
                "preview_ms": summary_json(&previews), "reset_ms": summary_json(&resets),
            });
            if label == "near_20000" {
                verdicts.push(Verdict::at_most(
                    "the_refusal_at_20000_events_is_answered_within_a_second",
                    "slowest refusal (preview or reset) at 20,000 events",
                    slowest,
                    1_000.0,
                    "ms",
                ));
            }
        }
        println!("{scenario}: {} events: {case}", source.events);
        cases.push(case);
    }
    let fragment = json!({
        "ceiling": RESET_PREFIX_EVENTS, "hard_ceiling": 20_000, "events_per_step": per_step, "events_fixed": base,
        "samples_preview": SAMPLES, "resets_per_case": RESETS, "cases": cases,
    });
    conclude(scenario, environment, before, fragment, &verdicts);
}
