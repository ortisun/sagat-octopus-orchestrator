//! US01 — inspect timeline and attempt: a run with three attempts has a timeline in which every
//! transition has a time, a cause and a trace, an attempt endpoint that tells each attempt with the
//! span it has in the trace, and a statement of how far the spans have got in relation to the journal
//! (US01-AC01, FR-008-002, the edge cases "trace missing but journal present" and "lagging
//! read-model").

use reqwest::Method;
use sagat_telemetry::{SpanId, TraceContext, TraceId, attempt_span_id, traceparent};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::collector::Collector;
use crate::support::worker::{HUMAN, NAMESPACE, Platform, TENANT, platform, single_ir};
use crate::us02::projector;

fn retrying() -> Value {
    single_ir(
        "payment",
        Some(
            json!({"maxAttempts": 3, "strategy": "fixed", "initialDelayMs": 1, "maxDelayMs": 1, "jitter": false}),
        ),
    )
}

/// A run that fails twice with a code and succeeds the third time. Returns the run, the trace it
/// belongs to and the traceparent each lease carried.
async fn three_attempts(p: &Platform) -> (String, TraceId, Vec<String>) {
    p.bind_everything("worker-1").await;
    let workflow = p.publish("payment", &retrying()).await;
    let (trace, caller) = (TraceId::generate(), SpanId::generate());
    let response = p
        .api
        .as_user(Method::POST, "/runs", HUMAN, TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", format!("key-{}", Uuid::now_v7()))
        .header("traceparent", traceparent(trace, caller, true))
        .json(&json!({"workflowId": workflow, "input": {"n": 1}}))
        .send()
        .await
        .expect("send");
    let run = response.json::<Value>().await.expect("json")["runId"]
        .as_str()
        .expect("run")
        .to_owned();
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let mut leases = Vec::new();
    for attempt in 1..=3 {
        let lease = worker.one_task().await;
        leases.push(lease.traceparent.clone());
        if attempt < 3 {
            worker.fail(&lease, true).await.expect("fail");
            p.elapse_retry_delay(&run, "a").await;
            p.tick().await;
        } else {
            worker
                .complete(&lease, &json!({"ok": true}))
                .await
                .expect("complete");
        }
    }
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    (run, trace, leases)
}

async fn get(p: &Platform, path: &str) -> (u16, Value) {
    p.rest(Method::GET, path, None).await
}

#[tokio::test]
async fn us01_every_transition_of_a_run_with_three_attempts_has_a_time_a_cause_and_a_trace() {
    let p = platform().await;
    let (run, trace, leases) = three_attempts(&p).await;

    let (status, page) = get(&p, &format!("/runs/{run}/events?limit=200")).await;
    assert_eq!(status, 200, "{page}");
    let events = page["items"].as_array().expect("items");
    assert!(events.len() >= 10, "{events:?}");
    for event in events {
        assert!(
            event["occurredAt"]
                .as_str()
                .is_some_and(|t| t.contains('T')),
            "a time: {event}"
        );
        assert_eq!(
            event["traceId"],
            trace.to_string(),
            "the run's trace on every transition: {event}"
        );
    }

    let by_type =
        |name: &str| -> Vec<&Value> { events.iter().filter(|e| e["eventType"] == name).collect() };
    // The two failed attempts say why they were retried.
    let retries = by_type("step.retry_scheduled");
    assert_eq!(retries.len(), 2);
    for retry in retries {
        assert_eq!(
            retry["cause"],
            json!({"kind": "retry", "code": "PAYMENT_DECLINED"}),
            "{retry}"
        );
    }
    // The run says how it ended.
    assert_eq!(
        by_type("run.finished")[0]["cause"],
        json!({"kind": "outcome", "code": "SUCCEEDED"})
    );
    // Progress has no cause.
    assert!(by_type("step.leased").iter().all(|e| e["cause"].is_null()));

    // The events of an attempt carry the span the lease gave the worker, and a different one per attempt.
    let leased = by_type("step.leased");
    assert_eq!(leased.len(), 3);
    for (event, lease) in leased.iter().zip(&leases) {
        let from_lease = TraceContext::from_traceparent(Some(lease))
            .parent_span_id()
            .expect("a span")
            .to_string();
        assert_eq!(
            event["spanId"],
            from_lease.as_str(),
            "the timeline names the attempt's span: {event}"
        );
    }
    let spans: std::collections::HashSet<_> = leased
        .iter()
        .map(|e| e["spanId"].as_str().expect("span"))
        .collect();
    assert_eq!(spans.len(), 3);
    assert!(
        by_type("run.started")[0]["spanId"].is_null(),
        "a run is not an attempt"
    );
}

#[tokio::test]
async fn us01_the_attempts_endpoint_tells_each_attempt_with_its_outcome_its_code_and_its_span() {
    let p = platform().await;
    let (run, trace, leases) = three_attempts(&p).await;
    let (status, body) = get(&p, &format!("/runs/{run}/attempts")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["traceId"], trace.to_string());
    let attempts = body["items"].as_array().expect("items");
    assert_eq!(attempts.len(), 3);
    let outcomes: Vec<(&str, Option<&str>)> = attempts
        .iter()
        .map(|a| (a["outcome"].as_str().expect("outcome"), a["code"].as_str()))
        .collect();
    assert_eq!(
        outcomes,
        [
            ("retry", Some("PAYMENT_DECLINED")),
            ("retry", Some("PAYMENT_DECLINED")),
            ("succeeded", None)
        ]
    );
    let step: Uuid =
        sqlx::query_scalar("SELECT step_id FROM steps WHERE run_id = $1::uuid AND node = 'a'")
            .bind(&run)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("step");
    for (n, (attempt, lease)) in attempts.iter().zip(&leases).enumerate() {
        let id = attempt_span_id(step, u32::try_from(n + 1).expect("small")).to_string();
        assert_eq!(attempt["spanId"], id.as_str());
        assert_eq!(
            TraceContext::from_traceparent(Some(lease))
                .parent_span_id()
                .expect("span")
                .to_string(),
            id
        );
        assert_eq!(attempt["attempt"], n + 1);
        assert!(attempt["endedAt"].is_string() && attempt["leasedAt"].is_string());
        assert!(
            attempt["scheduleLatencyMs"].as_u64().is_some(),
            "the journal says when it became ready: {attempt}"
        );
    }
}

#[tokio::test]
async fn us01_the_state_of_the_telemetry_says_how_far_the_spans_are_behind_the_journal_which_stays_whole()
 {
    let p = platform().await;
    let collector = Collector::start().await;
    let (run, trace, _) = three_attempts(&p).await;

    // The journal moved and nothing has been projected: catching up, and the timeline is whole.
    let (status, state) = get(&p, &format!("/runs/{run}/telemetry")).await;
    assert_eq!(status, 200, "{state}");
    assert_eq!(state["state"], "CATCHING_UP");
    assert_eq!(state["traceId"], trace.to_string());
    assert!(state["appliedSeq"].as_u64() < state["headSeq"].as_u64());
    assert_eq!(state["budgetMs"], 30_000);
    let (_, timeline) = get(&p, &format!("/runs/{run}/events?limit=200")).await;
    let whole = timeline["items"].as_array().expect("items").len();
    assert!(whole >= 10);

    // The collector is down for a long time: the trace is missing steps, the journal is not.
    collector.go_down();
    let projector = projector(&p, Some(&collector));
    assert!(projector.run_once().await.failed);
    sqlx::query("UPDATE telemetry_pending SET behind_since = clock_timestamp() - interval '10 minutes' WHERE run_id = $1::uuid")
        .bind(&run)
        .execute(&p.api.db.pool)
        .await
        .expect("time passes");
    let (_, state) = get(&p, &format!("/runs/{run}/telemetry")).await;
    assert_eq!(state["state"], "BEHIND");
    assert!(state["lagMs"].as_u64().expect("lag") >= 590_000, "{state}");
    let (_, timeline) = get(&p, &format!("/runs/{run}/events?limit=200")).await;
    assert_eq!(
        timeline["items"].as_array().expect("items").len(),
        whole,
        "the journal is whole while the trace is not"
    );

    // It comes back: the projector catches up and the state says so.
    collector.recover();
    assert!(!projector.run_once().await.failed);
    let (_, state) = get(&p, &format!("/runs/{run}/telemetry")).await;
    assert_eq!(state["state"], "CURRENT");
    assert_eq!(state["lagMs"], 0);
    assert_eq!(state["appliedSeq"], state["headSeq"]);
}

#[tokio::test]
async fn us01_a_run_with_no_trace_and_a_run_that_predates_the_projection_say_so() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let workflow = p.publish("older", &single_ir("older", None)).await;
    let run = p.start(&workflow, &json!({})).await;
    sqlx::query("UPDATE runs SET trace_id = NULL WHERE id = $1::uuid")
        .bind(&run)
        .execute(&p.api.db.pool)
        .await
        .expect("a run with no trace");
    assert_eq!(
        get(&p, &format!("/runs/{run}/telemetry")).await.1["state"],
        "NO_TRACE"
    );
    let (_, attempts) = get(&p, &format!("/runs/{run}/attempts")).await;
    assert_eq!(
        attempts["items"],
        json!([]),
        "a run with no trace has no attempts to derive"
    );

    let other = p.start(&workflow, &json!({})).await;
    sqlx::query("DELETE FROM telemetry_pending WHERE run_id = $1::uuid")
        .bind(&other)
        .execute(&p.api.db.pool)
        .await
        .expect("a run from before notes");
    assert_eq!(
        get(&p, &format!("/runs/{other}/telemetry")).await.1["state"],
        "UNTRACKED"
    );
}

#[tokio::test]
async fn us01_only_those_who_may_read_runs_see_them_and_never_another_tenants() {
    let p = platform().await;
    let (run, _, _) = three_attempts(&p).await;
    p.grant(p.tenant, "viewer-1", sagat_domain::identity::Role::Viewer)
        .await;
    p.grant(p.tenant, "worker-9", sagat_domain::identity::Role::Worker)
        .await;
    for path in [
        format!("/runs/{run}/telemetry"),
        format!("/runs/{run}/attempts"),
    ] {
        assert_eq!(
            p.rest_as("viewer-1", Method::GET, &path, None).await.0,
            200,
            "{path}"
        );
        assert_eq!(
            p.rest_as("worker-9", Method::GET, &path, None).await.0,
            403,
            "{path}"
        );
        let foreign = p
            .api
            .as_user(Method::GET, &path, "bob", "globex")
            .send()
            .await
            .expect("send");
        assert_eq!(
            foreign.status().as_u16(),
            404,
            "{path}: another tenant's run does not exist for them"
        );
    }
    assert_eq!(
        get(&p, &format!("/runs/{}/telemetry", Uuid::now_v7()))
            .await
            .0,
        404
    );
}
