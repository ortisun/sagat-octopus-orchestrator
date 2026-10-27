//! Native scoped event admission receipts, outcome witnesses and scheduler clocks.
use super::*;

/// One staged native ingress receipt. Pending witnesses must not be committed alone.
pub(super) struct ScopedEventDelivery {
    pub(super) instance: ExecutionInstanceId,
    pub(super) output_digest: Vec<u8>,
    pub(super) persisted: bool,
}

/// Once a scoped receipt operation is attempted, any failed persist permanently refuses commit.
/// Arm before validation/awaits; success restores prior failure history, never erases it.
/// An abandoned future stays armed. Legacy retry behavior without receipt attempts is unchanged.
pub(super) fn begin_persistence(attempted: bool, failed: &mut bool) -> Result<bool, StoreError> {
    if attempted && *failed {
        return Err(corrupt(
            "scoped event transaction contains a failed persistence",
        ));
    }
    Ok(std::mem::replace(failed, true))
}

/// A retained scoped event value can only come from this transaction's locked admission.
/// Returns whether the complete successful persistence must consume the staged witness.
pub(super) fn require_delivery(
    definition: &WorkflowDefinition,
    events: &[AddressedRunEvent],
    outputs: &[(ExecutionInstanceId, Vec<u8>)],
    effects: &Effects,
    witness: Option<&ScopedEventDelivery>,
) -> Result<bool, StoreError> {
    let is_wait = |instance: &ExecutionInstanceId| {
        !instance.path().is_root()
            && matches!(
                definition.node_for_instance(instance).map(Node::kind),
                Some(NodeKind::WaitEvent { .. })
            )
    };
    let mut satisfied = None;
    for addressed in events {
        let Some(instance) = addressed.instance() else {
            continue;
        };
        if matches!(addressed.event(), RunEvent::WaitSatisfied { .. }) {
            if !is_wait(&instance) || satisfied.replace(instance).is_some() {
                return Err(corrupt("scoped event satisfaction requires one exact wait"));
            }
        } else if is_wait(&instance)
            && matches!(
                addressed.event(),
                RunEvent::StepCompleted { .. }
                    | RunEvent::ReconciliationConcluded { .. }
                    | RunEvent::StepResolved { .. }
            )
        {
            return Err(corrupt(
                "generic completion cannot manufacture a scoped event value",
            ));
        }
    }
    if outputs
        .iter()
        .any(|(instance, _)| is_wait(instance) && satisfied.as_ref() != Some(instance))
    {
        return Err(corrupt("scoped event output has no satisfaction"));
    }
    let Some(instance) = satisfied else {
        if witness.is_some_and(|receipt| !receipt.persisted) {
            return Err(corrupt("scoped event receipt has no matching outcome"));
        }
        return Ok(false);
    };
    let receipt = witness
        .filter(|receipt| !receipt.persisted && receipt.instance == instance)
        .ok_or_else(|| corrupt("scoped event satisfaction has no current receipt"))?;
    if outputs.len() != 1
        || outputs[0].0 != instance
        || Sha256::digest(&outputs[0].1).as_slice() != receipt.output_digest
        || effects.lease.is_some()
        || effects.renewal.is_some()
        || effects.reported.is_some()
        || !effects.blobs.is_empty()
        || !effects.children.is_empty()
    {
        return Err(corrupt(
            "scoped event satisfaction and canonical value diverged",
        ));
    }
    Ok(true)
}

pub(super) fn allows_outcome(state: &sagat_domain::execution::PathExecutionState) -> bool {
    !state.global().status().is_terminal()
        && state.global().stop_reason().is_none()
        && state
            .ancestors()
            .iter()
            .all(sagat_domain::execution::AncestorExecutionFence::allows_forward_progress)
}

