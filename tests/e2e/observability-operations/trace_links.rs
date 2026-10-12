//! FR-008-001, G79: what links the trace of a request to the run it started, the events that start
//! runs, the lease a worker receives and the delivery a subscriber gets.

use reqwest::Method;
use sagat_telemetry::{SpanId, TraceId, traceparent};
use serde_json::json;
use uuid::Uuid;

use crate::support::collector::Collector;
use crate::support::worker::{HUMAN, NAMESPACE, TENANT, platform, single_ir};

async fn start_with_header(
    p: &crate::support::worker::Platform,
    workflow: &str,
    header: Option<&str>,
) -> (u16, serde_json::Value) {
    let mut request = p
        .api
        .as_user(Method::POST, "/runs", HUMAN, TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", format!("key-{}", Uuid::now_v7()))
        .json(&json!({"workflowId": workflow, "input": {"n": 1}}));
    if let Some(value) = header {
        request = request.header("traceparent", value);
    }
    let response = request.send().await.expect("send");
    let status = response.status().as_u16();
    (status, response.json().await.expect("json"))
}

#[tokio::test]
async fn a_run_started_with_a_traceparent_belongs_to_that_trace_and_hangs_from_the_span_of_the_request()
 {
    let collector = Collector::start().await;
    let (_guard, sink) = collector.install(1.0);
    let p = platform().await;
    let workflow = p.publish("traced", &single_ir("traced", None)).await;

    let (trace, caller) = (TraceId::generate(), SpanId::generate());
    let (status, started) =
        start_with_header(&p, &workflow, Some(&traceparent(trace, caller, true))).await;
    assert_eq!(status, 202, "{started}");
    let run = started["runId"].as_str().expect("run id").to_owned();

    let row: (Option<String>, Option<String>) =
        sqlx::query_as("SELECT trace_id, trace_parent_span FROM runs WHERE id = $1::uuid")
            .bind(&run)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("the run");
    assert_eq!(row.0.as_deref(), Some(trace.to_string().as_str()));
    let hangs_from = row.1.expect("the run records the span it hangs from");
    assert_ne!(
        hangs_from,
        caller.to_string(),
        "the run hangs from the request's span, not the caller's"
    );

    // Wait for the server span to close before stopping its exporter: shutdown drains spans that
    // are already queued, but the client receiving the response can precede the server dropping
    // its last span reference.
    let request = collector
        .wait_for(1, |s| s.name == "http.request" && s.span_id == hangs_from)
        .await
        .remove(0);
    sink.shutdown(std::time::Duration::from_secs(5)).await;
    assert_eq!(request.trace_id, trace.to_string());
    assert_eq!(
        request.parent.as_deref(),
        Some(caller.to_string().as_str()),
        "the request is a child of the caller's span"
    );
    assert_eq!(request.kind, 2, "server");
    assert_eq!(request.attr("http.status_code"), Some("202"));
}

#[tokio::test]
async fn a_run_started_without_a_traceparent_still_belongs_to_a_trace_the_response_names() {
    let collector = Collector::start().await;
    let (_guard, sink) = collector.install(1.0);
    let p = platform().await;
    let workflow = p.publish("fresh", &single_ir("fresh", None)).await;
    let (status, started) = start_with_header(&p, &workflow, None).await;
    assert_eq!(status, 202, "{started}");
    let run = started["runId"].as_str().expect("run id");
    let (trace, parent): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT trace_id, trace_parent_span FROM runs WHERE id = $1::uuid")
            .bind(run)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("the run");
    let trace = trace.expect("a trace");
    assert_eq!(trace.len(), 32);
    let exported = collector
        .wait_for(1, |s| s.span_id == parent.clone().unwrap_or_default())
        .await;
    sink.shutdown(std::time::Duration::from_secs(5)).await;
    assert_eq!(exported[0].trace_id, trace);
    assert!(
        exported[0].parent.is_none(),
        "with no caller the request span is the root"
    );
}

#[tokio::test]
async fn the_lease_a_worker_receives_carries_the_span_of_its_attempt() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let workflow = p
        .publish(
            "retrying",
            &single_ir(
                "retrying",
                Some(json!({"maxAttempts": 3, "strategy": "fixed", "initialDelayMs": 1, "maxDelayMs": 1, "jitter": false})),
            ),
        )
        .await;
    let (trace, caller) = (TraceId::generate(), SpanId::generate());
    let (_, started) =
        start_with_header(&p, &workflow, Some(&traceparent(trace, caller, true))).await;
    let run = started["runId"].as_str().expect("run id").to_owned();
    p.tick().await;

    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let first = worker.one_task().await;
    let parsed = sagat_telemetry::TraceContext::from_traceparent(Some(&first.traceparent));
    assert!(parsed.is_continuation(), "{}", first.traceparent);
    assert_eq!(
        parsed.trace_id(),
        trace,
        "the worker works in the run's trace"
    );
    let step: Uuid =
        sqlx::query_scalar("SELECT step_id FROM steps WHERE run_id = $1::uuid AND node = 'a'")
            .bind(&run)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("the step");
    assert_eq!(
        parsed.parent_span_id(),
        Some(sagat_telemetry::attempt_span_id(step, 1))
    );

    // A retried attempt is another span of the same trace.
    worker.fail(&first, true).await.expect("fail");
    p.elapse_retry_delay(&run, "a").await;
    p.tick().await;
    let second = worker.one_task().await;
    let again = sagat_telemetry::TraceContext::from_traceparent(Some(&second.traceparent));
    assert_eq!(again.trace_id(), trace);
    assert_eq!(
        again.parent_span_id(),
        Some(sagat_telemetry::attempt_span_id(step, 2))
    );
    assert_ne!(again.parent_span_id(), parsed.parent_span_id());
}

