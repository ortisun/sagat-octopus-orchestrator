//! US03 — analyze a real execution: the graph reflects the journal (US03-AC01), is followed by
//! journal sequence without reloading it and without missing a transition (FR-007-005,
//! FR-007-006), by polling or by a stream, and a stream is bounded in number, in time and by the
//! token that opened it (D142).

use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::Method;
use sagat_api_http::streams::StreamLimits;
use serde_json::{Value, json};

use crate::support::api::Options;
use crate::support::worker::{HUMAN, TENANT, platform, platform_with};
use crate::{Stream, activities, apply, by_id, chain, changes, graph};

fn names(delta: &Value, field: &str) -> Vec<String> {
    delta[field]
        .as_array()
        .expect("list")
        .iter()
        .map(|v| {
            v["eventType"]
                .as_str()
                .or(v["id"].as_str())
                .expect("name")
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn us03_the_changes_after_a_sequence_carry_the_nodes_the_journal_touched_and_no_other() {
    let p = platform().await;
    let wf = p.publish("order-flow", &chain("order-flow", 4)).await;
    let run = p.start(&wf, &json!({})).await;
    let acts = activities(4);
    let acts: Vec<&str> = acts.iter().map(String::as_str).collect();
    let mut w = p.worker("wendy").await;
    w.register(&acts).await;

    let before = graph(&p, &run).await;
    let seq = before["projectionSeq"].as_u64().expect("seq");

    // Nothing happened since: an empty answer, not an error.
    let quiet = changes(&p, &run, seq).await;
    assert_eq!(quiet["reset"], false);
    assert_eq!(quiet["events"], json!([]));
    assert_eq!(quiet["nodes"], json!([]));
    assert_eq!(quiet["projectionSeq"], seq);

    // A worker completes the first step: the second is released. a2 and a3 are untouched.
    let task = w.one_task().await;
    assert_eq!(task.activity_name, "act.0");
    w.complete(&task, &json!({})).await.expect("a0");
    let delta = changes(&p, &run, seq).await;
    assert_eq!(delta["reset"], false, "{delta}");
    assert_eq!(delta["fromSeq"], seq);
    assert!(delta["projectionSeq"].as_u64().expect("seq") > seq);
    let touched: Vec<String> = by_id(&delta).keys().cloned().collect();
    assert!(
        touched.contains(&"a0".to_owned()) && touched.contains(&"a1".to_owned()),
        "{touched:?}"
    );
    assert!(
        !touched.contains(&"a2".to_owned()) && !touched.contains(&"a3".to_owned()),
        "steps that did not move are not sent again: {touched:?}"
    );
    let kinds = names(&delta, "events");
    assert!(kinds.contains(&"step.completed".to_owned()), "{kinds:?}");
    let seqs: Vec<u64> = delta["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| e["seq"].as_u64().expect("seq"))
        .collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]) && seqs[0] > seq,
        "{seqs:?}"
    );
    assert_eq!(
        *seqs.last().expect("events"),
        delta["projectionSeq"],
        "the last event is the new projection"
    );
    assert!(
        delta.get("edges").is_none(),
        "edges travel only with a reset: {delta}"
    );
}

#[tokio::test]
async fn us03_applying_every_change_in_order_always_gives_the_graph_the_server_would_send_whole() {
    let p = platform().await;
    let retry = json!({"maxAttempts": 2, "strategy": "fixed", "initialDelayMs": 1000, "maxDelayMs": 1000, "jitter": false});
    let mut ir = chain("order-flow", 3);
    ir["nodes"][1]["retry"] = retry;
    let wf = p.publish("order-flow", &ir).await;
    let run = p.start(&wf, &json!({})).await;
    let acts = activities(3);
    let acts: Vec<&str> = acts.iter().map(String::as_str).collect();
    let mut w = p.worker("wendy").await;
    w.register(&acts).await;

    // What the Studio holds: the graph it loaded, then only deltas.
    let loaded = graph(&p, &run).await;
    let mut model: BTreeMap<String, Value> = by_id(&loaded);
    let mut seq = loaded["projectionSeq"].as_u64().expect("seq");

    macro_rules! sync {
        ($label:expr) => {{
            let delta = changes(&p, &run, seq).await;
            apply(&mut model, &mut seq, &delta);
            let whole = graph(&p, &run).await;
            assert_eq!(
                model,
                by_id(&whole),
                "after {}: the deltas and the whole graph disagree",
                $label
            );
            assert_eq!(seq, whole["projectionSeq"].as_u64().expect("seq"));
        }};
    }

    let first = w.one_task().await;
    sync!("a0 leased");
    w.complete(&first, &json!({})).await.expect("a0");
    sync!("a0 completed");
    let second = w.one_task().await;
    sync!("a1 leased");
    w.fail_with(&second, "BUSY", true)
        .await
        .expect("a1 fails, retryable");
    sync!("a1 failed and a retry scheduled");
    assert_eq!(model["a1"]["status"], "RETRY_WAIT");
    p.elapse_retry_delay(&run, "a1").await;
    p.tick().await;
    sync!("the retry came due");
    let again = w.one_task().await;
    assert_eq!(again.attempt, 2);
    sync!("a1 leased again");

    // A request that names no step: the run is asked to cancel, and every started step changes.
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/runs/{run}/operations"),
            Some(&json!({"action": "CANCEL", "reason": "the customer changed their mind"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    sync!("cancel requested");
    assert_eq!(
        model["a1"]["cancelRequested"], true,
        "a run-level event reaches the started steps"
    );
    w.complete(&again, &json!({}))
        .await
        .expect("a1 reports anyway");
    p.tick().await;
    sync!("the run ended");
    assert!(
        ["CANCELLED", "SUCCEEDED"]
            .contains(&graph(&p, &run).await["status"].as_str().expect("status")),
    );
}

/// Three steps of a saga, each undone by an activity of its own: when the third fails for good,
/// the undo of the second runs first and the undo of the first waits for it.
fn three_steps_with_undo() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "three-steps"}, "entrypoint": "tx",
        "nodes": [
            {"id": "tx", "type": "saga", "begin": "a", "next": "done",
             "onCompensationFailure": "manual_review", "cancelPolicy": "compensate_completed"},
            {"id": "a", "type": "activity", "activity": "act.a", "next": "b", "compensateWith": "undo.a"},
            {"id": "b", "type": "activity", "activity": "act.b", "next": "c", "compensateWith": "undo.b"},
            {"id": "c", "type": "activity", "activity": "act.c", "retry": {"maxAttempts": 1}},
            {"id": "done", "type": "end"}
        ]
    })
}