pub(super) async fn prepare_delivery(
    tx: &mut Transaction<'_, Postgres>,
    definition: &WorkflowDefinition,
    info: &RunInfo,
    source: &BTreeMap<ExecutionInstanceId, EventWaitReadState>,
    step: &sagat_application::dispatch::WaitingStep,
    event: &sagat_application::dispatch::ClaimedEvent,
    eligible: bool,
) -> Result<Option<(ScopedEventDelivery, Vec<u8>)>, StoreError> {
    if !eligible
        || step.instance.path().is_root()
        || step.tenant_id != info.tenant_id
        || event.tenant_id != info.tenant_id
        || step.namespace_id != info.namespace_id
        || event.namespace_id != info.namespace_id
        || step.run_id != info.run_id
        || step.step_id != instance_step_id(info.version_id, info.run_id, &step.instance)
    {
        return Ok(None);
    }
    let Some(node) = definition.node_for_instance(&step.instance) else {
        return Ok(None);
    };
    let Some(read) = source.get(&step.instance) else {
        return Ok(None);
    };
    if !matches!(node.kind(), NodeKind::WaitEvent { .. })
        || read.status != StepStatus::Waiting
        || read.routed
        || read.key.as_ref() != Some(&step.correlation_key)
        || read.event_type(Some(node)) != Some(step.event_type.as_str())
        || read.since(Some(node))? != Some(step.since)
    {
        return Ok(None);
    }
    let row=sqlx::query("SELECT tenant_id,namespace_id,source,event_id,event_type,correlation_key,subject,occurred_at,payload,payload_digest,received_at,attempts,trace_id,trace_parent_span FROM inbox_events WHERE tenant_id=$1 AND namespace_id=$2 AND source=$3 AND event_id=$4 FOR SHARE")
        .bind(info.tenant_id.as_uuid()).bind(info.namespace_id.as_uuid()).bind(event.envelope.key.source.as_str()).bind(event.envelope.key.event_id.as_str())
        .fetch_optional(&mut **tx).await.map_err(store_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let current = crate::dispatch::claimed_from_row(&row)?;
    if current.envelope != event.envelope
        || current.occurred_at != event.occurred_at
        || current.payload != event.payload
        || current.received_at != event.received_at
    {
        return Ok(None);
    }
    if sagat_application::events::event_admission_digest(&current.envelope, &current.payload)
        != col::<Vec<u8>>(&row, "payload_digest")?
        || !serde_json::from_slice::<serde_json::Value>(&current.payload)
            .is_ok_and(|value| value.is_object())
    {
        return Err(corrupt("invalid current scoped event admission"));
    }
    if current.envelope.event_type != step.event_type
        || current.envelope.correlation_key.as_ref() != Some(&step.correlation_key)
        || current.received_at < step.since
    {
        return Ok(None);
    }
    let (nodes, indices) = stored_path_parts(step.instance.path());
    let row=sqlx::query(concat!("SELECT s.namespace_id,s.step_id,s.node_kind AS stored_node_kind,s.status,s.attempt,s.generation,s.cancel_requested,s.routed,s.condition,s.switch_case_id,s.timed_out,s.failure_code,s.next_attempt_at,s.waiting_kind,s.due_at,s.correlation_key,s.event_type,s.event_since,s.child_run_id,s.approval_request_id, (",scoped_event_metadata_valid!(),") AS event_metadata_valid FROM steps s WHERE tenant_id=$1 AND run_id=$2 AND node=$3 AND instance_path_nodes=$4 AND instance_path_indices=$5 FOR UPDATE"))
        .bind(info.tenant_id.as_uuid()).bind(info.run_id.as_uuid()).bind(step.instance.node().as_str()).bind(&nodes).bind(&indices)
        .fetch_optional(&mut **tx).await.map_err(store_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let current_step = step_state(&row)?;
    validate_scoped_control_row(Some(node), &current_step, &row)?;
    validate_scoped_event_identity(Some(node), info.namespace_id, step.step_id, &row)?;
    if EventWaitReadState::from_step(&current_step) != *read {
        return Ok(None);
    }
    let received = sagat_application::dispatch::ClaimedEvent::rfc3339(current.received_at);
    let occurred = current
        .occurred_at
        .map(sagat_application::dispatch::ClaimedEvent::rfc3339);
    let document =
        sagat_application::dispatch::event_document(&sagat_application::dispatch::EventView {
            id: current.envelope.key.event_id.as_str(),
            source: current.envelope.key.source.as_str(),
            event_type: current.envelope.event_type.as_str(),
            subject: current.envelope.subject.as_deref(),
            correlation_key: current
                .envelope
                .correlation_key
                .as_ref()
                .map(CorrelationKey::as_str),
            occurred_at: occurred.as_deref(),
            received_at: &received,
            data: &current.payload,
        });
    if document.len() > sagat_application::execution::MAX_PAYLOAD_BYTES {
        return Err(StoreError::QuotaExceeded {
            what: "event document",
        });
    }
    let consumer = format!("wait-instance:{}/{}", info.run_id, step.step_id);
    let inserted=sqlx::query("INSERT INTO event_deliveries (tenant_id,namespace_id,source,event_id,consumer,run_id,node,trigger_id,delivered_at,instance_id,instance_path_nodes,instance_path_indices) VALUES ($1,$2,$3,$4,$5,$6,$7,NULL,clock_timestamp(),$8,$9,$10) ON CONFLICT DO NOTHING")
        .bind(info.tenant_id.as_uuid()).bind(info.namespace_id.as_uuid()).bind(current.envelope.key.source.as_str()).bind(current.envelope.key.event_id.as_str()).bind(&consumer)
        .bind(info.run_id.as_uuid()).bind(step.instance.node().as_str()).bind(step.step_id).bind(&nodes).bind(&indices).execute(&mut **tx).await.map_err(store_error)?.rows_affected();
    if inserted == 0 {
        // The runtime role has no UPDATE grant on receipts. This is a no-work observation,
        // not authority for an outcome; the parent inbox remains locked through this read.
        let existing=sqlx::query("SELECT run_id,node,trigger_id,instance_id,instance_path_nodes,instance_path_indices FROM event_deliveries WHERE tenant_id=$1 AND namespace_id=$2 AND source=$3 AND event_id=$4 AND consumer=$5")
            .bind(info.tenant_id.as_uuid()).bind(info.namespace_id.as_uuid()).bind(current.envelope.key.source.as_str()).bind(current.envelope.key.event_id.as_str()).bind(&consumer).fetch_optional(&mut **tx).await.map_err(store_error)?;
        let existing =
            existing.ok_or_else(|| corrupt("scoped event receipt conflict disappeared"))?;
        if col::<Option<Uuid>>(&existing, "run_id")? != Some(info.run_id.as_uuid())
            || col::<Option<String>>(&existing, "node")?.as_deref()
                != Some(step.instance.node().as_str())
            || col::<Option<Uuid>>(&existing, "trigger_id")?.is_some()
            || col::<Option<Uuid>>(&existing, "instance_id")? != Some(step.step_id)
            || col::<Option<Vec<String>>>(&existing, "instance_path_nodes")? != Some(nodes)
            || col::<Option<Vec<i32>>>(&existing, "instance_path_indices")? != Some(indices)
        {
            return Err(corrupt("scoped event receipt conflict identity"));
        }
        return Ok(None);
    }
    let output_digest = Sha256::digest(&document).to_vec();
    Ok(Some((
        ScopedEventDelivery {
            instance: step.instance.clone(),
            output_digest,
            persisted: false,
        },
        document,
    )))
}

pub(crate) fn validate_scoped_event_identity(
    node: Option<&Node>,
    namespace: sagat_domain::identity::NamespaceId,
    step_id: Uuid,
    row: &PgRow,
) -> Result<(), StoreError> {
    if matches!(node.map(Node::kind), Some(NodeKind::WaitEvent { .. }))
        && (col::<Uuid>(row, "namespace_id")? != namespace.as_uuid()
            || col::<Uuid>(row, "step_id")? != step_id)
    {
        return Err(corrupt("scoped event physical identity"));
    }
    Ok(())
}

pub(super) fn require_scoped_event_clock(
    definition: &WorkflowDefinition,
    events: &[AddressedRunEvent],
    source: &BTreeMap<ExecutionInstanceId, EventWaitReadState>,
    own_time: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    let mut touched = BTreeSet::new();
    for addressed in events {
        if addressed.path().is_root() {
            continue;
        }
        let Some(instance) = addressed.instance() else {
            continue;
        };
        let event_wait = matches!(
            definition.node_for_instance(&instance).map(Node::kind),
            Some(NodeKind::WaitEvent { .. })
        );
        let clocked = match addressed.event() {
            RunEvent::WaitStarted { .. } | RunEvent::WaitTimedOut { .. } => {
                if !event_wait {
                    return Err(corrupt("scoped event clock targeted another kind"));
                }
                true
            }
            RunEvent::StepFailed { code, fence, .. } if event_wait => {
                if fence.is_some() || *code != FailureCode::expression_error() {
                    return Err(corrupt(
                        "scoped event failure requires its declared transition",
                    ));
                }
                true
            }
            _ => false,
        };
        if !clocked {
            continue;
        }
        if own_time != Some(now) {
            return Err(corrupt(
                "scoped event clock requires acknowledged own scheduler authority",
            ));
        }
        if !touched.insert(instance.clone()) {
            return Err(corrupt("scoped event write repeats a clock decision"));
        }
        let read = source
            .get(&instance)
            .ok_or_else(|| corrupt("scoped event clock has no stored source"))?;
        match addressed.event() {
            RunEvent::WaitStarted { deadline, .. } => {
                let Some(NodeKind::WaitEvent { timeout, .. }) =
                    definition.node_for_instance(&instance).map(Node::kind)
                else {
                    return Err(corrupt("scoped event clock declaration"));
                };
                let expected = time::Duration::try_from(*timeout)
                    .ok()
                    .and_then(|duration| now.checked_add(duration))
                    .ok_or_else(|| corrupt("scoped event clock overflow"))?;
                if read.status != StepStatus::Pending
                    || read.routed
                    || read.deadline.is_some()
                    || read.key.is_some()
                    || *deadline != expected
                {
                    return Err(corrupt("scoped event start source or clock diverged"));
                }
            }
            RunEvent::StepFailed { .. } => {
                if read.status != StepStatus::Pending
                    || read.routed
                    || read.deadline.is_some()
                    || read.key.is_some()
                {
                    return Err(corrupt("scoped event failed start source diverged"));
                }
            }
            RunEvent::WaitTimedOut { handled, .. } => {
                let Some(NodeKind::WaitEvent { on_timeout, .. }) =
                    definition.node_for_instance(&instance).map(Node::kind)
                else {
                    return Err(corrupt("scoped event clock declaration"));
                };
                if read.status != StepStatus::Waiting
                    || read.routed
                    || read.key.is_none()
                    || !read.deadline.is_some_and(|deadline| deadline <= now)
                    || *handled != on_timeout.is_some()
                {
                    return Err(corrupt("scoped event timeout source or clock diverged"));
                }
            }
            _ => return Err(corrupt("unknown scoped event clock action")),
        }
    }
    Ok(())
}

/// Before-write binding captured from the evaluated/reopened exact logical instance.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct EventWaitReadState {
    pub(super) status: StepStatus,
    pub(super) routed: bool,
    pub(super) deadline: Option<OffsetDateTime>,
    pub(super) key: Option<CorrelationKey>,
}

impl EventWaitReadState {
    pub(super) fn from_step(step: &StepState) -> Self {
        let (deadline, key) = match step.waiting_on() {
            Some(WaitingOn::Event {
                deadline,
                correlation_key,
            }) => (Some(*deadline), Some(correlation_key.clone())),
            _ => (None, None),
        };
        Self {
            status: step.status(),
            routed: step.routed(),
            deadline,
            key,
        }
    }

    pub(super) fn event_type<'a>(&self, node: Option<&'a Node>) -> Option<&'a str> {
        match node.map(Node::kind) {
            Some(NodeKind::WaitEvent { event_type, .. }) if self.deadline.is_some() => {
                Some(event_type.as_str())
            }
            _ => None,
        }
    }

    pub(super) fn since(&self, node: Option<&Node>) -> Result<Option<OffsetDateTime>, StoreError> {
        let Some(deadline) = self.deadline else {
            return Ok(None);
        };
        let node = node.ok_or_else(|| corrupt("event binding requires its pinned declaration"))?;
        sagat_domain::execution::scoped_event_wait_since(node, deadline)
            .map(Some)
            .map_err(|_| corrupt("invalid scoped event window"))
    }
}

pub(super) fn validate_event_row(
    node: &Node,
    step: &StepState,
    row: &PgRow,
) -> Result<(), StoreError> {
    let read = EventWaitReadState::from_step(step);
    if !col::<bool>(row, "event_metadata_valid")?
        || col::<String>(row, "stored_node_kind")? != "waitEvent"
        || col::<Option<String>>(row, "event_type")?.as_deref() != read.event_type(Some(node))
        || col::<Option<OffsetDateTime>>(row, "event_since")? != read.since(Some(node))?
    {
        return Err(corrupt("scoped event physical metadata"));
    }
    Ok(())
}