#[tokio::test]
async fn an_event_published_with_a_traceparent_starts_a_run_in_the_same_trace() {
    use crate::helpers::{endpoint, engine, webhook, within};
    let collector = Collector::start().await;
    let (_guard, sink) = collector.install(1.0);
    let p = platform().await;
    let _engine = engine(&p, &[]).await;
    let endpoint_id = endpoint(&p, "billing").await;
    let mut ir = single_ir("on-invoice", None);
    ir["triggers"] =
        json!([{"kind": "webhook", "endpoint": "billing", "eventType": "invoice.paid"}]);
    let workflow = p.publish("on-invoice", &ir).await;

    let (trace, caller) = (TraceId::generate(), SpanId::generate());
    let header = traceparent(trace, caller, true);
    let (status, accepted) = webhook(
        &p,
        &endpoint_id,
        "evt-traced",
        "invoice.paid",
        &json!({"amount": 250}),
        &[("traceparent", header.as_str())],
    )
    .await;
    assert_eq!(
        (status, accepted["status"].as_str()),
        (202, Some("ACCEPTED")),
        "{accepted}"
    );

    let run = within("the run to be started", 20, || async {
        let (_, page) = p
            .rest(
                Method::GET,
                &format!("/runs?workflowId={workflow}&limit=10"),
                None,
            )
            .await;
        page["items"][0]["runId"].as_str().map(str::to_owned)
    })
    .await;
    let (run_trace, run_parent): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT trace_id, trace_parent_span FROM runs WHERE id = $1::uuid")
            .bind(&run)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("the run");
    assert_eq!(
        run_trace.as_deref(),
        Some(trace.to_string().as_str()),
        "the run continues the publisher's trace"
    );
    let hangs_from = run_parent.expect("and hangs from the request that admitted the event");

    let request = collector
        .wait_for(1, |s| s.name == "http.request" && s.span_id == hangs_from)
        .await
        .remove(0);
    sink.shutdown(std::time::Duration::from_secs(5)).await;
    assert_eq!(request.trace_id, trace.to_string());
    assert_eq!(request.parent.as_deref(), Some(caller.to_string().as_str()));
}

#[tokio::test]
async fn a_run_started_by_an_event_with_no_trace_gets_a_trace_of_its_own() {
    use crate::helpers::{endpoint, engine, webhook, within};
    let p = platform().await;
    let _engine = engine(&p, &[]).await;
    let endpoint_id = endpoint(&p, "billing").await;
    let mut ir = single_ir("on-invoice", None);
    ir["triggers"] =
        json!([{"kind": "webhook", "endpoint": "billing", "eventType": "invoice.paid"}]);
    let workflow = p.publish("on-invoice", &ir).await;
    let (status, _) = webhook(
        &p,
        &endpoint_id,
        "evt-plain",
        "invoice.paid",
        &json!({}),
        &[],
    )
    .await;
    assert_eq!(status, 202);
    let run = within("the run to be started", 20, || async {
        let (_, page) = p
            .rest(
                Method::GET,
                &format!("/runs?workflowId={workflow}&limit=10"),
                None,
            )
            .await;
        page["items"][0]["runId"].as_str().map(str::to_owned)
    })
    .await;
    let (trace, parent): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT trace_id, trace_parent_span FROM runs WHERE id = $1::uuid")
            .bind(&run)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("the run");
    assert_eq!(
        trace.expect("a trace").len(),
        32,
        "a run no request started still has a trace"
    );
    assert!(parent.is_none(), "and hangs from nothing");
}

