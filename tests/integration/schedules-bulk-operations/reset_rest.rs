//! Reset to a point over HTTP (`T018-054`, `FR-018-035`..`FR-018-037`): the points a run can be reset to, the
//! preview, the reset and the register of the resets of a run. A real API, a real PostgreSQL, a real worker
//! over gRPC, real tokens.
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use crate::support::api::Options;
use crate::support::worker::{HUMAN, Platform, TENANT, chain_ir, platform, platform_with};

const REASON: &str = "the second step used a wrong price list";

fn reason(error: &Value) -> &str {
    error["details"]["reason"].as_str().unwrap_or_default()
}

/// A run of two activities, both completed by a worker.
async fn ended(p: &Platform) -> String {
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    ended_from(p, &workflow).await
}

/// A run of `workflow` whose two activities a worker completes.
async fn ended_from(p: &Platform, workflow: &str) -> String {
    let run = p.start(workflow, &json!({"item": "book"})).await;
    complete_both(p, &run).await;
    run
}

/// The worker completes the two activities of `run`, which then ends.
async fn complete_both(p: &Platform, run: &str) {
    let mut worker = p.worker("reset-worker").await;
    worker.register(&["act.a", "act.b"]).await;
    for _ in 0..2 {
        p.tick().await;
        let task = worker.one_task().await;
        worker
            .complete(&task, &json!({"ok": true}))
            .await
            .expect("the step completes");
    }
    assert_eq!(p.status(run).await, "SUCCEEDED");
}

async fn points(p: &Platform, run: &str, query: &str) -> (u16, Value) {
    p.rest(
        Method::GET,
        &format!("/runs/{run}/reset-points{query}"),
        None,
    )
    .await
}

/// `POST /runs/{run}/resets` with the key given.
async fn reset_with(p: &Platform, run: &str, key: &str, body: &Value) -> (u16, Value) {
    let response = p
        .api
        .as_user(Method::POST, &format!("/runs/{run}/resets"), HUMAN, TENANT)
        .header("idempotency-key", key)
        .json(body)
        .send()
        .await
        .expect("send");
    let status = response.status().as_u16();
    let text = response.text().await.expect("body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn sequences(page: &Value) -> Vec<u64> {
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["sequence"].as_u64().expect("sequence"))
        .collect()
}

#[tokio::test]
async fn the_points_of_a_run_are_listed_in_order_by_kind_and_node_and_page_by_cursor() {
    let p = platform().await;
    let run = ended(&p).await;

    let (status, page) = points(&p, &run, "").await;
    assert_eq!(status, 200, "{page}");
    let items = page["items"].as_array().expect("items");
    let kinds: Vec<_> = items.iter().map(|item| item["kind"].clone()).collect();
    assert_eq!(kinds, ["RUN_STARTED", "STEP_COMPLETED", "STEP_COMPLETED"]);
    let nodes: Vec<_> = items.iter().map(|item| item["node"].clone()).collect();
    assert_eq!(nodes, [Value::Null, json!("a"), json!("b")]);
    // and each says when the event that ends its prefix happened: the journal's own time for that sequence
    let (status, journal) = p
        .rest(Method::GET, &format!("/runs/{run}/events?limit=200"), None)
        .await;
    assert_eq!(status, 200, "{journal}");
    for item in items {
        let entry = journal["items"]
            .as_array()
            .expect("journal")
            .iter()
            .find(|entry| entry["seq"] == item["sequence"])
            .expect("the event of the point");
        assert_eq!(
            item["occurredAt"], entry["occurredAt"],
            "{} {}",
            item["kind"], item["sequence"]
        );
    }
    let all = sequences(&page);
    assert!(all.windows(2).all(|pair| pair[0] < pair[1]), "{all:?}");
    assert_eq!(all[0], 1);
    assert_eq!(page["nextCursor"], Value::Null);

    // a page of two, then the rest by the cursor it hands out
    let (_, first) = points(&p, &run, "?limit=2").await;
    assert_eq!(sequences(&first), all[..2]);
    let cursor = first["nextCursor"].as_str().expect("a cursor");
    let (_, second) = points(&p, &run, &format!("?limit=2&cursor={cursor}")).await;
    assert_eq!(sequences(&second), all[2..]);
    assert_eq!(second["nextCursor"], Value::Null);
    // a page that holds exactly what is left has no next one, whether it is all of them or the last ones
    let (_, exact) = points(&p, &run, "?limit=3").await;
    assert_eq!(sequences(&exact), all);
    assert_eq!(exact["nextCursor"], Value::Null);
    let (_, one) = points(&p, &run, "?limit=1").await;
    let after_first = one["nextCursor"].as_str().expect("a cursor");
    let (_, tail) = points(&p, &run, &format!("?limit=2&cursor={after_first}")).await;
    assert_eq!(sequences(&tail), all[1..]);
    assert_eq!(tail["nextCursor"], Value::Null);

    // outside 1..200 is clamped, as for every list; a cursor nobody handed out is a bad request
    let (_, clamped) = points(&p, &run, "?limit=0").await;
    assert_eq!(sequences(&clamped).len(), 1);
    let (_, wide) = points(&p, &run, "?limit=100000").await;
    assert_eq!(sequences(&wide), all);
    let (status, bad) = points(&p, &run, "?cursor=not-a-cursor").await;
    assert_eq!(status, 400, "{bad}");
}

