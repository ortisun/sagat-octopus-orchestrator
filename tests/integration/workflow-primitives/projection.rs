//! Journal-derived graph and telemetry projection against real PostgreSQL.

use reqwest::{Method, StatusCode};
use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_application::quotas::QuotaService;
use sagat_application::runs::RunStore;
use sagat_application::telemetry::{TelemetryStore, derive, derive_unsupported};
use sagat_application::worker::{Capabilities, WorkerService, WorkerSettings};
use sagat_domain::execution::{ExecutionInstanceId, InstancePath, RunId, SwitchSelection};
use sagat_domain::identity::{Membership, Role, Subject};
use sagat_domain::quota::QuotaResource;
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{
    Keeper, KeeperSettings, Placement, ProjectorSettings, Scheduler, SchedulerSettings,
    TelemetryProjector,
};
use sagat_telemetry::{
    AttrValue, WorkflowPrimitiveKind, WorkflowPrimitiveOutcome, WorkflowPrimitiveSpan, run_span_id,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support;
use crate::support::api::{TestApi, json_of};
use crate::support::engine::{ctx, task_ref, world};

async fn scheduler(
    w: &crate::support::engine::World,
    name: &str,
) -> Scheduler<sagat_infra_postgres::PgRunStore, JsonEvaluator, SystemEntropy> {
    let placement = Placement::new(format!("projection-scheduler-{name}"));
    Keeper::new(
        w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: std::time::Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    Scheduler::new(
        w.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_children(w.cache.clone())
    .with_placement(placement)
}

fn collection_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "items",
        "bodies": {
            "line": {
                "entrypoint": "work",
                "nodes": [
                    {
                        "id": "work",
                        "type": "activity",
                        "activity": "projection.work",
                        "next": "returned"
                    },
                    {"id": "returned", "type": "return", "result": "steps.work.output"}
                ]
            }
        },
        "nodes": [
            {
                "id": "items",
                "type": "forEach",
                "items": "input.items",
                "body": "line",
                "result": "bodyResult",
                "maxItems": 2,
                "maxConcurrency": 2,
                "next": "done"
            },
            {"id": "done", "type": "end"}
        ]
    })
}

fn continuation_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {
                "id": "work",
                "type": "activity",
                "activity": "projection.segment",
                "next": "rollover"
            },
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    })
}

fn switch_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "choose",
        "nodes": [
            {
                "id": "choose",
                "type": "switch",
                "cases": [
                    {"id": "positive", "when": "input.amount > 0", "target": "accepted"}
                ],
                "default": "rejected"
            },
            {"id": "accepted", "type": "activity", "activity": "projection.accepted"},
            {"id": "rejected", "type": "activity", "activity": "projection.rejected"}
        ]
    })
}

fn path(scope: &str, index: u32) -> InstancePath {
    InstancePath::root()
        .child(NodeId::parse(scope).expect("scope node"), index)
        .expect("instance path")
}

fn attr<'a>(span: &'a sagat_telemetry::SpanData, name: &str) -> Option<&'a AttrValue> {
    span.attributes
        .iter()
        .find_map(|(key, value)| (key == name).then_some(value))
}

