//! Typed additive-v1 chain policy and continue-as-new declarations.

use std::collections::BTreeMap;
use std::time::Duration;

use sagat_application::definitions::IrError;
use sagat_domain::identity::Slug;
use sagat_domain::workflow::{
    ContinuationPolicy, DefinitionWarning, Digest, Expression, NodeId, WorkflowDefinition,
};
use serde_json::Value;

use crate::canonical;
use crate::declared_state::{
    DeclaredStateDefinition, compiled_expression, issue, parse as parse_state,
};

/// Immutable lifetime budget shared by every segment in a continuation chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainPolicyDefinition {
    max_segments: u32,
    timeout: Duration,
}

impl ChainPolicyDefinition {
    #[must_use]
    pub const fn max_segments(self) -> u32 {
        self.max_segments
    }

    #[must_use]
    pub const fn timeout(self) -> Duration {
        self.timeout
    }
}

/// One top-level terminal that selects input and optional declared state for its successor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinueAsNewDefinition {
    node: NodeId,
    input: Expression,
    state: Option<Expression>,
}

impl ContinueAsNewDefinition {
    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }

    #[must_use]
    pub const fn input(&self) -> &Expression {
        &self.input
    }

    #[must_use]
    pub const fn state(&self) -> Option<&Expression> {
        self.state.as_ref()
    }
}

/// Canonical continuation metadata validated without making the workflow executable.
#[derive(Debug, Clone)]
pub struct ParsedContinuation {
    definition: WorkflowDefinition,
    workflow_name: Slug,
    entrypoint: NodeId,
    policy: ChainPolicyDefinition,
    continuations: BTreeMap<NodeId, ContinueAsNewDefinition>,
    state: Option<DeclaredStateDefinition>,
    canonical: Vec<u8>,
    digest: Digest,
    warnings: Vec<DefinitionWarning>,
}

impl ParsedContinuation {
    /// Typed graph retained for planner integration once the runtime gate is complete.
    #[must_use]
    pub const fn definition(&self) -> &WorkflowDefinition {
        &self.definition
    }

    #[must_use]
    pub const fn workflow_name(&self) -> &Slug {
        &self.workflow_name
    }

    #[must_use]
    pub const fn entrypoint(&self) -> &NodeId {
        &self.entrypoint
    }

    #[must_use]
    pub const fn policy(&self) -> ChainPolicyDefinition {
        self.policy
    }

    pub fn continuations(&self) -> impl Iterator<Item = &ContinueAsNewDefinition> {
        self.continuations.values()
    }

    #[must_use]
    pub fn continuation(&self, node: &NodeId) -> Option<&ContinueAsNewDefinition> {
        self.continuations.get(node)
    }

    #[must_use]
    pub const fn state(&self) -> Option<&DeclaredStateDefinition> {
        self.state.as_ref()
    }

    #[must_use]
    pub fn canonical(&self) -> &[u8] {
        &self.canonical
    }

    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    #[must_use]
    pub fn warnings(&self) -> &[DefinitionWarning] {
        &self.warnings
    }
}

fn policy(document: &Value) -> Result<ChainPolicyDefinition, IrError> {
    let policy = document
        .get("chainPolicy")
        .and_then(Value::as_object)
        .ok_or_else(|| issue("/chainPolicy", "a continuation requires a chain policy"))?;
    let max_segments = policy
        .get("maxSegments")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| (1..=10_000).contains(value))
        .ok_or_else(|| {
            issue(
                "/chainPolicy/maxSegments",
                "expected an integer in 1..=10000",
            )
        })?;
    let timeout_ms = policy
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .filter(|value| (1..=31_622_400_000).contains(value))
        .ok_or_else(|| {
            issue(
                "/chainPolicy/timeoutMs",
                "expected milliseconds in 1..=31622400000",
            )
        })?;
    Ok(ChainPolicyDefinition {
        max_segments,
        timeout: Duration::from_millis(timeout_ms),
    })
}

