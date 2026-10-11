//! US01 — start a workflow via webhook: a provider sends a signed event, the inbox confirms with
//! 202, and a run is created exactly once however many times the provider sends it (US01-AC01).

use reqwest::Method;
use serde_json::json;

use crate::support::worker::{platform, single_ir};
use crate::{SECRET, endpoint, engine, runs_of, webhook, within};

#[tokio::test]
async fn us01_a_signed_webhook_starts_a_workflow_exactly_once() {
    let p = platform().await;
    let _engine = engine(&p, &[]).await;
    let endpoint_id = endpoint(&p, "billing", json!({})).await;

    let mut ir = single_ir("on-invoice", None);
    ir["triggers"] = json!([{"kind": "webhook", "endpoint": "billing", "eventType": "invoice.paid",
                             "filter": "data.amount > 100"}]);
    let workflow = p.publish("on-invoice", &ir).await;

    // The provider is told 202 at once; what the event starts is decided afterwards.
    let big = json!({"invoice": "inv-7", "amount": 250});
    let (status, accepted) = webhook(&p, &endpoint_id, "evt-1", "invoice.paid", &big, SECRET).await;
    assert_eq!(
        (status, accepted["status"].as_str()),
        (202, Some("ACCEPTED")),
        "{accepted}"
    );

    let runs = within("the run to be started", 20, || async {
        let runs = runs_of(&p, &workflow).await;
        (!runs.is_empty()).then_some(runs)
    })
    .await;
    assert_eq!(runs.len(), 1);

    // The provider did not see our 202 and sends it again, as providers do: the same answer, one run.
    let (status, again) = webhook(&p, &endpoint_id, "evt-1", "invoice.paid", &big, SECRET).await;
    assert_eq!(
        (status, again["status"].as_str()),
        (202, Some("DUPLICATE")),
        "{again}"
    );
    // An invoice under the threshold is an event the workflow does not want.
    let small = json!({"invoice": "inv-8", "amount": 5});
    assert_eq!(
        webhook(&p, &endpoint_id, "evt-2", "invoice.paid", &small, SECRET)
            .await
            .0,
        202
    );

    // The inbox says what became of each event.
    let ns = p.namespace_id().await;
    let first = within("evt-1 to be dispatched", 20, || async {
        let (status, event) = p
            .rest(
                Method::GET,
                &format!("/namespaces/{ns}/events/billing/evt-1"),
                None,
            )
            .await;
        (status == 200 && event["status"] == "DISPATCHED").then_some(event)
    })
    .await;
    assert_eq!(first["deliveries"].as_array().expect("deliveries").len(), 1);
    assert!(
        first["deliveries"][0]["consumer"]
            .as_str()
            .expect("consumer")
            .starts_with("trigger:")
    );
    let second = within("evt-2 to be dispatched", 20, || async {
        let (_, event) = p
            .rest(
                Method::GET,
                &format!("/namespaces/{ns}/events/billing/evt-2"),
                None,
            )
            .await;
        (event["status"] == "DISPATCHED").then_some(event)
    })
    .await;
    assert!(
        second["deliveries"]
            .as_array()
            .expect("deliveries")
            .is_empty(),
        "nothing started for it"
    );
    assert_eq!(
        runs_of(&p, &workflow).await.len(),
        1,
        "one run, for the one event that qualified"
    );

    // A request that does not verify is the same 401 whatever is wrong with it, and starts nothing.
    let (status, refused) = webhook(
        &p,
        &endpoint_id,
        "evt-3",
        "invoice.paid",
        &big,
        "not-the-secret-not-the-secret-0000",
    )
    .await;
    assert_eq!(status, 401, "{refused}");
    assert_eq!(refused["message"], "the request could not be verified");
    let (unknown, _) = webhook(
        &p,
        "019b5d6e-0000-7000-8000-000000000000",
        "evt-3",
        "invoice.paid",
        &big,
        SECRET,
    )
    .await;
    assert_eq!(
        unknown, 401,
        "an endpoint that does not exist answers like a signature that does not verify"
    );
    assert_eq!(runs_of(&p, &workflow).await.len(), 1);
}
