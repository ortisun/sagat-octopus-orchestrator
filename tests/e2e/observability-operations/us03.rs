//! US03 — receive alerts for stuck runs: when the outbox is older than the threshold or the leases
//! that ran out were not taken back, the monitor raises an alert that names the right namespace and
//! the runbook, tells the namespace's subscribers, and resolves it when the condition is gone
//! (US03-AC01, FR-008-005).

use reqwest::Method;
use sagat_scheduler::{Monitor, MonitorSettings};
use serde_json::{Value, json};

use crate::support::worker::{Platform, platform, single_ir};

fn monitor(p: &Platform) -> Monitor<sagat_infra_postgres::PgRunStore> {
    Monitor::new(p.api.executions.clone(), MonitorSettings::default())
}

async fn alerts(p: &Platform, namespace: &str, query: &str) -> Vec<Value> {
    let (status, page) = p
        .rest(
            Method::GET,
            &format!("/namespaces/{namespace}/alerts{query}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{page}");
    page["items"].as_array().expect("items").clone()
}

fn of_kind<'a>(alerts: &'a [Value], kind: &str) -> Vec<&'a Value> {
    alerts.iter().filter(|a| a["kind"] == kind).collect()
}

/// Subscribes the namespace to alerts, with the secret and the egress policy a subscription needs.
async fn subscribe(p: &Platform, namespace: &str, name: &str) -> String {
    for (method, path, body) in [
        (
            Method::PUT,
            format!("/namespaces/{namespace}/secrets/alert-signing"),
            json!({"value": crate::helpers::SECRET}),
        ),
        (
            Method::PUT,
            format!("/namespaces/{namespace}/egress-policy"),
            json!({"rules": ["hooks.example.com"]}),
        ),
    ] {
        let (status, answer) = p.rest(method, &path, Some(&body)).await;
        assert_eq!(status, 200, "{answer}");
    }
    let (status, subscription) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{namespace}/webhook-subscriptions"),
            Some(&json!({"name": name, "url": "https://hooks.example.com/in", "secretName": "alert-signing",
                         "events": ["ops.alert.fired", "ops.alert.resolved"]})),
        )
        .await;
    assert_eq!(status, 201, "{subscription}");
    subscription["id"].as_str().expect("id").to_owned()
}

async fn deliveries(p: &Platform, subscription: &str) -> Vec<(String, Value)> {
    let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT kind, payload FROM webhook_deliveries WHERE subscription_id = $1::uuid ORDER BY created_at",
    )
    .bind(subscription)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("deliveries");
    rows.into_iter()
        .map(|(kind, payload)| {
            (
                kind,
                serde_json::from_slice(&payload).expect("json payload"),
            )
        })
        .collect()
}