#[tokio::test]
async fn us03_the_deltas_still_equal_the_whole_graph_when_a_plan_activates_steps_that_wait_their_turn()
 {
    let p = platform().await;
    let wf = p.publish("three-steps", &three_steps_with_undo()).await;
    let run = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a", "act.b", "act.c", "undo.a", "undo.b"])
        .await;

    let loaded = graph(&p, &run).await;
    let mut model: BTreeMap<String, Value> = by_id(&loaded);
    let mut seq = loaded["projectionSeq"].as_u64().expect("seq");
    macro_rules! sync {
        ($label:expr) => {{
            let delta = changes(&p, &run, seq).await;
            apply(&mut model, &mut seq, &delta);
            let whole = graph(&p, &run).await;
            assert_eq!(
                model,
                by_id(&whole),
                "after {}: the deltas and the whole graph disagree",
                $label
            );
        }};
    }

    for label in ["a", "b"] {
        let task = w.one_task().await;
        sync!(format!("{label} leased"));
        w.complete(&task, &json!({})).await.expect("completes");
        sync!(format!("{label} completed"));
    }
    let third = w.one_task().await;
    w.fail(&third, false).await.expect("c is declined for good");
    // The plan names the two undos; the first of them runs now and the other waits for it. A client
    // that was told only about the steps the events name would still show the waiting undo as one
    // the run has not reached.
    sync!("the undo was planned");
    assert_eq!(
        model["a~c"]["status"], "PENDING",
        "the undo of a waits for the undo of b: {}",
        model["a~c"]
    );
    let undo_b = w.one_task().await;
    sync!("the undo of b leased");
    w.complete(&undo_b, &json!({})).await.expect("undo b");
    sync!("the undo of b completed");
    let undo_a = w.one_task().await;
    w.complete(&undo_a, &json!({})).await.expect("undo a");
    p.tick().await;
    sync!("the saga was compensated");
    assert_eq!(graph(&p, &run).await["status"], "COMPENSATED");
}

