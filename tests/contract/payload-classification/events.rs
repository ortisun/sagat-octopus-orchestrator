use std::fs;

use super::support::{self, api::TestApi, contract::OpenApi};
use reqwest::Method;
use serde_json::Value;
use uuid::Uuid;

async fn read(api: &TestApi, path: &str, template: &str, expected: u16) -> Value {
    let response = api
        .as_user(Method::GET, path, "admin", "event-contract")
        .send()
        .await
        .expect("event read");
    assert_eq!(response.status().as_u16(), expected);
    if expected == 200 {
        assert_eq!(response.headers()["cache-control"], "private, no-store");
    }
    let body = support::api::json_of(response).await;
    OpenApi::load()
        .validate_body("GET", template, expected, &body)
        .expect("documented response")
        .expect("response schema");
    body
}

#[tokio::test]
async fn actual_event_and_letter_reads_validate_reveal_refusals_and_revocation() {
    let api = support::api::start().await;
    let tenant = api.tenant("event-contract", "admin").await;
    let ws = api
        .create_workspace("admin", "event-contract", "work")
        .await;
    let ns = api
        .create_namespace("admin", "event-contract", &ws, "events", "dev")
        .await;
    let namespace = Uuid::parse_str(&ns).expect("namespace UUID");
    let connector = Uuid::now_v7();
    let letter = Uuid::now_v7();
    const CANARY: &str = "EVENT-CONTRACT-PII-CANARY";
    let payload = serde_json::to_vec(&serde_json::json!({"personal":CANARY})).expect("payload");
    sqlx::query("INSERT INTO inbox_events (tenant_id,namespace_id,source,event_id,event_type,payload,payload_digest,received_at,expires_at,next_attempt_at) VALUES ($1,$2,'contract','event','contract.created',$3,decode(repeat('01',32),'hex'),clock_timestamp(),clock_timestamp()+interval '1 hour',clock_timestamp())")
        .bind(tenant.as_uuid()).bind(namespace).bind(&payload).execute(&api.db.pool).await.expect("event fixture");
    sqlx::query("INSERT INTO event_connectors (id,tenant_id,namespace_id,slug,kind,settings,rate_per_second,created_by) VALUES ($1,$2,$3,'contract-connector','KAFKA','{\"servers\":[\"tls://broker.invalid:9092\"],\"topic\":\"contract\",\"startFrom\":\"LATEST\"}',10,'admin')")
        .bind(connector).bind(tenant.as_uuid()).bind(namespace).execute(&api.db.pool).await.expect("connector fixture");
    sqlx::query("INSERT INTO connector_dead_letters (id,tenant_id,namespace_id,connector_id,position,reason,headers,body) VALUES ($1,$2,$3,$4,'contract/0/1','BAD_ENVELOPE',$5,$6)")
        .bind(letter).bind(tenant.as_uuid()).bind(namespace).bind(connector)
        .bind(serde_json::json!({"authorization":CANARY})).bind(payload).execute(&api.db.pool).await.expect("letter fixture");
    let endpoints = [
        (
            format!("/namespaces/{ns}/events/contract/event"),
            "/namespaces/{namespaceId}/events/{source}/{eventId}",
        ),
        (
            format!("/namespaces/{ns}/connectors/{connector}/dead-letters/{letter}"),
            "/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters/{deadLetterId}",
        ),
    ];
    for (path, template) in &endpoints {
        let default = read(&api, path, template, 200).await;
        assert!(!default.to_string().contains(CANARY));
        let denied = read(&api, &format!("{path}?reveal=pii"), template, 403).await;
        assert_eq!(denied["details"]["reason"], "PII_REVEAL_NOT_GRANTED");
        let invalid = read(&api, &format!("{path}?reveal={CANARY}"), template, 422).await;
        assert!(!invalid.to_string().contains(CANARY));
    }
    let grant = format!("/namespaces/{ns}/payload-reveal-grants/admin");
    assert_eq!(
        api.as_user(Method::PUT, &grant, "admin", "event-contract")
            .send()
            .await
            .expect("grant")
            .status()
            .as_u16(),
        200
    );
    for (path, template) in &endpoints {
        let revealed = read(&api, &format!("{path}?reveal=pii"), template, 200).await;
        assert!(revealed.to_string().contains(CANARY));
    }
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='payload.pii_revealed'",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("reveal audit");
    assert_eq!(
        audit_count, 2,
        "one mandatory audit per successful response"
    );
    // A real PostgreSQL audit write failure must withhold both bodies over HTTP.
    sqlx::raw_sql("CREATE FUNCTION refuse_contract_reveal_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='payload.pii_revealed' THEN RAISE EXCEPTION 'injected audit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER refuse_contract_reveal_audit BEFORE INSERT ON audit_records FOR EACH ROW EXECUTE FUNCTION refuse_contract_reveal_audit();")
        .execute(&api.db.pool).await.expect("audit fault");
    for (path, template) in &endpoints {
        let refused = read(&api, &format!("{path}?reveal=pii"), template, 503).await;
        assert!(!refused.to_string().contains(CANARY));
    }
    sqlx::raw_sql("DROP TRIGGER refuse_contract_reveal_audit ON audit_records; DROP FUNCTION refuse_contract_reveal_audit();")
        .execute(&api.db.pool).await.expect("remove audit fault");
    assert_eq!(
        api.as_user(Method::DELETE, &grant, "admin", "event-contract")
            .send()
            .await
            .expect("revoke")
            .status()
            .as_u16(),
        200
    );
    for (path, template) in &endpoints {
        let denied = read(&api, &format!("{path}?reveal=pii"), template, 403).await;
        assert_eq!(denied["details"]["reason"], "PII_REVEAL_NOT_GRANTED");
    }
    let after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='payload.pii_revealed'",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("reveal audit after fault/revoke");
    assert_eq!(after, audit_count);

    // Alongside the existing JSON letter, make a full 200-row page. Non-JSON and a
    // truncated but syntactically valid JSON body independently exercise opacity.
    sqlx::query("INSERT INTO connector_dead_letters (id,tenant_id,namespace_id,connector_id,position,reason,headers,body,truncated) SELECT gen_random_uuid(),$1,$2,$3,'page/'||i,'BAD_ENVELOPE',$4,CASE WHEN i=1 THEN convert_to($5,'UTF8') ELSE convert_to($6,'UTF8') END,i=2 FROM generate_series(1,199) i")
        .bind(tenant.as_uuid()).bind(namespace).bind(connector)
        .bind(serde_json::json!({"authorization":CANARY})).bind(CANARY)
        .bind(serde_json::json!({"personal":CANARY}).to_string())
        .execute(&api.db.pool).await.expect("full page fixture");
    let opaque_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM connector_dead_letters WHERE tenant_id=$1 AND connector_id=$2 AND position IN ('page/1','page/2')")
        .bind(tenant.as_uuid()).bind(connector).fetch_all(&api.db.pool).await.expect("opaque fixtures");
    assert_eq!(opaque_ids.len(), 2);
    for id in opaque_ids {
        let detail = read(
            &api,
            &format!("/namespaces/{ns}/connectors/{connector}/dead-letters/{id}"),
            endpoints[1].1,
            200,
        )
        .await;
        assert_eq!(detail["bodyStructure"]["kind"], "opaque");
        assert!(detail.get("body").is_none());
        assert!(detail.get("headers").is_none());
    }
    assert_eq!(
        api.as_user(Method::PUT, &grant, "admin", "event-contract")
            .send()
            .await
            .expect("restore grant")
            .status()
            .as_u16(),
        200
    );
    let page = read(
        &api,
        &format!("/namespaces/{ns}/connectors/{connector}/dead-letters?limit=200&reveal=pii"),
        "/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters",
        200,
    )
    .await;
    let items = page["items"].as_array().expect("full page");
    assert_eq!(items.len(), 200);
    for item in items {
        assert_eq!(item["bodyDisclosure"], "structure-only");
        assert!(item.get("body").is_none());
        assert!(item.get("headers").is_none());
        assert!(!item.to_string().contains(CANARY));
    }
    let after_list: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='payload.pii_revealed'",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("list must not reveal");
    assert_eq!(after_list, audit_count);
}

