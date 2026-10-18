//! Public collection completion must honor the already declared End output contract.

use crate::support::scope_flow::{Flow, collection_document};
use sagat_application::execution::MAX_PAYLOAD_BYTES;
use serde_json::{Value, json};
use sqlx::Row;

fn decoded(value: Option<Vec<u8>>) -> Option<Value> {
    value.map(|bytes| serde_json::from_slice(&bytes).expect("retained JSON output"))
}

#[tokio::test]
async fn collection_end_retains_owner_aggregate_and_evaluated_run_result() {
    let name = "collection-end-result";
    let mut document = collection_document(
        name,
        vec![
            json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
            json!({"id":"returned","type":"return","result":"steps.work.output"}),
        ],
        1,
    );
    // A scalar, distinct from the aggregate, proves the declared expression was evaluated.
    document["nodes"][1]["outputExpr"] = json!("steps.lines.output[0].amount");
    let f = Flow::start(name, document, json!([{"id":1}])).await;
    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    f.complete(&tasks[0], json!({"amount":17})).await;
    f.drive().await;
    assert_eq!(f.run_status().await, "SUCCEEDED");
    f.verify().await;

    let row = sqlx::query(
        "SELECT scope.lifecycle,aggregate.content AS aggregate,\
         owner_payload.content AS owner_output,end_payload.content AS end_output,\
         run_payload.content AS run_result,\
         owner.output_digest AS owner_digest,scope.result_digest AS aggregate_digest,\
         ended.output_digest AS end_digest,r.result_digest AS run_digest,\
         (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
          AND e.run_id=r.id AND e.event_type='run.finished') AS finished_events \
         FROM runs r JOIN iteration_scopes scope ON scope.tenant_id=r.tenant_id \
          AND scope.run_id=r.id AND scope.owner_node='lines' \
          AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
         JOIN steps owner ON owner.tenant_id=r.tenant_id AND owner.run_id=r.id \
          AND owner.node='lines' AND owner.instance_path_nodes='{}' \
          AND owner.instance_path_indices='{}' \
         JOIN steps ended ON ended.tenant_id=r.tenant_id AND ended.run_id=r.id \
          AND ended.node='done' AND ended.instance_path_nodes='{}' \
          AND ended.instance_path_indices='{}' \
         JOIN payloads aggregate ON aggregate.tenant_id=r.tenant_id \
          AND aggregate.digest=scope.result_digest \
         LEFT JOIN payloads owner_payload ON owner_payload.tenant_id=r.tenant_id \
          AND owner_payload.digest=owner.output_digest \
         LEFT JOIN payloads end_payload ON end_payload.tenant_id=r.tenant_id \
          AND end_payload.digest=ended.output_digest \
         LEFT JOIN payloads run_payload ON run_payload.tenant_id=r.tenant_id \
          AND run_payload.digest=r.result_digest \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual successful collection and terminal output projection");
    let aggregate = json!([{"amount":17}]);
    assert_eq!(row.get::<String, _>("lifecycle"), "SUCCEEDED");
    assert_eq!(decoded(Some(row.get("aggregate"))), Some(aggregate.clone()));
    assert_eq!(row.get::<i64, _>("finished_events"), 1);
    let outputs = (
        decoded(row.get("owner_output")),
        decoded(row.get("end_output")),
        decoded(row.get("run_result")),
    );
    assert_eq!(
        outputs,
        (Some(aggregate), Some(json!(17)), Some(json!(17))),
        "collection aggregate, evaluated End output and Run result must all be retained"
    );
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("owner_digest"),
        Some(row.get::<Vec<u8>, _>("aggregate_digest"))
    );
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("end_digest"),
        row.get::<Option<Vec<u8>>, _>("run_digest")
    );
}

