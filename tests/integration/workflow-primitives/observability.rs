//! Telemetry and audit evidence of declared Query/Update operations over the real HTTP and
//! PostgreSQL boundaries (T014-009; feature 014 `contracts/README.md` §6): the closed span and
//! metric vocabulary, trace and request correlation, a fail-closed access audit and no
//! caller-chosen or secret-shaped text in spans, metrics or error bodies.

use std::collections::BTreeSet;

use reqwest::{Method, Response, StatusCode};
use sagat_telemetry::{
    Metrics, SpanId, TraceId, WorkflowPrimitiveKind, WorkflowPrimitiveMetric,
    WorkflowPrimitiveOutcome, WorkflowPrimitiveRejectionCode,
};
use serde_json::{Value, json};

use crate::operations_http::{Fixture, fixture, query_path, update_path};
use crate::support::api::json_of;
use crate::support::collector::{Collector, Received};

/// A credential shape the telemetry scrubber recognises; a caller may still choose it as text.
const SECRET_SHAPED: &str = "sk-live1234567890abcdef";

/// Every primitive metric of the contract; the closed-label check covers all of them, whichever
/// test in this process emitted the series.
const PRIMITIVE_SERIES: [WorkflowPrimitiveMetric; 7] = [
    WorkflowPrimitiveMetric::Transitions,
    WorkflowPrimitiveMetric::Active,
    WorkflowPrimitiveMetric::AdmissionWait,
    WorkflowPrimitiveMetric::ApprovalWait,
    WorkflowPrimitiveMetric::OperationDuration,
    WorkflowPrimitiveMetric::Rejections,
    WorkflowPrimitiveMetric::Continuation,
];

/// The attributes a primitive operation span may carry; anything else would be free text.
const SPAN_ATTRIBUTES: [&str; 4] = [
    "sagat.primitive.kind",
    "sagat.outcome",
    "sagat.rejection.code",
    "sagat.operation.replayed",
];

fn traceparent() -> (TraceId, String) {
    let trace = TraceId::generate();
    (trace, format!("00-{trace}-{}-01", SpanId::generate()))
}

async fn post_traced(fixture: &Fixture, path: &str, traceparent: &str, body: &Value) -> Response {
    fixture
        .api
        .as_user(Method::POST, path, "alice", "acme")
        .header("traceparent", traceparent)
        .json(body)
        .send()
        .await
        .expect("send declared operation")
}

/// The sum of every series of `name` whose labels include all of `labels`.
fn total(page: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
    page.lines()
        .filter(|line| {
            line.starts_with(name)
                && matches!(line[name.len()..].chars().next(), Some('{' | ' '))
                && labels
                    .iter()
                    .all(|(key, value)| line.contains(&format!("{key}=\"{value}\"")))
        })
        .filter_map(|line| line.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}

fn labels(line: &str) -> Vec<(&str, &str)> {
    let Some((_, rest)) = line.split_once('{') else {
        return Vec::new();
    };
    let Some((inner, _)) = rest.rsplit_once('}') else {
        return Vec::new();
    };
    inner
        .split(',')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((key, value.trim_matches('"')))
        })
        .collect()
}

/// Every primitive series on the page has only the contracted label keys, and every value comes
/// from the closed telemetry enums.
fn assert_closed_primitive_series(page: &str) {
    let kinds = WorkflowPrimitiveKind::ALL.map(WorkflowPrimitiveKind::as_str);
    let outcomes = WorkflowPrimitiveOutcome::ALL.map(WorkflowPrimitiveOutcome::as_str);
    let codes = WorkflowPrimitiveRejectionCode::ALL.map(WorkflowPrimitiveRejectionCode::as_str);
    for line in page.lines().filter(|line| {
        PRIMITIVE_SERIES
            .iter()
            .any(|metric| line.starts_with(metric.as_str()))
    }) {
        for (key, value) in labels(line) {
            match key {
                "kind" => assert!(kinds.contains(&value), "{line}"),
                "outcome" => assert!(outcomes.contains(&value), "{line}"),
                "code" => assert!(codes.contains(&value), "{line}"),
                "le" | "quantile" => {}
                other => panic!("unexpected label {other} in {line}"),
            }
        }
    }
}

fn assert_closed_span(span: &Received, kind: &str) {
    assert_eq!(span.attr("sagat.primitive.kind"), Some(kind), "{span:?}");
    for (key, value) in &span.attributes {
        assert!(SPAN_ATTRIBUTES.contains(&key.as_str()), "{key} in {span:?}");
        assert!(!value.contains("balance"), "{span:?}");
        assert!(!value.contains(SECRET_SHAPED), "{span:?}");
    }
}

