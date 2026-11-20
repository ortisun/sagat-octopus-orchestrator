//! The probe of an Iceberg connection, as a node of a definition (feature 035: T035-045, D035-150 slice S1, ADR-0080).
//!
//! Domain only: the node, the kind that makes it managed, and its place in a definition. A probe has no table, mapping or records, only
//! the connection it tests; it does not retry and it has no effect. It is a **third kind of managed step**: every place that asks
//! whether a step is managed asks the kind, so these tests also hold that the probe is neither a SQL operation nor an Iceberg write.
#![allow(clippy::expect_used, clippy::panic)]

mod common;

use common::{expr, id};
use sagat_domain::{
    binding::BindingProfile,
    database::{
        ConnectionRef, DatabaseFailure, ICEBERG_PROBE_ACTIVITY, ICEBERG_WRITE_ACTIVITY, LakeProbe,
        ManagedKind, ProbeCheck, ProbeCheckResult, ProbeOutcome, ProbeResult,
    },
    identity::Slug,
    saga::EffectPolicy,
    workflow::{
        ActivityName, BodyDefinition, ContinuationPolicy, Node, NodeKind, RetryPolicy,
        WorkflowDefinition, WorkflowError, WorkflowLimits,
    },
};
use std::time::Duration;

fn probe() -> LakeProbe {
    LakeProbe::new(
        ConnectionRef::new(Slug::parse("lake-main").expect("slug"), Some(3)).expect("ref"),
    )
}

fn probe_node() -> Node {
    Node::lake_probe(id("probe"), probe())
}

fn definition(node: Node) -> Result<WorkflowDefinition, WorkflowError> {
    WorkflowDefinition::new(
        Slug::parse("database-probe").expect("name"),
        id("probe"),
        vec![node],
    )
}

#[test]
fn a_probe_node_is_an_activity_on_the_reserved_name_that_never_retries_and_has_no_effect() {
    let built = definition(probe_node()).expect("a definition of one probe node");
    let node = built.node(&id("probe")).expect("the node");
    let NodeKind::Activity {
        activity,
        retry,
        compensate_with,
        reconcile_with,
        idempotency_template,
        secrets,
    } = node.kind()
    else {
        panic!("a probe is an activity");
    };
    assert_eq!(activity.as_str(), ICEBERG_PROBE_ACTIVITY);
    assert_eq!(*retry, RetryPolicy::no_retry());
    assert!(compensate_with.is_none() && reconcile_with.is_none());
    assert!(idempotency_template.is_none() && secrets.is_empty());
    assert_eq!(
        *node.effect(),
        EffectPolicy::NONE,
        "a probe changes nothing"
    );
}

#[test]
fn a_probe_is_a_managed_step_of_the_lake_profile_and_is_neither_a_sql_operation_nor_a_write() {
    let node = probe_node();
    assert_eq!(node.managed_kind(), Some(ManagedKind::LakeProbe));
    assert_eq!(ManagedKind::LakeProbe.profile(), BindingProfile::Lake);
    assert!(node.database_operation().is_none());
    assert!(node.iceberg_write().is_none());
    let tested = node.lake_probe_target().expect("the probe");
    assert_eq!(tested.connection().name.as_str(), "lake-main");
    assert_eq!(tested.connection().revision, Some(3));
    // The reserved names are three, and each is its own kind's.
    assert_ne!(ICEBERG_PROBE_ACTIVITY, ICEBERG_WRITE_ACTIVITY);
}

#[test]
fn the_reserved_probe_name_without_its_payload_is_refused() {
    let named = Node::new(
        id("probe"),
        NodeKind::Activity {
            activity: ActivityName::parse(ICEBERG_PROBE_ACTIVITY).expect("activity"),
            retry: RetryPolicy::no_retry(),
            compensate_with: None,
            reconcile_with: None,
            idempotency_template: None,
            secrets: Vec::new(),
        },
    );
    assert_eq!(
        definition(named).err(),
        Some(WorkflowError::InvalidDatabaseNode),
        "an author cannot name a managed activity and carry no connection"
    );
}

