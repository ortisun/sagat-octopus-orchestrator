//! US01 — define the compensation on publish: the plan records the dependencies and the durable
//! compensation ids (US01-AC01).

use reqwest::Method;
use serde_json::json;

use crate::support::worker::platform;
use crate::{input, payment_saga};

async fn plans(
    p: &crate::support::worker::Platform,
    workflow: &str,
    number: u32,
) -> (u16, serde_json::Value) {
    p.rest(
        Method::GET,
        &format!("/workflows/{workflow}/versions/{number}/compensation-plans"),
        None,
    )
    .await
}

#[tokio::test]
async fn us01_publishing_a_saga_records_its_plan_with_durable_compensation_ids() {
    let p = platform().await;
    let wf = p.publish("corporate-payment", &payment_saga()).await;

    let (status, body) = plans(&p, &wf, 1).await;
    assert_eq!(status, 200, "{body}");
    let sagas = body["sagas"].as_array().expect("sagas");
    assert_eq!(sagas.len(), 1, "{body}");
    let plan = &sagas[0];
    assert_eq!(plan["saga"], "settlement");
    assert_eq!(plan["onCompensationFailure"], "manual_review");
    assert_eq!(plan["cancelPolicy"], "compensate_completed");

    // Last done, first undone: the payment is returned before the limit is released.
    let order: Vec<(&str, &str)> = plan["compensations"]
        .as_array()
        .expect("compensations")
        .iter()
        .map(|c| {
            (
                c["node"].as_str().expect("node"),
                c["activity"].as_str().expect("activity"),
            )
        })
        .collect();
    assert_eq!(
        order,
        [
            ("submit", "payment.return.request"),
            ("reserve", "limit.release")
        ]
    );
    assert_eq!(plan["compensations"][0]["after"], json!([]));
    assert_eq!(plan["compensations"][1]["after"], json!(["submit"]));

    // The ids are durable handles: distinct per step, the same on every read.
    let ids: Vec<&str> = plan["compensations"]
        .as_array()
        .expect("compensations")
        .iter()
        .map(|c| c["compensationId"].as_str().expect("id"))
        .collect();
    assert_ne!(ids[0], ids[1]);
    let (_, again) = plans(&p, &wf, 1).await;
    assert_eq!(again, body);
}

#[tokio::test]
async fn us01_the_plan_the_workers_follow_is_the_plan_that_was_recorded() {
    let p = platform().await;
    let wf = p.publish("corporate-payment", &payment_saga()).await;
    let (_, body) = plans(&p, &wf, 1).await;
    let reserve_id = body["sagas"][0]["compensations"][1]["compensationId"]
        .as_str()
        .expect("id")
        .to_owned();

    let run = p.start(&wf, &input()).await;
    let mut w = p.worker("wendy").await;
    w.register(&[
        "payment.validate",
        "limit.reserve",
        "payment.submit",
        "limit.release",
        "payment.return.request",
    ])
    .await;
    let validate = w.one_task().await;
    w.complete(&validate, &json!({})).await.expect("validate");
    let reserve = w.one_task().await;
    assert_eq!(reserve.task_kind, "FORWARD");
    w.complete(&reserve, &json!({"reservation": "r-1"}))
        .await
        .expect("reserve");
    let submit = w.one_task().await;
    w.fail(&submit, false).await.expect("declined");

    // `submit` never completed: only the reservation is undone, with the recorded id in its key.
    let undo = w.one_task().await;
    assert_eq!(undo.activity_name, "limit.release");
    assert_eq!(
        undo.idempotency_key,
        format!("compensation:{reserve_id}:{run}")
    );
}

#[tokio::test]
async fn us01_a_compensation_that_cannot_work_is_refused_when_the_author_publishes() {
    let p = platform().await;
    // An effect that cannot be undone and has an undo: a contradiction the author must resolve.
    let mut ir = payment_saga();
    for node in ir["nodes"].as_array_mut().expect("nodes") {
        if node["id"] == "submit" {
            node["irreversible"] = json!(true);
        }
    }
    let (status, created) = p
        .rest(
            Method::POST,
            "/workflows",
            Some(&json!({"name": "corporate-payment", "ir": ir})),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().expect("id");
    let (status, refused) = p
        .rest(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            Some(&json!({"draftRevision": 1})),
        )
        .await;
    assert_eq!(status, 422, "{refused}");
    let text = refused.to_string();
    assert!(text.contains("submit"), "the error names the node: {text}");
    // Nothing was published, so there is nothing to read.
    let (status, _) = plans(&p, id, 1).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn us01_a_compensation_declared_outside_a_saga_is_published_with_a_warning() {
    let p = platform().await;
    let ir = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "lonely"}, "entrypoint": "reserve",
        "nodes": [{"id": "reserve", "type": "activity", "activity": "limit.reserve",
                   "compensateWith": "limit.release"}]
    });
    let (status, created) = p
        .rest(
            Method::POST,
            "/workflows",
            Some(&json!({"name": "lonely", "ir": ir})),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().expect("id");
    let (status, published) = p
        .rest(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            Some(&json!({"draftRevision": 1})),
        )
        .await;
    assert_eq!(status, 201, "{published}");
    let warnings = published["warnings"].to_string();
    assert!(
        warnings.to_lowercase().contains("saga"),
        "the author is told: {warnings}"
    );
    // No saga, so no plan: the compensation would never run, and the platform says so.
    let (_, body) = plans(&p, id, 1).await;
    assert_eq!(body["sagas"], json!([]));
}

#[tokio::test]
async fn us01_another_tenant_cannot_read_a_plan_and_a_worker_identity_cannot_either() {
    let p = platform().await;
    let wf = p.publish("corporate-payment", &payment_saga()).await;
    let foreign = p
        .api
        .as_user(
            Method::GET,
            &format!("/workflows/{wf}/versions/1/compensation-plans"),
            "bob",
            "globex",
        )
        .header("x-octopus-namespace", "payments")
        .send()
        .await
        .expect("send");
    assert_eq!(
        foreign.status(),
        404,
        "another tenant does not learn that it exists"
    );

    p.grant(p.tenant, "wendy", sagat_domain::identity::Role::Worker)
        .await;
    let (status, _) = p
        .rest_as(
            "wendy",
            Method::GET,
            &format!("/workflows/{wf}/versions/1/compensation-plans"),
            None,
        )
        .await;
    assert_eq!(status, 403);
}
