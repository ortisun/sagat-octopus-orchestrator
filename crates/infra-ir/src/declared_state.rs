//! Typed additive-v1 declarations for workflow business state.

use std::collections::BTreeMap;

use jsonschema::Validator;
use sagat_application::definitions::{IrError, IrIssue, ParsedDefinition};
use sagat_domain::execution::ResultDisclosure;
use sagat_domain::workflow::{ActivityName, Digest, Expression};
use sagat_infra_expr::Compiled;
use serde_json::{Map, Value};

use crate::canonical;

pub const MAX_DECLARED_SCHEMA_BYTES: usize = 64 * 1024;
pub const MAX_DECLARED_SCHEMA_DEPTH: usize = 16;
const MAX_DECLARED_OPERATIONS: usize = 32;

/// A compiled, canonical schema pinned by the workflow version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredSchema {
    canonical: Vec<u8>,
    digest: Digest,
}

impl DeclaredSchema {
    #[must_use]
    pub fn canonical(&self) -> &[u8] {
        &self.canonical
    }

    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }
}

/// One immutable Query declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredQueryDefinition {
    name: ActivityName,
    args_schema: DeclaredSchema,
    result_schema: DeclaredSchema,
    expression: Expression,
    disclosure: ResultDisclosure,
}

impl DeclaredQueryDefinition {
    #[must_use]
    pub const fn name(&self) -> &ActivityName {
        &self.name
    }

    #[must_use]
    pub const fn args_schema(&self) -> &DeclaredSchema {
        &self.args_schema
    }

    #[must_use]
    pub const fn result_schema(&self) -> &DeclaredSchema {
        &self.result_schema
    }

    #[must_use]
    pub const fn expression(&self) -> &Expression {
        &self.expression
    }

    #[must_use]
    pub const fn disclosure(&self) -> ResultDisclosure {
        self.disclosure
    }
}

/// One immutable Update declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredUpdateDefinition {
    name: ActivityName,
    args_schema: DeclaredSchema,
    validator: Option<Expression>,
    state: Expression,
    result: Expression,
    result_schema: DeclaredSchema,
    disclosure: ResultDisclosure,
}

impl DeclaredUpdateDefinition {
    #[must_use]
    pub const fn name(&self) -> &ActivityName {
        &self.name
    }

    #[must_use]
    pub const fn args_schema(&self) -> &DeclaredSchema {
        &self.args_schema
    }

    #[must_use]
    pub const fn validator(&self) -> Option<&Expression> {
        self.validator.as_ref()
    }

    #[must_use]
    pub const fn state(&self) -> &Expression {
        &self.state
    }

    #[must_use]
    pub const fn result(&self) -> &Expression {
        &self.result
    }

    #[must_use]
    pub const fn result_schema(&self) -> &DeclaredSchema {
        &self.result_schema
    }

    #[must_use]
    pub const fn disclosure(&self) -> ResultDisclosure {
        self.disclosure
    }
}

/// Schema-checked state initialization and its immutable operation declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredStateDefinition {
    schema: DeclaredSchema,
    initial: Expression,
    queries: BTreeMap<ActivityName, DeclaredQueryDefinition>,
    updates: BTreeMap<ActivityName, DeclaredUpdateDefinition>,
}

impl DeclaredStateDefinition {
    #[must_use]
    pub const fn schema(&self) -> &DeclaredSchema {
        &self.schema
    }

    #[must_use]
    pub const fn initial(&self) -> &Expression {
        &self.initial
    }

    pub fn queries(&self) -> impl Iterator<Item = &DeclaredQueryDefinition> {
        self.queries.values()
    }

    pub fn updates(&self) -> impl Iterator<Item = &DeclaredUpdateDefinition> {
        self.updates.values()
    }

    #[must_use]
    pub fn query(&self, name: &ActivityName) -> Option<&DeclaredQueryDefinition> {
        self.queries.get(name)
    }

    #[must_use]
    pub fn update(&self, name: &ActivityName) -> Option<&DeclaredUpdateDefinition> {
        self.updates.get(name)
    }
}

/// A fully canonicalized structured definition paired with its preserved state declaration.
#[derive(Debug, Clone)]
pub struct ParsedDeclaredState {
    parsed: ParsedDefinition,
    state: DeclaredStateDefinition,
}