#[tokio::test]
async fn a_subscriber_is_delivered_the_finished_run_with_a_traceparent_that_continues_its_trace() {
    use crate::helpers::Standing;
    use crate::support::receiver::{LoopbackSender, Receiver};
    use sagat_application::entropy::SystemEntropy;
    use sagat_scheduler::{DeliverySettings, DeliveryWorker};

    let collector = Collector::start().await;
    let (_guard, sink) = collector.install(1.0);
    let p = platform().await;
    let receiver = Receiver::start().await;
    receiver.answer(200);
    let ns = p.namespace_id().await;
    for (method, path, body) in [
        (
            Method::PUT,
            format!("/namespaces/{ns}/secrets/provider-signing"),
            json!({"value": crate::helpers::SECRET}),
        ),
        (
            Method::PUT,
            format!("/namespaces/{ns}/egress-policy"),
            json!({"rules": ["hooks.example.com"]}),
        ),
    ] {
        let (status, answer) = p.rest(method, &path, Some(&body)).await;
        assert_eq!(status, 200, "{answer}");
    }
    let (status, subscription) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/webhook-subscriptions"),
            Some(
                &json!({"name": "finance", "url": "https://hooks.example.com/in",
                         "secretName": "provider-signing", "events": ["run.finished"]}),
            ),
        )
        .await;
    assert_eq!(status, 201, "{subscription}");

    let workflow = p
        .publish("close-books", &single_ir("close-books", None))
        .await;
    let (trace, caller) = (TraceId::generate(), SpanId::generate());
    let (_, started) =
        start_with_header(&p, &workflow, Some(&traceparent(trace, caller, true))).await;
    let run = started["runId"].as_str().expect("run id").to_owned();
    p.bind_everything("wendy").await;
    let mut worker = p.worker("wendy").await;
    worker.register(&["act.a"]).await;
    let task = worker.one_task().await;
    worker
        .complete(&task, &json!({"closed": true}))
        .await
        .expect("complete");
    assert_eq!(p.status(&run).await, "SUCCEEDED");

    let delivery = DeliveryWorker::new(
        p.api.executions.clone(),
        Standing {
            inner: LoopbackSender::new(),
            receiver: receiver.url.clone(),
        },
        SystemEntropy,
        p.api.keys.cipher(),
        DeliverySettings::default(),
    );
    assert_eq!(delivery.tick().await.delivered, 1);

    let seen = receiver.seen();
    assert_eq!(seen.len(), 1);
    let header = seen[0]
        .headers
        .get("traceparent")
        .expect("the delivery carries a traceparent");
    let continued = sagat_telemetry::TraceContext::from_traceparent(Some(header));
    assert!(continued.is_continuation(), "{header}");
    assert_eq!(
        continued.trace_id(),
        trace,
        "the receiver continues the run's trace"
    );
    let body: serde_json::Value = serde_json::from_slice(&seen[0].body).expect("json body");
    assert_eq!(body["data"]["traceId"], trace.to_string());

    // The span the receiver's work hangs from is the delivery attempt, a child of the run's span.
    let delivered = collector
        .wait_for(1, |s| s.name == "webhook.delivery")
        .await
        .remove(0);
    sink.shutdown(std::time::Duration::from_secs(5)).await;
    assert_eq!(
        Some(delivered.span_id.clone()),
        continued.parent_span_id().map(|s| s.to_string())
    );
    let run_uuid = Uuid::parse_str(&run).expect("run uuid");
    assert_eq!(
        delivered.parent,
        Some(sagat_telemetry::run_span_id(run_uuid).to_string())
    );
    assert_eq!(delivered.kind, 3, "client");
}
