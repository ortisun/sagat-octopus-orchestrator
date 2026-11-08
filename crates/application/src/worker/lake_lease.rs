//! The lease document of a lake step, from the run's own data (feature 035, T035-056, D035-138).
//!
//! [`lake_attempt_input`] is a pure function of what is given to it. This is the step before it: it evaluates the record source of the
//! step over the data of the run, takes the strictest limits of the step and of the connection, and builds the document. It is still
//! pure (the evaluator and the data are arguments), so it is tested without a database; the lease path decides what to do with a
//! refusal.
//!
//! A refusal is a closed [`DatabaseFailure`], and every one of them is **permanent**: the input of a run does not change, so the same
//! step would fail the same way on a retry. `Configuration` is a record source that does not select a list of objects, `Capability` is
//! a source the lease cannot serve yet (a blob source, whose descriptors the lease would have to mint as a capability, and the result
//! of a SQL step, which the runner's reader refuses by name, G035-067), and `IcebergLimitExceeded` is a document over the bound the
//! runner reads.

use sagat_domain::database::{DatabaseFailure, IcebergWrite, LakeLimits, RecordSource};

use super::lake_input::{LakeFacts, LakeInputError, LakeRecords, lake_attempt_input};
use crate::collection_source::CollectionSource;
use crate::execution::{ExpressionEvaluator, RunData};

/// The `input_json` of the lease of `write`, or the closed failure that says why the step cannot be leased.
///
/// `connection` is the limits of the connection at the revision the run was admitted with.
///
/// # Errors
/// [`DatabaseFailure::Configuration`] when the record source does not select a list of JSON objects;
/// [`DatabaseFailure::Capability`] for a source the lease cannot serve yet;
/// [`DatabaseFailure::IcebergLimitExceeded`] when the document is over the bound the runner reads.
pub fn lake_lease_document<E: ExpressionEvaluator + ?Sized>(
    evaluator: &E,
    write: &IcebergWrite,
    data: &RunData,
    facts: &LakeFacts<'_>,
    connection: LakeLimits,
) -> Result<Vec<u8>, DatabaseFailure> {
    let records = match write.records() {
        RecordSource::Inline(expression) => match evaluator.collection_source(expression, data) {
            Ok(CollectionSource::Items(items)) => LakeRecords::Inline(
                items
                    .iter()
                    .map(|item| serde_json::from_slice(item))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| DatabaseFailure::Configuration)?,
            ),
            // The value is a blob, or the expression fails: inline means the records themselves.
            Ok(CollectionSource::Blob(_)) | Err(_) => return Err(DatabaseFailure::Configuration),
        },
        RecordSource::Blobs(_) | RecordSource::DatabaseResult(_) => {
            return Err(DatabaseFailure::Capability);
        }
    };
    lake_attempt_input(write, facts, records, write.limits().effective(connection)).map_err(
        |error| match error {
            LakeInputError::TooLarge => DatabaseFailure::IcebergLimitExceeded,
            LakeInputError::Record | LakeInputError::Blobs | LakeInputError::EffectKey => {
                DatabaseFailure::Configuration
            }
        },
    )
}
