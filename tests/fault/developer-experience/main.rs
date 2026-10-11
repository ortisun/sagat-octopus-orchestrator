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

/// `worker dev` that goes away a second after it has nothing to do: to see what it says first.
async fn serve_idle(p: &Platform, dir: &Dir, engine: &RunningEngine, template: &str) -> Outcome {
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
            "5",
            "--idle-exit",
            "1",
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
        ..Default::default()
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
        ..Default::default()
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
            ..Default::default()
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

// ------------------------------------------------------------------------------------------------
// What the review of the tooling found: each of these failed before it was fixed.

/// The compensation template, with the retries of its compensations cut to one so that a failing
/// compensation reaches a person in a moment.
fn quick_compensations(dir: &Dir) {
    let mut document: Value =
        serde_json::from_str(&dir.read("compensation/workflow.json")).expect("json");
    for node in document["nodes"].as_array_mut().expect("nodes") {
        if node["compensateWith"].is_string() {
            node["compensationRetry"] = serde_json::json!({
                "maxAttempts": 1, "strategy": "fixed", "initialDelayMs": 1, "maxDelayMs": 1, "jitter": false
            });
        }
    }
    dir.write(
        "compensation/workflow.json",
        &serde_json::to_string_pretty(&document).expect("json"),
    );
}

#[tokio::test]
async fn a_run_that_waits_for_a_person_is_reported_at_once_and_not_after_the_whole_timeout() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    cli(&p, &dir, &["new", "compensation"]).await.expect(0);
    quick_compensations(&dir);
    // The card is declined and the undo of the reservation fails for good: nobody can finish it.
    let mocks = dir.read("compensation/mocks.yaml").replace(
        "  limit.release:\n    output: { released: true }",
        "  limit.release:\n    fail: { code: RELEASE_REFUSED, retryable: false }",
    );
    dir.write("compensation/mocks.yaml", &mocks);
    cli(
        &p,
        &dir,
        &["workflow", "publish", "compensation/workflow.json"],
    )
    .await
    .expect(0);
    let started = std::time::Instant::now();
    let (served, ran) = tokio::join!(
        serve(&p, &dir, &engine, "compensation", 4),
        cli(
            &p,
            &dir,
            &[
                "workflow",
                "run",
                "payment-saga",
                "--input",
                r#"{"paymentId":"p-1","amount":10}"#,
                "--wait",
                "--timeout",
                "120",
            ],
        ),
    );
    ran.expect(6)
        .mentions("MANUAL_REVIEW")
        .mentions("waits for a person")
        .mentions("It was not cancelled");
    assert!(
        started.elapsed().as_secs() < 60,
        "it waited {:?}",
        started.elapsed()
    );
    let _ = served;
}

#[tokio::test]
async fn the_cap_of_tasks_is_the_cap_even_when_the_engine_has_many_ready() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    dir.write(
        "fan/workflow.json",
        r#"{
          "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
          "metadata": {"name": "fan-out"}, "entrypoint": "fan",
          "nodes": [
            {"id": "fan", "type": "parallel", "branches": ["a", "b", "c"], "join": "all", "next": "done"},
            {"id": "a", "type": "activity", "activity": "fan.a"},
            {"id": "b", "type": "activity", "activity": "fan.b"},
            {"id": "c", "type": "activity", "activity": "fan.c"},
            {"id": "done", "type": "activity", "activity": "fan.done"}
          ]
        }"#,
    );
    dir.write(
        "fan/mocks.yaml",
        "activities:\n  fan.a: { output: {} }\n  fan.b: { output: {} }\n  fan.c: { output: {} }\n  fan.done: { output: {} }\n",
    );
    cli(&p, &dir, &["workflow", "publish", "fan/workflow.json"])
        .await
        .expect(0);
    let started = cli(&p, &dir, &["workflow", "run", "fan-out"]).await;
    started.expect(0);
    let run = started.run_id();
    // Three tasks are ready at once and the worker was told to take one.
    serve(&p, &dir, &engine, "fan", 1)
        .await
        .expect(0)
        .mentions("served 1 task");
    // Nothing was leased and left to run out: the others are ready for whoever comes next.
    serve(&p, &dir, &engine, "fan", 3)
        .await
        .expect(0)
        .mentions("served 3 tasks");
    within("the run to end", 30, || async {
        (status_of(&p, &run).await == "SUCCEEDED").then_some(())
    })
    .await;
}

