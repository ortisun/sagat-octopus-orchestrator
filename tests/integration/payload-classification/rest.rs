//! Live REST conformance for classified run payload disclosure (T016-024).
#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use reqwest::{Method, Response, StatusCode};
use sagat_domain::identity::{Membership, NamespaceId, Role, TenantId};
use serde_json::{Value, json};
use uuid::Uuid;

use super::support;
use super::support::api::{TestApi, json_of};
use super::support::engine::{document, end_node};

const PUBLIC_NUMBER: u64 = 9_007_199_254_740_993;
const INTERNAL_CANARY: &str = "internal-wire-canary";
const PII_CANARY: &str = "pii-wire-canary";
const SECRET_CANARY: &str = "secret-wire-canary";
const REFLECTION_CANARY: &str = "ERROR-REFLECTION-CANARY";
const MICROTOKENS: i64 = 1_000_000;

struct Fixture {
    api: TestApi,
    tenant: TenantId,
    namespace: String,
    run: String,
}

impl Fixture {
    async fn new() -> Self {
        let api = support::api::start().await;
        let tenant = api.tenant("classification-http", "admin").await;
        let workspace = api
            .create_workspace("admin", "classification-http", "classified")
            .await;
        let namespace = api
            .create_namespace(
                "admin",
                "classification-http",
                &workspace,
                "payments",
                "dev",
            )
            .await;
        for (subject, role) in [
            ("viewer", Role::Viewer),
            ("developer", Role::Developer),
            ("operator", Role::Operator),
            ("worker", Role::Worker),
            ("runner", Role::Runner),
        ] {
            api.owner_store
                .insert_membership(&Membership::restore(
                    tenant,
                    support::subject(subject),
                    role,
                    support::now(),
                ))
                .await
                .expect("insert audience membership");
        }

        let workflow_name = format!("classified-http-{}", Uuid::now_v7());
        let mut ir = document(&workflow_name, "done", vec![end_node("done")]);
        ir["inputSchema"] = json!({
            "type": "object",
            "additionalProperties": false,
            "x-sagat-classification": "public",
            "properties": {
                "publicNumber": {"type": "number"},
                "internalValue": {"type": "string", "x-sagat-classification": "internal"},
                "piiValue": {"type": "string", "x-sagat-classification": "pii"},
                "secretValue": {"type": "string", "x-sagat-classification": "secret"}
            }
        });
        let created = api
            .as_user(Method::POST, "/workflows", "admin", "classification-http")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"name": workflow_name, "ir": ir}))
            .send()
            .await
            .expect("create workflow");
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = json_of(created).await;
        let workflow = created["id"].as_str().expect("workflow id");
        let revision = created["draftRevision"].as_u64().expect("draft revision");
        let published = api
            .as_user(
                Method::POST,
                &format!("/workflows/{workflow}/publish"),
                "admin",
                "classification-http",
            )
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"draftRevision": revision}))
            .send()
            .await
            .expect("publish workflow");
        assert_eq!(published.status(), StatusCode::CREATED);
        let started = api
            .as_user(Method::POST, "/runs", "admin", "classification-http")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({
                "workflowId": workflow,
                "input": {
                    "publicNumber": PUBLIC_NUMBER,
                    "internalValue": INTERNAL_CANARY,
                    "piiValue": PII_CANARY,
                    "secretValue": SECRET_CANARY
                }
            }))
            .send()
            .await
            .expect("start run");
        assert_eq!(started.status(), StatusCode::ACCEPTED);
        let run = json_of(started).await["runId"]
            .as_str()
            .expect("run id")
            .to_owned();
        Self {
            api,
            tenant,
            namespace,
            run,
        }
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
        .expect("reset reveal grant");
        if granted {
            sqlx::query(
                "INSERT INTO payload_reveal_grants \
                 (tenant_id,namespace_id,subject,granted_by,granted_at) \
                 VALUES ($1,$2,$3,'admin',clock_timestamp())",
            )
            .bind(self.tenant.as_uuid())
            .bind(namespace_id(&self.namespace).as_uuid())
            .bind(subject)
            .execute(&self.api.db.pool)
            .await
            .expect("grant reveal");
        }
    }

    async fn disclose(&self, subject: &str, reveal: bool) -> Response {
        let body = if reveal {
            json!({"source":"input", "reveal":["pii"]})
        } else {
            json!({"source":"input"})
        };
        self.api
            .post_as(
                &format!("/runs/{}/payload-disclosures", self.run),
                subject,
                "classification-http",
                &body,
            )
            .await
    }
}

