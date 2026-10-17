//! Schema-pinned evaluation for declared business operations.

use std::fmt;

use jsonschema::Validator;
use sagat_application::declared_operations::{
    DeclaredOperationEvaluator, DeclaredOperationSnapshot, DeclaredUpdateDecision,
    EvaluatedDeclaredQuery, EvaluatedOperationWrite,
};
use sagat_application::operation_evaluation::{
    DeclaredOperationArguments, OperationDataField, OperationEvaluationContext,
    OperationEvaluationError, RetainedOperationValue,
};
use sagat_domain::execution::{
    MAX_OPERATION_VALUE_BYTES, OperationValue, QueryResponse, QuerySnapshot, UpdateEvaluation,
    UpdateRejection,
};
use sagat_domain::workflow::{ActivityName, Digest, Expression};
use sagat_infra_expr::{Compiled, Env};
use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

use crate::declared_state::{DeclaredSchema, DeclaredStateDefinition};
use crate::{JsonIrCodec, ParsedDeclaredState};

/// Capability-free evaluator for the bounded expression subset.
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonDeclaredOperationEvaluator;

/// Canonical JSON bytes and their content address after schema validation.
#[derive(Clone, PartialEq, Eq)]
pub struct EvaluatedOperationValue {
    value: OperationValue,
    content: Vec<u8>,
}

impl EvaluatedOperationValue {
    fn from_schema_checked(
        content: Vec<u8>,
        schema_digest: Digest,
    ) -> Result<Self, OperationEvaluationError> {
        let size_bytes = u32::try_from(content.len())
            .ok()
            .filter(|size| *size <= MAX_OPERATION_VALUE_BYTES)
            .ok_or(OperationEvaluationError::ValueTooLarge)?;
        let digest: [u8; 32] = Sha256::digest(&content).into();
        let value = OperationValue::new(Digest::new(digest), schema_digest, size_bytes)
            .map_err(|_| OperationEvaluationError::ValueTooLarge)?;
        Ok(Self { value, content })
    }

    #[must_use]
    pub const fn value(&self) -> &OperationValue {
        &self.value
    }

    #[must_use]
    pub fn content(&self) -> &[u8] {
        &self.content
    }
}

impl fmt::Debug for EvaluatedOperationValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvaluatedOperationValue")
            .field("size_bytes", &self.content.len())
            .finish_non_exhaustive()
    }
}

/// A schema-checked Query result bound to the state and journal version used to compute it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryEvaluation {
    source_state: OperationValue,
    observed_run_version: u64,
    result: EvaluatedOperationValue,
}

impl QueryEvaluation {
    const fn new(
        source_state: OperationValue,
        observed_run_version: u64,
        result: EvaluatedOperationValue,
    ) -> Self {
        Self {
            source_state,
            observed_run_version,
            result,
        }
    }

    #[must_use]
    pub const fn result(&self) -> &EvaluatedOperationValue {
        &self.result
    }

