use sagat_application::chain_progress::{ChainInspection, ChainRunItem, ChainRunPage};
use sagat_domain::execution::{ChainFinalOutcome, SegmentCompletionKind};
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

/// One segment of a chain in the paged list: lineage and lifecycle only, never a payload.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainRunBody {
    pub index: u16,
    pub run_id: String,
    pub status: &'static str,
    pub completion_kind: Option<&'static str>,
    pub predecessor_run_id: Option<String>,
    pub successor_run_id: Option<String>,
    pub created_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainRunPageBody {
    pub items: Vec<ChainRunBody>,
    pub next_cursor: Option<String>,
}

impl ChainRunPageBody {
    /// The page, with `cursor` turning the last index of a page that has more behind it into the
    /// opaque value the client sends back.
    #[must_use]
    pub fn from_page(page: ChainRunPage, cursor: impl Fn(u16) -> String) -> Self {
        Self {
            next_cursor: page.next_after.map(cursor),
            items: page.items.into_iter().map(ChainRunBody::from).collect(),
        }
    }
}

impl From<ChainRunItem> for ChainRunBody {
    fn from(value: ChainRunItem) -> Self {
        let instant = |at: time::OffsetDateTime| {
            at.format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default()
        };
        Self {
            index: value.index,
            run_id: value.run_id.to_string(),
            status: value.status.as_str(),
            completion_kind: value.completion_kind.map(|kind| match kind {
                SegmentCompletionKind::Continued => "CONTINUED",
                SegmentCompletionKind::Final => "FINAL",
            }),
            predecessor_run_id: value.predecessor.map(|run| run.to_string()),
            successor_run_id: value.successor.map(|run| run.to_string()),
            created_at: instant(value.created_at),
            finished_at: value.finished_at.map(instant),
        }
    }
}
