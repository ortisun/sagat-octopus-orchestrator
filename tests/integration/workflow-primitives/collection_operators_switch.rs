//! The `collection-operators` switch of design 042 (T042-47, FR-042-003, D042-105, D042-106), against real PostgreSQL
//! and the real codec. With it off, publication refuses a `forEach` chain; what was published while it was on stays
//! readable, can be activated, rolled back and promoted where it already runs, and a publication retried with its own
//! key gets its version back. Until the scheduler applies a chain (T042-8) a chained run fails closed at entry with
//! `COLLECTION_OPERATORS_UNSUPPORTED` instead of running the body over the list `items` yields (G042-100).

use sagat_application::definitions::{IrCodec, IrError};
use sagat_application::error::AppError;
use sagat_application::publishing::Promotion;
use sagat_domain::definition::WorkflowId;
use sagat_domain::identity::{Environment, Role};
use sagat_domain::versioning::{RequestState, Verdict};
use sagat_domain::workflow::{NodeId, NodeKind};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::support::engine::{World, ctx, world};

fn chained(name: &str, take: u32) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": "line.process", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": [
            {"id": "lines", "type": "forEach", "items": "input.lines",
             "operators": [{"filter": {"where": "element > 0"}}, {"take": {"count": take}}],
             "body": "line", "result": "bodyResult", "maxItems": 3, "maxConcurrency": 1, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

fn bytes(document: &Value) -> Vec<u8> {
    serde_json::to_vec(document).expect("json")
}

fn switched_off(result: Result<impl std::fmt::Debug, AppError>) {
    match result {
        Err(AppError::InvalidDefinition(error)) => assert!(
            matches!(*error, IrError::CollectionOperatorsDisabled { ref node } if node == "lines"),
            "{error:?}"
        ),
        other => panic!("expected COLLECTION_OPERATORS_DISABLED, got {other:?}"),
    }
}

/// A draft of `name` in `namespace`, created by a service that is on.
async fn draft(w: &World, namespace: &str, document: &Value) -> (WorkflowId, u64) {
    let on = w.defs.clone().with_collection_operators_enabled(true);
    let created = on
        .create_workflow(
            &w.admin,
            namespace,
            document["metadata"]["name"].as_str().expect("name"),
            &bytes(document),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("created");
    (created.workflow.id(), created.workflow.draft_revision())
}

async fn status_and_code(w: &World, run: Uuid) -> (String, Option<String>) {
    let row = sqlx::query("SELECT status, stop_code FROM runs WHERE id = $1")
        .bind(run)
        .fetch_one(&w.db.pool)
        .await
        .expect("run row");
    (row.get("status"), row.get("stop_code"))
}

#[tokio::test]
async fn a_stored_chain_parses_through_parse_stored_while_the_switch_is_off() {
    let w = world().await;

    // Off (the default): refused at publication, naming the node.
    switched_off(
        w.try_publish_as(&w.admin, chained("collection-operators-switch", 2))
            .await,
    );

    // On: published.
    let on = w.defs.clone().with_collection_operators_enabled(true);
    let (workflow, revision) =
        draft(&w, "payments", &chained("collection-operators-stored", 2)).await;
    let published = on
        .publish(
            &w.admin,
            workflow,
            revision,
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("published while the switch is on");

    // Off again: the stored version is read and restored as it was, chain included.
    let stored = w
        .defs
        .get_version(&w.admin, workflow, published.version.number(), &ctx())
        .await
        .expect("the stored version");
    let restored = JsonIrCodec
        .parse_stored(&stored.document)
        .expect("a stored chain is never made unreadable by the switch");
    match restored
        .definition
        .node(&NodeId::parse("lines").expect("id"))
        .expect("the collection")
        .kind()
    {
        NodeKind::ForEach {
            operators: Some(chain),
            ..
        } => assert_eq!(chain.len(), 2),
        other => panic!("expected the chain, got {other:?}"),
    }
    assert_eq!(restored.digest, *published.version.digest());

    // The run starts (its restore knows no switch), and its scheduler fails it closed at entry: no body runs over the
    // three elements a `take 2` would never let through (G042-100).
    crate::limits::allow_iterations(&w, 4).await;
    let scheduler = crate::limits::scheduler(&w, "collection-operators").await;
    let run = w.start(workflow, &json!({"lines": [1, 2, 3]})).await;
    let first = scheduler
        .advance_collection_from_run(
            w.admin.tenant_id(),
            run,
            &sagat_domain::execution::ExecutionInstanceId::root(
                NodeId::parse("lines").expect("id"),
            ),
        )
        .await;
    assert!(
        matches!(
            first,
            Err(sagat_scheduler::CollectionAdvanceError::OperatorsUnsupported)
        ),
        "{first:?}"
    );
    scheduler.tick().await;
    assert_eq!(
        status_and_code(&w, run.as_uuid()).await,
        (
            "FAILED".to_owned(),
            Some("COLLECTION_OPERATORS_UNSUPPORTED".to_owned())
        )
    );
    let scopes: i64 = sqlx::query_scalar("SELECT count(*) FROM iteration_scopes WHERE run_id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("scope count");
    assert_eq!(scopes, 0, "the collection was never entered");
}

/// D042-105: a publication asked again with its own key after the switch was turned off is answered by the store's
/// replay, before the switch is read; a new key is a new publication and is refused.
#[tokio::test]
async fn a_publication_retried_with_its_key_after_the_switch_is_off_gets_its_version() {
    let w = world().await;
    let (workflow, revision) =
        draft(&w, "payments", &chained("collection-operators-retry", 2)).await;
    let key = format!("publish-{}", Uuid::now_v7());
    let first = w
        .defs
        .clone()
        .with_collection_operators_enabled(true)
        .publish(&w.admin, workflow, revision, None, &key, &ctx())
        .await
        .expect("published while the switch is on");
    assert!(first.created);

    let again = w
        .defs
        .publish(&w.admin, workflow, revision, None, &key, &ctx())
        .await
        .expect("the retry is answered by the replay");
    assert!(!again.created);
    assert_eq!(again.version.id(), first.version.id());

    switched_off(
        w.defs
            .publish(
                &w.admin,
                workflow,
                revision,
                None,
                &format!("publish-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await,
    );
}

/// D042-106: activating, rolling back and promoting where it already runs move a pointer to a version that exists;
/// none is a new publication, so none reads the switch.
#[tokio::test]
async fn a_chained_version_is_activated_rolled_back_and_promoted_again_with_the_switch_off() {
    let w = world().await;
    let on = w.publishing.clone().with_collection_operators_enabled(true);
    let off = w.publishing.clone();
    let defs_on = w.defs.clone().with_collection_operators_enabled(true);
    let (workflow, revision) =
        draft(&w, "payments", &chained("collection-operators-pointer", 2)).await;
    defs_on
        .publish(
            &w.admin,
            workflow,
            revision,
            None,
            "publish-pointer-v1-0123456789",
            &ctx(),
        )
        .await
        .expect("v1");
    let edited = defs_on
        .update_draft(
            &w.admin,
            workflow,
            revision,
            &bytes(&chained("collection-operators-pointer", 1)),
            &ctx(),
        )
        .await
        .expect("edited");
    defs_on
        .publish(
            &w.admin,
            workflow,
            edited.workflow.draft_revision(),
            None,
            "publish-pointer-v2-0123456789",
            &ctx(),
        )
        .await
        .expect("v2");

    let back = off
        .activate(&w.admin, workflow, 1, "roll back to v1", None, true, &ctx())
        .await
        .expect("a rollback is not a publication");
    assert!(back.changed);
    let forward = off
        .activate(&w.admin, workflow, 2, "forward to v2", None, true, &ctx())
        .await
        .expect("an activation is not a publication");
    assert!(forward.changed);

    let staging = w
        .add_namespace_in("operators-staging", Environment::Staging)
        .await;
    let promoted = on
        .promote(
            &w.admin,
            workflow,
            2,
            staging.id(),
            "tried in dev",
            true,
            "promote-pointer-0123456789",
            &ctx(),
        )
        .await
        .expect("promoted while the switch is on");
    assert!(matches!(promoted, Promotion::Published(_)), "{promoted:?}");
    let again = off
        .promote(
            &w.admin,
            workflow,
            2,
            staging.id(),
            "tried in dev",
            true,
            "promote-pointer-again-0123456789",
            &ctx(),
        )
        .await
        .expect("the target already runs it");
    assert!(
        matches!(again, Promotion::AlreadyActive { version: 1 }),
        "{again:?}"
    );
}

/// A request that was published is asked again with its key after the switch was turned off: the store answers it.
#[tokio::test]
async fn a_published_request_replays_with_the_switch_off() {
    let w = world().await;
    let on = w.publishing.clone().with_collection_operators_enabled(true);
    let off = w.publishing.clone();
    w.add_namespace_in("operators-prod", Environment::Prod)
        .await;
    let author = w.member("author", Role::Developer).await;
    let reviewer = w.member("reviewer", Role::Developer).await;
    let (workflow, revision) = draft(
        &w,
        "operators-prod",
        &chained("collection-operators-request", 2),
    )
    .await;
    let request = on
        .open_request(&author, workflow, revision, None, &ctx())
        .await
        .expect("a request");
    on.decide(
        &reviewer,
        request.id(),
        Verdict::Approve,
        "reviewed the chain",
        &ctx(),
    )
    .await
    .expect("approved");
    let key = "publish-request-0123456789";
    let published = on
        .publish_request(&author, request.id(), key, &ctx())
        .await
        .expect("published while the switch is on");
    let replayed = off
        .publish_request(&author, request.id(), key, &ctx())
        .await
        .expect("the replay of a published request");
    assert!(!replayed.created);
    assert_eq!(replayed.version.id(), published.version.id());
    let request = off
        .get_request(&author, request.id(), &ctx())
        .await
        .expect("the request");
    assert_eq!(request.state(), RequestState::Published);
}
