//! Wire types of the definition and run routes (camelCase, RFC 3339 timestamps, no tenant).

use sagat_application::definitions::{StoredPlan, VersionRecord, WorkflowRecord};
use sagat_application::execution::RunRecord;
use sagat_application::runs::{GraphChanges, GraphSnapshot, JournalEntry};
use sagat_domain::definition::WorkflowVersion;
use sagat_domain::execution::{ExecutionInstanceId, InstancePath, SwitchSelection};
use sagat_domain::workflow::{Node, NodeId, NodeKind};
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
    let mut body = json!({
        "valid": validation.is_valid(),
        "digest": validation.digest,
        "problems": validation.problems.iter().map(problem_body).collect::<Vec<_>>(),
        "warnings": validation.warnings.iter().map(problem_body).collect::<Vec<_>>(),
    });
    if let Some(analysis) = &validation.classification {
        use sagat_application::classification::CarrierPointer;
        body["classification"] = Value::Array(analysis.report.iter().map(|entry| {
            let mut value = json!({"path": entry.path.as_str(), "level": entry.level.as_str(), "declared": entry.declared});
            match &entry.carrier {
                CarrierPointer::Input => value["carrier"] = json!("inputSchema"),
                CarrierPointer::Result => value["carrier"] = json!("outputSchema"),
                CarrierPointer::NodeOutput(node) => {
                    value["carrier"] = json!("nodeOutputSchema");
                    value["nodeId"] = json!(node.as_str());
                },
                CarrierPointer::BodyNodeOutput { body, node } => {
                    value["carrier"] = json!("nodeOutputSchema");
                    value["nodeId"] = json!(node.as_str());
                    value["bodyName"] = json!(body.as_str());
                },
            }
            value
        }).collect());
        body["classificationTruncated"] = json!(analysis.report_truncated);
    }
    body
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
    /// The payload classification rules pinned to the version: `0` for a version published before
    /// the feature (no value of its runs is disclosed), `1` since.
    pub classification_rules: u8,
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
            classification_rules: v.classification_rules().as_u8(),
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
    /// The digests of the script artifacts the version pins (only when one version is read).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scripts: Option<Vec<String>>,
}

impl From<&VersionRecord> for VersionBody {
    fn from(r: &VersionRecord) -> Self {
        Self {
            version: VersionSummary::from(&r.version),
            release_notes: r.release_notes.clone(),
            document: serde_json::from_slice(&r.document).unwrap_or(Value::Null),
            scripts: None,
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
    #[serde(default, deserialize_with = "present_json")]
    pub input: Option<Value>,
    #[serde(default, deserialize_with = "present_blob_ref")]
    pub input_blob_ref: Option<String>,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub deadline: Option<OffsetDateTime>,
}

fn present_json<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

fn present_blob_ref<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
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
    /// For a decision about a step of an iteration of a collection or loop: the scopes around the
    /// step, outermost first, each with the index of the iteration. Absent for a step of the root
    /// graph.
    #[serde(default)]
    pub instance_path: Option<Vec<ScopeSegmentBody>>,
}

/// One scope around a step: the collection or loop node and the iteration the step runs in
/// (`ApprovalScope` in the OpenAPI, the same shape the runtime graph reports).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeSegmentBody {
    pub scope_node_id: String,
    pub iteration_index: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResultBody {
    pub operation_id: String,
    pub accepted: bool,
    pub run_status: String,
    pub run_version: u64,
    /// For `RERUN_AS_NEW`: the run that was started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_run_id: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<&'static str>,
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
    /// The ended run this one was started again from (`RERUN_AS_NEW`), when it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerun_of: Option<String>,
    /// How deeply nested the run is: 0 when started through the API.
    pub depth: u8,
    /// The run's segment in its continuation chain; absent for a run outside any chain. A segment
    /// that is `SUCCEEDED` with `completionKind` `CONTINUED` is not the process outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain: Option<RunChainBody>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunChainBody {
    pub chain_id: String,
    pub segment_index: u16,
    pub completion_kind: Option<&'static str>,
    pub successor_run_id: Option<String>,
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
            origin: r
                .info
                .origin
                .map(sagat_application::runs::RunOrigin::as_str),
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
            rerun_of: r.info.rerun_of.map(|r| r.to_string()),
            depth: r.info.depth,
            chain: r.chain.map(|segment| RunChainBody {
                chain_id: segment.chain_id.to_string(),
                segment_index: segment.index,
                completion_kind: segment.completion_kind.map(|kind| match kind {
                    sagat_domain::execution::SegmentCompletionKind::Continued => "CONTINUED",
                    sagat_domain::execution::SegmentCompletionKind::Final => "FINAL",
                }),
                successor_run_id: segment.successor.map(|run| run.to_string()),
            }),
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
        "chain": graph.chain.as_ref().map(chain_body),
    })
}

