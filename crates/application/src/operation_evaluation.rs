//! Capability-free boundary for declared Query and Update evaluation.
//!
//! Values crossing this boundary are retained JSON bytes and content-addressed references. The
//! evaluator receives no storage, network, secret, clock or callback capability. Byte content is
//! deliberately absent from `Debug` output.

use std::fmt;

use sagat_domain::execution::{MAX_OPERATION_VALUE_BYTES, OperationValue};
use sagat_domain::workflow::Digest;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

pub const MAX_OPERATION_CONTEXT_BYTES: usize = 8 * 1024 * 1024;

/// Fixed value positions used in safe errors and metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationDataField {
    WorkflowInput,
    State,
    CompletedResults,
    Arguments,
    QueryResult,
    NewState,
    UpdateResult,
}

impl fmt::Display for OperationDataField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WorkflowInput => "workflow_input",
            Self::State => "state",
            Self::CompletedResults => "completed_results",
            Self::Arguments => "arguments",
            Self::QueryResult => "query_result",
            Self::NewState => "new_state",
            Self::UpdateResult => "update_result",
        })
    }
}

/// Safe failures of the declarative evaluator. No variant contains workflow data or expression
/// source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OperationEvaluationError {
    #[error("{field} is not valid JSON")]
    MalformedJson { field: OperationDataField },
    #[error("{field} does not match its pinned schema")]
    SchemaMismatch { field: OperationDataField },
    #[error("the pinned schema for {field} is unavailable")]
    InvalidSchema { field: OperationDataField },
    #[error("the declared operation does not exist")]
    UndeclaredOperation,
    #[error("the evaluated declaration does not match the immutable workflow declaration")]
    DeclarationMismatch,
    #[error("the bounded declared expression failed")]
    ExpressionFailed,
    #[error("operation arguments exceed the retained value limit")]
    ArgumentsTooLarge,
    #[error("the declared operation context exceeds its byte limit")]
    ContextTooLarge,
    #[error("retained operation bytes do not match their content address")]
    RetainedValueMismatch,
    #[error("a schema-checked operation value exceeds its byte limit")]
    ValueTooLarge,
    #[error("the operation state changed before its evaluation was consumed")]
    StateChanged,
}

/// Bounded, canonical JSON arguments used both by evaluation and request deduplication.
#[derive(Clone, PartialEq, Eq)]
pub struct DeclaredOperationArguments {
    canonical: Vec<u8>,
}

impl DeclaredOperationArguments {
    /// Parses and canonicalizes one operation argument value.
    ///
    /// # Errors
    /// Returns a value-free error for malformed or oversized arguments.
    pub fn parse(content: &[u8]) -> Result<Self, OperationEvaluationError> {
        if content.len() > MAX_OPERATION_VALUE_BYTES as usize {
            return Err(OperationEvaluationError::ArgumentsTooLarge);
        }
        let value: Value = serde_json::from_slice(content).map_err(|_| {
            OperationEvaluationError::MalformedJson {
                field: OperationDataField::Arguments,
            }
        })?;
        let mut canonical = Vec::with_capacity(content.len());
        write_canonical_json(&value, &mut canonical);
        if canonical.len() > MAX_OPERATION_VALUE_BYTES as usize {
            return Err(OperationEvaluationError::ArgumentsTooLarge);
        }
        Ok(Self { canonical })
    }

    #[must_use]
    pub fn canonical(&self) -> &[u8] {
        &self.canonical
    }
}

impl fmt::Debug for DeclaredOperationArguments {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeclaredOperationArguments")
            .field("size_bytes", &self.canonical.len())
            .finish_non_exhaustive()
    }
}

fn write_canonical_json(value: &Value, output: &mut Vec<u8>) {
    match value {
        Value::Object(values) => {
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            output.push(b'{');
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_json_string(key, output);
                output.push(b':');
                write_canonical_json(value, output);
            }
            output.push(b'}');
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(value, output);
            }
            output.push(b']');
        }
        Value::String(value) => write_json_string(value, output),
        other => output.extend_from_slice(other.to_string().as_bytes()),
    }
}