#[tokio::test]
async fn a_preview_says_what_is_kept_and_what_runs_again_and_changes_nothing() {
    let p = platform().await;
    let run = ended(&p).await;
    let (_, page) = points(&p, &run, "").await;
    let seq = sequences(&page);
    let preview = |sequence: u64| {
        let path = format!("/runs/{run}/reset-previews");
        let p = &p;
        async move {
            p.rest(Method::POST, &path, Some(&json!({"sequence": sequence})))
                .await
        }
    };

    let (status, plan) = preview(seq[1]).await;
    assert_eq!(status, 200, "{plan}");
    assert_eq!(plan["sequence"], seq[1]);
    assert_eq!(plan["prefixEvents"], seq[1]);
    assert_eq!(plan["restored"], json!([{"node": "a", "kind": "STEP"}]));
    assert_eq!(
        plan["reexecuted"],
        json!([{"node": "b", "completedInSource": true, "markers": []}])
    );
    assert_eq!(plan["requiresAcknowledgement"], true);
    assert_eq!(plan["reapply"], json!([]));

    // the start keeps nothing: every step the source ran runs again, the first one included
    let (_, start) = preview(seq[0]).await;
    assert_eq!(start["restored"], json!([]));
    assert_eq!(
        start["reexecuted"],
        json!([
            {"node": "a", "completedInSource": true, "markers": []},
            {"node": "b", "completedInSource": true, "markers": []}
        ])
    );
    assert_eq!(start["requiresAcknowledgement"], true);

    // at the last point nothing runs again, so there is nothing to acknowledge
    let (_, last) = preview(seq[2]).await;
    assert_eq!(last["reexecuted"], json!([]));
    assert_eq!(last["requiresAcknowledgement"], false);

    // the refusals say why
    let (status, refused) = preview(seq[1] + 1).await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(reason(&refused), "RESET_POINT_NOT_ELIGIBLE");
    let (status, refused) = preview(5001).await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(reason(&refused), "RESET_PREFIX_TOO_LONG");
    let (status, _) = preview(0).await;
    assert_eq!(status, 422);
    // a reapply is served, and a run that received nothing after the point has nothing to deliver again
    let (status, listed) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/reset-previews"),
            Some(&json!({"sequence": seq[1], "reapply": "SIGNALS"})),
        )
        .await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["reapply"], json!([]));
    let (status, unknown) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/reset-previews"),
            Some(&json!({"sequence": seq[1], "reapply": "SOMETIMES"})),
        )
        .await;
    assert_eq!(status, 422, "{unknown}");
    let (status, _) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/reset-previews"),
            Some(&json!({"sequence": seq[1], "nonsense": 1})),
        )
        .await;
    assert_eq!(status, 422);

    // and nothing was made by any of it
    let (_, resets) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(resets["items"], json!([]));
}