#[derive(Clone, Copy)]
enum NodeExpectation {
    Value,
    Omitted,
    Withheld(&'static str),
}

fn openapi_validator(schema: &str) -> jsonschema::Validator {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../specs/001-platform-foundation/contracts/openapi.yaml");
    let openapi: Value =
        serde_saphyr::from_str(&fs::read_to_string(path).expect("canonical OpenAPI exists"))
            .expect("canonical OpenAPI is YAML");
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": format!("#/components/schemas/{schema}"),
        "components": openapi["components"].clone()
    });
    jsonschema::options()
        .should_validate_formats(true)
        .build(&root)
        .unwrap_or_else(|error| panic!("{schema} compiles: {error}"))
}

fn assert_openapi(schema: &str, body: &Value) {
    let errors = openapi_validator(schema)
        .iter_errors(body)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{schema}: {errors:?}\n{body}");
}

fn child<'a>(body: &'a Value, name: &str) -> Option<&'a Value> {
    body["tree"]["children"]
        .as_array()
        .expect("root children")
        .iter()
        .find(|child| child["name"] == name)
        .map(|child| &child["node"])
}

fn assert_node(body: &Value, name: &str, expected: NodeExpectation) {
    let actual = child(body, name);
    match expected {
        NodeExpectation::Value => assert_eq!(actual.expect("disclosed node")["kind"], "value"),
        NodeExpectation::Omitted => assert!(actual.is_none(), "{name} must be omitted: {actual:?}"),
        NodeExpectation::Withheld(reason) => {
            let actual = actual.expect("withheld node");
            assert_eq!(actual["kind"], "withheld");
            assert_eq!(actual["reason"], reason);
        }
    }
}

fn expected(role: Role, grant: bool, reveal: bool) -> Option<[NodeExpectation; 4]> {
    if matches!(role, Role::Worker | Role::Runner) || (reveal && !effective_grant(role, grant)) {
        return None;
    }
    let internal = if matches!(role, Role::Developer | Role::Operator | Role::Admin) {
        NodeExpectation::Value
    } else {
        NodeExpectation::Omitted
    };
    let pii = if !matches!(role, Role::Developer | Role::Operator | Role::Admin) {
        NodeExpectation::Omitted
    } else if reveal {
        NodeExpectation::Value
    } else if matches!(role, Role::Operator | Role::Admin) {
        NodeExpectation::Withheld(if effective_grant(role, grant) {
            "REVEAL_NOT_REQUESTED"
        } else {
            "LEVEL_NOT_PERMITTED"
        })
    } else {
        NodeExpectation::Omitted
    };
    let secret = if matches!(role, Role::Operator | Role::Admin) {
        NodeExpectation::Withheld("SECRET")
    } else {
        NodeExpectation::Omitted
    };
    Some([NodeExpectation::Value, internal, pii, secret])
}

fn effective_grant(role: Role, grant: bool) -> bool {
    grant && matches!(role, Role::Developer | Role::Operator | Role::Admin)
}

