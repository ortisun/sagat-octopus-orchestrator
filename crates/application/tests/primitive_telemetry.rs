//! Journal-derived telemetry for public workflow primitives.

#![allow(clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use sagat_application::telemetry::{
    CONTEXT_EVENTS, Derived, JournalEntry, Observation, RunFacts, derive,
};
use sagat_domain::execution::{
    ApprovalDecision, ApprovalReasonCode, ApprovalReceiptId, ApprovalRequestId,
    CollectionDrainReason, CollectionOutcome, ExecutionInstanceId, FailureCode, InstancePath,
    LoopContinuation, LoopKind, RunEvent, RunId,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::workflow::{Digest, NodeId};
use sagat_telemetry::{
    AttrValue, SpanData, SpanLink, SpanStatus, TraceId, WorkflowPrimitiveKind,
    WorkflowPrimitiveOutcome, run_span_id,
};
use time::OffsetDateTime;
use uuid::Uuid;

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node id")
}

fn facts() -> RunFacts {
    RunFacts {
        tenant_id: TenantId::new(Uuid::new_v4()),
        namespace_id: NamespaceId::new(Uuid::new_v4()),
        run_id: RunId::generate(),
        workflow: "primitive-telemetry".to_owned(),
        version: 1,
        trace_id: TraceId::generate(),
        parent_span: None,
        parent_run: None,
        created_at: OffsetDateTime::from_unix_timestamp(1_800_000_000).expect("time"),
        steps: HashMap::new(),
    }
}

fn attribute<'a>(span: &'a SpanData, key: &str) -> Option<&'a AttrValue> {
    span.attributes
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

fn nanos(at: OffsetDateTime) -> u64 {
    u64::try_from(at.unix_timestamp_nanos()).expect("positive test timestamp")
}

#[test]
fn collection_lifecycle_projects_closed_outcomes_and_scoped_identity() {
    let facts = facts();
    let owner = node("items");
    let member_path = InstancePath::root()
        .child(owner.clone(), 2)
        .expect("collection member path");
    let member = ExecutionInstanceId::in_path(member_path.clone(), node("work"));
    let returned = ExecutionInstanceId::in_path(member_path, node("returned"));
    let entries = [
        JournalEntry {
            seq: 1,
            event: RunEvent::CollectionEntered {
                node: owner.clone(),
                item_count: 3,
                max_items: 3,
                max_concurrency: 1,
                snapshot_digest: Digest::new([7; 32]),
            },
            instance_path: InstancePath::root(),
            actor: "Bearer secret-marker".to_owned(),
            occurred_at: facts.created_at,
        },
        JournalEntry {
            seq: 2,
            event: RunEvent::CollectionIterationAdmitted {
                node: owner.clone(),
                index: 2,
                entry: member,
                item_digest: Digest::new([8; 32]),
            },
            instance_path: InstancePath::root(),
            actor: "Bearer secret-marker".to_owned(),
            occurred_at: facts.created_at + time::Duration::milliseconds(1),
        },
        JournalEntry {
            seq: 3,
            event: RunEvent::CollectionIterationCheckpointed {
                node: owner.clone(),
                index: 2,
                return_instance: returned,
                result_digest: Digest::new([9; 32]),
            },
            instance_path: InstancePath::root(),
            actor: "Bearer secret-marker".to_owned(),
            occurred_at: facts.created_at + time::Duration::milliseconds(2),
        },
        JournalEntry {
            seq: 4,
            event: RunEvent::CollectionDrainStarted {
                node: owner.clone(),
                reason: CollectionDrainReason::Cancelled,
            },
            instance_path: InstancePath::root(),
            actor: "Bearer secret-marker".to_owned(),
            occurred_at: facts.created_at + time::Duration::milliseconds(3),
        },
        JournalEntry {
            seq: 5,
            event: RunEvent::CollectionFinished {
                node: owner,
                outcome: CollectionOutcome::Cancelled,
                result_digest: None,
            },
            instance_path: InstancePath::root(),
            actor: "Bearer secret-marker".to_owned(),
            occurred_at: facts.created_at + time::Duration::milliseconds(4),
        },
    ];

    let derived = derive(&facts, &entries, 0);
    assert_eq!(
        derived
            .spans
            .iter()
            .map(|span| span.name.as_str())
            .collect::<Vec<_>>(),
        [
            "workflow.scope.admit",
            "workflow.scope.admit",
            "workflow.iteration.commit",
            "workflow.iteration.commit",
            "workflow.iteration.commit",
        ]
    );
    assert_eq!(
        derived.observations,
        [
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Collection,
                outcome: WorkflowPrimitiveOutcome::Entered,
            },
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Collection,
                outcome: WorkflowPrimitiveOutcome::Admitted,
            },
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Collection,
                outcome: WorkflowPrimitiveOutcome::Committed,
            },
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Collection,
                outcome: WorkflowPrimitiveOutcome::Draining,
            },
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Collection,
                outcome: WorkflowPrimitiveOutcome::Cancelled,
            },
        ]
    );
    assert_eq!(
        attribute(&derived.spans[1], "sagat.instance.path"),
        Some(&AttrValue::from("items[2]"))
    );
    assert_eq!(
        attribute(&derived.spans[2], "sagat.scope.depth"),
        Some(&AttrValue::from(1_u32))
    );
    assert!(!format!("{derived:?}").contains("secret-marker"));
    assert_eq!(
        derived.spans[4].status,
        SpanStatus::Error("PRIMITIVE_CANCELLED".to_owned())
    );
    assert_eq!(derive(&facts, &entries, 5), Derived::default());
}

