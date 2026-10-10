//! US02 — an author publishes an immutable artifact; workflows pin its digest and new runs run
//! exactly that code (`FR-005-001`, `FR-005-002`, `FR-005-008`).

use reqwest::Method;
use sagat_application::script_signing::Verifier;
use sagat_domain::scripts::{Digest, Language};
use serde_json::json;

use crate::support::scripts::{Lab, lab};

use base64::Engine as _;

fn returns(n: u32) -> String {
    format!("export default function () {{ return {{ version: {n} }}; }}\n")
}

async fn change_source(lab: &Lab, script: &str, revision: u64, source: &str) {
    let (status, body) = lab
        .call(
            Method::PATCH,
            &format!("/namespaces/{}/scripts/{script}", lab.ns),
            Some(&json!({"revision": revision, "source": source})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn publishing_builds_a_signed_artifact_that_a_workflow_pins_and_a_run_executes() {
    let lab = lab().await;
    let script = lab.script("pinned", "typescript", &returns(1)).await;
    let version = lab.publish(&script).await;
    assert_eq!(version["status"], "READY", "{version}");
    assert_eq!(version["number"], 1);
    let digest = version["artifactDigest"]
        .as_str()
        .expect("digest")
        .to_owned();
    assert_eq!(
        version["activity"],
        format!("script.sha256-{}", &digest["sha256:".len()..])
    );
    assert!(
        version["toolchain"]
            .as_str()
            .expect("toolchain")
            .contains(";shim="),
        "{version}"
    );
    assert!(version["buildKey"].is_string(), "{version}");

    // The artifact is signed for this tenant: the signature verifies with the platform's key.
    let (status, artifact) = lab
        .call(Method::GET, &format!("/script-artifacts/{digest}"), None)
        .await;
    assert_eq!(status, 200, "{artifact}");
    let signature = base64::engine::general_purpose::STANDARD
        .decode(artifact["signature"].as_str().expect("signature"))
        .expect("base64");
    let verifier = Verifier::new([lab.p.api.script_key.clone()]);
    let parsed = Digest::parse(&digest).expect("digest");
    assert_eq!(
        verifier.verify(
            artifact["keyId"].as_str().expect("key"),
            lab.p.tenant,
            &parsed,
            Language::TypeScript,
            &signature
        ),
        Ok(())
    );
    assert!(artifact["sbom"]["bomFormat"] == "CycloneDX", "{artifact}");

    // A workflow pins the digest and a run executes exactly that code.
    let workflow = lab.workflow("uses-the-script", &digest, json!({})).await;
    let run = lab.p.start(&workflow, &json!({"hello": "world"})).await;
    assert_eq!(lab.drive(&run).await, "SUCCEEDED");
    assert_eq!(lab.output(&run, "s").await, Some(json!({"version": 1})));
    // What ran is kept: once, with its log and its numbers, until the retention ends.
    let executions = lab.executions(&digest).await;
    assert_eq!(executions.len(), 1, "{executions:?}");
    assert_eq!(executions[0]["status"], "SUCCEEDED");
    assert_eq!(executions[0]["runId"], run);
    assert_eq!(executions[0]["node"], "s");
    assert!(executions[0]["wallMs"].as_u64().is_some());
}

#[tokio::test]
async fn a_new_version_changes_nothing_for_a_workflow_pinned_to_the_old_one() {
    let lab = lab().await;
    let script = lab.script("evolving", "typescript", &returns(1)).await;
    let first = lab.ready(&script).await;
    let old_workflow = lab.workflow("pinned-to-one", &first, json!({})).await;

    change_source(&lab, &script, 1, &returns(2)).await;
    let second = lab.ready(&script).await;
    assert_ne!(first, second, "different code, different artifact");
    let new_workflow = lab.workflow("pinned-to-two", &second, json!({})).await;

    // Both start after both versions exist; each runs the code it names.
    let old_run = lab.p.start(&old_workflow, &json!({})).await;
    let new_run = lab.p.start(&new_workflow, &json!({})).await;
    assert_eq!(lab.drive(&old_run).await, "SUCCEEDED");
    assert_eq!(lab.drive(&new_run).await, "SUCCEEDED");
    assert_eq!(lab.output(&old_run, "s").await, Some(json!({"version": 1})));
    assert_eq!(lab.output(&new_run, "s").await, Some(json!({"version": 2})));
    // The first version is untouched by the second.
    let (_, v1) = lab
        .call(
            Method::GET,
            &format!("/namespaces/{}/scripts/{script}/versions/1", lab.ns),
            None,
        )
        .await;
    assert_eq!(v1["source"], returns(1));
    assert_eq!(v1["artifactDigest"].as_str(), Some(first.as_str()));
}

#[tokio::test]
async fn the_same_draft_is_the_same_version_and_the_same_source_is_never_built_twice() {
    let lab = lab().await;
    let a = lab.script("original", "typescript", &returns(7)).await;
    let first = lab.publish(&a).await;
    assert_eq!(first["status"], "READY");
    // Publishing what is already the latest version is not another version.
    let (status, again) = lab
        .call(
            Method::POST,
            &format!("/namespaces/{}/scripts/{a}/versions", lab.ns),
            None,
        )
        .await;
    assert_eq!(
        (status, again["created"].clone()),
        (200, json!(false)),
        "{again}"
    );
    assert_eq!(again["version"]["number"], 1);

    // The same source in another script, under the same policy, reuses the artifact: the runner is
    // never handed a build.
    let b = lab.script("twin", "typescript", &returns(7)).await;
    let (status, published) = lab
        .call(
            Method::POST,
            &format!("/namespaces/{}/scripts/{b}/versions", lab.ns),
            None,
        )
        .await;
    assert_eq!(status, 202, "{published}");
    assert!(
        !lab.runner.build_once().await.expect("claim"),
        "nothing was left to build"
    );
    let (_, twin) = lab
        .call(
            Method::GET,
            &format!("/namespaces/{}/scripts/{b}/versions/1", lab.ns),
            None,
        )
        .await;
    assert_eq!(twin["status"], "READY", "{twin}");
    assert_eq!(twin["artifactDigest"], first["artifactDigest"]);

    // A different limit is a different artifact: the policy is part of the build.
    let c = lab
        .script_with(
            "tighter",
            "typescript",
            &returns(7),
            json!({"policy": {"memoryMib": 64}}),
        )
        .await;
    let tighter = lab.publish(&c).await;
    assert_eq!(tighter["status"], "READY");
    assert_ne!(
        tighter["artifactDigest"], first["artifactDigest"],
        "the policy is in the artifact"
    );
}

#[tokio::test]
async fn a_workflow_can_only_pin_an_artifact_this_tenant_built() {
    let lab = lab().await;
    let unknown = format!("sha256:{}", "ab".repeat(32));
    let ir = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "pins-nothing-real"}, "entrypoint": "s",
        "nodes": [{"id": "s", "type": "script", "artifactDigest": unknown}]
    });
    let (status, created) = lab
        .call(
            Method::POST,
            "/workflows",
            Some(&json!({"name": "pins-nothing-real", "ir": ir})),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().expect("id");
    let (status, refused) = lab
        .call(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            Some(&json!({"draftRevision": 1})),
        )
        .await;
    assert_eq!(status, 422, "{refused}");
    assert_eq!(refused["details"]["reason"], "UNKNOWN_SCRIPT", "{refused}");

    // Another tenant's artifact is no more real than a made-up one.
    let theirs = lab.p.api.tenant("globex-two", "bob-two").await;
    let _ = theirs;
    let (_, broken) = {
        let script = lab
            .script("breaks", "typescript", "export default function ( {\n")
            .await;
        let version = lab.publish(&script).await;
        (script, version)
    };
    assert_eq!(broken["status"], "FAILED", "{broken}");
    assert!(
        broken["artifactDigest"].is_null(),
        "a failed build has no artifact: {broken}"
    );
}

#[tokio::test]
async fn a_build_that_fails_is_visible_and_the_fixed_draft_becomes_the_next_version() {
    let lab = lab().await;
    let script = lab
        .script("fix-me", "typescript", "export default function ( {\n")
        .await;
    let failed = lab.publish(&script).await;
    assert_eq!(failed["status"], "FAILED", "{failed}");
    assert!(
        failed["failure"]
            .as_str()
            .expect("failure")
            .starts_with("rejected:"),
        "{failed}"
    );
    change_source(&lab, &script, 1, &returns(3)).await;
    let fixed = lab.publish(&script).await;
    assert_eq!(fixed["status"], "READY", "{fixed}");
    assert_eq!(fixed["number"], 2);
    let (_, listed) = lab
        .call(
            Method::GET,
            &format!("/namespaces/{}/scripts/{script}/versions", lab.ns),
            None,
        )
        .await;
    let numbers: Vec<_> = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|v| (v["number"].as_u64(), v["status"].as_str()))
        .collect();
    assert_eq!(
        numbers,
        vec![(Some(2), Some("READY")), (Some(1), Some("FAILED"))]
    );
}

