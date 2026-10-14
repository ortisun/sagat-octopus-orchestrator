//! Live conformance of the operations marked `x-implemented: '004'`.

use std::collections::BTreeSet;

use reqwest::Method;
use sagat_domain::identity::{Membership, Role};
use serde_json::{Value, json};

use crate::support::api::{TestApi, start};
use crate::support::contract::{As, OpenApi, Scenario, check, check_body, scenario};
use crate::support::{now, subject};

const SECRET: &str = "whsec-0123456789abcdef0123456789abcdef";
const MISSING: &str = "019b5d6e-0000-7000-8000-000000000000";

async fn grant(api: &TestApi, tenant: sagat_domain::identity::TenantId, sub: &str, role: Role) {
    api.owner_store
        .insert_membership(&Membership::restore(tenant, subject(sub), role, now()))
        .await
        .expect("grant");
}

/// The headers a provider sends to sign `body` for `event_id`, `age_seconds` ago.
fn signed_headers(
    secret: &str,
    event_id: &str,
    body: &Value,
    age_seconds: i64,
) -> Vec<(&'static str, String)> {
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp() - age_seconds;
    let bytes = serde_json::to_vec(body).expect("json");
    vec![
        (
            "x-octopus-signature",
            crate::support::signed::inbound(secret, timestamp, event_id, "invoice.paid", &bytes),
        ),
        ("x-octopus-timestamp", timestamp.to_string()),
        ("x-octopus-event-id", event_id.to_owned()),
        ("x-octopus-event-type", "invoice.paid".to_owned()),
    ]
}

/// As [`signed_headers`], for a request that names no event type (the endpoint's default applies):
/// the signature covers an absent type.
fn signed_headers_untyped(
    secret: &str,
    event_id: &str,
    body: &Value,
    age_seconds: i64,
) -> Vec<(&'static str, String)> {
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp() - age_seconds;
    let bytes = serde_json::to_vec(body).expect("json");
    vec![
        (
            "x-octopus-signature",
            crate::support::signed::inbound(secret, timestamp, event_id, "", &bytes),
        ),
        ("x-octopus-timestamp", timestamp.to_string()),
        ("x-octopus-event-id", event_id.to_owned()),
    ]
}

fn with(mut scenario: Scenario, headers: Vec<(&'static str, String)>) -> Scenario {
    for (name, value) in headers {
        scenario = scenario.with_header(name, value);
    }
    scenario
}

struct Run<'a> {
    api: &'a TestApi,
    spec: &'a OpenApi,
    covered: BTreeSet<(String, String)>,
}

impl Run<'_> {
    async fn all(&mut self, scenarios: Vec<Scenario>) {
        for s in &scenarios {
            let (method, template, _) = check(self.api, self.spec, s).await;
            self.covered.insert((method, template));
        }
    }

    async fn one(&mut self, s: Scenario) -> Value {
        let (method, template, _, body) = check_body(self.api, self.spec, &s).await;
        self.covered.insert((method, template));
        body
    }
}

