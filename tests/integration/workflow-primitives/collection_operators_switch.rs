//! The `collection-operators` switch of design 042 (T042-47, FR-042-003), against real PostgreSQL and the real codec:
//! with it off, publication refuses a `forEach` chain; a chain published while it was on stays readable with it off,
//! through the restore every run uses (`parse_stored`).

use sagat_application::definitions::{IrCodec, IrError};
use sagat_application::error::AppError;
use sagat_domain::workflow::{NodeId, NodeKind};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{ctx, world};

fn chained(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": "line.process", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": [
            {"id": "lines", "type": "forEach", "items": "input.lines",
             "operators": [{"filter": {"where": "element > 0"}}, {"take": {"count": 2}}],
             "body": "line", "result": "bodyResult", "maxItems": 3, "maxConcurrency": 1, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

#[tokio::test]
async fn a_stored_chain_parses_through_parse_stored_while_the_switch_is_off() {
    let w = world().await;
    let document = chained("collection-operators-switch");

    // Off (the default): refused at publication, naming the node.
    match w.try_publish_as(&w.admin, document.clone()).await {
        Err(AppError::InvalidDefinition(error)) => assert!(
            matches!(*error, IrError::CollectionOperatorsDisabled { ref node } if node == "lines"),
            "{error:?}"
        ),
        other => panic!("expected COLLECTION_OPERATORS_DISABLED, got {other:?}"),
    }

    // On: published.
    let on = w.defs.clone().with_collection_operators_enabled(true);
    let created = on
        .create_workflow(
            &w.admin,
            "payments",
            "collection-operators-stored",
            &{
                let mut renamed = document.clone();
                renamed["metadata"]["name"] = json!("collection-operators-stored");
                serde_json::to_vec(&renamed).expect("json")
            },
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("created");
    let workflow = created.workflow.id();
    let published = on
        .publish(
            &w.admin,
            workflow,
            created.workflow.draft_revision(),
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
    // The restore every run start uses goes through the same parse and knows no switch.
    w.start(workflow, &json!({"lines": [1, 2, 3]})).await;
}
