//! Real PostgreSQL/API canary evidence for payload boundaries and telemetry (T016-026/T016-049).
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Method, StatusCode};
use sagat_domain::connectors::ConnectorId;
use sagat_domain::identity::{Membership, NamespaceId, Role, TenantId};
use sagat_telemetry::Metrics;
use serde_json::{Value, json};
use uuid::Uuid;

#[path = "../../support/canary.rs"]
mod canary_support;

use canary_support::{Canaries, assert_absent, assert_error, assert_present, audience_cases};

use super::support;
use super::support::api::{TestApi, json_of};
use super::support::collector::Collector;
use super::support::engine::{document, end_node};

const TENANT: &str = "classification-canary";
const NAMESPACE: &str = "payments";
const ADMIN: &str = "admin";

struct Fixture {
    api: TestApi,
    tenant: TenantId,
    namespace: String,
    canaries: Canaries,
}

impl Fixture {
    async fn new(label: &str) -> Self {
        let api = support::api::start().await;
        let tenant = api.tenant(TENANT, ADMIN).await;
        let workspace = api
            .create_workspace(ADMIN, TENANT, "canary-workspace")
            .await;
        let namespace = api
            .create_namespace(ADMIN, TENANT, &workspace, NAMESPACE, "dev")
            .await;
        for (subject, role) in [
            ("viewer-canary", Role::Viewer),
            ("developer-canary", Role::Developer),
            ("operator-canary", Role::Operator),
            ("worker-canary", Role::Worker),
            ("runner-canary", Role::Runner),
        ] {
            api.owner_store
                .insert_membership(&Membership::restore(
                    tenant,
                    support::subject(subject),
                    role,
                    support::now(),
                ))
                .await
                .expect("insert canary membership");
        }
        Self {
            api,
            tenant,
            namespace,
            canaries: Canaries::unique(label),
        }
    }

    async fn run(&self, name: &str, schema: Value, input: Value) -> String {
        let name = format!("{name}-{}", Uuid::now_v7());
        let mut ir = document(&name, "done", vec![end_node("done")]);
        ir["inputSchema"] = schema;
        let created = self
            .api
            .as_user(Method::POST, "/workflows", ADMIN, TENANT)
            .header("x-octopus-namespace", NAMESPACE)
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"name":name, "ir":ir}))
            .send()
            .await
            .expect("create canary workflow");
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = json_of(created).await;
        let workflow = created["id"].as_str().expect("workflow id");
        let revision = created["draftRevision"].as_u64().expect("draft revision");
        let published = self
            .api
            .as_user(
                Method::POST,
                &format!("/workflows/{workflow}/publish"),
                ADMIN,
                TENANT,
            )
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"draftRevision":revision}))
            .send()
            .await
            .expect("publish canary workflow");
        assert_eq!(published.status(), StatusCode::CREATED);
        let started = self
            .api
            .as_user(Method::POST, "/runs", ADMIN, TENANT)
            .header("x-octopus-namespace", NAMESPACE)
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"workflowId":workflow, "input":input}))
            .send()
            .await
            .expect("start canary run");
        assert_eq!(started.status(), StatusCode::ACCEPTED);
        json_of(started).await["runId"]
            .as_str()
            .expect("run id")
            .to_owned()
    }

    async fn set_grant(&self, subject: &str, granted: bool) {
        sqlx::query(
            "DELETE FROM payload_reveal_grants \
             WHERE tenant_id=$1 AND namespace_id=$2 AND subject=$3",
        )
        .bind(self.tenant.as_uuid())
        .bind(namespace_id(&self.namespace).as_uuid())
        .bind(subject)
        .execute(&self.api.db.pool)
        .await
        .expect("reset canary grant");
        if granted {
            sqlx::query(
                "INSERT INTO payload_reveal_grants \
                 (tenant_id,namespace_id,subject,granted_by,granted_at) \
                 VALUES ($1,$2,$3,$4,clock_timestamp())",
            )
            .bind(self.tenant.as_uuid())
            .bind(namespace_id(&self.namespace).as_uuid())
            .bind(subject)
            .bind(ADMIN)
            .execute(&self.api.db.pool)
            .await
            .expect("insert canary grant");
        }
    }

    async fn call_text(
        &self,
        subject: &str,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> (StatusCode, String) {
        let mut request = self.api.as_user(method, path, subject, TENANT);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.expect("canary HTTP request");
        let status = response.status();
        let text = response.text().await.expect("canary HTTP body");
        (status, text)
    }
}

fn namespace_id(value: &str) -> NamespaceId {
    value.parse().expect("namespace id")
}

fn assert_json_error(surface: &str, text: &str, canaries: &Canaries) {
    let body: Value = serde_json::from_str(text).expect("closed JSON error");
    assert_error(surface, &body, canaries);
}

