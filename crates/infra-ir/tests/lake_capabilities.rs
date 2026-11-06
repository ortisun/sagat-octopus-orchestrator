//! Feature 035 capabilities that the canonical schema describes and the engine cannot run yet (T035-008).
//!
//! The schema is the editor-time gate. A workflow that the schema accepts but the engine would run without the
//! step (a destination that is dropped, a write that never happens) must not be published: the codec refuses it with
//! `IrError::Unsupported`, which the API reports as `UNSUPPORTED_NODE`. The domain tasks that parse these nodes
//! (T035-093 and the tasks before it) replace these refusals with acceptance tests. The Iceberg write step left this
//! file with T035-053: the codec parses it (`iceberg_write.rs`) and the definition service decides whether a
//! deployment may publish it.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::definitions::{IrCodec, IrError};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

fn document(nodes: Value) -> Vec<u8> {
    json!({"apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "lake-flow"}, "entrypoint": "first", "nodes": nodes})
    .to_string()
    .into_bytes()
}

fn destination() -> Value {
    json!({"name": "notify", "kind": "broker_stream", "subscriptionRef": "order-events",
        "target": {}, "records": {"output": true}})
}

fn unsupported(nodes: Value) -> (String, String) {
    match JsonIrCodec.parse(&document(nodes)) {
        Err(IrError::Unsupported { node, kind, .. }) => (node, kind),
        other => panic!("expected an unsupported capability, got {other:?}"),
    }
}

#[test]
fn destinations_on_an_activity_are_refused_and_not_dropped() {
    let (node, kind) = unsupported(json!([
        {"id": "first", "type": "activity", "activity": "orders.extract", "next": "done",
         "destinations": [destination()]},
        {"id": "done", "type": "end"}
    ]));
    assert_eq!((node.as_str(), kind.as_str()), ("first", "destinations"));
}

#[test]
fn destinations_on_a_database_node_are_refused_and_not_dropped() {
    let (node, kind) = unsupported(json!([
        {"id": "first", "type": "database", "connectionRef": {"name": "orders-db"},
         "operation": "query", "statement": "SELECT 1", "next": "done",
         "destinations": [destination()]},
        {"id": "done", "type": "end"}
    ]));
    assert_eq!((node.as_str(), kind.as_str()), ("first", "destinations"));
}

/// The refusal is about the capability, not about the neighbours: the same steps without destinations publish.
#[test]
fn the_same_steps_without_destinations_are_still_published() {
    let nodes = json!([
        {"id": "first", "type": "activity", "activity": "orders.extract", "next": "read"},
        {"id": "read", "type": "database", "connectionRef": {"name": "orders-db"},
         "operation": "query", "statement": "SELECT 1", "next": "done"},
        {"id": "done", "type": "end"}
    ]);
    let parsed = JsonIrCodec.parse(&document(nodes));
    assert!(parsed.is_ok(), "{parsed:?}");
}
