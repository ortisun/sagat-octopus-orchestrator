//! Pure declarative constructors. The server validates and executes the resulting common IR.
//! Builders share joined flows; build reads their current contents and returns an owned document.
use crate::SagatError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
mod primitives;
pub use primitives::*;
mod state;
pub use state::*;
mod database;
pub use database::*;

fn invalid(message: &str) -> SagatError {
    SagatError::new("VALIDATION_FAILED", message)
}

/// RFC 8785 text, with the shared SDK safe whole-number policy. REST payloads do not use this.
pub fn canonical_json(value: &Value) -> Result<String, SagatError> {
    fn write(value: &Value, out: &mut String) -> Result<(), SagatError> {
        match value {
            Value::Null => out.push_str("null"),
            Value::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
            Value::String(v) => {
                out.push_str(&serde_json::to_string(v).map_err(|_| invalid("invalid JSON string"))?)
            }
            Value::Number(v) => {
                let number = v
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| invalid("a number is outside the range of a double"))?;
                if number.abs() >= 9_007_199_254_740_992.0
                    && number.abs() < 1e21
                    && number.fract() == 0.0
                {
                    return Err(invalid(
                        "a whole number beyond 2^53 - 1 cannot be written in canonical text; send it as a string",
                    ));
                }
                if number == 0.0 {
                    out.push('0');
                } else {
                    out.push_str(ryu_js::Buffer::new().format_finite(number));
                }
            }
            Value::Array(values) => {
                out.push('[');
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(value, out)?;
                }
                out.push(']');
            }
            Value::Object(values) => {
                let mut keys = values.keys().collect::<Vec<_>>();
                keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
                out.push('{');
                for (i, key) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(&Value::String((*key).clone()), out)?;
                    out.push(':');
                    write(&values[*key], out)?;
                }
                out.push('}');
            }
        }
        Ok(())
    }
    let mut out = String::new();
    write(value, &mut out)?;
    Ok(out)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RetryStrategy {
    Fixed,
    Exponential,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_delay_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_delay_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<RetryStrategy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter: Option<bool>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compensate_with: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconcile_with: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compensation_retry: Option<RetryOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compensation_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub irreversible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high_risk: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Join {
    All,
    Any,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompensationFailure {
    Fail,
    ManualReview,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SagaCancelPolicy {
    CompensateCompleted,
    AbortNoCompensation,
    WaitInFlightThenCompensate,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SagaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_compensation_failure: Option<CompensationFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_policy: Option<SagaCancelPolicy>,
}

#[derive(Clone, Default)]
struct Data {
    name: Option<String>,
    metadata: BTreeMap<String, Value>,
    fields: BTreeMap<String, Value>,
    nodes: Vec<Value>,
    embedded: Vec<Builder>,
    bodies: BTreeMap<String, Builder>,
    error: Option<SagatError>,
}
/// A workflow or nested flow. Methods retain the first local error; build returns it.
#[derive(Clone, Default)]
pub struct Builder(Arc<Mutex<Data>>);
impl std::fmt::Debug for Builder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Builder").finish_non_exhaustive()
    }
}
impl Builder {
    pub fn new(name: &str) -> Self {
        let builder = Self::flow();
        builder.change(|d| d.name = Some(name.into()));
        if !(3..=64).contains(&name.len())
            || !name.as_bytes()[0].is_ascii_lowercase()
            || !name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            builder.fail("a workflow name is lower case letters, digits and dashes, 3 to 64");
        }
        builder
    }
    pub fn flow() -> Self {
        Self::default()
    }
    fn root_only(&self) -> bool {
        if self.snapshot().is_ok_and(|data| data.name.is_some()) {
            true
        } else {
            self.fail("this option is valid only in the top-level workflow");
            false
        }
    }
    fn change(&self, f: impl FnOnce(&mut Data)) {
        if let Ok(mut data) = self.0.lock() {
            f(&mut data);
        }
    }
    fn snapshot(&self) -> Result<Data, SagatError> {
        self.0
            .lock()
            .map(|d| d.clone())
            .map_err(|_| invalid("builder state is unavailable"))
    }
    fn fail(&self, message: &str) -> &Self {
        self.change(|d| {
            if d.error.is_none() {
                d.error = Some(invalid(message));
            }
        });
        self
    }
    fn push(&self, node: Value) -> &Self {
        self.change(|d| {
            if d.error.is_some() {
                return;
            }
            if node["id"].as_str().is_none_or(str::is_empty) {
                d.error = Some(invalid("a node needs an id"));
            } else if d.nodes.iter().any(|n| n["id"] == node["id"]) {
                d.error = Some(invalid("the node id is used twice"));
            } else {
                d.nodes.push(node);
            }
        });
        self
    }
    fn options<T: Serialize>(&self, mut node: Value, options: T) -> &Self {
        match serde_json::to_value(options) {
            Ok(Value::Object(fields)) => {
                if let Some(map) = node.as_object_mut() {
                    map.extend(fields);
                }
                self.push(node)
            }
            _ => self.fail("invalid node options"),
        }
    }
    pub fn step(&self, id: &str, activity: &str, options: StepOptions) -> &Self {
        if activity.is_empty() {
            return self.fail("a step needs an activity name");
        }
        if options.irreversible == Some(true) && options.compensate_with.is_some() {
            return self.fail("an irreversible step cannot compensateWith");
        }
        if options
            .secrets
            .as_ref()
            .is_some_and(|names| names.iter().any(|n| !valid_secret(n)))
        {
            return self.fail("a step names a secret that is not a name");
        }
        self.options(
            json!({"id":id,"type":"activity","activity":activity}),
            options,
        )
    }
    pub fn sleep(&self, id: &str, duration_ms: u64) -> &Self {
        if duration_ms < 1 {
            return self.fail("sleep needs a duration of at least 1 ms");
        }
        self.push(json!({"id":id,"type":"timer","durationMs":duration_ms}))
    }
    pub fn end(&self, id: &str, output_expr: Option<&str>) -> &Self {
        let mut node = json!({"id":id,"type":"end"});
        if let Some(value) = output_expr {
            node["outputExpr"] = value.into();
        }
        self.push(node)
    }
    pub fn condition(&self, id: &str, expression: &str, if_true: &str, if_false: &str) -> &Self {
        if expression.is_empty() {
            return self.fail("condition needs an expression");
        }
        self.push(json!({"id":id,"type":"condition","expression":expression,"ifTrue":if_true,"ifFalse":if_false}))
    }
    fn reaches(&self, target: &Builder, seen: &mut BTreeSet<usize>) -> bool {
        if Arc::ptr_eq(&self.0, &target.0) {
            return true;
        }
        if !seen.insert(Arc::as_ptr(&self.0) as usize) {
            return false;
        }
        self.snapshot().is_ok_and(|d| {
            d.embedded
                .iter()
                .chain(d.bodies.values())
                .any(|child| child.reaches(target, seen))
        })
    }
    pub fn parallel(&self, id: &str, join: Join, branches: &[Builder]) -> &Self {
        if branches.len() < 2 {
            return self.fail("parallel needs at least two branches");
        }
        let mut heads = Vec::new();
        for branch in branches {
            if let Ok(data) = branch.snapshot()
                && let Some(error) = data.error
            {
                return self.fail(&error.message);
            }
            if branch.reaches(self, &mut BTreeSet::new()) {
                return self.fail("joined flow cycle is not allowed");
            }
            match branch.snapshot() {
                Ok(d) if !d.nodes.is_empty() => heads.push(d.nodes[0]["id"].clone()),
                _ => return self.fail("parallel has an empty branch"),
            }
        }
        self.push(json!({"id":id,"type":"parallel","join":join,"branches":heads}));
        self.change(|d| {
            if d.error.is_none() {
                d.embedded.extend_from_slice(branches);
            }
        });
        self
    }
    pub fn saga(&self, id: &str, body: &Builder, options: SagaOptions) -> &Self {
        if let Ok(data) = body.snapshot()
            && let Some(error) = data.error
        {
            return self.fail(&error.message);
        }
        if body.reaches(self, &mut BTreeSet::new()) {
            return self.fail("joined flow cycle is not allowed");
        }
        let data = match body.snapshot() {
            Ok(d) if !d.nodes.is_empty() => d,
            _ => return self.fail("saga has an empty body"),
        };
        self.options(
            json!({"id":id,"type":"saga","begin":data.nodes[0]["id"]}),
            options,
        );
        self.change(|d| {
            if d.error.is_none() {
                d.embedded.push(body.clone());
            }
        });
        self
    }
    fn flatten(
        &self,
        inside_saga: bool,
        stack: &mut BTreeSet<usize>,
    ) -> Result<Vec<Value>, SagatError> {
        let identity = Arc::as_ptr(&self.0) as usize;
        if !stack.insert(identity) {
            return Err(invalid("joined flow cycle is not allowed"));
        }
        let data = self.snapshot()?;
        if let Some(error) = data.error {
            return Err(error);
        }
        let mut out = data.nodes.clone();
        for (i, node) in out.iter_mut().enumerate() {
            let kind = node["type"].as_str().unwrap_or_default();
            if inside_saga && kind == "end" {
                return Err(invalid("saga cannot hold an end node"));
            }
            if i + 1 < data.nodes.len()
                && !matches!(
                    kind,
                    "condition" | "switch" | "approval" | "continueAsNew" | "end" | "return"
                )
                && node.get("next").is_none()
            {
                node["next"] = data.nodes[i + 1]["id"].clone();
            }
        }
        for child in &data.embedded {
            let child_data = child.snapshot()?;
            let saga = inside_saga
                || data.nodes.iter().any(|n| {
                    n["type"] == "saga"
                        && child_data
                            .nodes
                            .first()
                            .is_some_and(|head| head["id"] == n["begin"])
                });
            out.extend(child.flatten(saga, stack)?);
        }
        stack.remove(&identity);
        Ok(out)
    }
    pub fn build(&self) -> Result<Value, SagatError> {
        let data = self.snapshot()?;
        if let Some(error) = data.error {
            return Err(error);
        }
        if data.nodes.is_empty() {
            return Err(invalid("a workflow needs at least one node"));
        }
        let nodes = self.flatten(false, &mut BTreeSet::new())?;
        let name = data
            .name
            .ok_or_else(|| invalid("a nested flow cannot be built without a workflow name"))?;
        validate_scope(&nodes)?;
        if nodes.iter().any(|node| node["type"] == "return") {
            return Err(invalid("return is only valid in a named body"));
        }
        if nodes.iter().any(|node| node["type"] == "continueAsNew") {
            if !data.fields.contains_key("chainPolicy") {
                return Err(invalid("a workflow with continueAsNew needs chainPolicy"));
            }
            if nodes
                .iter()
                .skip(data.nodes.len())
                .any(|node| node["type"] == "continueAsNew")
            {
                return Err(invalid(
                    "continueAsNew is valid only in the top-level workflow",
                ));
            }
        }
        let mut bodies = BTreeMap::new();
        for (name, body) in &data.bodies {
            let scope = body.flatten(false, &mut BTreeSet::new())?;
            if scope.is_empty() {
                return Err(invalid("a named body is empty"));
            }
            if scope.iter().any(|node| node["type"] == "end") {
                return Err(invalid("a body cannot hold an end node"));
            }
            if scope.iter().any(|node| node["type"] == "continueAsNew") {
                return Err(invalid(
                    "continueAsNew is valid only in the top-level workflow",
                ));
            }
            if scope.iter().filter(|node| node["type"] == "return").count() != 1 {
                return Err(invalid("a body needs exactly one return node"));
            }
            validate_scope(&scope)?;
            bodies.insert(name.clone(), scope);
        }
        validate_body_references(&nodes, &bodies)?;
        for scope in bodies.values() {
            validate_body_references(scope, &bodies)?;
        }
        for name in bodies.keys() {
            visit_body(name, &bodies, &mut BTreeSet::new(), &mut BTreeSet::new())?;
        }
        let mut metadata = data.metadata;
        metadata.insert("name".into(), name.into());
        let mut doc = json!({"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow","metadata":metadata,"entrypoint":data.nodes[0]["id"],"nodes":nodes});
        if let Some(map) = doc.as_object_mut() {
            map.extend(data.fields);
        }
        if !bodies.is_empty() {
            doc["bodies"] = json!(
                bodies
                    .into_iter()
                    .map(|(name, nodes)| {
                        let body = json!({"entrypoint":nodes[0]["id"],"nodes":nodes});
                        (name, body)
                    })
                    .collect::<BTreeMap<_, _>>()
            );
        }
        Ok(doc)
    }
    pub fn json(&self) -> Result<String, SagatError> {
        canonical_json(&self.build()?)
    }
}
fn validate_scope(nodes: &[Value]) -> Result<(), SagatError> {
    let mut ids = BTreeSet::new();
    for node in nodes {
        if !ids.insert(node["id"].as_str().unwrap_or_default()) {
            return Err(invalid("the node id is used twice"));
        }
    }
    for node in nodes {
        for field in [
            "next",
            "onError",
            "onTimeout",
            "ifTrue",
            "ifFalse",
            "begin",
            "onApprove",
            "onReject",
            "onExpire",
            "default",
        ] {
            if node
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|target| !ids.contains(target))
            {
                return Err(invalid(
                    "a reference points at a target which is not a node",
                ));
            }
        }
        for target in node
            .get("branches")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .chain(
                node.get("cases")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|case| case.get("target")),
            )
        {
            if target.as_str().is_none_or(|target| !ids.contains(target)) {
                return Err(invalid(
                    "a reference points at a target which is not a node",
                ));
            }
        }
    }
    Ok(())
}
fn valid_secret(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.as_bytes()[0].is_ascii_lowercase()
        && name.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-')
        })
}

