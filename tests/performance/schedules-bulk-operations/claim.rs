//! Business-id claim under 32-way contention (NFR-018-005, T018-034, T018-036): 32 starts released together, in one process
//! over a pool that can hold 32 of them at once (a pool of 10, the default of a process, would queue the starts at the pool and
//! not at the row).
//!
//! Four shapes, in turn, round after round (so that drift of the machine falls on all four alike): 32 starts of one identifier
//! under the policy that refuses (one run, 31 `BUSINESS_ID_IN_USE`), 32 of one identifier under the policy that returns the
//! existing run (one run created, 31 answered with it), 32 starts with 32 different identifiers (no contention: the cost of the
//! claim) and 32 plain starts without an identifier (what a start costs without it).
//!
//! Bounds asserted (the spec promises none): the outcome is exactly right in every round (a hard check, not a number); the p99 of
//! a start that waits for the holder of a hot identifier stays within one second; the claim costs no more than 50% over a plain
//! start at the median and a factor of two at the p95.
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use sagat_application::error::AppError;
use sagat_application::runs::{BusinessStart, StartOptions, StartRun, Started};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use serde_json::json;
use tokio::sync::Barrier;
use uuid::Uuid;

use crate::harness::bench;
use crate::measure::{Summary, Verdict, conclude, environment, load_now, ms, summary_json};
use crate::support::engine::{Runs, ctx};

const WAYS: usize = 32;
const WARM_UP_ROUNDS: usize = 3;

type Outcome = (Duration, Result<Started, AppError>);

fn options(id: Option<String>, conflict: ConflictPolicy) -> StartOptions {
    StartOptions {
        business: id.map(|id| BusinessStart {
            id: BusinessId::new(&id).expect("an identifier"),
            conflict,
            reuse: ReusePolicy::AllowDuplicate,
            window: ReuseWindow::default(),
        }),
        ..StartOptions::default()
    }
}

/// `WAYS` starts released together; the latency of each is taken from the release to its answer.
async fn burst(
    runs: &Arc<Runs>,
    workflow: sagat_domain::definition::WorkflowId,
    admin: &sagat_domain::identity::Principal,
    id_of: impl Fn(usize) -> Option<String>,
    conflict: ConflictPolicy,
) -> Vec<Outcome> {
    let barrier = Arc::new(Barrier::new(WAYS));
    let mut tasks = Vec::new();
    for way in 0..WAYS {
        let (runs, barrier, admin) = (Arc::clone(runs), Arc::clone(&barrier), admin.clone());
        let id = id_of(way);
        tasks.push(tokio::spawn(async move {
            let key = format!("claim-{way}-{}", Uuid::now_v7());
            barrier.wait().await;
            let began = Instant::now();
            let answer = runs
                .start_with_options(
                    &admin,
                    &StartRun {
                        namespace: "payments",
                        workflow,
                        version: None,
                        input: b"{}",
                        correlation_id: None,
                        deadline: None,
                        key: &key,
                    },
                    options(id, conflict),
                    &ctx(),
                )
                .await;
            (began.elapsed(), answer)
        }));
    }
    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.expect("a start"));
    }
    outcomes
}

#[derive(Default)]
struct Samples {
    winner: Vec<f64>,
    other: Vec<f64>,
}