#[tokio::test]
async fn collection_success_does_not_bypass_the_successor_end_instance_limit() {
    let name = "collection-end-instance-limit";
    let mut document = collection_document(
        name,
        vec![
            json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
            json!({"id":"returned","type":"return","result":"steps.work.output"}),
        ],
        1,
    );
    document["limits"] = json!({"maxInstances":3});
    document["nodes"][1]["outputExpr"] = json!("steps.lines.output[0].amount");
    let f = Flow::start(name, document, json!([{"id":1}])).await;
    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    f.complete(&tasks[0], json!({"amount":17})).await;
    f.drive().await;
    assert_eq!(
        f.run_status().await,
        "FAILED",
        "End needs its own admitted instance"
    );
    let boundary: (String, String, bool, bool, i64, Option<String>) = sqlx::query_as(
        "SELECT scope.lifecycle,ended.status,ended.output_digest IS NULL,\
         r.result_digest IS NULL,\
         (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
          AND e.run_id=r.id AND e.event_type='end.reached'),\
         (SELECT e.detail->>'code' FROM run_events e WHERE e.tenant_id=r.tenant_id \
          AND e.run_id=r.id AND e.event_type='run.stop_requested') \
         FROM runs r JOIN iteration_scopes scope ON scope.tenant_id=r.tenant_id \
          AND scope.run_id=r.id AND scope.owner_node='lines' \
         JOIN steps ended ON ended.tenant_id=r.tenant_id AND ended.run_id=r.id \
          AND ended.node='done' AND ended.instance_path_nodes='{}' \
          AND ended.instance_path_indices='{}' \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual collection-success/End-refusal boundary");
    assert_eq!(
        boundary,
        (
            "SUCCEEDED".into(),
            "CANCELLED".into(),
            true,
            true,
            0,
            Some("INSTANCE_LIMIT".into()),
        )
    );
    f.verify().await;
}