#[test]
fn failed_primitive_span_uses_a_closed_error_without_leaking_failure_data() {
    let facts = facts();
    let entries = [JournalEntry {
        seq: 1,
        event: RunEvent::CollectionFinished {
            node: node("items"),
            outcome: CollectionOutcome::Failed(
                FailureCode::parse("SECRET_MARKER").expect("valid failure code"),
            ),
            result_digest: Some(Digest::new([0x53; 32])),
        },
        instance_path: InstancePath::root(),
        actor: "Bearer secret-marker".to_owned(),
        occurred_at: facts.created_at,
    }];

    let derived = derive(&facts, &entries, 0);
    assert_eq!(
        derived.spans[0].status,
        SpanStatus::Error("PRIMITIVE_FAILED".to_owned())
    );
    assert!(!format!("{derived:?}").contains("SECRET_MARKER"));
    assert!(!format!("{derived:?}").contains("secret-marker"));
    assert_eq!(derive(&facts, &entries, 1), Derived::default());
}

#[test]
fn data_limit_drain_span_and_observation_expose_no_candidate_evidence() {
    let facts = facts();
    let entries = [JournalEntry {
        seq: 1,
        event: RunEvent::CollectionDrainStarted {
            node: node("items"),
            reason: CollectionDrainReason::RunDataLimit,
        },
        instance_path: InstancePath::root(),
        actor: "worker:secret-marker".to_owned(),
        occurred_at: facts.created_at,
    }];

    let derived = derive(&facts, &entries, 0);
    assert_eq!(
        derived.observations,
        [Observation::PrimitiveTransition {
            kind: WorkflowPrimitiveKind::Collection,
            outcome: WorkflowPrimitiveOutcome::Draining,
        }]
    );
    assert_eq!(derived.spans.len(), 1);
    assert_eq!(derived.spans[0].name, "workflow.iteration.commit");
    assert_eq!(derived.spans[0].status, SpanStatus::Ok);
    for (key, value) in &derived.spans[0].attributes {
        for forbidden in [
            "payload",
            "output",
            "digest",
            "size",
            "expression",
            "reason",
        ] {
            assert!(!key.contains(forbidden), "unexpected span field {key}");
        }
        assert!(!format!("{value:?}").contains("secret-marker"));
    }
    let rendered = format!("{derived:?}");
    for forbidden in [
        "secret-marker",
        "RUN_DATA_LIMIT:4194305",
        "sha256:secret-marker",
        "steps.items.output",
    ] {
        assert!(!rendered.contains(forbidden), "{rendered}");
    }
}

