//! The declared telemetry of an approval (feature 014 `contracts/README.md` §6, T014-024): the
//! journal-derived `workflow.approval.decide` span under the run's own trace, the closed metric
//! series, the refusal counter with a closed code, and nothing that names a person, a key or a
//! reason. Over the real HTTP route, scheduler and PostgreSQL.

use reqwest::StatusCode;
use sagat_application::telemetry::Observation;
use sagat_telemetry::{
    AttrValue, Metrics, SpanId, TraceId, WorkflowPrimitiveKind, WorkflowPrimitiveMetric,
    WorkflowPrimitiveOutcome, WorkflowPrimitiveRejectionCode, WorkflowPrimitiveSpan,
};
use serde_json::json;
use uuid::Uuid;

use crate::support::approval_flow::{
    ApprovalFlow, INITIATOR, OPERATOR, OTHER_OPERATOR, VIEWER, approval_chain, approval_with_state,
};

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &Uuid::now_v7().simple().to_string()[20..])
}

fn traceparent() -> (TraceId, String) {
    let trace = TraceId::generate();
    (trace, format!("00-{trace}-{}-01", SpanId::generate()))
}

/// The sum of every series of `name` whose labels include all of `labels`.
fn total(page: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
    page.lines()
        .filter(|line| {
            line.starts_with(name)
                && matches!(line[name.len()..].chars().next(), Some('{' | ' '))
                && labels
                    .iter()
                    .all(|(key, value)| line.contains(&format!("{key}=\"{value}\"")))
        })
        .filter_map(|line| line.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}

/// The attributes an approval span may carry: the run's identity and the closed labels. Never an
/// actor, a decision key, a reason code or a deadline.
const APPROVAL_SPAN_ATTRIBUTES: [&str; 8] = [
    "sagat.tenant.id",
    "sagat.namespace.id",
    "sagat.run.id",
    "sagat.workflow",
    "sagat.workflow.version",
    "sagat.primitive.kind",
    "sagat.outcome",
    "sagat.node",
];

fn approval_spans(
    derived: &sagat_application::telemetry::Derived,
) -> Vec<&sagat_telemetry::SpanData> {
    derived
        .spans
        .iter()
        .filter(|span| span.name == WorkflowPrimitiveSpan::ApprovalDecide.as_str())
        .collect()
}

fn outcomes(derived: &sagat_application::telemetry::Derived) -> Vec<WorkflowPrimitiveOutcome> {
    derived
        .observations
        .iter()
        .filter_map(|observation| match observation {
            Observation::PrimitiveTransition {
                kind: WorkflowPrimitiveKind::Approval,
                outcome,
            } => Some(*outcome),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_decision_and_an_expiry_project_one_span_each_under_the_runs_trace() {
    let flow = ApprovalFlow::boot(&[]).await;
    let name = unique("tele-state");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 600_000))
        .await
        .expect("publishes");
    let (trace, header) = traceparent();
    let run = flow
        .start_traced(&workflow, json!({"account": {"balance": 1}}), &header)
        .await;
    let approval = flow.the_approval(run).await;
    let (status, _) = flow
        .decide(&approval, OPERATOR, "tele-decision-key", "APPROVE")
        .await;
    assert_eq!(status, StatusCode::OK);
    flow.drive().await;

    let derived = flow.derived(run).await;
    let spans = approval_spans(&derived);
    assert_eq!(spans.len(), 1, "{spans:?}");
    let span = spans[0];
    assert_eq!(
        span.trace_id.to_string(),
        trace.to_string(),
        "the run's own trace"
    );
    assert_eq!(flow.trace_of(run).await, trace.to_string());
    for (key, value) in &span.attributes {
        assert!(
            APPROVAL_SPAN_ATTRIBUTES.contains(&key.as_str()),
            "{key} in {span:?}"
        );
        let rendered = format!("{value:?}");
        for forbidden in [OPERATOR, INITIATOR, "tele-decision-key", "POLICY_OK"] {
            assert!(
                !rendered.contains(forbidden),
                "{forbidden} in {key}={rendered}"
            );
        }
    }
    assert_eq!(
        span.attributes
            .iter()
            .find_map(|(key, value)| (key == "sagat.outcome").then_some(value)),
        Some(&AttrValue::from(
            WorkflowPrimitiveOutcome::Approved.as_str()
        ))
    );
    assert_eq!(
        outcomes(&derived),
        [
            WorkflowPrimitiveOutcome::Entered,
            WorkflowPrimitiveOutcome::Approved
        ]
    );

    // An expiry is the same span with the closed `expired` outcome.
    let short = unique("tele-expiry");
    let expiring = flow
        .publish(&short, approval_with_state(&short, 1_200))
        .await
        .expect("publishes");
    let run = flow
        .start(&expiring, json!({"account": {"balance": 1}}))
        .await;
    flow.the_approval(run).await;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        flow.drive().await;
        if flow.run_status(run).await == "SUCCEEDED" {
            break;
        }
    }
    let derived = flow.derived(run).await;
    assert_eq!(approval_spans(&derived).len(), 1);
    assert_eq!(
        outcomes(&derived),
        [
            WorkflowPrimitiveOutcome::Entered,
            WorkflowPrimitiveOutcome::Expired
        ]
    );
}

#[tokio::test]
async fn a_successor_segments_approval_is_projected_under_its_own_run() {
    let flow = ApprovalFlow::boot(&["segment.work"]).await;
    let name = unique("tele-chain");
    let workflow = flow
        .publish(&name, approval_chain(&name, 600_000, 3))
        .await
        .expect("publishes");
    let chain = flow
        .start(&workflow, json!({"segment": 1, "next": {"segment": 2}}))
        .await;
    flow.drive().await;
    flow.work(json!({"done": 1})).await;
    let first = flow.the_approval(chain).await;
    flow.decide(&first, OPERATOR, "tele-round-1", "APPROVE")
        .await;
    flow.drive().await;
    let head = flow.head(chain).await;
    flow.work_on(head, json!({"done": 2})).await;
    let second = flow.the_approval(head).await;
    flow.decide(&second, OPERATOR, "tele-round-2", "REJECT")
        .await;
    flow.drive().await;

    for (run, outcome) in [
        (chain, WorkflowPrimitiveOutcome::Approved),
        (head, WorkflowPrimitiveOutcome::Rejected),
    ] {
        let derived = flow.derived(run).await;
        let spans = approval_spans(&derived);
        assert_eq!(spans.len(), 1, "one decision span per segment: {spans:?}");
        assert_eq!(spans[0].trace_id.to_string(), flow.trace_of(run).await);
        assert!(
            spans[0]
                .attributes
                .iter()
                .any(|(key, value)| key == "sagat.run.id"
                    && value == &AttrValue::from(run.to_string())),
            "the span belongs to the segment that decided: {:?}",
            spans[0]
        );
        assert!(outcomes(&derived).contains(&outcome));
    }
}

#[tokio::test]
async fn refused_decisions_count_in_the_closed_rejection_series_and_nothing_else_is_invented() {
    let flow = ApprovalFlow::boot(&[]).await;
    let metrics = Metrics::global();
    let name = unique("tele-refusals");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 600_000))
        .await
        .expect("publishes");
    let run = flow
        .start(&workflow, json!({"account": {"balance": 1}}))
        .await;
    let approval = flow.the_approval(run).await;
    let count = |code: WorkflowPrimitiveRejectionCode| {
        total(
            &metrics.render(),
            WorkflowPrimitiveMetric::Rejections.as_str(),
            &[("kind", "approval"), ("code", code.as_str())],
        )
    };
    let before = [
        count(WorkflowPrimitiveRejectionCode::NotEligible),
        count(WorkflowPrimitiveRejectionCode::InitiatorCannotDecide),
        count(WorkflowPrimitiveRejectionCode::AlreadyDecided),
        count(WorkflowPrimitiveRejectionCode::IdempotencyKeyReused),
    ];

    assert_eq!(
        flow.decide(&approval, VIEWER, "viewer-key", "APPROVE")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        flow.decide(&approval, INITIATOR, "initiator-key", "APPROVE")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        flow.decide(&approval, OPERATOR, "winner-key", "APPROVE")
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        flow.decide(&approval, OTHER_OPERATOR, "loser-key", "REJECT")
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        flow.decide(&approval, OPERATOR, "winner-key", "REJECT")
            .await
            .0,
        StatusCode::CONFLICT
    );

    let after = [
        count(WorkflowPrimitiveRejectionCode::NotEligible),
        count(WorkflowPrimitiveRejectionCode::InitiatorCannotDecide),
        count(WorkflowPrimitiveRejectionCode::AlreadyDecided),
        count(WorkflowPrimitiveRejectionCode::IdempotencyKeyReused),
    ];
    let grown: Vec<f64> = after
        .iter()
        .zip(before)
        .map(|(after, before)| after - before)
        .collect();
    assert_eq!(
        grown,
        [1.0, 1.0, 1.0, 1.0],
        "one count per refusal, by its closed code"
    );

    // Every label of every primitive series is from a closed enum, and the approval routes invent no
    // series of their own outside the declared vocabulary.
    let page = metrics.render();
    assert!(
        !page.contains("workflow_approval_http"),
        "approval HTTP series outside the declared vocabulary:\n{page}"
    );
    for line in page.lines().filter(|line| {
        line.starts_with("workflow_primitive_rejections_total")
            && line.contains("kind=\"approval\"")
    }) {
        for key in ["actor", "subject", "run", "tenant", "approval", "key"] {
            assert!(!line.contains(&format!("{key}=")), "{line}");
        }
    }
}