#[tokio::test]
async fn declared_operations_emit_the_closed_span_and_metric_vocabulary() {
    let collector = Collector::start().await;
    let (_guard, sink) = collector.install(1.0);
    let fixture = fixture("observed-declared-operations").await;
    let metrics = Metrics::global();
    let before = metrics.render();
    let (trace, parent) = traceparent();

    let query = post_traced(
        &fixture,
        &query_path(&fixture),
        &parent,
        &json!({"args": 2}),
    )
    .await;
    assert_eq!(query.status(), StatusCode::OK);
    let applied_body = json!({
        "updateKey": SECRET_SHAPED,
        "args": {"allowed": true, "nextState": {"balance": 41}}
    });
    let applied = post_traced(&fixture, &update_path(&fixture), &parent, &applied_body).await;
    assert_eq!(applied.status(), StatusCode::OK);
    let replayed = post_traced(&fixture, &update_path(&fixture), &parent, &applied_body).await;
    assert_eq!(replayed.status(), StatusCode::OK);
    let rejected = post_traced(
        &fixture,
        &update_path(&fixture),
        &parent,
        &json!({"updateKey": "denied", "args": {"allowed": false, "nextState": {"balance": 42}}}),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let reused = post_traced(
        &fixture,
        &update_path(&fixture),
        &parent,
        &json!({"updateKey": SECRET_SHAPED, "args": {"allowed": true, "nextState": {"balance": 43}}}),
    )
    .await;
    assert_eq!(reused.status(), StatusCode::CONFLICT);
    let reused = json_of(reused).await;
    assert_eq!(reused["details"]["reason"], "IDEMPOTENCY_KEY_REUSED");
    assert!(!reused.to_string().contains(SECRET_SHAPED));

    let trace = trace.to_string();
    let queries = collector
        .wait_for(1, |span| {
            span.name == "workflow.query" && span.trace_id == trace
        })
        .await;
    let updates = collector
        .wait_for(4, |span| {
            span.name == "workflow.update" && span.trace_id == trace
        })
        .await;
    let requests: BTreeSet<String> = collector
        .wait_for(5, |span| {
            span.name == "http.request" && span.trace_id == trace
        })
        .await
        .into_iter()
        .map(|span| span.span_id)
        .collect();
    sink.shutdown(std::time::Duration::from_secs(5)).await;

    assert_closed_span(&queries[0], "query");
    assert_eq!(queries[0].attr("sagat.outcome"), Some("succeeded"));
    for span in queries.iter().chain(&updates) {
        let parent = span
            .parent
            .as_ref()
            .expect("an operation span has a parent");
        assert!(
            requests.contains(parent),
            "hangs from its request: {span:?}"
        );
    }
    let mut observed: Vec<_> = updates
        .iter()
        .map(|span| {
            assert_closed_span(span, "update");
            (
                span.attr("sagat.outcome").unwrap_or_default().to_owned(),
                span.attr("sagat.rejection.code").map(str::to_owned),
                span.attr("sagat.operation.replayed").map(str::to_owned),
            )
        })
        .collect();
    observed.sort();
    assert_eq!(
        observed,
        vec![
            ("applied".to_owned(), None, Some("false".to_owned())),
            ("applied".to_owned(), None, Some("true".to_owned())),
            (
                "rejected".to_owned(),
                Some("IDEMPOTENCY_KEY_REUSED".to_owned()),
                None
            ),
            (
                "rejected".to_owned(),
                Some("UPDATE_REJECTED".to_owned()),
                None
            ),
        ]
    );

    let after = metrics.render();
    let grew = |name: &str, labels: &[(&str, &str)], by: f64| {
        assert!(
            total(&after, name, labels) >= total(&before, name, labels) + by,
            "{name} {labels:?}"
        );
    };
    let duration = "workflow_operation_duration_seconds_count";
    grew(
        duration,
        &[("kind", "query"), ("outcome", "succeeded")],
        1.0,
    );
    grew(duration, &[("kind", "update"), ("outcome", "applied")], 2.0);
    grew(
        duration,
        &[("kind", "update"), ("outcome", "rejected")],
        2.0,
    );
    let rejections = "workflow_primitive_rejections_total";
    grew(
        rejections,
        &[("kind", "update"), ("code", "UPDATE_REJECTED")],
        1.0,
    );
    grew(
        rejections,
        &[("kind", "update"), ("code", "IDEMPOTENCY_KEY_REUSED")],
        1.0,
    );
    assert_closed_primitive_series(&after);
    assert!(!after.contains("workflow_declared_operations_total"));
    assert!(!after.contains(SECRET_SHAPED));
    assert!(!after.contains("balance."));
    assert!(!after.contains(&fixture.run.to_string()));
    assert!(!after.contains(&fixture.tenant.to_string()));
}

#[tokio::test]
async fn trace_and_request_ids_correlate_the_error_and_the_access_audit() {
    let fixture = fixture("correlated-declared-operations").await;
    let (trace, parent) = traceparent();

    let query = post_traced(
        &fixture,
        &query_path(&fixture),
        &parent,
        &json!({"args": 1}),
    )
    .await;
    assert_eq!(query.status(), StatusCode::OK);
    let audited: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT action, trace_id FROM audit_records \
         WHERE tenant_id=$1 AND resource_id=$2 AND action='run.queried'",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.to_string())
    .fetch_all(&fixture.api.db.pool)
    .await
    .expect("query access audit");
    assert_eq!(
        audited,
        vec![("run.queried".to_owned(), Some(trace.to_string()))]
    );

    let conflict = post_traced(
        &fixture,
        &update_path(&fixture),
        &parent,
        &json!({
            "updateKey": "stale-correlated",
            "args": {"allowed": true, "nextState": {"balance": 5}},
            "expectedStateRevision": 3
        }),
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let conflict = json_of(conflict).await;
    assert_eq!(conflict["details"]["reason"], "REVISION_CONFLICT");
    assert_eq!(conflict["traceId"], trace.to_string());
    let request_id = conflict["requestId"].as_str().expect("request id");
    assert_eq!(request_id.len(), 32);
    assert_ne!(
        request_id,
        trace.to_string(),
        "a request id is never the trace"
    );
}

#[tokio::test]
async fn a_failed_audit_write_fails_the_operation_closed_and_discloses_nothing() {
    let fixture = fixture("unaudited-declared-operations").await;
    sqlx::raw_sql(
        "CREATE FUNCTION reject_operation_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
           BEGIN \
             IF NEW.action IN ('run.queried', 'run.updated') THEN \
               RAISE EXCEPTION 'injected operation audit failure'; \
             END IF; \
             RETURN NEW; \
           END $$; \
         CREATE TRIGGER reject_operation_audit BEFORE INSERT ON audit_records \
           FOR EACH ROW EXECUTE FUNCTION reject_operation_audit();",
    )
    .execute(&fixture.api.db.pool)
    .await
    .expect("install audit fault");
    let metrics = Metrics::global();
    let before = metrics.render();

    let query = fixture
        .api
        .post_as(
            &query_path(&fixture),
            "alice",
            "acme",
            &json!({"args": 9_000_001}),
        )
        .await;
    assert_eq!(query.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(query.headers()["cache-control"], "no-store");
    let query = json_of(query).await;
    assert_eq!(query["code"], "INTERNAL");
    assert!(query.get("result").is_none());
    assert!(!query.to_string().contains("9000011"), "no evaluated value");
    assert!(!query.to_string().contains("injected"));

    let update = fixture
        .api
        .post_as(
            &update_path(&fixture),
            "alice",
            "acme",
            &json!({"updateKey": "unaudited", "args": {"allowed": true, "nextState": {"balance": 8_675_309}}}),
        )
        .await;
    assert_eq!(update.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let update = json_of(update).await;
    assert!(!update.to_string().contains("8675309"), "{update}");

    let unchanged: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT r.run_version, s.state_revision, \
                (SELECT count(*) FROM operation_update_receipts x WHERE x.tenant_id=r.tenant_id AND x.resolved_run_id=r.id), \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.event_type='state.updated'), \
                (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=r.tenant_id AND o.aggregate_id=r.id) \
         FROM runs r JOIN workflow_operation_states s ON s.tenant_id=r.tenant_id AND s.run_id=r.id \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("state after the refused audit");
    assert_eq!(unchanged, (1, 0, 0, 0, 0));

    let after = metrics.render();
    let duration = "workflow_operation_duration_seconds_count";
    for kind in ["query", "update"] {
        let labels = [("kind", kind), ("outcome", "failed")];
        assert!(
            total(&after, duration, &labels) >= total(&before, duration, &labels) + 1.0,
            "{kind}"
        );
    }
}
