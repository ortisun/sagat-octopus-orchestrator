//! Performance measurements of feature 018 (schedules and bulk run operations, NFR-018-005, T018-072): firing lag at 1,000 and
//! 10,000 schedules, the rate of batch jobs, the business-id claim under 32-way contention, reset at 1,000, 5,000 and 20,000
//! events, and the costs of the trigger handover at 10,000 and 100,000 cron triggers.
//!
//! Every scenario is `#[ignore]`: it starts a real PostgreSQL and runs from seconds to half an hour, so it is run on purpose, never
//! by a pull request (the CI runs this target without `--ignored`, so only the unit tests of the helpers in `measure.rs` run there).
//! The sizes default to a smoke run (a hundred schedules, a few hundred runs); the sizes of the report are set by the environment
//! (`SAGAT_PERF_SCHEDULES`, `SAGAT_PERF_BATCH_RUNS`, `SAGAT_PERF_BATCH_SELECTION`, `SAGAT_PERF_BATCH_EACH`,
//! `SAGAT_PERF_CLAIM_ROUNDS`, `SAGAT_PERF_RESET_EVENTS`, `SAGAT_PERF_TRIGGERS`, `SAGAT_PERF_TRIGGER_SECONDS`) and
//! `tests/performance/schedules-bulk-operations/run.sh` sets them, starts a PostgreSQL of its own with the configuration the report
//! declares, repeats each scenario and writes the fragments the report is made from.
//!
//! ```sh
//! SAGAT_TEST_AS_RUNTIME_ROLE=1 SAGAT_PERF_FRAGMENTS=/path/to/fragments \
//!   cargo test -p sagat-tests --test perf_schedules_bulk_operations --release -- --ignored --nocapture --test-threads=1
//! ```
//!
//! A scenario states its bounds, writes its fragment, and then fails when the data exceeds a bound: the numbers are the point, and a
//! bound is not loosened to make a run green (`tests/performance/reports/schedules-bulk-operations.md`).
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

#[path = "../../support/mod.rs"]
mod support;

mod batch;
mod claim;
mod firing;
mod harness;
mod measure;
mod reset;
mod triggers;

use std::time::Duration;

use measure::{sizes, variant};

// --- firing lag --------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn firing_lag_all_due_at_the_same_instant() {
    for n in sizes("SAGAT_PERF_SCHEDULES", &[100]) {
        firing::run(&format!("firing-instant-{n}{}", variant()), n, 0.0, 1).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn firing_lag_spread_over_a_minute() {
    for n in sizes("SAGAT_PERF_SCHEDULES", &[100]) {
        firing::run(&format!("firing-spread-{n}{}", variant()), n, 60.0, 1).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn firing_lag_two_replicas_all_due_at_the_same_instant() {
    for n in sizes("SAGAT_PERF_SCHEDULES", &[100]) {
        firing::run(
            &format!("firing-instant-two-replicas-{n}{}", variant()),
            n,
            0.0,
            2,
        )
        .await;
    }
}

// --- batch rate --------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn batch_one_job_at_the_default_rate() {
    for n in sizes("SAGAT_PERF_BATCH_RUNS", &[60]) {
        batch::one_job(
            &format!("batch-one-job-default-rate-{n}{}", variant()),
            n,
            None,
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn batch_one_job_at_the_highest_rate() {
    for n in sizes("SAGAT_PERF_BATCH_RUNS", &[60]) {
        batch::one_job(
            &format!("batch-one-job-highest-rate-{n}{}", variant()),
            n,
            Some(200),
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn batch_selection_and_drain_of_many_runs() {
    for n in sizes("SAGAT_PERF_BATCH_SELECTION", &[500]) {
        batch::selection_and_drain(&format!("batch-selection-and-drain-{n}{}", variant()), n).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn batch_sixteen_jobs_at_the_default_rate() {
    for each in sizes("SAGAT_PERF_BATCH_EACH", &[20]) {
        batch::jobs_at_once(
            &format!("batch-sixteen-jobs-default-rate-{each}{}", variant()),
            &[each; 16],
            None,
            true,
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn batch_sixteen_jobs_at_the_highest_rate() {
    for each in sizes("SAGAT_PERF_BATCH_EACH", &[20]) {
        batch::jobs_at_once(
            &format!("batch-sixteen-jobs-highest-rate-{each}{}", variant()),
            &[each; 16],
            Some(200),
            true,
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn batch_one_big_job_beside_fifteen_small_ones() {
    for each in sizes("SAGAT_PERF_BATCH_EACH", &[20]) {
        // one big job that is still working when the fifteen small ones have ended
        let mut sizes = vec![each * 30];
        sizes.extend([each; 15]);
        batch::jobs_at_once(
            &format!("batch-one-big-fifteen-small-{each}{}", variant()),
            &sizes,
            None,
            false,
        )
        .await;
    }
}

// --- the claim of a business identifier ---------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn business_id_claim_thirty_two_way() {
    for rounds in sizes("SAGAT_PERF_CLAIM_ROUNDS", &[5]) {
        claim::thirty_two_way(
            &format!("claim-thirty-two-way-{rounds}{}", variant()),
            rounds,
        )
        .await;
    }
}

// --- reset ---------------------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn reset_at_the_sizes_of_the_ceiling() {
    let events = sizes("SAGAT_PERF_RESET_EVENTS", &[300]);
    reset::at_sizes("reset", &events).await;
}

// --- the trigger handover ------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn trigger_handover_steady_state() {
    triggers::steady_state("triggers-steady", &sizes("SAGAT_PERF_TRIGGERS", &[1_000])).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: a real PostgreSQL, up to minutes; run on purpose (run.sh)"]
async fn trigger_handover_backlog_after_an_upgrade() {
    let seconds = sizes("SAGAT_PERF_TRIGGER_SECONDS", &[10])[0];
    triggers::backlog(
        "triggers-backlog",
        &sizes("SAGAT_PERF_TRIGGERS", &[1_000]),
        Duration::from_secs(u64::try_from(seconds).expect("small")),
    )
    .await;
}
