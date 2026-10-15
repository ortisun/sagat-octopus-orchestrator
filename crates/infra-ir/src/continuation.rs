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