#[tokio::test]
async fn scoped_graph_delta_and_telemetry_keep_the_complete_instance_identity() {
    let w = world().await;
    let name = format!("projection-scoped-{}", Uuid::now_v7());
    let (workflow, _) = w.publish(collection_document(&name)).await;
    QuotaService::set(
        &w.quotas,
        &w.admin,
        None,
        QuotaResource::IterationConcurrency,
        2,
        &ctx(),
    )
    .await
    .expect("configure collection capacity");
    let run = w
        .start(workflow, &json!({"items": [{"value": 1}, {"value": 2}]}))
        .await;
    let scheduler = scheduler(&w, &name).await;

    for expected in ["open collection", "admit first", "admit second"] {
        let report = scheduler.tick().await;
        assert_eq!(report.collections_advanced, 1, "{expected}: {report:?}");
        assert_eq!(report.failed, 0, "{expected}: {report:?}");
    }

    let scoped_store =
        PgRunStore::new(w.db.pool.clone(), w.cache.clone()).with_scoped_worker_protocol();
    let workers = WorkerService::new(
        scoped_store,
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    )
    .with_vault(w.keys.cipher());
    let registered = workers
        .register(
            &w.worker,
            "payments",
            &format!("projection-worker-{}", Uuid::now_v7()),
            &Capabilities {
                activities: vec!["projection.work".into()],
                languages: vec!["rust".into()],
                pools: Vec::new(),
                max_inflight: 2,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .expect("register scoped worker");
    assert_eq!(registered.accepted_protocol_version, "1.5");
    let token = registered.session_token.expose().to_owned();
    let mut tasks = workers
        .poll(
            &w.worker,
            registered.worker_id,
            &token,
            2,
            Some("0af7651916cd43dd8448eb211c80319c"),
        )
        .await
        .expect("poll scoped tasks");
    tasks.sort_by_key(|task| task.instance.path().segments()[0].iteration_index());
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].instance.path(), &path("items", 0));
    assert_eq!(tasks[1].instance.path(), &path("items", 1));

    let before = w
        .run_record(run)
        .await
        .expect("run before completion")
        .version;
    workers
        .complete(
            &w.worker,
            &task_ref(&tasks[0]),
            &serde_json::to_vec(&json!({"accepted": true})).expect("completion JSON"),
            &format!("projection-complete-{}", Uuid::now_v7()),
        )
        .await
        .expect("complete first scoped task");

    let changes = w
        .runs
        .graph_changes(&w.admin, run, before, &ctx())
        .await
        .expect("scoped graph delta");
    assert!(!changes.reset);
    assert!(!changes.events.is_empty());
    let touched = changes
        .events
        .iter()
        .flat_map(|event| event.touched.iter().flatten())
        .collect::<Vec<_>>();
    assert!(
        touched
            .iter()
            .any(|instance| **instance == tasks[0].instance)
    );
    assert!(
        touched
            .iter()
            .all(|instance| instance.path() != tasks[1].instance.path()),
        "a delta for the first iteration must not identify the equal local node in the second"
    );
    let graph = &changes.graph;
    for index in 0..2 {
        let instance = ExecutionInstanceId::in_path(
            path("items", index),
            NodeId::parse("work").expect("work node"),
        );
        assert!(graph.steps.iter().any(|step| step.instance == instance));
    }

    let pending = w
        .store
        .pending_runs(100)
        .await
        .expect("pending telemetry")
        .into_iter()
        .find(|pending| pending.run_id == run)
        .expect("run pending telemetry");
    let projected = w
        .store
        .load_for_projection(&pending)
        .await
        .expect("load telemetry projection")
        .expect("traceable run");
    let derived = derive(&projected.facts, &projected.entries, projected.applied_seq);
    let attempt = derived
        .spans
        .iter()
        .find(|span| {
            span.name == "step.attempt"
                && attr(span, "sagat.node") == Some(&AttrValue::from("work"))
                && attr(span, "sagat.instance.path") == Some(&AttrValue::from("items[0]"))
        })
        .expect("scoped attempt span");
    assert_eq!(
        attr(attempt, "sagat.scope.depth"),
        Some(&AttrValue::from(1_u32))
    );
}

#[tokio::test]
async fn graph_and_telemetry_project_the_same_continuation_link() {
    let w = world().await;
    let name = format!("projection-chain-{}", Uuid::now_v7());
    let (workflow, _) = w.publish(continuation_document(&name)).await;
    let predecessor = w.start(workflow, &json!({"next": {"segment": 2}})).await;
    let session = w.session(&["projection.segment"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"done": true}))
        .await
        .expect("complete predecessor work");

    let scheduler = scheduler(&w, &name).await;
    let report = scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 1, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");

    let graph = w
        .store
        .graph(w.admin.tenant_id(), predecessor)
        .await
        .expect("chain graph")
        .expect("visible predecessor");
    let chain = graph.chain.expect("chain projection");
    let successor = chain.successor_run_id.expect("successor link");
    assert_eq!(chain.chain_id, predecessor);
    assert_eq!(chain.segment_count, 2);

    let changes = w
        .runs
        .graph_changes(&w.admin, predecessor, 0, &ctx())
        .await
        .expect("chain graph changes");
    assert_eq!(
        changes
            .graph
            .chain
            .as_ref()
            .and_then(|chain| chain.successor_run_id),
        Some(successor)
    );

    let pending = w
        .store
        .pending_runs(100)
        .await
        .expect("pending telemetry")
        .into_iter()
        .find(|pending| pending.run_id == predecessor)
        .expect("predecessor pending telemetry");
    let projected = w
        .store
        .load_for_projection(&pending)
        .await
        .expect("load chain telemetry")
        .expect("traceable predecessor");
    let derived = derive(&projected.facts, &projected.entries, projected.applied_seq);
    let continuation = derived
        .spans
        .iter()
        .find(|span| {
            span.name == WorkflowPrimitiveSpan::Continue.as_str()
                && span
                    .links
                    .iter()
                    .any(|link| link.span_id == run_span_id(successor.as_uuid()))
        })
        .expect("continuation span linked to successor");
    assert_eq!(continuation.links.len(), 1);
}

