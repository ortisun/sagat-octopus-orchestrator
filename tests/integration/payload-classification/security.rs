//! Live tenant/namespace opacity, admission and restricted audit boundaries (T016-051).
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use sagat_domain::identity::{Membership, Role, TenantId};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use super::support::{
    self,
    api::TestApi,
    engine::{activity, document},
};

const CANARY: &str = "SECURITY-BOUNDARY-CANARY";
const PRIMARY: &str = "security-primary";
const FOREIGN: &str = "security-foreign";

async fn call(
    api: &TestApi,
    actor: &str,
    tenant: &str,
    method: Method,
    path: &str,
    body: Option<&Value>,
    expected: u16,
) -> Value {
    let mut request = api.as_user(method, path, actor, tenant);
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.expect("security HTTP request");
    assert_eq!(response.status().as_u16(), expected);
    let body = support::api::json_of(response).await;
    if expected >= 400 {
        assert!(
            !body.to_string().contains(CANARY),
            "error reflected a canary"
        );
    }
    body
}

async fn run(api: &TestApi, tenant: &str, actor: &str, namespace: &str, name: &str) -> String {
    let mut ir = document(name, "work", vec![activity("work")]);
    ir["inputSchema"] = json!({"type":"object","x-sagat-classification":"pii",
        "additionalProperties":false,"properties":{"personal":{"type":"string"}}});
    let response = api
        .as_user(Method::POST, "/workflows", actor, tenant)
        .header("x-octopus-namespace", namespace)
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name":name,"ir":ir}))
        .send()
        .await
        .expect("create workflow");
    assert_eq!(response.status().as_u16(), 201);
    let created = support::api::json_of(response).await;
    let workflow = created["id"].as_str().expect("workflow id");
    let response = api
        .as_user(
            Method::POST,
            &format!("/workflows/{workflow}/publish"),
            actor,
            tenant,
        )
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"draftRevision":created["draftRevision"]}))
        .send()
        .await
        .expect("publish");
    assert_eq!(response.status().as_u16(), 201);
    let response = api
        .as_user(Method::POST, "/runs", actor, tenant)
        .header("x-octopus-namespace", namespace)
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId":workflow,"input":{"personal":CANARY}}))
        .send()
        .await
        .expect("start");
    assert_eq!(response.status().as_u16(), 202);
    support::api::json_of(response).await["runId"]
        .as_str()
        .expect("run id")
        .to_owned()
}

async fn event_and_letter(
    api: &TestApi,
    tenant: TenantId,
    namespace: &str,
    actor: &str,
) -> (Uuid, Uuid) {
    let namespace = Uuid::parse_str(namespace).expect("namespace");
    let payload = serde_json::to_vec(&json!({"personal":CANARY})).expect("event bytes");
    sqlx::query("INSERT INTO inbox_events (tenant_id,namespace_id,source,event_id,event_type,payload,payload_digest,received_at,expires_at,next_attempt_at) VALUES ($1,$2,'security','event','security.created',$3,$4,clock_timestamp(),clock_timestamp()+interval '1 hour',clock_timestamp())")
        .bind(tenant.as_uuid()).bind(namespace).bind(&payload).bind(Sha256::digest(&payload).to_vec())
        .execute(&api.db.pool).await.expect("event fixture");
    let connector = Uuid::now_v7();
    let letter = Uuid::now_v7();
    sqlx::query("INSERT INTO event_connectors (id,tenant_id,namespace_id,slug,kind,settings,rate_per_second,created_by) VALUES ($1,$2,$3,'security-connector','KAFKA','{\"servers\":[\"tls://broker.invalid:9092\"],\"topic\":\"security\",\"startFrom\":\"LATEST\"}',10,$4)")
        .bind(connector).bind(tenant.as_uuid()).bind(namespace).bind(actor).execute(&api.db.pool).await.expect("connector fixture");
    sqlx::query("INSERT INTO connector_dead_letters (id,tenant_id,namespace_id,connector_id,position,reason,headers,body) VALUES ($1,$2,$3,$4,'security/0/1','BAD_ENVELOPE',$5,$6)")
        .bind(letter).bind(tenant.as_uuid()).bind(namespace).bind(connector)
        .bind(json!({"authorization":CANARY})).bind(payload).execute(&api.db.pool).await.expect("letter fixture");
    (connector, letter)
}

type Bucket = (Uuid, String, String, Uuid, i64, OffsetDateTime);
async fn buckets(api: &TestApi, tenant: TenantId) -> Vec<Bucket> {
    sqlx::query_as("SELECT namespace_id,resource,source_kind,source_id,microtokens,updated_at FROM quota_rate_buckets WHERE tenant_id=$1 ORDER BY namespace_id,resource,source_kind,source_id")
        .bind(tenant.as_uuid()).fetch_all(&api.db.pool).await.expect("foreign admission snapshot")
}

