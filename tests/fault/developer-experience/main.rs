//! What goes wrong on the first day and around the tooling, against a real platform (PostgreSQL,
//! the HTTP API, the engine as a process): a worker that stops and starts again, a run that is
//! waited for in vain, the same request twice, another tenant, and the token (`NFR-012-DUR`,
//! `NFR-012-SEC`, `T012-022`, `T012-027`).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

use reqwest::Method;
use sagat_cli::Env;
use serde_json::Value;

use support::dx::{Dir, Outcome, cli, engine, rest, run_with, within};
use support::worker::{Platform, RunningEngine, platform};

async fn publish(p: &Platform, dir: &Dir, template: &str) {
    cli(p, dir, &["new", template]).await.expect(0);
    cli(
        p,
        dir,
        &["workflow", "publish", &format!("{template}/workflow.json")],
    )
    .await
    .expect(0);
}

/// `worker dev` for `tasks` tasks.
async fn serve(
    p: &Platform,
    dir: &Dir,
    engine: &RunningEngine,
    template: &str,
    tasks: u64,
) -> Outcome {
    let tasks = tasks.to_string();
    let mocks = format!("{template}/mocks.yaml");
    let workflow = format!("{template}/workflow.json");
    cli(
        p,
        dir,
        &[
            "worker",
            "dev",
            "--mocks",
            &mocks,
            "--workflow",
            &workflow,
            "--bind",
            "--grpc",
            &engine.grpc_url,
            "--max-tasks",
            &tasks,
            "--idle-exit",
            "40",
        ],
    )
    .await
}

async fn status_of(p: &Platform, run: &str) -> String {
    let (_, body) = rest(p, Method::GET, &format!("/runs/{run}"), None).await;
    body["status"].as_str().unwrap_or("?").to_owned()
}

async fn journal(p: &Platform, run: &str) -> Vec<String> {
    let (_, body) = rest(
        p,
        Method::GET,
        &format!("/runs/{run}/events?limit=200"),
        None,
    )
    .await;
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|e| {
            format!(
                "{} {}",
                e["eventType"].as_str().unwrap_or("?"),
                e["stepId"].as_str().unwrap_or("-")
            )
        })
        .collect()
}

#[tokio::test]
async fn a_worker_that_stops_and_starts_again_leaves_the_run_where_it_was_and_it_goes_on() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    publish(&p, &dir, "hello-world").await;
    let started = cli(
        &p,
        &dir,
        &[
            "workflow",
            "run",
            "hello-world",
            "--input",
            r#"{"name":"Ada"}"#,
        ],
    )
    .await;
    started.expect(0);
    let run = started.run_id();

    // The first worker takes one task and goes away; the run has done half of its work and waits.
    serve(&p, &dir, &engine, "hello-world", 1)
        .await
        .expect(0)
        .mentions("greeting.compose  completes")
        .mentions("served 1 task");
    within("deliver to be waiting", 30, || async {
        let (_, graph) = rest(&p, Method::GET, &format!("/runs/{run}/graph"), None).await;
        graph["nodes"]
            .as_array()?
            .iter()
            .any(|n| n["id"] == "compose" && n["status"] == "SUCCEEDED")
            .then_some(())
    })
    .await;
    assert_eq!(status_of(&p, &run).await, "RUNNING");

    // Another one finishes it: nothing was lost and compose did not run again.
    serve(&p, &dir, &engine, "hello-world", 1)
        .await
        .expect(0)
        .mentions("greeting.deliver  completes")
        .omits("greeting.compose");
    within("the run to end", 30, || async {
        let status = status_of(&p, &run).await;
        (status == "SUCCEEDED").then_some(())
    })
    .await;
    let events = journal(&p, &run).await;
    assert_eq!(
        events
            .iter()
            .filter(|e| *e == "step.completed compose")
            .count(),
        1,
        "{events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| *e == "step.completed deliver")
            .count(),
        1,
        "{events:?}"
    );
}

#[tokio::test]
async fn a_run_waited_for_in_vain_is_not_cancelled_and_a_worker_that_comes_later_finishes_it() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    publish(&p, &dir, "hello-world").await;
    let waited = cli(
        &p,
        &dir,
        &[
            "workflow",
            "run",
            "hello-world",
            "--input",
            r#"{"name":"Ada"}"#,
            "--wait",
            "--timeout",
            "1",
        ],
    )
    .await;
    waited
        .expect(6)
        .mentions("It was not cancelled")
        .mentions("sagat workflow inspect");
    let run = waited.run_id();
    assert_eq!(status_of(&p, &run).await, "RUNNING");

    serve(&p, &dir, &engine, "hello-world", 2).await.expect(0);
    within("the run to end", 30, || async {
        let status = status_of(&p, &run).await;
        (status == "SUCCEEDED").then_some(())
    })
    .await;
}

