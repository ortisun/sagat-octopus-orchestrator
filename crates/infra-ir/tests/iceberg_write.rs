//! The canonical `icebergWrite` node, parsed (feature 035: T035-053, FR-035-008, FR-035-033).
//!
//! The codec is a parser: it accepts the node the schema describes and builds the domain step. Whether a deployment may
//! publish it (`lake.enabled`, access, modes, egress) is the definition service's job (T035-054), so the old refusal of
//! `lake_capabilities.rs` moves there. Only append is built; overwrite, upsert and a record schema are refused as
//! unsupported, never dropped: a step that silently wrote less or leaked more than it said would be worse than none.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::definitions::{IrCodec, IrError};
use sagat_domain::{
    database::{ICEBERG_REVERT_ACTIVITY, ICEBERG_WRITE_ACTIVITY, RecordSource, WriteIntent},
    workflow::{NodeId, NodeKind, WorkflowError},
};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

fn document(nodes: Value) -> Vec<u8> {
    json!({"apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "lake-flow"}, "entrypoint": "save", "nodes": nodes})
    .to_string()
    .into_bytes()
}

fn write() -> Value {
    json!({"id": "save", "type": "icebergWrite", "connectionRef": {"name": "lake-main", "revision": 3},
        "table": {"namespace": ["sales", "eu"], "name": "orders"}, "ref": "audit", "mode": "append",
        "records": {"inline": "input.rows"},
        "mapping": [{"field": "/id", "column": "id"}, {"field": "/customer/name", "column": "customer_name"}],
        "ignoreUnknownFields": true,
        "limits": {"maxRecordBytes": 1048576, "maxRecords": 5000, "timeoutMs": 60000},
        "next": "done"})
}

fn flow(step: Value) -> Vec<u8> {
    document(json!([step, {"id": "done", "type": "end"}]))
}

fn id(text: &str) -> NodeId {
    NodeId::parse(text).expect("node id")
}

#[test]
fn a_canonical_iceberg_write_is_parsed_into_the_domain_step() {
    let parsed = JsonIrCodec.parse(&flow(write())).expect("a published step");
    let node = parsed.definition.node(&id("save")).expect("the node");
    let step = node.iceberg_write().expect("an iceberg write");
    assert_eq!(step.connection().name.as_str(), "lake-main");
    assert_eq!(step.connection().revision, Some(3));
    assert_eq!(
        step.table().namespace(),
        ["sales".to_owned(), "eu".to_owned()]
    );
    assert_eq!(step.table().name(), "orders");
    assert_eq!(step.reference(), "audit");
    assert_eq!(step.intent(), WriteIntent::Append);
    assert!(
        matches!(step.records(), RecordSource::Inline(expression) if expression.as_str() == "input.rows")
    );
    assert_eq!(step.mapping().len(), 2);
    assert_eq!(
        (step.mapping()[1].field(), step.mapping()[1].column()),
        ("/customer/name", "customer_name")
    );
    assert!(step.ignore_unknown_fields());
    assert_eq!(
        (
            step.limits().max_record_bytes,
            step.limits().max_records,
            step.limits().timeout_ms
        ),
        (Some(1_048_576), Some(5000), Some(60_000))
    );
    assert!(!step.revert());
    let NodeKind::Activity { activity, .. } = node.kind() else {
        panic!("an activity");
    };
    assert_eq!(activity.as_str(), ICEBERG_WRITE_ACTIVITY);
    assert_eq!(node.next(), Some(&id("done")));
}

#[test]
fn the_defaults_are_the_contracts_defaults() {
    let minimal = json!({"id": "save", "type": "icebergWrite", "connectionRef": {"name": "lake-main"},
        "table": {"namespace": ["sales"], "name": "orders"}, "mode": "append",
        "records": {"inline": "input.rows"}, "next": "done"});
    let parsed = JsonIrCodec.parse(&flow(minimal)).expect("a step");
    let step = parsed
        .definition
        .node(&id("save"))
        .expect("node")
        .iceberg_write()
        .expect("step");
    assert_eq!(step.reference(), "main");
    assert_eq!(step.connection().revision, None);
    assert!(step.mapping().is_empty());
    assert!(!step.ignore_unknown_fields());
    assert_eq!(step.limits().max_records, None);
    assert!(!step.revert());
}

#[test]
fn each_record_source_is_parsed_and_exactly_one_is_needed() {
    for (records, is_expected) in [
        (
            json!({"inline": "input.rows"}),
            (|s: &RecordSource| matches!(s, RecordSource::Inline(_))) as fn(&RecordSource) -> bool,
        ),
        (json!({"blobs": "steps.export.output.blobs"}), |s| {
            matches!(s, RecordSource::Blobs(_))
        }),
    ] {
        let mut step = write();
        step["records"] = records.clone();
        let parsed = JsonIrCodec.parse(&flow(step)).expect("a step");
        let step = parsed
            .definition
            .node(&id("save"))
            .expect("node")
            .iceberg_write()
            .expect("step");
        assert!(is_expected(step.records()), "{records}");
    }
    for records in [
        json!({}),
        json!({"inline": "a", "blobs": "b"}),
        json!({"output": true}),
        json!({"outputPath": "/a"}),
    ] {
        let mut step = write();
        step["records"] = records;
        assert!(JsonIrCodec.parse(&flow(step)).is_err());
    }
}

#[test]
fn a_database_result_source_names_an_earlier_node() {
    let doc = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "lake-flow"}, "entrypoint": "read",
        "nodes": [
            {"id": "read", "type": "database", "connectionRef": {"name": "orders-db"},
             "operation": "query", "statement": "SELECT 1", "next": "save"},
            {"id": "save", "type": "icebergWrite", "connectionRef": {"name": "lake-main"},
             "table": {"namespace": ["sales"], "name": "orders"}, "mode": "append",
             "records": {"databaseResult": "read"}, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    });
    let parsed = JsonIrCodec
        .parse(doc.to_string().as_bytes())
        .expect("a step fed by a database result");
    let step = parsed
        .definition
        .node(&id("save"))
        .expect("node")
        .iceberg_write()
        .expect("step");
    assert!(
        matches!(step.records(), RecordSource::DatabaseResult(node) if node.as_str() == "read")
    );
}

#[test]
fn a_revert_declares_the_derived_compensation() {
    let mut step = write();
    step["compensation"] = json!("revert");
    let parsed = JsonIrCodec.parse(&flow(step)).expect("a revertible step");
    let node = parsed.definition.node(&id("save")).expect("node");
    assert!(node.iceberg_write().expect("step").revert());
    assert!(!node.effect().irreversible && node.effect().high_risk);
    let derived = parsed
        .definition
        .node(&NodeId::parse_stored("save~c").expect("stored id"))
        .expect("compensation");
    let NodeKind::Activity { activity, .. } = derived.kind() else {
        panic!("an activity");
    };
    assert_eq!(activity.as_str(), ICEBERG_REVERT_ACTIVITY);
}

#[test]
fn what_is_not_built_is_refused_as_unsupported_and_never_dropped() {
    for (change, kind) in [
        (
            json!({"mode": "overwrite", "overwrite": {"scope": "table"}}),
            "icebergWrite.overwrite",
        ),
        (
            json!({"mode": "upsert", "upsert": {"keyColumns": ["id"]}}),
            "icebergWrite.upsert",
        ),
        (
            json!({"recordSchema": {"type": "object"}}),
            "icebergWrite.recordSchema",
        ),
    ] {
        let mut step = write();
        for (key, value) in change.as_object().expect("object") {
            step[key] = value.clone();
        }
        match JsonIrCodec.parse(&flow(step)) {
            Err(IrError::Unsupported {
                node, kind: found, ..
            }) => {
                assert_eq!((node.as_str(), found.as_str()), ("save", kind));
            }
            other => panic!("{kind}: expected an unsupported capability, got {other:?}"),
        }
    }
}

#[test]
fn a_field_outside_the_contract_is_refused_without_echoing_it() {
    for field in [
        "unknown",
        "retry",
        "highRisk",
        "irreversible",
        "timeoutMs",
        "secrets",
        "idempotencyKey",
    ] {
        let mut step = write();
        step[field] = json!("SENSITIVE_LAKE_CANARY");
        let error = JsonIrCodec.parse(&flow(step)).expect_err(field);
        assert!(
            !format!("{error:?} {error}").contains("SENSITIVE_LAKE_CANARY"),
            "{field}: {error:?}"
        );
    }
    let mut step = write();
    step["mapping"] = json!([{"field": "/id", "column": "id", "SENSITIVE_LAKE_CANARY": 1}]);
    let error = JsonIrCodec
        .parse(&flow(step))
        .expect_err("a mapping with an extra key");
    assert!(
        !format!("{error:?} {error}").contains("SENSITIVE_LAKE_CANARY"),
        "{error:?}"
    );
}

#[test]
fn the_bounds_of_the_domain_are_refused_with_a_reason_that_names_the_part() {
    for (mutate, part) in [
        (
            json!({"mapping": [{"field": "/a", "column": "c"}, {"field": "/a", "column": "d"}]}),
            "mapped",
        ),
        (
            json!({"mapping": [{"field": "/a", "column": "c"}, {"field": "/b", "column": "c"}]}),
            "mapped",
        ),
        (json!({"limits": {"timeoutMs": 999}}), "limit"),
        (json!({"table": {"namespace": [], "name": "t"}}), "table"),
    ] {
        let mut step = write();
        for (key, value) in mutate.as_object().expect("object") {
            step[key] = value.clone();
        }
        let error = JsonIrCodec.parse(&flow(step)).expect_err(part);
        assert!(matches!(error, IrError::Schema(_)), "{part}: {error:?}");
    }
}

#[test]
fn an_invalid_expression_is_refused_at_publication() {
    for records in [
        json!({"inline": "input.rows["}),
        json!({"blobs": "steps.export.output["}),
    ] {
        let mut step = write();
        step["records"] = records;
        let error = JsonIrCodec
            .parse(&flow(step))
            .expect_err("a broken expression");
        assert!(
            format!("{error:?}").to_lowercase().contains("expression"),
            "{error:?}"
        );
    }
}

#[test]
fn ordinary_activities_cannot_forge_the_reserved_iceberg_names() {
    for field in ["activity", "compensateWith", "reconcileWith"] {
        let mut node =
            json!({"id": "save", "type": "activity", "activity": "regular", "next": "done"});
        node[field] = json!(ICEBERG_WRITE_ACTIVITY);
        assert!(JsonIrCodec.parse(&flow(node)).is_err(), "{field}");
    }
    let mut node = json!({"id": "save", "type": "activity", "activity": "regular", "next": "done"});
    node["compensateWith"] = json!(ICEBERG_REVERT_ACTIVITY);
    assert!(JsonIrCodec.parse(&flow(node)).is_err());
}

#[test]
fn an_iceberg_node_is_root_only() {
    let mut before_loop = write();
    before_loop["next"] = json!("lines");
    let beside_collection = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "scoped-lake"}, "entrypoint": "save",
        "nodes": [before_loop, {"id": "lines", "type": "forEach", "items": "input.items",
            "body": "line", "result": "bodyResult", "maxItems": 2, "maxConcurrency": 1,
            "next": "done"}, {"id": "done", "type": "end"}],
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": "line.process", "next": "return"},
            {"id": "return", "type": "return", "result": "steps.work.output"}
        ]}}
    });
    let error = JsonIrCodec
        .parse_structured(beside_collection.to_string().as_bytes())
        .expect_err("an iceberg node beside a collection");
    assert!(
        matches!(error, IrError::Invalid(WorkflowError::IcebergNodeNotRoot)),
        "{error:?}"
    );

    // A body does not even know the node type: the schema refuses it before the domain does.
    let mut inside = beside_collection.clone();
    inside["nodes"][0] =
        json!({"id": "save", "type": "activity", "activity": "x.y", "next": "lines"});
    let mut in_body = write();
    in_body["id"] = json!("work");
    in_body["next"] = json!("return");
    inside["bodies"]["line"]["nodes"][0] = in_body;
    assert!(
        matches!(
            JsonIrCodec.parse_structured(inside.to_string().as_bytes()),
            Err(IrError::Schema(_))
        ),
        "an iceberg node inside a body"
    );
}

