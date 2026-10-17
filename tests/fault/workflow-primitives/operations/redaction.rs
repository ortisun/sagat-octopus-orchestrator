//! Secrets and secret-shaped text around the declared operations. A real secret is stored, resolved
//! by a worker for a step of the very run that is then queried and updated, and a credential-shaped
//! marker is pushed through every refusal path. Neither appears in any response or error body, in
//! any row of any table (audit, outbox, journal, receipts, payloads), or in any span, log line or
//! metric the requests produced.

use std::time::Duration;

use reqwest::StatusCode;
use sagat_application::bindings::BindingService;
use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_application::worker::{Capabilities, WorkerService, WorkerSettings};
use sagat_domain::identity::{Membership, Principal, Role};
use sagat_infra_ir::JsonEvaluator;
use sagat_telemetry::Metrics;
use serde_json::{Value, json};
use sqlx::PgPool;

use super::engine::declared;
use super::{Fixture, fixture_for, query_path, update_body, update_path};
use crate::support::api::json_of;
use crate::support::collector::Collector;
use crate::support::engine::{activity, ctx, task_ref, with};
use crate::support::subject;

/// The value of a secret in the vault, as an API key would look.
const SECRET: &str = "sk-live-4eC39HqLyjWDarjtT1zdp7dc-operations";
/// Text a caller chooses, shaped like a credential, that is refused before it is stored.
const MARKER: &str = "sk-live-9fA77ZqLyjWDarjtT1zdp7dc-marker";

/// `work -> hold`, where `work` declares the secret `op-key`.
fn document(name: &str) -> Value {
    declared(
        name,
        "work",
        vec![
            with(
                with(activity("work"), "secrets", json!(["op-key"])),
                "next",
                json!("hold"),
            ),
            activity("hold"),
        ],
    )
}

/// Every table whose rows mention `needle`, as text or as stored bytes, with how many rows do.
async fn tables_mentioning(pool: &PgPool, needle: &str) -> Vec<(String, i64)> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename FROM pg_tables WHERE schemaname = 'public' ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .expect("tables");
    let hex: String = needle.bytes().map(|byte| format!("{byte:02x}")).collect();
    let mut found = Vec::new();
    for table in tables {
        // Table names come from the catalogue, not from a caller. A `bytea` column prints as
        // `\x` followed by hex, so the needle is searched both as text and as hex.
        let sql = format!(
            "SELECT count(*) FROM \"{table}\" x \
             WHERE strpos(x::text, $1) > 0 OR strpos(x::text, $2) > 0"
        );
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(needle)
            .bind(&hex)
            .fetch_one(pool)
            .await
            .expect("count");
        if count > 0 {
            found.push((table, count));
        }
    }
    found
}

