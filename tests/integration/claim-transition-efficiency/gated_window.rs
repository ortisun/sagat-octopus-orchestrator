//! A gated step that waits is not in the claim's window (T022-099; FR-022-019, SC-022-024, FR-038-002; ADR-0089
//! decision 1): 64 steps of one key of an activity wait outside `READY` for a permit (and, in the second test, for a
//! slot) behind the key's leased holder, and one step of another key is ready. The next poll leases the other key's step,
//! and the claim's counters show it read, tried and opened nothing else: no candidate of the waiting key.
//!
//! Under ADR-0056 decision 3 as first written (the permit taken at claim), the 64 would be `READY` and older than the
//! other key's step: the window would be full of candidates the claim must refuse.

use serde_json::{Value, json};

use crate::support::engine::{World, activity, document, with};
use crate::support::flow_control::{gated_world, publish};

const WAITING: usize = 64;

/// `worker_claim_candidates_total{stage}` so far, zero before its first increment.
fn claim_stage(telemetry: &sagat_telemetry::Metrics, name: &str) -> u64 {
    telemetry
        .render()
        .lines()
        .find(|line| {
            line.starts_with("worker_claim_candidates_total{")
                && line.contains(&format!("stage=\"{name}\""))
        })
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn stages(telemetry: &sagat_telemetry::Metrics) -> [u64; 4] {
    ["fetched", "probed", "opened", "leased"].map(|stage| claim_stage(telemetry, stage))
}

/// Runs `body` on a runtime of its own with a metrics recorder local to it.
fn with_claim_counters(
    body: fn(
        &sagat_telemetry::Metrics,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + '_>>,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (telemetry, recorder) =
        sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
    metrics::with_local_recorder(&recorder, || runtime.block_on(body(&telemetry)));
}

/// `gate` on `input.accountId` of node `g` of activity `act.g`.
async fn one_key_waits_and_another_is_ready(
    telemetry: &sagat_telemetry::Metrics,
    name: &str,
    gate: (&str, Value),
) {
    let w: World = gated_world().await;
    let workflow = publish(
        &w,
        document(name, "g", vec![with(activity("g"), gate.0, gate.1)]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let key_a = json!({"accountId": "acct-a"});
    let holder = w.start(workflow, &key_a).await;
    let leased = w.poll(&session, 1).await;
    assert_eq!(leased.len(), 1);
    assert_eq!(leased[0].run_id, holder, "the key's holder is leased");
    for _ in 0..WAITING {
        w.start(workflow, &key_a).await;
    }
    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE node = 'g' AND status = 'WAITING' AND waiting_kind IS NOT NULL",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("waiting steps");
    assert_eq!(waiting, i64::try_from(WAITING).expect("count"));
    let other = w.start(workflow, &json!({"accountId": "acct-b"})).await;

    let before = stages(telemetry);
    let taken = w.poll(&session, 8).await;
    let after = stages(telemetry);
    assert_eq!(
        taken.iter().map(|task| task.run_id).collect::<Vec<_>>(),
        vec![other],
        "the next poll leases the other key's step"
    );
    let spent: Vec<u64> = after.iter().zip(before).map(|(a, b)| a - b).collect();
    assert_eq!(
        spent,
        vec![1, 1, 1, 1],
        "fetched, probed, opened, leased: one candidate, the other key's"
    );
}

/// Steps of key A waiting for a permit do not fill the claim's window.
#[test]
fn steps_waiting_for_a_permit_are_not_candidates_of_the_claim() {
    with_claim_counters(|telemetry| {
        Box::pin(one_key_waits_and_another_is_ready(
            telemetry,
            "window-permit",
            ("concurrency", json!({"key": "input.accountId", "limit": 1})),
        ))
    });
}

/// Steps of key A waiting for a slot do not fill the claim's window.
#[test]
fn steps_waiting_for_a_slot_are_not_candidates_of_the_claim() {
    with_claim_counters(|telemetry| {
        Box::pin(one_key_waits_and_another_is_ready(
            telemetry,
            "window-slot",
            (
                "rateLimit",
                json!({"key": "input.accountId", "limit": 1, "per": "hour"}),
            ),
        ))
    });
}