fn continuations(document: &Value) -> Result<BTreeMap<NodeId, ContinueAsNewDefinition>, IrError> {
    let nodes = document
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| issue("/nodes", "expected an array"))?;
    let mut parsed = BTreeMap::new();
    for (index, value) in nodes.iter().enumerate() {
        let Some(node) = value.as_object() else {
            continue;
        };
        if node.get("type").and_then(Value::as_str) != Some("continueAsNew") {
            continue;
        }
        let path = format!("/nodes/{index}");
        let raw_id = node
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| issue(format!("{path}/id"), "expected a node id"))?;
        let id = NodeId::parse(raw_id)
            .map_err(|error| issue(format!("{path}/id"), error.to_string()))?;
        let input = node
            .get("input")
            .and_then(Value::as_str)
            .ok_or_else(|| issue(format!("{path}/input"), "expected an expression"))?;
        let state = node
            .get("state")
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| issue(format!("{path}/state"), "expected an expression"))
                    .and_then(|source| {
                        compiled_expression(
                            source,
                            raw_id.to_owned(),
                            "state",
                            &["input", "state", "result", "completedResults"],
                        )
                    })
            })
            .transpose()?;
        let continuation = ContinueAsNewDefinition {
            node: id.clone(),
            input: compiled_expression(
                input,
                raw_id.to_owned(),
                "input",
                &["input", "state", "result", "completedResults"],
            )?,
            state,
        };
        if parsed.insert(id, continuation).is_some() {
            return Err(issue(format!("{path}/id"), "duplicate continuation node"));
        }
    }
    if parsed.is_empty() {
        return Err(issue(
            "/nodes",
            "the continuation parser requires a continueAsNew node",
        ));
    }
    Ok(parsed)
}

fn graph_document(document: &Value) -> Result<Value, IrError> {
    let mut graph = document.clone();
    let root = graph
        .as_object_mut()
        .ok_or_else(|| issue("", "expected an object"))?;
    root.remove("chainPolicy");
    root.remove("state");
    Ok(graph)
}

pub(crate) fn parse(document: &Value) -> Result<ParsedContinuation, IrError> {
    let policy = policy(document)?;
    let domain_policy = ContinuationPolicy::new(
        u16::try_from(policy.max_segments()).map_err(|_| {
            issue(
                "/chainPolicy/maxSegments",
                "expected an integer in 1..=10000",
            )
        })?,
        policy.timeout(),
    )
    .map_err(IrError::Invalid)?;
    let continuations = continuations(document)?;
    let state = document
        .get("state")
        .map(|_| parse_state(document))
        .transpose()?;
    let graph = graph_document(document)?;
    let definition = crate::convert::continuation_definition(&graph, domain_policy)?;
    crate::check_expressions(&definition)?;
    let canonical = canonical::canonical_json(document);
    let digest = canonical::digest_of(&canonical);
    let warnings = definition.warnings();
    let workflow_name = definition.name().clone();
    let entrypoint = definition.entrypoint().clone();
    Ok(ParsedContinuation {
        definition,
        workflow_name,
        entrypoint,
        policy,
        continuations,
        state,
        canonical,
        digest,
        warnings,
    })
}

