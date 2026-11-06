//! The step an Apache Iceberg write reads a database result from (feature 035: T035-054; G035-035 point 2).
//!
//! `records.databaseResult` names a node, and a name proves nothing: the node has to exist, to be a managed database step, and
//! to run before the write, so that its result is there when the write starts. Control has to be able to pass from it to the
//! write. This is a rule of the graph, so it is checked where the graph is built and not when a run reaches the step.
#![allow(clippy::expect_used, clippy::panic)]

mod common;

use common::{activity, condition, end, expr, id};
use sagat_domain::{
    database::{
        ColumnMapping, ConnectionRef, DatabaseLimits, DatabaseOperation, IcebergWrite,
        IcebergWriteParts, LakeLimits, OperationKind, RecordSource, TableIdentifier, WriteIntent,
    },
    identity::Slug,
    workflow::{Node, WorkflowDefinition, WorkflowError},
};

fn query(node: &str, next: &str) -> Node {
    let operation = DatabaseOperation::new(
        ConnectionRef::new(Slug::parse("orders-db").expect("name"), None).expect("reference"),
        OperationKind::Query,
        "SELECT 1",
        Vec::new(),
        DatabaseLimits::default(),
    )
    .expect("a valid operation");
    Node::database(id(node), operation).with_next(id(next))
}

fn save(source: &str, next: &str) -> Node {
    let write = IcebergWrite::new(IcebergWriteParts {
        connection: ConnectionRef::new(Slug::parse("lake-main").expect("name"), None).expect("ref"),
        table: TableIdentifier::new(vec!["sales".to_owned()], "orders").expect("table"),
        reference: "main".to_owned(),
        intent: WriteIntent::Append,
        records: RecordSource::DatabaseResult(id(source)),
        mapping: vec![ColumnMapping::new("/id", "id").expect("mapping")],
        ignore_unknown_fields: false,
        limits: LakeLimits::default(),
        revert: false,
    })
    .expect("a valid write");
    Node::iceberg(id("save"), write).with_next(id(next))
}

fn definition(entry: &str, nodes: Vec<Node>) -> Result<WorkflowDefinition, WorkflowError> {
    WorkflowDefinition::new(Slug::parse("lake-flow").expect("name"), id(entry), nodes)
}

#[test]
fn the_result_of_an_earlier_database_step_is_a_source() {
    definition(
        "read",
        vec![query("read", "save"), save("read", "done"), end("done")],
    )
    .expect("a write that reads the result of the step before it");
}

#[test]
fn a_database_step_that_comes_before_a_branch_is_a_source_for_a_write_inside_it() {
    definition(
        "read",
        vec![
            query("read", "choose"),
            condition("choose", "save", "done"),
            save("read", "done"),
            end("done"),
        ],
    )
    .expect("the result is there on every path that reaches the write");
}

#[test]
fn a_node_that_does_not_exist_is_not_a_source() {
    let refused = definition("save", vec![save("ghost", "done"), end("done")]);
    assert_eq!(refused.err(), Some(WorkflowError::InvalidIcebergNode));
}

#[test]
fn a_node_that_is_not_a_database_step_is_not_a_source() {
    let refused = definition(
        "fetch",
        vec![
            activity("fetch").with_next(id("save")),
            save("fetch", "done"),
            end("done"),
        ],
    );
    assert_eq!(refused.err(), Some(WorkflowError::InvalidIcebergNode));
}

#[test]
fn the_write_itself_is_not_a_source() {
    let refused = definition("save", vec![save("save", "done"), end("done")]);
    assert_eq!(refused.err(), Some(WorkflowError::InvalidIcebergNode));
}

#[test]
fn a_database_step_that_comes_after_the_write_is_not_a_source() {
    // Its result does not exist when the write starts.
    let refused = definition(
        "save",
        vec![save("read", "read"), query("read", "done"), end("done")],
    );
    assert_eq!(refused.err(), Some(WorkflowError::InvalidIcebergNode));
}

#[test]
fn a_database_step_on_another_branch_is_not_a_source() {
    // Control never passes from `read` to `save`: the two are the arms of a condition.
    let refused = definition(
        "choose",
        vec![
            condition("choose", "read", "save"),
            query("read", "done"),
            save("read", "done"),
            end("done"),
        ],
    );
    assert_eq!(refused.err(), Some(WorkflowError::InvalidIcebergNode));
}

#[test]
fn the_other_sources_need_no_database_step() {
    // An inline list and blobs name an expression, not a node.
    let inline = IcebergWrite::new(IcebergWriteParts {
        connection: ConnectionRef::new(Slug::parse("lake-main").expect("name"), None).expect("ref"),
        table: TableIdentifier::new(vec!["sales".to_owned()], "orders").expect("table"),
        reference: "main".to_owned(),
        intent: WriteIntent::Append,
        records: RecordSource::Inline(expr("input.rows")),
        mapping: Vec::new(),
        ignore_unknown_fields: false,
        limits: LakeLimits::default(),
        revert: false,
    })
    .expect("a valid write");
    definition(
        "save",
        vec![
            Node::iceberg(id("save"), inline).with_next(id("done")),
            end("done"),
        ],
    )
    .expect("an inline source needs no earlier step");
}

#[test]
fn a_long_chain_of_branches_is_searched_in_linear_time() {
    // Forty diamonds in a row have 2^40 paths. The write is on the other side of the first branch, so no path from the
    // database step reaches it and every one of them would be tried by a search that forgets where it has been.
    const DIAMONDS: usize = 40;
    let mut nodes = vec![condition("start", "read", "save")];
    nodes.push(query("read", "c0"));
    for n in 0..DIAMONDS {
        let next = if n + 1 == DIAMONDS {
            "done".to_owned()
        } else {
            format!("c{}", n + 1)
        };
        nodes.push(condition(
            &format!("c{n}"),
            &format!("a{n}"),
            &format!("b{n}"),
        ));
        nodes.push(activity(&format!("a{n}")).with_next(id(&next)));
        nodes.push(activity(&format!("b{n}")).with_next(id(&next)));
    }
    nodes.push(save("read", "done"));
    nodes.push(end("done"));

    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(definition("start", nodes));
    });
    let refused = receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the search must not try every path of a long chain of branches");
    assert_eq!(refused.err(), Some(WorkflowError::InvalidIcebergNode));
}
