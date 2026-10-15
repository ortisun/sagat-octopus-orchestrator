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
