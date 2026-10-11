//! US01–US04 of `specs/002-durable-execution/spec.md`, end to end: definitions and runs over REST,
//! the worker protocol over gRPC, the scheduler as the engine runs it, real PostgreSQL.

use futures_util::future::join_all;
use reqwest::Method;
use serde_json::{Value, json};

use crate::support::worker::{Platform, Worker, chain_ir, machine_code, platform};

async fn journal(p: &Platform, run: &str) -> Vec<(String, Option<String>)> {
    let (status, page) = p
        .rest(Method::GET, &format!("/runs/{run}/events?limit=200"), None)
        .await;
    assert_eq!(status, 200, "{page}");
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|event| {
            (
                event["eventType"].as_str().expect("type").to_owned(),
                event["stepId"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

fn count(events: &[(String, Option<String>)], kind: &str, step: Option<&str>) -> usize {
    events
        .iter()
        .filter(|(k, s)| k == kind && step.is_none_or(|wanted| s.as_deref() == Some(wanted)))
        .count()
}

async fn graph(p: &Platform, run: &str) -> Value {
    let (status, body) = p
        .rest(Method::GET, &format!("/runs/{run}/graph"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    body
}

fn node_status(graph: &Value, id: &str) -> String {
    graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["id"] == id)
        .and_then(|n| n["status"].as_str())
        .unwrap_or_else(|| panic!("no node {id} in {graph}"))
        .to_owned()
}

/// US01-AC01: a published definition with A→B; the client starts the run; A and B are executed and
/// the checkpoints and the journal record the progress.
#[tokio::test]
async fn us01_a_dag_runs_to_completion_and_every_step_is_checkpointed() {
    let p = platform().await;
    let wf = p.publish("order-flow", &chain_ir("order-flow")).await;
    let run = p.start(&wf, &json!({"orderId": "o-77"})).await;
    let before = journal(&p, &run).await;
    assert_eq!(count(&before, "run.started", None), 1);

    let mut w = p.worker("wendy").await;
    w.register(&["act.a", "act.b"]).await;
    let a = w.one_task().await;
    assert_eq!(a.activity_name, "act.a");
    let mid = graph(&p, &run).await;
    assert_eq!(node_status(&mid, "a"), "LEASED");
    w.complete(&a, &json!({"authorised": true}))
        .await
        .expect("a");

    // The checkpoint of A is durable: the graph already shows it before B has even started.
    let mid = graph(&p, &run).await;
    assert_eq!(node_status(&mid, "a"), "SUCCEEDED");
    assert_eq!(node_status(&mid, "b"), "READY");
    let b = w.one_task().await;
    assert_eq!(b.activity_name, "act.b");
    w.complete(&b, &json!({"shipped": true})).await.expect("b");

    let events = journal(&p, &run).await;
    assert_eq!(count(&events, "step.completed", Some("a")), 1);
    assert_eq!(count(&events, "step.completed", Some("b")), 1);
    assert_eq!(count(&events, "run.finished", None), 1);
    let position = |kind: &str, step: &str| {
        events
            .iter()
            .position(|(k, s)| k == kind && s.as_deref() == Some(step))
            .unwrap_or_else(|| panic!("{kind} {step} missing in {events:?}"))
    };
    assert!(position("step.completed", "a") < position("step.leased", "b"));
    assert_eq!(
        events.last().map(|(kind, _)| kind.as_str()),
        Some("run.finished"),
        "the finish is the last thing journaled: {events:?}"
    );
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    let done = graph(&p, &run).await;
    assert_eq!(node_status(&done, "a"), "SUCCEEDED");
    assert_eq!(node_status(&done, "b"), "SUCCEEDED");
}

/// US02-AC01: a step is leased; the worker dies and the lease expires; another worker resumes
/// without reusing the stale generation, and the dead worker's late report changes nothing.
#[tokio::test]
async fn us02_a_dead_workers_step_is_taken_over_and_the_stale_generation_is_dead() {
    let p = platform().await;
    let wf = p.publish("order-flow", &chain_ir("order-flow")).await;
    let run = p.start(&wf, &json!({"orderId": "o-1"})).await;

    let mut doomed = p.worker("wendy").await;
    doomed.register(&["act.a", "act.b"]).await;
    let stale = doomed.one_task().await;

    // The worker goes silent: no heartbeat, the lease runs out, the scheduler notices.
    p.expire_lease(&run, "a").await;
    let report = p.tick().await;
    assert_eq!(report.leases_expired, 1);
    assert!(doomed.poll(8).await.is_empty(), "the retry is backing off");
    p.elapse_retry_delay(&run, "a").await;
    assert_eq!(p.tick().await.retries_released, 1);

    let mut rescuer = p.worker("rhea").await;
    rescuer.register(&["act.a", "act.b"]).await;
    let fresh = rescuer.one_task().await;
    assert_eq!(fresh.activity_name, "act.a");
    assert!(fresh.generation > stale.generation, "a new generation");
    assert_eq!(fresh.attempt, stale.attempt + 1);
    assert_ne!(fresh.lease_token, stale.lease_token);

    // The dead worker wakes up (a long GC pause, a partition healing) and reports.
    let late = doomed.complete(&stale, &json!({"authorised": true})).await;
    let status = late.expect_err("a stale generation is refused");
    assert_eq!(status.code(), tonic::Code::Aborted);
    assert_eq!(machine_code(&status), "STALE_ATTEMPT");
    let beat = doomed
        .client
        .heartbeat(doomed.req(doomed.heartbeat_message(&stale)))
        .await
        .expect_err("and so is its heartbeat");
    assert_eq!(machine_code(&beat), "STALE_ATTEMPT");

    // The rescuer finishes the run; the journal holds exactly one commit of A.
    rescuer
        .complete(&fresh, &json!({"authorised": true}))
        .await
        .expect("a");
    let b = rescuer.one_task().await;
    rescuer.complete(&b, &json!({})).await.expect("b");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    let events = journal(&p, &run).await;
    assert_eq!(count(&events, "step.completed", Some("a")), 1);
    assert!(count(&events, "step.leased", Some("a")) >= 2);
}

/// US03-AC01: a run enters a one-hour timer; no worker remains allocated; the scheduler creates the
/// next task when the timer expires.
#[tokio::test]
async fn us03_a_one_hour_timer_holds_no_worker_and_the_scheduler_resumes_it() {
    let p = platform().await;
    let ir = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "reminder"}, "entrypoint": "pause",
        "nodes": [
            {"id": "pause", "type": "timer", "durationMs": 3_600_000, "next": "a"},
            {"id": "a", "type": "activity", "activity": "act.a"}
        ]
    });
    let wf = p.publish("reminder", &ir).await;
    let run = p.start(&wf, &json!({})).await;

    assert_eq!(p.status(&run).await, "WAITING");
    let waiting = graph(&p, &run).await;
    assert_eq!(node_status(&waiting, "pause"), "WAITING");
    let pause = waiting["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["id"] == "pause")
        .expect("pause");
    assert_eq!(pause["waitingOn"], "timer");
    assert!(pause["dueAt"].is_string());

    // Nothing is allocated to it: no task exists, a worker polling gets nothing, and the
    // scheduler has nothing to do until the timer is due.
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    assert!(w.poll(8).await.is_empty());
    assert!(!p.tick().await.did_work());

    p.elapse_timer(&run, "pause").await;
    assert_eq!(p.tick().await.timers_fired, 1);
    let task = w.one_task().await;
    assert_eq!(task.activity_name, "act.a");
    w.complete(&task, &json!({})).await.expect("a");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    let events = journal(&p, &run).await;
    assert_eq!(count(&events, "timer.started", Some("pause")), 1);
    assert_eq!(count(&events, "timer.fired", Some("pause")), 1);
}

/// US03 (event flavour): a run waiting for a signal holds nothing either, and the signal resumes it.
#[tokio::test]
async fn us03_a_run_waiting_for_a_signal_resumes_when_it_arrives() {
    let p = platform().await;
    let ir = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "await-payment"}, "entrypoint": "w",
        "nodes": [
            {"id": "w", "type": "waitEvent", "eventType": "payment.confirmed",
             "correlationExpr": "input.paymentId", "timeoutMs": 600_000, "next": "a"},
            {"id": "a", "type": "activity", "activity": "act.a"}
        ]
    });
    let wf = p.publish("await-payment", &ir).await;
    let run = p.start(&wf, &json!({"paymentId": "p-9"})).await;
    assert_eq!(p.status(&run).await, "WAITING");
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    assert!(w.poll(8).await.is_empty());

    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/signals/payment.confirmed"),
            Some(&json!({"amount": 12})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let task = w.one_task().await;
    assert_eq!(task.activity_name, "act.a");
    w.complete(&task, &json!({})).await.expect("a");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
}

/// US04-AC01: the same task is announced twice and several workers contend for it; only one
/// generation is granted and only one commit is accepted.
#[tokio::test]
async fn us04_contending_workers_get_each_task_once_and_duplicate_reports_commit_once() {
    let p = platform().await;
    let single = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "single"}, "entrypoint": "a",
        "nodes": [{"id": "a", "type": "activity", "activity": "act.a"}]
    });
    let wf = p.publish("single", &single).await;
    let mut runs = Vec::new();
    for n in 0..5 {
        runs.push(p.start(&wf, &json!({"n": n})).await);
    }

    let mut workers = Vec::new();
    for n in 0..8 {
        let mut w = p.worker(&format!("worker-{n}")).await;
        w.register(&["act.a"]).await;
        workers.push(w);
    }
    // Eight workers poll at the same instant for five ready steps.
    let polls = join_all(workers.iter_mut().map(|w| async move { w.poll(5).await })).await;
    let mut claimed: Vec<(usize, _)> = polls
        .into_iter()
        .enumerate()
        .flat_map(|(who, tasks)| tasks.into_iter().map(move |t| (who, t)))
        .collect();
    assert_eq!(claimed.len(), 5, "every step is leased exactly once");
    let mut seen: Vec<&str> = claimed.iter().map(|(_, t)| t.run_id.as_str()).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 5, "no step went to two workers");

    // The winner of one step reports it twice at once (a redelivered request).
    let (who, lease) = claimed.remove(0);
    let first = Worker::complete_message(&lease, "{\"v\":1}");
    let second = first.clone();
    let owner = &mut workers[who];
    let (a, b) = {
        let request_a = owner.req(first);
        let request_b = owner.req(second);
        let mut client_b = owner.client.clone();
        let mut client_a = owner.client.clone();
        tokio::join!(
            client_a.complete_step(request_a),
            client_b.complete_step(request_b)
        )
    };
    let (a, b) = (a.expect("a").into_inner(), b.expect("b").into_inner());
    assert_eq!(
        [a.already_applied, b.already_applied]
            .iter()
            .filter(|applied| !**applied)
            .count(),
        1,
        "exactly one of the two reports was the one that committed"
    );
    assert_eq!(a.command_id, b.command_id);
    let events = journal(&p, &lease.run_id).await;
    assert_eq!(count(&events, "step.completed", Some("a")), 1);
    assert_eq!(count(&events, "step.leased", Some("a")), 1);
}