fn write_json_string(value: &str, output: &mut Vec<u8>) {
    if let Ok(escaped) = serde_json::to_string(value) {
        output.extend_from_slice(escaped.as_bytes());
    }
}

/// Borrowed retained bytes paired with the exact content address read from storage.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RetainedOperationValue<'a> {
    value: &'a OperationValue,
    content: &'a [u8],
}

impl<'a> RetainedOperationValue<'a> {
    /// Checks the size and digest before content can enter the evaluator.
    ///
    /// # Errors
    /// Returns [`OperationEvaluationError::RetainedValueMismatch`] for mismatched retained bytes.
    pub fn new(
        value: &'a OperationValue,
        content: &'a [u8],
    ) -> Result<Self, OperationEvaluationError> {
        let size_matches = usize::try_from(value.size_bytes()).ok() == Some(content.len());
        let digest: [u8; 32] = Sha256::digest(content).into();
        if !size_matches || value.digest() != Digest::new(digest) {
            return Err(OperationEvaluationError::RetainedValueMismatch);
        }
        Ok(Self { value, content })
    }

    #[must_use]
    pub const fn value(self) -> &'a OperationValue {
        self.value
    }

    #[must_use]
    pub const fn content(self) -> &'a [u8] {
        self.content
    }
}

impl fmt::Debug for RetainedOperationValue<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetainedOperationValue")
            .field("size_bytes", &self.content.len())
            .finish_non_exhaustive()
    }
}

/// The only data visible while evaluating one declared operation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct OperationEvaluationContext<'a> {
    observed_run_version: u64,
    workflow_input: &'a [u8],
    state: RetainedOperationValue<'a>,
    completed_results: &'a [u8],
    arguments: &'a [u8],
}

impl<'a> OperationEvaluationContext<'a> {
    /// Creates a context after enforcing the operation argument and aggregate context bounds.
    ///
    /// # Errors
    /// Returns a bounded error when arguments or aggregate retained bytes exceed their limits.
    pub fn new(
        observed_run_version: u64,
        workflow_input: &'a [u8],
        state: RetainedOperationValue<'a>,
        completed_results: &'a [u8],
        arguments: &'a [u8],
    ) -> Result<Self, OperationEvaluationError> {
        if arguments.len() > MAX_OPERATION_VALUE_BYTES as usize {
            return Err(OperationEvaluationError::ArgumentsTooLarge);
        }
        let total = workflow_input
            .len()
            .checked_add(state.content.len())
            .and_then(|size| size.checked_add(completed_results.len()))
            .and_then(|size| size.checked_add(arguments.len()))
            .ok_or(OperationEvaluationError::ContextTooLarge)?;
        if total > MAX_OPERATION_CONTEXT_BYTES {
            return Err(OperationEvaluationError::ContextTooLarge);
        }
        Ok(Self {
            observed_run_version,
            workflow_input,
            state,
            completed_results,
            arguments,
        })
    }

    #[must_use]
    pub const fn observed_run_version(&self) -> u64 {
        self.observed_run_version
    }

    #[must_use]
    pub const fn workflow_input(&self) -> &'a [u8] {
        self.workflow_input
    }

    #[must_use]
    pub const fn state(&self) -> RetainedOperationValue<'a> {
        self.state
    }

    #[must_use]
    pub const fn completed_results(&self) -> &'a [u8] {
        self.completed_results
    }

    #[must_use]
    pub const fn arguments(&self) -> &'a [u8] {
        self.arguments
    }
}

impl fmt::Debug for OperationEvaluationContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationEvaluationContext")
            .field("observed_run_version", &self.observed_run_version)
            .field("workflow_input_size", &self.workflow_input.len())
            .field("state_size", &self.state.content.len())
            .field("completed_results_size", &self.completed_results.len())
            .field("arguments_size", &self.arguments.len())
            .finish_non_exhaustive()
    }
}
