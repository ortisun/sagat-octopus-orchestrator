//! Leases that never reach a host are withdrawn (feature 006, decisions D006-001 to D006-018 of
//! `specs/006-sdk-multilanguage/research.md`; `stream-completion.md`, "Leases that never reach a host"; ADR-0098), over the
//! real tonic server, identity provider and PostgreSQL store.
//!
//! The races are forced with barriers, never sleeps (decision D006-008): the server holds a claim after its leases
//! committed (`with_delivery_probe`) until it knows its consumer is gone, and the test lets the consumer go only once the
//! claim has committed. Each test asserts the step back to `READY`, no `step.needs_review`, the attempt not consumed, and
//! the handler run exactly once afterwards. The activity of every workflow here has one attempt: a lease that consumed it
//! would leave nothing to run.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use sagat_api_grpc::{Abandonment, DeliveryProbe};
use sagat_application::instances::{Requirement, Requirements, ServingGate};
use serde_json::{Value, json};
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

use crate::support::api::Options;
use crate::support::worker::{Platform, Worker, platform_with, single_ir};

const WAIT: Duration = Duration::from_secs(15);

/// A gate that keeps `lease-withdrawal-v1` required of control planes and engines, as the operator's activation leaves it.
fn withdrawing() -> Arc<ServingGate> {
    let gate = ServingGate::opened();
    gate.keep(&Requirements {
        epoch: 1,
        required: vec![Requirement {
            capability: "lease-withdrawal-v1".to_owned(),
            kinds: vec!["CONTROL_PLANE".to_owned(), "EXECUTION_ENGINE".to_owned()],
        }],
    });
    Arc::new(gate)
}

/// Holds the first claim that commits leases until the server knows its consumer is gone; the claims after it pass.
struct Barrier {
    arrived: Arc<Notify>,
    probe: DeliveryProbe,
}

fn barrier() -> Barrier {
    let arrived = Arc::new(Notify::new());
    let seen = Arc::new(AtomicUsize::new(0));
    let signal = Arc::clone(&arrived);
    let probe: DeliveryProbe = Arc::new(move |abandoned: Abandonment| {
        let first = seen.fetch_add(1, Ordering::SeqCst) == 0;
        let signal = Arc::clone(&signal);
        Box::pin(async move {
            if first {
                signal.notify_one();
                abandoned.wait().await;
            }
        })
    });
    Barrier { arrived, probe }
}

/// The barrier, released once only by the test (a semaphore it adds the permit to), with the abandonment ignored: a claim
/// that commits while the stream is alive and is held past the window deadline.
struct Gate {
    arrived: Arc<Notify>,
    release: Arc<Semaphore>,
    probe: DeliveryProbe,
}

fn held_past_the_window() -> Gate {
    let arrived = Arc::new(Notify::new());
    let release = Arc::new(Semaphore::new(0));
    let seen = Arc::new(AtomicUsize::new(0));
    let (signal, permit) = (Arc::clone(&arrived), Arc::clone(&release));
    let probe: DeliveryProbe = Arc::new(move |abandoned: Abandonment| {
        let first = seen.fetch_add(1, Ordering::SeqCst) == 0;
        let (signal, permit) = (Arc::clone(&signal), Arc::clone(&permit));
        Box::pin(async move {
            if first {
                signal.notify_one();
                // the window ends while the claim is held: the body tells the producer to stop
                abandoned.wait().await;
                let _ = permit.acquire().await;
            }
        })
    });
    Gate {
        arrived,
        release,
        probe,
    }
}

async fn platform(gate: Option<Arc<ServingGate>>, probe: Option<DeliveryProbe>) -> Platform {
    let mut options = Options::default();
    options.worker.idle_retry = Duration::from_millis(50);
    options.lease_withdrawal = gate;
    options.delivery_probe = probe;
    platform_with(options).await
}

fn one_attempt(name: &str) -> Value {
    single_ir(
        name,
        Some(
            json!({"maxAttempts": 1, "strategy": "fixed", "initialDelayMs": 1, "maxDelayMs": 1, "jitter": false}),
        ),
    )
}

async fn stream_worker<'a>(p: &'a Platform, subject: &str) -> Worker<'a> {
    let mut worker = p.worker(subject).await;
    worker.register_version(&["act.a"], "1.4").await;
    worker
}

/// `(status, attempt, generation)` of step `a`.
async fn step(p: &Platform, run: &str) -> (String, i32, i64) {
    sqlx::query_as(
        "SELECT status, attempt, generation FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'a'",
    )
    .bind(p.tenant.as_uuid())
    .bind(Uuid::parse_str(run).expect("run id"))
    .fetch_one(&p.api.db.pool)
    .await
    .expect("step projection")
}

async fn journal(p: &Platform, run: &str) -> Vec<(String, Value)> {
    sqlx::query_as("SELECT event_type, detail FROM run_events WHERE tenant_id = $1 AND run_id = $2 ORDER BY seq")
        .bind(p.tenant.as_uuid())
        .bind(Uuid::parse_str(run).expect("run id"))
        .fetch_all(&p.api.db.pool)
        .await
        .expect("journal")
}