#[tokio::test]
async fn the_token_is_not_sent_to_a_worker_endpoint_in_the_clear() {
    let p = platform().await;
    let dir = Dir::new();
    publish(&p, &dir, "hello-world").await;
    // Nothing is listening there and nothing is tried: the address is refused for what it is.
    for address in [
        "http://203.0.113.9:19000",
        "http://example.com:19000",
        "http://[2001:db8::1]:19000",
    ] {
        let refused = cli(
            &p,
            &dir,
            &[
                "worker",
                "dev",
                "--mocks",
                "hello-world/mocks.yaml",
                "--workflow",
                "hello-world/workflow.json",
                "--grpc",
                address,
            ],
        )
        .await;
        refused
            .expect(1)
            .mentions("the token must not travel in the clear");
    }
}

#[tokio::test]
async fn a_workflow_from_somewhere_else_cannot_make_the_tool_bind_a_pattern() {
    let p = platform().await;
    let dir = Dir::new();
    publish(&p, &dir, "hello-world").await;
    let mut document: Value =
        serde_json::from_str(&dir.read("hello-world/workflow.json")).expect("json");
    document["nodes"][0]["activity"] = serde_json::json!("greeting.*");
    dir.write(
        "hello-world/workflow.json",
        &serde_json::to_string(&document).expect("json"),
    );
    cli(
        &p,
        &dir,
        &["worker", "bind", "--workflow", "hello-world/workflow.json"],
    )
    .await
    .expect(1)
    .mentions("never to patterns")
    .omits("bound alice");
    // And the identity was left as it was: it can still serve what it was bound to before.
    document["nodes"][0]["activity"] = serde_json::json!("*");
    dir.write(
        "hello-world/workflow.json",
        &serde_json::to_string(&document).expect("json"),
    );
    cli(
        &p,
        &dir,
        &["worker", "bind", "--workflow", "hello-world/workflow.json"],
    )
    .await
    .expect(1)
    .mentions("never to patterns");
}

#[tokio::test]
async fn canned_answers_are_given_to_a_dev_namespace_and_to_no_other_unless_asked() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    let workspace = p.api.create_workspace("alice", "acme", "staging-ws").await;
    p.api
        .create_namespace("alice", "acme", &workspace, "stage", "staging")
        .await;
    publish(&p, &dir, "hello-world").await;
    let serve_in_stage = |extra: &'static [&'static str]| {
        let (p, dir, grpc) = (&p, &dir, engine.grpc_url.clone());
        async move {
            let mut args = vec![
                "--namespace",
                "stage",
                "worker",
                "dev",
                "--mocks",
                "hello-world/mocks.yaml",
                "--workflow",
                "hello-world/workflow.json",
                "--grpc",
                grpc.as_str(),
                "--max-tasks",
                "1",
                "--idle-exit",
                "1",
            ];
            args.extend_from_slice(extra);
            cli(p, dir, &args).await
        }
    };
    serve_in_stage(&[])
        .await
        .expect(1)
        .mentions("staging namespace")
        .mentions("--allow-non-dev")
        .omits("serving");
    cli(
        &p,
        &dir,
        &[
            "--namespace",
            "stage",
            "worker",
            "bind",
            "--workflow",
            "hello-world/workflow.json",
        ],
    )
    .await
    .expect(1)
    .mentions("--allow-non-dev");
    // Asked for, it is done, and said.
    cli(
        &p,
        &dir,
        &[
            "--namespace",
            "stage",
            "worker",
            "bind",
            "--workflow",
            "hello-world/workflow.json",
            "--allow-non-dev",
        ],
    )
    .await
    .expect(0)
    .mentions("is staging");
    // The development namespace is as it was.
    serve_idle(&p, &dir, &engine, "hello-world")
        .await
        .expect(0)
        .mentions("(dev)");
}

#[tokio::test]
async fn an_answer_in_the_wrong_section_of_the_mock_file_is_said_and_not_taken_for_another() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    publish(&p, &dir, "compensation").await;
    // The refund is the undo of the charge, and the file gave it as if it were a step of the workflow.
    let mocks = "activities:
  limit.reserve: { output: { reservationId: r-1 } }
  payment.charge: { output: { chargeId: ch-1 } }
  payment.refund: { fail: { code: REFUND_REFUSED, retryable: false } }
  customer.notify: { echo: true }
compensations:
  limit.release: { output: { released: true } }
reconcilers:
  payment.find-charge: { effect: not_applied }
";
    dir.write("compensation/mocks.yaml", mocks);
    let told = serve_idle(&p, &dir, &engine, "compensation").await;
    told.expect(0)
        .mentions("'payment.refund' undoes a step and has no answer under compensations")
        .mentions("the file has an answer for it under activities");
}