async fn disclose(
    fixture: &Fixture,
    run: &str,
    subject: &str,
    reveal: bool,
) -> (StatusCode, String) {
    let body = if reveal {
        json!({"source":"input", "reveal":["pii"]})
    } else {
        json!({"source":"input"})
    };
    fixture
        .call_text(
            subject,
            Method::POST,
            &format!("/runs/{run}/payload-disclosures"),
            Some(&body),
        )
        .await
}

#[tokio::test]
async fn forbidden_payloads_stay_out_of_public_surfaces_errors_audit_and_telemetry() {
    let collector = Collector::start().await;
    let (_telemetry_guard, sink, logs) = collector.install_with_logs(1.0);
    let metrics = Metrics::global();
    let fixture = Fixture::new("boundary").await;
    let canaries = &fixture.canaries;
    let run = fixture
        .run(
            "classified-canary",
            canaries.classified_schema(),
            canaries.classified_input(),
        )
        .await;

    fixture.set_grant(ADMIN, true).await;
    let (status, revealed) = disclose(&fixture, &run, ADMIN, true).await;
    assert_eq!(status, StatusCode::OK);
    assert_present("classified reveal", &revealed, &canaries.public_value);
    assert_present("classified reveal", &revealed, &canaries.internal_value);
    assert_present("classified reveal", &revealed, &canaries.pii_value);
    for key in [
        &canaries.public_key,
        &canaries.internal_key,
        &canaries.pii_key,
    ] {
        assert_present("classified reveal keys", &revealed, key);
    }
    assert_absent("classified reveal", &revealed, &canaries.always_forbidden());

    let invalid = fixture
        .run(
            "invalid-canary",
            json!({
                "type":"object", "additionalProperties":false,
                "x-sagat-classification":"public",
                "properties":{"declared":{"type":"number"}}
            }),
            json!({"declared":canaries.invalid_value.value(), (canaries.invalid_key.value()):true}),
        )
        .await;
    for subject in ["developer-canary", "operator-canary"] {
        let (status, body) = disclose(&fixture, &invalid, subject, false).await;
        assert_eq!(status, StatusCode::OK);
        assert_absent(
            "invalid disclosure",
            body,
            &[&canaries.invalid_value, &canaries.invalid_key],
        );
    }

    let unmarked = fixture
        .run(
            "unmarked-canary",
            json!({"type":"object","properties":{"value":{"type":"string"}}}),
            json!({"value":canaries.unmarked_value.value(), (canaries.unmarked_key.value()):true}),
        )
        .await;
    let (status, hidden) = disclose(&fixture, &unmarked, "developer-canary", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_absent(
        "unmarked default",
        hidden,
        &[&canaries.unmarked_value, &canaries.unmarked_key],
    );
    fixture.set_grant("developer-canary", true).await;
    let (status, unmarked_reveal) = disclose(&fixture, &unmarked, "developer-canary", true).await;
    assert_eq!(status, StatusCode::OK);
    assert_present(
        "unmarked explicit reveal",
        unmarked_reveal,
        &canaries.unmarked_value,
    );

    let legacy = fixture
        .run(
            "legacy-canary",
            json!({
                "type":"object", "x-sagat-classification":"public",
                "properties":{"value":{"type":"string"}}
            }),
            json!({"value":canaries.legacy_value.value(), (canaries.legacy_key.value()):true}),
        )
        .await;
    sqlx::query("ALTER TABLE workflow_versions DISABLE TRIGGER workflow_versions_immutable")
        .execute(&fixture.api.db.pool)
        .await
        .expect("open the isolated legacy fixture");
    sqlx::query(
        "UPDATE workflow_versions SET classification_rules=0 \
         WHERE id=(SELECT workflow_version_id FROM runs WHERE tenant_id=$1 AND id=$2)",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(Uuid::parse_str(&legacy).expect("legacy run id"))
    .execute(&fixture.api.db.pool)
    .await
    .expect("mark fixture version legacy");
    sqlx::query("ALTER TABLE workflow_versions ENABLE TRIGGER workflow_versions_immutable")
        .execute(&fixture.api.db.pool)
        .await
        .expect("close the isolated legacy fixture");
    let (status, legacy_answer) = disclose(&fixture, &legacy, ADMIN, true).await;
    assert_eq!(status, StatusCode::OK);
    assert_absent(
        "legacy disclosure",
        legacy_answer,
        &[&canaries.legacy_value, &canaries.legacy_key],
    );

    for (surface, method, path, body) in [
        ("run", Method::GET, format!("/runs/{run}"), None),
        (
            "journal",
            Method::GET,
            format!("/runs/{run}/events?limit=200"),
            None,
        ),
        ("graph", Method::GET, format!("/runs/{run}/graph"), None),
        (
            "graph changes",
            Method::GET,
            format!("/runs/{run}/graph/changes?since=0"),
            None,
        ),
        (
            "telemetry",
            Method::GET,
            format!("/runs/{run}/telemetry"),
            None,
        ),
        (
            "attempts",
            Method::GET,
            format!("/runs/{run}/attempts"),
            None,
        ),
        (
            "inspection",
            Method::POST,
            format!("/runs/{run}/payload-inspections"),
            Some(json!({"source":"input"})),
        ),
    ] {
        let (status, answer) = fixture.call_text(ADMIN, method, &path, body.as_ref()).await;
        assert_eq!(status, StatusCode::OK, "surface {surface}");
        assert_absent(surface, answer, &canaries.all());
    }

    let stream = fixture
        .api
        .as_user(
            Method::GET,
            &format!("/runs/{run}/stream?since=0"),
            ADMIN,
            TENANT,
        )
        .send()
        .await
        .expect("open run stream");
    assert_eq!(stream.status(), StatusCode::OK);
    let first = tokio::time::timeout(Duration::from_secs(5), stream.bytes_stream().next())
        .await
        .expect("stream event in time")
        .expect("stream item")
        .expect("stream bytes");
    assert_absent("stream", first, &canaries.all());

    let invalid_selector = json!({"source":canaries.secret_value.value()});
    let (status, error) = fixture
        .call_text(
            ADMIN,
            Method::POST,
            &format!("/runs/{run}/payload-disclosures"),
            Some(&invalid_selector),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_json_error("invalid selector", &error, canaries);

    let (status, audit) = fixture
        .call_text(ADMIN, Method::GET, "/audit?limit=200", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_absent("audit API", audit, &canaries.all());

    sink.shutdown(Duration::from_secs(5)).await;
    let spans = format!("{:#?}", collector.spans());
    assert!(
        collector
            .spans()
            .iter()
            .any(|span| span.name == "payload_disclosure"),
        "the canary test must capture the disclosure span"
    );
    assert_absent("OTLP spans", spans, &canaries.all());
    assert_absent("structured logs", logs.text(), &canaries.all());
    let metric_page = metrics.render();
    assert!(metric_page.contains("payload_disclosures_total"));
    assert_absent("metric names and labels", metric_page, &canaries.all());
}

struct EventResources {
    source: String,
    event_id: String,
    connector: ConnectorId,
    letter: Uuid,
}

async fn seed_event_resources(fixture: &Fixture) -> EventResources {
    let source = "canary-source".to_owned();
    let event_id = format!("event-{}", Uuid::now_v7());
    let event = json!({
        fixture.canaries.event_key.value(): fixture.canaries.event_value.value()
    });
    sqlx::query(
        "INSERT INTO inbox_events \
         (tenant_id,namespace_id,source,event_id,event_type,payload,payload_digest,received_at,expires_at,next_attempt_at) \
         VALUES ($1,$2,$3,$4,'canary.created',$5,decode(repeat('7a',32),'hex'),clock_timestamp(),clock_timestamp()+interval '1 hour',clock_timestamp())",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(namespace_id(&fixture.namespace).as_uuid())
    .bind(&source)
    .bind(&event_id)
    .bind(serde_json::to_vec(&event).expect("event JSON"))
    .execute(&fixture.api.db.pool)
    .await
    .expect("seed inbox event");

    let connector = ConnectorId::new(Uuid::now_v7());
    sqlx::query(
        "INSERT INTO event_connectors \
         (id,tenant_id,namespace_id,slug,kind,settings,rate_per_second,created_by) \
         VALUES ($1,$2,$3,'canary-connector','KAFKA','{\"servers\":[\"tls://broker:9092\"],\"topic\":\"a\",\"startFrom\":\"LATEST\"}',10,$4)",
    )
    .bind(connector.as_uuid())
    .bind(fixture.tenant.as_uuid())
    .bind(namespace_id(&fixture.namespace).as_uuid())
    .bind(ADMIN)
    .execute(&fixture.api.db.pool)
    .await
    .expect("seed connector");
    let letter = Uuid::now_v7();
    let headers = json!({
        fixture.canaries.dead_letter_header_key.value(): fixture.canaries.dead_letter_header_value.value()
    });
    let body = json!({
        fixture.canaries.dead_letter_body_key.value(): fixture.canaries.dead_letter_body_value.value()
    });
    sqlx::query(
        "INSERT INTO connector_dead_letters \
         (id,tenant_id,namespace_id,connector_id,position,reason,headers,body) \
         VALUES ($1,$2,$3,$4,'canary/0/1','BAD_ENVELOPE',$5,$6)",
    )
    .bind(letter)
    .bind(fixture.tenant.as_uuid())
    .bind(namespace_id(&fixture.namespace).as_uuid())
    .bind(connector.as_uuid())
    .bind(headers)
    .bind(serde_json::to_vec(&body).expect("letter JSON"))
    .execute(&fixture.api.db.pool)
    .await
    .expect("seed dead letter");
    EventResources {
        source,
        event_id,
        connector,
        letter,
    }
}

#[tokio::test]
async fn event_and_dead_letter_values_follow_all_role_grant_and_reveal_cases() {
    let collector = Collector::start().await;
    let (_telemetry_guard, sink, logs) = collector.install_with_logs(1.0);
    let metrics = Metrics::global();
    let fixture = Fixture::new("events").await;
    let resources = seed_event_resources(&fixture).await;
    let mut cells = 0;
    for case in audience_cases() {
        fixture.set_grant(case.subject, case.grant).await;
        let query = if case.reveal { "?reveal=pii" } else { "" };
        let event_path = format!(
            "/namespaces/{}/events/{}/{}{}",
            fixture.namespace, resources.source, resources.event_id, query
        );
        let detail_path = format!(
            "/namespaces/{}/connectors/{}/dead-letters/{}{}",
            fixture.namespace, resources.connector, resources.letter, query
        );
        let list_path = format!(
            "/namespaces/{}/connectors/{}/dead-letters{}",
            fixture.namespace, resources.connector, query
        );
        let (event_status, event) = fixture
            .call_text(case.subject, Method::GET, &event_path, None)
            .await;
        let (detail_status, detail) = fixture
            .call_text(case.subject, Method::GET, &detail_path, None)
            .await;
        let (list_status, list) = fixture
            .call_text(case.subject, Method::GET, &list_path, None)
            .await;
        cells += 3;

        if case.may_reveal() && case.may_read_events() {
            assert_eq!(event_status, StatusCode::OK);
            assert_eq!(detail_status, StatusCode::OK);
            assert_present("event reveal", &event, &fixture.canaries.event_key);
            assert_present("event reveal", &event, &fixture.canaries.event_value);
            for expected in fixture.canaries.dead_letter_forbidden() {
                assert_present("dead-letter reveal", &detail, expected);
            }
        } else {
            assert_absent(
                "event non-reveal",
                &event,
                &fixture.canaries.event_forbidden(),
            );
            assert_absent(
                "dead-letter non-reveal",
                &detail,
                &fixture.canaries.dead_letter_forbidden(),
            );
            if case.reveal && case.may_read_events() {
                assert_eq!(event_status, StatusCode::FORBIDDEN);
                assert_eq!(detail_status, StatusCode::FORBIDDEN);
                assert_json_error("event reveal denial", &event, &fixture.canaries);
                assert_json_error("dead-letter reveal denial", &detail, &fixture.canaries);
            } else if case.may_read_events() {
                assert_eq!(event_status, StatusCode::OK);
                assert_eq!(detail_status, StatusCode::OK);
            } else {
                assert_eq!(event_status, StatusCode::FORBIDDEN);
                assert_eq!(detail_status, StatusCode::FORBIDDEN);
            }
        }

        assert_absent(
            "dead-letter list",
            &list,
            &fixture.canaries.dead_letter_forbidden(),
        );
        assert_eq!(
            list_status,
            if case.may_read_events() {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            }
        );
    }
    assert_eq!(cells, 6 * 2 * 2 * 3);

    let audits: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT action,resource_id,reason FROM audit_records \
         WHERE tenant_id=$1 AND action IN ('payload.pii_revealed','payload.reveal_denied')",
    )
    .bind(fixture.tenant.as_uuid())
    .fetch_all(&fixture.api.db.pool)
    .await
    .expect("read event canary audits");
    let audit_bytes = serde_json::to_vec(&audits).expect("audit JSON");
    assert_absent("event audit rows", audit_bytes, &fixture.canaries.all());
    let reveals = audits
        .iter()
        .filter(|(action, _, _)| action == "payload.pii_revealed")
        .count();
    assert_eq!(
        reveals, 6,
        "three roles reveal one event and one letter each"
    );
    sink.shutdown(Duration::from_secs(5)).await;
    assert!(
        !collector.spans().is_empty(),
        "actual event HTTP spans must be captured"
    );
    assert_absent(
        "event OTLP spans",
        format!("{:#?}", collector.spans()),
        &fixture.canaries.all(),
    );
    assert_absent(
        "event structured logs",
        logs.text(),
        &fixture.canaries.all(),
    );
    let metric_page = metrics.render();
    assert!(metric_page.contains("event_value_reads_total"));
    assert_absent("event metric labels", metric_page, &fixture.canaries.all());
}
