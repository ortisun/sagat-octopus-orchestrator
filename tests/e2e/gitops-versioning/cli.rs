//! The command line on its own terms: what it does when the platform asks for approvals, when a
//! change breaks callers, when it cannot reach the platform, and how a namespace built elsewhere
//! comes under Git.

use reqwest::Method;
use sagat_cli::Env;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use crate::support::worker::platform;
use crate::{Dir, Outcome, cli, flow, rest_in, workflow_id};

const DEV: &str = "payments";
const PROD: &str = "payments-prod";

fn with_inputs(name: &str, marker: &str, properties: Value) -> Value {
    let mut doc = flow(name, marker);
    doc["inputSchema"] = json!({"type": "object", "properties": properties});
    doc
}

#[tokio::test]
async fn publishing_a_manifest_into_a_namespace_that_asks_for_approvals_waits_and_then_converges() {
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    p.grant(p.tenant, "dev2", Role::Developer).await;
    let (_, workspaces) = rest_in(&p, "alice", DEV, Method::GET, "/workspaces", None).await;
    let workspace = workspaces["items"][0]["id"].as_str().expect("workspace");
    let (status, _) = rest_in(
        &p,
        "alice",
        DEV,
        Method::POST,
        &format!("/workspaces/{workspace}/namespaces"),
        Some(&json!({"slug": PROD, "environment": "prod"})),
    )
    .await;
    assert_eq!(status, 201);

    let dir = Dir::new();
    dir.document("checkout", &flow("checkout", "one"));
    dir.manifest(PROD, &["checkout"]);

    // The direct publish is refused by the platform; the command opens the request and says so.
    cli(&p, &dir, "dev1", &["publish"])
        .await
        .expect(3)
        .mentions("created   checkout")
        .mentions("waiting    checkout: request")
        .mentions("0 of 1 approvals")
        .mentions("0 published, 1 waiting for approval");
    // Again: it is the same request, not another one.
    cli(&p, &dir, "dev1", &["publish"]).await.expect(3);
    let checkout = workflow_id(&p, PROD, "checkout").await;
    let (_, requests) = rest_in(
        &p,
        "alice",
        PROD,
        Method::GET,
        &format!("/workflows/{checkout}/publication-requests"),
        None,
    )
    .await;
    let items = requests["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{requests}");
    let request = items[0]["id"].as_str().expect("id").to_owned();

    // Changing the document in Git supersedes the request the approvers were looking at.
    dir.document("checkout", &flow("checkout", "two"));
    cli(&p, &dir, "dev1", &["publish"])
        .await
        .expect(3)
        .mentions("draft     checkout replaced by the manifest");
    let (_, requests) = rest_in(
        &p,
        "alice",
        PROD,
        Method::GET,
        &format!("/workflows/{checkout}/publication-requests"),
        None,
    )
    .await;
    let states: Vec<&str> = requests["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|r| r["state"].as_str())
        .collect();
    assert_eq!(states.len(), 2, "{requests}");
    assert!(
        states.contains(&"SUPERSEDED") && states.contains(&"PENDING"),
        "{states:?}"
    );
    let current = requests["items"][0]["id"].as_str().expect("id");
    assert_ne!(current, request);

    // Somebody else approves what is in Git now, and the next run publishes it.
    cli(
        &p,
        &dir,
        "dev2",
        &[
            "approve",
            "--request",
            current,
            "--reason",
            "reviewed the document in Git",
            "--namespace",
            PROD,
        ],
    )
    .await
    .expect(0);
    cli(&p, &dir, "dev1", &["publish"])
        .await
        .expect(0)
        .mentions("published  checkout v1");
    cli(&p, &dir, "dev1", &["diff", "--check"]).await.expect(0);
}

#[tokio::test]
async fn a_change_that_breaks_callers_is_refused_by_the_tool_until_it_is_accepted() {
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    p.grant(p.tenant, "olga", Role::Operator).await;
    let dir = Dir::new();
    dir.manifest(DEV, &["checkout"]);
    dir.document(
        "checkout",
        &with_inputs(
            "checkout",
            "one",
            json!({"orderId": {"type": "string"}, "note": {"type": "string"}}),
        ),
    );
    cli(&p, &dir, "dev1", &["publish"]).await.expect(0);

    // The second version drops an input property callers may send.
    dir.document(
        "checkout",
        &with_inputs("checkout", "two", json!({"orderId": {"type": "string"}})),
    );
    let refused = cli(&p, &dir, "dev1", &["publish"]).await;
    refused
        .expect(4)
        .mentions("breaking  ")
        .mentions("refused, nothing was sent");
    let checkout = workflow_id(&p, DEV, "checkout").await;
    let (_, versions) = rest_in(
        &p,
        "alice",
        DEV,
        Method::GET,
        &format!("/workflows/{checkout}/versions"),
        None,
    )
    .await;
    assert_eq!(
        versions["items"].as_array().expect("items").len(),
        1,
        "nothing was published"
    );
    cli(&p, &dir, "dev1", &["publish", "--accept-breaking"])
        .await
        .expect(0)
        .mentions("published  checkout v2");

    // Going back to version 1 would bring the property back and is fine; forward again drops it,
    // which the platform itself refuses to do unless the operator accepts it.
    cli(
        &p,
        &dir,
        "olga",
        &[
            "rollback",
            "--workflow",
            "checkout",
            "--reason",
            "two broke a caller",
        ],
    )
    .await
    .expect(0);
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
            "the caller is fixed",
        ],
    )
    .await
    .expect(4)
    .mentions("breaks what callers and emitters rely on");
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
            "the caller is fixed",
            "--accept-breaking",
        ],
    )
    .await
    .expect(0)
    .mentions("now runs v2");
}

