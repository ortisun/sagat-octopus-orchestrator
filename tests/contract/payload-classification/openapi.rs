//! Canonical OpenAPI additions for disclosure, grants, validation and event reads (T016-007/T016-008/T016-060).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn document() -> Value {
    let path =
        super::repository_root().join("specs/001-platform-foundation/contracts/openapi.yaml");
    serde_saphyr::from_str(&fs::read_to_string(path).expect("canonical OpenAPI exists"))
        .expect("canonical OpenAPI is YAML")
}

fn operation<'a>(openapi: &'a Value, path: &str, method: &str) -> &'a Value {
    &openapi["paths"][path][method]
}

fn validator(openapi: &Value, schema: &str) -> jsonschema::Validator {
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

fn assert_valid(openapi: &Value, schema: &str, instance: Value) {
    let errors = validator(openapi, schema)
        .iter_errors(&instance)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{schema}: {errors:?}\n{instance}");
}

#[test]
fn the_four_feature_operations_are_canonical_and_the_disclosure_is_not_cacheable() {
    let openapi = document();
    for (path, method, id) in [
        (
            "/runs/{runId}/payload-disclosures",
            "post",
            "discloseRunPayload",
        ),
        (
            "/namespaces/{namespaceId}/payload-reveal-grants",
            "get",
            "listPayloadRevealGrants",
        ),
        (
            "/namespaces/{namespaceId}/payload-reveal-grants/{subject}",
            "put",
            "grantPayloadReveal",
        ),
        (
            "/namespaces/{namespaceId}/payload-reveal-grants/{subject}",
            "delete",
            "revokePayloadReveal",
        ),
    ] {
        let operation = operation(&openapi, path, method);
        assert_eq!(operation["operationId"], id);
        assert_eq!(operation["x-implemented"], "016");
    }
    assert_eq!(
        operation(&openapi, "/runs/{runId}/payload-disclosures", "post")["responses"]["200"]["headers"]
            ["Cache-Control"]["schema"]["const"],
        "private, no-store"
    );
}

#[test]
fn disclosure_and_grant_examples_conform_to_closed_schemas() {
    let openapi = document();
    for selector in [
        json!({"source": "input"}),
        json!({"source": "result"}),
        json!({"source": "journal", "sequence": 42}),
        json!({"source": "input", "reveal": ["pii"]}),
    ] {
        assert_valid(&openapi, "PayloadDisclosureRequest", selector);
    }

    assert_valid(
        &openapi,
        "PayloadDisclosure",
        json!({
            "runId": "018f47c0-1111-7000-8000-000000000003",
            "runVersion": 7,
            "selection": {"source": "input"},
            "policy": "classified-v1",
            "caller": {"disclosedLevels": ["public", "internal"], "revealPii": "not_granted", "revealed": false},
            "kind": "disclosed",
            "schema": {"carrier": "inputSchema", "versionNumber": 3, "rules": 1},
            "sizeBytes": 27,
            "truncated": false,
            "tree": {"kind": "value", "level": "public", "type": "string", "value": "ready"}
        }),
    );
    assert_valid(
        &openapi,
        "PayloadRevealGrantPage",
        json!({
            "items": [{
                "subject": "user:operator@example.test",
                "grantedBy": "user:admin@example.test",
                "grantedAt": "2026-10-17T12:00:00Z"
            }],
            "nextCursor": null
        }),
    );

    for invalid in [
        json!({"source": "input", "unknown": true}),
        json!({"source": "journal"}),
        json!({"source": "input", "sequence": 1}),
        json!({"source": "input", "reveal": ["secret"]}),
        json!({"source": "input", "reveal": ["pii", "pii"]}),
    ] {
        assert!(
            !validator(&openapi, "PayloadDisclosureRequest").is_valid(&invalid),
            "request schema accepted {invalid}"
        );
    }
}

#[test]
fn classification_codes_and_error_reasons_are_closed_and_documented() {
    let openapi = document();
    let error_codes = openapi["components"]["schemas"]["Error"]["properties"]["code"]["enum"]
        .as_array()
        .expect("Error.code enum")
        .iter()
        .map(|value| value.as_str().expect("code is text"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        error_codes,
        [
            "BAD_REQUEST",
            "UNAUTHENTICATED",
            "FORBIDDEN",
            "NOT_FOUND",
            "CONFLICT",
            "VALIDATION_FAILED",
            "RATE_LIMITED",
            "QUOTA_EXCEEDED",
            "UNAVAILABLE",
            "INTERNAL"
        ]
        .into_iter()
        .collect()
    );

    let definition =
        openapi["components"]["schemas"]["DefinitionProblem"]["properties"]["code"]["description"]
            .as_str()
            .expect("problem code description");
    for code in [
        "CLASSIFICATION_INVALID_LEVEL",
        "CLASSIFICATION_LOOSENED",
        "CLASSIFICATION_UNSUPPORTED_KEYWORD",
        "CLASSIFICATION_UNRESOLVED_REF",
        "CLASSIFICATION_NOT_APPLICABLE",
        "CLASSIFICATION_TOO_COMPLEX",
        "CLASSIFICATION_INHERITED_OPEN",
    ] {
        assert!(definition.contains(code), "missing {code}");
    }

    let diff = openapi["components"]["schemas"]["DiffFinding"]["properties"]["code"]["description"]
        .as_str()
        .expect("diff code description");
    for code in ["CLASSIFICATION_RELAXED", "CLASSIFICATION_TIGHTENED"] {
        assert!(diff.contains(code), "missing {code}");
    }

    let reasons = openapi["components"]["schemas"]["Error"]["properties"]["details"]["description"]
        .as_str()
        .expect("reason description");
    for reason in [
        "PII_REVEAL_NOT_GRANTED",
        "CLASSIFICATION",
        "UNKNOWN_SUBJECT",
        "ROLE_CANNOT_REVEAL",
        "GRANT_LIMIT",
    ] {
        assert!(reasons.contains(reason), "missing {reason}");
    }
}

#[test]
fn validation_report_is_bounded_and_uses_the_four_levels() {
    let openapi = document();
    let validation = &openapi["components"]["schemas"]["Validation"];
    assert_eq!(validation["properties"]["classification"]["maxItems"], 500);
    assert_eq!(
        openapi["components"]["schemas"]["ClassificationReportEntry"]["properties"]["level"]["$ref"],
        "#/components/schemas/ClassificationLevel"
    );
    assert_eq!(
        openapi["components"]["schemas"]["ClassificationLevel"]["enum"],
        json!(["public", "internal", "pii", "secret"])
    );
    assert_valid(
        &openapi,
        "Validation",
        json!({
            "valid": true,
            "problems": [],
            "warnings": [],
            "classification": [{
                "carrier": "inputSchema", "path": "/orderId", "level": "public", "declared": false
            }],
            "classificationTruncated": false
        }),
    );
}

#[test]
fn event_detail_defaults_to_structure_and_dead_letter_lists_never_offer_reveal() {
    let openapi = document();
    for (path, id) in [
        (
            "/namespaces/{namespaceId}/events/{source}/{eventId}",
            "getInboxEvent",
        ),
        (
            "/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters/{deadLetterId}",
            "getConnectorDeadLetter",
        ),
        (
            "/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters",
            "listConnectorDeadLetters",
        ),
    ] {
        let op = operation(&openapi, path, "get");
        assert_eq!(op["operationId"], id);
        assert_eq!(op["x-extended"], "016");
        assert_eq!(
            op["responses"]["200"]["headers"]["Cache-Control"]["schema"]["const"],
            "private, no-store"
        );
    }

    let detail_parameters = operation(
        &openapi,
        "/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters/{deadLetterId}",
        "get",
    )["parameters"]
        .as_array()
        .expect("detail parameters");
    assert!(
        detail_parameters
            .iter()
            .any(|parameter| parameter["name"] == "reveal")
    );

    let list_parameters = operation(
        &openapi,
        "/namespaces/{namespaceId}/connectors/{connectorId}/dead-letters",
        "get",
    )["parameters"]
        .as_array()
        .expect("list parameters");
    assert!(
        !list_parameters
            .iter()
            .any(|parameter| parameter["name"] == "reveal")
    );

    let dead_letter_required = openapi["components"]["schemas"]["DeadLetter"]["required"]
        .as_array()
        .expect("dead letter required")
        .iter()
        .map(|value| value.as_str().expect("required field"))
        .collect::<BTreeSet<_>>();
    assert!(!dead_letter_required.contains("body"));
    assert!(!dead_letter_required.contains("headers"));
    assert!(dead_letter_required.contains("bodyDisclosure"));
    assert!(dead_letter_required.contains("bodySizeBytes"));
    assert!(dead_letter_required.contains("headerCount"));

    assert_valid(
        &openapi,
        "InboxEvent",
        json!({
            "eventId": "order-42",
            "source": "api",
            "type": "order.ready",
            "receivedAt": "2026-10-17T12:00:00Z",
            "expiresAt": "2026-10-18T12:00:00Z",
            "status": "PENDING",
            "attempts": 0,
            "deliveries": [],
            "dataDisclosure": "structure-only",
            "dataSizeBytes": 27,
            "dataStructure": {"type": "object", "childCount": 1, "omittedChildren": 0, "children": []}
        }),
    );
    let leaked_event = json!({
        "eventId": "order-42",
        "source": "api",
        "type": "order.ready",
        "receivedAt": "2026-10-17T12:00:00Z",
        "expiresAt": "2026-10-18T12:00:00Z",
        "status": "PENDING",
        "attempts": 0,
        "deliveries": [],
        "dataDisclosure": "structure-only",
        "dataSizeBytes": 27,
        "dataStructure": {"type": "object", "childCount": 1, "omittedChildren": 0, "children": []},
        "data": {"secret": "must not be present by default"}
    });
    assert!(
        !validator(&openapi, "InboxEvent").is_valid(&leaked_event),
        "a structure-only event must reject data"
    );

    let summary_with_values = json!({
        "id": "018f47c0-1111-7000-8000-000000000004",
        "position": "stream/42",
        "reason": "BAD_ENVELOPE",
        "truncated": false,
        "receivedAt": "2026-10-17T12:00:00Z",
        "bodyDisclosure": "structure-only",
        "bodySizeBytes": 12,
        "headerCount": 1,
        "bodyStructure": {"kind": "opaque"},
        "body": "must never appear in a list",
        "headers": {"authorization": "must never appear in a list"}
    });
    assert!(
        !validator(&openapi, "DeadLetterSummary").is_valid(&summary_with_values),
        "dead-letter list items must reject body and headers"
    );
}

#[test]
fn operations_outside_the_approved_additions_and_event_extensions_are_byte_stable() {
    let openapi = document();
    let mut stable = BTreeMap::new();
    for (path, item) in openapi["paths"].as_object().expect("paths") {
        for method in ["get", "post", "put", "patch", "delete"] {
            let operation = &item[method];
            if operation.is_null()
                || operation["x-implemented"] == "016"
                || operation["x-extended"] == "016"
                // Feature 017 is merged independently with this branch. Its additive operations
                // were not part of the T016-007 baseline and must not create a false regression.
                || operation["x-implemented"] == "017"
            {
                continue;
            }
            stable.insert(format!("{method} {path}"), operation.clone());
        }
    }
    let digest = Sha256::digest(serde_json::to_vec(&stable).expect("stable paths"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    // Approved 014 dependency 112f1e1d appends the pagination guidance to getChain
    // (the only operation diff from bc6b74c0 to 5f4f1c6b). Keep every operation in
    // this digest; the baseline update does not exclude the changed operation.
    assert_eq!(
        digest, "60c58b37add3b597a3d94a25e000f25b04d6dd3505e076ed283eaf192a8ed614",
        "an existing operation outside feature 016 changed"
    );
}