#[tokio::test]
async fn the_same_request_twice_is_the_same_run_and_the_same_key_for_another_input_is_refused() {
    let p = platform().await;
    let dir = Dir::new();
    publish(&p, &dir, "hello-world").await;

    // Publishing what is already published changes nothing.
    cli(
        &p,
        &dir,
        &["workflow", "publish", "hello-world/workflow.json"],
    )
    .await
    .expect(0)
    .mentions("0 published")
    .mentions("1 unchanged");

    let run = |input: &'static str| {
        let (p, dir) = (&p, &dir);
        async move {
            cli(
                p,
                dir,
                &[
                    "workflow",
                    "run",
                    "hello-world",
                    "--input",
                    input,
                    "--key",
                    "dx-fault-1",
                ],
            )
            .await
        }
    };
    let first = run(r#"{"name":"Ada"}"#).await;
    first.expect(0);
    let again = run(r#"{"name":"Ada"}"#).await;
    again.expect(0);
    assert_eq!(
        first.run_id(),
        again.run_id(),
        "a repeat of the key started another run"
    );

    // The key is Ada's; Grace cannot have the same.
    let other = run(r#"{"name":"Grace"}"#).await;
    other.expect(1).mentions("CONFLICT");
    let (_, runs) = rest(&p, Method::GET, "/runs?limit=50", None).await;
    assert_eq!(runs["items"].as_array().expect("items").len(), 1, "{runs}");
}

fn env_of(p: &Platform, subject: &str, tenant: &str, dir: &Dir) -> Env {
    Env {
        server: Some(p.api.base.clone()),
        token: Some(p.api.idp.token(subject, tenant).build()),
        namespace: Some("payments".to_owned()),
        cwd: dir.0.clone(),
    }
}

#[tokio::test]
async fn another_tenant_cannot_see_run_bind_or_serve_what_is_not_its_own() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    publish(&p, &dir, "hello-world").await;
    let acme = cli(
        &p,
        &dir,
        &[
            "workflow",
            "run",
            "hello-world",
            "--input",
            r#"{"name":"Ada"}"#,
        ],
    )
    .await;
    acme.expect(0);
    let run = acme.run_id();

    // Bob administers globex. Its tenant has no namespace payments: nothing of acme's is there to run,
    // inspect, cancel or bind, and what the platform says does not describe acme's.
    let bob = |args: Vec<String>| {
        let env = env_of(&p, "bob", "globex", &dir);
        async move {
            let words: Vec<&str> = args.iter().map(String::as_str).collect();
            run_with(env, &words).await
        }
    };
    let strings = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    for args in [
        strings(&["workflow", "run", "hello-world"]),
        strings(&["workflow", "inspect", &run]),
        strings(&["workflow", "cancel", &run, "--reason", "not mine"]),
        strings(&["worker", "bind", "greeting.compose"]),
        strings(&[
            "worker",
            "dev",
            "--mocks",
            "hello-world/mocks.yaml",
            "--workflow",
            "hello-world/workflow.json",
            "--bind",
            "--grpc",
            &engine.grpc_url,
            "--max-tasks",
            "1",
            "--idle-exit",
            "2",
        ]),
    ] {
        let answer = bob(args.clone()).await;
        answer.expect(1);
        assert!(
            !answer.all().contains("Hello")
                && !answer.all().contains("alice")
                && !answer.all().contains("acme"),
            "{args:?} told bob about acme: {}",
            answer.all()
        );
    }
    // And acme's run is as it was: bob did not cancel it, and his bind did not take the activities.
    assert_eq!(status_of(&p, &run).await, "RUNNING");
    serve(&p, &dir, &engine, "hello-world", 2)
        .await
        .expect(0)
        .mentions("served 2 tasks");
    within("the run to end", 30, || async {
        (status_of(&p, &run).await == "SUCCEEDED").then_some(())
    })
    .await;
}

#[tokio::test]
async fn no_command_prints_the_token_not_when_it_works_and_not_when_it_fails() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    let token = p.api.idp.token("alice", "acme").build();
    let parts: Vec<&str> = token.split('.').collect();
    let env = |server: Option<&str>, namespace: &str| Env {
        server: server.map(str::to_owned),
        token: Some(token.clone()),
        namespace: Some(namespace.to_owned()),
        cwd: dir.0.clone(),
    };
    let base = p.api.base.as_str();
    let mut said: Vec<(String, Outcome)> = Vec::new();
    let run = |label: &str, env: Env, args: &[&str]| {
        let label = label.to_owned();
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        async move {
            let words: Vec<&str> = args.iter().map(String::as_str).collect();
            (label, run_with(env, &words).await)
        }
    };
    said.push(run("new", env(Some(base), "payments"), &["new", "hello-world"]).await);
    said.push(
        run(
            "validate",
            env(Some(base), "payments"),
            &["workflow", "validate", "hello-world/workflow.json"],
        )
        .await,
    );
    said.push(
        run(
            "publish",
            env(Some(base), "payments"),
            &["workflow", "publish", "hello-world/workflow.json"],
        )
        .await,
    );
    said.push(
        run(
            "bind",
            env(Some(base), "payments"),
            &["worker", "bind", "--workflow", "hello-world/workflow.json"],
        )
        .await,
    );
    said.push(
        run(
            "doctor",
            env(Some(base), "payments"),
            &["doctor", "--grpc", &engine.grpc_url],
        )
        .await,
    );
    said.push(
        run(
            "run",
            env(Some(base), "payments"),
            &["workflow", "run", "hello-world", "--input", "{}"],
        )
        .await,
    );
    // The ways it fails.
    said.push(
        run(
            "no namespace",
            env(Some(base), "nowhere"),
            &["workflow", "run", "hello-world"],
        )
        .await,
    );
    said.push(
        run(
            "no server",
            env(Some("http://127.0.0.1:1/v1"), "payments"),
            &["workflow", "run", "hello-world"],
        )
        .await,
    );
    said.push(
        run(
            "clear http",
            env(Some("http://example.com/v1"), "payments"),
            &["workflow", "run", "hello-world"],
        )
        .await,
    );
    said.push(
        run(
            "no workflow",
            env(Some(base), "payments"),
            &["workflow", "run", "missing-workflow"],
        )
        .await,
    );
    said.push(
        run(
            "bad run id",
            env(Some(base), "payments"),
            &["workflow", "inspect", "not-a-run"],
        )
        .await,
    );
    said.push(
        run(
            "dev token",
            env(Some(base), "payments"),
            &["dev", "token", "--idp", "http://127.0.0.1:1"],
        )
        .await,
    );
    // The ones that should work did, and the ones that should fail did: the absence of the token below
    // is not the absence of a command.
    for (label, outcome) in &said {
        let works =
            ["new", "validate", "publish", "bind", "doctor", "run"].contains(&label.as_str());
        assert_eq!(outcome.code == 0, works, "{label}:\n{}", outcome.all());
    }
    for (label, outcome) in &said {
        let all = outcome.all();
        assert!(!all.contains(&token), "{label} printed the token:\n{all}");
        for part in &parts {
            assert!(
                !all.contains(part),
                "{label} printed a part of the token:\n{all}"
            );
        }
    }
    // And the token in a file, wrong: the file's content is not shown either.
    dir.write("token.txt", &format!("{token}x y"));
    let wrong = run_with(
        Env {
            token: None,
            ..env(Some(base), "payments")
        },
        &["doctor", "--token-file", "token.txt"],
    )
    .await;
    assert!(!wrong.all().contains(parts[1]), "{}", wrong.all());
}

#[tokio::test]
async fn the_platform_being_unreachable_is_said_at_once_and_a_run_is_not_started() {
    let p = platform().await;
    let dir = Dir::new();
    publish(&p, &dir, "hello-world").await;
    let started = std::time::Instant::now();
    let nothing = run_with(
        Env {
            server: Some("http://127.0.0.1:1/v1".to_owned()),
            token: Some(p.api.idp.token("alice", "acme").build()),
            namespace: Some("payments".to_owned()),
            cwd: dir.0.clone(),
        },
        &[
            "workflow",
            "run",
            "hello-world",
            "--wait",
            "--timeout",
            "30",
        ],
    )
    .await;
    nothing.expect(1);
    assert!(
        started.elapsed().as_secs() < 15,
        "it waited {:?}",
        started.elapsed()
    );
    let (_, runs) = rest(&p, Method::GET, "/runs?limit=10", None).await;
    assert_eq!(runs["items"].as_array().map(Vec::len), Some(0), "{runs}");
    let _: &Value = &runs;
}
