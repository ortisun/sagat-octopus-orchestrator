//! Live conformance of the reset operations marked `x-implemented: '018'` (`T018-054`): every response the API
//! really produces uses a documented status, conforms to the documented schema and carries the documented
//! headers, on the normal path and on every refusal.

use std::collections::BTreeSet;

use reqwest::Method;
use sagat_domain::identity::{Membership, Role};
use serde_json::{Value, json};

use crate::support::api::start;
use crate::support::contract::{As, OpenApi, Scenario, check_body, scenario};
use crate::support::{now, subject};

const POINTS: &str = "/runs/{runId}/reset-points";
const PREVIEWS: &str = "/runs/{runId}/reset-previews";
const RESETS: &str = "/runs/{runId}/resets";
const RESET: &str = "/runs/{runId}/resets/{resetId}";

fn keyed(s: Scenario, n: u32) -> Scenario {
    s.with_header("idempotency-key", format!("reset-contract-{n:04}"))
}

fn namespaced(s: Scenario) -> Scenario {
    s.with_header("x-octopus-namespace", "payments")
}

#[tokio::test]
async fn every_reset_operation_conforms_to_the_contract_on_its_normal_and_error_paths() {
    let spec = OpenApi::load();
    let api = start().await;
    let acme = api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    api.owner_store
        .insert_membership(&Membership::restore(
            acme,
            subject("vera"),
            Role::Viewer,
            now(),
        ))
        .await
        .expect("grant");
    let ws = api.create_workspace("alice", "acme", "platform").await;
    api.create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;
    let flow = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "orders"}, "entrypoint": "a",
        "nodes": [{"id": "a", "type": "activity", "activity": "act.a"}]
    });
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "reset-setup-0001")
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
        .header("idempotency-key", "reset-setup-0002")
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("send");
    assert_eq!(published.status(), 201);

    let alice = As::User("alice", "acme");
    let vera = As::User("vera", "acme");
    let bob = As::User("bob", "globex");
    let mut n = 0;
    let mut seen = BTreeSet::new();
    let check = |s: Scenario| {
        let api = &api;
        let spec = &spec;
        async move { check_body(api, spec, &s).await }
    };

    // a run that is still open: the start of it is a point, and it cannot be reset yet
    n += 1;
    let (_, _, _, run) = check(keyed(
        namespaced(scenario(
            "POST",
            "/runs",
            "/runs",
            alice.clone(),
            Some(json!({"workflowId": workflow, "input": {"item": "book"}})),
            202,
        )),
        n,
    ))
    .await;
    let id = run["runId"].as_str().expect("run id").to_owned();
    let points = format!("/runs/{id}/reset-points");
    let previews = format!("/runs/{id}/reset-previews");
    let resets = format!("/runs/{id}/resets");

    for (who, query, expect) in [
        (alice.clone(), "", 200),
        (alice.clone(), "?limit=1", 200),
        (alice.clone(), "?cursor=not-a-cursor", 400),
        (vera.clone(), "", 403),
        (bob.clone(), "", 404),
        (As::Anonymous, "", 401),
    ] {
        let (m, t, s, page) = check(scenario(
            "GET",
            POINTS,
            &format!("{points}{query}"),
            who,
            None,
            expect,
        ))
        .await;
        seen.insert((m, t));
        if s == 200 {
            assert_eq!(page["items"][0]["kind"], "RUN_STARTED");
            assert_eq!(page["items"][0]["sequence"], 1);
        }
    }

    for (who, body, expect) in [
        (alice.clone(), json!({"sequence": 1}), 200),
        (
            alice.clone(),
            json!({"sequence": 1, "reapply": "NONE"}),
            200,
        ),
        (alice.clone(), json!({"sequence": 7}), 422),
        (alice.clone(), json!({"sequence": 0}), 422),
        (alice.clone(), json!({"sequence": 1, "reapply": "ALL"}), 200),
        (
            alice.clone(),
            json!({"sequence": 1, "reapply": "SIGNALS"}),
            200,
        ),
        (
            alice.clone(),
            json!({"sequence": 1, "reapply": "SOMETIMES"}),
            422,
        ),
        (alice.clone(), json!({"sequence": 1, "nonsense": true}), 422),
        (vera.clone(), json!({"sequence": 1}), 403),
        (bob.clone(), json!({"sequence": 1}), 404),
        (As::Anonymous, json!({"sequence": 1}), 401),
    ] {
        let (m, t, s, plan) = check(scenario(
            "POST",
            PREVIEWS,
            &previews,
            who,
            Some(body),
            expect,
        ))
        .await;
        seen.insert((m, t));
        if s == 200 {
            assert_eq!(plan["restored"], json!([]));
            assert_eq!(plan["requiresAcknowledgement"], false);
        }
    }

    // an open run is not reset
    n += 1;
    let (m, t, _, refused) = check(keyed(
        scenario(
            "POST",
            RESETS,
            &resets,
            alice.clone(),
            Some(json!({"sequence": 1, "reason": "the price list was wrong"})),
            409,
        ),
        n,
    ))
    .await;
    seen.insert((m, t));
    assert_eq!(refused["details"]["reason"], "RESET_SOURCE_NOT_TERMINAL");

    // `CANCEL_SOURCE` on a run that is open cancels it and leaves an entry that waits: 202, no run yet
    n += 1;
    let (_, _, _, other) = check(keyed(
        namespaced(scenario(
            "POST",
            "/runs",
            "/runs",
            alice.clone(),
            Some(json!({"workflowId": workflow, "input": {"item": "pen"}})),
            202,
        )),
        n,
    ))
    .await;
    let waiting_source = other["runId"].as_str().expect("run id").to_owned();
    n += 1;
    let (m, t, _, pending) = check(keyed(
        scenario(
            "POST",
            RESETS,
            &format!("/runs/{waiting_source}/resets"),
            alice.clone(),
            Some(json!({
                "sequence": 1, "reason": "the price list was wrong",
                "sourcePolicy": "CANCEL_SOURCE", "acknowledgeReexecution": true
            })),
            202,
        ),
        n,
    ))
    .await;
    seen.insert((m, t));
    // nothing is in flight, so the cancellation ended the source: the entry is due, and waits for the engine
    assert_eq!(pending["state"], "PENDING_SOURCE");
    assert_eq!(pending["newRunId"], Value::Null);
    assert_eq!(pending["failureReason"], Value::Null);
    assert_eq!(pending["sourceRunId"], waiting_source);
    let pending_id = pending["resetId"].as_str().expect("reset id").to_owned();
    let (_, _, _, got) = check(scenario(
        "GET",
        RESET,
        &format!("/runs/{waiting_source}/resets/{pending_id}"),
        alice.clone(),
        None,
        200,
    ))
    .await;
    assert_eq!(got["state"], "PENDING_SOURCE");

    // once it has ended it is
    n += 1;
    check(keyed(
        scenario(
            "POST",
            "/runs/{runId}/operations",
            &format!("/runs/{id}/operations"),
            alice.clone(),
            Some(json!({"action": "CANCEL", "reason": "ended to be reset"})),
            202,
        ),
        n,
    ))
    .await;
    n += 1;
    let (m, t, _, made) = check(keyed(
        scenario(
            "POST",
            RESETS,
            &resets,
            alice.clone(),
            Some(json!({"sequence": 1, "reason": "the price list was wrong"})),
            202,
        ),
        n,
    ))
    .await;
    seen.insert((m, t));
    assert_eq!(made["state"], "CREATED");
    assert_eq!(made["sourceRunId"], id);
    let reset = made["resetId"].as_str().expect("reset id").to_owned();
    // the run the reset made says where it came from (G2057, FR-018-037): read by id and in the list, both held to the
    // `Run` schema, which declares `resetFrom`
    let new_run = made["newRunId"].as_str().expect("the new run").to_owned();
    let (_, _, _, created) = check(namespaced(scenario(
        "GET",
        "/runs/{runId}",
        &format!("/runs/{new_run}"),
        alice.clone(),
        None,
        200,
    )))
    .await;
    assert_eq!(created["rerunOf"], id.as_str());
    assert_eq!(
        created["resetFrom"],
        json!({"runId": id, "sequence": 1}),
        "{created}"
    );
    let (_, _, _, listed) = check(namespaced(scenario(
        "GET",
        "/runs",
        "/runs",
        alice.clone(),
        None,
        200,
    )))
    .await;
    let from_the_list = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["runId"] == new_run.as_str())
        .expect("the run is listed");
    assert_eq!(from_the_list["resetFrom"], created["resetFrom"]);
    assert!(
        listed["items"]
            .as_array()
            .expect("items")
            .iter()
            .filter(|item| item["runId"] != new_run.as_str())
            .all(|item| item.get("resetFrom").is_none()),
        "absent, not null, for the runs a reset did not make"
    );

    // the key, once used, answers the same; a body that is none is a 422; who may not, may not
    let (_, _, _, again) = check(
        scenario(
            "POST",
            RESETS,
            &resets,
            alice.clone(),
            Some(json!({"sequence": 1, "reason": "the price list was wrong"})),
            202,
        )
        .with_header("idempotency-key", format!("reset-contract-{n:04}")),
    )
    .await;
    assert_eq!(again["resetId"], made["resetId"]);
    for (who, body, expect) in [
        (alice.clone(), json!({"sequence": 1}), 422),
        (
            alice.clone(),
            json!({"sequence": 1, "reason": "short"}),
            422,
        ),
        (
            alice.clone(),
            json!({"sequence": 1, "reason": "the price list was wrong", "sourcePolicy": "SOMETIMES"}),
            422,
        ),
        (
            alice.clone(),
            json!({"sequence": 1, "reason": "the price list was wrong", "reapply": "SOMETIMES"}),
            422,
        ),
        (
            vera.clone(),
            json!({"sequence": 1, "reason": "the price list was wrong"}),
            403,
        ),
        (
            bob.clone(),
            json!({"sequence": 1, "reason": "the price list was wrong"}),
            404,
        ),
        (
            As::Anonymous,
            json!({"sequence": 1, "reason": "the price list was wrong"}),
            401,
        ),
    ] {
        n += 1;
        check(keyed(
            scenario("POST", RESETS, &resets, who, Some(body), expect),
            n,
        ))
        .await;
    }
    // a reapply is served: the run received nothing after the start, so there is nothing to report
    n += 1;
    let (_, _, _, none) = check(keyed(
        scenario(
            "POST",
            RESETS,
            &resets,
            alice.clone(),
            Some(
                json!({"sequence": 1, "reason": "the price list was wrong", "reapply": "ALL", "businessId": "DROP"}),
            ),
            202,
        ),
        n,
    ))
    .await;
    assert_eq!(none["reapplyReport"], json!([]));

    // and a run that was signalled has something to report: the shapes of the plan and of the report conform
    let waiting = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "signalled"}, "entrypoint": "w",
        "nodes": [
            {"id": "w", "type": "waitEvent", "eventType": "payment.confirmed",
             "correlationExpr": "input.k", "timeoutMs": 300000, "next": "b"},
            {"id": "b", "type": "activity", "activity": "act.b"}
        ]
    });
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "reset-setup-0003")
        .json(&json!({"name": "signalled", "ir": waiting}))
        .send()
        .await
        .expect("send");
    assert_eq!(created.status(), 201);
    let signalled = created.json::<Value>().await.expect("json")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let published = api
        .as_user(
            Method::POST,
            &format!("/workflows/{signalled}/publish"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", "reset-setup-0004")
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("send");
    assert_eq!(published.status(), 201);
    n += 1;
    let (_, _, _, started) = check(keyed(
        namespaced(scenario(
            "POST",
            "/runs",
            "/runs",
            alice.clone(),
            Some(json!({"workflowId": signalled, "input": {"k": "s"}})),
            202,
        )),
        n,
    ))
    .await;
    let waiting_run = started["runId"].as_str().expect("run id").to_owned();
    n += 1;
    check(keyed(
        scenario(
            "POST",
            "/runs/{runId}/signals/{signalName}",
            &format!("/runs/{waiting_run}/signals/payment.confirmed"),
            alice.clone(),
            Some(json!({"paid": true})),
            202,
        ),
        n,
    ))
    .await;
    n += 1;
    check(keyed(
        scenario(
            "POST",
            "/runs/{runId}/operations",
            &format!("/runs/{waiting_run}/operations"),
            alice.clone(),
            Some(json!({"action": "CANCEL", "reason": "ended to be reset"})),
            202,
        ),
        n,
    ))
    .await;
    let (_, _, _, plan) = check(scenario(
        "POST",
        PREVIEWS,
        &format!("/runs/{waiting_run}/reset-previews"),
        alice.clone(),
        Some(json!({"sequence": 1, "reapply": "SIGNALS"})),
        200,
    ))
    .await;
    assert_eq!(plan["reapply"].as_array().map(Vec::len), Some(1), "{plan}");
    assert_eq!(plan["reapply"][0]["kind"], "SIGNAL");
    assert_eq!(plan["reapply"][0]["name"], "payment.confirmed");
    assert_eq!(plan["reapply"][0]["outcome"], "WILL_TRY");
    n += 1;
    let (_, _, _, report) = check(keyed(
        scenario(
            "POST",
            RESETS,
            &format!("/runs/{waiting_run}/resets"),
            alice.clone(),
            Some(json!({
                "sequence": 1, "reason": "the price list was wrong", "reapply": "SIGNALS",
                "acknowledgeReexecution": true
            })),
            202,
        ),
        n,
    ))
    .await;
    assert_eq!(
        report["reapplyReport"].as_array().map(Vec::len),
        Some(1),
        "{report}"
    );
    assert_eq!(report["reapplyReport"][0]["outcome"], "REAPPLIED");
    assert_eq!(report["reapplyReport"][0]["name"], "payment.confirmed");
    assert!(
        !report.to_string().contains("paid"),
        "the report holds no content: {report}"
    );

    // without a key there is no reset
    check(scenario(
        "POST",
        RESETS,
        &resets,
        alice.clone(),
        Some(json!({"sequence": 1, "reason": "the price list was wrong"})),
        400,
    ))
    .await;

    // the register
    for (who, expect) in [
        (alice.clone(), 200),
        (vera.clone(), 200),
        (bob.clone(), 404),
        (As::Anonymous, 401),
    ] {
        let (m, t, s, listed) = check(scenario("GET", RESETS, &resets, who, None, expect)).await;
        seen.insert((m, t));
        if s == 200 {
            // newest first: the first reset is among the two this source made
            assert!(
                listed["items"]
                    .as_array()
                    .expect("items")
                    .iter()
                    .any(|item| item["resetId"] == json!(reset)),
                "{listed}"
            );
        }
    }
    let one = format!("{resets}/{reset}");
    let nobody = format!("{resets}/0194f5e2-7b1c-7000-8000-000000000009");
    for (who, path, expect) in [
        (alice.clone(), one.clone(), 200),
        (vera.clone(), one.clone(), 200),
        (bob.clone(), one.clone(), 404),
        (alice.clone(), nobody, 404),
        (alice.clone(), format!("{resets}/not-an-id"), 404),
        (As::Anonymous, one, 401),
    ] {
        let (m, t, _, _) = check(scenario("GET", RESET, &path, who, None, expect)).await;
        seen.insert((m, t));
    }

    let operations: BTreeSet<(String, String)> = [
        ("GET", POINTS),
        ("POST", PREVIEWS),
        ("GET", RESETS),
        ("POST", RESETS),
        ("GET", RESET),
    ]
    .into_iter()
    .map(|(method, template)| (method.to_owned(), template.to_owned()))
    .collect();
    assert_eq!(seen, operations, "every reset operation was exercised");
}