#[tokio::test]
async fn selected_route_is_visible_in_the_journal_delta() {
    let w = world().await;
    let name = format!("projection-switch-{}", Uuid::now_v7());
    let (workflow, _) = w.publish(switch_document(&name)).await;
    let run = w.start(workflow, &json!({"amount": 7})).await;

    let changes = w
        .runs
        .graph_changes(&w.admin, run, 0, &ctx())
        .await
        .expect("switch graph changes");
    assert!(!changes.reset);
    let choose = changes
        .graph
        .steps
        .iter()
        .find(|step| step.node.as_str() == "choose")
        .expect("switch projection");
    assert_eq!(
        choose.switch_selection,
        Some(SwitchSelection::Case(
            NodeId::parse("positive").expect("case id")
        ))
    );
    let selection = changes
        .events
        .iter()
        .find(|event| event.event_type == "switch.evaluated")
        .expect("selection journal event");
    assert_eq!(
        selection.touched.as_deref(),
        Some(
            &[ExecutionInstanceId::root(
                NodeId::parse("choose").expect("switch node")
            )][..]
        )
    );
}

#[tokio::test]
async fn telemetry_advances_past_an_unknown_journal_event() {
    let w = world().await;
    let name = format!("projection-future-event-{}", Uuid::now_v7());
    let (workflow, _) = w.publish(switch_document(&name)).await;
    let run = w.start(workflow, &json!({"amount": 7})).await;
    let tenant = w.admin.tenant_id();
    let last: i64 =
        sqlx::query_scalar("SELECT max(seq) FROM run_events WHERE tenant_id=$1 AND run_id=$2")
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("last journal sequence");
    let future = last + 1;
    let mut tx = w.db.pool.begin().await.expect("begin future journal write");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id,run_id,seq,event_type,detail,actor,occurred_at) \
         VALUES ($1,$2,$3,'future.node.observed','{}','system:projector-test',clock_timestamp())",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(future)
    .execute(&mut *tx)
    .await
    .expect("append newer event kind");
    sqlx::query(
        "UPDATE telemetry_pending SET head_seq=$3,behind_since=coalesce(behind_since,clock_timestamp()) \
         WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(future)
    .execute(&mut *tx)
    .await
    .expect("note new journal head");
    tx.commit().await.expect("commit newer event");

    let pending = w
        .store
        .pending_runs(100)
        .await
        .expect("pending telemetry")
        .into_iter()
        .find(|pending| pending.run_id == run)
        .expect("run pending telemetry");
    let projected = w
        .store
        .load_for_projection(&pending)
        .await
        .expect("load newer journal")
        .expect("traceable run");
    assert_eq!(projected.head_seq, future as u64);
    assert!(
        projected
            .entries
            .iter()
            .all(|entry| entry.seq != future as u64)
    );
    let projector = TelemetryProjector::new(w.store.clone(), None, ProjectorSettings::default());
    let round = projector.run_once().await;
    assert!(!round.failed, "{round:?}");
    assert_eq!(round.runs, 1);
    assert_eq!(w.store.backlog().await.expect("projection backlog").runs, 0);
}

// ---------------------------------------------------------------------------------------------
// HTTP projection of approvals and scoped owners (T014-045)
// ---------------------------------------------------------------------------------------------

fn approval_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "review",
        "nodes": [
            {
                "id": "review",
                "type": "approval",
                "timeoutMs": 60000,
                "policy": {"roles": ["operator"], "excludeInitiator": true},
                "onApprove": "approved",
                "onReject": "rejected",
                "onExpire": "expired"
            },
            {"id": "approved", "type": "activity", "activity": "projection.approved"},
            {"id": "rejected", "type": "activity", "activity": "projection.rejected"},
            {"id": "expired", "type": "activity", "activity": "projection.expired"}
        ]
    })
}

