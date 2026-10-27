//! Native scoped wait clock acknowledgements; external event delivery remains unsupported.
use super::*;

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
#[derive(Clone)]
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