#[tokio::test]
async fn every_event_operation_conforms_to_the_contract_on_its_normal_and_error_paths() {
    let spec = OpenApi::load();
    let api = start().await;
    let acme = api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    grant(&api, acme, "vera", Role::Viewer).await;
    grant(&api, acme, "dave", Role::Developer).await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    let ns = api
        .create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;
    let globex_ws = api.create_workspace("bob", "globex", "platform").await;
    api.create_namespace("bob", "globex", &globex_ws, "payments", "dev")
        .await;

    let alice = As::User("alice", "acme");
    let vera = As::User("vera", "acme");
    let dave = As::User("dave", "acme");
    let bob = As::User("bob", "globex");

    // The signing secret, the right of a developer to use it, and where the namespace may deliver.
    for (method, path, body) in [
        (
            Method::PUT,
            format!("/namespaces/{ns}/secrets/provider-signing"),
            Some(json!({"value": SECRET})),
        ),
        (
            Method::PUT,
            format!("/namespaces/{ns}/secrets/provider-signing/access/dave"),
            None,
        ),
        (
            Method::PUT,
            format!("/namespaces/{ns}/egress-policy"),
            Some(
                json!({"rules": ["hooks.example.com", "kafka.example.com"], "ports": [443, 9092]}),
            ),
        ),
    ] {
        let mut request = api.as_user(method, &path, "alice", "acme");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let status = request.send().await.expect("send").status();
        assert!(status == 200 || status == 204, "{path}: {status}");
    }

    let mut run = Run {
        api: &api,
        spec: &spec,
        covered: BTreeSet::new(),
    };

    // ------------------------------------------------------------------ event endpoints
    let endpoints_t = "/namespaces/{namespaceId}/event-endpoints";
    let endpoint_t = "/namespaces/{namespaceId}/event-endpoints/{endpointId}";
    let endpoints = format!("/namespaces/{ns}/event-endpoints");
    let create = |who: &As, body: Value, expect: u16| {
        scenario(
            "POST",
            endpoints_t,
            &endpoints,
            who.clone(),
            Some(body),
            expect,
        )
    };
    let plain = json!({"slug": "billing", "secretName": "provider-signing"});
    run.all(vec![
        create(&As::Anonymous, plain.clone(), 401),
        create(&As::Garbage, plain.clone(), 401),
        create(&vera, plain.clone(), 403),
        create(&bob, plain.clone(), 404),
        create(&alice, json!({"slug": "Bad_Slug", "secretName": "provider-signing"}), 422),
        create(&alice, json!({"slug": "api", "secretName": "provider-signing"}), 422),
        create(&alice, json!({"slug": "billing", "secretName": "provider-signing", "extra": 1}), 422),
        create(&alice, json!({"slug": "billing", "secretName": "provider-signing", "status": "SLEEPING"}), 422),
        create(&alice, json!({"slug": "billing", "secretName": "provider-signing", "skewSeconds": 600, "dedupeTtlSeconds": 600}), 422),
        create(&alice, json!({"slug": "billing", "secretName": "no-such-secret"}), 403),
    ])
    .await;
    // A developer may manage endpoints with a secret they may use.
    run.one(create(
        &dave,
        json!({"slug": "ledger", "secretName": "provider-signing", "maxBodyBytes": 2048}),
        201,
    ))
    .await;
    let billing = run
        .one(create(
            &alice,
            json!({"slug": "billing", "secretName": "provider-signing"}),
            201,
        ))
        .await;
    let intake = run
        .one(create(&alice, json!({"slug": "intake", "secretName": "provider-signing", "defaultEventType": "invoice.paid", "ratePerSecond": 1}), 201))
        .await;
    let billing_id = billing["id"].as_str().expect("id").to_owned();
    let intake_id = intake["id"].as_str().expect("id").to_owned();
    assert_eq!(billing["webhookPath"], format!("/v1/webhooks/{billing_id}"));
    assert_eq!(billing["status"], "ACTIVE");

    let one = |id: &str| format!("/namespaces/{ns}/event-endpoints/{id}");
    run.all(vec![
        create(&alice, plain.clone(), 409),
        scenario("GET", endpoints_t, &endpoints, vera.clone(), None, 200),
        scenario("GET", endpoints_t, &endpoints, bob.clone(), None, 404),
        scenario("GET", endpoints_t, &endpoints, As::Anonymous, None, 401),
        scenario(
            "GET",
            endpoint_t,
            &one(&billing_id),
            vera.clone(),
            None,
            200,
        ),
        scenario("GET", endpoint_t, &one(&billing_id), bob.clone(), None, 404),
        scenario("GET", endpoint_t, &one(MISSING), alice.clone(), None, 404),
        scenario(
            "GET",
            endpoint_t,
            &one("not-an-id"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "PUT",
            endpoint_t,
            &one(&billing_id),
            vera.clone(),
            Some(json!({"secretName": "provider-signing"})),
            403,
        ),
        scenario(
            "PUT",
            endpoint_t,
            &one(&billing_id),
            bob.clone(),
            Some(json!({"secretName": "provider-signing"})),
            404,
        ),
        scenario(
            "PUT",
            endpoint_t,
            &one(&billing_id),
            alice.clone(),
            Some(json!({"secretName": "provider-signing", "status": "SLEEPING"})),
            422,
        ),
        scenario(
            "PUT",
            endpoint_t,
            &one(&billing_id),
            alice.clone(),
            Some(json!({"secretName": "provider-signing", "defaultEventType": "invoice.paid"})),
            200,
        ),
        scenario(
            "PUT",
            endpoint_t,
            &one(MISSING),
            alice.clone(),
            Some(json!({"secretName": "provider-signing"})),
            404,
        ),
    ])
    .await;

    // ------------------------------------------------------------------ publishing and reading events
    let publish_t = "/events";
    let event = |id: &str, amount: u32| json!({"eventId": id, "type": "order.created", "correlationKey": "order-7", "data": {"amount": amount}});
    let in_payments = |s: Scenario| s.with_header("x-octopus-namespace", "payments");
    run.all(vec![
        in_payments(scenario(
            "POST",
            publish_t,
            "/events",
            alice.clone(),
            Some(event("order-1", 10)),
            202,
        )),
        in_payments(scenario(
            "POST",
            publish_t,
            "/events",
            alice.clone(),
            Some(event("order-1", 10)),
            202,
        )),
        in_payments(scenario(
            "POST",
            publish_t,
            "/events",
            alice.clone(),
            Some(event("order-1", 99)),
            409,
        )),
        in_payments(scenario(
            "POST",
            publish_t,
            "/events",
            alice.clone(),
            Some(json!({"type": "order.created", "data": {}})),
            202,
        )),
        in_payments(scenario(
            "POST",
            publish_t,
            "/events",
            alice.clone(),
            Some(json!({"eventId": "o-2", "type": "order.created", "data": [1]})),
            422,
        )),
        in_payments(scenario(
            "POST",
            publish_t,
            "/events",
            alice.clone(),
            Some(json!({"eventId": "has space", "type": "order.created", "data": {}})),
            422,
        )),
        in_payments(scenario(
            "POST",
            publish_t,
            "/events",
            alice.clone(),
            Some(json!({"eventId": "o-3", "type": "Not A Type!", "data": {}})),
            422,
        )),
        in_payments(scenario(
            "POST",
            publish_t,
            "/events",
            alice.clone(),
            Some(json!({"eventId": "o-3", "type": "order.created", "data": {}, "extra": true})),
            422,
        )),
        in_payments(scenario(
            "POST",
            publish_t,
            "/events",
            vera.clone(),
            Some(event("order-2", 1)),
            403,
        )),
        in_payments(scenario(
            "POST",
            publish_t,
            "/events",
            As::Anonymous,
            Some(event("order-2", 1)),
            401,
        )),
        scenario(
            "POST",
            publish_t,
            "/events",
            alice.clone(),
            Some(event("order-2", 1)),
            400,
        ),
        scenario(
            "POST",
            publish_t,
            "/events",
            alice.clone(),
            Some(event("order-2", 1)),
            404,
        )
        .with_header("x-octopus-namespace", "elsewhere"),
    ])
    .await;
    let inbox_t = "/namespaces/{namespaceId}/events/{source}/{eventId}";
    run.all(vec![
        scenario(
            "GET",
            inbox_t,
            &format!("/namespaces/{ns}/events/api/order-1"),
            vera.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            inbox_t,
            &format!("/namespaces/{ns}/events/api/order-1"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            inbox_t,
            &format!("/namespaces/{ns}/events/api/never-sent"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            inbox_t,
            &format!("/namespaces/{ns}/events/NOT_A_SOURCE/order-1"),
            alice.clone(),
            None,
            404,
        ),
    ])
    .await;

    // ------------------------------------------------------------------ the public URL
    let hook_t = "/webhooks/{endpointId}";
    let hook = |id: &str| format!("/webhooks/{id}");
    let body = json!({"invoice": "inv-1", "amount": 10});
    let post = |id: &str, headers: Vec<(&'static str, String)>, body: Value, expect: u16| {
        with(
            scenario("POST", hook_t, &hook(id), As::Anonymous, Some(body), expect),
            headers,
        )
    };
    run.all(vec![
        post(
            &billing_id,
            signed_headers(SECRET, "evt-1", &body, 0),
            body.clone(),
            202,
        ),
        // A copy of the same request, replayed: the same answer, no second event.
        post(
            &billing_id,
            signed_headers(SECRET, "evt-1", &body, 0),
            body.clone(),
            202,
        ),
        // Everything that does not verify is the same 401.
        post(
            &billing_id,
            signed_headers("another-secret-another-secret-0000", "evt-2", &body, 0),
            body.clone(),
            401,
        ),
        post(
            &billing_id,
            signed_headers(SECRET, "evt-2", &body, 3600),
            body.clone(),
            401,
        ),
        post(&billing_id, vec![], body.clone(), 401),
        post(
            &billing_id,
            signed_headers(SECRET, "evt-2", &json!({"tampered": true}), 0),
            body.clone(),
            401,
        ),
        post(
            MISSING,
            signed_headers(SECRET, "evt-2", &body, 0),
            body.clone(),
            401,
        ),
        post(
            "not-an-id",
            signed_headers(SECRET, "evt-2", &body, 0),
            body.clone(),
            401,
        ),
        // The same id for another payload is a conflict, and a body that is not an event is refused.
        post(
            &billing_id,
            signed_headers(SECRET, "evt-1", &json!({"amount": 11}), 0),
            json!({"amount": 11}),
            409,
        ),
        post(
            &billing_id,
            signed_headers(SECRET, "evt-3", &json!([1, 2]), 0),
            json!([1, 2]),
            422,
        ),
        // An endpoint with a default type takes a request that names none.
        post(
            &intake_id,
            signed_headers_untyped(SECRET, "evt-4", &body, 0),
            body.clone(),
            202,
        ),
    ])
    .await;
    // An endpoint over its rate (one a second) asks the sender to wait.
    let mut limited = 0;
    for i in 0..6 {
        let id = format!("burst-{i}");
        let s = post(
            &intake_id,
            signed_headers(SECRET, &id, &body, 0),
            body.clone(),
            202,
        );
        let response = api.request(Method::POST, &hook(&intake_id)).json(&body);
        let mut response = response;
        for (name, value) in &s.headers {
            response = response.header(*name, value);
        }
        let response = response.send().await.expect("send");
        if response.status() == 429 {
            limited += 1;
            assert!(
                response.headers().contains_key("retry-after"),
                "a refusal to wait says for how long"
            );
        }
    }
    assert!(limited >= 3, "{limited} of 6 were over the rate");
    // A paused endpoint asks the provider to retry, and a deleted one is a 401 like any unknown.
    run.all(vec![
        scenario(
            "PUT",
            endpoint_t,
            &one(&intake_id),
            alice.clone(),
            Some(json!({"secretName": "provider-signing", "status": "PAUSED"})),
            200,
        ),
        post(
            &intake_id,
            signed_headers(SECRET, "evt-5", &body, 0),
            body.clone(),
            503,
        ),
        scenario(
            "DELETE",
            endpoint_t,
            &one(&intake_id),
            vera.clone(),
            None,
            403,
        ),
        scenario(
            "DELETE",
            endpoint_t,
            &one(&intake_id),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "DELETE",
            endpoint_t,
            &one(&intake_id),
            alice.clone(),
            None,
            204,
        ),
        scenario(
            "DELETE",
            endpoint_t,
            &one(&intake_id),
            alice.clone(),
            None,
            404,
        ),
        post(
            &intake_id,
            signed_headers(SECRET, "evt-6", &body, 0),
            body.clone(),
            401,
        ),
    ])
    .await;

    // ------------------------------------------------------------------ outbound webhook subscriptions
    let subs_t = "/namespaces/{namespaceId}/webhook-subscriptions";
    let sub_t = "/namespaces/{namespaceId}/webhook-subscriptions/{subscriptionId}";
    let deliveries_t =
        "/namespaces/{namespaceId}/webhook-subscriptions/{subscriptionId}/deliveries";
    let delivery_t =
        "/namespaces/{namespaceId}/webhook-subscriptions/{subscriptionId}/deliveries/{deliveryId}";
    let redrive_one_t = "/namespaces/{namespaceId}/webhook-subscriptions/{subscriptionId}/deliveries/{deliveryId}/redrive";
    let redrive_t = "/namespaces/{namespaceId}/webhook-subscriptions/{subscriptionId}/redrive";
    let subs = format!("/namespaces/{ns}/webhook-subscriptions");
    let subscribe = |who: &As, body: Value, expect: u16| {
        scenario("POST", subs_t, &subs, who.clone(), Some(body), expect)
    };
    let good = json!({"name": "notify", "url": "https://hooks.example.com/in", "secretName": "provider-signing", "events": ["run.finished"]});
    run.all(vec![
        subscribe(&As::Anonymous, good.clone(), 401),
        subscribe(&vera, good.clone(), 403),
        subscribe(&bob, good.clone(), 404),
        subscribe(&alice, json!({"name": "notify", "url": "https://elsewhere.example.net/in", "secretName": "provider-signing", "events": ["run.finished"]}), 422),
        subscribe(&alice, json!({"name": "notify", "url": "http://hooks.example.com/in", "secretName": "provider-signing", "events": ["run.finished"]}), 422),
        subscribe(&alice, json!({"name": "notify", "url": "https://user:pw@hooks.example.com/in", "secretName": "provider-signing", "events": ["run.finished"]}), 422),
        subscribe(&alice, json!({"name": "notify", "url": "https://hooks.example.com/in", "secretName": "provider-signing", "events": []}), 422),
        subscribe(&alice, json!({"name": "notify", "url": "https://hooks.example.com/in", "secretName": "provider-signing", "events": ["run.started"]}), 422),
        subscribe(&alice, json!({"name": "notify", "url": "https://hooks.example.com/in", "secretName": "provider-signing", "events": ["run.finished"], "ratePerSecond": 500}), 422),
        subscribe(&alice, json!({"name": "notify", "url": "https://hooks.example.com/in", "secretName": "provider-signing", "events": ["run.finished"], "status": "DISABLED"}), 422),
        subscribe(&alice, json!({"name": "notify", "url": "https://hooks.example.com/in", "secretName": "no-such-secret", "events": ["run.finished"]}), 403),
        subscribe(&alice, json!({"name": "notify", "url": "https://hooks.example.com/in", "secretName": "provider-signing", "events": ["run.finished"], "extra": 1}), 422),
    ])
    .await;
    let created = run.one(subscribe(&alice, good.clone(), 201)).await;
    let sub_id = created["id"].as_str().expect("id").to_owned();
    assert_eq!(created["status"], "ACTIVE");
    assert_eq!(created["health"]["consecutiveDead"], 0);
    let sub = |id: &str| format!("/namespaces/{ns}/webhook-subscriptions/{id}");
    let update = json!({"url": "https://hooks.example.com/in", "secretName": "provider-signing", "events": ["run.finished"], "ratePerSecond": 5});

    // A delivery that is dead, put there as a dead delivery gets there.
    let delivery_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO webhook_deliveries (id, tenant_id, namespace_id, subscription_id, event_id, kind, \
         payload, status, attempts, next_attempt_at, last_outcome, last_http_status, last_error) \
         SELECT $1, tenant_id, namespace_id, id, 'event-9', 'run.finished', '{\"id\":\"event-9\"}', 'DEAD', 1, \
                clock_timestamp(), 'REJECTED', 400, 'the receiver refused the event' \
         FROM webhook_subscriptions WHERE id = $2::uuid",
    )
    .bind(delivery_id)
    .bind(&sub_id)
    .execute(&api.db.pool)
    .await
    .expect("a dead delivery");
    sqlx::query(
        "INSERT INTO webhook_attempts (delivery_id, attempt, tenant_id, started_at, duration_ms, outcome, http_status, error) \
         SELECT $1, 1, tenant_id, clock_timestamp(), 12, 'REJECTED', 400, 'the receiver refused the event' \
         FROM webhook_deliveries WHERE id = $1",
    )
    .bind(delivery_id)
    .execute(&api.db.pool)
    .await
    .expect("its attempt");
    let deliveries = format!("/namespaces/{ns}/webhook-subscriptions/{sub_id}/deliveries");
    let one_delivery = |id: &str| format!("{deliveries}/{id}");
    run.all(vec![
        subscribe(&alice, good.clone(), 409),
        scenario("GET", subs_t, &subs, vera.clone(), None, 200),
        scenario("GET", subs_t, &subs, bob.clone(), None, 404),
        scenario("GET", sub_t, &sub(&sub_id), vera.clone(), None, 200),
        scenario("GET", sub_t, &sub(&sub_id), bob.clone(), None, 404),
        scenario("GET", sub_t, &sub(MISSING), alice.clone(), None, 404),
        scenario("PUT", sub_t, &sub(&sub_id), vera.clone(), Some(update.clone()), 403),
        scenario("PUT", sub_t, &sub(&sub_id), bob.clone(), Some(update.clone()), 404),
        scenario("PUT", sub_t, &sub(&sub_id), alice.clone(), Some(json!({"url": "https://hooks.example.com/in", "secretName": "provider-signing", "events": ["run.finished"], "status": "DISABLED"})), 422),
        scenario("PUT", sub_t, &sub(&sub_id), alice.clone(), Some(update.clone()), 200),
        scenario("GET", deliveries_t, &deliveries, vera.clone(), None, 200),
        scenario("GET", deliveries_t, &format!("{deliveries}?status=DEAD&limit=10"), alice.clone(), None, 200),
        scenario("GET", deliveries_t, &format!("{deliveries}?status=BROKEN"), alice.clone(), None, 400),
        scenario("GET", deliveries_t, &format!("{deliveries}?cursor=not-a-cursor"), alice.clone(), None, 400),
        scenario("GET", deliveries_t, &deliveries, bob.clone(), None, 404),
        scenario("GET", delivery_t, &one_delivery(&delivery_id.to_string()), vera.clone(), None, 200),
        scenario("GET", delivery_t, &one_delivery(MISSING), alice.clone(), None, 404),
        scenario("GET", delivery_t, &one_delivery(&delivery_id.to_string()), bob.clone(), None, 404),
        scenario("POST", redrive_one_t, &format!("{}/redrive", one_delivery(&delivery_id.to_string())), vera.clone(), None, 403),
        scenario("POST", redrive_one_t, &format!("{}/redrive", one_delivery(&delivery_id.to_string())), alice.clone(), None, 200),
        // It is no longer dead, so there is nothing to put back.
        scenario("POST", redrive_one_t, &format!("{}/redrive", one_delivery(&delivery_id.to_string())), alice.clone(), None, 409),
        scenario("POST", redrive_t, &format!("/namespaces/{ns}/webhook-subscriptions/{sub_id}/redrive"), vera.clone(), Some(json!({})), 403),
        scenario("POST", redrive_t, &format!("/namespaces/{ns}/webhook-subscriptions/{sub_id}/redrive"), alice.clone(), Some(json!({"limit": 5})), 200),
        scenario("POST", redrive_t, &format!("/namespaces/{ns}/webhook-subscriptions/{sub_id}/redrive"), alice.clone(), Some(json!({"limit": "many"})), 422),
        scenario("DELETE", sub_t, &sub(&sub_id), vera.clone(), None, 403),
        scenario("DELETE", sub_t, &sub(&sub_id), bob.clone(), None, 404),
        scenario("DELETE", sub_t, &sub(&sub_id), alice.clone(), None, 204),
        scenario("DELETE", sub_t, &sub(&sub_id), alice.clone(), None, 404),
    ])
    .await;

    // ------------------------------------------------------------------ outbound broker subscriptions
    let broker_subs_t = "/namespaces/{namespaceId}/broker-subscriptions";
    let broker_sub_t = "/namespaces/{namespaceId}/broker-subscriptions/{subscriptionId}";
    let broker_deliveries_t =
        "/namespaces/{namespaceId}/broker-subscriptions/{subscriptionId}/deliveries";
    let broker_delivery_t =
        "/namespaces/{namespaceId}/broker-subscriptions/{subscriptionId}/deliveries/{deliveryId}";
    let broker_redrive_one_t = "/namespaces/{namespaceId}/broker-subscriptions/{subscriptionId}/deliveries/{deliveryId}/redrive";
    let broker_redrive_t =
        "/namespaces/{namespaceId}/broker-subscriptions/{subscriptionId}/redrive";
    let broker_subs = format!("/namespaces/{ns}/broker-subscriptions");
    let broker_good = json!({
        "name": "broker-notify",
        "kind": "KAFKA",
        "settings": {"servers": ["tls://kafka.example.com:9092"], "topic": "run-finished"},
        "events": ["run.finished"]
    });
    run.all(vec![
        scenario("POST", broker_subs_t, &broker_subs, As::Anonymous, Some(broker_good.clone()), 401),
        scenario("POST", broker_subs_t, &broker_subs, vera.clone(), Some(broker_good.clone()), 403),
        scenario("POST", broker_subs_t, &broker_subs, bob.clone(), Some(broker_good.clone()), 404),
        scenario("POST", broker_subs_t, &broker_subs, alice.clone(), Some(json!({"name": "bad-broker", "kind": "AMQP", "settings": {}, "events": ["run.finished"]})), 422),
    ])
    .await;
    let broker_created = run
        .one(scenario(
            "POST",
            broker_subs_t,
            &broker_subs,
            alice.clone(),
            Some(broker_good.clone()),
            201,
        ))
        .await;
    let broker_id = broker_created["id"].as_str().expect("id").to_owned();
    let broker_sub = |id: &str| format!("/namespaces/{ns}/broker-subscriptions/{id}");
    let broker_update = json!({
        "kind": "KAFKA",
        "settings": {"servers": ["tls://kafka.example.com:9092"], "topic": "run-finished"},
        "events": ["run.finished"],
        "ratePerSecond": 5
    });
    let broker_delivery_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO webhook_deliveries (id, tenant_id, namespace_id, subscription_id, event_id, kind, \
         payload, status, attempts, next_attempt_at, last_outcome, last_error) \
         SELECT $1, tenant_id, namespace_id, id, 'broker-event-9', 'run.finished', \
                '{\"id\":\"broker-event-9\"}', 'DEAD', 1, clock_timestamp(), 'REJECTED', \
                'the broker refused the event' \
         FROM webhook_subscriptions WHERE id = $2::uuid AND destination_kind <> 'WEBHOOK'",
    )
    .bind(broker_delivery_id)
    .bind(&broker_id)
    .execute(&api.db.pool)
    .await
    .expect("a dead broker delivery");
    let broker_deliveries = format!("/namespaces/{ns}/broker-subscriptions/{broker_id}/deliveries");
    let broker_one_delivery = |id: &str| format!("{broker_deliveries}/{id}");
    run.all(vec![
        scenario("GET", broker_subs_t, &broker_subs, vera.clone(), None, 200),
        scenario(
            "GET",
            broker_sub_t,
            &broker_sub(&broker_id),
            vera.clone(),
            None,
            200,
        ),
        scenario(
            "PUT",
            broker_sub_t,
            &broker_sub(&broker_id),
            alice.clone(),
            Some(broker_update),
            200,
        ),
        scenario(
            "GET",
            broker_deliveries_t,
            &broker_deliveries,
            vera.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            broker_delivery_t,
            &broker_one_delivery(&broker_delivery_id.to_string()),
            vera.clone(),
            None,
            200,
        ),
        scenario(
            "POST",
            broker_redrive_one_t,
            &format!(
                "{}/redrive",
                broker_one_delivery(&broker_delivery_id.to_string())
            ),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "POST",
            broker_redrive_t,
            &format!("/namespaces/{ns}/broker-subscriptions/{broker_id}/redrive"),
            alice.clone(),
            Some(json!({"limit": 5})),
            200,
        ),
        scenario(
            "DELETE",
            broker_sub_t,
            &broker_sub(&broker_id),
            alice.clone(),
            None,
            204,
        ),
    ])
    .await;

    // ------------------------------------------------------------------ connectors
    let conns_t = "/namespaces/{namespaceId}/connectors";
    let conn_t = "/namespaces/{namespaceId}/connectors/{connectorId}";
    let letters_t = "/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters";
    let letter_t = "/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters/{deadLetterId}";
    let redrive_letter_t =
        "/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters/{deadLetterId}/redrive";
    let conns = format!("/namespaces/{ns}/connectors");
    let kafka = json!({"servers": ["tls://kafka.example.com:9092"], "topic": "orders"});
    let make = |who: &As, body: Value, expect: u16| {
        scenario("POST", conns_t, &conns, who.clone(), Some(body), expect)
    };
    let good_connector = json!({"slug": "orders", "kind": "KAFKA", "settings": kafka});
    run.all(vec![
        make(&As::Anonymous, good_connector.clone(), 401),
        make(&vera, good_connector.clone(), 403),
        make(&bob, good_connector.clone(), 404),
        make(&alice, json!({"slug": "orders", "kind": "AMQP", "settings": kafka}), 422),
        make(&alice, json!({"slug": "orders", "kind": "KAFKA", "settings": {"topic": "orders"}}), 422),
        make(&alice, json!({"slug": "orders", "kind": "KAFKA", "settings": {"servers": ["tls://kafka.example.com:9092"], "topic": "orders", "extra": 1}}), 422),
        make(&alice, json!({"slug": "orders", "kind": "KAFKA", "settings": {"servers": ["not a broker"], "topic": "orders"}}), 422),
        make(&alice, json!({"slug": "orders", "kind": "KAFKA", "settings": {"servers": ["tls://elsewhere.example.net:9092"], "topic": "orders"}}), 422),
        make(&alice, json!({"slug": "orders", "kind": "KAFKA", "settings": {"servers": ["kafka.example.com:9092"], "topic": "orders"}}), 422),
        make(&alice, json!({"slug": "cron", "kind": "KAFKA", "settings": kafka}), 422),
        make(&alice, json!({"slug": "orders", "kind": "KAFKA", "settings": kafka, "ratePerSecond": 0}), 422),
        make(&alice, json!({"slug": "orders", "kind": "KAFKA", "settings": kafka, "secretName": "no-such-secret"}), 403),
        make(&alice, json!({"slug": "orders", "kind": "KAFKA", "settings": kafka, "status": "SLEEPING"}), 422),
        make(&alice, json!({"slug": "orders", "kind": "KAFKA", "settings": kafka, "extra": 1}), 422),
        make(&alice, json!({"slug": "billing", "kind": "KAFKA", "settings": kafka}), 409),
    ])
    .await;
    let created = run.one(make(&alice, good_connector.clone(), 201)).await;
    let connector_id = created["id"].as_str().expect("id").to_owned();
    assert_eq!(created["kind"], "KAFKA");
    assert_eq!(created["ratePerSecond"], 200);
    assert_eq!(created["health"]["consumed"], 0);
    let one_connector = |id: &str| format!("/namespaces/{ns}/connectors/{id}");
    let update_body = json!({"kind": "KAFKA", "settings": kafka, "ratePerSecond": 50});

    // A message the connector set aside, as it gets there.
    let first_letter = uuid::Uuid::now_v7();
    let second_letter = uuid::Uuid::now_v7();
    for (id, position) in [(first_letter, "orders/0/1"), (second_letter, "orders/0/2")] {
        sqlx::query(
            "INSERT INTO connector_dead_letters (id, tenant_id, namespace_id, connector_id, position, \
             reason, headers, body) \
             SELECT $1, tenant_id, namespace_id, id, $3, 'NO_EVENT_TYPE', '{\"eventId\": \"m-1\"}', \
                    convert_to('{\"amount\": 5}', 'UTF8') \
             FROM event_connectors WHERE id = $2::uuid",
        )
        .bind(id)
        .bind(&connector_id)
        .bind(position)
        .execute(&api.db.pool)
        .await
        .expect("a dead letter");
    }
    let letters = format!("/namespaces/{ns}/connectors/{connector_id}/dead-letters");
    let one_letter = |id: uuid::Uuid| format!("{letters}/{id}");
    run.all(vec![
        make(&alice, good_connector.clone(), 409),
        scenario("GET", conns_t, &conns, vera.clone(), None, 200),
        scenario("GET", conns_t, &conns, bob.clone(), None, 404),
        scenario("GET", conn_t, &one_connector(&connector_id), vera.clone(), None, 200),
        scenario("GET", conn_t, &one_connector(&connector_id), bob.clone(), None, 404),
        scenario("GET", conn_t, &one_connector(MISSING), alice.clone(), None, 404),
        scenario("PUT", conn_t, &one_connector(&connector_id), vera.clone(), Some(update_body.clone()), 403),
        scenario("PUT", conn_t, &one_connector(&connector_id), bob.clone(), Some(update_body.clone()), 404),
        scenario("PUT", conn_t, &one_connector(&connector_id), alice.clone(), Some(json!({"kind": "SQS", "settings": {"queueUrl": "https://kafka.example.com/1/q", "region": "us-east-1"}})), 422),
        scenario("PUT", conn_t, &one_connector(&connector_id), alice.clone(), Some(json!({"kind": "KAFKA", "settings": kafka, "status": "SLEEPING"})), 422),
        scenario("PUT", conn_t, &one_connector(&connector_id), alice.clone(), Some(update_body.clone()), 200),
        scenario("GET", letters_t, &letters, vera.clone(), None, 200),
        scenario("GET", letters_t, &format!("{letters}?limit=1"), alice.clone(), None, 200),
        scenario("GET", letters_t, &format!("{letters}?cursor=not-a-cursor"), alice.clone(), None, 400),
        scenario("GET", letters_t, &letters, bob.clone(), None, 404),
        scenario("GET", letter_t, &one_letter(first_letter), vera.clone(), None, 200),
        scenario("GET", letter_t, &one_letter(first_letter), bob.clone(), None, 404),
        scenario("GET", letter_t, &one_letter(uuid::Uuid::nil()), alice.clone(), None, 404),
        // No default event type yet: the message is still not an event.
        scenario("POST", redrive_letter_t, &format!("{}/redrive", one_letter(first_letter)), vera.clone(), None, 403),
        scenario("POST", redrive_letter_t, &format!("{}/redrive", one_letter(first_letter)), alice.clone(), None, 422),
        scenario("PUT", conn_t, &one_connector(&connector_id), alice.clone(), Some(json!({"kind": "KAFKA", "settings": kafka, "defaultEventType": "order.created"})), 200),
        scenario("POST", redrive_letter_t, &format!("{}/redrive", one_letter(first_letter)), alice.clone(), None, 200),
        scenario("POST", redrive_letter_t, &format!("{}/redrive", one_letter(first_letter)), alice.clone(), None, 404),
        scenario("DELETE", letter_t, &one_letter(second_letter), vera.clone(), None, 403),
        scenario("DELETE", letter_t, &one_letter(second_letter), bob.clone(), None, 404),
        scenario("DELETE", letter_t, &one_letter(second_letter), alice.clone(), None, 204),
        scenario("DELETE", letter_t, &one_letter(second_letter), alice.clone(), None, 404),
        scenario("DELETE", conn_t, &one_connector(&connector_id), vera.clone(), None, 403),
        scenario("DELETE", conn_t, &one_connector(&connector_id), bob.clone(), None, 404),
        scenario("DELETE", conn_t, &one_connector(&connector_id), alice.clone(), None, 204),
        scenario("DELETE", conn_t, &one_connector(&connector_id), alice.clone(), None, 404),
    ])
    .await;

    // Every operation marked as implemented by this feature was exercised.
    let marked: BTreeSet<_> = spec
        .operations_marked("x-implemented", "004")
        .into_iter()
        .collect();
    assert!(
        !marked.is_empty(),
        "the contract marks the event operations"
    );
    let untested: Vec<_> = marked.difference(&run.covered).collect();
    assert!(
        untested.is_empty(),
        "operations without a conformance scenario: {untested:?}"
    );
}
