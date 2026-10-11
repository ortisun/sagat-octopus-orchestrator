//! US02 — correlate external calls: an activity that calls a provider with the trace the lease gave
//! it is one trace from the request that started the run, through the scheduler and the worker, to
//! the provider (US02-AC01). The spans of the run and of its attempts are derived from the journal
//! by the projector, so they are there whatever happened to the collector in between (FR-008-001).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use reqwest::Method;
use sagat_scheduler::{ProjectorSettings, TelemetryProjector};
use sagat_telemetry::{SpanId, TraceContext, TraceId, attempt_span_id, run_span_id, traceparent};
use serde_json::json;
use uuid::Uuid;

use crate::support::collector::Collector;
use crate::support::worker::{HUMAN, NAMESPACE, Platform, TENANT, platform, single_ir};

/// A provider that records the `traceparent` of every call it receives.
async fn provider() -> (String, Arc<Mutex<Vec<String>>>) {
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    async fn charge(
        State(seen): State<Arc<Mutex<Vec<String>>>>,
        headers: HeaderMap,
    ) -> &'static str {
        let header = headers
            .get("traceparent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        seen.lock().expect("seen").push(header.to_owned());
        "charged"
    }
    let app = Router::new()
        .route("/charge", post(charge))
        .with_state(Arc::clone(&seen));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let url = format!("http://{}/charge", listener.local_addr().expect("addr"));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (url, seen)
}

pub fn projector(
    p: &Platform,
    collector: Option<&Collector>,
) -> TelemetryProjector<sagat_infra_postgres::PgRunStore> {
    TelemetryProjector::new(
        p.api.executions.clone(),
        collector.map(Collector::client),
        ProjectorSettings::default(),
    )
}

async fn start_traced(p: &Platform, workflow: &str, header: &str) -> String {
    let response = p
        .api
        .as_user(Method::POST, "/runs", HUMAN, TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", format!("key-{}", Uuid::now_v7()))
        .header("traceparent", header)
        .json(&json!({"workflowId": workflow, "input": {"n": 1}}))
        .send()
        .await
        .expect("send");
    assert_eq!(response.status().as_u16(), 202);
    response.json::<serde_json::Value>().await.expect("json")["runId"]
        .as_str()
        .expect("run id")
        .to_owned()
}

fn retrying() -> serde_json::Value {
    single_ir(
        "payment",
        Some(
            json!({"maxAttempts": 3, "strategy": "fixed", "initialDelayMs": 1, "maxDelayMs": 1, "jitter": false}),
        ),
    )
}

#[tokio::test]
async fn us02_a_call_to_a_provider_is_one_trace_with_the_request_the_scheduler_the_attempt_and_the_provider()
 {
    let collector = Collector::start().await;
    let (_guard, sink) = collector.install(1.0);
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let (provider_url, seen) = provider().await;
    let workflow = p.publish("payment", &retrying()).await;
    let (trace, caller) = (TraceId::generate(), SpanId::generate());
    let run = start_traced(&p, &workflow, &traceparent(trace, caller, true)).await;
    let run_uuid = Uuid::parse_str(&run).expect("run id");

    // Three attempts: two fail (the second after a retry that the scheduler released), the third
    // calls the provider with the trace its lease carries, and succeeds.
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let mut leases = Vec::new();
    for _ in 0..2 {
        let lease = worker.one_task().await;
        worker.fail(&lease, true).await.expect("fail");
        leases.push(lease);
        p.elapse_retry_delay(&run, "a").await;
        p.tick().await;
    }
    let lease = worker.one_task().await;
    let from_lease = TraceContext::from_traceparent(Some(&lease.traceparent));
    // What an instrumented activity does: a span of its own under the attempt, sent to the provider.
    let call = traceparent(from_lease.trace_id(), SpanId::generate(), true);
    reqwest::Client::new()
        .post(&provider_url)
        .header("traceparent", &call)
        .send()
        .await
        .expect("provider call");
    worker
        .complete(&lease, &json!({"charged": true}))
        .await
        .expect("complete");
    leases.push(lease);
    assert_eq!(p.status(&run).await, "SUCCEEDED");

    // The journal becomes spans.
    let projector = projector(&p, Some(&collector));
    let round = projector.run_once().await;
    assert!(!round.failed && !round.passive, "{round:?}");
    sink.shutdown(std::time::Duration::from_secs(5)).await;

    let spans = collector.of_trace(&trace.to_string());
    let names: HashMap<&str, usize> = spans.iter().fold(HashMap::new(), |mut acc, s| {
        *acc.entry(s.name.as_str()).or_default() += 1;
        acc
    });
    assert_eq!(
        names.get("http.request"),
        Some(&1),
        "the request that started the run: {names:?}"
    );
    assert_eq!(names["step.attempt"], 3, "{names:?}");
    assert_eq!(names["workflow.run"], 1, "{names:?}");
    assert!(
        names.get("scheduler.action").copied().unwrap_or(0) >= 1,
        "the scheduler's work is in the trace: {names:?}"
    );

    // The attempt spans are the spans the leases carried, and hang from the run.
    let step: Uuid =
        sqlx::query_scalar("SELECT step_id FROM steps WHERE run_id = $1::uuid AND node = 'a'")
            .bind(&run)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("the step");
    let attempt_spans: Vec<_> = spans.iter().filter(|s| s.name == "step.attempt").collect();
    for (n, lease) in leases.iter().enumerate() {
        let leased = TraceContext::from_traceparent(Some(&lease.traceparent));
        let id = leased
            .parent_span_id()
            .expect("the lease carries a span")
            .to_string();
        assert_eq!(
            id,
            attempt_span_id(step, u32::try_from(n + 1).expect("small")).to_string()
        );
        let exported = attempt_spans
            .iter()
            .find(|s| s.span_id == id)
            .expect("the span the lease carried is exported");
        assert_eq!(
            exported.parent.as_deref(),
            Some(run_span_id(run_uuid).to_string().as_str())
        );
        assert_eq!(
            exported.attr("sagat.attempt"),
            Some((n + 1).to_string().as_str())
        );
    }
    assert_eq!(
        attempt_spans.iter().filter(|s| s.status == 2).count(),
        2,
        "two attempts failed"
    );

    // The provider saw the activity's call: the run's trace, under the attempt's span chain.
    let calls = seen.lock().expect("seen").clone();
    assert_eq!(calls, vec![call.clone()]);
    let provider_context = TraceContext::from_traceparent(Some(&calls[0]));
    assert_eq!(provider_context.trace_id(), trace);

    // The run's span hangs from the request's, which hangs from the caller's.
    let run_span = spans
        .iter()
        .find(|s| s.name == "workflow.run")
        .expect("run span");
    let (hangs_from,): (Option<String>,) =
        sqlx::query_as("SELECT trace_parent_span FROM runs WHERE id = $1::uuid")
            .bind(&run)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("run");
    assert_eq!(run_span.parent, hangs_from);
    let request = spans
        .iter()
        .find(|s| Some(&s.span_id) == hangs_from.as_ref())
        .expect("request span");
    assert_eq!(request.parent.as_deref(), Some(caller.to_string().as_str()));
    assert_eq!(run_span.attr("sagat.run.status"), Some("SUCCEEDED"));
}