pub(crate) fn parse_published(document: &Value) -> Result<ParsedContinuation, IrError> {
    for field in ["bodies", "triggers"] {
        if document.get(field).is_some() {
            return Err(IrError::UnsupportedDeclaration {
                field,
                reason: "this combination with continuation is not enabled by the runtime",
            });
        }
    }
    let nodes = document
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| issue("/nodes", "expected an array"))?;
    let entrypoint = document
        .get("entrypoint")
        .and_then(Value::as_str)
        .ok_or_else(|| issue("/entrypoint", "expected a node id"))?;
    let Some(entry) = nodes
        .iter()
        .find(|node| node.get("id").and_then(Value::as_str) == Some(entrypoint))
    else {
        return Err(issue("/entrypoint", "entrypoint does not name a node"));
    };
    // An activity entrypoint, or an event wait whose successor is created already waiting (G372).
    // Anything else would need the hand-off to plan more than one kind of entry.
    let entry_type = entry.get("type").and_then(Value::as_str);
    let wait_entry = entry_type == Some("waitEvent");
    if entry_type != Some("activity") && !wait_entry {
        return Err(IrError::Unsupported {
            node: entrypoint.chars().take(64).collect(),
            kind: entry_type.unwrap_or("unknown").chars().take(32).collect(),
            reason: "a published continuation starts with an activity or an event wait",
        });
    }
    // The successor begins to wait in the hand-off transaction, so the wait must be the plain one:
    // no timeout route (a timed-out wait would have to be routed by a segment that has no past), no
    // look-back (it would re-read the event that ended the predecessor's wait) and no declared state.
    if wait_entry
        && (entry.get("onTimeout").is_some()
            || entry
                .get("lookbackMs")
                .is_some_and(|value| value.as_u64() != Some(0))
            || document.get("state").is_some())
    {
        return Err(IrError::Unsupported {
            node: entrypoint.chars().take(64).collect(),
            kind: "waitEvent".to_owned(),
            reason: "a continuation that starts with an event wait supports no timeout route, no look-back and no declared state",
        });
    }
    let find_node = |id: &str| {
        nodes
            .iter()
            .find(|node| node.get("id").and_then(Value::as_str) == Some(id))
    };
    let successor = entry.get("next").and_then(Value::as_str);
    let direct_continuation = successor
        .and_then(find_node)
        .filter(|node| node.get("type").and_then(Value::as_str) == Some("continueAsNew"));
    let has_declared_state = document.get("state").is_some();
    let direct = nodes.len() == 2
        && direct_continuation.is_some_and(|node| {
            if has_declared_state {
                node.get("state").and_then(Value::as_str) == Some("state")
            } else {
                node.get("state").is_none()
            }
        });
    let waiting = successor
        .and_then(find_node)
        .filter(|node| node.get("type").and_then(Value::as_str) == Some("waitEvent"))
        .is_some_and(|wait| {
            let continuation = wait.get("next").and_then(Value::as_str).and_then(find_node);
            !has_declared_state
                && nodes.len() == 3
                && wait.get("onTimeout").is_none()
                && wait
                    .get("lookbackMs")
                    .is_none_or(|value| value.as_u64() == Some(0))
                && continuation.is_some_and(|node| {
                    node.get("type").and_then(Value::as_str) == Some("continueAsNew")
                        && node.get("state").is_none()
                })
        });
    let conditional = successor
        .and_then(find_node)
        .filter(|node| node.get("type").and_then(Value::as_str) == Some("condition"))
        .is_some_and(|condition| {
            let Some(if_true) = condition.get("ifTrue").and_then(Value::as_str) else {
                return false;
            };
            let Some(if_false) = condition.get("ifFalse").and_then(Value::as_str) else {
                return false;
            };
            let Some(true_node) = find_node(if_true) else {
                return false;
            };
            let Some(false_node) = find_node(if_false) else {
                return false;
            };
            let continuation_and_end = |continuation: &Value, end: &Value| {
                let transfers_declared_state = if has_declared_state {
                    continuation.get("state").and_then(Value::as_str) == Some("state")
                } else {
                    continuation.get("state").is_none()
                };
                continuation.get("type").and_then(Value::as_str) == Some("continueAsNew")
                    && transfers_declared_state
                    && end.get("type").and_then(Value::as_str) == Some("end")
                    && end.get("output").is_none()
            };
            nodes.len() == 4
                && (continuation_and_end(true_node, false_node)
                    || continuation_and_end(false_node, true_node))
        });
    // An approval after the entry activity: its approval rolls the chain over, its rejection and its
    // expiry end it (decision D295). The approval is settled before the boundary is reached, so the
    // hand-off never meets an open request; the original deadline and the cancel latch still settle
    // one that is open.
    let approval_gated = successor
        .and_then(find_node)
        .filter(|node| node.get("type").and_then(Value::as_str) == Some("approval"))
        .is_some_and(|gate| {
            let route = |field: &str| gate.get(field).and_then(Value::as_str).and_then(find_node);
            let transfers_declared_state = |continuation: &Value| {
                if has_declared_state {
                    continuation.get("state").and_then(Value::as_str) == Some("state")
                } else {
                    continuation.get("state").is_none()
                }
            };
            let continues = |node: &Value| {
                node.get("type").and_then(Value::as_str) == Some("continueAsNew")
                    && transfers_declared_state(node)
            };
            let ends = |node: &Value| {
                node.get("type").and_then(Value::as_str) == Some("end")
                    && node.get("output").is_none()
                    && node.get("outputExpr").is_none()
            };
            let (Some(approve), Some(reject), Some(expire)) =
                (route("onApprove"), route("onReject"), route("onExpire"))
            else {
                return false;
            };
            let ids = |routes: &[&Value]| {
                let mut ids: Vec<&str> = routes
                    .iter()
                    .filter_map(|node| node.get("id").and_then(Value::as_str))
                    .collect();
                ids.sort_unstable();
                ids.dedup();
                ids.len()
            };
            // entry, approval, the continuation and the distinct end nodes, nothing else.
            continues(approve)
                && ends(reject)
                && ends(expire)
                && gate.get("onError").is_none()
                && nodes.len() == 2 + ids(&[approve, reject, expire])
        });
    // After an event wait only the direct continuation and the single condition are published.
    let shape_allowed = if wait_entry {
        direct || conditional
    } else {
        direct || waiting || conditional || approval_gated
    };
    if entry.get("onError").is_some() || !shape_allowed {
        return Err(IrError::Unsupported {
            node: entrypoint.chars().take(64).collect(),
            kind: "continuation".to_owned(),
            reason: "the runtime supports an activity followed by a continuation that transfers input and, when declared, the exact state snapshot; one condition may select that continuation or an end; an input-only waitEvent after the entry activity may continue after its signal; an approval after the entry activity may route its approval to the continuation and its rejection and expiry to end nodes; an entry event wait supports only an input-only continuation, or one condition choosing between it and an end",
        });
    }
    parse(document)
}
