//! Wire types of the reset routes (feature 018, US06): the points a run can be reset to, what a reset would
//! do, the request that makes one, and the entry of the register that answers it (camelCase, RFC 3339).

use sagat_application::runs::{
    ReapplyEntry, ReportEntry, ResetPointInfo, ResetPreview, ResetRecord,
};
use sagat_domain::execution::{EffectMarkers, ResetReapply};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::dto_exec::RerunBusinessIdBody;

/// What is delivered again after the point (`ReapplyPolicy`).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReapplyPolicyBody {
    None,
    Signals,
    Updates,
    All,
}

impl From<ReapplyPolicyBody> for ResetReapply {
    fn from(policy: ReapplyPolicyBody) -> Self {
        match policy {
            ReapplyPolicyBody::None => Self::None,
            ReapplyPolicyBody::Signals => Self::Signals,
            ReapplyPolicyBody::Updates => Self::Updates,
            ReapplyPolicyBody::All => Self::All,
        }
    }
}

/// What a reset does with an open source (`sourcePolicy`).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourcePolicyBody {
    RequireTerminal,
    CancelSource,
}

/// `POST /runs/{runId}/reset-previews` (`ResetPreviewRequest`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResetPreviewBody {
    pub sequence: u64,
    #[serde(default)]
    pub reapply: Option<ReapplyPolicyBody>,
}

/// `POST /runs/{runId}/resets` (`ResetRequest`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResetRequestBody {
    pub sequence: u64,
    pub reason: String,
    #[serde(default)]
    pub reapply: Option<ReapplyPolicyBody>,
    #[serde(default)]
    pub source_policy: Option<SourcePolicyBody>,
    #[serde(default)]
    pub business_id: Option<RerunBusinessIdBody>,
    #[serde(default)]
    pub acknowledge_reexecution: bool,
}

/// One place a run can be reset to (`ResetPoint`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPointBody {
    pub sequence: u64,
    pub kind: &'static str,
    pub node: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
}