fn named<'a>(journal: &'a [(String, Value)], name: &str) -> Vec<&'a Value> {
    journal
        .iter()
        .filter(|(event, _)| event == name)
        .map(|(_, detail)| detail)
        .collect()
}

/// The withdrawal runs after the consumer is gone, on the server's own task: wait until the lease was handed to it and the
/// withdrawal ended (a barrier on the server's own count, never a sleep).
async fn until_withdrawal_ran(p: &Platform, leases: u64) {
    tokio::time::timeout(WAIT, p.api.worker_withdrawals.until_handed_over(leases))
        .await
        .expect("the lease was handed to the withdrawal");
    assert!(
        p.api.worker_withdrawals.drain(WAIT).await,
        "the withdrawal ended"
    );
}

/// The withdrawn step is leased again with the same attempt number and the next generation, and its handler runs once.
async fn runs_once_afterwards(p: &Platform, run: &str, withdrawn_generation: i64) {
    let mut worker = stream_worker(p, "second").await;
    let lease = worker.one_task().await;
    assert_eq!(lease.attempt, 1, "the attempt that never began is reused");
    assert_eq!(
        i64::try_from(lease.generation).expect("generation"),
        withdrawn_generation + 1
    );
    worker
        .client
        .heartbeat(worker.req(worker.heartbeat_message(&lease)))
        .await
        .expect("the handler starts");
    worker
        .complete(&lease, &json!({"ok": true}))
        .await
        .expect("complete");
    assert_eq!(p.status(run).await, "SUCCEEDED");
    let journal = journal(p, run).await;
    assert_eq!(
        named(&journal, "step.started").len(),
        1,
        "the handler ran once"
    );
    assert_eq!(named(&journal, "step.completed").len(), 1);
    assert!(named(&journal, "step.needs_review").is_empty());
    assert!(named(&journal, "reconciliation.requested").is_empty());
}

fn assert_withdrawn(journal: &[(String, Value)], cause: &str) -> i64 {
    let withdrawn = named(journal, "step.lease_withdrawn");
    assert_eq!(withdrawn.len(), 1, "{journal:?}");
    assert_eq!(withdrawn[0]["attempt"], 1);
    assert_eq!(withdrawn[0]["cause"], cause);
    assert!(named(journal, "step.needs_review").is_empty());
    withdrawn[0]["generation"].as_i64().expect("generation")
}

#[tokio::test]
async fn a_stream_its_client_dropped_while_its_claim_committed_withdraws_the_lease() {
    let barrier = barrier();
    let p = platform(Some(withdrawing()), Some(barrier.probe)).await;
    let workflow = p
        .publish("withdraw-stream", &one_attempt("withdraw-stream"))
        .await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = stream_worker(&p, "first").await;
    let stream = worker
        .client
        .stream_tasks(worker.req(worker.poll_message(1)))
        .await
        .expect("open the stream")
        .into_inner();
    barrier.arrived.notified().await;
    assert_eq!(
        step(&p, &run).await,
        ("LEASED".to_owned(), 1, 1),
        "the claim committed"
    );
    // the client goes away while the claim is held: the server drops the body and the producer is told to stop
    drop(stream);
    until_withdrawal_ran(&p, 1).await;
    assert_eq!(
        step(&p, &run).await,
        ("READY".to_owned(), 0, 1),
        "the attempt is given back"
    );
    let generation = assert_withdrawn(&journal(&p, &run).await, "NOT_SENT");
    assert_eq!(generation, 1);
    runs_once_afterwards(&p, &run, generation).await;
}

/// The window of the stream ends while its claim is held after the commit: the claim is not cut off, and what it returns
/// past the window goes to the withdrawal (decision D006-002).
#[tokio::test]
async fn a_claim_that_ends_after_the_window_withdraws_its_lease() {
    let gate = held_past_the_window();
    let p = platform(Some(withdrawing()), Some(gate.probe)).await;
    let workflow = p
        .publish("withdraw-window", &one_attempt("withdraw-window"))
        .await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = stream_worker(&p, "first").await;
    let mut stream = worker
        .client
        .stream_tasks(worker.req(worker.poll_message(1)))
        .await
        .expect("open the stream")
        .into_inner();
    gate.arrived.notified().await;
    // the reader keeps up: the stream ends at its window without the frame of the held claim
    let mut frames = 0;
    loop {
        match tokio::time::timeout(WAIT, stream.message())
            .await
            .expect("the window ends")
        {
            Ok(Some(frame)) => {
                assert!(
                    frame.tasks.is_empty(),
                    "the held lease never reaches the reader"
                );
                frames += 1;
            }
            Ok(None) | Err(_) => break,
        }
    }
    assert!(frames <= 1, "at most the terminal hint");
    gate.release.add_permits(1);
    until_withdrawal_ran(&p, 1).await;
    assert_eq!(step(&p, &run).await, ("READY".to_owned(), 0, 1));
    let generation = assert_withdrawn(&journal(&p, &run).await, "NOT_SENT");
    runs_once_afterwards(&p, &run, generation).await;
}

