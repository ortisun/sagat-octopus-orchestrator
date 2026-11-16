//! Design 043 against a real PostgreSQL, rows 11 and 12 of the creators: a child run and a continue-as-new, whose parent
//! or predecessor may have been created before the check was on (T043-8, T043-9, FR-043-008, FR-043-009,
//! OD-2026-11-14-9).

use sagat_application::definitions::InputValidationMode;
use sagat_application::execution::child_run_id;
use sagat_domain::execution::{RunStatus, StepStatus};
use sagat_domain::workflow::NodeId;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{World, activity, child_node, document, world};
use crate::support::run_input::{order_schema, publish_in, unusable_schema, with_schema};

async fn step_code(w: &World, run: sagat_domain::execution::RunId, node: &str) -> Option<String> {
    sqlx::query_scalar("SELECT failure_code FROM steps WHERE run_id = $1 AND node = $2")
        .bind(run.as_uuid())
        .bind(node)
        .fetch_one(&w.db.pool)
        .await
        .expect("code")
}

/// Row 11: a child whose version refuses the input the parent's node evaluated is not created, and the parent step fails
/// `CHILD_INPUT_INVALID`; a schema outside the subset fails it `RUN_INPUT_SCHEMA_UNUSABLE`; a matching input starts it.
#[tokio::test]
async fn row_11_a_refused_child_is_not_created_and_its_parent_step_fails_with_the_code() {
    let w = world().await;
    let kid = format!("row-11-kid-{}", Uuid::now_v7());
    w.publish(with_schema(
        document(&kid, "a", vec![activity("a")]),
        order_schema(),
    ))
    .await;
    let unusable = format!("row-11-unusable-{}", Uuid::now_v7());
    publish_in(
        &w,
        InputValidationMode::Report,
        &with_schema(
            document(&unusable, "a", vec![activity("a")]),
            unusable_schema(),
        ),
    )
    .await;
    for (label, child, input, expected) in [
        (
            "refused",
            kid.as_str(),
            json!({"orderId": 42}),
            Some("CHILD_INPUT_INVALID"),
        ),
        (
            "unusable",
            unusable.as_str(),
            json!({"orderId": "o"}),
            Some("RUN_INPUT_SCHEMA_UNUSABLE"),
        ),
        ("matching", kid.as_str(), json!({"orderId": "o-1"}), None),
    ] {
        let (parent_workflow, _) = w
            .publish(document(
                &format!("row-11-parent-{label}-{}", Uuid::now_v7()),
                "c",
                vec![child_node("c", child, None)],
            ))
            .await;
        // the parent is a run like one created before the upgrade: it has no schema, its child is checked all the same
        let parent = w.start(parent_workflow, &input).await;
        w.scheduler.tick().await;
        let child_run = child_run_id(parent, &NodeId::parse("c").expect("node"));
        match expected {
            Some(code) => {
                assert!(w.run_record(child_run).await.is_none(), "{label}: no child");
                assert_eq!(
                    w.step_status(parent, "c").await,
                    Some(StepStatus::Failed),
                    "{label}"
                );
                assert_eq!(
                    step_code(&w, parent, "c").await.as_deref(),
                    Some(code),
                    "{label}"
                );
                assert_eq!(w.run_status(parent).await, RunStatus::Failed, "{label}");
            }
            None => assert!(
                w.run_record(child_run).await.is_some(),
                "{label}: the child exists"
            ),
        }
    }
}

fn rolling(name: &str, schema: Value) -> Value {
    with_schema(
        json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": name},
            "entrypoint": "work",
            "nodes": [
                {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
                {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
            ],
            "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
        }),
        schema,
    )
}

async fn stop_code(w: &World, run: sagat_domain::execution::RunId) -> Value {
    let stop: Value = sqlx::query_scalar(
        "SELECT detail FROM run_events WHERE run_id = $1 AND event_type = 'run.stop_requested'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the stop that ended the run");
    stop["code"].clone()
}

