//! Closed telemetry vocabulary for workflow primitives.

use std::collections::BTreeSet;

use sagat_telemetry::{
    Metrics, REDACTED, WorkflowPrimitiveKind, WorkflowPrimitiveMetric, WorkflowPrimitiveOutcome,
    WorkflowPrimitiveRejectionCode, WorkflowPrimitiveSpan, WorkflowPrimitiveSpanField, scrub,
};

#[test]
fn metric_names_and_label_keys_match_the_contract() {
    let schemas = [
        (
            WorkflowPrimitiveMetric::Transitions,
            "workflow_primitive_transitions_total",
            &["kind", "outcome"][..],
        ),
        (
            WorkflowPrimitiveMetric::Active,
            "workflow_primitive_active",
            &["kind"][..],
        ),
        (
            WorkflowPrimitiveMetric::AdmissionWait,
            "workflow_primitive_admission_wait_seconds",
            &["kind"][..],
        ),
        (
            WorkflowPrimitiveMetric::ApprovalWait,
            "workflow_approval_wait_seconds",
            &["outcome"][..],
        ),
        (
            WorkflowPrimitiveMetric::OperationDuration,
            "workflow_operation_duration_seconds",
            &["kind", "outcome"][..],
        ),
        (
            WorkflowPrimitiveMetric::Rejections,
            "workflow_primitive_rejections_total",
            &["kind", "code"][..],
        ),
        (
            WorkflowPrimitiveMetric::Continuation,
            "workflow_continuation_total",
            &["outcome"][..],
        ),
    ];

    for (metric, name, labels) in schemas {
        assert_eq!(metric.as_str(), name);
        assert_eq!(metric.label_keys(), labels);
        for forbidden in ["tenant", "run", "node", "instance", "actor", "subject"] {
            assert!(!metric.label_keys().contains(&forbidden));
        }
    }
}

#[test]
fn every_metric_label_value_comes_from_a_small_closed_set() {
    fn unique(values: impl Iterator<Item = &'static str>, maximum: usize) {
        let values = values.collect::<Vec<_>>();
        assert!(
            values.len() <= maximum,
            "label set grew past its review bound"
        );
        assert_eq!(
            values.iter().copied().collect::<BTreeSet<_>>().len(),
            values.len()
        );
        assert!(
            values
                .iter()
                .all(|value| !value.is_empty() && value.len() <= 32)
        );
    }

    unique(
        WorkflowPrimitiveKind::ALL
            .iter()
            .copied()
            .map(WorkflowPrimitiveKind::as_str),
        8,
    );
    unique(
        WorkflowPrimitiveOutcome::ALL
            .iter()
            .copied()
            .map(WorkflowPrimitiveOutcome::as_str),
        16,
    );
    unique(
        WorkflowPrimitiveRejectionCode::ALL
            .iter()
            .copied()
            .map(WorkflowPrimitiveRejectionCode::as_str),
        32,
    );
}

#[test]
fn span_names_and_fields_are_fixed_and_carry_no_free_text() {
    assert_eq!(
        WorkflowPrimitiveSpan::ALL.map(WorkflowPrimitiveSpan::as_str),
        [
            "workflow.scope.admit",
            "workflow.iteration.commit",
            "workflow.approval.decide",
            "workflow.query",
            "workflow.update",
            "workflow.continue",
            "workflow.switch.evaluate",
            "workflow.unsupported",
        ]
    );

    let fields = [
        WorkflowPrimitiveSpanField::Kind(WorkflowPrimitiveKind::Collection),
        WorkflowPrimitiveSpanField::Outcome(WorkflowPrimitiveOutcome::Committed),
        WorkflowPrimitiveSpanField::Rejection(WorkflowPrimitiveRejectionCode::InvalidScope),
        WorkflowPrimitiveSpanField::ScopeDepth(8),
        WorkflowPrimitiveSpanField::IterationIndex(9_999),
        WorkflowPrimitiveSpanField::SegmentIndex(9_999),
    ];
    let forbidden = [
        "payload",
        "body",
        "state",
        "argument",
        "expression",
        "secret",
        "token",
    ];
    for field in fields {
        let (key, value) = field.attribute();
        assert!(forbidden.iter().all(|word| !key.contains(word)), "{key}");
        let rendered = format!("{value:?}");
        assert!(
            forbidden.iter().all(|word| !rendered.contains(word)),
            "{rendered}"
        );
    }
}

#[test]
fn secret_markers_are_redacted_from_telemetry_text() {
    let marker = "sk-live1234567890abcdef";
    let text = format!("primitive rejected credential {marker}");
    let rendered = scrub(&text);

    assert!(!rendered.contains(marker));
    assert!(rendered.contains(REDACTED));
}

#[test]
fn run_data_limit_is_a_finite_private_rejection_series() {
    let code = WorkflowPrimitiveRejectionCode::from_reason("RUN_DATA_LIMIT")
        .expect("the durable data-limit reason is in the closed vocabulary");
    assert_eq!(code, WorkflowPrimitiveRejectionCode::RunDataLimit);
    assert_eq!(code.as_str(), "RUN_DATA_LIMIT");

    let private = [
        "run_data_limit",
        "RUN_DATA_LIMIT:4194305",
        "payload=secret-marker",
        "sha256:secret-marker",
        "steps.items.output",
    ];
    assert!(
        private
            .into_iter()
            .all(|reason| WorkflowPrimitiveRejectionCode::from_reason(reason).is_none())
    );

    let (export, recorder) = Metrics::detached(std::time::Duration::from_secs(60));
    metrics::with_local_recorder(&recorder, || {
        metrics::counter!(
            WorkflowPrimitiveMetric::Rejections.as_str(),
            "kind" => WorkflowPrimitiveKind::Collection.as_str(),
            "code" => code.as_str(),
        )
        .increment(1);
        for reason in private {
            if let Some(unexpected) = WorkflowPrimitiveRejectionCode::from_reason(reason) {
                metrics::counter!(
                    WorkflowPrimitiveMetric::Rejections.as_str(),
                    "kind" => WorkflowPrimitiveKind::Collection.as_str(),
                    "code" => unexpected.as_str(),
                )
                .increment(1);
            }
        }
    });
    let page = export.render();
    let sample = page
        .lines()
        .find(|line| line.starts_with("workflow_primitive_rejections_total"))
        .expect("exported rejection sample");
    assert!(sample.contains("code=\"RUN_DATA_LIMIT\""), "{sample}");
    assert!(sample.contains("kind=\"collection\""), "{sample}");
    assert!(sample.ends_with(" 1"), "{sample}");
    for forbidden in [
        "secret-marker",
        "4194305",
        "sha256",
        "steps.items.output",
        "payload",
    ] {
        assert!(!page.contains(forbidden), "{page}");
    }
}
