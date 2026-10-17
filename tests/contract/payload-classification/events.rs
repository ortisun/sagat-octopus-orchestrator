use std::fs;

use serde_json::Value;

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
