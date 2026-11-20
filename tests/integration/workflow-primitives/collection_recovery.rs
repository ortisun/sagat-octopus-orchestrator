//! Local error routes in a bounded collection body preserve sibling progress and saga drain.
use crate::support::scope_flow::{
    Flow, activity, collection_document, iteration, node, retry, summary,
};
use serde_json::{Value, json};

fn document(name: &str) -> Value {
    let mut value = collection_document(
        name,
        vec![
            activity(
                "reserve",
                "line.reserve",
                "charge",
                json!({"compensateWith":"line.release", "compensationRetry":retry()}),
            ),
            activity(
                "charge",
                "line.charge",
                "returned",
                json!({"onError":"recover"}),
            ),
            activity("recover", "line.notify", "returned", json!({})),
            json!({"id":"returned", "type":"return", "result":"item"}),
        ],
        2,
    );
    value["limits"] = json!({"maxInstances":12,"maxDepth":1});
    value
}

async fn at_recovery(name: &str) -> Flow {
    let flow = Flow::start(name, document(name), json!([1, 2])).await;
    flow.drive().await;
    let reserves = flow.poll().await;
    assert_eq!(reserves.len(), 2);
    for task in &reserves {
        assert_eq!(node(task), "reserve");
        flow.complete(task, json!({"reserved":iteration(task)}))
            .await;
    }
    flow.drive().await;
    let charges = flow.poll().await;
    assert_eq!(charges.len(), 2);
    assert_eq!(iteration(&charges[0]), 0);
    assert_eq!(iteration(&charges[1]), 1);
    assert!(charges.iter().all(|task| node(task) == "charge"));
    flow.complete(&charges[0], json!({"charged":true})).await;
    flow.fail(&charges[1], "PAYMENT_DECLINED", false).await;
    flow.drive().await;
    flow.verify().await;
    flow
}

