//! Business identifiers over HTTP (`T018-035`, `FR-018-018..020`, `FR-018-022`, `FR-018-043`): the
//! start options, `200` for a holder returned in place of a new run, the typed refusals, and the
//! list filters. A real API, a real PostgreSQL, real tokens.
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use serde_json::{Value, json};

use crate::support::worker::{Platform, chain_ir, platform};

struct Rig {
    p: Platform,
    workflow: String,
}

async fn rig() -> Rig {
    let p = platform().await;
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    Rig { p, workflow }
}

impl Rig {
    async fn start(&self, extra: Value) -> (u16, Value) {
        let mut body = json!({"workflowId": self.workflow, "input": {"item": "book"}});
        for (key, value) in extra.as_object().expect("an object") {
            body[key] = value.clone();
        }
        self.p.rest(Method::POST, "/runs", Some(&body)).await
    }

    async fn runs(&self, query: &str) -> Vec<Value> {
        let (status, page) = self
            .p
            .rest(Method::GET, &format!("/runs?{query}"), None)
            .await;
        assert_eq!(status, 200, "{query}: {page}");
        page["items"].as_array().expect("items").clone()
    }
}

fn reason(error: &Value) -> &str {
    error["details"]["reason"].as_str().unwrap_or_default()
}

#[tokio::test]
async fn a_start_carries_its_identifier_the_run_says_so_and_a_second_start_is_refused_naming_the_holder()
 {
    let r = rig().await;
    let (status, run) = r.start(json!({"businessId": "order-1001"})).await;
    assert_eq!(status, 202, "{run}");
    assert_eq!(run["businessId"], "order-1001");
    let id = run["runId"].as_str().expect("id").to_owned();
    let (_, read) = r.p.rest(Method::GET, &format!("/runs/{id}"), None).await;
    assert_eq!(read["businessId"], "order-1001");

    let (status, refused) = r.start(json!({"businessId": "order-1001"})).await;
    assert_eq!(status, 409, "{refused}");
    assert_eq!(refused["code"], "CONFLICT");
    assert_eq!(reason(&refused), "BUSINESS_ID_IN_USE");
    assert_eq!(
        refused["details"]["runId"], id,
        "the holder is named to a caller who may read runs"
    );
    assert!(
        !refused.to_string().contains("order-1001"),
        "the identifier is not echoed: {refused}"
    );
}

#[tokio::test]
async fn use_existing_answers_200_with_the_holder_and_creates_nothing() {
    let r = rig().await;
    let (_, held) = r.start(json!({"businessId": "invoice-7"})).await;
    let (status, again) = r
        .start(json!({
            "businessId": "invoice-7",
            "businessIdConflictPolicy": "USE_EXISTING"
        }))
        .await;
    assert_eq!(status, 200, "{again}");
    assert_eq!(again["runId"], held["runId"]);
    assert_eq!(r.runs("businessId=invoice-7").await.len(), 1);
}

#[tokio::test]
async fn the_reuse_policy_refuses_over_http_with_the_instant_it_ends() {
    let r = rig().await;
    let (_, held) = r.start(json!({"businessId": "ticket-9"})).await;
    let id = held["runId"].as_str().expect("id").to_owned();
    let (status, _) =
        r.p.rest(
            Method::POST,
            &format!("/runs/{id}/operations"),
            Some(&json!({"action": "CANCEL", "reason": "the customer changed their mind"})),
        )
        .await;
    assert!(status == 200 || status == 202, "{status}");
    let (status, refused) = r
        .start(json!({
            "businessId": "ticket-9",
            "businessIdReusePolicy": "REJECT_DUPLICATE"
        }))
        .await;
    assert_eq!(status, 409, "{refused}");
    assert_eq!(reason(&refused), "BUSINESS_ID_REUSE_REJECTED");
    assert!(
        refused["details"]["reusableAfter"].is_string(),
        "it says until when: {refused}"
    );
    // and with the default policy the same start is allowed
    let (status, again) = r.start(json!({"businessId": "ticket-9"})).await;
    assert_eq!(status, 202, "{again}");
}

#[tokio::test]
async fn an_option_that_is_none_is_refused_in_words_and_nothing_is_bound() {
    let r = rig().await;
    let too_long = "n".repeat(201);
    for (body, field) in [
        (json!({"businessId": ""}), "/businessId"),
        (json!({"businessId": "has a space"}), "/businessId"),
        (json!({"businessId": "tab\there"}), "/businessId"),
        (json!({"businessId": "ünïcode"}), "/businessId"),
        (json!({"businessId": too_long}), "/businessId"),
        (
            json!({"businessId": "ok-1", "businessIdConflictPolicy": "TERMINATE_EXISTING"}),
            "/businessIdConflictPolicy",
        ),
        (
            json!({"businessId": "ok-1", "businessIdReusePolicy": "SOMETIMES"}),
            "/businessIdReusePolicy",
        ),
        (
            json!({"businessIdConflictPolicy": "USE_EXISTING"}),
            "/businessId",
        ),
    ] {
        let (status, refused) = r.start(body.clone()).await;
        assert_eq!(status, 422, "{body}: {refused}");
        assert_eq!(reason(&refused), "BUSINESS_ID_INVALID", "{body}");
        assert_eq!(refused["details"]["field"], field, "{body}");
    }
    assert!(
        r.runs("").await.is_empty(),
        "no run was created by any of them"
    );
}

#[tokio::test]
async fn an_identifier_cannot_ride_with_an_uploaded_input() {
    let r = rig().await;
    let (status, refused) =
        r.p.rest(
            Method::POST,
            "/runs",
            Some(&json!({
                "workflowId": r.workflow,
                "inputBlobRef": "0194f5e2-7b1c-7000-8000-000000000001",
                "businessId": "upload-1"
            })),
        )
        .await;
    assert_eq!(status, 422, "{refused}");
}

#[tokio::test]
async fn runs_are_listed_by_identifier_and_by_the_start_of_one_and_a_prefix_has_no_wildcards() {
    let r = rig().await;
    for id in ["order-1", "order-2", "order_3", "order%4", "invoice-1"] {
        let (status, run) = r.start(json!({"businessId": id})).await;
        assert_eq!(status, 202, "{id}: {run}");
    }
    r.start(json!({})).await;

    let ids = |runs: Vec<Value>| {
        let mut ids: Vec<String> = runs
            .iter()
            .map(|run| run["businessId"].as_str().expect("an id").to_owned())
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(r.runs("businessId=order-2").await), ["order-2"]);
    assert_eq!(
        ids(r.runs("businessIdPrefix=order").await),
        ["order%4", "order-1", "order-2", "order_3"]
    );
    assert_eq!(
        ids(r.runs("businessIdPrefix=order_").await),
        ["order_3"],
        "an underscore is an underscore, not any character"
    );
    assert_eq!(
        ids(r.runs("businessIdPrefix=order%25").await),
        ["order%4"],
        "a percent sign is a percent sign"
    );
    assert!(
        r.runs("businessId=order").await.is_empty(),
        "whole, not a prefix"
    );
    assert_eq!(r.runs("").await.len(), 6);

    for query in ["businessId=", "businessIdPrefix=", "businessId=has%20space"] {
        let (status, refused) = r.p.rest(Method::GET, &format!("/runs?{query}"), None).await;
        assert_eq!(status, 422, "{query}: {refused}");
        assert_eq!(reason(&refused), "BUSINESS_ID_INVALID", "{query}");
    }
}