impl ParsedDeclaredState {
    pub(crate) const fn new(parsed: ParsedDefinition, state: DeclaredStateDefinition) -> Self {
        Self { parsed, state }
    }

    #[must_use]
    pub const fn parsed(&self) -> &ParsedDefinition {
        &self.parsed
    }

    #[must_use]
    pub const fn state(&self) -> &DeclaredStateDefinition {
        &self.state
    }

    #[must_use]
    pub fn into_parts(self) -> (ParsedDefinition, DeclaredStateDefinition) {
        (self.parsed, self.state)
    }
}

pub(crate) fn issue(path: impl Into<String>, message: impl Into<String>) -> IrError {
    IrError::Schema(vec![IrIssue {
        path: path.into(),
        message: message.into(),
    }])
}

fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, IrError> {
    value
        .as_object()
        .ok_or_else(|| issue(path, "expected an object"))
}

fn text<'a>(object: &'a Map<String, Value>, field: &str, path: &str) -> Result<&'a str, IrError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| issue(format!("{path}/{field}"), "expected a string"))
}

fn document_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(document_depth).max().unwrap_or(0),
        Value::Object(values) => 1 + values.values().map(document_depth).max().unwrap_or(0),
        _ => 1,
    }
}

fn contains_reference(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(contains_reference),
        Value::Object(values) => {
            values
                .keys()
                .any(|key| key == "$ref" || key == "$dynamicRef")
                || values.values().any(contains_reference)
        }
        _ => false,
    }
}

