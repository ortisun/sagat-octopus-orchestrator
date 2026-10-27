//! Native API contracts before the shared real-platform conformance gate.
#![allow(clippy::expect_used)]
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use sagat_octopus_orchestrator_sdk::{
    CallOptions, Client, ClientOptions, TokenProviderError, TokenSource,
};
use serde_json::json;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

const RUN: &str = "019b5d6e-0000-7000-8000-000000000001";
const WORKFLOW: &str = "019b5d6e-0000-7000-8000-000000000002";
type CapturedRequest = (HeaderMap, Vec<u8>, axum::http::Uri);

#[derive(Clone, Default)]
struct Seen {
    calls: Arc<Mutex<Vec<CapturedRequest>>>,
    mode: &'static str,
}
async fn answer(
    State(state): State<Seen>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let count = {
        let mut calls = state.calls.lock().expect("capture");
        calls.push((headers, body.to_vec(), uri.clone()));
        calls.len()
    };
    match state.mode {
        "bad-page" => (StatusCode::OK,[("x-trace-id","0123456789abcdef0123456789abcdef")],axum::Json(json!({"items":false}))).into_response(),
        "repeat-page" => (StatusCode::OK,[("x-trace-id","0123456789abcdef0123456789abcdef")],axum::Json(json!({"items":[basic_run("RUNNING")],"nextCursor":"same"}))).into_response(),
        "bad-query"|"bad-update" => (StatusCode::OK,[("x-trace-id","0123456789abcdef0123456789abcdef")],axum::Json(json!({"runId":RUN,"runVersion":1,"stateRevision":1}))).into_response(),
        "bad-approvals" => (StatusCode::OK,[("x-trace-id","0123456789abcdef0123456789abcdef")],axum::Json(json!({"items":[{"approvalId":RUN,"runId":RUN,"nodeId":"review","instancePath":[],"status":"PENDING","deadline":"2026-10-27T12:00:00Z"}],"nextCursor":null}))).into_response(),
        "approvals"|"approval-pages" => (StatusCode::OK,axum::Json(json!({"items":[{"approvalId":RUN,"runId":RUN,"nodeId":"review","instancePath":[{"scopeNodeId":"lines","iterationIndex":3,"futureField":true}],"status":"PENDING","deadline":"2026-10-27T12:00:00Z","settledAt":null,"newField":true}],"nextCursor":if state.mode=="approval-pages" && count==1{Some("next approval")}else{None}}))).into_response(),
        "decision-retry" if count==1 => (StatusCode::SERVICE_UNAVAILABLE,axum::Json(json!({"code":"UNAVAILABLE","retryable":true}))).into_response(),
        "decision-retry" => (StatusCode::OK,axum::Json(json!({"receiptId":WORKFLOW,"approvalId":RUN,"decisionKey":"approve-1","decision":"APPROVE","reasonCode":"review:ok","actor":"user:reviewer","decidedAt":"2026-10-27T17:30:00+05:30","outcomeSequence":12,"replayed":true}))).into_response(),
        "head-changed" => (StatusCode::CONFLICT,axum::Json(json!({"code":"CONFLICT","retryable":true,"details":{"reason":"CHAIN_HEAD_CHANGED"}}))).into_response(),
        "bad-chain-page" => (StatusCode::OK,[("x-trace-id","0123456789abcdef0123456789abcdef")],axum::Json(json!({"items":[{"index":10000,"runId":RUN,"status":"SUCCEEDED","completionKind":"FINAL","predecessorRunId":null,"successorRunId":null,"createdAt":"2026-10-27T12:00:00Z","finishedAt":null}],"nextCursor":null}))).into_response(),
        "chain-completion" => (StatusCode::OK,axum::Json(json!({"chainId":RUN,"currentRunId":RUN,"revision":count,"cancelRequested":false,"finalOutcome":if count==1 {None}else{Some("SUCCEEDED")},"segments":[{"index":0,"runId":RUN,"predecessorRunId":null,"successorRunId":null}]}))).into_response(),
        "chain-command" if count==1 => (StatusCode::SERVICE_UNAVAILABLE,axum::Json(json!({"code":"UNAVAILABLE","retryable":true}))).into_response(),
        "chain-command" if uri.path().ends_with("/cancel") => StatusCode::ACCEPTED.into_response(),
        "chain-command" => (StatusCode::ACCEPTED,axum::Json(json!({"deliveredTo":["wait"]}))).into_response(),
        "chain-pages" => (StatusCode::OK,axum::Json(json!({"items":[{"index":count-1,"runId":if count==1{RUN}else{WORKFLOW},"status":"SUCCEEDED","completionKind":if count==1{"CONTINUED"}else{"FINAL"},"predecessorRunId":if count==1{None}else{Some(RUN)},"successorRunId":if count==1{Some(WORKFLOW)}else{None},"createdAt":"2026-10-27T12:00:00Z","finishedAt":"2026-10-27T12:01:00Z"}],"nextCursor":if count==1{Some("next segment")}else{None}}))).into_response(),
        "query"|"chain-query" => (StatusCode::OK,axum::Json(json!({"runId":RUN,"runVersion":9007199254740993_u64,"stateRevision":42,"result":{"amount":"12.3400","at":"2026-10-27T12:00:00Z"}}))).into_response(),
        "update-retry" if count==1 => (StatusCode::SERVICE_UNAVAILABLE,axum::Json(json!({"code":"UNAVAILABLE","retryable":true}))).into_response(),
        "update-retry"|"chain-update" => (StatusCode::OK,axum::Json(json!({"runId":RUN,"operation":"adjust","updateKey":"update-1","status":"APPLIED","result":null,"beforeStateRevision":42,"afterStateRevision":43,"runVersion":50,"replayed":true}))).into_response(),
        "event-retry" if count==1 => (StatusCode::SERVICE_UNAVAILABLE,axum::Json(json!({"code":"UNAVAILABLE","retryable":true}))).into_response(),
        "event-retry" => {let request:serde_json::Value=serde_json::from_slice(&body).expect("event body");(StatusCode::ACCEPTED,axum::Json(json!({"eventId":request["eventId"],"source":"api","status":"DUPLICATE"}))).into_response()},
        "pages" => (StatusCode::OK,axum::Json(json!({"items":[basic_run("RUNNING")],"nextCursor":if count==1 {Some("opaque +/?&=é")}else{None}}))).into_response(),
        "journal" => (StatusCode::OK,axum::Json(json!({"items":[{"seq":1,"eventType":"RunStarted","occurredAt":"2026-10-27T17:30:00+05:30","actor":"system:test","futureField":true}],"nextCursor":null,"newField":"retained"}))).into_response(),
        "completion" => (StatusCode::OK,axum::Json(basic_run(if count==1 {"RUNNING"}else{"SUCCEEDED"}))).into_response(),
        "retry" if count == 1 => (StatusCode::SERVICE_UNAVAILABLE, axum::Json(json!({"code":"UNAVAILABLE", "retryable":true}))).into_response(),
        "child" => (StatusCode::OK,axum::Json(json!({"runId":RUN,"workflowId":WORKFLOW,"versionId":WORKFLOW,"version":1,"createdAt":"2026-10-27T12:00:00Z","status":"RUNNING","depth":1,"parent":{"runId":WORKFLOW,"node":"child","cancelPolicy":"DETACH"}}))).into_response(),
        "unsupported" => (StatusCode::NOT_IMPLEMENTED,axum::Json(json!({"code":"INTERNAL","retryable":true}))).into_response(),
        "forged" => (StatusCode::UNAUTHORIZED, axum::Json(json!({"code":"UNAUTHENTICATED", "retryable":true,"details":{"reason":"TOKEN_PROVIDER_FAILED"}}))).into_response(),
        "redirect" => (StatusCode::FOUND, [("location", "/must-not-be-followed")]).into_response(),
        "large" => (StatusCode::OK, "x".repeat(1025)).into_response(),
        "incomplete" => (StatusCode::ACCEPTED, [("x-trace-id", "0123456789abcdef0123456789abcdef")], axum::Json(json!({"runId":RUN,"status":"RUNNING"}))).into_response(),
        "signal-empty" => (StatusCode::ACCEPTED, axum::Json(json!({}))).into_response(),
        "signal" => (StatusCode::ACCEPTED, axum::Json(json!({"deliveredTo":["wait"]}))).into_response(),
        "operation" => (StatusCode::ACCEPTED, axum::Json(json!({"operationId":RUN,"accepted":true,"runStatus":"RUNNING","runVersion":12}))).into_response(),
        "malformed" => (StatusCode::OK, [("x-trace-id", "0123456789abcdef0123456789abcdef")], "{broken").into_response(),
        _ => (StatusCode::ACCEPTED, axum::Json(json!({"runId":RUN,"workflowId":WORKFLOW,"versionId":WORKFLOW,"version":1,"createdAt":"2026-10-27T12:00:00Z", "status":if state.mode == "paused" {"PAUSED"} else {"RUNNING"}, "futureField":{"number":9007199254740993_u64}}))).into_response(),
    }
}
fn basic_run(status: &str) -> serde_json::Value {
    json!({"runId":RUN,"workflowId":WORKFLOW,"versionId":WORKFLOW,"version":1,"createdAt":"2026-10-27T12:00:00Z","status":status})
}
struct Server {
    base: String,
    state: Seen,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(mode: &'static str) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("owned listener");
    let base = format!("http://{}/v1", listener.local_addr().expect("address"));
    let state = Seen {
        mode,
        ..Seen::default()
    };
    let app = Router::new()
        .fallback(any(answer))
        .with_state(state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("owned server");
    });
    Server { base, state, task }
}
fn options(base: &str, token: TokenSource) -> ClientOptions {
    let mut options = ClientOptions::new(base, "payments", token);
    options.retry.initial_delay = Duration::from_millis(1);
    options.retry.max_delay = Duration::from_millis(2);
    options.retry.max_attempts = 3;
    options
}
#[tokio::test]
async fn retry_preserves_exact_body_and_key_but_fetches_a_fresh_token() {
    let server = server("retry").await;
    let calls = Arc::new(AtomicUsize::new(0));
    let tokens = calls.clone();
    let token = TokenSource::dynamic(move || {
        let n = tokens.fetch_add(1, Ordering::SeqCst);
        async move { Ok(format!("token-{n}")) }
    });
    let client = Client::new(options(&server.base, token)).expect("client");
    let run = client
        .start(
            WORKFLOW,
            &json!({"exact":9007199254740993_u64}),
            CallOptions::default(),
        )
        .await
        .expect("start");
    assert_eq!(run.id.to_string(), RUN);
    assert_eq!(
        run.raw["futureField"]["number"].as_u64(),
        Some(9007199254740993)
    );
    let seen = server.state.calls.lock().expect("seen");
    assert_eq!(seen.len(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(seen[0].0["idempotency-key"], seen[1].0["idempotency-key"]);
    assert_eq!(seen[0].1, seen[1].1);
    assert_eq!(seen[0].0["authorization"], "Bearer token-0");
    assert_eq!(seen[1].0["authorization"], "Bearer token-1");
    assert_eq!(seen[0].0["x-octopus-namespace"], "payments");
}
#[tokio::test]
async fn redirects_and_server_forged_token_failure_are_never_retried() {
    for (mode, code) in [("redirect", "PROTOCOL"), ("forged", "UNAUTHENTICATED")] {
        let server = server(mode).await;
        let client = Client::new(options(&server.base, TokenSource::bearer("fixture-token")))
            .expect("client");
        let error = client
            .start(WORKFLOW, &json!({}), CallOptions::default())
            .await
            .expect_err("refused");
        assert_eq!(error.code, code);
        assert_eq!(server.state.calls.lock().expect("seen").len(), 1);
        assert!(!format!("{client:?} {error:?}").contains("fixture-token"));
    }
}
#[tokio::test]
async fn a_local_token_provider_failure_retries_before_network_io() {
    let server = server("ok").await;
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let token = TokenSource::dynamic(move || {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            if n == 0 {
                Err(TokenProviderError)
            } else {
                Ok("fresh-token".to_owned())
            }
        }
    });
    let client = Client::new(options(&server.base, token)).expect("client");
    client
        .start(WORKFLOW, &json!({}), CallOptions::default())
        .await
        .expect("provider recovers");
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(server.state.calls.lock().expect("seen").len(), 1);
}
#[tokio::test]
async fn cancellation_invalid_trace_and_response_bounds_fail_closed() {
    let server = server("large").await;
    let mut config = options(&server.base, TokenSource::bearer("fixture-token"));
    config.max_response_bytes = 1024;
    let client = Client::new(config).expect("client");
    let cancelled = CallOptions::default();
    cancelled.cancel.cancel();
    assert_eq!(
        client
            .start(WORKFLOW, &json!({}), cancelled)
            .await
            .expect_err("cancelled")
            .code,
        "CANCELLED"
    );
    let invalid = CallOptions {
        traceparent: Some("private-token".to_owned()),
        ..CallOptions::default()
    };
    assert_eq!(
        client
            .start(WORKFLOW, &json!({}), invalid)
            .await
            .expect_err("invalid trace")
            .code,
        "BAD_REQUEST"
    );
    assert!(server.state.calls.lock().expect("seen").is_empty());
    assert_eq!(
        client
            .start(WORKFLOW, &json!({}), CallOptions::default())
            .await
            .expect_err("bounded body")
            .code,
        "PROTOCOL"
    );
    assert_eq!(server.state.calls.lock().expect("seen").len(), 1);
}
#[test]
fn embedded_error_catalog_is_the_canonical_document() {
    let canonical = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../error-catalog.json");
    assert_eq!(
        std::fs::read(canonical).expect("catalog"),
        include_bytes!("../src/error-catalog.json")
    );
}

#[tokio::test]
async fn review_required_run_fields_are_validated_and_keep_response_metadata() {
    let server = server("incomplete").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let error = client
        .start(WORKFLOW, &json!({}), CallOptions::default())
        .await
        .expect_err("an incomplete Run is a protocol error");
    assert_eq!(error.code, "PROTOCOL");
    assert_eq!(error.status, Some(202));
    assert_eq!(
        error.trace_id.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
}
#[tokio::test]
async fn review_unknown_run_operation_is_rejected_before_network_io() {
    let server = server("ok").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    assert_eq!(
        client
            .operate(
                RUN,
                "FORCE_COMPLETE",
                "a valid justification",
                CallOptions::default()
            )
            .await
            .expect_err("action is not in canonical contract")
            .code,
        "VALIDATION_FAILED"
    );
    assert!(server.state.calls.lock().expect("seen").is_empty());
}
#[tokio::test]
async fn review_empty_signal_success_is_a_protocol_error() {
    let server = server("signal-empty").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    assert_eq!(
        client
            .signal(RUN, "named", &json!({}), CallOptions::default())
            .await
            .expect_err("missing deliveredTo")
            .code,
        "PROTOCOL"
    );
}
#[tokio::test]
async fn review_malformed_json_keeps_the_received_status_and_trace_id() {
    let server = server("malformed").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let error = client
        .get(RUN, CallOptions::default())
        .await
        .expect_err("JSON malformed");
    assert_eq!(error.code, "PROTOCOL");
    assert_eq!(error.status, Some(200));
    assert_eq!(
        error.trace_id.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
}
#[tokio::test]
async fn review_paused_is_a_known_nonterminal_run_status() {
    let server = server("paused").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let run = client
        .start(WORKFLOW, &json!({}), CallOptions::default())
        .await
        .expect("run");
    assert!(!matches!(
        run.status,
        sagat_octopus_orchestrator_sdk::RunStatus::Unknown(_)
    ));
    assert!(!run.status.is_terminal());
}
#[tokio::test]
async fn review_signal_names_are_encoded_as_one_url_segment() {
    let server = server("signal").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    client
        .signal(RUN, "a/b", &json!({}), CallOptions::default())
        .await
        .expect("signal names use path encoding");
    assert!(
        server.state.calls.lock().expect("seen")[0]
            .2
            .path()
            .ends_with("/signals/a%2Fb")
    );
}

#[tokio::test]
async fn start_options_freeze_the_version_correlation_and_utc_deadline_across_retries() {
    use sagat_octopus_orchestrator_sdk::{RunDeadline, StartOptions};
    let server = server("retry").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    client
        .start(
            WORKFLOW,
            &json!({}),
            StartOptions {
                version_id: Some(WORKFLOW.to_owned()),
                correlation_id: Some("correlation-1".to_owned()),
                deadline: Some(RunDeadline::After(Duration::from_secs(60))),
                ..StartOptions::default()
            },
        )
        .await
        .expect("start");
    let seen = server.state.calls.lock().expect("seen");
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].1, seen[1].1);
    let body: serde_json::Value = serde_json::from_slice(&seen[0].1).expect("body");
    assert_eq!(body["versionId"], WORKFLOW);
    assert_eq!(body["correlationId"], "correlation-1");
    let deadline = body["deadline"].as_str().expect("deadline");
    assert!(deadline.ends_with('Z'));
    assert_eq!(
        time::OffsetDateTime::parse(deadline, &time::format_description::well_known::Rfc3339)
            .expect("instant")
            .offset(),
        time::UtcOffset::UTC
    );
}
#[tokio::test]
async fn step_operation_preserves_exact_scope_outcome_and_expected_revision() {
    use sagat_octopus_orchestrator_sdk::{OperationOptions, ScopeSegment, StepOutcome};
    let server = server("operation").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let result = client
        .operate(
            RUN,
            "RESOLVE_STEP",
            "the provider confirmed the effect",
            OperationOptions {
                step_id: Some("send".to_owned()),
                outcome: Some(StepOutcome::Succeeded),
                expected_run_version: Some(11),
                instance_path: Some(vec![ScopeSegment {
                    scope_node_id: "lines".to_owned(),
                    iteration_index: 2,
                }]),
                ..OperationOptions::default()
            },
        )
        .await
        .expect("operation");
    assert!(result.accepted);
    assert_eq!(result.run_version, Some(12));
    let seen = server.state.calls.lock().expect("seen");
    let body: serde_json::Value = serde_json::from_slice(&seen[0].1).expect("body");
    assert_eq!(body["stepId"], "send");
    assert_eq!(body["outcome"], "SUCCEEDED");
    assert_eq!(body["expectedRunVersion"], 11);
    assert_eq!(
        body["instancePath"],
        json!([{"scopeNodeId":"lines","iterationIndex":2}])
    );
}
#[tokio::test]
async fn a_server_reason_is_public_but_never_grants_the_local_provider_retry_exception() {
    let server = server("forged").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let error = client
        .start(WORKFLOW, &json!({}), CallOptions::default())
        .await
        .expect_err("unauthenticated");
    assert_eq!(error.reason(), Some("TOKEN_PROVIDER_FAILED"));
    assert_eq!(server.state.calls.lock().expect("seen").len(), 1);
}

#[tokio::test]
async fn failed_resolution_requires_a_failure_code_before_network_io() {
    use sagat_octopus_orchestrator_sdk::{OperationOptions, StepOutcome};
    let server = server("operation").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let error = client
        .operate(
            RUN,
            "RESOLVE_STEP",
            "confirmed failure",
            OperationOptions {
                step_id: Some("send".into()),
                outcome: Some(StepOutcome::Failed),
                ..OperationOptions::default()
            },
        )
        .await
        .expect_err("missing failure code");
    assert_eq!(error.code, "VALIDATION_FAILED");
    assert!(server.state.calls.lock().expect("calls").is_empty());
}
#[tokio::test]
async fn operation_receipt_must_match_the_requested_action() {
    let server = server("operation").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let error = client
        .operate(
            RUN,
            "RERUN_AS_NEW",
            "run again please",
            CallOptions::default(),
        )
        .await
        .expect_err("missing new run");
    assert_eq!(error.code, "PROTOCOL");
    assert_eq!(error.status, Some(202));
}
#[tokio::test]
async fn token_provider_timeout_has_provider_reason_without_a_network_request() {
    let server = server("operation").await;
    let mut config = options(
        &server.base,
        TokenSource::dynamic(std::future::pending::<Result<String, TokenProviderError>>),
    );
    config.timeout = Duration::from_millis(5);
    config.retry.max_attempts = 1;
    let client = Client::new(config).expect("client");
    let error = client
        .get(RUN, CallOptions::default())
        .await
        .expect_err("provider timeout");
    assert_eq!(error.code, "UNAUTHENTICATED");
    assert_eq!(error.reason(), Some("TOKEN_PROVIDER_FAILED"));
    assert!(server.state.calls.lock().expect("calls").is_empty());
}
#[tokio::test]
async fn object_only_shapes_are_refused_before_network_io() {
    let server = server("signal").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    assert_eq!(
        client
            .start(WORKFLOW, &json!(7), CallOptions::default())
            .await
            .expect_err("object input")
            .code,
        "VALIDATION_FAILED"
    );
    assert_eq!(
        client
            .signal(RUN, "ready", &json!([1]), CallOptions::default())
            .await
            .expect_err("object payload")
            .code,
        "VALIDATION_FAILED"
    );
    assert!(server.state.calls.lock().expect("calls").is_empty());
}
#[tokio::test]
async fn unsupported_routes_do_not_advertise_retryable_not_found() {
    let server = server("unsupported").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let error = client
        .get(RUN, CallOptions::default())
        .await
        .expect_err("unsupported");
    assert_eq!(error.code, "NOT_FOUND");
    assert!(!error.retryable);
}
#[tokio::test]
async fn cancel_sends_the_observed_version_and_the_user_agent_suffix() {
    use sagat_octopus_orchestrator_sdk::OperationOptions;
    let server = server("operation").await;
    let mut config = options(&server.base, TokenSource::bearer("fixture-token"));
    config.user_agent_suffix = Some("example-app/1".into());
    let client = Client::new(config).expect("client");
    client
        .cancel(
            RUN,
            "operator requested",
            OperationOptions {
                expected_run_version: Some(42),
                ..OperationOptions::default()
            },
        )
        .await
        .expect("cancel");
    let seen = server.state.calls.lock().expect("calls");
    assert_eq!(
        seen[0].0["user-agent"],
        "sagat-sdk-rust/0.3.0 example-app/1"
    );
    let body: serde_json::Value = serde_json::from_slice(&seen[0].1).expect("json");
    assert_eq!(body["expectedRunVersion"], 42);
}

#[tokio::test]
async fn signal_parses_the_canonical_array_of_woken_nodes() {
    let server = server("signal").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let result = client
        .signal(RUN, "ready", &json!({}), CallOptions::default())
        .await
        .expect("canonical array");
    assert_eq!(result.raw["deliveredTo"], json!(["wait"]));
}
#[tokio::test]
async fn operator_resolution_preserves_valid_lowercase_and_punctuated_failure_codes() {
    use sagat_octopus_orchestrator_sdk::{OperationOptions, StepOutcome};
    let server = server("operation").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    client
        .operate(
            RUN,
            "RESOLVE_STEP",
            "provider confirmed",
            OperationOptions {
                step_id: Some("send".into()),
                outcome: Some(StepOutcome::Failed),
                error_code: Some("http:503".into()),
                ..OperationOptions::default()
            },
        )
        .await
        .expect("canonical failure code");
}
#[tokio::test]
async fn malformed_static_bearer_is_a_local_nonretryable_validation_failure() {
    let server = server("operation").await;
    let client = Client::new(options(&server.base, TokenSource::bearer(""))).expect("client");
    let error = client
        .get(RUN, CallOptions::default())
        .await
        .expect_err("invalid static bearer");
    assert_eq!(error.code, "VALIDATION_FAILED");
    assert_eq!(error.reason(), None);
    assert!(server.state.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn child_run_lineage_has_typed_fields_and_retains_raw_data() {
    let server = server("child").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let run = client
        .get(RUN, CallOptions::default())
        .await
        .expect("child run");
    assert_eq!(run.depth, 1);
    let parent = run.parent.expect("parent");
    assert_eq!(parent.run_id.to_string(), WORKFLOW);
    assert_eq!(parent.node, "child");
    assert_eq!(parent.cancel_policy, "DETACH");
    assert_eq!(run.raw["depth"], 1);
}

#[tokio::test]
async fn run_pagination_follows_opaque_cursors_without_growing_a_result_collection() {
    use futures_util::StreamExt as _;
    use sagat_octopus_orchestrator_sdk::{ListOptions, RunStatus};
    let server = server("pages").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let stream = client.list_all(
        ListOptions {
            workflow_id: Some(WORKFLOW.into()),
            status: Some(RunStatus::Running),
            limit: Some(1),
            ..ListOptions::default()
        },
        CallOptions::default(),
    );
    futures_util::pin_mut!(stream);
    let first = stream.next().await.expect("item").expect("run");
    assert_eq!(first.id.to_string(), RUN);
    assert_eq!(
        server.state.calls.lock().expect("calls").len(),
        1,
        "the second page is lazy"
    );
    stream.next().await.expect("second item").expect("run");
    assert!(stream.next().await.is_none());
    let requests = server.state.calls.lock().expect("calls");
    let parsed = url::Url::parse(&format!("http://localhost{}", requests[1].2)).expect("url");
    let query: std::collections::BTreeMap<_, _> = parsed.query_pairs().into_owned().collect();
    assert_eq!(query["cursor"], "opaque +/?&=é");
    assert_eq!(query["workflowId"], WORKFLOW);
    assert_eq!(query["status"], "RUNNING");
    assert_eq!(query["limit"], "1");
}
#[tokio::test]
async fn journal_events_have_typed_fields_and_keep_additive_raw_json() {
    use sagat_octopus_orchestrator_sdk::PageOptions;
    let server = server("journal").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let page = client
        .events(
            RUN,
            PageOptions {
                limit: Some(1),
                cursor: Some("next/+".into()),
            },
            CallOptions::default(),
        )
        .await
        .expect("events");
    assert_eq!(page.items[0].seq, 1);
    assert_eq!(page.items[0].event_type, "RunStarted");
    assert_eq!(page.items[0].occurred_at.offset(), time::UtcOffset::UTC);
    assert_eq!(page.items[0].raw["futureField"], true);
    assert_eq!(page.raw["newField"], "retained");
}
#[tokio::test]
async fn waiting_uses_known_terminal_states_and_does_not_cancel_the_run_on_timeout() {
    use sagat_octopus_orchestrator_sdk::{RunStatus, WaitOptions};
    let server = server("completion").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let done = client
        .wait_for_completion(
            RUN,
            WaitOptions {
                poll_interval: Duration::from_millis(1),
                timeout: Duration::from_secs(1),
                ..WaitOptions::default()
            },
        )
        .await
        .expect("terminal");
    assert_eq!(done.status, RunStatus::Succeeded);
    let seen = server.state.calls.lock().expect("calls");
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|(_, body, _)| body.is_empty()));
}

#[tokio::test]
async fn malformed_and_non_progressing_pages_keep_response_metadata() {
    use futures_util::StreamExt as _;
    use sagat_octopus_orchestrator_sdk::ListOptions;
    let malformed = server("bad-page").await;
    let client = Client::new(options(
        &malformed.base,
        TokenSource::bearer("fixture-token"),
    ))
    .expect("client");
    let error = client
        .list(ListOptions::default(), CallOptions::default())
        .await
        .expect_err("invalid items");
    assert_eq!(error.code, "PROTOCOL");
    assert_eq!(error.status, Some(200));
    let repeated = server("repeat-page").await;
    let client = Client::new(options(
        &repeated.base,
        TokenSource::bearer("fixture-token"),
    ))
    .expect("client");
    let stream = client.list_all(
        ListOptions {
            cursor: Some("same".into()),
            ..ListOptions::default()
        },
        CallOptions::default(),
    );
    futures_util::pin_mut!(stream);
    let error = stream
        .next()
        .await
        .expect("result")
        .expect_err("same cursor cannot progress");
    assert_eq!(error.code, "PROTOCOL");
    assert_eq!(error.status, Some(200));
    assert_eq!(
        error.trace_id.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert!(stream.next().await.is_none());
}
#[tokio::test]
async fn wait_timeout_and_cancellation_stop_polling_without_issuing_an_operation() {
    use sagat_octopus_orchestrator_sdk::WaitOptions;
    let server = server("child").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let error = client
        .wait_for_completion(
            RUN,
            WaitOptions {
                timeout: Duration::from_millis(20),
                poll_interval: Duration::from_secs(30),
                ..WaitOptions::default()
            },
        )
        .await
        .expect_err("timeout");
    assert_eq!(error.code, "TRANSPORT");
    assert_eq!(error.reason(), Some("WAIT_TIMEOUT"));
    let before = server.state.calls.lock().expect("requests").len();
    let call = CallOptions::default();
    call.cancel.cancel();
    let error = client
        .wait_for_completion(
            RUN,
            WaitOptions {
                call,
                ..WaitOptions::default()
            },
        )
        .await
        .expect_err("cancelled");
    assert_eq!(error.code, "CANCELLED");
    let requests = server.state.calls.lock().expect("requests");
    assert_eq!(requests.len(), before);
    assert!(
        requests
            .iter()
            .all(|(_, body, uri)| body.is_empty() && uri.path().ends_with(RUN))
    );
}

#[tokio::test]
async fn declared_queries_keep_exact_revisions_and_disclosed_values() {
    let server = server("query").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let result = client
        .query(
            RUN,
            "summary.total",
            &json!({"offset":9007199254740993_u64}),
            CallOptions::default(),
        )
        .await
        .expect("query");
    assert_eq!(result.run_version, 9007199254740993);
    assert_eq!(result.state_revision, 42);
    assert_eq!(result.result["amount"], "12.3400");
    assert_eq!(result.result["at"], "2026-10-27T12:00:00Z");
    let requests = server.state.calls.lock().expect("requests");
    assert!(!requests[0].0.contains_key("idempotency-key"));
}
#[tokio::test]
async fn durable_updates_retry_the_body_key_and_exact_arguments_without_an_idempotency_header() {
    use sagat_octopus_orchestrator_sdk::UpdateRequest;
    let server = server("update-retry").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let receipt = client
        .update(
            RUN,
            "adjust",
            UpdateRequest {
                update_key: "update-1".into(),
                args: json!({"amount":"12.3400"}),
                expected_state_revision: Some(42),
            },
            CallOptions {
                idempotency_key: Some("unrelated-header-key".into()),
                ..CallOptions::default()
            },
        )
        .await
        .expect("update replay");
    assert_eq!(receipt.after_state_revision, 43);
    assert!(receipt.replayed);
    let requests = server.state.calls.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].1, requests[1].1);
    assert!(
        requests
            .iter()
            .all(|(headers, _, _)| !headers.contains_key("idempotency-key"))
    );
    let body: serde_json::Value = serde_json::from_slice(&requests[0].1).expect("body");
    assert_eq!(body["updateKey"], "update-1");
    assert_eq!(body["expectedStateRevision"], 42);
}
#[tokio::test]
async fn event_publication_freezes_its_generated_event_id_and_utc_instant_across_retries() {
    use sagat_octopus_orchestrator_sdk::{EventStatus, PublishEvent};
    let server = server("event-retry").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let mut event = PublishEvent::new("order.created", json!({"value":9007199254740993_u64}));
    event.occurred_at = Some(
        time::OffsetDateTime::parse(
            "2026-10-27T17:30:00+05:30",
            &time::format_description::well_known::Rfc3339,
        )
        .expect("instant"),
    );
    let receipt = client
        .publish_event(event, CallOptions::default())
        .await
        .expect("deduplicated retry");
    assert_eq!(receipt.status, EventStatus::Duplicate);
    let requests = server.state.calls.lock().expect("requests");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].1, requests[1].1);
    assert!(
        requests
            .iter()
            .all(|(headers, _, _)| !headers.contains_key("idempotency-key"))
    );
    let body: serde_json::Value = serde_json::from_slice(&requests[0].1).expect("body");
    assert_eq!(body["eventId"], receipt.event_id);
    assert_eq!(body["occurredAt"], "2026-10-27T12:00:00Z");
}