fn document() -> Value {
    let path =
        super::repository_root().join("specs/001-platform-foundation/contracts/openapi.yaml");
    serde_saphyr::from_str(&fs::read_to_string(path).expect("canonical OpenAPI exists"))
        .expect("canonical OpenAPI is YAML")
}

#[test]
fn detail_reads_accept_only_the_closed_pii_reveal_query() {
    let api = document();
    for path in [
        "/namespaces/{namespaceId}/events/{source}/{eventId}",
        "/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters/{deadLetterId}",
    ] {
        let reveal = api["paths"][path]["get"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "reveal")
            .expect("detail reveal query");
        assert_eq!(reveal["schema"]["enum"], serde_json::json!(["pii"]));
    }
    let list =
        &api["paths"]["/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters"]["get"];
    assert!(
        !list["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "reveal")
    );
}

#[test]
fn event_and_letter_values_are_conditional_but_disclosure_metadata_is_required() {
    let api = document();
    let schemas = &api["components"]["schemas"];
    let inbox_required = schemas["InboxEvent"]["required"].as_array().unwrap();
    for field in ["dataDisclosure", "dataSizeBytes"] {
        assert!(inbox_required.iter().any(|v| v == field));
    }
    assert!(!inbox_required.iter().any(|v| v == "data"));
    let letter_required = schemas["DeadLetter"]["required"].as_array().unwrap();
    for field in ["bodyDisclosure", "bodySizeBytes", "headerCount"] {
        assert!(letter_required.iter().any(|v| v == field));
    }
    for value in ["body", "headers"] {
        assert!(!letter_required.iter().any(|v| v == value));
    }
}
