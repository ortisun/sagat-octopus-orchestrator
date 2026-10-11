//! US01 — publish an immutable version: the manifest in Git becomes versions whose document and
//! digest are fixed, running the same command again changes nothing, and what cannot be published
//! is refused before anything is sent (US01-AC01).

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::json;

use crate::support::worker::platform;
use crate::{Dir, active, cli, digest_of, flow, rest_in, version, workflow_id};

const NS: &str = "payments";

#[tokio::test]
async fn us01_a_manifest_becomes_immutable_versions_and_a_rerun_changes_nothing() {
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    let dir = Dir::new();
    let (v1, v2, v3) = (
        flow("checkout", "one"),
        flow("checkout", "two"),
        flow("checkout", "three"),
    );
    dir.document("checkout", &v1);
    dir.document("refund", &flow("refund", "one"));
    dir.manifest(NS, &["checkout", "refund"]);

    // Before anything is sent the documents are checked, and the platform has nothing yet.
    cli(&p, &dir, "dev1", &["validate"])
        .await
        .expect(0)
        .mentions("2 documents are valid");
    cli(&p, &dir, "dev1", &["diff", "--check"])
        .await
        .expect(5)
        .mentions("+ checkout")
        .mentions("2 of 2 workflows would change");
    cli(&p, &dir, "dev1", &["publish", "--dry-run"])
        .await
        .expect(0)
        .mentions("nothing was sent");
    let (_, listed) = rest_in(&p, "alice", NS, Method::GET, "/workflows", None).await;
    assert_eq!(
        listed["items"].as_array().expect("items").len(),
        0,
        "a dry run creates nothing"
    );

    // Publishing creates both workflows and publishes their first version; the digest the
    // platform froze is the digest of the document in Git.
    cli(&p, &dir, "dev1", &["publish"])
        .await
        .expect(0)
        .mentions("published  checkout v1")
        .mentions("published  refund v1")
        .mentions("2 published, 0 waiting for approval, 0 unchanged");
    let checkout = workflow_id(&p, NS, "checkout").await;
    let first = version(&p, NS, &checkout, 1).await;
    assert_eq!(first["digest"], digest_of(&v1));
    assert_eq!(first["publishedBy"], "dev1");

    // Running it again is the same operation: nothing is published, nothing is created.
    cli(&p, &dir, "dev1", &["publish"])
        .await
        .expect(0)
        .mentions("0 published, 0 waiting for approval, 2 unchanged");
    cli(&p, &dir, "dev1", &["diff", "--check"])
        .await
        .expect(0)
        .mentions("0 of 2 workflows would change");
    let (_, versions) = rest_in(
        &p,
        "alice",
        NS,
        Method::GET,
        &format!("/workflows/{checkout}/versions"),
        None,
    )
    .await;
    assert_eq!(versions["items"].as_array().expect("items").len(), 1);

    // A change in Git is a new version; the ones before are exactly as they were.
    dir.document("checkout", &v2);
    cli(&p, &dir, "dev1", &["diff"])
        .await
        .expect(0)
        .mentions("~ checkout  v1 ->");
    cli(&p, &dir, "dev1", &["publish"])
        .await
        .expect(0)
        .mentions("published  checkout v2");
    dir.document("checkout", &v3);
    cli(&p, &dir, "dev1", &["publish"])
        .await
        .expect(0)
        .mentions("published  checkout v3");
    assert_eq!(active(&p, NS, &checkout).await, 3);
    let (again, third) = (
        version(&p, NS, &checkout, 1).await,
        version(&p, NS, &checkout, 3).await,
    );
    assert_eq!(again["digest"], first["digest"], "version 1 is immutable");
    assert_eq!(again["document"], first["document"]);
    assert_eq!(
        third["digest"],
        digest_of(&v3),
        "the digest of version 3 is fixed at publication"
    );
    assert_ne!(third["digest"], first["digest"]);

    // New runs use version 3.
    let (status, run) = rest_in(
        &p,
        "alice",
        NS,
        Method::POST,
        "/runs",
        Some(&json!({"workflowId": checkout, "input": {}})),
    )
    .await;
    assert_eq!(status, 202, "{run}");
    let (_, started) = rest_in(
        &p,
        "alice",
        NS,
        Method::GET,
        &format!("/runs/{}", run["runId"].as_str().expect("run")),
        None,
    )
    .await;
    assert_eq!(started["versionId"], third["versionId"]);
}

#[tokio::test]
async fn us01_what_cannot_be_published_is_refused_before_anything_is_sent() {
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    let dir = Dir::new();
    dir.document("checkout", &flow("checkout", "one"));
    // The entrypoint names a node that is not there.
    let mut broken = flow("refund", "one");
    broken["entrypoint"] = json!("nowhere");
    dir.document("refund", &broken);
    dir.manifest(NS, &["checkout", "refund"]);

    cli(&p, &dir, "dev1", &["validate"])
        .await
        .expect(1)
        .mentions("FAIL  refund");
    let published = cli(&p, &dir, "dev1", &["publish"]).await;
    published
        .expect(1)
        .mentions("nothing was sent")
        .mentions("refund");
    let (_, listed) = rest_in(&p, "alice", NS, Method::GET, "/workflows", None).await;
    assert_eq!(
        listed["items"].as_array().expect("items").len(),
        0,
        "the good document is not published without the bad one"
    );

    // A manifest that points outside its directory is refused as a manifest.
    dir.write("sagat.yaml", "apiVersion: sagat-octopus-orchestrator.io/v1\nkind: Manifest\nnamespace: payments\nworkflows:\n  - name: checkout\n    file: ../outside.json\n");
    cli(&p, &dir, "dev1", &["publish"])
        .await
        .expect(1)
        .mentions("a relative path below the directory of the manifest");

    // Somebody who may not publish is told so by the platform, in its own words.
    p.grant(p.tenant, "vera", Role::Viewer).await;
    dir.manifest(NS, &["checkout"]);
    cli(&p, &dir, "vera", &["publish"])
        .await
        .expect(1)
        .mentions("FORBIDDEN");
}
