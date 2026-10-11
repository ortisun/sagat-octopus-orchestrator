//! US01, US02 and US03's templates: a first workflow in minutes, a saga rehearsed against a mock
//! provider, and every template doing what it says (US01-AC01, US02-AC01, FR-012-004).

use reqwest::Method;
use serde_json::{Value, json};

use crate::support::worker::{Platform, RunningEngine, platform};
use crate::{Dir, Outcome, cli, engine, rest, within};

/// `worker dev` for `tasks` tasks, with the mocks and the workflow in `dir`.
async fn worker(
    p: &Platform,
    dir: &Dir,
    engine: &RunningEngine,
    template: &str,
    tasks: u64,
) -> Outcome {
    cli(
        p,
        dir,
        &[
            "worker",
            "dev",
            "--mocks",
            &format!("{template}/mocks.yaml"),
            "--workflow",
            &format!("{template}/workflow.json"),
            "--bind",
            "--grpc",
            &engine.grpc_url,
            "--max-tasks",
            &tasks.to_string(),
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
        .filter_map(|e| e["eventType"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn us01_following_the_quickstart_a_two_step_workflow_runs_and_can_be_seen() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();

    // Before anything else: is this machine ready?
    cli(&p, &dir, &["doctor", "--grpc", &engine.grpc_url])
        .await
        .expect(0)
        .mentions("credentials")
        .mentions("nothing to fix");

    // From a template to a published version.
    cli(&p, &dir, &["new", "hello-world"])
        .await
        .expect(0)
        .mentions("wrote the hello-world template");
    let file = "hello-world/workflow.json";
    cli(&p, &dir, &["workflow", "validate", file])
        .await
        .expect(0)
        .mentions("ok    hello-world");
    cli(&p, &dir, &["workflow", "publish", file])
        .await
        .expect(0)
        .mentions("published  hello-world v1");
    cli(&p, &dir, &["workflow", "publish", file])
        .await
        .expect(0)
        .mentions("0 published")
        .mentions("1 unchanged");

    // Served with canned answers, and run to the end.
    let (served, ran) = tokio::join!(
        worker(&p, &dir, &engine, "hello-world", 2),
        cli(
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
                "60"
            ]
        ),
    );
    ran.expect(0)
        .mentions("SUCCEEDED")
        .mentions("journal (")
        .mentions("attempts:");
    served
        .expect(0)
        .mentions("greeting.compose  completes")
        .mentions("greeting.deliver  completes")
        .mentions("served 2 tasks");
    let run = ran.run_id();

    // What the Studio draws is what the API says: the run, its graph and its journal.
    assert_eq!(status_of(&p, &run).await, "SUCCEEDED");
    let (status, graph) = rest(&p, Method::GET, &format!("/runs/{run}/graph"), None).await;
    assert_eq!(status, 200, "{graph}");
    let nodes: Vec<&str> = graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert!(
        nodes.contains(&"compose") && nodes.contains(&"deliver"),
        "{nodes:?}"
    );
    let events = journal(&p, &run).await;
    assert_eq!(
        events.iter().filter(|e| *e == "step.completed").count(),
        2,
        "{events:?}"
    );
    assert!(events.contains(&"run.finished".to_owned()), "{events:?}");

    // And the tool can read it back.
    cli(&p, &dir, &["workflow", "runs", "hello-world"])
        .await
        .expect(0)
        .mentions(&run)
        .mentions("SUCCEEDED");
    cli(&p, &dir, &["workflow", "inspect", &run])
        .await
        .expect(0)
        .mentions("compose  #1  succeeded")
        .mentions("deliver  #1  succeeded")
        .mentions("run.finished");
}

#[tokio::test]
async fn us01_a_run_that_waits_for_a_worker_says_so_instead_of_hanging_and_can_be_cancelled() {
    let p = platform().await;
    let _engine = engine(&p).await;
    let dir = Dir::new();
    publish(&p, &dir, "hello-world").await;
    // Nobody serves the activities: the wait ends at the time it was given, and says what to do.
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
            "2",
        ],
    )
    .await;
    waited
        .expect(6)
        .mentions("worker that serves its activities")
        .mentions("It was not cancelled");
    let run = waited.run_id();
    cli(
        &p,
        &dir,
        &[
            "workflow",
            "cancel",
            &run,
            "--reason",
            "nobody is serving this yet",
        ],
    )
    .await
    .expect(0)
    .mentions("accepted");
    within("the run to be cancelled", 20, || async {
        (status_of(&p, &run).await == "CANCELLED").then_some(())
    })
    .await;
    // The same input and key start the same run once.
    let first = cli(
        &p,
        &dir,
        &[
            "workflow",
            "run",
            "hello-world",
            "--input",
            "{\"name\":\"x\"}",
            "--key",
            "same-key-0123456789",
        ],
    )
    .await;
    let again = cli(
        &p,
        &dir,
        &[
            "workflow",
            "run",
            "hello-world",
            "--input",
            "{\"name\":\"x\"}",
            "--key",
            "same-key-0123456789",
        ],
    )
    .await;
    assert_eq!(
        first.run_id(),
        again.run_id(),
        "a repeat with the key is the same run"
    );
}

#[tokio::test]
async fn us02_a_payment_saga_is_rehearsed_against_a_mock_provider_that_fails_on_demand() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    publish(&p, &dir, "compensation").await;
    let input = r#"{"paymentId":"p-1","amount":10}"#;

    // 1. The provider declines the card for good: the reservation is undone.
    let run_args = [
        "workflow",
        "run",
        "payment-saga",
        "--input",
        input,
        "--wait",
        "--timeout",
        "60",
    ];
    let (served, ran) = tokio::join!(
        worker(&p, &dir, &engine, "compensation", 3),
        cli(&p, &dir, &run_args),
    );
    ran.expect(1)
        .mentions("COMPENSATED")
        .mentions("reserve~c")
        .mentions("[CARD_DECLINED]");
    served
        .expect(0)
        .mentions("payment.charge  fails [CARD_DECLINED]")
        .mentions("limit.release (compensation)  completes");
    let declined = ran.run_id();
    let events = journal(&p, &declined).await;
    assert!(
        events.iter().any(|e| e.starts_with("compensation.")),
        "the journal shows the compensation: {events:?}"
    );

    // 2. The provider times out once and then works: the platform's own retry carries the run.
    let mocks = dir.read("compensation/mocks.yaml").replace(
        "    fail: { code: CARD_DECLINED, retryable: false, message: \"the card was declined\" }\n",
        "    sequence:\n      - fail: { code: PROVIDER_TIMEOUT, retryable: true }\n      - output: { chargeId: ch-1 }\n",
    );
    assert!(
        mocks.contains("PROVIDER_TIMEOUT"),
        "the mock file was edited"
    );
    dir.write("compensation/mocks.yaml", &mocks);
    let run_args = [
        "workflow",
        "run",
        "payment-saga",
        "--input",
        input,
        "--wait",
        "--timeout",
        "60",
    ];
    let (served, ran) = tokio::join!(
        worker(&p, &dir, &engine, "compensation", 4),
        cli(&p, &dir, &run_args),
    );
    ran.expect(0).mentions("SUCCEEDED");
    served
        .expect(0)
        .mentions("payment.charge  fails [PROVIDER_TIMEOUT] (retryable)")
        .mentions("payment.charge  completes");
    cli(&p, &dir, &["workflow", "inspect", &ran.run_id()])
        .await
        .expect(0)
        .mentions("charge  #2  succeeded");
}