async fn second_namespace(p: &Platform) -> String {
    let (status, workspaces) = p.rest(Method::GET, "/workspaces", None).await;
    assert_eq!(status, 200, "{workspaces}");
    let workspace = workspaces["items"][0]["id"]
        .as_str()
        .expect("workspace")
        .to_owned();
    let (status, created) = p
        .rest(
            Method::POST,
            &format!("/workspaces/{workspace}/namespaces"),
            Some(&json!({"slug": "billing", "environment": "dev"})),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    created["id"].as_str().expect("id").to_owned()
}

#[tokio::test]
async fn us03_an_outbox_that_waits_too_long_raises_an_alert_for_its_namespace_with_the_runbook_and_resolves_when_it_drains()
 {
    let p = platform().await;
    let payments = p.namespace_id().await;
    let billing = second_namespace(&p).await;
    let subscription = subscribe(&p, &payments, "ops").await;
    let other_subscription = subscribe(&p, &billing, "ops").await;

    // A run starts; nothing publishes its hint (no dispatcher is running), so the message waits.
    let workflow = p.publish("stuck", &single_ir("stuck", None)).await;
    p.start(&workflow, &json!({})).await;
    assert!(p.outbox_backlog().await >= 1);
    let monitor = monitor(&p);

    // Young messages are not a condition.
    let round = monitor.run_once().await;
    assert_eq!((round.fired, round.unseen), (0, 0), "{round:?}");
    assert!(alerts(&p, &payments, "").await.is_empty());

    // Two hours pass.
    sqlx::query("UPDATE event_outbox SET created_at = clock_timestamp() - interval '2 hours' WHERE delivered_at IS NULL")
        .execute(&p.api.db.pool)
        .await
        .expect("time passes");
    let round = monitor.run_once().await;
    assert_eq!((round.fired, round.unseen), (1, 0), "{round:?}");

    let listed = alerts(&p, &payments, "").await;
    let stuck = of_kind(&listed, "OUTBOX_STUCK");
    assert_eq!(stuck.len(), 1, "{listed:?}");
    let alert = stuck[0];
    assert_eq!(alert["state"], "FIRING");
    assert_eq!(alert["severity"], "critical");
    assert_eq!(
        alert["namespaceId"],
        payments.as_str(),
        "the namespace the message belongs to"
    );
    assert!(
        alert["runbook"]
            .as_str()
            .expect("runbook")
            .ends_with("docs/runbooks/alerts.md#outbox-stuck"),
        "{alert}"
    );
    assert!(alert["items"].as_u64().expect("items") >= 1);
    assert!(
        alert["oldestSeconds"].as_f64().expect("age") >= 7000.0,
        "{alert}"
    );
    assert_eq!(alert["firedCount"], 1);
    // The other namespace of the same tenant has none.
    assert!(alerts(&p, &billing, "").await.is_empty());
    // And nothing in it is data of a run.
    let raw = alert.to_string();
    assert!(!raw.contains("act.a") && !raw.contains(&workflow), "{raw}");

    // The subscribers of that namespace are told, once; the other namespace's are not.
    let told = deliveries(&p, &subscription).await;
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0].0, "ops.alert.fired");
    assert_eq!(told[0].1["data"]["alertId"], alert["id"]);
    assert_eq!(told[0].1["data"]["namespaceId"], payments.as_str());
    assert_eq!(told[0].1["data"]["kind"], "OUTBOX_STUCK");
    assert!(deliveries(&p, &other_subscription).await.is_empty());

    // Looking again does not raise it again.
    let round = monitor.run_once().await;
    assert_eq!((round.fired, round.resolved), (0, 0), "{round:?}");
    assert_eq!(deliveries(&p, &subscription).await.len(), 1);

    // The messages are delivered: the alert resolves, and the subscribers are told.
    sqlx::query(
        "UPDATE event_outbox SET delivered_at = clock_timestamp() WHERE delivered_at IS NULL",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("drained");
    let round = monitor.run_once().await;
    assert_eq!((round.fired, round.resolved), (0, 1), "{round:?}");
    let resolved = alerts(&p, &payments, "?state=RESOLVED").await;
    assert_eq!(of_kind(&resolved, "OUTBOX_STUCK").len(), 1);
    assert!(
        of_kind(
            &alerts(&p, &payments, "?state=FIRING").await,
            "OUTBOX_STUCK"
        )
        .is_empty()
    );
    let told = deliveries(&p, &subscription).await;
    assert_eq!(
        told.iter()
            .map(|(kind, _)| kind.as_str())
            .collect::<Vec<_>>(),
        ["ops.alert.fired", "ops.alert.resolved"]
    );

    // It comes back: it fires again, as the second time.
    sqlx::query("INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload, created_at) \
                 SELECT gen_random_uuid(), tenant_id, aggregate_id, event_type, payload, clock_timestamp() - interval '3 hours' \
                 FROM event_outbox LIMIT 1")
        .execute(&p.api.db.pool)
        .await
        .expect("another message that waits");
    let round = monitor.run_once().await;
    assert_eq!(round.fired, 1, "{round:?}");
    let again = alerts(&p, &payments, "?state=FIRING").await;
    assert_eq!(of_kind(&again, "OUTBOX_STUCK")[0]["firedCount"], 2);
}

#[tokio::test]
async fn us03_a_lease_that_ran_out_and_was_not_taken_back_raises_an_alert_and_the_scheduler_resolves_it()
 {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let payments = p.namespace_id().await;
    let workflow = p.publish("leased", &single_ir("leased", None)).await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let _lease = worker.one_task().await;
    let monitor = monitor(&p);

    // The worker is alive and holds its lease: nothing to say.
    assert_eq!(monitor.run_once().await.fired, 0);

    // Its lease ran out long ago and the scheduler did not run.
    sqlx::query("UPDATE steps SET lease_expires_at = clock_timestamp() - interval '10 minutes', attempt_deadline_at = NULL WHERE run_id = $1::uuid")
        .bind(&run)
        .execute(&p.api.db.pool)
        .await
        .expect("the lease ran out");
    assert_eq!(monitor.run_once().await.fired, 1);
    let listed = alerts(&p, &payments, "?state=FIRING").await;
    let alert = of_kind(&listed, "LEASE_NOT_REAPED")[0];
    assert_eq!(alert["severity"], "critical");
    assert!(
        alert["runbook"]
            .as_str()
            .expect("runbook")
            .ends_with("#lease-not-reaped")
    );
    assert!(alert["oldestSeconds"].as_f64().expect("age") >= 590.0);

    // The scheduler takes the attempt back; the alert resolves.
    p.tick().await;
    let round = monitor.run_once().await;
    assert_eq!(round.resolved, 1, "{round:?}");
    assert!(
        of_kind(
            &alerts(&p, &payments, "?state=FIRING").await,
            "LEASE_NOT_REAPED"
        )
        .is_empty()
    );
}

