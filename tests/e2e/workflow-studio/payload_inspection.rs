//! ADR-0028 structural inspection through the real REST, PostgreSQL and execution adapters.

use std::time::Duration;

use reqwest::Method;
use sagat_api_grpc::pb::WriteBlobHeader;
use sagat_application::blobs::BlobLimits;
use sagat_domain::identity::Role;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::support::api::Options;
use crate::support::contract::{As, OpenApi, check, scenario};
use crate::support::worker::{NAMESPACE, TENANT, platform, platform_with, single_ir};

async fn platform_with_blobs() -> crate::support::worker::Platform {
    platform_with(Options {
        blobs: Some(BlobLimits {
            max_bytes: 4 * 1024 * 1024,
            max_pending: 8,
            upload_ttl: Duration::from_secs(24 * 3600),
            abandoned_after: Duration::from_secs(3600),
        }),
        ..Options::default()
    })
    .await
}

fn hex_sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn inspect_as(
    p: &crate::support::worker::Platform,
    subject: &str,
    run: &str,
    selector: Value,
) -> (u16, Value) {
    let response = p
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{run}/payload-inspections"),
            subject,
            TENANT,
        )
        .header("x-octopus-namespace", NAMESPACE)
        .json(&selector)
        .send()
        .await
        .expect("inspect request");
    let status = response.status().as_u16();
    if status == 200 {
        assert_eq!(
            response
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("private, no-store")
        );
    }
    let text = response.text().await.expect("response body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

#[tokio::test]
async fn structure_is_useful_but_every_key_and_scalar_is_absent_for_exact_roles_and_tenants() {
    let p = platform().await;
    for (subject, role) in [
        ("olivia", Role::Operator),
        ("dev", Role::Developer),
        ("vera", Role::Viewer),
        ("worker", Role::Worker),
        ("runner", Role::Runner),
    ] {
        p.grant(p.tenant, subject, role).await;
    }
    let workflow = p
        .publish("inspect-shape", &single_ir("inspect-shape", None))
        .await;
    let canaries = json!({
        "ordinary-private-key": "ORDINARY-SECRET-CANARY",
        "schema-public-name": {"default": 18446744073709551615_u64},
        "truth": true,
        "unicode-key-☃": [null, "UNICODE-VALUE-☃"]
    });
    let forbidden_text = [
        "ordinary-private-key",
        "ORDINARY-SECRET-CANARY",
        "schema-public-name",
        "default",
        "18446744073709551615",
        "truth",
        "unicode-key-☃",
        "UNICODE-VALUE-☃",
    ];
    let run = p.start(&workflow, &canaries).await;

    for invalid in [
        json!({"source": "input", "digest": "caller-controlled"}),
        json!({"source": "journal", "sequence": u64::MAX}),
    ] {
        let response = p
            .api
            .as_user(
                Method::POST,
                &format!("/runs/{run}/payload-inspections"),
                "olivia",
                TENANT,
            )
            .header("x-octopus-namespace", NAMESPACE)
            .json(&invalid)
            .send()
            .await
            .expect("invalid selector request");
        assert_eq!(
            response.status().as_u16(),
            422,
            "{}",
            response.text().await.expect("error")
        );
    }
    let oversized = p
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{run}/payload-inspections"),
            "olivia",
            TENANT,
        )
        .header("x-octopus-namespace", NAMESPACE)
        .json(&json!({"source": "input", "padding": "x".repeat(2_048)}))
        .send()
        .await
        .expect("oversized selector request");
    assert_eq!(oversized.status().as_u16(), 413);

    let contract = OpenApi::load();
    let checked = check(
        &p.api,
        &contract,
        &scenario(
            "POST",
            "/runs/{runId}/payload-inspections",
            &format!("/runs/{run}/payload-inspections"),
            As::User("olivia", TENANT),
            Some(json!({"source": "input"})),
            200,
        ),
    )
    .await;
    assert_eq!(checked.2, 200);

    for subject in ["alice", "olivia"] {
        let (status, body) = inspect_as(&p, subject, &run, json!({"source": "input"})).await;
        assert_eq!(status, 200, "{subject}: {body}");
        assert_eq!(body["policy"], "structure-only-v1");
        assert_eq!(body["kind"], "inline-json");
        assert_eq!(body["tree"]["type"], "object");
        assert_eq!(body["tree"]["childCount"], 4);
        let encoded = body.to_string();
        for canary in forbidden_text {
            assert!(
                !encoded.contains(canary),
                "response retained {canary:?}: {body}"
            );
        }
    }

    for subject in ["dev", "vera", "worker", "runner"] {
        let (status, body) = inspect_as(&p, subject, &run, json!({"source": "input"})).await;
        assert_eq!(status, 403, "{subject}: {body}");
        let error = body.to_string();
        for canary in forbidden_text {
            assert!(!error.contains(canary), "error retained {canary:?}: {body}");
        }
    }

    let foreign = p
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{run}/payload-inspections"),
            "bob",
            "globex",
        )
        .json(&json!({"source": "input"}))
        .send()
        .await
        .expect("foreign inspection");
    assert_eq!(foreign.status().as_u16(), 404);
    let foreign_error = foreign.text().await.expect("foreign error body");
    for canary in forbidden_text {
        assert!(
            !foreign_error.contains(canary),
            "foreign error retained {canary:?}: {foreign_error}"
        );
    }

    let audit_rows: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT actor, action, resource_type, resource_id, reason FROM audit_records \
         WHERE action IN ('payload.inspected', 'payload.inspection_denied', \
                          'access.denied.cross_tenant')",
    )
    .fetch_all(&p.api.db.pool)
    .await
    .expect("inspection audit rows");
    let audit_text = format!("{audit_rows:?}");
    for canary in forbidden_text {
        assert!(
            !audit_text.contains(canary),
            "audit retained {canary:?}: {audit_text}"
        );
    }
}

