//! US04 — a safe rollback: when a version misbehaves an operator points new runs at the one before,
//! nothing that was written is rewritten, runs in flight keep their version, and the history says
//! who moved the pointer and why (US04-AC01).

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::json;

use crate::support::worker::platform;
use crate::{Dir, active, cli, digest_of, flow, rest_in, version, workflow_id};

const NS: &str = "payments";

async fn start(p: &crate::support::worker::Platform, workflow: &str) -> String {
    let (status, run) = rest_in(
        p,
        "alice",
        NS,
        Method::POST,
        "/runs",
        Some(&json!({"workflowId": workflow, "input": {}})),
    )
    .await;
    assert_eq!(status, 202, "{run}");
    let id = run["runId"].as_str().expect("run").to_owned();
    let (_, body) = rest_in(p, "alice", NS, Method::GET, &format!("/runs/{id}"), None).await;
    body["versionId"].as_str().expect("version id").to_owned()
}

#[tokio::test]
async fn us04_rolling_back_moves_the_pointer_and_rewrites_nothing() {
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    p.grant(p.tenant, "olga", Role::Operator).await;
    let dir = Dir::new();
    dir.manifest(NS, &["checkout"]);
    let docs = [
        flow("checkout", "one"),
        flow("checkout", "two"),
        flow("checkout", "three"),
    ];
    for doc in &docs {
        dir.document("checkout", doc);
        cli(&p, &dir, "dev1", &["publish"]).await.expect(0);
    }
    let checkout = workflow_id(&p, NS, "checkout").await;
    assert_eq!(active(&p, NS, &checkout).await, 3);
    let on_three = start(&p, &checkout).await;
    let before = [
        version(&p, NS, &checkout, 1).await,
        version(&p, NS, &checkout, 2).await,
        version(&p, NS, &checkout, 3).await,
    ];

    // A developer can publish but not move the pointer.
    cli(
        &p,
        &dir,
        "dev1",
        &[
            "rollback",
            "--workflow",
            "checkout",
            "--reason",
            "three misbehaves",
        ],
    )
    .await
    .expect(1)
    .mentions("FORBIDDEN");
    assert_eq!(active(&p, NS, &checkout).await, 3);

    // The operator goes back to the version before, and says why.
    cli(
        &p,
        &dir,
        "olga",
        &[
            "rollback",
            "--workflow",
            "checkout",
            "--reason",
            "three misbehaves in checkout",
        ],
    )
    .await
    .expect(0)
    .mentions("now runs v2 (was v3)")
    .mentions("runs in flight keep the version they started with");
    assert_eq!(active(&p, NS, &checkout).await, 2);

    // New runs use version 2; the run that started on 3 keeps 3; no version was changed.
    assert_eq!(
        start(&p, &checkout).await,
        before[1]["versionId"].as_str().expect("id")
    );
    let (_, old) = rest_in(&p, "alice", NS, Method::GET, "/runs?limit=10", None).await;
    assert!(
        old["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|r| r["versionId"] == before[2]["versionId"]),
        "the run on version 3 is still on version 3"
    );
    assert_eq!(on_three, before[2]["versionId"].as_str().expect("id"));
    for (n, was) in before.iter().enumerate() {
        let now = version(&p, NS, &checkout, u32::try_from(n + 1).expect("n")).await;
        assert_eq!(&now, was, "version {} is exactly as it was", n + 1);
        assert_eq!(now["digest"], digest_of(&docs[n]));
    }

    // The history says who did it, and the earlier rows are still there.
    let history = cli(&p, &dir, "olga", &["history", "--workflow", "checkout"]).await;
    history
        .expect(0)
        .mentions("ROLLBACK")
        .mentions("v3 -> v2  by olga")
        .mentions("three misbehaves in checkout");
    assert_eq!(history.out.matches("PUBLISH").count(), 3, "{}", history.out);

    // Git still holds version 3: the tool says what publishing would do before it does it.
    cli(&p, &dir, "dev1", &["diff"])
        .await
        .expect(0)
        .mentions("this is the document of v3, published before: publishing activates it again");

    // Asking again for the version that is active changes nothing; a wrong expectation is refused.
    cli(
        &p,
        &dir,
        "olga",
        &[
            "rollback",
            "--workflow",
            "checkout",
            "--to",
            "2",
            "--reason",
            "already there, checking",
        ],
    )
    .await
    .expect(0)
    .mentions("nothing changed");
}