/// A unary poll whose client goes away while its claim is held after the commit: the claim runs in its own task, and its
/// leases, with nobody to answer, are withdrawn (decision D006-017).
#[tokio::test]
async fn a_unary_poll_dropped_in_the_middle_of_its_claim_withdraws_the_lease() {
    let barrier = barrier();
    let p = platform(Some(withdrawing()), Some(barrier.probe)).await;
    let workflow = p
        .publish("withdraw-unary", &one_attempt("withdraw-unary"))
        .await;
    let run = p.start(&workflow, &json!({})).await;
    let worker = stream_worker(&p, "first").await;
    let request = worker.req(worker.poll_message(1));
    let mut client = worker.client.clone();
    let call = tokio::spawn(async move { client.poll_tasks(request).await });
    barrier.arrived.notified().await;
    assert_eq!(step(&p, &run).await, ("LEASED".to_owned(), 1, 1));
    call.abort();
    let _ = call.await;
    until_withdrawal_ran(&p, 1).await;
    assert_eq!(step(&p, &run).await, ("READY".to_owned(), 0, 1));
    let generation = assert_withdrawn(&journal(&p, &run).await, "NOT_SENT");
    runs_once_afterwards(&p, &run, generation).await;
}

/// Until the operator records `lease-withdrawal-v1`, nothing changes: the undelivered lease stays `LEASED` and lapses as
/// before (decision D006-004, gap G006-003).
#[tokio::test]
async fn without_the_recorded_requirement_an_undelivered_lease_lapses_as_before() {
    for gate in [None, Some(Arc::new(ServingGate::opened()))] {
        let barrier = barrier();
        let p = platform(gate, Some(barrier.probe)).await;
        let workflow = p
            .publish("lapse-stream", &one_attempt("lapse-stream"))
            .await;
        let run = p.start(&workflow, &json!({})).await;
        let mut worker = stream_worker(&p, "first").await;
        let stream = worker
            .client
            .stream_tasks(worker.req(worker.poll_message(1)))
            .await
            .expect("open the stream")
            .into_inner();
        barrier.arrived.notified().await;
        drop(stream);
        until_withdrawal_ran(&p, 1).await;
        let mut other = stream_worker(&p, "second").await;
        assert!(
            other.poll(1).await.is_empty(),
            "the lease was not given back"
        );
        assert_eq!(step(&p, &run).await, ("LEASED".to_owned(), 1, 1));
        assert!(named(&journal(&p, &run).await, "step.lease_withdrawn").is_empty());
    }
}

/// A host hands back a lease it will not run (decision D006-005): while the step is still `LEASED` it is withdrawn, the
/// report is idempotent, and the next lease reuses the attempt.
#[tokio::test]
async fn a_lease_handed_back_with_task_not_started_is_withdrawn_while_it_is_still_leased() {
    let p = platform(Some(withdrawing()), None).await;
    let workflow = p.publish("hand-back", &one_attempt("hand-back")).await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = stream_worker(&p, "first").await;
    let lease = worker.one_task().await;
    for repeat in [false, true] {
        let ack = worker
            .fail_with(&lease, "TASK_NOT_STARTED", true)
            .await
            .expect("the hand-back is accepted");
        assert_eq!(ack.step_status, "READY");
        assert_eq!(ack.already_applied, repeat);
    }
    assert_eq!(step(&p, &run).await, ("READY".to_owned(), 0, 1));
    let generation = assert_withdrawn(&journal(&p, &run).await, "NOT_STARTED");
    runs_once_afterwards(&p, &run, generation).await;
}

/// The step is no longer `LEASED` (a heartbeat started its handler), or the requirement is not recorded: the hand-back is an
/// ordinary retryable failure, never an unknown outcome. With one attempt allowed it fails the step for good.
#[tokio::test]
async fn a_hand_back_of_a_started_step_or_without_the_requirement_is_an_ordinary_failure() {
    for (gate, started) in [
        (Some(withdrawing()), true),
        (None, false),
        (Some(Arc::new(ServingGate::opened())), false),
    ] {
        let p = platform(gate, None).await;
        let workflow = p
            .publish("hand-back-late", &one_attempt("hand-back-late"))
            .await;
        let run = p.start(&workflow, &json!({})).await;
        let mut worker = stream_worker(&p, "first").await;
        let lease = worker.one_task().await;
        if started {
            worker
                .client
                .heartbeat(worker.req(worker.heartbeat_message(&lease)))
                .await
                .expect("started");
        }
        let ack = worker
            .fail_with(&lease, "TASK_NOT_STARTED", true)
            .await
            .expect("reported");
        assert_eq!(ack.step_status, "FAILED", "started={started}");
        assert_eq!(p.status(&run).await, "FAILED");
        let journal = journal(&p, &run).await;
        assert!(named(&journal, "step.lease_withdrawn").is_empty());
        assert!(
            named(&journal, "step.needs_review").is_empty(),
            "never an unknown outcome"
        );
        assert_eq!(
            named(&journal, "step.failed")[0]["code"],
            "TASK_NOT_STARTED"
        );
    }
}