impl From<&ResetPointInfo> for ResetPointBody {
    fn from(point: &ResetPointInfo) -> Self {
        Self {
            sequence: point.sequence,
            kind: point.kind.as_str(),
            node: point.node.clone(),
            occurred_at: point.occurred_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredStepBody {
    pub node: String,
    /// `APPROVAL` for a decision the reset keeps instead of asking again, `STEP` for any other step.
    pub kind: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReexecutedStepBody {
    pub node: String,
    pub completed_in_source: bool,
    pub markers: Vec<&'static str>,
}

/// One signal or Update that a reset delivers again, or did (`ReapplyEntry`): the preview says `WILL_TRY` or
/// `NOT_REAPPLIED`, the report of a reset `REAPPLIED`, `REJECTED` or `NOT_REAPPLIED`. A signal has a `name`, an
/// Update an `operation`; never a payload, an argument or a digest of them.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReapplyEntryBody {
    pub sequence: u64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl From<&ReportEntry> for ReapplyEntryBody {
    fn from(entry: &ReportEntry) -> Self {
        Self {
            sequence: entry.sequence,
            kind: entry.kind.clone(),
            name: entry.name.clone(),
            operation: entry.operation.clone(),
            outcome: entry.outcome.clone(),
            reason: entry.reason.clone(),
        }
    }
}

impl From<&ReapplyEntry> for ReapplyEntryBody {
    fn from(entry: &ReapplyEntry) -> Self {
        Self::from(&ReportEntry::from(entry))
    }
}

/// What a reset would restore, run again and reapply (`ResetPlan`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPlanBody {
    pub sequence: u64,
    pub restored: Vec<RestoredStepBody>,
    pub reexecuted: Vec<ReexecutedStepBody>,
    /// The signals and Updates the reset would try to deliver again, in journal order; empty for `NONE`.
    pub reapply: Vec<ReapplyEntryBody>,
    pub requires_acknowledgement: bool,
    pub prefix_events: u64,
}

fn markers(effects: &EffectMarkers) -> Vec<&'static str> {
    [
        (effects.compensate_with, "compensateWith"),
        (effects.reconcile_with, "reconcileWith"),
        (effects.irreversible, "irreversible"),
        (effects.high_risk, "highRisk"),
    ]
    .into_iter()
    .filter_map(|(set, name)| set.then_some(name))
    .collect()
}

impl From<&ResetPreview> for ResetPlanBody {
    fn from(preview: &ResetPreview) -> Self {
        let plan = &preview.plan;
        Self {
            sequence: plan.point.sequence,
            restored: plan
                .restored
                .iter()
                .map(|step| RestoredStepBody {
                    node: step.node.as_str().to_owned(),
                    kind: if step.approval { "APPROVAL" } else { "STEP" },
                })
                .collect(),
            reexecuted: plan
                .reexecuted
                .iter()
                .map(|step| ReexecutedStepBody {
                    node: step.node.as_str().to_owned(),
                    completed_in_source: step.completed_in_source,
                    markers: markers(&step.effects),
                })
                .collect(),
            reapply: preview.reapply.iter().map(ReapplyEntryBody::from).collect(),
            requires_acknowledgement: plan.needs_acknowledgement(),
            prefix_events: plan.point.sequence,
        }
    }
}

/// An entry of the register of resets (`Reset`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetBody {
    pub reset_id: String,
    pub source_run_id: String,
    pub sequence: u64,
    pub state: &'static str,
    pub failure_reason: Option<String>,
    pub new_run_id: Option<String>,
    /// What the reapply did, in journal order; empty for a reset that did not ask for one.
    pub reapply_report: Vec<ReapplyEntryBody>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl From<&ResetRecord> for ResetBody {
    fn from(record: &ResetRecord) -> Self {
        Self {
            reset_id: record.id.to_string(),
            source_run_id: record.source.to_string(),
            sequence: record.sequence,
            state: record.state.as_str(),
            failure_reason: record.failure_reason.clone(),
            new_run_id: record.new_run.map(|run| run.to_string()),
            reapply_report: record
                .reapply_report
                .iter()
                .map(ReapplyEntryBody::from)
                .collect(),
            created_at: record.created_at,
        }
    }
}

/// The resets made from a run (the answer of `listRunResets`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetListBody {
    pub items: Vec<ResetBody>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use sagat_application::runs::{ReapplyKind, ReapplyOutcome, ReapplyReason};
    use sagat_domain::execution::{ReexecutedStep, ResetPlan, ResetPoint, RestoredStep};
    use sagat_domain::workflow::NodeId;
    use serde_json::json;

    use super::*;

    fn node(name: &str) -> NodeId {
        NodeId::parse(name).expect("a node id")
    }

    #[test]
    fn a_plan_names_the_effects_a_step_declares_and_the_kind_of_what_is_kept() {
        let plan = ResetPlan {
            point: ResetPoint {
                sequence: 9,
                completes: Some(node("pay")),
            },
            restored: vec![
                RestoredStep {
                    node: node("pay"),
                    approval: false,
                },
                RestoredStep {
                    node: node("sign"),
                    approval: true,
                },
            ],
            reexecuted: vec![
                ReexecutedStep {
                    node: node("ship"),
                    completed_in_source: true,
                    in_flight: false,
                    effects: EffectMarkers {
                        compensate_with: true,
                        reconcile_with: true,
                        irreversible: true,
                        high_risk: true,
                    },
                },
                ReexecutedStep {
                    node: node("notify"),
                    completed_in_source: false,
                    in_flight: false,
                    effects: EffectMarkers::default(),
                },
                ReexecutedStep {
                    node: node("log"),
                    completed_in_source: false,
                    in_flight: false,
                    effects: EffectMarkers {
                        compensate_with: false,
                        reconcile_with: true,
                        irreversible: false,
                        high_risk: false,
                    },
                },
            ],
        };
        let preview = ResetPreview {
            plan,
            reapply: vec![
                ReapplyEntry {
                    sequence: 12,
                    kind: ReapplyKind::Signal,
                    name: "payment.confirmed".to_owned(),
                    outcome: ReapplyOutcome::WillTry,
                    reason: None,
                },
                ReapplyEntry {
                    sequence: 14,
                    kind: ReapplyKind::Update,
                    name: "balance.replace".to_owned(),
                    outcome: ReapplyOutcome::NotReapplied,
                    reason: Some(ReapplyReason::NotRetained),
                },
            ],
        };
        let body = serde_json::to_value(ResetPlanBody::from(&preview)).expect("json");
        assert_eq!(
            body,
            json!({
                "sequence": 9,
                "restored": [
                    {"node": "pay", "kind": "STEP"},
                    {"node": "sign", "kind": "APPROVAL"}
                ],
                "reexecuted": [
                    {"node": "ship", "completedInSource": true,
                     "markers": ["compensateWith", "reconcileWith", "irreversible", "highRisk"]},
                    {"node": "notify", "completedInSource": false, "markers": []},
                    {"node": "log", "completedInSource": false, "markers": ["reconcileWith"]}
                ],
                "reapply": [
                    {"sequence": 12, "kind": "SIGNAL", "name": "payment.confirmed",
                     "outcome": "WILL_TRY"},
                    {"sequence": 14, "kind": "UPDATE", "operation": "balance.replace",
                     "outcome": "NOT_REAPPLIED", "reason": "NOT_RETAINED"}
                ],
                "requiresAcknowledgement": true,
                "prefixEvents": 9
            })
        );
    }
}