#[tokio::test]
async fn a_reset_makes_a_run_and_an_entry_a_retry_is_the_same_entry_and_the_register_lists_it() {
    let p = platform().await;
    let run = ended(&p).await;
    let (_, page) = points(&p, &run, "").await;
    let after_a = sequences(&page)[1];
    let ask =
        |ack: bool| json!({"sequence": after_a, "reason": REASON, "acknowledgeReexecution": ack});

    // the request is refused before anything is made: no key, short reason, not acknowledged, not available
    let (status, _) = p
        .api
        .as_user(Method::POST, &format!("/runs/{run}/resets"), HUMAN, TENANT)
        .json(&ask(true))
        .send()
        .await
        .map(|r| (r.status().as_u16(), ()))
        .expect("send");
    assert_eq!(status, 400, "the key is required");
    let (status, refused) = reset_with(&p, &run, "reset-key-0001", &ask(false)).await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(reason(&refused), "RESET_REEXECUTION_NOT_ACKNOWLEDGED");
    // the start of the run is no exception: its first step ran in the source and would run again
    let (status, refused) = reset_with(
        &p,
        &run,
        "reset-key-0007",
        &json!({"sequence": 1, "reason": REASON}),
    )
    .await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(reason(&refused), "RESET_REEXECUTION_NOT_ACKNOWLEDGED");
    let (status, _) = reset_with(
        &p,
        &run,
        "reset-key-0002",
        &json!({"sequence": after_a, "reason": "short", "acknowledgeReexecution": true}),
    )
    .await;
    assert_eq!(status, 422);
    for extra in [json!({"sourcePolicy": "SOMETIMES"})] {
        let mut body = ask(true);
        for (key, value) in extra.as_object().expect("an object") {
            body[key] = value.clone();
        }
        let (status, refused) = reset_with(&p, &run, "reset-key-0003", &body).await;
        assert_eq!(status, 422, "{body}: {refused}");
    }
    let (_, resets) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(resets["items"], json!([]), "a refused reset leaves nothing");

    // the reset
    let (status, made) = reset_with(&p, &run, "reset-key-0004", &ask(true)).await;
    assert_eq!(status, 202, "{made}");
    assert_eq!(made["state"], "CREATED");
    assert_eq!(made["sourceRunId"], run);
    assert_eq!(made["sequence"], after_a);
    assert_eq!(made["failureReason"], Value::Null);
    assert_eq!(made["reapplyReport"], json!([]));
    assert!(made["createdAt"].is_string());
    let reset_id = made["resetId"].as_str().expect("the id").to_owned();
    let new_run = made["newRunId"].as_str().expect("the new run").to_owned();
    let created = p.run(&new_run).await;
    assert_eq!(created["rerunOf"], run);
    assert_eq!(created["status"], "RUNNING");

    // a retry is the same entry; the key of another point is not a retry
    let (status, again) = reset_with(&p, &run, "reset-key-0004", &ask(true)).await;
    assert_eq!(status, 202, "{again}");
    assert_eq!(again["resetId"], reset_id);
    assert_eq!(again["newRunId"], new_run);
    let (status, other) = reset_with(
        &p,
        &run,
        "reset-key-0004",
        &json!({"sequence": 1, "reason": REASON, "acknowledgeReexecution": true}),
    )
    .await;
    assert_eq!(status, 409, "{other}");
    assert_eq!(other["code"], "CONFLICT");
    assert_eq!(reason(&other), "IDEMPOTENCY_KEY_REUSED");

    // the register
    let (status, listed) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["items"].as_array().expect("items").len(), 1);
    assert_eq!(listed["items"][0]["resetId"], reset_id);
    let (status, one) = p
        .rest(Method::GET, &format!("/runs/{run}/resets/{reset_id}"), None)
        .await;
    assert_eq!(status, 200, "{one}");
    assert_eq!(one, listed["items"][0]);
    for missing in [
        "0194f5e2-7b1c-7000-8000-000000000009".to_owned(),
        "not-an-id".to_owned(),
    ] {
        let (status, _) = p
            .rest(Method::GET, &format!("/runs/{run}/resets/{missing}"), None)
            .await;
        assert_eq!(status, 404, "{missing}");
    }
    // the run the reset made did not make a reset of its own
    let (_, none) = p
        .rest(Method::GET, &format!("/runs/{new_run}/resets"), None)
        .await;
    assert_eq!(none["items"], json!([]));
}

