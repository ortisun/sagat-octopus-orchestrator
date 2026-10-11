//! Child runs (FR-002-009): a `child` step reserves a run, the scheduler creates it, the parent
//! waits without holding a worker, and the child's end — or the parent's stop — is carried across
//! under the policy the node declares.

use sagat_application::execution::{MAX_CHILD_DEPTH, child_run_id};
use sagat_application::runs::RunOperation;
use sagat_domain::execution::{RunId, RunStatus, StepStatus};
use sagat_domain::workflow::CancelPolicy;
use serde_json::{Value, json};

use crate::support::engine::{
    World, activity, chain, child_node, document, parallel_node, with, world,
};

/// A workflow with one activity, `act.kid`, to be used as a child.
fn kid(name: &str) -> Value {
    document(
        name,
        "kid",
        vec![json!({ "id": "kid", "type": "activity", "activity": "act.kid" })],
    )
}

/// `c` (a child node) followed by the activity `after`.
fn parent(name: &str, node: Value) -> Value {
    document(
        name,
        "c",
        vec![with(node, "next", json!("after")), activity("after")],
    )
}

fn child_of(parent: RunId, node: &str) -> RunId {
    child_run_id(
        parent,
        &sagat_domain::workflow::NodeId::parse(node).expect("node"),
    )
}

async fn run_status(w: &World, run: RunId) -> RunStatus {
    w.run_status(run).await
}

/// Starts the parent and lets the scheduler create the child; returns both ids.
async fn started(w: &World, node: Value) -> (RunId, RunId) {
    w.publish(kid("kid-flow")).await;
    let (parent_wf, _) = w.publish(parent("mother-flow", node)).await;
    let run = w.start(parent_wf, &json!({"order": {"id": 7}})).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_started, 1, "{report:?}");
    (run, child_of(run, "c"))
}

#[tokio::test]
async fn a_child_is_reserved_created_run_and_awaited_without_a_worker() {
    let w = world().await;
    let (kid_wf, _) = w.publish(kid("kid-flow")).await;
    let (parent_wf, _) = w
        .publish(parent("mother-flow", child_node("c", "kid-flow", None)))
        .await;
    let run = w.start(parent_wf, &json!({"order": {"id": 7}})).await;
    let child = child_of(run, "c");

    // The reservation is journaled with the parent; the child does not exist yet.
    assert_eq!(w.run_status(run).await, RunStatus::Waiting);
    assert_eq!(w.step_status(run, "c").await, Some(StepStatus::Waiting));
    assert!(w.journal(run).await.contains(&"child.started".to_owned()));
    assert!(w.run_record(child).await.is_none());

    // One scheduler tick creates and starts it.
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_started, 1);
    let record = w.run_record(child).await.expect("the child exists");
    assert_eq!(record.status, RunStatus::Running);
    assert_eq!(record.info.depth, 1);
    let link = record.info.parent.expect("a parent link");
    assert_eq!((link.run, link.node.as_str()), (run, "c"));
    assert_eq!(link.cancel_policy, CancelPolicy::Propagate);
    assert_eq!(record.info.started_by, "system:engine");
    assert_eq!(
        record.info.workflow_id, kid_wf,
        "the active version of the named workflow"
    );
    assert!(
        w.journal(child)
            .await
            .starts_with(&["run.started".to_owned()])
    );
    assert_eq!(w.scheduler.tick().await.children_started, 0, "created once");

    // A worker runs the child's activity; the parent is told on the next tick.
    let session = w.session(&["act.kid", "act.after"]).await;
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, child);
    w.complete(&session, &task, &json!({"charged": true}))
        .await
        .expect("child step");
    assert_eq!(run_status(&w, child).await, RunStatus::Succeeded);
    assert_eq!(w.run_status(run).await, RunStatus::Waiting, "not told yet");
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_finished, 1);
    assert_eq!(w.step_status(run, "c").await, Some(StepStatus::Succeeded));

    let after = w.one_task(&session).await;
    assert_eq!(after.run_id, run);
    w.complete(&session, &after, &json!({}))
        .await
        .expect("after");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    let journal = w.journal(run).await;
    assert!(
        journal.contains(&"child.finished".to_owned()),
        "{journal:?}"
    );
    w.assert_consistent(run).await;
    w.assert_consistent(child).await;
    let quiet = w.scheduler.tick().await;
    assert_eq!(quiet.children_finished, 0, "told once");
    assert_eq!(
        (quiet.skipped, quiet.failed),
        (0, 0),
        "and not offered again to a parent that already knows: {quiet:?}"
    );
}