#[tokio::test]
async fn us02_a_lost_answer_is_reconciled_before_anything_is_repeated() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    publish(&p, &dir, "compensation").await;
    // The charge is taken by a worker that dies with it: its answer is lost. The platform asks
    // the reconciler whether it happened; here it did not, so the charge is attempted again.
    let mocks = dir.read("compensation/mocks.yaml").replace(
        "    fail: { code: CARD_DECLINED, retryable: false, message: \"the card was declined\" }\n",
        "    sequence:\n      - lose: true\n      - output: { chargeId: ch-1 }\n",
    );
    dir.write("compensation/mocks.yaml", &mocks);
    let input = r#"{"paymentId":"p-2","amount":10}"#;

    let driver = async {
        let started = cli(
            &p,
            &dir,
            &["workflow", "run", "payment-saga", "--input", input],
        )
        .await;
        started.expect(0);
        let run = started.run_id();
        // Once the worker holds the charge and says nothing, its lease is made to run out (the
        // engine would wait out the real one).
        within("the charge to be taken", 30, || async {
            let (_, attempts) = rest(&p, Method::GET, &format!("/runs/{run}/attempts"), None).await;
            attempts["items"]
                .as_array()?
                .iter()
                .any(|a| a["node"] == "charge" && a["outcome"] == "open")
                .then_some(())
        })
        .await;
        p.expire_lease(&run, "charge").await;
        within("the run to end", 40, || async {
            let status = status_of(&p, &run).await;
            ["SUCCEEDED", "FAILED", "COMPENSATED"]
                .contains(&status.as_str())
                .then_some(status)
        })
        .await;
        run
    };
    let (served, run) = tokio::join!(worker(&p, &dir, &engine, "compensation", 5), driver);
    served
        .expect(0)
        .mentions("payment.charge  takes the task and never answers")
        .mentions("payment.find-charge (reconciliation)  answers: not_applied");
    assert_eq!(status_of(&p, &run).await, "SUCCEEDED");
    let events = journal(&p, &run).await;
    let at = |kind: &str| {
        events
            .iter()
            .position(|e| e == kind)
            .unwrap_or_else(|| panic!("no {kind} in {events:?}"))
    };
    assert!(
        at("reconciliation.requested") < at("reconciliation.concluded"),
        "{events:?}"
    );
}