fn chain_body(chain: &sagat_application::runs::ChainView) -> Value {
    json!({
        "chainId": chain.chain_id.to_string(),
        "segmentIndex": chain.segment_index,
        "segmentCount": chain.segment_count,
        "currentRunId": chain.current_run_id.to_string(),
        "predecessorRunId": chain.predecessor_run_id.map(|run| run.to_string()),
        "successorRunId": chain.successor_run_id.map(|run| run.to_string()),
        "completionKind": chain.completion_kind,
        "finalOutcome": chain.final_outcome,
    })
}

/// The nodes of the runtime graph that `wanted` accepts (by the node and the state of its step).
fn runtime_nodes(graph: &GraphSnapshot, wanted: impl Fn(&NodeView<'_>) -> bool) -> Vec<Value> {
    let steps: std::collections::BTreeMap<_, _> = graph
        .steps
        .iter()
        .filter(|step| step.instance.path().is_root())
        .map(|step| (step.node.as_str(), step))
        .collect();
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
        let switch_selection =
            switch_selection_body(node.kind(), step.and_then(|s| s.switch_selection.as_ref()));
        nodes.push(json!({
            "id": id,
            "type": if node.database_operation().is_some() { "database" } else { node.kind().label() },
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
            "switchSelection": switch_selection,
            "approvalDecision": approval_decision_body(step.and_then(|s| s.approval.as_ref())),
        }));
    }
    for step in graph.steps.iter().filter(|step| {
        !step.instance.path().is_root()
            || graph.definition.node_for_instance(&step.instance).is_none()
    }) {
        let id = instance_graph_id(&step.instance);
        if !wanted(&NodeView { id: &id }) {
            continue;
        }
        let node = graph.definition.node_for_instance(&step.instance);
        let mut projected = json!({
            "id": id,
            "type": node.map_or("unsupported", |node| node.kind().label()),
            "kind": if step.node.is_compensation() {
                "COMPENSATION"
            } else if step.node.is_reconciliation() {
                "RECONCILIATION"
            } else {
                "FORWARD"
            },
            "forward": node.and_then(|node| node.origin().forward()).map(|id| id.as_str()),
            "saga": null,
            "status": step.status.as_str(),
            "attempt": step.attempt,
            "generation": step.generation,
            "cancelRequested": step.cancel_requested,
            "failureCode": step.failure_code,
            "nextAttemptAt": step.next_attempt_at.map(rfc3339),
            "dueAt": step.due_at.map(rfc3339),
            "waitingOn": step.waiting_on,
            "childRunId": step.child_run.map(|run| run.to_string()),
            "switchSelection": node.map_or(Value::Null, |node| switch_selection_body(node.kind(), step.switch_selection.as_ref())),
            "approvalDecision": approval_decision_body(step.approval.as_ref()),
        });
        if !step.instance.path().is_root() {
            projected["nodeId"] = json!(step.node.as_str());
            projected["instancePath"] = json!(
                step.instance
                    .path()
                    .segments()
                    .iter()
                    .map(|segment| json!({
                        "scopeNodeId": segment.scope_node().as_str(),
                        "iterationIndex": segment.iteration_index(),
                    }))
                    .collect::<Vec<_>>()
            );
        }
        nodes.push(projected);
    }
    nodes
}