#[tokio::test]
async fn the_child_starts_with_the_input_the_node_evaluates() {
    let w = world().await;
    w.publish(kid("kid-flow")).await;
    let node = with(
        child_node("c", "kid-flow", None),
        "input",
        json!("input.order"),
    );
    let (parent_wf, _) = w.publish(parent("mother-flow", node)).await;
    w.start(parent_wf, &json!({"order": {"id": 7}, "secret": "s"}))
        .await;
    w.scheduler.tick().await;
    let session = w.session(&["act.kid"]).await;
    let task = w.one_task(&session).await;
    let input: Value = serde_json::from_slice(&task.input).expect("json");
    assert_eq!(input["input"], json!({"id": 7}), "{input}");
    assert!(
        !input.to_string().contains("secret"),
        "only what the expression selected reaches the child"
    );

    // Without an expression the child gets the parent's input.
    let w = world().await;
    w.publish(kid("kid-flow")).await;
    let (parent_wf, _) = w
        .publish(parent("mother-flow", child_node("c", "kid-flow", None)))
        .await;
    w.start(parent_wf, &json!({"order": {"id": 9}})).await;
    w.scheduler.tick().await;
    let session = w.session(&["act.kid"]).await;
    let task = w.one_task(&session).await;
    let input: Value = serde_json::from_slice(&task.input).expect("json");
    assert_eq!(input["input"], json!({"order": {"id": 9}}));
}

#[tokio::test]
async fn an_input_that_is_not_an_object_fails_the_step_before_any_child_exists() {
    let w = world().await;
    w.publish(kid("kid-flow")).await;
    let node = with(
        child_node("c", "kid-flow", None),
        "input",
        json!("input.order.id"),
    );
    let (parent_wf, _) = w.publish(parent("mother-flow", node)).await;
    let run = w.start(parent_wf, &json!({"order": {"id": 7}})).await;
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    assert_eq!(w.step_status(run, "c").await, Some(StepStatus::Failed));
    assert_eq!(w.scheduler.tick().await.children_started, 0);
    assert!(w.run_record(child_of(run, "c")).await.is_none());
}

#[tokio::test]
async fn a_pinned_version_is_used_and_a_missing_workflow_or_version_fails_the_step() {
    let w = world().await;
    let (kid_wf, v1) = w.publish(kid("kid-flow")).await;
    let v2 = w
        .publish_new_version(
            kid_wf,
            document(
                "kid-flow",
                "kid",
                vec![json!({ "id": "kid", "type": "activity", "activity": "act.kid2" })],
            ),
        )
        .await;
    assert_ne!(v1, v2);

    // Pinned to version 1 while 2 is active.
    let pinned = with(child_node("c", "kid-flow", None), "version", json!(1));
    let (wf, _) = w.publish(parent("pinned-flow", pinned)).await;
    let run = w.start(wf, &json!({})).await;
    w.scheduler.tick().await;
    let record = w.run_record(child_of(run, "c")).await.expect("child");
    assert_eq!(record.info.version_id, v1);

    // Unpinned: the active version.
    let (wf, _) = w
        .publish(parent("active-flow", child_node("c", "kid-flow", None)))
        .await;
    let run = w.start(wf, &json!({})).await;
    w.scheduler.tick().await;
    assert_eq!(
        w.run_record(child_of(run, "c"))
            .await
            .expect("child")
            .info
            .version_id,
        v2
    );

    // A version that was never published, a workflow that does not exist, and one that has only a
    // draft: the parent step fails at once instead of waiting forever.
    w.create_draft(kid("draft-only")).await;
    for (label, node) in [
        (
            "unknown version",
            with(child_node("c", "kid-flow", None), "version", json!(9)),
        ),
        ("unknown workflow", child_node("c", "no-such-flow", None)),
        ("never published", child_node("c", "draft-only", None)),
    ] {
        let (wf, _) = w
            .publish(parent(
                &format!("missing-{}", label.replace(' ', "-")),
                node,
            ))
            .await;
        let run = w.start(wf, &json!({})).await;
        w.scheduler.tick().await;
        assert_eq!(w.run_status(run).await, RunStatus::Failed, "{label}");
        assert_eq!(
            w.step_status(run, "c").await,
            Some(StepStatus::Failed),
            "{label}"
        );
        assert!(w.run_record(child_of(run, "c")).await.is_none(), "{label}");
        let code: Option<String> =
            sqlx::query_scalar("SELECT failure_code FROM steps WHERE run_id = $1 AND node = 'c'")
                .bind(run.as_uuid())
                .fetch_one(&w.db.pool)
                .await
                .expect("code");
        assert_eq!(code.as_deref(), Some("CHILD_NOT_FOUND"), "{label}");
    }
}

