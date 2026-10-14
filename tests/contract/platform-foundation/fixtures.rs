//! Static consumer fixtures held to the canonical OpenAPI and AsyncAPI schemas (T001-002).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

fn repository_fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("contract/platform-foundation/fixtures")
        .join(relative)
}

fn load_json(relative: &str) -> Value {
    let path = repository_fixture(relative);
    serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture exists"))
        .unwrap_or_else(|error| panic!("{} is valid JSON: {error}", path.display()))
}

fn load_yaml(relative: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../specs/001-platform-foundation/contracts")
        .join(relative);
    serde_saphyr::from_str(&std::fs::read_to_string(&path).expect("contract exists"))
        .unwrap_or_else(|error| panic!("{} is valid YAML: {error}", path.display()))
}

fn assert_conforms(document: &Value, pointer: &str, instance: &Value, label: &str) {
    let schema = document
        .pointer(pointer)
        .unwrap_or_else(|| panic!("{label}: schema pointer {pointer} exists"));
    let mut root = schema.clone();
    root.as_object_mut()
        .expect("schema is an object")
        .insert("components".into(), document["components"].clone());
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&root)
        .unwrap_or_else(|error| panic!("{label}: contract schema is valid: {error}"));
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| format!("{error} (at {})", error.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{label} does not conform: {}\n{instance}",
        errors.join("; ")
    );
}

#[test]
fn rest_fixtures_conform_to_their_openapi_operations() {
    let document = load_yaml("openapi.yaml");
    let cases = [
        (
            "rest/create-workspace.request.json",
            "/paths/~1workspaces/post/requestBody/content/application~1json/schema",
        ),
        (
            "rest/create-workspace.response.json",
            "/paths/~1workspaces/post/responses/201/content/application~1json/schema",
        ),
        (
            "rest/create-namespace.request.json",
            "/paths/~1workspaces~1{workspaceId}~1namespaces/post/requestBody/content/application~1json/schema",
        ),
        (
            "rest/create-namespace.response.json",
            "/paths/~1workspaces~1{workspaceId}~1namespaces/post/responses/201/content/application~1json/schema",
        ),
        ("rest/error.response.json", "/components/schemas/Error"),
    ];

    for (fixture, schema) in cases {
        assert_conforms(&document, schema, &load_json(fixture), fixture);
    }
}

#[test]
fn event_fixtures_conform_to_their_asyncapi_message() {
    let document = load_yaml("events.asyncapi.yaml");
    assert_conforms(
        &document,
        "/components/messages/TaskReady/headers",
        &load_json("asyncapi/task-ready.headers.json"),
        "TaskReady headers",
    );
    assert_conforms(
        &document,
        "/components/messages/TaskReady/payload",
        &load_json("asyncapi/task-ready.payload.json"),
        "TaskReady payload",
    );
}

#[test]
fn fixture_validation_rejects_breaking_shapes() {
    let openapi = load_yaml("openapi.yaml");
    let invalid_workspace = json!({"slug": "platform", "tenantId": "spoofed"});
    let schema = openapi
        .pointer("/paths/~1workspaces/post/requestBody/content/application~1json/schema")
        .expect("workspace request schema");
    let mut root = schema.clone();
    root.as_object_mut()
        .expect("schema object")
        .insert("components".into(), openapi["components"].clone());
    assert!(
        !jsonschema::validator_for(&root)
            .expect("valid contract schema")
            .is_valid(&invalid_workspace),
        "a request fixture must not be able to select its tenant"
    );

    let asyncapi = load_yaml("events.asyncapi.yaml");
    let missing_tenant = json!({
        "namespaceId": "018f47c0-1111-7000-8000-000000000002",
        "runId": "018f47c0-1111-7000-8000-000000000003",
        "node": "charge",
        "activity": "payments.charge"
    });
    let task_ready = &asyncapi["components"]["messages"]["TaskReady"]["payload"];
    assert!(
        !jsonschema::validator_for(task_ready)
            .expect("valid event schema")
            .is_valid(&missing_tenant),
        "a task-ready fixture must name its tenant"
    );
}