#[test]
fn the_canonical_bytes_do_not_depend_on_the_order_of_the_keys_and_do_not_materialise_defaults() {
    let first = JsonIrCodec.parse(&flow(write())).expect("first");
    let mut reordered = json!({});
    let original = write();
    let mut keys: Vec<_> = original
        .as_object()
        .expect("object")
        .keys()
        .cloned()
        .collect();
    keys.reverse();
    for key in keys {
        reordered[&key] = original[&key].clone();
    }
    let second = JsonIrCodec.parse(&flow(reordered)).expect("second");
    assert_eq!(first.canonical, second.canonical);
    assert_eq!(first.digest, second.digest);
    let mut minimal = write();
    minimal.as_object_mut().expect("object").remove("ref");
    let without = JsonIrCodec.parse(&flow(minimal)).expect("without a ref");
    assert!(
        !String::from_utf8(without.canonical)
            .expect("utf8")
            .contains("\"ref\""),
        "a default must not be written into the canonical form"
    );
}

#[test]
fn options_of_a_mode_that_is_not_built_are_refused_even_beside_append() {
    for (key, options) in [
        ("overwrite", json!({"scope": "table"})),
        ("upsert", json!({"keyColumns": ["id"]})),
    ] {
        let mut step = write();
        step[key] = options;
        match JsonIrCodec.parse(&flow(step)) {
            Err(IrError::Unsupported { kind, .. }) => {
                assert_eq!(kind, format!("icebergWrite.{key}"))
            }
            other => panic!("{key}: expected an unsupported capability, got {other:?}"),
        }
    }
}