#[tokio::test]
async fn a_resolved_secret_and_secret_shaped_text_never_leave_the_operation_boundary() {
    let collector = Collector::start().await;
    let (_guard, sink, logs) = collector.install_with_logs(1.0);
    let before_metrics = Metrics::global().render();

    let fixture: Fixture = fixture_for(
        "operations-redaction",
        document("operations-redaction"),
        &[("op-key", SECRET)],
    )
    .await;
    let pool = &fixture.api.db.pool;

    // The secret is stored sealed, and a worker resolves it for the `work` step of this run.
    let admin = Principal::new(fixture.tenant, subject("alice"), Role::Admin);
    let store = fixture.api.executions.clone();
    let worker = Principal::new(fixture.tenant, subject("worker-1"), Role::Worker);
    // Production registration rechecks current membership; constructing a trusted principal alone
    // does not provision a machine identity in the tenant.
    fixture
        .api
        .owner_store
        .insert_membership(&Membership::restore(
            fixture.tenant,
            worker.subject().clone(),
            Role::Worker,
            crate::support::now(),
        ))
        .await
        .expect("current worker membership");
    BindingService::new(store.clone(), SystemClock)
        .bind(
            &admin,
            fixture.namespace,
            worker.subject().as_str(),
            &["*".to_owned()],
            &ctx(),
        )
        .await
        .expect("bind the worker");
    let workers = WorkerService::new(
        store,
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    )
    .with_vault(fixture.api.keys.cipher());
    let registered = workers
        .register(
            &worker,
            "payments",
            "redaction-worker",
            &Capabilities {
                activities: vec!["act.work".into(), "act.hold".into()],
                languages: vec!["rust".into()],
                pools: Vec::new(),
                max_inflight: 4,
                protocol_version: "1".into(),
            },
        )
        .await
        .expect("register the worker");
    let token = registered.session_token.expose().to_owned();
    let tasks = workers
        .poll(&worker, registered.worker_id, &token, 4, None)
        .await
        .expect("poll");
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.run_id, fixture.run);
    let resolved = workers
        .resolve_secrets(&worker, &task_ref(task), None)
        .await
        .expect("resolve the secret for the step");
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].value.expose(),
        SECRET.as_bytes(),
        "the step really was handed the secret"
    );

    // Operations of the same run: successes and every refusal path, with the marker in each.
    let mut bodies: Vec<(String, StatusCode, Value)> = Vec::new();
    let mut send = |label: &str, response: (StatusCode, Value)| {
        bodies.push((label.to_owned(), response.0, response.1));
    };
    let post = |path: String, body: Value| {
        let fixture = &fixture;
        async move {
            let response = fixture.api.post_as(&path, "alice", "acme", &body).await;
            let status = response.status();
            (status, json_of(response).await)
        }
    };
    send(
        "query",
        post(query_path(&fixture), json!({"args": 3})).await,
    );
    send(
        "query with a marker argument",
        post(query_path(&fixture), json!({"args": MARKER})).await,
    );
    send(
        "query of a marker-named operation",
        post(
            format!("/runs/{}/queries/{MARKER}", fixture.run),
            json!({"args": 1}),
        )
        .await,
    );
    send(
        "update applied",
        post(update_path(&fixture), update_body("redaction-applied", 21)).await,
    );
    send(
        "update replayed",
        post(update_path(&fixture), update_body("redaction-applied", 21)).await,
    );
    send(
        "update with a marker property",
        post(
            update_path(&fixture),
            json!({
                "updateKey": "redaction-property",
                "args": {"allowed": true, "nextState": {"balance": 1}, MARKER: 1},
                "expectedStateRevision": 1
            }),
        )
        .await,
    );
    send(
        "update with a marker value",
        post(
            update_path(&fixture),
            json!({
                "updateKey": "redaction-value",
                "args": {"allowed": true, "nextState": {"balance": MARKER}},
                "expectedStateRevision": 1
            }),
        )
        .await,
    );
    send(
        "update with a marker key",
        post(
            update_path(&fixture),
            json!({
                "updateKey": format!("{MARKER} {MARKER}"),
                "args": {"allowed": true, "nextState": {"balance": 2}},
                "expectedStateRevision": 1
            }),
        )
        .await,
    );
    send(
        "update refused by the validator",
        post(
            update_path(&fixture),
            json!({
                "updateKey": "redaction-denied",
                "args": {"allowed": false, "nextState": {"balance": 3}},
                "expectedStateRevision": 1
            }),
        )
        .await,
    );
    send(
        "update with a stale revision",
        post(update_path(&fixture), update_body("redaction-stale", 4)).await,
    );
    send(
        "update reusing a key",
        post(update_path(&fixture), update_body("redaction-applied", 22)).await,
    );
    send(
        "update of a marker-named operation",
        post(
            format!("/runs/{}/updates/{MARKER}", fixture.run),
            update_body("redaction-unknown", 5),
        )
        .await,
    );
    // The step that held the secret finishes; the run goes on and is queried again.
    workers
        .complete(
            &worker,
            &task_ref(task),
            br#"{"amount": 7}"#,
            "redaction-complete",
        )
        .await
        .expect("complete the step");
    send(
        "query after the step",
        post(query_path(&fixture), json!({"args": 4})).await,
    );

    // The answers: the expected ones came back, so every refusal path really ran.
    let statuses: Vec<(String, u16)> = bodies
        .iter()
        .map(|(label, status, _)| (label.clone(), status.as_u16()))
        .collect();
    let status_of = |label: &str| {
        statuses
            .iter()
            .find(|(candidate, _)| candidate == label)
            .map(|(_, status)| *status)
            .unwrap_or_else(|| panic!("no answer for {label}"))
    };
    assert_eq!(status_of("query"), 200);
    assert_eq!(status_of("query with a marker argument"), 422);
    assert_eq!(status_of("query of a marker-named operation"), 404);
    assert_eq!(status_of("update applied"), 200);
    assert_eq!(status_of("update replayed"), 200);
    assert_eq!(status_of("update with a marker property"), 422);
    assert_eq!(status_of("update with a marker value"), 422);
    assert_eq!(status_of("update with a marker key"), 422);
    assert_eq!(status_of("update refused by the validator"), 422);
    assert_eq!(status_of("update with a stale revision"), 409);
    assert_eq!(status_of("update reusing a key"), 409);
    assert_eq!(status_of("update of a marker-named operation"), 404);
    assert_eq!(status_of("query after the step"), 200);
    for (label, _, body) in &bodies {
        let text = body.to_string();
        assert!(!text.contains(SECRET), "{label}: {text}");
        assert!(!text.contains(MARKER), "{label}: {text}");
    }

    // Storage: no row of any table, the journal, outbox, audit, receipts and payloads included.
    assert_eq!(tables_mentioning(pool, SECRET).await, Vec::new());
    assert_eq!(tables_mentioning(pool, MARKER).await, Vec::new());
    // The control that proves the sweep sees stored bytes: the run's own input, which the fixture
    // started it with and no operation reads, is found in the payload store.
    let stored = tables_mentioning(pool, super::CANARY).await;
    assert!(
        stored.iter().any(|(table, _)| table == "payloads"),
        "the sweep cannot see payload bytes: {stored:?}"
    );

    // Telemetry: spans with their events, log lines at every level, and the metrics page.
    sink.shutdown(Duration::from_secs(5)).await;
    let spans = collector.spans();
    assert!(
        spans.iter().any(|span| span.name == "workflow.update"),
        "the update spans were exported"
    );
    let spans = format!("{spans:?}");
    let logs = logs.text();
    let metrics = Metrics::global().render();
    assert!(
        logs.contains("declared update resolved"),
        "the log capture saw the requests"
    );
    for (what, text) in [("spans", &spans), ("logs", &logs), ("metrics", &metrics)] {
        assert!(!text.contains(SECRET), "{what} leak the secret");
        assert!(!text.contains(MARKER), "{what} leak the marker");
    }
    assert!(!before_metrics.contains(SECRET));
}