#[tokio::test]
async fn a_workflow_of_another_tenant_is_never_a_child() {
    let w = world().await;
    // The other tenant has a workflow with exactly the name the parent asks for.
    w.publish_as_other_tenant(kid("kid-flow")).await;
    let (wf, _) = w
        .publish(parent("mother-flow", child_node("c", "kid-flow", None)))
        .await;
    let run = w.start(wf, &json!({})).await;
    w.scheduler.tick().await;
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE parent_run_id IS NOT NULL")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(
        runs, 0,
        "no child was created from the other tenant's workflow"
    );
}

#[tokio::test]
async fn a_failed_child_takes_the_error_route_or_fails_the_parent_with_its_code() {
    let w = world().await;
    w.publish(kid("kid-flow")).await;
    // The child's only step fails for good.
    let routed = with(
        child_node("c", "kid-flow", Some("after")),
        "onError",
        json!("recover"),
    );
    let (wf, _) = w
        .publish(document(
            "mother-flow",
            "c",
            vec![routed, activity("after"), activity("recover")],
        ))
        .await;
    let run = w.start(wf, &json!({})).await;
    w.scheduler.tick().await;
    let session = w.session(&["act.kid", "act.recover", "act.after"]).await;
    let task = w.one_task(&session).await;
    w.fail(&session, &task, false)
        .await
        .expect("child step fails");
    assert_eq!(w.run_status(child_of(run, "c")).await, RunStatus::Failed);
    w.scheduler.tick().await;
    assert_eq!(w.step_status(run, "c").await, Some(StepStatus::Failed));
    let next = w.one_task(&session).await;
    assert_eq!(next.activity.as_str(), "act.recover", "the error route ran");

    // No error route: the parent run fails with the child's code.
    let w = world().await;
    w.publish(kid("kid-flow")).await;
    let (wf, _) = w
        .publish(parent("mother-flow", child_node("c", "kid-flow", None)))
        .await;
    let run = w.start(wf, &json!({})).await;
    w.scheduler.tick().await;
    let session = w.session(&["act.kid"]).await;
    let task = w.one_task(&session).await;
    w.fail(&session, &task, false).await.expect("fails");
    w.scheduler.tick().await;
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    let code: Option<String> = sqlx::query_scalar("SELECT stop_code FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("code");
    assert_eq!(code.as_deref(), Some("CHILD_FAILED"));
}

#[tokio::test]
async fn a_child_cancelled_on_its_own_fails_the_parent_step_as_cancelled() {
    let w = world().await;
    let (run, child) = started(&w, child_node("c", "kid-flow", None)).await;
    w.operate_run(child, RunOperation::Cancel).await;
    assert_eq!(run_status(&w, child).await, RunStatus::Cancelled);
    w.scheduler.tick().await;
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    let code: Option<String> = sqlx::query_scalar("SELECT stop_code FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("code");
    assert_eq!(code.as_deref(), Some("CHILD_CANCELLED"));
}

// ------------------------------------------------------------------ cancellation policies

#[tokio::test]
async fn cancelling_the_parent_cancels_a_propagating_child_and_the_parent_does_not_wait() {
    let w = world().await;
    let (run, child) = started(&w, child_node("c", "kid-flow", None)).await;
    w.operate(run, RunOperation::Cancel).await.expect("cancel");
    assert_eq!(
        w.run_status(run).await,
        RunStatus::Cancelled,
        "the parent is done at once"
    );
    assert_eq!(run_status(&w, child).await, RunStatus::Running, "not yet");
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_cancelled, 1);
    assert_eq!(run_status(&w, child).await, RunStatus::Cancelled);
    let quiet = w.scheduler.tick().await;
    assert_eq!(quiet.children_cancelled, 0, "asked once");
    assert_eq!(
        (quiet.skipped, quiet.failed),
        (0, 0),
        "a cancelled child is not reported to a parent that stopped waiting: {quiet:?}"
    );
    w.assert_consistent(run).await;
    w.assert_consistent(child).await;
}

#[tokio::test]
async fn a_detached_child_outlives_a_cancelled_parent_and_nobody_is_told() {
    let w = world().await;
    let node = with(
        child_node("c", "kid-flow", None),
        "cancelPolicy",
        json!("detach"),
    );
    let (run, child) = started(&w, node).await;
    w.operate(run, RunOperation::Cancel).await.expect("cancel");
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
    assert_eq!(w.scheduler.tick().await.children_cancelled, 0);
    assert_eq!(run_status(&w, child).await, RunStatus::Running);

    let session = w.session(&["act.kid"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("child finishes");
    assert_eq!(run_status(&w, child).await, RunStatus::Succeeded);
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_finished, 0, "the parent no longer waits");
    assert!(
        !report.did_work(),
        "and nothing is retried forever: {report:?}"
    );
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
}

#[tokio::test]
async fn request_cancel_makes_the_parent_wait_until_the_child_has_settled() {
    let w = world().await;
    let node = with(
        child_node("c", "kid-flow", None),
        "cancelPolicy",
        json!("request_cancel"),
    );
    let (run, child) = started(&w, node).await;
    w.operate(run, RunOperation::Cancel).await.expect("cancel");
    assert_eq!(
        w.run_status(run).await,
        RunStatus::Cancelling,
        "the parent waits for its child"
    );
    assert_eq!(w.step_status(run, "c").await, Some(StepStatus::Waiting));

    // The child is cancelled (its step is not leased, so it ends at once); the parent learns it.
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_cancelled, 1);
    assert_eq!(run_status(&w, child).await, RunStatus::Cancelled);
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_finished, 1);
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
    w.assert_consistent(run).await;
}

