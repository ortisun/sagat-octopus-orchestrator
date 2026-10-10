//! US01 — create a workflow without programming: the validator displays errors and the IR draft is
//! saved (US01-AC01), a draft is compared with the version it came from before it is published
//! (FR-007-004), and two people editing the same draft cannot overwrite each other (edge case).

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use crate::chain;
use crate::support::worker::{Platform, platform};

async fn create(p: &Platform, name: &str, ir: &Value) -> String {
    let (status, created) = p
        .rest(
            Method::POST,
            "/workflows",
            Some(&json!({"name": name, "ir": ir})),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    created["id"].as_str().expect("id").to_owned()
}

async fn validate(p: &Platform, id: &str, body: &Value) -> Value {
    let (status, validation) = p
        .rest(
            Method::POST,
            &format!("/workflows/{id}/validate"),
            Some(body),
        )
        .await;
    assert_eq!(
        status, 200,
        "validation is an answer, not an error: {validation}"
    );
    validation
}

fn codes(validation: &Value, list: &str) -> Vec<String> {
    validation[list]
        .as_array()
        .expect("list")
        .iter()
        .map(|p| p["code"].as_str().expect("code").to_owned())
        .collect()
}

#[tokio::test]
async fn us01_an_invalid_draft_is_saved_validated_with_every_problem_and_refused_at_publish() {
    let p = platform().await;
    let id = create(&p, "order-flow", &chain("order-flow", 3)).await;

    // The author is mid-edit: a node with no activity and an unknown type. The draft is saved.
    let mut broken = chain("order-flow", 3);
    broken["nodes"][1]
        .as_object_mut()
        .expect("node")
        .remove("activity");
    broken["nodes"][2]["type"] = json!("teleport");
    let (status, saved) = p
        .rest(
            Method::PUT,
            &format!("/workflows/{id}"),
            Some(&json!({"draftRevision": 1, "ir": broken})),
        )
        .await;
    assert_eq!(
        status, 200,
        "a draft is saved even when it does not validate: {saved}"
    );
    assert_eq!(saved["draftRevision"], 2);

    // The validator lists the problems of the stored draft, each with a path into the document.
    let validation = validate(&p, &id, &json!({})).await;
    assert_eq!(validation["valid"], false, "{validation}");
    assert_eq!(validation["digest"], Value::Null);
    let problems = validation["problems"].as_array().expect("problems");
    assert!(!problems.is_empty(), "{validation}");
    assert!(
        problems
            .iter()
            .all(|p| p["severity"] == "error" && p["code"] == "SCHEMA")
    );
    let paths: Vec<&str> = problems.iter().filter_map(|p| p["path"].as_str()).collect();
    assert!(
        paths.iter().any(|p| p.starts_with("/nodes/1"))
            && paths.iter().any(|p| p.starts_with("/nodes/2")),
        "the problems point at the nodes the author broke: {paths:?}"
    );

    // Publishing is what is gated: the same document is refused with the same story.
    let (status, refused) = p
        .rest(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            Some(&json!({"draftRevision": 2})),
        )
        .await;
    assert_eq!(status, 422, "{refused}");

    // The author fixes it; the validator says so and its digest is the one the version gets.
    let (status, fixed) = p
        .rest(
            Method::PUT,
            &format!("/workflows/{id}"),
            Some(&json!({"draftRevision": 2, "ir": chain("order-flow", 3)})),
        )
        .await;
    assert_eq!(status, 200, "{fixed}");
    let validation = validate(&p, &id, &json!({})).await;
    assert_eq!(validation["valid"], true, "{validation}");
    let digest = validation["digest"].as_str().expect("digest").to_owned();
    let (status, version) = p
        .rest(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            Some(&json!({"draftRevision": 3})),
        )
        .await;
    assert_eq!(status, 201, "{version}");
    assert_eq!(version["digest"], digest, "validate and publish agree");
}

#[tokio::test]
async fn us01_a_graph_rule_names_the_node_it_is_about() {
    let p = platform().await;
    let id = create(&p, "order-flow", &chain("order-flow", 3)).await;

    let mut cyclic = chain("order-flow", 3);
    cyclic["nodes"][2]["next"] = json!("a0");
    let validation = validate(&p, &id, &json!({"ir": cyclic})).await;
    assert_eq!(validation["valid"], false);
    let problem = &validation["problems"][0];
    assert_eq!(problem["code"], "CYCLE", "{validation}");
    assert!(problem["nodeId"].is_string(), "{validation}");

    let mut orphan = chain("order-flow", 3);
    orphan["nodes"]
        .as_array_mut()
        .expect("nodes")
        .push(json!({"id": "lonely", "type": "activity", "activity": "act.x"}));
    let validation = validate(&p, &id, &json!({"ir": orphan})).await;
    assert_eq!(
        codes(&validation, "problems"),
        ["UNREACHABLE"],
        "{validation}"
    );
    assert_eq!(validation["problems"][0]["nodeId"], "lonely");

    let mut dangling = chain("order-flow", 3);
    dangling["nodes"][0]["next"] = json!("nowhere");
    let validation = validate(&p, &id, &json!({"ir": dangling})).await;
    assert_eq!(
        codes(&validation, "problems"),
        ["UNKNOWN_TARGET"],
        "{validation}"
    );
    assert_eq!(validation["problems"][0]["nodeId"], "a0");

    let mut human = chain("order-flow", 3);
    human["nodes"][1] = json!({"id": "a1", "type": "humanApproval", "next": "a2"});
    let validation = validate(&p, &id, &json!({"ir": human})).await;
    // Whatever the schema says about the form of the node, the engine does not run it yet: the
    // problem is reported on the document, not by a crash.
    assert_eq!(validation["valid"], false, "{validation}");
}

#[tokio::test]
async fn us01_validating_is_a_dry_run_that_changes_nothing() {
    let p = platform().await;
    let id = create(&p, "order-flow", &chain("order-flow", 3)).await;
    let (_, before) = p.rest(Method::GET, &format!("/workflows/{id}"), None).await;

    let mut other = chain("order-flow", 3);
    other["nodes"][0]["activity"] = json!("act.changed");
    let validation = validate(&p, &id, &json!({"ir": other})).await;
    assert_eq!(validation["valid"], true, "{validation}");

    let (_, after) = p.rest(Method::GET, &format!("/workflows/{id}"), None).await;
    assert_eq!(
        after, before,
        "the draft, its revision and its timestamps did not move"
    );
    let (_, versions) = p
        .rest(Method::GET, &format!("/workflows/{id}/versions"), None)
        .await;
    assert_eq!(versions["items"], json!([]), "nothing was published");

    // A document that names another workflow does not validate for this one.
    let mut renamed = chain("order-flow", 3);
    renamed["metadata"]["name"] = json!("another-flow");
    let validation = validate(&p, &id, &json!({"ir": renamed})).await;
    assert_eq!(
        codes(&validation, "problems"),
        ["NAME_MISMATCH"],
        "{validation}"
    );
}

#[tokio::test]
async fn us01_validating_needs_write_access_and_never_crosses_a_tenant() {
    let p = platform().await;
    let id = create(&p, "order-flow", &chain("order-flow", 3)).await;
    p.grant(p.tenant, "vera", Role::Viewer).await;
    p.grant(p.tenant, "dev", Role::Developer).await;

    let (status, body) = p
        .rest_as(
            "vera",
            Method::POST,
            &format!("/workflows/{id}/validate"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 403, "a viewer reads, an author writes: {body}");
    let (status, body) = p
        .rest_as(
            "dev",
            Method::POST,
            &format!("/workflows/{id}/validate"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // Another tenant's workflow does not exist for us, whoever we are.
    let response = p
        .api
        .as_user(
            Method::POST,
            &format!("/workflows/{id}/validate"),
            "bob",
            "globex",
        )
        .header("x-octopus-namespace", "payments")
        .json(&json!({}))
        .send()
        .await
        .expect("send");
    assert!(
        [403, 404].contains(&response.status().as_u16()),
        "{}",
        response.status()
    );
    let unauthenticated = p
        .api
        .request(Method::POST, &format!("/workflows/{id}/validate"))
        .json(&json!({}))
        .send()
        .await
        .expect("send");
    assert_eq!(unauthenticated.status(), 401);

    // The body is strict: an unknown field is an error, not silently ignored.
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/workflows/{id}/validate"),
            Some(&json!({"ir": chain("order-flow", 3), "persist": true})),
        )
        .await;
    assert_eq!(status, 422, "{body}");
}

#[tokio::test]
async fn us01_the_draft_is_compared_with_the_version_it_came_from_and_a_contract_break_is_named() {
    let p = platform().await;
    let mut v1 = chain("order-flow", 3);
    v1["inputSchema"] = json!({
        "type": "object",
        "required": ["orderId"],
        "properties": {"orderId": {"type": "string"}, "note": {"type": "string"}}
    });
    v1["triggers"] = json!([{"kind": "manual"}]);
    let id = create(&p, "order-flow", &v1).await;

    // No version yet: the draft is compared with nothing.
    let (status, first) = p
        .rest(Method::GET, &format!("/workflows/{id}/diff"), None)
        .await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["from"]["kind"], "empty");
    assert_eq!(first["to"]["kind"], "draft");
    assert_eq!(first["breaking"], json!([]));
    assert_eq!(first["nodes"]["added"], json!(["a0", "a1", "a2"]));

    let (status, published) = p
        .rest(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            Some(&json!({"draftRevision": 1})),
        )
        .await;
    assert_eq!(status, 201, "{published}");

    // The draft is identical to the active version until somebody edits it.
    let (_, same) = p
        .rest(Method::GET, &format!("/workflows/{id}/diff"), None)
        .await;
    assert_eq!(same["identical"], true, "{same}");
    assert_eq!(
        same["from"],
        json!({"kind": "version", "number": 1, "revision": null})
    );

    // The author drops a property, requires another, and points a node at a new activity.
    let mut v2 = v1.clone();
    v2["inputSchema"]["properties"]
        .as_object_mut()
        .expect("props")
        .remove("note");
    v2["inputSchema"]["required"] = json!(["orderId", "customer"]);
    v2["inputSchema"]["properties"]["customer"] = json!({"type": "string"});
    v2["nodes"][1]["activity"] = json!("act.changed");
    let (status, saved) = p
        .rest(
            Method::PUT,
            &format!("/workflows/{id}"),
            Some(&json!({"draftRevision": 1, "ir": v2})),
        )
        .await;
    assert_eq!(status, 200, "{saved}");

    let (status, diff) = p
        .rest(Method::GET, &format!("/workflows/{id}/diff"), None)
        .await;
    assert_eq!(status, 200, "{diff}");
    assert_eq!(diff["identical"], false);
    assert_eq!(
        diff["to"],
        json!({"kind": "draft", "number": null, "revision": 2})
    );
    assert_eq!(
        codes(&diff, "breaking"),
        ["INPUT_PROPERTY_REMOVED", "INPUT_REQUIRED_ADDED"],
        "{diff}"
    );
    assert_eq!(codes(&diff, "attention"), ["ACTIVITY_CHANGED"], "{diff}");
    assert_eq!(
        diff["nodes"]["changed"],
        json!([{"id": "a1", "fields": ["activity"]}])
    );

    // Publish it, and compare the two versions by number.
    let (status, _) = p
        .rest(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            Some(&json!({"draftRevision": 2})),
        )
        .await;
    assert_eq!(status, 201);
    let (status, versions) = p
        .rest(
            Method::GET,
            &format!("/workflows/{id}/diff?from=1&to=2"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{versions}");
    assert_eq!(
        codes(&versions, "breaking"),
        ["INPUT_PROPERTY_REMOVED", "INPUT_REQUIRED_ADDED"]
    );
    let (_, backwards) = p
        .rest(
            Method::GET,
            &format!("/workflows/{id}/diff?from=2&to=1"),
            None,
        )
        .await;
    // Going back adds nothing to the input and drops `customer`, which version 2 described.
    assert_eq!(
        codes(&backwards, "breaking"),
        ["INPUT_PROPERTY_REMOVED"],
        "{backwards}"
    );
}

#[tokio::test]
async fn us01_a_diff_asks_for_documents_that_exist_and_names_them_correctly() {
    let p = platform().await;
    let id = p.publish("order-flow", &chain("order-flow", 2)).await;
    for bad in [
        "from=latest",
        "to=0",
        "from=-1",
        "to=1.5",
        "from=%20",
        "to=99999999999",
    ] {
        let (status, body) = p
            .rest(Method::GET, &format!("/workflows/{id}/diff?{bad}"), None)
            .await;
        assert_eq!(status, 400, "{bad}: {body}");
    }
    let (status, body) = p
        .rest(Method::GET, &format!("/workflows/{id}/diff?from=7"), None)
        .await;
    assert_eq!(status, 404, "a version that was never published: {body}");
    let (status, _) = p
        .rest(
            Method::GET,
            "/workflows/019b5d6e-0000-7000-8000-000000000000/diff",
            None,
        )
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn us01_two_people_editing_the_same_draft_cannot_overwrite_each_other() {
    let p = platform().await;
    let id = create(&p, "order-flow", &chain("order-flow", 2)).await;
    p.grant(p.tenant, "dev", Role::Developer).await;

    // Both opened revision 1. Alice saves first.
    let mut alices = chain("order-flow", 2);
    alices["nodes"][0]["activity"] = json!("act.alice");
    let (status, saved) = p
        .rest(
            Method::PUT,
            &format!("/workflows/{id}"),
            Some(&json!({"draftRevision": 1, "ir": alices})),
        )
        .await;
    assert_eq!(status, 200, "{saved}");

    // The developer saves what they edited from revision 1: refused, with the revision to start from.
    let mut devs = chain("order-flow", 2);
    devs["nodes"][0]["activity"] = json!("act.dev");
    let (status, conflict) = p
        .rest_as(
            "dev",
            Method::PUT,
            &format!("/workflows/{id}"),
            Some(&json!({"draftRevision": 1, "ir": devs})),
        )
        .await;
    assert_eq!(status, 409, "{conflict}");

    // Nothing of the developer's reached the draft; the diff against the server's copy is what
    // the Studio shows before anybody saves over.
    let (_, current) = p.rest(Method::GET, &format!("/workflows/{id}"), None).await;
    assert_eq!(current["ir"]["nodes"][0]["activity"], "act.alice");
    assert_eq!(current["draftRevision"], 2);
    let (status, validation) = p
        .rest_as(
            "dev",
            Method::POST,
            &format!("/workflows/{id}/validate"),
            Some(&json!({"ir": devs})),
        )
        .await;
    assert_eq!(status, 200, "{validation}");
    assert_eq!(
        validation["valid"], true,
        "their copy is still theirs to keep or to save as a copy"
    );
}