#[tokio::test]
async fn us03_a_client_that_is_ahead_or_far_behind_is_given_the_whole_graph() {
    let p = platform().await;
    let wf = p.publish("long-flow", &chain("long-flow", 140)).await;
    let run = p.start(&wf, &json!({})).await;
    let acts = activities(140);
    let acts: Vec<&str> = acts.iter().map(String::as_str).collect();
    let mut w = p.worker("wendy").await;
    w.register(&acts).await;
    for _ in 0..140 {
        let task = w.one_task().await;
        w.complete(&task, &json!({})).await.expect("step");
    }
    let whole = graph(&p, &run).await;
    let seq = whole["projectionSeq"].as_u64().expect("seq");
    assert!(
        seq > 500,
        "the journal is longer than a delta carries: {seq}"
    );

    // Ahead of the run (a stale cache of another run, a rewind): replace everything.
    let ahead = changes(&p, &run, seq + 10).await;
    assert_eq!(ahead["reset"], true);
    assert_eq!(ahead["fromSeq"], 0);
    assert_eq!(by_id(&ahead), by_id(&whole));
    assert!(
        ahead["edges"].as_array().expect("edges").len() >= 139,
        "{}",
        ahead["edges"]
    );
    assert_eq!(ahead["events"], json!([]));

    // Far behind: cheaper to start again than to replay hundreds of events node by node.
    let behind = changes(&p, &run, 0).await;
    assert_eq!(behind["reset"], true);
    assert_eq!(by_id(&behind), by_id(&whole));

    // A little behind is an ordinary delta.
    let near = changes(&p, &run, seq - 3).await;
    assert_eq!(near["reset"], false);
    assert_eq!(near["events"].as_array().expect("events").len(), 3);
}

#[tokio::test]
async fn us03_the_changes_are_the_runs_own_and_need_run_read() {
    let p = platform().await;
    let wf = p.publish("order-flow", &chain("order-flow", 2)).await;
    let run = p.start(&wf, &json!({})).await;

    for bad in ["", "?since=-1", "?since=x", "?since=1.5", "?since="] {
        let (status, body) = p
            .rest(
                Method::GET,
                &format!("/runs/{run}/graph/changes{bad}"),
                None,
            )
            .await;
        assert_eq!(status, 400, "{bad:?}: {body}");
    }
    let response = p
        .api
        .as_user(
            Method::GET,
            &format!("/runs/{run}/graph/changes?since=0"),
            "bob",
            "globex",
        )
        .send()
        .await
        .expect("send");
    assert_eq!(
        response.status(),
        404,
        "another tenant's run does not exist for us"
    );
    let unauthenticated = p
        .api
        .request(Method::GET, &format!("/runs/{run}/graph/changes?since=0"))
        .send()
        .await
        .expect("send");
    assert_eq!(unauthenticated.status(), 401);
    let (status, _) = p
        .rest(
            Method::GET,
            "/runs/019b5d6e-0000-7000-8000-000000000000/graph/changes?since=0",
            None,
        )
        .await;
    assert_eq!(status, 404);

    // A machine identity that executes tasks may not read runs, and is refused the changes and the
    // stream alike (a member of the tenant, so the refusal is about the permission and not the tenant).
    p.grant(p.tenant, "wanda", sagat_domain::identity::Role::Worker)
        .await;
    let (status, body) = p
        .rest_as(
            "wanda",
            Method::GET,
            &format!("/runs/{run}/graph/changes?since=0"),
            None,
        )
        .await;
    assert_eq!(status, 403, "{body}");
    let (status, body) = Stream::open(&p, &run, "wanda", TENANT, None)
        .await
        .err()
        .expect("a worker may not watch a run");
    assert_eq!(status, 403, "{body}");
}

