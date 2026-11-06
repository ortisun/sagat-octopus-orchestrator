//! The `icebergWrite` workflow node (feature 035: T035-047, T035-052; FR-035-008, FR-035-009, FR-035-034, G035-019).
//!
//! Domain only: the node, its bounds, its place in a definition and its effect. The IR codec, the publication checks and
//! the run-time admission are the next tasks (T035-053 to T035-055). Only the append mode is built here; overwrite and
//! upsert arrive with their user stories (T035-080, T035-085).
#![allow(clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::{expr, id, sid};
use sagat_domain::{
    database::{
        ColumnMapping, ConnectionRef, ICEBERG_REVERT_ACTIVITY, ICEBERG_WRITE_ACTIVITY,
        IcebergWrite, IcebergWriteError, IcebergWriteParts, LakeLimits, MAX_MAPPED_COLUMNS,
        MAX_MAPPING_DEPTH, MAX_RECORD_BLOBS, RecordSource, TableIdentifier, WriteIntent,
    },
    identity::Slug,
    saga::EffectPolicy,
    workflow::{
        ActivityName, BodyDefinition, ContinuationPolicy, Node, NodeKind, RetryPolicy,
        WorkflowDefinition, WorkflowError, WorkflowLimits,
    },
};

fn table() -> TableIdentifier {
    TableIdentifier::new(vec!["sales".to_owned()], "orders").expect("a valid table")
}

fn mapping() -> Vec<ColumnMapping> {
    vec![
        ColumnMapping::new("/id", "id").expect("mapping"),
        ColumnMapping::new("/customer/name", "customer_name").expect("mapping"),
    ]
}

fn parts() -> IcebergWriteParts {
    IcebergWriteParts {
        connection: ConnectionRef::new(Slug::parse("lake-main").expect("name"), None).expect("ref"),
        table: table(),
        reference: "main".to_owned(),
        intent: WriteIntent::Append,
        records: RecordSource::Inline(expr("input.orders")),
        mapping: mapping(),
        ignore_unknown_fields: false,
        limits: LakeLimits::default(),
        revert: false,
    }
}

fn write() -> IcebergWrite {
    IcebergWrite::new(parts()).expect("a valid write")
}

