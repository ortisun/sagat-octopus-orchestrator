//! Resets: a new run made from an eligible prefix of a run that ended. The register of the resets
//! of a run is read with `list_resets` and `get_reset`.
use super::wire::{Fields, body_of, id_segment, summary_debug, wire_enum};
use super::*;
use crate::Page;
use serde::{Deserialize, Serialize};

wire_enum! {
    /// Whether the new run takes over the business identifier of the source.
    BusinessIdDisposition { Keep => "KEEP", Drop => "DROP" }
}
wire_enum! {
    /// Which signals and updates of the source are delivered again.
    ReapplyPolicy {
        Nothing => "NONE",
        Signals => "SIGNALS",
        Updates => "UPDATES",
        All => "ALL",
    }
}
wire_enum! {
    /// What happens to a source that has not ended.
    SourcePolicy { RequireTerminal => "REQUIRE_TERMINAL", CancelSource => "CANCEL_SOURCE" }
}

/// Where a run that ended can be reset to.
#[derive(Clone)]
pub struct ResetPoint {
    pub sequence: u64,
    /// `RUN_STARTED`, `STEP_COMPLETED`, ...: the set only grows.
    pub kind: String,
    pub node: Option<String>,
    pub occurred_at: OffsetDateTime,
    pub raw: Value,
}
summary_debug!(ResetPoint { sequence, kind });
impl ResetPoint {
    fn read(raw: &Value) -> Result<Self, SagatError> {
        let f = Fields::of(raw, "a reset point")?;
        Ok(Self {
            sequence: f.at_least("sequence", 1)?,
            kind: f.text("kind")?,
            node: f.opt_text("node"),
            occurred_at: f.instant("occurredAt")?,
            raw: raw.clone(),
        })
    }
}

/// A step the new run keeps as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoredStep {
    pub node: String,
    pub kind: String,
}
/// A step the source ran after the point, which the new run runs again, with the effects it declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReexecutedStep {
    pub node: String,
    pub completed_in_source: bool,
    pub markers: Vec<String>,
}

/// What a reset to `sequence` would restore and run again.
#[derive(Clone)]
pub struct ResetPlan {
    pub sequence: u64,
    pub restored: Vec<RestoredStep>,
    pub reexecuted: Vec<ReexecutedStep>,
    pub reapply: Vec<Value>,
    /// When true the reset needs `acknowledge_reexecution`.
    pub requires_acknowledgement: bool,
    pub prefix_events: u64,
    pub raw: Value,
}
summary_debug!(ResetPlan {
    sequence,
    requires_acknowledgement
});
impl ResetPlan {
    fn parse(raw: Value) -> Result<Self, SagatError> {
        let f = Fields::of(&raw, "a reset plan")?;
        let restored = f
            .values("restored")
            .iter()
            .map(|item| {
                let r = Fields::of(item, "a restored step")?;
                Ok(RestoredStep {
                    node: r.text("node")?,
                    kind: r.text("kind")?,
                })
            })
            .collect::<Result<Vec<_>, SagatError>>()?;
        let reexecuted = f
            .values("reexecuted")
            .iter()
            .map(|item| {
                let r = Fields::of(item, "a re-executed step")?;
                Ok(ReexecutedStep {
                    node: r.text("node")?,
                    completed_in_source: r.flag("completedInSource"),
                    markers: r.strings("markers"),
                })
            })
            .collect::<Result<Vec<_>, SagatError>>()?;
        Ok(Self {
            sequence: f.at_least("sequence", 1)?,
            restored,
            reexecuted,
            reapply: f.values("reapply"),
            requires_acknowledgement: f.flag("requiresAcknowledgement"),
            prefix_events: f.at_least("prefixEvents", 1)?,
            raw: raw.clone(),
        })
    }
}

/// A reset to make. `reason` is 8 to 2000 characters and audited with the reset; the server judges
/// it and everything else but the sequence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetRequest {
    pub sequence: u64,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reapply: Option<ReapplyPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_policy: Option<SourcePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_id: Option<BusinessIdDisposition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledge_reexecution: Option<bool>,
}
impl ResetRequest {
    pub fn new(sequence: u64, reason: impl Into<String>) -> Self {
        Self {
            sequence,
            reason: reason.into(),
            reapply: None,
            source_policy: None,
            business_id: None,
            acknowledge_reexecution: None,
        }
    }
}

