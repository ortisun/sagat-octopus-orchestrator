//! Live conformance of the business identifier options of `startRun`, `getRun` and `listRuns`
//! (`T018-035`, `FR-018-043`): the answers the API really gives to a start with an identifier use a
//! documented status, conform to the documented schema and carry the documented headers.

use reqwest::Method;
use serde_json::{Value, json};

use crate::support::api::start;
use crate::support::contract::{OpenApi, Scenario, check_body, scenario};

fn keyed(s: Scenario, n: u32) -> Scenario {
    s.with_header("idempotency-key", format!("business-key-{n:04}"))
        .with_header("x-octopus-namespace", "payments")
}

fn namespaced(s: Scenario) -> Scenario {
    s.with_header("x-octopus-namespace", "payments")
}

#[tokio::test]
async fn a_start_with_a_business_identifier_conforms_to_the_contract_on_every_answer() {
    let spec = OpenApi::load();
    let api = start().await;
    api.tenant("acme", "alice").await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    let namespace = api
        .create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;
    let flow = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "orders"}, "entrypoint": "a",
        "nodes": [{"id": "a", "type": "activity", "activity": "act.a"}]
    });
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "business-setup-0001")
        .json(&json!({"name": "orders", "ir": flow}))
        .send()
        .await
        .expect("send");
    assert_eq!(created.status(), 201);
    let workflow = created.json::<Value>().await.expect("json")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let published = api
        .as_user(
            Method::POST,
            &format!("/workflows/{workflow}/publish"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "business-setup-0002")
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("send");
    assert_eq!(published.status(), 201);

    let alice = crate::support::contract::As::User("alice", "acme");
    let body = |extra: Value| {
        let mut body = json!({"workflowId": workflow, "input": {"item": "book"}});
        for (key, value) in extra.as_object().expect("an object") {
            body[key] = value.clone();
        }
        body
    };
    let mut n = 0;
    let mut start = |extra: Value, expect: u16| {
        n += 1;
        keyed(
            scenario(
                "POST",
                "/runs",
                "/runs",
                alice.clone(),
                Some(body(extra)),
                expect,
            ),
            n,
        )
    };

    // a new run is 202; the same identifier is 409 naming the holder; USE_EXISTING is 200
    let (_, _, _, run) = check_body(
        &api,
        &spec,
        &start(json!({"businessId": "order-1001"}), 202),
    )
    .await;
    assert_eq!(run["businessId"], "order-1001");
    let (_, _, _, refused) = check_body(
        &api,
        &spec,
        &start(json!({"businessId": "order-1001"}), 409),
    )
    .await;
    assert_eq!(refused["details"]["reason"], "BUSINESS_ID_IN_USE");
    assert_eq!(refused["details"]["runId"], run["runId"]);
    let (_, _, _, held) = check_body(
        &api,
        &spec,
        &start(
            json!({"businessId": "order-1001", "businessIdConflictPolicy": "USE_EXISTING"}),
            200,
        ),
    )
    .await;
    assert_eq!(held["runId"], run["runId"]);

    // an option that is none is 422 with the reason and the place
    for extra in [
        json!({"businessId": "has a space"}),
        json!({"businessId": "ok-1", "businessIdReusePolicy": "SOMETIMES"}),
        json!({"businessIdConflictPolicy": "USE_EXISTING"}),
    ] {
        let (_, _, _, invalid) = check_body(&api, &spec, &start(extra, 422)).await;
        assert_eq!(invalid["details"]["reason"], "BUSINESS_ID_INVALID");
        assert!(invalid["details"]["field"].is_string());
    }

    // the run says what it was started under, and the list can be narrowed by it
    let id = run["runId"].as_str().expect("id");
    let (_, _, _, read) = check_body(
        &api,
        &spec,
        &namespaced(scenario(
            "GET",
            "/runs/{runId}",
            &format!("/runs/{id}"),
            alice.clone(),
            None,
            200,
        )),
    )
    .await;
    assert_eq!(read["businessId"], "order-1001");
    for (query, expect, count) in [
        ("businessId=order-1001", 200, Some(1)),
        ("businessId=order-9", 200, Some(0)),
        ("businessIdPrefix=order", 200, Some(1)),
        ("businessIdPrefix=invoice", 200, Some(0)),
        ("businessId=has%20space", 422, None),
        ("businessIdPrefix=", 422, None),
    ] {
        let (_, _, _, page) = check_body(
            &api,
            &spec,
            &namespaced(scenario(
                "GET",
                "/runs",
                &format!("/runs?{query}"),
                alice.clone(),
                None,
                expect,
            )),
        )
        .await;
        if let Some(count) = count {
            assert_eq!(
                page["items"].as_array().map(Vec::len),
                Some(count),
                "{query}: {page}"
            );
        }
    }

    // and the binding can be read: held, then not there
    let (_, _, _, binding) = check_body(
        &api,
        &spec,
        &scenario(
            "GET",
            "/namespaces/{namespaceId}/business-ids/{businessId}",
            &format!("/namespaces/{namespace}/business-ids/order-1001"),
            alice.clone(),
            None,
            200,
        ),
    )
    .await;
    assert_eq!(binding["executionId"], run["runId"]);
    assert_eq!(binding["open"], true);
    for absent in ["order-9999", "has%20space"] {
        check_body(
            &api,
            &spec,
            &scenario(
                "GET",
                "/namespaces/{namespaceId}/business-ids/{businessId}",
                &format!("/namespaces/{namespace}/business-ids/{absent}"),
                alice.clone(),
                None,
                404,
            ),
        )
        .await;
    }
}