fn definition(node: Node) -> Result<WorkflowDefinition, WorkflowError> {
    WorkflowDefinition::new(
        Slug::parse("lake-flow").expect("name"),
        id("save"),
        vec![
            node.with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
    )
}

fn save() -> Node {
    Node::iceberg(id("save"), write())
}

// ---- The node and its place --------------------------------------------------------------------------------------

#[test]
fn an_iceberg_node_is_an_activity_on_the_reserved_name_and_a_definition_accepts_it() {
    let built = definition(save()).expect("a definition with an iceberg write");
    let node = built.node(&id("save")).expect("the node");
    let NodeKind::Activity {
        activity,
        retry,
        compensate_with,
        ..
    } = node.kind()
    else {
        panic!("an iceberg write is an activity");
    };
    assert_eq!(activity.as_str(), ICEBERG_WRITE_ACTIVITY);
    assert_eq!(ICEBERG_WRITE_ACTIVITY, "sagat.database.iceberg.write");
    assert_eq!(
        *retry,
        RetryPolicy::DEFAULT,
        "recovery is lookup-backed, so retries are automatic"
    );
    assert!(compensate_with.is_none());
    assert_eq!(node.iceberg_write(), Some(&write()));
    assert!(node.database_operation().is_none());
}

#[test]
fn the_step_is_high_risk_and_irreversible_unless_it_declares_a_revert() {
    let plain = definition(save()).expect("definition");
    let effect = plain.node(&id("save")).expect("node").effect();
    assert!(effect.high_risk && effect.irreversible);

    let mut revertible = parts();
    revertible.revert = true;
    let revertible = definition(Node::iceberg(
        id("save"),
        IcebergWrite::new(revertible).expect("write"),
    ))
    .expect("definition");
    let node = revertible.node(&id("save")).expect("node");
    assert!(
        node.effect().high_risk,
        "a revert does not make a commit safe"
    );
    assert!(!node.effect().irreversible);
    let NodeKind::Activity {
        compensate_with, ..
    } = node.kind()
    else {
        panic!("an activity");
    };
    assert_eq!(
        compensate_with.as_ref().map(ActivityName::as_str),
        Some(ICEBERG_REVERT_ACTIVITY)
    );
    assert_eq!(ICEBERG_REVERT_ACTIVITY, "sagat.database.iceberg.revert");
}

#[test]
fn a_revert_is_a_derived_compensation_that_runs_the_reserved_revert_activity() {
    let mut revertible = parts();
    revertible.revert = true;
    let built = definition(Node::iceberg(
        id("save"),
        IcebergWrite::new(revertible).expect("write"),
    ))
    .expect("definition");
    let derived = built
        .node(&sid("save~c"))
        .expect("the derived compensation");
    let NodeKind::Activity { activity, .. } = derived.kind() else {
        panic!("a compensation is an activity");
    };
    assert_eq!(activity.as_str(), ICEBERG_REVERT_ACTIVITY);
    assert!(
        derived.iceberg_write().is_none(),
        "the pin is found through the forward step, not copied"
    );
}

#[test]
fn a_step_without_a_revert_has_no_derived_compensation() {
    let built = definition(save()).expect("definition");
    assert!(built.node(&sid("save~c")).is_none());
}

#[test]
fn an_author_cannot_loosen_the_effect_of_an_iceberg_node() {
    assert!(definition(save().with_effect(EffectPolicy::NONE)).is_err());
    let mut only_high_risk = EffectPolicy::NONE;
    only_high_risk.high_risk = true;
    assert!(definition(save().with_effect(only_high_risk)).is_err());
}

#[test]
fn an_ordinary_activity_cannot_forge_the_reserved_iceberg_names() {
    let forged = |activity: &str, compensate_with: Option<&str>| {
        Node::new(
            id("save"),
            NodeKind::Activity {
                activity: ActivityName::parse(activity).expect("activity name"),
                retry: RetryPolicy::DEFAULT,
                compensate_with: compensate_with
                    .map(|name| ActivityName::parse(name).expect("name")),
                reconcile_with: None,
                idempotency_template: None,
                secrets: Vec::new(),
            },
        )
    };
    assert_eq!(
        definition(forged(ICEBERG_WRITE_ACTIVITY, None)).err(),
        Some(WorkflowError::InvalidDatabaseNode)
    );
    assert_eq!(
        definition(forged("orders.save", Some(ICEBERG_REVERT_ACTIVITY))).err(),
        Some(WorkflowError::InvalidDatabaseNode)
    );
    assert_eq!(
        definition(forged("sagat.database.iceberg.other", None)).err(),
        Some(WorkflowError::InvalidDatabaseNode)
    );
}

// ---- Root only (G035-019) ----------------------------------------------------------------------------------------

fn iteration_body() -> BodyDefinition {
    BodyDefinition::new(
        id("iteration"),
        id("check"),
        vec![
            Node::new(
                id("check"),
                NodeKind::Activity {
                    activity: ActivityName::parse("loop.check").expect("activity"),
                    retry: RetryPolicy::DEFAULT,
                    compensate_with: None,
                    reconcile_with: None,
                    idempotency_template: None,
                    secrets: Vec::new(),
                },
            )
            .with_next(id("returned")),
            Node::new(
                id("returned"),
                NodeKind::Return {
                    result: expr("steps.check.output"),
                },
            ),
        ],
    )
    .expect("loop body")
}

fn scoped_root(kind: &str) -> NodeKind {
    match kind {
        "while" => NodeKind::While {
            condition: expr("carry < 2"),
            body: id("iteration"),
            initial: expr("0"),
            carry: expr("bodyResult"),
            max_iterations: 2,
            delay: None,
        },
        "repeatUntil" => NodeKind::RepeatUntil {
            condition: expr("carry >= 2"),
            body: id("iteration"),
            initial: expr("0"),
            carry: expr("bodyResult"),
            max_iterations: 2,
            delay: None,
        },
        _ => NodeKind::ForEach {
            items: expr("input.items"),
            body: id("iteration"),
            result: expr("bodyResult"),
            max_items: 4,
            max_concurrency: 1,
        },
    }
}

#[test]
fn an_iceberg_node_is_refused_beside_a_for_each_while_or_repeat_until_at_the_root() {
    for kind in ["while", "repeatUntil", "forEach"] {
        let outcome = WorkflowDefinition::new_structured(
            Slug::parse("scoped-lake").expect("name"),
            id("loop"),
            vec![
                Node::new(id("loop"), scoped_root(kind))
                    .with_timeout(Duration::from_secs(60))
                    .with_next(id("save")),
                save().with_next(id("done")),
                Node::new(id("done"), NodeKind::End { output: None }),
            ],
            vec![iteration_body()],
            WorkflowLimits::default(),
        );
        assert_eq!(
            outcome.err(),
            Some(WorkflowError::IcebergNodeNotRoot),
            "{kind}"
        );
    }
}

#[test]
fn an_iceberg_node_is_refused_inside_a_body() {
    let body = BodyDefinition::new(
        id("iteration"),
        id("save"),
        vec![
            save().with_next(id("returned")),
            Node::new(
                id("returned"),
                NodeKind::Return {
                    result: expr("steps.save.output"),
                },
            ),
        ],
    )
    .expect("body");
    let outcome = WorkflowDefinition::new_structured(
        Slug::parse("body-lake").expect("name"),
        id("loop"),
        vec![
            Node::new(id("loop"), scoped_root("forEach")).with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body],
        WorkflowLimits::default(),
    );
    assert_eq!(outcome.err(), Some(WorkflowError::IcebergNodeNotRoot));
}

#[test]
fn an_iceberg_node_is_refused_under_a_continuation_policy() {
    let outcome = WorkflowDefinition::new_structured_with_continuation(
        Slug::parse("continued-lake").expect("name"),
        id("save"),
        vec![
            save().with_next(id("continue")),
            Node::new(
                id("continue"),
                NodeKind::ContinueAsNew {
                    input: expr("result.nextInput"),
                    state: None,
                },
            ),
        ],
        Vec::new(),
        WorkflowLimits::default(),
        ContinuationPolicy::new(3, Duration::from_secs(60)).expect("policy"),
    );
    assert_eq!(outcome.err(), Some(WorkflowError::IcebergNodeNotRoot));
}

#[test]
fn an_iceberg_node_is_allowed_beside_the_nodes_that_stay_at_the_root() {
    let parallel = Node::new(
        id("fork"),
        NodeKind::Parallel {
            branches: vec![id("save"), id("other")],
            join: sagat_domain::workflow::JoinPolicy::All,
        },
    );
    let other = Node::new(
        id("other"),
        NodeKind::Activity {
            activity: ActivityName::parse("orders.notify").expect("activity"),
            retry: RetryPolicy::DEFAULT,
            compensate_with: None,
            reconcile_with: None,
            idempotency_template: None,
            secrets: Vec::new(),
        },
    );
    let built = WorkflowDefinition::new_structured(
        Slug::parse("parallel-lake").expect("name"),
        id("fork"),
        vec![parallel, save(), other],
        Vec::new(),
        WorkflowLimits::default(),
    );
    assert!(built.is_ok(), "{:?}", built.err());
}

// ---- Record sources, mapping and the other bounds -----------------------------------------------------------------

#[test]
fn a_step_has_exactly_one_record_source() {
    for records in [
        RecordSource::Inline(expr("input.orders")),
        RecordSource::Blobs(expr("steps.export.output.blobs")),
        RecordSource::DatabaseResult(id("read")),
    ] {
        let mut candidate = parts();
        candidate.records = records.clone();
        assert_eq!(
            IcebergWrite::new(candidate).expect("a write").records(),
            &records
        );
    }
    assert_eq!(
        MAX_RECORD_BLOBS, 32,
        "a step names at most 32 record blobs; the runner and publication enforce it"
    );
}

#[test]
fn the_mapping_stays_within_its_hard_ceilings() {
    assert_eq!((MAX_MAPPED_COLUMNS, MAX_MAPPING_DEPTH), (1024, 16));
    let columns = |count: usize| -> Vec<ColumnMapping> {
        (0..count)
            .map(|index| {
                ColumnMapping::new(&format!("/f{index}"), &format!("c{index}")).expect("mapping")
            })
            .collect()
    };
    let mut at = parts();
    at.mapping = columns(1024);
    assert!(IcebergWrite::new(at).is_ok());
    let mut past = parts();
    past.mapping = columns(1025);
    assert_eq!(
        IcebergWrite::new(past).err(),
        Some(IcebergWriteError::TooManyColumns)
    );

    let pointer = |depth: usize| format!("/{}", vec!["a"; depth].join("/"));
    assert!(ColumnMapping::new(&pointer(16), "deep").is_ok());
    assert_eq!(
        ColumnMapping::new(&pointer(17), "deeper").err(),
        Some(IcebergWriteError::MappingTooDeep)
    );
}

#[test]
fn a_field_and_a_column_are_each_mapped_once() {
    let mut twice_field = parts();
    twice_field.mapping = vec![
        ColumnMapping::new("/id", "id").expect("mapping"),
        ColumnMapping::new("/id", "other").expect("mapping"),
    ];
    assert_eq!(
        IcebergWrite::new(twice_field).err(),
        Some(IcebergWriteError::DuplicateField)
    );
    let mut twice_column = parts();
    twice_column.mapping = vec![
        ColumnMapping::new("/id", "id").expect("mapping"),
        ColumnMapping::new("/key", "id").expect("mapping"),
    ];
    assert_eq!(
        IcebergWrite::new(twice_column).err(),
        Some(IcebergWriteError::DuplicateColumn)
    );
}

#[test]
fn a_field_is_a_json_pointer_and_a_column_is_a_plain_name() {
    for bad in ["", "id", "/", "//a", "/a//b", "/a/", "/a\0b"] {
        assert_eq!(
            ColumnMapping::new(bad, "c").err(),
            Some(IcebergWriteError::Field),
            "{bad:?}"
        );
    }
    assert_eq!(
        ColumnMapping::new(&format!("/{}", "a".repeat(512)), "c").err(),
        Some(IcebergWriteError::Field)
    );
    assert!(ColumnMapping::new(&format!("/{}", "a".repeat(511)), "c").is_ok());
    for bad in ["", "a\nb", "a\0b"] {
        assert_eq!(
            ColumnMapping::new("/a", bad).err(),
            Some(IcebergWriteError::Column),
            "{bad:?}"
        );
    }
    assert_eq!(
        ColumnMapping::new("/a", &"c".repeat(256)).err(),
        Some(IcebergWriteError::Column)
    );
    assert!(ColumnMapping::new("/a", &"c".repeat(255)).is_ok());
}

#[test]
fn a_table_names_one_to_eight_namespace_levels_and_a_name_within_255_bytes() {
    let level = |text: &str| text.to_owned();
    assert!(TableIdentifier::new((0..8).map(|_| level("n")).collect(), "t").is_ok());
    assert_eq!(
        TableIdentifier::new((0..9).map(|_| level("n")).collect(), "t").err(),
        Some(IcebergWriteError::Table)
    );
    assert_eq!(
        TableIdentifier::new(Vec::new(), "t").err(),
        Some(IcebergWriteError::Table)
    );
    assert_eq!(
        TableIdentifier::new(vec![level("")], "t").err(),
        Some(IcebergWriteError::Table)
    );
    assert_eq!(
        TableIdentifier::new(vec!["a".repeat(256)], "t").err(),
        Some(IcebergWriteError::Table)
    );
    assert!(TableIdentifier::new(vec!["a".repeat(255)], &"t".repeat(255)).is_ok());
    assert_eq!(
        TableIdentifier::new(vec![level("n")], "").err(),
        Some(IcebergWriteError::Table)
    );
    assert_eq!(
        TableIdentifier::new(vec![level("n")], &"t".repeat(256)).err(),
        Some(IcebergWriteError::Table)
    );
    assert_eq!(
        TableIdentifier::new(vec![level("n\n")], "t").err(),
        Some(IcebergWriteError::Table)
    );
    assert_eq!(
        TableIdentifier::new(vec![level("n")], "t\0").err(),
        Some(IcebergWriteError::Table)
    );
}

#[test]
fn a_target_ref_is_one_to_255_characters_without_control_characters() {
    for ok in ["main", "audit-branch", &"r".repeat(255)] {
        let mut candidate = parts();
        candidate.reference = ok.to_owned();
        assert!(IcebergWrite::new(candidate).is_ok(), "{ok:?}");
    }
    for bad in ["", "a\nb", &"r".repeat(256)] {
        let mut candidate = parts();
        candidate.reference = bad.to_owned();
        assert_eq!(
            IcebergWrite::new(candidate).err(),
            Some(IcebergWriteError::Reference),
            "{bad:?}"
        );
    }
}

#[test]
fn a_step_built_by_hand_around_a_zero_revision_is_refused() {
    let mut candidate = parts();
    candidate.connection = ConnectionRef {
        name: Slug::parse("lake-main").expect("name"),
        revision: Some(0),
    };
    assert_eq!(
        IcebergWrite::new(candidate).err(),
        Some(IcebergWriteError::Connection)
    );
    let mut pinned = parts();
    pinned.connection =
        ConnectionRef::new(Slug::parse("lake-main").expect("name"), Some(7)).expect("ref");
    assert!(IcebergWrite::new(pinned).is_ok());
}

#[test]
fn a_connection_revision_of_zero_is_not_a_revision() {
    assert!(ConnectionRef::new(Slug::parse("lake-main").expect("name"), Some(0)).is_err());
}

#[test]
fn the_limits_of_a_step_are_the_ones_of_a_connection() {
    let mut inside = parts();
    inside.limits = LakeLimits {
        max_record_bytes: Some(1024),
        max_records: Some(100_000_000),
        timeout_ms: Some(3_600_000),
    };
    assert!(IcebergWrite::new(inside).is_ok());
    for limits in [
        LakeLimits {
            max_record_bytes: Some(1023),
            ..LakeLimits::default()
        },
        LakeLimits {
            max_records: Some(0),
            ..LakeLimits::default()
        },
        LakeLimits {
            timeout_ms: Some(999),
            ..LakeLimits::default()
        },
        LakeLimits {
            timeout_ms: Some(3_600_001),
            ..LakeLimits::default()
        },
    ] {
        let mut outside = parts();
        outside.limits = limits;
        assert_eq!(
            IcebergWrite::new(outside).err(),
            Some(IcebergWriteError::Limits),
            "{limits:?}"
        );
    }
}

#[test]
fn a_definition_weighs_what_its_iceberg_step_holds() {
    let small = definition(save()).expect("small");
    let mut larger = parts();
    larger.mapping = (0..100)
        .map(|index| {
            ColumnMapping::new(&format!("/field{index:03}"), &format!("column{index:03}"))
                .expect("mapping")
        })
        .collect();
    let large = definition(Node::iceberg(
        id("save"),
        IcebergWrite::new(larger).expect("write"),
    ))
    .expect("large");
    assert!(large.footprint() > small.footprint());
    let extra = large.footprint() - small.footprint();
    assert!(
        extra >= 100 * ("/field000".len() + "column000".len()) - 2 * 40,
        "{extra}"
    );
}

#[test]
fn the_ignore_unknown_fields_flag_is_kept() {
    let mut candidate = parts();
    candidate.ignore_unknown_fields = true;
    assert!(
        IcebergWrite::new(candidate)
            .expect("write")
            .ignore_unknown_fields()
    );
    assert!(!write().ignore_unknown_fields());
}

#[test]
fn the_weight_of_a_step_is_exactly_the_bytes_it_holds() {
    let weight =
        |candidate: IcebergWriteParts| IcebergWrite::new(candidate).expect("write").footprint();
    let mut bare = parts();
    bare.mapping = Vec::new();
    let base = weight(bare.clone());

    let mut longer_name = bare.clone();
    longer_name.table =
        TableIdentifier::new(vec!["sales".to_owned()], &"o".repeat(6 + 10)).expect("table");
    assert_eq!(weight(longer_name), base + 10, "the table name");

    let mut deeper = bare.clone();
    deeper.table =
        TableIdentifier::new(vec!["sales".to_owned(), "eu".to_owned()], "orders").expect("table");
    assert_eq!(weight(deeper), base + 2, "a namespace level");

    let mut longer_ref = bare.clone();
    longer_ref.reference = "main-audit".to_owned();
    assert_eq!(weight(longer_ref), base + 6, "the target ref");

    let mut longer_connection = bare.clone();
    longer_connection.connection =
        ConnectionRef::new(Slug::parse("lake-main-two").expect("name"), None).expect("ref");
    assert_eq!(weight(longer_connection), base + 4, "the connection name");

    let mut longer_expression = bare.clone();
    longer_expression.records = RecordSource::Inline(expr("input.orders.items"));
    assert_eq!(weight(longer_expression), base + 6, "an inline expression");
    let mut blobs = bare.clone();
    blobs.records = RecordSource::Blobs(expr("input.orders"));
    assert_eq!(
        weight(blobs),
        base,
        "a blobs expression weighs what an inline one does"
    );
    let mut result = bare.clone();
    result.records = RecordSource::DatabaseResult(id("read"));
    assert_eq!(
        weight(result),
        base - "input.orders".len() + "read".len(),
        "a database result names a node"
    );

    let mut mapped = bare;
    mapped.mapping = vec![ColumnMapping::new("/id", "id").expect("mapping")];
    assert_eq!(
        weight(mapped),
        base + std::mem::size_of::<ColumnMapping>() + "/id".len() + "id".len(),
        "one mapped column"
    );
}
