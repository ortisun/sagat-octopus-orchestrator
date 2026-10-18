//! Closed construction boundary for a native collection `RUN_DATA_LIMIT` drain.
#![allow(clippy::expect_used)]

use sagat_application::primitives::{
    CollectionDataLimitEvidence, CollectionDrain, CollectionScopeId, CollectionTransition,
    PrimitiveInputError, SchedulerAuthority,
};
use sagat_domain::{
    audit::{AuditAction, AuditRecord, ResourceType, RunDataLimitAuditReason},
    execution::{CollectionDrainReason, ExecutionInstanceId, RunId},
    identity::{NamespaceId, Subject, TenantId},
    workflow::NodeId,
};
use time::OffsetDateTime;
use uuid::Uuid;

fn audit(
    tenant_id: TenantId,
    action: AuditAction,
    resource_type: ResourceType,
    resource_id: String,
    reason: RunDataLimitAuditReason,
) -> AuditRecord {
    AuditRecord::engine(
        tenant_id,
        action,
        resource_type,
        resource_id,
        reason.as_str(),
        Some("trace-data-limit".to_owned()),
        OffsetDateTime::UNIX_EPOCH,
    )
}

fn transition(
    tenant_id: TenantId,
    run_id: RunId,
    audit: Option<AuditRecord>,
) -> CollectionTransition {
    CollectionTransition::new(
        tenant_id,
        NamespaceId::generate(),
        run_id,
        17,
        ExecutionInstanceId::root(NodeId::parse("items").expect("owner")),
        SchedulerAuthority::for_run("scheduler-a", run_id, 3),
        Subject::engine(),
        Some("trace-data-limit".to_owned()),
        OffsetDateTime::UNIX_EPOCH,
        audit,
    )
}

fn checked(
    tenant_id: TenantId,
    run_id: RunId,
    audit: Option<AuditRecord>,
    evidence: CollectionDataLimitEvidence,
) -> Result<CollectionDrain, PrimitiveInputError> {
    CollectionDrain::for_data_limit(
        transition(tenant_id, run_id, audit),
        CollectionScopeId::generate(),
        evidence,
        Uuid::now_v7(),
    )
}

