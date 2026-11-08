//! The lease document of a lake step, from the run's own data (feature 035, T035-056, D035-138).
//!
//! [`lake_attempt_input`] is a pure function of what is given to it. This is the step before it: it evaluates the record source of the
//! step over the data of the run, takes the strictest limits of the step and of the connection, and builds the document. It is still
//! pure (the evaluator, the data and what the catalog said about the blobs are arguments), so it is tested without a database; the
//! lease path decides what to do with a refusal.
//!
//! A refusal is a closed [`DatabaseFailure`], and every one of them is **permanent**: the input of a run does not change, so the same
//! step would fail the same way on a retry. `Configuration` is a record source that does not select what the step says it does,
//! `Capability` is a source the lease cannot serve yet (the result of a SQL step, which the runner's reader refuses by name, G035-067),
//! `ResultUnavailable` is a blob the catalog does not vouch for, and `IcebergLimitExceeded` is a document, or a set of blobs, over the
//! bound the runner reads.

use std::collections::BTreeSet;

use sagat_domain::blobs::{BlobDescriptor, BlobId};
use sagat_domain::database::{
    DatabaseFailure, IcebergWrite, LakeLimits, MAX_RECORD_BLOBS, MAX_RECORD_INPUT_BYTES,
    RecordSource,
};

use super::lake_input::{
    BlobFormat, LakeBlob, LakeFacts, LakeInputError, LakeRecords, lake_attempt_input,
};
use crate::collection_source::{CollectionSource, parse_descriptor};
use crate::execution::{ExpressionEvaluator, RunData};

/// What the record source of a step names, before anything has vouched for it.
#[derive(Debug, Clone, PartialEq)]
pub enum NamedRecords {
    /// The records themselves, each a JSON value (the document builder refuses one that is not an object).
    Inline(Vec<serde_json::Value>),
    /// The blobs the expression names, as the run's data described them. A descriptor in a JSON value proves nothing: only what the
    /// catalog says about the blob counts.
    Blobs(Vec<BlobDescriptor>),
}

impl NamedRecords {
    /// The blobs a source names, in the order it names them: what the catalog is asked about (and nothing for inline records).
    #[must_use]
    pub fn blob_ids(&self) -> Vec<BlobId> {
        match self {
            Self::Inline(_) => Vec::new(),
            Self::Blobs(named) => named.iter().map(BlobDescriptor::id).collect(),
        }
    }
}

/// The blobs a `blobs` expression selected: a list of one to [`MAX_RECORD_BLOBS`] descriptors, or a lone descriptor (a list of one),
/// each blob once. The count is held before a descriptor is read.
fn named_blobs(selected: CollectionSource) -> Result<Vec<BlobDescriptor>, DatabaseFailure> {
    let named = match selected {
        CollectionSource::Blob(descriptor) => vec![descriptor],
        CollectionSource::Items(items) => {
            if !(1..=MAX_RECORD_BLOBS).contains(&items.len()) {
                return Err(DatabaseFailure::Configuration);
            }
            items
                .iter()
                .map(|item| parse_descriptor(item))
                .collect::<Option<Vec<_>>>()
                .ok_or(DatabaseFailure::Configuration)?
        }
    };
    let mut seen = BTreeSet::new();
    if named.iter().all(|descriptor| seen.insert(descriptor.id())) {
        Ok(named)
    } else {
        Err(DatabaseFailure::Configuration)
    }
}

/// The blobs of the document, each vouched for by the catalog: the descriptor the run's data holds must be the one the catalog holds
/// (size, digest and media type, not only the id), the media type decides the format, and the sizes together stay within the
/// platform ceiling. The step's order is kept; a blob the catalog returned and the step did not name is not in the document.
fn admitted_blobs(
    named: &[BlobDescriptor],
    found: &[BlobDescriptor],
) -> Result<Vec<LakeBlob>, DatabaseFailure> {
    let mut total: u64 = 0;
    let mut admitted = Vec::with_capacity(named.len());
    for descriptor in named {
        let vouched = found
            .iter()
            .find(|candidate| candidate.id() == descriptor.id())
            .filter(|candidate| *candidate == descriptor)
            .ok_or(DatabaseFailure::ResultUnavailable)?;
        let format = BlobFormat::of_media_type(vouched.content_type().as_str())
            .ok_or(DatabaseFailure::Configuration)?;
        total = total
            .checked_add(vouched.size())
            .ok_or(DatabaseFailure::IcebergLimitExceeded)?;
        admitted.push(LakeBlob {
            id: vouched.id(),
            format,
        });
    }
    if total > MAX_RECORD_INPUT_BYTES {
        return Err(DatabaseFailure::IcebergLimitExceeded);
    }
    Ok(admitted)
}

/// What the record source of `write` names over the data of the run.
///
/// # Errors
/// [`DatabaseFailure::Configuration`] when the source does not select what it says it does;
/// [`DatabaseFailure::Capability`] for a source the lease cannot serve yet.
pub fn named_records<E: ExpressionEvaluator + ?Sized>(
    evaluator: &E,
    write: &IcebergWrite,
    data: &RunData,
) -> Result<NamedRecords, DatabaseFailure> {
    match write.records() {
        RecordSource::Inline(expression) => match evaluator.collection_source(expression, data) {
            Ok(CollectionSource::Items(items)) => items
                .iter()
                .map(|item| serde_json::from_slice(item))
                .collect::<Result<Vec<_>, _>>()
                .map(NamedRecords::Inline)
                .map_err(|_| DatabaseFailure::Configuration),
            // The value is a blob, or the expression fails: inline means the records themselves.
            Ok(CollectionSource::Blob(_)) | Err(_) => Err(DatabaseFailure::Configuration),
        },
        RecordSource::Blobs(expression) => evaluator
            .collection_source(expression, data)
            .map_err(|_| DatabaseFailure::Configuration)
            .and_then(named_blobs)
            .map(NamedRecords::Blobs),
        // The result the SQL step retained, read from the run's own data: a node that produced none (it failed, or an error edge went
        // around it) leaves nothing to write.
        RecordSource::DatabaseResult(node) => data
            .root_output(node)
            .ok_or(DatabaseFailure::ResultUnavailable)
            .and_then(super::lake_result::result_records)
            .map(NamedRecords::Inline),
    }
}

/// The `input_json` of the lease of `write`, or the closed failure that says why the step cannot be leased.
///
/// `connection` is the limits of the connection at the revision the run was admitted with. `found` is what the catalog says about the
/// blobs of a blob source (nothing for an inline one): a blob that is not in it is not the run's to read.
///
/// # Errors
/// [`DatabaseFailure::Configuration`] when the records are not what the step says;
/// [`DatabaseFailure::ResultUnavailable`] for a blob the catalog does not vouch for;
/// [`DatabaseFailure::IcebergLimitExceeded`] when the document, or the blobs together, are over the bound the runner reads.
pub fn lake_lease_document(
    write: &IcebergWrite,
    facts: &LakeFacts<'_>,
    records: NamedRecords,
    found: &[BlobDescriptor],
    connection: LakeLimits,
) -> Result<Vec<u8>, DatabaseFailure> {
    let records = match records {
        NamedRecords::Inline(items) => LakeRecords::Inline(items),
        NamedRecords::Blobs(named) => LakeRecords::Blobs(admitted_blobs(&named, found)?),
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