#[tokio::test]
async fn us03_a_stream_sends_the_graph_then_every_change_and_ends_when_the_run_does() {
    let p = platform().await;
    let wf = p.publish("order-flow", &chain("order-flow", 3)).await;
    let run = p.start(&wf, &json!({})).await;
    let acts = activities(3);
    let acts: Vec<&str> = acts.iter().map(String::as_str).collect();
    let mut w = p.worker("wendy").await;
    w.register(&acts).await;

    let mut stream = Stream::open_as_alice(&p, &run).await;
    let first = stream
        .next(Duration::from_secs(5))
        .await
        .expect("the first event is sent at once");
    assert_eq!(first.event, "graph");
    assert_eq!(first.data["reset"], true, "{first:?}");
    assert_eq!(
        first.id.as_deref(),
        Some(first.data["projectionSeq"].to_string().as_str())
    );
    let mut model: BTreeMap<String, Value> = BTreeMap::new();
    let mut seq = 0;
    apply(&mut model, &mut seq, &first.data);
    assert_eq!(model.len(), 3, "the whole graph, not started nodes too");

    // Every task the worker completes arrives as a change; the ids only grow.
    let mut last_id = seq;
    for _ in 0..3 {
        let task = w.one_task().await;
        w.complete(&task, &json!({})).await.expect("step");
        loop {
            let message = stream
                .next(Duration::from_secs(10))
                .await
                .expect("the change arrives");
            assert_eq!(message.event, "graph", "{message:?}");
            let id: u64 = message.id.as_deref().expect("id").parse().expect("number");
            assert!(id > last_id, "{id} after {last_id}");
            last_id = id;
            apply(&mut model, &mut seq, &message.data);
            // The chain names its activities `act.N` after the nodes `aN`.
            let node = task.activity_name.replace("act.", "a");
            if model[&node]["status"] == "SUCCEEDED" {
                break;
            }
        }
    }
    p.tick().await;
    let (end, rest) = stream
        .until("end", Duration::from_secs(10))
        .await
        .expect("the stream says the run ended and closes");
    for message in rest {
        apply(&mut model, &mut seq, &message.data);
    }
    let _ = end;
    assert_eq!(
        model,
        by_id(&graph(&p, &run).await),
        "what the stream said is the graph"
    );
    assert!(
        stream.next(Duration::from_secs(3)).await.is_none(),
        "nothing after `end`: the server closes"
    );
}

#[tokio::test]
async fn us03_a_client_that_reconnects_with_the_last_event_id_misses_nothing() {
    let p = platform().await;
    let wf = p.publish("order-flow", &chain("order-flow", 3)).await;
    let run = p.start(&wf, &json!({})).await;
    let acts = activities(3);
    let acts: Vec<&str> = acts.iter().map(String::as_str).collect();
    let mut w = p.worker("wendy").await;
    w.register(&acts).await;

    let mut stream = Stream::open_as_alice(&p, &run).await;
    let first = stream.next(Duration::from_secs(5)).await.expect("first");
    let seen = first.id.clone().expect("id");
    drop(stream); // the network went away

    // Things happen while nobody listens, including a signal-like event of no step at all.
    let task = w.one_task().await;
    w.complete(&task, &json!({})).await.expect("step");
    let task = w.one_task().await;
    w.complete(&task, &json!({})).await.expect("step");

    let mut again = Stream::open(&p, &run, HUMAN, TENANT, Some(&seen))
        .await
        .expect("opens");
    let resumed = again
        .next(Duration::from_secs(5))
        .await
        .expect("changes since");
    assert_eq!(resumed.data["reset"], false, "{resumed:?}");
    assert_eq!(resumed.data["fromSeq"].to_string(), seen);
    let kinds = names(&resumed.data, "events");
    assert!(
        kinds.iter().filter(|k| *k == "step.completed").count() >= 2,
        "both completions that happened while the client was away: {kinds:?}"
    );
    let mut model: BTreeMap<String, Value> = BTreeMap::new();
    let mut seq = 0;
    apply(&mut model, &mut seq, &first.data);
    apply(&mut model, &mut seq, &resumed.data);
    assert_eq!(model, by_id(&graph(&p, &run).await));

    // A header that is not a sequence is refused, not guessed at.
    let bad = Stream::open(&p, &run, HUMAN, TENANT, Some("abc")).await;
    assert_eq!(bad.err().map(|e| e.0), Some(400));
}