fn loop_body() -> BodyDefinition {
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

fn for_each() -> NodeKind {
    NodeKind::ForEach {
        items: expr("input.items"),
        body: id("iteration"),
        result: expr("bodyResult"),
        max_items: 4,
        max_concurrency: 1,
    }
}

#[test]
fn a_probe_is_root_only_like_every_managed_step() {
    // beside a collection at the root
    let beside = WorkflowDefinition::new_structured(
        Slug::parse("scoped-probe").expect("name"),
        id("loop"),
        vec![
            Node::new(id("loop"), for_each())
                .with_timeout(Duration::from_secs(60))
                .with_next(id("probe")),
            probe_node().with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![loop_body()],
        WorkflowLimits::default(),
    );
    assert_eq!(beside.err(), Some(WorkflowError::IcebergNodeNotRoot));
    // inside a body
    let body = BodyDefinition::new(
        id("iteration"),
        id("probe"),
        vec![
            probe_node().with_next(id("returned")),
            Node::new(
                id("returned"),
                NodeKind::Return {
                    result: expr("steps.probe.output"),
                },
            ),
        ],
    )
    .expect("body");
    let inside = WorkflowDefinition::new_structured(
        Slug::parse("body-probe").expect("name"),
        id("loop"),
        vec![
            Node::new(id("loop"), for_each()).with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body],
        WorkflowLimits::default(),
    );
    assert_eq!(inside.err(), Some(WorkflowError::IcebergNodeNotRoot));
    // under a continuation policy
    let continued = WorkflowDefinition::new_structured_with_continuation(
        Slug::parse("continued-probe").expect("name"),
        id("probe"),
        vec![
            probe_node().with_next(id("continue")),
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
    assert_eq!(continued.err(), Some(WorkflowError::IcebergNodeNotRoot));
}

#[test]
fn every_managed_kind_has_a_profile_and_the_lake_kinds_are_the_lake_profile() {
    for (kind, profile) in [
        (ManagedKind::Sql, BindingProfile::Database),
        (ManagedKind::Lake, BindingProfile::Lake),
        (ManagedKind::LakeProbe, BindingProfile::Lake),
    ] {
        assert_eq!(kind.profile(), profile, "{kind:?}");
    }
}

#[test]
fn a_probe_that_claims_an_effect_is_refused_because_a_probe_changes_nothing() {
    for effect in [
        EffectPolicy {
            high_risk: true,
            ..EffectPolicy::NONE
        },
        EffectPolicy {
            irreversible: true,
            ..EffectPolicy::NONE
        },
    ] {
        assert_eq!(
            definition(probe_node().with_effect(effect)).err(),
            Some(WorkflowError::InvalidIcebergNode),
            "{effect:?}"
        );
    }
}

#[test]
fn the_checks_of_a_probe_are_a_closed_vocabulary_in_a_fixed_order() {
    // The names are the contract's (`contracts/connections.md`, `ConnectionProbeResult.checks[]`), and the order is the order they run:
    // a check that cannot run because an earlier one failed is skipped, so the order is part of what a result means.
    let names: Vec<&str> = ProbeCheck::ALL.iter().map(|check| check.as_str()).collect();
    assert_eq!(names, ["CATALOG_AUTH", "WAREHOUSE", "STORAGE_ACCESS"]);
    for check in ProbeCheck::ALL {
        assert_eq!(ProbeCheck::parse(check.as_str()), Some(check));
    }
    for not_a_check in [
        "",
        "catalog_auth",
        "CATALOG_AUTH ",
        "DATABASE_AUTH",
        "CONDITIONAL_CREATE",
        "TABLE_DEDUPLICATION",
    ] {
        assert_eq!(
            ProbeCheck::parse(not_a_check),
            None,
            "{not_a_check:?} is not a check of an Iceberg probe"
        );
    }
}

// ---- The result of a probe (D035-150 slice S4a) ----------------------------------------------------------------------------------

fn passed(check: ProbeCheck) -> ProbeCheckResult {
    ProbeCheckResult::new(check, ProbeOutcome::Passed, None)
}

fn skipped(check: ProbeCheck) -> ProbeCheckResult {
    ProbeCheckResult::new(check, ProbeOutcome::Skipped, None)
}

fn failed(check: ProbeCheck, code: DatabaseFailure) -> ProbeCheckResult {
    ProbeCheckResult::new(check, ProbeOutcome::Failed, Some(code))
}

use ProbeCheck::{CatalogAuth, StorageAccess, Warehouse};

#[test]
fn a_result_is_the_three_checks_in_order_and_each_ends_as_the_checks_before_it_allow() {
    // everything passed
    let all = ProbeResult::new(vec![
        passed(CatalogAuth),
        passed(Warehouse),
        passed(StorageAccess),
    ])
    .expect("all passed");
    assert_eq!(all.first_failure(), None);
    // a check that failed, and what could not run after it
    for (failure, first) in [
        (
            vec![
                failed(CatalogAuth, DatabaseFailure::Authentication),
                skipped(Warehouse),
                skipped(StorageAccess),
            ],
            DatabaseFailure::Authentication,
        ),
        (
            vec![
                passed(CatalogAuth),
                failed(Warehouse, DatabaseFailure::IcebergLocationNotAllowed),
                skipped(StorageAccess),
            ],
            DatabaseFailure::IcebergLocationNotAllowed,
        ),
        (
            vec![
                passed(CatalogAuth),
                passed(Warehouse),
                failed(StorageAccess, DatabaseFailure::StorageUnavailable),
            ],
            DatabaseFailure::StorageUnavailable,
        ),
    ] {
        let result = ProbeResult::new(failure).expect("a failed check, the rest skipped");
        assert_eq!(result.first_failure(), Some(first));
    }
    // the storage check is skipped without a failure when the connection vends its credentials: nothing is vended without a table
    let vended = ProbeResult::new(vec![
        passed(CatalogAuth),
        passed(Warehouse),
        skipped(StorageAccess),
    ])
    .expect("storage skipped on vended credentials");
    assert_eq!(vended.first_failure(), None);
}

#[test]
fn a_result_that_is_not_the_three_checks_in_order_is_refused() {
    let bad: Vec<(&str, Vec<ProbeCheckResult>)> = vec![
        ("none", vec![]),
        ("one", vec![passed(CatalogAuth)]),
        ("two", vec![passed(CatalogAuth), passed(Warehouse)]),
        (
            "four",
            vec![
                passed(CatalogAuth),
                passed(Warehouse),
                passed(StorageAccess),
                passed(StorageAccess),
            ],
        ),
        (
            "out of order",
            vec![
                passed(Warehouse),
                passed(CatalogAuth),
                passed(StorageAccess),
            ],
        ),
        (
            "a check twice",
            vec![
                passed(CatalogAuth),
                passed(CatalogAuth),
                passed(StorageAccess),
            ],
        ),
    ];
    for (name, checks) in bad {
        assert!(ProbeResult::new(checks).is_err(), "{name}");
    }
}

#[test]
fn a_failed_check_has_a_code_that_is_allowed_for_it_and_no_other_has_one() {
    // a failed check without a code, and a passed or skipped one with a code
    assert!(
        ProbeResult::new(vec![
            ProbeCheckResult::new(CatalogAuth, ProbeOutcome::Failed, None),
            skipped(Warehouse),
            skipped(StorageAccess),
        ])
        .is_err()
    );
    assert!(
        ProbeResult::new(vec![
            ProbeCheckResult::new(
                CatalogAuth,
                ProbeOutcome::Passed,
                Some(DatabaseFailure::Tls)
            ),
            passed(Warehouse),
            passed(StorageAccess),
        ])
        .is_err()
    );
    assert!(
        ProbeResult::new(vec![
            failed(CatalogAuth, DatabaseFailure::Authentication),
            ProbeCheckResult::new(
                Warehouse,
                ProbeOutcome::Skipped,
                Some(DatabaseFailure::Timeout)
            ),
            skipped(StorageAccess),
        ])
        .is_err()
    );
    // the codes each check may fail with are closed, and a code that says which side failed is not allowed on the other
    for (check, allowed) in [
        (
            CatalogAuth,
            vec![
                DatabaseFailure::Authentication,
                DatabaseFailure::Tls,
                DatabaseFailure::DestinationDenied,
                DatabaseFailure::CatalogUnavailable,
                DatabaseFailure::Timeout,
            ],
        ),
        (
            Warehouse,
            vec![
                DatabaseFailure::Authentication,
                DatabaseFailure::CatalogUnavailable,
                DatabaseFailure::IcebergLocationNotAllowed,
                DatabaseFailure::Timeout,
            ],
        ),
        (
            StorageAccess,
            vec![
                DatabaseFailure::Authentication,
                DatabaseFailure::Tls,
                DatabaseFailure::DestinationDenied,
                DatabaseFailure::StorageUnavailable,
                DatabaseFailure::Timeout,
            ],
        ),
    ] {
        for code in DatabaseFailure::ALL {
            assert_eq!(
                check.allows(*code),
                allowed.contains(code),
                "{} may{} fail with {}",
                check.as_str(),
                if allowed.contains(code) { "" } else { " not" },
                code.code()
            );
        }
    }
}

#[test]
fn a_check_after_a_failure_is_skipped_and_a_skip_needs_a_cause() {
    // after a failure, every later check is skipped
    assert!(
        ProbeResult::new(vec![
            failed(CatalogAuth, DatabaseFailure::Authentication),
            passed(Warehouse),
            skipped(StorageAccess),
        ])
        .is_err()
    );
    assert!(
        ProbeResult::new(vec![
            failed(CatalogAuth, DatabaseFailure::Authentication),
            skipped(Warehouse),
            passed(StorageAccess),
        ])
        .is_err()
    );
    assert!(
        ProbeResult::new(vec![
            passed(CatalogAuth),
            failed(Warehouse, DatabaseFailure::CatalogUnavailable),
            passed(StorageAccess),
        ])
        .is_err()
    );
    // a skip is the consequence of an earlier failure, except the storage check's on vended credentials
    assert!(
        ProbeResult::new(vec![
            skipped(CatalogAuth),
            passed(Warehouse),
            passed(StorageAccess)
        ])
        .is_err()
    );
    assert!(
        ProbeResult::new(vec![
            passed(CatalogAuth),
            skipped(Warehouse),
            passed(StorageAccess)
        ])
        .is_err()
    );
    assert!(
        ProbeResult::new(vec![
            passed(CatalogAuth),
            skipped(Warehouse),
            skipped(StorageAccess)
        ])
        .is_err()
    );
}

#[test]
fn the_outcomes_are_a_closed_vocabulary() {
    for (outcome, name) in [
        (ProbeOutcome::Passed, "PASSED"),
        (ProbeOutcome::Failed, "FAILED"),
        (ProbeOutcome::Skipped, "SKIPPED"),
    ] {
        assert_eq!(outcome.as_str(), name);
        assert_eq!(ProbeOutcome::parse(name), Some(outcome));
    }
    for not_an_outcome in ["", "passed", "WARNING", "OK", "PASSED "] {
        assert_eq!(
            ProbeOutcome::parse(not_an_outcome),
            None,
            "{not_an_outcome:?}"
        );
    }
}