// ------------------------------------------------------------------ races and bounds

#[tokio::test]
async fn a_child_is_created_once_when_schedulers_race() {
    let w = world().await;
    w.publish(kid("kid-flow")).await;
    let (wf, _) = w
        .publish(parent("mother-flow", child_node("c", "kid-flow", None)))
        .await;
    let mut parents = Vec::new();
    for _ in 0..6 {
        parents.push(w.start(wf, &json!({})).await);
    }
    // Six schedulers tick at the same instant over the same six reservations.
    let reports = futures_util::future::join_all((0..6).map(|_| w.scheduler.tick())).await;
    let started: u32 = reports.iter().map(|r| r.children_started).sum();
    assert_eq!(started, 6, "every child created exactly once: {reports:?}");
    let children: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE parent_run_id IS NOT NULL")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(children, 6);
    for parent in parents {
        w.assert_consistent(child_of(parent, "c")).await;
        w.assert_consistent(parent).await;
    }
}

#[tokio::test]
async fn no_child_is_created_for_a_step_that_was_cancelled_before_the_scheduler_got_to_it() {
    let w = world().await;
    w.publish(kid("kid-flow")).await;
    let (wf, _) = w
        .publish(parent("mother-flow", child_node("c", "kid-flow", None)))
        .await;
    let run = w.start(wf, &json!({})).await;
    w.operate(run, RunOperation::Cancel).await.expect("cancel");
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_started, 0, "{report:?}");
    assert!(w.run_record(child_of(run, "c")).await.is_none());
}

