//! The event contracts of design 044 (Technical Plan 7.1 and 15, Task 044-2): the in-app alert kind
//! `WORKFLOW_TASK_UNSERVED` wherever the alert kinds are a contract, and the hint that announces a due workflow task.

use serde_json::Value;

const KIND: &str = "WORKFLOW_TASK_UNSERVED";
/// A kind every alert enumeration has had since feature 008, so an enumeration of alert kinds is found by it.
const OLD_KIND: &str = "STEP_UNCLAIMED";

fn text(relative: &str) -> String {
    let path = super::repository_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn yaml(relative: &str) -> Value {
    serde_saphyr::from_str(&text(relative))
        .unwrap_or_else(|error| panic!("{relative} is YAML: {error}"))
}

fn json(relative: &str) -> Value {
    serde_json::from_str(&text(relative))
        .unwrap_or_else(|error| panic!("{relative} is JSON: {error}"))
}

/// Every `enum` (or, in the baseline, every entry's `enum`) of the document that lists an alert kind.
fn alert_kind_enums(value: &Value, found: &mut Vec<Vec<String>>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(values)) = map.get("enum") {
                let names: Vec<String> = values
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                if names.iter().any(|name| name == OLD_KIND) {
                    found.push(names);
                }
            }
            for child in map.values() {
                alert_kind_enums(child, found);
            }
        }
        Value::Array(items) => {
            for child in items {
                alert_kind_enums(child, found);
            }
        }
        _ => {}
    }
}

fn assert_every_alert_enum_has_the_kind(document: &Value, name: &str) {
    let mut found = Vec::new();
    alert_kind_enums(document, &mut found);
    assert!(!found.is_empty(), "{name} lists the alert kinds somewhere");
    for names in found {
        assert!(
            names.iter().any(|kind| kind == KIND),
            "{name}: an enumeration of alert kinds lacks {KIND}: {names:?}"
        );
    }
}

const OPENAPI: &str = "specs/001-platform-foundation/contracts/openapi.yaml";
const ASYNCAPI: &str = "specs/001-platform-foundation/contracts/events.asyncapi.yaml";
const BASELINE: &str = "specs/001-platform-foundation/contracts/events.baseline.json";

#[test]
fn the_alert_kind_workflow_task_unserved_is_in_every_contract() {
    let openapi = yaml(OPENAPI);
    let alert = &openapi["components"]["schemas"]["Alert"]["properties"];
    assert!(
        alert["kind"]["enum"]
            .as_array()
            .expect("Alert.kind is an enumeration")
            .iter()
            .any(|kind| kind == KIND),
        "Alert.kind has {KIND}"
    );
    let subject = alert["subject"]["description"]
        .as_str()
        .expect("Alert.subject says what it holds per kind");
    assert!(
        subject.contains(KIND) && subject.contains("workflow type"),
        "Alert.subject says that the subject of {KIND} is the workflow type"
    );
    assert_every_alert_enum_has_the_kind(&openapi, "openapi.yaml");

    let asyncapi = yaml(ASYNCAPI);
    let webhook = &asyncapi["components"]["messages"]["OperationalAlertWebhook"]["payload"]["properties"]
        ["data"]["properties"]["kind"]["enum"];
    assert!(
        webhook
            .as_array()
            .expect("the alert delivery names its kinds")
            .iter()
            .any(|kind| kind == KIND),
        "OperationalAlertWebhook data.kind has {KIND}"
    );
    assert_every_alert_enum_has_the_kind(&asyncapi, "events.asyncapi.yaml");

    // The baseline records the grown enumeration, so the kind can never be removed again.
    let baseline = json(BASELINE);
    let recorded =
        &baseline["OperationalAlertWebhook"]["payload"]["properties"]["data.kind"]["enum"];
    assert!(
        recorded
            .as_array()
            .expect("the baseline records the alert kinds")
            .iter()
            .any(|kind| kind == KIND),
        "events.baseline.json records {KIND}"
    );
    assert_every_alert_enum_has_the_kind(&baseline, "events.baseline.json");
}

/// The workflow task hint is a `TaskReady` on the same subject with the reserved node and activity name: no member a
/// `TaskReady` consumer does not know, nothing more required, and no payload.
#[test]
fn the_workflow_task_hint_is_a_task_ready_without_a_payload() {
    let asyncapi = yaml(ASYNCAPI);
    let messages = &asyncapi["components"]["messages"];
    let hint = &messages["WorkflowTaskReady"];
    assert!(
        hint.is_object(),
        "events.asyncapi.yaml declares WorkflowTaskReady"
    );
    assert_eq!(
        asyncapi["channels"]["taskReady"]["messages"]["workflowTaskReady"]["$ref"],
        "#/components/messages/WorkflowTaskReady",
        "the hint is published on the taskReady channel"
    );
    let task = &messages["TaskReady"]["payload"];
    let payload = &hint["payload"];
    assert_eq!(
        payload["required"], task["required"],
        "the same required members as TaskReady"
    );
    let known = task["properties"].as_object().expect("TaskReady members");
    for (member, shape) in payload["properties"]
        .as_object()
        .expect("WorkflowTaskReady members")
    {
        assert!(
            known.contains_key(member),
            "WorkflowTaskReady carries `{member}`, which TaskReady does not"
        );
        assert_eq!(
            shape["type"], known[member]["type"],
            "`{member}` has TaskReady's type"
        );
    }
    assert_eq!(payload["properties"]["node"]["const"], "~workflow");
    let pattern = payload["properties"]["activity"]["pattern"]
        .as_str()
        .expect("the reserved activity name has a pattern");
    assert!(
        pattern.starts_with("^sagat\\.workflow"),
        "the reserved name: {pattern}"
    );
    let description = hint["description"]
        .as_str()
        .expect("the hint describes itself");
    assert!(
        description.contains("no payload"),
        "the hint carries no payload"
    );

    let baseline = json(BASELINE);
    assert!(
        baseline["WorkflowTaskReady"]["payload"]["required"].is_array(),
        "events.baseline.json records WorkflowTaskReady"
    );
}