#[tokio::test]
async fn missing_declared_operation_fields_are_protocol_errors_with_response_metadata() {
    use sagat_octopus_orchestrator_sdk::UpdateRequest;
    for mode in ["bad-query", "bad-update"] {
        let server = server(mode).await;
        let client = Client::new(options(&server.base, TokenSource::bearer("fixture-token")))
            .expect("client");
        let error = if mode == "bad-query" {
            client
                .query(RUN, "summary", &json!({}), CallOptions::default())
                .await
                .expect_err("missing result")
        } else {
            client
                .update(
                    RUN,
                    "adjust",
                    UpdateRequest {
                        update_key: "update-1".into(),
                        args: json!({}),
                        expected_state_revision: None,
                    },
                    CallOptions::default(),
                )
                .await
                .expect_err("missing receipt fields")
        };
        assert_eq!(error.code, "PROTOCOL");
        assert_eq!(error.status, Some(200));
        assert_eq!(
            error.trace_id.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
    }
}
#[tokio::test]
async fn invalid_operation_keys_names_and_event_shapes_never_reach_the_server() {
    use sagat_octopus_orchestrator_sdk::{PublishEvent, UpdateRequest};
    let server = server("query").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    assert_eq!(
        client
            .query(RUN, "bad/name", &json!({}), CallOptions::default())
            .await
            .expect_err("name")
            .code,
        "VALIDATION_FAILED"
    );
    assert_eq!(
        client
            .update(
                RUN,
                "adjust",
                UpdateRequest {
                    update_key: "key with spaces".into(),
                    args: json!({}),
                    expected_state_revision: None
                },
                CallOptions::default()
            )
            .await
            .expect_err("key")
            .code,
        "VALIDATION_FAILED"
    );
    assert_eq!(
        client
            .publish_event(
                PublishEvent::new("created", json!([])),
                CallOptions::default()
            )
            .await
            .expect_err("object")
            .code,
        "VALIDATION_FAILED"
    );
    assert!(server.state.calls.lock().expect("requests").is_empty());
}

#[tokio::test]
async fn approval_pages_keep_scoped_identity_and_ignore_additive_scope_fields() {
    use sagat_octopus_orchestrator_sdk::{ApprovalStatus, PageOptions};
    let server = server("approvals").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let page = client
        .list_approvals(RUN, PageOptions::default(), CallOptions::default())
        .await
        .expect("approvals");
    assert_eq!(page.items[0].status, ApprovalStatus::Pending);
    assert_eq!(page.items[0].instance_path[0].scope_node_id, "lines");
    assert_eq!(page.items[0].instance_path[0].iteration_index, 3);
    assert_eq!(page.items[0].raw["newField"], true);
}
#[tokio::test]
async fn approval_decisions_retry_the_unchanged_body_key_and_return_typed_receipts() {
    use sagat_octopus_orchestrator_sdk::{ApprovalDecision, ApprovalDecisionRequest};
    let server = server("decision-retry").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let receipt = client
        .decide_approval(
            RUN,
            ApprovalDecisionRequest {
                decision_key: "approve-1".into(),
                decision: ApprovalDecision::Approve,
                reason_code: Some("review:ok".into()),
            },
            CallOptions::default(),
        )
        .await
        .expect("decision");
    assert_eq!(receipt.approval_id.to_string(), RUN);
    assert!(receipt.replayed);
    assert_eq!(receipt.outcome_sequence, 12);
    assert_eq!(receipt.decided_at.offset(), time::UtcOffset::UTC);
    let calls = server.state.calls.lock().expect("calls");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1, calls[1].1);
    assert!(
        calls
            .iter()
            .all(|(headers, _, _)| !headers.contains_key("idempotency-key"))
    );
    let body: serde_json::Value = serde_json::from_slice(&calls[0].1).expect("body");
    assert_eq!(body["decisionKey"], "approve-1");
    assert_eq!(body["decision"], "APPROVE");
    assert_eq!(body["reasonCode"], "review:ok");
}