    /// Associates the result with the run and revisions observed by the audited snapshot read.
    ///
    /// # Errors
    /// Returns [`OperationEvaluationError::StateChanged`] unless both state and run version match.
    pub fn response(
        &self,
        snapshot: QuerySnapshot,
    ) -> Result<QueryResponse, OperationEvaluationError> {
        if snapshot.state() != &self.source_state
            || snapshot.run_version() != self.observed_run_version
        {
            return Err(OperationEvaluationError::StateChanged);
        }
        Ok(QueryResponse::new(snapshot, self.result.value.clone()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UpdateOutcome {
    Applied {
        new_state: EvaluatedOperationValue,
        result: EvaluatedOperationValue,
    },
    Rejected,
}

/// A schema-checked Update decision bound to the exact pre-state used by every expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateEvaluationPlan {
    source_state: OperationValue,
    observed_run_version: u64,
    outcome: UpdateOutcome,
}

impl UpdateEvaluationPlan {
    const fn applied(
        source_state: OperationValue,
        observed_run_version: u64,
        new_state: EvaluatedOperationValue,
        result: EvaluatedOperationValue,
    ) -> Self {
        Self {
            source_state,
            observed_run_version,
            outcome: UpdateOutcome::Applied { new_state, result },
        }
    }

    const fn rejected(source_state: OperationValue, observed_run_version: u64) -> Self {
        Self {
            source_state,
            observed_run_version,
            outcome: UpdateOutcome::Rejected,
        }
    }

    #[must_use]
    pub const fn new_state(&self) -> Option<&EvaluatedOperationValue> {
        match &self.outcome {
            UpdateOutcome::Applied { new_state, .. } => Some(new_state),
            UpdateOutcome::Rejected => None,
        }
    }

    #[must_use]
    pub const fn result(&self) -> Option<&EvaluatedOperationValue> {
        match &self.outcome {
            UpdateOutcome::Applied { result, .. } => Some(result),
            UpdateOutcome::Rejected => None,
        }
    }

    /// Converts the decision only if the store locked the exact state and journal version used to
    /// evaluate it.
    ///
    /// # Errors
    /// Returns [`OperationEvaluationError::StateChanged`] after concurrent run progress.
    pub fn domain_evaluation(
        &self,
        current_state: &OperationValue,
        current_run_version: u64,
    ) -> Result<UpdateEvaluation, OperationEvaluationError> {
        if current_state != &self.source_state || current_run_version != self.observed_run_version {
            return Err(OperationEvaluationError::StateChanged);
        }
        Ok(match &self.outcome {
            UpdateOutcome::Applied { new_state, result } => UpdateEvaluation::Applied {
                new_state: new_state.value.clone(),
                result: result.value.clone(),
            },
            UpdateOutcome::Rejected => {
                UpdateEvaluation::Rejected(UpdateRejection::ValidatorRejected)
            }
        })
    }

    /// Converts checked bytes into a store decision while preserving the snapshot binding.
    ///
    /// # Errors
    /// Returns [`OperationEvaluationError::StateChanged`] for a different locked snapshot.
    pub fn store_decision(
        &self,
        snapshot: &DeclaredOperationSnapshot,
    ) -> Result<DeclaredUpdateDecision, OperationEvaluationError> {
        self.domain_evaluation(snapshot.state(), snapshot.run_version())?;
        match &self.outcome {
            UpdateOutcome::Applied { new_state, result } => {
                let new_state = EvaluatedOperationWrite::restore(
                    new_state.value.clone(),
                    new_state.content.clone(),
                )
                .map_err(|_| OperationEvaluationError::RetainedValueMismatch)?;
                let result =
                    EvaluatedOperationWrite::restore(result.value.clone(), result.content.clone())
                        .map_err(|_| OperationEvaluationError::RetainedValueMismatch)?;
                Ok(DeclaredUpdateDecision::Applied { new_state, result })
            }
            UpdateOutcome::Rejected => Ok(DeclaredUpdateDecision::Rejected(
                UpdateRejection::ValidatorRejected,
            )),
        }
    }
}

struct ParsedContext {
    workflow_input: Value,
    state: Value,
    completed_results: Value,
    arguments: Value,
}

fn json(bytes: &[u8], field: OperationDataField) -> Result<Value, OperationEvaluationError> {
    serde_json::from_slice(bytes).map_err(|_| OperationEvaluationError::MalformedJson { field })
}

fn validator(
    schema: &DeclaredSchema,
    field: OperationDataField,
) -> Result<Validator, OperationEvaluationError> {
    let value = serde_json::from_slice(schema.canonical())
        .map_err(|_| OperationEvaluationError::InvalidSchema { field })?;
    Validator::options()
        .should_validate_formats(true)
        .build(&value)
        .map_err(|_| OperationEvaluationError::InvalidSchema { field })
}

fn validate(
    schema: &DeclaredSchema,
    value: &Value,
    field: OperationDataField,
) -> Result<(), OperationEvaluationError> {
    if validator(schema, field)?.is_valid(value) {
        Ok(())
    } else {
        Err(OperationEvaluationError::SchemaMismatch { field })
    }
}

fn parsed_context(
    state: &DeclaredStateDefinition,
    context: &OperationEvaluationContext<'_>,
    args_schema: &DeclaredSchema,
) -> Result<ParsedContext, OperationEvaluationError> {
    if context.state().value().schema_digest() != state.schema().digest() {
        return Err(OperationEvaluationError::SchemaMismatch {
            field: OperationDataField::State,
        });
    }
    let parsed = ParsedContext {
        workflow_input: json(context.workflow_input(), OperationDataField::WorkflowInput)?,
        state: json(context.state().content(), OperationDataField::State)?,
        completed_results: json(
            context.completed_results(),
            OperationDataField::CompletedResults,
        )?,
        arguments: json(context.arguments(), OperationDataField::Arguments)?,
    };
    validate(state.schema(), &parsed.state, OperationDataField::State)?;
    validate(
        args_schema,
        &parsed.arguments,
        OperationDataField::Arguments,
    )?;
    Ok(parsed)
}

fn environment(context: &ParsedContext, new_state: Option<&Value>) -> Env {
    let mut env = Env::new()
        .with("input", context.workflow_input.clone())
        .with("state", context.state.clone())
        .with("completedResults", context.completed_results.clone())
        .with("args", context.arguments.clone());
    if let Some(new_state) = new_state {
        env = env.with("newState", new_state.clone());
    }
    env
}

fn compiled(expression: &Expression) -> Result<Compiled, OperationEvaluationError> {
    Compiled::compile(expression.as_str()).map_err(|_| OperationEvaluationError::ExpressionFailed)
}

fn value(expression: &Expression, environment: &Env) -> Result<Value, OperationEvaluationError> {
    compiled(expression)?
        .eval(environment)
        .map_err(|_| OperationEvaluationError::ExpressionFailed)
}

fn boolean(expression: &Expression, environment: &Env) -> Result<bool, OperationEvaluationError> {
    compiled(expression)?
        .eval_bool(environment)
        .map_err(|_| OperationEvaluationError::ExpressionFailed)
}

fn evaluated_value(
    value: &Value,
    schema: &DeclaredSchema,
    field: OperationDataField,
) -> Result<EvaluatedOperationValue, OperationEvaluationError> {
    validate(schema, value, field)?;
    EvaluatedOperationValue::from_schema_checked(
        crate::canonical::canonical_json(value),
        schema.digest(),
    )
}

fn completed_results(
    snapshot: &DeclaredOperationSnapshot,
) -> Result<Vec<u8>, OperationEvaluationError> {
    let mut results = Map::new();
    for completed in snapshot.completed_results() {
        let value = json(
            completed.payload().content(),
            OperationDataField::CompletedResults,
        )?;
        results.insert(completed.node().as_str().to_owned(), value);
    }
    Ok(crate::canonical::canonical_json(&Value::Object(results)))
}

impl JsonDeclaredOperationEvaluator {
    /// Evaluates and schema-checks the state created for a new run.
    ///
    /// The expression receives only the immutable run input. The returned bytes are canonical and
    /// content-addressed before they can cross into the run creation transaction.
    ///
    /// # Errors
    /// Returns a value-free JSON, expression, schema or size error.
    pub fn initial_state(
        &self,
        state: &DeclaredStateDefinition,
        workflow_input: &[u8],
    ) -> Result<EvaluatedOperationWrite, OperationEvaluationError> {
        let workflow_input = json(workflow_input, OperationDataField::WorkflowInput)?;
        let evaluated = value(state.initial(), &Env::new().with("input", workflow_input))?;
        let checked = evaluated_value(&evaluated, state.schema(), OperationDataField::State)?;
        EvaluatedOperationWrite::restore(checked.value, checked.content)
            .map_err(|_| OperationEvaluationError::RetainedValueMismatch)
    }

    /// Checks immutable declaration identity and the argument schema before revision admission.
    ///
    /// # Errors
    /// Returns a value-free declaration, JSON or schema error.
    pub fn validate_update_arguments(
        &self,
        state: &DeclaredStateDefinition,
        operation: &ActivityName,
        snapshot: &DeclaredOperationSnapshot,
        arguments: &DeclaredOperationArguments,
    ) -> Result<(), OperationEvaluationError> {
        let update = state
            .update(operation)
            .ok_or(OperationEvaluationError::UndeclaredOperation)?;
        if update.digest() != snapshot.declaration_digest() {
            return Err(OperationEvaluationError::DeclarationMismatch);
        }
        let arguments = json(arguments.canonical(), OperationDataField::Arguments)?;
        validate(
            update.args_schema(),
            &arguments,
            OperationDataField::Arguments,
        )
    }

    /// Evaluates a Query directly from the complete snapshot returned by the durable store.
    ///
    /// # Errors
    /// Returns the same bounded errors as [`Self::query`].
    pub fn query_snapshot(
        &self,
        state: &DeclaredStateDefinition,
        operation: &ActivityName,
        snapshot: &DeclaredOperationSnapshot,
        arguments: &[u8],
    ) -> Result<QueryEvaluation, OperationEvaluationError> {
        if state
            .query(operation)
            .ok_or(OperationEvaluationError::UndeclaredOperation)?
            .digest()
            != snapshot.declaration_digest()
        {
            return Err(OperationEvaluationError::DeclarationMismatch);
        }
        let completed_results = completed_results(snapshot)?;
        let retained_state =
            RetainedOperationValue::new(snapshot.state(), snapshot.state_content())?;
        let context = OperationEvaluationContext::new(
            snapshot.run_version(),
            snapshot.workflow_input().content(),
            retained_state,
            &completed_results,
            arguments,
        )?;
        self.query(state, operation, &context)
    }

    /// Evaluates a named Query over one retained state snapshot.
    ///
    /// # Errors
    /// Returns only bounded, value-free failures for undeclared operations, malformed inputs,
    /// schema mismatches or expression failure.
    pub fn query(
        &self,
        state: &DeclaredStateDefinition,
        operation: &ActivityName,
        context: &OperationEvaluationContext<'_>,
    ) -> Result<QueryEvaluation, OperationEvaluationError> {
        let query = state
            .query(operation)
            .ok_or(OperationEvaluationError::UndeclaredOperation)?;
        let parsed = parsed_context(state, context, query.args_schema())?;
        let result = value(query.expression(), &environment(&parsed, None))?;
        let result = evaluated_value(
            &result,
            query.result_schema(),
            OperationDataField::QueryResult,
        )?;
        Ok(QueryEvaluation::new(
            context.state().value().clone(),
            context.observed_run_version(),
            result,
        ))
    }

    /// Evaluates a named Update from one immutable pre-state.
    ///
    /// A false validator produces a durable business rejection without evaluating state or result.
    /// A successful decision validates and content-addresses both values before it can be passed to
    /// the serialized store callback.
    ///
    /// # Errors
    /// Returns only bounded, value-free failures for undeclared operations, malformed inputs,
    /// schema mismatches or expression failure.
    pub fn update(
        &self,
        state: &DeclaredStateDefinition,
        operation: &ActivityName,
        context: &OperationEvaluationContext<'_>,
    ) -> Result<UpdateEvaluationPlan, OperationEvaluationError> {
        let update = state
            .update(operation)
            .ok_or(OperationEvaluationError::UndeclaredOperation)?;
        let parsed = parsed_context(state, context, update.args_schema())?;
        let pre_state = environment(&parsed, None);
        if update
            .validator()
            .map(|expression| boolean(expression, &pre_state))
            .transpose()?
            == Some(false)
        {
            return Ok(UpdateEvaluationPlan::rejected(
                context.state().value().clone(),
                context.observed_run_version(),
            ));
        }
        let new_state = value(update.state(), &pre_state)?;
        let checked_state =
            evaluated_value(&new_state, state.schema(), OperationDataField::NewState)?;
        let result = value(update.result(), &environment(&parsed, Some(&new_state)))?;
        let checked_result = evaluated_value(
            &result,
            update.result_schema(),
            OperationDataField::UpdateResult,
        )?;
        Ok(UpdateEvaluationPlan::applied(
            context.state().value().clone(),
            context.observed_run_version(),
            checked_state,
            checked_result,
        ))
    }

    /// Evaluates an Update directly under the complete snapshot supplied by the serialized store
    /// callback.
    ///
    /// # Errors
    /// Returns the same bounded errors as [`Self::update`].
    pub fn update_snapshot(
        &self,
        state: &DeclaredStateDefinition,
        operation: &ActivityName,
        snapshot: &DeclaredOperationSnapshot,
        arguments: &DeclaredOperationArguments,
    ) -> Result<UpdateEvaluationPlan, OperationEvaluationError> {
        self.validate_update_arguments(state, operation, snapshot, arguments)?;
        let completed_results = completed_results(snapshot)?;
        let retained_state =
            RetainedOperationValue::new(snapshot.state(), snapshot.state_content())?;
        let context = OperationEvaluationContext::new(
            snapshot.run_version(),
            snapshot.workflow_input().content(),
            retained_state,
            &completed_results,
            arguments.canonical(),
        )?;
        self.update(state, operation, &context)
    }
}

fn snapshot_declaration(
    snapshot: &DeclaredOperationSnapshot,
) -> Result<ParsedDeclaredState, OperationEvaluationError> {
    let parsed = JsonIrCodec
        .parse_declared_state(snapshot.definition().content())
        .map_err(|_| OperationEvaluationError::DeclarationMismatch)?;
    if parsed.parsed().digest != snapshot.definition().digest() {
        return Err(OperationEvaluationError::DeclarationMismatch);
    }
    Ok(parsed)
}

impl DeclaredOperationEvaluator for JsonDeclaredOperationEvaluator {
    fn query(
        &self,
        snapshot: &DeclaredOperationSnapshot,
        operation: &ActivityName,
        arguments: &DeclaredOperationArguments,
    ) -> Result<EvaluatedDeclaredQuery, OperationEvaluationError> {
        let declared = snapshot_declaration(snapshot)?;
        let evaluated =
            self.query_snapshot(declared.state(), operation, snapshot, arguments.canonical())?;
        let response = evaluated.response(snapshot.snapshot().clone())?;
        let result = EvaluatedOperationWrite::restore(
            evaluated.result().value().clone(),
            evaluated.result().content().to_vec(),
        )
        .map_err(|_| OperationEvaluationError::RetainedValueMismatch)?;
        EvaluatedDeclaredQuery::restore(response, result)
    }

    fn validate_update_arguments(
        &self,
        snapshot: &DeclaredOperationSnapshot,
        operation: &ActivityName,
        arguments: &DeclaredOperationArguments,
    ) -> Result<(), OperationEvaluationError> {
        let declared = snapshot_declaration(snapshot)?;
        self.validate_update_arguments(declared.state(), operation, snapshot, arguments)
    }

    fn update(
        &self,
        snapshot: &DeclaredOperationSnapshot,
        operation: &ActivityName,
        arguments: &DeclaredOperationArguments,
    ) -> Result<DeclaredUpdateDecision, OperationEvaluationError> {
        let declared = snapshot_declaration(snapshot)?;
        self.update_snapshot(declared.state(), operation, snapshot, arguments)?
            .store_decision(snapshot)
    }
}