fn loop_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "again",
        "bodies": {
            "iteration": {
                "entrypoint": "increment",
                "nodes": [
                    {
                        "id": "increment",
                        "type": "activity",
                        "activity": "projection.increment",
                        "next": "returned"
                    },
                    {"id": "returned", "type": "return", "result": "steps.increment.output"}
                ]
            }
        },
        "nodes": [
            {
                "id": "again",
                "type": "while",
                "condition": "carry < 2",
                "body": "iteration",
                "initial": "0",
                "carry": "bodyResult",
                "maxIterations": 3,
                "timeoutMs": 60000,
                "next": "done"
            },
            {"id": "done", "type": "end", "outputExpr": "steps.again.output"}
        ]
    })
}

struct HttpWorld {
    api: TestApi,
    tenant: sagat_domain::identity::TenantId,
    placement: Placement,
}

async fn http_world() -> HttpWorld {
    let api = support::api::start().await;
    let tenant = api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "projection").await;
    api.create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    let capacity = api
        .as_user(
            Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit": 4}))
        .send()
        .await
        .expect("configure iteration capacity");
    assert_eq!(capacity.status(), StatusCode::OK);
    let placement = Placement::new(format!("projection-http-{}", Uuid::now_v7()));
    Keeper::new(
        api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: std::time::Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    HttpWorld {
        api,
        tenant,
        placement,
    }
}

impl HttpWorld {
    fn scheduler(&self) -> Scheduler<PgRunStore, JsonEvaluator, SystemEntropy> {
        Scheduler::new(
            self.api.executions.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            SchedulerSettings {
                discovery_rest: std::time::Duration::ZERO,
                ..SchedulerSettings::default()
            },
        )
        .with_placement(self.placement.clone())
    }

    async fn publish_and_start(&self, document: &Value, input: &Value) -> RunId {
        let name = document["metadata"]["name"].as_str().expect("name");
        let created = self
            .api
            .as_user(Method::POST, "/workflows", "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"name": name, "ir": document}))
            .send()
            .await
            .expect("create workflow");
        assert_eq!(created.status(), StatusCode::CREATED);
        let workflow = json_of(created).await["id"]
            .as_str()
            .expect("workflow id")
            .to_owned();
        let published = self
            .api
            .as_user(
                Method::POST,
                &format!("/workflows/{workflow}/publish"),
                "alice",
                "acme",
            )
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"draftRevision": 1}))
            .send()
            .await
            .expect("publish workflow");
        let status = published.status();
        let body = json_of(published).await;
        assert_eq!(status, StatusCode::CREATED, "publish: {body}");
        let started = self
            .api
            .as_user(Method::POST, "/runs", "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"workflowId": workflow, "input": input}))
            .send()
            .await
            .expect("start run");
        assert_eq!(started.status(), StatusCode::ACCEPTED);
        json_of(started).await["runId"]
            .as_str()
            .expect("run id")
            .parse()
            .expect("run id")
    }

    async fn derived(&self, run: RunId) -> sagat_application::telemetry::Derived {
        let pending = self
            .api
            .executions
            .pending_runs(100)
            .await
            .expect("pending telemetry")
            .into_iter()
            .find(|pending| pending.run_id == run)
            .expect("run pending telemetry");
        let projected = self
            .api
            .executions
            .load_for_projection(&pending)
            .await
            .expect("load telemetry projection")
            .expect("traceable run");
        derive(&projected.facts, &projected.entries, projected.applied_seq)
    }

    async fn graph(&self, run: RunId) -> Value {
        let response = self
            .api
            .as_user(Method::GET, &format!("/runs/{run}/graph"), "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .send()
            .await
            .expect("read graph");
        assert_eq!(response.status(), StatusCode::OK);
        json_of(response).await
    }

    async fn changes(&self, run: RunId, since: u64) -> Value {
        let response = self
            .api
            .as_user(
                Method::GET,
                &format!("/runs/{run}/graph/changes?since={since}"),
                "alice",
                "acme",
            )
            .header("x-octopus-namespace", "payments")
            .send()
            .await
            .expect("read graph changes");
        assert_eq!(response.status(), StatusCode::OK);
        json_of(response).await
    }
}

fn edge<'a>(graph: &'a Value, from: &str, to: &str, kind: &str) -> Option<&'a Value> {
    graph["edges"]
        .as_array()
        .expect("edges")
        .iter()
        .find(|edge| edge["from"] == from && edge["to"] == to && edge["kind"] == kind)
}

fn node<'a>(graph: &'a Value, id: &str) -> &'a Value {
    graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|node| node["id"] == id)
        .unwrap_or_else(|| panic!("node {id} missing from {graph}"))
}