#[tokio::test]
async fn an_open_run_is_not_reset_and_the_register_is_private_to_who_may_read_it() {
    let p = platform().await;
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    // the run that ends goes first: the worker must see one run at a time
    let run = ended_from(&p, &workflow).await;
    let open = p.start(&workflow, &json!({"item": "book"})).await;
    let (status, page) = points(&p, &open, "").await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(sequences(&page), [1], "the start of an open run is a point");

    let (status, refused) = reset_with(
        &p,
        &open,
        "reset-key-0005",
        &json!({"sequence": 1, "reason": REASON, "acknowledgeReexecution": true}),
    )
    .await;
    assert_eq!(status, 409, "{refused}");
    assert_eq!(refused["code"], "CONFLICT");
    assert_eq!(reason(&refused), "RESET_SOURCE_NOT_TERMINAL");

    // a viewer reads runs and may not look at their points or reset them; a stranger sees nothing
    let (_, page) = points(&p, &run, "").await;
    let after_a = sequences(&page)[1];
    let (status, made) = reset_with(
        &p,
        &run,
        "reset-key-0006",
        &json!({"sequence": after_a, "reason": REASON, "acknowledgeReexecution": true}),
    )
    .await;
    assert_eq!(status, 202, "{made}");
    let reset_id = made["resetId"].as_str().expect("id").to_owned();
    p.grant(p.tenant, "vera", Role::Viewer).await;
    let (status, _) = p
        .rest_as(
            "vera",
            Method::GET,
            &format!("/runs/{run}/reset-points"),
            None,
        )
        .await;
    assert_eq!(status, 403);
    let (status, _) = p
        .rest_as(
            "vera",
            Method::POST,
            &format!("/runs/{run}/resets"),
            Some(&json!({"sequence": after_a, "reason": REASON, "acknowledgeReexecution": true})),
        )
        .await;
    assert_eq!(status, 403);
    let (status, listed) = p
        .rest_as("vera", Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["items"][0]["resetId"], reset_id);
    for path in [
        format!("/runs/{run}/reset-points"),
        format!("/runs/{run}/resets"),
        format!("/runs/{run}/resets/{reset_id}"),
    ] {
        let response = p.api.get_as(&path, "bob", "globex").await;
        assert_eq!(response.status(), 404, "{path}");
    }
    let response = p
        .api
        .post_as(
            &format!("/runs/{run}/reset-previews"),
            "bob",
            "globex",
            &json!({"sequence": after_a}),
        )
        .await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn the_business_identifier_follows_the_choice_and_a_holder_is_named_to_who_may_read() {
    let p = platform().await;
    let workflow = p.publish("orders", &chain_ir("orders")).await;
    let (status, started) = p
        .rest(
            Method::POST,
            "/runs",
            Some(&json!({
                "workflowId": workflow, "input": {"item": "book"}, "businessId": "order-1001"
            })),
        )
        .await;
    assert_eq!(status, 202, "{started}");
    let run = started["runId"].as_str().expect("run id").to_owned();
    complete_both(&p, &run).await;
    let (_, page) = points(&p, &run, "").await;
    let after_a = sequences(&page)[1];
    let ask = |extra: Value| {
        let mut body =
            json!({"sequence": after_a, "reason": REASON, "acknowledgeReexecution": true});
        for (key, value) in extra.as_object().expect("an object") {
            body[key] = value.clone();
        }
        body
    };

    // DROP starts the new run without the identifier, so the source's can be bound again later
    let (status, dropped) = reset_with(
        &p,
        &run,
        "reset-key-0008",
        &ask(json!({"businessId": "DROP"})),
    )
    .await;
    assert_eq!(status, 202, "{dropped}");
    let new_run = dropped["newRunId"].as_str().expect("run");
    assert_eq!(p.run(new_run).await["businessId"], Value::Null);

    // KEEP (the default) binds it to the new run
    let (status, kept) = reset_with(&p, &run, "reset-key-0009", &ask(json!({}))).await;
    assert_eq!(status, 202, "{kept}");
    let holder = kept["newRunId"].as_str().expect("run").to_owned();
    assert_eq!(p.run(&holder).await["businessId"], "order-1001");

    // a second KEEP finds an open run holding it, and says which to a caller who may read runs
    let (status, refused) = reset_with(
        &p,
        &run,
        "reset-key-0010",
        &ask(json!({"businessId": "KEEP"})),
    )
    .await;
    assert_eq!(status, 409, "{refused}");
    assert_eq!(reason(&refused), "BUSINESS_ID_IN_USE");
    assert_eq!(refused["details"]["runId"], holder);
    let (_, resets) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(
        resets["items"].as_array().expect("items").len(),
        2,
        "the refused reset left no entry"
    );
}

/// `a`, then a wait for a signal, then `b`.
fn waiting_ir(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "a",
        "nodes": [
            {"id": "a", "type": "activity", "activity": "act.a", "next": "w"},
            {"id": "w", "type": "waitEvent", "eventType": "payment.confirmed",
             "correlationExpr": "input.k", "timeoutMs": 300000, "next": "b"},
            {"id": "b", "type": "activity", "activity": "act.b"}
        ]
    })
}

