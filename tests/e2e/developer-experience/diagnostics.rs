//! US04: a document with a mistake in it is told where, what to do and where it is explained,
//! before it is sent, and a document that is fine is not held up.

use serde_json::json;

use crate::support::worker::platform;
use crate::{Dir, cli};

fn broken() -> String {
    serde_json::to_string_pretty(&json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "checkout"}, "entrypoint": "validate",
        "nodes": [
            {"id": "validate", "type": "activity", "activity": "order.validate", "next": "chrage"},
            {"id": "charge", "type": "activity", "activity": "payment.charge"}
        ]
    }))
    .expect("json")
}

#[tokio::test]
async fn us04_a_workflow_with_an_invalid_node_is_told_where_what_to_do_and_where_to_read_more() {
    let p = platform().await;
    let dir = Dir::new();
    dir.write("checkout.json", &broken());

    let told = cli(&p, &dir, &["workflow", "validate", "checkout.json"]).await;
    told.expect(1)
        .mentions("error[UNKNOWN_TARGET]: node 'validate' refers to 'chrage', which does not exist")
        .mentions("--> node 'validate'")
        .mentions("help: A node points")
        .mentions("docs: docs/quickstarts/errors.md#unknown_target")
        .mentions("is not publishable: 1 problem");
    // The platform's graph rules stop at the first one broken (fix it and validate again): here the
    // typo, and once it is fixed nothing is left, because `charge` is then reached.
    dir.write("checkout.json", &broken().replace("chrage", "charge"));
    cli(&p, &dir, &["workflow", "validate", "checkout.json"])
        .await
        .expect(0)
        .mentions("ok    checkout");
    dir.write("checkout.json", &broken());

    // Nothing was sent: publishing the same document stops the same way and creates nothing.
    cli(&p, &dir, &["workflow", "publish", "checkout.json"])
        .await
        .expect(1)
        .mentions("UNKNOWN_TARGET");
    let (_, page) = p.rest(reqwest::Method::GET, "/workflows", None).await;
    assert_eq!(page["items"].as_array().expect("items").len(), 0);
}

#[tokio::test]
async fn us04_a_manifest_with_an_invalid_document_is_told_the_same_way() {
    let p = platform().await;
    let dir = Dir::new();
    dir.write("workflows/checkout.json", &broken());
    dir.write(
        "sagat.yaml",
        "apiVersion: sagat-octopus-orchestrator.io/v1\nkind: Manifest\nnamespace: payments\nworkflows:\n  - name: checkout\n    file: workflows/checkout.json\n",
    );
    cli(&p, &dir, &["validate"])
        .await
        .expect(1)
        .mentions("FAIL  checkout")
        .mentions("error[UNKNOWN_TARGET]")
        .mentions("help:")
        .mentions("docs:");
    cli(&p, &dir, &["publish"])
        .await
        .expect(1)
        .mentions("nothing was sent")
        .mentions("UNKNOWN_TARGET");
}

#[tokio::test]
async fn us04_what_only_the_platform_can_find_is_told_with_the_same_help() {
    let p = platform().await;
    let dir = Dir::new();
    // A script nobody built, pinned by a digest that is shaped right.
    let doc = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "enrich"}, "entrypoint": "s",
        "nodes": [{"id": "s", "type": "script", "language": "typescript",
                   "artifactDigest": format!("sha256:{}", "ab".repeat(32))}]
    });
    dir.write(
        "enrich.json",
        &serde_json::to_string_pretty(&doc).expect("json"),
    );
    // The workflow does not exist yet: the document is valid, and the platform checks it at publication.
    cli(&p, &dir, &["workflow", "validate", "enrich.json"])
        .await
        .expect(0)
        .mentions("ok    enrich");
    let refused = cli(&p, &dir, &["workflow", "publish", "enrich.json"]).await;
    refused.expect(1).mentions("UNKNOWN_SCRIPT");
}

#[tokio::test]
async fn us04_a_file_that_is_not_a_document_is_told_what_a_document_is() {
    let p = platform().await;
    let dir = Dir::new();
    dir.write("bad.json", "{ not json");
    dir.write("list.json", "[1, 2]");
    dir.write(
        "huge.json",
        &format!("{{\"pad\": \"{}\"}}", "x".repeat(1_100_000)),
    );
    cli(&p, &dir, &["workflow", "validate", "bad.json"])
        .await
        .expect(1)
        .mentions("error[DOCUMENT_MALFORMED]")
        .mentions("help: The file is not one JSON or YAML object");
    cli(&p, &dir, &["workflow", "validate", "list.json"])
        .await
        .expect(1)
        .mentions("error[DOCUMENT_MALFORMED]");
    cli(&p, &dir, &["workflow", "validate", "huge.json"])
        .await
        .expect(1)
        .mentions("error[DOCUMENT_TOO_LARGE]");
    cli(&p, &dir, &["workflow", "validate", "missing.json"])
        .await
        .expect(1)
        .mentions("missing.json");
}