async fn head(w: &World, chain: sagat_domain::execution::RunId) -> Uuid {
    sqlx::query_scalar("SELECT current_run_id FROM execution_chains WHERE chain_id = $1")
        .bind(chain.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("head")
}

/// Row 12: a continuation whose evaluated input the run's version refuses creates no successor, and the predecessor ends
/// `FAILED` with `CONTINUATION_INPUT_INVALID`; a schema outside the subset ends it `RUN_INPUT_SCHEMA_UNUSABLE`.
#[tokio::test]
async fn row_12_a_refused_continuation_fails_its_run_and_creates_no_successor() {
    for (label, unusable) in [("refused", false), ("unusable", true)] {
        let w = world().await;
        let name = format!("row-12-{label}-{}", Uuid::now_v7());
        let schema = json!({
            "type": "object",
            "required": ["segment"],
            "properties": {"segment": {"type": "integer"}}
        });
        let (workflow, _) = if unusable {
            // stored as a version of 0.2.x would be: the chain started before the check, its continuation is checked
            publish_in(
                &w,
                InputValidationMode::Report,
                &rolling(&name, unusable_schema()),
            )
            .await
        } else {
            w.publish(rolling(&name, schema)).await
        };
        let chain = if unusable {
            // the first segment is started in report mode, as one started before the upgrade was
            let reported = sagat_application::runs::RunService::new(
                w.store.clone(),
                sagat_application::clock::SystemClock,
                sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
                w.cache.clone(),
            )
            .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
                InputValidationMode::Report,
            ));
            let input =
                serde_json::to_vec(&json!({"segment": 1, "next": {"segment": 2}})).expect("json");
            reported
                .start(
                    &w.admin,
                    &sagat_application::runs::StartRun {
                        namespace: "payments",
                        workflow,
                        version: None,
                        input: &input,
                        correlation_id: None,
                        deadline: None,
                        key: &format!("start-{}", Uuid::now_v7()),
                    },
                    &crate::support::engine::ctx(),
                )
                .await
                .expect("started in report mode")
                .info
                .run_id
        } else {
            w.start(workflow, &json!({"segment": 1, "next": {"segment": "two"}}))
                .await
        };
        let scheduler = crate::chains::scheduler(&w, &format!("row-12-{label}")).await;
        let session = w.session(&["segment.work"]).await;
        let task = w.one_task(&session).await;
        w.complete(&session, &task, &json!({}))
            .await
            .expect("complete");
        scheduler.tick().await;
        assert_eq!(w.run_status(chain).await, RunStatus::Failed, "{label}");
        let expected = if unusable {
            "RUN_INPUT_SCHEMA_UNUSABLE"
        } else {
            "CONTINUATION_INPUT_INVALID"
        };
        assert_eq!(stop_code(&w, chain).await, json!(expected), "{label}");
        assert_eq!(
            head(&w, chain).await,
            chain.as_uuid(),
            "{label}: no successor"
        );
    }
}