#[test]
fn loop_checkpoint_reports_only_closed_lifecycle_outcomes() {
    let facts = facts();
    let owner = node("poll");
    let first_path = InstancePath::root()
        .child(owner.clone(), 0)
        .expect("first loop path");
    let second_path = InstancePath::root()
        .child(owner.clone(), 1)
        .expect("second loop path");
    let entries = [
        JournalEntry {
            seq: 1,
            event: RunEvent::LoopEntered {
                node: owner.clone(),
                kind: LoopKind::While,
                max_iterations: 2,
                entered_at: facts.created_at,
                deadline: facts.created_at + time::Duration::minutes(1),
                initial_carry_digest: Digest::new([1; 32]),
                initial_condition: Some(true),
            },
            instance_path: InstancePath::root(),
            actor: "system:scheduler".to_owned(),
            occurred_at: facts.created_at,
        },
        JournalEntry {
            seq: 2,
            event: RunEvent::LoopIterationAdmitted {
                node: owner.clone(),
                index: 0,
                entry: ExecutionInstanceId::in_path(first_path.clone(), node("work")),
            },
            instance_path: InstancePath::root(),
            actor: "system:scheduler".to_owned(),
            occurred_at: facts.created_at,
        },
        JournalEntry {
            seq: 3,
            event: RunEvent::LoopIterationCheckpointed {
                node: owner.clone(),
                index: 0,
                return_instance: ExecutionInstanceId::in_path(first_path, node("returned")),
                result_digest: Digest::new([2; 32]),
                carry_digest: Digest::new([3; 32]),
                condition: true,
                checked_at: facts.created_at,
                continuation: LoopContinuation::Continue,
            },
            instance_path: InstancePath::root(),
            actor: "system:scheduler".to_owned(),
            occurred_at: facts.created_at,
        },
        JournalEntry {
            seq: 4,
            event: RunEvent::LoopIterationAdmitted {
                node: owner.clone(),
                index: 1,
                entry: ExecutionInstanceId::in_path(second_path.clone(), node("work")),
            },
            instance_path: InstancePath::root(),
            actor: "system:scheduler".to_owned(),
            occurred_at: facts.created_at,
        },
        JournalEntry {
            seq: 5,
            event: RunEvent::LoopIterationCheckpointed {
                node: owner,
                index: 1,
                return_instance: ExecutionInstanceId::in_path(second_path, node("returned")),
                result_digest: Digest::new([4; 32]),
                carry_digest: Digest::new([5; 32]),
                condition: false,
                checked_at: facts.created_at,
                continuation: LoopContinuation::Complete,
            },
            instance_path: InstancePath::root(),
            actor: "system:scheduler".to_owned(),
            occurred_at: facts.created_at,
        },
    ];

    assert_eq!(
        derive(&facts, &entries, 0).observations,
        [
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Loop,
                outcome: WorkflowPrimitiveOutcome::Entered,
            },
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Loop,
                outcome: WorkflowPrimitiveOutcome::Admitted,
            },
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Loop,
                outcome: WorkflowPrimitiveOutcome::Committed,
            },
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Loop,
                outcome: WorkflowPrimitiveOutcome::Admitted,
            },
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Loop,
                outcome: WorkflowPrimitiveOutcome::Succeeded,
            },
        ]
    );
}