/// A run of [`waiting_ir`] that a worker and a signal take to the end, and the sequence after `a`.
async fn signalled(p: &Platform) -> (String, u64) {
    let workflow = p.publish("signalled", &waiting_ir("signalled")).await;
    let run = p.start(&workflow, &json!({"k": "s"})).await;
    let mut worker = p.worker("reapply-worker").await;
    worker.register(&["act.a", "act.b"]).await;
    p.tick().await;
    let task = worker.one_task().await;
    worker
        .complete(&task, &json!({"ok": true}))
        .await
        .expect("a completes");
    let (status, woken) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/signals/payment.confirmed"),
            Some(&json!({"paid": true})),
        )
        .await;
    assert_eq!(status, 202, "{woken}");
    p.tick().await;
    let task = worker.one_task().await;
    worker
        .complete(&task, &json!({"ok": true}))
        .await
        .expect("b completes");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    let (_, page) = points(p, &run, "").await;
    (run, sequences(&page)[1])
}

#[tokio::test]
async fn the_preview_lists_the_signals_to_deliver_again_and_the_reset_reports_what_became_of_them()
{
    let p = platform().await;
    let (run, after_a) = signalled(&p).await;

    let (status, plan) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/reset-previews"),
            Some(&json!({"sequence": after_a, "reapply": "SIGNALS"})),
        )
        .await;
    assert_eq!(status, 200, "{plan}");
    let listed = plan["reapply"].as_array().expect("the candidates");
    assert_eq!(listed.len(), 1, "{plan}");
    assert_eq!(listed[0]["kind"], "SIGNAL");
    assert_eq!(listed[0]["name"], "payment.confirmed");
    assert_eq!(listed[0]["outcome"], "WILL_TRY");
    assert!(listed[0]["sequence"].as_u64().expect("sequence") > after_a);
    assert!(listed[0].get("reason").is_none(), "{plan}");
    // NONE (the default) lists nothing
    let (_, none) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/reset-previews"),
            Some(&json!({"sequence": after_a})),
        )
        .await;
    assert_eq!(none["reapply"], json!([]));

    let (status, made) = reset_with(
        &p,
        &run,
        "reset-key-0020",
        &json!({"sequence": after_a, "reason": REASON, "acknowledgeReexecution": true,
                "reapply": "SIGNALS"}),
    )
    .await;
    assert_eq!(status, 202, "{made}");
    assert_eq!(
        made["reapplyReport"],
        json!([{
            "sequence": listed[0]["sequence"], "kind": "SIGNAL", "name": "payment.confirmed",
            "outcome": "REAPPLIED"
        }])
    );
    // the report holds no content, and the register and the entry answer it the same way
    assert!(!made.to_string().contains("paid"), "{made}");
    let reset_id = made["resetId"].as_str().expect("the id");
    let (_, one) = p
        .rest(Method::GET, &format!("/runs/{run}/resets/{reset_id}"), None)
        .await;
    assert_eq!(one, made);
    let (_, listed_resets) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(listed_resets["items"][0], made);
    // a retry answers the same report
    let (status, again) = reset_with(
        &p,
        &run,
        "reset-key-0020",
        &json!({"sequence": after_a, "reason": REASON, "acknowledgeReexecution": true,
                "reapply": "SIGNALS"}),
    )
    .await;
    assert_eq!(status, 202, "{again}");
    assert_eq!(again, made);

    // the new run went past the wait: the signal was delivered to it
    let new_run = made["newRunId"].as_str().expect("the new run");
    let journal = p
        .rest(Method::GET, &format!("/runs/{new_run}/events"), None)
        .await
        .1;
    assert!(
        journal.to_string().contains("wait.satisfied"),
        "the signal woke the wait: {journal}"
    );
}

#[tokio::test]
async fn the_keys_of_the_platform_are_not_for_a_caller() {
    let p = platform().await;
    let (run, after_a) = signalled(&p).await;
    let body = json!({"sequence": after_a, "reason": REASON, "acknowledgeReexecution": true});
    // a key a reset derives for what it delivers again cannot be the key of a request
    let (status, refused) = reset_with(
        &p,
        &run,
        "reset:0194f5e2-7b1c-7000-8000-000000000009:12",
        &body,
    )
    .await;
    assert_eq!(status, 400, "{refused}");
    let (_, resets) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(resets["items"], json!([]));
    // batch: and schedule: stay reserved as they were
    for key in ["batch:one-0001", "schedule:one-0001"] {
        let (status, _) = reset_with(&p, &run, key, &body).await;
        assert_eq!(status, 400, "{key}");
    }
    // and a key that only looks like one is a key
    let (status, made) = reset_with(&p, &run, "my-reset:0001", &body).await;
    assert_eq!(status, 202, "{made}");
}