#[tokio::test]
async fn us03_a_step_no_worker_takes_is_an_alert_about_its_activity_and_not_about_its_data() {
    let p = platform().await;
    let payments = p.namespace_id().await;
    let workflow = p.publish("unserved", &single_ir("unserved", None)).await;
    p.start(
        &workflow,
        &json!({"secret-looking": "value-that-must-not-leak"}),
    )
    .await;
    sqlx::query(
        "UPDATE steps SET ready_at = clock_timestamp() - interval '1 hour' WHERE status = 'READY'",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("time passes");
    assert_eq!(monitor(&p).run_once().await.fired, 1);
    let listed = alerts(&p, &payments, "").await;
    let alert = of_kind(&listed, "STEP_UNCLAIMED")[0];
    assert_eq!(alert["subject"], "act.a");
    assert_eq!(alert["severity"], "warning");
    assert!(!alert.to_string().contains("value-that-must-not-leak"));
}

#[tokio::test]
async fn us03_only_those_who_may_see_operations_see_alerts_and_only_in_their_own_tenant() {
    let p = platform().await;
    let payments = p.namespace_id().await;
    p.grant(p.tenant, "viewer-1", sagat_domain::identity::Role::Viewer)
        .await;
    p.grant(p.tenant, "worker-9", sagat_domain::identity::Role::Worker)
        .await;
    let (status, _) = p
        .rest_as(
            "viewer-1",
            Method::GET,
            &format!("/namespaces/{payments}/alerts"),
            None,
        )
        .await;
    assert_eq!(status, 200, "a viewer sees alerts");
    let (status, body) = p
        .rest_as(
            "worker-9",
            Method::GET,
            &format!("/namespaces/{payments}/alerts"),
            None,
        )
        .await;
    assert_eq!(status, 403, "a worker identity does not: {body}");
    // The other tenant's administrator asks for this tenant's namespace: it does not exist for them.
    let response = p
        .api
        .as_user(
            Method::GET,
            &format!("/namespaces/{payments}/alerts"),
            "bob",
            "globex",
        )
        .send()
        .await
        .expect("send");
    assert_eq!(response.status().as_u16(), 404);
    let (status, body) = p
        .rest(
            Method::GET,
            &format!("/namespaces/{payments}/alerts?state=SLEEPING"),
            None,
        )
        .await;
    assert_eq!(status, 422, "{body}");
}

/// The AsyncAPI document the platform promises its subscribers.
fn asyncapi() -> Value {
    serde_saphyr::from_str(include_str!(
        "../../../specs/001-platform-foundation/contracts/events.asyncapi.yaml"
    ))
    .expect("the AsyncAPI document")
}

fn validator(schema: &Value) -> jsonschema::Validator {
    let mut root = schema.clone();
    root.as_object_mut()
        .expect("object")
        .insert("components".into(), asyncapi()["components"].clone());
    jsonschema::options()
        .should_validate_formats(true)
        .build(&root)
        .expect("a valid schema")
}

#[tokio::test]
async fn us03_the_alert_a_subscriber_is_sent_conforms_to_the_document() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let payments = p.namespace_id().await;
    let subscription = subscribe(&p, &payments, "ops").await;
    let workflow = p.publish("unserved", &single_ir("unserved", None)).await;
    p.start(&workflow, &json!({})).await;
    sqlx::query(
        "UPDATE steps SET ready_at = clock_timestamp() - interval '1 hour' WHERE status = 'READY'",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("time passes");
    let monitor = monitor(&p);
    monitor.run_once().await;
    sqlx::query("UPDATE steps SET status = 'CANCELLED', ready_at = NULL, next_attempt_at = NULL WHERE status = 'READY'")
        .execute(&p.api.db.pool)
        .await
        .ok();
    sqlx::query("UPDATE steps SET ready_at = clock_timestamp() WHERE status = 'READY'")
        .execute(&p.api.db.pool)
        .await
        .expect("served");
    monitor.run_once().await;

    let doc = asyncapi();
    let schema = &doc["components"]["messages"]["OperationalAlertWebhook"]["payload"];
    let told = deliveries(&p, &subscription).await;
    assert_eq!(told.len(), 2, "{told:?}");
    for (kind, payload) in &told {
        let errors: Vec<String> = validator(schema)
            .iter_errors(payload)
            .map(|e| format!("{e} (at {})", e.instance_path()))
            .collect();
        assert!(
            errors.is_empty(),
            "{kind} does not conform: {}\n{payload}",
            errors.join("; ")
        );
        assert_eq!(payload["type"], kind.as_str());
    }
    let ids: std::collections::HashSet<_> = told
        .iter()
        .map(|(_, p)| p["id"].as_str().expect("id").to_owned())
        .collect();
    assert_eq!(
        ids.len(),
        2,
        "fired and resolved are two events with two ids"
    );
}