#[tokio::test]
async fn historical_selection_forged_descriptors_and_audit_failure_are_fail_closed() {
    let p = platform_with_blobs().await;
    p.grant(p.tenant, "olivia", Role::Operator).await;
    let workflow = p
        .publish("inspect-history", &single_ir("inspect-history", None))
        .await;
    let forged = json!({"$blob": {"id": Uuid::now_v7().to_string(), "size": 999999, "sha256": "f".repeat(64), "contentType": "application/json"}});
    let run = p.start(&workflow, &forged).await;

    let (_, forged_view) = inspect_as(&p, "olivia", &run, json!({"source": "input"})).await;
    assert_eq!(forged_view["kind"], "inline-json", "{forged_view}");
    assert_eq!(forged_view["tree"]["type"], "object");

    let mut worker = p.worker("inspector-worker").await;
    worker.register_version(&["act.a"], "1.3").await;
    let task = worker.one_task().await;
    worker
        .complete(&task, &json!({"attempt-secret": "HISTORICAL-CANARY"}))
        .await
        .expect("complete output");
    let (status, journal) = p
        .rest(Method::GET, &format!("/runs/{run}/events?limit=200"), None)
        .await;
    assert_eq!(status, 200, "{journal}");
    let events = journal["items"].as_array().expect("events");
    let output_sequence = events
        .iter()
        .find(|event| event["eventType"] == "step.completed" && event["payloadRef"].is_string())
        .and_then(|event| event["sequence"].as_u64().or_else(|| event["seq"].as_u64()))
        .expect("completed output sequence");
    let no_payload_sequence = events
        .iter()
        .find(|event| event["payloadRef"].is_null())
        .and_then(|event| event["sequence"].as_u64().or_else(|| event["seq"].as_u64()))
        .expect("event without payload");

    let (status, output) = inspect_as(
        &p,
        "olivia",
        &run,
        json!({"source": "journal", "sequence": output_sequence}),
    )
    .await;
    assert_eq!(status, 200, "{output}");
    assert_eq!(output["kind"], "inline-json");
    assert_eq!(output["nodeId"], "a");
    assert_eq!(output["attempt"], 1);
    assert!(!output.to_string().contains("HISTORICAL-CANARY"));
    assert!(!output.to_string().contains("attempt-secret"));
    let (repeat_status, repeat) = inspect_as(
        &p,
        "olivia",
        &run,
        json!({"source": "journal", "sequence": output_sequence}),
    )
    .await;
    assert_eq!(repeat_status, 200, "{repeat}");
    let collapsed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records \
         WHERE action = 'payload.inspected' AND resource_id = $1 AND reason = $2",
    )
    .bind(&run)
    .bind(format!("structure-only-v1:journal:{output_sequence}"))
    .fetch_one(&p.api.db.pool)
    .await
    .expect("collapsed audit count");
    assert_eq!(
        collapsed, 1,
        "the same actor/run/selector is collapsed for 300 seconds"
    );

    let (status, unavailable) = inspect_as(
        &p,
        "olivia",
        &run,
        json!({"source": "journal", "sequence": no_payload_sequence}),
    )
    .await;
    assert_eq!(status, 200, "{unavailable}");
    assert_eq!(unavailable["kind"], "unavailable");
    assert_eq!(unavailable["reason"], "NO_RETAINED_PAYLOAD");

    let (status, missing) = inspect_as(
        &p,
        "olivia",
        &run,
        json!({"source": "journal", "sequence": 999999}),
    )
    .await;
    assert_eq!(status, 404, "{missing}");

    let external_run = p
        .start(
            &workflow,
            &json!({"input-key": "EXTERNAL-RUN-INPUT-CANARY"}),
        )
        .await;
    let external_task = worker.one_task().await;
    let external_bytes = br#"{"external-key":"EXTERNAL-OUTPUT-CANARY"}"#;
    let stored = worker
        .write_blob(
            &external_task,
            WriteBlobHeader {
                expected_size: i64::try_from(external_bytes.len()).expect("payload size fits"),
                expected_sha256: hex_sha256(external_bytes),
                content_type: "application/json".into(),
                ..WriteBlobHeader::default()
            },
            external_bytes,
            16,
        )
        .await
        .expect("upload external output");
    worker
        .complete_with_blob(&external_task, &stored.blob_id)
        .await
        .expect("commit external output");
    let (status, external_journal) = p
        .rest(
            Method::GET,
            &format!("/runs/{external_run}/events?limit=200"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{external_journal}");
    let external_sequence = external_journal["items"]
        .as_array()
        .expect("external run events")
        .iter()
        .find(|event| event["eventType"] == "step.completed" && event["payloadRef"].is_string())
        .and_then(|event| event["sequence"].as_u64().or_else(|| event["seq"].as_u64()))
        .expect("external output sequence");
    let (status, external) = inspect_as(
        &p,
        "olivia",
        &external_run,
        json!({"source": "journal", "sequence": external_sequence}),
    )
    .await;
    assert_eq!(status, 200, "{external}");
    assert_eq!(external["kind"], "external");
    assert_eq!(external["contentKind"], "json");
    assert_eq!(external["sizeBytes"], external_bytes.len());
    assert_eq!(external["reason"], "EXTERNAL_CONTENT_NOT_INSPECTED");
    assert_eq!(external["nodeId"], "a");
    assert_eq!(external["attempt"], 1);
    assert!(!external.to_string().contains("EXTERNAL-OUTPUT-CANARY"));
    assert!(!external.to_string().contains("external-key"));

    let external_input_bytes = br#"{"input-key":"EXTERNAL-INPUT-CANARY"}"#;
    let staged = p
        .api
        .as_user(Method::POST, "/run-input-blobs", "alice", TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .header("content-type", "application/json")
        .header("x-octopus-blob-size", external_input_bytes.len())
        .header("x-octopus-blob-sha256", hex_sha256(external_input_bytes))
        .body(external_input_bytes.to_vec())
        .send()
        .await
        .expect("stage external input");
    assert_eq!(staged.status().as_u16(), 201);
    let staged: Value = staged.json().await.expect("staged input response");
    let input_blob = staged["id"].as_str().expect("staged input id");
    let started = p
        .api
        .as_user(Method::POST, "/runs", "alice", TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", "payload-inspection-external-input")
        .json(&json!({"workflowId": workflow, "inputBlobRef": input_blob}))
        .send()
        .await
        .expect("start with external input");
    assert_eq!(started.status().as_u16(), 202);
    let started: Value = started.json().await.expect("external input run response");
    let input_run = started["runId"].as_str().expect("external input run id");
    let (status, external_input) =
        inspect_as(&p, "olivia", input_run, json!({"source": "input"})).await;
    assert_eq!(status, 200, "{external_input}");
    assert_eq!(external_input["kind"], "external");
    assert_eq!(external_input["contentKind"], "json");
    assert_eq!(external_input["sizeBytes"], external_input_bytes.len());
    assert!(!external_input.to_string().contains("EXTERNAL-INPUT-CANARY"));
    assert!(!external_input.to_string().contains("input-key"));

    sqlx::query(
        "ALTER TABLE audit_records ADD CONSTRAINT test_reject_payload_inspection \
         CHECK (action <> 'payload.inspected') NOT VALID",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("inject audit failure");
    let (status, refused) =
        inspect_as(&p, "olivia", &external_run, json!({"source": "input"})).await;
    assert_eq!(status, 503, "{refused}");
    let refused_text = refused.to_string();
    assert!(!refused_text.contains("$blob"));
    assert!(!refused_text.contains("HISTORICAL-CANARY"));
}