#[tokio::test]
async fn a_recursive_workflow_ends_at_the_depth_limit_with_a_failure_that_unwinds() {
    let w = world().await;
    // `loop-flow` starts itself, with nothing to stop the recursion.
    let (wf, _) = w
        .publish(document(
            "loop-flow",
            "again",
            vec![child_node("again", "loop-flow", None)],
        ))
        .await;
    let root = w.start(wf, &json!({})).await;
    for _ in 0..60 {
        w.scheduler.tick().await;
    }
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(
        total,
        i64::from(MAX_CHILD_DEPTH) + 1,
        "a bounded tree, not a runaway"
    );
    let unfinished: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE finished_at IS NULL")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(unfinished, 0, "the failure unwound all the way up");
    assert_eq!(w.run_status(root).await, RunStatus::Failed);
    let deepest: Option<String> = sqlx::query_scalar("SELECT stop_code FROM runs WHERE depth = $1")
        .bind(i16::from(MAX_CHILD_DEPTH))
        .fetch_one(&w.db.pool)
        .await
        .expect("deepest");
    assert_eq!(deepest.as_deref(), Some("CHILD_DEPTH_EXCEEDED"));
}

#[tokio::test]
async fn a_child_never_outlives_its_parents_deadline() {
    let w = world().await;
    w.publish(kid("kid-flow")).await;
    let node = with(
        child_node("c", "kid-flow", None),
        "timeoutMs",
        json!(3_600_000),
    );
    let (wf, _) = w.publish(parent("mother-flow", node)).await;
    let deadline = time::OffsetDateTime::now_utc() + time::Duration::minutes(10);
    let run = w
        .start_with(wf, &json!({}), "k-deadline-12345", Some(deadline))
        .await
        .expect("start");
    w.scheduler.tick().await;
    let child = w.run_record(child_of(run, "c")).await.expect("child");
    let child_deadline = child.info.deadline_at.expect("a deadline");
    let parent_deadline = w
        .run_record(run)
        .await
        .expect("run")
        .info
        .deadline_at
        .expect("deadline");
    assert!(
        child_deadline <= parent_deadline,
        "the node's one-hour timeout cannot extend the parent's ten minutes"
    );

    // The parent's deadline passes: the parent times out and its child is cancelled with it.
    w.age_run_deadline(run).await;
    assert_eq!(w.scheduler.tick().await.deadlines_enforced, 1);
    assert_eq!(w.run_status(run).await, RunStatus::TimedOut);
    w.scheduler.tick().await;
    assert_eq!(
        run_status(&w, child_of(run, "c")).await,
        RunStatus::Cancelled
    );
}

#[tokio::test]
async fn runs_without_child_nodes_cause_no_child_work() {
    let w = world().await;
    let (wf, _) = w.publish(chain("plain")).await;
    w.start(wf, &json!({})).await;
    let report = w.scheduler.tick().await;
    assert_eq!(
        (
            report.children_started,
            report.children_finished,
            report.children_cancelled
        ),
        (0, 0, 0)
    );
}

