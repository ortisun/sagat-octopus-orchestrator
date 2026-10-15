use sagat_application::chain_progress::ChainInspection;
use sagat_domain::execution::ChainFinalOutcome;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChainSignalBody {
    pub expected_run_id: String,
    pub payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChainCancelBody {
    pub expected_run_id: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSignalResultBody {
    pub delivered_to: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSegmentBody {
    pub index: u16,
    pub run_id: String,
    pub predecessor_run_id: Option<String>,
    pub successor_run_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainInspectionBody {
    pub chain_id: String,
    pub current_run_id: String,
    pub revision: u64,
    pub cancel_requested: bool,
    pub final_outcome: Option<&'static str>,
    pub segments: Vec<ChainSegmentBody>,
}

impl From<ChainInspection> for ChainInspectionBody {
    fn from(value: ChainInspection) -> Self {
        Self {
            chain_id: value.chain_id.to_string(),
            current_run_id: value.current_head.to_string(),
            revision: value.revision,
            cancel_requested: value.cancel_requested,
            final_outcome: value.final_outcome.map(outcome),
            segments: value
                .segments
                .into_iter()
                .map(|segment| ChainSegmentBody {
                    index: segment.index,
                    run_id: segment.run_id.to_string(),
                    predecessor_run_id: segment.predecessor.map(|run| run.to_string()),
                    successor_run_id: segment.successor.map(|run| run.to_string()),
                })
                .collect(),
        }
    }
}

const fn outcome(value: ChainFinalOutcome) -> &'static str {
    match value {
        ChainFinalOutcome::Succeeded => "SUCCEEDED",
        ChainFinalOutcome::Failed => "FAILED",
        ChainFinalOutcome::Cancelled => "CANCELLED",
        ChainFinalOutcome::TimedOut => "TIMED_OUT",
        ChainFinalOutcome::Compensated => "COMPENSATED",
        ChainFinalOutcome::Partial => "PARTIAL",
        ChainFinalOutcome::CompensationFailed => "COMPENSATION_FAILED",
    }
}