fn coherent_bounds(value: &Value, path: &str) -> Result<(), IrError> {
    match value {
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                coherent_bounds(value, &format!("{path}/{index}"))?;
            }
        }
        Value::Object(values) => {
            for (lower, upper) in [
                ("minItems", "maxItems"),
                ("minLength", "maxLength"),
                ("minimum", "maximum"),
                ("minProperties", "maxProperties"),
            ] {
                if let (Some(lower), Some(upper)) = (
                    values.get(lower).and_then(Value::as_f64),
                    values.get(upper).and_then(Value::as_f64),
                ) && lower > upper
                {
                    return Err(issue(path, "schema lower bound exceeds its upper bound"));
                }
            }
            for (key, value) in values {
                coherent_bounds(value, &format!("{path}/{key}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn schema(value: &Value, path: &str) -> Result<DeclaredSchema, IrError> {
    let canonical = canonical::canonical_json(value);
    if canonical.len() > MAX_DECLARED_SCHEMA_BYTES {
        return Err(issue(path, "a declared schema is larger than 64 KiB"));
    }
    if document_depth(value) > MAX_DECLARED_SCHEMA_DEPTH {
        return Err(issue(path, "a declared schema exceeds nesting depth 16"));
    }
    if contains_reference(value) {
        return Err(issue(path, "declared schemas cannot contain references"));
    }
    coherent_bounds(value, path)?;
    Validator::options()
        .should_validate_formats(true)
        .build(value)
        .map_err(|_| issue(path, "declared schema does not compile"))?;
    Ok(DeclaredSchema {
        digest: canonical::digest_of(&canonical),
        canonical,
    })
}

fn operation_name(value: &str, path: &str) -> Result<ActivityName, IrError> {
    ActivityName::parse(value).map_err(|_| issue(path, "invalid declared operation name"))
}

pub(crate) fn compiled_expression(
    source: &str,
    node: String,
    field: &'static str,
    allowed: &[&str],
) -> Result<Expression, IrError> {
    let expression = Expression::parse(source).map_err(|error| issue(&node, error.to_string()))?;
    let compiled = Compiled::compile(expression.as_str()).map_err(|error| IrError::Expression {
        node: node.clone(),
        field,
        message: error.to_string(),
    })?;
    if let Some(variable) = compiled
        .variables()
        .into_iter()
        .find(|variable| !allowed.contains(&variable.as_str()))
    {
        return Err(IrError::Expression {
            node,
            field,
            message: format!("variable `{variable}` is unavailable in this operation"),
        });
    }
    Ok(expression)
}

fn disclosure(object: &Map<String, Value>, path: &str) -> Result<ResultDisclosure, IrError> {
    ResultDisclosure::parse(text(object, "disclosure", path)?).map_err(|_| {
        issue(
            format!("{path}/disclosure"),
            "unsupported disclosure policy",
        )
    })
}

fn queries(
    value: Option<&Value>,
) -> Result<BTreeMap<ActivityName, DeclaredQueryDefinition>, IrError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let entries = object(value, "/state/queries")?;
    if entries.len() > MAX_DECLARED_OPERATIONS {
        return Err(issue(
            "/state/queries",
            "at most 32 queries may be declared",
        ));
    }
    let mut parsed = BTreeMap::new();
    for (raw_name, value) in entries {
        let path = format!("/state/queries/{raw_name}");
        let operation = object(value, &path)?;
        let name = operation_name(raw_name, &path)?;
        let definition = DeclaredQueryDefinition {
            name: name.clone(),
            args_schema: schema(
                operation
                    .get("argsSchema")
                    .ok_or_else(|| issue(format!("{path}/argsSchema"), "missing schema"))?,
                &format!("{path}/argsSchema"),
            )?,
            result_schema: schema(
                operation
                    .get("resultSchema")
                    .ok_or_else(|| issue(format!("{path}/resultSchema"), "missing schema"))?,
                &format!("{path}/resultSchema"),
            )?,
            expression: compiled_expression(
                text(operation, "expression", &path)?,
                path.clone(),
                "expression",
                &["input", "state", "completedResults", "args"],
            )?,
            disclosure: disclosure(operation, &path)?,
        };
        parsed.insert(name, definition);
    }
    Ok(parsed)
}

fn updates(
    value: Option<&Value>,
) -> Result<BTreeMap<ActivityName, DeclaredUpdateDefinition>, IrError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let entries = object(value, "/state/updates")?;
    if entries.len() > MAX_DECLARED_OPERATIONS {
        return Err(issue(
            "/state/updates",
            "at most 32 updates may be declared",
        ));
    }
    let mut parsed = BTreeMap::new();
    for (raw_name, value) in entries {
        let path = format!("/state/updates/{raw_name}");
        let operation = object(value, &path)?;
        let name = operation_name(raw_name, &path)?;
        let validator = operation
            .get("validate")
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| issue(format!("{path}/validate"), "expected a string"))
                    .and_then(|source| {
                        compiled_expression(
                            source,
                            path.clone(),
                            "validate",
                            &["input", "state", "completedResults", "args"],
                        )
                    })
            })
            .transpose()?;
        let definition = DeclaredUpdateDefinition {
            name: name.clone(),
            args_schema: schema(
                operation
                    .get("argsSchema")
                    .ok_or_else(|| issue(format!("{path}/argsSchema"), "missing schema"))?,
                &format!("{path}/argsSchema"),
            )?,
            validator,
            state: compiled_expression(
                text(operation, "state", &path)?,
                path.clone(),
                "state",
                &["input", "state", "completedResults", "args"],
            )?,
            result: compiled_expression(
                text(operation, "result", &path)?,
                path.clone(),
                "result",
                &["input", "state", "completedResults", "args", "newState"],
            )?,
            result_schema: schema(
                operation
                    .get("resultSchema")
                    .ok_or_else(|| issue(format!("{path}/resultSchema"), "missing schema"))?,
                &format!("{path}/resultSchema"),
            )?,
            disclosure: disclosure(operation, &path)?,
        };
        parsed.insert(name, definition);
    }
    Ok(parsed)
}

pub(crate) fn parse(document: &Value) -> Result<DeclaredStateDefinition, IrError> {
    let value = document
        .get("state")
        .ok_or_else(|| issue("/state", "a declared state is required"))?;
    let state = object(value, "/state")?;
    Ok(DeclaredStateDefinition {
        schema: schema(
            state
                .get("schema")
                .ok_or_else(|| issue("/state/schema", "missing schema"))?,
            "/state/schema",
        )?,
        initial: compiled_expression(
            text(state, "initial", "/state")?,
            "state".to_owned(),
            "initial",
            &["input"],
        )?,
        queries: queries(state.get("queries"))?,
        updates: updates(state.get("updates"))?,
    })
}