#[test]
fn approval_span_covers_the_durable_wait_without_identity_or_reason() {
    let facts = facts();
    let request = ApprovalRequestId::generate();
    let opened_at = facts.created_at + time::Duration::seconds(2);
    let decided_at = opened_at + time::Duration::seconds(5);
    let entries = [
        JournalEntry {
            seq: 10,
            event: RunEvent::ApprovalOpened {
                node: node("review"),
                request_id: request,
                deadline: decided_at + time::Duration::minutes(1),
            },
            instance_path: InstancePath::root(),
            actor: "Bearer secret-marker".to_owned(),
            occurred_at: opened_at,
        },
        JournalEntry {
            seq: 11,
            event: RunEvent::ApprovalDecided {
                node: node("review"),
                request_id: request,
                receipt_id: ApprovalReceiptId::generate(),
                decision: ApprovalDecision::Approve,
                actor: Subject::parse("secret-marker").expect("bounded subject"),
                reason_code: Some(
                    ApprovalReasonCode::parse("SECRET_MARKER").expect("bounded reason"),
                ),
            },
            instance_path: InstancePath::root(),
            actor: "Bearer secret-marker".to_owned(),
            occurred_at: decided_at,
        },
    ];

    let first = derive(&facts, &entries, 0);
    assert_eq!(first.spans.len(), 1);
    assert_eq!(first.spans[0].name, "workflow.approval.decide");
    assert_eq!(first.spans[0].start_unix_nano, nanos(opened_at));
    assert_eq!(first.spans[0].end_unix_nano, nanos(decided_at));
    assert_eq!(
        first.observations,
        [
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Approval,
                outcome: WorkflowPrimitiveOutcome::Entered,
            },
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Approval,
                outcome: WorkflowPrimitiveOutcome::Approved,
            },
        ]
    );
    assert!(!format!("{first:?}").contains("secret-marker"));

    let after_open = derive(&facts, &entries, 10);
    assert_eq!(after_open.spans, first.spans);
    assert_eq!(
        after_open.observations,
        [Observation::PrimitiveTransition {
            kind: WorkflowPrimitiveKind::Approval,
            outcome: WorkflowPrimitiveOutcome::Approved,
        }]
    );
    assert_eq!(derive(&facts, &entries, 11), Derived::default());
    assert!(CONTEXT_EVENTS.contains(&"approval.opened"));
}

#[test]
fn continuation_links_predecessor_and_successor_without_transfer_values() {
    let facts = facts();
    let predecessor = facts.run_id;
    let successor = RunId::generate();
    let entries = [
        JournalEntry {
            seq: 1,
            event: RunEvent::ExecutionChainStarted {
                max_segments: 3,
                original_deadline: facts.created_at + time::Duration::hours(1),
            },
            instance_path: InstancePath::root(),
            actor: "system:engine".to_owned(),
            occurred_at: facts.created_at,
        },
        JournalEntry {
            seq: 2,
            event: RunEvent::ContinuationSelected {
                node: node("rollover"),
                successor,
                input_digest: Digest::new([6; 32]),
                state_digest: Some(Digest::new([7; 32])),
            },
            instance_path: InstancePath::root(),
            actor: "Bearer secret-marker".to_owned(),
            occurred_at: facts.created_at,
        },
    ];

    let selected = derive(&facts, &entries, 0);
    assert_eq!(selected.spans.len(), 1);
    assert_eq!(selected.spans[0].name, "workflow.continue");
    assert_eq!(
        selected.spans[0].links,
        [SpanLink {
            trace_id: facts.trace_id,
            span_id: run_span_id(successor.as_uuid()),
        }]
    );
    assert_eq!(
        selected.observations,
        [
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Continuation,
                outcome: WorkflowPrimitiveOutcome::Entered,
            },
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Continuation,
                outcome: WorkflowPrimitiveOutcome::Continued,
            },
        ]
    );
    assert!(!format!("{selected:?}").contains("secret-marker"));

    let mut successor_facts = facts.clone();
    successor_facts.run_id = successor;
    let linked = derive(
        &successor_facts,
        &[JournalEntry {
            seq: 2,
            event: RunEvent::ExecutionChainSuccessorLinked {
                chain_id: predecessor,
                predecessor,
                segment_index: 1,
                max_segments: 3,
                original_deadline: facts.created_at + time::Duration::hours(1),
                input_digest: Digest::new([6; 32]),
                state_digest: Some(Digest::new([7; 32])),
            },
            instance_path: InstancePath::root(),
            actor: "system:engine".to_owned(),
            occurred_at: facts.created_at,
        }],
        0,
    );
    assert_eq!(linked.spans.len(), 1);
    assert_eq!(
        linked.spans[0].links,
        [SpanLink {
            trace_id: facts.trace_id,
            span_id: run_span_id(predecessor.as_uuid()),
        }]
    );
    assert_eq!(
        attribute(&linked.spans[0], "sagat.continuation.segment_index"),
        Some(&AttrValue::from(1_u32))
    );
    assert!(linked.observations.is_empty());
    assert_eq!(derive(&successor_facts, &[], 2), Derived::default());
}