fn validate_body_references(
    nodes: &[Value],
    bodies: &BTreeMap<String, Vec<Value>>,
) -> Result<(), SagatError> {
    for node in nodes.iter().filter(|n| {
        matches!(
            n["type"].as_str(),
            Some("forEach" | "while" | "repeatUntil")
        )
    }) {
        if !node["body"]
            .as_str()
            .is_some_and(|name| bodies.contains_key(name))
        {
            return Err(invalid(
                "a reference points at a target which is not a body",
            ));
        }
        if node.get("next").is_none() {
            return Err(invalid("a collection or loop needs a next node"));
        }
    }
    Ok(())
}
fn visit_body(
    name: &str,
    bodies: &BTreeMap<String, Vec<Value>>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> Result<(), SagatError> {
    if visited.contains(name) {
        return Ok(());
    }
    if !visiting.insert(name.into()) {
        return Err(invalid("body reference cycle is not allowed"));
    }
    for node in &bodies[name] {
        if matches!(
            node["type"].as_str(),
            Some("forEach" | "while" | "repeatUntil")
        ) {
            visit_body(
                node["body"].as_str().unwrap_or_default(),
                bodies,
                visiting,
                visited,
            )?;
        }
    }
    visiting.remove(name);
    visited.insert(name.into());
    Ok(())
}