#[tokio::test]
async fn foreign_resources_never_debit_their_namespace_or_escape_through_tenant_audit() {
    let api = support::api::start().await;
    let tenant = api.tenant(PRIMARY, "admin").await;
    let ws = api.create_workspace("admin", PRIMARY, "work").await;
    let a = api
        .create_namespace("admin", PRIMARY, &ws, "primary", "dev")
        .await;
    let b = api
        .create_namespace("admin", PRIMARY, &ws, "secondary", "dev")
        .await;
    let foreign = api.tenant(FOREIGN, "foreign-admin").await;
    let fws = api
        .create_workspace("foreign-admin", FOREIGN, "foreign-work")
        .await;
    let f = api
        .create_namespace("foreign-admin", FOREIGN, &fws, "foreign", "dev")
        .await;
    api.owner_store
        .insert_membership(&Membership::restore(
            tenant,
            support::subject("reader"),
            Role::Developer,
            support::now(),
        ))
        .await
        .expect("reader");
    api.owner_store
        .insert_membership(&Membership::restore(
            foreign,
            support::subject("foreign-reader"),
            Role::Developer,
            support::now(),
        ))
        .await
        .expect("foreign reader");
    let own = run(&api, PRIMARY, "admin", "primary", "security-own").await;
    let other_namespace = run(&api, PRIMARY, "admin", "secondary", "security-secondary").await;
    let foreign_run = run(
        &api,
        FOREIGN,
        "foreign-admin",
        "foreign",
        "security-foreign-run",
    )
    .await;
    let (foreign_connector, foreign_letter) =
        event_and_letter(&api, foreign, &f, "foreign-admin").await;
    let (local_connector, local_letter) = event_and_letter(&api, tenant, &b, "admin").await;
    let before = buckets(&api, foreign).await;
    assert!(
        !before.is_empty(),
        "the foreign namespace must have an actual admission bucket"
    );

    call(
        &api,
        "admin",
        PRIMARY,
        Method::PUT,
        &format!("/namespaces/{a}/payload-reveal-grants/reader"),
        None,
        200,
    )
    .await;
    let revealed = call(
        &api,
        "reader",
        PRIMARY,
        Method::POST,
        &format!("/runs/{own}/payload-disclosures"),
        Some(&json!({"source":"input","reveal":["pii"]})),
        200,
    )
    .await;
    assert!(
        revealed.to_string().contains(CANARY),
        "the own-namespace grant must actually reveal"
    );
    for path in [
        format!("/runs/{other_namespace}/payload-disclosures"),
        format!("/namespaces/{b}/events/security/event?reveal=pii"),
        format!(
            "/namespaces/{b}/connectors/{local_connector}/dead-letters/{local_letter}?reveal=pii"
        ),
    ] {
        let run_path = path.contains("payload-disclosures");
        let body = call(
            &api,
            "reader",
            PRIMARY,
            if run_path { Method::POST } else { Method::GET },
            &path,
            run_path.then_some(&json!({"source":"input","reveal":["pii"]})),
            403,
        )
        .await;
        assert_eq!(body["details"]["reason"], "PII_REVEAL_NOT_GRANTED");
    }
    for run in [&foreign_run, &Uuid::now_v7().to_string()] {
        for selector in [
            json!({"source":"input"}),
            json!({"source":"journal","sequence":1}),
        ] {
            let body = call(
                &api,
                "admin",
                PRIMARY,
                Method::POST,
                &format!("/runs/{run}/payload-disclosures"),
                Some(&selector),
                404,
            )
            .await;
            assert_eq!(body["code"], "NOT_FOUND");
        }
    }
    for namespace in [&f, &Uuid::now_v7().to_string()] {
        for path in [
            format!("/namespaces/{namespace}/events/security/event"),
            format!("/namespaces/{namespace}/connectors/{foreign_connector}/dead-letters"),
            format!(
                "/namespaces/{namespace}/connectors/{foreign_connector}/dead-letters/{foreign_letter}?reveal=pii"
            ),
            format!("/namespaces/{namespace}/payload-reveal-grants"),
        ] {
            call(&api, "admin", PRIMARY, Method::GET, &path, None, 404).await;
        }
        call(
            &api,
            "admin",
            PRIMARY,
            Method::PUT,
            &format!("/namespaces/{namespace}/payload-reveal-grants/reader"),
            None,
            404,
        )
        .await;
    }
    for subject in ["foreign-reader", "missing-reader"] {
        let body = call(
            &api,
            "admin",
            PRIMARY,
            Method::PUT,
            &format!("/namespaces/{a}/payload-reveal-grants/{subject}"),
            None,
            422,
        )
        .await;
        assert_eq!(body["details"]["reason"], "UNKNOWN_SUBJECT");
        assert!(!body.to_string().contains(subject));
    }
    for selector in [
        json!({"source":CANARY}),
        json!({"source":"input","reveal":[CANARY]}),
    ] {
        call(
            &api,
            "admin",
            PRIMARY,
            Method::POST,
            &format!("/runs/{own}/payload-disclosures"),
            Some(&selector),
            422,
        )
        .await;
    }
    assert_eq!(buckets(&api, foreign).await, before);
    let restricted: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='access.denied.cross_tenant' AND resource_id=$2")
        .bind(tenant.as_uuid()).bind(&foreign_run).fetch_one(&api.db.pool).await.expect("restricted audit witness");
    assert!(restricted > 0);
    let audit = call(
        &api,
        "admin",
        PRIMARY,
        Method::GET,
        "/audit?action=access.denied.cross_tenant&limit=200",
        None,
        200,
    )
    .await;
    assert_eq!(audit["items"].as_array().expect("audit items").len(), 0);
}
