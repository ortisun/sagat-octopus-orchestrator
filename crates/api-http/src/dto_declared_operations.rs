//! Wire shapes for declared business Query and Update operations.

use sagat_domain::execution::{UpdateReceipt, UpdateReceiptStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclaredQueryBody {
    pub args: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclaredUpdateBody {
    pub update_key: String,
    pub args: Value,
    #[serde(default)]
    pub expected_state_revision: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredQueryResponseBody {
    pub run_id: String,
    pub run_version: u64,
    pub state_revision: u64,
    pub result: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredUpdateResponseBody {
    pub run_id: String,
    pub operation: String,
    pub update_key: String,
    pub status: &'static str,
    pub result: Value,
    pub before_state_revision: u64,
    pub after_state_revision: u64,
    pub run_version: u64,
    pub replayed: bool,
}

impl DeclaredUpdateResponseBody {
    #[must_use]
    pub fn applied(receipt: &UpdateReceipt, result: Value, replayed: bool) -> Self {
        debug_assert_eq!(receipt.status(), UpdateReceiptStatus::Applied);
        Self {
            run_id: receipt.resolved_run_id().to_string(),
            operation: receipt.operation().as_str().to_owned(),
            update_key: receipt.update_key().as_str().to_owned(),
            status: "APPLIED",
            result,
            before_state_revision: receipt.before_state_revision(),
            after_state_revision: receipt.after_state_revision(),
            run_version: receipt.run_version(),
            replayed,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedUpdateDetails {
    pub reason: &'static str,
    pub run_id: String,
    pub operation: String,
    pub update_key: String,
    pub status: &'static str,
    pub before_state_revision: u64,
    pub after_state_revision: u64,
    pub run_version: u64,
    pub replayed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_state_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_state_revision: Option<u64>,
}

impl RejectedUpdateDetails {
    #[must_use]
    pub fn from_receipt(receipt: &UpdateReceipt, replayed: bool) -> Self {
        let (expected, actual) = match receipt.rejection() {
            Some(sagat_domain::execution::UpdateRejection::RevisionConflict {
                expected,
                actual,
            }) => (Some(*expected), Some(*actual)),
            _ => (None, None),
        };
        Self {
            reason: receipt.rejection().map_or(
                "UPDATE_REJECTED",
                sagat_domain::execution::UpdateRejection::reason,
            ),
            run_id: receipt.resolved_run_id().to_string(),
            operation: receipt.operation().as_str().to_owned(),
            update_key: receipt.update_key().as_str().to_owned(),
            status: "REJECTED",
            before_state_revision: receipt.before_state_revision(),
            after_state_revision: receipt.after_state_revision(),
            run_version: receipt.run_version(),
            replayed,
            expected_state_revision: expected,
            actual_state_revision: actual,
        }
    }
}