#[tokio::test]
async fn audience_matrix_covers_six_roles_four_levels_reveal_and_namespace_grant() {
    let fixture = Fixture::new().await;
    let validator = openapi_validator("PayloadDisclosure");
    let roles = [
        ("viewer", Role::Viewer),
        ("developer", Role::Developer),
        ("operator", Role::Operator),
        ("admin", Role::Admin),
        ("worker", Role::Worker),
        ("runner", Role::Runner),
    ];
    let mut cells = 0;
    for (subject, role) in roles {
        for grant in [false, true] {
            fixture.set_grant(subject, grant).await;
            for reveal in [false, true] {
                let response = fixture.disclose(subject, reveal).await;
                cells += 4;
                let status = response.status();
                let cache_control = response
                    .headers()
                    .get("cache-control")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                let wire = response.text().await.expect("response body");
                let body: Value = serde_json::from_str(&wire).expect("JSON response");
                if let Some(nodes) = expected(role, grant, reveal) {
                    assert_eq!(
                        status,
                        StatusCode::OK,
                        "{subject} grant={grant} reveal={reveal}: {body}"
                    );
                    assert_eq!(cache_control.as_deref(), Some("private, no-store"));
                    let errors = validator
                        .iter_errors(&body)
                        .map(|error| format!("{}: {error}", error.instance_path()))
                        .collect::<Vec<_>>();
                    assert!(errors.is_empty(), "PayloadDisclosure: {errors:?}\n{body}");
                    assert_eq!(body["policy"], "classified-v1");
                    assert_eq!(
                        body["caller"]["revealPii"],
                        if effective_grant(role, grant) {
                            "granted"
                        } else {
                            "not_granted"
                        }
                    );
                    assert_eq!(body["caller"]["revealed"], reveal);
                    for ((name, expectation), level) in [
                        ("publicNumber", nodes[0]),
                        ("internalValue", nodes[1]),
                        ("piiValue", nodes[2]),
                        ("secretValue", nodes[3]),
                    ]
                    .into_iter()
                    .zip(["public", "internal", "pii", "secret"])
                    {
                        assert_node(&body, name, expectation);
                        if matches!(expectation, NodeExpectation::Value) {
                            assert_eq!(child(&body, name).expect("value")["level"], level);
                        }
                    }
                    assert!(
                        wire.contains(&format!("\"value\":{PUBLIC_NUMBER}")),
                        "numeric lexeme changed: {wire}"
                    );
                    assert_eq!(
                        wire.contains(INTERNAL_CANARY),
                        matches!(nodes[1], NodeExpectation::Value)
                    );
                    assert_eq!(
                        wire.contains(PII_CANARY),
                        matches!(nodes[2], NodeExpectation::Value)
                    );
                    assert!(!wire.contains(SECRET_CANARY));
                } else {
                    assert_eq!(
                        status,
                        StatusCode::FORBIDDEN,
                        "{subject} grant={grant} reveal={reveal}: {body}"
                    );
                    assert_openapi("Error", &body);
                    assert!(!wire.contains(INTERNAL_CANARY));
                    assert!(!wire.contains(PII_CANARY));
                    assert!(!wire.contains(SECRET_CANARY));
                    if reveal && !matches!(role, Role::Worker | Role::Runner) {
                        assert_eq!(body["details"]["reason"], "PII_REVEAL_NOT_GRANTED");
                    }
                }
            }
        }
    }
    assert_eq!(cells, 6 * 4 * 2 * 2, "one assertion row per audience cell");
}

async fn raw_disclosure(
    fixture: &Fixture,
    subject: &str,
    tenant: &str,
    run: &str,
    body: String,
) -> Response {
    fixture
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{run}/payload-disclosures"),
            subject,
            tenant,
        )
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("disclosure request")
}

async fn assert_closed_error(response: Response, status: StatusCode, code: &str) -> Value {
    assert_eq!(response.status(), status);
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("private, no-store")
    );
    let wire = response.text().await.expect("error body");
    assert!(
        !wire.contains(REFLECTION_CANARY),
        "error reflected untrusted input: {wire}"
    );
    let body: Value = serde_json::from_str(&wire).expect("JSON error");
    assert_eq!(body["code"], code);
    assert_openapi("Error", &body);
    body
}

