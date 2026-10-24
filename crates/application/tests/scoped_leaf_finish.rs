//! Public application contract only; no storage or source authority is established.
#![allow(clippy::expect_used)]

use std::collections::HashSet;

use sagat_application::{
    error::StoreError,
    leaf_certification::{LeafCollectionPathIdentity, LeafCollectionSuccessError},
    primitives::SchedulerAuthority,
    scoped_leaf_finish::*,
};
use sagat_domain::{
    audit::AuditId,
    execution::{
        CapacityGroupId, CapacityMemberId, CapacityPossession, InstancePath, MAX_ITERATION_INDEX,
        RunId,
    },
    identity::{NamespaceId, TenantId},
    workflow::NodeId,
};
use time::OffsetDateTime;
use uuid::Uuid;

const SIGNED_COUNTER_MAX: u64 = i64::MAX as u64;

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

#[allow(clippy::too_many_arguments)]
fn command(
    tenant: u128,
    namespace: u128,
    group: u128,
    member: u128,
    run: u128,
    owner: NodeId,
    index: u32,
    group_revision: u64,
    group_epoch: u64,
    run_revision: u64,
) -> ScopedLeafFinishCommand {
    let path = InstancePath::root()
        .child(owner, index)
        .expect("bounded path");
    let run_id = RunId::new(uuid(run));
    ScopedLeafFinishCommand {
        operation_key: uuid(12),
        group_id: CapacityGroupId::new(uuid(group)),
        member_id: CapacityMemberId::new(uuid(member)),
        identity: LeafCollectionPathIdentity::new(
            TenantId::new(uuid(tenant)),
            NamespaceId::new(uuid(namespace)),
            run_id,
            path,
        )
        .expect("depth-one identity"),
        expected: ScopedLeafFinishExpectations {
            group_revision,
            group_epoch,
            run_revision,
        },
        scheduler: SchedulerAuthority::for_run("scheduler-a", run_id, 13),
    }
}

fn declared(value: &str) -> NodeId {
    NodeId::parse(value).expect("declared node")
}

fn base_command() -> ScopedLeafFinishCommand {
    command(1, 2, 3, 4, 5, declared("items"), 7, 9, 3, 11)
}

#[test]
fn structural_validation_accepts_only_the_shared_depth_one_ordinary_path_bounds() {
    for (owner, index) in [("a".to_owned(), 0), ("z".repeat(64), 9_999)] {
        assert_eq!(
            command(1, 2, 3, 4, 5, declared(&owner), index, 1, 1, 1).validate(),
            Ok(())
        );
    }

    let mut derived = base_command();
    let run_id = derived.identity.run_id();
    derived.identity = LeafCollectionPathIdentity::new(
        derived.identity.tenant_id(),
        derived.identity.namespace_id(),
        run_id,
        InstancePath::root()
            .child(declared("items").compensation(), 7)
            .expect("internal derived path"),
    )
    .expect("depth-one identity accepts internal nodes");
    assert_eq!(
        derived.validate(),
        Err(ScopedLeafFinishError::UnsupportedPath)
    );
    assert_eq!(
        derived.fingerprint(),
        Err(ScopedLeafFinishError::UnsupportedPath)
    );

    let tenant = TenantId::new(uuid(1));
    let namespace = NamespaceId::new(uuid(2));
    assert!(matches!(
        LeafCollectionPathIdentity::new(tenant, namespace, run_id, InstancePath::root()),
        Err(LeafCollectionSuccessError::UnsupportedPath)
    ));
    let nested = InstancePath::root()
        .child(declared("outer"), 0)
        .expect("outer")
        .child(declared("inner"), 0)
        .expect("inner");
    assert!(matches!(
        LeafCollectionPathIdentity::new(tenant, namespace, run_id, nested),
        Err(LeafCollectionSuccessError::UnsupportedPath)
    ));
    assert!(
        InstancePath::root()
            .child(declared("items"), MAX_ITERATION_INDEX + 1)
            .is_err()
    );
}

#[test]
fn revision_validation_preserves_invalid_then_exhausted_error_precedence() {
    for expected in [
        ScopedLeafFinishExpectations {
            group_revision: 1,
            group_epoch: 1,
            run_revision: 1,
        },
        ScopedLeafFinishExpectations {
            group_revision: SIGNED_COUNTER_MAX,
            group_epoch: SIGNED_COUNTER_MAX,
            run_revision: SIGNED_COUNTER_MAX,
        },
    ] {
        let mut command = base_command();
        command.expected = expected;
        assert_eq!(command.validate(), Ok(()));
    }

    for expected in [
        ScopedLeafFinishExpectations {
            group_revision: 0,
            group_epoch: 1,
            run_revision: 1,
        },
        ScopedLeafFinishExpectations {
            group_revision: 1,
            group_epoch: 0,
            run_revision: 1,
        },
        ScopedLeafFinishExpectations {
            group_revision: 1,
            group_epoch: 1,
            run_revision: 0,
        },
        ScopedLeafFinishExpectations {
            group_revision: 1,
            group_epoch: 2,
            run_revision: 1,
        },
        // The relation error wins even though the epoch also exceeds PostgreSQL's range.
        ScopedLeafFinishExpectations {
            group_revision: 1,
            group_epoch: 1_u64 << 63,
            run_revision: 1,
        },
    ] {
        let mut command = base_command();
        command.expected = expected;
        assert_eq!(
            command.validate(),
            Err(ScopedLeafFinishError::InvalidRevision)
        );
        assert_eq!(
            command.fingerprint(),
            Err(ScopedLeafFinishError::InvalidRevision)
        );
    }

    for expected in [
        ScopedLeafFinishExpectations {
            group_revision: 1_u64 << 63,
            group_epoch: 1,
            run_revision: 1,
        },
        ScopedLeafFinishExpectations {
            group_revision: u64::MAX,
            group_epoch: 1_u64 << 63,
            run_revision: 1,
        },
        ScopedLeafFinishExpectations {
            group_revision: 1,
            group_epoch: 1,
            run_revision: 1_u64 << 63,
        },
    ] {
        let mut command = base_command();
        command.expected = expected;
        assert_eq!(
            command.validate(),
            Err(ScopedLeafFinishError::CounterExhausted)
        );
        assert_eq!(
            command.fingerprint(),
            Err(ScopedLeafFinishError::CounterExhausted)
        );
    }
}