#[tokio::test]
async fn a_child_cannot_be_created_for_a_step_that_stopped_waiting_after_the_candidate_was_read() {
    use sagat_application::definitions::{Applied, Idempotency, IrCodec};
    use sagat_application::execution::{ParentLink, RunInfo};
    use sagat_application::maintenance::MaintenanceStore;
    use sagat_application::runs::{NewRun, RunStore};
    use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};

    let w = world().await;
    let (kid_wf, kid_version) = w.publish(kid("kid-flow")).await;
    let (wf, _) = w
        .publish(parent("mother-flow", child_node("c", "kid-flow", None)))
        .await;
    let run = w.start(wf, &json!({})).await;

    // The scheduler reads the candidate…
    let candidate = w
        .store
        .pending_children(10)
        .await
        .expect("candidates")
        .pop()
        .expect("one reservation");
    // …the parent is cancelled before it gets to create the child…
    w.operate(run, RunOperation::Cancel).await.expect("cancel");
    assert_eq!(w.step_status(run, "c").await, Some(StepStatus::Cancelled));

    // …and creating the child now must be refused by the database transaction itself.
    let record = w.run_record(run).await.expect("parent");
    let version = sagat_application::definitions::DefinitionStore::get_version_by_id(
        &w.store,
        w.admin.tenant_id(),
        kid_version,
    )
    .await
    .expect("version")
    .expect("exists");
    let definition = std::sync::Arc::new(
        sagat_infra_ir::JsonIrCodec
            .parse(&version.document)
            .expect("parses")
            .definition,
    );
    let child = candidate.child;
    let new = NewRun {
        limits: sagat_application::quotas::RunLimits::UNLIMITED,
        info: RunInfo {
            run_id: child,
            tenant_id: w.admin.tenant_id(),
            namespace_id: record.info.namespace_id,
            workflow_id: kid_wf,
            version_id: kid_version,
            started_by: "system:engine".into(),
            correlation_id: None,
            trace_id: None,
            parent_span: None,
            rerun_of: None,
            deadline_at: None,
            created_at: candidate.as_of,
            parent: Some(ParentLink {
                run,
                node: candidate.step.node.clone(),
                cancel_policy: CancelPolicy::Propagate,
            }),
            depth: 1,
            root: run,
        },
        definition,
        input: candidate.input.clone(),
        audit: AuditRecord::engine(
            w.admin.tenant_id(),
            AuditAction::RunStarted,
            ResourceType::Run,
            child.to_string(),
            "race",
            None,
            candidate.as_of,
        ),
        idempotency: Idempotency {
            scope: format!("child:{run}"),
            key: "child-start:c".into(),
            fingerprint: child.as_uuid().as_bytes().to_vec(),
        },
    };
    let refused = w.store.create(new).await;
    assert!(
        matches!(
            refused,
            Err(sagat_application::error::StoreError::MissingReference)
        ),
        "{:?}",
        refused.map(|applied| matches!(applied, Applied::Done(_)))
    );
    assert!(
        w.run_record(child).await.is_none(),
        "no orphan child exists"
    );
}

#[tokio::test]
async fn a_fan_out_bomb_stops_at_the_tree_budget_and_fails_instead_of_exhausting_the_platform() {
    use sagat_application::execution::MAX_TREE_RUNS;

    let w = world().await;
    // `bomb-flow` starts itself eight times in parallel: depth 8 alone would allow 8^8 runs.
    let branches: Vec<String> = (0..8).map(|i| format!("c{i}")).collect();
    let names: Vec<&str> = branches.iter().map(String::as_str).collect();
    let mut nodes = vec![parallel_node("p", &names, "all")];
    nodes.extend(names.iter().map(|b| child_node(b, "bomb-flow", None)));
    let (wf, _) = w.publish(document("bomb-flow", "p", nodes)).await;
    let root = w.start(wf, &json!({})).await;

    // Ticks until the tree has settled. A failed action is swallowed (and logged) by the scheduler,
    // so the test counts them: refusing at the budget must be a clean outcome, never an error.
    let mut failed = 0;
    for _ in 0..400 {
        let report = w.scheduler.tick().await;
        failed += report.failed;
        if !report.did_work() {
            break;
        }
    }
    assert_eq!(failed, 0, "no scheduler action failed on the way");
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(
        total,
        i64::try_from(MAX_TREE_RUNS).expect("fits"),
        "the tree grew to its budget and not one run further"
    );
    let unfinished: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE finished_at IS NULL")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(unfinished, 0, "and every run ended: nothing waits forever");
    assert_eq!(w.run_status(root).await, RunStatus::Failed);
    let refused: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE failure_code = 'CHILD_LIMIT_EXCEEDED'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert!(
        refused > 0,
        "the budget, not only the depth limit, ended the tree"
    );
}
