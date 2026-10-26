#![allow(clippy::expect_used)]

#[path = "../src/carrier_schema_check.rs"]
mod carrier_schema_check;

use serde_json::json;

#[test]
fn annotated_root_and_every_supported_node_output_schema_are_compiled() {
    let invalid = json!({"type":"not-a-json-schema-type"});
    let document = json!({
        "inputSchema": {"type":"object", "x-sagat-classification":"pii"},
        "outputSchema": invalid,
        "nodes":[
            {"id":"activity","type":"activity","outputSchema":invalid},
            {"id":"timer","type":"timer","outputSchema":invalid}
        ],
        "bodies":{"nested":{"nodes":[
            {"id":"script","type":"script","outputSchema":invalid},
            {"id":"control","type":"condition","outputSchema":invalid}
        ]}}
    });

    let issues = carrier_schema_check::issues(&document);
    let paths: Vec<_> = issues.iter().map(|issue| issue.path.as_str()).collect();
    assert!(paths.contains(&"/nodes/0/outputSchema"));
    assert!(paths.contains(&"/bodies/nested/nodes/0/outputSchema"));
    assert!(!paths.contains(&"/outputSchema"), "unannotated legacy root");
    assert!(
        !paths.contains(&"/nodes/1/outputSchema"),
        "timer is not a carrier"
    );
    assert!(!paths.contains(&"/bodies/nested/nodes/1/outputSchema"));
}

#[test]
fn annotated_root_refs_are_compiled_without_network_access_or_value_diagnostics() {
    let document = json!({"inputSchema":{
        "x-sagat-classification":"pii",
        "$ref":"https://schema-canary.invalid/private"
    }});
    let issues = carrier_schema_check::issues(&document);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, "/inputSchema");
    assert!(!issues[0].message.contains("schema-canary"));
}