#[test]
fn an_invalid_iceberg_node_is_diagnosed_as_one_and_not_as_an_unknown_type() {
    let mut step = write();
    step["mode"] = json!("sideways");
    let IrError::Schema(issues) = JsonIrCodec.parse(&flow(step)).expect_err("a bad mode") else {
        panic!("a schema problem");
    };
    assert!(!issues.is_empty());
    assert!(
        issues
            .iter()
            .all(|issue| !issue.message.contains("unknown node type")),
        "the node type is known: {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.message.contains("icebergWrite")),
        "{issues:?}"
    );
}

/// `records.databaseResult` names a node, and the graph decides whether that node can be the source (G035-035 point 2):
/// the codec reports it as the invalid Iceberg node it is.
#[test]
fn a_database_result_must_name_a_database_step_that_runs_before_the_write() {
    let read = json!({"id": "read", "type": "database", "connectionRef": {"name": "orders-db"},
        "operation": "query", "statement": "SELECT 1"});
    let save = |source: &str| {
        json!({"id": "save", "type": "icebergWrite", "connectionRef": {"name": "lake-main"},
            "table": {"namespace": ["sales"], "name": "orders"}, "mode": "append",
            "records": {"databaseResult": source}})
    };
    let end = json!({"id": "done", "type": "end"});
    let with_next = |mut node: Value, next: &str| {
        node["next"] = json!(next);
        node
    };
    let cases = [
        (
            "a node that does not exist",
            "save",
            vec![with_next(save("ghost"), "done"), end.clone()],
        ),
        (
            "the write itself",
            "save",
            vec![with_next(save("save"), "done"), end.clone()],
        ),
        (
            "a database step that runs after the write",
            "save",
            vec![
                with_next(save("read"), "read"),
                with_next(read.clone(), "done"),
                end.clone(),
            ],
        ),
        (
            // A statement that executes retains no column: the write could never have a record, and would fail at its lease.
            "a database step that executes and selects nothing",
            "insert",
            vec![
                with_next(
                    json!({"id": "insert", "type": "database", "connectionRef": {"name": "orders-db"},
                        "operation": "execute", "statement": "INSERT INTO orders VALUES (1)"}),
                    "save",
                ),
                with_next(save("insert"), "done"),
                end.clone(),
            ],
        ),
    ];
    for (label, entrypoint, nodes) in cases {
        let doc = json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
            "metadata": {"name": "lake-flow"}, "entrypoint": entrypoint, "nodes": nodes
        });
        let refused = JsonIrCodec.parse(doc.to_string().as_bytes());
        assert!(
            matches!(
                refused,
                Err(IrError::Invalid(WorkflowError::InvalidIcebergNode))
            ),
            "{label}: {refused:?}"
        );
    }
}
