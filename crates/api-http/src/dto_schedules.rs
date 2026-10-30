//! Wire types of the schedule routes (feature 018).
//!
//! The spec, the policies and the action travel as the JSON documents the contract describes: the
//! application's codec (`schedule_codec`) reads and writes them, so the wire shape and the stored
//! shape cannot drift apart. The input of the runs is never returned.

use std::collections::BTreeMap;

use sagat_application::schedule_codec::{policies_to_json, spec_to_json, target_to_json};
use sagat_application::schedules::{
    ActionView, ScheduleDescription, ScheduleDocument, ScheduleRecord, ScheduleState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

/// The body of `createSchedule`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleWriteBody {
    pub name: String,
    pub spec: Value,
    #[serde(default)]
    pub policies: Option<Value>,
    pub action: Value,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub remaining_actions: Option<u32>,
}

impl ScheduleWriteBody {
    #[must_use]
    pub fn into_document(self) -> ScheduleDocument {
        ScheduleDocument {
            name: self.name,
            spec: self.spec,
            policies: self.policies,
            action: self.action,
            paused: self.paused,
            note: self.note,
            remaining_actions: self.remaining_actions,
        }
    }
}

/// The body of `updateSchedule`: the whole schedule again, at the revision it was read at.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleUpdateBody {
    pub name: String,
    pub spec: Value,
    #[serde(default)]
    pub policies: Option<Value>,
    pub action: Value,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub remaining_actions: Option<u32>,
    pub expected_revision: u32,
}

impl ScheduleUpdateBody {
    /// The document and the revision it was written against.
    #[must_use]
    pub fn into_parts(self) -> (ScheduleDocument, u32) {
        (
            ScheduleDocument {
                name: self.name,
                spec: self.spec,
                policies: self.policies,
                action: self.action,
                paused: self.paused,
                note: self.note,
                remaining_actions: self.remaining_actions,
            },
            self.expected_revision,
        )
    }
}

/// The body of `pauseSchedule` and `resumeSchedule`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleNoteBody {
    pub note: String,
}

/// The body of `triggerSchedule`; the whole body is optional.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TriggerBody {
    #[serde(default)]
    pub overlap: Option<OverlapBody>,
}

/// The overlap rules a request may name.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OverlapBody {
    Skip,
    BufferOne,
    BufferAll,
    CancelOther,
    AllowAll,
}

impl From<OverlapBody> for sagat_domain::schedules::Overlap {
    fn from(overlap: OverlapBody) -> Self {
        match overlap {
            OverlapBody::Skip => Self::Skip,
            OverlapBody::BufferOne => Self::BufferOne,
            OverlapBody::BufferAll => Self::BufferAll,
            OverlapBody::CancelOther => Self::CancelOther,
            OverlapBody::AllowAll => Self::AllowAll,
        }
    }
}

/// A schedule. The input of its runs is described by its digest, never returned.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleBody {
    pub schedule_id: String,
    pub name: String,
    pub origin: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_id: Option<String>,
    pub state: &'static str,
    pub exhausted: bool,
    pub revision: u32,
    pub spec: Value,
    pub policies: Value,
    pub action: Value,
    pub note: Option<String>,
    pub note_by: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub note_at: Option<OffsetDateTime>,
    pub authorized_by: String,
    pub remaining_actions: Option<u32>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub next_action_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

impl From<&ScheduleRecord> for ScheduleBody {
    fn from(record: &ScheduleRecord) -> Self {
        let mut action = target_to_json(&record.target);
        if let (Value::Object(map), Some(digest)) = (&mut action, record.input_digest) {
            map.insert("inputDigest".to_owned(), Value::String(hex(&digest)));
        }
        Self {
            schedule_id: record.id.to_string(),
            name: record.name.as_str().to_owned(),
            origin: record.origin.as_str(),
            trigger_id: record.trigger_id.map(|id| id.to_string()),
            state: match record.state {
                ScheduleState::Active => "ACTIVE",
                // a deleted schedule is never returned; the arm keeps the match total
                ScheduleState::Paused | ScheduleState::Deleted => "PAUSED",
            },
            exhausted: record.remaining_actions == Some(0),
            revision: record.revision,
            spec: spec_to_json(&record.spec),
            policies: policies_to_json(&record.policies),
            action,
            note: record.note.clone(),
            note_by: record.note_by.clone(),
            note_at: record.note_at,
            authorized_by: record.authorized_by.clone(),
            remaining_actions: record.remaining_actions,
            next_action_at: record.next_action_at,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

/// One page of schedules, newest first.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulePageBody {
    pub items: Vec<ScheduleBody>,
    pub next_cursor: Option<String>,
}

/// A time a schedule will act on next.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingBody {
    #[serde(with = "time::serde::rfc3339")]
    pub nominal_time: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub actual_time: OffsetDateTime,
}

/// A decision of a schedule.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleActionBody {
    #[serde(with = "time::serde::rfc3339")]
    pub nominal_time: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub actual_time: Option<OffsetDateTime>,
    pub kind: &'static str,
    pub origin: &'static str,
    pub revision: u32,
    pub outcome: String,
    pub reason: Option<String>,
    pub run_id: Option<String>,
    pub backfill_id: Option<String>,
    pub summary_count: Option<u32>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub summary_last_nominal_time: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub decided_at: OffsetDateTime,
}

impl From<&ActionView> for ScheduleActionBody {
    fn from(action: &ActionView) -> Self {
        Self {
            nominal_time: action.nominal_at,
            actual_time: action.actual_at,
            kind: action.kind.as_str(),
            origin: action.origin.as_str(),
            revision: action.revision,
            outcome: action.outcome.clone(),
            reason: action.reason.clone(),
            run_id: action.run_id.map(|id| id.to_string()),
            backfill_id: action.backfill_id.map(|id| id.to_string()),
            summary_count: action.summary_count,
            summary_last_nominal_time: action.summary_last_nominal_at,
            decided_at: action.decided_at,
        }
    }
}

/// A schedule with its next and recent actions.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleDescriptionBody {
    #[serde(flatten)]
    pub schedule: ScheduleBody,
    pub next_actions: Vec<UpcomingBody>,
    pub recent_actions: Vec<ScheduleActionBody>,
    pub open_runs: Vec<String>,
    pub buffered_count: u32,
    pub counters: BTreeMap<String, u64>,
}

impl From<&ScheduleDescription> for ScheduleDescriptionBody {
    fn from(description: &ScheduleDescription) -> Self {
        Self {
            schedule: ScheduleBody::from(&description.record),
            next_actions: description
                .next
                .iter()
                .map(|&(nominal_time, actual_time)| UpcomingBody {
                    nominal_time,
                    actual_time,
                })
                .collect(),
            recent_actions: description
                .activity
                .recent
                .iter()
                .map(ScheduleActionBody::from)
                .collect(),
            open_runs: description
                .record
                .tracked_runs
                .iter()
                .map(ToString::to_string)
                .collect(),
            buffered_count: description.record.buffered,
            counters: description.activity.counters.clone(),
        }
    }
}