#[tokio::test]
async fn approval_routes_and_the_recorded_decision_are_projected_over_http() {
    let world = http_world().await;
    let tenant = world.tenant;
    world
        .api
        .owner_store
        .insert_membership(&Membership::restore(
            tenant,
            Subject::parse("olga").expect("approver"),
            Role::Operator,
            support::now(),
        ))
        .await
        .expect("operator membership");
    let name = format!("projection-approval-{}", Uuid::now_v7());
    let run = world
        .publish_and_start(&approval_document(&name), &json!({}))
        .await;

    // Before the scheduler opens it the routes are already known from the pinned definition.
    let before = world.graph(run).await;
    for (target, kind) in [
        ("approved", "approve"),
        ("rejected", "reject"),
        ("expired", "expire"),
    ] {
        assert!(
            edge(&before, "review", target, kind).is_some(),
            "approval route {kind} -> {target} missing: {before}"
        );
    }
    assert_eq!(node(&before, "review")["approvalDecision"], Value::Null);

    let opened = world.scheduler().tick().await;
    assert_eq!(opened.approvals_opened, 1, "{opened:?}");
    let pending = world.graph(run).await;
    assert_eq!(
        node(&pending, "review")["approvalDecision"],
        json!({"kind": "PENDING", "target": null}),
        "{pending}"
    );
    assert_eq!(node(&pending, "review")["waitingOn"], "approval");
    let seen = pending["projectionSeq"].as_u64().expect("projection seq");

    let request: Uuid = sqlx::query_scalar(
        "SELECT request_id FROM approval_requests WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&world.api.db.pool)
    .await
    .expect("opened approval request");
    let (_guard, logs) = support::logs::capture();
    let decided = world
        .api
        .post_as(
            &format!("/approvals/{request}/decisions"),
            "olga",
            "acme",
            &json!({"decisionKey": "projection-approve", "decision": "REJECT"}),
        )
        .await;
    let decided_status = decided.status();
    assert_eq!(
        decided_status,
        StatusCode::OK,
        "{} {}",
        json_of(decided).await,
        logs.text()
    );

    let after = world.graph(run).await;
    assert_eq!(
        node(&after, "review")["approvalDecision"],
        json!({"kind": "REJECTED", "target": "rejected"}),
        "{after}"
    );
    let serialized = after.to_string();
    assert!(
        !serialized.contains("olga") && !serialized.contains("projection-approve"),
        "the graph must not disclose the approver or the decision key: {after}"
    );

    let delta = world.changes(run, seen).await;
    assert_eq!(delta["reset"], false, "{delta}");
    assert_eq!(
        node(&delta, "review")["approvalDecision"],
        json!({"kind": "REJECTED", "target": "rejected"}),
        "{delta}"
    );

    // The same committed events project into the telemetry, without the approver.
    let derived = world.derived(run).await;
    let decide = derived
        .spans
        .iter()
        .find(|span| span.name == WorkflowPrimitiveSpan::ApprovalDecide.as_str())
        .expect("approval decision span");
    assert_eq!(
        attr(decide, "sagat.outcome"),
        Some(&AttrValue::from(
            WorkflowPrimitiveOutcome::Rejected.as_str()
        ))
    );
    assert_eq!(attr(decide, "sagat.node"), Some(&AttrValue::from("review")));
    assert!(
        !format!("{decide:?}").contains("olga"),
        "the approver must not reach a span: {decide:?}"
    );
}

#[tokio::test]
async fn a_loop_owner_points_at_the_entry_of_each_admitted_iteration() {
    let world = http_world().await;
    let name = format!("projection-loop-{}", Uuid::now_v7());
    let run = world
        .publish_and_start(&loop_document(&name), &json!({}))
        .await;
    let entered = world.scheduler().tick().await;
    assert_eq!(entered.loops_advanced, 1, "{entered:?}");
    assert_eq!(entered.failed, 0, "{entered:?}");

    let graph = world.graph(run).await;
    let entry = node(&graph, "again[0]/increment");
    assert_eq!(entry["nodeId"], "increment");
    assert_eq!(
        entry["instancePath"],
        json!([{"scopeNodeId": "again", "iterationIndex": 0}])
    );
    assert!(
        edge(&graph, "again", "again[0]/increment", "begin").is_some(),
        "loop owner -> first body step missing: {graph}"
    );

    // The loop's scope is projected into the telemetry with its closed kind label.
    let derived = world.derived(run).await;
    assert!(
        derived.observations.iter().any(|observation| matches!(
            observation,
            sagat_application::telemetry::Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Loop,
                outcome: WorkflowPrimitiveOutcome::Entered | WorkflowPrimitiveOutcome::Admitted,
            }
        )),
        "loop entry missing from {:?}",
        derived.observations
    );
}

