//! US03 — promote across environments: a version tried in dev reaches staging and then production
//! as the same document, production asks for an approval from somebody other than the author, and
//! the history of each namespace says where it came from and who approved (US03-AC01).

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::json;

use crate::support::worker::platform;
use crate::{Dir, cli, digest_of, flow, rest_in, version, workflow_id};

const DEV: &str = "payments";
const STAGING: &str = "payments-staging";
const PROD: &str = "payments-prod";

async fn namespace(
    p: &crate::support::worker::Platform,
    workspace: &str,
    slug: &str,
    environment: &str,
) {
    let (status, created) = rest_in(
        p,
        "alice",
        DEV,
        Method::POST,
        &format!("/workspaces/{workspace}/namespaces"),
        Some(&json!({"slug": slug, "environment": environment})),
    )
    .await;
    assert_eq!(status, 201, "{created}");
}

#[tokio::test]
async fn us03_a_version_goes_from_dev_to_staging_to_production_with_its_history_and_its_approval() {
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    p.grant(p.tenant, "dev2", Role::Developer).await;
    let (_, workspaces) = rest_in(&p, "alice", DEV, Method::GET, "/workspaces", None).await;
    let workspace = workspaces["items"][0]["id"]
        .as_str()
        .expect("workspace")
        .to_owned();
    namespace(&p, &workspace, STAGING, "staging").await;
    namespace(&p, &workspace, PROD, "prod").await;

    // The manifest in Git is published in dev.
    let doc = flow("checkout", "release train 42");
    let dir = Dir::new();
    dir.document("checkout", &doc);
    dir.manifest(DEV, &["checkout"]);
    cli(&p, &dir, "dev1", &["publish"])
        .await
        .expect(0)
        .mentions("published  checkout v1");

    // Staging asks for no approval: the same document is published there at once.
    cli(
        &p,
        &dir,
        "dev1",
        &[
            "promote",
            "--workflow",
            "checkout",
            "--to",
            STAGING,
            "--reason",
            "release train 42, dev is green",
            "--namespace",
            DEV,
        ],
    )
    .await
    .expect(0)
    .mentions("published as v1");
    let staging = workflow_id(&p, STAGING, "checkout").await;
    assert_eq!(
        version(&p, STAGING, &staging, 1).await["digest"],
        digest_of(&doc)
    );

    // Production asks for one approval: the promotion becomes a request and the command says so.
    let promote_prod = [
        "promote",
        "--workflow",
        "checkout",
        "--to",
        PROD,
        "--reason",
        "release train 42, staging is green",
        "--namespace",
        STAGING,
    ];
    let waiting = cli(&p, &dir, "dev1", &promote_prod).await;
    waiting
        .expect(3)
        .mentions("waits for approvals")
        .mentions("0 of 1");
    let listed = cli(
        &p,
        &dir,
        "dev1",
        &["requests", "--workflow", "checkout", "--namespace", PROD],
    )
    .await;
    listed.expect(0).mentions("PENDING").mentions("PROMOTE");
    let request = listed
        .out
        .split_whitespace()
        .next()
        .expect("a request id")
        .to_owned();

    // The author cannot approve their own request; somebody else can.
    cli(
        &p,
        &dir,
        "dev1",
        &[
            "approve",
            "--request",
            &request,
            "--reason",
            "it is mine and I like it",
            "--namespace",
            PROD,
        ],
    )
    .await
    .expect(1)
    .mentions("AUTHOR_CANNOT_DECIDE");
    cli(
        &p,
        &dir,
        "dev2",
        &[
            "approve",
            "--request",
            &request,
            "--reason",
            "same document as staging, checked",
            "--namespace",
            PROD,
        ],
    )
    .await
    .expect(0)
    .mentions("1 of 1 approvals");

    // Running the same promotion again publishes the approved request.
    cli(&p, &dir, "dev1", &promote_prod)
        .await
        .expect(0)
        .mentions("published in payments-prod as v1");
    let prod = workflow_id(&p, PROD, "checkout").await;
    let promoted = version(&p, PROD, &prod, 1).await;
    assert_eq!(
        promoted["digest"],
        digest_of(&doc),
        "the same document reached production"
    );
    assert_eq!(
        promoted["scripts"],
        json!([]),
        "the dependencies of the version travel with it"
    );

    // The history of production says it was a promotion, which request and who approved it.
    let history = cli(
        &p,
        &dir,
        "dev1",
        &["history", "--workflow", "checkout", "--namespace", PROD],
    )
    .await;
    history
        .expect(0)
        .mentions("PROMOTE")
        .mentions(&request)
        .mentions("release train 42, staging is green");
    let decided = cli(
        &p,
        &dir,
        "dev1",
        &["requests", "--workflow", "checkout", "--namespace", PROD],
    )
    .await;
    decided
        .expect(0)
        .mentions("PUBLISHED")
        .mentions("approved dev2");

    // Going back to an earlier environment is not a promotion.
    cli(
        &p,
        &dir,
        "dev1",
        &[
            "promote",
            "--workflow",
            "checkout",
            "--to",
            DEV,
            "--reason",
            "back to dev, please",
            "--namespace",
            PROD,
        ],
    )
    .await
    .expect(1)
    .mentions("same environment or a later one");
}