pub async fn thirty_two_way(scenario: &str, rounds: usize) {
    let before = load_now();
    let b = bench().await;
    let environment = environment(&b.w.db.pool).await;
    let replica = b.w.replica_with_pool(48).await;
    let runs = Arc::new(replica.runs);
    let (mut refuse, mut existing, mut distinct, mut plain) = (
        Samples::default(),
        Samples::default(),
        Samples::default(),
        Samples::default(),
    );
    let mut sizes = [0usize; 4];

    for round in 0..(rounds + WARM_UP_ROUNDS) {
        let counted = round >= WARM_UP_ROUNDS;
        // 1. one identifier, the policy that refuses
        let hot = format!("hot-fail-{round}-{}", Uuid::now_v7());
        let answers = burst(
            &runs,
            b.workflow,
            &b.w.admin,
            |_| Some(hot.clone()),
            ConflictPolicy::Fail,
        )
        .await;
        let (mut created, mut refused) = (0, 0);
        for (took, answer) in &answers {
            let bucket = match answer {
                Ok(started) if !started.existing => {
                    created += 1;
                    &mut refuse.winner
                }
                Err(AppError::BusinessIdConflict { reason, .. }) => {
                    assert_eq!(*reason, "BUSINESS_ID_IN_USE");
                    refused += 1;
                    &mut refuse.other
                }
                other => panic!("an answer a start of a held identifier must not give: {other:?}"),
            };
            if counted {
                bucket.push(ms(*took));
            }
        }
        assert_eq!(
            (created, refused),
            (1, WAYS - 1),
            "one run, the others refused"
        );

        // 2. one identifier, the policy that returns the run that holds it
        let hot = format!("hot-use-{round}-{}", Uuid::now_v7());
        let answers = burst(
            &runs,
            b.workflow,
            &b.w.admin,
            |_| Some(hot.clone()),
            ConflictPolicy::UseExisting,
        )
        .await;
        let mut seen = std::collections::BTreeSet::new();
        let mut created = 0;
        for (took, answer) in &answers {
            let started = answer.as_ref().expect("a start under USE_EXISTING answers");
            seen.insert(started.run.info.run_id);
            let bucket = if started.existing {
                &mut existing.other
            } else {
                created += 1;
                &mut existing.winner
            };
            if counted {
                bucket.push(ms(*took));
            }
        }
        assert_eq!(
            (created, seen.len()),
            (1, 1),
            "one run created and every start answered with it"
        );

        // 3. thirty-two identifiers, no contention
        let prefix = format!("own-{round}-{}", Uuid::now_v7());
        let answers = burst(
            &runs,
            b.workflow,
            &b.w.admin,
            |way| Some(format!("{prefix}-{way}")),
            ConflictPolicy::Fail,
        )
        .await;
        for (took, answer) in &answers {
            assert!(
                !answer
                    .as_ref()
                    .expect("a free identifier is claimed")
                    .existing
            );
            if counted {
                distinct.winner.push(ms(*took));
            }
        }

        // 4. thirty-two plain starts
        let answers = burst(
            &runs,
            b.workflow,
            &b.w.admin,
            |_| None,
            ConflictPolicy::Fail,
        )
        .await;
        for (took, answer) in &answers {
            assert!(!answer.as_ref().expect("a plain start").existing);
            if counted {
                plain.winner.push(ms(*took));
            }
        }
    }
    sizes[0] = refuse.winner.len() + refuse.other.len();
    sizes[1] = existing.winner.len() + existing.other.len();
    sizes[2] = distinct.winner.len();
    sizes[3] = plain.winner.len();

    // the rows say the same as the answers: for every hot identifier one open run
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT business_id FROM runs WHERE business_id LIKE 'hot-%' AND finished_at IS NULL \
         GROUP BY business_id HAVING count(*) > 1) t",
    )
    .fetch_one(&b.w.db.pool)
    .await
    .expect("count");
    assert_eq!(open, 0, "no identifier has two open runs");

    let all_fail: Vec<f64> = refuse.winner.iter().chain(&refuse.other).copied().collect();
    let claim = Summary::of(&distinct.winner).expect("samples");
    let plain_all = Summary::of(&plain.winner).expect("samples");
    let loser = Summary::of(&refuse.other).expect("samples");
    let fragment = json!({
        "ways": WAYS, "rounds_counted": rounds, "rounds_warm_up": WARM_UP_ROUNDS, "samples": sizes,
        "pool_per_replica": 48,
        "hot_identifier_fail_ms": {
            "all": summary_json(&all_fail), "the_run_that_wins": summary_json(&refuse.winner),
            "the_refused": summary_json(&refuse.other),
        },
        "hot_identifier_use_existing_ms": {
            "the_run_that_is_created": summary_json(&existing.winner), "the_answers_with_the_existing_run": summary_json(&existing.other),
        },
        "thirty_two_identifiers_ms": summary_json(&distinct.winner),
        "thirty_two_plain_starts_ms": summary_json(&plain.winner),
        "claim_over_plain_start": {"p50": claim.p50 / plain_all.p50, "p95": claim.p95 / plain_all.p95, "p99": claim.p99 / plain_all.p99},
        "refused_over_plain_start_p50": loser.p50 / plain_all.p50,
    });
    let verdicts = [
        Verdict::at_most(
            "a_start_that_waits_for_the_holder_is_answered_within_a_second",
            "p99 of the starts of a held identifier",
            loser.p99,
            1_000.0,
            "ms",
        ),
        Verdict::at_most(
            "the_claim_costs_little_at_the_median",
            "p50 of a start with its own identifier over a plain start",
            claim.p50 / plain_all.p50,
            1.5,
            "x",
        ),
        Verdict::at_most(
            "the_claim_costs_little_at_the_tail",
            "p95 of a start with its own identifier over a plain start",
            claim.p95 / plain_all.p95,
            2.0,
            "x",
        ),
    ];
    println!(
        "{scenario}: {WAYS}-way, {rounds} rounds: held-id refused p50 {:.1} ms p99 {:.1} ms; own id p50 {:.1} ms; plain p50 {:.1} ms",
        loser.p50, loser.p99, claim.p50, plain_all.p50
    );
    conclude(scenario, environment, before, fragment, &verdicts);
}