/// Both reviewers of phase 2: in `report` mode a continuation whose input is refused hands off, and the refusal is
/// audited once, in the hand-off transaction (D043-039).
#[tokio::test]
async fn report_mode_audits_a_refused_continuation_with_its_hand_off() {
    let w = world().await;
    let name = format!("row-12-report-{}", Uuid::now_v7());
    let (workflow, _) = w
        .publish(rolling(
            &name,
            json!({"type": "object", "required": ["segment"], "properties": {"segment": {"type": "integer"}}}),
        ))
        .await;
    let chain = w
        .start(workflow, &json!({"segment": 1, "next": {"segment": "two"}}))
        .await;
    let scheduler =
        crate::chains::scheduler_in(&w, "row-12-report", InputValidationMode::Report).await;
    let session = w.session(&["segment.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("complete");
    scheduler.tick().await;
    assert_ne!(
        head(&w, chain).await,
        chain.as_uuid(),
        "the successor was created"
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.input_refused' AND reason LIKE 'reported: RUN_INPUT_INVALID%'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("audits");
    assert_eq!(audits, 1);
}

/// Row 11 with a parent created before the check was on (`report` mode, an input its own schema refuses): its child is
/// checked all the same (OD-2026-11-14-9).
#[tokio::test]
async fn row_11_the_child_of_a_parent_admitted_before_the_check_is_checked() {
    let w = world().await;
    let kid = format!("row-11-old-kid-{}", Uuid::now_v7());
    w.publish(with_schema(
        document(&kid, "a", vec![activity("a")]),
        order_schema(),
    ))
    .await;
    let (parent_workflow, _) = w
        .publish(with_schema(
            document(&format!("row-11-old-parent-{}", Uuid::now_v7()), "c", vec![child_node("c", &kid, None)]),
            json!({"type": "object", "required": ["orderId"], "properties": {"orderId": {"type": "string"}}}),
        ))
        .await;
    let old = sagat_application::runs::RunService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
        InputValidationMode::Report,
    ));
    let input = serde_json::to_vec(&json!({"orderId": 42})).expect("json");
    let parent = old
        .start(
            &w.admin,
            &sagat_application::runs::StartRun {
                namespace: "payments",
                workflow: parent_workflow,
                version: None,
                input: &input,
                correlation_id: None,
                deadline: None,
                key: &format!("start-{}", Uuid::now_v7()),
            },
            &crate::support::engine::ctx(),
        )
        .await
        .expect("admitted before the check")
        .info
        .run_id;
    w.scheduler.tick().await;
    assert!(
        w.run_record(child_run_id(parent, &NodeId::parse("c").expect("node")))
            .await
            .is_none()
    );
    assert_eq!(
        step_code(&w, parent, "c").await.as_deref(),
        Some("CHILD_INPUT_INVALID")
    );
}

fn scheduler_with(
    w: &World,
    answers: Vec<Result<(), sagat_application::run_input::SchemaRefusal>>,
) -> crate::support::engine::Sched {
    use std::sync::Arc;
    sagat_scheduler::Scheduler::new(
        w.store.clone(),
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_scheduler::SchedulerSettings::default(),
    )
    .with_run_inputs(Arc::new(sagat_application::run_input::RunInputGate::new(
        Arc::new(sagat_application::run_input::test_support::Scripted::answering(answers)),
        InputValidationMode::Enforce,
    )))
    .with_children(w.cache.clone())
}

/// T043-21 for row 11: the work budget is final the first time (the step fails `CHILD_INPUT_INVALID`); no room in the
/// validator is transient (the reservation is left for the next pass, no child, no failure), and the next pass with room
/// starts the child.
#[tokio::test]
async fn row_11_the_budget_is_final_and_no_room_leaves_the_child_for_the_next_pass() {
    use sagat_application::run_input::SchemaRefusal;

    let w = world().await;
    let kid = format!("row-11-budget-kid-{}", Uuid::now_v7());
    w.publish(with_schema(
        document(&kid, "a", vec![activity("a")]),
        json!({"type": "object"}),
    ))
    .await;
    let (parent_workflow, _) = w
        .publish(document(
            &format!("row-11-budget-{}", Uuid::now_v7()),
            "c",
            vec![child_node("c", &kid, None)],
        ))
        .await;

    let costly = w.start(parent_workflow, &json!({})).await;
    scheduler_with(&w, vec![Err(SchemaRefusal::WorkLimit)])
        .tick()
        .await;
    assert_eq!(
        step_code(&w, costly, "c").await.as_deref(),
        Some("CHILD_INPUT_INVALID")
    );

    let busy = w.start(parent_workflow, &json!({})).await;
    let scheduler = scheduler_with(
        &w,
        vec![Err(SchemaRefusal::Busy), Err(SchemaRefusal::Backstop)],
    );
    scheduler.tick().await;
    scheduler.tick().await;
    let child = child_run_id(busy, &NodeId::parse("c").expect("node"));
    assert!(w.run_record(child).await.is_none(), "no child yet");
    assert_eq!(step_code(&w, busy, "c").await, None, "not failed");
    assert_ne!(w.step_status(busy, "c").await, Some(StepStatus::Failed));
    scheduler.tick().await;
    assert!(
        w.run_record(child).await.is_some(),
        "with room, the child starts"
    );
}

/// T043-21 for row 12: no room in the validator leaves the boundary waiting (no successor, no failure); the work budget
/// ends the run `FAILED` with `CONTINUATION_INPUT_INVALID`.
#[tokio::test]
async fn row_12_no_room_waits_and_the_budget_is_final() {
    use sagat_application::run_input::SchemaRefusal;

    let w = world().await;
    let name = format!("row-12-budget-{}", Uuid::now_v7());
    let (workflow, _) = w.publish(rolling(&name, json!({"type": "object"}))).await;
    let chain = w
        .start(workflow, &json!({"segment": 1, "next": {"segment": 2}}))
        .await;
    let session = w.session(&["segment.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("complete");

    let placement = sagat_scheduler::Placement::new(format!("row-12-budget-{}", Uuid::now_v7()));
    sagat_scheduler::Keeper::new(
        w.store.clone(),
        placement.clone(),
        sagat_scheduler::KeeperSettings {
            ttl: std::time::Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("shards");
    let busy = scheduler_with(&w, vec![Err(SchemaRefusal::Busy)]).with_placement(placement.clone());
    let report = busy.tick().await;
    assert_eq!(head(&w, chain).await, chain.as_uuid(), "no successor");
    assert!(
        !w.run_status(chain).await.is_terminal(),
        "the boundary waits"
    );
    assert_eq!(report.failed, 0, "waiting is not a failure: {report:?}");
    let costly = scheduler_with(&w, vec![Err(SchemaRefusal::WorkLimit)]).with_placement(placement);
    costly.tick().await;
    assert_eq!(w.run_status(chain).await, RunStatus::Failed);
    assert_eq!(
        stop_code(&w, chain).await,
        json!("CONTINUATION_INPUT_INVALID")
    );
}

/// D043-038, G043-043: a child a peer created between this replica's read of the reservation and its check (a 0.2.x engine,
/// or one in `report`) is answered as created, and the parent step is not failed, though this replica's gate refuses it.
#[tokio::test]
async fn a_child_a_peer_created_before_the_check_is_not_failed_by_the_refusal() {
    use sagat_application::run_input::{PathSegment, Rule, SchemaRefusal};
    use std::sync::Arc;

    let w = Arc::new(world().await);
    let kid = format!("row-11-peer-kid-{}", Uuid::now_v7());
    w.publish(with_schema(
        document(&kid, "a", vec![activity("a")]),
        json!({"type": "object"}),
    ))
    .await;
    let (parent_workflow, _) = w
        .publish(document(
            &format!("row-11-peer-{}", Uuid::now_v7()),
            "c",
            vec![child_node("c", &kid, None)],
        ))
        .await;
    let parent = w.start(parent_workflow, &json!({})).await;
    // the peer: a scheduler in `report` mode starts the child of the same reservation
    let peer = Arc::new(crate::support::engine::scheduler_in_report(&w));
    let hook_peer = Arc::clone(&peer);
    let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_ran = Arc::clone(&ran);
    let hook: sagat_scheduler::BeforeChildCheck = Arc::new(move |_child| {
        let peer = Arc::clone(&hook_peer);
        let ran = Arc::clone(&hook_ran);
        Box::pin(async move {
            if !ran.swap(true, std::sync::atomic::Ordering::SeqCst) {
                peer.tick().await;
            }
        })
    });
    let refusing = scheduler_with(
        &w,
        vec![Err(SchemaRefusal::Invalid {
            segments: vec![PathSegment::Property("k".into())],
            rule: Rule::Type,
        })],
    )
    .with_before_child_check(hook);
    refusing.tick().await;
    let child = child_run_id(parent, &NodeId::parse("c").expect("node"));
    assert!(
        w.run_record(child).await.is_some(),
        "the peer created the child"
    );
    assert_eq!(
        step_code(&w, parent, "c").await,
        None,
        "the step is not failed"
    );
    assert_ne!(w.step_status(parent, "c").await, Some(StepStatus::Failed));
}

/// Correctness review of 2026-11-16, Major 3 (D043-164): a peer claims the reservation of the child (the claim of
/// `claim_reservation`, held uncommitted by the test) while this replica refuses the child's input. The refusal reads the
/// step under its row lock, so it waits for the claim, sees the child started, and does not fail the step. The claim is
/// committed only once the refusal is seen waiting on the step's row (to read it, or, without the read, to write it).
#[tokio::test]
async fn a_child_claimed_while_its_refusal_waits_for_the_step_is_not_failed() {
    use sagat_application::run_input::{PathSegment, Rule, SchemaRefusal};
    use std::sync::Arc;

    let w = Arc::new(world().await);
    let kid = format!("row-11-claim-kid-{}", Uuid::now_v7());
    w.publish(with_schema(
        document(&kid, "a", vec![activity("a")]),
        json!({"type": "object"}),
    ))
    .await;
    let (parent_workflow, _) = w
        .publish(document(
            &format!("row-11-claim-{}", Uuid::now_v7()),
            "c",
            vec![child_node("c", &kid, None)],
        ))
        .await;
    let parent = w.start(parent_workflow, &json!({})).await;
    let pool = w.db.pool.clone();
    let claim: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>> = Arc::default();
    let hook_claim = Arc::clone(&claim);
    let hook: sagat_scheduler::BeforeChildCheck = Arc::new(move |_child| {
        let pool = pool.clone();
        let claim = Arc::clone(&hook_claim);
        Box::pin(async move {
            let mut held = pool.begin().await.expect("begin");
            sqlx::query("UPDATE steps SET child_started = true WHERE run_id = $1 AND node = 'c'")
                .bind(parent.as_uuid())
                .execute(&mut *held)
                .await
                .expect("the peer's claim");
            let watcher = pool.clone();
            *claim.lock().await = Some(tokio::spawn(async move {
                loop {
                    let waiting: i64 = sqlx::query_scalar(
                        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                           AND wait_event_type = 'Lock' AND (ltrim(query) LIKE 'SELECT child_started%' \
                           OR ltrim(query) LIKE 'UPDATE steps%')",
                    )
                    .fetch_one(&watcher)
                    .await
                    .expect("pg_stat_activity");
                    if waiting > 0 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
                held.commit().await.expect("the claim commits");
            }));
        })
    });
    let refusing = scheduler_with(
        &w,
        vec![Err(SchemaRefusal::Invalid {
            segments: vec![PathSegment::Property("k".into())],
            rule: Rule::Type,
        })],
    )
    .with_before_child_check(hook);
    refusing.tick().await;
    if let Some(claim) = claim.lock().await.take() {
        claim.await.expect("the claim task");
    }
    assert_eq!(
        step_code(&w, parent, "c").await,
        None,
        "the step whose child was claimed is not failed"
    );
    assert_ne!(w.step_status(parent, "c").await, Some(StepStatus::Failed));
}

/// Row 2: a start with an uploaded input is checked like any start, against the canonical descriptor
/// `{"$blob": {id, size, sha256, contentType}}` (the bytes stay opaque). A schema the descriptor does not satisfy is
/// answered `422` with the pointer and the rule, never a value of the descriptor; no run is created and the upload is not
/// claimed, so the same upload then starts a version whose schema the descriptor satisfies.
#[tokio::test]
async fn row_2_an_uploaded_input_start_checks_the_descriptor_and_a_refusal_claims_nothing() {
    use crate::support::blob_collections::{platform, start_with_blob, upload};
    use crate::support::worker::{HUMAN, NAMESPACE, TENANT, single_ir};

    let p = platform().await;
    let refusing = p
        .publish(
            "row-2-refusing",
            &with_schema(single_ir("row-2-refusing", None), order_schema()),
        )
        .await;
    let accepting = p
        .publish(
            "row-2-accepting",
            &with_schema(
                single_ir("row-2-accepting", None),
                json!({
                    "type": "object",
                    "required": ["$blob"],
                    "properties": {"$blob": {"type": "object", "required": ["id", "size", "sha256"]}}
                }),
            ),
        )
        .await;
    // T043-17, row 2 (round 2 of 2026-11-16, implementation readiness 3 and 13): the uploaded bytes carry a sentinel value
    // and key; they stay opaque, and neither reaches the answer, the logs and spans captured at TRACE or the audit rows.
    const VALUE: &str = "sentinel-043-blob-value-8a6d";
    const KEY: &str = "sentinel-043-blob-key-2c7f";
    let blob = upload(
        &p,
        format!(r#"{{"orderId": "{VALUE}", "{KEY}": "{VALUE}"}}"#).as_bytes(),
    )
    .await;
    let blob_id = blob["id"].as_str().expect("id");
    let (guard, captured) = crate::support::logs::capture_with_spans(tracing::Level::TRACE);
    let response = p
        .api
        .as_user(reqwest::Method::POST, "/runs", HUMAN, TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", format!("start-{}", Uuid::now_v7()))
        .json(&json!({"workflowId": refusing, "inputBlobRef": blob_id}))
        .send()
        .await
        .expect("start with a blob");
    let status = response.status();
    let body = crate::support::api::json_of(response).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], "VALIDATION_FAILED", "{body}");
    assert_eq!(
        body["details"],
        json!({"reason": "RUN_INPUT_INVALID", "field": "/orderId", "rule": "required"}),
    );
    let text = body.to_string();
    for value in [blob_id, blob["sha256"].as_str().expect("sha")] {
        assert!(
            !text.contains(value),
            "the refusal names no value of the descriptor"
        );
    }
    let runs_of_refusing: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE workflow_id = $1::uuid")
            .bind(&refusing)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("count");
    assert_eq!(runs_of_refusing, 0, "a refused start creates nothing");
    drop(guard);
    let audits: Vec<Option<String>> = sqlx::query_scalar("SELECT reason FROM audit_records")
        .fetch_all(&p.api.db.pool)
        .await
        .expect("audit");
    let audits = audits.into_iter().flatten().collect::<Vec<_>>().join("\n");
    let logs = captured.text();
    assert!(
        logs.contains("422") || logs.contains("RUN_INPUT_INVALID") || logs.contains("run_input"),
        "the capture saw the refused request: {logs}"
    );
    for (label, text) in [
        ("the answer", &text),
        ("the logs and spans", &logs),
        ("the audit rows", &audits),
    ] {
        assert!(
            !text.contains(VALUE),
            "{label}: the sentinel value leaked: {text}"
        );
        assert!(
            !text.contains(KEY),
            "{label}: the sentinel key leaked: {text}"
        );
    }

    let run = start_with_blob(&p, &accepting, blob_id).await;
    let input_blob: Option<Uuid> =
        sqlx::query_scalar("SELECT input_blob_id FROM runs WHERE id = $1")
            .bind(run)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("the run");
    assert_eq!(
        input_blob.map(|id| id.to_string()).as_deref(),
        Some(blob_id),
        "the upload the refusal did not claim starts the matching version"
    );
}