#[tokio::test]
async fn a_worker_with_nothing_to_do_goes_when_it_was_told_to_and_an_activity_without_an_answer_fails_for_good()
 {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    publish(&p, &dir, "hello-world").await;
    // --idle-exit: nothing comes, and it says so and ends well.
    let tasks = "5".to_owned();
    let idle = cli(
        &p,
        &dir,
        &[
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
            &tasks,
            "--idle-exit",
            "1",
        ],
    )
    .await;
    idle.expect(0)
        .mentions("no task for 1 s: stopping")
        .mentions("served 0 tasks");

    // The file answers compose and not deliver: deliver fails for good, as NO_MOCK, and the tool said so
    // before it began.
    dir.write(
        "hello-world/mocks.yaml",
        "activities:\n  greeting.compose:\n    output: { greeting: Hello }\n",
    );
    let serve_args = [
        "worker",
        "dev",
        "--mocks",
        "hello-world/mocks.yaml",
        "--workflow",
        "hello-world/workflow.json",
        "--bind",
        "--show-payloads",
        "--grpc",
        engine.grpc_url.as_str(),
        "--max-tasks",
        "2",
        "--idle-exit",
        "30",
    ];
    let run_args = [
        "workflow",
        "run",
        "hello-world",
        "--input",
        r#"{"name":"Ada"}"#,
        "--wait",
        "--timeout",
        "60",
    ];
    let (served, ran) = tokio::join!(cli(&p, &dir, &serve_args), cli(&p, &dir, &run_args));
    ran.expect(1).mentions("FAILED").mentions("NO_MOCK");
    served
        .expect(0)
        .mentions("no answer for 'greeting.deliver' under activities")
        .mentions("fails [NO_MOCK]")
        // --show-payloads: what the worker was given, which is the run's input and the outputs so far.
        .mentions(r#""name":"Ada""#);
}

#[tokio::test]
async fn a_worker_told_to_stop_stops_whatever_it_is_doing() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    publish(&p, &dir, "hello-world").await;
    // Every answer takes eight seconds, so that the worker can be told to stop while it is in the
    // middle of one, while it is waiting for work, and while it is asking for it.
    let mocks = dir.read("hello-world/mocks.yaml");
    dir.write("hello-world/mocks.yaml", &format!("delayMs: 8000\n{mocks}"));
    let env = |dir: &Dir| Env {
        server: Some(p.api.base.clone()),
        token: Some(p.api.idp.token("alice", "acme").build()),
        namespace: Some("payments".to_owned()),
        cwd: dir.0.clone(),
        ..Default::default()
    };
    let words: Vec<String> = [
        "worker",
        "dev",
        "--mocks",
        "hello-world/mocks.yaml",
        "--workflow",
        "hello-world/workflow.json",
        "--bind",
        "--grpc",
        &engine.grpc_url,
        "--idle-exit",
        "300",
    ]
    .iter()
    .map(|w| (*w).to_owned())
    .collect();

    // 1. Idle: nothing to do, and told to stop at several moments of the polling and the waiting.
    for after_ms in [50, 130, 380, 700, 1300] {
        let (tell, hear) = tokio::sync::watch::channel(false);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let environment = env(&dir);
        let running = sagat_cli::run_until(&words, &environment, &mut out, &mut err, hear);
        let stopper = async {
            tokio::time::sleep(std::time::Duration::from_millis(after_ms)).await;
            tell.send(true).expect("send");
            std::time::Instant::now()
        };
        let (code, told_at) = tokio::join!(
            async { tokio::time::timeout(std::time::Duration::from_secs(10), running).await },
            stopper
        );
        let code = code.unwrap_or_else(|_| panic!("it did not stop {after_ms} ms in"));
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        assert!(
            told_at.elapsed().as_secs() < 2,
            "told at {after_ms} ms, it took {:?} to end",
            told_at.elapsed()
        );
        assert!(
            String::from_utf8_lossy(&out).contains("stopping"),
            "{}",
            String::from_utf8_lossy(&out)
        );
    }

    // 2. In the middle of an answer that takes eight seconds: it does not wait for it, and leaves the
    // task to its lease as a worker that went away would.
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
    let (tell, hear) = tokio::sync::watch::channel(false);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let environment = env(&dir);
    let running = sagat_cli::run_until(&words, &environment, &mut out, &mut err, hear);
    let began = std::time::Instant::now();
    let stopper = async {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        tell.send(true).expect("send");
    };
    let (code, ()) = tokio::join!(running, stopper);
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
    assert!(
        began.elapsed().as_secs() < 6,
        "it waited for the answer: {:?}",
        began.elapsed()
    );
    let said = String::from_utf8_lossy(&out).into_owned();
    assert!(
        said.contains("stopping") && said.contains("served 0 tasks"),
        "{said}"
    );
}