/// Scans of the table of steps so far (sequential and by index), as PostgreSQL counts them. The
/// counters reach the statistics a moment after the work, so the reader waits for them.
async fn step_scans(p: &crate::support::worker::Platform) -> i64 {
    tokio::time::sleep(Duration::from_millis(2500)).await;
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(seq_scan, 0) + COALESCE(idx_scan, 0) FROM pg_stat_user_tables WHERE relname = 'steps'",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the statistics of the steps table")
}

#[tokio::test]
async fn us03_a_stream_of_a_quiet_run_costs_a_look_at_the_run_each_tick_and_not_a_read_of_its_graph()
 {
    let p = platform_with(Options {
        streams: Some(StreamLimits {
            poll_min: Duration::from_millis(50),
            poll_max: Duration::from_millis(50),
            ..StreamLimits::default()
        }),
        ..Options::default()
    })
    .await;
    let wf = p.publish("order-flow", &chain("order-flow", 2)).await;
    let run = p.start(&wf, &json!({})).await;
    let mut stream = Stream::open_as_alice(&p, &run).await;
    stream
        .next(Duration::from_secs(5))
        .await
        .expect("the graph");

    let before = step_scans(&p).await;
    // A run nobody works on does not move: about eighty ticks of the stream go by.
    assert!(
        stream.next(Duration::from_secs(4)).await.is_none(),
        "a quiet run sends nothing but pings"
    );
    let after = step_scans(&p).await;
    // Each tick used to read every step of the run, so this would have been about eighty.
    assert!(
        after - before <= 5,
        "{} scans of the steps table in four seconds of a stream of a run that did not move",
        after - before
    );
}

#[tokio::test]
async fn us03_a_stream_ends_for_someone_who_is_no_longer_a_member_without_waiting_for_its_lifetime()
{
    let p = platform_with(Options {
        streams: Some(StreamLimits {
            poll_min: Duration::from_millis(50),
            poll_max: Duration::from_millis(50),
            recheck: Duration::from_millis(300),
            ..StreamLimits::default()
        }),
        ..Options::default()
    })
    .await;
    let wf = p.publish("order-flow", &chain("order-flow", 2)).await;
    let run = p.start(&wf, &json!({})).await;
    p.grant(p.tenant, "vera", sagat_domain::identity::Role::Viewer)
        .await;
    let mut stream = Stream::open(&p, &run, "vera", TENANT, None)
        .await
        .expect("a viewer may watch");
    stream
        .next(Duration::from_secs(5))
        .await
        .expect("the graph");

    // An administrator removes her. Every ordinary request of hers is refused from now on.
    sqlx::query("DELETE FROM memberships WHERE tenant_id = $1 AND subject = 'vera'")
        .bind(p.tenant.as_uuid())
        .execute(&p.api.db.pool)
        .await
        .expect("removing the member");

    // The stream that was open does not go on telling her about the run for its fifteen minutes.
    let ended = stream.until("end", Duration::from_secs(5)).await;
    assert!(ended.is_some(), "the stream of a removed member went on");
}

