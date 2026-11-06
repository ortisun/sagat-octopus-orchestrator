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
        (alice.clone(), json!({"sequence": 1, "reapply": "ALL"}), 422),
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
            json!({"sequence": 1, "reason": "the price list was wrong", "sourcePolicy": "CANCEL_SOURCE"}),
            422,
        ),
        (
            alice.clone(),
            json!({"sequence": 1, "reason": "the price list was wrong", "reapply": "SIGNALS"}),
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
            assert_eq!(listed["items"][0]["resetId"], json!(reset));
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
