//! T042-9 (NFR-042-001, SC-042-008): what a chain adds to scope entry, measured, not asserted. Ignored by default; run
//! with `cargo test -p sagat-tests --test integration_workflow_primitives -- --ignored collection_operators_measure
//! --nocapture` and copy the printed lines into `specs/042-stream-operators/validation-report.md` with the machine.
//!
//! Three entries, each timed over `ROUNDS` runs: a `forEach` of 1,000 small elements without a chain, the same with
//! eight stages, and the worst case at the ceilings (eight `map` stages over a blob of sixteen 256 KiB elements, each
//! stage 4 MiB). The tick that carries the worst-case entry is timed too, the duration `scheduler_tick_seconds` records.

use std::time::{Duration, Instant};

use sagat_domain::execution::RunId;
use sagat_telemetry::Metrics;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::collection_operators::{Mode, Watched, chained_ir, metric, owner, replica};
use crate::support::blob_collections::{
    blob_source, collection_ir, platform_with_operators, start_inline, start_with_blob, upload,
};
use crate::support::worker::Platform;

const ROUNDS: usize = 20;

fn percentile(samples: &mut [Duration], p: f64) -> Duration {
    samples.sort();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let index = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples[index]
}

fn report(name: &str, samples: &mut [Duration]) {
    let p50 = percentile(samples, 0.5);
    let p95 = percentile(samples, 0.95);
    let max = percentile(samples, 1.0);
    println!(
        "T042-9 {name}: n={} p50={:.2} ms p95={:.2} ms max={:.2} ms",
        samples.len(),
        p50.as_secs_f64() * 1e3,
        p95.as_secs_f64() * 1e3,
        max.as_secs_f64() * 1e3
    );
}

/// Times the entry of `ROUNDS` runs started by `start`, one replica for all.
async fn entries<F, Fut>(p: &Platform, blob: bool, mut start: F) -> Vec<Duration>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Uuid>,
{
    let (scheduler, keeper) =
        replica(p, Watched::new(Mode::Apply), blob.then(|| blob_source(p))).await;
    let mut samples = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let run = start().await;
        let started = Instant::now();
        let entered = scheduler
            .advance_collection_from_run(p.tenant, RunId::new(run), &owner())
            .await;
        samples.push(started.elapsed());
        assert!(
            matches!(entered, Ok(sagat_scheduler::CollectionAdvance::Entered)),
            "{entered:?}"
        );
    }
    keeper.leave().await;
    samples
}

fn eight_small_stages() -> Value {
    json!([
        {"map": {"select": "element + 1"}},
        {"filter": {"where": "element > 0"}},
        {"map": {"select": "element * 2"}},
        {"distinct": {}},
        {"map": {"select": "element - 1"}},
        {"skip": {"count": 0}},
        {"map": {"select": "element"}},
        {"take": {"count": 1000}}
    ])
}

#[tokio::test]
#[ignore = "a measurement (T042-9): run on purpose and record the result"]
async fn collection_operators_measure() {
    let p = platform_with_operators().await;
    let lines: Vec<u32> = (0..1000).collect();
    let input = json!({ "lines": lines });

    let plain = p
        .publish(
            "measure-plain",
            &collection_ir("measure-plain", "input.lines", 1000),
        )
        .await;
    let mut samples = entries(&p, false, || start_inline(&p, &plain, &input)).await;
    report("entry without a chain, 1,000 small elements", &mut samples);

    let small = p
        .publish(
            "measure-eight",
            &chained_ir("measure-eight", "input.lines", &eight_small_stages(), 1000),
        )
        .await;
    let mut samples = entries(&p, false, || start_inline(&p, &small, &input)).await;
    report(
        "entry with eight stages, 1,000 small elements",
        &mut samples,
    );

    // The worst case: each of the eight `map` stages produces 4 MiB (sixteen elements at 256 KiB).
    let worst_stages = Value::Array(vec![json!({"map": {"select": "element"}}); 8]);
    let worst = p
        .publish(
            "measure-worst",
            &chained_ir("measure-worst", "input", &worst_stages, 20),
        )
        .await;
    let element = Value::String("x".repeat(256 * 1024 - 2));
    let source = serde_json::to_vec(&vec![element; 16]).expect("json");
    let mut blobs = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        blobs.push(upload(&p, &source).await);
    }
    let mut next = blobs.into_iter();
    let mut samples = entries(&p, true, || {
        let blob = next.next().expect("a blob per round");
        let p = &p;
        let worst = worst.clone();
        async move { start_with_blob(p, &worst, blob["id"].as_str().expect("id")).await }
    })
    .await;
    report(
        "entry at the ceilings, eight map stages of 4 MiB over a blob",
        &mut samples,
    );

    // The tick that carries one worst-case entry, against a tick that carries one entry without a chain.
    let metrics = Metrics::global();
    let before = metrics.render();
    let mut with_chain = Vec::with_capacity(5);
    let mut without = Vec::with_capacity(5);
    // Each tick in a platform of its own, so that it carries that one entry and nothing left by another.
    for _ in 0..5 {
        let q = platform_with_operators().await;
        let worst = q
            .publish(
                "measure-tick-worst",
                &chained_ir("measure-tick-worst", "input", &worst_stages, 20),
            )
            .await;
        let blob = upload(&q, &source).await;
        let run = start_with_blob(&q, &worst, blob["id"].as_str().expect("id")).await;
        let (scheduler, _keeper) =
            replica(&q, Watched::new(Mode::Apply), Some(blob_source(&q))).await;
        let started = Instant::now();
        scheduler.tick().await;
        with_chain.push(started.elapsed());
        assert_eq!(crate::collection_operators::scope_of(&q, run).await.0, 16);

        let q = platform_with_operators().await;
        let plain = q
            .publish(
                "measure-tick-plain",
                &collection_ir("measure-tick-plain", "input.lines", 1000),
            )
            .await;
        let run = start_inline(&q, &plain, &input).await;
        let (scheduler, _keeper) = replica(&q, Watched::new(Mode::Apply), None).await;
        let started = Instant::now();
        scheduler.tick().await;
        without.push(started.elapsed());
        assert_eq!(crate::collection_operators::scope_of(&q, run).await.0, 1000);
    }
    report("tick with one worst-case entry", &mut with_chain);
    report("tick with one entry without a chain", &mut without);
    let after = metrics.render();
    for reason in ["fence", "revision", "source_unavailable", "storage_quota"] {
        let labels = [("reason", reason)];
        println!(
            "T042-9 collection_chain_reentries_total{{reason=\"{reason}\"}} during the measurement: {}",
            metric(&after, "collection_chain_reentries_total", &labels)
                - metric(&before, "collection_chain_reentries_total", &labels)
        );
    }
}
