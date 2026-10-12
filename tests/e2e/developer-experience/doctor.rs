//! `sagat doctor`: every thing that goes wrong on the first day is told, with what to do about it.

use std::time::Duration;

use sagat_cli::Env;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

use crate::support::worker::platform;
use crate::{Dir, Outcome, cli, run_with};

/// A listener that greets the way a NATS server does, and one that says nothing.
async fn fake_broker(greeting: Option<&'static str>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            if let Some(text) = greeting {
                let _ = socket.write_all(text.as_bytes()).await;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
    format!("nats://127.0.0.1:{port}")
}

fn env_for(
    p: &crate::support::worker::Platform,
    dir: &Dir,
    token: Option<String>,
    namespace: Option<&str>,
) -> Env {
    Env {
        server: Some(p.api.base.clone()),
        token,
        namespace: namespace.map(str::to_owned),
        cwd: dir.0.clone(),
        ..Default::default()
    }
}

async fn doctor(env: Env, extra: &[&str]) -> Outcome {
    let mut args = vec!["doctor"];
    args.extend_from_slice(extra);
    run_with(env, &args).await
}

#[tokio::test]
async fn a_machine_that_is_ready_is_told_so_check_by_check() {
    let p = platform().await;
    let engine = crate::engine(&p).await;
    let dir = Dir::new();
    let broker = fake_broker(Some("INFO {\"server_id\":\"x\"}\r\n")).await;
    // (The default token lives five minutes, which is where the tool starts to warn: it would be "ok" only
    // if the command ran in the second the token was made. This one has half an hour.)
    let token = p.api.idp.token("alice", "acme").issued(0, 1800).build();
    let told = doctor(
        env_for(&p, &dir, Some(token), Some("payments")),
        &["--grpc", &engine.grpc_url, "--nats", &broker],
    )
    .await;
    told.expect(0)
        .mentions("ok    address")
        .mentions("ok    token")
        .mentions("for alice")
        .mentions("ok    reachable")
        .mentions("ok    ready")
        .mentions("ok    credentials")
        .mentions("as admin in tenant acme")
        .mentions("can do everything the tool does")
        .mentions("ok    versions")
        .mentions(&format!("this tool {}", env!("CARGO_PKG_VERSION")))
        .mentions("ok    namespace")
        .mentions("payments exists")
        .mentions("ok    worker")
        .mentions("ok    broker")
        .mentions("is a NATS server")
        .mentions("nothing to fix");
    // The token is read and never printed.
    let token = p.api.idp.token("alice", "acme").issued(0, 1800).build();
    let again = doctor(
        env_for(&p, &dir, Some(token.clone()), Some("payments")),
        &[],
    )
    .await;
    again
        .omits(&token)
        .omits(token.split('.').nth(1).expect("payload"));
}

#[tokio::test]
async fn a_token_near_expiry_is_a_warning_and_not_a_failed_check() {
    let p = platform().await;
    let dir = Dir::new();
    let token = p.api.idp.token("alice", "acme").issued(0, 240).build();
    doctor(env_for(&p, &dir, Some(token), Some("payments")), &[])
        .await
        .expect(0)
        .mentions("warn  token")
        .mentions("expires in")
        .mentions("for alice")
        .mentions("get a new token before a long command")
        .mentions("nothing to fix");
}

#[tokio::test]
async fn the_machine_readable_form_says_the_same_thing() {
    let p = platform().await;
    let dir = Dir::new();
    let token = p.api.idp.token("alice", "acme").build();
    let told = doctor(
        env_for(&p, &dir, Some(token), Some("payments")),
        &["--json"],
    )
    .await;
    told.expect(0);
    let report: Value = serde_json::from_str(&told.out).expect("json");
    assert_eq!(report["ok"], true);
    let names: Vec<&str> = report["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    for name in [
        "address",
        "token",
        "reachable",
        "ready",
        "credentials",
        "versions",
        "namespace",
        "worker",
    ] {
        assert!(names.contains(&name), "{name} in {names:?}");
    }
}

#[tokio::test]
async fn a_token_the_platform_does_not_accept_is_told_with_what_to_do() {
    let p = platform().await;
    let dir = Dir::new();
    let garbage = doctor(
        env_for(
            &p,
            &dir,
            Some("not-a-token-0123456789".into()),
            Some("payments"),
        ),
        &[],
    )
    .await;
    garbage
        .expect(1)
        .mentions("warn  token")
        .mentions("not a JWT")
        .mentions("FAIL  credentials")
        .mentions("UNAUTHENTICATED")
        .mentions("help: the token is not accepted");
    // A token of a member of no tenant looks like any other refusal.
    let stranger = p.api.idp.token("stranger", "acme").build();
    doctor(env_for(&p, &dir, Some(stranger), Some("payments")), &[])
        .await
        .expect(1)
        .mentions("FAIL  credentials");
    // No token at all.
    doctor(env_for(&p, &dir, None, Some("payments")), &[])
        .await
        .expect(1)
        .mentions("FAIL  token")
        .mentions("pass --token-file");
    // A token file that is not there, and one with a line break in it.
    let missing = run_with(
        env_for(&p, &dir, None, Some("payments")),
        &["doctor", "--token-file", "no-such-file"],
    )
    .await;
    missing.expect(1).mentions("the token file cannot be read");
    dir.write("two-lines", "abc\ndef\n");
    let two = run_with(
        env_for(&p, &dir, None, Some("payments")),
        &["doctor", "--token-file", "two-lines"],
    )
    .await;
    two.expect(1)
        .mentions("characters that cannot travel in a header");
}

