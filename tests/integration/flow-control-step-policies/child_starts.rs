//! A child node whose child's start the gate refuses (task T022-102; FR-022-055; decisions D022-024 and D022-080): a full
//! buffer, a full `queued_runs`, the live-keys ceiling, a capability switched off by the deployment or not activated for
//! the fleet each fail the parent's node with the refusal's own reason, a typed failure its `onError` route takes; no
//! child run is created, and the pass that tried reports a finished child instead of failing on every tick.
//!
//! A child node has no retry policy in IR v1 (only an activity retries, `Node::retry_policy`), so its failure route is
//! what handles the refusal (decision D022-080). A `child` node inside a collection body is refused at publication on
//! this tree, so the body's case has no path to a child start yet; the test below holds that refusal.

use sagat_application::instances::{Capability, Removed};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::RunId;
use sagat_domain::identity::Subject;
use sagat_domain::quota::QuotaResource;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgInstanceStore;
use sagat_scheduler::{Scheduler, SchedulerSettings};
use serde_json::{Value, json};

use crate::run_admission::set;
use crate::support::engine::{World, activity, child_node, document, with};
use crate::support::flow_control::{gated_world, publish, switches, try_publish_as};
use crate::support::instances::nothing;

/// A parent whose entry `c` starts `target` (with the parent's input) and routes a failure to `recover`.
async fn parent(w: &World, name: &str, target: &str) -> WorkflowId {
    publish(
        w,
        document(
            name,
            "c",
            vec![
                with(child_node("c", target, None), "onError", json!("recover")),
                activity("recover"),
            ],
        ),
    )
    .await
}

/// A one-step target whose start declares `gate` (`concurrency` or `rateLimit`) on `input.accountId`.
fn target(name: &str, gate: &str) -> Value {
    let mut doc = document(name, "g", vec![activity("g")]);
    doc[gate] = if gate == "rateLimit" {
        json!({"group": "accounts", "key": "input.accountId", "limit": 1, "per": "hour"})
    } else {
        json!({"group": "accounts", "key": "input.accountId", "limit": 1})
    };
    doc
}

