//! US02 — edit code in a node: a script node is published only after validation (US02-AC01). The
//! sandbox runs of a script (test runs, builds) are feature 005's; what the Studio needs of the
//! server for this story is that the validator tells the author, node by node, when a script node
//! pins an artifact nobody built, before publish refuses it.

use reqwest::Method;
use serde_json::{Value, json};

use crate::chain;
use crate::support::worker::platform;

fn with_script(digest: &str) -> Value {
    let mut ir = chain("scripted-flow", 2);
    ir["nodes"][1] = json!({
        "id": "a1", "type": "script", "language": "typescript", "artifactDigest": digest
    });
    ir
}

#[tokio::test]
async fn us02_a_script_node_whose_artifact_was_never_built_is_reported_on_its_node_and_refused_at_publish()
 {
    let p = platform().await;
    let digest = format!("sha256:{}", "a".repeat(64));
    let (status, created) = p
        .rest(
            Method::POST,
            "/workflows",
            Some(&json!({"name": "scripted-flow", "ir": with_script(&digest)})),
        )
        .await;
    assert_eq!(
        status, 201,
        "a draft with an unbuilt script is a draft: {created}"
    );
    let id = created["id"].as_str().expect("id");

    let (status, validation) = p
        .rest(
            Method::POST,
            &format!("/workflows/{id}/validate"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 200, "{validation}");
    assert_eq!(validation["valid"], false);
    assert_eq!(
        validation["problems"][0]["code"], "UNKNOWN_SCRIPT",
        "{validation}"
    );
    assert_eq!(validation["problems"][0]["nodeId"], "a1");
    assert!(
        !validation.to_string().contains("secret"),
        "no value of the document beyond a name"
    );

    let (status, refused) = p
        .rest(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            Some(&json!({"draftRevision": 1})),
        )
        .await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(refused["details"]["reason"], "UNKNOWN_SCRIPT");
}

#[tokio::test]
async fn us02_a_script_document_that_is_malformed_is_a_schema_problem_on_the_node() {
    let p = platform().await;
    let (_, created) = p
        .rest(
            Method::POST,
            "/workflows",
            Some(&json!({"name": "scripted-flow", "ir": chain("scripted-flow", 2)})),
        )
        .await;
    let id = created["id"].as_str().expect("id");
    let mut bad = with_script("not-a-digest");
    bad["nodes"][1]["language"] = json!("cobol");
    let (_, validation) = p
        .rest(
            Method::POST,
            &format!("/workflows/{id}/validate"),
            Some(&json!({"ir": bad})),
        )
        .await;
    assert_eq!(validation["valid"], false, "{validation}");
    assert!(validation["problems"].as_array().expect("problems").len() >= 1);
}