#[tokio::test]
async fn a_handled_member_failure_runs_local_recovery_without_stopping_siblings() {
    let flow = at_recovery("collection-local-recovery").await;
    let tasks = flow.poll().await;
    assert_eq!(tasks.len(), 1, "{:?}", summary(&tasks));
    assert_eq!(iteration(&tasks[0]), 1);
    assert_eq!(node(&tasks[0]), "recover");
    assert_eq!(tasks[0].task_kind, "FORWARD");
    let input: Value = serde_json::from_str(&tasks[0].input_json).expect("recovery context");
    assert_eq!(input["item"], 2);
    assert_eq!(input["index"], 1);
    let stopped: bool =
        sqlx::query_scalar("SELECT stop_reason IS NOT NULL FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(flow.tenant.as_uuid())
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("run stop state");
    assert!(!stopped, "handled failure does not stop the Run");
    flow.complete(&tasks[0], json!({"recovered":true})).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    assert!(flow.poll().await.is_empty());
    let results: Vec<u8> = sqlx::query_scalar(
        "SELECT payload.content FROM iteration_scopes scope JOIN payloads payload \
           ON payload.tenant_id=scope.tenant_id AND payload.digest=scope.result_digest \
          WHERE scope.tenant_id=$1 AND scope.run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("ordered collection result");
    assert_eq!(
        serde_json::from_slice::<Value>(&results).expect("result"),
        json!([1, 2])
    );
    let wrong_path: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='recover' \
          AND instance_path_indices=ARRAY[0]::integer[]",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("unselected recovery path");
    assert_eq!(wrong_path, 0);
    let response = flow
        .api
        .as_user(
            reqwest::Method::GET,
            &format!("/runs/{}/graph", flow.run),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .send()
        .await
        .expect("runtime graph");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let graph = crate::support::api::json_of(response).await;
    assert!(
        graph["edges"]
            .as_array()
            .expect("edges")
            .iter()
            .any(|edge| {
                edge["from"] == "lines[1]/charge"
                    && edge["to"] == "lines[1]/recover"
                    && edge["kind"] == "error"
            }),
        "journal-derived runtime graph retains the qualified local error route"
    );
    let usage: (i32, i32, i32) = sqlx::query_as(
        "SELECT admitted_instances,reserved_instances,retired_reservations \
           FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("lifetime instance charges");
    assert_eq!(
        usage,
        (9, 2, 1),
        "unselected forward work retires without refunding lifetime budget"
    );
    flow.verify().await;
}

#[tokio::test]
async fn an_unhandled_recovery_failure_drains_with_its_own_code_and_compensates() {
    let flow = at_recovery("collection-recovery-fails").await;
    let tasks = flow.poll().await;
    assert_eq!(tasks.len(), 1, "{:?}", summary(&tasks));
    assert_eq!(node(&tasks[0]), "recover");
    flow.fail(&tasks[0], "RECOVERY_FAILED", false).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATING");
    let releases = flow.poll().await;
    assert_eq!(releases.len(), 2, "both reservations must be drained");
    for task in &releases {
        assert_eq!(task.task_kind, "COMPENSATION");
        assert_eq!(node(task), "reserve~c");
        flow.complete(task, json!({"released":true})).await;
    }
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATED");
    let code: String = sqlx::query_scalar(
        "SELECT drain_failure_code FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("unhandled recovery code");
    assert_eq!(code, "RECOVERY_FAILED");
    flow.verify().await;
}

#[tokio::test]
async fn native_drain_refuses_a_failure_already_routed_to_local_recovery() {
    use sagat_application::primitives::{
        CollectionDrain, CollectionScopeId, CollectionStore, CollectionTransition,
        PrimitiveStoreError, SchedulerAuthority,
    };
    use sagat_domain::{
        execution::{CollectionDrainReason, ExecutionInstanceId, FailureCode},
        identity::{NamespaceId, Subject},
        workflow::NodeId,
    };
    let flow = at_recovery("collection-stale-handled-drain").await;
    let (namespace, version, trace): (uuid::Uuid, i64, Option<String>) = sqlx::query_as(
        "SELECT namespace_id,run_version,trace_id FROM runs WHERE tenant_id=$1 AND id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("current run");
    let (scheduler, generation): (String, i64) =
        sqlx::query_as("SELECT owner,generation FROM scheduler_shards WHERE shard=run_shard($1)")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("actual scheduler fence");
    let scope: uuid::Uuid = sqlx::query_scalar(
        "SELECT scope_id FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("current scope");
    let transition = CollectionTransition::new(
        flow.tenant,
        NamespaceId::new(namespace),
        flow.run,
        u64::try_from(version).expect("version"),
        ExecutionInstanceId::root(NodeId::parse("lines").expect("owner")),
        SchedulerAuthority::for_run(
            scheduler,
            flow.run,
            u64::try_from(generation).expect("generation"),
        ),
        Subject::engine(),
        trace,
        crate::support::now(),
        None,
    );
    let request = CollectionDrain::new(
        transition,
        CollectionScopeId::new(scope),
        CollectionDrainReason::Failed {
            iteration: 1,
            code: FailureCode::parse("PAYMENT_DECLINED").expect("code"),
        },
        uuid::Uuid::now_v7(),
    );
    let result = flow.api.executions.start_collection_drain(&request).await;
    assert!(
        matches!(result, Err(PrimitiveStoreError::InvalidTransition)),
        "a handled error is no current authority to drain: {result:?}"
    );
    let after: (i64, Option<String>, String) = sqlx::query_as(
        "SELECT run.run_version,run.stop_reason,scope.lifecycle FROM runs run \
          JOIN iteration_scopes scope ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("refused drain state");
    assert_eq!(after, (version, None, "OPEN".into()));
    flow.verify().await;
}

async fn oracle_case(
    whole: bool,
    recovery_fails: bool,
) -> (Vec<sagat_scheduler::CollectionAdvance>, Vec<Value>, String) {
    use sagat_domain::execution::ExecutionInstanceId;
    use sagat_domain::workflow::NodeId;
    use sagat_scheduler::CollectionAdvance;
    let name = format!("recovery-oracle-{}-{}", whole, recovery_fails);
    let flow = Flow::start(&name, document(&name), json!([1, 2])).await;
    let owner = ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"));
    let mut transitions = Vec::new();
    for _ in 0..80 {
        let advance = if whole {
            flow.scheduler
                .advance_collection_from_run(flow.tenant, flow.run, &owner)
                .await
        } else {
            flow.scheduler
                .advance_collection(flow.tenant, flow.run, &owner)
                .await
        }
        .expect("fenced collection transition");
        transitions.push(advance);
        if matches!(
            advance,
            CollectionAdvance::Finished | CollectionAdvance::DrainFinished
        ) {
            break;
        }
        if advance == CollectionAdvance::Waiting {
            let tasks = flow.poll().await;
            assert!(
                !tasks.is_empty(),
                "no external work can advance {transitions:?}"
            );
            for task in tasks {
                match node(&task).as_str() {
                    "charge" if iteration(&task) == 1 => {
                        flow.fail(&task, "PAYMENT_DECLINED", false).await
                    }
                    "recover" if recovery_fails => flow.fail(&task, "RECOVERY_FAILED", false).await,
                    "reserve" | "charge" | "recover" | "reserve~c" => {
                        flow.complete(&task, json!({"ok":true})).await
                    }
                    unexpected => panic!("unexpected task {unexpected}"),
                }
            }
        }
    }
    let status = flow.run_status().await;
    assert_eq!(
        status,
        if recovery_fails {
            "COMPENSATED"
        } else {
            "SUCCEEDED"
        }
    );
    flow.verify().await;
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('seq',seq,'kind',event_type,'node',node, \
            'nodes',instance_path_nodes,'indices',instance_path_indices,'detail',detail) \
         FROM run_events WHERE tenant_id=$1 AND run_id=$2 ORDER BY seq",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("complete recovery journal");
    let uuid = regex::Regex::new("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
        .expect("UUID pattern");
    let journal = rows
        .into_iter()
        .map(|row| {
            serde_json::from_str(&uuid.replace_all(&row.to_string(), "<id>"))
                .expect("normalized event")
        })
        .collect();
    (transitions, journal, status)
}

#[tokio::test]
async fn handled_recovery_and_failed_recovery_match_the_full_run_oracle() {
    let _server = crate::support::hold_server();
    for recovery_fails in [false, true] {
        let frontier = oracle_case(false, recovery_fails).await;
        let full = oracle_case(true, recovery_fails).await;
        assert_eq!(
            frontier, full,
            "recovery_fails={recovery_fails}: transitions, journal and status"
        );
    }
}

async fn compensation_branch_case(select_recovery: bool) {
    let name = format!("compensable-recovery-{select_recovery}");
    let mut document = collection_document(
        &name,
        vec![
            activity("a", "line.reserve", "b", json!({"onError":"recover"})),
            activity("b", "line.charge", "returned", json!({})),
            activity(
                "recover",
                "line.notify",
                "b",
                json!({"compensateWith":"line.refund", "compensationRetry":retry()}),
            ),
            json!({"id":"returned","type":"return","result":"item"}),
        ],
        1,
    );
    document["limits"] = json!({"maxInstances":7,"maxDepth":1});
    let flow = Flow::start(&name, document, json!([1])).await;
    flow.drive().await;
    let a = flow.poll().await;
    assert_eq!(a.len(), 1);
    assert_eq!(node(&a[0]), "a");
    if select_recovery {
        flow.fail(&a[0], "RECOVERABLE", false).await;
        flow.drive().await;
        let recovery = flow.poll().await;
        assert_eq!(recovery.len(), 1);
        assert_eq!(node(&recovery[0]), "recover");
        flow.complete(&recovery[0], json!({"effect":true})).await;
    } else {
        flow.complete(&a[0], json!({"ok":true})).await;
    }
    flow.drive().await;
    let b = flow.poll().await;
    assert_eq!(b.len(), 1);
    assert_eq!(node(&b[0]), "b");
    flow.fail(&b[0], "DOWNSTREAM_FAILED", false).await;
    flow.until_planning().await;
    let drained_usage: (i32, i32, i32) = sqlx::query_as(
        "SELECT admitted_instances,reserved_instances,retired_reservations \
           FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("drained branch reservations");
    assert_eq!(
        drained_usage,
        if select_recovery {
            (4, 1, 1)
        } else {
            (3, 0, 3)
        },
        "drain retains only the selected recovery effect reservation"
    );
    flow.drive().await;
    let undo = flow.poll().await;
    if select_recovery {
        assert_eq!(
            undo.len(),
            1,
            "selected successful recovery must retain its compensation"
        );
        assert_eq!(node(&undo[0]), "recover~c");
        assert_eq!(undo[0].task_kind, "COMPENSATION");
        assert_eq!(undo[0].target_node, "recover");
        flow.complete(&undo[0], json!({"undone":true})).await;
        flow.drive().await;
    } else {
        assert!(
            undo.is_empty(),
            "unselected recovery creates no effect or compensation"
        );
    }
    let usage: (i32, i32, i32) = sqlx::query_as(
        "SELECT admitted_instances,reserved_instances,retired_reservations \
           FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("terminal branch reservations");
    assert_eq!(
        usage,
        if select_recovery {
            (5, 0, 1)
        } else {
            (3, 0, 3)
        },
        "drain retires unreachable reservations and consumes only the real compensation"
    );
    let remaining: i32 = sqlx::query_scalar(
        "SELECT remaining_reservations FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("settled member reservation");
    assert_eq!(remaining, 0);
    let code: String = sqlx::query_scalar(
        "SELECT drain_failure_code FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("downstream failure");
    assert_eq!(code, "DOWNSTREAM_FAILED");
    assert_eq!(flow.run_status().await, "COMPENSATED");
    let sequences: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT saga.planned_sequence,saga.ended_sequence,planned.seq,ended.seq \
           FROM scope_sagas saga JOIN run_events planned \
             ON planned.tenant_id=saga.tenant_id AND planned.run_id=saga.run_id \
            AND planned.event_type='scope.compensation_planned' AND planned.node=saga.owner_node \
           JOIN run_events ended ON ended.tenant_id=saga.tenant_id AND ended.run_id=saga.run_id \
            AND ended.event_type='scope.compensated' AND ended.node=saga.owner_node \
          WHERE saga.tenant_id=$1 AND saga.run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("exact compensation event references");
    assert_eq!((sequences.0, sequences.1), (sequences.2, sequences.3));
    assert!(
        sequences.0 < sequences.1,
        "plan precedes finish, including the empty atomic plan"
    );
    flow.verify().await;
}

#[tokio::test]
async fn an_unselected_compensable_recovery_retires_every_unreachable_reservation() {
    compensation_branch_case(false).await;
}

#[tokio::test]
async fn a_selected_compensable_recovery_preserves_its_real_drain_obligation() {
    compensation_branch_case(true).await;
}