/// The parent's child node failed with `code` through its route, and no child run exists.
async fn refused(w: &World, parent_run: RunId, code: &str) {
    let (status, failure): (String, Option<String>) =
        sqlx::query_as("SELECT status, failure_code FROM steps WHERE run_id = $1 AND node = 'c'")
            .bind(parent_run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("the child node");
    assert_eq!(
        (status.as_str(), failure.as_deref()),
        ("FAILED", Some(code))
    );
    let recover: String =
        sqlx::query_scalar("SELECT status FROM steps WHERE run_id = $1 AND node = 'recover'")
            .bind(parent_run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("the route's step");
    assert_eq!(recover, "READY", "the failure route takes it");
    let children: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE parent_run_id = $1")
        .bind(parent_run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("children");
    assert_eq!(children, 0, "no child run is created");
    let journal = w.journal(parent_run).await;
    assert_eq!(
        journal
            .iter()
            .filter(|event| *event == "child.finished")
            .count(),
        1,
        "the reservation ends once: {journal:?}"
    );
    w.assert_consistent(parent_run).await;
}

/// Runs one pass for the parent: it tells the parent its child cannot start and fails nothing.
async fn one_pass_tells_the_parent(scheduler: &crate::support::engine::Sched) {
    let report = scheduler.tick().await;
    assert_eq!(
        (
            report.children_finished,
            report.children_started,
            report.failed
        ),
        (1, 0, 0),
        "{report:?}"
    );
}

/// FR-022-055: a child whose start meets a full buffer (`RATE_LIMIT_QUEUE_FULL` for a rate limit,
/// `CONCURRENCY_QUEUE_FULL` for a permit), a full `queued_runs` or the live-keys ceiling (`FLOW_CONTROL_LIMIT`) fails
/// the child node through its route; no child run exists, the pass fails nothing, and the next pass finds nothing left
/// to start.
#[tokio::test]
async fn a_child_refused_by_a_full_buffer_or_a_ceiling_fails_its_node_through_the_route() {
    let w = gated_world().await;
    let rated = publish(&w, target("child-full-rate", "rateLimit")).await;
    let serial = publish(&w, target("child-full-serial", "concurrency")).await;
    set(&w, QuotaResource::FlowControlKeyWaiters, 1).await;
    for workflow in [rated, serial] {
        // a holder and one waiter: the key's buffer of one is full
        w.start(workflow, &json!({"accountId": "acct-1"})).await;
        w.start(workflow, &json!({"accountId": "acct-1"})).await;
    }
    let to_rated = parent(&w, "child-full-rate-parent", "child-full-rate").await;
    let to_serial = parent(&w, "child-full-serial-parent", "child-full-serial").await;

    let run = w.start(to_rated, &json!({"accountId": "acct-1"})).await;
    one_pass_tells_the_parent(&w.scheduler).await;
    refused(&w, run, "RATE_LIMIT_QUEUE_FULL").await;

    let run = w.start(to_serial, &json!({"accountId": "acct-1"})).await;
    one_pass_tells_the_parent(&w.scheduler).await;
    refused(&w, run, "CONCURRENCY_QUEUE_FULL").await;

    // the namespace's `queued_runs` is full (two waiters): a child that would queue on a key with room in its own buffer
    set(&w, QuotaResource::FlowControlKeyWaiters, 10).await;
    set(&w, QuotaResource::QueuedRuns, 2).await;
    w.start(serial, &json!({"accountId": "acct-3"})).await;
    let run = w.start(to_serial, &json!({"accountId": "acct-3"})).await;
    one_pass_tells_the_parent(&w.scheduler).await;
    refused(&w, run, "FLOW_CONTROL_LIMIT").await;

    // the namespace's live concurrency keys are at their ceiling (acct-1 and acct-3): a child on a new key
    set(&w, QuotaResource::QueuedRuns, 1_000).await;
    set(&w, QuotaResource::ConcurrencyLiveKeys, 2).await;
    let run = w.start(to_serial, &json!({"accountId": "acct-4"})).await;
    one_pass_tells_the_parent(&w.scheduler).await;
    refused(&w, run, "FLOW_CONTROL_LIMIT").await;

    let report = w.scheduler.tick().await;
    assert_eq!(
        (
            report.children_finished,
            report.children_started,
            report.failed
        ),
        (0, 0, 0),
        "nothing is left to start: {report:?}"
    );
    w.assert_keys_consistent().await;
}

/// FR-022-055, the follow-up of decision D022-062: a running parent whose child's definition needs a capability the
/// deployment has switched off since fails the child node `CAPABILITY_DISABLED`, and one whose capability is no longer
/// activated for the fleet `FLOW_CONTROL_ACTIVATION_PENDING`, each through its route, no child run created. Before, the
/// pass failed on every tick and the node neither completed nor failed.
#[tokio::test]
async fn a_child_refused_by_its_capability_fails_its_node_through_the_route() {
    let w = gated_world().await;
    publish(&w, target("child-capability", "rateLimit")).await;
    let to_rated = parent(&w, "child-capability-parent", "child-capability").await;

    let switched_off = Scheduler::new(
        w.store.clone().with_flow_control(switches(false)),
        sagat_application::execution::Engine::new(JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
        sagat_application::run_input::InputValidationMode::Enforce,
    ))
    .with_children(w.cache.clone());
    let run = w.start(to_rated, &json!({"accountId": "acct-1"})).await;
    one_pass_tells_the_parent(&switched_off).await;
    refused(&w, run, "CAPABILITY_DISABLED").await;

    let registry = PgInstanceStore::new(w.db.pool.clone());
    let actor = Subject::parse("operator:flow-control").expect("subject");
    assert!(matches!(
        registry
            .remove(Capability::FlowControlRateV1, &actor, nothing)
            .await,
        Ok(Removed::Removed { .. })
    ));
    let run = w.start(to_rated, &json!({"accountId": "acct-1"})).await;
    one_pass_tells_the_parent(&w.scheduler).await;
    refused(&w, run, "FLOW_CONTROL_ACTIVATION_PENDING").await;
}

/// Review of 2026-11-25 (correctness minor 3; decision D022-080 as amended): a child start refused
/// `BINARY_CAPABILITY_MISSING` (this process does not serve the gate: it holds no digest key ring) is a fact of the
/// replica, not of the child: the reservation stays for a replica that serves it, the parent's node does not fail, and
/// a replica with the ring starts the child.
#[tokio::test]
async fn a_child_a_replica_cannot_serve_is_left_for_one_that_can() {
    let w = gated_world().await;
    publish(&w, target("child-unserved", "rateLimit")).await;
    let to_rated = parent(&w, "child-unserved-parent", "child-unserved").await;
    let unserved = Scheduler::new(
        sagat_infra_postgres::PgRunStore::new(w.db.pool.clone(), w.cache.clone())
            .with_flow_control(switches(true)),
        sagat_application::execution::Engine::new(JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
        sagat_application::run_input::InputValidationMode::Enforce,
    ))
    .with_children(w.cache.clone());
    let run = w.start(to_rated, &json!({"accountId": "acct-1"})).await;
    let report = unserved.tick().await;
    assert_eq!(
        (report.children_finished, report.children_started),
        (0, 0),
        "{report:?}"
    );
    let (status, failure): (String, Option<String>) =
        sqlx::query_as("SELECT status, failure_code FROM steps WHERE run_id = $1 AND node = 'c'")
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("the child node");
    assert_ne!(status, "FAILED", "{failure:?}");
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_started, 1, "{report:?}");
    let children: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE parent_run_id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("children");
    assert_eq!(children, 1);
}

/// The body's case of FR-022-055: a `child` node inside a collection body is refused at publication on this tree (the
/// body subset is activities, conditions, switches, timers and event waits), so no body instance reaches a child start.
/// When a body `child` node is enabled, its start goes through the same pass and this test fails, to be replaced by the
/// body's own case.
#[tokio::test]
async fn a_child_node_inside_a_collection_body_is_refused_at_publication() {
    let w = gated_world().await;
    publish(&w, target("child-body-target", "rateLimit")).await;
    let doc = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "child-body-parent"},
        "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": "c", "nodes": [
            {"id": "c", "type": "child", "workflowId": "child-body-target", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.c.output"}
        ]}},
        "nodes": [
            {"id": "lines", "type": "forEach", "items": "input.lines", "body": "line",
             "result": "bodyResult", "maxItems": 8, "maxConcurrency": 1, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    });
    let refusal = try_publish_as(&w, &w.admin, doc).await;
    let Err(sagat_application::error::AppError::InvalidDefinition(error)) = refusal else {
        panic!("refused as an invalid definition: {refusal:?}");
    };
    // the body that holds the child node, by name, as a body this tree does not run: a child node is not in the body
    // subset (review of 2026-11-25, correctness minor 4)
    let problems: Vec<_> = sagat_application::definitions::problems_of(&error)
        .into_iter()
        .map(|problem| (problem.code, problem.node))
        .collect();
    assert_eq!(
        problems,
        vec![("UNSUPPORTED_NODE", Some("line".to_owned()))],
        "{error:?}"
    );
    assert!(
        matches!(&*error, sagat_application::definitions::IrError::Unsupported { kind, .. } if kind == "body"),
        "{error:?}"
    );
}