#[tokio::test]
async fn us03_the_event_template_starts_a_run_for_each_event_and_a_repeat_starts_nothing() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    publish(&p, &dir, "event-trigger").await;
    let send = |id: &'static str| {
        let p = &p;
        async move {
            rest(
                p,
                Method::POST,
                "/events",
                Some(&json!({"type": "order.created", "eventId": id, "data": {"orderId": id}})),
            )
            .await
        }
    };
    let (served, _) = tokio::join!(worker(&p, &dir, &engine, "event-trigger", 1), async {
        let (status, _) = send("order-1").await;
        assert_eq!(status, 202);
        let (status, again) = send("order-1").await;
        assert_eq!(status, 202);
        assert_eq!(again["status"], "DUPLICATE");
        within("the event to start a run", 30, || async {
            let runs = cli(&p, &dir, &["workflow", "runs", "on-order-created"]).await;
            runs.out.contains("SUCCEEDED").then_some(())
        })
        .await;
    });
    served.expect(0).mentions("orders.handle  completes");
    let runs = cli(&p, &dir, &["workflow", "runs", "on-order-created"]).await;
    assert_eq!(
        runs.out.lines().count(),
        1,
        "one event, one run: {}",
        runs.out
    );
}

#[tokio::test]
async fn us03_the_approval_template_waits_for_a_decision_and_acts_on_it() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    publish(&p, &dir, "approval").await;
    for (request, approved, expected) in [
        ("req-1", true, "changes.apply"),
        ("req-2", false, "changes.decline"),
    ] {
        let driver = async {
            let started = cli(
                &p,
                &dir,
                &[
                    "workflow",
                    "run",
                    "change-approval",
                    "--input",
                    &json!({"requestId": request}).to_string(),
                ],
            )
            .await;
            started.expect(0);
            let run = started.run_id();
            within("the run to wait for the decision", 30, || async {
                (status_of(&p, &run).await == "WAITING").then_some(())
            })
            .await;
            let (status, _) = rest(
                &p,
                Method::POST,
                "/events",
                Some(&json!({"type": "approval.decided", "eventId": format!("decided-{request}"), "correlationKey": request, "data": {"approved": approved}})),
            )
            .await;
            assert_eq!(status, 202);
            within("the run to end", 30, || async {
                (status_of(&p, &run).await == "SUCCEEDED").then_some(())
            })
            .await;
        };
        let (served, ()) = tokio::join!(worker(&p, &dir, &engine, "approval", 2), driver);
        served.expect(0).mentions(&format!("{expected}  completes"));
    }
}

#[tokio::test]
async fn us03_the_cron_and_script_templates_publish_the_way_they_say() {
    let p = platform().await;
    let dir = Dir::new();
    publish(&p, &dir, "cron").await;
    let schedules: i64 =
        sqlx::query_scalar("SELECT count(*) FROM workflow_triggers WHERE kind = 'cron'")
            .fetch_one(&p.api.db.pool)
            .await
            .expect("count");
    assert_eq!(
        schedules, 1,
        "the schedule is registered when the workflow is published"
    );

    // The script template is valid as a document and is refused as a publication until the
    // script it pins exists: the platform has no script with the placeholder digest.
    cli(&p, &dir, &["new", "inline-script"]).await.expect(0);
    cli(
        &p,
        &dir,
        &["workflow", "validate", "inline-script/workflow.json"],
    )
    .await
    .expect(0)
    .mentions("ok    enrich-order");
    let refused = cli(
        &p,
        &dir,
        &["workflow", "publish", "inline-script/workflow.json"],
    )
    .await;
    refused.expect(1).mentions("UNKNOWN_SCRIPT");
}

#[tokio::test]
async fn us03_every_template_publishes_as_the_workflow_its_readme_names() {
    let p = platform().await;
    let dir = Dir::new();
    for (template, name) in [
        ("hello-world", "hello-world"),
        ("event-trigger", "on-order-created"),
        ("cron", "nightly-report"),
        ("approval", "change-approval"),
        ("compensation", "payment-saga"),
    ] {
        publish(&p, &dir, template).await;
        let readme = dir.read(&format!("{template}/README.md"));
        assert!(
            readme.contains(name),
            "the README of {template} names {name}"
        );
        let (_, page) = rest(&p, Method::GET, "/workflows?limit=100", None).await;
        let found: Vec<&str> = page["items"]
            .as_array()
            .expect("items")
            .iter()
            .filter_map(|w| w["name"].as_str())
            .collect();
        assert!(found.contains(&name), "{name} in {found:?}");
    }
    let _: Value = json!(null);
}