fn instance_graph_id(instance: &ExecutionInstanceId) -> String {
    let mut id = instance_path_prefix(instance.path());
    id.push_str(instance.node().as_str());
    id
}

fn instance_path_prefix(path: &InstancePath) -> String {
    let mut id = String::new();
    for segment in path.segments() {
        use std::fmt::Write;
        let _ = write!(
            id,
            "{}[{}]/",
            segment.scope_node().as_str(),
            segment.iteration_index()
        );
    }
    id
}

fn switch_selection_body(kind: &NodeKind, selection: Option<&SwitchSelection>) -> Value {
    match (kind, selection) {
        (NodeKind::Switch { cases, .. }, Some(SwitchSelection::Case(case_id))) => cases
            .iter()
            .find(|case| case.id() == case_id)
            .map_or_else(
                || json!({ "kind": "UNKNOWN", "caseId": case_id.as_str(), "target": null }),
                |case| json!({ "kind": "CASE", "caseId": case_id.as_str(), "target": case.target().as_str() }),
            ),
        (NodeKind::Switch { default, .. }, Some(SwitchSelection::Default)) => {
            json!({ "kind": "DEFAULT", "target": default.as_str() })
        }
        _ => Value::Null,
    }
}

/// How the approval request of an approval node stands: `PENDING`, then the settled outcome and the
/// pinned target it routed to. Null until the request opens. It never names the approver, the
/// decision key or the reason code.
fn approval_decision_body(approval: Option<&sagat_application::runs::ApprovalView>) -> Value {
    approval.map_or(Value::Null, |view| {
        json!({
            "kind": view.status.as_str(),
            "target": view.routed_node.as_ref().map(NodeId::as_str),
        })
    })
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
    let present: std::collections::BTreeSet<_> = graph
        .steps
        .iter()
        .map(|step| instance_graph_id(&step.instance))
        .collect();
    let paths: std::collections::BTreeSet<_> = graph
        .steps
        .iter()
        .map(|step| step.instance.path().clone())
        .filter(|path| !path.is_root())
        .collect();
    for path in paths {
        let mut parent = InstancePath::root();
        for segment in &path.segments()[..path.depth() - 1] {
            if let Ok(next) = parent.child(segment.scope_node().clone(), segment.iteration_index())
            {
                parent = next;
            }
        }
        let Some(owner) = path.segments().last() else {
            continue;
        };
        let owner_instance = ExecutionInstanceId::in_path(parent, owner.scope_node().clone());
        let Some(owner_node) = graph.definition.node_for_instance(&owner_instance) else {
            continue;
        };
        let body_name = match owner_node.kind() {
            NodeKind::ForEach { body, .. }
            | NodeKind::While { body, .. }
            | NodeKind::RepeatUntil { body, .. } => body,
            _ => continue,
        };
        let Some(body) = graph.definition.body(body_name) else {
            continue;
        };
        let prefix = instance_path_prefix(&path);
        let entry = format!("{}{}", prefix, body.entrypoint().as_str());
        let owner_id = instance_graph_id(&owner_instance);
        if present.contains(&owner_id) && present.contains(&entry) {
            edges.push(json!({ "from": owner_id, "to": entry, "kind": "begin" }));
        }
        for node in body.all_nodes() {
            let mut local = Vec::new();
            edges_of(node, &mut local);
            for mut edge in local {
                let (Some(from), Some(to)) = (
                    edge.get("from").and_then(Value::as_str),
                    edge.get("to").and_then(Value::as_str),
                ) else {
                    continue;
                };
                let from = format!("{prefix}{from}");
                let to = format!("{prefix}{to}");
                if present.contains(&from) && present.contains(&to) {
                    edge["from"] = json!(from);
                    edge["to"] = json!(to);
                    edges.push(edge);
                }
            }
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
    let reset = changes.reset;
    let touched: std::collections::BTreeSet<String> = changes
        .events
        .iter()
        .flat_map(|event| event.touched.iter().flatten().map(instance_graph_id))
        .collect();
    let any = !changes.events.is_empty();
    let nodes = if reset {
        runtime_nodes(graph, |_| true)
    } else if any {
        runtime_nodes(graph, |node| touched.contains(node.id))
    } else {
        Vec::new()
    };
    let mut body = json!({
        "runId": graph.run.info.run_id.to_string(),
        "status": graph.run.status.as_str(),
        // A reset is relative to nothing: the client replaces what it has.
        "fromSeq": if reset { 0 } else { changes.since },
        "projectionSeq": graph.run.version,
        "reset": reset,
        "events": changes.events.iter().map(|e| json!({
            "seq": e.seq,
            "eventType": e.event_type,
            "occurredAt": rfc3339(e.occurred_at),
            "stepId": e.node,
            "attempt": e.attempt,
        })).collect::<Vec<_>>(),
        "nodes": nodes,
        "chain": graph.chain.as_ref().map(chain_body),
    });
    if reset {
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
    let edge = |out: &mut Vec<Value>, to: &str, kind: &str| {
        out.push(json!({ "from": from, "to": to, "kind": kind }));
    };
    match node.kind() {
        NodeKind::Condition {
            if_true, if_false, ..
        } => {
            edge(out, if_true.as_str(), "true");
            edge(out, if_false.as_str(), "false");
        }
        NodeKind::Switch { cases, default } => {
            for case in cases {
                out.push(json!({
                    "from": from,
                    "to": case.target().as_str(),
                    "kind": "case",
                    "caseId": case.id().as_str(),
                }));
            }
            edge(out, default.as_str(), "default");
        }
        NodeKind::Parallel { branches, .. } => {
            for branch in branches {
                edge(out, branch.as_str(), "branch");
            }
        }
        NodeKind::WaitEvent {
            on_timeout: Some(target),
            ..
        } => edge(out, target.as_str(), "timeout"),
        // The three routes of an approval are all declared; the decision that took one is the
        // node's `approvalDecision`.
        NodeKind::Approval {
            on_approve,
            on_reject,
            on_expire,
            ..
        } => {
            edge(out, on_approve.as_str(), "approve");
            edge(out, on_reject.as_str(), "reject");
            edge(out, on_expire.as_str(), "expire");
        }
        // The body of a saga starts where it says: without this edge the steps inside a saga look
        // like separate graphs.
        NodeKind::Saga { begin, .. } => edge(out, begin.as_str(), "begin"),
        _ => {}
    }
    if let Some(next) = node.next() {
        edge(out, next.as_str(), "next");
    }
    if let Some(target) = node.on_error() {
        edge(out, target.as_str(), "error");
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

#[cfg(test)]
mod switch_graph_tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;
    use sagat_application::runs::StepView;
    use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
    use sagat_domain::execution::{InstancePath, RunId, RunStatus, StepStatus};
    use sagat_domain::identity::{NamespaceId, Slug, TenantId};
    use sagat_domain::workflow::{
        BodyDefinition, Expression, NodeId, SwitchCase, WorkflowDefinition, WorkflowLimits,
    };

    fn id(value: &str) -> NodeId {
        NodeId::parse(value).expect("valid node id")
    }

    #[test]
    fn scoped_graph_ids_distinguish_equal_local_names_and_iterations() {
        let root = ExecutionInstanceId::root(id("charge"));
        let first = InstancePath::root().child(id("items"), 0).unwrap();
        let second = InstancePath::root().child(id("items"), 1).unwrap();
        let nested = first.child(id("retry"), 2).unwrap();
        assert_eq!(instance_graph_id(&root), "charge");
        assert_eq!(
            instance_graph_id(&ExecutionInstanceId::in_path(first, id("charge"))),
            "items[0]/charge"
        );
        assert_eq!(
            instance_graph_id(&ExecutionInstanceId::in_path(second, id("charge"))),
            "items[1]/charge"
        );
        assert_eq!(
            instance_graph_id(&ExecutionInstanceId::in_path(nested, id("charge"))),
            "items[0]/retry[2]/charge"
        );
    }

    #[test]
    fn runtime_graph_projects_scoped_steps_and_precise_deltas() {
        let definition = WorkflowDefinition::new_structured(
            Slug::parse("items-flow").unwrap(),
            id("items"),
            vec![
                Node::new(
                    id("items"),
                    NodeKind::ForEach {
                        items: Expression::parse("input.items").unwrap(),
                        body: id("line"),
                        result: Expression::parse("bodyResult").unwrap(),
                        max_items: 2,
                        max_concurrency: 2,
                    },
                )
                .with_next(id("done")),
                Node::new(id("done"), NodeKind::End { output: None }),
            ],
            vec![
                BodyDefinition::new(
                    id("line"),
                    id("return"),
                    vec![Node::new(
                        id("return"),
                        NodeKind::Return {
                            result: Expression::parse("item").unwrap(),
                        },
                    )],
                )
                .unwrap(),
            ],
            WorkflowLimits::default(),
        )
        .unwrap();
        let run_id = RunId::new(uuid::Uuid::from_u128(1));
        let now = OffsetDateTime::UNIX_EPOCH;
        let step = |index| StepView {
            instance: ExecutionInstanceId::in_path(
                InstancePath::root().child(id("items"), index).unwrap(),
                id("return"),
            ),
            node: id("return"),
            status: StepStatus::Succeeded,
            attempt: 0,
            generation: 0,
            cancel_requested: false,
            failure_code: None,
            next_attempt_at: None,
            due_at: None,
            waiting_on: None,
            child_run: None,
            switch_selection: None,
            approval: None,
        };
        let graph = GraphSnapshot {
            run: RunRecord {
                info: sagat_application::execution::RunInfo {
                    run_id,
                    tenant_id: TenantId::new(uuid::Uuid::from_u128(2)),
                    namespace_id: NamespaceId::new(uuid::Uuid::from_u128(3)),
                    workflow_id: WorkflowId::new(uuid::Uuid::from_u128(4)),
                    version_id: WorkflowVersionId::new(uuid::Uuid::from_u128(5)),
                    started_by: "test".into(),
                    correlation_id: None,
                    trace_id: None,
                    rerun_of: None,
                    parent_span: None,
                    deadline_at: None,
                    created_at: now,
                    parent: None,
                    depth: 0,
                    root: run_id,
                    origin: None,
                },
                status: RunStatus::Running,
                version: 3,
                stop_reason: None,
                paused: false,
                updated_at: now,
                finished_at: None,
                chain: None,
            },
            definition: std::sync::Arc::new(definition),
            steps: vec![
                StepView {
                    instance: ExecutionInstanceId::root(id("items")),
                    node: id("items"),
                    status: StepStatus::Waiting,
                    waiting_on: Some("branches"),
                    ..step(0)
                },
                step(0),
                step(1),
                StepView {
                    instance: ExecutionInstanceId::root(id("future-node")),
                    node: id("future-node"),
                    status: StepStatus::Running,
                    ..step(0)
                },
            ],
            sagas: Vec::new(),
            chain: Some(sagat_application::runs::ChainView {
                chain_id: run_id,
                segment_index: 1,
                segment_count: 2,
                current_run_id: run_id,
                predecessor_run_id: Some(RunId::new(uuid::Uuid::from_u128(6))),
                successor_run_id: None,
                completion_kind: None,
                final_outcome: None,
            }),
        };
        let body = runtime_graph(&graph);
        assert_eq!(body["chain"]["segmentIndex"], 1);
        assert_eq!(
            body["chain"]["predecessorRunId"],
            uuid::Uuid::from_u128(6).to_string()
        );
        let nodes = body["nodes"].as_array().unwrap();
        assert!(nodes.iter().any(|node| {
            node["id"] == "items[0]/return" && node["instancePath"][0]["iterationIndex"] == 0
        }));
        assert!(nodes.iter().any(|node| {
            node["id"] == "items[1]/return" && node["instancePath"][0]["iterationIndex"] == 1
        }));
        assert!(nodes.iter().any(|node| {
            node["id"] == "future-node"
                && node["type"] == "unsupported"
                && node.get("instancePath").is_none()
        }));
        assert!(body["edges"].as_array().unwrap().iter().any(|edge| {
            edge["from"] == "items" && edge["to"] == "items[0]/return" && edge["kind"] == "begin"
        }));
        let changes = graph_changes_body(&GraphChanges {
            graph,
            since: 2,
            reset: false,
            events: vec![JournalEntry {
                seq: 3,
                event_type: "step.completed".into(),
                node: Some("return".into()),
                attempt: Some(0),
                actor: "worker:test".into(),
                trace_id: None,
                occurred_at: now,
                payload_ref: None,
                cause: None,
                span_id: None,
                touched: Some(vec![ExecutionInstanceId::in_path(
                    InstancePath::root().child(id("items"), 1).unwrap(),
                    id("return"),
                )]),
            }],
        });
        assert_eq!(changes["reset"], false);
        assert_eq!(changes["fromSeq"], 2);
        assert_eq!(changes["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(changes["nodes"][0]["id"], "items[1]/return");
        assert_eq!(changes["chain"], body["chain"]);
    }

    #[test]
    fn switch_graph_exposes_persisted_selection_and_ordered_routes() {
        let node = Node::new(
            id("choose"),
            NodeKind::Switch {
                cases: vec![
                    SwitchCase::new(id("first"), Expression::parse("true").unwrap(), id("left")),
                    SwitchCase::new(
                        id("second"),
                        Expression::parse("true").unwrap(),
                        id("right"),
                    ),
                ],
                default: id("fallback"),
            },
        );
        let mut edges = Vec::new();
        edges_of(&node, &mut edges);
        assert_eq!(
            edges,
            vec![
                json!({"from":"choose","to":"left","kind":"case","caseId":"first"}),
                json!({"from":"choose","to":"right","kind":"case","caseId":"second"}),
                json!({"from":"choose","to":"fallback","kind":"default"}),
            ]
        );
        assert_eq!(
            switch_selection_body(node.kind(), Some(&SwitchSelection::Case(id("second")))),
            json!({"kind":"CASE","caseId":"second","target":"right"})
        );
        assert_eq!(
            switch_selection_body(node.kind(), Some(&SwitchSelection::Default)),
            json!({"kind":"DEFAULT","target":"fallback"})
        );
        assert_eq!(switch_selection_body(node.kind(), None), Value::Null);
    }

    #[test]
    fn approval_routes_are_edges_and_the_decision_names_no_one() {
        use sagat_application::runs::ApprovalView;
        use sagat_domain::execution::{ApprovalPolicy, ApprovalStatus};
        use sagat_domain::identity::Role;

        let node = Node::new(
            id("review"),
            NodeKind::Approval {
                policy: ApprovalPolicy::new(Vec::new(), vec![Role::Operator]).unwrap(),
                on_approve: id("approved"),
                on_reject: id("rejected"),
                on_expire: id("expired"),
            },
        );
        let mut edges = Vec::new();
        edges_of(&node, &mut edges);
        assert_eq!(
            edges,
            vec![
                json!({"from":"review","to":"approved","kind":"approve"}),
                json!({"from":"review","to":"rejected","kind":"reject"}),
                json!({"from":"review","to":"expired","kind":"expire"}),
            ]
        );
        assert_eq!(approval_decision_body(None), Value::Null);
        assert_eq!(
            approval_decision_body(Some(&ApprovalView {
                status: ApprovalStatus::Pending,
                routed_node: None,
            })),
            json!({"kind":"PENDING","target":null})
        );
        assert_eq!(
            approval_decision_body(Some(&ApprovalView {
                status: ApprovalStatus::Expired,
                routed_node: Some(id("expired")),
            })),
            json!({"kind":"EXPIRED","target":"expired"})
        );
    }
}