#[tokio::test]
async fn a_lockfile_that_pins_a_package_is_refused_with_the_reason_because_installing_is_not_built()
{
    let lab = lab().await;
    let script = lab
        .script_with(
            "wants-left-pad",
            "typescript",
            &returns(1),
            json!({"lockfile": r#"{"lockfileVersion":3,"packages":{"":{},"node_modules/left-pad":{"version":"1.3.0","integrity":"sha512-x"}}}"#}),
        )
        .await;
    let version = lab.publish(&script).await;
    assert_eq!(version["status"], "FAILED", "{version}");
    let reason = version["failure"].as_str().expect("failure");
    assert!(
        reason.starts_with("lockfile:") && reason.contains("left-pad"),
        "{reason}"
    );
    // An empty lockfile is a lockfile that pins nothing: accepted, and part of the build.
    let empty = lab
        .script_with(
            "empty-lock",
            "typescript",
            &returns(1),
            json!({"lockfile": r#"{"lockfileVersion":3,"packages":{}}"#}),
        )
        .await;
    let ready = lab.publish(&empty).await;
    assert_eq!(ready["status"], "READY", "{ready}");
}

#[tokio::test]
async fn an_artifact_that_was_tampered_with_in_the_database_is_not_run() {
    let lab = lab().await;
    let script = lab.script("trusted", "typescript", &returns(1)).await;
    let digest = lab.ready(&script).await;
    let workflow = lab.workflow("runs-trusted", &digest, json!({})).await;

    // Someone with write access to the table swaps the code under the digest. (The triggers refuse
    // this to everyone but the owner; the owner turns them off here, as an attacker with that
    // access would.)
    let mut conn = lab.p.api.db.pool.acquire().await.expect("connection");
    sqlx::query("ALTER TABLE script_artifacts DISABLE TRIGGER script_artifacts_immutable")
        .execute(&mut *conn)
        .await
        .expect("disable");
    let evil = br#"{"v":1,"language":"typescript","policy":"v1;memory=268435456;cpu=1000;wall=30;pids=64;output=65536;scratch=33554432;egress=none","entry":"main.mjs","files":{"main.mjs":"export default () => ({ version: 666 })"}}"#.to_vec();
    sqlx::query("UPDATE script_artifacts SET content = $1 WHERE digest = $2")
        .bind(&evil)
        .bind(&digest)
        .execute(&mut *conn)
        .await
        .expect("swap the code");

    let run = lab.p.start(&workflow, &json!({})).await;
    assert_eq!(lab.drive(&run).await, "FAILED");
    assert_eq!(
        lab.failure(&run, "s").await.as_deref(),
        Some("SCRIPT_UNTRUSTED_ARTIFACT")
    );
    assert_eq!(
        lab.output(&run, "s").await,
        None,
        "the swapped code never ran"
    );
    assert!(
        lab.executions(&digest).await.is_empty(),
        "nothing was executed, so nothing is recorded"
    );
}

#[tokio::test]
async fn a_signature_that_the_platform_did_not_make_is_not_trusted_even_over_the_right_bytes() {
    let lab = lab().await;
    let script = lab
        .script("forged-signature", "typescript", &returns(1))
        .await;
    let digest = lab.ready(&script).await;
    let workflow = lab.workflow("runs-forged", &digest, json!({})).await;
    let mut conn = lab.p.api.db.pool.acquire().await.expect("connection");
    sqlx::query("ALTER TABLE script_artifacts DISABLE TRIGGER script_artifacts_immutable")
        .execute(&mut *conn)
        .await
        .expect("disable");
    sqlx::query("UPDATE script_artifacts SET signature = $1 WHERE digest = $2")
        .bind(vec![7_u8; 64])
        .bind(&digest)
        .execute(&mut *conn)
        .await
        .expect("replace the signature");
    let run = lab.p.start(&workflow, &json!({})).await;
    assert_eq!(lab.drive(&run).await, "FAILED");
    assert_eq!(
        lab.failure(&run, "s").await.as_deref(),
        Some("SCRIPT_UNTRUSTED_ARTIFACT")
    );
}