#[tokio::test]
async fn a_namespace_built_elsewhere_comes_under_git_with_export() {
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    let dir = Dir::new();
    dir.manifest(DEV, &["checkout", "refund"]);
    dir.document("checkout", &flow("checkout", "one"));
    dir.document("refund", &flow("refund", "one"));
    cli(&p, &dir, "dev1", &["publish"]).await.expect(0);
    // A workflow nobody published has no active version to export.
    let (status, _) = rest_in(
        &p,
        "alice",
        DEV,
        Method::POST,
        "/workflows",
        Some(&json!({"name": "scratch", "ir": flow("scratch", "x")})),
    )
    .await;
    assert_eq!(status, 201);

    let exported = Dir::new();
    cli(
        &p,
        &exported,
        "dev1",
        &["export", "--out", "git", "--namespace", DEV],
    )
    .await
    .expect(0)
    .mentions("exported 2 workflows of payments")
    .mentions("skipped scratch (no active version)");
    // What was written is a manifest that says the platform has nothing to change.
    cli(
        &p,
        &exported,
        "dev1",
        &["diff", "--check", "-f", "git/sagat.yaml"],
    )
    .await
    .expect(0)
    .mentions("0 of 2 workflows would change");
    cli(&p, &exported, "dev1", &["validate", "-f", "git/sagat.yaml"])
        .await
        .expect(0);
    // Nothing is overwritten.
    cli(
        &p,
        &exported,
        "dev1",
        &["export", "--out", "git", "--namespace", DEV],
    )
    .await
    .expect(1)
    .mentions("already has a sagat.yaml");
}

async fn plain(env: Env, args: &[&str]) -> Outcome {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = sagat_cli::run(&args, &env, &mut out, &mut err).await;
    Outcome {
        code,
        out: String::from_utf8_lossy(&out).into_owned(),
        err: String::from_utf8_lossy(&err).into_owned(),
    }
}

