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

impl Rig {
    /// Ends a run the way an operator does.
    async fn cancel(&self, run: &str) {
        let (status, body) = self
            .p
            .rest(
                Method::POST,
                &format!("/runs/{run}/operations"),
                Some(&json!({"action": "CANCEL", "reason": "the customer changed their mind"})),
            )
            .await;
        assert!(status == 200 || status == 202, "{status} {body}");
    }

    async fn rerun(&self, run: &str, extra: Value) -> (u16, Value) {
        let mut body = json!({"action": "RERUN_AS_NEW", "reason": "start it again after the fix"});
        for (key, value) in extra.as_object().expect("an object") {
            body[key] = value.clone();
        }
        self.p
            .rest(
                Method::POST,
                &format!("/runs/{run}/operations"),
                Some(&body),
            )
            .await
    }

    async fn run(&self, id: &str) -> Value {
        let (status, run) = self.p.rest(Method::GET, &format!("/runs/{id}"), None).await;
        assert_eq!(status, 200, "{run}");
        run
    }
}

#[tokio::test]
async fn a_rerun_keeps_the_identifier_unless_it_is_asked_to_drop_it() {
    let r = rig().await;
    let (_, first) = r.start(json!({"businessId": "rerun-http-1"})).await;
    let first = first["runId"].as_str().expect("id").to_owned();
    r.cancel(&first).await;

    let (status, kept) = r.rerun(&first, json!({})).await;
    assert_eq!(status, 202, "{kept}");
    let kept_run = kept["newRunId"].as_str().expect("the new run").to_owned();
    assert_eq!(r.run(&kept_run).await["businessId"], "rerun-http-1");
    assert_eq!(
        r.runs("businessId=rerun-http-1").await.len(),
        2,
        "the source and the rerun"
    );

    r.cancel(&kept_run).await;
    let (status, explicit) = r.rerun(&first, json!({"businessId": "KEEP"})).await;
    assert_eq!(status, 202, "{explicit}");
    let (status, dropped) = r.rerun(&first, json!({"businessId": "DROP"})).await;
    assert_eq!(status, 202, "{dropped}");
    let dropped_run = dropped["newRunId"].as_str().expect("the new run");
    assert!(
        r.run(dropped_run).await.get("businessId").is_none(),
        "a dropped identifier is not on the run"
    );
}

#[tokio::test]
async fn a_rerun_that_keeps_a_held_identifier_is_refused_naming_the_holder() {
    let r = rig().await;
    let (_, first) = r.start(json!({"businessId": "rerun-http-2"})).await;
    let first = first["runId"].as_str().expect("id").to_owned();
    r.cancel(&first).await;
    let (_, holder) = r.start(json!({"businessId": "rerun-http-2"})).await;

    let (status, refused) = r.rerun(&first, json!({})).await;
    assert_eq!(status, 409, "{refused}");
    assert_eq!(reason(&refused), "BUSINESS_ID_IN_USE");
    assert_eq!(refused["details"]["runId"], holder["runId"]);
    assert_eq!(r.runs("businessId=rerun-http-2").await.len(), 2);

    let (status, dropped) = r.rerun(&first, json!({"businessId": "DROP"})).await;
    assert_eq!(
        status, 202,
        "dropping needs nothing from the binding: {dropped}"
    );
}

#[tokio::test]
async fn the_identifier_choice_of_an_operation_is_checked() {
    let r = rig().await;
    let (_, first) = r.start(json!({"businessId": "rerun-http-3"})).await;
    let first = first["runId"].as_str().expect("id").to_owned();
    r.cancel(&first).await;

    let (status, refused) = r.rerun(&first, json!({"businessId": "SOMETIMES"})).await;
    assert_eq!(status, 422, "{refused}");
    // it belongs to a rerun and to nothing else
    let (status, refused) =
        r.p.rest(
            Method::POST,
            &format!("/runs/{first}/operations"),
            Some(&json!({
                "action": "PAUSE",
                "reason": "hold it while we look",
                "businessId": "DROP"
            })),
        )
        .await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(r.runs("businessId=rerun-http-3").await.len(), 1);
}

// --- the lookup of a binding -------------------------------------------------------------------