#[tokio::test]
async fn selectors_envelopes_and_opaque_admission_fail_closed() {
    let fixture = Fixture::new().await;
    for body in [
        json!({"source": REFLECTION_CANARY}).to_string(),
        json!({"source":"journal"}).to_string(),
        json!({"source":"journal", "sequence":0}).to_string(),
        json!({"source":"journal", "sequence":-1}).to_string(),
        json!({"source":"journal", "sequence":1.5}).to_string(),
        json!({"source":"journal", "sequence":9_223_372_036_854_775_808u64}).to_string(),
        json!({"source":"input", "sequence":1}).to_string(),
        json!({"source":"input", "reveal":[REFLECTION_CANARY]}).to_string(),
        json!({"source":"input", "reveal":["pii", "pii"]}).to_string(),
    ] {
        assert_closed_error(
            raw_disclosure(&fixture, "admin", "classification-http", &fixture.run, body).await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_FAILED",
        )
        .await;
    }
    assert_closed_error(
        raw_disclosure(
            &fixture,
            "admin",
            "classification-http",
            &fixture.run,
            json!({"source":"input", REFLECTION_CANARY:REFLECTION_CANARY}).to_string(),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
    )
    .await;
    assert_closed_error(
        raw_disclosure(
            &fixture,
            "admin",
            "classification-http",
            &fixture.run,
            format!("{{\"source\":\"input\",\"bad\":\"{REFLECTION_CANARY}"),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
    )
    .await;
    let too_large = assert_closed_error(
        raw_disclosure(
            &fixture,
            "admin",
            "classification-http",
            &fixture.run,
            json!({"source":"input", "padding":format!("{REFLECTION_CANARY}{}", "x".repeat(1200))})
                .to_string(),
        )
        .await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "BAD_REQUEST",
    )
    .await;
    assert_eq!(too_large["details"]["reason"], "PAYLOAD_TOO_LARGE");

    let foreign_tenant = fixture.api.tenant("foreign-http", "bob").await;
    let foreign_workspace = fixture
        .api
        .create_workspace("bob", "foreign-http", "foreign")
        .await;
    let foreign_namespace = fixture
        .api
        .create_namespace(
            "bob",
            "foreign-http",
            &foreign_workspace,
            "foreign-payments",
            "dev",
        )
        .await;
    set_rate_and_seed(
        &fixture.api,
        fixture.tenant,
        &fixture.namespace,
        "admin",
        "classification-http",
    )
    .await;
    set_rate_and_seed(
        &fixture.api,
        foreign_tenant,
        &foreign_namespace,
        "bob",
        "foreign-http",
    )
    .await;
    let owner_before = bucket_tokens(&fixture.api, fixture.tenant, &fixture.namespace).await;
    let foreign_before = bucket_tokens(&fixture.api, foreign_tenant, &foreign_namespace).await;

    assert_closed_error(
        raw_disclosure(
            &fixture,
            "bob",
            "foreign-http",
            &fixture.run,
            json!({"source":"input", "reveal":["pii"]}).to_string(),
        )
        .await,
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
    )
    .await;
    assert_eq!(
        bucket_tokens(&fixture.api, fixture.tenant, &fixture.namespace).await,
        owner_before
    );
    assert_eq!(
        bucket_tokens(&fixture.api, foreign_tenant, &foreign_namespace).await,
        foreign_before
    );

    let missing = Uuid::now_v7().to_string();
    assert_closed_error(
        raw_disclosure(
            &fixture,
            "admin",
            "classification-http",
            &missing,
            json!({"source":"input"}).to_string(),
        )
        .await,
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
    )
    .await;
    assert_eq!(
        bucket_tokens(&fixture.api, fixture.tenant, &fixture.namespace).await,
        owner_before
    );

    assert_closed_error(
        raw_disclosure(
            &fixture,
            "admin",
            "classification-http",
            &fixture.run,
            json!({"source":"journal", "sequence":i64::MAX}).to_string(),
        )
        .await,
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
    )
    .await;
    let after_missing_sequence = owner_before.map(|tokens| tokens - MICROTOKENS);
    assert_eq!(
        bucket_tokens(&fixture.api, fixture.tenant, &fixture.namespace).await,
        after_missing_sequence,
        "a resolved run is admitted exactly once before its absent journal record is selected"
    );

    let success = fixture.disclose("admin", false).await;
    assert_eq!(success.status(), StatusCode::OK);
    assert_eq!(
        bucket_tokens(&fixture.api, fixture.tenant, &fixture.namespace).await,
        after_missing_sequence.map(|tokens| tokens - MICROTOKENS)
    );
}

fn namespace_id(value: &str) -> NamespaceId {
    value.parse().expect("namespace id")
}

async fn set_rate_and_seed(
    api: &TestApi,
    tenant: TenantId,
    namespace: &str,
    admin: &str,
    tenant_slug: &str,
) {
    let set = api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{namespace}/quotas/api_requests_per_second"),
            admin,
            tenant_slug,
        )
        .json(&json!({"limit": 10}))
        .send()
        .await
        .expect("set request rate");
    assert_eq!(set.status(), StatusCode::OK);
    sqlx::query(
        "DELETE FROM quota_rate_buckets WHERE tenant_id=$1 AND namespace_id=$2 \
         AND resource='api_requests_per_second'",
    )
    .bind(tenant.as_uuid())
    .bind(namespace_id(namespace).as_uuid())
    .execute(&api.db.pool)
    .await
    .expect("reset admission bucket");
    sqlx::query(
        "INSERT INTO quota_rate_buckets \
         (tenant_id,namespace_id,resource,source_kind,source_id,microtokens,updated_at) \
         VALUES ($1,$2,'api_requests_per_second','NAMESPACE',$3,$4,clock_timestamp()+interval '1 hour')",
    )
    .bind(tenant.as_uuid())
    .bind(namespace_id(namespace).as_uuid())
    .bind(Uuid::nil())
    .bind(20 * MICROTOKENS)
    .execute(&api.db.pool)
    .await
    .expect("seed admission bucket");
}

async fn bucket_tokens(api: &TestApi, tenant: TenantId, namespace: &str) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT microtokens FROM quota_rate_buckets WHERE tenant_id=$1 AND namespace_id=$2 \
         AND resource='api_requests_per_second' AND source_kind='NAMESPACE' AND source_id=$3",
    )
    .bind(tenant.as_uuid())
    .bind(namespace_id(namespace).as_uuid())
    .bind(Uuid::nil())
    .fetch_optional(&api.db.pool)
    .await
    .expect("read admission bucket")
}
