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
use crate::support::worker::{Platform, Worker, machine_code, platform_with, single_ir};

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
    // judged as the 1.8 build will judge; `as_protocol_1_8` raises the stored minor of a session
    let mut options = Options {
        hand_back_ceiling: 8,
        ..Options::default()
    };
    options.worker.idle_retry = Duration::from_millis(50);
    options.lease_withdrawal = gate;
    options.delivery_probe = probe;
    platform_with(options).await
}

/// Records `lease-withdrawal-v1` of both kinds in the registry, as the operator's activation does: the withdrawal reads it
/// again, under its lock, in every transaction (decision D006-032).
async fn record_requirement(p: &Platform) {
    let outcome: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_activate('lease-withdrawal-v1', \
         ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE'], 'test')",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("activate");
    assert_eq!(outcome, "RECORDED");
}

/// A platform whose gate keeps the requirement and whose registry records it.
async fn withdrawing_platform(probe: Option<DeliveryProbe>) -> Platform {
    let p = platform(Some(withdrawing()), probe).await;
    record_requirement(&p).await;
    p
}

/// Raises the protocol minor the sessions of `subject` negotiated to 8, the minor of delivery acknowledgement that reserves
/// `TASK_NOT_STARTED` (decision D006-030). This build negotiates at most 1.6, so a test of the honoured hand-back has no
/// other way to hold a 1.8 session; the delivery acknowledgement build replaces it with a real one.
async fn as_protocol_1_8(p: &Platform, subject: &str) {
    let done = sqlx::query(
        "UPDATE worker_sessions SET accepted_protocol_minor = 8 WHERE tenant_id = $1 AND subject = $2",
    )
    .bind(p.tenant.as_uuid())
    .bind(subject)
    .execute(&p.api.db.pool)
    .await
    .expect("raise the minor");
    assert!(done.rows_affected() >= 1, "no session of {subject}");
}

/// The journal reproduces the projection of the run (constitution: replay equals projection).
async fn replays(p: &Platform, run: &str) {
    p.api
        .executions
        .verify_run(p.tenant, run.parse().expect("run id"))
        .await
        .expect("the journal reproduces the projections");
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
    assert_eq!(lease.generation, withdrawn_generation + 1);
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
    replays(p, run).await;
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
    let p = withdrawing_platform(Some(barrier.probe)).await;
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
    let p = withdrawing_platform(Some(gate.probe)).await;
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
    while let Ok(Some(frame)) = tokio::time::timeout(WAIT, stream.message())
        .await
        .expect("the window ends")
    {
        assert!(
            frame.tasks.is_empty(),
            "the held lease never reaches the reader"
        );
        frames += 1;
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
    let p = withdrawing_platform(Some(barrier.probe)).await;
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

/// A host of protocol 1.8 hands back a lease it will not run (decisions D006-005 and D006-030): while the step is still
/// `LEASED` it is withdrawn, the report is idempotent, and the next lease reuses the attempt.
#[tokio::test]
async fn a_lease_handed_back_with_task_not_started_is_withdrawn_while_it_is_still_leased() {
    let p = withdrawing_platform(None).await;
    let workflow = p.publish("hand-back", &one_attempt("hand-back")).await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = stream_worker(&p, "first").await;
    as_protocol_1_8(&p, "first").await;
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

/// Decision D006-086 (round-2 correctness minor 1, security minor 4): a token handed back is dead for heartbeats too. A host
/// that broke "never after start" and keeps beating its handler is told `STALE_ATTEMPT`, never an OK that would let the
/// handler run on while the step is leased again.
#[tokio::test]
async fn a_heartbeat_with_a_handed_back_token_is_stale() {
    let p = withdrawing_platform(None).await;
    let workflow = p.publish("dead-token", &one_attempt("dead-token")).await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = stream_worker(&p, "first").await;
    as_protocol_1_8(&p, "first").await;
    let lease = worker.one_task().await;
    let ack = worker
        .fail_with(&lease, "TASK_NOT_STARTED", true)
        .await
        .expect("the hand-back is accepted");
    assert_eq!(ack.step_status, "READY");
    let stale = worker
        .client
        .heartbeat(worker.req(worker.heartbeat_message(&lease)))
        .await
        .expect_err("the handed-back token is dead");
    assert_eq!(machine_code(&stale), "STALE_ATTEMPT", "{stale:?}");
    assert_eq!(step(&p, &run).await, ("READY".to_owned(), 0, 1));
    assert!(named(&journal(&p, &run).await, "step.started").is_empty());
}

/// Decision D006-089 (round-3 correctness Major 1): a session of 1.8 that hands back on a fleet that does not withdraw (no
/// gate, a gate that does not keep the requirement, or the requirement recorded with the engine's switch off) records
/// nothing: the host is told `STALE_ATTEMPT` and the lease lapses as on `develop`, where it opens a review. It never spends
/// the attempt of a step whose work never ran.
#[tokio::test]
async fn a_hand_back_on_a_fleet_that_does_not_withdraw_leaves_the_lease_to_lapse() {
    for (case, gate, switched_off) in [
        ("no gate", None, false),
        ("not required", Some(Arc::new(ServingGate::opened())), false),
        ("switched off", Some(withdrawing()), true),
    ] {
        let mut options = Options {
            hand_back_ceiling: 8,
            ..Options::default()
        };
        options.worker.idle_retry = Duration::from_millis(50);
        options.lease_withdrawal = gate;
        options.new_withdrawals = !switched_off;
        let p = platform_with(options).await;
        if switched_off {
            record_requirement(&p).await;
        }
        let workflow = p.publish("lapse", &one_attempt("lapse")).await;
        let run = p.start(&workflow, &json!({})).await;
        let mut worker = stream_worker(&p, "first").await;
        as_protocol_1_8(&p, "first").await;
        let lease = worker.one_task().await;
        let refused = worker
            .fail_with(&lease, "TASK_NOT_STARTED", true)
            .await
            .expect_err("nothing is recorded");
        assert_eq!(
            machine_code(&refused),
            "STALE_ATTEMPT",
            "{case}: {refused:?}"
        );
        assert_eq!(step(&p, &run).await, ("LEASED".to_owned(), 1, 1), "{case}");
        let journal = journal(&p, &run).await;
        assert!(named(&journal, "step.failed").is_empty(), "{case}");
        assert!(named(&journal, "step.lease_withdrawn").is_empty(), "{case}");
        assert_eq!(p.status(&run).await, "RUNNING", "{case}");
    }
}

/// The step is no longer `LEASED` (a heartbeat started its handler): the hand-back is an ordinary retryable failure, never
/// an unknown outcome. With one attempt allowed it fails the step for good.
#[tokio::test]
async fn a_hand_back_of_a_started_step_is_an_ordinary_failure() {
    for (gate, started) in [(Some(withdrawing()), true)] {
        let p = platform(gate, None).await;
        if started {
            record_requirement(&p).await;
        }
        let workflow = p
            .publish("hand-back-late", &one_attempt("hand-back-late"))
            .await;
        let run = p.start(&workflow, &json!({})).await;
        let mut worker = stream_worker(&p, "first").await;
        as_protocol_1_8(&p, "first").await;
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

fn three_attempts(name: &str) -> Value {
    single_ir(
        name,
        Some(
            json!({"maxAttempts": 3, "strategy": "fixed", "initialDelayMs": 60000, "maxDelayMs": 60000, "jitter": false}),
        ),
    )
}

/// Holds every claim that commits leases until its consumer is gone, and says each time it arrived.
fn barrier_every() -> Barrier {
    let arrived = Arc::new(Notify::new());
    let signal = Arc::clone(&arrived);
    let probe: DeliveryProbe = Arc::new(move |abandoned: Abandonment| {
        let signal = Arc::clone(&signal);
        Box::pin(async move {
            signal.notify_one();
            abandoned.wait().await;
        })
    });
    Barrier { arrived, probe }
}

async fn open_stream(
    worker: &mut Worker<'_>,
    credit: i32,
) -> tonic::Streaming<sagat_api_grpc::pb::PollTasksResponse> {
    worker
        .client
        .stream_tasks(worker.req(worker.poll_message(credit)))
        .await
        .expect("open the stream")
        .into_inner()
}

/// Security Blocker 1 of the 2026-11-21 review (decision D006-030): from a session older than 1.8 a `TASK_NOT_STARTED` is
/// an ordinary failure. A handler that raises it on every lease, with no heartbeat and `retryable=false`, fails its step
/// for good at once; with `retryable=true` it spends the attempt and waits out the backoff. Nothing is withdrawn.
#[tokio::test]
async fn a_hand_back_from_a_session_older_than_1_8_is_an_ordinary_failure_with_its_flag() {
    let p = withdrawing_platform(None).await;
    let workflow = p.publish("old-minor", &three_attempts("old-minor")).await;
    for retryable in [false, true] {
        let run = p.start(&workflow, &json!({})).await;
        let mut worker = stream_worker(&p, &format!("first-{retryable}")).await;
        let lease = worker.one_task().await;
        let ack = worker
            .fail_with(&lease, "TASK_NOT_STARTED", retryable)
            .await
            .expect("reported");
        let journal = journal(&p, &run).await;
        assert!(
            named(&journal, "step.lease_withdrawn").is_empty(),
            "retryable={retryable}"
        );
        assert!(named(&journal, "step.needs_review").is_empty());
        if retryable {
            assert_eq!(named(&journal, "step.retry_scheduled").len(), 1);
            let (status, attempt, _) = step(&p, &run).await;
            assert_eq!(
                (status.as_str(), attempt),
                ("RETRY_WAIT", 1),
                "the attempt is spent"
            );
            assert!(
                worker.poll(1).await.is_empty(),
                "the backoff holds the step"
            );
        } else {
            assert_eq!(ack.step_status, "FAILED", "the flag is honoured");
            assert_eq!(p.status(&run).await, "FAILED");
        }
        replays(&p, &run).await;
    }
}

/// Decision D006-031 at its edge for a host's hand-back: four hand-backs of one attempt are withdrawn, the fifth is the
/// ordinary failure (one attempt allowed: the step fails for good, never a review).
#[tokio::test]
async fn one_attempt_is_handed_back_at_most_four_times() {
    let p = withdrawing_platform(None).await;
    let workflow = p
        .publish("hand-back-cap", &one_attempt("hand-back-cap"))
        .await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = stream_worker(&p, "first").await;
    as_protocol_1_8(&p, "first").await;
    for generation in 1..=4 {
        let lease = worker.one_task().await;
        assert_eq!((lease.attempt, lease.generation), (1, generation));
        let ack = worker
            .fail_with(&lease, "TASK_NOT_STARTED", true)
            .await
            .expect("hand-back");
        assert_eq!(ack.step_status, "READY", "hand-back {generation}");
    }
    let fifth = worker.one_task().await;
    assert_eq!((fifth.attempt, fifth.generation), (1, 5));
    let ack = worker
        .fail_with(&fifth, "TASK_NOT_STARTED", true)
        .await
        .expect("hand-back");
    assert_eq!(
        ack.step_status, "FAILED",
        "the fifth spends the only attempt"
    );
    let journal = journal(&p, &run).await;
    let withdrawn = named(&journal, "step.lease_withdrawn");
    assert_eq!(withdrawn.len(), 4);
    assert!(
        withdrawn
            .iter()
            .all(|w| w["attempt"] == 1 && w["cause"] == "NOT_STARTED")
    );
    assert_eq!(
        named(&journal, "step.failed")[0]["code"],
        "TASK_NOT_STARTED"
    );
    assert!(named(&journal, "step.needs_review").is_empty());
    replays(&p, &run).await;
}

/// Decision D006-031 at its edge for a lease the server did not send: four dropped streams withdraw, the fifth lease is
/// left to lapse as before this feature.
#[tokio::test]
async fn a_lease_not_sent_is_withdrawn_at_most_four_times_for_one_attempt() {
    let barrier = barrier_every();
    let p = withdrawing_platform(Some(barrier.probe)).await;
    let workflow = p
        .publish("not-sent-cap", &one_attempt("not-sent-cap"))
        .await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = stream_worker(&p, "first").await;
    for dropped in 1..=5u64 {
        let stream = open_stream(&mut worker, 1).await;
        barrier.arrived.notified().await;
        assert_eq!(step(&p, &run).await.0, "LEASED");
        drop(stream);
        until_withdrawal_ran(&p, dropped).await;
    }
    assert_eq!(
        step(&p, &run).await,
        ("LEASED".to_owned(), 1, 5),
        "the fifth is not withdrawn"
    );
    let journal = journal(&p, &run).await;
    assert_eq!(named(&journal, "step.lease_withdrawn").len(), 4);
    replays(&p, &run).await;
}

/// Correctness Majors 1 and 2 of the review (decision D006-035): two ready runs, one claim; the consumer goes away after
/// the first run's transaction committed. The first run's lease is withdrawn and the second run is never leased. A claim
/// cut off at the barrier would lose the first lease (no guard holds it yet); a claim that ignores its consumer would lease
/// the second.
#[tokio::test]
async fn a_claim_whose_consumer_is_gone_leases_no_further_run() {
    for unary in [false, true] {
        let barrier = barrier();
        let p = withdrawing_platform(Some(barrier.probe)).await;
        let workflow = p.publish("two-runs", &one_attempt("two-runs")).await;
        let first = p.start(&workflow, &json!({})).await;
        let second = p.start(&workflow, &json!({})).await;
        let mut worker = stream_worker(&p, "first").await;
        if unary {
            let request = worker.req(worker.poll_message(2));
            let mut client = worker.client.clone();
            let call = tokio::spawn(async move { client.poll_tasks(request).await });
            barrier.arrived.notified().await;
            call.abort();
            let _ = call.await;
        } else {
            let stream = open_stream(&mut worker, 2).await;
            barrier.arrived.notified().await;
            drop(stream);
        }
        until_withdrawal_ran(&p, 1).await;
        let (claimed, untouched) = if step(&p, &first).await.2 == 1 {
            (first, second)
        } else {
            (second, first)
        };
        assert_eq!(
            step(&p, &claimed).await,
            ("READY".to_owned(), 0, 1),
            "unary={unary}"
        );
        assert_withdrawn(&journal(&p, &claimed).await, "NOT_SENT");
        assert_eq!(
            step(&p, &untouched).await,
            ("READY".to_owned(), 0, 0),
            "unary={unary}: the second run was never leased"
        );
        assert!(named(&journal(&p, &untouched).await, "step.leased").is_empty());
        replays(&p, &claimed).await;
    }
}

/// Decision D006-092: the consumer goes away while the claim is held just before the commit of a run's transaction that
/// leased. On a platform that withdraws, the transaction is still rolled back, not committed and then withdrawn: the step
/// was never leased, nothing went to the withdrawal (no slot of the cap of four is used), and the next lease is the step's
/// first. For a stream and for a unary poll.
#[tokio::test]
async fn a_claim_whose_consumer_goes_before_its_commit_leases_nothing_and_withdraws_nothing() {
    for unary in [false, true] {
        let barrier = barrier();
        let mut options = Options {
            hand_back_ceiling: 8,
            ..Options::default()
        };
        options.worker.idle_retry = Duration::from_millis(50);
        options.lease_withdrawal = Some(withdrawing());
        options.commit_probe = Some(barrier.probe);
        let p = platform_with(options).await;
        record_requirement(&p).await;
        let workflow = p
            .publish("gone-before-commit", &one_attempt("gone-before-commit"))
            .await;
        let run = p.start(&workflow, &json!({})).await;
        let mut worker = stream_worker(&p, "first").await;
        if unary {
            let request = worker.req(worker.poll_message(1));
            let mut client = worker.client.clone();
            let call = tokio::spawn(async move { client.poll_tasks(request).await });
            tokio::time::timeout(WAIT, barrier.arrived.notified())
                .await
                .expect("the claim reached its commit");
            call.abort();
            let _ = call.await;
        } else {
            let stream = open_stream(&mut worker, 1).await;
            tokio::time::timeout(WAIT, barrier.arrived.notified())
                .await
                .expect("the claim reached its commit");
            drop(stream);
        }
        assert!(
            p.api.worker_withdrawals.drain(WAIT).await,
            "unary={unary}: the claim ended"
        );
        assert_eq!(
            p.api.worker_withdrawals.handed_over(),
            0,
            "unary={unary}: nothing went to the withdrawal"
        );
        assert_eq!(
            step(&p, &run).await,
            ("READY".to_owned(), 0, 0),
            "unary={unary}: the run's transaction was rolled back"
        );
        let journal = journal(&p, &run).await;
        assert!(named(&journal, "step.leased").is_empty(), "unary={unary}");
        assert!(
            named(&journal, "step.lease_withdrawn").is_empty(),
            "unary={unary}"
        );
        runs_once_afterwards(&p, &run, 0).await;
    }
}

/// Architecture Blocker 2 (decision D006-032): once a journal holds `step.lease_withdrawn`, the requirement is a replay
/// floor and cannot be removed; it is still recorded afterwards.
#[tokio::test]
async fn the_requirement_cannot_be_removed_once_a_journal_holds_a_withdrawal() {
    let barrier = barrier();
    let p = withdrawing_platform(Some(barrier.probe)).await;
    let workflow = p.publish("floor", &one_attempt("floor")).await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = stream_worker(&p, "first").await;
    let stream = open_stream(&mut worker, 1).await;
    barrier.arrived.notified().await;
    drop(stream);
    until_withdrawal_ran(&p, 1).await;
    assert_withdrawn(&journal(&p, &run).await, "NOT_SENT");
    let refused =
        sqlx::query("SELECT * FROM platform_requirement_remove('lease-withdrawal-v1', 'ops')")
            .execute(&p.api.db.pool)
            .await
            .expect_err("the removal is refused");
    assert!(
        refused
            .to_string()
            .contains("cannot be removed while a journal holds step.lease_withdrawn"),
        "{refused}"
    );
    // the switch stops the withdrawals of unmarked leases only: from 1.8 it does not reach a marked one (request (6) of
    // decision D006-059, Task 006-070)
    assert!(
        refused
            .to_string()
            .contains("to stop new withdrawals of unmarked leases"),
        "{refused}"
    );
    let still: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM platform_requirements WHERE capability = 'lease-withdrawal-v1')",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("requirement");
    assert!(still);
}

/// The withdrawal reads the requirement again in its transaction (decision D006-032): a gate that still keeps it while the
/// registry no longer records it (a removal the engine has not learned yet) withdraws nothing. And an engine with
/// `SAGAT_LEASE_WITHDRAWAL=off` withdraws nothing either. In both the lease lapses as before.
#[tokio::test]
async fn a_requirement_the_registry_does_not_record_or_the_switch_off_withdraws_nothing() {
    for switched_off in [false, true] {
        let barrier = barrier();
        let mut options = Options::default();
        options.worker.idle_retry = Duration::from_millis(50);
        options.lease_withdrawal = Some(withdrawing());
        options.delivery_probe = Some(barrier.probe);
        options.new_withdrawals = !switched_off;
        let p = platform_with(options).await;
        if switched_off {
            record_requirement(&p).await;
        }
        let workflow = p
            .publish("not-recorded", &one_attempt("not-recorded"))
            .await;
        let run = p.start(&workflow, &json!({})).await;
        let mut worker = stream_worker(&p, "first").await;
        let stream = open_stream(&mut worker, 1).await;
        barrier.arrived.notified().await;
        drop(stream);
        until_withdrawal_ran(&p, 1).await;
        assert_eq!(
            step(&p, &run).await,
            ("LEASED".to_owned(), 1, 1),
            "switched_off={switched_off}"
        );
        assert!(named(&journal(&p, &run).await, "step.lease_withdrawn").is_empty());
    }
}

/// Request (8) of decision D006-059 (Task 006-070, decision D006-095): `lease-withdrawal-v1` recorded by hand SQL for the
/// engines only is not the requirement. The gate of this engine keeps it for both kinds (a gate that has not read the
/// registry again), and the withdrawal's own read under the lock finds it for one kind: nothing is withdrawn, the lease
/// lapses as before, and a 1.8 hand-back is answered `STALE_ATTEMPT` with nothing written (decision D006-089).
#[tokio::test]
async fn a_requirement_recorded_for_one_kind_withdraws_nothing() {
    let barrier = barrier();
    let p = platform(Some(withdrawing()), Some(barrier.probe)).await;
    let outcome: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_activate('lease-withdrawal-v1', \
         ARRAY['EXECUTION_ENGINE'], 'test')",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("recorded for the engines only");
    assert_eq!(outcome, "RECORDED");
    let workflow = p.publish("one-kind", &one_attempt("one-kind")).await;
    let dropped = p.start(&workflow, &json!({})).await;
    let mut worker = stream_worker(&p, "first").await;
    let stream = open_stream(&mut worker, 1).await;
    barrier.arrived.notified().await;
    drop(stream);
    until_withdrawal_ran(&p, 1).await;
    assert_eq!(
        step(&p, &dropped).await,
        ("LEASED".to_owned(), 1, 1),
        "the lease not sent lapses"
    );
    assert!(named(&journal(&p, &dropped).await, "step.lease_withdrawn").is_empty());

    let handed = p.start(&workflow, &json!({})).await;
    as_protocol_1_8(&p, "first").await;
    let lease = worker.one_task().await;
    let refused = worker
        .fail_with(&lease, "TASK_NOT_STARTED", true)
        .await
        .expect_err("nothing is recorded");
    assert_eq!(machine_code(&refused), "STALE_ATTEMPT", "{refused:?}");
    assert_eq!(step(&p, &handed).await, ("LEASED".to_owned(), 1, 1));
    let journal = journal(&p, &handed).await;
    assert!(named(&journal, "step.lease_withdrawn").is_empty());
    assert!(named(&journal, "step.failed").is_empty());
}

/// Correctness Major 3 and architecture Major 2 (decision D006-036): the stream's producer runs on the tracker a graceful
/// shutdown drains, so the drain waits for a claim in progress and then for its withdrawal. A producer outside the tracker
/// would let the drain end at once while the claim is still held.
#[tokio::test]
async fn a_graceful_shutdown_waits_for_a_claim_in_progress_and_its_withdrawal() {
    // held until the server knows the consumer is gone and the test releases it, so the frame is never handed over
    let gate = held_past_the_window();
    let p = withdrawing_platform(Some(gate.probe)).await;
    let workflow = p.publish("shutdown", &one_attempt("shutdown")).await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = stream_worker(&p, "first").await;
    let stream = open_stream(&mut worker, 1).await;
    gate.arrived.notified().await;
    assert!(
        !p.api
            .worker_withdrawals
            .drain(Duration::from_millis(200))
            .await,
        "the drain waits while the claim is held"
    );
    drop(stream);
    gate.release.add_permits(1);
    assert!(
        p.api.worker_withdrawals.drain(WAIT).await,
        "the claim and its withdrawal ended"
    );
    assert_eq!(p.api.worker_withdrawals.handed_over(), 1);
    assert_eq!(step(&p, &run).await, ("READY".to_owned(), 0, 1));
    assert_withdrawn(&journal(&p, &run).await, "NOT_SENT");
}

/// Decision D006-036 for the unary poll: its claim runs on the tracker a graceful shutdown drains, so the drain waits for a
/// unary claim in progress and then for its withdrawal. A claim spawned outside the tracker would let the drain end while
/// the claim is still held.
#[tokio::test]
async fn a_graceful_shutdown_waits_for_a_unary_claim_in_progress_and_its_withdrawal() {
    let gate = held_past_the_window();
    let p = withdrawing_platform(Some(gate.probe)).await;
    let workflow = p
        .publish("shutdown-unary", &one_attempt("shutdown-unary"))
        .await;
    let run = p.start(&workflow, &json!({})).await;
    let worker = stream_worker(&p, "first").await;
    let request = worker.req(worker.poll_message(1));
    let mut client = worker.client.clone();
    let call = tokio::spawn(async move { client.poll_tasks(request).await });
    gate.arrived.notified().await;
    assert!(
        !p.api
            .worker_withdrawals
            .drain(Duration::from_millis(200))
            .await,
        "the drain waits while the unary claim is held"
    );
    call.abort();
    let _ = call.await;
    gate.release.add_permits(1);
    assert!(
        p.api.worker_withdrawals.drain(WAIT).await,
        "the unary claim and its withdrawal ended"
    );
    assert_eq!(p.api.worker_withdrawals.handed_over(), 1);
    assert_eq!(step(&p, &run).await, ("READY".to_owned(), 0, 1));
    assert_withdrawn(&journal(&p, &run).await, "NOT_SENT");
}

/// Security Major 2 (decision D006-037): a unary poll that its client cancels keeps the tenant's concurrency slot until
/// its claim ends, so cancelled polls cannot run more claims at once than the tenant's cap.
#[tokio::test]
async fn a_cancelled_unary_poll_keeps_its_tenant_slot_until_its_claim_ends() {
    // each claim says when it arrived and when the server knows its handler is gone, then waits for the test's permit
    let gone = Arc::new(Semaphore::new(0));
    let gate = {
        let arrived = Arc::new(Notify::new());
        let release = Arc::new(Semaphore::new(0));
        let (signal, permit, seen) = (
            Arc::clone(&arrived),
            Arc::clone(&release),
            Arc::clone(&gone),
        );
        let held = Arc::new(AtomicUsize::new(0));
        let probe: DeliveryProbe = Arc::new(move |abandoned: Abandonment| {
            let (signal, permit, seen) =
                (Arc::clone(&signal), Arc::clone(&permit), Arc::clone(&seen));
            // the two cancelled polls are held; the poll after them passes
            let first_two = held.fetch_add(1, Ordering::SeqCst) < 2;
            Box::pin(async move {
                if !first_two {
                    return;
                }
                signal.notify_one();
                abandoned.wait().await;
                seen.add_permits(1);
                let _ = permit
                    .acquire()
                    .await
                    .map(tokio::sync::SemaphorePermit::forget);
            })
        });
        Gate {
            arrived,
            release,
            probe,
        }
    };
    let mut options = Options::default();
    options.worker.idle_retry = Duration::from_millis(50);
    options.lease_withdrawal = Some(withdrawing());
    options.delivery_probe = Some(gate.probe);
    options.limits.max_in_flight_per_tenant = 2;
    let p = platform_with(options).await;
    record_requirement(&p).await;
    let workflow = p.publish("slots", &one_attempt("slots")).await;
    for _ in 0..2 {
        p.start(&workflow, &json!({})).await;
    }
    let worker = stream_worker(&p, "first").await;
    let mut calls = Vec::new();
    for _ in 0..2 {
        let request = worker.req(worker.poll_message(1));
        let mut client = worker.client.clone();
        calls.push(tokio::spawn(
            async move { client.poll_tasks(request).await },
        ));
        gate.arrived.notified().await;
    }
    for call in calls {
        call.abort();
        let _ = call.await;
    }
    // the server dropped both handlers: with the slots in the handlers they would be free by now
    let _ = gone
        .acquire_many(2)
        .await
        .map(tokio::sync::SemaphorePermit::forget);
    let mut client = worker.client.clone();
    let refused = client
        .poll_tasks(worker.req(worker.poll_message(1)))
        .await
        .expect_err("both slots are still held by the claims");
    assert_eq!(
        refused.code(),
        tonic::Code::ResourceExhausted,
        "{refused:?}"
    );
    gate.release.add_permits(2);
    until_withdrawal_ran(&p, 2).await;
    client
        .poll_tasks(worker.req(worker.poll_message(1)))
        .await
        .expect("the slots came back with the claims");
}

/// Request (10) of decision D006-059 (Task 006-074, decision D006-099; it replaces the test that the hand-back was an
/// ordinary failure): a session stored at minor 8, under a build whose ceiling is 6 (this build's own), is judged at 6, so
/// it is never withdrawn; and since the stored minor reserves the code, the host's word that nothing ran is not turned into a
/// failure either. It lapses: `STALE_ATTEMPT`, nothing written, the lease still live. The case is a binary rolled back to
/// this build while hosts registered at 1.8 hand back before they register again. At the root and in a scope.
#[tokio::test]
async fn a_hand_back_from_a_session_stored_at_1_8_that_this_build_cannot_honour_lapses() {
    for scoped in [false, true] {
        let mut options = Options::default();
        options.worker.idle_retry = Duration::from_millis(50);
        options.lease_withdrawal = Some(withdrawing());
        let p = platform_with(options).await;
        record_requirement(&p).await;
        let name = format!("ceiling-{scoped}");
        let workflow = p.publish(&name, &cancellable(&name, scoped, &[])).await;
        let mut worker = p.worker("first").await;
        worker
            .register_version(&["act.a"], if scoped { "1.5" } else { "1.4" })
            .await;
        as_protocol_1_8(&p, "first").await;
        let (run, lease) = first_lease(&p, &mut worker, &workflow, scoped).await;
        let refused = worker
            .fail_with(&lease, "TASK_NOT_STARTED", true)
            .await
            .expect_err("nothing is recorded");
        assert_eq!(
            machine_code(&refused),
            "STALE_ATTEMPT",
            "scoped={scoped}: {refused:?}"
        );
        let journal = journal(&p, &run).await;
        assert!(
            named(&journal, "step.lease_withdrawn").is_empty(),
            "scoped={scoped}"
        );
        assert!(named(&journal, "step.failed").is_empty(), "scoped={scoped}");
        assert!(
            named(&journal, "step.started").is_empty(),
            "scoped={scoped}"
        );
        assert_eq!(p.status(&run).await, "RUNNING", "scoped={scoped}");
        if !scoped {
            assert_eq!(step(&p, &run).await, ("LEASED".to_owned(), 1, 1));
        }
        // the lease is still the step's: its heartbeat is answered
        worker
            .client
            .heartbeat(worker.req(worker.heartbeat_message(&lease)))
            .await
            .unwrap_or_else(|stale| panic!("scoped={scoped}: the lease is still live: {stale:?}"));
    }
}

/// Whether the lease is of an instance in a scope (a root lease names the root instance, with an empty path).
fn in_a_scope(lease: &sagat_api_grpc::pb::TaskLease) -> bool {
    lease
        .instance
        .as_ref()
        .is_some_and(|instance| !instance.instance_path.is_empty())
}

/// Starts `workflow` (one item for a scoped one, with the iteration quota and a scheduler) and polls its first lease.
async fn first_lease(
    p: &Platform,
    worker: &mut Worker<'_>,
    workflow: &str,
    scoped: bool,
) -> (String, sagat_api_grpc::pb::TaskLease) {
    let (run, scheduler) = if scoped {
        let (status, body) = p
            .rest(
                reqwest::Method::PUT,
                "/quotas/iteration_concurrency",
                Some(&json!({"limit": 8})),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        let run = p.start(workflow, &json!({"lines": [1]})).await;
        (
            run,
            Some(crate::support::blob_collections::scheduler(p, None).await),
        )
    } else {
        (p.start(workflow, &json!({})).await, None)
    };
    for _ in 0..20 {
        if let Some(scheduler) = &scheduler {
            scheduler.tick().await;
        }
        if let Some(lease) = worker.poll(1).await.pop() {
            assert_eq!(in_a_scope(&lease), scoped);
            return (run, lease);
        }
    }
    panic!("scoped={scoped}: no lease");
}

/// Stores the secret `name` in the namespace and lets the publisher (`alice`) use it.
async fn declared_secret(p: &Platform, name: &str) {
    let namespace = p.namespace_id().await;
    let (stored, _) = p
        .rest(
            reqwest::Method::PUT,
            &format!("/namespaces/{namespace}/secrets/{name}"),
            Some(&json!({"value": "a-secret-value-long-enough-0001"})),
        )
        .await;
    assert_eq!(stored, 200);
    let (granted, _) = p
        .rest(
            reqwest::Method::PUT,
            &format!("/namespaces/{namespace}/secrets/{name}/access/alice"),
            None,
        )
        .await;
    assert_eq!(granted, 204);
}

/// One activity of one attempt, at the root or in the body of a `forEach` of one item, declaring `secrets`.
fn cancellable(name: &str, scoped: bool, secrets: &[&str]) -> Value {
    let mut activity = json!({
        "id": "a", "type": "activity", "activity": "act.a",
        "retry": {"maxAttempts": 1, "strategy": "fixed", "initialDelayMs": 1, "maxDelayMs": 1, "jitter": false}
    });
    if !secrets.is_empty() {
        activity["secrets"] = json!(secrets);
    }
    if !scoped {
        return json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
            "metadata": {"name": name}, "entrypoint": "a", "nodes": [activity]
        });
    }
    activity["next"] = json!("returned");
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": "a", "nodes": [
            activity,
            {"id": "returned", "type": "return", "result": "1"}
        ]}},
        "nodes": [
            {"id": "lines", "type": "forEach", "items": "input.lines", "body": "line",
             "result": "bodyResult", "maxItems": 1, "maxConcurrency": 1, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

/// Request (1) of decision D006-059, the part of this build (Task 006-069, decision D006-097): a host of 1.8 hands back a
/// lease still `LEASED` of a run whose cancellation was requested. It is withdrawn whatever the cancellation and the stop
/// reason say, at the root and in a scope, with and without declared secrets (the secret report fence let a stop through
/// only for `CANCELLED`): nothing fails, the attempt is not spent, and the stopping run ends `CANCELLED`.
#[tokio::test]
async fn a_hand_back_of_a_run_whose_cancellation_was_requested_is_withdrawn() {
    for (scoped, secrets) in [(false, false), (false, true), (true, false), (true, true)] {
        let case = format!("scoped={scoped} secrets={secrets}");
        let p = withdrawing_platform(None).await;
        let declared: &[&str] = if secrets { &["handback-key"] } else { &[] };
        if secrets {
            declared_secret(&p, "handback-key").await;
        }
        let name = format!("cancel-hand-back-{scoped}-{secrets}");
        let workflow = p
            .publish(&name, &cancellable(&name, scoped, declared))
            .await;
        let mut worker = p.worker("first").await;
        worker
            .register_version(&["act.a"], if scoped { "1.5" } else { "1.4" })
            .await;
        as_protocol_1_8(&p, "first").await;
        let (run, scheduler) = if scoped {
            let (status, body) = p
                .rest(
                    reqwest::Method::PUT,
                    "/quotas/iteration_concurrency",
                    Some(&json!({"limit": 8})),
                )
                .await;
            assert_eq!(status, 200, "{body}");
            let run = p.start(&workflow, &json!({"lines": [1]})).await;
            let scheduler = crate::support::blob_collections::scheduler(&p, None).await;
            (run, Some(scheduler))
        } else {
            (p.start(&workflow, &json!({})).await, None)
        };
        let mut lease = None;
        for _ in 0..20 {
            if let Some(scheduler) = &scheduler {
                scheduler.tick().await;
            }
            if let Some(found) = worker.poll(1).await.pop() {
                lease = Some(found);
                break;
            }
        }
        let lease = lease.unwrap_or_else(|| panic!("{case}: one lease"));
        assert_eq!(in_a_scope(&lease), scoped, "{case}");
        let (status, _) = p
            .rest(
                reqwest::Method::POST,
                &format!("/runs/{run}/operations"),
                Some(&json!({"action": "CANCEL", "reason": "the customer changed their mind"})),
            )
            .await;
        assert_eq!(status, 202, "{case}");
        if let Some(scheduler) = &scheduler {
            scheduler.tick().await;
        }
        worker
            .fail_with(&lease, "TASK_NOT_STARTED", true)
            .await
            .unwrap_or_else(|refused| panic!("{case}: the hand-back is refused: {refused:?}"));
        let run_id: Uuid = run.parse().expect("run id");
        if let Some(scheduler) = &scheduler {
            crate::support::blob_collections::tick_until_settled(&p, scheduler, run_id, 20).await;
        }
        assert_eq!(p.status(&run).await, "CANCELLED", "{case}");
        let journal = journal(&p, &run).await;
        let withdrawn = named(&journal, "step.lease_withdrawn");
        assert_eq!(withdrawn.len(), 1, "{case}: {journal:?}");
        assert_eq!(withdrawn[0]["cause"], "NOT_STARTED", "{case}");
        assert_eq!(withdrawn[0]["attempt"], 1, "{case}");
        assert!(named(&journal, "step.failed").is_empty(), "{case}");
        assert!(named(&journal, "step.needs_review").is_empty(), "{case}");
        assert!(named(&journal, "step.started").is_empty(), "{case}");
        replays(&p, &run).await;
    }
}

/// A member's gate, open, whose last decision under its epoch started `ago` before now.
fn gate_decided(ago: Duration) -> Arc<ServingGate> {
    let gate = ServingGate::closed();
    gate.decided_by_the_registry();
    gate.open(1);
    let now = std::time::Instant::now();
    gate.decided_at(now.checked_sub(ago).unwrap_or(now));
    Arc::new(gate)
}

/// Request (7) of decision D006-059 (Task 006-072, decision D006-098; the amendment of ADR-0082 decision 5): an engine whose
/// last beat decided under its epoch started more than 25 s ago serves no capability call (`ResolveSecrets`,
/// `ResolveDatabase`, `ReadBlob`, `WriteBlob`): each is `UNAVAILABLE` with `REGISTRY_SILENT` and a retry hint, while
/// `Heartbeat` and the reports are served. A beat that decides again serves them again.
#[tokio::test]
async fn a_silent_engine_serves_no_capability_call_and_keeps_heartbeats_and_reports() {
    let gate = gate_decided(Duration::from_secs(26));
    let mut options = Options::default();
    options.worker.idle_retry = Duration::from_millis(50);
    options.serving_gate = Some(Arc::clone(&gate));
    options.blobs = Some(sagat_application::blobs::BlobLimits {
        max_bytes: 1024 * 1024,
        max_pending: 8,
        upload_ttl: Duration::from_secs(3600),
        abandoned_after: Duration::from_secs(3600),
    });
    let p = platform_with(options).await;
    let workflow = p.publish("silent", &one_attempt("silent")).await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = stream_worker(&p, "first").await;
    let lease = worker.one_task().await;
    let silent = |status: &tonic::Status, call: &str| {
        assert_eq!(
            status.code(),
            tonic::Code::Unavailable,
            "{call}: {status:?}"
        );
        assert_eq!(
            machine_code(status),
            "REGISTRY_SILENT",
            "{call}: {status:?}"
        );
        assert_eq!(
            status
                .metadata()
                .get("retry-after-ms")
                .and_then(|value| value.to_str().ok()),
            Some("1000"),
            "{call}: a retry hint"
        );
    };
    let refused = worker
        .resolve_secrets(&lease)
        .await
        .expect_err("no secret is handed out");
    silent(&refused, "ResolveSecrets");
    let request = worker.req(Worker::resolve_secrets_message(&lease));
    let refused = worker
        .client
        .resolve_database(request)
        .await
        .expect_err("no credential is handed out");
    silent(&refused, "ResolveDatabase");
    let header = sagat_api_grpc::pb::WriteBlobHeader {
        content_type: "application/json".into(),
        ..sagat_api_grpc::pb::WriteBlobHeader::default()
    };
    let refused = worker
        .write_blob(&lease, header.clone(), b"{}", 2)
        .await
        .expect_err("nothing is stored");
    silent(&refused, "WriteBlob");
    let refused = worker
        .read_blob(&lease, &Uuid::now_v7().to_string())
        .await
        .expect_err("nothing is read");
    silent(&refused, "ReadBlob");
    // heartbeats and reports keep flowing
    worker
        .client
        .heartbeat(worker.req(worker.heartbeat_message(&lease)))
        .await
        .expect("the heartbeat is served");

    // a beat decided under its epoch: the calls are served again
    gate.decided_at(std::time::Instant::now());
    assert!(
        worker
            .resolve_secrets(&lease)
            .await
            .expect("served again")
            .is_empty()
    );
    let stored = worker
        .write_blob(&lease, header, b"{}", 2)
        .await
        .expect("stored");
    worker
        .read_blob(&lease, &stored.blob_id)
        .await
        .expect("read");
    worker
        .complete(&lease, &json!({"ok": true}))
        .await
        .expect("the report is served");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
}
