//! Gaps G016-002 and G016-003, found by the security review of bugfix T016-066: a write whose answer depends on
//! whether a guess equals a payload someone else stored confirms a classified value online, as a digest did
//! offline (FR-016-030). These tests say what must hold and are seen failing on this branch; they are ignored
//! until the bugfix tasks that own them (T016-067, T016-068) land, and each says so.
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use super::support::worker::{HUMAN, NAMESPACE, Platform, TENANT, platform};

async fn post_keyed(
    p: &Platform,
    subject: &str,
    path: &str,
    key: &str,
    body: &Value,
) -> (u16, Value) {
    let response = p
        .api
        .as_user(Method::POST, path, subject, TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", key)
        .json(body)
        .send()
        .await
        .expect("send");
    let status = response.status().as_u16();
    let text = response.text().await.expect("body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// G016-002: the idempotency scope of a start is the namespace and its fingerprint holds the input, not the caller.
/// Another caller who reuses the key learns whether a guessed input equals the stored one: a right guess replays
/// (with the run id), a wrong one is `409 IDEMPOTENCY_KEY_REUSED`.
#[tokio::test]
#[ignore = "red: G016-002, fixed by bugfix T016-067 (the idempotency scope of start, signal and update must hold the caller)"]
async fn another_caller_reusing_a_start_key_learns_nothing_of_the_input() {
    let p = platform().await;
    // Two callers who are not the author each try one guess once, so the test holds whichever way the key is scoped
    // by T016-067: neither may learn more than the other.
    p.grant(p.tenant, "olivia", Role::Operator).await;
    p.grant(p.tenant, "oscar", Role::Operator).await;
    let mut ir = super::support::engine::document(
        "classified-start-key",
        "done",
        vec![super::support::engine::end_node("done")],
    );
    ir["inputSchema"] = json!({
        "type": "object", "additionalProperties": false, "x-sagat-classification": "public",
        "properties": {"flag": {"type": "boolean", "x-sagat-classification": "pii"}}
    });
    let workflow = p.publish("classified-start-key", &ir).await;
    let key = "order-12345";
    let start = |flag: bool| json!({"workflowId": workflow, "input": {"flag": flag}});
    let (status, first) = post_keyed(&p, HUMAN, "/runs", key, &start(true)).await;
    assert_eq!(status, 202, "{first}");
    let (right_status, right) = post_keyed(&p, "olivia", "/runs", key, &start(true)).await;
    let (wrong_status, wrong) = post_keyed(&p, "oscar", "/runs", key, &start(false)).await;
    assert_eq!(
        right_status, wrong_status,
        "a right and a wrong guess of a pii input are told apart: {right} / {wrong}"
    );
    for answer in [&right, &wrong] {
        assert_ne!(
            answer["runId"], first["runId"],
            "another caller's key replays their run: {answer}"
        );
    }
}

/// G016-003: every API publisher of a namespace shares the source `api`, and a repeated event id is compared by
/// payload digest: a right guess is a `202` duplicate, a wrong one a `409`.
#[tokio::test]
#[ignore = "red: G016-003, fixed by bugfix T016-068 (an API event id must be deduplicated per publisher)"]
async fn another_publisher_reusing_an_event_id_learns_nothing_of_the_data() {
    let p = platform().await;
    // Two publishers who are not the author each try one guess once (see the start test).
    p.grant(p.tenant, "devon", Role::Developer).await;
    p.grant(p.tenant, "dana", Role::Developer).await;
    let event = |personal: &str| json!({"eventId": "evt-guess-1", "type": "classification.demo", "data": {"personal": personal}});
    let (status, first) = post_keyed(&p, HUMAN, "/events", "evt-key-1", &event("1970-01-01")).await;
    assert_eq!(status, 202, "{first}");
    let (right_status, right) =
        post_keyed(&p, "devon", "/events", "evt-key-2", &event("1970-01-01")).await;
    let (wrong_status, wrong) =
        post_keyed(&p, "dana", "/events", "evt-key-3", &event("1970-01-02")).await;
    assert_eq!(
        (right_status, right.get("status").cloned()),
        (wrong_status, wrong.get("status").cloned()),
        "a right and a wrong guess of an event's data are told apart: {right} / {wrong}"
    );
}
