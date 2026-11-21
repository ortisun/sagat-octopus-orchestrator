//! The classification of what an `icebergWrite` step writes, against the ceiling of its connection (feature 035: T035-054;
//! FR-035-005, FR-035-025, D035-051).
//!
//! Pure: the levels are data of the step, set by whoever read the schemas of the source (the IR codec), and the check is a
//! comparison. A step whose levels were never computed proves nothing, so nothing leaves.
#![allow(clippy::expect_used, clippy::panic)]

mod common;

use common::{expr, id};
use sagat_domain::{
    classification::{EgressCeiling, Level},
    database::{
        ColumnMapping, ConnectionRef, EgressField, EgressRefusal, IcebergWrite, IcebergWriteParts,
        LakeLimits, RecordSource, TableIdentifier, WriteIntent,
    },
    identity::Slug,
    workflow::{Node, NodeKind, WorkflowDefinition},
};

fn write() -> IcebergWrite {
    IcebergWrite::new(IcebergWriteParts {
        connection: ConnectionRef::new(Slug::parse("lake-main").expect("name"), None).expect("ref"),
        table: TableIdentifier::new(vec!["sales".to_owned()], "orders").expect("table"),
        reference: "main".to_owned(),
        intent: WriteIntent::Append,
        records: RecordSource::Inline(expr("input.orders")),
        mapping: vec![
            ColumnMapping::new("/id", "id").expect("mapping"),
            ColumnMapping::new("/email", "email").expect("mapping"),
        ],
        ignore_unknown_fields: false,
        limits: LakeLimits::default(),
        revert: false,
    })
    .expect("a valid write")
}

fn field(path: &str, level: Level) -> EgressField {
    EgressField::new(path, level)
}

fn refused(path: &str, level: Level) -> EgressRefusal {
    EgressRefusal::Field {
        path: path.to_owned(),
        level,
    }
}

#[test]
fn a_step_whose_levels_were_never_computed_is_refused_whole() {
    let step = write();
    assert!(step.egress().is_none());
    for ceiling in [
        EgressCeiling::Public,
        EgressCeiling::Internal,
        EgressCeiling::Pii,
    ] {
        assert_eq!(
            step.egress_refusals(ceiling),
            vec![EgressRefusal::NotComputed],
            "a ceiling of {ceiling:?} cannot vouch for fields nobody classified"
        );
    }
}

#[test]
fn a_pii_field_is_refused_against_an_internal_ceiling_and_named_without_its_value() {
    let step = write().with_egress(vec![
        field("/id", Level::Internal),
        field("/email", Level::Pii),
    ]);
    assert_eq!(
        step.egress_refusals(EgressCeiling::Internal),
        vec![refused("/email", Level::Pii)]
    );
}

#[test]
fn every_field_above_the_ceiling_is_named_not_only_the_first() {
    let step = write().with_egress(vec![
        field("/id", Level::Internal),
        field("/email", Level::Pii),
    ]);
    assert_eq!(
        step.egress_refusals(EgressCeiling::Public),
        vec![
            refused("/id", Level::Internal),
            refused("/email", Level::Pii)
        ]
    );
}

#[test]
fn a_field_at_the_ceiling_leaves_and_one_below_it_leaves() {
    let step = write().with_egress(vec![
        field("/id", Level::Public),
        field("/email", Level::Internal),
    ]);
    assert!(step.egress_refusals(EgressCeiling::Internal).is_empty());
    assert!(step.egress_refusals(EgressCeiling::Pii).is_empty());
}

#[test]
fn a_pii_ceiling_lets_a_pii_field_leave() {
    let step = write().with_egress(vec![field("/email", Level::Pii)]);
    assert!(step.egress_refusals(EgressCeiling::Pii).is_empty());
}

#[test]
fn a_secret_field_never_leaves_whatever_the_ceiling() {
    let step = write().with_egress(vec![
        field("/id", Level::Public),
        field("/email", Level::Secret),
    ]);
    for ceiling in [
        EgressCeiling::Public,
        EgressCeiling::Internal,
        EgressCeiling::Pii,
    ] {
        assert_eq!(
            step.egress_refusals(ceiling),
            vec![refused("/email", Level::Secret)],
            "ceiling {ceiling:?}"
        );
    }
}

#[test]
fn a_step_with_no_fields_to_write_has_nothing_to_refuse() {
    // The codec lists the whole record as one entry when the author lists no columns, so an empty list is the empty
    // table of a step that was classified, not an absence of classification.
    let step = write().with_egress(Vec::new());
    assert!(step.egress().is_some());
    assert!(step.egress_refusals(EgressCeiling::Public).is_empty());
}

#[test]
fn the_levels_belong_to_the_step_and_are_what_a_later_check_reads_back() {
    let levels = vec![field("/id", Level::Public), field("/email", Level::Pii)];
    let step = write().with_egress(levels.clone());
    assert_eq!(step.egress(), Some(levels.as_slice()));
    assert_eq!(levels[1].path(), "/email");
    assert_eq!(levels[1].level(), Level::Pii);
}

fn definition_of(nodes: Vec<Node>) -> WorkflowDefinition {
    let entrypoint = nodes.first().expect("a node").id().clone();
    WorkflowDefinition::new(Slug::parse("lake-flow").expect("name"), entrypoint, nodes)
        .expect("a valid definition")
}

#[test]
fn a_definition_gives_each_iceberg_step_the_levels_computed_for_it_and_leaves_other_nodes_alone() {
    let plain = Node::new(
        id("before"),
        NodeKind::Activity {
            activity: sagat_domain::workflow::ActivityName::parse("act.before").expect("name"),
            retry: sagat_domain::workflow::RetryPolicy::DEFAULT,
            compensate_with: None,
            reconcile_with: None,
            idempotency_template: None,
            secrets: Vec::new(),
        },
    )
    .with_next(id("save"));
    let definition = definition_of(vec![plain, Node::iceberg(id("save"), write())]);
    assert!(
        definition
            .nodes()
            .filter_map(|node| node.iceberg_write())
            .all(|step| step.egress().is_none()),
        "a definition built from nodes has no levels until someone computes them"
    );

    let mut asked = Vec::new();
    let definition = definition.with_iceberg_egress(|node, step| {
        asked.push((node.as_str().to_owned(), step.mapping().len()));
        vec![field("/email", Level::Pii)]
    });

    assert_eq!(
        asked,
        vec![("save".to_owned(), 2)],
        "asked once, for the Iceberg step only"
    );
    let steps: Vec<_> = definition
        .nodes()
        .filter_map(|node| node.iceberg_write())
        .collect();
    assert_eq!(steps.len(), 1);
    assert_eq!(
        steps[0].egress(),
        Some([field("/email", Level::Pii)].as_slice())
    );
}