#[test]
fn canonical_fingerprint_matches_the_independent_python_vector_and_domain() {
    let expected = [
        0xd3, 0x05, 0xb4, 0x8d, 0x52, 0x7e, 0x1c, 0xfa, 0x02, 0xa1, 0x98, 0x31, 0x1b, 0x7d, 0x4d,
        0x78, 0xd2, 0xa8, 0x5f, 0x19, 0x94, 0xd3, 0x31, 0x4e, 0x6c, 0xb6, 0x57, 0x3b, 0x06, 0x68,
        0x99, 0x3b,
    ];
    let child_root_domain_for_same_tail = [
        0x79, 0xa4, 0x4d, 0xba, 0x31, 0x3e, 0x67, 0x5e, 0xf7, 0xa4, 0xa7, 0xa7, 0x4b, 0x83, 0xef,
        0x81, 0xba, 0x8d, 0x4f, 0x94, 0xe1, 0x70, 0xcc, 0xd8, 0xa0, 0xeb, 0xc4, 0x37, 0x52, 0x8c,
        0xae, 0xe2,
    ];
    let no_owner_length = [
        0xfa, 0x5e, 0xd3, 0xb4, 0xb8, 0xf8, 0xa1, 0x62, 0x7d, 0x35, 0x5b, 0x27, 0xed, 0x6b, 0x3f,
        0x50, 0x98, 0xed, 0xe1, 0x66, 0xec, 0x8f, 0xf3, 0x27, 0x0e, 0x3c, 0xff, 0xe2, 0x49, 0x2e,
        0xe6, 0x96,
    ];
    let little_endian_owner_length = [
        0x8f, 0xfa, 0xe3, 0x9e, 0x1a, 0x72, 0x9e, 0x5e, 0x55, 0xd8, 0x9a, 0x0d, 0x0a, 0x05, 0x94,
        0x4d, 0xeb, 0xff, 0xef, 0xd2, 0x57, 0x75, 0x36, 0x3c, 0x85, 0xd2, 0xe4, 0xa5, 0x9e, 0x65,
        0x69, 0x60,
    ];

    let actual = base_command().fingerprint().expect("valid fingerprint");
    assert_eq!(actual, expected);
    assert_ne!(actual, child_root_domain_for_same_tail);
    assert_ne!(actual, no_owner_length);
    assert_ne!(actual, little_endian_owner_length);
}

#[test]
fn every_identity_and_expectation_field_changes_the_fingerprint() {
    let base = base_command().fingerprint().expect("base fingerprint");
    let variants = [
        command(10, 2, 3, 4, 5, declared("items"), 7, 9, 3, 11),
        command(1, 20, 3, 4, 5, declared("items"), 7, 9, 3, 11),
        command(1, 2, 30, 4, 5, declared("items"), 7, 9, 3, 11),
        command(1, 2, 3, 40, 5, declared("items"), 7, 9, 3, 11),
        command(1, 2, 3, 4, 50, declared("items"), 7, 9, 3, 11),
        command(1, 2, 3, 4, 5, declared("item"), 7, 9, 3, 11),
        command(1, 2, 3, 4, 5, declared("tasks"), 7, 9, 3, 11),
        command(1, 2, 3, 4, 5, declared("items"), 8, 9, 3, 11),
        command(1, 2, 3, 4, 5, declared("items"), 7, 10, 3, 11),
        command(1, 2, 3, 4, 5, declared("items"), 7, 9, 4, 11),
        command(1, 2, 3, 4, 5, declared("items"), 7, 9, 3, 12),
    ];
    let mut distinct = HashSet::from([base]);
    for variant in variants {
        let fingerprint = variant.fingerprint().expect("variant fingerprint");
        assert_ne!(fingerprint, base);
        assert!(distinct.insert(fingerprint));
    }
}

#[test]
fn operation_key_and_scheduler_renewal_do_not_change_the_fingerprint() {
    let mut command = base_command();
    let expected = command.fingerprint().expect("base fingerprint");
    command.operation_key = uuid(99);
    command.scheduler =
        SchedulerAuthority::for_run("scheduler-renewed", command.identity.run_id(), 99);
    assert_eq!(command.fingerprint(), Ok(expected));
}

#[test]
fn result_names_the_original_run_and_error_formatting_redacts_store_detail() {
    let original_run_id = RunId::new(uuid(5));
    let result = ScopedLeafFinishResult {
        group_id: CapacityGroupId::new(uuid(3)),
        member_id: CapacityMemberId::new(uuid(4)),
        original_run_id,
        revision: 10,
        epoch: 3,
        active_members: 1,
        possession: CapacityPossession::Vacant,
        finished_at: OffsetDateTime::UNIX_EPOCH,
        audit_id: AuditId::new(uuid(6)),
        outbox_id: uuid(7),
        replayed: false,
    };
    assert_eq!(result.original_run_id, original_run_id);

    let canary = "sk-live1234567890abcdef";
    let error = ScopedLeafFinishError::Storage(StoreError::Internal {
        detail: format!("driver text with {canary}"),
    });
    assert!(!error.to_string().contains(canary));
    assert!(!format!("{error:?}").contains(canary));
}
