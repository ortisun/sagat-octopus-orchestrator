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
        calls.push((headers, body.to_vec(), uri));
        calls.len()
    };
    match state.mode {
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