#[tokio::test]
async fn collection_end_preserves_optional_null_and_typed_evaluation_semantics() {
    struct Case {
        name: &'static str,
        output_expr: Option<&'static str>,
        worker_output: Value,
        expected_status: &'static str,
        expected_end_status: &'static str,
        expected_result: Option<Value>,
        expected_code: Option<&'static str>,
    }

    let ordinary = json!({"amount":17});
    let large_source = Value::String("x".repeat(139_998));
    assert_eq!(large_source.to_string().len(), 140_000);
    assert!(
        Value::Array(vec![large_source.clone()]).to_string().len() <= MAX_PAYLOAD_BYTES,
        "the genuine collection aggregate must fit before End evaluation"
    );
    let cases = [
        Case {
            name: "collection-end-output-omitted",
            output_expr: None,
            worker_output: ordinary.clone(),
            expected_status: "SUCCEEDED",
            expected_end_status: "SUCCEEDED",
            expected_result: None,
            expected_code: None,
        },
        Case {
            name: "collection-end-output-null",
            output_expr: Some("null"),
            worker_output: ordinary.clone(),
            expected_status: "SUCCEEDED",
            expected_end_status: "SUCCEEDED",
            expected_result: Some(Value::Null),
            expected_code: None,
        },
        Case {
            name: "collection-end-output-missing-field",
            output_expr: Some("steps.lines.output[0].missing"),
            worker_output: ordinary,
            expected_status: "FAILED",
            expected_end_status: "FAILED",
            expected_result: None,
            expected_code: Some("EXPRESSION_ERROR"),
        },
        Case {
            name: "collection-end-output-over-value-limit",
            output_expr: Some("[steps.lines.output[0], steps.lines.output[0]]"),
            worker_output: large_source,
            expected_status: "FAILED",
            expected_end_status: "FAILED",
            expected_result: None,
            expected_code: Some("RUN_DATA_LIMIT"),
        },
    ];

    for case in cases {
        let mut document = collection_document(
            case.name,
            vec![
                json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
                json!({"id":"returned","type":"return","result":"steps.work.output"}),
            ],
            1,
        );
        if let Some(expression) = case.output_expr {
            document["nodes"][1]["outputExpr"] = json!(expression);
        }
        let f = Flow::start(case.name, document, json!([{"id":1}])).await;
        f.drive().await;
        let tasks = f.poll().await;
        assert_eq!(tasks.len(), 1, "{} public member lease", case.name);
        f.complete(&tasks[0], case.worker_output.clone()).await;
        f.drive().await;
        assert_eq!(f.run_status().await, case.expected_status, "{}", case.name);

        let row = sqlx::query(
            "SELECT scope.lifecycle,owner.status AS owner_status,ended.status AS end_status, \
                    owner_payload.content AS owner_output,end_payload.content AS end_output, \
                    run_payload.content AS run_result,owner.output_digest AS owner_digest, \
                    scope.result_digest AS aggregate_digest,ended.output_digest AS end_digest, \
                    r.result_digest AS run_digest,r.stop_code, \
                    (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
                      AND e.run_id=r.id AND e.event_type='end.reached') AS end_events, \
                    (SELECT e.payload_digest FROM run_events e WHERE e.tenant_id=r.tenant_id \
                      AND e.run_id=r.id AND e.event_type='end.reached' LIMIT 1) AS end_event_digest, \
                    (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
                      AND e.run_id=r.id AND e.event_type='run.stop_requested') AS stop_events, \
                    (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
                      AND e.run_id=r.id AND e.event_type='run.finished') AS finished_events, \
                    (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=r.tenant_id \
                      AND o.aggregate_id=r.id AND o.event_type='run.finished') AS finished_outboxes, \
                    COALESCE((SELECT bool_and(o.payload->>'status'=r.status) FROM event_outbox o \
                      WHERE o.tenant_id=r.tenant_id AND o.aggregate_id=r.id \
                        AND o.event_type='run.finished'),false) AS outbox_status_matches \
               FROM runs r JOIN iteration_scopes scope ON scope.tenant_id=r.tenant_id \
                AND scope.run_id=r.id AND scope.owner_node='lines' \
                AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
               JOIN steps owner ON owner.tenant_id=r.tenant_id AND owner.run_id=r.id \
                AND owner.node='lines' AND owner.instance_path_nodes='{}' \
                AND owner.instance_path_indices='{}' \
               JOIN steps ended ON ended.tenant_id=r.tenant_id AND ended.run_id=r.id \
                AND ended.node='done' AND ended.instance_path_nodes='{}' \
                AND ended.instance_path_indices='{}' \
               LEFT JOIN payloads owner_payload ON owner_payload.tenant_id=r.tenant_id \
                AND owner_payload.digest=owner.output_digest \
               LEFT JOIN payloads end_payload ON end_payload.tenant_id=r.tenant_id \
                AND end_payload.digest=ended.output_digest \
               LEFT JOIN payloads run_payload ON run_payload.tenant_id=r.tenant_id \
                AND run_payload.digest=r.result_digest \
              WHERE r.tenant_id=$1 AND r.id=$2",
        )
        .bind(f.tenant.as_uuid())
        .bind(f.run.as_uuid())
        .fetch_one(&f.api.db.pool)
        .await
        .expect("public collection End semantic projection");

        let aggregate = Value::Array(vec![case.worker_output]);
        assert_eq!(
            row.get::<String, _>("lifecycle"),
            "SUCCEEDED",
            "{}",
            case.name
        );
        assert_eq!(
            row.get::<String, _>("owner_status"),
            "SUCCEEDED",
            "{}",
            case.name
        );
        assert_eq!(
            decoded(row.get("owner_output")),
            Some(aggregate),
            "{}",
            case.name
        );
        assert_eq!(
            row.get::<Option<Vec<u8>>, _>("owner_digest"),
            row.get::<Option<Vec<u8>>, _>("aggregate_digest"),
            "{} owner output is the successful scope aggregate",
            case.name,
        );
        assert_eq!(
            row.get::<String, _>("end_status"),
            case.expected_end_status,
            "{}",
            case.name
        );
        assert_eq!(
            decoded(row.get("end_output")),
            case.expected_result.clone(),
            "{}",
            case.name,
        );
        assert_eq!(
            decoded(row.get("run_result")),
            case.expected_result.clone(),
            "{}",
            case.name,
        );
        assert_eq!(
            row.get::<Option<String>, _>("stop_code").as_deref(),
            case.expected_code,
            "{}",
            case.name
        );

        let expected_terminal_event: i64 = if case.expected_code.is_none() { 1 } else { 0 };
        assert_eq!(
            row.get::<i64, _>("end_events"),
            expected_terminal_event,
            "{}",
            case.name
        );
        assert_eq!(
            row.get::<i64, _>("stop_events"),
            1 - expected_terminal_event,
            "{}",
            case.name
        );
        assert_eq!(row.get::<i64, _>("finished_events"), 1, "{}", case.name);
        assert_eq!(row.get::<i64, _>("finished_outboxes"), 1, "{}", case.name);
        assert!(row.get::<bool, _>("outbox_status_matches"), "{}", case.name);

        let end_digest = row.get::<Option<Vec<u8>>, _>("end_digest");
        let run_digest = row.get::<Option<Vec<u8>>, _>("run_digest");
        let event_digest = row.get::<Option<Vec<u8>>, _>("end_event_digest");
        if case.expected_result.is_some() {
            assert!(
                end_digest.is_some(),
                "{} retained explicit result",
                case.name
            );
            assert_eq!(end_digest, run_digest, "{} End and Run digest", case.name);
            assert_eq!(
                end_digest, event_digest,
                "{} EndReached payload digest",
                case.name
            );
        } else {
            assert_eq!(
                (end_digest, run_digest, event_digest),
                (None, None, None),
                "{}",
                case.name
            );
        }
        f.verify().await;
    }
}