/// A path segment: everything that is not unreserved is percent-encoded, so an identifier with a
/// `/`, a `%`, a `#` or a `?` in it arrives whole.
fn segment(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

impl Rig {
    async fn binding(&self, id: &str) -> (u16, Value) {
        let ns = self.p.namespace_id().await;
        self.p
            .rest(
                Method::GET,
                &format!("/namespaces/{ns}/business-ids/{}", segment(id)),
                None,
            )
            .await
    }
}

#[tokio::test]
async fn a_binding_is_read_by_its_identifier_open_and_then_closed() {
    let r = rig().await;
    let (_, run) = r.start(json!({"businessId": "lookup-http-1"})).await;
    let run = run["runId"].as_str().expect("id").to_owned();

    let (status, open) = r.binding("lookup-http-1").await;
    assert_eq!(status, 200, "{open}");
    assert_eq!(open["businessId"], "lookup-http-1");
    assert_eq!(open["executionId"], run);
    assert_eq!(open["currentRunId"], run);
    assert_eq!(open["open"], true);
    assert!(open["lastStatus"].is_null(), "{open}");
    assert!(open["reusableAfter"].is_null(), "{open}");
    assert!(open["workflowId"].is_string() && open["boundAt"].is_string());

    r.cancel(&run).await;
    let (status, closed) = r.binding("lookup-http-1").await;
    assert_eq!(status, 200, "{closed}");
    assert_eq!(closed["open"], false);
    assert_eq!(closed["lastStatus"], "CANCELLED");
    assert!(
        closed["reusableAfter"].is_string(),
        "it says until when it is remembered: {closed}"
    );
}

#[tokio::test]
async fn an_identifier_with_reserved_characters_is_read_whole() {
    let r = rig().await;
    let odd = "a/b%c#d?e+f&g";
    let (status, run) = r.start(json!({"businessId": odd})).await;
    assert_eq!(status, 202, "{run}");

    let (status, binding) = r.binding(odd).await;
    assert_eq!(status, 200, "{binding}");
    assert_eq!(binding["businessId"], odd);
    assert_eq!(binding["executionId"], run["runId"]);
}

#[tokio::test]
async fn an_unbound_identifier_and_a_namespace_that_is_not_one_are_not_found() {
    let r = rig().await;
    r.start(json!({"businessId": "lookup-http-2"})).await;

    let (status, absent) = r.binding("never-bound").await;
    assert_eq!(status, 404, "{absent}");
    assert_eq!(absent["code"], "NOT_FOUND");
    assert_ne!(
        absent["details"]["reason"], "ROUTE_NOT_FOUND",
        "the route exists and the identifier does not"
    );
    assert!(
        !absent.to_string().contains("never-bound"),
        "the identifier is not echoed: {absent}"
    );

    for namespace in ["not-a-namespace", &uuid::Uuid::now_v7().to_string()] {
        let (status, other) =
            r.p.rest(
                Method::GET,
                &format!("/namespaces/{namespace}/business-ids/lookup-http-2"),
                None,
            )
            .await;
        assert_eq!(status, 404, "{namespace}: {other}");
        assert_ne!(other["details"]["reason"], "ROUTE_NOT_FOUND", "{namespace}");
    }
}

#[tokio::test]
async fn another_tenant_probing_a_binding_finds_nothing_and_the_probe_is_on_record() {
    let r = rig().await;
    let (_, run) = r.start(json!({"businessId": "probe-1"})).await;
    let namespace = r.p.namespace_id().await;
    let denials = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_records a JOIN tenants t ON t.id = a.tenant_id \
             WHERE t.slug = 'globex' AND a.action = 'access.denied.cross_tenant'",
        )
        .fetch_one(&r.p.api.db.pool)
        .await
        .expect("count")
    };
    let before = denials().await;

    let response =
        r.p.api
            .as_user(
                Method::GET,
                &format!("/namespaces/{namespace}/business-ids/probe-1"),
                "bob",
                "globex",
            )
            .send()
            .await
            .expect("send");

    assert_eq!(response.status().as_u16(), 404);
    let body = response.text().await.expect("body");
    assert!(
        !body.contains("probe-1") && !body.contains(run["runId"].as_str().expect("id")),
        "the answer shows nothing of the victim: {body}"
    );
    let mut recorded = denials().await;
    for _ in 0..50 {
        if recorded > before {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        recorded = denials().await;
    }
    assert_eq!(
        recorded,
        before + 1,
        "the probe of another tenant is recorded against it"
    );
}
