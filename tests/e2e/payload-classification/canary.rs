//! Public API journey over all built-in audiences (T016-049).

use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Method, StatusCode};
use sagat_domain::identity::{Membership, NamespaceId, Role, TenantId};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::canary_support::{
    Canaries, assert_absent, assert_error, assert_present, audience_cases,
};
use crate::support;
use crate::support::api::{TestApi, json_of};
use crate::support::engine::{document, end_node};

pub(crate) const TENANT: &str = "classification-e2e";
pub(crate) const NAMESPACE: &str = "payments";
pub(crate) const ADMIN: &str = "admin";
pub(crate) const DEVELOPER: &str = "developer-canary";

pub(crate) struct Fixture {
    pub api: TestApi,
    pub tenant: TenantId,
    pub namespace: String,
    pub run: String,
    pub canaries: Canaries,
}

pub(crate) async fn fixture(label: &str) -> Fixture {
    let api = support::api::start().await;
    let tenant = api.tenant(TENANT, ADMIN).await;
    let workspace = api.create_workspace(ADMIN, TENANT, "canary-e2e").await;
    let namespace = api
        .create_namespace(ADMIN, TENANT, &workspace, NAMESPACE, "dev")
        .await;
    for (subject, role) in [
        ("viewer-canary", Role::Viewer),
        (DEVELOPER, Role::Developer),
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
            .expect("insert e2e canary membership");
    }
    let canaries = Canaries::unique(label);
    let name = format!("classification-e2e-{}", Uuid::now_v7());
    let mut ir = document(&name, "done", vec![end_node("done")]);
    ir["inputSchema"] = json!({
        "type":"object", "additionalProperties":true,
        "x-sagat-classification":"public",
        "properties":{
            "publicField":{"type":"string"},
            "internalField":{"type":"string","x-sagat-classification":"internal"},
            "piiField":{"type":"string","x-sagat-classification":"pii"},
            "secretField":{"type":"string","x-sagat-classification":"secret"}
        }
    });
    let mut input = Map::new();
    input.insert(
        "publicField".to_owned(),
        json!(canaries.public_value.value()),
    );
    input.insert(
        "internalField".to_owned(),
        json!(canaries.internal_value.value()),
    );
    input.insert("piiField".to_owned(), json!(canaries.pii_value.value()));
    input.insert(
        "secretField".to_owned(),
        json!(canaries.secret_value.value()),
    );
    input.insert(
        canaries.undeclared_key.value().to_owned(),
        json!(canaries.undeclared_value.value()),
    );
    let created = api
        .as_user(Method::POST, "/workflows", ADMIN, TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name":name,"ir":ir}))
        .send()
        .await
        .expect("create e2e canary workflow");
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_of(created).await;
    let workflow = created["id"].as_str().expect("workflow id");
    let revision = created["draftRevision"].as_u64().expect("draft revision");
    let published = api
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
        .expect("publish e2e canary workflow");
    assert_eq!(published.status(), StatusCode::CREATED);
    let started = api
        .as_user(Method::POST, "/runs", ADMIN, TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId":workflow,"input":Value::Object(input)}))
        .send()
        .await
        .expect("start e2e canary run");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let run = json_of(started).await["runId"]
        .as_str()
        .expect("run id")
        .to_owned();
    Fixture {
        api,
        tenant,
        namespace,
        run,
        canaries,
    }
}

impl Fixture {
    pub(crate) async fn set_grant(&self, subject: &str, granted: bool) {
        let namespace = self.namespace.parse::<NamespaceId>().expect("namespace id");
        sqlx::query(
            "DELETE FROM payload_reveal_grants \
             WHERE tenant_id=$1 AND namespace_id=$2 AND subject=$3",
        )
        .bind(self.tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject)
        .execute(&self.api.db.pool)
        .await
        .expect("reset e2e canary grant");
        if granted {
            sqlx::query(
                "INSERT INTO payload_reveal_grants \
                 (tenant_id,namespace_id,subject,granted_by,granted_at) \
                 VALUES ($1,$2,$3,$4,clock_timestamp())",
            )
            .bind(self.tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(subject)
            .bind(ADMIN)
            .execute(&self.api.db.pool)
            .await
            .expect("insert e2e canary grant");
        }
    }

    pub(crate) async fn text(
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
        let response = request.send().await.expect("e2e canary request");
        let status = response.status();
        let text = response.text().await.expect("e2e canary body");
        (status, text)
    }
}

#[tokio::test]
async fn six_roles_with_and_without_grant_keep_every_public_surface_value_free() {
    let fixture = fixture("public-surfaces").await;
    let run = &fixture.run;
    let payload_free = [
        format!("/runs/{run}"),
        format!("/runs/{run}/events?limit=200"),
        format!("/runs/{run}/graph"),
        format!("/runs/{run}/graph/changes?since=0"),
        format!("/runs/{run}/telemetry"),
        format!("/runs/{run}/attempts"),
    ];
    let mut disclosure_cells = 0;
    for case in audience_cases() {
        fixture.set_grant(case.subject, case.grant).await;
        let selector = if case.reveal {
            json!({"source":"input","reveal":["pii"]})
        } else {
            json!({"source":"input"})
        };
        let (status, disclosure) = fixture
            .text(
                case.subject,
                Method::POST,
                &format!("/runs/{run}/payload-disclosures"),
                Some(&selector),
            )
            .await;
        disclosure_cells += 4;
        assert_absent(
            "e2e disclosure secret",
            &disclosure,
            &[&fixture.canaries.secret_value],
        );
        if !case.may_read_runs() || (case.reveal && !case.may_reveal()) {
            assert_eq!(status, StatusCode::FORBIDDEN);
            let body: Value = serde_json::from_str(&disclosure).expect("closed refusal");
            assert_error("e2e disclosure refusal", &body, &fixture.canaries);
        } else {
            assert_eq!(status, StatusCode::OK);
            assert_present(
                "e2e public disclosure",
                &disclosure,
                &fixture.canaries.public_value,
            );
            if case.may_read_internal() {
                assert_present(
                    "e2e internal disclosure",
                    &disclosure,
                    &fixture.canaries.internal_value,
                );
            } else {
                assert_absent(
                    "e2e internal omission",
                    &disclosure,
                    &[&fixture.canaries.internal_value],
                );
            }
            for pii in [
                &fixture.canaries.pii_value,
                &fixture.canaries.undeclared_key,
                &fixture.canaries.undeclared_value,
            ] {
                if case.may_reveal() {
                    assert_present("e2e PII reveal", &disclosure, pii);
                } else {
                    assert_absent("e2e PII omission", &disclosure, &[pii]);
                }
            }
        }

        for path in &payload_free {
            let (surface_status, surface) =
                fixture.text(case.subject, Method::GET, path, None).await;
            assert_eq!(
                surface_status,
                if case.may_read_runs() {
                    StatusCode::OK
                } else {
                    StatusCode::FORBIDDEN
                }
            );
            assert_absent(
                "payload-free REST surface",
                surface,
                &fixture.canaries.all(),
            );
        }

        let inspection = json!({"source":"input"});
        let (inspection_status, inspection_body) = fixture
            .text(
                case.subject,
                Method::POST,
                &format!("/runs/{run}/payload-inspections"),
                Some(&inspection),
            )
            .await;
        assert_eq!(
            inspection_status,
            if matches!(case.role, Role::Admin | Role::Operator) {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            }
        );
        assert_absent(
            "structure-only inspection",
            inspection_body,
            &fixture.canaries.all(),
        );
    }
    assert_eq!(disclosure_cells, 6 * 4 * 2 * 2);

    for subject in [
        "viewer-canary",
        DEVELOPER,
        "operator-canary",
        ADMIN,
        "worker-canary",
        "runner-canary",
    ] {
        let response = fixture
            .api
            .as_user(
                Method::GET,
                &format!("/runs/{run}/stream?since=0"),
                subject,
                TENANT,
            )
            .send()
            .await
            .expect("open audience stream");
        if matches!(subject, "worker-canary" | "runner-canary") {
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let body = response.text().await.expect("stream refusal");
            assert_absent("stream refusal", body, &fixture.canaries.all());
        } else {
            assert_eq!(response.status(), StatusCode::OK);
            let first =
                tokio::time::timeout(Duration::from_secs(5), response.bytes_stream().next())
                    .await
                    .expect("stream event in time")
                    .expect("stream item")
                    .expect("stream bytes");
            assert_absent("run stream", first, &fixture.canaries.all());
        }
    }

    let (audit_status, audit) = fixture
        .text(ADMIN, Method::GET, "/audit?limit=200", None)
        .await;
    assert_eq!(audit_status, StatusCode::OK);
    assert_absent("audit read", audit, &fixture.canaries.all());
}