#[tokio::test]
async fn a_deployment_that_did_not_turn_the_reapply_on_refuses_it_with_a_reason() {
    let p = platform_with(Options {
        reset_reapply: false,
        ..Options::default()
    })
    .await;
    let (run, after_a) = signalled(&p).await;
    for policy in ["SIGNALS", "UPDATES", "ALL"] {
        let (status, refused) = p
            .rest(
                Method::POST,
                &format!("/runs/{run}/reset-previews"),
                Some(&json!({"sequence": after_a, "reapply": policy})),
            )
            .await;
        assert_eq!(status, 422, "{policy}: {refused}");
        assert_eq!(reason(&refused), "RESET_REAPPLY_NOT_AVAILABLE");
        let (status, refused) = reset_with(
            &p,
            &run,
            "reset-key-0030",
            &json!({"sequence": after_a, "reason": REASON, "acknowledgeReexecution": true,
                    "reapply": policy}),
        )
        .await;
        assert_eq!(status, 422, "{policy}: {refused}");
        assert_eq!(reason(&refused), "RESET_REAPPLY_NOT_AVAILABLE");
        assert_eq!(refused["code"], "VALIDATION_FAILED");
    }
    // and nothing else changed: NONE is served, and nothing was made by the refusals
    let (_, resets) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(resets["items"], json!([]));
    let (status, made) = reset_with(
        &p,
        &run,
        "reset-key-0031",
        &json!({"sequence": after_a, "reason": REASON, "acknowledgeReexecution": true,
                "reapply": "NONE"}),
    )
    .await;
    assert_eq!(status, 202, "{made}");
    assert_eq!(made["reapplyReport"], json!([]));
}
#[tokio::test]
async fn a_report_that_a_newer_version_wrote_is_read_as_written_and_never_refused() {
    let p = platform().await;
    let (run, after_a) = signalled(&p).await;
    let body = json!({"sequence": after_a, "reason": REASON, "acknowledgeReexecution": true,
                      "reapply": "SIGNALS"});
    let (status, made) = reset_with(&p, &run, "reset-key-0040", &body).await;
    assert_eq!(status, 202, "{made}");
    let id = made["resetId"].as_str().expect("the id").to_owned();

    // a later version wrote an outcome, a reason and a kind this one does not know, with a field of its own; an
    // entry that is not an entry at all is the only thing left out
    sqlx::query("UPDATE run_resets SET reapply_report = $2 WHERE id = $1::uuid")
        .bind(&id)
        .bind(json!([
            {"sequence": 7, "kind": "SIGNAL", "name": "payment.confirmed", "outcome": "FUTURE_OUTCOME",
             "reason": "FUTURE", "someday": true},
            {"sequence": 9, "kind": "QUERY", "operation": "later.thing", "outcome": "REAPPLIED"},
            {"kind": "SIGNAL", "name": "no.sequence", "outcome": "REAPPLIED"},
            "not an entry"
        ]))
        .execute(&p.api.db.pool)
        .await
        .expect("the register");
    let expected = json!([
        {"sequence": 7, "kind": "SIGNAL", "name": "payment.confirmed", "outcome": "FUTURE_OUTCOME",
         "reason": "FUTURE"},
        {"sequence": 9, "kind": "QUERY", "operation": "later.thing", "outcome": "REAPPLIED"}
    ]);

    // the entry, the list and the replay of the request answer it
    let (status, one) = p
        .rest(Method::GET, &format!("/runs/{run}/resets/{id}"), None)
        .await;
    assert_eq!(status, 200, "{one}");
    assert_eq!(one["reapplyReport"], expected);
    let (status, listed) = p
        .rest(Method::GET, &format!("/runs/{run}/resets"), None)
        .await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["items"][0]["reapplyReport"], expected);
    let (status, again) = reset_with(&p, &run, "reset-key-0040", &body).await;
    assert_eq!(status, 202, "{again}");
    assert_eq!(again["reapplyReport"], expected);
}
