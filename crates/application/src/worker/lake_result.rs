//! The result a SQL step retained, as the records of a lake write (feature 035, D035-145, G035-067).
//!
//! A SQL step keeps its result as the typed document of feature 017: the columns, and rows of typed cells. The records the host reads
//! are objects, so a row becomes one, keyed by its column, each value as the cell holds it (an integer and a decimal are the strings
//! of their digits, which the host reads exactly; a timestamp and a date are text; a JSON value is itself), and a `binary` cell is kept
//! as the typed cell, which is how the host reads a binary column (`docs/guides/iceberg-type-mapping.md`).
//!
//! The document was validated before it was kept, by the validator of the platform, so what cannot be read here is a result that was
//! damaged afterwards, or that is not a result at all: `DATABASE_RESULT_UNAVAILABLE`. What the step's author wrote and cannot be
//! records (a statement that selects no column, two columns of one name) is `DATABASE_CONFIGURATION_INVALID`. Both are permanent: a
//! result does not change, so a retry would fail the same way. A result above the inline bound is a blob (`{"$blob": …}`) and is refused
//! by name, `DATABASE_CAPABILITY_UNAVAILABLE`, until the lease reads it.

use std::collections::BTreeSet;

use sagat_domain::database::DatabaseFailure;
use serde_json::{Map, Value};

use crate::collection_source::parse_descriptor;

const UNAVAILABLE: DatabaseFailure = DatabaseFailure::ResultUnavailable;

/// An object with exactly these members: the validator that kept the document refuses any other, so does this.
fn exactly<'a>(
    value: &'a Value,
    members: &[&str],
) -> Result<&'a Map<String, Value>, DatabaseFailure> {
    let object = value.as_object().ok_or(UNAVAILABLE)?;
    if object.len() == members.len() && members.iter().all(|name| object.contains_key(*name)) {
        Ok(object)
    } else {
        Err(UNAVAILABLE)
    }
}

/// The records of the retained result in `content`.
///
/// # Errors
/// [`DatabaseFailure::Capability`] for a result that is a blob;
/// [`DatabaseFailure::ResultUnavailable`] for a document that is not a whole, readable result;
/// [`DatabaseFailure::Configuration`] for a result that cannot be records (no column, a column twice).
pub fn result_records(content: &[u8]) -> Result<Vec<Value>, DatabaseFailure> {
    if parse_descriptor(content).is_some() {
        return Err(DatabaseFailure::Capability);
    }
    let document: Value = serde_json::from_slice(content).map_err(|_| UNAVAILABLE)?;
    let retained = exactly(&document, &["columns", "rows", "affectedRows", "truncated"])?;
    // A result cut short by its limits is not what the statement selected: it is never retained, and never written.
    if retained["truncated"] != Value::Bool(false) || !retained["affectedRows"].is_string() {
        return Err(UNAVAILABLE);
    }
    let columns = retained["columns"].as_array().ok_or(UNAVAILABLE)?;
    let mut names = Vec::with_capacity(columns.len());
    for column in columns {
        let column = exactly(column, &["name", "type"])?;
        if !column["type"].is_string() {
            return Err(UNAVAILABLE);
        }
        names.push(column["name"].as_str().ok_or(UNAVAILABLE)?);
    }
    let distinct: BTreeSet<&str> = names.iter().copied().collect();
    if names.is_empty() || distinct.len() != names.len() || distinct.contains("") {
        return Err(DatabaseFailure::Configuration);
    }
    retained["rows"]
        .as_array()
        .ok_or(UNAVAILABLE)?
        .iter()
        .map(|row| {
            let cells = row.as_array().ok_or(UNAVAILABLE)?;
            if cells.len() != names.len() {
                return Err(UNAVAILABLE);
            }
            let mut record = Map::with_capacity(cells.len());
            for (name, cell) in names.iter().zip(cells) {
                record.insert((*name).to_owned(), value_of(cell)?);
            }
            Ok(Value::Object(record))
        })
        .collect()
}

/// A cell as the host reads it: its value, except a binary one, which stays the typed cell.
fn value_of(cell: &Value) -> Result<Value, DatabaseFailure> {
    let cell = exactly(cell, &["type", "value"])?;
    let kind = cell["type"].as_str().ok_or(UNAVAILABLE)?;
    Ok(if kind == "binary" {
        Value::Object(cell.clone())
    } else {
        cell["value"].clone()
    })
}