#[tokio::test]
async fn collection_result_uses_pinned_output_schema_for_http_disclosure() {
    const PUBLIC_ANSWER: &str = "collection-public-answer-7f4c";
    const INTERNAL_MARKER: &str = "collection-internal-marker-7f4c";
    const PII_MARKER: &str = "collection-pii-marker-7f4c";
    const UNMARKED_KEY: &str = "collection-unmarked-key-7f4c";
    const UNMARKED_MARKER: &str = "collection-unmarked-marker-7f4c";
    const SECRET_MARKER: &str = "collection-secret-marker-7f4c";

    let name = "collection-result-classification";
    let mut document = collection_document(
        name,
        vec![
            json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
            json!({"id":"returned","type":"return","result":"steps.work.output"}),
        ],
        1,
    );
    document["nodes"][1]["outputExpr"] = json!("steps.lines.output[0]");
    document["outputSchema"] = json!({
        "type":"object",
        "additionalProperties":true,
        "x-sagat-classification":"public",
        "properties":{
            "publicAnswer":{"type":"string","x-sagat-classification":"public"},
            "internalMarker":{"type":"string","x-sagat-classification":"internal"},
            "piiMarker":{"type":"string","x-sagat-classification":"pii"},
            "secretMarker":{"type":"string","x-sagat-classification":"secret"}
        }
    });
    let f = Flow::start(name, document, json!([{"id":1}])).await;
    let (_log_guard, logs) = crate::support::logs::capture_at(tracing::Level::TRACE);

    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    f.complete(
        &tasks[0],
        json!({
            "publicAnswer":PUBLIC_ANSWER,
            "internalMarker":INTERNAL_MARKER,
            "piiMarker":PII_MARKER,
            (UNMARKED_KEY):UNMARKED_MARKER,
            "secretMarker":SECRET_MARKER
        }),
    )
    .await;
    f.drive().await;
    assert_eq!(f.run_status().await, "SUCCEEDED");

    let ordinary = f
        .api
        .as_user(
            reqwest::Method::GET,
            &format!("/runs/{}", f.run),
            "alice",
            "acme",
        )
        .send()
        .await
        .expect("read ordinary run resource");
    assert_eq!(ordinary.status(), reqwest::StatusCode::OK);
    let ordinary_wire = ordinary.text().await.expect("ordinary run body");
    for marker in [
        PUBLIC_ANSWER,
        INTERNAL_MARKER,
        PII_MARKER,
        UNMARKED_KEY,
        UNMARKED_MARKER,
        SECRET_MARKER,
    ] {
        assert!(
            !ordinary_wire.contains(marker),
            "ordinary run response exposed collection result marker {marker}"
        );
    }

    let response = f
        .api
        .as_user(
            reqwest::Method::POST,
            &format!("/runs/{}/payload-disclosures", f.run),
            "alice",
            "acme",
        )
        .json(&json!({"source":"result"}))
        .send()
        .await
        .expect("disclose actual collection Run result");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("private, no-store")
    );
    let disclosure_wire = response.text().await.expect("result disclosure body");
    let disclosure: Value =
        serde_json::from_str(&disclosure_wire).expect("classified result response");
    assert_eq!(disclosure["selection"]["source"], "result");
    assert_eq!(disclosure["policy"], "classified-v1");
    assert_eq!(disclosure["caller"]["revealPii"], "not_granted");
    assert_eq!(disclosure["caller"]["revealed"], false);
    assert_eq!(disclosure["kind"], "disclosed");
    assert_eq!(disclosure["schema"]["carrier"], "outputSchema");
    assert_eq!(disclosure["schema"]["versionNumber"], 1);
    assert_eq!(disclosure["schema"]["rules"], 1);

    let children = disclosure["tree"]["children"]
        .as_array()
        .expect("classified result children");
    let child = |name: &str| {
        children
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("classified result child {name}"))
    };
    assert_eq!(child("publicAnswer")["node"]["kind"], "value");
    assert_eq!(child("publicAnswer")["node"]["level"], "public");
    assert_eq!(child("publicAnswer")["node"]["value"], PUBLIC_ANSWER);
    assert_eq!(child("internalMarker")["node"]["kind"], "value");
    assert_eq!(child("internalMarker")["node"]["value"], INTERNAL_MARKER);
    assert_eq!(child("piiMarker")["node"]["kind"], "withheld");
    assert_eq!(child("piiMarker")["node"]["reason"], "LEVEL_NOT_PERMITTED");
    assert_eq!(child("secretMarker")["node"]["kind"], "withheld");
    assert_eq!(child("secretMarker")["node"]["reason"], "SECRET");
    for marker in [PII_MARKER, UNMARKED_KEY, UNMARKED_MARKER, SECRET_MARKER] {
        assert!(
            !disclosure_wire.contains(marker),
            "classified result response exposed private marker {marker}"
        );
    }

    let audit = f
        .api
        .as_user(
            reqwest::Method::GET,
            &format!(
                "/audit?action=payload.disclosed&resourceId={}&actor=alice&limit=20",
                f.run
            ),
            "alice",
            "acme",
        )
        .send()
        .await
        .expect("read disclosure audit through current tenant authorization");
    assert_eq!(audit.status(), reqwest::StatusCode::OK);
    let audit_wire = audit.text().await.expect("audit response body");
    let audit: Value = serde_json::from_str(&audit_wire).expect("audit page");
    let record = audit["items"]
        .as_array()
        .expect("audit items")
        .iter()
        .find(|record| {
            record["action"] == "payload.disclosed"
                && record["resourceId"] == f.run.to_string()
                && record["reason"] == "classified-v1:result"
        })
        .expect("classified collection result disclosure audit");
    assert_eq!(record["actor"], "alice");
    assert_eq!(record["resourceType"], "run");
    assert!(
        record["traceId"]
            .as_str()
            .is_some_and(|trace| !trace.is_empty()),
        "disclosure audit retains trace correlation"
    );
    for marker in [
        PUBLIC_ANSWER,
        "publicAnswer",
        "internalMarker",
        "piiMarker",
        "secretMarker",
        INTERNAL_MARKER,
        PII_MARKER,
        UNMARKED_KEY,
        UNMARKED_MARKER,
        SECRET_MARKER,
    ] {
        assert!(
            !audit_wire.contains(marker),
            "audit response exposed payload marker {marker}"
        );
        assert!(
            !logs.text().contains(marker),
            "structured logs exposed payload marker {marker}"
        );
    }
    f.verify().await;
}