#[tokio::test]
async fn the_tool_says_what_is_missing_and_never_sends_a_token_where_it_would_be_read() {
    let p = platform().await;
    let dir = Dir::new();
    dir.document("checkout", &flow("checkout", "one"));
    dir.manifest(DEV, &["checkout"]);
    let env = |server: Option<&str>, token: Option<&str>| Env {
        server: server.map(str::to_owned),
        token: token.map(str::to_owned),
        namespace: None,
        cwd: dir.0.clone(),
    };

    plain(env(None, Some("t")), &["diff"])
        .await
        .expect(1)
        .mentions("--server or set SAGAT_SERVER");
    plain(env(Some(&p.api.base), None), &["diff"])
        .await
        .expect(1)
        .mentions("--token-file or set SAGAT_TOKEN");
    // The platform's answer to a token it does not know is printed, the token is not.
    let bad = plain(
        env(Some(&p.api.base), Some("not-a-token-0123456789")),
        &["diff"],
    )
    .await;
    bad.expect(1).mentions("UNAUTHENTICATED");
    assert!(
        !bad.all().contains("not-a-token-0123456789"),
        "{}",
        bad.all()
    );
    // A plain-http address that is not this machine is refused before anything is sent.
    plain(
        env(Some("http://example.com/v1"), Some("t0123456789")),
        &["diff"],
    )
    .await
    .expect(1)
    .mentions("the token must not travel in the clear");
    // Nobody listens: the person is told, with no stack and no token.
    plain(
        env(Some("http://127.0.0.1:1/v1"), Some("t0123456789")),
        &["diff"],
    )
    .await
    .expect(1)
    .mentions("could not be reached");
    // The token can come from a file, whose content is not in the arguments.
    dir.write("token", &p.api.idp.token("alice", "acme").build());
    plain(
        env(Some(&p.api.base), None),
        &["diff", "--token-file", "token"],
    )
    .await
    .expect(0);
    // And what is not a command is a usage error with the usage.
    plain(env(None, None), &["frobnicate"])
        .await
        .expect(2)
        .mentions("USAGE");
}

#[tokio::test]
async fn a_reason_that_would_rewrite_the_terminal_is_refused_by_the_platform() {
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    p.grant(p.tenant, "dev2", Role::Developer).await;
    let (_, workspaces) = rest_in(&p, "alice", DEV, Method::GET, "/workspaces", None).await;
    let workspace = workspaces["items"][0]["id"].as_str().expect("workspace");
    let (status, _) = rest_in(
        &p,
        "alice",
        DEV,
        Method::POST,
        &format!("/workspaces/{workspace}/namespaces"),
        Some(&json!({"slug": PROD, "environment": "prod"})),
    )
    .await;
    assert_eq!(status, 201);
    let dir = Dir::new();
    dir.document("checkout", &flow("checkout", "one"));
    dir.manifest(PROD, &["checkout"]);
    cli(&p, &dir, "dev1", &["publish"]).await.expect(3);
    let listed = cli(&p, &dir, "dev1", &["requests", "--workflow", "checkout"]).await;
    let request = listed.out.split_whitespace().next().expect("id").to_owned();

    // The platform refuses a reason with control characters in it: an escape sequence that clears
    // the screen, a carriage return that would hide the start of a line. The tool shows nothing of the
    // kind either (`printable`, tested in the crate of the tool), in case it ever talks to a platform
    // that does not.
    let reason = "\u{1b}[2J\u{1b}[31mlooks right\u{1b}[0m\rAPPROVED BY SECURITY";
    let (status, answer) = rest_in(
        &p,
        "dev2",
        PROD,
        Method::POST,
        &format!("/publication-requests/{request}/approve"),
        Some(&json!({"reason": reason})),
    )
    .await;
    assert_eq!(status, 422, "{answer}");
    let shown = cli(&p, &dir, "dev1", &["requests", "--workflow", "checkout"]).await;
    shown.expect(0);
    assert!(!shown.all().contains('\u{1b}') && !shown.all().contains('\r'));
}
