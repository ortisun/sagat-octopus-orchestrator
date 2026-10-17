//! Wire types for interactive workflow operations.

use sagat_domain::execution::{
    ApprovalDecision, ApprovalDecisionResolution, ApprovalReasonCode, DecisionKey,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApprovalDecisionBody {
    Approve,
    Reject,
}

impl From<ApprovalDecisionBody> for ApprovalDecision {
    fn from(value: ApprovalDecisionBody) -> Self {
        match value {
            ApprovalDecisionBody::Approve => Self::Approve,
            ApprovalDecisionBody::Reject => Self::Reject,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecideApprovalBody {
    pub decision_key: String,
    pub decision: ApprovalDecisionBody,
    #[serde(default)]
    pub reason_code: Option<String>,
}

impl DecideApprovalBody {
    pub fn decision_key(&self) -> Result<DecisionKey, sagat_domain::execution::ApprovalError> {
        DecisionKey::parse(&self.decision_key)
    }

    pub fn reason_code(
        &self,
    ) -> Result<Option<ApprovalReasonCode>, sagat_domain::execution::ApprovalError> {
        self.reason_code
            .as_deref()
            .map(ApprovalReasonCode::parse)
            .transpose()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionReceiptBody {
    pub receipt_id: String,
    pub approval_id: String,
    pub decision_key: String,
    pub decision: &'static str,
    pub reason_code: Option<String>,
    pub actor: String,
    #[serde(with = "time::serde::rfc3339")]
    pub decided_at: time::OffsetDateTime,
    pub outcome_sequence: u64,
    pub replayed: bool,
}

impl From<&ApprovalDecisionResolution> for ApprovalDecisionReceiptBody {
    fn from(resolution: &ApprovalDecisionResolution) -> Self {
        let receipt = resolution.receipt();
        Self {
            receipt_id: receipt.id().to_string(),
            approval_id: receipt.request_id().to_string(),
            decision_key: receipt.decision_key().as_str().to_owned(),
            decision: receipt.decision().as_str(),
            reason_code: receipt.reason_code().map(|code| code.as_str().to_owned()),
            actor: receipt.actor().to_string(),
            decided_at: receipt.decided_at(),
            outcome_sequence: receipt.outcome_sequence(),
            replayed: resolution.replayed(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalScopeBody {
    pub scope_node_id: String,
    pub iteration_index: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalListItemBody {
    pub approval_id: String,
    pub run_id: String,
    pub node_id: String,
    pub instance_path: Vec<ApprovalScopeBody>,
    pub status: &'static str,
    #[serde(with = "time::serde::rfc3339")]
    pub deadline: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub settled_at: Option<time::OffsetDateTime>,
}

impl From<&sagat_application::approval_progress::ApprovalListItem> for ApprovalListItemBody {
    fn from(item: &sagat_application::approval_progress::ApprovalListItem) -> Self {
        Self {
            approval_id: item.request_id().to_string(),
            run_id: item.run_id().to_string(),
            node_id: item.owner().node().as_str().to_owned(),
            instance_path: item
                .owner()
                .path()
                .segments()
                .iter()
                .map(|segment| ApprovalScopeBody {
                    scope_node_id: segment.scope_node().as_str().to_owned(),
                    iteration_index: segment.iteration_index(),
                })
                .collect(),
            status: item.status().as_str(),
            deadline: item.deadline(),
            settled_at: item.settled_at(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalListBody {
    pub items: Vec<ApprovalListItemBody>,
    pub next_cursor: Option<String>,
}