#[tokio::test]
async fn a_namespace_that_is_not_there_is_told_with_the_ones_that_are() {
    let p = platform().await;
    let dir = Dir::new();
    let token = p.api.idp.token("alice", "acme").build();
    doctor(
        env_for(&p, &dir, Some(token.clone()), Some("paymnets")),
        &[],
    )
    .await
    .expect(1)
    .mentions("FAIL  namespace")
    .mentions("'paymnets' is not a namespace of this tenant")
    .mentions("it has: payments");
    doctor(env_for(&p, &dir, Some(token), None), &[])
        .await
        .expect(0)
        .mentions("warn  namespace")
        .mentions("none chosen; the tenant has: payments");
}

#[tokio::test]
async fn an_address_that_leads_nowhere_is_told_what_kind_of_nowhere() {
    let p = platform().await;
    let dir = Dir::new();
    let token = p.api.idp.token("alice", "acme").build();
    let with = |server: &str| Env {
        server: Some(server.to_owned()),
        token: Some(token.clone()),
        namespace: Some("payments".into()),
        cwd: dir.0.clone(),
        ..Default::default()
    };
    // Nothing listens.
    doctor(with("http://127.0.0.1:1/v1"), &[])
        .await
        .expect(1)
        .mentions("FAIL  reachable")
        .mentions("connection refused: nothing listens");
    // Something listens, and it is not the API (the prefix is missing: the platform's origin).
    let origin = p.api.base.trim_end_matches("/v1").to_owned();
    doctor(with(&origin), &[])
        .await
        .expect(1)
        .mentions("FAIL  reachable")
        .mentions("the API's address ends in /v1");
    // A name that does not exist, over TLS.
    doctor(with("https://no-such-host.invalid/v1"), &[])
        .await
        .expect(1)
        .mentions("the host name does not resolve (DNS)");
    // A plain-http address that is not this machine is not tried: the token would travel in the clear.
    doctor(with("http://example.com/v1"), &[])
        .await
        .expect(1)
        .mentions("FAIL  address")
        .mentions("must not travel in the clear");
    // No address at all.
    doctor(
        Env {
            cwd: dir.0.clone(),
            ..Env::default()
        },
        &[],
    )
    .await
    .expect(1)
    .mentions("no address for the platform")
    .mentions("SAGAT_SERVER");
}

#[tokio::test]
async fn the_worker_endpoint_and_the_broker_are_checked_when_they_are_named() {
    let p = platform().await;
    let dir = Dir::new();
    let token = p.api.idp.token("alice", "acme").build();
    let env = || env_for(&p, &dir, Some(token.clone()), Some("payments"));
    doctor(env(), &[])
        .await
        .expect(0)
        .mentions("skip  worker")
        .mentions("pass --grpc");
    doctor(env(), &["--grpc", "http://127.0.0.1:1"])
        .await
        .expect(1)
        .mentions("FAIL  worker")
        .mentions("connection refused");
    doctor(env(), &["--nats", "nats://127.0.0.1:1"])
        .await
        .expect(1)
        .mentions("FAIL  broker")
        .mentions("connection refused");
    doctor(env(), &["--nats", "not a url"])
        .await
        .expect(1)
        .mentions("FAIL  broker")
        .mentions("nats://host:4222");
    let silent = fake_broker(None).await;
    doctor(env(), &["--nats", &silent])
        .await
        .expect(0)
        .mentions("warn  broker")
        .mentions("does not greet like NATS");
    doctor(env(), &["--nats", "nats://no-such-host.invalid:4222"])
        .await
        .expect(1)
        .mentions("does not resolve (DNS)");
}

#[tokio::test]
async fn the_command_line_and_the_platform_are_compared_by_release() {
    let p = platform().await;
    let dir = Dir::new();
    let token = p.api.idp.token("alice", "acme").build();
    // The platform of the suite is the release of this workspace: they match, and the check says so.
    let told = doctor(
        env_for(&p, &dir, Some(token), Some("payments")),
        &["--json"],
    )
    .await;
    let report: Value = serde_json::from_str(&told.out).expect("json");
    let versions = report["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .find(|c| c["name"] == "versions")
        .expect("versions");
    assert_eq!(versions["status"], "ok", "{versions}");
    // And what the platform says of itself is the contract.
    let (status, body) = p.rest(reqwest::Method::GET, "/version", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["service"], "sagat-octopus-orchestrator");
    assert_eq!(body["apiVersion"], "v1");
    assert_eq!(body["workerProtocol"]["maximum"], "1.2");
    let _ = cli;
}
