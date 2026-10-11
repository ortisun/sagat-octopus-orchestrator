//! Wire types of the definition and run routes (camelCase, RFC 3339 timestamps, no tenant).

use sagat_application::definitions::{StoredPlan, VersionRecord, WorkflowRecord};
use sagat_application::execution::RunRecord;
use sagat_application::runs::{GraphChanges, GraphSnapshot, JournalEntry};
use sagat_domain::definition::WorkflowVersion;
use sagat_domain::workflow::{Node, NodeKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;

// ----------------------------------------------------------------------------- definitions

// ------------------------------------------------------------------- validation and diff

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateBody {
    /// The document to check; absent, the stored draft.
    #[serde(default)]
    pub ir: Option<Value>,
}

fn problem_body(problem: &sagat_application::definitions::Problem) -> Value {
    json!({
        "severity": problem.severity.as_str(),
        "code": problem.code,
        "message": problem.message,
        "nodeId": problem.node,
        "path": problem.path,
    })
}

/// What a dry-run validation found.
pub fn validation_body(validation: &sagat_application::definitions::Validation) -> Value {
    json!({
        "valid": validation.is_valid(),
        "digest": validation.digest,
        "problems": validation.problems.iter().map(problem_body).collect::<Vec<_>>(),
        "warnings": validation.warnings.iter().map(problem_body).collect::<Vec<_>>(),
    })
}

fn diff_side(side: sagat_application::diff::DiffSide) -> Value {
    use sagat_application::diff::DiffSide;
    match side {
        DiffSide::Version(number) => {
            json!({ "kind": "version", "number": number, "revision": null })
        }
        DiffSide::Draft(revision) => {
            json!({ "kind": "draft", "number": null, "revision": revision })
        }
        DiffSide::Empty => json!({ "kind": "empty", "number": null, "revision": null }),
    }
}

fn finding_body(finding: &sagat_application::diff::DiffFinding) -> Value {
    json!({
        "code": finding.code,
        "message": finding.message,
        "nodeId": finding.node,
        "path": finding.path,
    })
}

fn edge_body(edge: &sagat_application::diff::DiffEdge) -> Value {
    json!({ "from": edge.from, "to": edge.to, "kind": edge.kind })
}

/// The differences between two documents of a workflow.
pub fn diff_body(diff: &sagat_application::diff::DefinitionDiff) -> Value {
    let d = &diff.diff;
    json!({
        "from": diff_side(diff.from),
        "to": diff_side(diff.to),
        "identical": d.identical,
        "nodes": {
            "added": d.nodes_added,
            "removed": d.nodes_removed,
            "changed": d.nodes_changed.iter()
                .map(|c| json!({ "id": c.id, "fields": c.fields }))
                .collect::<Vec<_>>(),
        },
        "edges": {
            "added": d.edges_added.iter().map(edge_body).collect::<Vec<_>>(),
            "removed": d.edges_removed.iter().map(edge_body).collect::<Vec<_>>(),
        },
        "changes": d.changes.iter()
            .map(|c| json!({ "path": c.path, "change": c.change.as_str() }))
            .collect::<Vec<_>>(),
        "changesOmitted": d.changes_omitted,
        "breaking": d.breaking.iter().map(finding_body).collect::<Vec<_>>(),
        "attention": d.attention.iter().map(finding_body).collect::<Vec<_>>(),
    })
}

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
    /// For `RESOLVE_STEP`: `SUCCEEDED` (the effect happened) or `FAILED` (it did not).
    #[serde(default)]
    pub outcome: Option<String>,
    /// For `RESOLVE_STEP` with `FAILED`: the code the step is recorded as failed with.
    #[serde(default)]
    pub error_code: Option<String>,
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
    /// The run and step that started this one, when it is a child run.
    pub parent: Option<ParentBody>,
    /// How deeply nested the run is: 0 when started through the API.
    pub depth: u8,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentBody {
    pub run_id: String,
    pub node: String,
    pub cancel_policy: &'static str,
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
            parent: r.info.parent.as_ref().map(|p| ParentBody {
                run_id: p.run.to_string(),
                node: p.node.to_string(),
                cancel_policy: p.cancel_policy.as_str(),
            }),
            depth: r.info.depth,
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
    /// Why it happened, when it has a cause.
    pub cause: Option<CauseBody>,
    /// The span of the attempt the event belongs to.
    pub span_id: Option<String>,
}

/// Why a transition happened: a closed kind and the code that goes with it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CauseBody {
    pub kind: &'static str,
    pub code: String,
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
            cause: e.cause.as_ref().map(|c| CauseBody {
                kind: c.kind,
                code: c.code.clone(),
            }),
            span_id: e.span_id.clone(),
        }
    }
}

