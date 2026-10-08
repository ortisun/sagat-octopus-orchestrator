//! Wire types of the definition and run routes (camelCase, RFC 3339 timestamps, no tenant).

use sagat_application::definitions::{VersionRecord, WorkflowRecord};
use sagat_application::execution::RunRecord;
use sagat_application::runs::{GraphSnapshot, JournalEntry};
use sagat_domain::definition::WorkflowVersion;
use sagat_domain::workflow::{Node, NodeKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;

// ----------------------------------------------------------------------------- definitions

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowBody {
    pub name: String,
    pub ir: Value,
    // `id` and `draftRevision` are read-only in the contract; a client echoing a draft back may
    // send them, and they are ignored.
    #[serde(default)]
    #[allow(dead_code)]
    pub id: Option<Value>,
    #[serde(default)]
    #[allow(dead_code)]
    pub draft_revision: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateDraftBody {
    pub draft_revision: u64,
    pub ir: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishBody {
    pub draft_revision: u64,
    #[serde(default)]
    pub release_notes: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowBody {
    pub id: String,
    pub name: String,
    pub draft_revision: u64,
    pub ir: Value,
    pub active_version: Option<VersionSummary>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl From<&WorkflowRecord> for WorkflowBody {
    fn from(r: &WorkflowRecord) -> Self {
        Self {
            id: r.workflow.id().to_string(),
            name: r.workflow.name().to_string(),
            draft_revision: r.workflow.draft_revision(),
            ir: serde_json::from_slice(&r.draft).unwrap_or(Value::Null),
            active_version: r.active.as_ref().map(VersionSummary::from),
            created_at: r.workflow.created_at(),
            updated_at: r.workflow.updated_at(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionSummary {
    pub workflow_id: String,
    pub version_id: String,
    pub version_number: u32,
    pub digest: String,
    pub published_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub published_at: OffsetDateTime,
}

impl From<&WorkflowVersion> for VersionSummary {
    fn from(v: &WorkflowVersion) -> Self {
        Self {
            workflow_id: v.workflow_id().to_string(),
            version_id: v.id().to_string(),
            version_number: v.number(),
            digest: v.digest().to_hex(),
            published_by: v.published_by().to_string(),
            published_at: v.published_at(),
        }
    }
}

/// The result of publishing: the version, whether it is new, and what the author should know.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedBody {
    #[serde(flatten)]
    pub version: VersionSummary,
    /// False when the document was identical to an existing version, which was re-activated.
    pub created: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionBody {
    #[serde(flatten)]
    pub version: VersionSummary,
    pub release_notes: Option<String>,
    /// The canonical document this version froze.
    pub document: Value,
}

impl From<&VersionRecord> for VersionBody {
    fn from(r: &VersionRecord) -> Self {
        Self {
            version: VersionSummary::from(&r.version),
            release_notes: r.release_notes.clone(),
            document: serde_json::from_slice(&r.document).unwrap_or(Value::Null),
        }
    }
}

// ----------------------------------------------------------------------------- runs

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRunBody {
    pub workflow_id: String,
    #[serde(default)]
    pub version_id: Option<String>,
    pub input: Value,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub deadline: Option<OffsetDateTime>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationBody {
    pub action: String,
    pub reason: String,
    #[serde(default)]
    pub step_id: Option<String>,
    #[serde(default)]
    pub expected_run_version: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResultBody {
    pub operation_id: String,
    pub accepted: bool,
    pub run_status: String,
    pub run_version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalResultBody {
    /// The `waitEvent` nodes the signal woke.
    pub delivered_to: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunBody {
    pub run_id: String,
    pub workflow_id: String,
    pub version_id: String,
    pub status: &'static str,
    pub version: u64,
    pub paused: bool,
    pub started_by: String,
    pub trace_id: Option<String>,
    pub correlation_id: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub deadline: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
}

impl From<&RunRecord> for RunBody {
    fn from(r: &RunRecord) -> Self {
        Self {
            run_id: r.info.run_id.to_string(),
            workflow_id: r.info.workflow_id.to_string(),
            version_id: r.info.version_id.to_string(),
            status: r.status.as_str(),
            version: r.version,
            paused: r.paused,
            started_by: r.info.started_by.clone(),
            trace_id: r.info.trace_id.clone(),
            correlation_id: r.info.correlation_id.clone(),
            deadline: r.info.deadline_at,
            created_at: r.info.created_at,
            updated_at: r.updated_at,
            finished_at: r.finished_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEventBody {
    pub seq: u64,
    pub event_type: String,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
    pub step_id: Option<String>,
    pub attempt: Option<u32>,
    pub actor: String,
    pub trace_id: Option<String>,
    pub payload_ref: Option<String>,
}

impl From<&JournalEntry> for JournalEventBody {
    fn from(e: &JournalEntry) -> Self {
        Self {
            seq: e.seq,
            event_type: e.event_type.clone(),
            occurred_at: e.occurred_at,
            step_id: e.node.clone(),
            attempt: e.attempt,
            actor: e.actor.clone(),
            trace_id: e.trace_id.clone(),
            payload_ref: e.payload_ref.clone(),
        }
    }
}

/// The definition's graph overlaid with the run's step states. Nodes that did not start yet are
/// `NOT_STARTED`; the editor can draw the planned graph and the runtime state in one picture.
pub fn runtime_graph(graph: &GraphSnapshot) -> Value {
    let steps: std::collections::BTreeMap<_, _> =
        graph.steps.iter().map(|s| (s.node.as_str(), s)).collect();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for node in graph.definition.nodes() {
        let id = node.id().as_str();
        let step = steps.get(id);
        nodes.push(json!({
            "id": id,
            "type": node.kind().label(),
            "status": step.map_or("NOT_STARTED", |s| s.status.as_str()),
            "attempt": step.map_or(0, |s| s.attempt),
            "generation": step.map_or(0, |s| s.generation),
            "cancelRequested": step.is_some_and(|s| s.cancel_requested),
            "failureCode": step.and_then(|s| s.failure_code.clone()),
            "nextAttemptAt": step.and_then(|s| s.next_attempt_at).map(rfc3339),
            "dueAt": step.and_then(|s| s.due_at).map(rfc3339),
            "waitingOn": step.and_then(|s| s.waiting_on),
        }));
        edges_of(node, &mut edges);
    }
    json!({
        "runId": graph.run.info.run_id.to_string(),
        "status": graph.run.status.as_str(),
        "projectionSeq": graph.run.version,
        "projectionLagMs": 0,
        "nodes": nodes,
        "edges": edges,
    })
}

fn rfc3339(at: OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn edges_of(node: &Node, out: &mut Vec<Value>) {
    let from = node.id().as_str();
    let mut edge = |to: &str, kind: &str| out.push(json!({ "from": from, "to": to, "kind": kind }));
    match node.kind() {
        NodeKind::Condition {
            if_true, if_false, ..
        } => {
            edge(if_true.as_str(), "true");
            edge(if_false.as_str(), "false");
        }
        NodeKind::Parallel { branches, .. } => {
            for branch in branches {
                edge(branch.as_str(), "branch");
            }
        }
        NodeKind::WaitEvent {
            on_timeout: Some(target),
            ..
        } => edge(target.as_str(), "timeout"),
        _ => {}
    }
    if let Some(next) = node.next() {
        edge(next.as_str(), "next");
    }
    if let Some(target) = node.on_error() {
        edge(target.as_str(), "error");
    }
}