/// An entry of the register of the resets of a run.
#[derive(Clone)]
pub struct Reset {
    pub reset_id: Uuid,
    pub source_run_id: Uuid,
    pub sequence: u64,
    pub state: String,
    pub failure_reason: Option<String>,
    pub new_run_id: Option<Uuid>,
    pub reapply_report: Vec<Value>,
    pub created_at: Option<OffsetDateTime>,
    pub raw: Value,
}
summary_debug!(Reset {
    reset_id,
    sequence,
    state
});
impl Reset {
    fn parse(raw: Value) -> Result<Self, SagatError> {
        Self::read(&raw)
    }
    fn read(raw: &Value) -> Result<Self, SagatError> {
        let f = Fields::of(raw, "a reset")?;
        Ok(Self {
            reset_id: f.uuid("resetId")?,
            source_run_id: f.uuid("sourceRunId")?,
            sequence: f.at_least("sequence", 1)?,
            state: f.text("state")?,
            failure_reason: f.opt_text("failureReason"),
            new_run_id: f.opt_uuid("newRunId")?,
            reapply_report: f.values("reapplyReport"),
            created_at: f.opt_instant("createdAt")?,
            raw: raw.clone(),
        })
    }
}

fn sequence_of(sequence: u64) -> Result<u64, SagatError> {
    if (1..=i64::MAX as u64).contains(&sequence) {
        Ok(sequence)
    } else {
        Err(SagatError::validation("sequence is a positive integer"))
    }
}

impl Client {
    /// Where a run that ended can be reset to, by ascending sequence.
    pub async fn reset_points(
        &self,
        run_id: &str,
        page: PageOptions,
        call: CallOptions,
    ) -> Result<Page<ResetPoint>, SagatError> {
        let run = id_segment(run_id, "a run id")?;
        let query = page_query(page.limit, page.cursor.as_deref())?;
        self.execute(
            WireRequest::new(Method::GET, &["runs", &run, "reset-points"], 200).query(query),
            call,
        )
        .await?
        .parse(|raw| wire::page_of(raw, ResetPoint::read))
    }

    /// What a reset to `sequence` would restore and run again; changes nothing.
    pub async fn preview_reset(
        &self,
        run_id: &str,
        sequence: u64,
        reapply: Option<ReapplyPolicy>,
        call: CallOptions,
    ) -> Result<ResetPlan, SagatError> {
        let run = id_segment(run_id, "a run id")?;
        let mut body = json!({"sequence":sequence_of(sequence)?});
        if let Some(reapply) = reapply {
            body["reapply"] = json!(reapply.as_str());
        }
        self.execute(
            WireRequest::new(Method::POST, &["runs", &run, "reset-previews"], 200)
                .json(&body)?
                .repeatable(),
            call,
        )
        .await?
        .parse(ResetPlan::parse)
    }

    /// Creates a new run from an eligible prefix of a run that ended. The same key and request
    /// answer the first reset.
    pub async fn reset(
        &self,
        run_id: &str,
        request: &ResetRequest,
        call: CallOptions,
    ) -> Result<Reset, SagatError> {
        let run = id_segment(run_id, "a run id")?;
        sequence_of(request.sequence)?;
        let body = body_of(request)?;
        self.execute(
            WireRequest::new(Method::POST, &["runs", &run, "resets"], 202)
                .json(&body)?
                .keyed(key(&call)?),
            call,
        )
        .await?
        .parse(Reset::parse)
    }

    /// The resets made from a run, newest first (at most 200).
    pub async fn list_resets(
        &self,
        run_id: &str,
        call: CallOptions,
    ) -> Result<Vec<Reset>, SagatError> {
        let run = id_segment(run_id, "a run id")?;
        self.execute(
            WireRequest::new(Method::GET, &["runs", &run, "resets"], 200),
            call,
        )
        .await?
        .parse(|raw| {
            Fields::of(&raw, "a list of resets")?
                .values("items")
                .iter()
                .map(Reset::read)
                .collect()
        })
    }

    pub async fn get_reset(
        &self,
        run_id: &str,
        reset_id: &str,
        call: CallOptions,
    ) -> Result<Reset, SagatError> {
        let run = id_segment(run_id, "a run id")?;
        let reset = id_segment(reset_id, "a reset id")?;
        self.execute(
            WireRequest::new(Method::GET, &["runs", &run, "resets", &reset], 200),
            call,
        )
        .await?
        .parse(Reset::parse)
    }
}
