//! US03 — inline C#. Not built in this delivery: Roslyn and an isolated .NET host are the largest
//! piece of the feature and nothing in the other languages' path depends on them. What is proven
//! here is that the platform says so, in a stable way, instead of accepting a script it cannot run
//! (see the gap recorded in `specs/005-inline-code-sandbox/research.md`).

use reqwest::Method;
use serde_json::json;

use crate::support::scripts::lab;

#[tokio::test]
async fn a_csharp_script_is_refused_with_a_reason_a_client_can_branch_on() {
    let lab = lab().await;
    for language in ["csharp", "go", "java"] {
        let (status, body) = lab
            .call(
                Method::POST,
                &format!("/namespaces/{}/scripts", lab.ns),
                Some(&json!({"name": "later", "language": language, "source": "x"})),
            )
            .await;
        assert_eq!(status, 422, "{language}: {body}");
        assert_eq!(
            body["details"]["reason"], "LANGUAGE_NOT_BUILT",
            "{language}: {body}"
        );
    }
    // The languages that exist are named by the workflow schema, so a document that says
    // `csharp` for a script node is still a document; what it pins has to be built.
    let (status, body) = lab
        .call(
            Method::POST,
            &format!("/namespaces/{}/scripts", lab.ns),
            Some(&json!({"name": "nonsense", "language": "cobol", "source": "x"})),
        )
        .await;
    assert_eq!(status, 422, "{body}");
}