#[tokio::test]
async fn us03_streams_are_limited_per_subject_and_a_closed_stream_frees_its_place() {
    let p = platform_with(Options {
        streams: Some(StreamLimits {
            per_subject: 2,
            per_tenant: 10,
            poll_min: Duration::from_millis(50),
            poll_max: Duration::from_millis(100),
            ping: Duration::from_millis(200),
            ..StreamLimits::default()
        }),
        ..Options::default()
    })
    .await;
    let wf = p.publish("order-flow", &chain("order-flow", 2)).await;
    let run = p.start(&wf, &json!({})).await;

    let mut one = Stream::open_as_alice(&p, &run).await;
    let _two = Stream::open_as_alice(&p, &run).await;
    let refused = Stream::open(&p, &run, HUMAN, TENANT, None).await;
    let (status, body) = refused.err().expect("a third stream is refused");
    assert_eq!(status, 429, "{body}");
    assert_eq!(body["details"]["reason"], "STREAMS_PER_SUBJECT");
    assert_eq!(body["retryable"], true);

    // Another subject has a place of its own.
    p.grant(p.tenant, "olivia", sagat_domain::identity::Role::Operator)
        .await;
    assert!(Stream::open(&p, &run, "olivia", TENANT, None).await.is_ok());

    // Leaving frees the place (the server notices at its next look).
    let _ = one.next(Duration::from_secs(2)).await;
    drop(one);
    let mut opened = false;
    for _ in 0..40 {
        if Stream::open(&p, &run, HUMAN, TENANT, None).await.is_ok() {
            opened = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(opened, "a stream that was closed still holds its place");
}

/// Opens streams as alice and olivia, and shows what a third subject (vera) is told.
async fn third_stream_is_refused(limits: StreamLimits) -> (u16, Value) {
    let p = platform_with(Options {
        streams: Some(StreamLimits {
            poll_min: Duration::from_millis(50),
            poll_max: Duration::from_millis(100),
            ..limits
        }),
        ..Options::default()
    })
    .await;
    let wf = p.publish("order-flow", &chain("order-flow", 2)).await;
    let run = p.start(&wf, &json!({})).await;
    p.grant(p.tenant, "olivia", sagat_domain::identity::Role::Operator)
        .await;
    p.grant(p.tenant, "vera", sagat_domain::identity::Role::Viewer)
        .await;
    let _a = Stream::open_as_alice(&p, &run).await;
    let _b = Stream::open(&p, &run, "olivia", TENANT, None)
        .await
        .expect("the second place");
    Stream::open(&p, &run, "vera", TENANT, None)
        .await
        .err()
        .expect("the third is refused")
}

#[tokio::test]
async fn us03_the_tenant_may_not_hold_more_streams_than_its_cap_whoever_asks() {
    let (status, body) = third_stream_is_refused(StreamLimits {
        per_tenant: 2,
        per_subject: 5,
        ..StreamLimits::default()
    })
    .await;
    assert_eq!(status, 429, "{body}");
    assert_eq!(body["details"]["reason"], "STREAMS_PER_TENANT");
    assert_eq!(body["retryable"], true);
}

#[tokio::test]
async fn us03_the_platform_as_a_whole_may_not_hold_more_streams_than_its_cap() {
    let (status, body) = third_stream_is_refused(StreamLimits {
        global: 2,
        per_tenant: 50,
        per_subject: 5,
        ..StreamLimits::default()
    })
    .await;
    assert_eq!(status, 429, "{body}");
    assert_eq!(body["details"]["reason"], "STREAMS_GLOBAL");
}

#[tokio::test]
async fn us03_a_stream_never_outlives_the_token_that_opened_it() {
    let p = platform().await;
    let wf = p.publish("order-flow", &chain("order-flow", 2)).await;
    let run = p.start(&wf, &json!({})).await;
    // A token that lives three more seconds.
    let token = p.api.idp.token(HUMAN, TENANT).issued(0, 4).build();
    let response = p
        .api
        .request(Method::GET, &format!("/runs/{run}/stream"))
        .bearer_auth(token)
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), 200);
    let started = std::time::Instant::now();
    let mut body = response;
    let mut text = String::new();
    while let Ok(Ok(Some(chunk))) =
        tokio::time::timeout(Duration::from_secs(15), body.chunk()).await
    {
        text.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(
        started.elapsed() < Duration::from_secs(12),
        "the stream went on for {:?} past a 4 s token",
        started.elapsed()
    );
    assert!(
        text.contains("event: reconnect"),
        "the client is told to come back with a new token: {text}"
    );
}

#[tokio::test]
async fn us03_a_stream_belongs_to_the_runs_tenant_and_needs_a_valid_token() {
    let p = platform().await;
    let wf = p.publish("order-flow", &chain("order-flow", 2)).await;
    let run = p.start(&wf, &json!({})).await;

    let other = Stream::open(&p, &run, "bob", "globex", None).await;
    assert_eq!(other.err().map(|e| e.0), Some(404));
    let unauthenticated = p
        .api
        .request(Method::GET, &format!("/runs/{run}/stream"))
        .send()
        .await
        .expect("send");
    assert_eq!(unauthenticated.status(), 401);
    let missing = Stream::open(
        &p,
        "019b5d6e-0000-7000-8000-000000000000",
        HUMAN,
        TENANT,
        None,
    )
    .await;
    assert_eq!(missing.err().map(|e| e.0), Some(404));
    let garbage = Stream::open(&p, "not-a-run", HUMAN, TENANT, None).await;
    assert_eq!(garbage.err().map(|e| e.0), Some(404));
}