#[tokio::test]
async fn a_collection_owner_points_at_the_entry_of_each_admitted_iteration() {
    let world = http_world().await;
    let name = format!("projection-collection-{}", Uuid::now_v7());
    let run = world
        .publish_and_start(
            &collection_document(&name),
            &json!({"items": [{"value": 1}, {"value": 2}]}),
        )
        .await;
    for expected in ["open collection", "admit first", "admit second"] {
        let report = world.scheduler().tick().await;
        assert_eq!(report.collections_advanced, 1, "{expected}: {report:?}");
    }
    let graph = world.graph(run).await;
    for index in 0..2 {
        let entry = format!("items[{index}]/work");
        assert!(
            edge(&graph, "items", &entry, "begin").is_some(),
            "collection owner -> {entry} missing: {graph}"
        );
    }
}

#[tokio::test]
async fn an_event_of_an_unknown_kind_is_projected_as_an_explicit_unsupported_span() {
    let w = world().await;
    let name = format!("projection-unsupported-{}", Uuid::now_v7());
    let (workflow, _) = w.publish(switch_document(&name)).await;
    let run = w.start(workflow, &json!({"amount": 7})).await;
    let tenant = w.admin.tenant_id();
    let last: i64 =
        sqlx::query_scalar("SELECT max(seq) FROM run_events WHERE tenant_id=$1 AND run_id=$2")
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("last journal sequence");
    let future = last + 1;
    let mut tx = w.db.pool.begin().await.expect("begin future journal write");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id,run_id,seq,event_type,node,detail,actor,occurred_at) \
         VALUES ($1,$2,$3,'future.node.observed','choose','{\"secret\":\"canary-value\"}', \
                 'system:projector-test',clock_timestamp())",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(future)
    .execute(&mut *tx)
    .await
    .expect("append newer event kind");
    sqlx::query(
        "UPDATE telemetry_pending SET head_seq=$3,behind_since=coalesce(behind_since,clock_timestamp()) \
         WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(future)
    .execute(&mut *tx)
    .await
    .expect("note new journal head");
    tx.commit().await.expect("commit newer event");

    let pending = w
        .store
        .pending_runs(100)
        .await
        .expect("pending telemetry")
        .into_iter()
        .find(|pending| pending.run_id == run)
        .expect("run pending telemetry");
    let projected = w
        .store
        .load_for_projection(&pending)
        .await
        .expect("load newer journal")
        .expect("traceable run");
    assert_eq!(projected.unsupported.len(), 1);
    assert_eq!(projected.unsupported[0].seq, future as u64);

    let derived = derive_unsupported(
        &projected.facts,
        &projected.unsupported,
        projected.applied_seq,
    );
    assert_eq!(derived.spans.len(), 1);
    let span = &derived.spans[0];
    assert_eq!(span.name, WorkflowPrimitiveSpan::Unsupported.as_str());
    assert_eq!(
        attr(span, "sagat.primitive.kind"),
        Some(&AttrValue::from(
            WorkflowPrimitiveKind::Unsupported.as_str()
        ))
    );
    assert_eq!(
        attr(span, "sagat.outcome"),
        Some(&AttrValue::from(
            WorkflowPrimitiveOutcome::Unsupported.as_str()
        ))
    );
    let rendered = format!("{span:?}");
    assert!(
        !rendered.contains("canary-value") && !rendered.contains("future.node.observed"),
        "an unsupported span must not copy the unknown event's type or payload: {rendered}"
    );
    assert_eq!(
        derived.observations,
        vec![
            sagat_application::telemetry::Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Unsupported,
                outcome: WorkflowPrimitiveOutcome::Unsupported,
            }
        ]
    );

    // Entries already taken are never reported twice.
    assert!(
        derive_unsupported(&projected.facts, &projected.unsupported, future as u64)
            .spans
            .is_empty()
    );

    let metrics = sagat_telemetry::Metrics::global();
    let projector = TelemetryProjector::new(w.store.clone(), None, ProjectorSettings::default());
    let round = projector.run_once().await;
    assert!(!round.failed, "{round:?}");
    assert!(metrics.render().contains(
        "workflow_primitive_transitions_total{kind=\"unsupported\",outcome=\"unsupported\"}"
    ));
    assert_eq!(w.store.backlog().await.expect("projection backlog").runs, 0);
}