/// How far a run's spans and measurements have got in relation to its journal.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTelemetryBody {
    pub run_id: String,
    pub trace_id: Option<String>,
    /// `CURRENT`, `CATCHING_UP`, `BEHIND`, `NO_TRACE` or `UNTRACKED`.
    pub state: &'static str,
    /// The journal's last sequence and the last whose spans were taken, when the run is tracked.
    pub head_seq: Option<u64>,
    pub applied_seq: Option<u64>,
    /// For how long the run has had changes that are not projected, in milliseconds.
    pub lag_ms: u64,
    /// How late spans may be before the state is `BEHIND`.
    pub budget_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptBody {
    pub node: String,
    pub attempt: u32,
    pub generation: u64,
    pub span_id: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub ready_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub leased_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub ended_at: Option<OffsetDateTime>,
    /// `open`, `succeeded`, `failed`, `retry`, `review`, `unknown_outcome` or `cancelled`.
    pub outcome: &'static str,
    pub code: Option<String>,
    /// From ready to leased, in milliseconds, when the journal says when it became ready.
    pub schedule_latency_ms: Option<u64>,
}

impl From<&sagat_application::telemetry::Attempt> for AttemptBody {
    fn from(a: &sagat_application::telemetry::Attempt) -> Self {
        Self {
            node: a.node.clone(),
            attempt: a.attempt,
            generation: a.generation,
            span_id: a.span.to_string(),
            ready_at: a.ready_at,
            leased_at: a.leased_at,
            ended_at: a.ended_at,
            outcome: a.outcome.as_str(),
            code: a.code.clone(),
            schedule_latency_ms: a.ready_at.map(|ready| {
                u64::try_from((a.leased_at - ready).whole_milliseconds().max(0)).unwrap_or(0)
            }),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptListBody {
    pub run_id: String,
    pub trace_id: Option<String>,
    pub items: Vec<AttemptBody>,
}

/// The definition's graph overlaid with the run's step states. Nodes that did not start yet are
/// `NOT_STARTED`; the editor can draw the planned graph and the runtime state in one picture.
pub fn runtime_graph(graph: &GraphSnapshot) -> Value {
    json!({
        "runId": graph.run.info.run_id.to_string(),
        "status": graph.run.status.as_str(),
        "projectionSeq": graph.run.version,
        "projectionLagMs": 0,
        "nodes": runtime_nodes(graph, |_| true),
        "edges": runtime_edges(graph),
    })
}

/// The nodes of the runtime graph that `wanted` accepts (by the node and the state of its step).
fn runtime_nodes(graph: &GraphSnapshot, wanted: impl Fn(&NodeView<'_>) -> bool) -> Vec<Value> {
    let steps: std::collections::BTreeMap<_, _> =
        graph.steps.iter().map(|s| (s.node.as_str(), s)).collect();
    let sagas: std::collections::BTreeMap<_, _> =
        graph.sagas.iter().map(|s| (s.node.as_str(), s)).collect();
    let mut nodes = Vec::new();
    // Every node the engine can run: the compensations and lookups derived from the author's
    // nodes are steps of the run too, and the ones an operator is asked to decide on.
    for node in graph.definition.all_nodes() {
        let id = node.id().as_str();
        let step = steps.get(id);
        let view = NodeView { id };
        if !wanted(&view) {
            continue;
        }
        let origin = node.origin();
        nodes.push(json!({
            "id": id,
            "type": node.kind().label(),
            "kind": if node.id().is_compensation() {
                "COMPENSATION"
            } else if node.id().is_reconciliation() {
                "RECONCILIATION"
            } else {
                "FORWARD"
            },
            "forward": origin.forward().map(|f| f.as_str()),
            "saga": sagas.get(id).map(|saga| json!({
                "phase": saga.phase,
                "cause": saga.cause,
                "outcome": saga.outcome,
                "irreversible": saga.irreversible.iter().map(|n| n.as_str()).collect::<Vec<_>>(),
            })),
            "status": step.map_or("NOT_STARTED", |s| s.status.as_str()),
            "attempt": step.map_or(0, |s| s.attempt),
            "generation": step.map_or(0, |s| s.generation),
            "cancelRequested": step.is_some_and(|s| s.cancel_requested),
            "failureCode": step.and_then(|s| s.failure_code.clone()),
            "nextAttemptAt": step.and_then(|s| s.next_attempt_at).map(rfc3339),
            "dueAt": step.and_then(|s| s.due_at).map(rfc3339),
            "waitingOn": step.and_then(|s| s.waiting_on),
            "childRunId": step.and_then(|s| s.child_run).map(|r| r.to_string()),
        }));
    }
    nodes
}

/// What a filter over the nodes of a runtime graph can look at.
pub struct NodeView<'a> {
    pub id: &'a str,
}

fn runtime_edges(graph: &GraphSnapshot) -> Vec<Value> {
    let mut edges = Vec::new();
    for node in graph.definition.all_nodes() {
        edges_of(node, &mut edges);
    }
    // The order of the compensations, and which node each lookup is about: a compensation runs
    // after the ones listed in its plan, and `x~r` answers for `x`.
    for plan in sagat_domain::saga::compensation_plans(&graph.definition) {
        for entry in &plan.entries {
            for waited_for in &entry.after {
                edges.push(json!({
                    "from": waited_for.compensation().as_str(),
                    "to": entry.compensation.as_str(),
                    "kind": "compensation",
                }));
            }
        }
    }
    for node in graph.definition.all_nodes() {
        if node.id().is_reconciliation()
            && let Some(forward) = node.origin().forward()
        {
            edges.push(json!({
                "from": forward.as_str(),
                "to": node.id().as_str(),
                "kind": "reconcile",
            }));
        }
    }
    edges
}

/// What changed in a run's graph since a journal sequence (D141): the events, and the nodes they
/// touched. A node changes only through an event that names it or activates it: a cancel request is
/// a `step.cancel_requested` of each step, the phases of a saga are events of the saga node, and the
/// events that create steps (a routing, a fork, a saga entered, a compensation plan, a lookup) carry
/// the ones they create (`crates/domain/src/execution/event.rs`, `RunEvent::changed_nodes`); the
/// events that name no node (`run.paused`, `run.resumed`, `run.stop_requested`, `run.finished`)
/// change the run, whose `status` is always current. `reset` brings the whole graph.
pub fn graph_changes_body(changes: &GraphChanges) -> Value {
    let graph = &changes.graph;
    let touched: std::collections::BTreeSet<&str> = changes
        .events
        .iter()
        .flat_map(|e| e.touched.iter().flatten().map(String::as_str))
        .collect();
    let any = !changes.events.is_empty();
    let nodes = if changes.reset {
        runtime_nodes(graph, |_| true)
    } else if any {
        runtime_nodes(graph, |n| touched.contains(n.id))
    } else {
        Vec::new()
    };
    let mut body = json!({
        "runId": graph.run.info.run_id.to_string(),
        "status": graph.run.status.as_str(),
        // A reset is relative to nothing: the client replaces what it has.
        "fromSeq": if changes.reset { 0 } else { changes.since },
        "projectionSeq": graph.run.version,
        "reset": changes.reset,
        "events": changes.events.iter().map(|e| json!({
            "seq": e.seq,
            "eventType": e.event_type,
            "occurredAt": rfc3339(e.occurred_at),
            "stepId": e.node,
            "attempt": e.attempt,
        })).collect::<Vec<_>>(),
        "nodes": nodes,
    });
    if changes.reset {
        body["edges"] = Value::Array(runtime_edges(graph));
    }
    body
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
        // The body of a saga starts where it says: without this edge the steps inside a saga look
        // like separate graphs.
        NodeKind::Saga { begin, .. } => edge(begin.as_str(), "begin"),
        _ => {}
    }
    if let Some(next) = node.next() {
        edge(next.as_str(), "next");
    }
    if let Some(target) = node.on_error() {
        edge(target.as_str(), "error");
    }
}

// ------------------------------------------------------------------------------ saga plans

/// The compensation plans a version was published with.
pub fn compensation_plans_body(plans: &[StoredPlan]) -> Value {
    json!({
        "sagas": plans.iter().map(|plan| json!({
            "saga": plan.saga.as_str(),
            "onCompensationFailure": plan.on_compensation_failure.as_str(),
            "cancelPolicy": plan.cancel_policy.as_str(),
            "irreversible": plan.irreversible.iter().map(|n| n.as_str()).collect::<Vec<_>>(),
            "compensations": plan.entries.iter().enumerate().map(|(position, entry)| json!({
                "position": position,
                "node": entry.node.as_str(),
                "compensationId": entry.compensation_id.to_string(),
                "activity": entry.activity.as_str(),
                "after": entry.after.iter().map(|n| n.as_str()).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}