#[tokio::test]
async fn approval_iteration_fetches_its_second_page_lazily() {
    use futures_util::StreamExt as _;
    use sagat_octopus_orchestrator_sdk::PageOptions;
    let server = server("approval-pages").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let stream = client.list_all_approvals(RUN, PageOptions::default(), CallOptions::default());
    futures_util::pin_mut!(stream);
    stream.next().await.expect("first").expect("approval");
    assert_eq!(server.state.calls.lock().expect("calls").len(), 1);
    stream.next().await.expect("second").expect("approval");
    assert!(stream.next().await.is_none());
    assert_eq!(server.state.calls.lock().expect("calls").len(), 2);
}
#[tokio::test]
async fn approval_nullability_is_required_and_invalid_decision_keys_stay_local() {
    use sagat_octopus_orchestrator_sdk::{ApprovalDecision, ApprovalDecisionRequest, PageOptions};
    let server = server("bad-approvals").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let error = client
        .list_approvals(RUN, PageOptions::default(), CallOptions::default())
        .await
        .expect_err("settledAt is required even while null");
    assert_eq!(error.code, "PROTOCOL");
    assert_eq!(error.status, Some(200));
    assert_eq!(
        error.trace_id.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    let error = client
        .decide_approval(
            RUN,
            ApprovalDecisionRequest {
                decision_key: "bad key".into(),
                decision: ApprovalDecision::Reject,
                reason_code: None,
            },
            CallOptions::default(),
        )
        .await
        .expect_err("invalid key");
    assert_eq!(error.code, "VALIDATION_FAILED");
    assert_eq!(server.state.calls.lock().expect("calls").len(), 1);
}

#[tokio::test]
async fn chain_inspection_and_completion_wait_use_the_authoritative_final_outcome() {
    use sagat_octopus_orchestrator_sdk::{RunStatus, WaitOptions};
    let server = server("chain-completion").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let chain = client
        .wait_for_chain_completion(
            WORKFLOW,
            RUN,
            WaitOptions {
                poll_interval: Duration::from_millis(1),
                timeout: Duration::from_secs(1),
                ..WaitOptions::default()
            },
        )
        .await
        .expect("final chain");
    assert_eq!(chain.final_outcome, Some(RunStatus::Succeeded));
    assert_eq!(chain.segments[0].index, 0);
    assert_eq!(server.state.calls.lock().expect("calls").len(), 2);
}
#[tokio::test]
async fn chain_commands_freeze_the_expected_head_and_per_call_idempotency_key() {
    use sagat_octopus_orchestrator_sdk::ChainCommand;
    let server = server("chain-command").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    client
        .signal_chain(
            WORKFLOW,
            RUN,
            "ready",
            &json!({"approved":true}),
            ChainCommand {
                expected_run_id: RUN.into(),
            },
            CallOptions::default(),
        )
        .await
        .expect("chain signal");
    client
        .cancel_chain(
            WORKFLOW,
            RUN,
            "operator requested",
            ChainCommand {
                expected_run_id: RUN.into(),
            },
            CallOptions::default(),
        )
        .await
        .expect("chain cancellation");
    let calls = server.state.calls.lock().expect("calls");
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].1, calls[1].1);
    assert_eq!(calls[0].0["idempotency-key"], calls[1].0["idempotency-key"]);
    for (_, bytes, _) in calls.iter() {
        let body: serde_json::Value = serde_json::from_slice(bytes).expect("body");
        assert_eq!(body["expectedRunId"], RUN);
    }
}
#[tokio::test]
async fn chain_state_calls_preserve_the_optional_head_and_the_durable_update_key() {
    use sagat_octopus_orchestrator_sdk::{ChainQueryRequest, ChainUpdateRequest};
    let query = server("chain-query").await;
    let client =
        Client::new(options(&query.base, TokenSource::bearer("fixture-token"))).expect("client");
    client
        .query_chain(
            WORKFLOW,
            RUN,
            "summary",
            ChainQueryRequest {
                args: json!({}),
                expected_run_id: Some(RUN.into()),
            },
            CallOptions::default(),
        )
        .await
        .expect("query");
    let update = server("chain-update").await;
    let client =
        Client::new(options(&update.base, TokenSource::bearer("fixture-token"))).expect("client");
    client
        .update_chain(
            WORKFLOW,
            RUN,
            "adjust",
            ChainUpdateRequest {
                update_key: "update-1".into(),
                args: json!({}),
                expected_state_revision: Some(42),
                expected_run_id: Some(RUN.into()),
            },
            CallOptions::default(),
        )
        .await
        .expect("update");
    for source in [&query, &update] {
        let calls = source.state.calls.lock().expect("calls");
        assert!(!calls[0].0.contains_key("idempotency-key"));
        let body: serde_json::Value = serde_json::from_slice(&calls[0].1).expect("body");
        assert_eq!(body["expectedRunId"], RUN);
    }
}
#[tokio::test]
async fn chain_run_pages_preserve_order_and_continuation_kind_without_payloads() {
    use futures_util::StreamExt as _;
    use sagat_octopus_orchestrator_sdk::{CompletionKind, PageOptions};
    let server = server("chain-pages").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let stream = client.list_all_chain_runs(
        WORKFLOW,
        RUN,
        PageOptions::default(),
        CallOptions::default(),
    );
    futures_util::pin_mut!(stream);
    let first = stream.next().await.expect("first").expect("segment");
    assert_eq!(first.index, 0);
    assert_eq!(first.completion_kind, Some(CompletionKind::Continued));
    assert_eq!(server.state.calls.lock().expect("calls").len(), 1);
    let second = stream.next().await.expect("second").expect("segment");
    assert_eq!(second.index, 1);
    assert_eq!(second.completion_kind, Some(CompletionKind::Final));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn a_stale_chain_head_is_never_retargeted_to_a_new_run() {
    use sagat_octopus_orchestrator_sdk::ChainCommand;
    let server = server("head-changed").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let error = client
        .signal_chain(
            WORKFLOW,
            RUN,
            "ready",
            &json!({}),
            ChainCommand {
                expected_run_id: RUN.into(),
            },
            CallOptions::default(),
        )
        .await
        .expect_err("head changed");
    assert_eq!(error.code, "CONFLICT");
    assert_eq!(error.reason(), Some("CHAIN_HEAD_CHANGED"));
    assert!(!error.retryable);
    assert_eq!(server.state.calls.lock().expect("calls").len(), 1);
}
#[tokio::test]
async fn chain_page_indices_outside_the_canonical_bound_are_protocol_failures() {
    use sagat_octopus_orchestrator_sdk::PageOptions;
    let server = server("bad-chain-page").await;
    let client =
        Client::new(options(&server.base, TokenSource::bearer("fixture-token"))).expect("client");
    let error = client
        .list_chain_runs(
            WORKFLOW,
            RUN,
            PageOptions::default(),
            CallOptions::default(),
        )
        .await
        .expect_err("index bound");
    assert_eq!(error.code, "PROTOCOL");
    assert_eq!(error.status, Some(200));
    assert_eq!(
        error.trace_id.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
}
