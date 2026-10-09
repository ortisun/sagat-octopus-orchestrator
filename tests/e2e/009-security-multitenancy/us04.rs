//! US04: a namespace that exhausts its limit gets "quota exceeded" for new requests, without
//! affecting other tenants (FR-009-006).

use reqwest::Method;
use serde_json::json;

use crate::support::worker::{chain_ir, platform};

#[tokio::test]
async fn a_full_namespace_answers_quota_exceeded_and_other_tenants_carry_on() {
    let p = platform().await;
    let ns = p.namespace_id().await;
    let wf = p.publish("chain-flow", &chain_ir("chain-flow")).await;

    // The administrator limits the namespace to two active runs.
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/quotas/active_runs"),
            Some(&json!({"limit": 2})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (body["limit"].as_u64(), body["source"].as_str()),
        (Some(2), Some("namespace"))
    );

    p.start(&wf, &json!({})).await;
    p.start(&wf, &json!({})).await;
    let (status, refused) = p
        .rest(
            Method::POST,
            "/runs",
            Some(&json!({"workflowId": wf, "input": {}})),
        )
        .await;
    assert_eq!(status, 429, "{refused}");
    assert_eq!(refused["code"], "QUOTA_EXCEEDED");
    assert_eq!(refused["retryable"], false);
    assert!(refused["traceId"].is_string());

    // The view shows the exact use.
    let (status, listed) = p
        .rest(Method::GET, &format!("/namespaces/{ns}/quotas"), None)
        .await;
    assert_eq!(status, 200);
    let active = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|q| q["resource"] == "active_runs")
        .expect("active runs");
    assert_eq!(
        (active["limit"].as_u64(), active["used"].as_u64()),
        (Some(2), Some(2))
    );

    // The other tenant is not affected: it publishes and starts as many as it likes.
    let globex_ws = p.api.create_workspace("bob", "globex", "platform").await;
    p.api
        .create_namespace("bob", "globex", &globex_ws, "payments", "dev")
        .await;
    let created = p
        .api
        .as_user(Method::POST, "/workflows", "bob", "globex")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "key-globex-create-0001")
        .json(&json!({"name": "chain-flow", "ir": chain_ir("chain-flow")}))
        .send()
        .await
        .expect("send");
    assert_eq!(created.status(), 201);
    let id = created.json::<serde_json::Value>().await.expect("json")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let published = p
        .api
        .as_user(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            "bob",
            "globex",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "key-globex-publish-0001")
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("send");
    assert_eq!(published.status(), 201);
    for n in 0..5 {
        let started = p
            .api
            .as_user(Method::POST, "/runs", "bob", "globex")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", format!("key-globex-start-{n:04}"))
            .json(&json!({"workflowId": id, "input": {}}))
            .send()
            .await
            .expect("send");
        assert_eq!(started.status(), 202, "globex run {n}");
    }

    // The administrator of the full namespace cannot lift the cap that protects the others by
    // setting it out of reach: the platform grants the tenant a ceiling, and nothing here raises it.
    for (path, limit) in [
        (
            format!("/namespaces/{ns}/quotas/active_runs"),
            1_000_000_000_u64,
        ),
        ("/quotas/tenant_active_runs".to_owned(), 1_000_000_000),
        ("/quotas/active_runs".to_owned(), 10_001),
    ] {
        let (status, refused) = p
            .rest(Method::PUT, &path, Some(&json!({"limit": limit})))
            .await;
        assert_eq!(status, 422, "{path}: {refused}");
    }
    let (_, still) = p
        .rest(Method::GET, &format!("/namespaces/{ns}/quotas"), None)
        .await;
    let active = still["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|q| q["resource"] == "active_runs")
        .expect("active runs");
    assert_eq!(
        (active["limit"].as_u64(), active["ceiling"].as_u64()),
        (Some(2), Some(10_000)),
        "the refused values changed nothing"
    );

    // Raising the limit, within the ceiling, takes effect at once.
    let (status, _) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/quotas/active_runs"),
            Some(&json!({"limit": 3})),
        )
        .await;
    assert_eq!(status, 200);
    p.start(&wf, &json!({})).await;
}