#[test]
fn aggregate_evidence_is_derived_from_candidate_and_bounded_ordered_sources() {
    let sources = [
        (0, br#"{"value":1}"#.to_vec()),
        (1, br#"{"value":2}"#.to_vec()),
    ];
    let aggregate =
        CollectionDataLimitEvidence::aggregate(br#"[{"mapped":1},{"mapped":2}]"#, &sources)
            .expect("source-bound aggregate");
    let different_candidate =
        CollectionDataLimitEvidence::aggregate(br#"[{"mapped":3},{"mapped":4}]"#, &sources)
            .expect("same-length candidate");
    let different_source = CollectionDataLimitEvidence::aggregate(
        br#"[{"mapped":1},{"mapped":2}]"#,
        &[(0, br#"{"value":9}"#.to_vec()), (1, sources[1].1.clone())],
    )
    .expect("different source");
    assert_ne!(aggregate, different_candidate, "candidate digest is bound");
    assert_ne!(aggregate, different_source, "source digests are bound");

    let tenant_id = TenantId::generate();
    let run_id = RunId::generate();
    let record = audit(
        tenant_id,
        AuditAction::RunDataLimitExceeded,
        ResourceType::Run,
        run_id.to_string(),
        RunDataLimitAuditReason::Aggregate,
    );
    let drain = checked(tenant_id, run_id, Some(record), aggregate)
        .expect("aggregate evidence and its exact audit are staged together");
    assert_eq!(drain.reason, CollectionDrainReason::RunDataLimit);

    for malformed in [
        vec![(1, vec![1]), (0, vec![0])],
        vec![(0, vec![1]), (0, vec![2])],
        vec![(10_000, vec![1])],
    ] {
        assert_eq!(
            CollectionDataLimitEvidence::aggregate(b"[0]", &malformed),
            Err(PrimitiveInputError::DataLimitEvidence)
        );
    }
    let too_many: Vec<_> = (0..=10_000).map(|index| (index, Vec::new())).collect();
    assert_eq!(
        CollectionDataLimitEvidence::aggregate(b"[]", &too_many),
        Err(PrimitiveInputError::DataLimitEvidence)
    );
    assert_eq!(
        CollectionDataLimitEvidence::aggregate(b"[0]", &[]),
        Err(PrimitiveInputError::DataLimitEvidence)
    );
    assert!(CollectionDataLimitEvidence::aggregate(b"[]", &[]).is_ok());
}

#[test]
fn data_limit_drain_requires_exact_run_audit_identity_action_and_reason() {
    let tenant_id = TenantId::generate();
    let run_id = RunId::generate();
    let evidence = || CollectionDataLimitEvidence::existing_outputs();
    let valid = || {
        audit(
            tenant_id,
            AuditAction::RunDataLimitExceeded,
            ResourceType::Run,
            run_id.to_string(),
            RunDataLimitAuditReason::ExistingOutputs,
        )
    };
    assert!(checked(tenant_id, run_id, Some(valid()), evidence()).is_ok());

    let wrong = [
        audit(
            tenant_id,
            AuditAction::RunStorageQuotaExceeded,
            ResourceType::Run,
            run_id.to_string(),
            RunDataLimitAuditReason::ExistingOutputs,
        ),
        audit(
            tenant_id,
            AuditAction::RunDataLimitExceeded,
            ResourceType::Run,
            run_id.to_string(),
            RunDataLimitAuditReason::Aggregate,
        ),
        audit(
            tenant_id,
            AuditAction::RunDataLimitExceeded,
            ResourceType::Namespace,
            run_id.to_string(),
            RunDataLimitAuditReason::ExistingOutputs,
        ),
        audit(
            tenant_id,
            AuditAction::RunDataLimitExceeded,
            ResourceType::Run,
            RunId::generate().to_string(),
            RunDataLimitAuditReason::ExistingOutputs,
        ),
        audit(
            TenantId::generate(),
            AuditAction::RunDataLimitExceeded,
            ResourceType::Run,
            run_id.to_string(),
            RunDataLimitAuditReason::ExistingOutputs,
        ),
    ];
    for invalid in wrong {
        assert_eq!(
            checked(tenant_id, run_id, Some(invalid), evidence()),
            Err(PrimitiveInputError::DataLimitAudit)
        );
    }
    assert_eq!(
        checked(tenant_id, run_id, None, evidence()),
        Err(PrimitiveInputError::DataLimitAudit)
    );
}

#[test]
fn storage_fact_evidence_has_no_caller_supplied_total_or_stop_code() {
    let tenant_id = TenantId::generate();
    let run_id = RunId::generate();
    for (evidence, reason) in [
        (
            CollectionDataLimitEvidence::existing_outputs(),
            RunDataLimitAuditReason::ExistingOutputs,
        ),
        (
            CollectionDataLimitEvidence::stopped_for_run_data_limit(),
            RunDataLimitAuditReason::StoppedForRunDataLimit,
        ),
    ] {
        let record = audit(
            tenant_id,
            AuditAction::RunDataLimitExceeded,
            ResourceType::Run,
            run_id.to_string(),
            reason,
        );
        let drain = checked(tenant_id, run_id, Some(record), evidence)
            .expect("closed storage-fact evidence");
        assert_eq!(drain.reason, CollectionDrainReason::RunDataLimit);
    }

    let ordinary = CollectionDrain::new(
        transition(tenant_id, run_id, None),
        CollectionScopeId::generate(),
        CollectionDrainReason::RunDataLimit,
        Uuid::now_v7(),
    );
    assert!(
        ordinary.data_limit_evidence().is_none(),
        "the legacy constructor cannot fabricate evidence for the native writer"
    );
}
