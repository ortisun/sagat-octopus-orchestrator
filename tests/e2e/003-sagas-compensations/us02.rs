//! US02 — compensate after a permanent failure: the release is executed idempotently and the run
//! indicates COMPENSATED (US02-AC01).

use reqwest::Method;
use serde_json::json;

use crate::support::worker::{Worker, machine_code, platform};
use crate::{decide, graph, input, journal, node, payment_saga};

const ACTIVITIES: &[&str] = &[
    "payment.validate",
    "limit.reserve",
    "payment.submit",
    "limit.release",
    "payment.return.request",
    "payment.lookup.status",
    "customer.notify",
];

/// `validate` and `reserve` done; `submit` declined for good. Returns the lease `reserve` had.
async fn declined(
    p: &crate::support::worker::Platform,
    w: &mut Worker<'_>,
    run: &str,
) -> sagat_api_grpc::pb::TaskLease {
    let _ = run;
    let validate = w.one_task().await;
    assert_eq!(validate.activity_name, "payment.validate");
    w.complete(&validate, &json!({})).await.expect("validate");
    let reserve = w.one_task().await;
    assert_eq!(reserve.activity_name, "limit.reserve");
    w.complete(&reserve, &json!({"reservation": "r-77"}))
        .await
        .expect("reserve");
    let submit = w.one_task().await;
    assert_eq!(submit.activity_name, "payment.submit");
    w.fail(&submit, false).await.expect("declined");
    let _ = p;
    reserve
}

#[tokio::test]
async fn us02_a_declined_payment_releases_the_reserved_limit_and_the_run_is_compensated() {
    let p = platform().await;
    let wf = p.publish("corporate-payment", &payment_saga()).await;
    let run = p.start(&wf, &input()).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;
    let reserve = declined(&p, &mut w, &run).await;

    // The coordinator took over in the same transaction as the failure.
    assert_eq!(p.status(&run).await, "COMPENSATING");
    let mid = graph(&p, &run).await;
    assert_eq!(node(&mid, "settlement")["saga"]["phase"], "COMPENSATING");
    assert_eq!(
        node(&mid, "settlement")["saga"]["cause"],
        "PAYMENT_DECLINED"
    );
    assert_eq!(node(&mid, "reserve~c")["kind"], "COMPENSATION");
    assert_eq!(node(&mid, "reserve~c")["forward"], "reserve");
    assert_eq!(node(&mid, "reserve~c")["status"], "READY");
    assert_eq!(
        node(&mid, "submit~c")["status"],
        "NOT_STARTED",
        "nothing to undo for what never happened"
    );

    // The release is an ordinary task of the worker protocol, with what the provider needs.
    let release = w.one_task().await;
    assert_eq!(release.activity_name, "limit.release");
    assert_eq!(release.task_kind, "COMPENSATION");
    assert_eq!(release.target_node, "reserve");
    assert_eq!(release.target_idempotency_key, reserve.idempotency_key);
    assert_eq!(reserve.idempotency_key, format!("reserve:{run}"));
    assert!(
        release.idempotency_key.starts_with("compensation:"),
        "{}",
        release.idempotency_key
    );
    assert!(
        release.idempotency_key.ends_with(&format!(":{run}")),
        "{}",
        release.idempotency_key
    );
    assert_ne!(release.idempotency_key, reserve.idempotency_key);
    let data: serde_json::Value = serde_json::from_str(&release.input_json).expect("json");
    assert_eq!(
        data["steps"]["reserve"]["output"]["reservation"], "r-77",
        "{data}"
    );

    // The worker reports twice (its first acknowledgement was lost): the second is the first again.
    w.complete(&release, &json!({})).await.expect("released");
    w.complete(&release, &json!({}))
        .await
        .expect("the duplicate is acknowledged");
    assert_eq!(p.status(&run).await, "COMPENSATED");

    let events = journal(&p, &run).await;
    let completed = events
        .iter()
        .filter(|(kind, step)| kind == "step.completed" && step.as_deref() == Some("reserve~c"))
        .count();
    assert_eq!(completed, 1, "the release is recorded once: {events:?}");
    let done = graph(&p, &run).await;
    assert_eq!(node(&done, "settlement")["saga"]["outcome"], "COMPENSATED");
    assert_eq!(
        node(&done, "wait_confirmation")["status"],
        "NOT_STARTED",
        "nothing after the saga ran"
    );
    // The run is over: no worker is handed anything else.
    assert!(w.poll(8).await.is_empty());
}

#[tokio::test]
async fn us02_a_worker_that_dies_while_releasing_is_replaced_and_the_provider_sees_one_key() {
    let p = platform().await;
    let wf = p.publish("corporate-payment", &payment_saga()).await;
    let run = p.start(&wf, &input()).await;
    let mut doomed = p.worker("wendy").await;
    doomed.register(ACTIVITIES).await;
    declined(&p, &mut doomed, &run).await;
    let first = doomed.one_task().await;
    assert_eq!(first.activity_name, "limit.release");

    // It dies holding the lease; the lease runs out and the retry falls due.
    p.expire_lease(&run, "reserve~c").await;
    p.tick().await;
    p.elapse_retry_delay(&run, "reserve~c").await;
    p.tick().await;
    let mut rescuer = p.worker("rhea").await;
    rescuer.register(ACTIVITIES).await;
    let second = rescuer.one_task().await;
    assert_eq!(second.activity_name, "limit.release");
    assert_eq!(second.attempt, first.attempt + 1);
    assert!(second.generation > first.generation);
    assert_eq!(
        second.idempotency_key, first.idempotency_key,
        "the provider recognises the repeat"
    );

    let late = doomed
        .complete(&first, &json!({}))
        .await
        .expect_err("stale");
    assert_eq!(machine_code(&late), "STALE_ATTEMPT");
    rescuer
        .complete(&second, &json!({}))
        .await
        .expect("released");
    assert_eq!(p.status(&run).await, "COMPENSATED");
}

#[tokio::test]
async fn us02_cancelling_a_payment_in_flight_undoes_what_completed_and_ends_cancelled() {
    let p = platform().await;
    let wf = p.publish("corporate-payment", &payment_saga()).await;
    let run = p.start(&wf, &input()).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;
    let validate = w.one_task().await;
    w.complete(&validate, &json!({})).await.expect("validate");
    let reserve = w.one_task().await;
    w.complete(&reserve, &json!({"reservation": "r-1"}))
        .await
        .expect("reserve");
    let submit = w.one_task().await;

    let (status, body) = decide(
        &p,
        None,
        &run,
        &json!({"action": "CANCEL", "reason": "customer called it off"}),
    )
    .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(
        body["runStatus"], "COMPENSATING",
        "not CANCELLED until the limit is released"
    );

    // The attempt in flight is asked to stop; the worker obeys.
    let request = w.req(w.heartbeat_message(&submit));
    let beat = w
        .client
        .heartbeat(request)
        .await
        .expect("heartbeat")
        .into_inner();
    assert!(beat.cancel_requested);
    w.fail_with(&submit, "CANCELLED", true)
        .await
        .expect("stops");
    let release = w.one_task().await;
    assert_eq!(
        (release.activity_name.as_str(), release.task_kind.as_str()),
        ("limit.release", "COMPENSATION")
    );
    w.complete(&release, &json!({})).await.expect("released");
    assert_eq!(p.status(&run).await, "CANCELLED");
    let (status, run_body) = p.rest(Method::GET, &format!("/runs/{run}"), None).await;
    assert_eq!(status, 200, "{run_body}");
}
